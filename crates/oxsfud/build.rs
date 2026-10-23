// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-OPS-SURFACE · model: claude-opus-5-5
//! 빌드 신원 — `{git SHA(-dirty)}.{빌드 시각}` 을 바이너리에 굽는다(운영 규격서 §8-2 `--version`).

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let ws = Path::new(&manifest).join("../..");
    for p in [".git/HEAD", ".git/index", ".git/refs", "crates", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={}", ws.join(p).display());
    }
    let sha = git(&ws, &["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let dirty = git(&ws, &["status", "--porcelain", "--untracked-files=no", "--", "crates", "Cargo.toml", "Cargo.lock"]).map(|s| !s.is_empty()).unwrap_or(false);
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    println!("cargo:rustc-env=OX_BUILD_ID={sha}{}.{at}", if dirty { "-dirty" } else { "" });
}
