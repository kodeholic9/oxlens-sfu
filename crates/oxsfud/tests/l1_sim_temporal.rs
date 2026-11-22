// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SIM-SWITCH · 정:IMPL-FWD-SOURCE-SWITCH · 정:IMPL-PUB-RID-SPATIAL · model: claude-opus-5-5
//! 1층 — 시간 층 고르기(context 202611/20261122a S2). 벡터 sim.json `temporal` · `tl_read` · 줄 상태 묶음 왕복 · 상한 합치기 · 고르기 + 재기록기 뺀 런을 이은 흐름(목표를 오가도 나가는 seq · picture ID 가 연속 · TID0 는 늘 나감 · Y 없이 오르지 않음).

use std::path::{Path, PathBuf};

use oxsfud::fwd::core::{M15, Rewriter, vp8_pic};
use oxsfud::sim::temporal::{TL_NONE, TL_TOP, TemporalSel, Vp8Tl, goal, vp8_tl};
use serde_json::Value;

fn spec() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec")
}

fn load(name: &str) -> Value {
    let p = spec().join("server/out/vectors").join(name);
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("벡터 없음 {}: {e}", p.display()));
    serde_json::from_str(&t).expect("벡터 JSON")
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0)).collect()
}

fn desc(pid: u16, tl0: u8, tid: u8, y: bool) -> Vec<u8> {
    vec![0x90, 0xe0, 0x80 | ((pid >> 8) & 0x7f) as u8, (pid & 0xff) as u8, tl0, (tid << 6) | if y { 0x20 } else { 0 }, 0x9d]
}

#[test]
fn temporal_vectors_match_product() {
    let v = load("sim.json");
    assert_eq!(v["tl_top"].as_u64(), Some(TL_TOP as u64), "S-sym-IMPL-SIM-SWITCH-5 TL_TOP equals the source");
    for r in v["tl_read"].as_array().expect("tl_read") {
        let got = vp8_tl(&hex(r["payload"].as_str().expect("payload")));
        let want = r["tl"].as_object().map(|o| Vp8Tl { tid: o["tid"].as_u64().expect("tid") as u8, y: o["y"].as_bool().expect("y"), pid: o["pid"].as_u64().expect("pid") as u16 });
        assert_eq!(got, want, "S-sym-IMPL-SIM-SWITCH-5 vp8_tl · {}", r["name"]);
    }
    let cases = v["temporal"].as_array().expect("temporal");
    assert!(!cases.is_empty(), "판 묶음 temporal 비었다");
    for c in cases {
        let name = c["name"].as_str().unwrap_or("");
        let mut sel = TemporalSel::default();
        for (i, st) in c["steps"].as_array().expect("steps").iter().enumerate() {
            let tl = vp8_tl(&hex(st["payload"].as_str().expect("payload"))).expect("tl");
            let (send, why) = sel.step(tl, st["key"].as_bool().expect("key"), st["target"].as_u64().expect("target") as u8);
            assert_eq!(send, st["send"].as_bool().expect("send"), "S-sym-IMPL-SIM-SWITCH-5 send · {name} · step {i}");
            assert_eq!(why.map(|w| w.name()), st["why"].as_str(), "S-sym-IMPL-SIM-SWITCH-5 why · {name} · step {i}");
            assert_eq!(sel.current as u64, st["current"].as_u64().expect("current"), "S-sym-IMPL-SIM-SWITCH-5 current · {name} · step {i}");
        }
    }
}

#[test]
fn state_packs_and_goal_clips() {
    for current in 0..=TL_TOP {
        for keep in [false, true] {
            for pid in [None, Some(0u16), Some(1), Some(M15 - 1), Some(12345)] {
                let s = TemporalSel { current, pid, keep };
                assert_eq!(TemporalSel::unpack(s.pack()), s, "S-sym-IMPL-SIM-SWITCH-5 pack round trip");
            }
        }
    }
    assert_eq!(goal(TL_NONE, TL_TOP), TL_TOP, "S-sym-IMPL-SIM-SWITCH-5 no manual cap = top");
    assert_eq!(goal(1, TL_TOP), 1, "S-sym-IMPL-SIM-SWITCH-5 manual cap");
    assert_eq!(goal(TL_NONE, 0), 0, "S-sym-IMPL-SIM-SWITCH-5 auto cap");
    assert_eq!(goal(2, 1), 1, "S-sym-IMPL-SIM-SWITCH-5 min of the two");
    assert_eq!(goal(9, 9), TL_TOP, "S-sym-IMPL-SIM-SWITCH-5 clipped to TL_TOP");
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
        self.next() % n.max(1)
    }
}

