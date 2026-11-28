// author: kodeholic (powered by Claude)
//! 1층 — 녹음 유닛 결과물(정:IMPL-OPS-REC-UNIT · 정:RULE-OPS-REC-MKV): Ogg · Matroska 바이트 결정론 · 이름 · 회전 · 디스크 하한 · 회수 · 덮음 요약 · 키프레임 요청

use std::path::{Path, PathBuf};
use std::time::Duration;

use oxpeer::record::{Output, Pipeline, Policy, SegmentKind};
use oxpeer::record::testkit::*;
use oxpeer::record::{Channel, Record};

use super::*;

#[test]
fn ogg_sink_bytes_are_identical_for_identical_records() {
    let d1 = tmp("ogg1");
    let d2 = tmp("ogg2");
    for d in [&d1, &d2] {
        let mut sink = OggSink::new(d).unwrap();
        for o in run_pipe(Policy::Utterance, &stream()) {
            sink.write(&o).unwrap();
        }
        assert_eq!(sink.files().len(), 2);
        assert_eq!(sink.files()[0].packets, 2);
        assert_eq!(sink.files()[0].samples, 1_920);
    }
    for name in ["r-0000-A.ogg", "r-0002-B.ogg"] {
        assert_eq!(std::fs::read(d1.join(name)).unwrap(), std::fs::read(d2.join(name)).unwrap(), "{name}");
    }
}

#[test]
fn ogg_files_are_named_by_the_server_talk_id() {
    // SDK:RULE-PLAT-REC-SPLIT 발언 번호 = `taken` 의 발언 id · 결과물 `{방}-{발언 id}-{화자}.ogg`
    let d = tmp("ogg-talk");
    let mut sink = OggSink::new(&d).unwrap();
    let out = run_pipe(Policy::Utterance, &with_talk(stream()));
    let talks: Vec<Option<String>> = out.iter().filter_map(|o| if let Output::Segment(s) = o { (s.event == SegmentKind::Opened && s.owner.is_none()).then(|| s.talk.clone()) } else { None }).collect();
    assert_eq!(talks, vec![Some("t1".to_string()), Some("t2".to_string())], "세그먼트가 그 허가의 발언 id 를 든다");
    for o in &out {
        sink.write(o).unwrap();
    }
    let names: Vec<String> = sink.files().iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["r-t1-A.ogg".to_string(), "r-t2-B.ogg".to_string()]);
    assert_eq!(sink.files().iter().map(|f| f.talk.clone()).collect::<Vec<_>>(), vec![Some("t1".into()), Some("t2".into())], "파일이 발언 id 를 든다(녹음 유닛이 `rec` 알림에 쓴다)");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn ogg_with_filled_frames_spans_rtp_time() {
    let d = tmp("oggfill");
    let mut sink = OggSink::new(&d).unwrap();
    for o in run_pipe(Policy::Utterance, &opus_stream(&[(1, 0), (2, 960), (5, 3_840)], 0x78)) {
        sink.write(&o).unwrap();
    }
    let f = &sink.files()[0];
    assert_eq!((f.packets, f.filled), (5, 2));
    assert_eq!(u64::from(f.end_ts.unwrap() - f.first_ts.unwrap()), f.samples, "메운 뒤 표본 수 = RTP 폭");
}

/// 같은 레코드열 → 같은 Matroska 바이트 · 줄이 바뀐 세그먼트는 `log_time` 차로 잇는다 · UTC 없으면 첫 프레임 레코드 시각
fn mkv_build(recs: &[Record], dir: &Path) -> Vec<MkvFile> {
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut sink = MkvSink::new(dir).unwrap();
    for r in recs {
        for o in pl.push(r) {
            sink.write(&o).unwrap();
        }
    }
    for o in pl.finish() {
        sink.write(&o).unwrap();
    }
    sink.files().to_vec()
}

