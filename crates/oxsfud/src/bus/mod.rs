// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:IMPL-BUS-HOME-LOST · 운영:LOG.log.LINE · 운영:RULE-LOG-KIND · 정:IMPL-ROOM-LIFE · 정:IMPL-FLR-TALK-REPORT · model: claude-opus-5-5
//! sfud 의 버스 — `common::bus` client(`{node}.sfud` · 로컬 hub 에만 · 202610/20261031e) · `room` 토큰 선언(토큰은 그 방이 든다 — 재선언은 방 집합에서) · `ev` put 은 낸 차례 그대로 한 줄로(데이터 레인 — 차면 버리고 센다) · `node` 토큰 history 구독(홈 잃음) · 로컬 hub 에 묻기(`q/node/{node}/talk`).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use common::bus::{Client, TokenEvent};
use common::envelope::Notice;
use common::keyspace::{Keys, Parsed};
use tokio::sync::mpsc;

pub type RoomToken = common::bus::Token;

static BUS: OnceLock<Arc<Bus>> = OnceLock::new();

enum Put {
    Room(String, Notice),
    User(String, Notice),
}

pub struct Bus {
    pub keys: Keys,
    pub node: String,
    pub epoch: String,
    session: Client,
    tx: mpsc::UnboundedSender<Put>,
    pub nodes: Mutex<HashMap<String, (String, bool, u64)>>,
    pub seen_other: Mutex<bool>,
    pub ev_drop: std::sync::atomic::AtomicU64,
    _subs: Mutex<Vec<tokio::task::AbortHandle>>,
}

pub fn get() -> Option<&'static Arc<Bus>> {
    BUS.get()
}

impl Bus {
    pub async fn router_session(&self) -> Option<(String, u64)> {
        self.session.router_session().await
    }

    pub fn ev_dropped(&self) -> u64 {
        self.ev_drop.load(std::sync::atomic::Ordering::Relaxed) + self.session.dropped()
    }

