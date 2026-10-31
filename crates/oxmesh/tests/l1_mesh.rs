// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:RULE-BUS-REQUEST · model: claude-opus-5-5
//! 1층 — 한 프로세스 안 router 셋 + client 로 그물을 세운다(127.0.0.1 · lease 1 s · 202610/20261031e 단계 B · C).

use std::time::Duration;

use oxmesh::{Config, Mesh, TokenEvent};
use tokio::time::{Instant, sleep, timeout};

const LEASE: Duration = Duration::from_secs(1);

async fn router(node: &str, seeds: &[&Mesh]) -> Mesh {
    let mut c = Config::router(node);
    c.listen = vec!["127.0.0.1:0".into()];
    c.connect = seeds.iter().map(|m| m.bound()[0].to_string()).collect();
    c.lease = LEASE;
    Mesh::open(c).await.expect("open router")
}

async fn router_at(node: &str, port: u16, seeds: &[String]) -> Mesh {
    let mut c = Config::router(node);
    c.listen = vec![format!("127.0.0.1:{port}")];
    c.connect = seeds.to_vec();
    c.lease = LEASE;
    Mesh::open(c).await.expect("open router")
}

async fn client(node: &str, r: &Mesh) -> Mesh {
    let mut c = Config::client(node, &r.bound()[0].to_string());
    c.lease = LEASE;
    Mesh::open(c).await.expect("open client")
}

