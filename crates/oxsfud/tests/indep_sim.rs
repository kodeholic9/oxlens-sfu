// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-SIM-TICK · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 독립 1층 시험(B′ 맹검) — GCC AIMD 신탁(libwebrtc AimdRateControl 문장) · 레이어 판단 순수 함수(강등 넷 · backoff · 유예 · 무너짐) · v2 프로브 실측 없음.

use oxsfud::sim::core::{Aimd, Decision, Layer, Policy, Reason, Signals, Usage, BACKOFF_START_MS, COLLAPSE_WINDOW_MS, LAYER_H_BPS, REMB_GRACE_MS};
use oxsfud::sim::Probe;
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

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(PartialEq)]
enum St {
    Hold,
    Inc,
    Dec,
}

struct OAimd {
    r: f64,
    st: St,
    last: u64,
    est: Option<f64>,
    dev: f64,
    init: bool,
}

impl OAimd {
    fn new(start: f64) -> OAimd {
        OAimd { r: start, st: St::Hold, last: 0, est: None, dev: 0.4, init: false }
    }
    fn sigma(&self, e: f64) -> f64 {
        3.0 * (self.dev * e).sqrt()
    }
    fn cap_update(&mut self, acked: f64) {
        let k = acked / 1000.0;
        let e = match self.est {
            None => k,
            Some(e) => 0.95 * e + 0.05 * k,
        };
        self.est = Some(e);
        let err = e - k;
        self.dev = (0.95 * self.dev + 0.05 * err * err / e.max(1.0)).clamp(0.4, 2.5);
    }
    fn update(&mut self, u: Usage, acked: f64, now: u64) -> u64 {
        match u {
            Usage::Normal if self.st == St::Hold => {
                self.st = St::Inc;
                self.last = now;
            }
            Usage::Overuse => self.st = St::Dec,
            Usage::Underuse => self.st = St::Hold,
            _ => {}
        }
        let mut new = self.r;
        match self.st {
            St::Inc => {
                if let Some(e) = self.est
                    && acked / 1000.0 > e + self.sigma(e)
                {
                    self.est = None;
                }
                let lim = 1.5 * acked + 10_000.0;
                if self.r < lim {
                    let el = (now - self.last) as f64 / 1000.0;
                    let inc = if self.est.is_some() {
                        let bpf = self.r / 30.0;
                        let ppf = (bpf / (8.0 * 1200.0)).ceil();
                        self.r + (bpf / ppf / (2.0 * 0.3)).max(4000.0) * el
                    } else {
                        self.r + (self.r * (1.08f64.powf(el.min(1.0)) - 1.0)).max(1000.0)
                    };
                    new = inc.min(lim);
                }
                self.last = now;
            }
            St::Dec => {
                let mut d = acked * 0.85;
                if d > 5000.0 {
                    d -= 5000.0;
                }
                if d > self.r
                    && let Some(e) = self.est
                {
                    d = 0.85 * e * 1000.0;
                }
                if d < self.r {
                    new = d;
                }
                if self.init
                    && let Some(e) = self.est
                    && acked / 1000.0 < (e - self.sigma(e)).max(0.0)
                {
                    self.est = None;
                }
                self.init = true;
                self.cap_update(acked);
                self.st = St::Hold;
                self.last = now;
            }
            St::Hold => {}
        }
        self.r = new.max(30_000.0).trunc();
        self.r as u64
    }
}

fn usage(s: &str) -> Usage {
    match s {
        "normal" => Usage::Normal,
        "overuse" => Usage::Overuse,
        _ => Usage::Underuse,
    }
}

