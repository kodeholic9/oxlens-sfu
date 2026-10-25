// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-SES-DATA 정:IMPL-SES-BIND 정:IMPL-SES-HANDOVER 정:IMPL-SES-RESUME 정:IMPL-SES-SOCKET-LIFE 정:RULE-SES-READ-AND-PROCESS 정:IMPL-SES-ADMISSION 정:RULE-GC-FOUR 정:CAND-SES-SUBROOMS-RECONCILE 정:CAND-SES-PREBIND-2001 연:RULE-SES-AUTH-TWO-WAYS 연:RULE-SES-EVICT-OLD 연:RULE-SES-PC-MODE-SESSION 연:RULE-SES-RESUME-BY-SESSION-ID 연:SES.S.bind_resume 연:SES.S.bind_new 연:SES.S.leave 연:SES.S.TWindow 연:SES.S.drop_reclaim 연:SES.S.resume_ok 연:SES.S.resume_2008 · model: claude-opus-5-5
//! 세션 — 신원 · `session_id` · 토큰값 넷 · 홈 · 서버별 입장 방 집합 · 소켓 하나 · 재개 창 · 폐기 경로 하나.

pub mod bind;
pub mod conn;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use oxsig::envelope::{Call, Ctx, Home};
use oxsig::msg::{PcMode, Permission};
use oxsig::{Code, Op};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc};

use crate::ops::hub::Hub;

#[derive(Debug)]
pub enum Out {
    Reply(oxsig::frame::Frame),
    Notify { op: u16, body: Vec<u8> },
    Leave(Code),
}

#[derive(Debug, Clone)]
pub struct ConnHandle {
    pub id: u64,
    pub tx: mpsc::UnboundedSender<Out>,
    pub resumed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub user: String,
    pub ptype: u8,
    pub hidden: bool,
    pub metadata: Option<serde_json::Value>,
    pub permission: Option<Permission>,
    pub pc_mode: PcMode,
}

#[derive(Debug, Default)]
pub struct Inner {
    pub conn: Option<ConnHandle>,
    pub gone_at: Option<Instant>,
    pub rooms: BTreeMap<String, BTreeSet<String>>,
    pub dead: bool,
}

pub struct Session {
    pub sid: String,
    pub fp: String,
    pub id: Identity,
    pub inner: Mutex<Inner>,
}

impl Session {
    pub fn ctx(&self, hub: &Hub) -> Ctx {
        Ctx {
            user_id: self.id.user.clone(),
            session: self.fp.clone(),
            pc_mode: self.id.pc_mode,
            home: Home { node: hub.cfg.node_id.clone(), inst: hub.inst.clone() },
            participant_type: self.id.ptype,
            hidden: self.id.hidden,
            metadata: self.id.metadata.clone(),
            permission: self.id.permission.clone(),
        }
    }

    pub async fn send(&self, o: Out) -> bool {
        let g = self.inner.lock().await;
        match &g.conn {
            Some(c) => c.tx.send(o).is_ok(),
            None => false,
        }
    }

    pub async fn nodes(&self) -> Vec<String> {
        self.inner.lock().await.rooms.iter().filter(|(_, r)| !r.is_empty()).map(|(n, _)| n.clone()).collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessData {
    pub sid: String,
    pub id: Identity,
    pub rooms: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Default)]
pub struct Sessions {
    pub by_sid: DashMap<String, Arc<Session>>,
    pub by_user: DashMap<String, String>,
    pub by_room: DashMap<String, HashSet<String>>,
}

impl Sessions {
    pub fn get(&self, sid: &str) -> Option<Arc<Session>> {
        self.by_sid.get(sid).map(|s| s.clone())
    }

    pub fn of_user(&self, user: &str) -> Option<Arc<Session>> {
        self.by_user.get(user).and_then(|sid| self.get(&sid))
    }

    pub fn count(&self) -> usize {
        self.by_sid.len()
    }

