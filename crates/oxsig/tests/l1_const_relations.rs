// author: kodeholic (powered by Claude)
// spec: v1.12 · 연:RULE-FLR-T9-VALUE · 연:RULE-FLR-TIMERS · 연:RULE-DOC-COND-INDEX · model: claude-opus-5-5
//! 1층 상수 대조 — 값끼리의 관계: `T9`(Retry-after)는 클라 `T101` 재전송 한 바퀴(`T101`×`C101`)보다 길다 — 그래야 대기자의 요청이 한 바퀴 돈다(context 202611/20261124b 서버 규칙 갈래).

use oxsig::floor::timers;

#[test]
fn t9_outlasts_one_round_of_t101_retransmits() {
    let round = timers::T101_MS * u64::from(timers::C101);
    assert_eq!(round, 1_500, "stand check — one T101 round is 0.5s × 3");
    assert!(timers::T9_MS > round, "C-cond-RULE-DOC-COND-INDEX-5① C-cond-RULE-FLR-T9-VALUE-1 T9 {} ms outlasts T101×C101 {round} ms", timers::T9_MS);
}
