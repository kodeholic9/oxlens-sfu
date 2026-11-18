// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-OPS-REC-UNIT · 정:IMPL-CHAT-TALK · 정:RULE-OPS-DROPS · 연:RULE-CHAT-REC-FILE · model: claude-opus-5-5
//! 1층 녹음 유닛 — 녹취는 `<root>/.rec-part/` 에서 다 쓰고 `<root>/rec/{space_id}/{space_id}-{발언 id}-{화자}.ogg` 로 이름을 바꿔 닫는다(그 전에는 `rec/` 에 안 보인다) · 닫으면 `rec` 알림이 버스 `q/node/{node}/talk` 로 게이트웨이에 간다 · 답이 없으면 상한까지 다시 하고 버려 센다(`talk_drop`) · 보관 기간을 넘긴 녹취만 지운다 · 방 상세의 `record`.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use common::envelope::Reply;
use common::talk::Report;
use oxmesh::{Config, Mesh};
use oxpeer::record::OggFile;
use oxrecd::{Layout, PART_DIR};
use tokio::sync::mpsc;

const TALK: &str = "81330e4c0190a1b2c3d4e5f60718293a4b5c";

fn root(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("oxrecd-l1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn written(l: &Layout, name: &str, bytes: usize, talk: Option<&str>) -> OggFile {
    std::fs::create_dir_all(l.staging()).expect("staging");
    let path = l.staging().join(name);
    std::fs::write(&path, vec![7u8; bytes]).expect("write");
    OggFile { path, segment: 3, track_id: "slot-a".into(), user_id: Some("bot-a".into()), source: Some(9), packets: 50, samples: 48_000, first_seq: None, last_seq: None, first_ts: None, end_ts: None, filled: 0, reason: Some("idle".into()), talk: talk.map(str::to_string) }
}

fn files_under(d: &Path) -> Vec<String> {
    std::fs::read_dir(d).map(|it| it.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default()
}

#[test]
fn a_recording_shows_under_rec_only_after_it_is_closed() {
    let l = Layout::new(root("settle"));
    let f = written(&l, "staged.ogg", 1234, Some(TALK));
    assert!(files_under(&l.rec_dir("ch-1")).is_empty(), "정:IMPL-OPS-REC-UNIT S-sym-IMPL-OPS-REC-UNIT-3 — a file being written is not under rec/ by its name");
    assert!(f.path.starts_with(l.root.join(PART_DIR)), "it is written in the staging place on the same store");
    let r = l.settle("ch-1", &f).expect("settle").expect("a talk id makes a report");
    let name = format!("ch-1-{TALK}-bot-a.ogg");
    assert_eq!(files_under(&l.rec_dir("ch-1")), vec![name.clone()], "정:IMPL-OPS-REC-UNIT 파일 — rec/{{space}}/{{space}}-{{talk}}-{{speaker}}.ogg");
    assert!(!f.path.exists(), "renamed, not copied");
    assert_eq!(r, Report::Rec { talk_id: TALK.into(), path: format!("rec/ch-1/{name}"), size: 1234, mime: "audio/ogg".into(), duration_ms: 1_000 }, "정:IMPL-CHAT-TALK 2 — rec{{talk_id, path, size, mime, duration_ms}}");
    let g = written(&l, "staged2.ogg", 10, Some(TALK));
    let r2 = l.settle("ch-1", &g).expect("settle").expect("report");
    assert!(matches!(&r2, Report::Rec { path, .. } if path.ends_with("-bot-a-0003.ogg")), "a second file of the same talk does not overwrite the first {r2:?}");
    let h = written(&l, "old-server.ogg", 10, None);
    assert_eq!(l.settle("ch-1", &h).expect("settle"), None, "no talk id (older server) — nothing to link");
    assert!(!h.path.exists(), "and the file is not left in staging");
}

#[test]
fn a_restart_clears_half_written_files() {
    let l = Layout::new(root("clear"));
    written(&l, "half.ogg", 10, Some(TALK));
    assert_eq!(l.clear_staging().expect("clear"), 1, "정:IMPL-OPS-REC-UNIT — a half-written file from a crashed run never reaches rec/");
    assert!(files_under(&l.staging()).is_empty());
}

#[test]
fn the_sweep_removes_only_files_past_the_keep_days() {
    let l = Layout::new(root("sweep"));
    let dir = l.rec_dir("ch-1");
    std::fs::create_dir_all(&dir).expect("dir");
    let now = SystemTime::now();
    for (name, age_days) in [("old.ogg", 31u64), ("edge.ogg", 29), ("new.ogg", 0)] {
        let p = dir.join(name);
        std::fs::write(&p, b"x").expect("write");
        let f = std::fs::File::options().write(true).open(&p).expect("open");
        f.set_modified(now - Duration::from_secs(age_days * 86_400)).expect("mtime");
    }
    assert_eq!(oxrecd::sweep(&l.root, 30, now).expect("sweep"), 1, "정:IMPL-OPS-REC-UNIT 보관 — [data] rec_keep_days(30)");
    let mut left = files_under(&dir);
    left.sort();
    assert_eq!(left, vec!["edge.ogg".to_string(), "new.ogg".to_string()]);
    assert_eq!(oxrecd::sweep(&l.root, 1, now).expect("sweep"), 1, "a shorter keep removes more");
}

#[test]
fn the_room_detail_record_flag_decides() {
    assert!(oxrecd::recorded(&serde_json::json!({ "room_id": "r", "record": true })), "연:ROOM.http.CREATE_ROOM record");
    assert!(!oxrecd::recorded(&serde_json::json!({ "room_id": "r", "record": false })));
    assert!(!oxrecd::recorded(&serde_json::json!({ "room_id": "r" })), "an older gateway without the field = not recorded");
    let h = oxrecd::hub_of("http://127.0.0.1:1974/media").expect("hub");
    assert_eq!((h.host.as_str(), h.port, h.path.as_str()), ("127.0.0.1", 1974, "/media"));
    let (st, body) = oxrecd::parse_reply(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nb\r\n{\"record\":t\r\n4\r\nrue}\r\n0\r\n\r\n").expect("parse");
    assert!(st == 200 && oxrecd::recorded(&body), "a chunked body is read whole");
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
    let unit = Mesh::open(Config::client("n1.rec", &hub.bound()[0].to_string())).await.expect("client");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let key = common::keyspace::Keys::new("t").q_node("n1", common::talk::WHAT);
    let drops = AtomicU64::new(0);
    let (tx, rx) = mpsc::unbounded_channel();
    let rec = Report::Rec { talk_id: TALK.into(), path: format!("rec/ch-1/ch-1-{TALK}-bot-a.ogg"), size: 9, mime: "audio/ogg".into(), duration_ms: 1_000 };
    tx.send(rec.clone()).expect("queue");
    drop(tx);
    let u2 = unit.clone();
    oxrecd::report_line(rx, Duration::from_secs(10), |p| oxrecd::ask_bus(u2.clone(), key.clone(), p), &drops).await;
    let (k, r) = tokio::time::timeout(Duration::from_secs(5), got_rx.recv()).await.expect("in time").expect("arrived");
    assert_eq!((k.as_str(), r), ("ox/t/int/q/node/n1/talk", rec), "정:IMPL-OPS-REC-UNIT 알림 — rec goes to q/node/{{node}}/talk");
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    let dead = common::keyspace::Keys::new("t").q_node("n9", common::talk::WHAT);
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(Report::Rec { talk_id: TALK.into(), path: "rec/x/y".into(), size: 1, mime: "audio/ogg".into(), duration_ms: 1 }).expect("queue");
    drop(tx);
    oxrecd::report_line(rx, Duration::from_millis(400), |p| oxrecd::ask_bus(unit.clone(), dead.clone(), p), &drops).await;
    assert_eq!(drops.load(Ordering::Relaxed), 1, "정:RULE-OPS-DROPS talk_drop — no answer past the retry limit is dropped and counted");
}
