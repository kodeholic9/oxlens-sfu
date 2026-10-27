// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-OPS-LOGGER · 정:RULE-OPS-SURFACE · 운영:RULE-LOG-LOGGER · 운영:RULE-LOG-SENDER · 운영:RULE-LOG-LINE · model: claude-opus-5-5
//! 로거 유닛(hub 안 · `kind = inproc` · `role = log`) — 입력 둘(sfud · 봇의 DGRAM 소켓 · hub 자신의 줄은 채널)을 도착순으로 파일 하나에 쓴다. 기동 때 남은 소켓 파일을 지우고 bind · 준비 = bind 와 파일 열기 끝. 회전 = 날짜 경계 ∨ 크기 상한(쓰는 순간 판정) · 보존 = 닫힌 파일만 센다 · fsync 없음 · spool 재흡수 없음.

use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::{Duration, SystemTime};

const DATAGRAM_MAX: usize = 65_536;
const DAY_SECS: u64 = 86_400;
const RECV_POLL_MS: u64 = 200;

#[derive(Debug, Clone)]
pub struct Config {
    pub dir: PathBuf,
    pub socket: PathBuf,
    pub rotate_max_bytes: u64,
    pub retention_days: u16,
    pub rcvbuf: u32,
    pub queue_max: usize,
}

pub fn needs_rotate(today: &str, file_date: &str, size: u64, add: u64, max: u64) -> bool {
    today != file_date || size + add > max
}

pub fn expired(age_secs: u64, retention_days: u16) -> bool {
    retention_days != 0 && age_secs >= u64::from(retention_days) * DAY_SECS
}

pub fn file_name(date: &str) -> String {
    format!("ox.log.{date}")
}

pub fn bind_socket(path: &Path, rcvbuf: u32) -> Result<UnixDatagram, String> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::symlink_metadata(path).is_ok() {
        std::fs::remove_file(path).map_err(|e| format!("log-socket {} 남은 파일을 못 지운다: {e}", path.display()))?;
    }
    let s = UnixDatagram::bind(path).map_err(|e| format!("log-socket {}: {e}", path.display()))?;
    let _ = nix::sys::socket::setsockopt(s.as_raw_fd(), nix::sys::socket::sockopt::RcvBuf, &(rcvbuf as usize));
    Ok(s)
}

struct LogFile {
    dir: PathBuf,
    date: String,
    file: Option<std::fs::File>,
    size: u64,
    max: u64,
    retention_days: u16,
}

impl LogFile {
    fn open(&mut self, date: &str) -> bool {
        let path = self.dir.join(file_name(date));
        self.file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
        self.size = self.file.as_ref().and_then(|f| f.metadata().ok()).map(|m| m.len()).unwrap_or(0);
        self.date = date.to_string();
        self.file.is_some()
    }

    fn rotate(&mut self, today: &str) {
        if let Some(mut f) = self.file.take() {
            let _ = f.flush();
        }
        if today == self.date {
            let cur = self.dir.join(file_name(&self.date));
            let mut n = 1;
            while self.dir.join(format!("{}.{n}", file_name(&self.date))).exists() {
                n += 1;
            }
            let _ = std::fs::rename(&cur, self.dir.join(format!("{}.{n}", file_name(&self.date))));
        }
        self.open(today);
        sweep(&self.dir, &file_name(today), self.retention_days);
    }

    fn write(&mut self, line: &str) {
        let today = common::time::today();
        let add = line.len() as u64 + 1;
        if self.file.is_none() {
            self.open(&today);
        } else if needs_rotate(&today, &self.date, self.size, add, self.max) {
            self.rotate(&today);
        }
        if let Some(f) = self.file.as_mut()
            && f.write_all(line.as_bytes()).and_then(|_| f.write_all(b"\n")).is_ok()
        {
            self.size += add;
        }
    }

    fn flush(&mut self) {
        if let Some(f) = self.file.as_mut() {
            let _ = f.flush();
        }
    }
}

fn sweep(dir: &Path, current: &str, retention_days: u16) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let closed = (name.starts_with("ox.log.") && name != current) || name.ends_with(".spool.log");
        if !closed {
            continue;
        }
        let age = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| now.duration_since(t).ok()).map(|d| d.as_secs()).unwrap_or(0);
        if expired(age, retention_days) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

pub struct Running {
    pub tx: SyncSender<String>,
    stop: Arc<AtomicBool>,
    pub done: tokio::sync::oneshot::Receiver<()>,
}

impl Running {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.tx.send(String::new());
    }
}

pub fn start(cfg: Config, unit: &str) -> Result<Running, String> {
    std::fs::create_dir_all(&cfg.dir).map_err(|e| format!("log-dir {}: {e}", cfg.dir.display()))?;
    let sock = bind_socket(&cfg.socket, cfg.rcvbuf)?;
    let _ = sock.set_read_timeout(Some(Duration::from_millis(RECV_POLL_MS)));
    let mut file = LogFile { dir: cfg.dir.clone(), date: String::new(), file: None, size: 0, max: cfg.rotate_max_bytes, retention_days: cfg.retention_days };
    let today = common::time::today();
    if !file.open(&today) {
        let _ = std::fs::remove_file(&cfg.socket);
        return Err(format!("log-dir {}: {} 를 못 연다", cfg.dir.display(), file_name(&today)));
    }
    sweep(&cfg.dir, &file_name(&today), cfg.retention_days);
    let (tx, rx) = std::sync::mpsc::sync_channel::<String>(cfg.queue_max.max(1));
    let stop = Arc::new(AtomicBool::new(false));
    let (done_tx, done) = tokio::sync::oneshot::channel();
    let rx_tx = tx.clone();
    let rx_stop = stop.clone();
    let rx_unit = unit.to_string();
    std::thread::Builder::new().name("ox-log-rx".into()).spawn(move || recv_loop(sock, rx_tx, rx_stop, rx_unit)).map_err(|e| format!("로거 수신 스레드: {e}"))?;
    let socket = cfg.socket.clone();
    std::thread::Builder::new()
        .name("ox-log-write".into())
        .spawn(move || {
            write_loop(&mut file, rx);
            let _ = std::fs::remove_file(&socket);
            let _ = done_tx.send(());
        })
        .map_err(|e| format!("로거 쓰기 스레드: {e}"))?;
    Ok(Running { tx, stop, done })
}

