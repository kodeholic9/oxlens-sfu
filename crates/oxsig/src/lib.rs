// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:RULE-TRN-THREE-CARRIERS · 연:TRN.oplist · 연:ERR.type.Failure · model: claude-opus-5-5
//! A 평면 wire 의 형 — WS 프레임 · op 표 · 오류 번호 공간 · body 형 · DC 프레임 · 발언권(MBCP) 부호화.

pub mod code;
pub mod dc;
pub mod floor;
pub mod frame;
pub mod msg;
pub mod op;

pub use code::{Code, Failure};
pub use frame::{Frame, FrameError, FrameKind, Header};
pub use op::{Lane, Op};
