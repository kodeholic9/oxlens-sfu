// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-OPS-LOGGER 운영:RULE-LOG-LOGGER 운영:RULE-LOG-LINE 운영:RULE-BOOT-ARGS 정:RULE-CFG-THREE-PLACES 정책:logging.socket_rcvbuf_bytes 정책:logging.rotate_max_bytes 정책:logging.retention_days · model: claude-opus-5-5
//! 로거 — Unix DGRAM 소켓 하나로 받고 · 파일 하나(`{log_dir}/ox.log.{날짜}`)에 도착순으로 쓴다 · 자기 줄은 같은 파일로.

mod writer;

use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::mpsc::sync_channel;

use common::log::{LogOpts, Sink};
use common::policy::Policy;

pub struct Args {
    pub policy: Option<String>,
    pub id: String,
    pub build: Option<String>,
    pub log_dir: String,
    pub log_socket: String,
}

pub enum Parsed {
    Version(Option<String>),
    Run(Args),
}

impl Args {
    pub fn parse(argv: &[String]) -> Result<Parsed, String> {
        let mut policy = None;
        let mut id = None;
        let mut build = None;
        let mut log_dir = None;
        let mut log_socket = None;
        let mut version = false;
        let mut i = 0;
        while i < argv.len() {
            let a = argv[i].as_str();
            let mut val = || -> Result<String, String> {
                i += 1;
                argv.get(i).cloned().ok_or_else(|| format!("{a} 값이 없다"))
            };
            match a {
                "--version" => version = true,
                "--policy" => policy = Some(val()?),
                "--id" => id = Some(val()?),
                "--build" => build = Some(val()?),
                "--log-dir" => log_dir = Some(val()?),
                "--log-socket" => log_socket = Some(val()?),
                "--zenoh-namespace" | "--zenoh-connect" => {
                    val()?;
                }
                other => return Err(format!("모르는 인자 {other}")),
            }
            i += 1;
        }
        if version {
            return Ok(Parsed::Version(build));
        }
        Ok(Parsed::Run(Args {
            policy,
            id: id.unwrap_or_else(|| "local".to_string()),
            build,
            log_dir: log_dir.ok_or("--log-dir 가 없다")?,
            log_socket: log_socket.ok_or("--log-socket 가 없다")?,
        }))
    }
}

fn bind(path: &str) -> Result<UnixDatagram, String> {
    match UnixDatagram::bind(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            let probe = UnixDatagram::unbound().map_err(|e| e.to_string())?;
            if probe.send_to(b"", path).is_ok() {
                return Err(format!("{path} — 다른 로거가 이 소켓을 쓰고 있다"));
            }
            std::fs::remove_file(path).map_err(|e| format!("{path} 낡은 소켓 지우기: {e}"))?;
            UnixDatagram::bind(path).map_err(|e| format!("{path} bind: {e}"))
        }
        Err(e) => Err(format!("{path} bind: {e}")),
    }
}

pub fn run(a: Args) -> Result<(), String> {
    let policy = match &a.policy {
        Some(p) => Policy::parse(&std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?)
            .map_err(|e| e.to_string())?,
        None => Policy::default(),
    };
    let inst = common::id::instance();
    std::fs::create_dir_all(&a.log_dir).map_err(|e| format!("{}: {e}", a.log_dir))?;
    if let Some(parent) = PathBuf::from(&a.log_socket).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let (tx, rx) = sync_channel::<Vec<u8>>(policy.logging.queue_max_lines as usize);
    let log = common::log::init(LogOpts {
        node: a.id.clone(),
        proc_: "log",
        level: common::log::env_level(policy.logging.level),
        line_max: policy.logging.line_max_bytes as usize,
        queue_max: policy.logging.queue_max_lines as usize,
        sink: Sink::Queue(tx.clone()),
    });
    let sock = bind(&a.log_socket)?;
    let rcv = policy.logging.socket_rcvbuf_bytes as usize;
    if let Err(e) = nix::sys::socket::setsockopt(
        std::os::fd::AsRawFd::as_raw_fd(&sock),
        nix::sys::socket::sockopt::RcvBuf,
        &rcv,
    ) {
        tracing::warn!("rcvbuf {rcv}: {e}");
    }
    let out = writer::Out::new(
        PathBuf::from(&a.log_dir),
        policy.logging.rotate_max_bytes,
        policy.logging.retention_days,
    );
    let w = std::thread::Builder::new()
        .name("ox-logw".into())
        .spawn(move || writer::run(rx, out))
        .map_err(|e| e.to_string())?;
    let rtx = tx.clone();
    let rs = sock.try_clone().map_err(|e| e.to_string())?;
    let _ = std::thread::Builder::new()
        .name("ox-logr".into())
        .spawn(move || {
            let mut buf = vec![0u8; 65_536];
            while let Ok(n) = rs.recv(&mut buf) {
                if n > 0 && rtx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
    {
        let mut so = std::io::stdout().lock();
        let _ = writeln!(so, "ready epoch={inst}");
        let _ = so.flush();
    }
    let build = common::build_id(a.build.as_deref());
    common::ev!(
        spec = "SV-STATE-UNIT-STATE",
        build = build.as_str(),
        "logger ready inst={inst} log_dir={} log_socket={} level={} rotate_max_bytes={} retention_days={} rcvbuf={rcv}",
        a.log_dir,
        a.log_socket,
        common::log::level_name(log.level()),
        policy.logging.rotate_max_bytes,
        policy.logging.retention_days
    );
    let mut sink = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut sink);
    common::ev!(
        spec = "SV-STATE-UNIT-STATE",
        "logger stopping — parent channel closed"
    );
    std::thread::sleep(std::time::Duration::from_millis(100));
    let _ = tx.send(Vec::new());
    let _ = w.join();
    let _ = std::fs::remove_file(&a.log_socket);
    Ok(())
}