#[test]
fn aimd_vectors_match_oracle_and_product() {
    let v = vectors("sim.json");
    let cases = v["aimd"].as_array().expect("aimd 판 묶음");
    assert!(!cases.is_empty(), "S-sym-RULE-CONC-SANS-IO-1 aimd 판이 비었다");
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let start = c["start_bps"].as_f64().unwrap();
        let (mut o, mut p) = (OAimd::new(start), Aimd::new(start));
        for st in c["steps"].as_array().unwrap() {
            let (u, a, t) = (usage(st["usage"].as_str().unwrap()), st["acked_bps"].as_f64().unwrap(), st["at_ms"].as_u64().unwrap());
            let want = st["out_bps"].as_u64().unwrap();
            assert_eq!(o.update(u, a, t), want, "S-sym-RULE-CONC-SANS-IO-1 신탁 ≠ 벡터 — {name} at {t}");
            assert_eq!(p.update(u, a, t), want, "S-sym-RULE-CONC-SANS-IO-1 제품 AIMD — {name} at {t}");
            assert_eq!(p.near_max(), st["out_near"].as_bool().unwrap(), "S-sym-RULE-CONC-SANS-IO-1 수렴 근처 = 추정 있음 — {name} at {t}");
        }
    }
}

#[test]
fn aimd_random_against_oracle() {
    let mut rng = Rng(0x00a1_3d20_26ff);
    for round in 0..300 {
        let start = 50_000.0 + rng.below(2_000_000) as f64;
        let (mut o, mut p) = (OAimd::new(start), Aimd::new(start));
        let mut t = rng.below(1_000_000);
        for step in 0..60 {
            t += 300 + rng.below(1500);
            let u = match rng.below(5) {
                0 => Usage::Overuse,
                1 => Usage::Underuse,
                _ => Usage::Normal,
            };
            let acked = 20_000.0 + rng.below(3_000_000) as f64;
            let a = o.update(u, acked, t);
            let b = p.update(u, acked, t);
            assert!(a.abs_diff(b) <= 1, "S-sym-RULE-CONC-SANS-IO-1 AIMD 신탁 {a} ≠ 제품 {b} (round {round} step {step})");
            assert!(b >= 30_000, "S-sym-RULE-SIM-TICK-1 바닥 30 kbps 아래 0");
            if a != b {
                break;
            }
        }
    }
}

fn sig(now: u64, est: Option<f64>, loss: Option<f64>, nack: f64, drops: u64) -> Signals {
    Signals { estimate_bps: est, estimate_at_ms: now, loss_pct: loss, loss_at_ms: now, nack_per_s: nack, drops, overusing: false, streams: 1 }
}

fn good(now: u64) -> Signals {
    sig(now, Some(20_000_000.0), Some(0.0), 0.0, 0)
}

fn bad_loss(now: u64) -> Signals {
    sig(now, Some(20_000_000.0), Some(30.0), 0.0, 0)
}

fn reach_high(p: &mut Policy, from: u64) -> u64 {
    let mut t = from;
    for _ in 0..200 {
        if p.layer == Layer::High {
            return t;
        }
        t += 1000;
        p.tick(t, &good(t));
    }
    panic!("깨끗한 신호로 High 에 닿지 못했다");
}

fn demote_by_loss(p: &mut Policy, from: u64) -> u64 {
    let mut t = from;
    for _ in 0..10 {
        t += 1000;
        if let Decision::Demote(_) = p.tick(t, &bad_loss(t)) {
            return t;
        }
    }
    panic!("손실 30% 로 강등이 안 왔다");
}

fn next_promote(p: &mut Policy, from: u64) -> u64 {
    let mut t = from;
    for _ in 0..200 {
        t += 1000;
        if p.tick(t, &good(t)) == Decision::Promote {
            return t;
        }
    }
    panic!("승격이 안 왔다");
}

#[test]
fn demote_needs_two_consecutive_bad_ticks() {
    let mut p = Policy::new(0);
    let mut t = reach_high(&mut p, 0) + REMB_GRACE_MS;
    t += 1000;
    assert_eq!(p.tick(t, &bad_loss(t)), Decision::Stay, "S-sym-RULE-SIM-TICK-1 나쁨 한 번은 강등 아님");
    t += 1000;
    assert_eq!(p.tick(t, &good(t)), Decision::Stay, "S-sym-RULE-SIM-TICK-1 사이에 깨끗하면 연속이 끊긴다");
    t += 1000;
    assert_eq!(p.tick(t, &bad_loss(t)), Decision::Stay, "S-sym-RULE-SIM-TICK-1 다시 한 번");
    t += 1000;
    assert_eq!(p.tick(t, &bad_loss(t)), Decision::Demote(Reason::Loss), "S-sym-RULE-SIM-TICK-1 2연속 → 강등 loss");
}

