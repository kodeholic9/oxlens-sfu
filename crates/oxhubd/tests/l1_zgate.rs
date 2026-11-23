// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-BUS-KEYSPACE · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! 1층 상호운용 — zenoh 입구(zgate)에 실제 zenoh 1.10.1 클라이언트(`zenoh = "=1.10.1"`)로 붙는다: queryable 선언 → 버스 토큰 · 질의 → reply = ACK · reply_err = 거절 · 답 없이 놓음 = 답 없음 · 512 KiB 질의 한 건(Fragment) · 지금 Interest 에 DeclareFinal(liveliness get 이 시한 전에 끝) · 3rd 의 put 버림 · 3rd 의 get 은 결과 없이 끝 · 허용 키 밖 선언 무시 · 다른 계정 키 선언 = 세션 닫힘 · 거절(인증서 없음 · 다른 CA · 모르는 계정 · 연동 허용 없음 · 구독 없음 · 평문 TCP) · Close 와 lease 넘김에 토큰이 걷힌다 · 판 · client 아님 거절은 우리 코덱으로 짠 날 클라이언트로.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use oxhubd::bus::{Bus, BusCfg};
use oxhubd::chat::zgate::judge::{Accounts, Standing};
use oxhubd::chat::zgate::session::Answer;
use oxhubd::chat::zgate::send::{GateDeps, GateKnobs};
use oxhubd::chat::zgate::{Gate, GateCfg, link, wire};
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

