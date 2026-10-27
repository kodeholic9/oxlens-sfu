// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-SUB-TABLE · 정:IMPL-SUB-SUBSCRIBER-TABLE · model: claude-opus-5-5
//! 독립 1층 시험(B′ 맹검) — 배정표 줄 재활용(kind · 튜플 · 작은 mid 먼저 · 다운링크 SSRC 불변 · 유일) · audio 상한 재배정 · 헤더 확장 다시 짓기(RFC 8285).

use oxsfud::fwd::hot::Downlink;
use oxsfud::link::transport::Transport;
use oxsfud::sub::ext::{self, ExtTable};
use oxsfud::sub::table::{Change, Class, PtTable, Table, Tuple, Want, FULL_AUDIO, MID_START_1PC, MID_START_2PC};
use oxsig::types::Kind;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

fn vectors(name: &str) -> Value {
    let base = match std::env::var("OXLENS_SPEC") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec"),
    };
    let p = base.join("server/out/vectors").join(name);
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("벡터 없음 {}: {e}", p.display()))).unwrap()
}

fn make(mid: u8, kind: Kind, slot: bool, ssrc: u32, rtx: u32, pt: u8, rtx_pt: u8) -> Arc<Downlink> {
    Arc::new(Downlink::new(mid, kind, slot, ssrc, rtx, pt, rtx_pt, Arc::from("u"), Weak::<Transport>::new(), ExtTable::default()))
}

fn codec(k: Kind) -> &'static str {
    match k {
        Kind::Audio => "opus",
        _ => "H264",
    }
}

fn want(track: &str, room: &str, kind: Kind, rank: u64) -> Want {
    Want { track_id: track.into(), room: room.into(), kind, class: Class::Full, tuple: Tuple { kind, codec: codec(kind).into() }, rank, waiting_since: 0 }
}

fn on(t: &mut Table, w: Want) -> Vec<Change> {
    t.on(w, &make).expect("on")
}

fn on_mid(ch: &[Change]) -> Option<u8> {
    ch.iter().find_map(|c| match c {
        Change::On { mid, .. } => Some(*mid),
        _ => None,
    })
}

#[test]
fn mid_starts_by_mode() {
    let mut t = Table::new("sfu", false, "u", PtTable::seed_2pc(), 4, 0);
    assert_eq!(on_mid(&on(&mut t, want("a1", "r", Kind::Audio, 0))), Some(MID_START_2PC as u8), "S-sym-IMPL-SUB-TABLE-1 2pc mid 0 부터");
    let mut t = Table::new("sfu", true, "u", PtTable::seed_2pc(), 4, 0);
    assert_eq!(on_mid(&on(&mut t, want("a1", "r", Kind::Audio, 0))), Some(MID_START_1PC as u8), "S-sym-IMPL-SUB-TABLE-1 1pc mid 32 부터");
    assert_eq!((MID_START_1PC, MID_START_2PC), (32, 0), "S-sym-IMPL-SUB-TABLE-1 규격 값");
}

#[test]
fn empty_row_reused_same_kind_only_smallest_mid_first() {
    let mut t = Table::new("sfu", false, "u", PtTable::seed_2pc(), 4, 0);
    let a1 = on_mid(&on(&mut t, want("a1", "r", Kind::Audio, 0))).unwrap();
    let a2 = on_mid(&on(&mut t, want("a2", "r", Kind::Audio, 0))).unwrap();
    let v1 = on_mid(&on(&mut t, want("v1", "r", Kind::Video, 0))).unwrap();
    let ssrc_a1 = t.find("a1", "r").unwrap().ssrc;
    t.off("a2", "r");
    t.off("a1", "r");
    let v2 = on_mid(&on(&mut t, want("v2", "r", Kind::Video, 0))).unwrap();
    assert!(v2 != a1 && v2 != a2 && v2 != v1, "S-sym-IMPL-SUB-TABLE-1 kind 교차 재활용 0 — video 가 빈 audio 줄 {a1}/{a2} 를 안 쓴다(받은 {v2})");
    let a3 = on_mid(&on(&mut t, want("a3", "r", Kind::Audio, 0))).unwrap();
    assert_eq!(a3, a1.min(a2), "S-sym-IMPL-SUB-TABLE-1 같은 kind 해제분이 있으면 새 값 발급 0 · 가장 작은 mid 먼저");
    assert_eq!(t.find("a3", "r").unwrap().ssrc, ssrc_a1, "S-sym-IMPL-SUB-TABLE-1 줄이 사는 동안 다운링크 SSRC 불변");
    let a4 = on_mid(&on(&mut t, want("a4", "r", Kind::Audio, 0))).unwrap();
    assert_eq!(a4, a1.max(a2), "S-sym-IMPL-SUB-TABLE-1 둘째 빈 줄도 새 값보다 먼저");
    let rows = t.rows().len();
    assert_eq!(rows, 4, "S-sym-IMPL-SUB-TABLE-1 줄 수는 필요만큼 — 새 값으로만 자라지 않는다");
}

