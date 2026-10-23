// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-OPS-LOGGER · 정:RULE-BUS-FATE · 정:RULE-OPS-OBSERVE · model: claude-opus-5-5
//! 로거 I/O — DGRAM 소켓 bind · 준비 신호 한 줄 · 도착순 쓰기 · 회전 · 보존 · stdin 닫힘이면 남은 것을 쓰고 끝낸다.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::core::{self, Current};

pub struct Conf {
    pub dir: PathBuf,
    pub socket: PathBuf,
    pub rcvbuf: u32,
    pub epoch: String,
}

pub struct Writer {
    dir: PathBuf,
    cur: Current,
    file: Option<File>,
    rotate_max: u64,
    retention_days: u16,
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

impl Writer {
    pub fn new(dir: &Path, rotate_max: u64, retention_days: u16) -> Writer {
        Writer {
            dir: dir.to_path_buf(),
            cur: Current { date: today(), part: 0, size: 0 },
            file: None,
            rotate_max,
            retention_days,
        }
    }

    fn open(&mut self) {
        let path = self.dir.join(core::file_name(&self.cur.date, self.cur.part));
        self.file = OpenOptions::new().create(true).append(true).open(&path).ok();
        self.cur.size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    }

    pub fn write_line(&mut self, line: &[u8]) {
        let incoming = line.len() as u64 + 1;
        if self.file.is_none() {
            self.open();
        }
        if let Some(n) = core::next(&self.cur, &today(), incoming, self.rotate_max) {
            if let Some(f) = self.file.as_mut() {
                let _ = f.flush();
            }
            self.cur = n;
            self.open();
            self.sweep();
        }
        if let Some(f) = self.file.as_mut() {
            let _ = f.write_all(line);
            let _ = f.write_all(b"\n");
            self.cur.size += incoming;
        }
    }

    pub fn flush(&mut self) {
        if let Some(f) = self.file.as_mut() {
            let _ = f.flush();
        }
    }

    pub fn sweep(&self) {
        let current = core::file_name(&self.cur.date, self.cur.part);
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !core::closed_log(&name, &current) {
                continue;
            }
            let age = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map(|d| d.as_secs()).unwrap_or(0);
            if core::expired(age, self.retention_days) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

pub fn run(conf: Conf, writer: Arc<Mutex<Writer>>) -> std::io::Result<()> {
    std::fs::create_dir_all(&conf.dir)?;
    if let Some(p) = conf.socket.parent() {
        std::fs::create_dir_all(p)?;
    }
    let _ = std::fs::remove_file(&conf.socket);
    let sock = UnixDatagram::bind(&conf.socket)?;
    let _ = nix::sys::socket::setsockopt(sock.as_raw_fd(), nix::sys::socket::sockopt::RcvBuf, &(conf.rcvbuf as usize));
    sock.set_read_timeout(Some(Duration::from_millis(500)))?;
    {
        let out = std::io::stdout();
        let mut l = out.lock();
        writeln!(l, "ready epoch={}", conf.epoch)?;
        l.flush()?;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut buf = String::new();
        let mut lock = stdin.lock();
        while matches!(lock.read_line(&mut buf), Ok(n) if n > 0) {
            buf.clear();
        }
        s2.store(true, Ordering::Relaxed);
    });
    let mut buf = vec![0u8; 65_536];
    while !stop.load(Ordering::Relaxed) {
        match sock.recv(&mut buf) {
            Ok(n) => {
                let mut w = match writer.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                let end = buf[..n].iter().rposition(|b| *b != b'\n').map(|i| i + 1).unwrap_or(0);
                w.write_line(&buf[..end]);
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if let Ok(mut w) = writer.lock() {
                    w.flush();
                }
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    while let Ok(n) = sock.recv(&mut buf) {
        if n == 0 {
            break;
        }
        if let Ok(mut w) = writer.lock() {
            w.write_line(&buf[..n]);
        }
    }
    if let Ok(mut w) = writer.lock() {
        w.flush();
    }
    let _ = std::fs::remove_file(&conf.socket);
    Ok(())
}
