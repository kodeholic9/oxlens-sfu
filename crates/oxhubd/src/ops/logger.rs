// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-OPS-LOGGER · 운영:RULE-LOG-LOGGER · 운영:RULE-LOG-SENDER · model: claude-opus-5-5
//! 로거 유닛(hub 안) — 남은 소켓 파일을 지우고 bind · 수신 버퍼 · 파일 하나(날짜 ∨ 크기 회전) · 보존 · hub 줄은 채널로 바로 · bind 와 파일 연 뒤 준비.

use std::io::Write;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use common::log::Sender;
use common::policy::Logging;

pub struct LoggerCfg {
    pub socket: PathBuf,
    pub dir: PathBuf,
    pub policy: Logging,
}

pub fn start(cfg: LoggerCfg, sender: &Arc<Sender>) -> Result<(), String> {
    std::fs::create_dir_all(&cfg.dir).map_err(|e| format!("log_dir {}: {e}", cfg.dir.display()))?;
    if cfg.socket.exists() {
        let _ = std::fs::remove_file(&cfg.socket);
    }
    if let Some(p) = cfg.socket.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let sock = UnixDatagram::bind(&cfg.socket).map_err(|e| format!("log socket {}: {e}", cfg.socket.display()))?;
    let s2 = socket2::SockRef::from(&sock);
    let _ = s2.set_recv_buffer_size(cfg.policy.socket_rcvbuf_bytes as usize);
    let mut w = FileWriter::open(&cfg.dir, cfg.policy.rotate_max_bytes, cfg.policy.retention_days)?;
    let (tx, rx): (SyncSender<String>, Receiver<String>) = sync_channel(cfg.policy.queue_max_lines.max(256) as usize);
    let tx2 = tx.clone();
    std::thread::Builder::new()
        .name("log-recv".into())
        .spawn(move || {
            let mut buf = vec![0u8; 4096];
            while let Ok(n) = sock.recv(&mut buf) {
                let line = String::from_utf8_lossy(&buf[..n]).trim_end_matches('\n').to_string();
                if tx2.try_send(line).is_err() {
                    continue;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("log-write".into())
        .spawn(move || {
            while let Ok(line) = rx.recv() {
                w.write(&line);
            }
        })
        .map_err(|e| e.to_string())?;
    sender.attach_inproc(tx);
    Ok(())
}

struct FileWriter {
    dir: PathBuf,
    date: String,
    n: u32,
    file: std::io::BufWriter<std::fs::File>,
    bytes: u64,
    max: u64,
    retention_days: u16,
    lines_since_flush: u32,
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn name(date: &str, n: u32) -> String {
    if n == 0 { format!("ox.log.{date}") } else { format!("ox.log.{date}.{n}") }
}

impl FileWriter {
    fn open(dir: &Path, max: u64, retention_days: u16) -> Result<FileWriter, String> {
        let date = today();
        let mut n = 0;
        while dir.join(name(&date, n + 1)).exists() {
            n += 1;
        }
        let path = dir.join(name(&date, n));
        let f = std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let bytes = f.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(FileWriter {
            dir: dir.to_path_buf(),
            date,
            n,
            file: std::io::BufWriter::new(f),
            bytes,
            max,
            retention_days,
            lines_since_flush: 0,
        })
    }

    fn rotate(&mut self) {
        let _ = self.file.flush();
        let d = today();
        if d != self.date {
            self.date = d;
            self.n = 0;
        } else {
            self.n += 1;
        }
        let path = self.dir.join(name(&self.date, self.n));
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            self.bytes = f.metadata().map(|m| m.len()).unwrap_or(0);
            self.file = std::io::BufWriter::new(f);
        }
        self.retain();
    }

    fn retain(&self) {
        if self.retention_days == 0 {
            return;
        }
        let horizon = std::time::Duration::from_secs(self.retention_days as u64 * 86_400);
        let current = name(&self.date, self.n);
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let fname = e.file_name().to_string_lossy().to_string();
                let ours = fname.starts_with("ox.log.") || fname.ends_with(".spool.log");
                if !ours || fname == current {
                    continue;
                }
                let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map(|a| a > horizon);
                if old == Some(true) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }

    fn write(&mut self, line: &str) {
        if today() != self.date || self.bytes + line.len() as u64 + 1 > self.max {
            self.rotate();
        }
        if writeln!(self.file, "{line}").is_ok() {
            self.bytes += line.len() as u64 + 1;
        }
        self.lines_since_flush += 1;
        if self.lines_since_flush >= 1 {
            let _ = self.file.flush();
            self.lines_since_flush = 0;
        }
    }
}
