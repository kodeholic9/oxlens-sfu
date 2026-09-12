// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§7 · §9 · §11 · §12 · §15-6 · model: claude-opus-5

//! `oxsfud` — 미디어 서버. 전송·트랙·fan-out·발언권이 여기 산다.

pub mod autolayer;
pub mod fanout;
pub mod floor;
pub mod grpc;
pub mod handle;
pub mod identity;
pub mod lifeline;
pub mod peer;
pub mod pt;
pub mod reaper;
pub mod rewriter;
pub mod room;
pub mod transport;
pub mod twcc;
