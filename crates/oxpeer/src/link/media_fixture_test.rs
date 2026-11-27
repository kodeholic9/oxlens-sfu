// author: kodeholic (powered by Claude)
//! 진짜 코덱 표본(ffmpeg 가 지은 VP8 · H264 · Opus — `scripts/media_check.sh`)을 보내는 쪽 패킷화(`send::packetize` — libwebrtc 꼴)로 쪼개 수신 정리층에 넣고,
//! 나온 프레임이 원본과 같은지 바이트로 견준다(20261107a 미결 — 재생기 호환 실측의 앞 절반). 뒤바뀜 · RTX 복구를 섞는다.
//! 나온 프레임은 `frames_<코덱>.bin` 으로 남긴다 — 예제 `mkvcheck` 가 녹화기의 MKV 쓰개로 파일을 짓고 스크립트가 ffprobe · ffmpeg 로 읽는다.
//! 표본 폴더는 `OXPEER_MEDIA_DIR` — 없으면 돌지 않는다(`#[ignore]`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;

use super::depack::Codec;
use super::recv::{Counters, Out, Recv, Rtp, Unit};
use super::send;
use super::transport::Route;

struct Frame {
    ts: u32,
    key: bool,
    data: Vec<u8>,
}

fn dir() -> Option<PathBuf> {
    std::env::var_os("OXPEER_MEDIA_DIR").map(PathBuf::from)
}

/// IVF(VP8) — 32바이트 머리 · 프레임마다 12바이트 머리(길이 LE · pts)
fn read_ivf(p: &Path) -> Vec<Vec<u8>> {
    let b = std::fs::read(p).expect("ivf");
    assert_eq!(&b[..4], b"DKIF");
    let mut i = usize::from(u16::from_le_bytes([b[6], b[7]]));
    let mut out = Vec::new();
    while i + 12 <= b.len() {
        let n = u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        out.push(b[i + 12..i + 12 + n].to_vec());
        i += 12 + n;
    }
    out
}

fn split_annexb(b: &[u8]) -> Vec<Vec<u8>> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let mut e = starts.get(k + 1).map(|n| n - 3).unwrap_or(b.len());
        while e > s && b[e - 1] == 0 {
            e -= 1;
        }
        out.push(b[s..e].to_vec());
    }
    out
}

/// H264 Annex-B — 접근 단위 경계는 AUD(9) · AUD 는 빼고 NAL 목록으로
fn read_h264(p: &Path) -> Vec<Vec<Vec<u8>>> {
    let mut aus: Vec<Vec<Vec<u8>>> = Vec::new();
    for n in split_annexb(&std::fs::read(p).expect("h264")) {
        if n[0] & 0x1f == 9 {
            aus.push(Vec::new());
        } else if let Some(a) = aus.last_mut() {
            a.push(n);
        }
    }
    aus.retain(|a| !a.is_empty());
    aus
}

/// Ogg Opus — 머리 두 패킷(OpusHead · OpusTags)은 뺀다
fn read_ogg(p: &Path) -> Vec<Vec<u8>> {
    let b = std::fs::read(p).expect("ogg");
    let (mut at, mut cur, mut out) = (0, Vec::new(), Vec::new());
    while at + 27 <= b.len() {
        assert_eq!(&b[at..at + 4], b"OggS");
        let n = usize::from(b[at + 26]);
        let mut body = at + 27 + n;
        for &x in &b[at + 27..at + 27 + n] {
            cur.extend_from_slice(&b[body..body + usize::from(x)]);
            body += usize::from(x);
            if x < 255 {
                out.push(std::mem::take(&mut cur));
            }
        }
        at = body;
    }
    out.split_off(2)
}

/// 제품 패킷화(`send::packetize` — 보내는 쪽과 같은 것)로 쪼갠다
fn pack(codec: Codec, f: &[u8]) -> Vec<Vec<u8>> {
    send::packetize(codec, f)
}

/// 접근 단위(NAL 목록) → Annex-B
fn annexb(nals: &[Vec<u8>]) -> Vec<u8> {
    nals.iter().flat_map(|n| [&[0, 0, 0, 1][..], n].concat()).collect()
}

fn route(kind: &str, codec: &str) -> Route {
    let video = kind == "video";
    Route { room: "r".into(), track_id: format!("t-{codec}"), user_id: Some("u".into()), kind: kind.into(), codec: codec.into(), mid: "1".into(), ssrc: 7, pt: 96, nack: video, pli: video, unit: Unit::Frame, layer_gen: 0, key_gen: 0 }
}

