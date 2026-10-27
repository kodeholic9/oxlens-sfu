// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FLR-LOCK · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-FLR-MEMBER-JOIN · 연:RULE-FLR-MSG-REQUEST · 연:RULE-FLR-PERM-REVOKE-ENFORCE · model: claude-opus-5-5
//! 독립 1층 시험(B′ 맹검) — 미디어 결합 번호 규칙(집행 차례 전수) · 요청 관문 차례 · 권한 회수 · 재입장 · T2 는 첫 RTP 부터.

use oxsfud::flr::bind::{Effect, Slot};
use oxsfud::flr::core::{self, deny, Env, Fx, Msg, Srv, Timer, R, T};
use oxsfud::flr::{Effect as FEffect, Floor, Key};
use serde_json::Value;
use std::path::PathBuf;

fn vectors(name: &str) -> Value {
    let base = match std::env::var("OXLENS_SPEC") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec"),
    };
    let p = base.join("server/out/vectors").join(name);
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("벡터 없음 {}: {e}", p.display()))).unwrap()
}

#[derive(Default, Clone, PartialEq, Debug)]
struct OSlot {
    bound: Option<(String, u64)>,
    max: Option<u64>,
}

impl OSlot {
    fn apply(&mut self, e: &Effect) -> bool {
        match e {
            Effect::Bind(u, g) => {
                if self.max.is_some_and(|m| *g <= m) {
                    return false;
                }
                self.bound = Some((u.clone(), *g));
                self.max = Some(*g);
                true
            }
            Effect::Release(g) => {
                self.max = Some(self.max.map_or(*g, |m| m.max(*g)));
                if self.bound.as_ref().is_some_and(|(_, b)| b == g) {
                    self.bound = None;
                    return true;
                }
                false
            }
        }
    }
}

fn perms(n: usize) -> Vec<Vec<usize>> {
    if n == 0 {
        return vec![vec![]];
    }
    let mut out = vec![];
    for p in perms(n - 1) {
        for i in 0..=p.len() {
            let mut q = p.clone();
            q.insert(i, n - 1);
            out.push(q);
        }
    }
    out
}

fn eff(v: &Value) -> Effect {
    let a = v.as_array().unwrap();
    match a[0].as_str().unwrap() {
        "bind" => Effect::Bind(a[1].as_str().unwrap().to_string(), a[2].as_u64().unwrap()),
        _ => Effect::Release(a[1].as_u64().unwrap()),
    }
}

fn decided_bound(d: &[Effect]) -> Option<(String, u64)> {
    let mut b = None;
    for e in d {
        match e {
            Effect::Bind(u, g) => b = Some((u.clone(), *g)),
            Effect::Release(g) => {
                if b.as_ref().is_some_and(|(_, x): &(String, u64)| x == g) {
                    b = None
                }
            }
        }
    }
    b
}

#[test]
fn media_bind_every_order_ends_as_decided_order() {
    let v = vectors("flr.json");
    let cases = v["media_bind"].as_array().expect("media_bind 판 묶음");
    assert!(cases.len() >= 6, "S-sym-IMPL-FLR-MEDIA-BIND-3 media_bind 판 {}", cases.len());
    let mut total = 0;
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let decided: Vec<Effect> = c["decided"].as_array().unwrap().iter().map(eff).collect();
        let want = decided_bound(&decided);
        let vb = c["bound"].as_array().map(|a| (a[0].as_str().unwrap().to_string(), a[1].as_u64().unwrap()));
        assert_eq!(want, vb, "S-sym-IMPL-FLR-MEDIA-BIND-3 판정 차례 결과 ≠ 벡터 — {name}");
        let all = perms(decided.len());
        assert_eq!(all.len(), c["orders"].as_array().unwrap().len(), "S-sym-IMPL-FLR-MEDIA-BIND-3 차례 전수 — {name}");
        for ord in &all {
            let (mut o, mut p) = (OSlot::default(), Slot::new());
            for &i in ord {
                let a = o.apply(&decided[i]);
                let b = p.apply(&decided[i]);
                assert_eq!(a, b, "S-sym-IMPL-FLR-MEDIA-BIND-3 적용 여부 — {name} 차례 {ord:?} 효과 {i}");
            }
            assert_eq!(p.bound, want, "S-sym-IMPL-FLR-MEDIA-BIND-3 끝 슬롯 = 판정 차례 결과 — {name} 차례 {ord:?}");
            if want.is_none() {
                assert_eq!(p.bound, None, "S-sym-IMPL-FLR-MEDIA-BIND-4 반환한 사람이 되살아나는 차례 0 — {name} {ord:?}");
            }
            total += 1;
        }
        for o in c["orders"].as_array().unwrap() {
            let ord: Vec<usize> = o["order"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as usize).collect();
            let applied: Vec<bool> = o["applied"].as_array().unwrap().iter().map(|x| x.as_bool().unwrap()).collect();
            let mut p = Slot::new();
            let got: Vec<bool> = ord.iter().map(|&i| p.apply(&decided[i])).collect();
            assert_eq!(got, applied, "S-sym-IMPL-FLR-MEDIA-BIND-3 벡터 applied — {name} {ord:?}");
        }
    }
    assert_eq!(total, 160, "S-sym-IMPL-FLR-MEDIA-BIND-3 집행 차례 160");
}