#[test]
fn mkv_sink_is_deterministic_and_rebases_across_rows() {
    let ms = 1_000_000u64;
    let t0 = 1_800_000_000_000_000_000u64;
    let u1 = serde_json::json!([{ "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 }]);
    let recs = [
        rec(1, t0, Channel::Track, track_list("r", u1.clone(), serde_json::json!({ "200": vrow("cam-u1") }))),
        rec(2, t0 + ms, Channel::Rtp(200), pkt(1, 3000, true, Some(9), KEY)),
        rec(3, t0 + 34 * ms, Channel::Rtp(200), pkt(2, 6000, true, Some(9), DELTA)),
        rec(4, t0 + 40 * ms, Channel::Track, track_list("r", u1, serde_json::json!({ "300": vrow("cam-u1") }))),
        rec(5, t0 + 534 * ms, Channel::Rtp(300), pkt(9, 4_000_000_000, true, Some(9), KEY)),
    ];
    let (a, b) = (tmp("mkv-a"), tmp("mkv-b"));
    let fa = mkv_build(&recs, &a);
    let fb = mkv_build(&recs, &b);
    assert_eq!(fa.len(), 1);
    assert_eq!(std::fs::read(&fa[0].path).unwrap(), std::fs::read(&fb[0].path).unwrap(), "같은 레코드열이면 같은 바이트");
    assert_eq!(fa[0].path.file_name().unwrap(), "r-u1-0000.mkv", "묶음(방 · 주인) · 파일 차례");
    let v = &fa[0].tracks[0];
    assert_eq!((v.kind.as_str(), v.frames, v.keyframes, v.rebased, v.synced), ("video", 3, 2, 1, 0));
    assert_eq!(fa[0].duration_ms, 33 + 500 + 33, "줄을 옮긴 뒤는 레코드 시각 차(500ms)로 잇는다 — 길이 = 마지막 블록 + 한 프레임");
    assert_eq!((fa[0].start_ms, fa[0].base), ((t0 + ms) / 1_000_000, "log_time"), "UTC 가 없으면 첫 프레임 레코드 시각");
    std::fs::remove_dir_all(&a).ok();
    std::fs::remove_dir_all(&b).ok();
}

#[test]
fn mkv_start_is_back_computed_from_first_synced_frame() {
    let ms = 1_000_000u64;
    let u1 = serde_json::json!([{ "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 }]);
    // SR — NTP 초 3_000_000_000(1900 기준) ↔ RTP 9000. 첫 프레임(3000)은 SR 전이라 UTC 없음 · 셋째(9000)가 첫 동기
    let ntp = 3_000_000_000u64 << 32;
    let recs = [
        rec(1, 0, Channel::Track, track_list("r", u1, serde_json::json!({ "200": vrow("cam-u1") }))),
        rec(2, ms, Channel::Rtp(200), pkt(1, 3000, true, Some(9), KEY)),
        rec(3, 2 * ms, Channel::Rtp(200), pkt(2, 6000, true, Some(9), DELTA)),
        rec(4, 3 * ms, Channel::Sr, format!(r#"{{"ssrc":200,"ntp":{ntp},"rtp":9000}}"#).into_bytes()),
        rec(5, 4 * ms, Channel::Rtp(200), pkt(3, 9000, true, Some(9), DELTA)),
    ];
    let d = tmp("mkv-sr");
    let f = mkv_build(&recs, &d);
    let utc_ms = (3_000_000_000u64 - 2_208_988_800) * 1_000;
    assert_eq!((f[0].start_ms, f[0].base), ((utc_ms * 1_000_000 - 66_666_666) / 1_000_000, "utc"), "첫 동기 프레임 UTC − 그때까지 흐른 RTP 경과(6000 틱 = 66.67ms · DateUTC 는 ns 로 적는다)");
    assert_eq!(f[0].tracks[0].synced, 1);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn mkv_rtp_time_wrap_keeps_blocks_moving_forward() {
    let ms = 1_000_000u64;
    let u1 = serde_json::json!([{ "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 }]);
    let mut recs = vec![rec(1, 0, Channel::Track, track_list("r", u1, serde_json::json!({ "200": vrow("cam-u1") })))];
    let start = u32::MAX - 3 * 3000 + 1;
    for i in 0..8u32 {
        recs.push(rec(2 + i, u64::from(i + 1) * 33 * ms, Channel::Rtp(200), pkt(i as u16, start.wrapping_add(i * 3000), true, Some(9), if i % 4 == 0 { KEY } else { DELTA })));
    }
    let d = tmp("mkv-wrap");
    let f = mkv_build(&recs, &d);
    let v = &f[0].tracks[0];
    assert_eq!((v.frames, v.first_rtp, v.last_rtp), (8, Some(start), Some(start.wrapping_add(7 * 3000))));
    assert_eq!(f[0].duration_ms, (7 * 3000 + 3000) / 90, "넘침을 지나도 앞으로만 간다 — 길이 = 마지막 블록 + 한 프레임");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn mkv_av_file_across_rtp_wrap_of_both_tracks_is_one_file_moving_forward() {
    let mut recs = av_stream(6, u32::MAX - 90_000 * 2, None);
    let off = u32::MAX - 48_000 * 3;
    for r in recs.iter_mut() {
        match &r.channel {
            Channel::Rtp(100) => {
                let ts = u32::from_be_bytes(r.data[4..8].try_into().unwrap()).wrapping_add(off);
                r.data[4..8].copy_from_slice(&ts.to_be_bytes());
            }
            Channel::Sr => {
                let mut j: serde_json::Value = serde_json::from_slice(&r.data).unwrap();
                if j["ssrc"] == 100 {
                    j["rtp"] = serde_json::json!((j["rtp"].as_u64().unwrap() as u32).wrapping_add(off));
                    r.data = serde_json::to_vec(&j).unwrap();
                }
            }
            _ => {}
        }
    }
    let d = tmp("mkv-av-wrap");
    let f = mkv_build(&recs, &d);
    assert_eq!(f.len(), 1, "넘침은 파일 경계가 아니다 — 사람 하나 · 파일 하나");
    let (v, a) = (f[0].tracks.iter().find(|t| t.kind == "video").unwrap(), f[0].tracks.iter().find(|t| t.kind == "audio").unwrap());
    assert!(v.last_rtp.unwrap() < v.first_rtp.unwrap() && a.last_rtp.unwrap() < a.first_rtp.unwrap(), "두 트랙 모두 넘침을 지났다");
    assert_eq!(f[0].base, "utc");
    assert!((5_900..=6_100).contains(&f[0].duration_ms), "넘침을 펼쳐 길이가 판 길이 그대로 — {}", f[0].duration_ms);
    std::fs::remove_dir_all(&d).ok();
}

// ── 시험 단계 T2(202611/20261110d) — RTP seq 넘침(설계 §10-3 「seq 넘침도 같은 방식」) ──

#[test]
fn pipeline_assembles_video_frame_across_seq_wrap_and_ogg_keeps_range() {
    let ms = 1_000_000u64;
    let u1 = serde_json::json!([{ "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 }]);
    let mut recs = vec![rec(1, 0, Channel::Track, track_list("r", u1, serde_json::json!({ "200": vrow("cam-u1") })))];
    // 키프레임 한 장이 조각 셋 — seq 65535 · 0 · 1(marker) · 다음 델타 2
    recs.push(rec(2, ms, Channel::Rtp(200), pkt(65535, 3000, false, Some(9), &[0x10, 0x00, 0x9d, 1])));
    recs.push(rec(3, ms, Channel::Rtp(200), pkt(0, 3000, false, Some(9), &[0x00, 2])));
    recs.push(rec(4, ms, Channel::Rtp(200), pkt(1, 3000, true, Some(9), &[0x00, 3])));
    recs.push(rec(5, 2 * ms, Channel::Rtp(200), pkt(2, 6000, true, Some(9), DELTA)));
    let mut pl = Pipeline::new(Policy::Utterance);
    let out: Vec<Output> = recs.iter().flat_map(|r| pl.push(r)).collect();
    let fr: Vec<(u16, u16, bool, Vec<u8>)> = out.iter().filter_map(|o| if let Output::Frame(f) = o { Some((f.seq_first, f.seq_last, f.keyframe, f.data.clone())) } else { None }).collect();
    assert_eq!(fr[0], (65535, 1, true, vec![0x00, 0x9d, 1, 2, 3]), "넘침을 가로지른 조각 셋이 한 프레임");
    assert_eq!((fr[1].0, fr[1].1), (2, 2));
    assert_eq!(pl.counts().broken, 0);

    // 발언 파일(Ogg) — seq 범위가 넘침을 가로질러도 첫 · 끝이 그대로
    let tracks = serde_json::json!([{ "track_id": "slot-a", "kind": "audio" }]);
    let rows = serde_json::json!({ "100": { "track_id": "slot-a", "kind": "audio", "codec": "opus", "pt": 111 } });
    let mut a = vec![rec(1, 0, Channel::Track, track_list("r", tracks, rows)), rec(1, 0, Channel::Floor, br#"{"room":"r","event":"taken","user_id":"A","sources":[7],"seq":1}"#.to_vec())];
    for (i, sq) in [65533u16, 65534, 65535, 0, 1, 2].into_iter().enumerate() {
        a.push(rec(2 + i as u32, i as u64 * 20 * ms, Channel::Rtp(100), pkt(sq, i as u32 * 960, true, Some(7), &[0x78, i as u8])));
    }
    let d = tmp("seq-wrap-ogg");
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut sink = OggSink::new(&d).unwrap();
    for r in &a {
        for o in pl.push(r) {
            sink.write(&o).unwrap();
        }
    }
    for o in pl.finish() {
        sink.write(&o).unwrap();
    }
    let f = &sink.files()[0];
    assert_eq!((f.packets, f.first_seq, f.last_seq, f.filled), (6, Some(65533), Some(2), 0));
    std::fs::remove_dir_all(&d).ok();
}

// ── #4(202611/20261110e) — Matroska 사람마다 영상 · 음성 한 파일 ──

fn mkv_av_build(recs: &[Record], dir: &Path, policy: Policy, cfg: MkvConfig) -> (Vec<MkvFile>, Vec<String>) {
    let mut pl = Pipeline::new(policy);
    let mut sink = MkvSink::with(dir, cfg).unwrap();
    let mut asked = Vec::new();
    for r in recs {
        for o in pl.push(r) {
            sink.write(&o).unwrap();
        }
        asked.extend(sink.take_keyframe_requests());
    }
    for o in pl.finish() {
        sink.write(&o).unwrap();
    }
    sink.finish().unwrap();
    (sink.files().to_vec(), asked)
}

#[test]
fn mkv_person_gets_one_file_with_video_and_audio_on_one_timeline() {
    let d = tmp("mkv-av");
    let (fs, _) = mkv_av_build(&av_stream(6, 1000, None), &d, Policy::Utterance, MkvConfig::default());
    assert_eq!(fs.len(), 1, "사람 하나 = 파일 하나");
    let f = &fs[0];
    assert_eq!(f.path.file_name().unwrap(), "r-u1-0000.mkv");
    assert_eq!(f.tracks.iter().map(|t| t.kind.as_str()).collect::<Vec<_>>(), vec!["video", "audio"]);
    assert_eq!(f.base, "utc", "두 트랙 모두 SR — 한 시각축");
    let (v, a) = (&f.tracks[0], &f.tracks[1]);
    assert!(v.frames > 170 && a.frames > 290, "영상 {} · 음성 {}", v.frames, a.frames);
    // 같은 순간(0초)에 시작한 두 트랙 — 파일 원점에서 첫 블록까지가 같다(RTP 경과 · SR 기준)
    assert!(v.first_ms.abs_diff(a.first_ms) <= 1, "영상 첫 블록 {} · 음성 첫 블록 {}", v.first_ms, a.first_ms);
    let b = std::fs::read(&f.path).unwrap();
    assert!(b.windows(15).any(|w| w == b"V_MPEG4/ISO/AVC") && b.windows(6).any(|w| w == b"A_OPUS"));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn mkv_span_cuts_both_tracks_at_video_keyframe_and_requests_key() {
    let d = tmp("mkv-span");
    let cfg = MkvConfig { span: Some(Duration::from_secs(4)), ..MkvConfig::default() };
    let (fs, asked) = mkv_av_build(&av_stream(11, 1000, None), &d, Policy::Span { time_ns: u64::MAX, bytes: u64::MAX }, cfg);
    assert_eq!(fs.len(), 3, "4초 지난 첫 키(1초마다)에서 — 0 ~ 4 · 4 ~ 8 · 8 ~ 11");
    assert!(fs[..2].iter().all(|f| f.reason.as_deref() == Some("span")) && fs.iter().all(|f| f.tracks.len() == 2));
    assert_eq!(asked.len(), 3, "파일마다 T − 1초에 한 번");
    // ★트랙마다 이어진다 — 앞 파일의 그 트랙 끝(시작 벽시계 + 첫 블록 + 프레임 수 × 길이) = 다음 파일의 그 트랙 시작. 빈틈 · 겹침 없음
    //   (파일 원점은 앞 파일 음성 끝 — 영상은 자른 키프레임에서 시작해 원점에서 조금 뒤)
    for w in fs.windows(2) {
        for (i, per_ms) in [(0usize, 1000.0 / 30.0), (1, 20.0)] {
            let (a, b) = (&w[0].tracks[i], &w[1].tracks[i]);
            let end = w[0].start_ms as f64 + a.first_ms as f64 + a.frames as f64 * per_ms;
            let next = (w[1].start_ms + b.first_ms) as f64;
            assert!((end - next).abs() <= 2.0, "{} 트랙 앞 끝 {end:.1} · 다음 시작 {next:.1}", a.kind);
        }
    }
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn mkv_span_with_audio_clock_behind_video_places_every_block_on_its_sr_time() {
    // ★보내는 쪽 음성 시계가 영상보다 3% 느리다 — 음성 RTP 는 프레임마다 960 그대로인데 20.6ms 마다 온다(2층 장시간 판 실측: 봇 음성은 초당 49프레임 · 분당 1.4초 · 202611/20261110e).
    //   SR 은 그 시계대로 RTP ↔ 벽시계를 잇는다. 자른 키프레임 뒤에 오는 음성은 그 키보다 이른 시각이다 — 이어지는 파일을 키 하나로 열면 그 음성이 원점 앞으로 떨어져 0 에 눌렸다.
    let t0 = 1_800_000_000_000_000_000u64;
    let sr_a = 500 * 48 * 97 / 100;
    let mut recs = av_stream(13, 1000, None);
    for r in &mut recs {
        match r.channel {
            Channel::Rtp(100) => r.log_time = t0 + (r.log_time - t0) / 97 * 100,
            Channel::Sr if r.data.starts_with(br#"{"ssrc":100,"#) => {
                r.data = String::from_utf8(r.data.clone()).unwrap().replace(&format!(r#""rtp":{}"#, 500 * 48), &format!(r#""rtp":{sr_a}"#)).into_bytes();
            }
            _ => {}
        }
    }
    recs.sort_by_key(|r| (r.log_time, r.sequence));
    for (i, r) in recs.iter_mut().enumerate() {
        r.sequence = i as u32 + 1;
    }
    let d = tmp("mkv-span-drift");
    let cfg = MkvConfig { span: Some(Duration::from_secs(4)), ..MkvConfig::default() };
    let (fs, _) = mkv_av_build(&recs, &d, Policy::Span { time_ns: u64::MAX, bytes: u64::MAX }, cfg);
    assert!(fs.len() >= 3 && fs.iter().all(|f| f.base == "utc" && f.tracks.len() == 2));
    let t0_ms = (t0 / 1_000_000) as f64;
    for f in &fs {
        for t in &f.tracks {
            let (clock, sr_rtp) = if t.kind == "video" { (90.0, 1000.0 + 500.0 * 90.0) } else { (48.0, f64::from(sr_a)) };
            let want = t0_ms + 500.0 + (f64::from(t.first_rtp.unwrap()) - sr_rtp) / clock;
            let got = (f.start_ms + t.first_ms) as f64;
            assert!((got - want).abs() <= 1.5, "{} {} 첫 블록 {got} · SR 로 셈한 {want:.1}", f.path.display(), t.kind);
        }
    }
    let sent = recs.iter().filter(|r| r.channel == Channel::Rtp(100)).count() as u64;
    assert_eq!(fs.iter().map(|f| f.tracks[1].frames).sum::<u64>(), sent, "음성은 빠짐없이 어느 한 파일에(채움 없음 — 음성 RTP 는 이어진다)");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn mkv_log_time_file_with_late_sr_moves_video_onto_audio_and_anchors_date_on_audio() {
    // SR 이 여는 보류(2초) 뒤에 온다 → 파일은 log_time 기준. 영상이 60ms 늦게 닿아 도착 차만큼 어긋난다(D7)
    //   ★늦은 SR 맞춤(정:RULE-OPS-REC-MKV) — 두 트랙이 UTC 를 받으면 영상만 견본마다 5ms 이하로 옮겨 목표 −60ms 에 닿는다 · DateUTC 닻 = 음성
    let t0_ms = (1_800_000_000_000_000_000u64 / 1_000_000) as f64;
    let d = tmp("mkv-late-sr");
    let (fs, _) = mkv_av_build(&late_sr_av(5, 3_000, 60), &d, Policy::Utterance, MkvConfig::default());
    assert_eq!(fs.len(), 1);
    let f = &fs[0];
    assert_eq!((f.base, f.date, f.date_track), ("log_time", "utc", Some("audio")));
    assert!(f.video_shift_us.is_some_and(|x| (x + 60_000).abs() <= 1_000), "영상 목표 −60ms · {:?}", f.video_shift_us);
    for t in &f.tracks {
        let (clock, sr_rtp) = if t.kind == "video" { (90.0, 1000.0 + 500.0 * 90.0) } else { (48.0, 500.0 * 48.0) };
        let want = t0_ms + 500.0 + (f64::from(t.last_rtp.unwrap()) - sr_rtp) / clock;
        let got = (f.start_ms + t.last_ms) as f64;
        assert!((got - want).abs() <= 1.5, "{} 마지막 블록 {got} · SR 로 셈한 {want:.1}", t.kind);
    }
    let (fu, _) = mkv_av_build(&av_stream(5, 1000, None), &d, Policy::Utterance, MkvConfig::default());
    assert_eq!((fu[0].base, fu[0].date_track, fu[0].video_shift_us), ("utc", None, None), "utc 기준 파일은 닻 · 맞춤이 없다 — 모든 트랙이 SR 시각");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn mkv_slot_utterance_is_one_file_per_speaker_with_both_kinds() {
    // 무전 슬롯 — 화자 A(출처 7) 영상 · 음성 → 허가 B → 화자 B(출처 8). 발언 번호가 바뀐 세그먼트에서 새 파일 · 그 파일 영상은 B 의 첫 키프레임부터
    let ms = 1_000_000u64;
    let tracks = serde_json::json!([{ "track_id": "slot-a", "kind": "audio" }, { "track_id": "slot-v", "kind": "video" }]);
    let rows = serde_json::json!({ "100": { "track_id": "slot-a", "kind": "audio", "codec": "opus", "pt": 111 }, "200": vrow("slot-v") });
    // ★같은 화자라도 음성 · 영상 CSRC 가 다르다(실제 — 2층에서 잡힌 결함) · 화자 사용자는 Floor Taken 의 출처 번호로 안다
    let mut recs = vec![rec(1, 0, Channel::Track, track_list("r", tracks, rows))];
    let mut n = 2u32;
    let (mut aseq, mut vseq) = (0u16, 0u16);
    for (src, t0, who) in [(7u32, 0u32, "A"), (8, 3000, "B")] {
        recs.push(rec(n, u64::from(t0) * ms, Channel::Floor, format!(r#"{{"room":"r","event":"taken","user_id":"{who}","sources":[{src},{}],"seq":{n}}}"#, src + 10).into_bytes()));
        n += 1;
        for i in 0..100u32 {
            let t = t0 + i * 20;
            recs.push(rec(n, u64::from(t) * ms, Channel::Rtp(100), pkt(aseq, t * 48, true, Some(src), &[0x78, i as u8])));
            n += 1;
            aseq = aseq.wrapping_add(1);
            if i % 2 == 1 {
                // 영상은 40ms 마다 · 새 화자의 첫 영상은 델타 둘 뒤 키(키프레임 기다림)
                let vi = i / 2;
                let key = vi == 2 || vi == 30;
                recs.push(rec(n, u64::from(t) * ms, Channel::Rtp(200), pkt(vseq, t * 90, true, Some(src + 10), if key { KEY } else { DELTA })));
                n += 1;
                vseq = vseq.wrapping_add(1);
            }
        }
    }
    let d = tmp("mkv-slot");
    let (fs, _) = mkv_av_build(&recs, &d, Policy::Utterance, MkvConfig::default());
    assert_eq!(fs.iter().map(|f| f.source).collect::<Vec<_>>(), vec![Some(7), Some(8)], "발언 하나 = 파일 하나");
    assert!(fs.iter().all(|f| f.tracks.len() == 2), "파일마다 영상 · 음성");
    assert_eq!(fs[1].tracks.iter().find(|t| t.kind == "audio").unwrap().frames, 100, "B 의 음성은 첫 프레임부터 B 파일");
    assert_eq!(fs[0].reason.as_deref(), Some("taken"), "다음 허가에서 앞 발언 파일을 닫는다");
    std::fs::remove_dir_all(&d).ok();
    // 정:RULE-OPS-REC-MKV — 서버 발언 id 가 오면 무전 슬롯 파일 이름 = `{방}-{발언 번호}-{화자}.mkv`(Ogg 와 짝)
    let d = tmp("mkv-slot-talk");
    let (fs, _) = mkv_av_build(&with_talk(recs), &d, Policy::Utterance, MkvConfig::default());
    let names: Vec<String> = fs.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["r-t2-A.mkv".to_string(), "r-t153-B.mkv".to_string()], "발언 하나 = 파일 하나 · 이름에 발언 id 와 화자");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn mkv_av_is_deterministic_for_same_records() {
    let (a, b) = (tmp("mkv-av-d1"), tmp("mkv-av-d2"));
    let recs = av_stream(5, 1000, None);
    let (fa, _) = mkv_av_build(&recs, &a, Policy::Utterance, MkvConfig::default());
    let (fb, _) = mkv_av_build(&recs, &b, Policy::Utterance, MkvConfig::default());
    assert_eq!(std::fs::read(&fa[0].path).unwrap(), std::fs::read(&fb[0].path).unwrap());
    std::fs::remove_dir_all(&a).ok();
    std::fs::remove_dir_all(&b).ok();
}

fn names_of(paths: impl Iterator<Item = PathBuf>) -> Vec<String> {
    paths.map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
}

fn ogg_pages(b: &[u8]) -> Vec<ogg::Page> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(p) = ogg::read_page(b, at) {
        at = p.end;
        out.push(p);
    }
    assert_eq!(at, b.len(), "SDK:RULE-PLAT-REC-API 회수 — 모든 페이지가 온전하다(머리 · CRC)");
    out
}

fn ogg_closed(b: &[u8]) -> Vec<ogg::Page> {
    let pages = ogg_pages(b);
    assert!(pages.len() >= 3, "머리 둘 + 끝 페이지");
    assert!(pages.windows(2).all(|w| w[1].seq == w[0].seq + 1 && w[1].serial == w[0].serial), "일련번호가 이어진다");
    assert_eq!(pages.last().map(|p| p.flags & 0x04), Some(0x04), "SDK:RULE-PLAT-REC-API 회수 — 마지막 페이지에 EOS");
    assert_eq!(pages.iter().filter(|p| p.flags & 0x04 != 0).count(), 1, "EOS 는 하나");
    pages
}

fn media_pages(pages: &[ogg::Page]) -> u64 {
    pages.iter().skip(2).filter(|p| p.body > 0).count() as u64
}

fn segment_frames(out: &[Output]) -> u64 {
    out.iter().filter(|o| matches!(o, Output::Frame(f) if f.segment.is_some())).count() as u64
}

fn mkv_closed(b: &[u8]) -> (f64, usize) {
    let s = mkv::scan_mkv(b).expect("EBML · Segment · Info · Tracks");
    let (size, _) = mkv::read_size(b, s.seg_size_at as usize).unwrap();
    assert_eq!(size, Some(b.len() as u64 - s.seg_data), "정:RULE-OPS-REC-MKV 닫기 — Segment 크기 = 파일 끝(크기 모름 꼴이 아니다)");
    let mut at = s.seg_data as usize;
    let mut cue_points = None;
    while at < b.len() {
        let (id, n) = mkv::read_id(b, at).unwrap();
        let (size, k) = mkv::read_size(b, at + n).unwrap();
        let size = size.expect("닫은 파일의 최상위 요소는 크기를 안다") as usize;
        if id == 0x1C53_BB6B {
            cue_points = Some(b[at + n + k..at + n + k + size].iter().filter(|x| **x == 0xBB).count());
        }
        at += n + k + size;
    }
    assert_eq!(at, b.len(), "요소 크기를 따라 걸으면 파일 끝에 닿는다");
    let at = s.duration_at as usize;
    let dur = f64::from_be_bytes(b[at..at + 8].try_into().unwrap());
    (dur, cue_points.expect("정:RULE-OPS-REC-MKV 닫기 — Cues"))
}

fn feed_mkv(sink: &mut MkvSink, pl: &mut Pipeline, recs: &[Record]) {
    for r in recs {
        sink.record(r);
        for o in pl.push(r) {
            sink.write(&o).unwrap();
        }
    }
}

#[test]
fn result_name_inserts_next_number_before_extension() {
    let p = Path::new("/x/r-0000-A.ogg");
    assert_eq!(ogg::numbered(p, 0), p.to_path_buf(), "SDK:RULE-PLAT-REC-API 덮어쓰기 금지 — 첫 이름은 그대로");
    assert_eq!(ogg::numbered(p, 1), PathBuf::from("/x/r-0000-A-0001.ogg"));
    assert_eq!(ogg::numbered(Path::new("/x/r-u1-0000.mkv"), 12), PathBuf::from("/x/r-u1-0000-0012.mkv"));
    assert_eq!(ogg::numbered(Path::new("/x/plain"), 3), PathBuf::from("/x/plain-0003"));
}

#[test]
fn exclusive_open_never_truncates_and_takes_next_free_number() {
    let d = tmp("excl");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("a.ogg"), b"keep").unwrap();
    std::fs::write(d.join("a-0001.ogg"), b"keep1").unwrap();
    let (_, p) = ogg::open_dest(&d.join("a.ogg"), false, &mut ogg::Disk::default()).unwrap();
    assert_eq!(p, d.join("a-0002.ogg"), "SDK:RULE-PLAT-REC-API 덮어쓰기 금지 — 이름이 있으면 다음 번호");
    assert_eq!(std::fs::read(d.join("a.ogg")).unwrap(), b"keep", "있던 파일은 비우지 않는다");
    assert_eq!(std::fs::read(d.join("a-0001.ogg")).unwrap(), b"keep1");
    let (_, q) = ogg::open_dest(&d.join("b.mkv"), true, &mut ogg::Disk::default()).unwrap();
    assert_eq!(q, d.join("b.mkv"), "없으면 그 이름");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn retry_is_due_once_per_second_of_frame_clock() {
    let s = 5_000_000_000u64;
    assert!(ogg::retry_due(None, 0), "멈춘 적 없으면 연다");
    assert!(!ogg::retry_due(Some(s), s), "SDK:RULE-PLAT-REC-API 회전 — 실패한 그 프레임에서 다시 열지 않는다");
    assert!(!ogg::retry_due(Some(s), s + 999_999_999));
    assert!(ogg::retry_due(Some(s), s + 1_000_000_000), "프레임 시각으로 1초 뒤에 다시 연다");
    assert!(!ogg::retry_due(Some(s), s - 1), "시각이 뒤로 가면 기다린다");
}

#[test]
fn reopen_gate_waits_a_second_then_needs_free_space_after_a_space_stop() {
    let s = 5_000_000_000u64;
    let space = Some(ogg::Stall { at: s, space: true });
    let other = Some(ogg::Stall { at: s, space: false });
    let never = || -> Option<u64> { panic!("1초 전에는 남은 공간을 읽지 않는다") };
    assert_eq!(ogg::gate(None, s, never, 100), ogg::Gate::Open, "멈춘 적 없으면 연다");
    assert_eq!(ogg::gate(space, s + 999_999_999, never, 100), ogg::Gate::Wait, "SDK:RULE-PLAT-REC-API 회전 — 프레임 시각 1초가 확인 주기");
    assert_eq!(ogg::gate(space, s + 1_000_000_000, || Some(99), 100), ogg::Gate::Recheck, "SDK:RULE-PLAT-REC-API 회전 — 디스크 부족으로 멈췄으면 여유가 돌아와야 연다");
    assert_eq!(ogg::gate(space, s + 1_000_000_000, || Some(100), 100), ogg::Gate::Open, "여유가 하한에 닿으면 다음 파일");
    assert_eq!(ogg::gate(space, s + 1_000_000_000, || None, 100), ogg::Gate::Open, "공간을 못 읽으면 열어 본다");
    assert_eq!(ogg::gate(other, s + 1_000_000_000, || Some(0), 100), ogg::Gate::Open, "공간 탓이 아닌 멈춤은 1초 뒤 다시 연다");
}

#[test]
fn only_space_errors_or_errors_on_an_open_file_take_the_rotation_path() {
    use std::io::ErrorKind as K;
    for k in [K::StorageFull, K::WriteZero, K::QuotaExceeded, K::FileTooLarge] {
        assert!(ogg::rotates(k, false), "SDK:RULE-PLAT-REC-API 회전 — 새 파일을 열다 공간이 모자람({k:?})");
    }
    for k in [K::NotFound, K::PermissionDenied, K::ReadOnlyFilesystem, K::InvalidInput, K::Other] {
        assert!(!ogg::rotates(k, false), "새 파일을 열지 못한 다른 까닭({k:?})은 앱에 돌려준다");
        assert!(ogg::rotates(k, true), "SDK:RULE-PLAT-REC-API 회전 — 연 파일에 쓰기 실패는 까닭이 무엇이든 회전({k:?})");
    }
}

#[test]
fn new_sinks_on_the_same_folder_write_new_files() {
    let d = tmp("ogg-again");
    let mut seen = Vec::new();
    for _ in 0..2 {
        let mut sink = OggSink::new(&d).unwrap();
        for o in run_pipe(Policy::Utterance, &stream()) {
            sink.write(&o).unwrap();
        }
        seen.push(names_of(sink.files().iter().map(|f| f.path.clone())));
    }
    assert_eq!(seen[0], vec!["r-0000-A.ogg".to_string(), "r-0002-B.ogg".to_string()]);
    assert_eq!(seen[1], vec!["r-0000-A-0001.ogg".to_string(), "r-0002-B-0001.ogg".to_string()], "SDK:RULE-PLAT-REC-API 덮어쓰기 금지 — 새로 지은 뒤의 녹음은 늘 새 파일");
    assert_eq!(std::fs::read(d.join("r-0000-A.ogg")).unwrap(), std::fs::read(d.join("r-0000-A-0001.ogg")).unwrap(), "앞 파일은 그대로");
    std::fs::remove_dir_all(&d).ok();
    let d = tmp("mkv-again");
    let recs = av_stream(2, 1000, None);
    let (a, _) = mkv_av_build(&recs, &d, Policy::Utterance, MkvConfig::default());
    let (b, _) = mkv_av_build(&recs, &d, Policy::Utterance, MkvConfig::default());
    assert_eq!(names_of(a.iter().map(|f| f.path.clone())), vec!["r-u1-0000.mkv".to_string()]);
    assert_eq!(names_of(b.iter().map(|f| f.path.clone())), vec!["r-u1-0000-0001.mkv".to_string()], "SDK:RULE-PLAT-REC-API 덮어쓰기 금지 — Matroska 도 다음 번호");
    assert_eq!(std::fs::read(&a[0].path).unwrap(), std::fs::read(&b[0].path).unwrap());
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn ogg_write_failure_closes_disk_low_waits_a_second_and_opens_next_number() {
    let d = tmp("ogg-fail");
    let out = run_pipe(Policy::Utterance, &long_opus(600));
    let mut sink = OggSink::with(&d, OggConfig { min_free_bytes: 0 }).unwrap();
    sink.inject(ogg::Disk::full_after(10_000, false));
    for o in &out {
        sink.write(o).unwrap();
    }
    let fs = sink.files().to_vec();
    assert_eq!(names_of(fs.iter().map(|f| f.path.clone())), vec!["r-0000-A.ogg".to_string(), "r-0000-A-0001.ogg".to_string()], "SDK:RULE-PLAT-REC-API 회전 — 다음 파일은 다음 번호");
    assert_eq!(fs[0].reason.as_deref(), Some("disk_low"), "SDK:RULE-PLAT-REC-SPLIT 결과물 닫힌 까닭 — 쓰기 실패");
    assert_eq!(fs[1].reason.as_deref(), Some("end"));
    assert_eq!(sink.dropped(), 50, "쓰다 실패한 프레임 하나 + 그 뒤 1초(20ms 마다 49개)는 버리고 센다");
    assert_eq!(fs[0].packets + fs[1].packets + sink.dropped(), segment_frames(&out), "담은 프레임 + 버린 프레임 = 받은 프레임");
    for f in &fs {
        let pages = ogg_closed(&std::fs::read(&f.path).unwrap());
        assert_eq!(media_pages(&pages), f.packets, "SDK:RULE-PLAT-REC-API 회전 — 정상으로 닫았다(끝 표지)");
        assert_eq!(pages.last().unwrap().granule, f.samples);
    }
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn ogg_close_failure_leaves_file_and_recovery_finishes_it() {
    let d = tmp("ogg-fail-hard");
    let out = run_pipe(Policy::Utterance, &long_opus(1500));
    let half = out.len() * 4 / 5;
    let mut sink = OggSink::with(&d, OggConfig { min_free_bytes: 0 }).unwrap();
    sink.inject(ogg::Disk::full_after(10_000, true));
    for o in &out[..half] {
        sink.write(o).unwrap();
    }
    let first = sink.files()[0].clone();
    assert_eq!(sink.files().len(), 1, "SDK:RULE-PLAT-REC-API 회전 — 디스크가 계속 차 있으면 다음 파일이 생기지 않는다");
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "1초마다 열어 본 파일은 머리를 못 쓰면 지운다 — 쌓이지 않는다");
    assert_eq!(first.reason.as_deref(), Some("disk_low"), "SDK:RULE-PLAT-REC-API 회전 — 닫기마저 실패해도 까닭은 disk_low");
    let b = std::fs::read(&first.path).unwrap();
    let s = ogg::scan_ogg(&b);
    assert_eq!((s.cut, s.eos), (b.len() as u64, false), "닫기마저 실패하면 그 파일을 그대로 둔다 — 온전한 페이지까지 · 끝 표지 없음");
    assert_eq!((s.frames, s.granule), (first.packets, first.samples), "SDK:RULE-PLAT-REC-API 회전 — 셈은 디스크에 있는 그대로");
    sink.inject(ogg::Disk::default());
    for o in &out[half..] {
        sink.write(o).unwrap();
    }
    let fs = sink.files().to_vec();
    assert_eq!(names_of(fs.iter().map(|f| f.path.clone())), vec!["r-0000-A.ogg".to_string(), "r-0000-A-0001.ogg".to_string()], "실패한 열기는 번호를 쓰지 않는다");
    assert_eq!(fs[0].packets + fs[1].packets + sink.dropped(), segment_frames(&out), "담은 프레임 + 버린 프레임 = 받은 프레임");
    let r = recover_ogg(&first.path).unwrap();
    let pages = ogg_closed(&std::fs::read(&first.path).unwrap());
    assert_eq!(r.media_frames, media_pages(&pages), "SDK:RULE-PLAT-REC-API 회수 — 프레임 수");
    assert_eq!(r.media_frames, first.packets);
    assert_eq!(r.duration_ms, r.media_frames * 20, "길이 = 마지막 그래뉼 / 48");
    assert_eq!(r.reason, "crash");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn ogg_space_stop_reopens_only_when_free_space_is_back() {
    let d = tmp("ogg-floor");
    let out = run_pipe(Policy::Utterance, &long_opus(600));
    let mut sink = OggSink::with(&d, OggConfig { min_free_bytes: u64::MAX }).unwrap();
    sink.inject(ogg::Disk::full_after(10_000, false));
    for o in &out {
        sink.write(o).unwrap();
    }
    assert_eq!(sink.files().len(), 1, "SDK:RULE-PLAT-REC-API 회전 — 디스크 부족이면 닫고 멈춘다 · 여유가 하한 밑이면 열지 않는다");
    assert_eq!(sink.files()[0].reason.as_deref(), Some("disk_low"));
    assert_eq!(sink.files()[0].packets + sink.dropped(), segment_frames(&out));
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn ogg_recovery_closes_a_crashed_copy_and_a_cut_tail() {
    let d = tmp("ogg-crash");
    let out = run_pipe(Policy::Utterance, &long_opus(600));
    let mut sink = OggSink::new(d.join("live")).unwrap();
    for o in out.iter().filter(|o| !matches!(o, Output::Segment(s) if s.event == SegmentKind::Closed)) {
        sink.write(o).unwrap();
    }
    let live = d.join("live").join("r-0000-A.ogg");
    let whole = d.join("whole.ogg");
    std::fs::copy(&live, &whole).unwrap();
    let r = recover_ogg(&whole).unwrap();
    let pages = ogg_closed(&std::fs::read(&whole).unwrap());
    assert_eq!(r.media_frames, media_pages(&pages));
    assert!(r.media_frames > 400 && r.media_frames < 600, "버퍼에 남은 꼬리는 없다 — {}", r.media_frames);
    assert_eq!(r.duration_ms, pages.last().unwrap().granule / 48);
    let cut = d.join("cut.ogg");
    let b = std::fs::read(&live).unwrap();
    std::fs::write(&cut, &b[..b.len() - 5]).unwrap();
    let rc = recover_ogg(&cut).unwrap();
    let cb = std::fs::read(&cut).unwrap();
    ogg_closed(&cb);
    assert_eq!(rc.media_frames, r.media_frames - 1, "SDK:RULE-PLAT-REC-API 회수 — 잘린 페이지만 잘라낸다");
    let kept = ogg::scan_ogg(&b[..b.len() - 5]).cut as usize;
    assert_eq!(&cb[..kept], &b[..kept], "그 앞은 바꾸지 않는다");
    let again = recover_ogg(&cut).unwrap();
    assert_eq!((again.media_frames, std::fs::read(&cut).unwrap()), (rc.media_frames, cb), "닫힌 파일을 다시 회수해도 같다");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn ogg_recovery_of_a_file_without_media_says_zero() {
    let d = tmp("ogg-empty");
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join("h.ogg");
    let mut b = ogg::page_bytes(b"OpusHead", 0x02, 0, 7, 0);
    b.extend(ogg::page_bytes(b"OpusTags", 0x00, 0, 7, 1));
    std::fs::write(&p, &b).unwrap();
    let r = recover_ogg(&p).unwrap();
    assert_eq!((r.media_frames, r.duration_ms), (0, 0), "SDK:RULE-PLAT-REC-API 회수 — 미디어 프레임이 0 이면 그렇다고 돌려준다");
    let pages = ogg_closed(&std::fs::read(&p).unwrap());
    assert_eq!((pages.len(), pages[2].serial, pages[2].seq), (3, 7, 2), "같은 일련 · 다음 순번의 빈 EOS 페이지");
    let e = d.join("e.ogg");
    std::fs::write(&e, b"OggS\0").unwrap();
    assert_eq!(recover_ogg(&e).unwrap_err().kind(), std::io::ErrorKind::InvalidData, "온전한 페이지가 하나도 없으면 회수하지 못한다(Matroska 와 같다)");
    assert_eq!(std::fs::read(&e).unwrap(), b"OggS\0", "SDK:RULE-PLAT-REC-API 회수 — 지우지 않는다 · 파일을 건드리지 않는다");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn ogg_scan_stops_at_first_page_with_bad_crc() {
    let mut b = ogg::page_bytes(b"OpusHead", 0x02, 0, 3, 0);
    b.extend(ogg::page_bytes(b"OpusTags", 0x00, 0, 3, 1));
    let two = b.len() as u64;
    b.extend(ogg::page_bytes(&[0x78, 1, 2], 0x00, 960, 3, 2));
    b.extend(ogg::page_bytes(&[0x78, 3, 4], 0x00, 1920, 3, 3));
    let s = ogg::scan_ogg(&b);
    assert_eq!((s.pages, s.frames, s.granule, s.next_seq, s.eos), (4, 2, 1920, 4, false));
    let mut bad = b.clone();
    let at = two as usize + 28;
    bad[at] ^= 0xFF;
    let s = ogg::scan_ogg(&bad);
    assert_eq!((s.cut, s.pages, s.frames, s.granule), (two, 2, 0, 0), "CRC 가 틀린 페이지에서 멈춘다");
    let mut other = ogg::page_bytes(b"OpusHead", 0x02, 0, 3, 0);
    other.extend(ogg::page_bytes(b"OpusTags", 0x00, 0, 4, 1));
    assert_eq!(ogg::scan_ogg(&other).pages, 1, "일련이 바뀌면 멈춘다");
}

#[test]
fn ogg_app_close_gives_its_reason_and_never_reopens() {
    let out = run_pipe(Policy::Utterance, &long_opus(100));
    for (reason, want) in [(Some("app_stop"), "app_stop"), (None, "end")] {
        let d = tmp("ogg-close");
        let mut sink = OggSink::new(&d).unwrap();
        for o in &out[..60] {
            sink.write(o).unwrap();
        }
        sink.close(reason).unwrap();
        for o in &out[60..] {
            sink.write(o).unwrap();
        }
        assert_eq!(sink.files().len(), 1, "SDK:RULE-PLAT-REC-API 회전 — 앱이 닫는 것은 회전이 아니다(닫고 열지 않는다)");
        assert_eq!(sink.files()[0].reason.as_deref(), Some(want), "SDK:RULE-PLAT-REC-API 회전 — 앱이 준 까닭 · 없으면 end");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        ogg_closed(&std::fs::read(&sink.files()[0].path).unwrap());
        std::fs::remove_dir_all(&d).ok();
    }
}

#[test]
fn mkv_recovery_rewrites_a_crashed_copy_like_close() {
    let d = tmp("mkv-crash");
    let recs = av_stream(4, 1000, None);
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut sink = MkvSink::new(d.join("live")).unwrap();
    feed_mkv(&mut sink, &mut pl, &recs);
    let live = d.join("live").join("r-u1-0000.mkv");
    let crashed = std::fs::read(&live).unwrap();
    assert_eq!(&crashed[crashed.len() / 2..].windows(8).filter(|w| *w == [0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]).count(), &1, "열린 클러스터 하나만 크기 모름");
    let copy = d.join("copy.mkv");
    std::fs::write(&copy, &crashed).unwrap();
    for o in pl.finish() {
        sink.write(&o).unwrap();
    }
    sink.finish().unwrap();
    let closed = sink.files()[0].clone();
    let r = recover_mkv(&copy).unwrap();
    let b = std::fs::read(&copy).unwrap();
    let (dur, cues) = mkv_closed(&b);
    assert_eq!(r.media_frames, closed.tracks.iter().map(|t| t.frames).sum::<u64>(), "SDK:RULE-PLAT-REC-API 회수 — 블록 수 = 프레임 수");
    assert!(dur > 3_000.0 && dur as u64 == r.duration_ms, "Duration {dur}");
    assert!(closed.duration_ms >= r.duration_ms && closed.duration_ms - r.duration_ms <= 40, "회수 길이는 마지막 블록 시각 — 닫은 길이 {} · 회수 {}", closed.duration_ms, r.duration_ms);
    assert_eq!(cues, 5, "키프레임 클러스터마다 Cue(1초 GOP · 0 ~ 3.96초 키 다섯)");
    let good = std::fs::read(&closed.path).unwrap();
    let s = mkv::scan_mkv(&good).unwrap();
    let at = s.duration_at as usize;
    assert_eq!(good.len(), b.len());
    assert!(good.iter().zip(&b).enumerate().all(|(i, (x, y))| x == y || (at..at + 8).contains(&i)), "정:RULE-OPS-REC-MKV 닫기 — 같은 고쳐 쓰기(Duration 만 다르다)");
    let cut = d.join("cut.mkv");
    std::fs::write(&cut, &crashed[..crashed.len() - 3]).unwrap();
    let rc = recover_mkv(&cut).unwrap();
    mkv_closed(&std::fs::read(&cut).unwrap());
    assert_eq!(rc.media_frames, r.media_frames - 1, "SDK:RULE-PLAT-REC-API 회수 — 잘린 블록만 잘라낸다");
    assert_eq!(rc.reason, "crash");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn mkv_write_failure_closes_disk_low_and_opens_next_file_after_a_second() {
    let d = tmp("mkv-fail");
    let recs = av_stream(6, 1000, None);
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut sink = MkvSink::with(&d, MkvConfig { min_free_bytes: 0, ..MkvConfig::default() }).unwrap();
    sink.inject(ogg::Disk::full_after(4_000, false));
    let mut fed = 0;
    for r in &recs {
        for o in pl.push(r) {
            fed += segment_frames(std::slice::from_ref(&o));
            sink.write(&o).unwrap();
        }
    }
    for o in pl.finish() {
        fed += segment_frames(std::slice::from_ref(&o));
        sink.write(&o).unwrap();
    }
    sink.finish().unwrap();
    let fs = sink.files().to_vec();
    assert_eq!(names_of(fs.iter().map(|f| f.path.clone())), vec!["r-u1-0000.mkv".to_string(), "r-u1-0001.mkv".to_string()], "SDK:RULE-PLAT-REC-API 회전 — 다음 파일");
    assert_eq!(fs[0].reason.as_deref(), Some("disk_low"));
    assert_eq!(fs[1].reason.as_deref(), Some("end"));
    assert!(sink.dropped() > 50, "1초 동안 버리고 센다 — {}", sink.dropped());
    assert_eq!(mkv_written(&fs) + sink.dropped() + sink.before_key(), fed, "담은 블록 + 버린 프레임 + 키 전 영상 = 받은 프레임(되돌린 블록은 버린 쪽)");
    assert_eq!(mkv_written(&fs[..1]), mkv::scan_mkv(&std::fs::read(&fs[0].path).unwrap()).unwrap().blocks, "SDK:RULE-PLAT-REC-API 회전 — 셈은 디스크에 있는 블록 그대로");
    for f in &fs {
        let (dur, cues) = mkv_closed(&std::fs::read(&f.path).unwrap());
        assert!(dur > 0.0 && cues > 0, "SDK:RULE-PLAT-REC-API 회전 — 정상으로 닫았다");
    }
    let gap = fs[1].start_ms - (fs[0].start_ms + fs[0].duration_ms);
    assert!((900..1_500).contains(&gap), "다음 파일은 실패한 프레임의 `log_time` 에서 1초 뒤 — 앞 파일 끝(마지막 블록 + 33ms)과의 틈 {gap}ms");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn mkv_close_failure_leaves_file_and_recovery_finishes_it() {
    let d = tmp("mkv-fail-hard");
    let recs = av_stream(12, 1000, None);
    let half = recs.len() * 3 / 4;
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut sink = MkvSink::with(&d, MkvConfig { min_free_bytes: 0, ..MkvConfig::default() }).unwrap();
    sink.inject(ogg::Disk::full_after(4_000, true));
    let mut fed = 0;
    for r in &recs[..half] {
        for o in pl.push(r) {
            fed += segment_frames(std::slice::from_ref(&o));
            sink.write(&o).unwrap();
        }
    }
    assert_eq!(sink.files().len(), 1, "SDK:RULE-PLAT-REC-API 회전 — 디스크가 계속 차 있으면 다음 파일이 생기지 않는다");
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "1초마다 열어 본 파일은 머리를 못 쓰면 지운다 — 쌓이지 않는다");
    let first = sink.files()[0].clone();
    assert_eq!(first.reason.as_deref(), Some("disk_low"), "SDK:RULE-PLAT-REC-API 회전 — 닫기마저 실패해도 까닭은 disk_low");
    let b = std::fs::read(&first.path).unwrap();
    let s = mkv::scan_mkv(&b).unwrap();
    assert_eq!(mkv::read_size(&b, s.seg_size_at as usize).unwrap().0, None, "닫기마저 실패하면 그 파일을 그대로 둔다(크기 모름)");
    assert_eq!(s.blocks, mkv_written(std::slice::from_ref(&first)), "SDK:RULE-PLAT-REC-API 회전 — 셈은 디스크에 있는 블록 그대로");
    sink.inject(ogg::Disk::default());
    for r in &recs[half..] {
        for o in pl.push(r) {
            fed += segment_frames(std::slice::from_ref(&o));
            sink.write(&o).unwrap();
        }
    }
    for o in pl.finish() {
        fed += segment_frames(std::slice::from_ref(&o));
        sink.write(&o).unwrap();
    }
    sink.finish().unwrap();
    let fs = sink.files().to_vec();
    assert_eq!(names_of(fs.iter().map(|f| f.path.clone())), vec!["r-u1-0000.mkv".to_string(), "r-u1-0001.mkv".to_string()], "실패한 열기는 파일 번호를 쓰지 않는다");
    assert_eq!(mkv_written(&fs) + sink.dropped() + sink.before_key(), fed, "담은 블록 + 버린 프레임 + 키 전 영상 = 받은 프레임");
    let r = recover_mkv(&first.path).unwrap();
    let (dur, cues) = mkv_closed(&std::fs::read(&first.path).unwrap());
    assert!(dur > 0.0 && cues > 0, "SDK:RULE-PLAT-REC-API 회수 — {r:?}");
    assert_eq!(r.media_frames, s.blocks);
    std::fs::remove_dir_all(&d).ok();
}

fn mkv_written(fs: &[MkvFile]) -> u64 {
    fs.iter().flat_map(|f| f.tracks.iter()).map(|t| t.frames).sum()
}

#[test]
fn mkv_failure_while_a_new_kind_opens_still_takes_the_new_track() {
    let d = tmp("mkv-late-kind");
    let ms = 1_000_000u64;
    let t0 = 1_800_000_000_000_000_000u64;
    let recs: Vec<Record> = av_stream(8, 1000, None).into_iter().filter(|r| r.channel != Channel::Rtp(200) || r.log_time >= t0 + 3_000 * ms).collect();
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut sink = MkvSink::with(&d, MkvConfig { min_free_bytes: 0, ..MkvConfig::default() }).unwrap();
    let disk = ogg::Disk::full_after(u64::MAX, true);
    sink.inject(disk.clone());
    let mut fed = 0;
    let mut hit = false;
    for r in &recs {
        for o in pl.push(r) {
            fed += segment_frames(std::slice::from_ref(&o));
            let video_opens = matches!(&o, Output::Segment(s) if s.event == SegmentKind::Opened && s.kind == "video");
            if video_opens {
                assert_eq!(sink.open_files(), 1, "stand check — 음성만으로 연 파일이 있다");
                disk.fill();
                hit = true;
            }
            sink.write(&o).unwrap();
            if video_opens {
                sink.inject(ogg::Disk::default());
            }
        }
    }
    for o in pl.finish() {
        fed += segment_frames(std::slice::from_ref(&o));
        sink.write(&o).unwrap();
    }
    sink.finish().unwrap();
    assert!(hit, "stand check — 영상 세그먼트가 열렸다");
    let fs = sink.files().to_vec();
    assert_eq!(fs.len(), 2, "{fs:?}");
    assert_eq!((fs[0].reason.as_deref(), fs[0].tracks.len()), (Some("disk_low"), 1), "앞 파일(음성만)을 닫다 실패");
    let kinds: Vec<&str> = fs[1].tracks.iter().map(|t| t.kind.as_str()).collect();
    assert_eq!(kinds, vec!["video", "audio"], "SDK:RULE-PLAT-REC-API 회전 — 열기 · 닫기가 실패해도 묶음은 새 영상 트랙을 받는다");
    assert!(fs[1].tracks[0].frames > 50, "영상 프레임이 사라지지 않는다(1초 멈춤 뒤 다음 키 5.94초부터 8초까지) — {}", fs[1].tracks[0].frames);
    assert_eq!(mkv_written(&fs) + sink.dropped() + sink.before_key(), fed, "담은 블록 + 버린 프레임 + 키 전 영상 = 받은 프레임");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn sinks_return_errors_that_are_not_a_full_disk() {
    let d = tmp("ogg-gone");
    let out = run_pipe(Policy::Utterance, &long_opus(100));
    let mut sink = OggSink::new(&d).unwrap();
    std::fs::remove_dir_all(&d).unwrap();
    let errs: Vec<std::io::ErrorKind> = out.iter().filter_map(|o| sink.write(o).err().map(|e| e.kind())).collect();
    assert_eq!(errs, vec![std::io::ErrorKind::NotFound; 2], "폴더가 없으면 회전이 아니다 — 앱에 돌려준다(1초마다 한 번)");
    assert!(sink.files().is_empty());
    assert_eq!(sink.dropped(), segment_frames(&out));
    let d = tmp("mkv-gone");
    let recs = av_stream(2, 1000, None);
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut sink = MkvSink::new(&d).unwrap();
    std::fs::remove_dir_all(&d).unwrap();
    let mut errs = Vec::new();
    for r in &recs {
        for o in pl.push(r) {
            errs.extend(sink.write(&o).err().map(|e| e.kind()));
        }
    }
    assert!(!errs.is_empty() && errs.iter().all(|k| *k == std::io::ErrorKind::NotFound), "Matroska 도 앱에 돌려준다 — {errs:?}");
    assert!(sink.files().is_empty() && !d.exists(), "폴더를 새로 짓지도 파일을 남기지도 않는다");
}

#[test]
fn mkv_app_close_gives_its_reason_and_never_reopens() {
    let d = tmp("mkv-close");
    let recs = av_stream(4, 1000, None);
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut sink = MkvSink::new(&d).unwrap();
    feed_mkv(&mut sink, &mut pl, &recs[..recs.len() / 2]);
    sink.close(Some("app_stop")).unwrap();
    feed_mkv(&mut sink, &mut pl, &recs[recs.len() / 2..]);
    for o in pl.finish() {
        sink.write(&o).unwrap();
    }
    sink.finish().unwrap();
    assert_eq!(sink.files().len(), 1, "SDK:RULE-PLAT-REC-API 회전 — 닫고 열지 않는다");
    assert_eq!(sink.files()[0].reason.as_deref(), Some("app_stop"), "앱이 준 까닭");
    assert_eq!(sink.open_files(), 0);
    mkv_closed(&std::fs::read(&sink.files()[0].path).unwrap());
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn coverage_tracks_first_listed_and_first_loaded_of_personal_tracks() {
    let ms = 1_000_000u64;
    let t0 = 7_000 * ms;
    let list = |extra: bool| {
        let mut t = vec![
            serde_json::json!({ "track_id": "slot-a", "kind": "audio" }),
            serde_json::json!({ "track_id": "cam-u1", "kind": "video", "user_id": "u1" }),
            serde_json::json!({ "track_id": "mic-u1", "kind": "audio", "user_id": "u1" }),
            serde_json::json!({ "track_id": "dc-u1", "kind": "data", "user_id": "u1" }),
        ];
        if extra {
            t.push(serde_json::json!({ "track_id": "cam-u2", "kind": "video", "user_id": "u2" }));
        }
        track_list("r", serde_json::Value::Array(t), serde_json::json!({}))
    };
    let mut c = cover::Cover::default();
    c.seen(t0);
    c.listed(t0 + 5 * ms, &list(false));
    c.loaded(t0 + 40 * ms, "cam-u1");
    c.loaded(t0 + 90 * ms, "cam-u1");
    c.loaded(t0 + 50 * ms, "slot-a");
    c.listed(t0 + 100 * ms, &list(true));
    c.listed(t0 + 150 * ms, b"not json");
    c.loaded(t0 + 300 * ms, "cam-u2");
    let got = c.summary();
    let row = |id: &str, user: &str, kind: &str, listed: u64, loaded: Option<u64>| cover::TrackCoverage { track_id: id.into(), user_id: user.into(), kind: kind.into(), listed_ms: listed, loaded_ms: loaded };
    assert_eq!(
        got.tracks,
        vec![row("cam-u1", "u1", "video", 5, Some(40)), row("cam-u2", "u2", "video", 100, Some(300)), row("mic-u1", "u1", "audio", 5, None)],
        "SDK:RULE-PLAT-REC-API 트랙 덮음 요약 — 개인 트랙마다 처음 실린 때(첫 레코드부터 ms) · 무전 슬롯 · 데이터는 밖"
    );
    assert_eq!(got.not_recorded, vec!["mic-u1".to_string()], "SDK:RULE-PLAT-REC-API 트랙 덮음 요약 — 끝내 못 실린 트랙");
}

#[test]
fn mkv_sink_reports_coverage_from_records_and_frames() {
    let d = tmp("mkv-cover");
    let ms = 1_000_000u64;
    let mut recs = av_stream(2, 1000, None);
    let t0 = recs[0].log_time;
    let more = serde_json::json!([
        { "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 },
        { "track_id": "mic-u1", "kind": "audio", "user_id": "u1", "csrc": 10 },
        { "track_id": "cam-u2", "kind": "video", "user_id": "u2", "csrc": 11 }
    ]);
    let rows = serde_json::json!({ "200": { "track_id": "cam-u1", "kind": "video", "codec": "H264", "pt": 102 }, "100": { "track_id": "mic-u1", "kind": "audio", "codec": "opus", "pt": 111 } });
    let at = recs.iter().position(|r| r.log_time >= t0 + 250 * ms).unwrap();
    recs.insert(at, rec(0, recs[at].log_time, Channel::Track, track_list("r", more, rows)));
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut sink = MkvSink::new(&d).unwrap();
    feed_mkv(&mut sink, &mut pl, &recs);
    let c = sink.coverage();
    let ids: Vec<(&str, u64, Option<u64>)> = c.tracks.iter().map(|t| (t.track_id.as_str(), t.listed_ms, t.loaded_ms)).collect();
    assert_eq!(ids.len(), 3);
    assert_eq!((ids[0].0, ids[0].1), ("cam-u1", 0));
    assert!(ids[0].2.is_some_and(|x| x < 50), "{ids:?}");
    assert_eq!((ids[1].0, ids[1].1, ids[1].2), ("cam-u2", recs[at].log_time.saturating_sub(t0) / ms, None));
    assert_eq!((ids[2].0, ids[2].1), ("mic-u1", 0));
    assert!(ids[2].2.is_some_and(|x| x < 50), "{ids:?}");
    assert_eq!(c.not_recorded, vec!["cam-u2".to_string()], "SDK:RULE-PLAT-REC-API 트랙 덮음 요약 — 끝내 못 실린 트랙");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn key_request_wants_from_target_minus_lead_until_a_key_at_target() {
    use KeyAsk::*;
    let k = |st, key, el| key_request(st, key, el, 4_000, 1_000);
    assert_eq!(k(Idle, false, 2_999), (false, Idle), "정:RULE-OPS-REC-MKV before T - 1");
    assert_eq!(k(Idle, false, 3_000), (true, Wanting), "정:RULE-OPS-REC-MKV T - 1 asks and starts wanting");
    assert_eq!(k(Idle, true, 3_000), (true, Wanting), "a key exactly at T - 1 was not answered yet - ask");
    assert_eq!(k(Wanting, false, 3_500), (false, Wanting), "the cleanup layer keeps asking each interval until a key");
    assert_eq!(k(Wanting, true, 3_150), (true, Wanting), "a key before T does not end the want - ask again");
    assert_eq!(k(Wanting, true, 3_900), (true, Wanting), "no cap on asks");
    assert_eq!(k(Wanting, false, 9_000), (false, Wanting), "without a key the cleanup layer keeps the earlier ask alive");
    assert_eq!(k(Wanting, true, 4_000), (false, Wanting), "a key at T cuts - the sink resets");
    assert_eq!(k(Idle, true, 0), (false, Idle), "the cutting key opens the next segment unasked");
    assert_eq!(key_request(Idle, true, 0, 1_000, 1_000), (true, Wanting), "T at or under the lead asks at once");
}

fn pli_step(now: u32, want: bool, latency: u32, drop: &[usize], plis: &mut Vec<u32>, due: &mut Vec<u32>) {
    if want && plis.last().is_none_or(|t| now.saturating_sub(*t) >= 1_000) {
        if !drop.contains(&plis.len()) {
            due.push(now + latency);
        }
        plis.push(now);
    }
}

fn av_on_request(secs: u32, sps_switch_at: Option<u32>, latency: u32, feed: &mut dyn FnMut(&Record) -> usize) -> Vec<u32> {
    av_on_request_dropping(secs, sps_switch_at, latency, &[], feed)
}

fn av_on_request_dropping(secs: u32, sps_switch_at: Option<u32>, latency: u32, drop: &[usize], feed: &mut dyn FnMut(&Record) -> usize) -> Vec<u32> {
    let ms = 1_000_000u64;
    let t0 = 1_800_000_000_000_000_000u64;
    let tracks = serde_json::json!([
        { "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 },
        { "track_id": "mic-u1", "kind": "audio", "user_id": "u1", "csrc": 10 }
    ]);
    let rows = serde_json::json!({ "200": { "track_id": "cam-u1", "kind": "video", "codec": "H264", "pt": 102 }, "100": { "track_id": "mic-u1", "kind": "audio", "codec": "opus", "pt": 111 } });
    let (mut plis, mut due) = (Vec::new(), Vec::new());
    let mut n = 1u32;
    let mut want = feed(&rec(n, t0, Channel::Track, track_list("r", tracks, rows))) > 0;
    pli_step(0, want, latency, drop, &mut plis, &mut due);
    n += 1;
    let (mut vseq, mut aseq) = (0u16, 0u16);
    let (mut tv, mut ta) = (0u32, 0u32);
    let ntp = |unix_ns: u64| -> u64 { (((unix_ns / 1_000_000_000) + 2_208_988_800) << 32) | (((unix_ns % 1_000_000_000) << 32) / 1_000_000_000) };
    let (mut sr_done, mut switched) = (false, false);
    while tv.min(ta) < secs * 1000 {
        let now = tv.min(ta);
        let mut out: Vec<Record> = Vec::new();
        if !sr_done && now >= 500 {
            sr_done = true;
            let at = t0 + 500 * ms;
            out.push(rec(n, at, Channel::Sr, format!(r#"{{"ssrc":200,"ntp":{},"rtp":{}}}"#, ntp(at), 1000u32.wrapping_add(500 * 90)).into_bytes()));
            out.push(rec(n + 1, at, Channel::Sr, format!(r#"{{"ssrc":100,"ntp":{},"rtp":{}}}"#, ntp(at), 500 * 48).into_bytes()));
            n += 2;
        }
        if tv <= ta {
            let i = tv / 33;
            let switch = sps_switch_at.is_some_and(|s| tv >= s) && !switched;
            switched |= switch;
            let answered = due.iter().any(|d| *d <= tv);
            due.retain(|d| *d > tv);
            let key = i == 0 || answered || switch;
            want &= !key;
            let ts = 1000u32.wrapping_add(i * 3000);
            let sps = if switched { SPS_B } else { SPS_A };
            let (idr, delta) = ([0x65, 0x88, 0x84, i as u8], [0x41, 0x9a, i as u8]);
            let nal: Vec<&[u8]> = if key { vec![sps, PPS, &idr] } else { vec![&delta] };
            for (k, x) in nal.iter().enumerate() {
                out.push(rec(n, t0 + u64::from(tv) * ms, Channel::Rtp(200), pkt(vseq, ts, k + 1 == nal.len(), Some(9), x)));
                n += 1;
                vseq = vseq.wrapping_add(1);
            }
            tv += 33;
        } else {
            out.push(rec(n, t0 + u64::from(ta) * ms, Channel::Rtp(100), pkt(aseq, ta * 48, true, Some(10), &[0x78, aseq as u8])));
            n += 1;
            aseq = aseq.wrapping_add(1);
            ta += 20;
        }
        for r in &out {
            want |= feed(r) > 0;
            pli_step(now, want, latency, drop, &mut plis, &mut due);
        }
    }
    plis
}

#[test]
fn mkv_span_publisher_that_keys_only_on_request_gets_files_near_span() {
    let d = tmp("mkv-on-request");
    let cfg = MkvConfig { span: Some(Duration::from_secs(4)), ..MkvConfig::default() };
    let mut pl = Pipeline::new(Policy::Span { time_ns: u64::MAX, bytes: u64::MAX });
    let mut sink = MkvSink::with(&d, cfg).unwrap();
    let mut asks = 0usize;
    let plis = av_on_request(14, None, 150, &mut |r| {
        for o in pl.push(r) {
            sink.write(&o).unwrap();
        }
        let n = sink.take_keyframe_requests().len();
        asks += n;
        n
    });
    for o in pl.finish() {
        sink.write(&o).unwrap();
    }
    sink.finish().unwrap();
    let fs = sink.files().to_vec();
    let frames: Vec<u64> = fs.iter().map(|f| f.tracks.iter().find(|t| t.kind == "video").map(|t| t.frames).unwrap_or(0)).collect();
    assert!(fs.len() >= 3, "정:RULE-OPS-REC-MKV span 4 s over 14 s: {frames:?} plis {plis:?}");
    assert!(fs[..fs.len() - 1].iter().all(|f| f.reason.as_deref() == Some("span")), "{frames:?}");
    assert!(frames[..frames.len() - 1].iter().all(|n| (120..=135).contains(n)), "정:RULE-OPS-REC-MKV a key before T asks once more at T - files end just past T: {frames:?} plis {plis:?}");
    assert_eq!(asks, 2 * (fs.len() - 1), "정:RULE-OPS-REC-MKV per closed file one ask at T - 1 and one at the early key: {plis:?}");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn a_key_before_target_asks_again_and_the_key_at_target_cuts_unasked() {
    let recs = av_stream_gop(10, 1000, None, 15);
    let d = tmp("mkv-early-then-cut");
    let cfg = MkvConfig { span: Some(Duration::from_secs(4)), ..MkvConfig::default() };
    let (fs, asked) = mkv_av_build(&recs, &d, Policy::Span { time_ns: u64::MAX, bytes: u64::MAX }, cfg);
    assert_eq!(fs.len(), 3, "정:RULE-OPS-REC-MKV keys every 0.5 s - cut at 4 s and 8 s");
    assert_eq!(asked.len(), 4, "정:RULE-OPS-REC-MKV per closed file: T - 1 asks, the 3.5 s key asks again, the 4.0 s key cuts unasked: {asked:?}");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn a_cutting_key_after_a_video_pause_is_not_asked() {
    let ms = 1_000_000u64;
    let mut recs = av_stream_gop(10, 1000, None, 15);
    let t0 = recs[0].log_time;
    recs.retain(|r| !(r.channel == Channel::Rtp(200) && (t0 + 2_900 * ms..t0 + 4_455 * ms).contains(&r.log_time)));
    let d = tmp("mkv-pause-then-cut");
    let cfg = MkvConfig { span: Some(Duration::from_secs(4)), ..MkvConfig::default() };
    let (fs, asked) = mkv_av_build(&recs, &d, Policy::Span { time_ns: u64::MAX, bytes: u64::MAX }, cfg);
    let reasons: Vec<_> = fs.iter().map(|f| f.reason.clone()).collect();
    assert_eq!(asked.len(), 2, "정:RULE-OPS-REC-MKV the first video frame after the pause is a key past T - it cuts before any ask: {asked:?} {reasons:?}");
    std::fs::remove_dir_all(&d).ok();
}


#[test]
fn sinks_never_read_the_system_clock() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sink");
    let mut seen = 0;
    for f in ["ogg.rs", "mkv.rs", "cover.rs"] {
        let src = std::fs::read_to_string(dir.join(f)).unwrap();
        let body = src.split("#[cfg(test)]").next().unwrap_or(&src);
        for bad in ["Instant::now", "SystemTime::now", "SystemTime", "UNIX_EPOCH", "chrono::"] {
            assert!(!body.contains(bad), "{f} 가 시스템 시계({bad})를 쓴다 — 단계 안 시계 금지(D7)");
        }
        seen += 1;
    }
    assert_eq!(seen, 3);
}

#[test]
fn a_session_cut_keeps_the_matroska_audio_timeline_continuous() {
    let d = tmp("mkv-cut");
    let (fs, _) = mkv_av_build(&cut_and_rejoin(false), &d, Policy::Utterance, MkvConfig::default());
    assert_eq!(fs.len(), 1, "{fs:?}");
    let b = std::fs::read(&fs[0].path).unwrap();
    let (dur, _) = mkv_closed(&b);
    assert!((9_600.0..9_800.0).contains(&dur), "정:RULE-OPS-REC-MKV 시각 — 끊긴 구간은 PLC 로 이어지고 새 줄은 log_time 차로 잇는다(마지막 프레임 9.658 s): {dur}");
}
