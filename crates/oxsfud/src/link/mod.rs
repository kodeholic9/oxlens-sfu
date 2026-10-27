// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-SERVER-SCOPE · 정:RULE-LINK-TRANSPORT · 정:IMPL-LINK-PEER-STATE · 정:IMPL-LINK-ICE-RESTART · 연:LINK · model: claude-opus-5-5
//! 연결 — STUN · ICE 장부 · TCP 프레이밍 · 전송 객체 · 등록부 · DTLS/SCTP · 인증서.

pub mod cert;
pub mod dtls;
pub mod ice;
pub mod io;
pub mod stun;
pub mod tcp;
pub mod transport;

pub use ice::{Creds, Via};
pub use io::{DcEvent, Io, LinkSignal};
pub use transport::{PeerState, Role, Transport};
