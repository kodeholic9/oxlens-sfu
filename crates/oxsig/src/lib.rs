// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§3 · 연§4 · 연§6 · 연§10 · model: claude-opus-5

//! `oxsig` — A 평면 wire 의 타입. 프레임·op·코드·공통 형·body 가 여기 산다.
//!
//! ★**body 의 권위는 이 크레이트의 타입이다.** `serde_json::Value` 로 받아 넘기지 않는다 —
//! 형을 비껴가면 어느 필드가 계약인지가 소스마다 갈린다.

pub mod body;
pub mod code;
pub mod frame;
pub mod op;
pub mod types;

pub use code::{Code, Family};
pub use frame::{Header, Kind};
pub use op::{Lane, Op};
pub use types::{Affiliation, Assign, Failure, MemberInfo, Permission, Source, StreamType, TrackEntry, Version};