#[test]
fn demote_reasons_each_and_in_order() {
    let cases: [(f64, f64, u64, Reason, &str); 3] = [
        (0.0, 25.0, 0, Reason::Nack, "nack > 20/s"),
        (0.0, 0.0, 3, Reason::Drop, "drop > 0"),
        (9.0, 25.0, 3, Reason::Loss, "loss · nack · drop 함께면 loss 가 먼저"),
    ];
    for (loss, nack, drops, want, what) in cases {
        let mut p = Policy::new(0);
        let mut t = reach_high(&mut p, 0) + REMB_GRACE_MS + 1000;
        let mut got = Decision::Stay;
        for _ in 0..2 {
            t += 1000;
            got = p.tick(t, &sig(t, Some(20_000_000.0), Some(loss), nack, drops));
        }
        assert_eq!(got, Decision::Demote(want), "S-sym-RULE-SIM-TICK-1 강등 사유 — {what}");
    }
    let mut p = Policy::new(0);
    let mut t = reach_high(&mut p, 0) + REMB_GRACE_MS + 1000;
    let short = 1.2 * LAYER_H_BPS * 0.5;
    let mut got = Decision::Stay;
    for _ in 0..2 {
        t += 1000;
        got = p.tick(t, &sig(t, Some(short), Some(9.0), 0.0, 0));
    }
    assert_eq!(got, Decision::Demote(Reason::Remb), "S-sym-RULE-SIM-TICK-1 remb 가 사유 순서 첫째");
}

#[test]
fn nack_threshold_is_strictly_above_twenty() {
    let mut p = Policy::new(0);
    let mut t = reach_high(&mut p, 0) + REMB_GRACE_MS + 1000;
    for _ in 0..4 {
        t += 1000;
        assert_eq!(p.tick(t, &sig(t, Some(20_000_000.0), Some(0.0), 20.0, 0)), Decision::Stay, "S-sym-RULE-SIM-TICK-4 nack 20/s 는 강등 아님(> 20)");
    }
}

#[test]
fn remb_skipped_within_grace_but_nack_demotes_within_grace() {
    let mut p = Policy::new(0);
    let mut t = 0;
    while p.layer != Layer::Low {
        t += 1000;
        p.tick(t, &bad_loss(t));
        assert!(t < 60_000, "Low 로 못 내려갔다");
    }
    let hi = next_promote(&mut p, t);
    t = hi;
    let short = 1.2 * LAYER_H_BPS * 0.5;
    for _ in 0..3 {
        t += 1000;
        assert_eq!(p.tick(t, &sig(t, Some(short), Some(0.0), 0.0, 0)), Decision::Stay, "S-sym-RULE-SIM-TICK-1 승격 유예 10s 중 remb 강등 skip");
    }
    let mut p = Policy::new(0);
    let mut t = 0;
    while p.layer != Layer::Low {
        t += 1000;
        p.tick(t, &bad_loss(t));
    }
    let hi = next_promote(&mut p, t);
    t = hi;
    let mut got = Decision::Stay;
    for _ in 0..2 {
        t += 1000;
        got = p.tick(t, &sig(t, Some(20_000_000.0), Some(0.0), 40.0, 0));
    }
    assert!(t - hi < REMB_GRACE_MS, "시험 형상 — 유예 안");
    assert_eq!(got, Decision::Demote(Reason::Nack), "S-sym-RULE-SIM-TICK-4 유예 안에서도 손실 없는 NACK 몰기는 강등");
    assert_eq!(p.layer, Layer::Low, "S-sym-RULE-SIM-TICK-4 l 로 내려간다");
}

