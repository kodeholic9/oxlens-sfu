// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-MESH-DECL · 정:RULE-MESH-LANE · 정:REJ-MESH-ZENOH-FORK · model: claude-opus-5-5
//! 1층 — zenoh 1.10.1 자체 시험의 대조판(202610/20261031e). 시험 이름은 원본 이름을 그대로 따른다 — 원본: `zenoh/tests/liveliness.rs` · `unicity.rs` ·
//! `queryable.rs` · `session.rs` · `io/zenoh-transport/tests/unicast_{fragmentation,priorities,openclose,intermittent,simultaneous}.rs`.
//! 형상 바꿈: zenoh 의 peer 는 우리에게 없어 router 로 둔다 · history 구독의 「일괄 끝」(zenoh 에 없음)은 견줄 때 뺀다 · 원본의 1 s 잠은 짧게.

use std::time::Duration;

use oxmesh::{Config, Mesh, TokenEvent, TokenSub};
use tokio::time::{Instant, sleep, timeout};

const KEY: &str = "test/liveliness/parity";
const SLEEP: Duration = Duration::from_millis(300);
const LEASE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Router,
    Client,
}

async fn router(name: &str, seeds: &[&Mesh]) -> Mesh {
    let mut c = Config::router(name);
    c.listen = vec!["127.0.0.1:0".into()];
    c.connect = seeds.iter().map(|m| m.bound()[0].to_string()).collect();
    c.lease = LEASE;
    Mesh::open(c).await.expect("open router")
}

async fn client(name: &str, r: &Mesh) -> Mesh {
    let mut c = Config::client(name, &r.bound()[0].to_string());
    c.lease = LEASE;
    let m = Mesh::open(c).await.expect("open client");
    let until = Instant::now() + Duration::from_secs(5);
    while m.info().await.map(|i| i.sessions.is_empty()).unwrap_or(true) && Instant::now() < until {
        sleep(Duration::from_millis(10)).await;
    }
    m
}

async fn node(mode: Mode, name: &str, r: &Mesh) -> Mesh {
    match mode {
        Mode::Router => {
            let m = router(name, &[r]).await;
            let until = Instant::now() + Duration::from_secs(5);
            while m.info().await.map(|i| i.sessions.is_empty()).unwrap_or(true) && Instant::now() < until {
                sleep(Duration::from_millis(10)).await;
            }
            m
        }
        Mode::Client => client(name, r).await,
    }
}

async fn drain(s: &mut TokenSub) -> Vec<TokenEvent> {
    let mut v = Vec::new();
    while let Ok(Some(e)) = timeout(SLEEP, s.recv()).await {
        if e != TokenEvent::BatchEnd {
            v.push(e);
        }
    }
    v
}

fn put() -> Vec<TokenEvent> {
    vec![TokenEvent::Put(KEY.into())]
}

fn del() -> Vec<TokenEvent> {
    vec![TokenEvent::Delete(KEY.into())]
}

#[derive(Clone, Copy)]
enum Order {
    Before,
    Middle,
    After,
}

async fn double(mode: Mode, history: bool, order: Order) {
    let r = router("r", &[]).await;
    let t = client("tok", &r).await;
    let s = node(mode, "sub", &r).await;
    let mut token = None;
    if matches!(order, Order::Before) {
        token = Some(t.declare_token(KEY).await.expect("token"));
        sleep(SLEEP).await;
    }
    let mut sub1 = s.token_sub(KEY, history).await.expect("sub1");
    let first = if matches!(order, Order::Before) && history { put() } else { vec![] };
    assert_eq!(drain(&mut sub1).await, first, "sub1 first");
    if matches!(order, Order::Middle) {
        token = Some(t.declare_token(KEY).await.expect("token"));
        sleep(SLEEP).await;
        assert_eq!(drain(&mut sub1).await, put(), "sub1 sees the token");
    }
    let mut sub2 = s.token_sub(KEY, history).await.expect("sub2");
    let second = if !matches!(order, Order::After) && history { put() } else { vec![] };
    assert_eq!(drain(&mut sub2).await, second, "sub2 first");
    if matches!(order, Order::After) {
        token = Some(t.declare_token(KEY).await.expect("token"));
        sleep(SLEEP).await;
        assert_eq!(drain(&mut sub1).await, put(), "sub1 sees the token");
        assert_eq!(drain(&mut sub2).await, put(), "sub2 sees the token");
    }
    drop(token);
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub1).await, del(), "sub1 delete");
    assert_eq!(drain(&mut sub2).await, del(), "sub2 delete");
}

