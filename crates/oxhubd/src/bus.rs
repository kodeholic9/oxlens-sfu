// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-BUS-PRESENCE 정:RULE-BUS-EVENTS 정:RULE-BUS-REQUEST 정:IMPL-BUS-FAULTS 정:RULE-BUS-PLACEMENT · model: claude-opus-5-5
//! hub 의 버스 — router 를 열고, `node` · `room` 토큰을 history 로 구독해 배치 맵을 배우고, `q/node/{me}/call` 에 답한다.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::keyspace::{Token, parse_token};
use oxsig::bplane::{NodeReq, NodeRes, UnitEvent};
use zenoh::sample::SampleKind;

use crate::state::{Hub, NodeEntry, Place};

pub async fn open(hub: &Arc<Hub>) -> Result<(), String> {
    let z = &hub.system.zenoh;
    let cfg = common::bus::BusConfig {
        mode: "router",
        listen: z.listen.clone(),
        connect: z.connect.clone(),
        multicast_scouting: z.multicast_scouting,
    };
    let s = common::bus::open(&cfg).await?;
    hub.bus.set(s).map_err(|_| "bus already open".to_string())?;
    watch_nodes(hub.clone()).await?;
    watch_rooms(hub.clone()).await?;
    serve_queries(hub.clone()).await?;
    Ok(())
}

pub async fn declare_node(hub: &Arc<Hub>, inst: &str) {
    let Some(bus) = hub.bus() else { return };
    let mut g = hub.node_token.lock().await;
    if g.as_ref().is_some_and(|(i, _)| i == inst) {
        return;
    }
    if let Some((_, old)) = g.take() {
        let _ = old.undeclare().await;
    }
    match bus.liveliness().declare_token(hub.keys.node(&hub.node, inst)).await {
        Ok(t) => {
            if let Ok(mut n) = hub.nodes.write() {
                n.insert(hub.node.clone(), NodeEntry { inst: inst.to_string(), gone_at: None });
            }
            g.replace((inst.to_string(), t));
            tracing::info!(kind = "ev", spec = "SV-STATE-NODE-LIVE", epoch = inst, "node token declared");
        }
        Err(e) => tracing::warn!(kind = "diag", "node token declare failed: {e}"),
    }
}

pub async fn declare_sess(hub: &Arc<Hub>, user: &str, sid: &str) -> Option<zenoh::liveliness::LivelinessToken> {
    let bus = hub.bus()?;
    bus.liveliness().declare_token(hub.keys.sess(user, &hub.node, sid)).await.ok()
}

pub async fn subscribe_user(hub: &Arc<Hub>, user: &str) -> Option<tokio::task::JoinHandle<()>> {
    let bus = hub.bus()?;
    let sub = bus.declare_subscriber(hub.keys.ev_user(user)).await.ok()?;
    let hub = hub.clone();
    let user = user.to_string();
    Some(tokio::spawn(async move {
        while let Ok(sample) = sub.recv_async().await {
            if let Ok(ev) = oxsig::bplane::decode::<UnitEvent>(&sample.payload().to_bytes()) {
                crate::session::deliver_user(&hub, &user, &ev);
            }
        }
    }))
}

pub async fn subscribe_room(hub: &Arc<Hub>, room: &str) -> tokio::task::JoinHandle<()> {
    let hub2 = hub.clone();
    let key = hub.keys.ev_room(room);
    let room = room.to_string();
    let sub = match hub.bus() {
        Some(bus) => bus.declare_subscriber(key).await.ok(),
        None => None,
    };
    tokio::spawn(async move {
        let Some(sub) = sub else { return };
        while let Ok(sample) = sub.recv_async().await {
            if let Ok(ev) = oxsig::bplane::decode::<UnitEvent>(&sample.payload().to_bytes()) {
                crate::session::deliver_room(&hub2, &room, &ev);
            }
        }
    })
}

