// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-FWD-MEDIA-WORKERS · model: claude-opus-5-5
//! 1층 — 미디어 워커의 쌓인 패킷 수는 캐시 줄(128B)을 혼자 쓴다 · 넣는 쪽이 패킷마다 읽는 잠듦 표시와 한 줄이 아니다.

use oxsfud::fwd::rt::{Line, Worker};

#[test]
fn queued_owns_its_cache_line() {
    assert_eq!(std::mem::align_of::<Line>(), 128, "정:RULE-FWD-MEDIA-WORKERS queued is 128B aligned");
    assert_eq!(std::mem::size_of::<Line>(), 128, "정:RULE-FWD-MEDIA-WORKERS queued fills its line alone");
    let q = std::mem::offset_of!(Worker, queued);
    let s = std::mem::offset_of!(Worker, sleeping);
    assert_eq!(q % 128, 0, "정:RULE-FWD-MEDIA-WORKERS queued starts a line");
    assert!(s < q || s >= q + 128, "정:RULE-FWD-MEDIA-WORKERS sleeping flag is not on the queued line · queued {q} sleeping {s}");
}
