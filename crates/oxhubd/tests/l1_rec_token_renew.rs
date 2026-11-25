// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-OPS-REC-UNIT · model: claude-opus-5-5
//! 1층 녹음 유닛 토큰 갱신 — 감독자는 녹음 유닛이 `Running` 인 동안 토큰 수명의 3분의 1 마다 새 토큰을 서명해 넘긴다(넘김 하나가 실패해도 만료 전에 한 번 더 기회가 있다)(context 202611/20261124b 수정 갈래 B I4).

use std::time::Duration;

use oxhubd::ops::supervisor::renew_every;

#[test]
fn the_rec_unit_token_is_renewed_at_a_third_of_its_life() {
    for ttl in [60u32, 3_600, 86_400] {
        let every = renew_every(ttl);
        assert_eq!(every, Duration::from_secs(u64::from(ttl / 3)), "S-sym-IMPL-OPS-REC-UNIT-4① the renewal period is a third of the token life ({ttl}s)");
        assert!(every * 2 < Duration::from_secs(u64::from(ttl)), "S-sym-IMPL-OPS-REC-UNIT-4① one failed handover still leaves a second try before the held token expires ({ttl}s)");
    }
    assert_eq!(renew_every(2), Duration::from_secs(1), "stand check — the period never falls to zero");
}
