// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 연:RULE-TRN-THREE-CARRIERS 연:RULE-TRN-OP-NUMBERS 정:IMPL-ERR-ISSUERS 정:RULE-BUS-SHAPE · model: claude-opus-5-5
//! 연동규격서 wire 계층 — WS 프레임 · op 번호와 단 · 오류 코드 · 메시지 형 · DC 프레임 · MBCP · B 평면 봉투.

pub mod code;
pub mod envelope;
pub mod frame;
pub mod mbcp;
pub mod msg;
pub mod op;

pub use code::{Code, Failure};
pub use op::{Lane, Op};