macro_rules! double_cases {
    ($($name:ident => $mode:expr, $hist:expr, $order:expr;)*) => {
        $(
            #[tokio::test]
            async fn $name() {
                double($mode, $hist, $order).await;
            }
        )*
    };
}

double_cases! {
    test_liveliness_subscriber_double_client_before => Mode::Client, false, Order::Before;
    test_liveliness_subscriber_double_client_middle => Mode::Client, false, Order::Middle;
    test_liveliness_subscriber_double_client_after => Mode::Client, false, Order::After;
    test_liveliness_subscriber_double_client_history_before => Mode::Client, true, Order::Before;
    test_liveliness_subscriber_double_client_history_middle => Mode::Client, true, Order::Middle;
    test_liveliness_subscriber_double_client_history_after => Mode::Client, true, Order::After;
    test_liveliness_subscriber_double_router_before => Mode::Router, false, Order::Before;
    test_liveliness_subscriber_double_router_middle => Mode::Router, false, Order::Middle;
    test_liveliness_subscriber_double_router_after => Mode::Router, false, Order::After;
    test_liveliness_subscriber_double_router_history_before => Mode::Router, true, Order::Before;
    test_liveliness_subscriber_double_router_history_middle => Mode::Router, true, Order::Middle;
    test_liveliness_subscriber_double_router_history_after => Mode::Router, true, Order::After;
}

async fn subget(mode: Mode, history: bool, before: bool) {
    let r = router("r", &[]).await;
    let t = client("tok", &r).await;
    let s = node(mode, "subget", &r).await;
    let mut token = None;
    if before {
        token = Some(t.declare_token(KEY).await.expect("token"));
        sleep(SLEEP).await;
    }
    let mut sub = s.token_sub(KEY, history).await.expect("sub");
    assert_eq!(drain(&mut sub).await, if before && history { put() } else { vec![] });
    if !before {
        token = Some(t.declare_token(KEY).await.expect("token"));
        sleep(SLEEP).await;
        assert_eq!(drain(&mut sub).await, put());
    }
    assert_eq!(s.tokens(KEY, SLEEP).await.expect("get"), vec![KEY.to_string()]);
    drop(token);
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, del());
    assert_eq!(s.tokens(KEY, SLEEP).await.expect("get"), Vec::<String>::new());
}

macro_rules! subget_cases {
    ($($name:ident => $mode:expr, $hist:expr, $before:expr;)*) => {
        $(
            #[tokio::test]
            async fn $name() {
                subget($mode, $hist, $before).await;
            }
        )*
    };
}

subget_cases! {
    test_liveliness_subget_client_before => Mode::Client, false, true;
    test_liveliness_subget_client_middle => Mode::Client, false, false;
    test_liveliness_subget_client_history_before => Mode::Client, true, true;
    test_liveliness_subget_client_history_middle => Mode::Client, true, false;
    test_liveliness_subget_router_before => Mode::Router, false, true;
    test_liveliness_subget_router_middle => Mode::Router, false, false;
    test_liveliness_subget_router_history_before => Mode::Router, true, true;
    test_liveliness_subget_router_history_middle => Mode::Router, true, false;
}