    pub fn all(&self) -> Vec<Arc<Session>> {
        self.by_sid.iter().map(|e| e.value().clone()).collect()
    }
}

pub async fn set_rooms(hub: &Hub, s: &Arc<Session>, node: &str, rooms: BTreeSet<String>) {
    let (added, removed) = {
        let mut g = s.inner.lock().await;
        if g.dead {
            return;
        }
        let old = g.rooms.get(node).cloned().unwrap_or_default();
        let added: Vec<String> = rooms.difference(&old).cloned().collect();
        let removed: Vec<String> = old.difference(&rooms).cloned().collect();
        if rooms.is_empty() {
            g.rooms.remove(node);
        } else {
            g.rooms.insert(node.to_string(), rooms);
        }
        (added, removed)
    };
    for r in added {
        hub.ses.by_room.entry(r.clone()).or_default().insert(s.sid.clone());
        if let Err(e) = hub.bus.room_interest(&r, 1).await {
            tracing::warn!(room = r.as_str(), "interest: {e}");
        }
    }
    for r in removed {
        if let Some(mut e) = hub.ses.by_room.get_mut(&r) {
            e.remove(&s.sid);
        }
        hub.ses.by_room.remove_if(&r, |_, v| v.is_empty());
        let _ = hub.bus.room_interest(&r, -1).await;
    }
}

pub fn parse_sub_rooms(op: Op, body: &[u8]) -> Option<BTreeSet<String>> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let arr = match op {
        Op::Affiliation => v.get("sub_rooms"),
        _ => v.get("affiliation").and_then(|a| a.get("sub_rooms")),
    }?;
    Some(arr.as_array()?.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gone {
    Leave,
    Window,
    Evicted,
    Cut,
    Shutdown,
}

impl Gone {
    fn name(self) -> &'static str {
        match self {
            Gone::Leave => "leave",
            Gone::Window => "window",
            Gone::Evicted => "evicted",
            Gone::Cut => "cut",
            Gone::Shutdown => "shutdown",
        }
    }
}

pub async fn destroy(hub: &Arc<Hub>, s: &Arc<Session>, why: Gone, leave: Option<Code>) {
    let (rooms, conn) = {
        let mut g = s.inner.lock().await;
        if g.dead {
            return;
        }
        g.dead = true;
        (std::mem::take(&mut g.rooms), g.conn.take())
    };
    if let Some(c) = conn
        && let Some(code) = leave
    {
        let _ = c.tx.send(Out::Leave(code));
    }
    hub.ses.by_sid.remove(&s.sid);
    hub.ses.by_user.remove_if(&s.id.user, |_, v| *v == s.sid);
    for (node, rs) in &rooms {
        for r in rs {
            if let Some(mut e) = hub.ses.by_room.get_mut(r) {
                e.remove(&s.sid);
            }
            hub.ses.by_room.remove_if(r, |_, v| v.is_empty());
            let _ = hub.bus.room_interest(r, -1).await;
        }
        let call = Call::SessionGone { user_id: s.id.user.clone(), session: s.fp.clone() };
        let _ = hub.call_node(node, None, &call).await;
    }
    hub.bus.undeclare_sess(&s.sid).await;
    if hub.ses.of_user(&s.id.user).is_none() {
        hub.bus.unwatch_user(&s.id.user).await;
    }
    common::ev!(
        spec = "CP-OP-LEAVE",
        session = s.fp.as_str(),
        user = s.id.user.as_str(),
        "session gone path={} rooms={}",
        why.name(),
        rooms.values().map(BTreeSet::len).sum::<usize>()
    );
}

pub async fn sweep(hub: Arc<Hub>) {
    let window = Duration::from_millis(u64::from(hub.cfg.policy.hub.resume_window_ms));
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        for s in hub.ses.all() {
            let expired = {
                let g = s.inner.lock().await;
                g.conn.is_none() && g.gone_at.is_some_and(|t| t.elapsed() >= window)
            };
            if expired {
                destroy(&hub, &s, Gone::Window, None).await;
            }
        }
    }
}

pub async fn deliver(hub: &Arc<Hub>, inc: crate::bus::Incoming) {
    match inc {
        crate::bus::Incoming::User { user, notice } => {
            let Some(s) = hub.ses.of_user(&user) else {
                return;
            };
            if notice.op == Op::RoomEvent.as_u16() {
                reconcile_room_event(hub, &s, &notice).await;
            }
            if !s.send(Out::Notify { op: notice.op, body: notice.body.into_bytes() }).await {
                hub.ev_lost.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        crate::bus::Incoming::Room { room, notice } => {
            let sids: Vec<String> = hub.ses.by_room.get(&room).map(|e| e.iter().cloned().collect()).unwrap_or_default();
            for sid in sids {
                let Some(s) = hub.ses.get(&sid) else { continue };
                if notice.exclude.iter().any(|u| *u == s.id.user) {
                    continue;
                }
                if !s.send(Out::Notify { op: notice.op, body: notice.body.clone().into_bytes() }).await {
                    hub.ev_lost.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        crate::bus::Incoming::RoomGone { room } => {
            let sids: Vec<String> = hub.ses.by_room.remove(&room).map(|(_, v)| v.into_iter().collect()).unwrap_or_default();
            for sid in sids {
                if let Some(s) = hub.ses.get(&sid) {
                    let mut g = s.inner.lock().await;
                    for rs in g.rooms.values_mut() {
                        rs.remove(&room);
                    }
                    g.rooms.retain(|_, v| !v.is_empty());
                }
            }
            hub.bus.drop_room_interest(&room).await;
            tracing::info!(room = room.as_str(), "room token gone — local routing dropped");
        }
    }
}

async fn reconcile_room_event(hub: &Arc<Hub>, s: &Arc<Session>, n: &oxsig::envelope::Notice) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&n.body) else {
        return;
    };
    if v.get("type").and_then(|t| t.as_str()) != Some("affiliation") {
        return;
    }
    let Some(sub) = v.get("affiliation").and_then(|a| a.get("sub_rooms")).and_then(|x| x.as_array()) else {
        return;
    };
    let rooms: BTreeSet<String> = sub.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
    let node = n.sfu_id.as_deref().and_then(|i| hub.node_of_inst(i)).or_else(|| n.room_id.as_deref().and_then(|r| hub.bus.room_where(r).map(|(n, _)| n)));
    if let Some(node) = node {
        set_rooms(hub, s, &node, rooms).await;
    }
}