#[test]
fn backoff_doubles_only_on_collapse_not_on_capacity_shortage() {
    let mut p = Policy::new(0);
    let t = reach_high(&mut p, 0) + REMB_GRACE_MS;
    let d0 = demote_by_loss(&mut p, t);
    let p1 = next_promote(&mut p, d0);
    assert!(p1 - d0 >= BACKOFF_START_MS && p1 - d0 <= BACKOFF_START_MS + 1000, "S-sym-RULE-SIM-TICK-1 첫 backoff 15s — 강등 {d0} 승격 {p1}");
    let d1 = demote_by_loss(&mut p, p1);
    assert!(d1 - p1 < COLLAPSE_WINDOW_MS, "시험 형상 — 무너짐 창 안 재강등");
    let p2 = next_promote(&mut p, d1);
    assert!(p2 - d1 >= 2 * BACKOFF_START_MS && p2 - d1 <= 2 * BACKOFF_START_MS + 1000, "S-sym-RULE-SIM-TICK-1 무너짐 → backoff ×2 = 30s — 강등 {d1} 승격 {p2}");

    let mut p = Policy::new(0);
    let t = reach_high(&mut p, 0) + REMB_GRACE_MS;
    let d0 = demote_by_loss(&mut p, t);
    let p1 = next_promote(&mut p, d0);
    assert!(p1 - d0 <= BACKOFF_START_MS + 1000, "S-sym-RULE-SIM-TICK-1 첫 backoff");
    let mut t = p1 + REMB_GRACE_MS;
    let short = 1.2 * LAYER_H_BPS * 0.5;
    let mut d1 = 0;
    for _ in 0..5 {
        t += 1000;
        if let Decision::Demote(r) = p.tick(t, &sig(t, Some(short), Some(0.0), 0.0, 0)) {
            assert_eq!(r, Reason::Remb, "S-sym-RULE-SIM-TICK-1 유예 뒤 remb 강등");
            d1 = t;
            break;
        }
    }
    assert!(d1 > 0, "S-sym-RULE-SIM-TICK-1 유예 뒤 용량 부족은 강등");
    let p2 = next_promote(&mut p, d1);
    assert!(p2 - d1 <= BACKOFF_START_MS + 1000, "S-sym-RULE-SIM-TICK-1 용량 부족(유예 뒤 remb)은 backoff 배증 0 — 강등 {d1} 승격 {p2}");
}

#[test]
fn backoff_caps_at_sixty_seconds() {
    let mut p = Policy::new(0);
    let t = reach_high(&mut p, 0) + REMB_GRACE_MS;
    let mut t = demote_by_loss(&mut p, t);
    let mut gaps = vec![];
    for _ in 0..5 {
        let pr = next_promote(&mut p, t);
        let d = demote_by_loss(&mut p, pr);
        let pr2 = next_promote(&mut p, d);
        gaps.push(pr2 - d);
        t = demote_by_loss(&mut p, pr2);
    }
    assert!(gaps.iter().all(|g| *g <= 60_000 + 1000), "S-sym-RULE-SIM-TICK-1 backoff 상한 60s — {gaps:?}");
    assert!(gaps.iter().any(|g| *g >= 60_000), "S-sym-RULE-SIM-TICK-1 무너짐 되풀이면 상한까지 오른다 — {gaps:?}");
}

#[test]
fn overusing_is_not_clean_so_no_promote() {
    let mut p = Policy::new(0);
    let t = reach_high(&mut p, 0) + REMB_GRACE_MS;
    let mut t = demote_by_loss(&mut p, t);
    for _ in 0..80 {
        t += 1000;
        let mut s = good(t);
        s.overusing = true;
        assert_ne!(p.tick(t, &s), Decision::Promote, "S-sym-RULE-SIM-TICK-3 과사용 중이면 승격 0");
    }
}

#[test]
fn probe_without_measurement_is_none_not_zero() {
    let pr = Probe { start_ms: 0, until_ms: 2500, judge_ms: 4000, rate_bps: 2_000_000.0, demand_bps: 1_980_000.0, seqs: vec![1, 2, 3], acked_bytes: 0, first_us: None, last_us: None };
    assert_eq!(pr.measured_bps(), None, "S-sym-RULE-SIM-TICK-3 못 잰 것은 None(0 으로 안 채운다)");
    let pr = Probe { acked_bytes: 250_000, first_us: Some(1_000_000), last_us: Some(2_000_000), ..pr };
    let m = pr.measured_bps().expect("S-sym-RULE-SIM-TICK-3 잰 것은 Some");
    assert!((m - 2_000_000.0).abs() < 1.0, "S-sym-RULE-SIM-TICK-3 실측 = 받은 바이트 ÷ 도착 구간 — {m}");
}
