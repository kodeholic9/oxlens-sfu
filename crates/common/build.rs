// author: kodeholic (powered by Claude)
// spec: v1.4 · 운영:RULE-BOOT-ARGS · 정:RULE-BUS-SHAPE · model: claude-opus-5-5
//! 빌드 신원(`{sha12}{-dirty}.{unix 초}`)을 굳히고 node 안 Handle(gRPC) 형을 짓는다.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str], root: &Path) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(root).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")?;
    let root = Path::new(&manifest).join("../..");
    let sha = git(&["rev-parse", "HEAD"], &root).map(|s| s.chars().take(12).collect::<String>()).unwrap_or_else(|| "unknown".into());
    let dirty = git(&["status", "--porcelain"], &root).map(|s| !s.is_empty()).unwrap_or(false);
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    println!("cargo:rustc-env=OX_BUILD_SHA={sha}");
    println!("cargo:rustc-env=OX_BUILD_DIRTY={}", if dirty { "-dirty" } else { "" });
    println!("cargo:rustc-env=OX_BUILD_AT={at}");
    for p in [".git/HEAD", ".git/index", "crates", "Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={}", root.join(p).display());
    }
    println!("cargo:rerun-if-changed=proto/handle.proto");
    tonic_build::configure().build_server(true).build_client(true).compile_protos(&["proto/handle.proto"], &["proto"])?;
    Ok(())
}
