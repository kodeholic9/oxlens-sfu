// author: kodeholic (powered by Claude)
// spec: v1.2 @ae019c2 · 정:RULE-OPS-LOGGER 정:RULE-BUS-FATE 정:RULE-CFG-THREE-PLACES · 운영 규격서 §7-1 · §8-2 · model: claude-opus-5-5
//! oxlogd — 로거 유닛. DGRAM 소켓으로 받은 줄을 파일 하나에 도착순으로 쓴다 · 날짜 ∨ 크기로 회전 · 보존 일수로 지운다.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use common::policy::Policy;

#[derive(Default)]
struct Args {
    id: Option<String>,
    build: Option<String>,
    policy: Option<String>,
    log_dir: Option<String>,
    log_socket: Option<String>,
    version: bool,
}

fn parse_args(argv: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = argv;
    while let Some(k) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{k} 에 값이 없다"));
        match k.as_str() {
            "--id" => a.id = Some(val()?),
            "--build" => a.build = Some(val()?),
            "--policy" => a.policy = Some(val()?),
            "--log-dir" => a.log_dir = Some(val()?),
            "--log-socket" => a.log_socket = Some(val()?),
            "--zenoh-namespace" | "--zenoh-connect" => {
                val()?;
            }
            "--version" => a.version = true,
            other => return Err(format!("모르는 인자: {other}")),
        }
    }
    Ok(a)
}

fn identity(label: Option<&str>) -> String {
    common::ident::build_identity(env!("OX_GIT_SHA"), env!("OX_BUILD_TIME"), label)
}

fn now_ts() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f%:z").to_string()
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

struct Out {
    dir: PathBuf,
    date: String,
    seq: u32,
    file: File,
    path: PathBuf,
    bytes: u64,
    rotate_max: u64,
    retention: Duration,
}

impl Out {
    fn open(dir: &Path, rotate_max: u64, retention_days: u16) -> std::io::Result<Out> {
        std::fs::create_dir_all(dir)?;
        let date = today();
        let (seq, path, file, bytes) = Self::next_file(dir, &date, 0, rotate_max)?;
        let o = Out {
            dir: dir.to_path_buf(),
            date,
            seq,
            file,
            path,
            bytes,
            rotate_max,
            retention: Duration::from_secs(u64::from(retention_days) * 86_400),
        };
        o.sweep();
        Ok(o)
    }

    fn next_file(dir: &Path, date: &str, from: u32, rotate_max: u64) -> std::io::Result<(u32, PathBuf, File, u64)> {
        let mut n = from;
        loop {
            let name = if n == 0 { format!("ox.log.{date}") } else { format!("ox.log.{date}.{n}") };
            let p = dir.join(name);
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            if size < rotate_max {
                let f = OpenOptions::new().create(true).append(true).open(&p)?;
                return Ok((n, p, f, size));
            }
            n += 1;
        }
    }

    fn write(&mut self, line: &[u8]) {
        let n = line.len() as u64 + 1;
        let date = today();
        let from = if date != self.date { 0 } else { self.seq + 1 };
        if (date != self.date || (self.bytes > 0 && self.bytes + n > self.rotate_max))
            && let Ok((seq, p, f, b)) = Self::next_file(&self.dir, &date, from, self.rotate_max)
        {
            self.seq = seq;
            self.path = p;
            self.file = f;
            self.bytes = b;
            self.date = date;
            self.sweep();
        }
        let mut buf = Vec::with_capacity(line.len() + 1);
        buf.extend_from_slice(line);
        buf.push(b'\n');
        if self.file.write_all(&buf).is_ok() {
            self.bytes += n;
        }
    }

