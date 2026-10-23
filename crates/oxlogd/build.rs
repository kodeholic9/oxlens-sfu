// author: kodeholic (powered by Claude)
// spec: v1.2 @250e836 · 운영 규격서 §8-2 `--version` · model: claude-opus-5-5
//! 빌드 신원 재료 — git SHA(+dirty)와 빌드 시각을 바이너리에 싣는다. 소스나 HEAD 가 바뀔 때만 다시 찍는다.

use std::process::Command;

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for p in [".git/HEAD", ".git/refs/heads", "crates", "Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={}", root.join(p).display());
    }
    let sha = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(&root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let dirty = Command::new("git")
        .args(["--no-optional-locks", "status", "--porcelain", "--untracked-files=no"])
        .current_dir(&root)
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    let built = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    println!("cargo:rustc-env=OX_GIT_SHA={}{}", sha, if dirty { "-dirty" } else { "" });
    println!("cargo:rustc-env=OX_BUILD_TIME={built}");
}
