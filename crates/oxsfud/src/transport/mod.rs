// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§12 · §17-2 · model: claude-opus-5

//! 전송 — ★**포트 하나, 자격 여러 벌.**

pub mod conn;
pub mod dc;
pub mod demux;
pub mod dispatch;
pub mod dtls;
pub mod framing;
pub mod ice;
pub mod sctp;
pub mod srtp;
pub mod stun;
pub mod tcp;
pub mod udp;

pub use dispatch::Dispatch;
pub use ice::{IceRole, IceTable, Route, TcpHandle};
