// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-OPS-SURFACE · 정:RULE-OPS-TOKEN · model: claude-opus-5-5
//! `oxadmin` — 운영 도구 하나(운영 규격서 §6). 붙은 곳 해석 사다리 `--hub` > `OXADMIN_URL` > 기본, 매 실행 stderr 첫 줄에 적는다.

mod ops;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "--version") {
        println!("{}", common::ident::build_identity(env!("OX_BUILD_ID"), None));
        return;
    }
    let o = match ops::parse(argv) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("oxadmin: {e}\n{}", ops::USAGE);
            std::process::exit(2);
        }
    };
    let (url, from) = ops::resolve(&o);
    eprintln!("oxadmin: hub={url} ({from})");
    let key = match (&o.key, &o.secret) {
        (Some(k), Some(s)) => Some((k.clone(), s.clone())),
        _ => None,
    };
    let hub = match ops::http::Hub::new(&url, key) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("oxadmin: {e}");
            std::process::exit(2);
        }
    };
    match ops::run(&o, &hub) {
        Ok(out) => println!("{out}"),
        Err(e) => {
            eprintln!("oxadmin: {e}");
            std::process::exit(1);
        }
    }
}
