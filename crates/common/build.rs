// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · model: claude-opus-5-5
//! node 안 Handle(gRPC) 형을 짓는다.

fn main() {
    println!("cargo:rerun-if-changed=proto/handle.proto");
    if let Err(e) = tonic_build::configure().compile_protos(&["proto/handle.proto"], &["proto"]) {
        panic!("handle.proto: {e}");
    }
}
