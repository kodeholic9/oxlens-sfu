// author: kodeholic (powered by Claude)
// spec: v1.1 · 운영 §3-4 · §8-2 · model: claude-opus-5

//! ★**빌드 신원을 컴파일 때 찍는다** — 돌고 있는 것이 **어느 소스의 산물인지**
//! 서버가 ★**스스로 말하게** 한다. 사람이 기억으로 막을 일이 아니다.

use std::process::Command;

fn main() {
    let sha = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "nogit".into());
    let dirty = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!(
        "cargo:rustc-env=OX_BUILD_STAMP={sha}{}+{at}",
        if dirty { "-dirty" } else { "" }
    );
    // ★소스가 바뀌면 다시 찍는다 — 안 그러면 stamp 가 첫 빌드에 얼어붙는다.
    println!("cargo:rerun-if-changed=../");
    println!("cargo:rerun-if-changed=../../Cargo.toml");
}
