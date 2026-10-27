// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-REQUEST · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:RULE-BUS-PLACEMENT · 정:IMPL-SES-HANDOVER · 정:IMPL-BUS-FAULTS · 정:RULE-GC-FOUR · 정:IMPL-SES-DATA · 운영:RULE-READ-BUS-ALL · model: claude-opus-5-5
//! 버스 위의 hub — `q/node/{me}/**` 에 답한다(답은 제 키로) · `room` · `node` 토큰을 `history` 로 본다 · 가라앉기 판정 · 후보 grace · 창 만료 sweep.

use std::collections::BTreeSet;
use std::time::Duration;

use common::bus::env::{Call, Reply};
use common::bus::keys::KeySpace;
use oxsig::code::Code;
use zenoh::bytes::ZBytes;
use zenoh::sample::SampleKind;

use super::{HubRef, now_ms, route};
use crate::room::TokenEv;

pub const SWEEP_MS: u64 = 1_000;
pub const SETTLE_TICK_MS: u64 = 100;

fn param(sel: &str, name: &str) -> Option<String> {
    sel.split(['?', '&', ';']).find_map(|kv| kv.strip_prefix(&format!("{name}=")).map(str::to_string))
}

async fn answer(hub: &HubRef, what: &str, params: &str, payload: Vec<u8>) -> Vec<u8> {
    match what {
        "call" => {
            let r = match Call::decode(&payload) {
                Ok(c) => route::local(hub, None, &c).await,
                Err(_) => Reply::fail(Code::INVALID_PAYLOAD),
            };
            r.encode()
        }
        "sess" => {
            let Some(sid) = param(params, "sid") else { return Vec::new() };
            let taken = hub.sessions.lock().ok().and_then(|mut t| t.remove(&sid));
            let Some(s) = taken else { return Vec::new() };
            if let Some(sock) = s.sock {
                hub.send_leave(sock, Code::DUPLICATE_SESSION);
            }
            for r in s.data.all_rooms() {
                hub.interest_dec(&r).await;
            }
            hub.sess_tokens.lock().await.remove(&sid);
            hub.user_interest_drop(&s.data.user_id).await;
            serde_json::to_vec(&s.data).unwrap_or_default()
        }
        "bus" => serde_json::to_vec(&super::admin::bus_body(hub).await).unwrap_or_default(),
        "probe" => {
            let user = param(params, "user").unwrap_or_default();
            serde_json::to_vec(&super::admin::probe_local(hub, &user).await).unwrap_or_default()
        }
        "cut" => {
            let user = param(params, "user").unwrap_or_default();
            serde_json::to_vec(&super::admin::cut_local(hub, &user).await).unwrap_or_default()
        }
        "servers" => {
            let user = param(params, "user").unwrap_or_default();
            let v: Vec<(String, String)> =
                hub.sessions.lock().ok().and_then(|t| t.of_user(&user).map(|s| s.data.servers.iter().map(|(k, v)| (k.clone(), v.node.clone())).collect())).unwrap_or_default();
            serde_json::to_vec(&v).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn token_ev(ks: &KeySpace, key: &str, put: bool) -> Option<TokenEv> {
    if let Some(t) = ks.parse_room(key) {
        return Some(if put { TokenEv::RoomPut(t) } else { TokenEv::RoomDel(t) });
    }
    ks.parse_node(key).map(|t| if put { TokenEv::NodePut(t) } else { TokenEv::NodeDel(t) })
}

fn on_token(hub: &HubRef, key: &str, put: bool) {
    let Some(ev) = token_ev(&hub.ks, key, put) else { return };
    match &ev {
        TokenEv::NodePut(t) => {
            if let Ok(mut c) = hub.cands.lock() {
                c.token_put(&t.node);
            }
        }
        TokenEv::NodeDel(t) => {
            if let Ok(mut c) = hub.cands.lock() {
                c.token_delete(&t.node, now_ms());
            }
        }
        _ => {}
    }
    if let Ok(mut m) = hub.rooms.lock() {
        m.on_event(ev, key, now_ms(), common::time::wall_ms());
    }
}

pub async fn start(hub: &HubRef) {
    let Some(bus) = hub.bus() else { return };
    if let Ok(q) = bus.queryable().await {
        let h = hub.clone();
        tokio::spawn(async move {
            while let Ok(query) = q.recv_async().await {
                let key = query.key_expr().as_str().to_string();
                let params = query.parameters().as_str().to_string();
                let payload = query.payload().map(|p| p.to_bytes().to_vec()).unwrap_or_default();
                let Some((_, what)) = h.ks.parse_q_node(&key) else { continue };
                let h2 = h.clone();
                tokio::spawn(async move {
                    let out = answer(&h2, &what, &params, payload).await;
                    let reply_key = h2.ks.q_node(h2.me(), &what);
                    let _ = query.reply(reply_key, ZBytes::from(out)).await;
                });
            }
        });
    }
    for key in [hub.ks.room_all(), hub.ks.node_all()] {
        if let Ok(sub) = bus.watch(&key).await {
            let h = hub.clone();
            tokio::spawn(async move {
                while let Ok(s) = sub.recv_async().await {
                    on_token(&h, s.key_expr().as_str(), s.kind() == SampleKind::Put);
                }
            });
        }
    }
    if let Ok(sub) = bus.subscribe(&hub.ks.ev_node(hub.me())).await {
        let h = hub.clone();
        tokio::spawn(async move {
            while let Ok(s) = sub.recv_async().await {
                let Ok(v) = serde_json::from_slice::<serde_json::Value>(&s.payload().to_bytes()) else { continue };
                if let Some(user) = v.get("evict_user").and_then(|x| x.as_str()) {
                    let sid = h.sessions.lock().ok().and_then(|t| t.of_user(user).map(|s| s.data.sid.clone()));
                    if let Some(sid) = sid {
                        h.discard_session(&sid, Code::DUPLICATE_SESSION, super::GoneBy::Server).await;
                    }
                }
            }
        });
    }
    let h = hub.clone();
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(SETTLE_TICK_MS));
        loop {
            iv.tick().await;
            let settled = h.rooms.lock().ok().and_then(|mut m| if m.settle_due(now_ms()) { Some(m.settle(now_ms(), common::time::wall_ms())) } else { None });
            if let Some(s) = settled {
                for r in &s.removed {
                    if let Some(j) = h.room_subs.lock().await.remove(r) {
                        j.abort();
                    }
                    let sids: Vec<String> = h.sessions.lock().map(|t| t.all().map(|x| x.data.sid.clone()).collect()).unwrap_or_default();
                    for sid in sids {
                        if let Ok(mut t) = h.sessions.lock()
                            && let Some(x) = t.get_mut(&sid)
                        {
                            for srv in x.data.servers.values_mut() {
                                srv.rooms.remove(r);
                            }
                            x.data.servers.retain(|_, v| !v.rooms.is_empty());
                        }
                    }
                    if let Ok(mut i) = h.interest.lock() {
                        while i.held(r) {
                            i.dec(r);
                        }
                    }
                }
            }
            let grace = h.grace_ms();
            if let Ok(mut c) = h.cands.lock() {
                c.expire(now_ms(), grace);
            }
        }
    });
    let h = hub.clone();
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(SWEEP_MS));
        loop {
            iv.tick().await;
            let win = u64::from(h.cfg.policy.hub.resume_window_ms);
            let expired: BTreeSet<String> = h.sessions.lock().map(|t| t.expired(now_ms(), win).into_iter().collect()).unwrap_or_default();
            for sid in expired {
                h.discard_session(&sid, Code(0), super::GoneBy::Window).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_params() {
        assert_eq!(param("sid=abc", "sid").as_deref(), Some("abc"));
        assert_eq!(param("a=1;user=u7", "user").as_deref(), Some("u7"));
        assert_eq!(param("", "user"), None);
    }

    #[test]
    fn token_keys_become_events() {
        let ks = KeySpace::new("c");
        assert!(matches!(token_ev(&ks, &ks.room("r", "n", "e"), true), Some(TokenEv::RoomPut(_))));
        assert!(matches!(token_ev(&ks, &ks.node("n", "h"), false), Some(TokenEv::NodeDel(_))));
        assert!(token_ev(&ks, "ox/c/int/ev/room/r", true).is_none());
    }
}
