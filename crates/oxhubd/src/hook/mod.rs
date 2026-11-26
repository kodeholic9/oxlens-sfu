// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-SCOPE · 정:RULE-HOOK-DIRECT · 정:RULE-HOOK-TRANSPORT · 정:RULE-DOC-CODE-MIRROR · model: claude-opus-5-5
//! 대외 연동(HOOK) — 가로지르는 것: zenoh 입구(`zgate` — 채팅 구독 · 사실 구독 · 현황 조회 · 제어가 같은 입구 하나를 쓴다). 개념(채팅)은 이것을 부르고, 이것은 채팅 넘기기 판단(`crate::chat::ext::plan`)을 직접 부른다 — 콜백 주입 없음.

pub mod zgate;
