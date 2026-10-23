// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-DOC-PLANES · 정:RULE-BUS-SHAPE · model: claude-opus-5-5
//! node 안 gRPC Handle 의 형을 짓는다.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/handle.proto");
    tonic_build::configure().build_server(true).build_client(true).compile_protos(&["proto/handle.proto"], &["proto"])?;
    Ok(())
}