#[test]
fn downlink_ssrcs_unique_in_connection_and_avoid_reserved() {
    let mut t = Table::new("sfu", true, "u", PtTable::seed_2pc(), 4, 0);
    t.reserve_ssrc(0x1111_1111);
    for i in 0..20 {
        on(&mut t, want(&format!("a{i}"), "r", Kind::Audio, 0));
        on(&mut t, want(&format!("v{i}"), "r", Kind::Video, 0));
    }
    let ss: Vec<u32> = t.rows().iter().map(|r| r.ssrc).collect();
    let set: HashSet<u32> = ss.iter().copied().collect();
    assert_eq!(set.len(), ss.len(), "S-sym-IMPL-SUB-TABLE-1 다운링크 SSRC 연결 안에서 유일");
    assert!(!set.contains(&0x1111_1111), "S-sym-IMPL-SUB-TABLE-1 1pc 업링크 SSRC 와 겹치지 않는다");
    let rtx: Vec<u32> = t.rows().iter().filter_map(|r| r.rtx_ssrc).collect();
    assert!(rtx.iter().all(|x| !set.contains(x)), "S-sym-IMPL-SUB-TABLE-1 RTX SSRC 도 다운링크 SSRC 와 겹치지 않는다");
}

#[test]
fn audio_over_cap_evicts_longest_silent_not_recent_speaker() {
    let mut t = Table::new("sfu", false, "u", PtTable::seed_2pc(), 4, 0);
    on(&mut t, want("a0", "r", Kind::Audio, 0));
    for i in 1..FULL_AUDIO {
        on(&mut t, want(&format!("x{i}"), "r", Kind::Audio, 0));
    }
    assert_eq!(t.rows().iter().filter(|r| r.kind == Kind::Audio).count(), FULL_AUDIO, "시험 형상 — audio 줄 상한");
    for i in 1..FULL_AUDIO {
        t.set_rank(&format!("x{i}"), "r", 100 + i as u64);
    }
    t.set_rank("a0", "r", 10_000);
    let ch = on(&mut t, want("n", "r2", Kind::Audio, 20_000));
    let off: Vec<String> = ch.iter().filter_map(|c| match c {
        Change::Off { track_id, .. } => Some(track_id.clone()),
        _ => None,
    }).collect();
    assert!(t.find("a0", "r").is_some(), "S-sym-IMPL-SUB-TABLE-4 계속 말한 a0 가 남는다 — 비운 것 {off:?}");
    assert!(t.find("n", "r2").is_some(), "S-sym-IMPL-SUB-TABLE-4 막 말한 n 이 실렸다");
    assert_eq!(off, vec!["x1".to_string()], "S-sym-IMPL-SUB-TABLE-4 가장 오래 말하지 않은 것 하나를 비운다");
    assert_eq!(t.rows().iter().filter(|r| r.kind == Kind::Audio).count(), FULL_AUDIO, "S-sym-IMPL-SUB-TABLE-4 상한 넘어 줄이 안 생긴다");
}

#[test]
fn reassign_needs_ack_before_send() {
    let mut t = Table::new("sfu", false, "u", PtTable::seed_2pc(), 4, 0);
    on(&mut t, want("a1", "r", Kind::Audio, 0));
    let r = t.find("a1", "r").unwrap();
    assert!(r.since > t.ack, "S-sym-IMPL-SUB-TABLE-3 확인 전 줄은 since > ack(보내지 않는다)");
    let since = r.since;
    assert!(t.ack(since), "S-sym-IMPL-SUB-TABLE-3 확인을 받는다");
    assert!(t.find("a1", "r").unwrap().since <= t.ack, "S-sym-IMPL-SUB-TABLE-3 확인 뒤 보낼 수 있다");
    assert!(t.arev >= since, "S-sym-IMPL-SUB-TABLE-3 arev 는 줄 차례 이상");
}