    pub async fn open(node: &str, epoch: &str, ns: &str, connect: &str) -> Result<Arc<Bus>, String> {
        let session = Client::open(node, "sfud", connect).await.map_err(|e| format!("mesh open: {e}"))?;
        let (tx, mut rx) = mpsc::unbounded_channel::<Put>();
        let bus = Arc::new(Bus {
            keys: Keys::new(ns),
            node: node.to_string(),
            epoch: epoch.to_string(),
            session,
            tx,
            nodes: Mutex::new(HashMap::new()),
            seen_other: Mutex::new(false),
            ev_drop: std::sync::atomic::AtomicU64::new(0),
            _subs: Mutex::new(Vec::new()),
        });
        let b2 = bus.clone();
        tokio::spawn(async move {
            while let Some(p) = rx.recv().await {
                let (key, n) = match p {
                    Put::Room(r, n) => (b2.keys.ev_room(&r), n),
                    Put::User(u, n) => (b2.keys.ev_user(&u), n),
                };
                let payload = serde_json::to_vec(&n).unwrap_or_default();
                if b2.session.put(&key, payload).is_err() {
                    b2.ev_drop.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });
        let me = Arc::downgrade(&bus);
        let keys = bus.keys.clone();
        let mut sub = bus.session.token_sub(&keys.node_all(), true).await.map_err(|e| format!("node sub: {e}"))?;
        let h = tokio::spawn(async move {
            while let Some(ev) = sub.recv().await {
                let (k, put) = match ev {
                    TokenEvent::Put(k) => (k, true),
                    TokenEvent::Delete(k) => (k, false),
                    TokenEvent::BatchEnd => continue,
                };
                if let (Some(b), Some(Parsed::Node { node, inst })) = (me.upgrade(), keys.parse(&k)) {
                    let back = match b.nodes.lock() {
                        Ok(mut g) => {
                            if node != b.node && put && let Ok(mut so) = b.seen_other.lock() {
                                let _ = std::mem::replace(&mut *so, true);
                            }
                            let key = format!("{node}/{inst}");
                            let was = g.get(&key).map(|(_, l, _)| *l).unwrap_or(false);
                            let back = node == b.node && put && !was;
                            g.insert(key, (node, put, common::now_ms()));
                            back
                        }
                        Err(_) => false,
                    };
                    if back {
                        tokio::spawn(crate::ops::rooms::redeclare());
                    }
                }
            }
        });
        if let Ok(mut g) = bus._subs.lock() {
            g.push(h.abort_handle());
        }
        let _ = BUS.set(bus.clone());
        Ok(bus)
    }

    pub fn room_notice(&self, room: &str, n: Notice) {
        let _ = self.tx.send(Put::Room(room.to_string(), n));
    }

    pub fn user_notice(&self, user: &str, n: Notice) {
        let _ = self.tx.send(Put::User(user.to_string(), n));
    }

    pub async fn ask(&self, what: &str, payload: Vec<u8>, timeout: std::time::Duration) -> Option<Vec<u8>> {
        let key = self.keys.q_node(&self.node, what);
        self.session.ask(&key, payload, timeout).await
    }

    pub async fn declare_room(&self, room: &str) -> Option<RoomToken> {
        let key = self.keys.room_token(room, &self.node, &self.epoch);
        match self.session.declare_token(&key).await {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!(kind = "diag", spec = "SV-RULE-BUS-PRESENCE", room = %room, "room token: {e}");
                None
            }
        }
    }

    pub fn home_alive(&self, node: &str, inst: &str) -> Option<(bool, u64)> {
        let g = self.nodes.lock().ok()?;
        g.get(&format!("{node}/{inst}")).map(|(_, live, at)| (*live, *at))
    }

    pub fn isolated(&self) -> bool {
        let seen = self.seen_other.lock().map(|g| *g).unwrap_or(false);
        if !seen {
            return false;
        }
        let g = match self.nodes.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        !g.values().any(|(n, live, _)| n != &self.node && *live)
    }

    pub fn live_nodes(&self) -> usize {
        let g = match self.nodes.lock() {
            Ok(g) => g,
            Err(_) => return 0,
        };
        let s: HashSet<&String> = g.values().filter(|(_, l, _)| *l).map(|(n, _, _)| n).collect();
        s.len()
    }

    pub async fn close(&self) {
        if let Ok(mut g) = self._subs.lock() {
            for h in g.drain(..) {
                h.abort();
            }
        }
        self.session.close().await;
    }
}

pub fn notice_line(n: &Notice) -> common::log::Line {
    let Some(op) = oxsig::Op::from_u16(n.op) else {
        return common::log::Line::diag("SV-RULE-BUS-EVENTS").rec("op", n.op).msg("notice of unknown op");
    };
    let mut l = common::log::Line::ev(&format!("CP-OP-{}", op.name().replace('_', "-")));
    if let Some(r) = &n.room_id {
        l = l.key("room", r);
    }
    if let Some(seq) = n.body.get("version").and_then(|v| v.get("seq")).and_then(|v| v.as_u64()) {
        l = l.key("seq", seq);
    }
    if let Some(arev) = n.body.get("table").and_then(|t| t.get("arev")).and_then(|v| v.as_u64()) {
        l = l.key("arev", arev);
    }
    l
}

pub fn room_notice(room: &str, n: Notice) {
    common::log::emit(notice_line(&n).key("room", room));
    if let Some(b) = get() {
        b.room_notice(room, n);
    }
}

pub fn user_notice(user: &str, n: Notice) {
    if let Some(b) = get() {
        b.user_notice(user, n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(op: oxsig::Op, body: serde_json::Value) -> Notice {
        Notice { op: op.as_u16(), body, exclude: Vec::new(), room_id: Some("r1".into()), sfu_id: None }
    }

    #[test]
    fn notice_line_carries_room_seq_arev_not_body() {
        let head = common::log::line::Head { node: "n1".into(), proc_name: "sfu" };
        let p = notice_line(&n(oxsig::Op::ParticipantEvent, serde_json::json!({"version": {"epoch": "e", "seq": 7}, "user_id": "secret"})));
        assert_eq!(p.render(&head, "T", 1, 1024), "ts=T mono=1 node=n1 proc=sfu kind=ev spec=CP-OP-PARTICIPANT-EVENT room=r1 seq=7");
        let m = notice_line(&n(oxsig::Op::MlineEvent, serde_json::json!({"table": {"arev": 3, "mlines": []}})));
        assert_eq!(m.render(&head, "T", 1, 1024), "ts=T mono=1 node=n1 proc=sfu kind=ev spec=CP-OP-MLINE-EVENT room=r1 arev=3");
    }
}
