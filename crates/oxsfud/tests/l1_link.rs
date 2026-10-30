// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-LINK-TRANSPORT · 정:IMPL-LINK-ICE-RESTART · model: claude-opus-5-5
//! 1층 — STUN 은 MESSAGE-INTEGRITY 검증 뒤에만 처리한다 · ICE 자격은 벌마다 최대 둘(늦게 적용된 옛 대기분은 통과하지 못한다).

use oxsfud::link::creds::{Checked, Cred, Creds};
use oxsfud::link::stun::{self, StunError};

#[test]
fn stun_integrity_before_processing() {
    let mut c = Creds::new();
    let ufrag = c.active.ufrag.clone();
    let pwd = c.active.pwd.clone();
    let good = stun::request(&ufrag, "peer", &pwd, [7u8; 12], true);
    let b = stun::check(&good, |u| c.pwd(u));
    assert!(b.as_ref().map(|b| b.use_candidate).unwrap_or(false), "S-sym-RULE-LINK-TRANSPORT-3 정상 검사는 통과하고 지명을 읽는다");
    let forged = stun::request(&ufrag, "peer", "not-the-password-0000", [8u8; 12], true);
    assert_eq!(stun::check(&forged, |u| c.pwd(u)).err(), Some(StunError::BadIntegrity), "S-sym-RULE-LINK-TRANSPORT-3 위조 Binding 은 처리 전에 버린다");
    let mut tampered = good.clone();
    let last = tampered.len() - 30;
    tampered[last] ^= 0x01;
    assert!(stun::check(&tampered, |u| c.pwd(u)).is_err(), "S-sym-RULE-LINK-TRANSPORT-3 바뀐 바이트는 검증에서 걸린다");
    let unknown = stun::request("zzzzzzzz", "peer", &pwd, [9u8; 12], true);
    assert_eq!(stun::check(&unknown, |u| c.pwd(u)).err(), Some(StunError::UnknownUfrag), "S-sym-RULE-LINK-TRANSPORT-3 모르는 ufrag 는 처리하지 않는다");
    assert_eq!(c.checked(&ufrag), Checked::Current, "S-sym-RULE-LINK-TRANSPORT-3 검증된 현 세대는 상태를 바꾸지 않는다");
}

#[test]
fn ice_restart_at_most_two_per_set() {
    let mut c = Creds::new();
    let active = c.active.clone();
    let first = Cred::fresh();
    let second = Cred::fresh();
    assert_eq!(c.restart(first.clone()), None, "S-sym-IMPL-LINK-ICE-RESTART-2 첫 재시작은 대기분 하나");
    assert_eq!(c.restart(second.clone()), Some(first.ufrag.clone()), "S-sym-IMPL-LINK-ICE-RESTART-2 겹친 재시작은 대기분을 바꾼다");
    assert_eq!(c.ufrags().len(), 2, "S-sym-IMPL-LINK-ICE-RESTART-2 벌마다 최대 둘");
    let late = stun::request(&first.ufrag, "peer", &first.pwd, [1u8; 12], true);
    assert!(stun::check(&late, |u| c.pwd(u)).is_err(), "S-sym-IMPL-LINK-ICE-RESTART-2 늦게 적용된 옛 응답의 자격은 통과하지 못한다");
    let old = stun::request(&active.ufrag, "peer", &active.pwd, [2u8; 12], false);
    assert!(stun::check(&old, |u| c.pwd(u)).is_ok(), "S-sym-IMPL-LINK-ICE-RESTART-2 새 것이 쓰이기 전에는 옛 것도 받는다");
    let new = stun::request(&second.ufrag, "peer", &second.pwd, [3u8; 12], true);
    assert!(stun::check(&new, |u| c.pwd(u)).is_ok(), "S-sym-IMPL-LINK-ICE-RESTART-2 새 자격은 통과한다");
    assert_eq!(c.checked(&second.ufrag), Checked::Promoted { retired: active.ufrag.clone() }, "S-sym-IMPL-LINK-ICE-RESTART-2 새 ufrag 첫 검사에 옛 것을 내린다");
    assert_eq!(c.ufrags(), vec![second.ufrag.clone()], "S-sym-IMPL-LINK-ICE-RESTART-2 승격 뒤 하나만 남는다");
    assert!(stun::check(&old, |u| c.pwd(u)).is_err(), "S-sym-IMPL-LINK-ICE-RESTART-2 내린 옛 자격은 더는 통과하지 못한다");
}