async fn eventually<F, Fut>(what: &str, f: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if f().await {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("not reached in 5 s: {what}");
}

async fn router_peers(m: &Mesh) -> Vec<(String, String)> {
    m.info().await.map(|i| i.sessions.into_iter().filter(|s| s.router).map(|s| (s.peer, s.dialer)).collect()).unwrap_or_default()
}

async fn mesh3() -> (Mesh, Mesh, Mesh) {
    let a = router("a", &[]).await;
    let b = router("b", &[&a]).await;
    let c = router("c", &[&a]).await;
    for m in [&a, &b, &c] {
        eventually("full mesh", || async { router_peers(m).await.len() == 2 }).await;
    }
    (a, b, c)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(0)
}

#[tokio::test]
async fn full_mesh_forms_from_one_seed_and_smaller_node_dials() {
    let (a, b, c) = mesh3().await;
    for (m, me) in [(&a, "a"), (&b, "b"), (&c, "c")] {
        for (peer, dialer) in router_peers(m).await {
            let want = if me < peer.as_str() { me.to_string() } else { peer.clone() };
            assert!(dialer == want || me == "a" || peer == "a", "{me}–{peer} dialed by {dialer}");
        }
    }
    let bc = router_peers(&b).await.into_iter().find(|(p, _)| p == "c").map(|(_, d)| d);
    assert_eq!(bc.as_deref(), Some("b"));
}

#[tokio::test]
async fn tokens_reach_every_router_with_history_and_leave_on_drop() {
    let (a, b, c) = mesh3().await;
    let s = client("sfud-b", &b).await;
    let tok = s.declare_token("ox/t/int/room/r1/b/i1").await.expect("token");
    for m in [&a, &b, &c] {
        eventually("token seen", || async { m.tokens("ox/t/int/room/**", Duration::from_secs(1)).await.unwrap_or_default() == vec!["ox/t/int/room/r1/b/i1".to_string()] })
            .await;
    }
    let mut hist = c.token_sub("ox/t/int/room/*/*/*", true).await.expect("sub");
    assert_eq!(timeout(LEASE, hist.recv()).await.ok().flatten(), Some(TokenEvent::Put("ox/t/int/room/r1/b/i1".into())));
    assert_eq!(timeout(LEASE, hist.recv()).await.ok().flatten(), Some(TokenEvent::BatchEnd));
    drop(tok);
    assert_eq!(timeout(LEASE, hist.recv()).await.ok().flatten(), Some(TokenEvent::Delete("ox/t/int/room/r1/b/i1".into())));
}

#[tokio::test]
async fn client_sees_deletes_and_batch_end_when_its_router_is_lost() {
    let (a, b, _c) = mesh3().await;
    let _node = a.declare_token("ox/t/int/node/a/h1").await.expect("token");
    let s = client("sfud-b", &b).await;
    let mut sub = s.token_sub("ox/t/int/node/*/*", true).await.expect("sub");
    eventually("client history", || async { s.tokens("ox/t/int/node/**", Duration::from_secs(1)).await.unwrap_or_default().len() == 1 }).await;
    let mut got = Vec::new();
    while let Ok(Some(e)) = timeout(Duration::from_millis(200), sub.recv()).await {
        got.push(e);
    }
    assert!(got.contains(&TokenEvent::Put("ox/t/int/node/a/h1".into())));
    b.close().await;
    let mut after = Vec::new();
    while let Ok(Some(e)) = timeout(Duration::from_millis(500), sub.recv()).await {
        after.push(e);
        if after.last() == Some(&TokenEvent::BatchEnd) {
            break;
        }
    }
    assert_eq!(after, vec![TokenEvent::Delete("ox/t/int/node/a/h1".into()), TokenEvent::BatchEnd]);
}

#[tokio::test]
async fn put_reaches_each_subscribing_node_once_in_one_hop() {
    let (a, b, c) = mesh3().await;
    let mut sa = a.subscribe("ox/t/int/ev/room/r1").await.expect("sub");
    let mut sb = b.subscribe("ox/t/int/ev/room/r1").await.expect("sub");
    let mut sb2 = b.subscribe("ox/t/int/ev/room/*").await.expect("sub");
    let s = client("sfud-c", &c).await;
    sleep(Duration::from_millis(300)).await;
    for i in 0..50u8 {
        s.put("ox/t/int/ev/room/r1", vec![i]).expect("put");
    }
    for sub in [&mut sa, &mut sb, &mut sb2] {
        for i in 0..50u8 {
            let got = timeout(LEASE, sub.recv()).await.ok().flatten().map(|x| x.payload);
            assert_eq!(got, Some(vec![i]), "order and count");
        }
        assert!(timeout(Duration::from_millis(200), sub.recv()).await.is_err(), "no duplicate");
    }
}

#[tokio::test]
async fn get_fans_out_and_answers_carry_the_node_key() {
    let (a, b, c) = mesh3().await;
    let s = client("sfud-a", &a).await;
    let mut qs = Vec::new();
    for m in [&a, &b, &c] {
        let mut q = m.queryable(&format!("ox/t/int/q/node/{}/**", m.node())).await.expect("qabl");
        let n = m.node().to_string();
        qs.push(tokio::spawn(async move {
            while let Some(query) = q.recv().await {
                query.reply(&format!("ox/t/int/q/node/{n}/call"), query.payload().to_vec());
            }
        }));
    }
    sleep(Duration::from_millis(300)).await;
    for asker in [&a, &s] {
        let mut r = asker.get("ox/t/int/q/node/*/call", b"hi".to_vec(), Duration::from_secs(3)).expect("get");
        let mut keys = Vec::new();
        let t0 = Instant::now();
        while let Some(rep) = timeout(Duration::from_secs(4), r.recv()).await.ok().flatten() {
            assert_eq!(rep.result, Ok(b"hi".to_vec()));
            keys.push(rep.key);
        }
        keys.sort();
        assert_eq!(keys, vec!["ox/t/int/q/node/a/call", "ox/t/int/q/node/b/call", "ox/t/int/q/node/c/call"]);
        assert!(t0.elapsed() < Duration::from_secs(1), "ends on Final, not on timeout");
    }
}

#[tokio::test]
async fn pending_query_ends_at_once_when_the_target_session_closes() {
    let (a, b, _c) = mesh3().await;
    let mut q = b.queryable("ox/t/int/q/node/b/**").await.expect("qabl");
    sleep(Duration::from_millis(300)).await;
    let mut r = a.get("ox/t/int/q/node/b/call", vec![], Duration::from_secs(10)).expect("get");
    let held = timeout(LEASE, q.recv()).await.ok().flatten().expect("query arrived");
    let t0 = Instant::now();
    b.close().await;
    assert_eq!(timeout(Duration::from_secs(3), r.recv()).await.ok().flatten(), None);
    assert!(t0.elapsed() < Duration::from_secs(2), "not the 10 s timeout");
    drop(held);
}

#[tokio::test]
async fn restart_with_same_node_id_resyncs_and_old_cleanup_keeps_new_tokens() {
    let a = router("a", &[]).await;
    let port = free_port();
    let seed = vec![a.bound()[0].to_string()];
    let b1 = router_at("b", port, &seed).await;
    let t1 = b1.declare_token("ox/t/int/node/b/boot1").await.expect("token");
    eventually("first boot seen", || async { a.tokens("ox/t/int/node/b/*", LEASE).await.unwrap_or_default() == vec!["ox/t/int/node/b/boot1".to_string()] }).await;
    drop(t1);
    b1.close().await;
    let b2 = router_at("b", port, &seed).await;
    let _t2 = b2.declare_token("ox/t/int/node/b/boot2").await.expect("token");
    eventually("second boot seen", || async { a.tokens("ox/t/int/node/b/*", LEASE).await.unwrap_or_default() == vec!["ox/t/int/node/b/boot2".to_string()] }).await;
    sleep(LEASE * 2).await;
    assert_eq!(a.tokens("ox/t/int/node/b/*", LEASE).await.unwrap_or_default(), vec!["ox/t/int/node/b/boot2".to_string()]);
}

#[tokio::test]
async fn both_seeding_each_other_keeps_one_session_dialed_by_the_smaller() {
    let (px, py) = (free_port(), free_port());
    let x = router_at("x", px, &[format!("127.0.0.1:{py}")]).await;
    let y = router_at("y", py, &[format!("127.0.0.1:{px}")]).await;
    eventually("one session each", || async { router_peers(&x).await.len() == 1 && router_peers(&y).await.len() == 1 }).await;
    sleep(LEASE).await;
    let sx = x.info().await.expect("info").sessions;
    let sy = y.info().await.expect("info").sessions;
    assert_eq!(sx.len(), 1);
    assert_eq!(sy.len(), 1);
    assert_eq!(sx[0].dialer, "x");
    assert_eq!(sy[0].dialer, "x");
    sleep(LEASE).await;
    let again = x.info().await.expect("info").sessions;
    assert_eq!((again.len(), again[0].sno), (1, sx[0].sno), "stable — no churn");
}
