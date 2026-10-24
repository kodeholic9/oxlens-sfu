// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-LOGGER · 정:RULE-BUS-FATE · 정:RULE-CFG-THREE-PLACES · 정:RULE-DOC-PLANES · 운영 규격서 §7-1 · 운영 규격서 §8-2 · model: claude-opus-5-5
//! oxlogd — 로거 유닛. 소켓을 bind 한 뒤 stdout 에 `ready epoch=` 한 줄을 내고, 그 뒤로 stdout 을 쓰지 않는다. stdin 이 닫히면 남은 줄을 쓰고 끝낸다.

mod ops;

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use common::args::Args;
use common::policy::Policy;

use ops::shell::{self, Own, Stats, Writer};

const VALUED: &[&str] = &["policy", "log-dir", "log-socket", "id", "build", "zenoh-namespace", "zenoh-connect"];
const SWITCHES: &[&str] = &["version"];

fn fail(msg: &str) -> ! {
    let _ = writeln!(std::io::stderr(), "oxlogd: {msg}");
    std::process::exit(2);
}

fn main() {
    common::time::start();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = Args::parse(&argv, VALUED, SWITCHES).unwrap_or_else(|e| fail(&e));
    let build = common::identity::build(args.get("build"));
    if args.has("version") {
        println!("{build}");
        return;
    }
    let policy = match args.get("policy") {
        Some(p) => Policy::load(&PathBuf::from(p)).unwrap_or_else(|e| fail(&e.to_string())),
        None => Policy::default(),
    };
    let dir = PathBuf::from(args.get("log-dir").unwrap_or_else(|| fail("--log-dir 가 없다")));
    let sock_path = PathBuf::from(args.get("log-socket").unwrap_or_else(|| fail("--log-socket 이 없다")));
    let node = args.get("id").unwrap_or("-").to_string();
    let inst = format!("log-{}", common::identity::fresh_hex(6));
    let own = Own { node, line_max: policy.logging.line_max_bytes as usize };
    let mut writer = Writer::open(&dir, &policy.logging).unwrap_or_else(|e| fail(&format!("{}: {e}", dir.display())));
    let sock = shell::bind(&sock_path, policy.logging.socket_rcvbuf_bytes).unwrap_or_else(|e| fail(&format!("{}: {e}", sock_path.display())));
    writer.write(&own.line(&[
        ("kind", "ev".into()),
        ("spec", "RULE-OPS-LOGGER".into()),
        ("epoch", inst.clone()),
        ("build", build.clone()),
        ("socket", sock_path.display().to_string()),
        ("log_dir", dir.display().to_string()),
        ("rcvbuf", policy.logging.socket_rcvbuf_bytes.to_string()),
        ("message", "logger ready".into()),
    ]));
    writer.flush();
    {
        let mut out = std::io::stdout();
        let _ = writeln!(out, "ready epoch={inst}");
        let _ = out.flush();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    common::lifeline::watch_stdin(move || s2.store(true, Ordering::Relaxed));
    let stats = Arc::new(Stats { lines: AtomicU64::new(0), bytes: AtomicU64::new(0) });
    let mut writer = shell::serve(sock, writer, stop, stats.clone());
    writer.write(&own.line(&[
        ("kind", "ev".into()),
        ("spec", "RULE-BUS-FATE".into()),
        ("epoch", inst),
        ("lines", stats.lines.load(Ordering::Relaxed).to_string()),
        ("bytes", stats.bytes.load(Ordering::Relaxed).to_string()),
        ("message", "parent closed — logger stops".into()),
    ]));
    writer.flush();
    let _ = std::fs::remove_file(&sock_path);
}
