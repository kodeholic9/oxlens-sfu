// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-FATE · 정:RULE-BUS-SHAPE · 정:RULE-OPS-LOGGER · 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! 미디어 서버 `oxsfud` — 기동 인자 · 로그 · 기동 신원(epoch = sfu_id = room 토큰 inst) · 운영 루프.

use common::args::Args;
use oxsfud::cfg::{ARGS, SfuCfg};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match Args::parse(&argv, ARGS) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("oxsfud: {e}");
            std::process::exit(2);
        }
    };
    if args.has("version") {
        println!("{}", common::build::identity(args.get("build")));
        return;
    }
    let cfg = match SfuCfg::load(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("oxsfud: {e}");
            std::process::exit(1);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("oxsfud: runtime: {e}");
            std::process::exit(1);
        }
    };
    let code = rt.block_on(async move {
        let epoch = common::build::new_epoch();
        let sink = common::log::sink_for("sfu", &cfg.node_id, cfg.log_dir.as_deref(), cfg.log_socket.as_deref(), &cfg.policy.logging);
        common::log::init(common::log::Init { node: cfg.node_id.clone(), proc_: "sfu", epoch: None, logging: cfg.policy.logging.clone(), sink });
        match oxsfud::ops::run(cfg, epoch).await {
            Ok(()) => 0,
            Err(e) => {
                tracing::error!(error = %e, "sfud 종료");
                eprintln!("oxsfud: {e}");
                1
            }
        }
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    std::process::exit(code);
}
