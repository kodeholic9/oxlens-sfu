// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-BUS-FATE · 정:RULE-OPS-SURFACE · 정:RULE-CFG-THREE-PLACES · model: claude-opus-5-5
//! `oxsfud` 실행 — 인자 · `--version` · 부모 생존 채널(stdin 파이프가 닫히거나 부모가 바뀌면 끝낸다) · 런타임 밖의 자가 감시(멎으면 진단을 남기고 끝낸다).

use std::io::Read;
use std::os::unix::fs::FileTypeExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::args::Args;
use oxsfud::cfg;

const HANG_MS: u64 = 10_000;

fn exit(msg: &str) -> ! {
    eprintln!("oxsfud: {msg}");
    std::process::exit(2);
}

fn watch_parent() -> bool {
    let piped = std::fs::metadata("/dev/stdin").map(|m| m.file_type().is_fifo()).unwrap_or(false);
    if !piped {
        return false;
    }
    let _ = std::thread::Builder::new().name("ox-parent".to_string()).spawn(|| {
        let mut buf = [0u8; 64];
        let mut stdin = std::io::stdin();
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => {
                    eprintln!("oxsfud: 부모 생존 채널 닫힘 — 종료");
                    std::process::exit(0);
                }
                Ok(_) => {}
            }
        }
    });
    true
}

fn watchdog(beat: Arc<AtomicU64>, parent: Option<u32>) {
    let _ = std::thread::Builder::new().name("ox-watchdog".to_string()).spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            if let Some(pp) = parent
                && std::os::unix::process::parent_id() != pp {
                    eprintln!("oxsfud: 부모가 바뀌었다({pp} → {}) — 종료", std::os::unix::process::parent_id());
                    std::process::exit(0);
                }
            let last = beat.load(Ordering::Acquire);
            let now = common::clock::mono_ms();
            if last > 0 && now.saturating_sub(last) > HANG_MS {
                eprintln!("oxsfud: 런타임 멎음 {}ms — 종료", now - last);
                std::process::exit(70);
            }
        }
    });
}

fn main() {
    let args = Args::parse(std::env::args().skip(1), &cfg::VALUED, &cfg::FLAGS).unwrap_or_else(|e| exit(&e));
    let raw = env!("OX_BUILD_ID");
    if args.flag("version") {
        println!("{}", common::ident::build_identity(raw, args.get("build")));
        return;
    }
    let boot = cfg::Boot::load(&args, raw).unwrap_or_else(|e| exit(&e));
    let _ = watch_parent();
    let beat = Arc::new(AtomicU64::new(0));
    let pp = std::os::unix::process::parent_id();
    watchdog(beat.clone(), (pp != 1).then_some(pp));
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap_or_else(|e| exit(&e.to_string()));
    rt.block_on(async move {
        let b = beat.clone();
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_millis(500));
            loop {
                iv.tick().await;
                b.store(common::clock::mono_ms(), Ordering::Release);
            }
        });
        if let Err(e) = oxsfud::run(boot).await {
            exit(&e);
        }
    });
}
