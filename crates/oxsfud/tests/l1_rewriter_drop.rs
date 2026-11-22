// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-FWD-SOURCE-SWITCH · 정:PROP-FWD-EGRESS-SEQ-RISES · 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS · model: claude-opus-5-5
//! 1층 — 재기록기의 한 출처 안 뺀 런(context 202611/20261122a S1 · 결재 「rewrite 가 망가지면 안 된다」). 벡터 fwd.json `drop` · 뺀 것이 없을 때 옛 재기록기(얼린 사본)와 반환값 · 상태 차분 · 뺀 런 성질(연속 · 유일 · NACK 되돌림 · 순서 섞기 · 전환 뒤 역행 없음 · 넘침 · 감김).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use oxsfud::fwd::core::{BOUNDARY_WINDOW, DROP_RUNS, M15, Rewriter, Vp8Pic, serial_after, serial_before, vp8_pic};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OldLast {
    seq: u16,
    ts: u32,
    at_ms: u64,
    covers: u32,
}

#[derive(Debug, Clone, Default)]
struct Old {
    clock: u32,
    src: Option<u64>,
    seq_off: u16,
    ts_off: u32,
    last: Option<OldLast>,
    boundary: Option<u16>,
    pid_off: u16,
    tl0_off: u8,
    last_pid: Option<u16>,
    last_tl0: Option<u8>,
}

type Out = (u16, u32, bool, Option<u16>, Option<u8>);

impl Old {
    fn new(clock: u32) -> Old {
        Old { clock, ..Default::default() }
    }

    fn pic_out(&self, pic: Option<&Vp8Pic>) -> (Option<u16>, Option<u8>) {
        let pid = pic.and_then(|d| d.pid).map(|v| v.wrapping_add(self.pid_off) % M15);
        let tl0 = pic.and_then(|d| d.tl0).map(|v| v.wrapping_add(self.tl0_off));
        (pid, tl0)
    }

    fn map_pic(&mut self, src: u64, seq: u16, ts: u32, at_ms: u64, covers: u32, pic: Option<&Vp8Pic>) -> Option<Out> {
        let mut switched = false;
        if self.src != Some(src) {
            self.pid_off = match (self.last_pid, pic.and_then(|d| d.pid)) {
                (Some(lp), Some(p)) => lp.wrapping_add(1).wrapping_sub(p) % M15,
                _ => 0,
            };
            self.tl0_off = match (self.last_tl0, pic.and_then(|d| d.tl0)) {
                (Some(lx), Some(x)) => lx.wrapping_add(1).wrapping_sub(x),
                _ => 0,
            };
            switched = self.src.is_some() || self.last.is_some();
            match self.last {
                None => {
                    self.seq_off = 0;
                    self.ts_off = 0;
                }
                Some(l) => {
                    let elapsed = at_ms.saturating_sub(l.at_ms).max(1);
                    let ticks = (elapsed as u128 * self.clock as u128 / 1000) as u64;
                    let tick = (l.covers.max(1) as u64).max(ticks) as u32;
                    self.seq_off = l.seq.wrapping_add(1).wrapping_sub(seq);
                    self.ts_off = l.ts.wrapping_add(tick).wrapping_sub(ts);
                }
            }
            self.src = Some(src);
            self.boundary = Some(seq.wrapping_add(self.seq_off));
        }
        let out_seq = seq.wrapping_add(self.seq_off);
        let out_ts = ts.wrapping_add(self.ts_off);
        if let Some(b) = self.boundary
            && serial_before(out_seq, b)
        {
            return None;
        }
        let advance = match self.last {
            None => true,
            Some(l) => switched || serial_after(out_seq, l.seq),
        };
        let (pid, tl0) = self.pic_out(pic);
        if advance {
            self.last = Some(OldLast { seq: out_seq, ts: out_ts, at_ms, covers });
            if pic.is_some() {
                self.last_pid = pid.or(self.last_pid);
                self.last_tl0 = tl0.or(self.last_tl0);
            }
        }
        if let (Some(b), Some(l)) = (self.boundary, self.last)
            && l.seq.wrapping_sub(b) >= BOUNDARY_WINDOW
        {
            self.boundary = None;
        }
        Some((out_seq, out_ts, switched, pid, tl0))
    }