#[tokio::test]
async fn test_liveliness_subscriber_brokered() {
    let r = router("r", &[]).await;
    let c1 = client("c1", &r).await;
    let c2 = client("c2", &r).await;
    let mut sub = c1.token_sub(KEY, false).await.expect("sub");
    sleep(SLEEP).await;
    let token = c2.declare_token(KEY).await.expect("token");
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, put());
    drop(token);
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, del());
}

#[tokio::test]
async fn test_liveliness_query_brokered() {
    let r = router("r", &[]).await;
    let c1 = client("c1", &r).await;
    let c2 = client("c2", &r).await;
    let _token = c1.declare_token(KEY).await.expect("token");
    sleep(SLEEP).await;
    assert_eq!(c2.tokens(KEY, SLEEP).await.expect("get"), vec![KEY.to_string()]);
}

#[tokio::test]
async fn test_liveliness_subscriber_local() {
    for mode in [Mode::Router, Mode::Client] {
        let r = router("r", &[]).await;
        let m = if mode == Mode::Router { r.clone() } else { client("c", &r).await };
        let mut sub = m.token_sub(KEY, false).await.expect("sub");
        let token = m.declare_token(KEY).await.expect("token");
        sleep(SLEEP).await;
        assert_eq!(drain(&mut sub).await, put());
        drop(token);
        sleep(SLEEP).await;
        assert_eq!(drain(&mut sub).await, del());
    }
}

#[tokio::test]
async fn test_liveliness_query_local() {
    for mode in [Mode::Router, Mode::Client] {
        let r = router("r", &[]).await;
        let m = if mode == Mode::Router { r.clone() } else { client("c", &r).await };
        let token = m.declare_token(KEY).await.expect("token");
        sleep(SLEEP).await;
        assert_eq!(m.tokens(KEY, SLEEP).await.expect("get"), vec![KEY.to_string()]);
        drop(token);
        sleep(SLEEP).await;
        assert_eq!(m.tokens(KEY, SLEEP).await.expect("get"), Vec::<String>::new());
    }
}

#[tokio::test]
async fn test_liveliness_after_close() {
    let r = router("r", &[]).await;
    let a = router("a", &[&r]).await;
    let b = client("b", &r).await;
    let mut sub = a.token_sub(KEY, false).await.expect("sub");
    sleep(SLEEP).await;
    let _token = b.declare_token(KEY).await.expect("token");
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, put());
    a.close().await;
    assert_eq!(timeout(SLEEP, sub.recv()).await.ok().flatten(), None, "closed session ends the subscriber");
}

#[tokio::test]
async fn test_liveliness_regression_1() {
    let r = router("r", &[]).await;
    let t = router("tok", &[&r]).await;
    let token = t.declare_token(KEY).await.expect("token");
    sleep(SLEEP).await;
    let s = router("sub", &[&r, &t]).await;
    sleep(SLEEP).await;
    let mut sub = s.token_sub(KEY, false).await.expect("sub");
    assert_eq!(drain(&mut sub).await, vec![]);
    drop(token);
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, del(), "a pre-existing token's Delete reaches a subscriber without history");
}

#[tokio::test]
async fn test_liveliness_regression_2() {
    let r = router("r", &[]).await;
    let t1 = router("tok1", &[&r]).await;
    let token1 = t1.declare_token(KEY).await.expect("token");
    sleep(SLEEP).await;
    let s = router("sub", &[&r]).await;
    sleep(SLEEP).await;
    let mut sub = s.token_sub(KEY, false).await.expect("sub");
    assert_eq!(drain(&mut sub).await, vec![]);
    let t2 = router("tok2", &[&r]).await;
    let token2 = t2.declare_token(KEY).await.expect("token");
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, vec![], "second declaration of the same key is not a new Put");
    drop(token1);
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, vec![], "still held by tok2");
    drop(token2);
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, del());
}

