// author: kodeholic (powered by Claude)
// spec: v1.6 · 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! 빌드 신원 `{SHA 12}{-dirty}.{빌드 시각 unix 초}` — 소스가 바뀌면 다시 셈한다.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    for p in ["src", "../common/src", "../oxsig/src", "../oxsfud/src", "../../.git/HEAD", "../../.git/index"] {
        println!("cargo:rerun-if-changed={p}");
    }
    let sha = git(&["rev-parse", "HEAD"]).map(|s| s.chars().take(12).collect::<String>()).unwrap_or_else(|| "unknown".into());
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).map(|s| !s.is_empty()).unwrap_or(false);
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    println!("cargo:rustc-env=OX_BUILD={sha}{}.{secs}", if dirty { "-dirty" } else { "" });
}
