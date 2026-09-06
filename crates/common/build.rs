// author: kodeholic (powered by Claude)
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ★빌드 신원 — 돌고 있는 것이 **어느 소스의 산물인지** 서버가 스스로 말하게 한다.
    //   이게 없으면 옛 바이너리를 상대로 회귀를 돌고도 초록으로 읽는다(20260906 실측 2회).
    let rev = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .is_some_and(|o| !o.stdout.is_empty());
    let built_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=OXLENS_BUILD_REV={rev}{}", if dirty { "-dirty" } else { "" });
    println!("cargo:rustc-env=OXLENS_BUILD_AT={built_at}");
    // ★소스가 바뀌면 다시 찍는다 — 안 그러면 stamp 가 첫 빌드에 얼어붙는다.
    println!("cargo:rerun-if-changed=../../crates");
    println!("cargo:rerun-if-changed=../../proto");

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["../../proto/oxlens_b_v1.proto"], &["../../proto"])?;
    Ok(())
}
