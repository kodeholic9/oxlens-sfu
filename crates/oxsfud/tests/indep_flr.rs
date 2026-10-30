// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FLR-LOCK · model: claude-opus-5-5
//! 맹검 독립 1층 — 슬롯 결합의 집행 차례 무관성(결합은 본 최댓값보다 클 때만 · 해제 g 는 지금 번호 ≤ g 면 비운다) · 신탁은 규격 문장에서 지어 벡터와 먼저 맞댄다.

use oxsfud::flr::bind::{Effect, Slot};
use std::path::PathBuf;

fn vectors_dir() -> PathBuf {
    match std::env::var("OXLENS_SPEC_VECTORS") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec/server/out/vectors"),
    }
}

fn load(name: &str) -> serde_json::Value {
    let p = vectors_dir().join(name);
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("vector file {} unreadable: {e}", p.display()));
    serde_json::from_str(&text).expect("vector json")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Oracle {
    bound: Option<(String, u64)>,
    max_seen: Option<u64>,
}

impl Oracle {
    fn apply(&mut self, e: &Effect) -> bool {
        match e {
            Effect::Bind { user, g } => {
                if self.max_seen.is_none_or(|m| *g > m) {
                    self.bound = Some((user.clone(), *g));
                    self.max_seen = Some(*g);
                    true
                } else {
                    false
                }
            }
            Effect::Release { g } => {
                self.max_seen = Some(self.max_seen.map_or(*g, |m| m.max(*g)));
                match &self.bound {
                    Some((_, bg)) if *bg <= *g => {
                        self.bound = None;
                        true
                    }
                    _ => false,
                }
            }
        }
    }
}

fn effect_of(v: &serde_json::Value) -> Effect {
    let a = v.as_array().expect("decided row");
    match a[0].as_str().expect("kind") {
        "bind" => Effect::Bind { user: a[1].as_str().expect("user").to_string(), g: a[2].as_u64().expect("g") },
        "release" => Effect::Release { g: a[1].as_u64().expect("g") },
        k => panic!("unknown effect {k}"),
    }
}

fn bound_of(v: &serde_json::Value) -> Option<(String, u64)> {
    if v.is_null() {
        return None;
    }
    let a = v.as_array().expect("bound");
    Some((a[0].as_str().expect("user").to_string(), a[1].as_u64().expect("g")))
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn decided_seq(rng: &mut Rng, len: usize) -> Vec<Effect> {
    let users = ["A", "B", "C", "D"];
    let mut out = Vec::new();
    let mut holder: Option<u64> = None;
    let mut next_g = 0u64;
    while out.len() < len {
        let grant = holder.is_none() || rng.below(2) == 0;
        if grant {
            let u = users[rng.below(users.len() as u64) as usize].to_string();
            out.push(Effect::Bind { user: u, g: next_g });
            holder = Some(next_g);
            next_g += 1;
        } else if let Some(g) = holder.take() {
            out.push(Effect::Release { g });
        }
    }
    out
}

fn shuffle(rng: &mut Rng, n: usize) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        v.swap(i, j);
    }
    v
}

#[test]
fn oracle_matches_vectors() {
    let v = load("flr.json");
    for case in v["media_bind"].as_array().expect("media_bind") {
        let name = case["name"].as_str().unwrap_or("");
        let decided: Vec<Effect> = case["decided"].as_array().expect("decided").iter().map(effect_of).collect();
        for o in case["orders"].as_array().expect("orders") {
            let order: Vec<usize> = o["order"].as_array().expect("order").iter().map(|x| x.as_u64().expect("idx") as usize).collect();
            let applied: Vec<bool> = o["applied"].as_array().expect("applied").iter().map(|x| x.as_bool().expect("bool")).collect();
            let mut or = Oracle::default();
            let got: Vec<bool> = order.iter().map(|&i| or.apply(&decided[i])).collect();
            assert_eq!(got, applied, "S-sym-IMPL-FLR-MEDIA-BIND-3 oracle applied vs vector · {name} · {order:?}");
            assert_eq!(or.bound, bound_of(&o["bound"]), "S-sym-IMPL-FLR-MEDIA-BIND-3 oracle bound vs vector · {name} · {order:?}");
        }
    }
}

#[test]
fn product_matches_vectors() {
    let v = load("flr.json");
    let mut n = 0;
    for case in v["media_bind"].as_array().expect("media_bind") {
        let name = case["name"].as_str().unwrap_or("");
        let decided: Vec<Effect> = case["decided"].as_array().expect("decided").iter().map(effect_of).collect();
        let want_final = bound_of(&case["bound"]);
        for o in case["orders"].as_array().expect("orders") {
            let order: Vec<usize> = o["order"].as_array().expect("order").iter().map(|x| x.as_u64().expect("idx") as usize).collect();
            let applied: Vec<bool> = o["applied"].as_array().expect("applied").iter().map(|x| x.as_bool().expect("bool")).collect();
            let mut s = Slot::default();
            let got: Vec<bool> = order.iter().map(|&i| s.apply(&decided[i])).collect();
            assert_eq!(got, applied, "S-sym-IMPL-FLR-MEDIA-BIND-4 product applied vs vector · {name} · {order:?}");
            assert_eq!(s.bound, bound_of(&o["bound"]), "S-sym-IMPL-FLR-MEDIA-BIND-3 product bound vs vector · {name} · {order:?}");
            assert_eq!(s.bound, want_final, "S-sym-IMPL-FLR-MEDIA-BIND-5 result is the decision-order result · {name} · {order:?}");
            assert_eq!(s.speaker(), s.bound.as_ref().map(|b| b.0.as_str()), "S-sym-IMPL-FLR-MEDIA-BIND-3 speaker mirrors bound");
            n += 1;
        }
    }
    assert!(n > 100, "S-sym-IMPL-FLR-MEDIA-BIND-3 vector orders walked: {n}");
}

