// author: kodeholic (powered by Claude)
// spec: v1.4 · 운영:RULE-LOG-SENDER · 운영:RULE-LOG-LOGGER · 정:RULE-OPS-LOGGER · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! 보내는 쪽 — 유한 채널 → 전담 스레드 하나 → DGRAM 소켓(비차단) · 실패 즉시 spool · 소켓 미설정이면 stdout · 회복 첫 줄 `kind=stat spooled=N dropped=M`.

use std::io::Write;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};

pub static QUEUE_FULL: AtomicU64 = AtomicU64::new(0);
pub static SPOOL_FULL: AtomicU64 = AtomicU64::new(0);
pub static SPOOLED: AtomicU64 = AtomicU64::new(0);
pub static SEND_EAGAIN: AtomicU64 = AtomicU64::new(0);
pub static SEND_EMSGSIZE: AtomicU64 = AtomicU64::new(0);
pub static SEND_ABSENT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum Sink {
    Stdout,
    Socket { path: PathBuf, spool: PathBuf, spool_max: u64 },
    Channel(SyncSender<String>),
}

#[derive(Debug, Clone)]
pub struct Tx(SyncSender<String>);

impl Tx {
    pub fn send(&self, line: String) {
        match self.0.try_send(line) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                QUEUE_FULL.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

pub fn drops() -> Vec<(&'static str, u64)> {
    vec![("log_queue_full", QUEUE_FULL.load(Ordering::Relaxed)), ("log_spool_full", SPOOL_FULL.load(Ordering::Relaxed))]
}

pub fn start(sink: Sink, queue_max: usize) -> Tx {
    let (tx, rx) = std::sync::mpsc::sync_channel::<String>(queue_max.max(1));
    let _ = std::thread::Builder::new().name("ox-log-tx".into()).spawn(move || run(sink, rx));
    Tx(tx)
}

struct Spool {
    path: PathBuf,
    max: u64,
    file: Option<std::fs::File>,
    size: u64,
    spooled: u64,
    dropped: u64,
}

impl Spool {
    fn write(&mut self, line: &str) {
        if self.file.is_none() {
            if let Some(dir) = self.path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
                Ok(f) => {
                    self.size = f.metadata().map(|m| m.len()).unwrap_or(0);
                    self.file = Some(f);
                }
                Err(_) => {
                    self.drop_one();
                    return;
                }
            }
        }
        let need = line.len() as u64 + 1;
        if self.size + need > self.max {
            self.drop_one();
            return;
        }
        let ok = self.file.as_mut().is_some_and(|f| f.write_all(line.as_bytes()).and_then(|_| f.write_all(b"\n")).is_ok());
        if ok {
            self.size += need;
            self.spooled += 1;
            SPOOLED.fetch_add(1, Ordering::Relaxed);
        } else {
            self.drop_one();
        }
    }

    fn drop_one(&mut self) {
        self.dropped += 1;
        SPOOL_FULL.fetch_add(1, Ordering::Relaxed);
    }
}

fn send_dgram(sock: &UnixDatagram, path: &PathBuf, line: &str) -> bool {
    match sock.send_to(line.as_bytes(), path) {
        Ok(_) => true,
        Err(e) => {
            match e.raw_os_error() {
                Some(code) if code == libc_emsgsize() => {
                    SEND_EMSGSIZE.fetch_add(1, Ordering::Relaxed);
                }
                _ if e.kind() == std::io::ErrorKind::WouldBlock => {
                    SEND_EAGAIN.fetch_add(1, Ordering::Relaxed);
                }
                _ => {
                    SEND_ABSENT.fetch_add(1, Ordering::Relaxed);
                }
            }
            false
        }
    }
}

fn libc_emsgsize() -> i32 {
    if cfg!(target_os = "linux") { 90 } else { 40 }
}

fn run(sink: Sink, rx: Receiver<String>) {
    match sink {
        Sink::Stdout => {
            let out = std::io::stdout();
            for line in rx {
                let mut o = out.lock();
                let _ = o.write_all(line.as_bytes());
                let _ = o.write_all(b"\n");
                let _ = o.flush();
            }
        }
        Sink::Channel(next) => {
            for line in rx {
                if next.send(line).is_err() {
                    return;
                }
            }
        }
        Sink::Socket { path, spool, spool_max } => {
            let sock = UnixDatagram::unbound().ok();
            if let Some(s) = &sock {
                let _ = s.set_nonblocking(true);
            }
            let mut sp = Spool { path: spool, max: spool_max, file: None, size: 0, spooled: 0, dropped: 0 };
            let mut failing = false;
            for line in rx {
                let sent = sock.as_ref().is_some_and(|s| send_dgram(s, &path, &line));
                if sent {
                    if failing {
                        failing = false;
                        let rec = super::recovery_line(sp.spooled, sp.dropped);
                        if let Some(s) = sock.as_ref() {
                            let _ = send_dgram(s, &path, &rec);
                        }
                        sp.spooled = 0;
                        sp.dropped = 0;
                    }
                } else {
                    failing = true;
                    sp.write(&line);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_logger_goes_to_spool_immediately() {
        let dir = std::env::temp_dir().join(format!("oxspool-{}", std::process::id()));
        let spool = dir.join("n.hub.spool.log");
        let tx = start(Sink::Socket { path: dir.join("absent.sock"), spool: spool.clone(), spool_max: 1 << 20 }, 16);
        tx.send("ts=T kind=diag msg=x".into());
        drop(tx);
        for _ in 0..50 {
            if std::fs::read_to_string(&spool).map(|s| s.contains("msg=x")).unwrap_or(false) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(std::fs::read_to_string(&spool).expect("spool").contains("msg=x"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn full_queue_counts_and_does_not_wait() {
        let (next, _keep) = std::sync::mpsc::sync_channel::<String>(0);
        let tx = start(Sink::Channel(next), 1);
        let before = QUEUE_FULL.load(Ordering::Relaxed);
        for i in 0..50 {
            tx.send(format!("l{i}"));
        }
        assert!(QUEUE_FULL.load(Ordering::Relaxed) > before);
    }
}
