// author: kodeholic (powered by Claude)
//! 1층 — 녹음 원시 기록(SDK:RULE-PLAT-REC-RAW · REC-CLOCK · REC-PART): 왕복 · 회전 · 사슬 · 머리 스냅숏 · 잘린 파트 · 디스크 하한

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::mpsc;

use std::sync::Arc;

use super::*;

fn quiet() -> Arc<crate::log::Logger> {
    crate::log::Logger::start(crate::log::LogConfig { enabled: false, ..crate::log::LogConfig::default() })
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("oxpeer-rec-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn cfg(dir: &Path) -> RecordConfig {
    let mut c = RecordConfig::new(dir, "r1");
    c.run = "run0".into();
    c.tick = Duration::from_secs(3_600);
    c.min_free_bytes = 0;
    c
}

fn start(c: RecordConfig, ev: mpsc::UnboundedSender<crate::api::Event>, log: Arc<crate::log::Logger>) -> std::io::Result<Arc<Recorder>> {
    let r = Recorder::open(c)?;
    r.attach(ev, log);
    Ok(r)
}

fn rtp(seq: u16) -> Vec<u8> {
    let mut v = vec![0x80, 0x6f, 0, 0, 0, 0, 0, 1, 0, 0, 0, 7];
    v[2..4].copy_from_slice(&seq.to_be_bytes());
    v.extend_from_slice(&[seq as u8; 20]);
    v
}

fn all(dir: &Path) -> (Vec<Record>, Vec<PartScan>) {
    let mut src = FileSource::open(dir, "r1-run0").unwrap();
    let mut got = Vec::new();
    src.run(|r| got.push(r));
    (got, src.scans().to_vec())
}

#[tokio::test]
async fn roundtrip_keeps_order_sequence_and_time() {
    let d = tmp("rt");
    let (ev, _rx) = mpsc::unbounded_channel();
    let r = start(cfg(&d), ev, quiet()).unwrap();
    r.track("room1", serde_json::json!({ "room": "room1", "tracks": [], "rows": {} }));
    for s in 0..50u16 {
        r.push(Channel::Rtp(1000 + (s % 2) as u32), rtp(s));
    }
    r.push(Channel::Sr, br#"{"ssrc":1000,"ntp":1,"rtp":2}"#.to_vec());
    r.close();
    let (got, scans) = all(&d);
    assert_eq!(scans.len(), 1);
    assert!(scans[0].complete, "정상으로 닫은 파트는 Footer 가 있다");
    assert_eq!(scans[0].recording.get("format").map(String::as_str), Some("1"));
    assert_eq!(got.len(), 52);
    assert_eq!(got[0].channel, Channel::Track, "SDK:RULE-PLAT-REC-PART 첫 RTP 에서 연 파트의 머리 스냅숏이 앞선 트랙 목록을 나른다");
    assert_eq!(got[1].data, rtp(0));
    for (i, x) in got.iter().enumerate().skip(1) {
        assert_eq!(x.sequence, got[1].sequence + i as u32 - 1, "순번은 파일 차례로 하나씩");
    }
    assert!(got[0].sequence < got[1].sequence, "머리 스냅숏은 첫 RTP 앞");
    assert!(got.windows(2).all(|w| w[0].log_time <= w[1].log_time), "시각도 파일 차례로 오른다");
    assert_eq!(got[2].channel, Channel::Rtp(1001));
    let end = scans[0].part_end.clone().unwrap();
    assert_eq!(end.get("reason").map(String::as_str), Some("close"));
    assert_eq!(end.get("last_sequence").map(String::as_str), Some(got[51].sequence.to_string().as_str()));
    assert_eq!(scans[0].recording.get("run").map(String::as_str), Some("run0"), "SDK:RULE-PLAT-REC-RAW oxlens.recording run");
}

#[tokio::test]
async fn a_part_opens_only_with_rtp_and_never_overwrites_a_same_name() {
    let d = tmp("first-rtp");
    let (ev, mut rx) = mpsc::unbounded_channel();
    let r = start(cfg(&d), ev, quiet()).unwrap();
    r.track("room1", serde_json::json!({ "room": "room1", "tracks": [], "rows": {} }));
    r.push(Channel::Tick, b"{}".to_vec());
    r.push(Channel::Sr, br#"{"ssrc":1,"ntp":1,"rtp":2}"#.to_vec());
    assert_eq!(std::fs::read_dir(&d).expect("the recorder made its folder").count(), 0, "SDK:RULE-PLAT-REC-PART 미디어가 없으면 파트를 만들지 않는다");
    r.push(Channel::Rtp(1), rtp(0));
    assert!(d.join("r1-run0-0000.mcap").exists(), "SDK:RULE-PLAT-REC-PART 파트 이름 <recording_id>-<run>-NNNN.mcap · 첫 RTP 에서 연다");
    r.close();
    assert!(rx.try_recv().is_err(), "stand check — 앞 기록기는 멈춤 없이 닫았다");
    let first = std::fs::read(d.join("r1-run0-0000.mcap")).unwrap();
    let (ev2, mut rx2) = mpsc::unbounded_channel();
    let r3 = start(cfg(&d), ev2, quiet()).unwrap();
    r3.push(Channel::Rtp(1), rtp(1));
    r3.push(Channel::Rtp(1), rtp(2));
    match rx2.try_recv() {
        Ok(Event::RecordStopped { reason }) => assert_eq!(reason, "name_taken", "SDK:RULE-PLAT-REC-PART 같은 이름이면 열지 않고 멈추고 사건"),
        x => panic!("멈춤 사건이 와야 한다: {x:?}"),
    }
    r3.push(Channel::Tick, b"{}".to_vec());
    r3.push(Channel::Rtp(1), rtp(3));
    assert!(rx2.try_recv().is_err(), "이름 겹침은 여유로 풀리지 않는다 — 다시 열지 않는다");
    assert_eq!(std::fs::read(d.join("r1-run0-0000.mcap")).unwrap(), first, "덮어쓰지 않는다 — 앞 실행의 파트 바이트 그대로");
    let (ev3, mut rx3) = mpsc::unbounded_channel();
    r3.attach(ev3, quiet());
    match rx3.try_recv() {
        Ok(Event::RecordStopped { reason }) => assert_eq!(reason, "name_taken", "멈춘 채로 새 세션에 붙으면 그 세션에도 알린다 — 상태를 숨기지 않는다"),
        x => panic!("붙을 때 멈춤 사건이 와야 한다: {x:?}"),
    }
    assert!(FileSource::open(&d, "r1-nope").is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound), "앞머리에 맞는 파트가 없으면 빈 결과가 아니라 오류");
    let mut other = cfg(&d);
    other.run = "run1".into();
    let r4 = start(other, mpsc::unbounded_channel().0, quiet()).unwrap();
    r4.push(Channel::Rtp(1), rtp(9));
    r4.close();
    assert!(d.join("r1-run1-0000.mcap").exists(), "실행 표지가 다르면 이름이 갈린다 · 새 기록기는 0 부터");
}

#[test]
fn disk_low_resumes_only_past_the_floor_plus_one_part() {
    assert!(!resumes(None, Some(u64::MAX), 10, 5), "멈추지 않았으면 다시 켤 것이 없다");
    assert!(!resumes(Some(Stop::DiskLow), Some(15), 10, 5), "SDK:RULE-PLAT-REC-PART 하한 + S 를 넘어야 — 같으면 아직");
    assert!(resumes(Some(Stop::DiskLow), Some(16), 10, 5), "하한 + S 를 넘으면 새 파트로 다시 켠다");
    assert!(!resumes(Some(Stop::DiskLow), None, 10, 5), "여유를 못 재면 켜지 않는다");
    assert!(!resumes(Some(Stop::NameTaken), Some(u64::MAX), 10, 5), "이름 겹침은 여유로 풀리지 않는다");
    assert!(!resumes(Some(Stop::Unwritable), Some(u64::MAX), 10, 5), "공간과 무관한 열기 실패는 여유로 풀리지 않는다 — tick 마다 멈춤 · 사건을 되풀이하지 않게");
}

#[test]
fn a_part_open_failure_is_classified_by_its_cause() {
    use std::io::{Error, ErrorKind};
    assert_eq!(open_failure(&Error::from(ErrorKind::AlreadyExists)), Stop::NameTaken, "SDK:RULE-PLAT-REC-PART 같은 이름");
    assert_eq!(open_failure(&Error::from(ErrorKind::StorageFull)), Stop::DiskLow, "공간 부족은 여유가 돌아오면 다시 켠다");
    assert_eq!(open_failure(&Error::from(ErrorKind::PermissionDenied)), Stop::Unwritable);
    assert_eq!(open_failure(&Error::from(ErrorKind::NotFound)), Stop::Unwritable);
    assert_eq!((stop_reason(Stop::DiskLow), stop_reason(Stop::Unwritable), stop_reason(Stop::NameTaken)), ("disk_low", "disk_low", "name_taken"), "SDK:RULE-PLAT-REC-API 회전 — 쓰기 실패의 까닭은 disk_low");
}

#[tokio::test]
async fn same_track_list_is_written_once() {
    let d = tmp("tr");
    let (ev, _rx) = mpsc::unbounded_channel();
    let r = start(cfg(&d), ev, quiet()).unwrap();
    r.push(Channel::Rtp(1), rtp(0));
    let b = serde_json::json!({ "room": "a", "tracks": [], "rows": { "5": "t1" } });
    r.track("a", b.clone());
    r.track("a", b);
    r.track("a", serde_json::json!({ "room": "a", "tracks": [], "rows": { "5": "t2" } }));
    r.close();
    let (got, _) = all(&d);
    assert_eq!(got.iter().filter(|x| x.channel == Channel::Track).count(), 2, "줄 재배정만 다시 적는다");
}

#[tokio::test]
async fn rotation_by_size_chains_parts_and_repeats_head_snapshot() {
    let d = tmp("rot");
    let (ev, _rx) = mpsc::unbounded_channel();
    let mut c = cfg(&d);
    c.part_bytes = 2_000;
    let r = start(c, ev, quiet()).unwrap();
    r.track("room1", serde_json::json!({ "room": "room1", "tracks": [], "rows": {} }));
    r.push(Channel::Floor, br#"{"room":"room1","event":"taken","user_id":"u1","sources":[7],"seq":3}"#.to_vec());
    r.push(Channel::Sr, br#"{"ssrc":1000,"ntp":9,"rtp":8}"#.to_vec());
    for s in 0..200u16 {
        r.push(Channel::Rtp(1000), rtp(s));
    }
    r.close();
    let (got, scans) = all(&d);
    assert!(scans.len() >= 5, "작은 S 로 여러 파트(파트 {})", scans.len());
    for (k, sc) in scans.iter().enumerate() {
        assert!(sc.complete, "파트 {k} 는 Footer 가 있다");
        assert_eq!(sc.part.get("part").map(String::as_str), Some(k.to_string().as_str()));
        if k == 0 {
            assert!(!sc.part.contains_key("prev_last_sequence"));
            continue;
        }
        let prev: u32 = sc.part["prev_last_sequence"].parse().unwrap();
        assert_eq!(prev, scans[k - 1].last_sequence.unwrap(), "사슬 — 직전 마지막 순번");
        assert_eq!(prev + 1, sc.first_sequence.unwrap(), "사슬 — 직전 마지막 + 1 = 첫 순번");
        assert_eq!(scans[k - 1].part_end.as_ref().unwrap()["reason"], "size");
    }
    // 파트 1 을 따로 읽으면 머리가 트랙 · 발언권 · SR 상태를 다시 세운다
    let mut one = FileSource::of(vec![scans[1].path.clone()]);
    let mut head = Vec::new();
    one.run(|r| head.push(r.channel));
    assert_eq!(&head[..3], &[Channel::Track, Channel::Floor, Channel::Sr]);
    let rtp_n = got.iter().filter(|x| matches!(x.channel, Channel::Rtp(_))).count();
    assert_eq!(rtp_n, 200, "파트 사이에 패킷이 빠지지 않는다");
}

#[tokio::test]
async fn rotation_by_time() {
    let d = tmp("rott");
    let (ev, _rx) = mpsc::unbounded_channel();
    let mut c = cfg(&d);
    c.part_time = Duration::from_millis(30);
    let r = start(c, ev, quiet()).unwrap();
    for s in 0..4u16 {
        r.push(Channel::Rtp(1), rtp(s));
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    r.close();
    let (got, scans) = all(&d);
    assert_eq!(scans.len(), 2, "SDK:RULE-PLAT-REC-PART 레코드를 쓴 뒤 한도를 본다 — 한도를 넘겨 도착한 레코드는 앞 파트에 쓰고 닫는다");
    assert!(scans.iter().all(|s| s.part_end.as_ref().unwrap()["reason"] == "time"), "회전 뒤 레코드 없이 끝나면 마지막도 time");
    assert_eq!(got.iter().filter(|x| matches!(x.channel, Channel::Rtp(_))).count(), 4);
}

#[tokio::test]
async fn unclosed_part_reads_up_to_last_flushed_chunk() {
    let d = tmp("crash");
    let (ev, _rx) = mpsc::unbounded_channel();
    let mut c = cfg(&d);
    c.flush = Duration::ZERO;
    let r = start(c, ev, quiet()).unwrap();
    for s in 0..10u16 {
        r.push(Channel::Rtp(1), rtp(s));
    }
    r.push(Channel::Tick, b"{}".to_vec());
    for s in 10..20u16 {
        r.push(Channel::Rtp(1), rtp(s));
    }
    // 프로세스가 죽은 것처럼 — 닫지 않은 파일을 그대로 베낀다
    let live = d.join("r1-run0-0000.mcap");
    let cut = d.join("cut.mcap");
    std::fs::copy(&live, &cut).unwrap();
    std::mem::forget(r);
    let sc = scan_part(&cut);
    assert!(!sc.complete, "Footer 가 없다");
    assert_eq!(sc.messages, 11, "닫힌 청크(tick 까지)만 읽힌다");
    assert_eq!(sc.last_sequence, Some(11));
}

#[tokio::test]
async fn disk_low_stops_with_event() {
    let d = tmp("disk");
    let (ev, mut rx) = mpsc::unbounded_channel();
    let mut c = cfg(&d);
    c.min_free_bytes = u64::MAX;
    let r = start(c, ev, quiet()).unwrap();
    r.push(Channel::Rtp(1), rtp(0));
    r.push(Channel::Rtp(1), rtp(1));
    match rx.try_recv() {
        Ok(Event::RecordStopped { reason }) => assert_eq!(reason, "disk_low"),
        x => panic!("멈춤 사건이 와야 한다: {x:?}"),
    }
    assert!(rx.try_recv().is_err(), "멈춘 뒤 다시 알리지 않는다");
    drop(r);
}

#[test]
fn channel_topics_round_trip() {
    for ch in [Channel::Rtp(4_000_000_001), Channel::Track, Channel::Floor, Channel::Session, Channel::Sr, Channel::Gap, Channel::Tick] {
        assert_eq!(Channel::parse(&ch.topic()), Some(ch));
    }
    assert_eq!(Channel::parse("rtp/x"), None);
}

// ── 2단계 — 파이프라인 · Ogg · 라이브 통(SDK:RULE-PLAT-REC-PIPE · REC-SPLIT · REC-API) ──

fn pkt(seq: u16, ts: u32, marker: bool, csrc: Option<u32>, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![if csrc.is_some() { 0x81 } else { 0x80 }, if marker { 0xEF } else { 0x6F }, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    v[2..4].copy_from_slice(&seq.to_be_bytes());
    v[4..8].copy_from_slice(&ts.to_be_bytes());
    if let Some(c) = csrc {
        v.extend_from_slice(&c.to_be_bytes());
    }
    v.extend_from_slice(payload);
    v
}

fn rec(seq: u32, t: u64, ch: Channel, data: Vec<u8>) -> Record {
    Record { sequence: seq, log_time: t, channel: ch, data }
}

fn tracks(room: &str) -> Vec<u8> {
    serde_json::json!({
        "room": room,
        "tracks": [{ "track_id": "slot-a", "kind": "audio" }, { "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 }],
        "rows": { "100": { "track_id": "slot-a", "kind": "audio", "codec": "opus", "pt": 111 }, "200": { "track_id": "cam-u1", "kind": "video", "codec": "VP8", "pt": 96 } }
    })
    .to_string()
    .into_bytes()
}

/// 무전 슬롯: 출처 없음(예열 침묵) · A(7) 둘 · B(8) 하나 · 출처 없음 / 영상: 델타(버림) · 키 · 델타 · gap · 델타(버림) · 키
fn stream() -> Vec<Record> {
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

fn run_pipe(p: Policy, recs: &[Record]) -> Vec<Output> {
    let mut pl = Pipeline::new(p);
    let mut out: Vec<Output> = recs.iter().flat_map(|r| pl.push(r)).collect();
    out.extend(pl.finish());
    out
}

#[test]
fn pipeline_is_deterministic() {
    assert_eq!(run_pipe(Policy::Utterance, &stream()), run_pipe(Policy::Utterance, &stream()));
}

#[test]
fn utterance_policy_splits_at_taken_and_leaves_frames_without_source_out() {
    let out = run_pipe(Policy::Utterance, &stream());
    let audio: Vec<(Option<u32>, Option<u64>, Option<String>)> = out
        .iter()
        .filter_map(|o| match o {
            Output::Frame(f) if f.kind == "audio" => Some((f.source, f.segment, f.user_id.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(audio, vec![(Some(7), Some(0), Some("A".into())), (Some(7), Some(0), Some("A".into())), (Some(8), Some(2), Some("B".into()))], "B 발언은 A 발언의 유예가 지난 뒤 연다(그 사이 연 영상 세그먼트가 1)");
    let orphans = out.iter().filter(|o| matches!(o, Output::Orphan(f) if f.data == [0xF8, 0xFF, 0xFE])).count();
    assert_eq!(orphans, 2, "출처 없는 프레임은 세그먼트 밖");
    let closed: Vec<(u64, Option<&str>)> = out.iter().filter_map(|o| if let Output::Segment(s) = o { (s.event == SegmentKind::Closed).then_some((s.id, s.reason)) } else { None }).collect();
    assert_eq!(closed[0], (0, Some("taken")), "경계는 발언권 허가 · 출처 없는 프레임은 세그먼트를 닫지 않는다");
    assert!(closed[1..].iter().all(|c| c.1 == Some("end")));
}

fn floor(n: u32, t: u64, ev: &str, who: &str, src: &[u32]) -> Record {
    let user = if ev == "taken" { format!("\"{who}\"") } else { "null".into() };
    rec(n, t, Channel::Floor, format!(r#"{{"room":"r","event":"{ev}","user_id":{user},"sources":{src:?},"seq":{n}}}"#).into_bytes())
}

fn slot_audio(n: u32, t: u64, seq: u16, src: u32) -> Record {
    rec(n, t, Channel::Rtp(100), pkt(seq, u32::from(seq) * 960, true, Some(src), &[0x78, seq as u8]))
}

/// 발언 정책 출력 → (세그먼트 번호, 출처) 열 · 고아 출처 열 · 닫힌 까닭 열
type Utter = (Vec<(Option<u64>, Option<u32>)>, Vec<Option<u32>>, Vec<&'static str>);

fn utter(out: &[Output]) -> Utter {
    let fr = out.iter().filter_map(|o| if let Output::Frame(f) = o && !f.filled { Some((f.segment, f.source)) } else { None }).collect();
    let orph = out.iter().filter_map(|o| if let Output::Orphan(f) = o { Some(f.source) } else { None }).collect();
    let closed = out.iter().filter_map(|o| if let Output::Segment(x) = o { (x.event == SegmentKind::Closed).then_some(x.reason.unwrap_or("-")) } else { None }).collect();
    (fr, orph, closed)
}

#[test]
fn new_speaker_media_before_its_taken_is_held_and_opens_its_utterance() {
    let ms = 1_000_000u64;
    let recs = [
        rec(1, 0, Channel::Track, tracks("r")),
        floor(2, 0, "taken", "A", &[7]),
        slot_audio(3, 20 * ms, 1, 7),
        floor(4, 40 * ms, "idle", "", &[]),
        slot_audio(5, 60 * ms, 2, 8),
        slot_audio(6, 80 * ms, 3, 8),
        floor(7, 90 * ms, "taken", "B", &[8]),
        slot_audio(8, 100 * ms, 4, 8),
    ];
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut out: Vec<Output> = recs.iter().flat_map(|r| pl.push(r)).collect();
    out.extend(pl.finish());
    let (fr, orph, closed) = utter(&out);
    assert_eq!(fr, vec![(Some(0), Some(7)), (Some(1), Some(8)), (Some(1), Some(8)), (Some(1), Some(8))], "허가보다 먼저 온 B 의 첫 두 프레임이 B 발언 앞에");
    assert!(orph.is_empty());
    assert_eq!(closed, vec!["idle", "end"]);
    let opened: Vec<Option<u64>> = out.iter().filter_map(|o| if let Output::Segment(x) = o { (x.event == SegmentKind::Opened).then_some(x.utterance) } else { None }).collect();
    assert_eq!(opened, vec![Some(0), Some(1)], "허가마다 발언 번호");
}

#[test]
fn previous_speaker_tail_goes_to_its_utterance_within_grace_and_never_to_the_new_one() {
    let ms = 1_000_000u64;
    let recs = [
        rec(1, 0, Channel::Track, tracks("r")),
        floor(2, 0, "taken", "A", &[7]),
        slot_audio(3, 20 * ms, 1, 7),
        floor(4, 30 * ms, "taken", "B", &[8]),
        slot_audio(5, 35 * ms, 2, 7),
        slot_audio(6, 40 * ms, 3, 8),
        slot_audio(7, 400 * ms, 4, 7),
        slot_audio(8, 420 * ms, 5, 8),
    ];
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut out: Vec<Output> = recs.iter().flat_map(|r| pl.push(r)).collect();
    out.extend(pl.finish());
    let (fr, orph, closed) = utter(&out);
    assert_eq!(fr, vec![(Some(0), Some(7)), (Some(0), Some(7)), (Some(1), Some(8)), (Some(1), Some(8))], "승계 — B 허가 뒤 유예(200ms) 안에 온 A 의 꼬리는 A 발언에 · B 의 첫 프레임은 유예 뒤 B 발언 앞에");
    assert_eq!(orph, vec![Some(7)], "유예를 넘긴 A 의 패킷은 어느 발언에도 넣지 않고 센다");
    assert_eq!(pl.counts().other_speaker, 1);
    assert_eq!(closed, vec!["taken", "end"]);
}

#[test]
fn own_grant_closes_the_previous_speaker_and_opens_no_one() {
    // 큐 승계 허가가 나에게 오면 남들은 TAKEN 을 받고 나는 GRANTED 를 받는다 — 앞 화자 발언이 그때 닫혀야 한다(20261112f)
    let ms = 1_000_000u64;
    let recs = [
        rec(1, 0, Channel::Track, tracks("r")),
        floor(2, 0, "taken", "A", &[7]),
        slot_audio(3, 20 * ms, 1, 7),
        rec(4, 30 * ms, Channel::Floor, br#"{"room":"r","event":"granted","user_id":"me","seq":null}"#.to_vec()),
        rec(41, 32 * ms, Channel::Floor, br#"{"room":"r","event":"granted","user_id":"me","seq":null}"#.to_vec()),
        slot_audio(5, 35 * ms, 2, 7),
        slot_audio(6, 400 * ms, 3, 7),
        floor(7, 1_000 * ms, "taken", "B", &[8]),
        slot_audio(8, 1_020 * ms, 4, 8),
    ];
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut out: Vec<Output> = recs[..7].iter().flat_map(|r| pl.push(r)).collect();
    assert_eq!(utter(&out).2, vec!["taken"], "A 발언은 내 허가 + 유예(400ms 레코드)에서 닫힌다 — B 허가를 기다리지 않는다 · 두 번째 granted(놓는 중 허가 재전송)는 해가 없다");
    out.extend(recs[7..].iter().flat_map(|r| pl.push(r)));
    out.extend(pl.finish());
    let (fr, orph, closed) = utter(&out);
    assert_eq!(fr, vec![(Some(0), Some(7)), (Some(0), Some(7)), (Some(1), Some(8))], "유예 안 A 꼬리는 A 발언에 · 내 허가는 발언을 열지 않는다(B 가 다음 번호)");
    assert_eq!(orph, vec![Some(7)], "유예를 넘긴 A 패킷은 센다");
    assert_eq!(closed, vec!["taken", "end"]);
}

#[test]
fn repeated_taken_of_the_same_grant_does_not_split_the_utterance() {
    let ms = 1_000_000u64;
    let recs = [
        rec(1, 0, Channel::Track, tracks("r")),
        floor(2, 0, "taken", "A", &[7]),
        slot_audio(3, 20 * ms, 1, 7),
        floor(2, 30 * ms, "taken", "A", &[7]),
        slot_audio(4, 40 * ms, 2, 7),
        floor(2, 50 * ms, "taken", "A", &[7]),
        slot_audio(5, 60 * ms, 3, 7),
    ];
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut out: Vec<Output> = recs.iter().flat_map(|r| pl.push(r)).collect();
    out.extend(pl.finish());
    let (fr, orph, closed) = utter(&out);
    assert_eq!(fr, vec![(Some(0), Some(7)); 3], "같은 허가 번호의 TAKEN(재전송 · 파트 머리 다시 적기)은 새 발언이 아니다");
    assert!(orph.is_empty());
    assert_eq!(closed, vec!["end"]);
}

#[test]
fn media_without_taken_for_over_a_second_is_left_out() {
    let ms = 1_000_000u64;
    let recs = [
        rec(1, 0, Channel::Track, tracks("r")),
        slot_audio(2, 0, 1, 8),
        slot_audio(3, 1_500 * ms, 2, 8),
        floor(4, 1_510 * ms, "taken", "B", &[8]),
        slot_audio(5, 1_520 * ms, 3, 8),
        floor(6, 1_600 * ms, "idle", "", &[]),
        slot_audio(7, 1_620 * ms, 4, 8),
        slot_audio(8, 1_900 * ms, 5, 8),
    ];
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut out: Vec<Output> = recs.iter().flat_map(|r| pl.push(r)).collect();
    out.extend(pl.finish());
    let (fr, orph, closed) = utter(&out);
    assert_eq!(fr, vec![(Some(0), Some(8)), (Some(0), Some(8)), (Some(0), Some(8))], "1초 안에 허가가 온 프레임 · 반환 뒤 유예 안의 꼬리가 그 발언에");
    assert_eq!(orph, vec![Some(8), Some(8)], "1초를 넘긴 것 · 반환 뒤 유예를 넘기고 허가 없이 끝난 것");
    assert_eq!(pl.counts().unheld, 2);
    assert_eq!(closed, vec!["idle"]);
}

#[test]
fn video_starts_at_keyframe_and_gap_breaks_until_next_key() {
    let out = run_pipe(Policy::Utterance, &stream());
    let video: Vec<(u32, bool)> = out.iter().filter_map(|o| if let Output::Frame(f) = o { (f.kind == "video").then_some((f.rtp_ts, f.keyframe)) } else { None }).collect();
    assert_eq!(video, vec![(6000, true), (9000, false), (18000, true)], "첫 델타 · gap 뒤 델타는 버린다");
    let mut pl = Pipeline::new(Policy::Utterance);
    for r in stream() {
        pl.push(&r);
    }
    assert_eq!((pl.counts().before_key, pl.counts().broken), (2, 0));
}

#[test]
fn sr_maps_rtp_time_to_utc() {
    let out = run_pipe(Policy::Utterance, &stream());
    let utc: Vec<Option<u64>> = out.iter().filter_map(|o| if let Output::Frame(f) = o { (f.kind == "video").then_some(f.utc_ns) } else { None }).collect();
    // NTP 16045690984503098046 = 유닉스 1527000000.5 초 무렵 · RTP 3000 기준 · 90kHz
    let base = (16045690984503098046u64 >> 32) - 2_208_988_800;
    let frac = ((16045690984503098046u64 & 0xffff_ffff) * 1_000_000_000) >> 32;
    let t0 = base * 1_000_000_000 + frac;
    assert_eq!(utc, vec![Some(t0 + 3_000 * 1_000_000_000 / 90_000), Some(t0 + 6_000 * 1_000_000_000 / 90_000), Some(t0 + 15_000 * 1_000_000_000 / 90_000)]);
    let audio_utc = out.iter().any(|o| matches!(o, Output::Frame(f) if f.kind == "audio" && f.utc_ns.is_some()));
    assert!(!audio_utc, "SR 전 · SR 없는 줄은 UTC 가 없다(받은 시각으로 메우지 않는다)");
}

#[test]
fn span_policy_cuts_audio_anywhere_and_video_at_keyframe() {
    let out = run_pipe(Policy::Span { time_ns: 30_000_000, bytes: u64::MAX }, &stream());
    let segs: Vec<(String, u64, Option<&str>)> = out.iter().filter_map(|o| if let Output::Segment(s) = o { (s.event == SegmentKind::Closed).then(|| (s.track_id.clone(), s.frames, s.reason)) } else { None }).collect();
    assert!(segs.iter().filter(|s| s.0 == "slot-a").count() >= 2, "음성은 T 뒤 아무 프레임에서 자른다: {segs:?}");
    let v: Vec<_> = segs.iter().filter(|s| s.0 == "cam-u1").collect();
    assert_eq!(v.len(), 2, "영상은 T 뒤 첫 키프레임(18000)에서: {segs:?}");
    let silence_in = out.iter().filter(|o| matches!(o, Output::Frame(f) if f.data == [0xF8, 0xFF, 0xFE] && f.segment.is_some())).count();
    assert_eq!(silence_in, 2, "연속 정책은 서버 침묵도 담는다");
}

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

/// 받은 `taken` 에 서버 발언 id(TLV `0x20` — 연:RULE-FLR-TLV)를 싣는다 — `talk` = "t" + 허가 번호
fn with_talk(recs: Vec<Record>) -> Vec<Record> {
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

#[tokio::test]
async fn live_tap_equals_file_records() {
    let d = tmp("tap");
    let (ev, _rx) = mpsc::unbounded_channel();
    let mut c = cfg(&d);
    c.part_bytes = 300;
    let r = start(c, ev, quiet()).unwrap();
    let (tx, rx) = mpsc::channel(10_000);
    let shared = Arc::new(Shared::default());
    r.subscribe(tx, shared.clone());
    let mut live = Records::new(rx, shared);
    for x in stream() {
        r.push(x.channel, x.data);
    }
    r.close();
    let mut got = Vec::new();
    while let Some(x) = live.next().await {
        got.push(x);
    }
    let (file, scans) = all(&d);
    assert!(scans.len() > 1, "회전이 끼어도");
    let mut it = got.iter();
    let mut missing = Vec::new();
    for f in &file {
        loop {
            let Some(x) = it.next() else { panic!("파일 레코드 {} 가 라이브 통에 없다", f.sequence) };
            if x == f {
                break;
            }
            missing.push(x.clone());
        }
    }
    missing.extend(it.cloned());
    let first_rtp = got.iter().find(|x| matches!(x.channel, Channel::Rtp(_))).map(|x| x.sequence).expect("stand check — RTP 가 있다");
    assert!(missing.iter().all(|x| x.sequence < first_rtp), "SDK:RULE-PLAT-REC-PART 파일 = 라이브 통에서 첫 파트 앞의 제어 레코드만 뺀 것 — 첫 파트 뒤로는 회전이 끼어도 레코드가 하나도 빠지지 않는다: {missing:?}");
    assert!(!missing.is_empty(), "stand check — 첫 RTP 앞에 제어 레코드가 있었다");
    for w in scans.windows(2) {
        let prev = w[0].part_end.as_ref().and_then(|e| e.get("last_sequence")).and_then(|s| s.parse::<u32>().ok()).expect("앞 파트 끝 순번");
        let head = w[1].part.get("prev_last_sequence").and_then(|s| s.parse::<u32>().ok()).expect("뒤 파트 머리 prev_last_sequence");
        assert_eq!(prev, head, "사슬 — 뒤 파트 머리는 앞 파트의 끝 순번");
        assert_eq!(w[1].first_sequence, Some(prev + 1), "SDK:RULE-PLAT-REC-PART 사슬 — 회전 뒤 첫 순번 = prev_last + 1(디스크 하한 멈춤 밖에서는 끊김 0)");
    }
}

#[tokio::test]
async fn a_time_rotation_reopens_on_the_next_record_even_without_rtp() {
    let d = tmp("rot-any");
    let (ev, _rx) = mpsc::unbounded_channel();
    let mut c = cfg(&d);
    c.part_time = Duration::ZERO;
    let r = start(c, ev, quiet()).unwrap();
    r.push(Channel::Rtp(1), rtp(0));
    r.push(Channel::Gap, br#"{"ssrc":1,"from":1,"to":3,"count":3,"reason":"nack_exhausted"}"#.to_vec());
    r.push(Channel::Tick, b"{}".to_vec());
    r.close();
    let (got, scans) = all(&d);
    assert!(got.iter().any(|x| x.channel == Channel::Gap), "SDK:RULE-PLAT-REC-PART 첫 파트 뒤에는 어떤 레코드든 곧바로 다음 파트를 연다 — 회전 뒤 gap 이 사라지지 않는다");
    assert!(got.iter().any(|x| x.channel == Channel::Tick), "tick 도 다음 파트를 연다");
    assert!(scans.len() >= 3, "레코드마다 시간 회전: {}", scans.len());
    assert!(scans.iter().all(|s| s.part_end.as_ref().and_then(|e| e.get("reason")).map(String::as_str) == Some("time")), "SDK:RULE-PLAT-REC-PART 레코드를 쓴 뒤 한도를 보고 닫는다 — 회전 뒤 레코드가 없이 끝나면 마지막 part_end 도 회전 사유(time) 그대로");
    let probe = [0xA0u8, 0x6f, 0, 1, 0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 4];
    assert!(opens_part(0, &Channel::Rtp(1), &rtp(0)), "첫 파트는 페이로드가 있는 RTP 에서 연다");
    assert!(!opens_part(0, &Channel::Rtp(1), &probe), "패딩뿐인 대역 probe 는 미디어가 아니다");
    assert!(!opens_part(0, &Channel::Gap, b"{}") && opens_part(1, &Channel::Gap, b"{}") && opens_part(1, &Channel::Tick, b"{}"), "첫 파트만 RTP 를 기다린다");
}

#[test]
fn rotation_and_payload_are_pure_judgements() {
    assert_eq!(rotation(10, 0, 10, 100), Some("size"), "SDK:RULE-PLAT-REC-PART 크기 · 시간 중 먼저 닿는 쪽");
    assert_eq!(rotation(9, 100, 10, 100), Some("time"));
    assert_eq!(rotation(9, 99, 10, 100), None);
    assert!(!rtp_has_payload(&[0x80, 0x6f, 0, 1, 0, 0, 0, 1, 0, 0, 0, 7]), "머리뿐");
    assert!(rtp_has_payload(&[0x80, 0x6f, 0, 1, 0, 0, 0, 1, 0, 0, 0, 7, 9]));
    let mut ext = vec![0x90u8, 0x6f, 0, 1, 0, 0, 0, 1, 0, 0, 0, 7, 0xBE, 0xDE, 0, 1, 1, 2, 3, 4];
    assert!(!rtp_has_payload(&ext), "확장 머리 뒤 페이로드 없음");
    ext.push(5);
    assert!(rtp_has_payload(&ext));
    assert!(!rtp_has_payload(&[0x81, 0x6f, 0, 1, 0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 1]), "CSRC 하나 뒤 페이로드 없음");
    assert!(!rtp_has_payload(&[0x80]), "잘린 머리");
}

#[tokio::test]
async fn overflow_downgrades_to_tail_read_without_loss_or_duplicates() {
    let d = tmp("tail");
    let (ev, _rx) = mpsc::unbounded_channel();
    let mut c = cfg(&d);
    c.part_bytes = 3_000;
    c.flush = Duration::ZERO;
    c.tick = Duration::from_millis(20);
    let r = start(c, ev, quiet()).unwrap();
    let (tx, rx) = mpsc::channel(16);
    let shared = Arc::new(Shared::default());
    r.subscribe(tx, shared.clone());
    let mut live = Records::new(rx, shared);
    for s in 0..300u16 {
        r.push(Channel::Rtp(1), rtp(s));
        if s % 10 == 0 {
            r.push(Channel::Tick, b"{}".to_vec());
        }
    }
    let reader = tokio::spawn(async move {
        let mut got = Vec::new();
        while let Some(x) = live.next().await {
            got.push(x.sequence);
        }
        (got, live.degraded())
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    for s in 300..320u16 {
        r.push(Channel::Rtp(1), rtp(s));
    }
    r.close();
    let (got, degraded) = tokio::time::timeout(Duration::from_secs(5), reader).await.unwrap().unwrap();
    assert!(degraded, "16 칸 통이 넘쳤다");
    let (file, _) = all(&d);
    let want: Vec<u32> = file.iter().map(|x| x.sequence).collect();
    assert_eq!(got, want, "넘친 뒤도 빠짐 · 겹침 없이 파일과 같은 차례");
}

#[tokio::test]
async fn overflow_without_raw_stops_with_event() {
    let (ev, mut evs) = mpsc::unbounded_channel();
    let mut c = RecordConfig::live("live");
    c.tick = Duration::from_secs(3_600);
    let r = start(c, ev, quiet()).unwrap();
    let (tx, rx) = mpsc::channel(16);
    let shared = Arc::new(Shared::default());
    r.subscribe(tx, shared.clone());
    let mut live = Records::new(rx, shared);
    for s in 0..40u16 {
        r.push(Channel::Rtp(1), rtp(s));
    }
    let mut n = 0;
    while live.next().await.is_some() {
        n += 1;
    }
    assert_eq!(n, 16, "넘치기 전까지만 · 섞인 결과를 몰래 내지 않는다");
    assert!(live.stopped());
    assert!(matches!(evs.try_recv(), Ok(Event::RecordConsumerStopped { .. })));
}

#[tokio::test]
async fn closed_recorder_takes_nothing_and_opens_no_new_part() {
    // 닫힌 뒤 tick 이 밀어 넣어도 새 파트를 열지 않는다(꼬리 읽기가 끝을 볼 수 있게 — 20261109d 에서 드러남)
    let d = tmp("closed");
    let (ev, _rx) = mpsc::unbounded_channel();
    let r = start(cfg(&d), ev, quiet()).unwrap();
    r.push(Channel::Rtp(1), rtp(0));
    r.close();
    r.push(Channel::Tick, b"{}".to_vec());
    r.push(Channel::Rtp(1), rtp(1));
    let (got, scans) = all(&d);
    assert_eq!(scans.len(), 1);
    assert_eq!(got.len(), 1);
}

#[tokio::test]
async fn late_subscriber_gets_current_state_first() {
    // 늦게 연 통 — 트랙 · 발언권 · SR 의 마지막 레코드를 원래 순번 · 시각으로 먼저 받아 줄 → 트랙을 안다(20261109d 2층에서 드러남 — 입장 뒤 연 통이 줄 표를 몰라 패킷 전부가 「줄 모름」)
    let (ev, _rx) = mpsc::unbounded_channel();
    let mut c = RecordConfig::live("late");
    c.tick = Duration::from_secs(3_600);
    let r = start(c, ev, quiet()).unwrap();
    let recs = stream();
    for x in &recs[..2] {
        r.push(x.channel.clone(), x.data.clone());
    }
    let (tx, rx) = mpsc::channel(1_000);
    let shared = Arc::new(Shared::default());
    r.subscribe(tx, shared.clone());
    let mut live = Records::new(rx, shared);
    for x in &recs[2..] {
        r.push(x.channel.clone(), x.data.clone());
    }
    r.close();
    let mut got = Vec::new();
    while let Some(x) = live.next().await {
        got.push(x);
    }
    assert_eq!((got[0].channel.clone(), got[0].sequence), (Channel::Track, 1));
    assert_eq!((got[1].channel.clone(), got[1].sequence), (Channel::Floor, 2));
    let mut pl = Pipeline::new(Policy::Utterance);
    for x in &got {
        pl.push(x);
    }
    assert_eq!(pl.counts().unrouted, 0);
}

// ── 3단계 — 음성 구멍 PLC(SDK:RULE-PLAT-REC-GAP) ──

fn opus_stream(seqs_ts: &[(u16, u32)], toc: u8) -> Vec<Record> {
    let mut v = vec![rec(1, 0, Channel::Track, tracks("r")), rec(1, 0, Channel::Floor, br#"{"room":"r","event":"taken","user_id":"A","sources":[7],"seq":1}"#.to_vec())];
    for (i, (seq, ts)) in seqs_ts.iter().enumerate() {
        v.push(rec(2 + i as u32, (i as u64 + 1) * 20_000_000, Channel::Rtp(100), pkt(*seq, *ts, true, Some(7), &[toc, 9, 9])));
    }
    v
}

#[test]
fn audio_hole_is_filled_with_plc_and_remainder_carries() {
    // 20ms 프레임(960) · seq 3 · 4 를 잃음(구멍 1,920) · 그다음 구멍 1,000(= 960 하나 + 40 넘김) · 그다음 920(+40 = 960 하나)
    let out = run_pipe(Policy::Utterance, &opus_stream(&[(1, 0), (2, 960), (5, 3_840), (6, 5_800), (7, 7_680)], 0x78));
    let frames: Vec<(u32, bool, usize)> = out.iter().filter_map(|o| if let Output::Frame(f) = o { Some((f.rtp_ts, f.filled, f.data.len())) } else { None }).collect();
    assert_eq!(frames, vec![(0, false, 3), (960, false, 3), (1_920, true, 1), (2_880, true, 1), (3_840, false, 3), (4_800, true, 1), (5_800, false, 3), (6_760, true, 1), (7_680, false, 3)]);
    let plc: Vec<u8> = out.iter().filter_map(|o| if let Output::Frame(f) = o { f.filled.then(|| f.data[0]) } else { None }).collect();
    assert!(plc.iter().all(|t| *t == 0x78 & 0xFC), "앞 TOC 의 config · code 0");
    let closed = out.iter().find_map(|o| if let Output::Segment(s) = o { (s.event == SegmentKind::Closed).then_some(s.filled) } else { None });
    assert_eq!(closed, Some(4));
}

#[test]
fn hole_over_cap_opens_new_segment_without_fill() {
    let mut pl = Pipeline::new(Policy::Utterance).with_gap_max(Duration::from_millis(100));
    let mut out = Vec::new();
    for r in opus_stream(&[(1, 0), (2, 960), (3, 48_000)], 0x78) {
        out.extend(pl.push(&r));
    }
    out.extend(pl.finish());
    let closes: Vec<Option<&str>> = out.iter().filter_map(|o| if let Output::Segment(s) = o { (s.event == SegmentKind::Closed).then_some(s.reason) } else { None }).collect();
    assert_eq!(closes, vec![Some("gap"), Some("end")]);
    assert!(!out.iter().any(|o| matches!(o, Output::Frame(f) if f.filled)));
}

#[test]
fn audio_hole_across_rtp_wrap_is_filled_like_any_other_hole() {
    let s = u32::MAX - 960 * 3 + 1;
    let at = |k: u32| s.wrapping_add(k * 960);
    let out = run_pipe(Policy::Utterance, &opus_stream(&[(1, at(0)), (2, at(1)), (5, at(4)), (6, at(5))], 0x78));
    let frames: Vec<(u32, bool)> = out.iter().filter_map(|o| if let Output::Frame(f) = o { Some((f.rtp_ts, f.filled)) } else { None }).collect();
    assert_eq!(frames, vec![(at(0), false), (at(1), false), (at(2), true), (at(3), true), (at(4), false), (at(5), false)], "구멍이 넘침을 가로질러도 RTP 차로 셈 — PLC 둘 · 시각도 넘침을 따라간다");
    assert!(at(2) > at(1) && at(3) < at(2), "구멍이 넘침 앞(앞 프레임 끝 at(2))에서 시작해 넘침 뒤에서 끝난다 — 빼기 자체가 넘침을 가로지른다");
    let closes = out.iter().filter(|o| matches!(o, Output::Segment(x) if x.event == SegmentKind::Closed)).count();
    assert_eq!(closes, 1, "넘침이 상한 넘는 구멍으로 읽혀 세그먼트를 새로 열지 않는다");
}

#[test]
fn hole_over_cap_across_rtp_wrap_still_opens_new_segment() {
    let s = u32::MAX - 10_000;
    let mut pl = Pipeline::new(Policy::Utterance).with_gap_max(Duration::from_millis(100));
    let mut out = Vec::new();
    for r in opus_stream(&[(1, s), (2, s.wrapping_add(960)), (3, s.wrapping_add(48_000))], 0x78) {
        out.extend(pl.push(&r));
    }
    out.extend(pl.finish());
    let closes: Vec<Option<&str>> = out.iter().filter_map(|o| if let Output::Segment(x) = o { (x.event == SegmentKind::Closed).then_some(x.reason) } else { None }).collect();
    assert_eq!(closes, vec![Some("gap"), Some("end")], "넘침 뒤 1초 구멍도 상한(100ms)을 넘는 구멍");
    assert!(!out.iter().any(|o| matches!(o, Output::Frame(f) if f.filled)));
}

#[test]
fn video_is_never_filled() {
    let out = run_pipe(Policy::Utterance, &stream());
    assert!(!out.iter().any(|o| matches!(o, Output::Frame(f) if f.filled && f.kind == "video")));
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

// ── 4단계(202611/20261110a) — 분기 = 출처 트랙 · 열쇠 = 트랙 · 영상은 키프레임에서 연다 · Matroska 결과물 ──

fn track_list(room: &str, tracks: serde_json::Value, rows: serde_json::Value) -> Vec<u8> {
    serde_json::json!({ "room": room, "tracks": tracks, "rows": rows }).to_string().into_bytes()
}

const KEY: &[u8] = &[0x10, 0x00, 0x9d, 0x01, 0x2a];
const DELTA: &[u8] = &[0x10, 0x01, 7];

fn vrow(tid: &str) -> serde_json::Value {
    serde_json::json!({ "track_id": tid, "kind": "video", "codec": "VP8", "pt": 96 })
}

fn segs(out: &[Output]) -> Vec<(SegmentKind, u64, String, Option<&'static str>)> {
    out.iter()
        .filter_map(|o| match o {
            Output::Segment(s) => Some((s.event, s.id, s.track_id.clone(), s.reason)),
            _ => None,
        })
        .collect()
}

#[test]
fn late_packet_of_departed_track_is_not_mixed_into_new_track() {
    let ms = 1_000_000u64;
    let u1 = serde_json::json!({ "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 });
    let u2 = serde_json::json!({ "track_id": "cam-u2", "kind": "video", "user_id": "u2", "csrc": 10 });
    let recs = [
        rec(1, 0, Channel::Track, track_list("r", serde_json::json!([u1]), serde_json::json!({ "200": vrow("cam-u1") }))),
        rec(2, ms, Channel::Rtp(200), pkt(1, 3000, true, Some(9), KEY)),
        // 재배정 — 줄 200 이 u2 로 · u1 은 목록에서 빠짐
        rec(3, 2 * ms, Channel::Track, track_list("r", serde_json::json!([u2]), serde_json::json!({ "200": vrow("cam-u2") }))),
        rec(4, 3 * ms, Channel::Rtp(200), pkt(2, 6000, true, Some(9), DELTA)),
        rec(5, 4 * ms, Channel::Rtp(200), pkt(3, 9000, true, Some(10), KEY)),
    ];
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut out: Vec<Output> = recs.iter().flat_map(|r| pl.push(r)).collect();
    out.extend(pl.finish());
    let frames: Vec<(String, u32)> = out.iter().filter_map(|o| if let Output::Frame(f) = o { Some((f.track_id.clone(), f.rtp_ts)) } else { None }).collect();
    assert_eq!(frames, vec![("cam-u1".into(), 3000), ("cam-u2".into(), 9000)], "늦은 u1 패킷은 u2 에 섞이지 않는다");
    assert_eq!(pl.counts().late, 1);
    assert_eq!(segs(&out)[1], (SegmentKind::Closed, 0, "cam-u1".into(), Some("track")), "목록에서 빠질 때 닫는다");
}

/// 퇴장 직후 파트가 바뀌고 늦은 패킷이 새 파트에 실린 판(2층 e2e-20261120-201922 `peer_raw_part_selfcontained` 빨강 · context 202611/20261119c D17).
/// 파트 머리는 그때의 목록 하나(목록에서 빠진 트랙은 안 실림)인데 줄 표는 빠진 트랙의 줄을 그대로 갖는다.
fn departed_then_late(part_alone: bool) -> Vec<(String, u32)> {
    let ms = 1_000_000u64;
    let a = serde_json::json!({ "track_id": "mic-u1", "kind": "audio", "user_id": "u1", "csrc": 9 });
    let b = serde_json::json!({ "track_id": "mic-u2", "kind": "audio", "user_id": "u2", "csrc": 10 });
    let rows = serde_json::json!({ "300": { "track_id": "mic-u1", "kind": "audio", "codec": "opus", "pt": 111 }, "301": { "track_id": "mic-u2", "kind": "audio", "codec": "opus", "pt": 111 } });
    let head = rec(3, 2 * ms, Channel::Track, track_list("r", serde_json::json!([b]), rows.clone()));
    let late = rec(4, 3 * ms, Channel::Rtp(300), pkt(2, 960, true, Some(9), &[0x78, 2]));
    let recs = if part_alone {
        vec![head, late]
    } else {
        vec![rec(1, 0, Channel::Track, track_list("r", serde_json::json!([a, b]), rows)), rec(2, ms, Channel::Rtp(300), pkt(1, 0, true, Some(9), &[0x78, 1])), head, late]
    };
    let mut pl = Pipeline::new(Policy::Span { time_ns: 60_000 * ms, bytes: u64::MAX });
    let mut out: Vec<Output> = recs.iter().flat_map(|r| pl.push(r)).collect();
    out.extend(pl.finish());
    out.iter().filter_map(|o| if let Output::Frame(f) = o { Some((f.track_id.clone(), f.rtp_ts)) } else { None }).collect()
}

#[test]
fn part_alone_treats_late_packet_of_departed_track_as_the_whole_stream_does() {
    let whole = departed_then_late(false);
    assert_eq!(whole, vec![("mic-u1".into(), 0)], "전체 읽기 — 목록에서 빠진 뒤 온 mic-u1 패킷은 늦은 패킷(프레임 아님)");
    let alone = departed_then_late(true);
    assert_eq!(alone, Vec::<(String, u32)>::new(), "SDK:RULE-PLAT-REC-PART 파트 k 는 앞 파트 없이 읽힌다 — 머리 목록에 없는 트랙의 패킷은 단독 읽기에서도 늦은 패킷");
}

#[test]
fn track_moving_rows_stays_one_segment() {
    let ms = 1_000_000u64;
    let u1 = serde_json::json!([{ "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 }]);
    let recs = [
        rec(1, 0, Channel::Track, track_list("r", u1.clone(), serde_json::json!({ "200": vrow("cam-u1") }))),
        rec(2, ms, Channel::Rtp(200), pkt(1, 3000, true, Some(9), KEY)),
        rec(3, 2 * ms, Channel::Track, track_list("r", u1, serde_json::json!({ "300": vrow("cam-u1") }))),
        rec(4, 40 * ms, Channel::Rtp(300), pkt(77, 500_000, true, Some(9), KEY)),
    ];
    for p in [Policy::Utterance, Policy::Span { time_ns: u64::MAX, bytes: u64::MAX }] {
        let out = run_pipe(p, &recs);
        assert_eq!(segs(&out), vec![(SegmentKind::Opened, 0, "cam-u1".into(), None), (SegmentKind::Closed, 0, "cam-u1".into(), Some("end"))], "{p:?}");
    }
}

#[test]
fn slot_video_segment_opens_only_at_keyframe_of_new_source() {
    let ms = 1_000_000u64;
    let recs = [
        rec(1, 0, Channel::Track, track_list("r", serde_json::json!([{ "track_id": "slot-v", "kind": "video" }]), serde_json::json!({ "400": vrow("slot-v") }))),
        rec(1, 0, Channel::Floor, br#"{"room":"r","event":"taken","user_id":"A","sources":[7],"seq":1}"#.to_vec()),
        rec(2, ms, Channel::Rtp(400), pkt(1, 3000, true, Some(7), KEY)),
        rec(3, 2 * ms, Channel::Rtp(400), pkt(2, 6000, true, Some(7), DELTA)),
        rec(3, 2 * ms, Channel::Floor, br#"{"room":"r","event":"taken","user_id":"B","sources":[8],"seq":2}"#.to_vec()),
        rec(4, 3 * ms, Channel::Rtp(400), pkt(3, 9000, true, Some(8), DELTA)),
        rec(5, 4 * ms, Channel::Rtp(400), pkt(4, 12000, true, Some(8), KEY)),
    ];
    let mut pl = Pipeline::new(Policy::Utterance);
    let mut out: Vec<Output> = recs.iter().flat_map(|r| pl.push(r)).collect();
    out.extend(pl.finish());
    let s = segs(&out);
    assert_eq!(s[1], (SegmentKind::Closed, 0, "slot-v".into(), Some("taken")));
    assert_eq!(s[2], (SegmentKind::Opened, 1, "slot-v".into(), None));
    let first_of_1 = out.iter().find_map(|o| if let Output::Frame(f) = o && f.segment == Some(1) { Some((f.rtp_ts, f.keyframe)) } else { None });
    assert_eq!(first_of_1, Some((12000, true)), "새 화자의 첫 키프레임부터");
    assert_eq!(pl.counts().before_key, 1);
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

// ── 5단계(202611/20261110b) — HLS 결과물(fMP4 · 재생 목록) ──

const SPS_A: &[u8] = &[0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16, 0xe4];
const SPS_B: &[u8] = &[0x67, 0x42, 0xc0, 0x1f, 0xda, 0x02, 0x80, 0x2d, 0xc8];
const PPS: &[u8] = &[0x68, 0xce, 0x3c, 0x80];

/// 사람 u1 — 영상(H264 · 줄 200 · CSRC 9) · 음성(Opus · 줄 100 · CSRC 10) · 같은 시계에서 `secs` 초 · 영상 키 1초마다 · SR 은 0.5초에(둘 다)
fn av_stream(secs: u32, start_v: u32, sps_switch_at: Option<u32>) -> Vec<Record> {
    av_stream_gop(secs, start_v, sps_switch_at, 30)
}

/// `gop` 프레임마다 키프레임(33ms 간격 — 30 이면 약 1초)
fn av_stream_gop(secs: u32, start_v: u32, sps_switch_at: Option<u32>, gop: u32) -> Vec<Record> {
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

fn hls_build(recs: &[Record], dir: &Path) -> (Vec<HlsStream>, Vec<String>) {
    let mut pl = Pipeline::new(Policy::Span { time_ns: u64::MAX, bytes: u64::MAX });
    let mut sink = HlsSink::new(dir, HlsConfig::default()).unwrap();
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
    (sink.streams().to_vec(), asked)
}

/// 트랙 번호 · tfdt · [(길이, 크기, 플래그)]
type MoofTraf = (u32, u64, Vec<(u32, u32, u32)>);

/// fMP4 조각 → 트랙마다 (tfdt, [(길이, 크기, 플래그)]) — 상자를 걸어 읽는다
fn read_moof(b: &[u8]) -> Vec<MoofTraf> {
    fn boxes(b: &[u8]) -> Vec<([u8; 4], &[u8])> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 8 <= b.len() {
            let n = u32::from_be_bytes(b[i..i + 4].try_into().unwrap()) as usize;
            out.push((b[i + 4..i + 8].try_into().unwrap(), &b[i + 8..i + n]));
            i += n;
        }
        out
    }
    let mut out = Vec::new();
    for (t, body) in boxes(b) {
        if &t != b"moof" {
            continue;
        }
        for (t, traf) in boxes(body) {
            if &t != b"traf" {
                continue;
            }
            let (mut id, mut tfdt, mut rows) = (0, 0, Vec::new());
            for (t, x) in boxes(traf) {
                match &t {
                    b"tfhd" => id = u32::from_be_bytes(x[4..8].try_into().unwrap()),
                    b"tfdt" => tfdt = u64::from_be_bytes(x[4..12].try_into().unwrap()),
                    b"trun" => {
                        let n = u32::from_be_bytes(x[4..8].try_into().unwrap()) as usize;
                        for k in 0..n {
                            let o = 12 + k * 12;
                            let r = |a: usize| u32::from_be_bytes(x[a..a + 4].try_into().unwrap());
                            rows.push((r(o), r(o + 4), r(o + 8)));
                        }
                    }
                    _ => {}
                }
            }
            out.push((id, tfdt, rows));
        }
    }
    out
}

#[test]
fn hls_segment_over_target_is_counted_and_logged_as_diag_warn() {
    let d = tmp("hls-over");
    let spool = d.join("spool");
    std::fs::create_dir_all(&spool).unwrap();
    let log = crate::log::LogConfig { socket: Some(d.join("no-such.sock")), spool_dir: Some(spool.clone()), node: "t".into(), ..crate::log::LogConfig::default() };
    let mut pl = Pipeline::new(Policy::Span { time_ns: u64::MAX, bytes: u64::MAX });
    let mut sink = HlsSink::new(d.join("hls"), HlsConfig { log: Some(log), ..HlsConfig::default() }).unwrap();
    for r in &av_stream_gop(12, 1000, None, 240) {
        for o in pl.push(r) {
            sink.write(&o).unwrap();
        }
    }
    for o in pl.finish() {
        sink.write(&o).unwrap();
    }
    sink.finish().unwrap();
    let s = &sink.streams()[0];
    let over: Vec<usize> = s.segments.iter().enumerate().filter(|(_, x)| x.duration.round() as u64 > 6).map(|(i, _)| i).collect();
    assert_eq!(s.over_target, over.len() as u64);
    assert!(!over.is_empty(), "키프레임이 8초마다라 첫 조각이 ⌈1.5 × 4⌉ = 6초를 넘는다: {:?}", s.segments.iter().map(|x| x.duration).collect::<Vec<_>>());
    let file = spool.join("t.peer.spool.log");
    let mut lines = Vec::new();
    for _ in 0..200 {
        lines = std::fs::read_to_string(&file).unwrap_or_default().lines().filter(|l| l.contains("over target")).map(str::to_string).collect();
        if lines.len() == over.len() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(lines.len(), over.len(), "넘은 조각마다 한 줄");
    let l = &lines[0];
    assert!(l.contains("kind=diag") && l.contains("spec=SD-RULE-PLAT-REC-HLS"), "{l}");
    assert!(l.contains(&format!("in.segment={}", over[0])) && l.contains("in.target_s=6"), "{l}");
}

#[test]
fn hls_cuts_at_keyframe_after_target_with_continuous_decode_time() {
    let d = tmp("hls-av");
    let (st, asked) = hls_build(&av_stream(11, 1000, None), &d);
    assert_eq!(st.len(), 1, "사람 하나 = 묶음 하나");
    let s = &st[0];
    assert_eq!(s.base, Some("utc"), "두 트랙 모두 SR 이 있다");
    assert_eq!(s.inits.len(), 1);
    assert_eq!(s.segments.iter().map(|x| x.duration.round() as u64).collect::<Vec<_>>(), vec![4, 4, 3], "4초 지난 첫 키(1초마다)에서");
    assert!(s.segments.iter().all(|x| !x.discontinuity));
    assert_eq!(s.over_target, 0);
    assert_eq!(asked.len(), 3, "조각마다 T − 1초에 한 번");
    let mut next: HashMap<u32, u64> = HashMap::new();
    for x in &s.segments {
        let m = read_moof(&std::fs::read(&x.file).unwrap());
        let v = m.iter().find(|t| t.0 == 1).unwrap();
        assert_eq!(v.2[0].2, 0x0200_0000, "조각의 첫 영상 샘플은 키");
        for (id, tfdt, rows) in &m {
            if let Some(want) = next.get(id) {
                assert_eq!(tfdt, want, "트랙 {id} 디코드 시각이 앞 조각 끝에서 잇는다");
            }
            next.insert(*id, tfdt + rows.iter().map(|r| u64::from(r.0)).sum::<u64>());
        }
    }
    let ev = std::fs::read_to_string(&s.event_playlist).unwrap();
    assert!(ev.contains("#EXT-X-PLAYLIST-TYPE:EVENT") && ev.contains("#EXT-X-TARGETDURATION:6") && ev.trim_end().ends_with("#EXT-X-ENDLIST"));
    assert_eq!(ev.matches("#EXTINF").count(), 3);
    assert_eq!(ev.matches("#EXT-X-MAP").count(), 1);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hls_samples_drop_parameter_sets_and_init_carries_them() {
    let d = tmp("hls-ps");
    let (st, _) = hls_build(&av_stream(5, 1000, None), &d);
    let init = std::fs::read(&st[0].inits[0]).unwrap();
    assert!(init.windows(SPS_A.len()).any(|w| w == SPS_A), "avcC 에 SPS");
    let seg = std::fs::read(&st[0].segments[0].file).unwrap();
    assert!(!seg.windows(SPS_A.len()).any(|w| w == SPS_A), "샘플에는 SPS 가 없다(avc1)");
    assert!(seg.windows(8).any(|w| w == [0, 0, 0, 4, 0x65, 0x88, 0x84, 0]), "IDR 은 길이 앞붙임");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hls_is_deterministic_for_same_records() {
    let (a, b) = (tmp("hls-d1"), tmp("hls-d2"));
    let recs = av_stream(9, 1000, None);
    let (sa, _) = hls_build(&recs, &a);
    let (sb, _) = hls_build(&recs, &b);
    let files = |s: &HlsStream| -> Vec<PathBuf> { s.inits.iter().chain(s.segments.iter().map(|x| &x.file)).chain([&s.event_playlist, &s.live_playlist]).cloned().collect() };
    for (x, y) in files(&sa[0]).iter().zip(files(&sb[0]).iter()) {
        assert_eq!(x.file_name(), y.file_name());
        assert_eq!(std::fs::read(x).unwrap(), std::fs::read(y).unwrap(), "{}", x.display());
    }
    std::fs::remove_dir_all(&a).ok();
    std::fs::remove_dir_all(&b).ok();
}

#[test]
fn hls_parameter_change_opens_new_init_with_discontinuity() {
    let d = tmp("hls-res");
    let (st, _) = hls_build(&av_stream(10, 1000, Some(5000)), &d);
    let s = &st[0];
    assert_eq!(s.inits.len(), 2, "SPS 가 바뀐 키프레임에서 새 init");
    let k = s.segments.iter().position(|x| x.discontinuity).expect("끊김 조각");
    assert_ne!(s.segments[k].init, s.segments[k - 1].init);
    let ev = std::fs::read_to_string(&s.event_playlist).unwrap();
    assert_eq!(ev.matches("#EXT-X-DISCONTINUITY\n").count(), 1);
    assert_eq!(ev.matches("#EXT-X-MAP").count(), 2);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hls_rtp_wrap_is_unfolded_not_a_discontinuity() {
    let d = tmp("hls-wrap");
    let (st, _) = hls_build(&av_stream(9, u32::MAX - 90_000 * 3, None), &d);
    let s = &st[0];
    assert!(s.segments.iter().all(|x| !x.discontinuity), "넘침은 펼친다");
    let mut last = 0;
    for x in &s.segments {
        let v = read_moof(&std::fs::read(&x.file).unwrap()).into_iter().find(|t| t.0 == 1).unwrap();
        assert!(v.1 >= last, "영상 디코드 시각이 앞으로만");
        last = v.1;
    }
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hls_refuses_non_h264_video_with_reason() {
    let d = tmp("hls-vp8");
    let ms = 1_000_000u64;
    let u1 = serde_json::json!([{ "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 }]);
    let recs = [
        rec(1, 0, Channel::Track, track_list("r", u1, serde_json::json!({ "200": vrow("cam-u1") }))),
        rec(2, ms, Channel::Rtp(200), pkt(1, 3000, true, Some(9), KEY)),
    ];
    let (st, _) = hls_build(&recs, &d);
    assert!(st[0].refused.as_deref().is_some_and(|r| r.contains("VP8")));
    assert!(st[0].segments.is_empty() && !d.join("r-u1").exists(), "거절한 묶음은 아무것도 쓰지 않는다");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hls_parameter_change_before_first_segment_keeps_old_parameters_for_earlier_frames() {
    // 첫 조각(4초)을 쓰기 전(2초)에 SPS 가 바뀐다 — 앞 프레임이 새 SPS 의 init 에 들면 디코드가 깨진다(media_check 실측 20261110)
    let d = tmp("hls-res-early");
    let (st, _) = hls_build(&av_stream(8, 1000, Some(2000)), &d);
    let s = &st[0];
    assert_eq!(s.inits.len(), 2);
    assert!(std::fs::read(&s.inits[0]).unwrap().windows(SPS_A.len()).any(|w| w == SPS_A), "첫 init 은 앞 SPS");
    assert!(std::fs::read(&s.inits[1]).unwrap().windows(SPS_B.len()).any(|w| w == SPS_B), "둘째 init 은 바뀐 SPS");
    assert!(s.segments[1].discontinuity && (s.segments[0].duration - 3.0).abs() < 0.01, "바뀐 첫 키프레임(2초 뒤 첫 키 = 90번째 프레임 · 3.0초)에서 자른다");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hls_audio_decode_time_is_cumulative_across_segments_despite_plc_remainder() {
    // 무전 슬롯 화자 바뀜처럼 RTP 시각이 프레임 배수가 아닌 폭(+672)으로 뛰면 PLC 가 메우고 나머지가 남는다 —
    // 조각 사이 디코드 시각은 견본 길이 누적으로 잇는다(2층 peer_record_hls_ptt 실측 20261110 — 672 표본 끊김)
    let ms = 1_000_000u64;
    let tracks = serde_json::json!([{ "track_id": "slot-a", "kind": "audio" }]);
    let rows = serde_json::json!({ "100": { "track_id": "slot-a", "kind": "audio", "codec": "opus", "pt": 111 } });
    let mut recs = vec![rec(1, 0, Channel::Track, track_list("r", tracks, rows))];
    let mut ts = 0u32;
    for i in 0..500u32 {
        if i == 100 {
            ts += 672 + 960 * 3;
        }
        recs.push(rec(2 + i, u64::from(i) * 20 * ms, Channel::Rtp(100), pkt(i as u16, ts, true, Some(7), &[0x78, i as u8])));
        ts += 960;
    }
    let d = tmp("hls-plc");
    let (st, _) = hls_build(&recs, &d);
    let s = &st[0];
    assert!(s.segments.len() >= 2);
    let mut next: Option<u64> = None;
    for x in &s.segments {
        let m = read_moof(&std::fs::read(&x.file).unwrap());
        let a = m.iter().find(|t| t.0 == 2).unwrap();
        if let Some(n) = next {
            assert_eq!(a.1, n, "{} 음성 디코드 시각이 앞 조각 끝에서 잇는다", x.file.display());
        }
        next = Some(a.1 + a.2.iter().map(|r| u64::from(r.0)).sum::<u64>());
    }
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

#[test]
fn hls_same_time_frame_on_layer_switch_replaces_previous_sample() {
    // 시뮬캐스트 층 바꿈 — 4초 지점 h 키프레임(T 로 자르는 프레임) 바로 뒤(+90 틱 = 1ms · 2층 실측)에 l 키프레임(다른 SPS)이 온다.
    // 같은 순간의 그림이니 앞 견본을 바꿔 끼워 1ms 조각이 없어야 한다(2층 peer_record_hls_simulcast 실측 20261110 — seg-00001 0.001초)
    let ms = 1_000_000u64;
    let tracks = serde_json::json!([{ "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 }]);
    let rows = serde_json::json!({ "200": { "track_id": "cam-u1", "kind": "video", "codec": "H264", "pt": 102 } });
    for off in [0u32, 90] {
        let mut recs = vec![rec(1, 0, Channel::Track, track_list("r", tracks.clone(), rows.clone()))];
        let (mut n, mut seq) = (2u32, 0u16);
        let mut put = |recs: &mut Vec<Record>, ts: u32, i: u32, sps: Option<&[u8]>| {
            let idr = [0x65, 0x88, i as u8];
            let delta = [0x41, 0x9a, i as u8];
            let nal: Vec<&[u8]> = match sps {
                Some(s) => vec![s, PPS, &idr],
                None => vec![&delta],
            };
            for (k, x) in nal.iter().enumerate() {
                recs.push(rec(n, u64::from(i) * 33 * ms, Channel::Rtp(200), pkt(seq, ts, k + 1 == nal.len(), Some(9), x)));
                n += 1;
                seq = seq.wrapping_add(1);
            }
        };
        let mut base = 0u32;
        for i in 0..240u32 {
            let key = i.is_multiple_of(30);
            if i == 120 {
                put(&mut recs, i * 3000, i, Some(SPS_A));
                base = off;
            }
            put(&mut recs, i * 3000 + base, i, if key { Some(if i < 120 { SPS_A } else { SPS_B }) } else { None });
        }
        let d = tmp(&format!("hls-dup-{off}"));
        let (st, _) = hls_build(&recs, &d);
        let s = &st[0];
        assert_eq!(s.replaced, 1, "+{off} 틱 — 같은 순간 앞 견본을 바꿔 꼈다");
        assert!(s.segments.iter().all(|x| x.duration > 0.5), "+{off} 틱 — 견본 하나짜리 조각이 없다 {:?}", s.segments.iter().map(|x| x.duration).collect::<Vec<_>>());
        assert_eq!((s.inits.len(), s.segments.iter().filter(|x| x.discontinuity).count()), (2, 1));
        std::fs::remove_dir_all(&d).ok();
    }
}

// ── 김대리 검토 #13(202611/20261110e) — 단계 안 시스템 시계 금지(SDK:RULE-PLAT-REC-PIPE 「★시계 금지」 · 설계 D7)를 기계로 ──

#[test]
fn pipeline_stages_and_sinks_never_read_the_system_clock() {
    // 기록기(`mod.rs`)만 `log_time` 닻으로 시계를 읽는다. 단계 · 결과물 · 꼬리 읽기 · 파일 읽기는 레코드의 시각만 쓴다 — 「라이브 = 파일」의 전제
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/record");
    let mut seen = 0;
    for f in ["pipeline.rs", "ogg.rs", "mkv.rs", "hls.rs", "live.rs", "file.rs"] {
        let src = std::fs::read_to_string(dir.join(f)).unwrap();
        // 시험 모듈(`#[cfg(test)]` 뒤)은 검사 밖 — 본문만
        let body = src.split("#[cfg(test)]").next().unwrap_or(&src);
        for bad in ["Instant::now", "SystemTime::now", "SystemTime", "UNIX_EPOCH", "chrono::"] {
            assert!(!body.contains(bad), "{f} 가 시스템 시계({bad})를 쓴다 — 단계 안 시계 금지(D7)");
        }
        seen += 1;
    }
    assert_eq!(seen, 6);
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

/// SR 을 `sr_at_ms` 에 받고(늦은 SR) · 영상이 `video_delay_ms` 늦게 닿는 사람 하나(도착 차 = 늦은 SR 맞춤이 고칠 오차)
fn late_sr_av(secs: u32, sr_at_ms: u64, video_delay_ms: u64) -> Vec<Record> {
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

#[test]
fn mkv_log_time_file_with_late_sr_moves_video_onto_audio_and_anchors_date_on_audio() {
    // SR 이 여는 보류(2초) 뒤에 온다 → 파일은 log_time 기준. 영상이 60ms 늦게 닿아 도착 차만큼 어긋난다(D7)
    //   ★늦은 SR 맞춤(SDK:RULE-PLAT-REC-HLS) — 두 트랙이 UTC 를 받으면 영상만 견본마다 5ms 이하로 옮겨 목표 −60ms 에 닿는다 · DateUTC 닻 = 음성
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
fn hls_log_time_stream_with_late_sr_moves_video_onto_audio_without_discontinuity() {
    // SR 이 첫 조각(4초) 뒤 6초에 온다 → 묶음은 log_time 기준 · 영상이 60ms 늦게 닿는다(D7 — 2층 장시간 판 49 ~ 65ms)
    //   ★늦은 SR 맞춤 — 끊김 없이 영상 견본 길이로 다가가 · 뒤 조각의 영상 − 음성 디코드 시각 차가 SR 이 일찍 온 판(utc 기준)과 같아진다
    let d = tmp("hls-late-sr");
    let late = late_sr_av(12, 6_000, 60);
    let (st, _) = hls_build(&late, &d.join("late"));
    let (ctl, _) = hls_build(&late_sr_av(12, 500, 60), &d.join("ctl"));
    assert_eq!((st[0].base, ctl[0].base), (Some("log_time"), Some("utc")));
    assert!(st[0].video_shift_us.is_some_and(|x| (x + 60_000).abs() <= 1_000), "영상 목표 −60ms · {:?}", st[0].video_shift_us);
    assert_eq!(ctl[0].video_shift_us, None);
    assert!(st[0].segments.iter().all(|x| !x.discontinuity), "다시 맞추는 끊김은 없다");
    assert_eq!(st[0].segments.len(), ctl[0].segments.len());
    let av = |s: &HlsStream, i: usize| -> f64 {
        let m = read_moof(&std::fs::read(&s.segments[i].file).unwrap());
        let tf = |id: u32| m.iter().find(|t| t.0 == id).map(|t| t.1).unwrap();
        (tf(1) as f64 / 90_000.0 - tf(2) as f64 / 48_000.0) * 1000.0
    };
    let last = st[0].segments.len() - 1;
    assert!((av(&st[0], last) - av(&ctl[0], last)).abs() <= 1.0, "마지막 조각 영상 − 음성 {:.1}ms · utc 기준 판 {:.1}ms", av(&st[0], last), av(&ctl[0], last));
    assert!((av(&st[0], 0) - av(&ctl[0], 0) - 60.0).abs() <= 1.0, "SR 전 첫 조각은 도착 차 60ms 그대로(기다리지 않는다)");
    let durs: Vec<u32> = st[0].segments.iter().flat_map(|x| read_moof(&std::fs::read(&x.file).unwrap()).into_iter().filter(|t| t.0 == 1).flat_map(|t| t.2.into_iter().map(|r| r.0))).collect();
    assert!(durs.iter().all(|d| (2_550..=3_450).contains(d)), "영상 견본 길이는 한 프레임(3,000 틱) ± 5ms(450 틱) 안 — 한 번에 옮기지 않는다: {:?}", durs.iter().filter(|d| !(2_550..=3_450).contains(*d)).collect::<Vec<_>>());
    assert!(durs.iter().filter(|d| **d != 3_000).count() >= 12, "60ms 를 5ms 씩 — 바뀐 견본이 12 이상");
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
    // SDK:RULE-PLAT-REC-MKV — 서버 발언 id 가 오면 무전 슬롯 파일 이름 = `{방}-{발언 번호}-{화자}.mkv`(Ogg 와 짝)
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

fn long_opus(n: u32) -> Vec<Record> {
    opus_stream(&(0..n).map(|i| (i as u16, i * 960)).collect::<Vec<_>>(), 0x78)
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
    assert_eq!(size, Some(b.len() as u64 - s.seg_data), "SDK:RULE-PLAT-REC-MKV 닫기 — Segment 크기 = 파일 끝(크기 모름 꼴이 아니다)");
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
    (dur, cue_points.expect("SDK:RULE-PLAT-REC-MKV 닫기 — Cues"))
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
    assert!(good.iter().zip(&b).enumerate().all(|(i, (x, y))| x == y || (at..at + 8).contains(&i)), "SDK:RULE-PLAT-REC-MKV 닫기 — 같은 고쳐 쓰기(Duration 만 다르다)");
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
fn key_request_asks_at_target_minus_lead_and_once_more_at_target_after_an_early_key() {
    use KeyAsk::*;
    let k = |st, key, el| key_request(st, key, el, 4_000, 1_000);
    assert_eq!(k(Idle, false, 2_999), (false, Idle), "SDK:RULE-PLAT-REC-HLS before T - 1");
    assert_eq!(k(Idle, false, 3_000), (true, Asked), "SDK:RULE-PLAT-REC-HLS T - 1 asks");
    assert_eq!(k(Idle, true, 3_000), (true, Asked), "a key exactly at T - 1 was not answered yet - ask");
    assert_eq!(k(Asked, false, 3_500), (false, Asked));
    assert_eq!(k(Asked, true, 3_150), (false, Early), "a key before T cannot cut");
    assert_eq!(k(Early, false, 3_900), (false, Early), "the second ask waits for T");
    assert_eq!(k(Early, false, 4_000), (true, Again), "SDK:RULE-PLAT-REC-HLS ask once more at T");
    assert_eq!(k(Again, true, 4_100), (false, Again), "at most two asks per segment");
    assert_eq!(k(Again, false, 9_000), (false, Again), "a publisher that ignores both is counted over target, not asked again");
    assert_eq!(k(Asked, true, 4_000), (false, Asked), "a key at T cuts - the sink resets");
    assert_eq!(k(Idle, true, 0), (false, Idle), "the cutting key opens the next segment unasked");
    assert_eq!(key_request(Idle, true, 0, 1_000, 1_000), (true, Asked), "T at or under the lead asks at once");
}

fn pli_step(now: u32, pending: bool, latency: u32, plis: &mut Vec<u32>, due: &mut Vec<u32>) -> bool {
    if pending && plis.last().is_none_or(|t| now.saturating_sub(*t) >= 1_000) {
        plis.push(now);
        due.push(now + latency);
        return false;
    }
    pending
}

fn av_on_request(secs: u32, sps_switch_at: Option<u32>, latency: u32, feed: &mut dyn FnMut(&Record) -> usize) -> Vec<u32> {
    let ms = 1_000_000u64;
    let t0 = 1_800_000_000_000_000_000u64;
    let tracks = serde_json::json!([
        { "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 },
        { "track_id": "mic-u1", "kind": "audio", "user_id": "u1", "csrc": 10 }
    ]);
    let rows = serde_json::json!({ "200": { "track_id": "cam-u1", "kind": "video", "codec": "H264", "pt": 102 }, "100": { "track_id": "mic-u1", "kind": "audio", "codec": "opus", "pt": 111 } });
    let (mut plis, mut due) = (Vec::new(), Vec::new());
    let mut n = 1u32;
    let asked = feed(&rec(n, t0, Channel::Track, track_list("r", tracks, rows)));
    let mut pending = pli_step(0, asked > 0, latency, &mut plis, &mut due);
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
            let asked = feed(r);
            pending = pli_step(now, pending || asked > 0, latency, &mut plis, &mut due);
        }
    }
    plis
}

fn hls_on_request(secs: u32, sps_switch_at: Option<u32>, latency: u32, dir: &Path) -> (HlsStream, Vec<u32>) {
    let mut pl = Pipeline::new(Policy::Span { time_ns: u64::MAX, bytes: u64::MAX });
    let mut sink = HlsSink::new(dir, HlsConfig::default()).unwrap();
    let plis = av_on_request(secs, sps_switch_at, latency, &mut |r| {
        for o in pl.push(r) {
            sink.write(&o).unwrap();
        }
        sink.take_keyframe_requests().len()
    });
    for o in pl.finish() {
        sink.write(&o).unwrap();
    }
    sink.finish().unwrap();
    (sink.streams()[0].clone(), plis)
}

#[test]
fn hls_publisher_that_keys_only_on_request_gets_segments_near_target() {
    let d = tmp("hls-on-request");
    let (s, plis) = hls_on_request(14, None, 150, &d);
    let durs: Vec<f64> = s.segments.iter().map(|x| x.duration).collect();
    assert!(durs.len() >= 3, "SDK:RULE-PLAT-REC-HLS 14 s at T 4 s: {durs:?} plis {plis:?}");
    assert!(durs[..durs.len() - 1].iter().all(|x| (4.0..=4.5).contains(x)), "SDK:RULE-PLAT-REC-HLS a key before T asks once more at T - segments end just past T: {durs:?} plis {plis:?}");
    assert!(durs[durs.len() - 1] < 3.0, "{durs:?}");
    assert_eq!(s.over_target, 0, "{durs:?}");
    assert!(s.segments.iter().all(|x| !x.discontinuity));
    assert_eq!(s.keyframe_requests as usize, 2 * (durs.len() - 1), "SDK:RULE-PLAT-REC-HLS two asks per closed segment (T - 1 and T): {plis:?}");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hls_cut_by_parameter_change_does_not_spend_the_next_segments_request() {
    let d = tmp("hls-on-request-sps");
    let (s, plis) = hls_on_request(14, Some(7_500), 150, &d);
    let durs: Vec<f64> = s.segments.iter().map(|x| x.duration).collect();
    assert!(s.segments.iter().any(|x| x.discontinuity), "SDK:RULE-PLAT-REC-HLS parameter change cuts: {durs:?}");
    assert!(!plis.iter().any(|t| (7_400..10_400).contains(t)), "the segment opened at 7.5 s by a parameter change asks at its own T - 1 (10.5 s), not at its start: {plis:?} {durs:?}");
    assert!(plis.iter().any(|t| (10_400..10_700).contains(t)), "{plis:?} {durs:?}");
    std::fs::remove_dir_all(&d).ok();
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
    assert!(fs.len() >= 3, "SDK:RULE-PLAT-REC-MKV span 4 s over 14 s: {frames:?} plis {plis:?}");
    assert!(fs[..fs.len() - 1].iter().all(|f| f.reason.as_deref() == Some("span")), "{frames:?}");
    assert!(frames[..frames.len() - 1].iter().all(|n| (120..=135).contains(n)), "SDK:RULE-PLAT-REC-MKV a key before T asks once more at T - files end just past T: {frames:?} plis {plis:?}");
    assert_eq!(asks, 2 * (fs.len() - 1), "SDK:RULE-PLAT-REC-MKV two asks per closed file: {plis:?}");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn a_key_at_target_after_an_early_key_cuts_without_a_second_ask() {
    let recs = av_stream_gop(10, 1000, None, 15);
    let d = tmp("mkv-early-then-cut");
    let cfg = MkvConfig { span: Some(Duration::from_secs(4)), ..MkvConfig::default() };
    let (fs, asked) = mkv_av_build(&recs, &d, Policy::Span { time_ns: u64::MAX, bytes: u64::MAX }, cfg);
    assert_eq!(fs.len(), 3, "SDK:RULE-PLAT-REC-MKV keys every 0.5 s - cut at 4 s and 8 s");
    assert_eq!(asked.len(), 2, "SDK:RULE-PLAT-REC-MKV T - 1 asks, the 3.5 s key is early, the 4.0 s key cuts before any second ask: {asked:?}");
    std::fs::remove_dir_all(&d).ok();
    let d = tmp("hls-early-then-cut");
    let (st, asked) = hls_build(&recs, &d);
    assert_eq!(st[0].segments.len(), 3, "SDK:RULE-PLAT-REC-HLS");
    assert_eq!(asked.len(), 2, "SDK:RULE-PLAT-REC-HLS the cutting key at T is not asked again: {asked:?}");
    std::fs::remove_dir_all(&d).ok();
}
