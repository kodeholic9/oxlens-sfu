// author: kodeholic (powered by Claude)
//! 1층 — 녹음 원시 기록(SDK:RULE-PLAT-REC-RAW · REC-CLOCK · REC-PART): 왕복 · 회전 · 사슬 · 머리 스냅숏 · 잘린 파트 · 디스크 하한 · 파이프라인(REC-PIPE · REC-SPLIT · REC-GAP)

use std::path::Path;
use std::time::Duration;

use tokio::sync::mpsc;

use std::sync::Arc;

use super::testkit::*;
use super::*;

fn quiet() -> Arc<crate::log::Logger> {
    crate::log::Logger::start(crate::log::LogConfig { enabled: false, ..crate::log::LogConfig::default() })
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
    let head: serde_json::Value = serde_json::from_slice(&got[0].data).unwrap();
    let rows = head["rows"].as_object().unwrap();
    let ssrcs: Vec<u32> = got.iter().filter_map(|x| if let Channel::Rtp(s) = x.channel { Some(s) } else { None }).collect();
    assert!(!ssrcs.is_empty());
    assert!(ssrcs.iter().all(|s| rows.contains_key(&s.to_string())), "늦게 연 통이 줄 표를 먼저 받아 RTP 줄을 모두 안다");
}

// ── 김대리 검토 #13(202611/20261110e) — 단계 안 시스템 시계 금지(SDK:RULE-PLAT-REC-PIPE 「★시계 금지」 · 설계 D7)를 기계로 ──

#[test]
fn pipeline_stages_and_tap_reading_never_read_the_system_clock() {
    // 기록기(`mod.rs`)만 `log_time` 닻으로 시계를 읽는다. 단계 · 결과물 · 꼬리 읽기 · 파일 읽기는 레코드의 시각만 쓴다 — 「라이브 = 파일」의 전제
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/record");
    let mut seen = 0;
    for f in ["pipeline.rs", "live.rs", "file.rs"] {
        let src = std::fs::read_to_string(dir.join(f)).unwrap();
        // 시험 모듈(`#[cfg(test)]` 뒤)은 검사 밖 — 본문만
        let body = src.split("#[cfg(test)]").next().unwrap_or(&src);
        for bad in ["Instant::now", "SystemTime::now", "SystemTime", "UNIX_EPOCH", "chrono::"] {
            assert!(!body.contains(bad), "{f} 가 시스템 시계({bad})를 쓴다 — 단계 안 시계 금지(D7)");
        }
        seen += 1;
    }
    assert_eq!(seen, 3);
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

#[test]
fn a_room_snapshot_is_a_fact_unless_the_room_is_rejoining() {
    use RoomPhase::*;
    assert!(track_fact(Joined, 1, 1));
    assert!(track_fact(Joined, 0, 0), "a joined room that really has no tracks");
    assert!(!track_fact(Joined, 1, 0), "joined again but its lines are not up yet");
    assert!(!track_fact(Rejoining, 0, 0), "the session dropped the room and will rejoin: the list is unknown, not empty");
    assert!(!track_fact(Rejoining, 0, 1));
    assert!(track_fact(Gone, 0, 0), "left, kicked or the room is gone: the tracks did go away");
}

fn slot_segments(recs: &[Record]) -> Vec<(Option<String>, Option<&'static str>, u64, u64)> {
    run_pipe(Policy::Utterance, recs)
        .into_iter()
        .filter_map(|o| if let Output::Segment(s) = o && s.event == SegmentKind::Closed && s.track_id == "slot-a" { Some((s.talk, s.reason, s.frames, s.filled)) } else { None })
        .collect()
}

#[test]
fn a_session_cut_and_rejoin_keeps_one_talk_in_one_segment_with_the_hole_filled() {
    let now = slot_segments(&cut_and_rejoin(false));
    assert_eq!(now.len(), 1, "SDK:RULE-PLAT-REC-API 주인 — 새 세션 · 새 줄로 이어져도 같은 발언은 한 세그먼트: {now:?}");
    let (talk, reason, frames, filled) = now[0].clone();
    assert_eq!((talk.as_deref(), reason), (Some(CUT_TALK), Some("idle")));
    assert!((90..=100).contains(&filled), "SDK:RULE-PLAT-REC-GAP — 끊긴 1.94 s 는 log_time 차로 재어 PLC 로 메운다: {filled}");
    assert_eq!(frames, 176 + 212 + filled);
    let before = slot_segments(&cut_and_rejoin(true));
    assert_eq!(before.iter().map(|s| s.1).collect::<Vec<_>>(), vec![Some("track"), Some("idle")], "as recorded before: the false empty snapshots split the talk: {before:?}");
}

#[test]
fn a_taken_with_the_same_talk_id_is_the_same_talk_whatever_its_seq() {
    let mut recs = cut_and_rejoin(false);
    recs.retain(|r| r.channel != Channel::Track || r.log_time == 0);
    for r in recs.iter_mut().filter(|r| matches!(r.channel, Channel::Rtp(200))) {
        r.channel = Channel::Rtp(100);
        let s = u16::from_be_bytes([r.data[2], r.data[3]]).wrapping_sub(96);
        r.data[2..4].copy_from_slice(&s.to_be_bytes());
    }
    let segs = slot_segments(&recs);
    assert_eq!(segs.len(), 1, "SDK:RULE-PLAT-REC-SPLIT 발언 — 발언 번호 = 발언 id · seq 가 달라도 같은 발언 id 의 taken 은 새 발언이 아니다: {segs:?}");
}