#[tokio::test]
async fn test_liveliness_regression_2_history() {
    let r = router("r", &[]).await;
    let t1 = router("tok1", &[&r]).await;
    let token1 = t1.declare_token(KEY).await.expect("token");
    sleep(SLEEP).await;
    let s = router("sub", &[&r]).await;
    sleep(SLEEP).await;
    let mut sub = s.token_sub(KEY, true).await.expect("sub");
    assert_eq!(drain(&mut sub).await, put());
    let t2 = client("tok2", &r).await;
    let token2 = t2.declare_token(KEY).await.expect("token");
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, vec![]);
    drop(token1);
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, vec![]);
    drop(token2);
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, del());
}

#[tokio::test]
async fn test_liveliness_regression_3() {
    let r = router("r", &[]).await;
    let t1 = router("tok1", &[&r]).await;
    let token1 = t1.declare_token(KEY).await.expect("token");
    sleep(SLEEP).await;
    let t2 = client("tok2", &r).await;
    let token2 = t2.declare_token(KEY).await.expect("token");
    sleep(SLEEP).await;
    let s = router("sub", &[&r]).await;
    sleep(SLEEP).await;
    let mut sub = s.token_sub(KEY, false).await.expect("sub");
    assert_eq!(drain(&mut sub).await, vec![]);
    drop(token1);
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, vec![]);
    drop(token2);
    sleep(SLEEP).await;
    assert_eq!(drain(&mut sub).await, del());
}

#[tokio::test]
async fn test_liveliness_issue_1470() {
    let r0 = router("r0", &[]).await;
    let _t0 = r0.declare_token("test/liveliness/1470/r0").await.expect("token");
    let r1 = router("r1", &[&r0]).await;
    let _t1 = r1.declare_token("test/liveliness/1470/r1").await.expect("token");
    let p = router("p", &[&r0, &r1]).await;
    let _tp = p.declare_token("test/liveliness/1470/p").await.expect("token");
    sleep(SLEEP).await;
    let all = vec!["test/liveliness/1470/p".to_string(), "test/liveliness/1470/r0".to_string(), "test/liveliness/1470/r1".to_string()];
    for at in [&r0, &p] {
        let c = client("c", at).await;
        let mut sub = c.token_sub("test/liveliness/1470/**", true).await.expect("sub");
        let mut got: Vec<String> = drain(&mut sub)
            .await
            .into_iter()
            .filter_map(|e| match e {
                TokenEvent::Put(k) => Some(k),
                _ => None,
            })
            .collect();
        got.sort();
        assert_eq!(got, all, "client on {} sees all three", at.node());
        c.close().await;
    }
}

#[tokio::test]
async fn test_liveliness_sub_history_conflict() {
    let r = router("r", &[]).await;
    let t = router("tok", &[&r]).await;
    let token = t.declare_token(KEY).await.expect("token");
    sleep(SLEEP).await;
    let c = client("subs", &r).await;
    let mut no_hist = c.token_sub(KEY, false).await.expect("sub");
    assert_eq!(drain(&mut no_hist).await, vec![]);
    let mut hist = c.token_sub(KEY, true).await.expect("sub");
    assert_eq!(drain(&mut hist).await, put());
    assert_eq!(drain(&mut no_hist).await, vec![], "history of one subscriber does not leak to the other");
    drop(token);
    sleep(SLEEP).await;
    assert_eq!(drain(&mut hist).await, del());
    assert_eq!(drain(&mut no_hist).await, del());
}

#[tokio::test]
async fn zenoh_unicity_brokered() {
    let r = router("r", &[]).await;
    let pub_c = client("pub", &r).await;
    let mut subs = Vec::new();
    for i in 0..3 {
        let c = client(&format!("sub{i}"), &r).await;
        let s = c.subscribe("test/unicity").await.expect("sub");
        subs.push((c, s));
    }
    sleep(SLEEP).await;
    for i in 0..100u8 {
        pub_c.put("test/unicity", vec![i]).expect("put");
    }
    for (_, s) in subs.iter_mut() {
        for i in 0..100u8 {
            assert_eq!(timeout(LEASE, s.recv()).await.ok().flatten().map(|x| x.payload), Some(vec![i]));
        }
        assert!(timeout(SLEEP, s.recv()).await.is_err(), "exactly once");
    }
}