#[test]
fn media_bind_late_release_never_clears_new_speaker() {
    let mut s = Slot::new();
    assert!(s.apply(&Effect::Bind("A".into(), 0)), "S-sym-IMPL-FLR-MEDIA-BIND-3 첫 결합");
    assert!(s.apply(&Effect::Bind("B".into(), 1)), "S-sym-IMPL-FLR-MEDIA-BIND-3 새 화자 결합이 먼저 닿는다");
    assert!(!s.apply(&Effect::Release(0)), "S-sym-IMPL-FLR-MEDIA-BIND-3 앞 화자의 늦은 해제는 남의 결합을 안 비운다");
    assert_eq!(s.bound, Some(("B".into(), 1)), "S-sym-IMPL-FLR-MEDIA-BIND-3 슬롯은 B 그대로");
    let mut s = Slot::new();
    assert!(!s.apply(&Effect::Release(4)), "S-sym-IMPL-FLR-MEDIA-BIND-4 결합 전 해제");
    assert!(!s.apply(&Effect::Bind("A".into(), 4)), "S-sym-IMPL-FLR-MEDIA-BIND-4 해제보다 늦은 결합은 버린다");
    assert_eq!(s.bound, None, "S-sym-IMPL-FLR-MEDIA-BIND-4 끝 슬롯 비어 있음");
    assert!(!s.apply(&Effect::Bind("Z".into(), 2)), "S-sym-IMPL-FLR-MEDIA-BIND-4 옛 번호 결합은 버린다");
    assert!(s.apply(&Effect::Bind("C".into(), 5)), "S-sym-IMPL-FLR-MEDIA-BIND-3 새 번호는 결합");
}

#[test]
fn media_bind_random_sequences_match_oracle() {
    let mut x: u64 = 0x5107_2026;
    for _ in 0..2000 {
        let mut decided = vec![];
        let people = ["A", "B", "C", "D"];
        let n = 1 + (x % 5) as usize;
        for g in 0..n as u64 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            decided.push(Effect::Bind(people[(x % 4) as usize].into(), g));
            if !x.is_multiple_of(3) {
                decided.push(Effect::Release(g));
            }
        }
        let mut idx: Vec<usize> = (0..decided.len()).collect();
        for i in (1..idx.len()).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            idx.swap(i, (x % (i as u64 + 1)) as usize);
        }
        let (mut o, mut p) = (OSlot::default(), Slot::new());
        for &i in &idx {
            assert_eq!(p.apply(&decided[i]), o.apply(&decided[i]), "S-sym-IMPL-FLR-MEDIA-BIND-3 규칙 ①② 적용 여부 — {idx:?}");
        }
        assert_eq!(p.bound, o.bound, "S-sym-IMPL-FLR-MEDIA-BIND-3 임의 판정 열 · 임의 집행 차례 — 규칙 ①② 그대로 {idx:?}");
    }
}

#[test]
#[ignore = "spec defect: S-sym-IMPL-FLR-MEDIA-BIND-4 — 승계(해제 없는 결합) 뒤 해제가 승계 결합보다 먼저 닿으면 규칙 ②(지금 결합 번호 == g 일 때만 비움)가 옛 결합을 남긴다"]
fn media_bind_succession_then_release_reordered() {
    let decided = [Effect::Bind("A".into(), 0), Effect::Bind("B".into(), 1), Effect::Release(1)];
    let want = decided_bound(&decided);
    let mut p = Slot::new();
    for i in [0usize, 2, 1] {
        p.apply(&decided[i]);
    }
    assert_eq!(p.bound, want, "S-sym-IMPL-FLR-MEDIA-BIND-4 반환한 사람이 슬롯에 남는 차례 0 — 판정 차례 결과 {want:?}");
}

