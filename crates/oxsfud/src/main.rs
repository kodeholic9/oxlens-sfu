// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-DOC-PLANES · 정:RULE-CFG-THREE-PLACES · 정:RULE-BUS-FATE · 운영 규격서 §8-2 · model: claude-opus-5-5
//! oxsfud 진입 — 기동 인자 · 로깅 · 런타임 · 정지 코드.

use std::io::Write;

fn main() {
    common::time::start();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let boot = match oxsfud::cfg::Boot::from_args(&argv) {
        Ok(b) => b,
        Err(e) => {
            if let Some(v) = e.strip_prefix("version:") {
                println!("{v}");
                return;
            }
            let _ = writeln!(std::io::stderr(), "oxsfud: {e}");
            std::process::exit(2);
        }
    };
    let _log = common::log::init(common::log::LogInit {
        node: boot.node_id.clone(),
        proc_: "sfu",
        log_dir: boot.log_dir.clone(),
        log_socket: boot.log_socket.clone(),
        policy: boot.policy.logging.clone(),
    });
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "oxsfud: runtime: {e}");
            std::process::exit(2);
        }
    };
    let code = rt.block_on(oxsfud::ops::run(boot));
    rt.shutdown_timeout(std::time::Duration::from_secs(2));
    std::process::exit(code);
}
