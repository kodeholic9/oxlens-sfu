// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 맹검 독립 1층 — egress 헤더 확장 다시 짓기(구독자 표의 ID 만 · mid = 줄의 mid · abs-send-time 식 · twcc 일련번호 · RFC 8285 꼴 고정) · 원소 차례는 안 보고 ID 로 읽는다.

use oxsfud::sub::ext::{ExtInput, ExtTable, abs_send_time, build};
use std::collections::BTreeMap;
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

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex")).collect()
}

#[derive(Debug, PartialEq, Eq)]
struct Block {
    two_byte: bool,
    words: usize,
    elems: BTreeMap<u8, Vec<u8>>,
}

fn parse_block(b: &[u8]) -> Block {
    assert!(b.len() >= 4, "extension block too short: {b:?}");
    let profile = u16::from_be_bytes([b[0], b[1]]);
    let words = u16::from_be_bytes([b[2], b[3]]) as usize;
    assert_eq!(b.len(), 4 + words * 4, "extension block length matches its word count: {b:?}");
    let data = &b[4..];
    let mut elems = BTreeMap::new();
    let two_byte = match profile {
        0xbede => false,
        p if p & 0xfff0 == 0x1000 => true,
        p => panic!("unknown extension profile {p:#06x}"),
    };
    let mut i = 0;
    while i < data.len() {
        if data[i] == 0 {
            i += 1;
            continue;
        }
        if two_byte {
            let id = data[i];
            let len = data[i + 1] as usize;
            elems.insert(id, data[i + 2..i + 2 + len].to_vec());
            i += 2 + len;
        } else {
            let id = data[i] >> 4;
            if id == 15 {
                break;
            }
            let len = (data[i] & 0x0f) as usize + 1;
            elems.insert(id, data[i + 1..i + 1 + len].to_vec());
            i += 1 + len;
        }
    }
    Block { two_byte, words, elems }
}

fn o_abs(t_us: u64) -> u32 {
    (((t_us % 64_000_000) << 18) / 1_000_000) as u32 & 0x00ff_ffff
}

