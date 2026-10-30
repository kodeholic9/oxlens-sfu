// author: kodeholic (powered by Claude)
// spec: v1.6 · 운영:RULE-LOG-SENDER · 운영:RULE-LOG-LOGGER · 정:RULE-OPS-LOGGER · 운영:RULE-LOG-LEVEL · model: claude-opus-5-5
//! 보내는 쪽 — 유한 채널(차면 드롭 + 계수) · 전담 스레드 하나 · 소켓 / hub 안 로거 채널 / stdout · 실패 즉시 spool · 회복 첫 줄.

use std::io::Write;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::line::{Head, Kind, Line};
use crate::policy::LoggingLevel;

#[derive(Debug, Default)]
pub struct Counters {
    pub queue_full: AtomicU64,
    pub spool_full: AtomicU64,
    pub sent: AtomicU64,
    pub spooled: AtomicU64,
    pub eagain: AtomicU64,
    pub emsgsize: AtomicU64,
    pub absent: AtomicU64,
}

#[derive(Debug, Clone)]
pub enum Sink {
    Stdout,
    Socket(PathBuf),
    Inproc,
}

pub struct Sender {
    tx: SyncSender<String>,
    pub head: Head,
    pub epoch: Option<String>,
    pub line_max: usize,
    pub counters: Arc<Counters>,
    started: Instant,
    level: AtomicU8,
    base_level: AtomicU8,
    level_until_ms: AtomicU64,
    inproc: Arc<Mutex<Option<SyncSender<String>>>>,
    inproc_running: Arc<AtomicBool>,
}

pub struct SenderConfig {
    pub node: String,
    pub proc_name: &'static str,
    pub epoch: Option<String>,
    pub sink: Sink,
    pub log_dir: Option<PathBuf>,
    pub line_max: usize,
    pub queue_max: usize,
    pub spool_max: u64,
    pub level: LoggingLevel,
}

impl Sender {
    pub fn start(cfg: SenderConfig) -> Arc<Sender> {
        let (tx, rx) = sync_channel::<String>(cfg.queue_max.max(1));
        let counters = Arc::new(Counters::default());
        let inproc: Arc<Mutex<Option<SyncSender<String>>>> = Arc::new(Mutex::new(None));
        let inproc_running = Arc::new(AtomicBool::new(false));
        let s = Arc::new(Sender {
            tx,
            head: Head { node: cfg.node.clone(), proc_name: cfg.proc_name },
            epoch: cfg.epoch.clone(),
            line_max: cfg.line_max,
            counters: counters.clone(),
            started: Instant::now(),
            level: AtomicU8::new(cfg.level.rank()),
            base_level: AtomicU8::new(cfg.level.rank()),
            level_until_ms: AtomicU64::new(0),
            inproc: inproc.clone(),
            inproc_running: inproc_running.clone(),
        });
        let spool_path = cfg
            .log_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(format!("{}.{}.spool.log", cfg.node, cfg.proc_name));
        let worker = Worker {
            rx,
            sink: cfg.sink,
            counters,
            spool_path,
            spool: None,
            spool_bytes: 0,
            spool_max: cfg.spool_max,
            failing: false,
            outage_spooled: 0,
            outage_dropped: 0,
            inproc,
            inproc_running,
            head: s.head.clone(),
            epoch: cfg.epoch,
            started: s.started,
            line_max: cfg.line_max,
            sock: None,
        };
        let _ = std::thread::Builder::new().name("log-sender".into()).spawn(move || worker.run());
        s
    }

    pub fn mono_us(&self) -> u64 {
        self.started.elapsed().as_micros() as u64
    }

    pub fn emit(&self, mut line: Line) {
        if let Some(e) = &self.epoch {
            line.set_key_if_absent("epoch", e);
        }
        let s = line.render(&self.head, &super::line::ts_now(), self.mono_us(), self.line_max);
        self.push(s);
    }

