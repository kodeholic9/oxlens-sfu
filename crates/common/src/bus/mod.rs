// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-BUS-SHAPE · 정:RULE-DOC-PLANES · model: claude-opus-5-5
//! B 평면 공용 — 키 공간 · 배치 함수 · 봉투 · zenoh 세션 · node 안 gRPC Handle 형.

pub mod envelope;
pub mod keys;
pub mod placement;
pub mod session;

pub mod grpc {
    tonic::include_proto!("oxlens.b.v1");
}
