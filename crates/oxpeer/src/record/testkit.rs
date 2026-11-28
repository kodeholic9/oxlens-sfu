// author: kodeholic (powered by Claude)
//! 1층 시험 재료 — 레코드열 짓기(정리층 출구 레코드 · 제어 레코드). oxpeer · oxtapd 시험이 함께 쓴다(기능 `testkit`).

use std::path::PathBuf;

use super::{Channel, Output, Pipeline, Policy, Record};

pub fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("oxpeer-rec-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

// ── 2단계 — 파이프라인 · Ogg · 라이브 통(SDK:RULE-PLAT-REC-PIPE · REC-SPLIT · REC-API) ──

pub fn pkt(seq: u16, ts: u32, marker: bool, csrc: Option<u32>, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![if csrc.is_some() { 0x81 } else { 0x80 }, if marker { 0xEF } else { 0x6F }, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    v[2..4].copy_from_slice(&seq.to_be_bytes());
    v[4..8].copy_from_slice(&ts.to_be_bytes());
    if let Some(c) = csrc {
        v.extend_from_slice(&c.to_be_bytes());
    }
    v.extend_from_slice(payload);
    v
}

pub fn rec(seq: u32, t: u64, ch: Channel, data: Vec<u8>) -> Record {
    Record { sequence: seq, log_time: t, channel: ch, data }
}

pub fn tracks(room: &str) -> Vec<u8> {
    serde_json::json!({
        "room": room,
        "tracks": [{ "track_id": "slot-a", "kind": "audio" }, { "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 }],
        "rows": { "100": { "track_id": "slot-a", "kind": "audio", "codec": "opus", "pt": 111 }, "200": { "track_id": "cam-u1", "kind": "video", "codec": "VP8", "pt": 96 } }
    })
    .to_string()
    .into_bytes()
}

/// 무전 슬롯: 출처 없음(예열 침묵) · A(7) 둘 · B(8) 하나 · 출처 없음 / 영상: 델타(버림) · 키 · 델타 · gap · 델타(버림) · 키
pub fn stream() -> Vec<Record> {
    let ms = 1_000_000u64;
    let mut v = vec![rec(1, 0, Channel::Track, tracks("r")), rec(2, ms, Channel::Floor, br#"{"room":"r","event":"taken","user_id":"A","sources":[7],"seq":1}"#.to_vec())];
    let mut n = 3u32;
    let mut put = |v: &mut Vec<Record>, ch: Channel, d: Vec<u8>| {
        v.push(rec(n, u64::from(n) * 20 * ms, ch, d));
        n += 1;
    };
    put(&mut v, Channel::Rtp(100), pkt(1, 0, true, None, &[0xF8, 0xFF, 0xFE]));
    put(&mut v, Channel::Rtp(100), pkt(2, 960, true, Some(7), &[0x78, 1]));
    put(&mut v, Channel::Rtp(100), pkt(3, 1920, true, Some(7), &[0x78, 2]));
    put(&mut v, Channel::Floor, br#"{"room":"r","event":"taken","user_id":"B","sources":[8],"seq":2}"#.to_vec());
    put(&mut v, Channel::Rtp(100), pkt(4, 2880, true, Some(8), &[0x78, 3]));
    put(&mut v, Channel::Rtp(100), pkt(5, 3840, true, None, &[0xF8, 0xFF, 0xFE]));
    put(&mut v, Channel::Sr, br#"{"ssrc":200,"ntp":16045690984503098046,"rtp":3000}"#.to_vec());
    put(&mut v, Channel::Rtp(200), pkt(1, 3000, true, Some(9), &[0x10, 0x01, 1]));
    put(&mut v, Channel::Rtp(200), pkt(2, 6000, true, Some(9), &[0x10, 0x00, 2]));
    put(&mut v, Channel::Rtp(200), pkt(3, 9000, true, Some(9), &[0x10, 0x01, 3]));
    put(&mut v, Channel::Gap, br#"{"ssrc":200,"from":4,"to":4,"count":1,"reason":"nack_exhausted"}"#.to_vec());
    put(&mut v, Channel::Rtp(200), pkt(5, 15000, true, Some(9), &[0x10, 0x01, 5]));
    put(&mut v, Channel::Rtp(200), pkt(6, 18000, true, Some(9), &[0x10, 0x00, 6]));
    v
}

pub fn floor(n: u32, t: u64, ev: &str, who: &str, src: &[u32]) -> Record {
    let user = if ev == "taken" { format!("\"{who}\"") } else { "null".into() };
    rec(n, t, Channel::Floor, format!(r#"{{"room":"r","event":"{ev}","user_id":{user},"sources":{src:?},"seq":{n}}}"#).into_bytes())
}

pub fn slot_audio(n: u32, t: u64, seq: u16, src: u32) -> Record {
    rec(n, t, Channel::Rtp(100), pkt(seq, u32::from(seq) * 960, true, Some(src), &[0x78, seq as u8]))
}

/// 받은 `taken` 에 서버 발언 id(TLV `0x20` — 연:RULE-FLR-TLV)를 싣는다 — `talk` = "t" + 허가 번호
pub fn with_talk(recs: Vec<Record>) -> Vec<Record> {
    recs.into_iter()
        .map(|mut r| {
            if r.channel == Channel::Floor
                && let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(&r.data)
                && v["event"] == "taken"
            {
                v["talk"] = serde_json::json!(format!("t{}", v["seq"]));
                r.data = v.to_string().into_bytes();
            }
            r
        })
        .collect()
}

// ── 3단계 — 음성 구멍 PLC(SDK:RULE-PLAT-REC-GAP) ──

pub fn opus_stream(seqs_ts: &[(u16, u32)], toc: u8) -> Vec<Record> {
    let mut v = vec![rec(1, 0, Channel::Track, tracks("r")), rec(1, 0, Channel::Floor, br#"{"room":"r","event":"taken","user_id":"A","sources":[7],"seq":1}"#.to_vec())];
    for (i, (seq, ts)) in seqs_ts.iter().enumerate() {
        v.push(rec(2 + i as u32, (i as u64 + 1) * 20_000_000, Channel::Rtp(100), pkt(*seq, *ts, true, Some(7), &[toc, 9, 9])));
    }
    v
}

// ── 4단계(202611/20261110a) — 분기 = 출처 트랙 · 열쇠 = 트랙 · 영상은 키프레임에서 연다 · Matroska 결과물 ──

pub fn track_list(room: &str, tracks: serde_json::Value, rows: serde_json::Value) -> Vec<u8> {
    serde_json::json!({ "room": room, "tracks": tracks, "rows": rows }).to_string().into_bytes()
}

pub const KEY: &[u8] = &[0x10, 0x00, 0x9d, 0x01, 0x2a];

pub const DELTA: &[u8] = &[0x10, 0x01, 7];

pub fn vrow(tid: &str) -> serde_json::Value {
    serde_json::json!({ "track_id": tid, "kind": "video", "codec": "VP8", "pt": 96 })
}

// ── 5단계(202611/20261110b) — 영상 · 음성 표본 ──

pub const SPS_A: &[u8] = &[0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16, 0xe4];

pub const SPS_B: &[u8] = &[0x67, 0x42, 0xc0, 0x1f, 0xda, 0x02, 0x80, 0x2d, 0xc8];

pub const PPS: &[u8] = &[0x68, 0xce, 0x3c, 0x80];

/// 사람 u1 — 영상(H264 · 줄 200 · CSRC 9) · 음성(Opus · 줄 100 · CSRC 10) · 같은 시계에서 `secs` 초 · 영상 키 1초마다 · SR 은 0.5초에(둘 다)
pub fn av_stream(secs: u32, start_v: u32, sps_switch_at: Option<u32>) -> Vec<Record> {
    av_stream_gop(secs, start_v, sps_switch_at, 30)
}

/// `gop` 프레임마다 키프레임(33ms 간격 — 30 이면 약 1초)
pub fn av_stream_gop(secs: u32, start_v: u32, sps_switch_at: Option<u32>, gop: u32) -> Vec<Record> {
    let ms = 1_000_000u64;
    let t0 = 1_800_000_000_000_000_000u64;
    let tracks = serde_json::json!([
        { "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 },
        { "track_id": "mic-u1", "kind": "audio", "user_id": "u1", "csrc": 10 }
    ]);
    let rows = serde_json::json!({ "200": { "track_id": "cam-u1", "kind": "video", "codec": "H264", "pt": 102 }, "100": { "track_id": "mic-u1", "kind": "audio", "codec": "opus", "pt": 111 } });
    let mut v = vec![rec(1, t0, Channel::Track, track_list("r", tracks, rows))];
    let mut n = 2u32;
    let (mut vseq, mut aseq) = (0u16, 0u16);
    // 음성은 20ms 마다(960 틱) · 영상은 33ms 마다 한 프레임(3000 틱 = 1/30 초 — 받은 시각만 ms 로 근사)
    let end_ms = secs * 1000;
    let mut tv = 0u32;
    let mut ta = 0u32;
    let ntp = |unix_ns: u64| -> u64 { (((unix_ns / 1_000_000_000) + 2_208_988_800) << 32) | (((unix_ns % 1_000_000_000) << 32) / 1_000_000_000) };
    let mut sr_done = false;
    while tv.min(ta) < end_ms {
        if !sr_done && tv.min(ta) >= 500 {
            sr_done = true;
            let at = t0 + 500 * ms;
            v.push(rec(n, at, Channel::Sr, format!(r#"{{"ssrc":200,"ntp":{},"rtp":{}}}"#, ntp(at), start_v.wrapping_add(500 * 90)).into_bytes()));
            n += 1;
            v.push(rec(n, at, Channel::Sr, format!(r#"{{"ssrc":100,"ntp":{},"rtp":{}}}"#, ntp(at), 500 * 48).into_bytes()));
            n += 1;
        }
        if tv <= ta {
            let i = tv / 33;
            let key = i.is_multiple_of(gop);
            let ts = start_v.wrapping_add(i * 3000);
            let sps = if sps_switch_at.is_some_and(|s| tv >= s) { SPS_B } else { SPS_A };
            let (idr, delta) = ([0x65, 0x88, 0x84, i as u8], [0x41, 0x9a, i as u8]);
            let nal: Vec<&[u8]> = if key { vec![sps, PPS, &idr] } else { vec![&delta] };
            for (k, x) in nal.iter().enumerate() {
                v.push(rec(n, t0 + u64::from(tv) * ms, Channel::Rtp(200), pkt(vseq, ts, k + 1 == nal.len(), Some(9), x)));
                n += 1;
                vseq = vseq.wrapping_add(1);
            }
            tv += 33;
        } else {
            v.push(rec(n, t0 + u64::from(ta) * ms, Channel::Rtp(100), pkt(aseq, ta * 48, true, Some(10), &[0x78, aseq as u8])));
            n += 1;
            aseq = aseq.wrapping_add(1);
            ta += 20;
        }
    }
    v
}

/// SR 을 `sr_at_ms` 에 받고(늦은 SR) · 영상이 `video_delay_ms` 늦게 닿는 사람 하나(도착 차 = 늦은 SR 맞춤이 고칠 오차)
pub fn late_sr_av(secs: u32, sr_at_ms: u64, video_delay_ms: u64) -> Vec<Record> {
    let t0 = 1_800_000_000_000_000_000u64;
    let mut recs = av_stream(secs, 1000, None);
    for r in &mut recs {
        match r.channel {
            Channel::Sr => r.log_time = t0 + sr_at_ms * 1_000_000,
            Channel::Rtp(200) => r.log_time += video_delay_ms * 1_000_000,
            _ => {}
        }
    }
    recs.sort_by_key(|r| (r.log_time, r.sequence));
    for (i, r) in recs.iter_mut().enumerate() {
        r.sequence = i as u32 + 1;
    }
    recs
}

pub fn long_opus(n: u32) -> Vec<Record> {
    opus_stream(&(0..n).map(|i| (i as u16, i * 960)).collect::<Vec<_>>(), 0x78)
}

pub fn run_pipe(p: Policy, recs: &[Record]) -> Vec<Output> {
    let mut pl = Pipeline::new(p);
    let mut out: Vec<Output> = recs.iter().flat_map(|r| pl.push(r)).collect();
    out.extend(pl.finish());
    out
}