fn env() -> Env {
    Env { qmax: 8, t2_secs: 30, t2_remain_secs: 30 }
}

fn req(prio: u32) -> Msg {
    Msg::new(T::Request, None, prio, 0)
}

fn denies(r: &R, u: &str) -> Vec<u32> {
    r.out.iter().filter(|o| o.to == u && o.m.t == T::Deny).map(|o| o.m.n).collect()
}

fn got(r: &R, u: &str, t: T) -> usize {
    r.out.iter().filter(|o| o.to == u && o.m.t == t).count()
}

fn room(users: &[(&str, bool, bool)]) -> Srv {
    let mut s = Srv::new();
    for (u, half, perm) in users {
        core::ws_join(&mut s, env(), u, *half, *perm);
        core::ws_pub_select(&mut s, env(), u);
        if *half {
            core::ws_reg_half(&mut s, env(), u);
        }
    }
    s
}

#[test]
fn request_gate_no_permission_before_priority() {
    let mut s = room(&[("a", true, false), ("b", true, true)]);
    let r = core::server_rx(&mut s, env(), "a", &req(255));
    assert_eq!(denies(&r, "a"), vec![deny::NO_PERMISSION], "S-sym-IMPL-ROOM-PERMISSION-5 긴급이어도 DENY(102)");
    assert_eq!(got(&r, "a", T::Granted), 0, "S-sym-IMPL-ROOM-PERMISSION-5 긴급이 통과하는 일 0");
    assert_eq!(s.holder(), None, "S-sym-IMPL-ROOM-PERMISSION-5 화자 없음");
    let r = core::server_rx(&mut s, env(), "b", &req(0));
    assert_eq!(got(&r, "b", T::Granted), 1, "S-sym-IMPL-ROOM-PERMISSION-5 대조군 — 권한 있으면 허가");
    let r = core::server_rx(&mut s, env(), "a", &req(255));
    assert_eq!(denies(&r, "a"), vec![deny::NO_PERMISSION], "S-sym-IMPL-ROOM-PERMISSION-5 화자가 있어도 선점 전에 102");
    assert_eq!(s.holder().map(|x| x.as_str()), Some("b"), "S-sym-IMPL-ROOM-PERMISSION-5 선점 0");
}

#[test]
fn request_gate_causes() {
    let mut s = room(&[("a", true, true)]);
    let r = core::server_rx(&mut s, env(), "a", &req(0));
    assert_eq!(denies(&r, "a"), vec![deny::ONLY_ONE], "S-sym-IMPL-FLR-MEMBER-JOIN-1 방에 나뿐이면 3(Idle)");
    let r = core::server_rx(&mut s, env(), "x", &req(0));
    assert_eq!(denies(&r, "x"), vec![deny::OTHER_REASON], "S-sym-IMPL-FLR-MEMBER-JOIN-1 미입장 255");
    let mut s = room(&[("a", true, true), ("b", false, true)]);
    let r = core::server_rx(&mut s, env(), "b", &req(0));
    assert_eq!(denies(&r, "b"), vec![deny::RECV_ONLY], "S-sym-IMPL-FLR-MEMBER-JOIN-1 반이중 발행 없음 → 5");
    let mut s = Srv::new();
    core::ws_join(&mut s, env(), "a", true, true);
    core::ws_reg_half(&mut s, env(), "a");
    core::ws_join(&mut s, env(), "b", true, true);
    core::ws_pub_select(&mut s, env(), "b");
    core::ws_reg_half(&mut s, env(), "b");
    let r = core::server_rx(&mut s, env(), "a", &req(0));
    assert_eq!(denies(&r, "a"), vec![deny::NOT_PUBROOM], "S-sym-IMPL-FLR-MEMBER-JOIN-1 pub_room 아님 → 101");
}

