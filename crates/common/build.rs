// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-BUS-SHAPE 정:CAND-OPS-BUILD-DIRTY-MARK · model: claude-opus-5-5
//! B 평면 Handle(proto) 짓기 · 빌드 신원(`{sha12}{-dirty}.{unix 초}`) 굳히기.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(["--no-optional-locks"])
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["proto/handle.proto"], &["proto"])
        .expect("proto");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sha = git(&[
        "-C",
        root.to_str().unwrap_or("."),
        "rev-parse",
        "--short=12",
        "HEAD",
    ])
    .unwrap_or_else(|| "000000000000".to_string());
    let dirty = git(&[
        "-C",
        root.to_str().unwrap_or("."),
        "status",
        "--porcelain",
        "--untracked-files=no",
        "--",
        "crates",
        "Cargo.toml",
        "Cargo.lock",
    ])
    .map(|s| !s.is_empty())
    .unwrap_or(false);
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!(
        "cargo:rustc-env=OX_BUILD_ID={}{}.{}",
        sha,
        if dirty { "-dirty" } else { "" },
        secs
    );
    println!("cargo:rerun-if-changed=proto/handle.proto");
    println!("cargo:rerun-if-changed=../../crates");
    println!("cargo:rerun-if-changed=../../Cargo.toml");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
}
