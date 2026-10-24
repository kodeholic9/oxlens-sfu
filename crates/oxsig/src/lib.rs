// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 연:RULE-TRN-THREE-CARRIERS · 연:RULE-TRN-UNKNOWN-JSON-FIELD · 정:RULE-DOC-CODE-MIRROR · model: claude-opus-5-5
//! A 평면 wire 의 형 — WS 프레임 · op 표 · 오류 번호 공간 · 공용 형 · op 몸통 · DC 프레임 · MBCP.

pub mod body;
pub mod code;
pub mod dc;
pub mod frame;
pub mod mbcp;
pub mod op;
pub mod parse;
pub mod types;

pub use body::*;
pub use code::{Code, CODES, RETIRED};
pub use frame::{Frame, FrameError, Kind};
pub use op::{Lane, Op};
pub use types::*;
