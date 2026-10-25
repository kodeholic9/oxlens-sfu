// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-OPS-LOGGER 정:RULE-BUS-FATE 운영:RULE-LOG-LOGGER 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! 로거 유닛 `oxlogd` — 인자 해석 · 기동 · 준비 신호 · 부모 생존(stdin 닫힘) 감시.

mod ops;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ops::Args::parse(&args) {
        Ok(ops::Parsed::Version(label)) => {
            println!("{}", common::build_id(label.as_deref()));
        }
        Ok(ops::Parsed::Run(a)) => {
            if let Err(e) = ops::run(a) {
                eprintln!("oxlogd: {e}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("oxlogd: {e}");
            std::process::exit(2);
        }
    }
}
