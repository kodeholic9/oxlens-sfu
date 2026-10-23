// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 연:RULE-TRN-THREE-CARRIERS · 연:RULE-TRN-OP-NUMBERS · 정:RULE-DOC-CODE-MIRROR · model: claude-opus-5-5
//! A 평면 wire 의 형 — 연동규격서 원천의 영역마다 한 모듈(운반 · 실패 번호 공간 · op 별 body · 발언권 MBCP).

pub mod aff;
pub mod aux;
pub mod err;
pub mod flr;
pub mod link;
pub mod r#pub;
pub mod room;
pub mod ses;
pub mod sub;
pub mod trk;
pub mod trn;

pub use err::{ErrCode, Failure};
pub use trn::{op, Kind, WsFrame};
