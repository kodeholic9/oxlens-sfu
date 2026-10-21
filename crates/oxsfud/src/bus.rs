// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-BUS-PRESENCE 정:RULE-BUS-EVENTS 정:IMPL-BUS-HOME-LOST 정:RULE-BUS-FATE · model: claude-opus-5-5
//! sfud 버스 — zenoh client. `room` 토큰을 선언하고 `ev` 를 Drop 으로 put 한다. `node` 토큰이 사라지면 그 홈의 Peer 를 거둔다.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::bus::BusConfig;
use common::keyspace::{Token, parse_token};
use zenoh::sample::SampleKind;

use crate::sfu::{Out, Sfu, lock};

pub fn spawn(sfu: &Arc<Sfu>, connect: Option<String>) {
    let Some(rx) = lock(&sfu.out_rx).take() else { return };
    let s = sfu.clone();
    match connect {
        Some(c) => {
            tokio::spawn(async move { run(s, c, rx).await });
        }
        None => {
            tokio::spawn(async move {
                let mut rx = rx;
                while rx.recv().await.is_some() {}
            });
        }
    }
}

async fn run(sfu: Arc<Sfu>, connect: String, mut rx: tokio::sync::mpsc::UnboundedReceiver<Out>) {
    let cfg = BusConfig { mode: "client", listen: vec![], connect: vec![connect], multicast_scouting: false };
    let session = loop {
        match common::bus::open(&cfg).await {
            Ok(s) => break s,
            Err(e) => {
                tracing::warn!(kind = "diag", "bus open: {e}");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    };
    sfu.bus_up.store(true, Ordering::SeqCst);
    tracing::info!(kind = "ev", spec = "RULE-BUS-PRESENCE", "bus connected");
    watch_nodes(&sfu, &session).await;
    let mut tokens: HashMap<String, zenoh::liveliness::LivelinessToken> = HashMap::new();
    let existing: Vec<String> = lock(&sfu.rooms).keys().cloned().collect();
    for r in existing {
        declare(&sfu, &session, &mut tokens, &r).await;
    }
    while let Some(o) = rx.recv().await {
        match o {
            Out::Room { room, ev } => {
                common::bus::put_event(&session, &sfu.keys.ev_room(&room), oxsig::bplane::encode(&ev)).await;
            }
            Out::User { user, ev } => {
                common::bus::put_event(&session, &sfu.keys.ev_user(&user), oxsig::bplane::encode(&ev)).await;
            }
            Out::Token { room, up: true } => declare(&sfu, &session, &mut tokens, &room).await,
            Out::Token { room, up: false } => {
                if let Some(t) = tokens.remove(&room) {
                    let _ = t.undeclare().await;
                }
            }
        }
    }
}

async fn declare(sfu: &Sfu, s: &zenoh::Session, tokens: &mut HashMap<String, zenoh::liveliness::LivelinessToken>, room: &str) {
    if tokens.contains_key(room) {
        return;
    }
    let key = sfu.keys.room(room, &sfu.node, &sfu.epoch);
    match s.liveliness().declare_token(key.clone()).await {
        Ok(t) => {
            tokens.insert(room.to_string(), t);
        }
        Err(e) => tracing::warn!(kind = "diag", key = key.as_str(), "room token: {e}"),
    }
}

async fn watch_nodes(sfu: &Arc<Sfu>, s: &zenoh::Session) {
    let sub = match s.liveliness().declare_subscriber(sfu.keys.node_all()).history(true).await {
        Ok(x) => x,
        Err(e) => {
            tracing::warn!(kind = "diag", "node watch: {e}");
            return;
        }
    };
    let sfu = sfu.clone();
    tokio::spawn(async move {
        while let Ok(sample) = sub.recv_async().await {
            let key = sample.key_expr().as_str().to_string();
            let Some(Token::Node { node, inst }) = parse_token(sfu.keys.prefix(), &key) else { continue };
            match sample.kind() {
                SampleKind::Put => {
                    lock(&sfu.nodes).insert(node, vec![inst]);
                }
                SampleKind::Delete => {
                    let others = {
                        let mut n = lock(&sfu.nodes);
                        let known_others = n.keys().filter(|k| **k != sfu.node).count();
                        if n.get(&node).is_some_and(|v| v.contains(&inst)) {
                            n.remove(&node);
                        }
                        let alive_others = n.keys().filter(|k| **k != sfu.node).count();
                        (known_others, alive_others)
                    };
                    let (known, alive) = others;
                    if node == sfu.node {
                        continue;
                    }
                    if known < 2 || alive == 0 {
                        tracing::info!(kind = "diag", spec = "IMPL-BUS-HOME-LOST", node = node.as_str(), known, alive, "node token gone — isolation or two-node, not reaping");
                        continue;
                    }
                    crate::session::home_lost(&sfu, &node, &inst);
                }
            }
        }
    });
}
