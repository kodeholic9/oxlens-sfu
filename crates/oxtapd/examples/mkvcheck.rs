// author: kodeholic (powered by Claude)
//! 재생기 호환 실측 도구(context 202611/20261107a 미결) — 수신 정리층이 다시 이은 진짜 코덱 프레임(`frames_<코덱>.bin` · lib 시험 `media_fixture_*`)을
//! ★녹음 유닛 결과물 `MkvSink`(정:RULE-OPS-REC-MKV)로 파일에 쓴다. 판정은 `scripts/media_check.sh` 가 ffprobe · ffmpeg 로 한다.
//!
//!   cargo run -p oxpeer --example mkvcheck --features record -- <표본 폴더>

use std::path::PathBuf;

use oxpeer::record::{Frame, Output, Segment, SegmentKind};
use oxtapd::sink::MkvSink;

/// `frames_<코덱>.bin` → [(키, RTP 시각, 본문)]
fn load(dir: &std::path::Path, name: &str) -> Vec<(bool, u32, Vec<u8>)> {
    let b = std::fs::read(dir.join(format!("frames_{name}.bin"))).expect("frames");
    let (mut i, mut out) = (0, Vec::new());
    while i + 9 <= b.len() {
        let len = u32::from_be_bytes([b[i + 5], b[i + 6], b[i + 7], b[i + 8]]) as usize;
        out.push((b[i] == 1, u32::from_be_bytes([b[i + 1], b[i + 2], b[i + 3], b[i + 4]]), b[i + 9..i + 9 + len].to_vec()));
        i += 9 + len;
    }
    out
}

fn seg(id: u64, kind: &str, codec: &str, event: SegmentKind) -> Segment {
    Segment {
        event,
        id,
        room: "mkvcheck".into(),
        track_id: codec.into(),
        owner: Some("u".into()),
        kind: kind.into(),
        codec: codec.into(),
        source: None,
        user_id: Some("u".into()),
        first_utc_ns: None,
        last_utc_ns: None,
        first_log_time: 0,
        last_log_time: 0,
        frames: 0,
        bytes: 0,
        filled: 0,
        reason: if event == SegmentKind::Closed { Some("end") } else { None },
        utterance: None,
        talk: None,
    }
}

/// H264 + Opus 를 RTP 시각(초) 순으로 섞어 한 사람의 결과물로 — Matroska 한 파일(`av.mkv` — 정:RULE-OPS-REC-MKV 사람마다 영상 · 음성 · 202611/20261110e)
fn av(dir: &std::path::Path) {
    let mut sink = MkvSink::new(dir.join("av")).expect("폴더");
    let tracks = [("h264", "video", 90_000u64, 1u64), ("opus", "audio", 48_000, 2)];
    let mut all: Vec<(u64, usize, bool, u32, Vec<u8>)> = Vec::new();
    for (k, (name, _, clock, _)) in tracks.iter().enumerate() {
        let fr = load(dir, name);
        let t0 = fr[0].1;
        for (key, ts, data) in fr {
            all.push((u64::from(ts.wrapping_sub(t0)) * 1_000_000_000 / clock, k, key, ts, data));
        }
    }
    all.sort_by_key(|x| (x.0, x.1));
    for (name, kind, _, id) in tracks {
        sink.write(&Output::Segment(seg(id, kind, name, SegmentKind::Opened))).expect("MKV");
    }
    let base = 1_800_000_000_000_000_000u64;
    for (at, k, key, ts, data) in all {
        let (name, kind, _, id) = tracks[k];
        let f = Frame {
            segment: Some(id),
            room: "mkvcheck".into(),
            track_id: name.into(),
            owner: Some("u".into()),
            kind: kind.into(),
            codec: name.into(),
            ssrc: id as u32,
            source: None,
            user_id: Some("u".into()),
            keyframe: key,
            rtp_ts: ts,
            utc_ns: Some(base + at),
            log_time: base + at,
            seq_first: 0,
            seq_last: 0,
            rec_first: 0,
            rec_last: 0,
            data,
            filled: false,
        };
        sink.write(&Output::Frame(f)).expect("MKV");
    }
    for (name, kind, _, id) in tracks {
        sink.write(&Output::Segment(seg(id, kind, name, SegmentKind::Closed))).expect("MKV");
    }
    sink.finish().expect("끝");
    let f = sink.files().last().expect("파일");
    let out = dir.join("av.mkv");
    std::fs::rename(&f.path, &out).expect("이름");
    println!("{} 트랙 {:?} · 기준 {}", out.display(), f.tracks.iter().map(|t| (t.kind.as_str(), t.frames)).collect::<Vec<_>>(), f.base);
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("표본 폴더"));
    let mut sink = MkvSink::new(&dir).expect("폴더");
    for (id, (name, kind, clock)) in [("vp8", "video", 90u64), ("h264", "video", 90), ("opus", "audio", 48)].into_iter().enumerate() {
        let b = std::fs::read(dir.join(format!("frames_{name}.bin"))).expect("frames");
        let seg = Segment {
            event: SegmentKind::Opened,
            id: id as u64,
            room: "mkvcheck".into(),
            track_id: name.into(),
            owner: Some(name.into()),
            kind: kind.into(),
            codec: name.into(),
            source: None,
            user_id: Some(name.into()),
            first_utc_ns: None,
            last_utc_ns: None,
            first_log_time: 0,
            last_log_time: 0,
            frames: 0,
            bytes: 0,
            filled: 0,
            reason: None,
            utterance: None,
            talk: None,
        };
        sink.write(&Output::Segment(seg.clone())).expect("열기");
        let (mut i, mut n, mut t0) = (0, 0u64, None);
        while i + 9 <= b.len() {
            let key = b[i] == 1;
            let ts = u32::from_be_bytes([b[i + 1], b[i + 2], b[i + 3], b[i + 4]]);
            let len = u32::from_be_bytes([b[i + 5], b[i + 6], b[i + 7], b[i + 8]]) as usize;
            let first = *t0.get_or_insert(ts);
            // UTC 는 1_800_000_000 초에서 RTP 경과만큼 — 레코드 시각도 같게
            let at = 1_800_000_000_000_000_000u64 + u64::from(ts.wrapping_sub(first)) * 1_000_000 / clock;
            let f = Frame {
                segment: Some(id as u64),
                room: "mkvcheck".into(),
                track_id: name.into(),
                owner: Some(name.into()),
                kind: kind.into(),
                codec: name.into(),
                ssrc: 1,
                source: None,
                user_id: Some(name.into()),
                keyframe: key,
                rtp_ts: ts,
                utc_ns: Some(at),
                log_time: at,
                seq_first: 0,
                seq_last: 0,
                rec_first: 0,
                rec_last: 0,
                data: b[i + 9..i + 9 + len].to_vec(),
                filled: false,
            };
            sink.write(&Output::Frame(f)).expect("프레임");
            n += 1;
            i += 9 + len;
        }
        sink.write(&Output::Segment(Segment { event: SegmentKind::Closed, reason: Some("end"), ..seg })).expect("닫기");
        // 판정 스크립트는 `<코덱>.mkv` 를 읽는다
        let out = dir.join(format!("{name}.mkv"));
        std::fs::rename(&sink.files().last().expect("파일").path, &out).expect("이름");
        println!("{} {n} 프레임", out.display());
    }
    av(&dir);
}