#[test]
fn selection_with_rewriter_keeps_numbers_contiguous() {
    let pattern = [0u8, 2, 1, 2];
    let ys = [false, true, true, false];
    for seed in 1..=50u64 {
        let mut r = Rng(0x243f_6a88_85a3_08d3 ^ seed);
        let mut sel = TemporalSel::default();
        let mut rw = Rewriter::new(90_000);
        let mut seq = r.next() as u16;
        let mut pid = (r.next() as u16) % M15;
        let mut tl0 = r.next() as u8;
        let mut target = TL_TOP;
        let mut last: Option<(u16, u16)> = None;
        let mut sent_by_tid = [0u32; 3];
        for f in 0..1_500u32 {
            if r.below(25) == 0 {
                target = r.below(3) as u8;
            }
            let k = (f % 4) as usize;
            let key = f % 300 == 0;
            let tid = if key { 0 } else { pattern[k] };
            if tid == 0 && f > 0 {
                tl0 = tl0.wrapping_add(1);
            }
            let y = key || ys[k];
            for _ in 0..1 + r.below(3) {
                let pay = desc(pid, tl0, tid, y);
                let tl = vp8_tl(&pay).expect("tl");
                let pic = vp8_pic(&pay);
                let (send, _) = sel.step(tl, key, target);
                if tid == 0 {
                    assert!(send, "S-sym-IMPL-SIM-SWITCH-5 TID0 is always sent · seed {seed} · frame {f}");
                }
                assert!(sel.current <= target, "S-sym-IMPL-SIM-SWITCH-5 current never above target after the frame head · seed {seed} · frame {f}");
                if !send {
                    rw.skip(1, seq, pic.as_ref());
                } else {
                    let m = rw.map_pic(1, seq, f.wrapping_mul(3000), f as u64, 1, pic.as_ref()).expect("forwarded");
                    if let (Some((ls, lp)), Some(p)) = (last, m.pid) {
                        assert_eq!(m.seq, ls.wrapping_add(1), "S-sym-IMPL-FWD-SOURCE-SWITCH-9① out seq +1 · seed {seed} · frame {f}");
                        let d = p.wrapping_sub(lp) % M15;
                        assert!(d == 0 || d == 1, "S-sym-IMPL-FWD-SOURCE-SWITCH-10① out pid same or +1 · seed {seed} · frame {f} · {lp} → {p}");
                    }
                    last = Some((m.seq, m.pid.unwrap_or(0)));
                    sent_by_tid[tid as usize] += 1;
                }
                seq = seq.wrapping_add(1);
            }
            pid = (pid + 1) % M15;
        }
        assert!(sent_by_tid[0] > 0, "S-sym-IMPL-SIM-SWITCH-5 base layer flows · seed {seed}");
    }
}

#[test]
fn never_rises_without_layer_sync() {
    let mut sel = TemporalSel::default();
    let _ = sel.step(Vp8Tl { tid: 0, y: false, pid: 10 }, true, 0);
    for p in 11..400u16 {
        let tid = [2u8, 1, 2, 0][(p % 4) as usize];
        let (send, _) = sel.step(Vp8Tl { tid, y: false, pid: p }, false, TL_TOP);
        assert_eq!(sel.current, 0, "S-sym-IMPL-SIM-SWITCH-6 no rise without Y · pid {p}");
        assert_eq!(send, tid == 0, "S-sym-IMPL-SIM-SWITCH-6 only TID0 while waiting for Y · pid {p}");
    }
    let (send, _) = sel.step(Vp8Tl { tid: 1, y: true, pid: 400 }, false, TL_TOP);
    assert!(send && sel.current == 1, "S-sym-IMPL-SIM-SWITCH-6 Y frame lifts one step");
}
