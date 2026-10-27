// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-DOC-CODE-MIRROR · 정:CAND-DOC-ORCHESTRATION-ASPECT · 정:RULE-DOC-OWNERSHIP · model: claude-opus-5-5
//! oxsfud — 미디어 서버. 최상위 모듈은 규격 영역 이름뿐이다 — 개념(`link` · `room` · `aff` · `pub` · `trk` · `sub` · `flr` · `bus`)은 제 상태와 판정을, 측면(`fwd` · `rtcp` · `sim` · `ops` · `gc` · `cfg`)은 길과 조율을 든다.

pub mod aff;
pub mod bus;
pub mod cfg;
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
