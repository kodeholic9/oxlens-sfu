// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-DOC-PLANES 정:RULE-DOC-OWNERSHIP 정:RULE-DOC-CODE-MIRROR 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! 게이트웨이 `oxhubd` — 세션 · 배치 · 라우팅 · 운영 표면 · supervisor · zenoh router.

mod aux;
mod bus;
mod cfg;
mod ops;
mod room;
mod ses;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let a = match cfg::parse_args(&argv) {
        Ok(cfg::Parsed::Version(label)) => {
            println!("{}", common::build_id(label.as_deref()));
            return;
        }
        Ok(cfg::Parsed::Run(a)) => a,
        Err(e) => {
            eprintln!("oxhubd: {e}");
            std::process::exit(2);
        }
    };
    let c = match cfg::load(&a) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("oxhubd: {e}");
            std::process::exit(1);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("oxhubd: runtime {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = rt.block_on(ops::run(c)) {
        eprintln!("oxhubd: {e}");
        std::process::exit(1);
    }
}
