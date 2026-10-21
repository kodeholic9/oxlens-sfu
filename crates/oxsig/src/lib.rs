// author: kodeholic (powered by Claude)
// spec: v1.2 @5f1b3b9 · 연:TRN 연:ERR 연:FLR.wire 정:RULE-BUS-REQUEST · model: claude-opus-5-5
//! A 평면 wire(WS 프레임 · op · 코드 · body 형 · DC 프레임 · MBCP)와 B 평면 봉투.

pub mod bplane;
pub mod code;
pub mod dc;
pub mod frame;
pub mod mbcp;
pub mod op;
pub mod wire;

pub use code::Code;
pub use frame::{Frame, FrameError, Kind};
pub use op::{Lane, Op};
