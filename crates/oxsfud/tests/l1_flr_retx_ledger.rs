// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-FLR-RTP-OBSERVE · 연:FLR.wire · model: claude-opus-5-5
//! 1층 제어 층 — 발언권 재전송은 원장 값대로 돈다: 선점 `REVOKE` 에 ACK 이 없으면 `T8` 마다, ACK 을 바라는 `TAKEN` 에 ACK 이 없으면 `T21` 마다 다시 보낸다 · 집행자는 가장 이른 만료 시각에 깨어나므로 다음 만료는 앞 만료 + 원장 값이다(고정 판정 주기의 배수가 아니다) · 보내는 횟수는 첫 송신을 합쳐 `C8` · `C21` 이다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::Arc;

use oxsfud::flr::core::{Msg, Mt};
use oxsfud::ops::floor::{self, Entry, T8_MS, T21_MS};
use oxsfud::room::Room;
use oxsig::floor::timers;

use support::*;

const SLACK_MS: u64 = 40;

fn step(r: &Arc<Room>, u: &str, f: impl FnOnce(&mut oxsfud::flr::core::Floor) -> oxsfud::flr::core::Step) {
    floor::run(r, Entry::of("l1", u), |fl, _| Some(f(fl)));
}

fn deadlines(r: &Arc<Room>, name: &'static str, u: &str, until_ms: u64) -> Vec<(u64, u64)> {
    let key = (name, Some(u.to_string()));
    drive(async {
        let mut seen: Vec<(u64, u64)> = Vec::new();
        let end = floor::now_ms() + until_ms;
        while floor::now_ms() < end {
            let at = r.exec.clock.lock().ok().and_then(|c| c.timers.get(&key).copied());
            if let Some(at) = at
                && seen.last().map(|x| x.1) != Some(at)
            {
                seen.push((floor::now_ms(), at));
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        seen
    })
}

fn joined(name: &str) -> (Arc<Room>, String, String) {
    let r = room(&tag(name));
    assert!(floor::start(&r), "stand check — the room's executor starts");
    let (a, b) = (tag(&format!("{name}-a")), tag(&format!("{name}-b")));
    for u in [&a, &b] {
        step(&r, u, |f| f.ws_join(u, true));
        step(&r, u, |f| f.ws_pub_select(u));
    }
    (r, a, b)
}

fn part(r: &Room, u: &str) -> &'static str {
    r.floor.lock().map(|f| f.part_name(u)).unwrap_or("-")
}

fn gaps(d: &[(u64, u64)]) -> Vec<u64> {
    d.windows(2).map(|w| w[1].1 - w[0].1).collect()
}

#[test]
fn an_unanswered_preemption_revoke_repeats_every_t8_up_to_c8_sends() {
    let _g = serial();
    let (r, a, b) = joined("rx8");
    step(&r, &a, |f| f.server_rx(&a, &Msg::new(Mt::Request, None, 1, 0), true, 0));
    assert_eq!(r.floor.lock().ok().and_then(|f| f.holder()).as_deref(), Some(a.as_str()), "stand check — a holds the floor");
    let t0 = floor::now_ms();
    step(&r, &b, |f| f.server_rx(&b, &Msg::new(Mt::Request, None, 7, 0), true, 0));
    assert_eq!(part(&r, &a), "PREVOKE", "stand check — b's higher priority preempts a (first REVOKE sent)");
    let d = deadlines(&r, "T8", &a, T8_MS * timers::C8 as u64 + 300);
    assert!(!d.is_empty() && d[0].1 >= t0 + T8_MS && d[0].1 <= t0 + T8_MS + SLACK_MS, "S-sym-IMPL-FLR-RTP-OBSERVE-3 the first REVOKE retransmission is due T8 after the preemption: {d:?} from {t0}");
    assert!(gaps(&d).iter().all(|g| *g >= T8_MS && *g <= T8_MS + SLACK_MS), "S-sym-IMPL-FLR-RTP-OBSERVE-3 each next REVOKE is due T8 after the previous one fired — the ledger value, not a tick multiple: {:?}", gaps(&d));
    assert_eq!(d.len() + 1, timers::C8 as usize, "S-sym-IMPL-FLR-RTP-OBSERVE-3 REVOKE goes out C8 times in all (the first and {} retransmissions): {d:?}", d.len());
}

#[test]
fn an_unacknowledged_taken_repeats_every_t21_up_to_c21_sends() {
    let _g = serial();
    let (r, a, b) = joined("rx21");
    let t0 = floor::now_ms();
    step(&r, &a, |f| f.server_rx(&a, &Msg::new(Mt::Request, None, 1, 0), true, 0));
    assert_eq!(part(&r, &b), "NP_TAKEN", "stand check — b is told TAKEN and owes an ACK");
    let d = deadlines(&r, "T21", &b, T21_MS * timers::C21 as u64 + 300);
    assert!(!d.is_empty() && d[0].1 >= t0 + T21_MS && d[0].1 <= t0 + T21_MS + SLACK_MS, "S-sym-IMPL-FLR-RTP-OBSERVE-3 the first TAKEN retransmission is due T21 after the grant: {d:?} from {t0}");
    assert!(gaps(&d).iter().all(|g| *g >= T21_MS && *g <= T21_MS + SLACK_MS), "S-sym-IMPL-FLR-RTP-OBSERVE-3 each next TAKEN is due T21 after the previous one fired — the ledger value, not a tick multiple: {:?}", gaps(&d));
    let sends = d.len();
    assert_eq!(sends, timers::C21 as usize, "S-sym-IMPL-FLR-RTP-OBSERVE-3 T21 fires C21 times — C21-1 retransmissions then the give-up — so TAKEN goes out C21 times in all: {d:?}");
    assert!(r.floor.lock().map(|f| !f.armed_participant(&b).contains(&"T21")).unwrap_or(false), "S-sym-IMPL-FLR-RTP-OBSERVE-3 after the give-up T21 is no longer armed");
}