#[test]
fn queue_full_and_succession_without_idle() {
    let e = Env { qmax: 1, t2_secs: 30, t2_remain_secs: 30 };
    let mut s = room(&[("a", true, true), ("b", true, true), ("c", true, true)]);
    let r = core::server_rx(&mut s, e, "a", &req(0));
    assert_eq!(got(&r, "a", T::Granted), 1, "S-sym-IMPL-FLR-MEDIA-BIND-1 허가");
    assert!(r.fx.iter().any(|f| *f == Fx::Bind("a".into())), "S-sym-IMPL-FLR-MEDIA-BIND-2 허가 = 결합 효과");
    let r = core::server_rx(&mut s, e, "b", &req(0));
    assert_eq!(got(&r, "b", T::Granted) + denies(&r, "b").len(), 0, "S-sym-IMPL-FLR-MEDIA-BIND-1 둘째는 대기");
    let r = core::server_rx(&mut s, e, "c", &req(0));
    assert_eq!(denies(&r, "c"), vec![deny::QUEUE_FULL], "S-sym-IMPL-FLR-MEDIA-BIND-1 큐가 차면 7");
    let r = core::server_rx(&mut s, e, "a", &Msg::new(T::Release, None, 0, 0));
    assert_eq!(got(&r, "b", T::Granted), 1, "S-sym-IMPL-FLR-MEDIA-BIND-1 놓으면 대기자 승계");
    let idles = r.out.iter().filter(|o| o.m.t == T::Idle).count();
    assert_eq!(idles, 0, "S-sym-IMPL-FLR-MEDIA-BIND-1 큐가 있으면 IDLE 0(깜빡임 0)");
    assert_eq!(s.holder().map(|x| x.as_str()), Some("b"), "S-sym-IMPL-FLR-MEDIA-BIND-1 화자 = b");
}

#[test]
fn perm_down_on_queued_dequeues_with_deny_102() {
    let mut s = room(&[("a", true, true), ("b", true, true), ("c", true, true)]);
    core::server_rx(&mut s, env(), "a", &req(0));
    core::server_rx(&mut s, env(), "b", &req(0));
    core::server_rx(&mut s, env(), "c", &req(0));
    let r = core::perm_down(&mut s, env(), "b");
    assert_eq!(denies(&r, "b"), vec![deny::NO_PERMISSION], "S-sym-IMPL-ROOM-PERMISSION-4 대기자에게 DENY(102) 1");
    assert!(s.queue.iter().all(|w| w.u != "b"), "S-sym-IMPL-ROOM-PERMISSION-4 큐에서 빠진다");
    assert!(s.queue.iter().any(|w| w.u == "c"), "S-sym-IMPL-ROOM-PERMISSION-4 남은 대기자 그대로");
    let r = core::server_rx(&mut s, env(), "a", &Msg::new(T::Release, None, 0, 0));
    assert_eq!(got(&r, "b", T::Granted), 0, "S-sym-IMPL-ROOM-PERMISSION-4 승계 GRANTED 가 그 사람에게 0");
    assert_eq!(got(&r, "c", T::Granted), 1, "S-sym-IMPL-ROOM-PERMISSION-4 다음 대기자 승계");
}

#[test]
fn perm_down_on_speaker_revokes_with_cause_3() {
    let mut s = room(&[("a", true, true), ("b", true, true)]);
    core::server_rx(&mut s, env(), "a", &req(0));
    let r = core::perm_down(&mut s, env(), "a");
    let rv: Vec<u32> = r.out.iter().filter(|o| o.to == "a" && o.m.t == T::Revoke).map(|o| o.m.n).collect();
    assert_eq!(rv, vec![core::revoke::NO_PERMISSION], "S-sym-IMPL-ROOM-PERMISSION-1 화자는 REVOKE(3)");
}

#[test]
fn rejoin_then_request_is_granted() {
    let mut s = room(&[("a", true, true), ("b", true, true)]);
    core::ws_leave(&mut s, env(), "a");
    core::ws_join(&mut s, env(), "a", true, true);
    core::ws_pub_select(&mut s, env(), "a");
    core::ws_reg_half(&mut s, env(), "a");
    let r = core::server_rx(&mut s, env(), "a", &req(0));
    assert_eq!(denies(&r, "a"), Vec::<u32>::new(), "S-sym-IMPL-FLR-MEMBER-JOIN-1 재입장 뒤 미입장 거절 DENY 0");
    assert_eq!(got(&r, "a", T::Granted), 1, "S-sym-IMPL-FLR-MEMBER-JOIN-1 재입장 뒤 GRANTED");
}

