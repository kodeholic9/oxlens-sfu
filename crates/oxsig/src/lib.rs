// author: kodeholic (powered by Claude)
// spec: v1.4 · 연:RULE-TRN-THREE-CARRIERS · 연:RULE-TRN-OP-NUMBERS · 정:IMPL-ERR-ISSUERS · model: claude-opus-5-5
//! A 평면 wire 의 형 — WS 프레임 · op · 오류 번호 공간 · 메시지 몸통 · DC 프레임 · 발언권 MBCP.

pub mod code;
pub mod dc;
pub mod frame;
pub mod mbcp;
pub mod msg;
pub mod op;
pub mod types;

pub use code::{Code, Failure};
pub use frame::{Frame, FrameError, FrameKind};
pub use op::{Lane, Op};

pub const WIRE_VERSION: u32 = 1;
