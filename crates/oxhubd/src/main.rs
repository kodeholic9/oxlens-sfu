// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-DOC-PLANES · 정:RULE-DOC-OWNERSHIP · 정:RULE-BUS-FATE · 정:RULE-OPS-SURFACE · 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! 게이트웨이 `oxhubd` — 세션 · 배치 · op 라우팅 · C 평면 · supervisor · zenoh router.

use common::args::Args;
use oxhubd::{cfg, ops};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match Args::parse(&argv, cfg::ARGS) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("oxhubd: {e}");
            std::process::exit(2);
        }
    };
    if args.has("version") {
        println!("{}", common::build::identity(args.get("build")));
        return;
    }
    let cfg = match cfg::HubCfg::load(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("oxhubd: {e}");
            std::process::exit(1);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("oxhubd: runtime: {e}");
            std::process::exit(1);
        }
    };
    let code = rt.block_on(async move {
        let sink = common::log::sink_for("hub", &cfg.node_id, cfg.log_dir.as_deref(), cfg.log_socket.as_deref(), &cfg.policy.logging);
        common::log::init(common::log::Init { node: cfg.node_id.clone(), proc_: "hub", epoch: None, logging: cfg.policy.logging.clone(), sink });
        let hub = ops::Hub::new(cfg);
        common::log::boot_line(&hub.cfg.build, &hub.epoch, &hub.cfg.banner().render());
        match ops::run(hub).await {
            Ok(()) => 0,
            Err(e) => {
                tracing::error!(error = %e, "hub 종료");
                eprintln!("oxhubd: {e}");
                1
            }
        }
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    std::process::exit(code);
}
