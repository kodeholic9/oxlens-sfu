// author: kodeholic (powered by Claude)
// spec: v1.14 · 정:IMPL-OPS-REC-UNIT · 정:IMPL-CHAT-TALK · 정:RULE-OPS-DROPS · 연:RULE-CHAT-REC-FILE · model: claude-opus-5-5
//! 1층 녹음 유닛 — 쓰는 중은 `.rec-part/{node}/{시각}/` 아래 닫힐 경로 그대로 · 닫기는 덮어쓰지 않는 링크 걸기(겹치면 `-NNNN` · 경합에도 한쪽만) · 발언 파일 · 개인 트랙 녹화 · 기동 회수(마감 · 빈 조각 지움 · 실패 조각 `failed/` · 다른 node 는 안 건드림) · 보관 네 폴더 · 등록부 · 되살림 판정 · `rec` 알림 · Handle.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use common::envelope::Reply;
use common::talk::Report;
use oxmesh::{Config, Mesh};
use oxpeer::record::{Channel, Output, Pipeline, Policy, Record};
use oxtapd::consumer::{RecConsumer, TalkFiles, TrackFiles};
use oxtapd::{Layout, RecoverCtx, Registry, RoomCtx};
use tokio::sync::mpsc;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

const TALK: &str = "81330e4c0190a1b2c3d4e5f60718293a4b5c";

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("oxtapd-l1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn names(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d).map(|it| it.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    v.sort();
    v
}

fn pkt(seq: u16, ts: u32, csrc: u32, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0x81, 0xEF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    v[2..4].copy_from_slice(&seq.to_be_bytes());
    v[4..8].copy_from_slice(&ts.to_be_bytes());
    v.extend_from_slice(&csrc.to_be_bytes());
    v.extend_from_slice(payload);
    v
}

fn record(seq: u32, t_ms: u64, channel: Channel, data: Vec<u8>) -> Record {
    Record { sequence: seq, log_time: t_ms * 1_000_000, channel, data }
}

fn talk_stream() -> Vec<Record> {
    talks_in("r", 100, 100)
}

fn talks_in(room: &str, a_frames: u64, b_frames: u64) -> Vec<Record> {
    let tracks = serde_json::json!({
        "room": room,
        "tracks": [{ "track_id": "slot-a", "kind": "audio" }, { "track_id": "slot-v", "kind": "video" }],
        "rows": { "100": { "track_id": "slot-a", "kind": "audio", "codec": "opus", "pt": 111 }, "200": { "track_id": "slot-v", "kind": "video", "codec": "VP8", "pt": 96 } }
    });
    let mut recs = vec![record(1, 0, Channel::Track, tracks.to_string().into_bytes())];
    let (mut aseq, mut vseq) = (0u16, 0u16);
    for (src, t0, who, talk, video, frames) in [(7u32, 0u64, "A", "tA", true, a_frames), (8, 3_000, "B", "tB", false, b_frames)] {
        let n = recs.len() as u32 + 1;
        recs.push(record(n, t0, Channel::Floor, serde_json::json!({ "room": room, "event": "taken", "user_id": who, "sources": [src, src + 10], "seq": n, "talk": talk }).to_string().into_bytes()));
        for i in 0..frames {
            let t = t0 + i * 20;
            let n = recs.len() as u32 + 1;
            recs.push(record(n, t, Channel::Rtp(100), pkt(aseq, (t * 48) as u32, src, &[0x78, i as u8])));
            aseq = aseq.wrapping_add(1);
            if video && i % 2 == 1 {
                let key = i / 2 == 2;
                let n = recs.len() as u32 + 1;
                recs.push(record(n, t, Channel::Rtp(200), pkt(vseq, (t * 90) as u32, src + 10, if key { &[0x10, 0x00, 0x9d, 0x01, 0x2a] } else { &[0x10, 0x01, 7] })));
                vseq = vseq.wrapping_add(1);
            }
        }
    }
    recs
}

fn track_stream() -> Vec<Record> {
    let tracks = serde_json::json!({
        "room": "r",
        "tracks": [{ "track_id": "cam-u1", "kind": "video", "user_id": "u1", "csrc": 9 }, { "track_id": "mic-u1", "kind": "audio", "user_id": "u1", "csrc": 10 }],
        "rows": { "300": { "track_id": "cam-u1", "kind": "video", "codec": "VP8", "pt": 96 }, "400": { "track_id": "mic-u1", "kind": "audio", "codec": "opus", "pt": 111 } }
    });
    let mut recs = vec![record(1, 0, Channel::Track, tracks.to_string().into_bytes())];
    for i in 0..150u64 {
        let t = i * 20;
        let n = recs.len() as u32 + 1;
        recs.push(record(n, t, Channel::Rtp(400), pkt(i as u16, (t * 48) as u32, 10, &[0x78, i as u8])));
        if i % 2 == 0 {
            let n = recs.len() as u32 + 1;
            recs.push(record(n, t, Channel::Rtp(300), pkt((i / 2) as u16, (t * 90) as u32, 9, if i == 0 { &[0x10, 0x00, 0x9d, 0x01, 0x2a] } else { &[0x10, 0x01, 7] })));
        }
    }
    recs
}

