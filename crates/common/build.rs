// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-BUS-SHAPE · 정:RULE-OPS-LOGGER · 운영 규격서 §8-2 · model: claude-opus-5-5
//! 빌드 — node 안 gRPC Handle 형을 짓고, 빌드 신원(`{sha12}{-dirty}.{unix 초}`)을 박는다.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["proto/oxlens_b_v1.proto"], &["proto"])
        .unwrap_or_else(|e| panic!("proto: {e}"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for p in ["crates", "Cargo.toml", "Cargo.lock", ".git/HEAD", ".git/index", ".git/refs/heads"] {
        println!("cargo:rerun-if-changed={}", root.join(p).display());
    }
    println!("cargo:rerun-if-changed=proto/oxlens_b_v1.proto");
    let sha = git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).map(|s| !s.is_empty()).unwrap_or(false);
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    println!("cargo:rustc-env=OX_BUILD_ID={sha}{}.{secs}", if dirty { "-dirty" } else { "" });
}
