// author: kodeholic (powered by Claude)
//! 미디어 연결 — 서버와 같은 webrtc-rs 계열(DTLS · SRTP · SCTP)로 서버(ICE-lite)에 붙는다(SDK:RULE-PLAT-SERVER 미디어 엔진).
//! SDP 가 없다 — 서버는 클라 SDP 를 받지 않고 자격(`server_config.ice`)과 배정표로 붙는다. 협상 = 배정표를 받는 경로표에 싣는 것이다.

pub(crate) mod bwe;
pub(crate) mod depack;
#[cfg(test)]
mod media_fixture_test;
pub(crate) mod recv;
pub(crate) mod send;
pub(crate) mod stun;
pub(crate) mod transport;
