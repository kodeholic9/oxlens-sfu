// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-BUS-KEYSPACE · 정:RULE-OPS-DROPS · 정:RULE-HOOK-DIRECT · 정:RULE-HOOK-CREDENTIAL · model: claude-opus-5-5
//! 1층 상호운용 — zenoh 입구(zgate)에 실제 zenoh 1.10.1 클라이언트(`zenoh = "=1.10.1"`)로 붙는다: queryable 선언 → 버스 토큰 · 질의 → reply = ACK · reply_err = 거절 · 답 없이 놓음 = 답 없음 · 512 KiB 질의 한 건(Fragment) · 지금 Interest 에 DeclareFinal(liveliness get 이 시한 전에 끝) · 3rd 의 put 버림 · 3rd 의 get 은 결과 없이 끝 · 허용 키 밖 선언 무시 · 다른 계정 키 선언 = 세션 닫힘 · 거절(인증서 없음 · 다른 CA · 모르는 계정 · 연동 허용 없음 · 평문 TCP) · 연동 허용 계정은 구독 칸이 비어도 붙는다 · Close 와 lease 넘김에 토큰이 걷힌다 · 판 · client 아님 거절은 우리 코덱으로 짠 날 클라이언트로.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use oxhubd::bus::{Bus, BusCfg};
use oxhubd::chat::ext::{ChatExt, ChatKnobs};
use oxhubd::hook::zgate::judge::{Accounts, Lane, Standing};
use oxhubd::hook::zgate::session::Answer;
use oxhubd::hook::zgate::send::{GateDeps, GateKnobs};
use oxhubd::hook::zgate::{Gate, GateCfg, link, wire};
use oxhubd::chat::store::Store;
use oxhubd::chat::{Chat, Limits};
use common::extsub::{ExtSubs, Subscribe};
use common::system::Visibility;
use rcgen::{BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair};
use tokio_rustls::rustls::pki_types::ServerName;
use zenoh_protocol::core::{Reliability, Resolution, WhatAmI, WireExpr, ZenohIdProto};
use zenoh_protocol::network::declare::queryable::ext::QueryableInfoType;
use zenoh_protocol::network::declare::{DeclareKeyExpr, DeclareQueryable};
use zenoh_protocol::network::{Declare, DeclareBody, Mapping, NetworkBody, NetworkMessage, declare};
use zenoh_protocol::transport::init::ext::PatchType;
use zenoh_protocol::transport::{Frame, InitSyn, OpenSyn, TransportBody, TransportMessage, frame};

const NS: &str = "t";

struct Pki {
    dir: PathBuf,
}

