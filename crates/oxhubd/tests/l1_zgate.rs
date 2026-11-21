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
use oxhubd::chat::zgate::{Gate, GateCfg, link, wire};
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
    _events: tokio::sync::mpsc::UnboundedReceiver<oxhubd::bus::BusEvent>,
}

async fn rig() -> Rig {
    let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_test_writer().try_init();
    let pki = Pki::new();
    let (bus, events) = Bus::open("n1", "e1", 1_000, BusCfg { listen: vec!["127.0.0.1:0".into()], advertise: vec![], connect: vec![], namespace: NS.into() }).await.expect("bus");
    let accounts = Accounts::new(vec![
        ("acme".into(), Standing::Ok),
        ("other".into(), Standing::Ok),
        ("nohook".into(), Standing::NoHook),
        ("nosub".into(), Standing::NoSubscribe),
    ]);
    let cfg = GateCfg { listen: "tls/127.0.0.1:0".into(), cert: pki.path("hub.pem"), key: pki.path("hub.key"), ca: pki.path("ca.pem") };
    let gate = Gate::bind(&cfg, accounts, bus.clone()).await.expect("gate");
    tokio::spawn(gate.clone().serve());
    Rig { pki, bus, gate, _events: events }
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
    assert!(cid.starts_with("e1"), "정:RULE-CHAT-EXT-SUB 3 — cid = 기동 신원 + 세션 번호 {cid}");
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