#[test]
fn rejoin_while_revoking_closes_old_episode() {
    let mut s = room(&[("a", true, true), ("b", true, true), ("c", true, true)]);
    core::server_rx(&mut s, env(), "a", &req(0));
    core::server_rx(&mut s, env(), "b", &req(0));
    core::perm_down(&mut s, env(), "a");
    assert_eq!(s.revoking.as_deref(), Some("a"), "시험 형상 — a 회수 중");
    core::ws_leave(&mut s, env(), "a");
    core::ws_join(&mut s, env(), "a", true, true);
    assert_ne!(s.revoking.as_deref(), Some("a"), "S-sym-IMPL-FLR-MEMBER-JOIN-1 회수 중 재입장이면 옛 회수 일화가 닫힌다");
    assert_eq!(s.holder().map(|x| x.as_str()), Some("b"), "S-sym-IMPL-FLR-MEMBER-JOIN-1 큐가 있으면 곧바로 승계");
}

fn step_rx(f: &mut Floor, now: u64, u: &str, m: Msg) -> Vec<FEffect> {
    let u = u.to_string();
    f.step(now, move |s, e| core::server_rx(s, e, &u, &m)).effects
}

#[test]
fn grant_no_counts_each_grant_and_release_carries_it() {
    let mut f = Floor::new(8, 30);
    for u in ["a", "b"] {
        let uu = u.to_string();
        f.step(0, move |s, e| {
            core::ws_join(s, e, &uu, true, true);
            core::ws_pub_select(s, e, &uu);
            core::ws_reg_half(s, e, &uu)
        });
    }
    let e1 = step_rx(&mut f, 10, "a", req(0));
    assert!(e1.contains(&FEffect::Bind("a".into(), 0)), "S-sym-IMPL-FLR-MEDIA-BIND-3 첫 허가 grant_no 0 — {e1:?}");
    let e2 = step_rx(&mut f, 20, "b", req(0));
    assert!(e2.is_empty(), "S-sym-IMPL-FLR-MEDIA-BIND-3 대기는 효과 없음 — {e2:?}");
    let e3 = step_rx(&mut f, 30, "a", Msg::new(T::Release, None, 0, 0));
    assert!(e3.contains(&FEffect::Bind("b".into(), 1)), "S-sym-IMPL-FLR-MEDIA-BIND-3 승계 grant_no 1 — {e3:?}");
    assert!(!e3.iter().any(|x| matches!(x, FEffect::Release(1, _))), "S-sym-IMPL-FLR-MEDIA-BIND-3 새 결합 번호로 해제 0 — {e3:?}");
    let e4 = step_rx(&mut f, 40, "b", Msg::new(T::Release, None, 0, 0));
    assert!(e4.iter().any(|x| matches!(x, FEffect::Release(1, _))), "S-sym-IMPL-FLR-MEDIA-BIND-3 해제는 끝내는 결합 번호 1 — {e4:?}");
    assert_eq!(f.grant_no, 2, "S-sym-IMPL-FLR-MEDIA-BIND-3 허가마다 1 증가");
}

#[test]
fn t2_counts_from_first_rtp_not_grant() {
    let mut f = Floor::new(8, 30);
    for u in ["a", "b"] {
        let uu = u.to_string();
        f.step(0, move |s, e| {
            core::ws_join(s, e, &uu, true, true);
            core::ws_pub_select(s, e, &uu);
            core::ws_reg_half(s, e, &uu)
        });
    }
    step_rx(&mut f, 1000, "a", req(0));
    f.step(6000, |s, e| core::rtp(s, e, "a"));
    assert_eq!(f.first_rtp_ms(), Some(6000), "S-sym-IMPL-FLR-RTP-OBSERVE-1 첫 RTP 시각");
    let ts = f.timers(6000);
    let t2: Vec<u64> = ts.iter().filter(|(n, _, _)| n == "T2").map(|(_, _, d)| *d).collect();
    assert_eq!(t2, vec![6000 + 30_000], "S-sym-IMPL-FLR-RTP-OBSERVE-1 T2 = 첫 RTP + T2(허가 시각 0) — {ts:?}");
    let due = f.due(1000 + 30_000 + 1);
    assert!(!due.contains(&Key::G(Timer::T2)), "S-sym-IMPL-FLR-RTP-OBSERVE-1 허가 + 30s 에 T2 만료 0 — {due:?}");
    assert!(f.due(6000 + 30_000).contains(&Key::G(Timer::T2)), "S-sym-IMPL-FLR-RTP-OBSERVE-1 첫 RTP + 30s 에 T2 만료");
}