fn ctx(layout: &Layout, run: &str) -> (RoomCtx, mpsc::UnboundedReceiver<Report>) {
    ctx_in(layout, run, "r")
}

fn ctx_in(layout: &Layout, run: &str, room: &str) -> (RoomCtx, mpsc::UnboundedReceiver<Report>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (RoomCtx { room: room.into(), run: run.into(), layout: layout.clone(), reports: tx }, rx)
}

fn crash_then_recover(l: &Layout, run: &str, room: &str, recs: &[Record]) -> Vec<(String, String, String, String)> {
    let _log = closed_lines();
    let (c, mut live) = ctx_in(l, run, room);
    let mut t = TalkFiles::open(&c).expect("talk");
    feed(&mut t, recs, None);
    drop(t);
    let (tx, mut rx) = mpsc::unbounded_channel();
    Registry::defaults().recover(&RecoverCtx { layout: l.clone(), runs: vec![l.run_dir(run)], reports: tx, reason: "crash" });
    let mut all = reports(&mut live);
    all.extend(reports(&mut rx));
    all.sort();
    all
}

#[test]
fn recovery_keeps_and_reports_talks_of_a_room_whose_id_is_not_a_plain_file_name() {
    let room = "회의 1";
    let l = Layout::new(root("recover-room"), "n1");
    let got = crash_then_recover(&l, "400", room, &talks_in(room, 100, 100));
    let talks: Vec<&str> = got.iter().map(|(t, _, _, _)| t.as_str()).collect();
    assert_eq!(talks, vec!["tA", "tB"], "정:IMPL-OPS-REC-UNIT 기동 회수 — 방 id 가 파일 이름 글자 밖이어도 발언을 찾아 알린다: {got:?}");
    assert_eq!(oxtapd::files::files_under(&l.closed("rec", room)).len(), 2, "both talks are kept under rec/{{room}}/");
}

#[test]
fn recovery_reports_an_audio_talk_as_ogg_and_a_video_talk_as_matroska() {
    let l = Layout::new(root("recover-kinds"), "n1");
    let got = crash_then_recover(&l, "500", "r", &talks_in("r", 100, 150));
    let b: Vec<&(String, String, String, String)> = got.iter().filter(|x| x.0 == "tB").collect();
    assert_eq!(b.len(), 1, "{got:?}");
    assert_eq!((b[0].2.as_str(), b[0].3.as_str()), ("audio/ogg", "crash"), "정:IMPL-OPS-REC-UNIT 파일 — 영상 트랙이 없는 발언은 .ogg(보류 뒤 음성만 연 matroska 는 버린다)");
    assert!(!names(&l.closed("rec", "r")).iter().any(|n| n.starts_with("r-tB-") && n.ends_with(".mkv")), "{:?}", names(&l.closed("rec", "r")));
    let cut: Vec<Record> = talks_in("r", 140, 0).into_iter().filter(|r| r.log_time < 2_700 * 1_000_000).collect();
    let l2 = Layout::new(root("recover-video"), "n1");
    let got = crash_then_recover(&l2, "600", "r", &cut);
    assert_eq!(got.iter().map(|x| (x.0.as_str(), x.2.as_str())).collect::<Vec<_>>(), vec![("tA", "video/x-matroska")], "정:IMPL-OPS-REC-UNIT — 영상 발언 가운데 죽어도 회수는 matroska(마감)를 그 발언의 녹취로 알린다: {got:?}");
    let kept = names(&l2.closed("rec", "r"));
    assert!(kept.contains(&"r-tA-A.mkv".to_string()), "{kept:?}");
    assert!(kept.contains(&"r-tA-A.ogg".to_string()), "the talk's ogg stays on disk as a second file — nothing recovered is deleted: {kept:?}");
}

#[test]
fn a_run_folder_locked_by_a_live_unit_is_not_recovered() {
    let l = Layout::new(root("recover-lock"), "n1");
    let (c, _rx) = ctx(&l, "700");
    let mut t = TalkFiles::open(&c).expect("talk");
    feed(&mut t, &talk_stream(), None);
    drop(t);
    let staged = |l: &Layout| oxtapd::files::files_under(&l.run_dir("700")).into_iter().filter(|p| !p.ends_with(oxtapd::files::LOCK_FILE)).collect::<Vec<_>>();
    let before = staged(&l);
    assert!(!before.is_empty(), "the crash left a file being written");
    let held = oxtapd::files::lock_run(&l.run_dir("700")).expect("lock").expect("free");
    let (tx, _r) = mpsc::unbounded_channel();
    Registry::defaults().recover(&RecoverCtx { layout: l.clone(), runs: l.runs(), reports: tx.clone(), reason: "crash" });
    assert_eq!(staged(&l), before, "정:IMPL-OPS-REC-UNIT 기동 회수 — 아직 쓰는 유닛의 실행 폴더는 건드리지 않는다");
    drop(held);
    Registry::defaults().recover(&RecoverCtx { layout: l.clone(), runs: l.runs(), reports: tx, reason: "crash" });
    assert!(!l.run_dir("700").exists(), "and the run folder is removed");
}