async fn watch_nodes(hub: Arc<Hub>) -> Result<(), String> {
    let Some(bus) = hub.bus() else { return Err("no bus".into()) };
    let sub = bus.liveliness().declare_subscriber(hub.keys.node_all()).history(true).await.map_err(|e| e.to_string())?;
    let hub_gc = hub.clone();
    tokio::spawn(async move {
        while let Ok(sample) = sub.recv_async().await {
            let key = sample.key_expr().as_str().to_string();
            let Some(Token::Node { node, inst }) = parse_token(hub.keys.prefix(), &key) else { continue };
            match sample.kind() {
                SampleKind::Put => {
                    hub.note_event("put", &key);
                    if let Ok(mut n) = hub.nodes.write() {
                        n.insert(node, NodeEntry { inst, gone_at: None });
                    }
                    hub.isolated.store(false, Ordering::Relaxed);
                }
                SampleKind::Delete => {
                    hub.note_event("delete", &key);
                    if let Ok(mut n) = hub.nodes.write() {
                        if n.get(&node).is_some_and(|e| e.inst == inst)
                            && let Some(e) = n.get_mut(&node) {
                                e.gone_at = Some(Instant::now());
                            }
                        let others: Vec<&NodeEntry> = n.iter().filter(|(k, _)| **k != hub.node).map(|(_, v)| v).collect();
                        let all_gone = others.len() >= 2 && others.iter().all(|e| e.gone_at.is_some());
                        hub.isolated.store(all_gone, Ordering::Relaxed);
                    }
                }
            }
        }
    });
    tokio::spawn(async move {
        let grace = Duration::from_secs(hub_gc.system.supervisor.node_grace_secs);
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if let Ok(mut n) = hub_gc.nodes.write() {
                n.retain(|_, e| e.gone_at.is_none_or(|t| t.elapsed() < grace));
            }
        }
    });
    Ok(())
}

async fn watch_rooms(hub: Arc<Hub>) -> Result<(), String> {
    let Some(bus) = hub.bus() else { return Err("no bus".into()) };
    let sub = bus.liveliness().declare_subscriber(hub.keys.room_all()).history(true).await.map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        while let Ok(sample) = sub.recv_async().await {
            let key = sample.key_expr().as_str().to_string();
            let Some(Token::Room { room, node, inst }) = parse_token(hub.keys.prefix(), &key) else { continue };
            match sample.kind() {
                SampleKind::Put => {
                    hub.note_event("put", &key);
                    if let Ok(mut m) = hub.rooms.write() {
                        let v = m.entry(room.clone()).or_default();
                        let p = Place { node, inst };
                        if !v.contains(&p) {
                            v.push(p);
                        }
                        if v.iter().map(|p| &p.node).collect::<std::collections::BTreeSet<_>>().len() > 1 {
                            hub.dup_rooms.fetch_add(1, Ordering::Relaxed);
                            tracing::warn!(kind = "ev", spec = "SV-STATE-ROOM-DUPLICATE", room = room.as_str(), "room on two nodes");
                        }
                    }
                }
                SampleKind::Delete => {
                    hub.note_event("delete", &key);
                    let emptied = match hub.rooms.write() {
                        Ok(mut m) => {
                            let mut emptied = false;
                            if let Some(v) = m.get_mut(&room) {
                                v.retain(|p| !(p.node == node && p.inst == inst));
                                emptied = v.is_empty();
                            }
                            if emptied {
                                m.remove(&room);
                            }
                            emptied
                        }
                        Err(_) => false,
                    };
                    if emptied && !hub.isolated.load(Ordering::Relaxed) {
                        crate::session::forget_room(&hub, &room);
                    }
                }
            }
        }
    });
    Ok(())
}

async fn serve_queries(hub: Arc<Hub>) -> Result<(), String> {
    let Some(bus) = hub.bus() else { return Err("no bus".into()) };
    let q = bus.declare_queryable(format!("{}/call", hub.keys.q_node(&hub.node))).await.map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        while let Ok(query) = q.recv_async().await {
            let hub = hub.clone();
            tokio::spawn(async move {
                let req = query.payload().map(|p| p.to_bytes().to_vec()).unwrap_or_default();
                let res = match oxsig::bplane::decode::<NodeReq>(&req) {
                    Ok(r) => handle(&hub, r).await,
                    Err(e) => NodeRes::Fail { code: 1002, message: e.to_string() },
                };
                let key = query.key_expr().clone();
                let _ = query.reply(key, oxsig::bplane::encode(&res)).await;
            });
        }
    });
    Ok(())
}

async fn handle(hub: &Arc<Hub>, req: NodeReq) -> NodeRes {
    match req {
        NodeReq::Unit { req } => match hub.units.sfu_call(&req, crate::route::CALL_TIMEOUT).await {
            Ok(r) => NodeRes::Unit(r),
            Err(c) => NodeRes::Fail { code: c.0, message: c.name().to_string() },
        },
        NodeReq::TakeSession { session_id } => NodeRes::Session(crate::session::give_session(hub, &session_id)),
        NodeReq::DropUser { user_id, code, except_session } => {
            NodeRes::Cut(crate::session::drop_user(hub, &user_id, code, except_session.as_deref()).await)
        }
        NodeReq::Bus => NodeRes::Bus(crate::admin::bus_view(hub).await),
    }
}