/// 패킷 열(프레임마다 [페이로드]) → 정리층 — 7번째마다 이웃과 뒤바꾸고 · 50번째마다 빼서 6 패킷 뒤 RTX 로 되돌린다
fn run(rt: &Route, frames: &[(u32, Vec<Vec<u8>>)]) -> (Vec<Frame>, Arc<Counters>) {
    let mut pkts: Vec<Rtp> = Vec::new();
    let mut seq = 40_000u16;
    for (ts, ps) in frames {
        for (i, p) in ps.iter().enumerate() {
            pkts.push(Rtp { seq, ts: *ts, marker: i == ps.len() - 1, pt: 96, payload: Bytes::copy_from_slice(p), recovered: false, source: None, raw: None });
            seq = seq.wrapping_add(1);
        }
    }
    let mut k = 6;
    while k + 1 < pkts.len() {
        pkts.swap(k, k + 1);
        k += 7;
    }
    let c = Arc::new(Counters::default());
    let mut r = Recv::new(9, c.clone());
    let t0 = Instant::now();
    let mut outs = Vec::new();
    let mut held: Vec<(usize, Rtp)> = Vec::new();
    for (i, p) in pkts.into_iter().enumerate() {
        let now = t0 + Duration::from_millis(i as u64);
        if i % 50 == 49 {
            held.push((i + 6, Rtp { recovered: true, ..p }));
        } else {
            outs.extend(r.on_rtp(rt, p, now, SystemTime::now()));
        }
        while let Some(pos) = held.iter().position(|(at, _)| *at <= i) {
            let (_, h) = held.remove(pos);
            outs.extend(r.on_rtp(rt, h, now, SystemTime::now()));
        }
        if i % 10 == 0 {
            outs.extend(r.tick(now).0);
        }
    }
    for (_, h) in held {
        outs.extend(r.on_rtp(rt, h, t0, SystemTime::now()));
    }
    let got = outs
        .into_iter()
        .filter_map(|o| match o {
            Out::Frame(f) => Some(Frame { ts: f.timestamp, key: f.keyframe, data: f.data.to_vec() }),
            _ => None,
        })
        .collect();
    (got, c)
}

fn save(dir: &Path, name: &str, fs: &[Frame]) {
    let mut b = Vec::new();
    for f in fs {
        b.push(u8::from(f.key));
        b.extend(f.ts.to_be_bytes());
        b.extend((f.data.len() as u32).to_be_bytes());
        b.extend_from_slice(&f.data);
    }
    std::fs::write(dir.join(format!("frames_{name}.bin")), b).expect("frames");
}

#[test]
#[ignore]
fn media_fixture_vp8_resolution_switch() {
    let Some(d) = dir() else { return };
    let src: Vec<Vec<u8>> = read_ivf(&d.join("vp8_a.ivf")).into_iter().chain(read_ivf(&d.join("vp8_b.ivf"))).collect();
    let frames: Vec<(u32, Vec<Vec<u8>>)> = src.iter().enumerate().map(|(i, f)| (i as u32 * 3000, pack(Codec::Vp8, f))).collect();
    let multi = frames.iter().filter(|(_, p)| p.len() > 1).count();
    let (got, c) = run(&route("video", "VP8"), &frames);
    assert_eq!(got.len(), src.len(), "프레임 수");
    for (i, (g, s)) in got.iter().zip(&src).enumerate() {
        assert_eq!(&g.data, s, "프레임 {i} 바이트");
        assert_eq!(g.key, s[0] & 1 == 0, "프레임 {i} 키");
    }
    eprintln!("VP8 {} 프레임(여러 조각 {multi}) · RTX 복구 {} · 잃음 {}", got.len(), c.rtx_recovered.load(std::sync::atomic::Ordering::Relaxed), c.lost.load(std::sync::atomic::Ordering::Relaxed));
    save(&d, "vp8", &got);
}

#[test]
#[ignore]
fn media_fixture_h264_resolution_switch() {
    let Some(d) = dir() else { return };
    let aus: Vec<Vec<Vec<u8>>> = read_h264(&d.join("h264_a.h264")).into_iter().chain(read_h264(&d.join("h264_b.h264"))).collect();
    let frames: Vec<(u32, Vec<Vec<u8>>)> = aus.iter().enumerate().map(|(i, a)| (i as u32 * 3000, pack(Codec::H264, &annexb(a)))).collect();
    let (stap, fua) = frames.iter().flat_map(|(_, p)| p).fold((0, 0), |(s, f), p| (s + usize::from(p[0] & 0x1f == 24), f + usize::from(p[0] & 0x1f == 28)));
    let (got, c) = run(&route("video", "H264"), &frames);
    assert_eq!(got.len(), aus.len(), "프레임 수");
    for (i, (g, a)) in got.iter().zip(&aus).enumerate() {
        assert_eq!(&split_annexb(&g.data), a, "접근 단위 {i} NAL");
        assert_eq!(g.key, a.iter().any(|n| n[0] & 0x1f == 5), "접근 단위 {i} 키");
    }
    eprintln!("H264 {} 프레임 · STAP-A {stap} · FU-A {fua} · RTX 복구 {}", got.len(), c.rtx_recovered.load(std::sync::atomic::Ordering::Relaxed));
    save(&d, "h264", &got);
}

#[test]
#[ignore]
fn media_fixture_opus() {
    let Some(d) = dir() else { return };
    let src = read_ogg(&d.join("opus.ogg"));
    let frames: Vec<(u32, Vec<Vec<u8>>)> = src.iter().enumerate().map(|(i, p)| (i as u32 * 960, vec![p.clone()])).collect();
    let (got, _) = run(&route("audio", "opus"), &frames);
    // 오디오는 NACK 이 없다 — RTX 로 되돌린 패킷은 순서 기다림(50ms) 안에 와서 제자리에 들어간다
    assert_eq!(got.iter().map(|f| f.data.clone()).collect::<Vec<_>>(), src);
    assert!(got.windows(2).all(|w| w[1].ts == w[0].ts + 960));
    save(&d, "opus", &got);
}
