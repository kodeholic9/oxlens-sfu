// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-LOGGER · 정:RULE-CONC-SIX · 정:RULE-OPS-OBSERVE · 운영 규격서 §7-2 · model: claude-opus-5-5
//! 보내는 쪽 — 유한 채널(차면 드롭 + 계수) → 전담 스레드 → DGRAM 소켓. 실패는 즉시 spool · 소켓이 없으면 stdout.

use std::io::Write;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct Counters {
    pub queue_full: AtomicU64,
    pub eagain: AtomicU64,
    pub emsgsize: AtomicU64,
    pub absent: AtomicU64,
    pub other: AtomicU64,
    pub spooled: AtomicU64,
    pub dropped: AtomicU64,
}

#[derive(Debug, Clone)]
pub struct SenderConfig {
    pub socket: Option<PathBuf>,
    pub spool: Option<PathBuf>,
    pub queue_max: usize,
    pub spool_max: u64,
}

#[derive(Clone)]
pub struct Sender {
    tx: SyncSender<Vec<u8>>,
    pub counters: Arc<Counters>,
}

impl Sender {
    pub fn start(cfg: SenderConfig, stat_line: Arc<dyn Fn(u64, u64) -> String + Send + Sync>) -> Sender {
        let (tx, rx) = sync_channel::<Vec<u8>>(cfg.queue_max.max(1));
        let counters = Arc::new(Counters::default());
        let c2 = counters.clone();
        let _ = std::thread::Builder::new().name("ox-log-send".into()).spawn(move || run(cfg, rx, c2, stat_line));
        Sender { tx, counters }
    }

    pub fn push(&self, line: Vec<u8>) {
        match self.tx.try_send(line) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
                self.counters.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

struct Spool {
    path: Option<PathBuf>,
    file: Option<std::fs::File>,
    size: u64,
    max: u64,
}

impl Spool {
    fn write(&mut self, line: &[u8], c: &Counters) {
        let Some(path) = self.path.clone() else {
            c.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if self.file.is_none() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                Ok(f) => {
                    self.size = f.metadata().map(|m| m.len()).unwrap_or(0);
                    self.file = Some(f);
                }
                Err(_) => {
                    c.dropped.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            }
        }
        if self.size + line.len() as u64 + 1 > self.max {
            c.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let ok = self.file.as_mut().map(|f| f.write_all(line).and_then(|_| f.write_all(b"\n")).is_ok()).unwrap_or(false);
        if ok {
            self.size += line.len() as u64 + 1;
            c.spooled.fetch_add(1, Ordering::Relaxed);
        } else {
            c.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn run(cfg: SenderConfig, rx: Receiver<Vec<u8>>, c: Arc<Counters>, stat_line: Arc<dyn Fn(u64, u64) -> String + Send + Sync>) {
    let mut spool = Spool { path: cfg.spool.clone(), file: None, size: 0, max: cfg.spool_max };
    let Some(target) = cfg.socket.clone() else {
        let out = std::io::stdout();
        while let Ok(line) = rx.recv() {
            let mut lock = out.lock();
            let _ = lock.write_all(&line);
            let _ = lock.write_all(b"\n");
            let _ = lock.flush();
        }
        return;
    };
    let sock = UnixDatagram::unbound().ok();
    if let Some(s) = &sock {
        let _ = s.set_nonblocking(true);
    }
    let mut failing = false;
    while let Ok(line) = rx.recv() {
        let sent = match &sock {
            Some(s) => s.send_to(&line, &target),
            None => Err(std::io::Error::from(std::io::ErrorKind::NotConnected)),
        };
        match sent {
            Ok(_) => {
                if failing {
                    failing = false;
                    let stat = stat_line(c.spooled.load(Ordering::Relaxed), c.dropped.load(Ordering::Relaxed));
                    if let Some(s) = &sock {
                        let _ = s.send_to(stat.as_bytes(), &target);
                    }
                }
            }
            Err(e) => {
                failing = true;
                let os = e.raw_os_error();
                if e.kind() == std::io::ErrorKind::WouldBlock || os == Some(libc::ENOBUFS) {
                    c.eagain.fetch_add(1, Ordering::Relaxed);
                } else if os == Some(libc::EMSGSIZE) {
                    c.emsgsize.fetch_add(1, Ordering::Relaxed);
                } else if matches!(e.kind(), std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound) {
                    c.absent.fetch_add(1, Ordering::Relaxed);
                } else {
                    c.other.fetch_add(1, Ordering::Relaxed);
                }
                spool.write(&line, &c);
            }
        }
    }
}