fn recv_loop(sock: UnixDatagram, tx: SyncSender<String>, stop: Arc<AtomicBool>, unit: String) {
    let mut buf = vec![0u8; DATAGRAM_MAX];
    while !stop.load(Ordering::Relaxed) {
        match sock.recv(&mut buf) {
            Ok(0) => continue,
            Ok(n) => {
                let line = String::from_utf8_lossy(&buf[..n]).trim_end_matches('\n').to_string();
                if !line.is_empty() && tx.send(line).is_err() {
                    return;
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted) => {}
            Err(e) => {
                tracing::warn!(unit = %unit, error = %e, "로거 소켓 수신 실패");
                std::thread::sleep(Duration::from_millis(RECV_POLL_MS));
            }
        }
    }
}

fn write_loop(file: &mut LogFile, rx: Receiver<String>) {
    for line in rx.iter() {
        if line.is_empty() {
            break;
        }
        file.write(&line);
    }
    while let Ok(line) = rx.try_recv() {
        if !line.is_empty() {
            file.write(&line);
        }
    }
    file.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_is_date_or_size() {
        assert!(!needs_rotate("2026-10-27", "2026-10-27", 10, 5, 100));
        assert!(needs_rotate("2026-10-28", "2026-10-27", 10, 5, 100));
        assert!(needs_rotate("2026-10-27", "2026-10-27", 96, 5, 100));
    }

    #[test]
    fn retention_zero_keeps_forever() {
        assert!(!expired(10 * DAY_SECS, 0));
        assert!(expired(14 * DAY_SECS, 14));
        assert!(!expired(14 * DAY_SECS - 1, 14));
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oxlg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn leftover_socket_file_is_removed_then_bound() {
        let dir = tmp("stale");
        std::fs::create_dir_all(&dir).expect("dir");
        let p = dir.join("log.sock");
        drop(UnixDatagram::bind(&p).expect("first"));
        assert!(p.exists(), "시험 형상 — 급사가 남긴 소켓 파일");
        assert!(UnixDatagram::bind(&p).is_err(), "대조 — 지우지 않으면 EADDRINUSE");
        assert!(bind_socket(&p, 65_536).is_ok(), "O-sym-RULE-LOG-LOGGER-2 S-sym-RULE-OPS-LOGGER-3 남은 소켓 파일을 지우고 bind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ready_means_bound_and_file_open_then_datagrams_land_in_one_file() {
        let dir = tmp("ready");
        let sock = dir.join("log.sock");
        let r = start(Config { dir: dir.clone(), socket: sock.clone(), rotate_max_bytes: 1 << 20, retention_days: 0, rcvbuf: 65_536, queue_max: 64 }, "log-n").expect("start");
        assert!(sock.exists(), "S-sym-RULE-OPS-LOGGER-2 준비(start 반환) = 소켓 bind 끝");
        let file = dir.join(file_name(&common::time::today()));
        assert!(file.exists(), "S-sym-RULE-OPS-LOGGER-2 준비 = 출력 파일 열기 끝");
        UnixDatagram::unbound().expect("c").send_to(b"ts=T node=n proc=sfu kind=diag msg=from-sfu", &sock).expect("send");
        r.tx.send("ts=T node=n proc=hub kind=diag msg=from-hub".into()).expect("hub");
        let mut text = String::new();
        for _ in 0..100 {
            text = std::fs::read_to_string(&file).unwrap_or_default();
            if text.contains("from-sfu") && text.contains("from-hub") {
                break;
            }
            r.tx.send(String::from("ts=T node=n proc=hub kind=diag msg=tick")).ok();
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut r = r;
        r.stop();
        let _ = tokio::time::timeout(Duration::from_secs(3), &mut r.done).await;
        let text = std::fs::read_to_string(&file).unwrap_or(text);
        assert!(text.contains("msg=from-sfu") && text.contains("msg=from-hub"), "O-sym-RULE-LOG-LOGGER-3 소켓 줄 · hub 채널 줄이 한 파일에 — {text}");
        assert!(!sock.exists(), "운영:RULE-LOG-LOGGER 끝내면 소켓 파일을 거둔다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_and_rotates_by_size() {
        let dir = std::env::temp_dir().join(format!("oxlogger-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let mut f = LogFile { dir: dir.clone(), date: String::new(), file: None, size: 0, max: 40, retention_days: 0 };
        f.write("a".repeat(30).as_str());
        f.write("b".repeat(30).as_str());
        f.flush();
        let today = common::time::today();
        let cur = std::fs::read_to_string(dir.join(file_name(&today))).expect("cur");
        let old = std::fs::read_to_string(dir.join(format!("{}.1", file_name(&today)))).expect("old");
        assert!(cur.starts_with('b') && old.starts_with('a'));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