fn table_of(v: &serde_json::Value) -> ExtTable {
    let g = |k: &str| v.get(k).and_then(|x| x.as_u64()).map(|x| x as u8);
    ExtTable { mid: g("mid"), audio_level: g("audio_level"), abs_send_time: g("abs_send_time"), twcc: g("twcc") }
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

#[test]
fn product_matches_ext_vectors() {
    let v = load("fwd.json");
    for case in v["ext"].as_array().expect("ext") {
        let name = case["name"].as_str().unwrap_or("");
        let table = table_of(&case["table"]);
        let mid = case["mid"].as_str().unwrap_or("").as_bytes().to_vec();
        let inp = ExtInput {
            mid: &mid,
            audio_level: case["audio_level_in"].as_u64().map(|x| x as u8),
            t_us: case["t_us"].as_u64().expect("t_us"),
            twcc_seq: Some(case["twcc_counter"].as_u64().expect("twcc") as u16),
        };
        let mut out = Vec::new();
        build(&table, &inp, &mut out);
        let got = parse_block(&out);
        let want = parse_block(&hex(case["hex"].as_str().expect("hex")));
        assert_eq!(got.two_byte, case["form"].as_str() == Some("two"), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 extension form · {name}");
        assert_eq!(got.words as u64, case["words"].as_u64().expect("words"), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 word count · {name}");
        assert_eq!(got.elems, want.elems, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 elements by subscriber id · {name}");
        if let Some(id) = table.mid
            && !mid.is_empty()
        {
            assert_eq!(got.elems.get(&id), Some(&mid), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-2 mid element is the row mid · {name}");
        }
    }
}

#[test]
fn abs_send_time_formula() {
    let mut rng = Rng(0x0abc_5e4d_0000_0001);
    for _ in 0..20_000 {
        let t = rng.next() >> rng.below(40);
        assert_eq!(abs_send_time(t), o_abs(t), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 abs-send-time 6.18 of t mod 64s · t {t}");
    }
    assert_eq!(abs_send_time(64_000_000), 0, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 abs-send-time wraps at 64s");
    assert_eq!(abs_send_time(1_000_000), 1 << 18, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 one second is 2^18");
}

#[test]
fn rebuilt_with_subscriber_ids_only() {
    let mut rng = Rng(0x5ab5_c71b_e700_0002);
    for round in 0..3_000 {
        let pick = |rng: &mut Rng| -> Option<u8> {
            if rng.below(4) == 0 {
                return None;
            }
            let span = if rng.below(5) == 0 { 40 } else { 14 };
            Some(1 + rng.below(span) as u8)
        };
        let mut ids: Vec<Option<u8>> = (0..4).map(|_| pick(&mut rng)).collect();
        for i in 0..4 {
            for j in 0..i {
                if ids[i].is_some() && ids[i] == ids[j] {
                    ids[i] = None;
                }
            }
        }
        let table = ExtTable { mid: ids[0], audio_level: ids[1], abs_send_time: ids[2], twcc: ids[3] };
        let present: Vec<u8> = ids.iter().flatten().copied().collect();
        if present.is_empty() {
            continue;
        }
        let one = present.iter().all(|&i| (1..=14).contains(&i));
        assert_eq!(table.one_byte(), one, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 one-byte iff every id in 1..14 · {table:?}");
        let mid_s = format!("{}", rng.below(300));
        let mid = if rng.below(5) == 0 { Vec::new() } else { mid_s.clone().into_bytes() };
        let level = if rng.below(3) == 0 { None } else { Some(rng.next() as u8) };
        let t_us = rng.next() >> 12;
        let tw = if rng.below(4) == 0 { None } else { Some(rng.next() as u16) };
        let inp = ExtInput { mid: &mid, audio_level: level, t_us, twcc_seq: tw };
        let mut out = Vec::new();
        build(&table, &inp, &mut out);
        if out.is_empty() {
            let any_value = (table.mid.is_some() && !mid.is_empty()) || (table.audio_level.is_some() && level.is_some()) || table.abs_send_time.is_some() || (table.twcc.is_some() && tw.is_some());
            assert!(!any_value, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 nothing written though a negotiated value exists · round {round} · {table:?}");
            continue;
        }
        let b = parse_block(&out);
        assert_eq!(b.two_byte, !one, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 form fixed by the subscriber table · round {round} · {table:?}");
        for id in b.elems.keys() {
            assert!(present.contains(id), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 element id {id} not in subscriber table · round {round} · {table:?}");
        }
        if let Some(id) = table.mid
            && !mid.is_empty()
        {
            assert_eq!(b.elems.get(&id), Some(&mid), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-2 mid element = row mid ASCII · round {round}");
        }
        if let (Some(id), Some(l)) = (table.audio_level, level) {
            assert_eq!(b.elems.get(&id), Some(&vec![l]), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 audio level copied under subscriber id · round {round}");
        }
        if table.audio_level.is_some() && level.is_none() {
            assert!(!b.elems.contains_key(&table.audio_level.unwrap_or(0)), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 no level element without an input level · round {round}");
        }
        if let Some(id) = table.abs_send_time {
            let v = o_abs(t_us).to_be_bytes();
            assert_eq!(b.elems.get(&id), Some(&v[1..].to_vec()), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 abs-send-time under subscriber id · round {round}");
        }
        if let (Some(id), Some(s)) = (table.twcc, tw) {
            assert_eq!(b.elems.get(&id), Some(&s.to_be_bytes().to_vec()), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 twcc seq under subscriber id · round {round}");
        }
    }
}

#[test]
fn ext_build_reads_time_only_from_argument() {
    let table = ExtTable { mid: Some(1), audio_level: Some(2), abs_send_time: Some(3), twcc: Some(4) };
    let mk = |t_us: u64| {
        let inp = ExtInput { mid: b"7", audio_level: Some(0x90), t_us, twcc_seq: Some(9) };
        let mut out = Vec::new();
        build(&table, &inp, &mut out);
        out
    };
    let a = mk(12_345_678);
    std::thread::sleep(std::time::Duration::from_millis(25));
    let b = mk(12_345_678);
    assert_eq!(a, b, "S-sym-RULE-CONC-SANS-IO-1 same t_us gives the same bytes whatever the wall clock");
    let c = parse_block(&mk(13_345_678));
    let want = (o_abs(13_345_678)).to_be_bytes()[1..].to_vec();
    assert_eq!(c.elems.get(&3), Some(&want), "S-sym-RULE-CONC-SANS-IO-1 abs-send-time follows the t_us argument");
}