#[tokio::test]
async fn test_queryable_same_session_and_different_sessions() {
    let r = router("r", &[]).await;
    let c = client("c", &r).await;
    for (owner, asker) in [(&r, &r), (&c, &c), (&r, &c), (&c, &r)] {
        let mut q = owner.queryable("test/queryable/**").await.expect("qabl");
        let h = tokio::spawn(async move {
            if let Some(query) = q.recv().await {
                query.reply("test/queryable/a", query.payload().to_vec());
                query.reply("test/queryable/b", b"2".to_vec());
            }
            q
        });
        sleep(SLEEP).await;
        let mut rx = asker.get("test/queryable/x", b"1".to_vec(), LEASE).expect("get");
        let mut got = Vec::new();
        while let Some(rep) = timeout(LEASE, rx.recv()).await.ok().flatten() {
            got.push((rep.key, rep.result));
        }
        assert_eq!(got, vec![("test/queryable/a".to_string(), Ok(b"1".to_vec())), ("test/queryable/b".to_string(), Ok(b"2".to_vec()))]);
        drop(h.await);
    }
}

#[tokio::test]
async fn test_undeclare_subscribers_same_keyexpr() {
    let r = router("r", &[]).await;
    let c = client("c", &r).await;
    let s1 = r.subscribe("test/undeclare").await.expect("sub");
    let mut s2 = r.subscribe("test/undeclare").await.expect("sub");
    sleep(SLEEP).await;
    drop(s1);
    sleep(SLEEP).await;
    c.put("test/undeclare", b"x".to_vec()).expect("put");
    assert_eq!(timeout(LEASE, s2.recv()).await.ok().flatten().map(|x| x.payload), Some(b"x".to_vec()));
}

