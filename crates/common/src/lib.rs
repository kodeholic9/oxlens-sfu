// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§18-1 · 정책서 · model: claude-opus-5

//! `common` — 값의 자리 셋(정§18-1)이 여기 산다: ★**시스템 파일 · 정책 파일 · CLI 인자.**
//!
//! ★**판별 기준은 하나다** — 프로세스가 2개 떴을 때 서로 **달라져야 하면 인자**, **같아야 하면 파일**.

pub mod args;
pub mod build;
pub mod policy;
pub mod system;

pub use args::Args;
pub use build::BuildId;
pub use policy::Policy;
pub use system::System;