    fn unmap_nack(&self, out_seq: u16) -> Option<u16> {
        self.src?;
        if let Some(b) = self.boundary
            && serial_before(out_seq, b)
        {
            return None;
        }
        Some(out_seq.wrapping_sub(self.seq_off))
    }
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

fn desc(pid: u16, tl0: Option<u8>, wide: bool) -> Vec<u8> {
    let x = 0x80 | if tl0.is_some() { 0x40 } else { 0 };
    let mut b = vec![0x90, x];
    if wide {
        b.push(0x80 | ((pid >> 8) & 0x7f) as u8);
        b.push((pid & 0xff) as u8);
    } else {
        b.push((pid & 0x7f) as u8);
    }
    if let Some(t) = tl0 {
        b.push(t);
    }
    b.push(0x9d);
    b
}

fn same_state(new: &Rewriter, old: &Old, at: &str) {
    assert_eq!(new.seq_off, old.seq_off, "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ seq_off · {at}");
    assert_eq!(new.ts_off, old.ts_off, "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ ts_off · {at}");
    assert_eq!(new.boundary, old.boundary, "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ boundary · {at}");
    assert_eq!(new.src, old.src, "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ src · {at}");
    assert_eq!(new.pid_off, old.pid_off, "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ pid_off · {at}");
    assert_eq!(new.tl0_off, old.tl0_off, "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ tl0_off · {at}");
    assert_eq!(new.last_pid, old.last_pid, "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ last_pid · {at}");
    assert_eq!(new.last_tl0, old.last_tl0, "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ last_tl0 · {at}");
    assert_eq!(new.last.map(|l| (l.seq, l.ts, l.at_ms, l.covers)), old.last.map(|l| (l.seq, l.ts, l.at_ms, l.covers)), "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ last · {at}");
    assert!(new.runs.is_empty(), "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ no runs without skip · {at}");
}

#[test]
fn drop_vectors_match_product() {
    let v = load("fwd.json");
    assert_eq!(v["drop_runs"].as_u64(), Some(DROP_RUNS as u64), "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑤ DROP_RUNS equals the source");
    let cases = v["drop"].as_array().expect("drop");
    assert!(!cases.is_empty(), "판 묶음 drop 비었다");
    for case in cases {
        let name = case["name"].as_str().unwrap_or("");
        let mut rw = Rewriter::new(90_000);
        let mut by_in: HashMap<u16, Vec<u8>> = HashMap::new();
        for st in case["steps"].as_array().expect("steps") {
            let p = hex(st["payload"].as_str().expect("payload"));
            let pic = vp8_pic(&p);
            let src = st["src"].as_u64().expect("src");
            let seq = st["seq"].as_u64().expect("seq") as u16;
            by_in.insert(seq, p.clone());
            if st["op"] == "skip" {
                rw.skip(src, seq, pic.as_ref());
                assert_eq!(rw.runs.len() as u64, st["runs"].as_u64().expect("runs"), "S-sym-IMPL-FWD-SOURCE-SWITCH-9 runs after skip · {name} · {seq}");
                continue;
            }
            let m = rw.map_pic(src, seq, st["ts"].as_u64().expect("ts") as u32, st["at_ms"].as_u64().expect("at"), 1, pic.as_ref());
            assert_eq!(m.map(|x| x.seq as u64), st["out_seq"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-9① out_seq · {name} · {seq}");
            assert_eq!(m.map(|x| x.ts as u64), st["out_ts"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-9 out_ts · {name} · {seq}");
            assert_eq!(m.and_then(|x| x.pid).map(|x| x as u64), st["out_pid"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-10① out_pid · {name} · {seq}");
            assert_eq!(m.and_then(|x| x.tl0).map(|x| x as u64), st["out_tl0"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-10 out_tl0 · {name} · {seq}");
        }
        for n in case["nack"].as_array().expect("nack") {
            let out = n["out_seq"].as_u64().expect("out") as u16;
            let back = rw.unmap_nack(out);
            assert_eq!(back.map(|x| x as u64), n["in_seq"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-9③⑤ unmap_nack · {name} · {out}");
            let pid = back.and_then(|b| by_in.get(&b)).and_then(|p| vp8_pic(p)).and_then(|d| rw.pic_out(Some(&d)).0);
            assert_eq!(pid.map(|x| x as u64), n["out_pid"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-10② retransmit pid · {name} · {out}");
        }
        let runs: Vec<[u64; 4]> = rw.runs.iter().map(|r| [r.seq as u64, r.seq_len as u64, r.pid as u64, r.pid_len as u64]).collect();
        let want: Vec<[u64; 4]> = case["runs"]
            .as_array()
            .expect("runs")
            .iter()
            .map(|r| {
                let a = r.as_array().expect("run");
                [0, 1, 2, 3].map(|i| a[i].as_u64().expect("run field"))
            })
            .collect();
        assert_eq!(runs, want, "S-sym-IMPL-FWD-SOURCE-SWITCH-9 final runs · {name}");
        assert_eq!(rw.seq_off as u64, case["seq_off"].as_u64().expect("seq_off"), "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑤ seq_off · {name}");
        assert_eq!(rw.pid_off as u64, case["pid_off"].as_u64().expect("pid_off"), "S-sym-IMPL-FWD-SOURCE-SWITCH-10 pid_off · {name}");
        assert_eq!(rw.boundary.map(|b| b as u64), case["boundary"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑤ boundary · {name}");
    }
}

#[test]
fn no_skip_matches_frozen_rewriter() {
    for seed in 1..=40u64 {
        let mut r = Rng(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0x2545_f491_4f6c_dd1d));
        let clock = if seed % 2 == 0 { 48_000 } else { 90_000 };
        let mut new = Rewriter::new(clock);
        let mut old = Old::new(clock);
        let mut src = 1u64;
        let mut seqs: HashMap<u64, u16> = HashMap::new();
        let mut pids: HashMap<u64, u16> = HashMap::new();
        let mut ts = r.next() as u32;
        let mut at = 1_000u64;
        let wide = seed % 3 != 0;
        let with_tl0 = seed % 5 != 0;
        let vp8 = seed % 7 != 0;
        for i in 0..6_000u32 {
            if r.below(40) == 0 {
                src = 1 + r.below(4);
            }
            let fresh_seq = r.next() as u16;
            let cur = seqs.get(&src).copied().unwrap_or(fresh_seq);
            let jump = match r.below(30) {
                0 => 0u16.wrapping_sub(1 + r.below(3) as u16),
                1 => 2 + r.below(5) as u16,
                _ => 1,
            };
            let seq = cur.wrapping_add(jump);
            seqs.insert(src, seq);
            let fresh_pid = (r.next() as u16) % M15;
            let pcur = pids.get(&src).copied().unwrap_or(fresh_pid);
            let pid = if r.below(3) == 0 { (pcur + 1) % M15 } else { pcur };
            pids.insert(src, pid);
            ts = ts.wrapping_add(r.below(4000) as u32);
            at += r.below(40);
            let pay = desc(pid, with_tl0.then_some((pid & 0xff) as u8), wide);
            let pic = if vp8 { vp8_pic(&pay) } else { None };
            let cov = 1 + r.below(2000) as u32;
            let a = new.map_pic(src, seq, ts, at, cov, pic.as_ref()).map(|m| (m.seq, m.ts, m.switched, m.pid, m.tl0));
            let b = old.map_pic(src, seq, ts, at, cov, pic.as_ref());
            let tag = format!("seed {seed} step {i}");
            assert_eq!(a, b, "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ map_pic · {tag}");
            same_state(&new, &old, &tag);
            let probe = r.next() as u16;
            assert_eq!(new.unmap_nack(probe), old.unmap_nack(probe), "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ unmap_nack random · {tag}");
            if let Some(l) = old.last {
                let near = l.seq.wrapping_sub(r.below(300) as u16);
                assert_eq!(new.unmap_nack(near), old.unmap_nack(near), "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ unmap_nack near · {tag}");
            }
            let other = desc((r.next() as u16) % M15, Some(r.next() as u8), r.below(2) == 0);
            let od = vp8_pic(&other);
            assert_eq!(new.pic_out(od.as_ref()), old.pic_out(od.as_ref()), "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ pic_out · {tag}");
            let sr = r.next() as u32;
            assert_eq!(new.translate_sr_ts(sr), sr.wrapping_add(old.ts_off), "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑥ translate_sr_ts · {tag}");
        }
    }
}

struct Pkt {
    seq: u16,
    pid: u16,
    tl0: u8,
    tid: u8,
    frame: u32,
}

fn stream(r: &mut Rng, seq0: u16, pid0: u16, frames: u32) -> Vec<Pkt> {
    let pattern = [0u8, 2, 1, 2];
    let mut out = Vec::new();
    let (mut seq, mut pid, mut tl0) = (seq0, pid0 % M15, r.next() as u8);
    for f in 0..frames {
        let tid = pattern[(f % 4) as usize];
        if tid == 0 && f > 0 {
            tl0 = tl0.wrapping_add(1);
        }
        for _ in 0..1 + r.below(3) {
            out.push(Pkt { seq, pid, tl0, tid, frame: f });
            seq = seq.wrapping_add(1);
        }
        pid = (pid + 1) % M15;
    }
    out
}

fn targets(r: &mut Rng, frames: u32) -> Vec<u8> {
    let mut t = Vec::with_capacity(frames as usize);
    let mut cur = r.below(3) as u8;
    for _ in 0..frames {
        if r.below(20) == 0 {
            cur = r.below(3) as u8;
        }
        t.push(cur);
    }
    t
}

fn run(rw: &mut Rewriter, src: u64, pkts: &[&Pkt], target: &[u8], at0: u64) -> HashMap<u16, (u16, Option<u16>)> {
    let mut got = HashMap::new();
    for (i, p) in pkts.iter().enumerate() {
        let pay = desc(p.pid, Some(p.tl0), true);
        let pic = vp8_pic(&pay);
        if p.tid > target[p.frame as usize] {
            rw.skip(src, p.seq, pic.as_ref());
            assert!(rw.runs.len() <= DROP_RUNS, "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑤ runs bounded");
            continue;
        }
        if let Some(m) = rw.map_pic(src, p.seq, 3000u32.wrapping_mul(p.frame), at0 + i as u64, 1, pic.as_ref()) {
            got.insert(p.seq, (m.seq, m.pid));
        }
    }
    got
}

#[test]
fn drops_close_holes_and_nack_returns_input() {
    for seed in 1..=60u64 {
        let mut r = Rng(0xd1b5_4a32_d192_ed03 ^ seed.wrapping_mul(0x9e37_79b9));
        let frames = 200 + r.below(400) as u32;
        let seq0 = if seed % 4 == 0 { 65_500 } else { r.next() as u16 };
        let pid0 = if seed % 5 == 0 { M15 - 30 } else { r.next() as u16 };
        let pkts = stream(&mut r, seq0, pid0, frames);
        let target = targets(&mut r, frames);
        let mut rw = Rewriter::new(90_000);
        let order: Vec<&Pkt> = pkts.iter().collect();
        let got = run(&mut rw, 1, &order, &target, 0);
        let mut sent: Vec<(u16, u16, Option<u16>, u32)> = order.iter().filter_map(|p| got.get(&p.seq).map(|&(o, pid)| (p.seq, o, pid, p.frame))).collect();
        sent.sort_by_key(|x| x.1.wrapping_sub(seq0));
        for w in sent.windows(2) {
            assert_eq!(w[1].1, w[0].1.wrapping_add(1), "S-sym-IMPL-FWD-SOURCE-SWITCH-9① out_seq +1 without upstream loss · seed {seed}");
            let step = w[1].2.zip(w[0].2).map(|(b, a)| b.wrapping_sub(a) % M15);
            let want = if w[1].3 == w[0].3 { 0 } else { 1 };
            assert_eq!(step, Some(want), "S-sym-IMPL-FWD-SOURCE-SWITCH-10① out pid per forwarded frame · seed {seed}");
        }
        let outs: HashSet<u16> = sent.iter().map(|x| x.1).collect();
        assert_eq!(outs.len(), sent.len(), "S-sym-IMPL-FWD-SOURCE-SWITCH-9② distinct inputs never share an out · seed {seed}");
        let skipped: HashSet<u16> = pkts.iter().filter(|p| p.tid > target[p.frame as usize]).map(|p| p.seq).collect();
        for &(inp, out, _, _) in &sent {
            match rw.unmap_nack(out) {
                Some(back) => {
                    assert_eq!(back, inp, "S-sym-IMPL-FWD-SOURCE-SWITCH-9③ unmap(map(s)) == s · seed {seed} · out {out}");
                    assert!(!skipped.contains(&back), "S-sym-IMPL-FWD-SOURCE-SWITCH-9③ unmap never yields a skipped seq · seed {seed}");
                }
                None => assert!(rw.boundary.is_some_and(|b| serial_before(out, b)), "S-sym-IMPL-FWD-SOURCE-SWITCH-9⑤ only folded fronts refuse NACK · seed {seed} · out {out}"),
            }
        }
    }
}

#[test]
fn reordered_forwarded_packets_keep_their_numbers() {
    for seed in 1..=40u64 {
        let mut r = Rng(0x6a09_e667_f3bc_c909 ^ seed);
        let frames = 300;
        let s0 = r.next() as u16;
        let p0 = r.next() as u16;
        let pkts = stream(&mut r, s0, p0, frames);
        let target = targets(&mut r, frames);
        let mut a = Rewriter::new(90_000);
        let straight: Vec<&Pkt> = pkts.iter().collect();
        let first = run(&mut a, 1, &straight, &target, 0);
        let mut mixed: Vec<&Pkt> = pkts.iter().collect();
        let fwd = |p: &Pkt| p.tid <= target[p.frame as usize];
        let mut i = 1;
        while i + 1 < mixed.len() {
            if r.below(4) == 0 && fwd(mixed[i]) && fwd(mixed[i + 1]) {
                mixed.swap(i, i + 1);
                i += 2;
            } else {
                i += 1;
            }
        }
        let mut b = Rewriter::new(90_000);
        let second = run(&mut b, 1, &mixed, &target, 0);
        for (seq, (o, _)) in &first {
            assert_eq!(second.get(seq).map(|x| x.0), Some(*o), "S-sym-IMPL-FWD-SOURCE-SWITCH-9④ same number when forwarded packets arrive out of order · seed {seed} · in {seq}");
        }
    }
}

#[test]
fn late_skip_is_not_recorded_and_upstream_loss_keeps_numbers() {
    let mut rw = Rewriter::new(90_000);
    let p = |pid: u16| desc(pid, Some(1), true);
    let m = |rw: &mut Rewriter, seq: u16, pid: u16| rw.map_pic(1, seq, 0, 0, 1, vp8_pic(&p(pid)).as_ref()).map(|x| x.seq);
    assert_eq!(m(&mut rw, 50, 100), Some(50));
    assert_eq!(m(&mut rw, 52, 102), Some(52));
    rw.skip(1, 51, vp8_pic(&p(101)).as_ref());
    assert!(rw.runs.is_empty(), "S-sym-IMPL-FWD-SOURCE-SWITCH-9 late skip is not recorded");
    rw.skip(1, 53, vp8_pic(&p(103)).as_ref());
    assert_eq!(m(&mut rw, 55, 105), Some(54), "S-sym-IMPL-FWD-SOURCE-SWITCH-9① upstream loss of 54 leaves one hole only");
    assert_eq!(m(&mut rw, 54, 104), Some(53), "S-sym-IMPL-FWD-SOURCE-SWITCH-9④ late forwarded packet gets its on-time number");
    assert_eq!(m(&mut rw, 53, 103), None, "S-sym-IMPL-FWD-SOURCE-SWITCH-9 a skipped seq that comes again is not sent");
}

#[test]
fn switch_after_drops_clears_runs_and_never_goes_back() {
    for seed in 1..=60u64 {
        let mut r = Rng(0xbb67_ae85_84ca_a73b ^ seed);
        let mut rw = Rewriter::new(90_000);
        let mut last: Option<u16> = None;
        let mut src = 1u64;
        let base = r.next() as u16;
        let mut seq = base;
        for i in 0..4_000u32 {
            if r.below(150) == 0 {
                src += 1;
                seq = base.wrapping_add(r.below(400) as u16);
            }
            seq = seq.wrapping_add(1);
            let pay = desc((i as u16) % M15, Some(i as u8), true);
            let pic = vp8_pic(&pay);
            if r.below(3) == 0 {
                rw.skip(src, seq, pic.as_ref());
                continue;
            }
            let before = rw.src;
            let Some(mo) = rw.map_pic(src, seq, i.wrapping_mul(3000), i as u64, 1, pic.as_ref()) else { continue };
            if before != Some(src) {
                assert!(rw.runs.is_empty(), "S-sym-IMPL-FWD-SOURCE-SWITCH-8① runs cleared on switch · seed {seed} · step {i}");
                if let Some(l) = last {
                    assert_eq!(mo.seq, l.wrapping_add(1), "S-sym-IMPL-FWD-SOURCE-SWITCH-8③ first out after switch = last + 1 · seed {seed} · step {i}");
                }
            }
            if let Some(l) = last {
                assert!(serial_after(mo.seq, l), "S-sym-IMPL-FWD-SOURCE-SWITCH-8② egress seq rises · seed {seed} · step {i} · {l} → {}", mo.seq);
            }
            last = Some(mo.seq);
        }
    }
}
