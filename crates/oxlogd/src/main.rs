// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-OPS-LOGGER · 정:RULE-CFG-THREE-PLACES · 정:RULE-OPS-SURFACE · model: claude-opus-5-5
//! `oxlogd` — hub 의 process 유닛 하나 · 한 node 의 로그를 한 파일로 모은다(운영 규격서 §7-1).

mod ops;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use common::args::Args;
use common::policy::Policy;

const VALUED: [&str; 7] = ["policy", "log-dir", "log-socket", "id", "build", "zenoh-namespace", "zenoh-connect"];
const FLAGS: [&str; 1] = ["version"];

fn main() {
    let args = match Args::parse(std::env::args().skip(1), &VALUED, &FLAGS) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("oxlogd: {e}");
            std::process::exit(2);
        }
    };
    let build = common::ident::build_identity(env!("OX_BUILD_ID"), args.get("build"));
    if args.flag("version") {
        println!("{build}");
        return;
    }
    let policy = match args.get("policy").map(std::fs::read_to_string) {
        Some(Ok(t)) => match Policy::parse(&t) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("oxlogd: {e}");
                std::process::exit(2);
            }
        },
        Some(Err(e)) => {
            eprintln!("oxlogd: policy: {e}");
            std::process::exit(2);
        }
        None => Policy::default(),
    };
    let dir = PathBuf::from(args.get("log-dir").unwrap_or("/tmp/oxlens"));
    let socket = args.get("log-socket").map(PathBuf::from).unwrap_or_else(|| dir.join("log.sock"));
    let node = args.get("id").unwrap_or("node").to_string();
    let epoch = common::ident::new_epoch("log");
    let _ = std::fs::create_dir_all(&dir);
    let writer = Arc::new(Mutex::new(ops::shell::Writer::new(&dir, policy.logging.rotate_max_bytes, policy.logging.retention_days)));
    let w2 = writer.clone();
    common::log::init(common::log::LogConf {
        node: node.clone(),
        proc_name: "log".to_string(),
        socket: None,
        log_dir: Some(dir.clone()),
        level: format!("{:?}", policy.logging.level).to_lowercase(),
        line_max: policy.logging.line_max_bytes as usize,
        queue_max: policy.logging.queue_max_lines as usize,
        spool_max: policy.logging.spool_max_bytes,
        sink: Some(Box::new(move |l: &str| {
            if let Ok(mut w) = w2.lock() {
                w.write_line(l.as_bytes());
            }
        })),
    });
    tracing::info!(kind = "diag", epoch = %epoch, build = %build, dir = %dir.display(), socket = %socket.display(), "logger up");
    writer.lock().map(|w| w.sweep()).ok();
    let conf = ops::shell::Conf {
        dir,
        socket,
        rcvbuf: policy.logging.socket_rcvbuf_bytes,
        epoch,
    };
    if let Err(e) = ops::shell::run(conf, writer) {
        eprintln!("oxlogd: {e}");
        std::process::exit(1);
    }
}