fn o_abs(t_us: u64) -> [u8; 3] {
    let v = ((t_us as u128) << 18) / 1_000_000;
    let v = (v & 0xff_ffff) as u32;
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

fn o_build(tb: &ExtTable, mid: Option<&str>, level: Option<u8>, t_us: u64, twcc: u64) -> Vec<u8> {
    let mut els: Vec<(u8, Vec<u8>)> = vec![];
    if let (Some(id), Some(m)) = (tb.mid, mid) {
        els.push((id, m.as_bytes().to_vec()));
    }
    if let (Some(id), Some(l)) = (tb.audio_level, level) {
        els.push((id, vec![l]));
    }
    if let Some(id) = tb.abs_send_time {
        els.push((id, o_abs(t_us).to_vec()));
    }
    if let Some(id) = tb.twcc {
        els.push((id, ((twcc & 0xffff) as u16).to_be_bytes().to_vec()));
    }
    if els.is_empty() {
        return vec![];
    }
    let two = [tb.mid, tb.audio_level, tb.abs_send_time, tb.twcc].iter().flatten().any(|id| *id > 14);
    let mut body = vec![];
    for (id, v) in &els {
        if two {
            body.push(*id);
            body.push(v.len() as u8);
        } else {
            body.push((id << 4) | (v.len() as u8 - 1));
        }
        body.extend(v);
    }
    while body.len() % 4 != 0 {
        body.push(0);
    }
    let mut out = if two { vec![0x10, 0x00] } else { vec![0xbe, 0xde] };
    out.extend(((body.len() / 4) as u16).to_be_bytes());
    out.extend(body);
    out
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap()).collect()
}

fn table(v: &Value) -> ExtTable {
    let g = |k: &str| v.get(k).and_then(|x| x.as_u64()).map(|x| x as u8);
    ExtTable { mid: g("mid"), audio_level: g("audio_level"), abs_send_time: g("abs_send_time"), twcc: g("twcc") }
}

#[test]
fn ext_vectors_match_oracle_and_product() {
    let v = vectors("fwd.json");
    let cases = v["ext"].as_array().expect("ext 판 묶음");
    assert!(!cases.is_empty(), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 ext 판이 비었다");
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let tb = table(&c["table"]);
        let mid = c["mid"].as_str();
        let lv = c["audio_level_in"].as_u64().map(|x| x as u8);
        let (t_us, tw) = (c["t_us"].as_u64().unwrap(), c["twcc_counter"].as_u64().unwrap());
        let want = hex(c["hex"].as_str().unwrap());
        assert_eq!(o_build(&tb, mid, lv, t_us, tw), want, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 신탁 ≠ 벡터 — {name}");
        assert_eq!(ext::build(&tb, mid, lv, t_us, tw), want, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 제품 확장 — {name}");
    }
}

#[test]
fn ext_random_tables_against_oracle_and_read_back() {
    let mut x: u64 = 0x00e8_7e20_26ff;
    let mut r = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..2000 {
        let mut ids: Vec<u8> = (1..=20).collect();
        for i in (1..ids.len()).rev() {
            let j = (r() % (i as u64 + 1)) as usize;
            ids.swap(i, j);
        }
        let pick = |b: u64, id: u8| if b.is_multiple_of(3) { None } else { Some(id) };
        let tb = ExtTable { mid: pick(r(), ids[0]), audio_level: pick(r(), ids[1]), abs_send_time: pick(r(), ids[2]), twcc: pick(r(), ids[3]) };
        let mids = ["0", "7", "12", "ox"];
        let mid = if r() % 4 == 0 { None } else { Some(mids[(r() % 4) as usize]) };
        let lv = if r() % 2 == 0 { None } else { Some((r() % 128) as u8) };
        let (t_us, tw) = (r() >> 10, r());
        let got = ext::build(&tb, mid, lv, t_us, tw);
        assert_eq!(got, o_build(&tb, mid, lv, t_us, tw), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 확장 다시 짓기 — {tb:?} mid {mid:?}");
        if let (Some(id), Some(m)) = (tb.mid, mid) {
            assert_eq!(ext::read(&got, id), Some(m.as_bytes()), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-2 mid 확장 값 = 그 줄 mid");
        }
        if let Some(id) = tb.abs_send_time {
            assert_eq!(ext::read(&got, id), Some(&o_abs(t_us)[..]), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 abs-send-time 6.18");
        }
    }
}