    pub fn push(&self, s: String) {
        match self.tx.try_send(s) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn attach_inproc(&self, tx: SyncSender<String>) {
        if let Ok(mut g) = self.inproc.lock() {
            g.replace(tx);
        }
        self.inproc_running.store(true, Ordering::Release);
    }

    pub fn detach_inproc(&self) {
        self.inproc_running.store(false, Ordering::Release);
        if let Ok(mut g) = self.inproc.lock() {
            g.take();
        }
    }

    pub fn level(&self, now_ms: u64) -> LoggingLevel {
        let until = self.level_until_ms.load(Ordering::Acquire);
        if until != 0 && now_ms >= until {
            self.level_until_ms.store(0, Ordering::Release);
            self.level.store(self.base_level.load(Ordering::Acquire), Ordering::Release);
        }
        rank_level(self.level.load(Ordering::Acquire))
    }

    pub fn set_level(&self, level: LoggingLevel, until_ms: Option<u64>) {
        self.level.store(level.rank(), Ordering::Release);
        self.level_until_ms.store(until_ms.unwrap_or(0), Ordering::Release);
        if until_ms.is_none() {
            self.base_level.store(level.rank(), Ordering::Release);
        }
    }

    pub fn level_override(&self) -> Option<(LoggingLevel, u64)> {
        let until = self.level_until_ms.load(Ordering::Acquire);
        if until == 0 {
            return None;
        }
        Some((rank_level(self.level.load(Ordering::Acquire)), until))
    }

    pub fn base_level(&self) -> LoggingLevel {
        rank_level(self.base_level.load(Ordering::Acquire))
    }
}

fn rank_level(r: u8) -> LoggingLevel {
    match r {
        0 => LoggingLevel::Trace,
        1 => LoggingLevel::Debug,
        2 => LoggingLevel::Info,
        3 => LoggingLevel::Warn,
        _ => LoggingLevel::Error,
    }
}

struct Worker {
    rx: Receiver<String>,
    sink: Sink,
    counters: Arc<Counters>,
    spool_path: PathBuf,
    spool: Option<std::fs::File>,
    spool_bytes: u64,
    spool_max: u64,
    failing: bool,
    outage_spooled: u64,
    outage_dropped: u64,
    inproc: Arc<Mutex<Option<SyncSender<String>>>>,
    inproc_running: Arc<AtomicBool>,
    head: Head,
    epoch: Option<String>,
    started: Instant,
    line_max: usize,
    sock: Option<UnixDatagram>,
}

impl Worker {
    fn run(mut self) {
        while let Ok(line) = self.rx.recv() {
            self.deliver(line);
        }
    }

    fn deliver(&mut self, line: String) {
        if matches!(self.sink, Sink::Stdout) {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{line}");
            self.counters.sent.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if self.failing && self.try_send(&self.recovery_line()) {
            self.failing = false;
            self.outage_spooled = 0;
            self.outage_dropped = 0;
        }
        if !self.failing && self.try_send(&line) {
            self.counters.sent.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.failing = true;
        self.spool_write(&line);
    }

    fn recovery_line(&self) -> String {
        let mut l = Line::new(Kind::Stat).rec("spooled", self.outage_spooled).rec("dropped", self.outage_dropped);
        if let Some(e) = &self.epoch {
            l.set_key_if_absent("epoch", e);
        }
        l.render(&self.head, &super::line::ts_now(), self.started.elapsed().as_micros() as u64, self.line_max)
    }

    fn try_send(&mut self, line: &str) -> bool {
        match &self.sink {
            Sink::Stdout => true,
            Sink::Inproc => {
                if !self.inproc_running.load(Ordering::Acquire) {
                    self.counters.absent.fetch_add(1, Ordering::Relaxed);
                    return false;
                }
                let g = match self.inproc.lock() {
                    Ok(g) => g,
                    Err(_) => return false,
                };
                match g.as_ref() {
                    Some(tx) => match tx.try_send(line.to_string()) {
                        Ok(()) => true,
                        Err(TrySendError::Full(_)) => {
                            self.counters.eagain.fetch_add(1, Ordering::Relaxed);
                            false
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            self.counters.absent.fetch_add(1, Ordering::Relaxed);
                            false
                        }
                    },
                    None => {
                        self.counters.absent.fetch_add(1, Ordering::Relaxed);
                        false
                    }
                }
            }
            Sink::Socket(path) => {
                let path = path.clone();
                if self.sock.is_none() {
                    match UnixDatagram::unbound() {
                        Ok(s) => {
                            let _ = s.set_nonblocking(true);
                            self.sock = Some(s);
                        }
                        Err(_) => return false,
                    }
                }
                let Some(sock) = &self.sock else { return false };
                match sock.send_to(line.as_bytes(), &path) {
                    Ok(_) => true,
                    Err(e) => {
                        let c = match e.raw_os_error() {
                            Some(35) | Some(11) | Some(55) => &self.counters.eagain,
                            Some(40) | Some(90) => &self.counters.emsgsize,
                            _ => &self.counters.absent,
                        };
                        c.fetch_add(1, Ordering::Relaxed);
                        false
                    }
                }
            }
        }
    }

    fn spool_write(&mut self, line: &str) {
        let n = line.len() as u64 + 1;
        if self.spool_bytes + n > self.spool_max {
            self.counters.spool_full.fetch_add(1, Ordering::Relaxed);
            self.outage_dropped += 1;
            return;
        }
        if self.spool.is_none() {
            match std::fs::OpenOptions::new().create(true).append(true).open(&self.spool_path) {
                Ok(f) => {
                    self.spool_bytes = f.metadata().map(|m| m.len()).unwrap_or(0);
                    self.spool = Some(f);
                }
                Err(_) => {
                    self.counters.spool_full.fetch_add(1, Ordering::Relaxed);
                    self.outage_dropped += 1;
                    return;
                }
            }
        }
        let ok = match self.spool.as_mut() {
            Some(f) => writeln!(f, "{line}").is_ok(),
            None => false,
        };
        if ok {
            self.spool_bytes += n;
            self.counters.spooled.fetch_add(1, Ordering::Relaxed);
            self.outage_spooled += 1;
        } else {
            self.counters.spool_full.fetch_add(1, Ordering::Relaxed);
            self.outage_dropped += 1;
        }
    }
}