#[test]
fn one_failing_consumer_is_closed_alone_reopened_after_backoff_and_stopped_past_the_intensity() {
    use oxtapd::job::{Consumers, Slot};
    struct Flaky(bool);
    impl RecConsumer for Flaky {
        fn record(&mut self, _r: &Record) -> Result<(), String> {
            if self.0 {
                panic!("qa boom");
            }
            Ok(())
        }
        fn finish(&mut self, _r: &str) -> Result<(), String> {
            Ok(())
        }
    }
    let mut reg = Registry::defaults();
    reg.register::<Flaky>("bad", |_| Ok(Flaky(true)));
    reg.register::<Flaky>("good", |_| Ok(Flaky(false)));
    let l = Layout::new(root("isolate"), "n1");
    let (c, _rx) = ctx(&l, "800");
    let open = |n: &str| Slot::new(n, reg.open(n, &c).expect("registered").expect("open"));
    let mut cs = Consumers::new("r", oxtapd::Restart::parse(Some("100"), Some("2"), Some("60")).expect("restart"), vec![open("bad"), open("good")]);
    let r = record(1, 0, Channel::Tick, Vec::new());
    let t0 = Instant::now();
    cs.each(|x| x.record(&r));
    assert_eq!((cs.live(), cs.waiting()), (vec!["good".to_string()], vec!["bad".to_string()]), "정:IMPL-OPS-REC-UNIT 결과물 소비자 — 소비자 하나의 패닉은 그 소비자만 닫는다");
    cs.revive(&reg, &c, t0 + Duration::from_millis(10));
    assert_eq!(cs.waiting(), vec!["bad".to_string()], "not before the backoff");
    cs.revive(&reg, &c, t0 + Duration::from_millis(200));
    assert_eq!(cs.live(), vec!["bad".to_string(), "good".to_string()], "그 종류는 backoff 뒤 다시 짓는다");
    cs.each(|x| x.record(&r));
    cs.revive(&reg, &c, Instant::now() + Duration::from_millis(200));
    cs.each(|x| x.record(&r));
    assert_eq!((cs.live(), cs.waiting()), (vec!["good".to_string()], Vec::<String>::new()), "정:IMPL-OPS-REC-UNIT 되살림 반복 — intensity 를 넘으면 그 종류만 멈춘다");
}

fn feed(c: &mut dyn RecConsumer, recs: &[Record], finish: Option<&str>) {
    let mut pipe = Pipeline::new(Policy::Utterance);
    let go = |c: &mut dyn RecConsumer, outs: Vec<Output>| {
        for o in outs {
            match &o {
                Output::Frame(f) => c.frame(f).expect("frame"),
                Output::Segment(s) => c.segment(s).expect("segment"),
                Output::Orphan(_) => {}
            }
        }
    };
    for r in recs {
        c.record(r).expect("record");
        go(c, pipe.push(r));
    }
    if let Some(why) = finish {
        go(c, pipe.finish());
        c.finish(why).expect("finish");
    }
}

