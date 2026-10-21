// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-BUS-SHAPE(node 안은 로컬 gRPC Handle 하나) · model: claude-opus-5-5
//! B 평면 로컬 Handle 의 gRPC 정의를 짓는다.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/b_v1.proto");
    tonic_build::configure().build_client(true).build_server(true).compile_protos(&["proto/b_v1.proto"], &["proto"])?;
    Ok(())
}