impl Pki {
    fn new() -> Pki {
        let dir = std::env::temp_dir().join(format!("zgate-{}-{}", std::process::id(), common::random_hex(6)));
        std::fs::create_dir_all(&dir).expect("dir");
        let (ca, ca_key) = Pki::ca("test ca");
        let (other_ca, other_key) = Pki::ca("other ca");
        let write = |name: &str, body: String| std::fs::write(dir.join(name), body).expect("write");
        write("ca.pem", ca.pem());
        let leaf = |cn: &str, server: bool, issuer: &rcgen::Certificate, issuer_key: &KeyPair| {
            let mut p = CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]).expect("params");
            p.distinguished_name.push(DnType::CommonName, cn);
            p.extended_key_usages = vec![if server { ExtendedKeyUsagePurpose::ServerAuth } else { ExtendedKeyUsagePurpose::ClientAuth }];
            let k = KeyPair::generate().expect("key");
            let c = p.signed_by(&k, issuer, issuer_key).expect("sign");
            (c.pem(), k.serialize_pem())
        };
        let (s, sk) = leaf("hub", true, &ca, &ca_key);
        write("hub.pem", s);
        write("hub.key", sk);
        for cn in ["acme", "other", "nohook", "nosub", "stranger"] {
            let (c, k) = leaf(cn, false, &ca, &ca_key);
            write(&format!("{cn}.pem"), c);
            write(&format!("{cn}.key"), k);
        }
        let (c, k) = leaf("acme", false, &other_ca, &other_key);
        write("rogue.pem", c);
        write("rogue.key", k);
        Pki { dir }
    }

    fn ca(name: &str) -> (rcgen::Certificate, KeyPair) {
        let mut p = CertificateParams::new(Vec::<String>::new()).expect("params");
        p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        p.distinguished_name.push(DnType::CommonName, name);
        let k = KeyPair::generate().expect("key");
        let c = p.self_signed(&k).expect("ca");
        (c, k)
    }

    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Pki {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Rig {
    pki: Pki,
    bus: Arc<Bus>,
    gate: Arc<Gate>,
    chat: Arc<Chat>,
    _events: tokio::sync::mpsc::UnboundedReceiver<oxhubd::bus::BusEvent>,
}

async fn rig() -> Rig {
    rig_with(knobs()).await
}

#[derive(Debug, Clone, Copy)]
struct TestKnobs {
    window: usize,
    reply_timeout: Duration,
    close_at: u32,
    max_tries: u32,
    sweep: Duration,
    live_max: usize,
    cap: usize,
    bundle_msgs: usize,
    bundle_bytes: usize,
}

fn knobs() -> TestKnobs {
    TestKnobs {
        window: 4,
        reply_timeout: Duration::from_millis(400),
        close_at: 2,
        max_tries: 3,
        sweep: Duration::from_secs(30),
        live_max: 65_536,
        cap: 524_288 * 4,
        bundle_msgs: 100,
        bundle_bytes: 524_288,
    }
}

async fn rig_with(k: TestKnobs) -> Rig {
    rig_full(k, true).await
}

async fn rig_full(k: TestKnobs, with_chat: bool) -> Rig {
    rig_accounts(k, with_chat, &[]).await
}

async fn rig_accounts(k: TestKnobs, with_chat: bool, webhooked: &[&str]) -> Rig {
    let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_test_writer().try_init();
    let pki = Pki::new();
    let (bus, events) = Bus::open("n1", "e1", 1_000, BusCfg { listen: vec!["127.0.0.1:0".into()], advertise: vec![], connect: vec![], namespace: NS.into() }).await.expect("bus");
    bus.follow_ext().await.expect("follow ext");
    bus.follow_facts().await.expect("follow facts");
    let accounts = Accounts::new(vec![
        ("acme".into(), Standing::Ok),
        ("other".into(), Standing::Ok),
        ("nohook".into(), Standing::NoHook),
        ("nosub".into(), Standing::Ok),
    ])
    .with_webhooks(webhooked);
    let subs = ExtSubs::new(vec![("acme".into(), Subscribe::try_from(vec![String::new()]).expect("subscribe"))]);
    let store = Store::open("memory:", Visibility::Shared).expect("store").subscribed(subs);
    let chat = Arc::new(Chat::new(store, Limits::default()));
    chat.store.create_space("S", None).expect("space");
    chat.store.join("S", "u1", None, Some(1)).expect("join");
    let cfg = GateCfg { listen: "tls/127.0.0.1:0".into(), cert: pki.path("hub.pem"), key: pki.path("hub.key"), ca: pki.path("ca.pem") };
    let ck = ChatKnobs { live_max: k.live_max, cap: k.cap, bundle_msgs: k.bundle_msgs, bundle_bytes: k.bundle_bytes, sweep: k.sweep, max_tries: k.max_tries };
    let ext = Arc::new(ChatExt::new(chat.store.clone(), chat.acks(), vec!["acme".into()], ck));
    let gk = GateKnobs { window: k.window, reply_timeout: k.reply_timeout, close_at: k.close_at, facts_cap: k.bundle_bytes * 4 };
    let deps = GateDeps { accounts, bus: bus.clone(), chat: with_chat.then(|| ext.clone()), knobs: gk };
    let gate = Gate::bind(&cfg, deps).await.expect("gate");
    tokio::spawn(gate.clone().serve());
    if with_chat {
        tokio::spawn(ext.drains(gate.clone(), bus.clone()));
    }
    Rig { pki, bus, gate, chat, _events: events }
}

fn client_cfg(r: &Rig, endpoint: &str, cert: Option<&str>) -> zenoh::Config {
    let tls = match cert {
        Some(c) => format!(
            r#"{{ root_ca_certificate: "{}", connect_certificate: "{}", connect_private_key: "{}", enable_mtls: true }}"#,
            r.pki.path("ca.pem"),
            r.pki.path(&format!("{c}.pem")),
            r.pki.path(&format!("{c}.key"))
        ),
        None => format!(r#"{{ root_ca_certificate: "{}" }}"#, r.pki.path("ca.pem")),
    };
    let s = format!(
        r#"{{ mode: "client", connect: {{ endpoints: ["{endpoint}"] }}, scouting: {{ multicast: {{ enabled: false }}, gossip: {{ enabled: false }} }}, transport: {{ link: {{ tls: {tls} }} }} }}"#
    );
    zenoh::Config::from_json5(&s).expect("client config")
}

fn tls_endpoint(r: &Rig) -> String {
    format!("tls/127.0.0.1:{}", r.gate.addr.port())
}

async fn open(r: &Rig, cert: &str) -> zenoh::Session {
    zenoh::open(client_cfg(r, &tls_endpoint(r), Some(cert))).await.expect("open")
}

async fn refused_open(cfg: zenoh::Config) -> bool {
    match tokio::time::timeout(Duration::from_secs(15), zenoh::open(cfg)).await {
        Ok(Ok(_s)) => false,
        Ok(Err(_)) => true,
        Err(_) => true,
    }
}

async fn counted(r: &Rig, before: u64) -> bool {
    wait(3_000, || r.gate.drops.refused.load(Ordering::Relaxed) > before).await
}

async fn wait<F: FnMut() -> bool>(ms: u64, mut f: F) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f()
}

async fn tokens(r: &Rig) -> Vec<(String, String, String)> {
    r.bus.ext_conns().await.expect("tokens")
}

async fn wait_tokens(r: &Rig, n: usize, ms: u64) -> Vec<(String, String, String)> {
    let end = Instant::now() + Duration::from_millis(ms);
    loop {
        let t = tokens(r).await;
        if t.len() == n || Instant::now() >= end {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

fn sub_key(k: &str) -> String {
    format!("ox/{NS}/pub/v1/chat/sub/{k}")
}

async fn open_cid(r: &Rig) -> String {
    assert!(wait(5_000, || r.gate.conns().iter().any(|c| c.open)).await, "형상 — 연결이 열렸다(queryable 선언)");
    r.gate.conns().into_iter().find(|c| c.open).map(|c| c.cid).expect("cid")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_zenoh_client_receives_queries_and_its_answers_come_back() {
    let r = rig().await;
    let s = open(&r, "acme").await;
    let q = s.declare_queryable(sub_key("acme")).await.expect("queryable");
    let cid = open_cid(&r).await;
    let t = wait_tokens(&r, 1, 3_000).await;
    assert_eq!(t, vec![("acme".to_string(), "n1".to_string(), cid.clone())], "HK-KEY-PUB-V1-CHAT-SUB-KEY① 정:RULE-CHAT-EXT-SUB 3 — 제 계정 키 queryable 이 서면 토큰 int/ext/acme/n1/{{cid}}");
    let now = common::now_ms();
    let at = oxhubd::chat::ext::plan::opened(&cid);
    assert!(at <= now && now - at < 60_000 && cid[12..].starts_with("e1"), "정:RULE-CHAT-EXT-SUB 3 — cid = 열린 시각 12 자리 hex + 기동 신원 + 세션 번호 {cid}");
    let replier = tokio::spawn(async move {
        let mut seen = Vec::new();
        for i in 0..4 {
            let query = q.recv_async().await.expect("query");
            seen.push(query.payload().map(|p| p.to_bytes().to_vec()).unwrap_or_default());
            match i {
                0 | 3 => query.reply(query.key_expr().clone(), b"ok".to_vec()).await.expect("reply"),
                1 => query.reply_err(b"no".to_vec()).await.expect("reply_err"),
                _ => drop(query),
            }
        }
        (q, seen)
    });
    let big: Vec<u8> = (0..512 * 1024).map(|i| (i % 251) as u8).collect();
    let mut answers = Vec::new();
    for body in [b"one".to_vec(), b"two".to_vec(), b"three".to_vec(), big.clone()] {
        let p = r.gate.query(&cid, Lane::Chat, body).expect("query");
        answers.push(tokio::time::timeout(Duration::from_secs(10), p.answer()).await.expect("answer in time"));
    }
    let (_q, seen) = tokio::time::timeout(Duration::from_secs(10), replier).await.expect("replier in time").expect("replier");
    assert_eq!(answers[0], Answer::Ack, "S-rule-RULE-HOOK-CHAT-SUB-7③ — query 키로 reply = ACK");
    assert_eq!(answers[1], Answer::Refused(b"no".to_vec()), "S-rule-RULE-HOOK-CHAT-SUB-7④ — reply_err = 거절(ACK 아님)");
    assert_eq!(answers[2], Answer::NoReply, "정:RULE-CHAT-EXT-SUB 4 — 답 없이 놓으면 답 없음(ACK 아님)");
    assert_eq!(answers[3], Answer::Ack, "형상 — 큰 질의도 ACK");
    assert_eq!(seen[0], b"one".to_vec(), "형상 — 묶음 몸이 그대로 간다");
    assert_eq!(seen[3].len(), big.len(), "S-rule-RULE-HOOK-CHAT-SUB-7② — 512 KiB 질의 한 건이 조각으로 나뉘어 한 건으로 온다");
    assert!(seen[3] == big, "S-rule-RULE-HOOK-CHAT-SUB-7② — 이은 몸이 같다");
    s.close().await.expect("close");
    let t = wait_tokens(&r, 0, 3_000).await;
    assert!(t.is_empty(), "정:RULE-CHAT-EXT-SUB 3 — Close 에 토큰이 걷힌다 {t:?}");
    assert!(wait(3_000, || r.gate.conns().is_empty()).await, "형상 — 연결표에서 빠진다");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reply_on_another_key_is_no_ack() {
    let r = rig().await;
    let s = open(&r, "acme").await;
    let q = s.declare_queryable(sub_key("acme")).await.expect("queryable");
    let cid = open_cid(&r).await;
    let replier = tokio::spawn(async move {
        let query = q.recv_async().await.expect("query");
        let blocked = query.reply(format!("ox/{NS}/elsewhere"), b"ok".to_vec()).await.is_err();
        drop(query);
        (q, blocked)
    });
    let a = tokio::time::timeout(Duration::from_secs(10), r.gate.query(&cid, Lane::Chat, b"x".to_vec()).expect("query").answer()).await.expect("answer");
    let (_q, blocked) = tokio::time::timeout(Duration::from_secs(10), replier).await.expect("replier in time").expect("replier");
    assert!(blocked, "S-sym-RULE-HOOK-CHAT-SUB-2 — 클라이언트가 다른 키의 답을 막는다");
    assert_eq!(a, Answer::NoReply, "S-sym-RULE-HOOK-CHAT-SUB-2 — ACK 0 · 그 묶음은 다시 온다(답 없음)");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_the_gate_does_not_take() {
    let r = rig().await;
    let s = open(&r, "acme").await;
    let q = s.declare_queryable(sub_key("acme")).await.expect("queryable");
    let _cid = open_cid(&r).await;
    let refused0 = r.gate.drops.refused.load(Ordering::Relaxed);
    let _outside = s.declare_queryable(format!("ox/{NS}/elsewhere/q")).await.expect("outside queryable");
    assert!(wait(3_000, || r.gate.drops.refused.load(Ordering::Relaxed) > refused0).await, "S-rule-RULE-HOOK-CHAT-SUB-12② · S-sym-RULE-BUS-PUBLIC-3① — 허용 키 밖 선언은 무시하고 센다");
    s.put(format!("ox/{NS}/pub/v1/x"), b"v".to_vec()).await.expect("put");
    assert!(wait(3_000, || r.gate.drops.push_dropped.load(Ordering::Relaxed) >= 1).await, "S-rule-RULE-HOOK-CHAT-SUB-12① — 3rd 의 put 은 버리고 센다");
    let started = Instant::now();
    let replies = s.get(format!("ox/{NS}/pub/v1/q")).await.expect("get");
    let mut n = 0;
    while replies.recv_async().await.is_ok() {
        n += 1;
    }
    assert_eq!(n, 0, "S-rule-RULE-HOOK-CHAT-SUB-12③ — 3rd 의 질의에는 결과가 없다");
    assert!(started.elapsed() < Duration::from_secs(5), "S-rule-RULE-HOOK-CHAT-SUB-12③ — 결과 없이 바로 끝(시한을 기다리지 않는다) {:?}", started.elapsed());
    let started = Instant::now();
    let tokens_got = s.liveliness().get(format!("ox/{NS}/pub/v1/live/**")).await.expect("liveliness get");
    while tokens_got.recv_async().await.is_ok() {}
    assert!(started.elapsed() < Duration::from_secs(5), "정:RULE-CHAT-EXT-SUB 7 — 지금 Interest 에 DeclareFinal 로 답한다(liveliness get 이 시한 전에 끝) {:?}", started.elapsed());
    r.bus.declare_sess("u1", "s1").await;
    assert_eq!(r.bus.find_sess("s1").await, Some(("u1".to_string(), "n1".to_string())), "양성 대조 — 안쪽 int/** 에는 세션 토큰이 서 있고 안쪽에서는 읽힌다");
    let pushed0 = r.gate.drops.push_dropped.load(Ordering::Relaxed);
    let _forged = s.liveliness().declare_token(format!("ox/{NS}/int/sess/u9/n9/s9")).await.expect("forged token");
    s.put(format!("ox/{NS}/int/ev/room/r1"), b"v".to_vec()).await.expect("put int");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(r.bus.find_sess("s9").await.is_none() && !r.bus.session_users().await.contains(&"u9".to_string()), "S-sym-RULE-BUS-PUBLIC-3① — 3rd 가 int/** 에 쓴 토큰은 안쪽에 서지 않는다");
    assert!(r.gate.drops.push_dropped.load(Ordering::Relaxed) > pushed0, "S-sym-RULE-BUS-PUBLIC-3① — 3rd 가 int/** 에 쓴 put 은 버리고 센다");
    _forged.undeclare().await.expect("undeclare forged");
    let started = Instant::now();
    let inner = s.get(format!("ox/{NS}/int/**")).await.expect("get int");
    let mut seen = 0;
    while inner.recv_async().await.is_ok() {
        seen += 1;
    }
    let inner_tokens = s.liveliness().get(format!("ox/{NS}/int/**")).await.expect("liveliness get int");
    let mut seen_tokens = 0;
    while inner_tokens.recv_async().await.is_ok() {
        seen_tokens += 1;
    }
    assert!(seen == 0 && seen_tokens == 0 && started.elapsed() < Duration::from_secs(5), "S-sym-RULE-BUS-PUBLIC-3① — 3rd 는 int/** 를 읽지 못한다(질의 {seen} · 토큰 {seen_tokens} · {:?})", started.elapsed());
    assert_eq!(r.gate.conns().len(), 1, "S-rule-RULE-HOOK-CHAT-SUB-12④ — 그동안 세션은 산다");
    drop(q);
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn declaring_another_accounts_key_closes_the_session() {
    let r = rig().await;
    let s = open(&r, "acme").await;
    let refused0 = r.gate.drops.refused.load(Ordering::Relaxed);
    let _q = s.declare_queryable(sub_key("other")).await;
    assert!(wait(5_000, || r.gate.conns().is_empty()).await, "S-rule-RULE-HOOK-CHAT-SUB-4① — 인증서 계정과 다른 chat/sub 키를 선언하면 세션이 닫힌다");
    assert!(r.gate.drops.refused.load(Ordering::Relaxed) > refused0, "S-rule-RULE-HOOK-CHAT-SUB-4① — ext_refused +1");
    assert!(tokens(&r).await.is_empty(), "형상 — 토큰이 서지 않았다");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refused_before_the_session() {
    let r = rig().await;
    let ep = tls_endpoint(&r);
    let tcp = format!("tcp/127.0.0.1:{}", r.gate.addr.port());
    assert!(refused_open(client_cfg(&r, &ep, None)).await && counted(&r, 0).await, "S-rule-RULE-HOOK-CREDENTIAL-1① · S-sym-RULE-BUS-PUBLIC-3② — 인증서 없음은 붙지 못하고 ext_refused +1");
    assert!(refused_open(client_cfg(&r, &ep, Some("rogue"))).await && counted(&r, 1).await, "S-rule-RULE-HOOK-CREDENTIAL-1② — 다른 CA 인증서는 붙지 못하고 ext_refused +1");
    assert!(refused_open(client_cfg(&r, &ep, Some("nohook"))).await && counted(&r, 2).await, "S-rule-RULE-HOOK-CREDENTIAL-1③ — 연동 허용 없는 계정은 붙지 못하고 ext_refused +1");
    let refused3 = r.gate.drops.refused.load(Ordering::Relaxed);
    let bare = open(&r, "nosub").await;
    assert!(
        wait(3_000, || r.gate.conns().len() == 1).await,
        "S-rule-RULE-HOOK-CREDENTIAL-1④ · S-rule-RULE-HOOK-DIRECT-3① — 연동 허용 계정은 채팅 구독 · 사실 칸이 비어도 붙는다"
    );
    assert_eq!(r.gate.drops.refused.load(Ordering::Relaxed), refused3, "S-rule-RULE-HOOK-DIRECT-3① — 붙은 계정은 ext_refused 를 올리지 않는다");
    drop(bare);
    assert!(wait(5_000, || r.gate.conns().is_empty()).await, "형상 — 닫힌 세션은 연결표에서 빠진다");
    assert!(refused_open(client_cfg(&r, &ep, Some("stranger"))).await && counted(&r, 3).await, "정:RULE-CHAT-EXT-SUB 7 — 모르는 계정은 붙지 못하고 ext_refused +1");
    assert!(refused_open(client_cfg(&r, &tcp, None)).await && counted(&r, 4).await, "S-rule-RULE-HOOK-CHAT-SUB-1② · S-sym-RULE-BUS-PUBLIC-3② — 평문 TCP 는 닫히고 ext_refused +1");
    assert!(r.gate.conns().is_empty(), "형상 — 거절된 것은 연결표에 들지 않는다");
    let s = open(&r, "acme").await;
    assert!(wait(3_000, || r.gate.conns().len() == 1).await, "S-rule-RULE-HOOK-CHAT-SUB-1① · S-rule-RULE-HOOK-CREDENTIAL-1⑤ — tls 로 붙는다");
    drop(s);
}

struct Raw {
    rd: tokio::io::ReadHalf<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>,
    wr: tokio::io::WriteHalf<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>,
}

impl Raw {
    async fn connect(r: &Rig, cn: &str) -> Raw {
        let ca = std::fs::read(r.pki.path("ca.pem")).expect("ca");
        let mut roots = tokio_rustls::rustls::RootCertStore::empty();
        for c in rustls_pemfile::certs(&mut ca.as_slice()) {
            roots.add(c.expect("ca cert")).expect("root");
        }
        let cert: Vec<_> = rustls_pemfile::certs(&mut std::fs::read(r.pki.path(&format!("{cn}.pem"))).expect("cert").as_slice()).map(|c| c.expect("c")).collect();
        let key = rustls_pemfile::private_key(&mut std::fs::read(r.pki.path(&format!("{cn}.key"))).expect("key").as_slice()).expect("k").expect("k");
        let cfg = tokio_rustls::rustls::ClientConfig::builder().with_root_certificates(roots).with_client_auth_cert(cert, key).expect("client tls");
        let tcp = tokio::net::TcpStream::connect(r.gate.addr).await.expect("tcp");
        let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg)).connect(ServerName::try_from("localhost").expect("name"), tcp).await.expect("tls");
        let (rd, wr) = tokio::io::split(tls);
        Raw { rd, wr }
    }

    async fn send(&mut self, m: TransportMessage) {
        link::write_batches(&mut self.wr, &[wire::encode(&m).expect("encode")]).await.expect("send");
    }

    async fn recv(&mut self) -> Option<TransportMessage> {
        let b = tokio::time::timeout(Duration::from_secs(5), link::read_batch(&mut self.rd)).await.ok()?.ok()?;
        wire::decode_batch(&b).ok()?.into_iter().next()
    }

    fn init(version: u8, whatami: WhatAmI) -> TransportMessage {
        TransportMessage::from(InitSyn {
            version,
            whatami,
            zid: ZenohIdProto::try_from([7u8; 16]).expect("zid"),
            resolution: Resolution::default(),
            batch_size: u16::MAX,
            ext_qos: None,
            ext_qos_link: None,
            ext_auth: None,
            ext_mlink: None,
            ext_lowlatency: None,
            ext_compression: None,
            ext_patch: PatchType::NONE,
            ext_region_name: None,
        })
    }

    async fn open(&mut self, lease: Duration) {
        self.send(Raw::init(zenoh_protocol::VERSION, WhatAmI::Client)).await;
        let Some(TransportMessage { body: TransportBody::InitAck(ack), .. }) = self.recv().await else { panic!("InitAck") };
        self.send(TransportMessage::from(OpenSyn {
            lease,
            initial_sn: 0,
            cookie: ack.cookie,
            ext_qos: None,
            ext_auth: None,
            ext_mlink: None,
            ext_lowlatency: None,
            ext_compression: None,
            ext_remote_bound: None,
        }))
        .await;
        let Some(TransportMessage { body: TransportBody::OpenAck(_), .. }) = self.recv().await else { panic!("OpenAck") };
    }

    async fn declare(&mut self, key: &str) {
        let d = |body: DeclareBody| NetworkMessage {
            body: NetworkBody::Declare(Declare { interest_id: None, ext_qos: declare::ext::QoSType::DECLARE, ext_tstamp: None, ext_nodeid: declare::ext::NodeIdType::DEFAULT, body }),
            reliability: Reliability::Reliable,
        };
        let msgs = vec![
            d(DeclareBody::DeclareKeyExpr(DeclareKeyExpr { id: 1, wire_expr: WireExpr::from(key.to_string()) })),
            d(DeclareBody::DeclareQueryable(DeclareQueryable { id: 9, wire_expr: WireExpr { scope: 1, suffix: "".into(), mapping: Mapping::Sender }, ext_info: QueryableInfoType::DEFAULT })),
        ];
        self.send(TransportMessage::from(Frame { reliability: Reliability::Reliable, sn: 0, ext_qos: frame::ext::QoSType::DEFAULT, payload: msgs })).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_silent_session_expires_its_lease_and_the_token_goes() {
    let r = rig().await;
    let mut raw = Raw::connect(&r, "acme").await;
    raw.open(Duration::from_millis(800)).await;
    raw.declare(&sub_key("acme")).await;
    let t = wait_tokens(&r, 1, 3_000).await;
    assert_eq!(t.len(), 1, "형상 — Sender 매핑 키 식 + queryable 선언으로 토큰이 선다 {t:?}");
    let started = Instant::now();
    let mut closed_with = None;
    while let Some(m) = raw.recv().await {
        if let TransportBody::Close(c) = m.body {
            closed_with = Some(c.reason);
            break;
        }
    }
    assert_eq!(closed_with, Some(zenoh_protocol::transport::close::reason::EXPIRED), "정:RULE-CHAT-EXT-SUB 3 — 말없는 세션은 그 lease 를 넘기면 닫힌다(EXPIRED)");
    assert!(started.elapsed() < Duration::from_secs(3), "형상 — lease 0.8 s 언저리 {:?}", started.elapsed());
    let t = wait_tokens(&r, 0, 3_000).await;
    assert!(t.is_empty(), "정:RULE-CHAT-EXT-SUB 3 — lease 넘김에 토큰이 걷힌다 {t:?}");
    assert!(wait(3_000, || r.gate.conns().is_empty()).await, "형상 — 연결표에서 빠진다");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_version_or_a_non_client_is_closed_in_the_handshake() {
    let r = rig().await;
    let before = r.gate.drops.refused.load(Ordering::Relaxed);
    let mut raw = Raw::connect(&r, "acme").await;
    raw.send(Raw::init(0x08, WhatAmI::Client)).await;
    let got = raw.recv().await.map(|m| m.body);
    assert!(matches!(got, Some(TransportBody::Close(_))) && counted(&r, before).await, "S-rule-RULE-HOOK-CHAT-SUB-2 — 판 0x09 가 아니면 InitAck 대신 Close · ext_refused +1 {got:?}");
    let before = r.gate.drops.refused.load(Ordering::Relaxed);
    let mut raw = Raw::connect(&r, "acme").await;
    raw.send(Raw::init(zenoh_protocol::VERSION, WhatAmI::Peer)).await;
    let got = raw.recv().await.map(|m| m.body);
    assert!(matches!(got, Some(TransportBody::Close(_))) && counted(&r, before).await, "S-rule-RULE-HOOK-CHAT-SUB-1③ — client 가 아니면 InitAck 대신 Close · ext_refused +1 {got:?}");
    assert!(r.gate.conns().is_empty(), "형상 — 연결표에 들지 않는다");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_with_usrpwd_on_fails_on_its_own_side() {
    let r = rig().await;
    let mut cfg = client_cfg(&r, &tls_endpoint(&r), Some("acme"));
    cfg.insert_json5("transport/auth/usrpwd", r#"{ user: "u", password: "p" }"#).expect("usrpwd");
    assert!(refused_open(cfg).await, "S-rule-RULE-HOOK-CHAT-SUB-1④ · S-rule-RULE-HOOK-CHAT-SUB-3④ — usrpwd 를 켠 클라이언트는 붙지 못한다(입구가 auth 확장을 되돌리지 않는다)");
    assert!(wait(3_000, || r.gate.conns().is_empty()).await, "형상 — 연결표에 남지 않는다");
}

#[test]
fn judgments_are_pure_with_reasons() {
    use oxhubd::hook::zgate::judge::{DeclareVerdict, Refuse, judge_account, judge_declare, judge_init};
    use zenoh_protocol::core::Bits;
    use zenoh_protocol::core::resolution::Field;
    let sys = common::system::System::parse(concat!(
        "[hub]\nlisten = \"0.0.0.0:1974\"\n[bus]\nnamespace = \"ox\"\n[chat]\ndb = \"memory:\"\n",
        "[[hub.auth.api_keys]]\nkey = \"acme\"\nsecret = \"s\"\nhook_allowed = true\nsubscribe = [\"\"]\n",
        "[[hub.auth.api_keys]]\nkey = \"nohook\"\nsecret = \"s\"\n",
        "[[hub.auth.api_keys]]\nkey = \"nosub\"\nsecret = \"s\"\nhook_allowed = true\n",
        "[[hub.auth.api_keys]]\nkey = \"facts\"\nsecret = \"s\"\nhook_allowed = true\nfacts = [\"*\"]\n",
    ))
    .expect("system");
    let acc = Accounts::of(&sys);
    assert_eq!(judge_account(Some("acme"), &acc), Ok("acme".to_string()));
    assert_eq!(judge_account(None, &acc), Err(Refuse::NoCn));
    assert_eq!(judge_account(Some("nohook"), &acc).map_err(|r| r.reason()), Err("hook not allowed"), "S-rule-RULE-HOOK-DIRECT-3② — 연동 허용 없는 계정은 닫힌다");
    assert_eq!(judge_account(Some("nosub"), &acc), Ok("nosub".to_string()), "S-rule-RULE-HOOK-CREDENTIAL-1④ · S-rule-RULE-HOOK-DIRECT-3① — 채팅 구독 · 사실 칸이 비어도 붙는다");
    assert_eq!(judge_account(Some("facts"), &acc), Ok("facts".to_string()), "S-rule-RULE-HOOK-DIRECT-3① — 사실 구독만 둔 계정도 붙는다");
    assert_eq!(judge_account(Some("x"), &acc).map_err(|r| r.reason()), Err("unknown account"));
    let syn = |v: u8, w: WhatAmI, res: Resolution, b: u16| InitSyn {
        version: v,
        whatami: w,
        zid: ZenohIdProto::try_from([7u8; 16]).expect("zid"),
        resolution: res,
        batch_size: b,
        ext_qos: None,
        ext_qos_link: None,
        ext_auth: None,
        ext_mlink: None,
        ext_lowlatency: None,
        ext_compression: None,
        ext_patch: PatchType::NONE,
        ext_region_name: None,
    };
    let mut small = Resolution::default();
    small.set(Field::FrameSN, Bits::U16);
    let a = judge_init(&syn(zenoh_protocol::VERSION, WhatAmI::Client, small, 1_000), Resolution::default(), u16::MAX).expect("agreed");
    assert_eq!((a.resolution.get(Field::FrameSN), a.resolution.get(Field::RequestID), a.batch), (Bits::U16, Bits::U32, 1_000), "정:RULE-CHAT-EXT-SUB 3 — 해상도 · 배치 크기는 둘 중 작은 것");
    assert_eq!(judge_init(&syn(0x08, WhatAmI::Client, Resolution::default(), 100), Resolution::default(), u16::MAX).map_err(|r| r.detail()), Err("got=0x08 want=0x09".to_string()), "판 · 사유");
    assert_eq!(judge_init(&syn(zenoh_protocol::VERSION, WhatAmI::Router, Resolution::default(), 100), Resolution::default(), u16::MAX).map_err(|r| r.reason()), Err("not a client"));
    let p = format!("ox/{NS}/pub/v1/");
    assert_eq!(judge_declare(&sub_key("acme"), &p, "acme"), DeclareVerdict::Own(Lane::Chat));
    assert_eq!(judge_declare(&sub_key("other"), &p, "acme"), DeclareVerdict::OtherAccount(sub_key("other")));
    assert_eq!(judge_declare(&sub_key("**"), &p, "acme"), DeclareVerdict::OtherAccount(sub_key("**")), "와일드카드로 남의 몫을 덮는 선언도 닫는다");
    assert_eq!(judge_declare("a/b", &p, "acme"), DeclareVerdict::Outside("a/b".into()));
}

#[test]
fn sequence_numbers_fragments_and_batches() {
    use oxhubd::hook::zgate::wire::{Defrag, Piece, RxSn, SnVerdict, TxSn, pack, parts, room};
    use zenoh_protocol::network::{Push, push};
    let mask = u32::MAX >> 4;
    let mut rx = RxSn::new(5, mask);
    assert_eq!([rx.judge(5), rx.judge(5), rx.judge(7), rx.judge(6)], [SnVerdict::Next, SnVerdict::Stale, SnVerdict::Gap, SnVerdict::Stale], "처음은 알린 첫 번호 · 같은 번호 · 뒤 번호는 묵은 것");
    let mut wrap = RxSn::new(mask, mask);
    assert_eq!([wrap.judge(mask), wrap.judge(0)], [SnVerdict::Next, SnVerdict::Next], "감김");
    assert_eq!(RxSn::new(0, mask).judge(mask + 1), SnVerdict::Stale, "마스크 밖 번호");
    let z = |b: &[u8]| zenoh_buffers::ZSlice::from(b.to_vec());
    let mut d = Defrag::default();
    assert_eq!(d.push(SnVerdict::Next, true, &z(b"ab")), Piece::More);
    assert_eq!(d.push(SnVerdict::Next, false, &z(b"cd")), Piece::Whole(b"abcd".to_vec()));
    assert_eq!(d.push(SnVerdict::Next, true, &z(b"ab")), Piece::More);
    assert_eq!(d.push(SnVerdict::Gap, false, &z(b"cd")), Piece::Dropped("fragment sn gap"), "번호가 비면 잇던 것을 버린다");
    let big = vec![0u8; oxhubd::hook::zgate::wire::DEFRAG_MAX + 1];
    assert_eq!(d.push(SnVerdict::Next, true, &zenoh_buffers::ZSlice::from(big)), Piece::Dropped("fragment over the defrag limit"));
    let push = |n: usize| NetworkMessage {
        body: NetworkBody::Push(Push {
            wire_expr: WireExpr::from("k".to_string()),
            ext_qos: push::ext::QoSType::DEFAULT,
            ext_tstamp: None,
            ext_nodeid: push::ext::NodeIdType::DEFAULT,
            ext_ts_stack: None,
            payload: zenoh_protocol::zenoh::PushBody::Put(zenoh_protocol::zenoh::Put {
                timestamp: None,
                encoding: zenoh_protocol::core::Encoding::empty(),
                ext_sinfo: None,
                ext_attachment: None,
                ext_unknown: vec![],
                payload: zenoh_buffers::ZBuf::from(vec![1u8; n]),
            }),
        }),
        reliability: Reliability::Reliable,
    };
    let batch = 1_000u16;
    let mut sn = TxSn::new(10, mask);
    let one = parts(&push(100), &mut sn, batch).expect("frame");
    assert_eq!(one.len(), 1, "배치 안이면 Frame 하나");
    let many = parts(&push(5_000), &mut sn, batch).expect("fragments");
    assert!(many.len() > 5 && many.iter().all(|p| p.len() <= room(batch)), "넘으면 조각 — 조각마다 배치 몸 한도 안 {:?}", many.iter().map(|p| p.len()).collect::<Vec<_>>());
    assert_eq!(sn.take(), 10 + 1 + many.len() as u32, "Frame · 조각마다 번호 하나");
    let packed = pack(many.clone(), batch);
    assert!(packed.iter().all(|b| b.len() <= room(batch)), "배치 몸 ≤ 배치 크기 − 길이 2 바이트");
    assert_eq!(packed.iter().map(|b| b.len()).sum::<usize>(), many.iter().map(|p| p.len()).sum::<usize>());
}

fn body_of(q: &zenoh::query::Query) -> serde_json::Value {
    let b = q.payload().map(|p| p.to_bytes().to_vec()).unwrap_or_default();
    serde_json::from_slice(&b).expect("query body json")
}

fn ids_of(v: &serde_json::Value) -> Vec<String> {
    v["messages"].as_array().map(|a| a.iter().filter_map(|m| m["msg_id"].as_str().map(str::to_string)).collect()).unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ack,
    Err,
    Hold,
}

fn answering(q: zenoh::query::Queryable<zenoh::handlers::FifoChannelHandler<zenoh::query::Query>>, mode: Mode) -> (tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>, tokio::task::JoinHandle<()>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let h = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok(query) = q.recv_async().await {
            let _ = tx.send(body_of(&query));
            match mode {
                Mode::Ack => {
                    let _ = query.reply(query.key_expr().clone(), b"ok".to_vec()).await;
                }
                Mode::Err => {
                    let _ = query.reply_err(b"no".to_vec()).await;
                }
                Mode::Hold => held.push(query),
            }
        }
    });
    (rx, h)
}

fn post(r: &Rig, i: usize) -> String {
    let id = format!("m{i:04}");
    match r.chat.store.send("u1", "S", &id, "hi", None).expect("send") {
        oxhubd::chat::store::Sent::Fresh { ext, .. } => assert_eq!(ext, vec!["acme".to_string()], "형상 — 글은 acme 구독에 걸린다"),
        _ => panic!("fresh"),
    }
    id
}

async fn rows_left(r: &Rig, n: usize, ms: u64) -> usize {
    let end = Instant::now() + Duration::from_millis(ms);
    loop {
        let left = r.chat.store.ext_rows("acme").expect("rows").len();
        if left == n || Instant::now() >= end {
            return left;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_waited_is_drained_when_the_client_comes_and_acks_clear_it() {
    let r = rig().await;
    let ids: Vec<String> = (0..3).map(|i| post(&r, i)).collect();
    assert_eq!(r.chat.store.ext_rows("acme").expect("rows").len(), 3, "형상 — 붙기 전 펜딩 셋");
    let s = open(&r, "acme").await;
    let (mut got, _h) = answering(s.declare_queryable(sub_key("acme")).await.expect("queryable"), Mode::Ack);
    let first = tokio::time::timeout(Duration::from_secs(5), got.recv()).await.expect("drain in time").expect("body");
    assert_eq!(first["live"], serde_json::json!(false), "정:RULE-CHAT-EXT-SUB 6 — 밀린 몫은 live:false 묶음");
    assert_eq!(ids_of(&first), ids, "정:RULE-CHAT-EXT-SUB 6 — 막 주인이 된 연결로 (t, msg_id) 차례 전부 · 묶음 하나");
    assert_eq!(rows_left(&r, 0, 3_000).await, 0, "정:RULE-CHAT-EXT-SUB 2 — 묶음 ACK 가 3rd ACK 줄로 펜딩을 지운다");
    let cid = open_cid(&r).await;
    let live = post(&r, 9);
    let msg = r.chat.store.history("u1", "S", oxhubd::chat::store::Dir::Latest, 1).expect("history").messages.remove(0);
    assert_eq!(msg.msg_id, live);
    let o = r.gate.deliver(&cid, true, vec![msg], None).expect("deliver");
    assert!(o.accepted(), "정:RULE-CHAT-EXT-SUB 4 — 연결이 있으면 넣고 성공");
    let b = tokio::time::timeout(Duration::from_secs(5), got.recv()).await.expect("live in time").expect("body");
    assert_eq!((b["live"].clone(), ids_of(&b)), (serde_json::json!(true), vec![live]), "S-rule-RULE-HOOK-CHAT-SUB-6① — 실시간 몸 {{live:true, messages}} · 메시지 꼴 = MESSAGES 의 것");
    assert_eq!(rows_left(&r, 0, 3_000).await, 0, "형상 — 실시간 ACK 도 지운다");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reply_timeouts_in_a_row_close_the_session_and_count_tries() {
    let r = rig().await;
    post(&r, 0);
    let s = open(&r, "acme").await;
    let (mut got, _h) = answering(s.declare_queryable(sub_key("acme")).await.expect("queryable"), Mode::Hold);
    let _ = tokio::time::timeout(Duration::from_secs(5), got.recv()).await.expect("drain in time");
    let cid = open_cid(&r).await;
    let msg = r.chat.store.history("u1", "S", oxhubd::chat::store::Dir::Latest, 1).expect("history").messages.remove(0);
    let _ = r.gate.deliver(&cid, true, vec![msg], None);
    assert!(wait(8_000, || !r.gate.conns().iter().any(|c| c.cid == cid)).await, "S-sym-RULE-CHAT-EXT-SUB-6① · PL-KEY-HOOK-EXT-REPLY-TIMEOUT-MS② · S-sym-RULE-HOOK-CHAT-SUB-3 — 잇단 시간 넘김 close_at 번에 그 세션을 닫는다(클라이언트는 새 연결로 다시 붙는다)");
    assert!(r.gate.drops.reply_timeout.load(Ordering::Relaxed) >= 2, "PL-KEY-HOOK-EXT-REPLY-TIMEOUT-MS① — 시간 넘김마다 ext_reply_timeout +1");
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end && r.chat.store.ext_rows("acme").expect("rows").first().is_none_or(|x| x.tries == 0) {
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(r.chat.store.ext_rows("acme").expect("rows").first().is_some_and(|x| x.tries >= 1), "정:RULE-CHAT-EXT-SUB 2 — 시간 넘김은 ACK 아님 · 시도 수 +1 · 행은 남는다");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_row_that_is_never_acked_is_given_up_after_max_tries() {
    let mut k = knobs();
    k.sweep = Duration::from_millis(1_000);
    let r = rig_with(k).await;
    post(&r, 0);
    let s = open(&r, "acme").await;
    let (_got, _h) = answering(s.declare_queryable(sub_key("acme")).await.expect("queryable"), Mode::Err);
    assert_eq!(rows_left(&r, 0, 12_000).await, 0, "S-sym-RULE-CHAT-EXT-SUB-7① · PL-KEY-HOOK-EXT-MAX-TRIES · PL-KEY-HOOK-EXT-SWEEP-SECS — 훑기가 다시 보내고 max_tries 번 뒤 지운다");
    assert_eq!(r.gate.drops.given_up.load(Ordering::Relaxed), 1, "S-sym-RULE-CHAT-EXT-SUB-7① — ext_given_up +1");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_full_queue_refuses_and_counts() {
    let mut k = knobs();
    k.window = 1;
    k.cap = 400;
    k.reply_timeout = Duration::from_secs(30);
    let r = rig_with(k).await;
    let s = open(&r, "acme").await;
    let (_got, _h) = answering(s.declare_queryable(sub_key("acme")).await.expect("queryable"), Mode::Hold);
    let cid = open_cid(&r).await;
    post(&r, 0);
    let msg = r.chat.store.history("u1", "S", oxhubd::chat::store::Dir::Latest, 1).expect("history").messages.remove(0);
    let offers: Vec<&str> = (0..40).map(|_| r.gate.deliver(&cid, true, vec![msg.clone()], None).expect("deliver").as_str()).collect();
    let first_refusal = offers.iter().position(|o| o.starts_with("refused")).expect("a refusal");
    assert_eq!(offers[first_refusal], "refused_full", "PL-KEY-HOOK-EXT-WINDOW① · 정:RULE-CHAT-EXT-SUB 5 — 창이 차 줄에 쌓이다 상한을 넘으면 밀림 · 거절 {offers:?}");
    assert!(offers[first_refusal + 1..].iter().all(|o| *o == "refused_backlog"), "정:RULE-CHAT-EXT-SUB 5 — 밀림이면 새 것은 거절 {offers:?}");
    assert!(offers[..first_refusal].iter().all(|o| *o == "queued" || *o == "merged"), "형상 {offers:?}");
    assert_eq!(r.gate.drops.queue_full.load(Ordering::Relaxed), (offers.len() - first_refusal) as u64, "정:RULE-CHAT-EXT-SUB 5 — 거절마다 ext_queue_full");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_overflows_the_window_merges_into_the_bundle_at_the_line_end() {
    let mut k = knobs();
    k.window = 1;
    k.reply_timeout = Duration::from_secs(30);
    let r = rig_with(k).await;
    let s = open(&r, "acme").await;
    let q = s.declare_queryable(sub_key("acme")).await.expect("queryable");
    let cid = open_cid(&r).await;
    let release = Arc::new(tokio::sync::Notify::new());
    let (tx, mut got) = tokio::sync::mpsc::unbounded_channel();
    let rel = release.clone();
    let _h = tokio::spawn(async move {
        let mut first = true;
        while let Ok(query) = q.recv_async().await {
            let _ = tx.send(body_of(&query));
            if first {
                first = false;
                rel.notified().await;
            }
            let _ = query.reply(query.key_expr().clone(), b"ok".to_vec()).await;
        }
    });
    let ids: Vec<String> = (0..4).map(|i| post(&r, i)).collect();
    let page = r.chat.store.history("u1", "S", oxhubd::chat::store::Dir::Latest, 10).expect("history").messages;
    let msg = |id: &str| page.iter().find(|m| m.msg_id == id).cloned().expect("msg");
    let o0 = r.gate.deliver(&cid, true, vec![msg(&ids[0])], None).expect("deliver").as_str();
    let held = tokio::time::timeout(Duration::from_secs(5), got.recv()).await.expect("first in time").expect("body");
    assert_eq!((o0, ids_of(&held)), ("queued", vec![ids[0].clone()]), "positive control — the first one goes alone and fills the window of one");
    let offers: Vec<&str> = ids[1..].iter().map(|id| r.gate.deliver(&cid, true, vec![msg(id)], None).expect("deliver").as_str()).collect();
    assert_eq!(offers, vec!["queued", "merged", "merged"], "PL-KEY-HOOK-EXT-WINDOW② over the window the first waits at the line end and the rest merge into that bundle");
    release.notify_one();
    let next = tokio::time::timeout(Duration::from_secs(5), got.recv()).await.expect("bundle in time").expect("body");
    assert_eq!((next["live"].clone(), ids_of(&next)), (serde_json::json!(true), ids[1..].to_vec()), "PL-KEY-HOOK-EXT-WINDOW② the overflow goes as one merged bundle once the window opens");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_given_up_message_does_not_hold_back_the_others() {
    let mut k = knobs();
    k.sweep = Duration::from_millis(1_000);
    let r = rig_with(k).await;
    let bad = post(&r, 0);
    let s = open(&r, "acme").await;
    let q = s.declare_queryable(sub_key("acme")).await.expect("queryable");
    let (tx, mut got) = tokio::sync::mpsc::unbounded_channel();
    let poison = bad.clone();
    let _h = tokio::spawn(async move {
        while let Ok(query) = q.recv_async().await {
            let b = body_of(&query);
            let refuse = ids_of(&b).contains(&poison);
            let _ = tx.send(b);
            if refuse {
                let _ = query.reply_err(b"no".to_vec()).await;
            } else {
                let _ = query.reply(query.key_expr().clone(), b"ok".to_vec()).await;
            }
        }
    });
    assert!(wait(12_000, || r.gate.drops.given_up.load(Ordering::Relaxed) >= 1).await, "positive control — the one always refused is given up (ext_given_up)");
    while got.try_recv().is_ok() {}
    let later: Vec<String> = (1..4).map(|i| post(&r, i)).collect();
    assert_eq!(rows_left(&r, 0, 5_000).await, 0, "S-sym-RULE-CHAT-EXT-SUB-7② after the give-up the other messages go and are cleared");
    let mut seen = Vec::new();
    while let Ok(b) = got.try_recv() {
        seen.extend(ids_of(&b));
    }
    assert!(!seen.contains(&bad), "S-sym-RULE-CHAT-EXT-SUB-7② the given-up one is not carried again {seen:?}");
    assert!(later.iter().all(|id| seen.contains(id)), "S-sym-RULE-CHAT-EXT-SUB-7② every other message went {seen:?}");
    assert_eq!(r.gate.drops.given_up.load(Ordering::Relaxed), 1, "S-sym-RULE-CHAT-EXT-SUB-7② the give-up dragged nothing else along");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_resent_older_message_arrives_after_a_newer_live_one() {
    let mut k = knobs();
    k.sweep = Duration::from_millis(1_000);
    let r = rig_with(k).await;
    let older = post(&r, 0);
    tokio::time::sleep(Duration::from_millis(5)).await;
    let s = open(&r, "acme").await;
    let q = s.declare_queryable(sub_key("acme")).await.expect("queryable");
    let (tx, mut got) = tokio::sync::mpsc::unbounded_channel();
    let first_refused = older.clone();
    let _h = tokio::spawn(async move {
        let mut refused_once = false;
        while let Ok(query) = q.recv_async().await {
            let b = body_of(&query);
            let refuse = !refused_once && ids_of(&b).contains(&first_refused);
            let _ = tx.send(b);
            if refuse {
                refused_once = true;
                let _ = query.reply_err(b"no".to_vec()).await;
            } else {
                let _ = query.reply(query.key_expr().clone(), b"ok".to_vec()).await;
            }
        }
    });
    let cid = open_cid(&r).await;
    let one = tokio::time::timeout(Duration::from_secs(5), got.recv()).await.expect("drain in time").expect("body");
    assert_eq!(ids_of(&one), vec![older.clone()], "positive control — the waiting message is carried first and refused once");
    let newer = post(&r, 1);
    let msg = r.chat.store.history("u1", "S", oxhubd::chat::store::Dir::Latest, 1).expect("history").messages.remove(0);
    assert_eq!(msg.msg_id, newer);
    assert!(r.gate.deliver(&cid, true, vec![msg], None).expect("deliver").accepted(), "positive control — the live one is taken");
    let mut bodies = vec![one];
    let end = Instant::now() + Duration::from_secs(10);
    while bodies.iter().flat_map(|b| ids_of(b)).filter(|i| *i == older).count() < 2 && Instant::now() < end {
        if let Ok(Some(b)) = tokio::time::timeout(Duration::from_secs(1), got.recv()).await {
            bodies.push(b);
        }
    }
    let arrivals: Vec<String> = bodies.iter().flat_map(|b| ids_of(b)).collect();
    let count = |id: &str| arrivals.iter().filter(|i| *i == id).count();
    assert_eq!(count(&older), 2, "S-rule-RULE-HOOK-CHAT-SUB-9① S-sym-RULE-HOOK-CHAT-SUB-1① — the refused message is carried again: the same msg_id arrives twice, so a 3rd that does not filter by msg_id handles it twice {arrivals:?}");
    assert!(count(&newer) >= 1, "형상 — the live one arrived {arrivals:?}");
    let first_newer = arrivals.iter().position(|i| *i == newer).expect("newer arrived");
    let last_older = arrivals.iter().rposition(|i| *i == older).expect("older arrived");
    let t_older = bodies.iter().map(|b| b["messages"].as_array().cloned().unwrap_or_default()).flatten().find(|m| m["msg_id"] == older).and_then(|m| m["t"].as_u64()).expect("t older");
    let t_newer = bodies.iter().map(|b| b["messages"].as_array().cloned().unwrap_or_default()).flatten().find(|m| m["msg_id"] == newer).and_then(|m| m["t"].as_u64()).expect("t newer");
    assert!(t_older < t_newer, "형상 — the older message has the earlier t");
    assert!(last_older > first_newer, "S-rule-RULE-HOOK-CHAT-SUB-10③ — the arrival order differs from the t order: the older message arrives again after the newer one {arrivals:?}");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_waited_past_the_retention_is_gone_when_the_client_comes() {
    let r = rig().await;
    let old = "mold0".to_string();
    match r.chat.store.send("u1", "S", &old, "waited too long", Some(1_000)).expect("send") {
        oxhubd::chat::store::Sent::Fresh { ext, .. } => assert_eq!(ext, vec!["acme".to_string()], "형상 — the old message waits for acme"),
        _ => panic!("fresh"),
    }
    let fresh = post(&r, 7);
    assert_eq!(r.chat.store.ext_rows("acme").expect("rows").len(), 2, "형상 — an old row and a new row wait");
    r.chat.purge_once(common::now_ms()).expect("purge");
    let left: Vec<String> = r.chat.store.ext_rows("acme").expect("rows").into_iter().map(|x| x.msg_id).collect();
    assert_eq!(left, vec![fresh.clone()], "S-rule-RULE-HOOK-CHAT-SUB-11 — the retention purge removes the old waiting row and keeps the new one");
    let s = open(&r, "acme").await;
    let (mut got, _h) = answering(s.declare_queryable(sub_key("acme")).await.expect("queryable"), Mode::Ack);
    let b = tokio::time::timeout(Duration::from_secs(5), got.recv()).await.expect("drain in time").expect("body");
    assert_eq!(ids_of(&b), vec![fresh], "S-rule-RULE-HOOK-CHAT-SUB-11 — a client that comes after the retention gets the new share only · the purged one never comes (positive control — the drain itself works)");
    assert!(!ids_of(&b).contains(&old), "S-rule-RULE-HOOK-CHAT-SUB-11 — the purged message is not carried");
    let more = tokio::time::timeout(Duration::from_millis(1_500), got.recv()).await;
    assert!(more.is_err(), "S-rule-RULE-HOOK-CHAT-SUB-11 — nothing else follows {more:?}");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_zid_with_two_connections_holds_two_rows() {
    let r = rig().await;
    let mut a = Raw::connect(&r, "acme").await;
    a.open(Duration::from_secs(30)).await;
    a.declare(&sub_key("acme")).await;
    let mut b = Raw::connect(&r, "acme").await;
    b.open(Duration::from_secs(30)).await;
    b.declare(&sub_key("acme")).await;
    let t = wait_tokens(&r, 2, 3_000).await;
    let mut cids: Vec<String> = t.iter().map(|(_, _, c)| c.clone()).collect();
    cids.sort();
    cids.dedup();
    assert_eq!((t.len(), cids.len()), (2, 2), "S-sym-RULE-CHAT-EXT-SUB-4① one zid on two connections — two rows in the ext table with different cids {t:?}");
    assert!(wait(3_000, || r.gate.conns().iter().filter(|c| c.open).count() == 2).await, "S-sym-RULE-CHAT-EXT-SUB-4① both connections stay open {:?}", r.gate.conns().len());
}

fn spec_dir() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/spec")
}

#[test]
fn one_ack_clears_the_streak_of_reply_timeouts() {
    use oxhubd::chat::ext::plan::{After, Item, Knobs, Outbox, Outcome};
    let mut o: Outbox<(), ()> = Outbox::new(Knobs { window: 1, cap: 1_000, live_max: 100, close_at: 3 });
    let mut step = |o: &mut Outbox<(), ()>, oc: Outcome| {
        let n = o.queued() as u64 + 1;
        o.offer(Item { live: false, ids: vec![format!("m{n}")], bytes: 1, msgs: vec![()], waiter: None });
        assert!(o.take().is_some(), "형상 — 창 하나에 하나를 꺼낸다");
        o.done(oc)
    };
    assert_ne!(step(&mut o, Outcome::Timeout), After::Close);
    assert_ne!(step(&mut o, Outcome::Timeout), After::Close);
    assert_eq!(o.streak, 2, "형상 — 잇단 시간 넘김 둘");
    assert_ne!(step(&mut o, Outcome::Ack), After::Close);
    assert_eq!(o.streak, 0, "PL-KEY-HOOK-EXT-REPLY-TIMEOUT-MS③ PL-KEY-HOOK-EXT-TIMEOUT-CLOSE③ — ACK 하나가 잇단 수를 0 으로 되돌린다");
    assert_ne!(step(&mut o, Outcome::Timeout), After::Close);
    assert_ne!(step(&mut o, Outcome::Timeout), After::Close, "PL-KEY-HOOK-EXT-TIMEOUT-CLOSE③ — ACK 뒤 둘이 더 와도 닫지 않는다(이으면 셋)");
    assert_eq!(step(&mut o, Outcome::Timeout), After::Close, "PL-KEY-HOOK-EXT-TIMEOUT-CLOSE② — 잇단 수가 3 에 닿으면 닫는다");
}

#[test]
fn send_plan_vectors() {
    use oxhubd::chat::ext::plan::{After, Due, Item, Knobs, Outbox, Outcome, Row, drain_due, drain_owner, drain_pick, ring};
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(spec_dir().join("server/out/vectors/chat.json")).expect("vectors")).expect("json");
    let arr = |k: &str| v[k].as_array().cloned().unwrap_or_else(|| panic!("판 묶음 {k} 없음"));
    let ringv = arr("ext_ring");
    assert!(!ringv.is_empty());
    for c in ringv {
        let want: Vec<usize> = c["order"].as_array().expect("order").iter().map(|x| x.as_u64().expect("i") as usize).collect();
        assert_eq!(ring(c["n"].as_u64().expect("n") as usize, c["c"].as_u64().expect("c")), want, "정:RULE-CHAT-EXT-SUB 4 — 차례 {c}");
    }
    let ids = |x: &serde_json::Value| -> Vec<String> { x.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default() };
    for c in arr("ext_outbox") {
        let k = &c["knobs"];
        let g = |n: &str| k[n].as_u64().expect("knob") as usize;
        let mut o: Outbox<(), ()> = Outbox::new(Knobs { window: g("window"), cap: g("cap"), live_max: g("live_max"), close_at: g("close_at") as u32 });
        for s in c["steps"].as_array().expect("steps") {
            let step = s["step"].as_array().expect("step");
            let got: serde_json::Value = match step[0].as_str().expect("op") {
                "offer" => {
                    let it = ids(&step[2]);
                    let n = it.len();
                    serde_json::json!(o.offer(Item { live: step[1].as_bool().expect("live"), ids: it, bytes: step[3].as_u64().expect("bytes") as usize, msgs: vec![(); n], waiter: None }).as_str())
                }
                "take" => match o.take() {
                    Some(it) => serde_json::json!({ "live": it.live, "ids": it.ids, "bytes": it.bytes }),
                    None => serde_json::Value::Null,
                },
                _ => {
                    let oc = match step[1].as_str().expect("outcome") {
                        "ack" => Outcome::Ack,
                        "fail" => Outcome::Fail,
                        _ => Outcome::Timeout,
                    };
                    let a: After = o.done(oc);
                    serde_json::json!(a.as_str())
                }
            };
            assert_eq!(got, s["got"], "정:RULE-CHAT-EXT-SUB 5 S-rule-RULE-HOOK-CHAT-SUB-8 — {} · {s}", c["name"]);
            assert_eq!((o.inflight as u64, o.queued() as u64, o.backlog, o.streak as u64), (s["inflight"].as_u64().expect("i"), s["queued"].as_u64().expect("q"), s["backlog"].as_bool().expect("b"), s["streak"].as_u64().expect("s")), "{} · {s}", c["name"]);
        }
    }
    for c in arr("ext_drain_pick") {
        let rows: Vec<Row> = c["rows"].as_array().expect("rows").iter().map(|r| Row { id: r["id"].as_str().expect("id").into(), bytes: r["bytes"].as_u64().expect("b") as usize, tries: r["tries"].as_u64().expect("t") as u32 }).collect();
        assert_eq!(drain_pick(&rows, c["max_msgs"].as_u64().expect("m") as usize, c["max_bytes"].as_u64().expect("b") as usize), ids(&c["pick"]), "정:RULE-CHAT-EXT-SUB 6 — {}", c["name"]);
    }
    for c in arr("ext_drain_due") {
        let due = drain_due(c["owner"].as_bool().expect("o"), c["was_owner"].as_bool().expect("w"), c["released"].as_bool().expect("r"), c["now"].as_u64().expect("n"), c["last_sweep"].as_u64().expect("l"), c["sweep_ms"].as_u64().expect("s"));
        let want = match c["due"]["mode"].as_str().expect("mode") {
            "none" => Due::None,
            "all" => Due::All,
            _ => Due::Older(c["due"]["before"].as_u64().expect("before")),
        };
        assert_eq!(due, want, "정:RULE-CHAT-EXT-SUB 6 — {}", c["name"]);
    }
    let owners = arr("ext_drain_owner");
    assert!(!owners.is_empty());
    for c in owners {
        let conns: Vec<(String, String)> = c["conns"].as_array().expect("conns").iter().map(|p| (p[0].as_str().expect("node").to_string(), p[1].as_str().expect("cid").to_string())).collect();
        assert_eq!(drain_owner(&conns), c["owner"].as_u64().map(|i| i as usize), "S-sym-RULE-CHAT-EXT-SUB-10① · 정:RULE-CHAT-EXT-SUB 3 — drain 주인 = 가장 오래 붙어 있는 연결 · {}", c["name"]);
    }
}

fn facts_key(k: &str) -> String {
    format!("ox/{NS}/pub/v1/facts/sub/{k}")
}

fn bundle(id: &str) -> oxhubd::hook::zgate::send::FactsBundle {
    let f = oxhubd::hook::facts::Fact { fact_id: id.into(), kind: "room_created".into(), t: 1, room_id: "r1".into(), body: serde_json::json!({ "room_id": "r1" }) };
    oxhubd::hook::zgate::send::FactsBundle { facts: vec![f], dropped: None }
}

async fn get_one(s: &zenoh::Session, key: String, payload: Option<Vec<u8>>) -> Option<Result<Vec<u8>, Vec<u8>>> {
    let g = s.get(key);
    let replies = match payload {
        Some(p) => g.payload(p).await,
        None => g.await,
    }
    .expect("get");
    let r = tokio::time::timeout(Duration::from_secs(10), replies.recv_async()).await.expect("get ended in time").ok()?;
    Some(match r.result() {
        Ok(sample) => Ok(sample.payload().to_bytes().to_vec()),
        Err(e) => Err(e.payload().to_bytes().to_vec()),
    })
}

fn failure_code(b: &[u8]) -> u16 {
    serde_json::from_slice::<oxsig::Failure>(b).expect("Failure body").code
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_facts_lane_has_its_own_token_and_carries_bundles() {
    let r = rig().await;
    let s = open(&r, "acme").await;
    let q = s.declare_queryable(facts_key("acme")).await.expect("facts queryable");
    let cid = open_cid(&r).await;
    assert!(wait(3_000, || r.bus.facts_list("acme").len() == 1).await, "S-rule-RULE-HOOK-DIRECT-2② — 사실 구독 선언 → int/facts/{{key}}/{{node}}/{{cid}} 하나");
    assert!(r.gate.lane_open(&cid, Lane::Facts) && !r.gate.lane_open(&cid, Lane::Chat), "정:RULE-CHAT-EXT-SUB 5 — 키 갈래마다 따로 열린다");
    let got = tokio::spawn(async move {
        let mut seen = Vec::new();
        for i in 0..2 {
            let query = q.recv_async().await.expect("query");
            seen.push((query.key_expr().to_string(), body_of(&query)));
            if i == 0 {
                let _ = query.reply(query.key_expr().clone(), b"ok".to_vec()).await;
            } else {
                let _ = query.reply_err(b"no".to_vec()).await;
            }
        }
        (q, seen)
    });
    let (tx, rx) = tokio::sync::oneshot::channel();
    assert!(r.gate.deliver_facts(&cid, &bundle("f1"), Some(tx)).expect("deliver").accepted());
    assert_eq!(tokio::time::timeout(Duration::from_secs(5), rx).await.expect("ended").expect("outcome"), oxhubd::chat::ext::plan::Outcome::Ack, "S-rule-RULE-HOOK-FACTS-SUB-6① — reply = 묶음 전부 ACK");
    let payload = serde_json::to_vec(&oxhubd::hook::zgate::send::ExtDeliver::Facts { key: "acme".into(), cid: cid.clone(), bundle: bundle("f2") }).expect("encode");
    let reply = common::envelope::Reply::decode(&oxhubd::hook::zgate::send::serve_ext(Some(&r.gate), &payload).await).expect("reply");
    assert_eq!(reply, common::envelope::Reply::ok(serde_json::json!({ "end": "fail" })), "정:RULE-BUS-REQUEST 사실 넘기기 — 받는 hub 는 묶음의 끝(reply_err = ACK 아님)까지 기다려 답한다");
    let (q, seen) = tokio::time::timeout(Duration::from_secs(5), got).await.expect("3rd saw both").expect("join");
    assert_eq!(seen[0].0, facts_key("acme"), "HK-KEY-PUB-V1-FACTS-SUB-KEY① · S-rule-RULE-HOOK-FACTS-SUB-3① — hub 가 facts/sub/{{key}} 로 묻는다");
    assert_eq!(seen[0].1, serde_json::to_value(bundle("f1")).expect("v"), "S-rule-RULE-HOOK-FACTS-SUB-5① — query 하나 = 묶음 하나 · 몸 {{facts, dropped?}}");
    assert!(seen[0].1.get("live").is_none(), "S-rule-RULE-HOOK-FACTS-SUB-5② — live 칸 0");
    drop(q);
    assert!(wait(3_000, || r.bus.facts_list("acme").is_empty()).await, "정:RULE-BUS-PRESENCE — 사실 queryable 이 걷히면 토큰을 놓는다");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_facts_bundle_never_acked_is_given_up_after_max_tries() {
    let r = rig().await;
    let hook = Arc::new(oxhubd::hook::Hook::new("e1", vec![("acme".into(), vec!["*".into()])], 100));
    let _ = hook.zgate.set(r.gate.clone());
    let s = open(&r, "acme").await;
    let q = s.declare_queryable(facts_key("acme")).await.expect("facts queryable");
    let _cid = open_cid(&r).await;
    assert!(wait(3_000, || r.bus.facts_list("acme").len() == 1).await, "형상 — 사실 구독 연결 하나");
    const MAX: u32 = 3;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<serde_json::Value>();
    let _h = tokio::spawn(async move {
        let mut n = 0u32;
        while let Ok(query) = q.recv_async().await {
            let _ = tx.send(body_of(&query));
            n += 1;
            if n <= MAX {
                let _ = query.reply_err(b"no".to_vec()).await;
            } else {
                let _ = query.reply(query.key_expr().clone(), b"ok".to_vec()).await;
            }
        }
    });
    let knobs = oxhubd::hook::carry::Knobs { max_tries: MAX, bundle_max_bytes: 65_536, reply_timeout: Duration::from_millis(800), window: 4, backoff_ms: vec![150, 300] };
    tokio::spawn(oxhubd::hook::carry::run(hook.clone(), r.bus.clone(), "acme".into(), knobs));
    hook.offer(hook.build("r1", "room_created", serde_json::json!({ "room_id": "r1" }), 1));
    let mut seen = Vec::new();
    let mut at = Vec::new();
    for _ in 0..MAX {
        seen.push(tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("query in time").expect("query"));
        at.push(Instant::now());
    }
    assert!(seen.iter().all(|b| b == &seen[0]), "S-rule-RULE-HOOK-FACTS-SUB-6② — ACK 아닌 끝마다 같은 묶음이 다시 온다");
    let gaps: Vec<Duration> = at.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(
        gaps[0] >= Duration::from_millis(130) && gaps[1] >= Duration::from_millis(270),
        "PL-KEY-HOOK-RETRY-BACKOFF-MS① · S-rule-RULE-HOOK-FACTS-SUB-6⑥ — 사실 다시 보내기 간격은 n 번째 ACK 아닌 끝 뒤 retry_backoff_ms 의 n 번째 값(한순간에 상한을 소진하지 않는다): {gaps:?}"
    );
    assert!(wait(3_000, || hook.given_up.load(Ordering::Relaxed) == 1).await, "S-rule-RULE-HOOK-FACTS-SUB-6⑤ · PL-KEY-HOOK-EXT-MAX-TRIES — 사실 묶음은 ext_max_tries 번 뒤 버리고 fact_given_up +1");
    let next = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("next in time").expect("next");
    assert_eq!(next["dropped"], 1, "S-rule-RULE-HOOK-FACTS-SUB-6⑤ · PL-KEY-HOOK-EXT-MAX-TRIES — 버린 사실 수가 다음 묶음의 버림 표지(dropped)에 든다");
    assert_eq!(next["facts"], serde_json::json!([]), "S-rule-RULE-HOOK-FACTS-SUB-5⑦ — 줄이 비어도 버린 수가 있으면 빈 묶음으로 알린다");
    hook.offer(hook.build("r1", "room_closed", serde_json::json!({ "room_id": "r1" }), 2));
    let after = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("after in time").expect("after");
    assert_eq!(after["facts"][0]["type"], "room_closed");
    assert!(after.get("dropped").is_none(), "S-rule-RULE-HOOK-FACTS-SUB-5③ — 알린 뒤에는 버림 표지가 0(칸 없음)");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_facts_connection_does_not_stall_the_others() {
    let r = rig().await;
    let hook = Arc::new(oxhubd::hook::Hook::new("e1", vec![("acme".into(), vec!["*".into()])], 100));
    let _ = hook.zgate.set(r.gate.clone());
    let slow = open(&r, "acme").await;
    let qs = slow.declare_queryable(facts_key("acme")).await.expect("slow queryable");
    assert!(wait(3_000, || r.bus.facts_list("acme").len() == 1).await, "형상 — 느린 연결");
    let fast = open(&r, "acme").await;
    let qf = fast.declare_queryable(facts_key("acme")).await.expect("fast queryable");
    assert!(wait(3_000, || r.bus.facts_list("acme").len() == 2).await, "형상 — 연결 둘");
    let (held_tx, mut held_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let _hs = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok(query) = qs.recv_async().await {
            let _ = held_tx.send(());
            held.push(query);
        }
    });
    let (fast_tx, mut fast_rx) = tokio::sync::mpsc::unbounded_channel::<serde_json::Value>();
    let _hf = tokio::spawn(async move {
        while let Ok(query) = qf.recv_async().await {
            let _ = fast_tx.send(body_of(&query));
            let _ = query.reply(query.key_expr().clone(), b"ok".to_vec()).await;
        }
    });
    const WINDOW: usize = 2;
    const N: usize = 8;
    let knobs = oxhubd::hook::carry::Knobs { max_tries: 10, bundle_max_bytes: 1, reply_timeout: Duration::from_secs(20), window: WINDOW, backoff_ms: vec![100] };
    tokio::spawn(oxhubd::hook::carry::run(hook.clone(), r.bus.clone(), "acme".into(), knobs));
    for i in 0..N {
        hook.offer(hook.build("r1", "participant_joined", serde_json::json!({ "user_id": format!("u{i}") }), 1 + i as u64));
    }
    let started = Instant::now();
    let mut fast_got = 0usize;
    while fast_got < N - WINDOW && started.elapsed() < Duration::from_secs(5) {
        if tokio::time::timeout(Duration::from_millis(500), fast_rx.recv()).await.ok().flatten().is_some() {
            fast_got += 1;
        }
    }
    let mut slow_got = 0usize;
    while held_rx.try_recv().is_ok() {
        slow_got += 1;
    }
    assert!(slow_got <= WINDOW, "S-rule-RULE-HOOK-FACTS-SUB-7① — 사실 키의 답 못 받은 query ≤ ext_window: {slow_got}");
    assert_eq!(fast_got + slow_got, N, "정:RULE-BUS-PUBLIC zenoh 넘기기 — 느린 연결이 창을 채워도 건강한 연결로 나머지가 간다(끝을 기다리지 않는다) · fast {fast_got} · slow {slow_got}");
    assert!(started.elapsed() < Duration::from_secs(5), "정:RULE-BUS-PUBLIC zenoh 넘기기 — 느린 연결의 시한(20s)을 기다리지 않는다 {:?}", started.elapsed());
    drop(slow);
    drop(fast);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_webhook_accounts_facts_key_is_ignored_and_counted() {
    let r = rig_accounts(knobs(), true, &["acme"]).await;
    let s = open(&r, "acme").await;
    assert!(wait(3_000, || r.gate.conns().len() == 1).await, "형상 — 붙었다");
    let refused0 = r.gate.drops.refused.load(Ordering::Relaxed);
    let _q = s.declare_queryable(facts_key("acme")).await.expect("facts queryable");
    assert!(wait(3_000, || r.gate.drops.refused.load(Ordering::Relaxed) > refused0).await, "S-rule-RULE-HOOK-FACTS-SUB-3③ — webhook 이 붙은 계정의 사실 구독 선언은 무시하고 ext_refused 를 센다");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(r.bus.facts_list("acme").is_empty(), "S-rule-RULE-HOOK-FACTS-SUB-3③ — 토큰을 세우지 않는다(계정마다 운반 하나)");
    assert_eq!(r.gate.conns().len(), 1, "S-rule-RULE-HOOK-FACTS-SUB-3③ — 세션은 유지");
    drop(s);
}

fn holding(q: zenoh::query::Queryable<zenoh::handlers::FifoChannelHandler<zenoh::query::Query>>, err_first: usize) -> (tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>, tokio::task::JoinHandle<()>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let h = tokio::spawn(async move {
        let mut held = Vec::new();
        let mut n = 0usize;
        while let Ok(query) = q.recv_async().await {
            let _ = tx.send(body_of(&query));
            n += 1;
            if n <= err_first {
                let _ = query.reply_err(b"no".to_vec()).await;
            } else {
                held.push(query);
            }
        }
    });
    (rx, h)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_due_bundle_with_every_window_full_does_not_spin() {
    let mut gk = knobs();
    gk.reply_timeout = Duration::from_secs(20);
    let r = rig_with(gk).await;
    let hook = Arc::new(oxhubd::hook::Hook::new("e1", vec![("acme".into(), vec!["*".into()])], 100));
    let _ = hook.zgate.set(r.gate.clone());
    let a = open(&r, "acme").await;
    let (mut ra, _ha) = holding(a.declare_queryable(facts_key("acme")).await.expect("a"), 0);
    assert!(wait(3_000, || r.bus.facts_list("acme").len() == 1).await, "형상 — 연결 A");
    let b = open(&r, "acme").await;
    let (mut rb, _hb) = holding(b.declare_queryable(facts_key("acme")).await.expect("b"), 1);
    assert!(wait(3_000, || r.bus.facts_list("acme").len() == 2).await, "형상 — 연결 B");
    let knobs = oxhubd::hook::carry::Knobs { max_tries: 10, bundle_max_bytes: 1, reply_timeout: Duration::from_secs(20), window: 1, backoff_ms: vec![50] };
    tokio::spawn(oxhubd::hook::carry::run(hook.clone(), r.bus.clone(), "acme".into(), knobs));
    for i in 0..3 {
        hook.offer(hook.build("r1", "participant_joined", serde_json::json!({ "user_id": format!("u{i}") }), 1 + i as u64));
    }
    let mut seen = 0usize;
    let end = Instant::now() + Duration::from_secs(2);
    while seen < 3 && Instant::now() < end {
        tokio::select! {
            Some(_) = ra.recv() => seen += 1,
            Some(_) = rb.recv() => seen += 1,
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    assert_eq!(seen, 3, "형상 — A 하나 · B 거절 하나 · B 하나 = 두 연결 창이 다 찼고 다시 보낼 묶음 하나가 기다린다");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let before = hook.carry_turns.load(Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let spun = hook.carry_turns.load(Ordering::Relaxed) - before;
    assert!(spun <= 5, "정:RULE-BUS-PUBLIC zenoh 넘기기 · 원천 HOOK.model.NextWait — 다시 보낼 묶음의 때가 지났어도 넣을 자리가 없으면 사건으로만 깬다(1초 동안 깸 {spun}번)");
    drop(a);
    drop(b);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bundle_waits_quietly_while_no_connection_and_goes_when_one_comes_back() {
    let mut gk = knobs();
    gk.reply_timeout = Duration::from_secs(20);
    let r = rig_with(gk).await;
    let hook = Arc::new(oxhubd::hook::Hook::new("e1", vec![("acme".into(), vec!["*".into()])], 100));
    let _ = hook.zgate.set(r.gate.clone());
    let a = open(&r, "acme").await;
    let qa = a.declare_queryable(facts_key("acme")).await.expect("a");
    assert!(wait(3_000, || r.bus.facts_list("acme").len() == 1).await, "형상 — 연결 A");
    let (mut ra, ha) = holding(qa, 1);
    let knobs = oxhubd::hook::carry::Knobs { max_tries: 10, bundle_max_bytes: 65_536, reply_timeout: Duration::from_secs(20), window: 1, backoff_ms: vec![300] };
    tokio::spawn(oxhubd::hook::carry::run(hook.clone(), r.bus.clone(), "acme".into(), knobs));
    hook.offer(hook.build("r1", "room_created", serde_json::json!({ "room_id": "r1" }), 1));
    let first = tokio::time::timeout(Duration::from_secs(3), ra.recv()).await.expect("first in time").expect("first");
    ha.abort();
    drop(a);
    assert!(wait(5_000, || r.bus.facts_list("acme").is_empty()).await, "형상 — 연결이 다 빠졌다(묶음은 다시 보낼 줄에)");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let before = hook.carry_turns.load(Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let spun = hook.carry_turns.load(Ordering::Relaxed) - before;
    assert!(spun <= 5, "정:RULE-BUS-PUBLIC zenoh 넘기기 · 원천 HOOK.model.NextWait — 연결이 없으면 연결이 생길 때까지 잔다(1초 동안 깸 {spun}번)");
    let c = open(&r, "acme").await;
    let qc = c.declare_queryable(facts_key("acme")).await.expect("c");
    let got = tokio::time::timeout(Duration::from_secs(5), qc.recv_async()).await.expect("bundle after reconnect").expect("query");
    let body = body_of(&got);
    let _ = got.reply(got.key_expr().clone(), b"ok".to_vec()).await;
    assert_eq!(body["facts"], first["facts"], "정:RULE-BUS-PUBLIC zenoh 넘기기 — 연결이 다시 붙으면 기다리던 그 묶음이 간다");
    drop(c);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_full_chat_window_does_not_hold_back_facts() {
    let r = rig().await;
    let s = open(&r, "acme").await;
    let qc = s.declare_queryable(sub_key("acme")).await.expect("chat queryable");
    let qf = s.declare_queryable(facts_key("acme")).await.expect("facts queryable");
    let cid = open_cid(&r).await;
    assert!(wait(3_000, || r.gate.lane_open(&cid, Lane::Chat) && r.gate.lane_open(&cid, Lane::Facts)).await, "형상 — 한 연결에 두 키");
    let (chat_tx, mut chat_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let _hc = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok(query) = qc.recv_async().await {
            let _ = chat_tx.send(());
            held.push(query);
        }
    });
    let (facts_tx, mut facts_rx) = tokio::sync::mpsc::unbounded_channel::<serde_json::Value>();
    let _hf = tokio::spawn(async move {
        while let Ok(query) = qf.recv_async().await {
            let _ = facts_tx.send(body_of(&query));
            let _ = query.reply(query.key_expr().clone(), b"ok".to_vec()).await;
        }
    });
    let window = knobs().window;
    for i in 0..window + 3 {
        post(&r, 100 + i);
        let msg = r.chat.store.history("u1", "S", oxhubd::chat::store::Dir::Latest, 1).expect("history").messages.remove(0);
        let _ = r.gate.deliver(&cid, true, vec![msg], None);
    }
    let mut chat_seen = 0;
    while tokio::time::timeout(Duration::from_millis(300), chat_rx.recv()).await.ok().flatten().is_some() {
        chat_seen += 1;
    }
    assert_eq!(chat_seen, window, "형상 — 채팅 키 창이 찼다(답 못 받은 query = ext_window)");
    let started = Instant::now();
    let (tx, rx) = tokio::sync::oneshot::channel();
    assert!(r.gate.deliver_facts(&cid, &bundle("floor1"), Some(tx)).expect("deliver").accepted());
    let got = tokio::time::timeout(Duration::from_secs(2), facts_rx.recv()).await.expect("facts not held back").expect("facts");
    assert_eq!(got["facts"][0]["fact_id"], "floor1");
    assert_eq!(tokio::time::timeout(Duration::from_secs(2), rx).await.expect("ended").expect("outcome"), oxhubd::chat::ext::plan::Outcome::Ack);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "S-rule-RULE-HOOK-FACTS-SUB-7② · S-sym-RULE-HOOK-FACTS-SUB-1 — 채팅 키 창이 차도 사실은 그 뒤에 묶이지 않는다(키마다 창) {:?}",
        started.elapsed()
    );
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn facts_timeouts_in_a_row_close_the_session() {
    let r = rig().await;
    let close_at = knobs().close_at;
    let s = open(&r, "acme").await;
    let qf = s.declare_queryable(facts_key("acme")).await.expect("facts queryable");
    let cid = open_cid(&r).await;
    let _h = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok(query) = qf.recv_async().await {
            held.push(query);
        }
    });
    let timeouts0 = r.gate.drops.reply_timeout.load(Ordering::Relaxed);
    for i in 0..close_at {
        assert!(r.gate.deliver_facts(&cid, &bundle(&format!("t{i}")), None).expect("deliver").accepted(), "형상 — 사실 키 줄에 섰다");
    }
    assert!(
        wait(6_000, || !r.gate.conns().iter().any(|c| c.cid == cid)).await,
        "S-rule-RULE-HOOK-FACTS-SUB-7③ — 사실 키의 잇단 시한 넘김 ext_timeout_close 번에 그 세션이 닫힌다(클라이언트는 새 연결로 다시 붙는다) · timeouts {} · conns {:?}",
        r.gate.drops.reply_timeout.load(Ordering::Relaxed) - timeouts0,
        r.gate.conns()
    );
    assert!(r.gate.drops.reply_timeout.load(Ordering::Relaxed) >= timeouts0 + u64::from(close_at), "S-rule-RULE-HOOK-FACTS-SUB-7③ — 시한 넘김을 센다(ext_reply_timeout)");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_accounts_facts_key_closes_the_session() {
    let r = rig().await;
    let s = open(&r, "acme").await;
    assert!(wait(3_000, || r.gate.conns().len() == 1).await, "형상 — 붙었다");
    let _q = s.declare_queryable(facts_key("other")).await;
    assert!(wait(5_000, || r.gate.conns().is_empty()).await, "S-rule-RULE-HOOK-FACTS-SUB-3② — 인증서 계정과 다른 facts/sub 키를 선언하면 세션이 닫힌다");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn open_keys_answer_one_response_each() {
    let r = rig().await;
    let s = open(&r, "acme").await;
    assert!(wait(3_000, || r.gate.conns().len() == 1).await, "형상 — 붙었다");
    let list = get_one(&s, format!("ox/{NS}/pub/v1/q/rooms"), None).await.expect("one reply").expect("list ok");
    let l: oxsig::msg::ListRoomsRes = serde_json::from_slice(&list).expect("LIST_ROOMS body");
    assert_eq!(l.total, 0, "HK-KEY-PUB-V1-Q-ROOMS② — q/rooms 에 방 목록 Response 하나(몸은 LIST_ROOMS)");
    let none = get_one(&s, format!("ox/{NS}/pub/v1/q/room/nowhere"), None).await.expect("one reply").expect_err("no room");
    assert_eq!(failure_code(&none), oxsig::Code::RoomNotFound.num(), "S-rule-RULE-HOOK-STATUS-5② — 없는 방은 reply_err 하나 · 몸 Failure ROOM_NOT_FOUND");
    let ctl = format!("ox/{NS}/pub/v1/ctl/room/nowhere/permission");
    let bad = get_one(&s, ctl.clone(), Some(b"{}".to_vec())).await.expect("one reply").expect_err("bad payload");
    assert_eq!(failure_code(&bad), oxsig::Code::InvalidPayload.num(), "정:RULE-HOOK-PERMISSION zenoh 입구의 꼴 — payload {{user_id, permission}} 을 못 읽으면 INVALID_PAYLOAD");
    let body = serde_json::to_vec(&serde_json::json!({ "user_id": "u1", "permission": oxsig::msg::Permission::ALL })).expect("encode");
    let gone = get_one(&s, ctl, Some(body)).await.expect("one reply").expect_err("no room");
    assert_eq!(failure_code(&gone), oxsig::Code::RoomNotFound.num(), "S-rule-RULE-HOOK-PERMISSION-15② — 실패는 reply_err 하나 · 없는 방 ROOM_NOT_FOUND");
    assert!(get_one(&s, format!("ox/{NS}/pub/v1/q/elsewhere"), None).await.is_none(), "정:RULE-CHAT-EXT-SUB 7 — 열린 키 밖의 get 은 답 없이 끝");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hub_without_chat_ignores_chat_sub_and_still_opens_facts() {
    let r = rig_full(knobs(), false).await;
    let s = open(&r, "acme").await;
    assert!(wait(3_000, || r.gate.conns().len() == 1).await, "정:RULE-HOOK-DIRECT — [chat] 없이 입구가 선다");
    let refused0 = r.gate.drops.refused.load(Ordering::Relaxed);
    let _chat = s.declare_queryable(sub_key("acme")).await.expect("chat queryable");
    assert!(wait(3_000, || r.gate.drops.refused.load(Ordering::Relaxed) > refused0).await, "정:RULE-CHAT-EXT-SUB 7 — [chat] 없는 hub 의 chat/sub 선언은 허용 키 밖처럼 무시하고 ext_refused 를 센다");
    assert_eq!(r.gate.conns().len(), 1, "정:RULE-CHAT-EXT-SUB 7 — 세션은 닫지 않는다");
    assert!(tokens(&r).await.is_empty(), "형상 — 채팅 토큰이 서지 않는다");
    let _facts = s.declare_queryable(facts_key("acme")).await.expect("facts queryable");
    assert!(wait(3_000, || r.bus.facts_list("acme").len() == 1).await, "S-rule-RULE-HOOK-DIRECT-2② — 사실 구독은 [chat] 없이 연다");
    drop(s);
}