thread_local! {
    static CLOSED: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

#[derive(Default)]
struct Fields(Vec<(String, String)>);

impl Visit for Fields {
    fn record_str(&mut self, f: &Field, v: &str) {
        self.0.push((f.name().to_string(), v.to_string()));
    }
    fn record_debug(&mut self, f: &Field, v: &dyn std::fmt::Debug) {
        self.0.push((f.name().to_string(), format!("{v:?}").trim_matches('"').to_string()));
    }
}

struct ClosedLines;

impl<S: tracing::Subscriber> Layer<S> for ClosedLines {
    fn on_event(&self, e: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut f = Fields::default();
        e.record(&mut f);
        let get = |k: &str| f.0.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
        if get("message") == "rec closed" {
            CLOSED.with(|c| c.borrow_mut().push((get("path"), get("reason"))));
        }
    }
}

fn closed_lines() -> tracing::subscriber::DefaultGuard {
    CLOSED.with(|c| c.borrow_mut().clear());
    tracing::subscriber::set_default(tracing_subscriber::registry().with(ClosedLines))
}

fn closed_reason(path: &str) -> String {
    CLOSED.with(|c| c.borrow().iter().filter(|(p, _)| p == path).map(|(_, r)| r.clone()).collect::<Vec<_>>().join("+"))
}

fn reports(rx: &mut mpsc::UnboundedReceiver<Report>) -> Vec<(String, String, String, String)> {
    let mut v = Vec::new();
    while let Ok(r) = rx.try_recv() {
        match r {
            Report::Rec { talk_id, path, mime, .. } => {
                let why = closed_reason(&path);
                v.push((talk_id, path, mime, why));
            }
            other => panic!("{other:?}"),
        }
    }
    v.sort();
    v
}

#[test]
fn a_closed_name_is_never_overwritten_even_by_two_closers_at_once() {
    let d = root("link");
    std::fs::create_dir_all(&d).expect("dir");
    let dst = d.join("rec").join("r");
    let mut made = Vec::new();
    for i in 0..3 {
        let p = d.join(format!("s{i}.ogg"));
        std::fs::write(&p, format!("body{i}")).expect("write");
        made.push(oxtapd::files::link_no_overwrite(&p, &dst, "r-t-u.ogg").expect("link"));
    }
    assert_eq!(names(&dst), vec!["r-t-u-0001.ogg", "r-t-u-0002.ogg", "r-t-u.ogg"], "정:IMPL-OPS-REC-UNIT 덮어쓰기 금지 — 같은 이름은 다음 차례 -NNNN 으로 비킨다");
    assert_eq!(std::fs::read_to_string(dst.join("r-t-u.ogg")).expect("read"), "body0", "the first stays as it was");
    assert!((0..3).all(|i| !d.join(format!("s{i}.ogg")).exists()), "the staged file is gone once the name is taken");
    let race = d.join("race");
    let srcs: Vec<PathBuf> = (0..8)
        .map(|i| {
            let p = d.join(format!("x{i}.ogg"));
            std::fs::write(&p, format!("{i}")).expect("write");
            p
        })
        .collect();
    let handles: Vec<_> = srcs
        .into_iter()
        .map(|p| {
            let race = race.clone();
            std::thread::spawn(move || oxtapd::files::link_no_overwrite(&p, &race, "same.ogg").expect("link"))
        })
        .collect();
    let got: HashSet<PathBuf> = handles.into_iter().map(|h| h.join().expect("thread")).collect();
    assert_eq!(got.len(), 8, "S-sym-IMPL-OPS-REC-UNIT-8 — closers racing for one name each win a different name");
    let bodies: HashSet<String> = names(&race).iter().map(|n| std::fs::read_to_string(race.join(n)).expect("read")).collect();
    assert_eq!(bodies.len(), 8, "no closer overwrote another");
}

#[test]
fn names_are_derived_from_the_room_the_talk_and_the_run() {
    use oxtapd::files::{numbered_name, room_name_ok, talk_of, tracks_closed_name};
    assert_eq!(numbered_name("a-b.ogg", 0), "a-b.ogg");
    assert_eq!(numbered_name("a-b.ogg", 3), "a-b-0003.ogg");
    assert_eq!(tracks_closed_name("r-u1-0002.mkv", "1795"), "r-u1-1795-0002.mkv", "정:IMPL-OPS-REC-UNIT 결과물 tracks — 묶음 키 · 실행 표지 · 차례");
    assert_eq!(talk_of("r", &format!("r-{TALK}-bot-a.ogg")).as_deref(), Some(TALK), "the talk id is read back from a staged talk file name");
    assert_eq!(talk_of("r-x", &format!("r-x-{TALK}-u-0001.mkv")).as_deref(), Some(TALK), "a room id with a dash");
    assert_eq!(talk_of("r", "q-t-u.ogg"), None);
    assert!(room_name_ok("ch-1", "1795836407989"));
    for bad in [".", "..", "a/b", ""] {
        assert!(!room_name_ok(bad, "1"), "정:IMPL-OPS-REC-UNIT 결과물 — 이름에 못 쓰는 room_id {bad:?}");
    }
    assert!(!room_name_ok(&"r".repeat(250), "1795836407989"), "a name past the file system length limit");
}

#[test]
fn talk_files_close_under_rec_one_per_talk_with_the_reason() {
    let _log = closed_lines();
    let l = Layout::new(root("talk"), "n1");
    let (c, mut rx) = ctx(&l, "100");
    let mut t = TalkFiles::open(&c).expect("talk");
    feed(&mut t, &talk_stream(), Some("end"));
    assert_eq!(names(&l.closed("rec", "r")), vec!["r-tA-A.mkv".to_string(), "r-tB-B.ogg".to_string()], "정:IMPL-OPS-REC-UNIT 파일 — 발언마다 하나 · 영상 트랙이 든 발언은 .mkv(음성도 그 안) · 아니면 .ogg");
    let got = reports(&mut rx);
    assert!(got.iter().any(|x| x.0 == "tB" && x.3 == "end") && got.iter().all(|x| !x.3.is_empty()), "정:IMPL-OPS-REC-UNIT 알림 — 닫힘 사유는 닫을 때 진단 줄 하나(rec closed)에 · 닫을 때 준 사유: {got:?}");
    let got: Vec<(&str, &str, &str)> = got.iter().map(|(t, p, m, _)| (t.as_str(), p.as_str(), m.as_str())).collect();
    assert_eq!(got, vec![("tA", "rec/r/r-tA-A.mkv", "video/x-matroska"), ("tB", "rec/r/r-tB-B.ogg", "audio/ogg")], "정:IMPL-CHAT-TALK 2 — rec 알림도 발언마다 하나");
    assert!(oxtapd::files::files_under(&l.run_dir("100")).is_empty(), "nothing half-written stays in the run folder");
    assert!(oxtapd::files::files_under(&l.closed("tracks", "r")).is_empty(), "talk files do not write personal tracks");
}

#[test]
fn track_files_close_under_tracks_with_the_run_in_the_name() {
    let l = Layout::new(root("tracks"), "n1");
    let (c, mut rx) = ctx(&l, "200");
    let mut t = TrackFiles::open(&c).expect("tracks");
    feed(&mut t, &track_stream(), Some("end"));
    let got = names(&l.closed("tracks", "r"));
    assert_eq!(got, vec!["r-u1-200-0000.mkv".to_string()], "정:IMPL-OPS-REC-UNIT 결과물 tracks — tracks/{{room}}/ 아래 · 묶음 키 · 실행 표지 · 차례");
    assert!(reports(&mut rx).is_empty(), "only talk files are reported");
    assert!(oxtapd::files::files_under(&l.closed("rec", "r")).is_empty());
}

fn ogg_cut(p: &Path, pages: usize) {
    let b = std::fs::read(p).expect("read");
    let at: Vec<usize> = b.windows(4).enumerate().filter(|(_, w)| *w == b"OggS").map(|(i, _)| i).collect();
    std::fs::write(p, &b[..at[pages]]).expect("cut");
}

#[test]
fn startup_recovery_closes_each_fragment_and_leaves_other_nodes_alone() {
    let _log = closed_lines();
    let l = Layout::new(root("recover"), "n1");
    let (c, _rx) = ctx(&l, "300");
    let mut t = TalkFiles::open(&c).expect("talk");
    feed(&mut t, &talk_stream(), None);
    drop(t);
    let run = l.run_dir("300");
    let staged = oxtapd::files::files_under(&run);
    assert!(!staged.is_empty(), "a crash leaves files being written in the run folder");
    let ogg = staged.iter().find(|p| p.extension().is_some_and(|e| e == "ogg")).expect("ogg").clone();
    let bytes = std::fs::read(&ogg).expect("read");
    let tail = bytes.len();
    std::fs::write(&ogg, [&bytes[..], &bytes[tail - 7..]].concat()).expect("torn tail");
    let empty = run.join("rec").join("r").join(format!("r-{TALK}-z.ogg"));
    std::fs::copy(&ogg, &empty).expect("copy");
    ogg_cut(&empty, 2);
    let broken = run.join("rec").join("r").join("r-tX-y.ogg");
    std::fs::write(&broken, b"not an ogg").expect("write");
    let stray = run.join("other").join("thing.bin");
    std::fs::create_dir_all(stray.parent().expect("p")).expect("dir");
    std::fs::write(&stray, b"x").expect("write");
    let other = Layout::new(&l.root, "n2");
    let theirs = other.run_dir("9").join("rec").join("r").join("r-tO-o.ogg");
    std::fs::create_dir_all(theirs.parent().expect("p")).expect("dir");
    std::fs::write(&theirs, b"theirs").expect("write");
    let (tx, mut rx) = mpsc::unbounded_channel();
    Registry::defaults().recover(&RecoverCtx { layout: l.clone(), runs: l.runs(), reports: tx, reason: "crash" });
    let closed = names(&l.closed("rec", "r"));
    assert!(closed.contains(&"r-tA-A.mkv".to_string()) && closed.contains(&"r-tB-B.ogg".to_string()), "정:IMPL-OPS-REC-UNIT 기동 회수 — 마감해 닫힌 경로로: {closed:?}");
    let o = std::fs::read(l.closed("rec", "r").join("r-tB-B.ogg")).expect("ogg");
    let last = o.windows(4).rposition(|w| w == b"OggS").expect("page");
    assert_eq!(o[last + 5] & 0x04, 0x04, "정:IMPL-OPS-REC-UNIT 기동 회수 — Ogg 는 끝 표지를 덧붙인다");
    let got = reports(&mut rx);
    assert!(!got.is_empty() && got.iter().all(|(_, _, _, why)| why == "crash"), "정:IMPL-OPS-REC-UNIT 기동 회수 — 닫힘 진단 줄의 사유 crash: {got:?}");
    assert!(!empty.exists() && !closed.iter().any(|n| n.ends_with("-z.ogg")), "정:IMPL-OPS-REC-UNIT 기동 회수 — 미디어 프레임이 없는 조각은 지운다");
    assert!(oxtapd::files::files_under(&l.failed()).iter().any(|p| p.ends_with("r-tX-y.ogg")), "정:IMPL-OPS-REC-UNIT 기동 회수 — 마감에 실패한 조각은 .rec-part/failed/");
    assert!(stray.exists(), "a fragment no kind claims is left in place");
    assert_eq!(std::fs::read(&theirs).expect("theirs"), b"theirs", "정:IMPL-OPS-REC-UNIT 기동 회수 — 다른 node 의 것은 건드리지 않는다");
    assert!(!run.join("rec").exists(), "emptied run folders are removed");
}

#[test]
fn the_sweep_removes_only_files_past_the_keep_days_in_the_four_kept_places() {
    let l = Layout::new(root("sweep"), "n1");
    let now = SystemTime::now();
    let mut planted = Vec::new();
    for dir in [l.closed("rec", "ch-1"), l.closed("tracks", "ch-1"), l.closed("raw", "ch-1"), l.failed().join("n1").join("1"), l.run_dir("5")] {
        std::fs::create_dir_all(&dir).expect("dir");
        for (name, age_days) in [("old.bin", 31u64), ("new.bin", 0)] {
            let p = dir.join(name);
            std::fs::write(&p, b"x").expect("write");
            let f = std::fs::File::options().write(true).open(&p).expect("open");
            f.set_modified(now - Duration::from_secs(age_days * 86_400)).expect("mtime");
            planted.push(p);
        }
    }
    assert_eq!(oxtapd::sweep(&l.root, 30, now).expect("sweep"), 4, "정:IMPL-OPS-REC-UNIT 보관 — rec/ · tracks/ · raw/ · .rec-part/failed/");
    assert!(l.run_dir("5").join("old.bin").exists(), "the other staging folders are not swept");
    assert!(planted.iter().filter(|p| p.ends_with("new.bin")).all(|p| p.exists()));
}

#[test]
fn the_registry_starts_with_three_kinds_and_only_replaces_or_adds() {
    struct Probe;
    impl RecConsumer for Probe {
        fn finish(&mut self, _r: &str) -> Result<(), String> {
            Ok(())
        }
    }
    let mut r = Registry::defaults();
    assert_eq!(r.names(), vec!["raw", "talk", "tracks"], "정:IMPL-OPS-REC-UNIT 결과물 소비자 — 기본 셋을 든 채 시작");
    r.register::<Probe>("qa", |_| Ok(Probe));
    r.register::<Probe>("talk", |_| Ok(Probe));
    r.register::<Probe>("", |_| Ok(Probe));
    assert_eq!(r.names(), vec!["qa", "raw", "talk", "tracks"], "같은 이름은 바꾸고 새 이름은 더한다 · 빈 이름은 등록하지 않는다");
    let l = Layout::new(root("reg"), "n1");
    let (c, _rx) = ctx(&l, "1");
    assert!(r.open("nope", &c).is_none(), "an unregistered kind is not opened");
    assert!(r.open("talk", &c).is_some_and(|x| x.is_ok()));
    assert_eq!(oxtapd::job::kinds_of(&serde_json::json!({ "record": ["talk", "raw", "talk", ""] })), vec!["talk", "raw"], "연:ROOM.http.CREATE_ROOM record — 차례 그대로 · 중복 없이");
    assert!(oxtapd::job::kinds_of(&serde_json::json!({ "record": true })).is_empty(), "the old bool form is no list");
    assert!(oxtapd::job::kinds_of(&serde_json::json!({})).is_empty());
}

#[test]
fn a_room_job_is_reopened_after_backoff_and_quarantined_past_the_intensity() {
    let r = oxtapd::Restart::parse(Some("100,200"), Some("3"), Some("60")).expect("restart");
    let mut hist = VecDeque::new();
    let t0 = Instant::now();
    let got: Vec<Option<u64>> = (0..4).map(|i| oxtapd::job::restart_after(&mut hist, t0 + Duration::from_secs(i), &r, i as usize).map(|d| d.as_millis() as u64)).collect();
    assert_eq!(got, vec![Some(100), Some(200), Some(200), None], "정:IMPL-OPS-REC-UNIT 되살림 반복 ② — backoff 차례 · 창 안 상한을 넘으면 격리");
    let mut hist = VecDeque::new();
    let spread: Vec<bool> = (0..6).map(|i| oxtapd::job::restart_after(&mut hist, t0 + Duration::from_secs(i * 30), &r, 0).is_some()).collect();
    assert!(spread.iter().all(|x| *x), "failures spread past the window never quarantine");
    assert_eq!(oxtapd::job::rung(0), Duration::ZERO, "연:RULE-SES-BACKOFF-LADDER 첫 칸");
    assert!(oxtapd::job::rung(1) > Duration::ZERO && oxtapd::job::rung(99) == oxtapd::job::rung(9), "the ladder holds its last rung");
}

#[test]
fn the_unit_answers_its_record_kinds_and_takes_the_supervisor_restart_values() {
    let mut env = common::envelope::Envelope::new(common::envelope::CallKind::Admin);
    env.method = Some("GET".into());
    env.path = Some(format!("/{}", common::system::REC_KINDS_PATH));
    let kinds = Registry::defaults().names();
    match oxtapd::admin_answer(&env, 0, None, &kinds) {
        Reply::Ok { body } => assert_eq!(body["kinds"], serde_json::json!(["raw", "talk", "tracks"]), "정:IMPL-OPS-REC-UNIT 결과물 소비자 — Call GET /unit/record_kinds → {{kinds}} = 등록된 이름 전부"),
        other => panic!("kinds answer {other:?}"),
    }
    let r = oxtapd::Restart::parse(Some("200,500,1000"), Some("5"), Some("60")).expect("parse");
    assert_eq!((r.backoff_ms, r.intensity_max, r.intensity_window.as_secs()), (vec![200, 500, 1000], 5, 60), "정:IMPL-CFG-SYSTEM-FILE rec 유닛 인자 --backoff-ms · --intensity-max · --intensity-window-secs");
    let d = oxtapd::Restart::parse(None, None, None).expect("defaults");
    assert_eq!(d.backoff_ms, common::system::Supervisor::default().backoff_ms, "인자가 없으면 [supervisor] 기본값");
    for bad in [(Some("x"), None, None), (Some(""), None, None), (None, Some("-1"), None), (None, None, Some("w"))] {
        assert!(oxtapd::Restart::parse(bad.0, bad.1, bad.2).is_err(), "틀린 값은 거절 {bad:?}");
    }
    let base: Vec<String> = ["--id", "n1", "--bus-namespace", "t", "--bus-connect", "tcp/1", "--data-root", "/d", "--token", "x", "--hub-base", "http://h:1/m", "--grpc-listen", "127.0.0.1:1"].iter().map(|s| s.to_string()).collect();
    assert!(oxtapd::unit::parse(base.clone()).is_ok());
    let mut bad = base;
    bad.extend(["--nope".to_string(), "1".to_string()]);
    assert!(oxtapd::unit::parse(bad).is_err(), "운영:RULE-BOOT-ARGS — 모르는 인자는 거절");
}

#[test]
fn a_hub_base_is_parsed_and_a_chunked_body_read_whole() {
    let h = oxtapd::hub_of("http://127.0.0.1:1974/media", None).expect("hub");
    assert_eq!((h.host.as_str(), h.port, h.path.as_str(), h.tls.is_none()), ("127.0.0.1", 1974, "/media", true), "정:IMPL-OPS-REC-UNIT 로컬 게이트웨이 — --hub-base");
    assert!(oxtapd::hub_of("https://127.0.0.1:1974/media", None).is_none(), "TLS needs --hub-ca (no system trust store)");
    let (st, body) = oxtapd::parse_reply(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nb\r\n{\"record\":[\r\n8\r\n\"talk\"]}\r\n0\r\n\r\n").expect("parse");
    assert!(st == 200 && oxtapd::job::kinds_of(&body) == vec!["talk"], "a chunked body is read whole");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_room_session_token_is_asked_with_the_node_token() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = l.local_addr().expect("addr").port();
    let (seen_tx, mut seen_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let Ok((mut s, _)) = l.accept().await else { return };
        let mut buf = vec![0u8; 4096];
        let n = s.read(&mut buf).await.unwrap_or(0);
        let _ = seen_tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
        let body = br#"{"token":"room-tok"}"#;
        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await;
        let _ = s.write_all(body).await;
    });
    let hub = oxtapd::hub_of(&format!("http://127.0.0.1:{port}/media"), None).expect("hub");
    let (st, body) = oxtapd::request_json(&hub, "POST", "/rec_unit/session_token", "node-tok", Some(&serde_json::json!({ "room_id": "ch-1" }))).await.expect("answer");
    assert_eq!((st, body["token"].as_str()), (200, Some("room-tok")));
    let req = seen_rx.recv().await.expect("request");
    assert!(req.starts_with("POST /media/rec_unit/session_token HTTP/1.1") && req.contains("Authorization: Bearer node-tok") && req.ends_with(r#"{"room_id":"ch-1"}"#), "정:IMPL-SES-TOKEN-ISSUE 녹음 유닛 방 세션 토큰 — node 토큰 Bearer · {{room_id}}: {req}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn over_tls_the_room_detail_is_read_with_the_hub_certificate_only() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::rustls::ServerConfig;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let mk = || {
        let c = rcgen::generate_simple_self_signed(vec!["hub.example".to_string()]).expect("cert");
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(c.key_pair.serialize_der()));
        (c.cert.pem().into_bytes(), CertificateDer::from(c.cert.der().to_vec()), key)
    };
    let (pem, der, key) = mk();
    let (other, _, _) = mk();
    let provider = std::sync::Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let cfg = ServerConfig::builder_with_provider(provider).with_safe_default_protocol_versions().expect("tls").with_no_client_auth().with_single_cert(vec![der], key).expect("cert");
    let acc = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(cfg));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = l.local_addr().expect("addr").port();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = l.accept().await else { return };
            let acc = acc.clone();
            tokio::spawn(async move {
                let Ok(mut s) = acc.accept(tcp).await else { return };
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await;
                let body = br#"{"room_id":"ch-1","record":["talk"]}"#;
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await;
                let _ = s.write_all(body).await;
                let _ = s.shutdown().await;
            });
        }
    });
    let base = format!("https://127.0.0.1:{port}/media");
    let hub = oxtapd::hub_of(&base, Some(&pem)).expect("hub");
    let (st, body) = oxtapd::get_json(&hub, "/rooms/ch-1", "tok").await.expect("read over TLS on loopback — no name check");
    assert!(st == 200 && oxtapd::job::kinds_of(&body) == vec!["talk"], "정:IMPL-OPS-REC-UNIT 로컬 게이트웨이 — --hub-ca 의 인증서 하나로 검증");
    let wrong = oxtapd::hub_of(&base, Some(&other)).expect("hub");
    assert!(oxtapd::get_json(&wrong, "/rooms/ch-1", "tok").await.is_none(), "another certificate is refused");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_unit_answers_hello_and_drops_over_its_handle() {
    use common::envelope::handle::handle_client::HandleClient;
    use common::envelope::handle::handle_server::HandleServer;
    use common::envelope::handle::{CallReq, HelloReq};
    use common::envelope::{CallKind, Envelope};
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("port");
    let addr = probe.local_addr().expect("addr");
    drop(probe);
    let drops = std::sync::Arc::new(AtomicU64::new(0));
    let (tok_tx, mut tok_rx) = tokio::sync::watch::channel("t0".to_string());
    let svc = oxtapd::Svc { epoch: "e-rec".into(), build: "b1".into(), drops: drops.clone(), token: tok_tx, kinds: Registry::defaults().names() };
    tokio::spawn(tonic::transport::Server::builder().add_service(HandleServer::new(svc)).serve(addr));
    let mut c = None;
    for _ in 0..50 {
        if let Ok(x) = HandleClient::connect(format!("http://{addr}")).await {
            c = Some(x);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut c = c.expect("the handle listens");
    let h = c.hello(HelloReq { node: "n1".into(), hub_inst: "i1".into() }).await.expect("hello").into_inner();
    assert_eq!((h.epoch.as_str(), h.build.as_str()), ("e-rec", "b1"), "정:IMPL-OPS-REC-UNIT 운영 표면 — Hello gives the boot identity (the ready signal)");
    drops.store(3, Ordering::Relaxed);
    let mut env = Envelope::new(CallKind::Admin);
    env.method = Some("GET".into());
    env.path = Some("/admin/drops".into());
    let r = Reply::decode(&c.call(CallReq { envelope: env.encode() }).await.expect("call").into_inner().reply).expect("reply");
    assert!(matches!(&r, Reply::Ok { body } if body["talk_drop"] == 3), "정:RULE-OPS-DROPS — talk_drop is read through Call /admin/drops {r:?}");
    env.path = Some("/admin/rooms".into());
    let r = Reply::decode(&c.call(CallReq { envelope: env.encode() }).await.expect("call").into_inner().reply).expect("reply");
    assert!(matches!(r, Reply::Fail { .. }), "정:IMPL-OPS-REC-UNIT — other Call paths fail");
    let mut give = Envelope::new(CallKind::Admin);
    give.method = Some("POST".into());
    give.path = Some("/unit/token".into());
    give.body = serde_json::json!({ "token": "t1" });
    let r = Reply::decode(&c.call(CallReq { envelope: give.encode() }).await.expect("call").into_inner().reply).expect("reply");
    assert!(matches!(r, Reply::Ok { .. }) && tok_rx.has_changed().unwrap_or(false), "S-sym-IMPL-OPS-REC-UNIT-4① the supervisor's token handover is the one more Call path the unit takes: {r:?}");
    assert_eq!(tok_rx.borrow_and_update().as_str(), "t1", "S-sym-IMPL-OPS-REC-UNIT-4① the handed token replaces the one the unit reads rooms and asks room session tokens with");
    give.body = serde_json::json!({ "token": "" });
    let r = Reply::decode(&c.call(CallReq { envelope: give.encode() }).await.expect("call").into_inner().reply).expect("reply");
    assert!(matches!(r, Reply::Fail { .. }) && tok_rx.borrow().as_str() == "t1", "S-sym-IMPL-OPS-REC-UNIT-4① an empty token is not a handover — it fails and the held token stays");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_recording_is_reported_to_the_gateway_over_the_bus() {
    let mut rc = Config::router("n1");
    rc.listen = vec!["127.0.0.1:0".into()];
    let hub = Mesh::open(rc).await.expect("router");
    let mut q = hub.queryable("ox/t/int/q/node/n1/**").await.expect("qabl");
    let (got_tx, mut got_rx) = mpsc::unbounded_channel::<(String, Report)>();
    tokio::spawn(async move {
        while let Some(query) = q.recv().await {
            let r: Report = serde_json::from_slice(query.payload()).expect("a talk report");
            let _ = got_tx.send((query.key_expr().to_string(), r));
            query.reply("ox/t/int/q/node/n1/talk", Reply::ok(serde_json::json!({ "t": 1 })).encode());
        }
    });
    let at = hub.bound()[0].to_string();
    let unit = oxtapd::bus::Bus::open("n1", "t", &at).await.expect("client");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let drops = AtomicU64::new(0);
    let (tx, rx) = mpsc::unbounded_channel();
    let rec = Report::Rec { talk_id: TALK.into(), path: format!("rec/ch-1/ch-1-{TALK}-bot-a.ogg"), size: 9, mime: "audio/ogg".into(), duration_ms: 1_000 };
    tx.send(rec.clone()).expect("queue");
    drop(tx);
    let u2 = unit.clone();
    oxtapd::report_line(
        rx,
        Duration::from_secs(10),
        |p| {
            let b = u2.clone();
            async move { b.ask(common::talk::WHAT, p).await }
        },
        &drops,
    )
    .await;
    let (k, r) = tokio::time::timeout(Duration::from_secs(5), got_rx.recv()).await.expect("in time").expect("arrived");
    assert_eq!((k.as_str(), r), ("ox/t/int/q/node/n1/talk", rec), "정:IMPL-OPS-REC-UNIT 알림 — rec{{talk_id, path, size, mime, duration_ms}} goes to q/node/{{node}}/talk");
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    let dead = oxtapd::bus::Bus::open("n9", "t", &at).await.expect("client");
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(Report::Rec { talk_id: TALK.into(), path: "rec/x/y".into(), size: 1, mime: "audio/ogg".into(), duration_ms: 1 }).expect("queue");
    drop(tx);
    oxtapd::report_line(
        rx,
        Duration::from_millis(400),
        |p| {
            let b = dead.clone();
            async move { b.ask(common::talk::WHAT, p).await }
        },
        &drops,
    )
    .await;
    assert_eq!(drops.load(Ordering::Relaxed), 1, "정:RULE-OPS-DROPS talk_drop — no answer past the retry limit is dropped and counted");
}
