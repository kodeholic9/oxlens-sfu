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

fn pkt(seq: u16, ts: u32, csrc: u32, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0x81, 0xEF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    v[2..4].copy_from_slice(&seq.to_be_bytes());
    v[4..8].copy_from_slice(&ts.to_be_bytes());
    v.extend_from_slice(&csrc.to_be_bytes());
    v.extend_from_slice(payload);
    v
}

fn record(seq: u32, t_ms: u64, channel: oxpeer::record::Channel, data: Vec<u8>) -> oxpeer::record::Record {
    oxpeer::record::Record { sequence: seq, log_time: t_ms * 1_000_000, channel, data }
}

#[test]
fn one_file_per_talk_mkv_with_video_else_ogg() {
    use oxpeer::record::{Channel, Pipeline, Policy};
    let l = Layout::new(root("one-per-talk"));
    l.clear_staging().expect("staging");
    let mut talks = oxrecd::Talks::new(Layout::new(&l.root)).expect("talks");
    let tracks = serde_json::json!({
        "room": "r",
        "tracks": [{ "track_id": "slot-a", "kind": "audio" }, { "track_id": "slot-v", "kind": "video" }],
        "rows": { "100": { "track_id": "slot-a", "kind": "audio", "codec": "opus", "pt": 111 }, "200": { "track_id": "slot-v", "kind": "video", "codec": "VP8", "pt": 96 } }
    });
    let mut recs = vec![record(1, 0, Channel::Track, tracks.to_string().into_bytes())];
    let (mut aseq, mut vseq) = (0u16, 0u16);
    for (src, t0, who, talk, video) in [(7u32, 0u64, "A", "tA", true), (8, 3_000, "B", "tB", false)] {
        let n = recs.len() as u32 + 1;
        recs.push(record(n, t0, Channel::Floor, format!(r#"{{"room":"r","event":"taken","user_id":"{who}","sources":[{src},{}],"seq":{n},"talk":"{talk}"}}"#, src + 10).into_bytes()));
        for i in 0..100u64 {
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
    let mut pipe = Pipeline::new(Policy::Utterance);
    let mut reports = Vec::new();
    for r in &recs {
        for o in pipe.push(r) {
            reports.extend(talks.feed(&o));
        }
    }
    for o in pipe.finish() {
        reports.extend(talks.feed(&o));
    }
    let mut files = files_under(&l.rec_dir("r"));
    files.sort();
    assert_eq!(files, vec!["r-tA-A.mkv".to_string(), "r-tB-B.ogg".to_string()], "정:IMPL-OPS-REC-UNIT 파일 — 발언마다 하나 · 영상 트랙이 든 발언은 .mkv(음성도 그 안) · 아니면 .ogg");
    let mut got: Vec<(String, String)> = reports
        .iter()
        .map(|r| match r {
            Report::Rec { talk_id, mime, .. } => (talk_id.clone(), mime.clone()),
            other => panic!("{other:?}"),
        })
        .collect();
    got.sort();
    assert_eq!(got, vec![("tA".to_string(), "video/x-matroska".to_string()), ("tB".to_string(), "audio/ogg".to_string())], "rec 알림도 발언마다 하나");
    assert!(files_under(&l.staging()).is_empty(), "the file not chosen is removed, nothing half-written stays");
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
    let h = oxrecd::hub_of("http://127.0.0.1:1974/media", None).expect("hub");
    assert_eq!((h.host.as_str(), h.port, h.path.as_str(), h.tls.is_none()), ("127.0.0.1", 1974, "/media", true), "정:IMPL-OPS-REC-UNIT 로컬 게이트웨이 — --hub-base");
    assert!(oxrecd::hub_of("https://127.0.0.1:1974/media", None).is_none(), "TLS needs --hub-ca (no system trust store)");
    let (st, body) = oxrecd::parse_reply(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nb\r\n{\"record\":t\r\n4\r\nrue}\r\n0\r\n\r\n").expect("parse");
    assert!(st == 200 && oxrecd::recorded(&body), "a chunked body is read whole");
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
                let body = br#"{"room_id":"ch-1","record":true}"#;
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await;
                let _ = s.write_all(body).await;
                let _ = s.shutdown().await;
            });
        }
    });
    let base = format!("https://127.0.0.1:{port}/media");
    let hub = oxrecd::hub_of(&base, Some(&pem)).expect("hub");
    let (st, body) = oxrecd::get_json(&hub, "/rooms/ch-1", "tok").await.expect("read over TLS on loopback — no name check");
    assert!(st == 200 && oxrecd::recorded(&body), "정:IMPL-OPS-REC-UNIT 로컬 게이트웨이 — --hub-ca 의 인증서 하나로 검증");
    let wrong = oxrecd::hub_of(&base, Some(&other)).expect("hub");
    assert!(oxrecd::get_json(&wrong, "/rooms/ch-1", "tok").await.is_none(), "another certificate is refused");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_unit_answers_hello_and_drops_over_its_handle() {
    use common::envelope::handle::handle_client::HandleClient;
    use common::envelope::handle::handle_server::HandleServer;
    use common::envelope::handle::{CallReq, HelloReq};
    use common::envelope::{CallKind, Envelope, Reply};
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("port");
    let addr = probe.local_addr().expect("addr");
    drop(probe);
    let drops = std::sync::Arc::new(AtomicU64::new(0));
    let svc = oxrecd::Svc { epoch: "e-rec".into(), build: "b1".into(), drops: drops.clone() };
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
    let unit = oxrecd::bus::Bus::open("n1", "t", &at).await.expect("client");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let drops = AtomicU64::new(0);
    let (tx, rx) = mpsc::unbounded_channel();
    let rec = Report::Rec { talk_id: TALK.into(), path: format!("rec/ch-1/ch-1-{TALK}-bot-a.ogg"), size: 9, mime: "audio/ogg".into(), duration_ms: 1_000 };
    tx.send(rec.clone()).expect("queue");
    drop(tx);
    let u2 = unit.clone();
    oxrecd::report_line(
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
    assert_eq!((k.as_str(), r), ("ox/t/int/q/node/n1/talk", rec), "정:IMPL-OPS-REC-UNIT 알림 — rec goes to q/node/{{node}}/talk");
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    let dead = oxrecd::bus::Bus::open("n9", "t", &at).await.expect("client");
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(Report::Rec { talk_id: TALK.into(), path: "rec/x/y".into(), size: 1, mime: "audio/ogg".into(), duration_ms: 1 }).expect("queue");
    drop(tx);
    oxrecd::report_line(
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