fn knobs() -> GateKnobs {
    GateKnobs {
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

async fn rig_with(k: GateKnobs) -> Rig {
    let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_test_writer().try_init();
    let pki = Pki::new();
    let (bus, events) = Bus::open("n1", "e1", 1_000, BusCfg { listen: vec!["127.0.0.1:0".into()], advertise: vec![], connect: vec![], namespace: NS.into() }).await.expect("bus");
    bus.follow_ext().await.expect("follow ext");
    let accounts = Accounts::new(vec![
        ("acme".into(), Standing::Ok),
        ("other".into(), Standing::Ok),
        ("nohook".into(), Standing::NoHook),
        ("nosub".into(), Standing::NoSubscribe),
    ]);
    let subs = ExtSubs::new(vec![("acme".into(), Subscribe::try_from(vec![String::new()]).expect("subscribe"))]);
    let store = Store::open("memory:", Visibility::Shared).expect("store").subscribed(subs);
    let chat = Arc::new(Chat::new(store, Limits::default()));
    chat.store.create_space("S", None).expect("space");
    chat.store.join("S", "u1", None, Some(1)).expect("join");
    let cfg = GateCfg { listen: "tls/127.0.0.1:0".into(), cert: pki.path("hub.pem"), key: pki.path("hub.key"), ca: pki.path("ca.pem") };
    let deps = GateDeps { accounts, bus: bus.clone(), store: chat.store.clone(), acks: chat.acks(), keys: vec!["acme".into()], knobs: k };
    let gate = Gate::bind(&cfg, deps).await.expect("gate");
    tokio::spawn(gate.clone().serve());
    tokio::spawn(gate.clone().drains());
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
    let at = oxhubd::chat::zgate::plan::opened(&cid);
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
        let p = r.gate.query(&cid, body).expect("query");
        answers.push(tokio::time::timeout(Duration::from_secs(10), p.answer()).await.expect("answer in time"));
    }
    let (_q, seen) = tokio::time::timeout(Duration::from_secs(10), replier).await.expect("replier in time").expect("replier");
    assert_eq!(answers[0], Answer::Ack, "H-rule-7-3-7③ — query 키로 reply = ACK");
    assert_eq!(answers[1], Answer::Refused(b"no".to_vec()), "H-rule-7-3-7④ — reply_err = 거절(ACK 아님)");
    assert_eq!(answers[2], Answer::NoReply, "정:RULE-CHAT-EXT-SUB 4 — 답 없이 놓으면 답 없음(ACK 아님)");
    assert_eq!(answers[3], Answer::Ack, "형상 — 큰 질의도 ACK");
    assert_eq!(seen[0], b"one".to_vec(), "형상 — 묶음 몸이 그대로 간다");
    assert_eq!(seen[3].len(), big.len(), "H-rule-7-3-7② — 512 KiB 질의 한 건이 조각으로 나뉘어 한 건으로 온다");
    assert!(seen[3] == big, "H-rule-7-3-7② — 이은 몸이 같다");
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
    let a = tokio::time::timeout(Duration::from_secs(10), r.gate.query(&cid, b"x".to_vec()).expect("query").answer()).await.expect("answer");
    let (_q, blocked) = tokio::time::timeout(Duration::from_secs(10), replier).await.expect("replier in time").expect("replier");
    assert!(blocked, "H-sym-7-3-2 — 클라이언트가 다른 키의 답을 막는다");
    assert_eq!(a, Answer::NoReply, "H-sym-7-3-2 — ACK 0 · 그 묶음은 다시 온다(답 없음)");
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
    assert!(wait(3_000, || r.gate.drops.refused.load(Ordering::Relaxed) > refused0).await, "H-rule-7-3-12② — 허용 키 밖 선언은 무시하고 센다");
    s.put(format!("ox/{NS}/pub/v1/x"), b"v".to_vec()).await.expect("put");
    assert!(wait(3_000, || r.gate.drops.push_dropped.load(Ordering::Relaxed) >= 1).await, "H-rule-7-3-12① — 3rd 의 put 은 버리고 센다");
    let started = Instant::now();
    let replies = s.get(format!("ox/{NS}/pub/v1/q")).await.expect("get");
    let mut n = 0;
    while replies.recv_async().await.is_ok() {
        n += 1;
    }
    assert_eq!(n, 0, "H-rule-7-3-12③ — 3rd 의 질의에는 결과가 없다");
    assert!(started.elapsed() < Duration::from_secs(5), "H-rule-7-3-12③ — 결과 없이 바로 끝(시한을 기다리지 않는다) {:?}", started.elapsed());
    let started = Instant::now();
    let tokens_got = s.liveliness().get(format!("ox/{NS}/pub/v1/live/**")).await.expect("liveliness get");
    while tokens_got.recv_async().await.is_ok() {}
    assert!(started.elapsed() < Duration::from_secs(5), "정:RULE-CHAT-EXT-SUB 7 — 지금 Interest 에 DeclareFinal 로 답한다(liveliness get 이 시한 전에 끝) {:?}", started.elapsed());
    assert_eq!(r.gate.conns().len(), 1, "H-rule-7-3-12④ — 그동안 세션은 산다");
    drop(q);
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn declaring_another_accounts_key_closes_the_session() {
    let r = rig().await;
    let s = open(&r, "acme").await;
    let refused0 = r.gate.drops.refused.load(Ordering::Relaxed);
    let _q = s.declare_queryable(sub_key("other")).await;
    assert!(wait(5_000, || r.gate.conns().is_empty()).await, "H-rule-7-3-4① — 인증서 계정과 다른 chat/sub 키를 선언하면 세션이 닫힌다");
    assert!(r.gate.drops.refused.load(Ordering::Relaxed) > refused0, "H-rule-7-3-4① — ext_refused +1");
    assert!(tokens(&r).await.is_empty(), "형상 — 토큰이 서지 않았다");
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refused_before_the_session() {
    let r = rig().await;
    let ep = tls_endpoint(&r);
    let tcp = format!("tcp/127.0.0.1:{}", r.gate.addr.port());
    assert!(refused_open(client_cfg(&r, &ep, None)).await && counted(&r, 0).await, "H-rule-3-9① — 인증서 없음은 붙지 못하고 ext_refused +1");
    assert!(refused_open(client_cfg(&r, &ep, Some("rogue"))).await && counted(&r, 1).await, "H-rule-3-9② — 다른 CA 인증서는 붙지 못하고 ext_refused +1");
    assert!(refused_open(client_cfg(&r, &ep, Some("nohook"))).await && counted(&r, 2).await, "H-rule-3-9③ — 연동 허용 없는 계정은 붙지 못하고 ext_refused +1");
    assert!(refused_open(client_cfg(&r, &ep, Some("nosub"))).await && counted(&r, 3).await, "H-rule-3-9④ — 구독 빈 계정은 붙지 못하고 ext_refused +1");
    assert!(refused_open(client_cfg(&r, &ep, Some("stranger"))).await && counted(&r, 4).await, "정:RULE-CHAT-EXT-SUB 7 — 모르는 계정은 붙지 못하고 ext_refused +1");
    assert!(refused_open(client_cfg(&r, &tcp, None)).await && counted(&r, 5).await, "H-rule-7-3-1② — 평문 TCP 는 닫히고 ext_refused +1");
    assert!(r.gate.conns().is_empty(), "형상 — 거절된 것은 연결표에 들지 않는다");
    let s = open(&r, "acme").await;
    assert!(wait(3_000, || r.gate.conns().len() == 1).await, "H-rule-7-3-1① · H-rule-3-9⑤ — tls 로 붙는다");
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
    assert!(matches!(got, Some(TransportBody::Close(_))) && counted(&r, before).await, "H-rule-7-3-2 — 판 0x09 가 아니면 InitAck 대신 Close · ext_refused +1 {got:?}");
    let before = r.gate.drops.refused.load(Ordering::Relaxed);
    let mut raw = Raw::connect(&r, "acme").await;
    raw.send(Raw::init(zenoh_protocol::VERSION, WhatAmI::Peer)).await;
    let got = raw.recv().await.map(|m| m.body);
    assert!(matches!(got, Some(TransportBody::Close(_))) && counted(&r, before).await, "H-rule-7-3-1③ — client 가 아니면 InitAck 대신 Close · ext_refused +1 {got:?}");
    assert!(r.gate.conns().is_empty(), "형상 — 연결표에 들지 않는다");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_with_usrpwd_on_fails_on_its_own_side() {
    let r = rig().await;
    let mut cfg = client_cfg(&r, &tls_endpoint(&r), Some("acme"));
    cfg.insert_json5("transport/auth/usrpwd", r#"{ user: "u", password: "p" }"#).expect("usrpwd");
    assert!(refused_open(cfg).await, "H-rule-7-3-1④ · H-rule-7-3-3④ — usrpwd 를 켠 클라이언트는 붙지 못한다(입구가 auth 확장을 되돌리지 않는다)");
    assert!(wait(3_000, || r.gate.conns().is_empty()).await, "형상 — 연결표에 남지 않는다");
}

#[test]
fn judgments_are_pure_with_reasons() {
    use oxhubd::chat::zgate::judge::{DeclareVerdict, Refuse, judge_account, judge_declare, judge_init};
    use zenoh_protocol::core::Bits;
    use zenoh_protocol::core::resolution::Field;
    let acc = Accounts::new(vec![("acme".into(), Standing::Ok), ("nohook".into(), Standing::NoHook), ("nosub".into(), Standing::NoSubscribe)]);
    assert_eq!(judge_account(Some("acme"), &acc), Ok("acme".to_string()));
    assert_eq!(judge_account(None, &acc), Err(Refuse::NoCn));
    assert_eq!(judge_account(Some("nohook"), &acc).map_err(|r| r.reason()), Err("hook not allowed"));
    assert_eq!(judge_account(Some("nosub"), &acc).map_err(|r| r.reason()), Err("no subscribe"));
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
    let p = format!("ox/{NS}/pub/v1/chat/sub/");
    assert_eq!(judge_declare(&sub_key("acme"), &p, "acme"), DeclareVerdict::Own);
    assert_eq!(judge_declare(&sub_key("other"), &p, "acme"), DeclareVerdict::OtherAccount(sub_key("other")));
    assert_eq!(judge_declare(&sub_key("**"), &p, "acme"), DeclareVerdict::OtherAccount(sub_key("**")), "와일드카드로 남의 몫을 덮는 선언도 닫는다");
    assert_eq!(judge_declare("a/b", &p, "acme"), DeclareVerdict::Outside("a/b".into()));
}

#[test]
fn sequence_numbers_fragments_and_batches() {
    use oxhubd::chat::zgate::wire::{Defrag, Piece, RxSn, SnVerdict, TxSn, pack, parts, room};
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
    let big = vec![0u8; oxhubd::chat::zgate::wire::DEFRAG_MAX + 1];
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
    assert_eq!((b["live"].clone(), ids_of(&b)), (serde_json::json!(true), vec![live]), "H-rule-7-3-6① — 실시간 몸 {{live:true, messages}} · 메시지 꼴 = MESSAGES 의 것");
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
    assert!(wait(8_000, || !r.gate.conns().iter().any(|c| c.cid == cid)).await, "S-sym-RULE-CHAT-EXT-SUB-6① · PL-KEY-HOOK-EXT-REPLY-TIMEOUT-MS② — 잇단 시간 넘김 close_at 번에 그 세션을 닫는다(클라이언트는 새 연결로 다시 붙는다)");
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

fn spec_dir() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/spec")
}

#[test]
fn send_plan_vectors() {
    use oxhubd::chat::zgate::plan::{After, Due, Item, Knobs, Outbox, Outcome, Row, drain_due, drain_owner, drain_pick, ring};
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
            assert_eq!(got, s["got"], "정:RULE-CHAT-EXT-SUB 5 — {} · {s}", c["name"]);
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
