// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 운영:RULE-BOOT-ARGS 정:RULE-CFG-THREE-PLACES · model: claude-opus-5-5
//! oxsfud 입구 — `--version` 은 빌드 신원 한 줄 · 그 밖은 기동.

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match oxsfud::cfg::parse_args(&argv) {
        Ok(oxsfud::cfg::Parsed::Version(label)) => {
            println!("{}", common::build_id(label.as_deref()));
            return;
        }
        Ok(oxsfud::cfg::Parsed::Run(a)) => a,
        Err(e) => {
            eprintln!("oxsfud: {e}");
            std::process::exit(2);
        }
    };
    let cfg = match oxsfud::cfg::load(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("oxsfud: {e}");
            std::process::exit(2);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("oxsfud: runtime {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = rt.block_on(oxsfud::ops::run(cfg)) {
        eprintln!("oxsfud: {e}");
        std::process::exit(1);
    }
}
