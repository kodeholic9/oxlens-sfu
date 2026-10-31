// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-ERR-ISSUERS · model: claude-opus-5-5
//! 1층 — HTTP 표면의 상태 번호 일반 사상: 코드 전량을 빈칸 없이 덮는다(빈칸이면 500 으로 떨군다) · 그 밖 2xxx 는 401.

use oxhubd::ops::http::status_of;
use oxsig::Code;

fn want(c: Code) -> u16 {
    let n = c.num();
    match c {
        Code::ClaimNotAllowed => 400,
        Code::SessionNotFound => 401,
        Code::NotAuthorized => 403,
        Code::RoomNotFound => 404,
        Code::PreconditionFailed => 409,
        Code::SfuUnavailable => 503,
        Code::SfuError => 502,
        _ => match n {
            1000..=1999 => 400,
            2000..=2999 => 401,
            3000..=3999 => 409,
            4000..=4999 => 429,
            _ => 500,
        },
    }
}

#[test]
fn every_code_maps_by_the_general_rule() {
    let bad: Vec<String> = Code::ALL
        .iter()
        .filter(|c| status_of(c.num()).as_u16() != want(**c))
        .map(|c| format!("{} → {} (want {})", c.num(), status_of(c.num()).as_u16(), want(*c)))
        .collect();
    assert!(bad.is_empty(), "정:IMPL-ERR-ISSUERS HTTP general mapping:\n{}", bad.join("\n"));
}

#[test]
fn other_2xxx_is_401() {
    for c in Code::ALL.iter().filter(|c| (2000..=2999).contains(&c.num())) {
        if matches!(c, Code::ClaimNotAllowed | Code::NotAuthorized) {
            continue;
        }
        assert_eq!(status_of(c.num()).as_u16(), 401, "정:IMPL-ERR-ISSUERS other 2xxx 401 · {}", c.num());
    }
}
