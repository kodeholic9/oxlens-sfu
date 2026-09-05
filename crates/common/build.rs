// author: kodeholic (powered by Claude)
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["../../proto/oxlens_b_v1.proto"], &["../../proto"])?;
    Ok(())
}
