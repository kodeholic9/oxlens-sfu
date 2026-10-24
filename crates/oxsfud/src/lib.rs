// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-DOC-CODE-MIRROR · 정:RULE-DOC-PLANES · 정:RULE-DOC-FIXED-PARTS · 정:CAND-DOC-ORCHESTRATION-ASPECT · 정:CAND-DOC-GUARD-PLACE · model: claude-opus-5-5
//! oxsfud — 미디어 서버. 방 상태의 마스터 · 연결(ICE-lite · DTLS · SRTP · SCTP) · 전달 · 발언권. 모듈은 규격 영역 거울(개념 · 가로)이다.

pub mod aff;
pub mod aux;
pub mod bus;
pub mod cfg;
pub mod err;
pub mod flr;
pub mod fwd;
pub mod gc;
pub mod link;
pub mod ops;
pub mod r#pub;
pub mod room;
pub mod rtcp;
pub mod sim;
pub mod sub;
pub mod trk;
