// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-BUS-SHAPE · 정:RULE-DOC-CODE-MIRROR · model: claude-opus-5-5
//! BUS 영역(hub 쪽) — core 판정 · shell I/O.

pub mod core;
pub mod shell;

pub use shell::{Bus, Delivery};
