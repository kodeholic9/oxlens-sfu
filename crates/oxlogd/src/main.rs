// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-OPS-LOGGER · 운영:RULE-LOG-LOGGER · 운영:RULE-BOOT-ARGS · 정:RULE-CFG-THREE-PLACES · model: claude-opus-5-5
//! 로거 유닛 `oxlogd` — DGRAM 소켓을 bind 한 뒤 stdout 에 `ready epoch=` 한 줄 · 파일 하나 · 회전 · 보존 · stdin 닫힘이면 남은 것을 쓰고 끝낸다.

mod ops;

use common::args::{Args, Spec, flag, opt};
use common::banner::Banner;
use common::policy::{Policy, Reader};

const ARGS: &[Spec] = &[
    opt("policy"),
    opt("log-dir"),
    opt("log-socket"),
    opt("id"),
    opt("build"),
    opt("zenoh-namespace"),
    opt("zenoh-connect"),
    flag("version"),
];

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match Args::parse(&argv, ARGS) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("oxlogd: {e}");
            std::process::exit(2);
        }
    };
    if args.has("version") {
        println!("{}", common::build::identity(args.get("build")));
        return;
    }
    if let Err(e) = run(&args) {
        eprintln!("oxlogd: {e}");
        std::process::exit(1);
    }
}

fn run(args: &Args) -> Result<(), String> {
    let policy = match args.get("policy") {
        Some(p) => Policy::load(p).map_err(|e| e.to_string())?,
        None => Policy::default(),
    };
    let log_dir = args.require("log-dir")?.to_string();
    let node = args.get("id").unwrap_or("-").to_string();
    let epoch = common::build::new_epoch();
    let build = common::build::identity(args.get("build"));
    let (own_tx, own_rx) = std::sync::mpsc::sync_channel::<String>(policy.logging.queue_max_lines as usize);
    common::log::init(common::log::Init {
        node: node.clone(),
        proc_: "log",
        epoch: None,
        logging: policy.logging.clone(),
        sink: common::log::sender::Sink::Channel(own_tx.clone()),
    });
    let mut b = Banner::new();
    for name in ["policy", "log-dir", "log-socket", "id", "build", "zenoh-namespace", "zenoh-connect"] {
        b.put_opt(name, args.get(name));
    }
    policy.banner(&mut b, Reader::Log);
    let cfg = ops::Config {
        dir: log_dir.into(),
        socket: args.get("log-socket").map(Into::into),
        rotate_max_bytes: policy.logging.rotate_max_bytes,
        retention_days: policy.logging.retention_days,
        rcvbuf: policy.logging.socket_rcvbuf_bytes,
    };
    let logger = ops::Logger::bind(cfg, own_tx)?;
    ops::announce_ready(&epoch);
    common::log::boot_line(&build, &epoch, &b.render());
    logger.serve(own_rx)
}
