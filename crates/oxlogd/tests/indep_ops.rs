// author: kodeholic (powered by Claude)
// spec: v1.4 · 운영:RULE-LOG-LOGGER · model: claude-opus-5-5
//! 독립 1층 시험(B′ 맹검) — 로거 회전(날짜 경계 ∨ 크기 상한) · 보존(일 단위) · 파일 이름 ox.log.{날짜}.

use oxlogd::ops::{expired, file_name, needs_rotate};

#[test]
fn rotate_on_date_boundary_or_size_cap() {
    assert!(!needs_rotate("20261027", "20261027", 100, 10, 1000), "운영:RULE-LOG-LOGGER 같은 날 · 상한 안 → 회전 0");
    assert!(needs_rotate("20261028", "20261027", 0, 10, 1000), "운영:RULE-LOG-LOGGER 날짜 경계 → 회전");
    assert!(needs_rotate("20261027", "20261027", 995, 10, 1000), "운영:RULE-LOG-LOGGER 크기 상한을 넘기면 회전");
    assert!(!needs_rotate("20261027", "20261027", 990, 10, 1000), "운영:RULE-LOG-LOGGER 상한에 딱 닿으면 아직");
}

#[test]
fn retention_in_days() {
    assert!(!expired(86_400 * 7 - 1, 7), "운영:RULE-LOG-LOGGER 보존 7일 안");
    assert!(expired(86_400 * 7 + 1, 7), "운영:RULE-LOG-LOGGER 보존 7일 넘으면 지운다");
}

#[test]
fn file_name_shape() {
    assert_eq!(file_name("20261027"), "ox.log.20261027", "운영:RULE-LOG-LOGGER {{log_dir}}/ox.log.{{날짜}}");
}