#[test]
fn release_does_not_clear_newer_bind() {
    let mut s = Slot::default();
    assert!(s.apply(&Effect::Bind { user: "A".into(), g: 0 }), "S-sym-IMPL-FLR-MEDIA-BIND-3 first bind applies");
    assert!(s.apply(&Effect::Bind { user: "B".into(), g: 1 }), "S-sym-IMPL-FLR-MEDIA-BIND-3 new speaker bind applies");
    let cleared = s.apply(&Effect::Release { g: 0 });
    assert!(!cleared, "S-sym-IMPL-FLR-MEDIA-BIND-3 late release of g=0 must not clear bind g=1");
    assert_eq!(s.bound, Some(("B".to_string(), 1)), "S-sym-IMPL-FLR-MEDIA-BIND-3 slot keeps the new speaker after old release lands late");
    assert_eq!(s.speaker(), Some("B"), "S-sym-IMPL-FLR-MEDIA-BIND-3 speaker stays B");
}

#[test]
fn late_old_bind_is_dropped() {
    let mut s = Slot::default();
    assert!(s.apply(&Effect::Bind { user: "B".into(), g: 1 }), "S-sym-IMPL-FLR-MEDIA-BIND-4 newer bind first");
    assert!(!s.apply(&Effect::Bind { user: "A".into(), g: 0 }), "S-sym-IMPL-FLR-MEDIA-BIND-4 old bind arriving late is dropped");
    assert_eq!(s.bound, Some(("B".to_string(), 1)), "S-sym-IMPL-FLR-MEDIA-BIND-4 old bind must not overwrite the granted one");

    let mut s = Slot::default();
    assert!(!s.apply(&Effect::Release { g: 0 }), "S-sym-IMPL-FLR-MEDIA-BIND-4 release before its bind clears nothing");
    assert!(!s.apply(&Effect::Bind { user: "A".into(), g: 0 }), "S-sym-IMPL-FLR-MEDIA-BIND-4 bind after its own release must not revive");
    assert_eq!(s.bound, None, "S-sym-IMPL-FLR-MEDIA-BIND-4 returned speaker stays unbound");

    let mut s = Slot::default();
    assert!(s.apply(&Effect::Bind { user: "A".into(), g: 3 }), "S-sym-IMPL-FLR-MEDIA-BIND-4 bind g=3");
    assert!(!s.apply(&Effect::Bind { user: "A".into(), g: 3 }), "S-sym-IMPL-FLR-MEDIA-BIND-4 same-number bind twice is not newer");
}

#[test]
fn release_clears_older_after_succession() {
    let decided = [Effect::Bind { user: "A".into(), g: 0 }, Effect::Bind { user: "B".into(), g: 1 }, Effect::Release { g: 1 }];
    let mut s = Slot::default();
    for i in [0usize, 2, 1] {
        s.apply(&decided[i]);
    }
    assert_eq!(s.bound, None, "S-sym-IMPL-FLR-MEDIA-BIND-5 release g=1 landing before bind g=1 clears the returned bind g=0 · order [0,2,1]");

    let mut s = Slot::default();
    s.apply(&Effect::Bind { user: "A".into(), g: 4 });
    assert!(s.apply(&Effect::Release { g: 9 }), "S-sym-IMPL-FLR-MEDIA-BIND-5 release with larger number clears an older bind");
    assert_eq!(s.bound, None, "S-sym-IMPL-FLR-MEDIA-BIND-5 nothing left after release g=9");
    assert!(!s.apply(&Effect::Bind { user: "C".into(), g: 7 }), "S-sym-IMPL-FLR-MEDIA-BIND-5 bind numbered below the seen max stays dropped");
}

#[test]
fn random_orders_match_oracle_and_decision_order() {
    let mut rng = Rng(0x2026_1030_b0b0_0001);
    let mut walked = 0;
    for _ in 0..400 {
        let len = 2 + rng.below(7) as usize;
        let decided = decided_seq(&mut rng, len);
        let mut reference = Oracle::default();
        for e in &decided {
            reference.apply(e);
        }
        for _ in 0..12 {
            let order = shuffle(&mut rng, decided.len());
            let mut s = Slot::default();
            let mut o = Oracle::default();
            for &i in &order {
                let a = s.apply(&decided[i]);
                let b = o.apply(&decided[i]);
                assert_eq!(a, b, "S-sym-IMPL-FLR-MEDIA-BIND-4 applied flag vs oracle · {decided:?} · {order:?}");
                assert_eq!(s.bound, o.bound, "S-sym-IMPL-FLR-MEDIA-BIND-3 bound vs oracle mid-way · {decided:?} · {order:?}");
            }
            assert_eq!(s.bound, reference.bound, "S-sym-IMPL-FLR-MEDIA-BIND-5 final bound equals decision-order result · {decided:?} · {order:?}");
            walked += 1;
        }
    }
    assert!(walked >= 4000, "S-sym-IMPL-FLR-MEDIA-BIND-3 random orders walked: {walked}");
}
