// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-DOC-PLANES · 정:RULE-DOC-OWNERSHIP · 정:RULE-DOC-CODE-MIRROR · 정:RULE-CFG-THREE-PLACES · 운영 규격서 §8-2 · model: claude-opus-5-5
//! oxhubd — 게이트웨이(hub). 세션 · 배치 · op 라우팅 · C 평면 · supervisor · zenoh router. 방 상태는 미디어 서버가 든다.

mod aux;
mod bus;
mod cfg;
mod err;
mod ops;
mod room;
mod ses;

use std::io::Write;

fn main() {
    common::time::start();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let boot = match cfg::Boot::from_args(&argv) {
        Ok(b) => b,
        Err(e) => {
            if let Some(v) = e.strip_prefix("version:") {
                println!("{v}");
                return;
            }
            let _ = writeln!(std::io::stderr(), "oxhubd: {e}");
            std::process::exit(2);
        }
    };
    let _log = common::log::init(common::log::LogInit {
        node: boot.node_id.clone(),
        proc_: "hub",
        log_dir: boot.log_dir.clone(),
        log_socket: boot.log_socket.clone(),
        policy: boot.policy.logging.clone(),
    });
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "oxhubd: runtime: {e}");
            std::process::exit(2);
        }
    };
    let code = rt.block_on(ops::run(boot));
    rt.shutdown_timeout(std::time::Duration::from_secs(2));
    std::process::exit(code);
}