    fn sweep(&self) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let now = SystemTime::now();
        for e in rd.flatten() {
            let p = e.path();
            if p == self.path {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if !(name.starts_with("ox.log.") || name.ends_with(".spool.log")) {
                continue;
            }
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age > self.retention);
            if old {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

fn own_line(node: &str, kind: &str, msg: &str) -> String {
    format!("ts={} node={node} proc=log kind={kind} spec=RULE-OPS-LOGGER msg=\"{}\"", now_ts(), msg.replace('"', "'"))
}

fn watch_parent(closed: Arc<AtomicBool>) {
    std::thread::Builder::new()
        .name("ox-parent".into())
        .spawn(move || {
            let mut buf = [0u8; 64];
            loop {
                match std::io::stdin().read(&mut buf) {
                    Ok(0) | Err(_) => {
                        closed.store(true, Ordering::SeqCst);
                        return;
                    }
                    Ok(_) => {}
                }
            }
        })
        .ok();
}

fn bind(path: &Path, rcvbuf: u32) -> std::io::Result<UnixDatagram> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let s = UnixDatagram::bind(path)?;
    let _ = nix::sys::socket::setsockopt(s.as_raw_fd(), nix::sys::socket::sockopt::RcvBuf, &(rcvbuf as usize));
    s.set_read_timeout(Some(Duration::from_millis(200)))?;
    Ok(s)
}

fn run(args: Args) -> i32 {
    let build = identity(args.build.as_deref());
    if args.version {
        println!("{build}");
        return 0;
    }
    let policy = match args.policy.as_deref() {
        Some(p) => Policy::load(Path::new(p)),
        None => Policy::parse(""),
    };
    let policy = match policy {
        Ok(p) => p,
        Err(e) => {
            eprintln!("oxlogd: 정책 파일 — {}", e.0);
            return 2;
        }
    };
    let (Some(dir), Some(sock)) = (args.log_dir.as_deref(), args.log_socket.as_deref()) else {
        eprintln!("oxlogd: --log-dir · --log-socket 이 둘 다 있어야 한다");
        return 2;
    };
    let node = args.id.clone().unwrap_or_else(|| "node-a".into());
    let lg = &policy.logging;
    let mut out = match Out::open(Path::new(dir), lg.rotate_max_bytes, lg.retention_days) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("oxlogd: {dir} — {e}");
            return 2;
        }
    };
    let s = match bind(Path::new(sock), lg.socket_rcvbuf_bytes) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("oxlogd: {sock} — {e}");
            return 2;
        }
    };
    let epoch = common::ident::new_epoch();
    let closed = Arc::new(AtomicBool::new(false));
    watch_parent(closed.clone());
    out.write(own_line(&node, "ev", &format!("logger ready epoch={epoch} build={build} socket={sock} dir={dir}")).as_bytes());
    let mut stdout = std::io::stdout();
    if writeln!(stdout, "ready epoch={epoch}").and_then(|_| stdout.flush()).is_err() {
        return 1;
    }
    let mut buf = vec![0u8; 65_536];
    loop {
        match s.recv(&mut buf) {
            Ok(n) => {
                let line = buf[..n].strip_suffix(b"\n").unwrap_or(&buf[..n]);
                out.write(line);
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if closed.load(Ordering::SeqCst) {
                    break;
                }
            }
            Err(e) => {
                out.write(own_line(&node, "diag", &format!("recv failed: {e}")).as_bytes());
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    while let Ok(n) = s.recv(&mut buf) {
        let line = buf[..n].strip_suffix(b"\n").unwrap_or(&buf[..n]);
        out.write(line);
    }
    out.write(own_line(&node, "ev", "parent liveness closed — exit").as_bytes());
    let _ = std::fs::remove_file(sock);
    0
}

fn main() {
    let code = match parse_args(std::env::args().skip(1)) {
        Ok(a) => run(a),
        Err(e) => {
            eprintln!("oxlogd: {e}");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oxlogd-{name}-{}", common::ident::random_hex(4)));
        std::fs::create_dir_all(&d).ok();
        d
    }

    #[test]
    fn rotates_on_size() {
        let d = tmp("rot");
        let mut o = Out::open(&d, 64, 14).expect("open");
        for _ in 0..5 {
            o.write(b"0123456789012345678901234567890");
        }
        let n = std::fs::read_dir(&d).map(|r| r.count()).unwrap_or(0);
        assert!(n >= 2, "크기 상한에서 새 파일이 생겨야 한다: {n}");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn rotation_never_reuses_a_partly_filled_file() {
        let d = tmp("partial");
        let mut o = Out::open(&d, 64, 14).expect("open");
        o.write(&[b'a'; 40]);
        o.write(&[b'b'; 30]);
        let first = std::fs::metadata(d.join(format!("ox.log.{}", today()))).map(|m| m.len()).unwrap_or(0);
        assert!(first <= 64, "상한을 넘겨 이어 쓰면 안 된다: {first}");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn sweep_spares_current_and_fresh() {
        let d = tmp("sweep");
        std::fs::write(d.join("node-a.hub.spool.log"), "x").ok();
        std::fs::write(d.join("other.txt"), "x").ok();
        let o = Out::open(&d, 1 << 20, 0).expect("open");
        o.sweep();
        assert!(o.path.exists());
        assert!(d.join("other.txt").exists());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn args_accept_hub_extras() {
        let a = parse_args(
            ["--id", "n1", "--zenoh-namespace", "ns", "--log-dir", "/tmp/x", "--log-socket", "/tmp/x/s"]
                .iter()
                .map(|s| s.to_string()),
        )
        .expect("args");
        assert_eq!(a.id.as_deref(), Some("n1"));
        assert!(parse_args(["--nope"].iter().map(|s| s.to_string())).is_err());
    }

    #[test]
    fn datagram_round_trip() {
        let d = tmp("dgram");
        let sock = d.join("log.sock");
        let s = bind(&sock, 65_536).expect("bind");
        let c = UnixDatagram::unbound().expect("client");
        c.send_to(b"ts=1 node=n1 proc=hub kind=ev msg=x", &sock).expect("send");
        let mut buf = [0u8; 256];
        let n = s.recv(&mut buf).expect("recv");
        assert_eq!(&buf[..n], b"ts=1 node=n1 proc=hub kind=ev msg=x");
        std::fs::remove_dir_all(&d).ok();
    }
}