#[tokio::test]
async fn fragmentation_unicast_tcp_only() {
    let r = router("r", &[]).await;
    let a = router("a", &[&r]).await;
    let mut s = a.subscribe("test/frag").await.expect("sub");
    let mut q = a.queryable("test/frag/q").await.expect("qabl");
    sleep(SLEEP).await;
    let big: Vec<u8> = (0..4 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    r.put("test/frag", big.clone()).expect("put");
    assert_eq!(timeout(Duration::from_secs(5), s.recv()).await.ok().flatten().map(|x| x.payload), Some(big.clone()));
    let b2 = big.clone();
    tokio::spawn(async move {
        if let Some(query) = q.recv().await {
            query.reply("test/frag/q", b2.clone());
        }
    });
    let mut rx = r.get("test/frag/q", vec![], Duration::from_secs(5)).expect("get");
    assert_eq!(timeout(Duration::from_secs(5), rx.recv()).await.ok().flatten().map(|x| x.result), Some(Ok(big)));
}

#[tokio::test]
async fn priorities_tcp_only() {
    let r = router("r", &[]).await;
    let a = router("a", &[&r]).await;
    let mut s = a.subscribe("test/prio/data").await.expect("sub");
    let mut tsub = a.token_sub("test/prio/tok", false).await.expect("tsub");
    sleep(SLEEP).await;
    let n = 1500usize;
    for i in 0..n {
        r.put("test/prio/data", vec![(i % 251) as u8; 32 * 1024]).expect("put");
    }
    let _token = r.declare_token("test/prio/tok").await.expect("token");
    let seen_at = loop {
        match timeout(Duration::from_secs(5), tsub.recv()).await.ok().flatten() {
            Some(TokenEvent::Put(_)) => break Instant::now(),
            Some(_) => continue,
            None => panic!("token never seen"),
        }
    };
    let mut got = 0usize;
    let mut last = Instant::now();
    while got < n {
        if timeout(Duration::from_secs(5), s.recv()).await.ok().flatten().is_none() {
            break;
        }
        got += 1;
        last = Instant::now();
    }
    assert_eq!(got, n, "data lane delivered everything");
    assert!(seen_at < last, "the token declared after {n} big puts overtook them (declare lane above data lane)");
}

#[tokio::test]
async fn openclose_tcp_only() {
    let r = router("r", &[]).await;
    let mut tsub = r.token_sub("test/openclose/*", false).await.expect("tsub");
    for i in 0..30 {
        let c = client("oc", &r).await;
        let _t = c.declare_token(&format!("test/openclose/{i}")).await.expect("token");
        sleep(Duration::from_millis(30)).await;
        c.close().await;
    }
    sleep(SLEEP).await;
    let ev = drain(&mut tsub).await;
    assert_eq!(ev.iter().filter(|e| matches!(e, TokenEvent::Put(_))).count(), 30);
    assert_eq!(ev.iter().filter(|e| matches!(e, TokenEvent::Delete(_))).count(), 30);
    assert_eq!(r.info().await.expect("info").sessions.len(), 0, "every client session cleaned");
    assert_eq!(r.tokens("test/openclose/*", SLEEP).await.expect("get"), Vec::<String>::new());
}

#[tokio::test]
async fn transport_tcp_intermittent() {
    let r = router("r", &[]).await;
    let a = router("a", &[&r]).await;
    let mut s = a.subscribe("test/intermittent").await.expect("sub");
    sleep(SLEEP).await;
    let r2 = r.clone();
    let flap = tokio::spawn(async move {
        for i in 0..20 {
            let c = client(&format!("flap{}", i % 3), &r2).await;
            let _t = c.declare_token("test/intermittent/flap").await;
            sleep(Duration::from_millis(20)).await;
            c.close().await;
        }
    });
    let n = 2000u32;
    for i in 0..n {
        r.put("test/intermittent", i.to_be_bytes().to_vec()).expect("put");
        if i % 100 == 0 {
            sleep(Duration::from_millis(10)).await;
        }
    }
    for i in 0..n {
        assert_eq!(timeout(LEASE, s.recv()).await.ok().flatten().map(|x| x.payload), Some(i.to_be_bytes().to_vec()), "in order while clients flap");
    }
    let _ = flap.await;
}

#[tokio::test]
async fn transport_tcp_simultaneous() {
    let pa = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|x| x.port()).unwrap_or(0);
    let pb = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|x| x.port()).unwrap_or(0);
    let open = |name: &str, me: u16, other: u16| {
        let mut c = Config::router(name);
        c.listen = vec![format!("127.0.0.1:{me}")];
        c.connect = vec![format!("127.0.0.1:{other}")];
        c.lease = LEASE;
        Mesh::open(c)
    };
    let (a, b) = tokio::join!(open("sa", pa, pb), open("sb", pb, pa));
    let (a, b) = (a.expect("a"), b.expect("b"));
    let mut sa = a.subscribe("test/simul").await.expect("sub");
    let until = Instant::now() + Duration::from_secs(5);
    while !(a.info().await.map(|i| i.sessions.len() == 1).unwrap_or(false) && b.info().await.map(|i| i.sessions.len() == 1).unwrap_or(false)) && Instant::now() < until {
        sleep(Duration::from_millis(20)).await;
    }
    sleep(SLEEP).await;
    for i in 0..100u8 {
        b.put("test/simul", vec![i]).expect("put");
    }
    for i in 0..100u8 {
        assert_eq!(timeout(LEASE, sa.recv()).await.ok().flatten().map(|x| x.payload), Some(vec![i]));
    }
    assert!(timeout(SLEEP, sa.recv()).await.is_err(), "one session — no duplicate");
}
