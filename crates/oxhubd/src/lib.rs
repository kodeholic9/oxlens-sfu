// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-DOC-CODE-MIRROR · 정:CAND-DOC-ORCHESTRATION-ASPECT · 정:RULE-DOC-OWNERSHIP · model: claude-opus-5-5
//! oxhubd — 게이트웨이. 최상위 모듈은 규격 영역 이름뿐이다 — 개념(`ses` · `room` · `bus`)은 제 상태와 판정을, 측면(`ops` · `aux` · `cfg`)은 조율을 든다.

pub mod aux;
pub mod bus;
pub mod cfg;
pub mod ops;
pub mod room;
pub mod ses;
