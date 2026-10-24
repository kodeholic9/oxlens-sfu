// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-SES-GATEWAY-SCOPE · 정:IMPL-SES-DATA · 정:IMPL-SES-BIND · 정:IMPL-SES-HANDOVER · 정:IMPL-BUS-HOME-LOST · 정:CAND-SES-SUBROOMS-RECONCILE · 정:RULE-GC-FOUR · 연:SES · 연:RULE-SES-AUTH-TWO-WAYS · 연:RULE-SES-EVICT-OLD · 연:DEV-TRN-SESSION-SECRET · model: claude-opus-5-5
//! 영역 SES — 게이트웨이가 드는 세션(신원 · 토큰값 넷 · 홈 · `resumed` · 소켓 하나 · 서버별 입장 방) · 세션 폐기 길은 하나.

pub mod conn;
pub mod core;
pub mod route;
pub mod token;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use common::bus::envelope::{Call, GoneCause, Home};
use common::policy::Policy;
use common::token::UserClaims;
use oxsig::{Code, PcMode, Permission};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::bus::Bus;
use self::core::Item;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub user: String,
    pub participant_type: u8,
    pub hidden: bool,
    pub metadata: Option<serde_json::Value>,
    pub permission: Option<Permission>,
}

impl Identity {
    pub fn from_claims(c: &UserClaims) -> Option<Identity> {
        Some(Identity { user: c.user()?.to_string(), participant_type: c.participant_type, hidden: c.hidden, metadata: c.metadata.clone(), permission: c.permission })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionWire {
    pub sid: String,
    pub id: Identity,
    pub pc_mode: PcMode,
    pub servers: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug)]
pub enum Out {
    Item(Item),
    Direct(Vec<u8>),
    Ack(u16, u32),
    Leave(Code),
    Close,
}

#[derive(Debug, Clone)]
pub struct Sock {
    pub tx: mpsc::UnboundedSender<Out>,
    pub generation: u64,
}

#[derive(Debug, Default)]
pub struct State {
    pub resumed: bool,
    pub socket: Option<Sock>,
    pub servers: BTreeMap<String, BTreeSet<String>>,
    pub pending: HashMap<String, Vec<Item>>,
    pub dropped_at: Option<u64>,
    pub gone: bool,
}

#[derive(Debug)]
pub struct Session {
    pub sid: String,
    pub fp: String,
    pub id: Identity,
    pub pc_mode: PcMode,
    pub home: Home,
    pub st: Mutex<State>,
}

impl Session {
    pub fn new(sid: String, id: Identity, pc_mode: PcMode, home: Home, st: State) -> Session {
        Session { fp: common::identity::fingerprint(&sid), sid, id, pc_mode, home, st: Mutex::new(st) }
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn in_room(&self, room: &str) -> bool {
        self.lock().servers.values().any(|rs| rs.contains(room))
    }

    pub fn send(&self, out: Out) -> bool {
        match &self.lock().socket {
            Some(s) => s.tx.send(out).is_ok(),
            None => false,
        }
    }
}

pub struct Ctx {
    pub node: String,
    pub bus: Arc<Bus>,
    pub policy: Arc<Policy>,
    pub secrets: Vec<String>,
    pub sessions: Arc<Sessions>,
    pub shutting: Arc<AtomicBool>,
    pub admission: Arc<crate::room::Admission>,
    pub counters: Arc<crate::err::Counters>,
    pub generation: AtomicU64,
}

impl Ctx {
    pub fn home(&self) -> Home {
        Home { node: self.node.clone(), inst: self.bus.my_inst().unwrap_or_default() }
    }
}

#[derive(Default)]
pub struct Sessions {
    by_sid: Mutex<HashMap<String, Arc<Session>>>,
}

impl Sessions {
    pub fn get(&self, sid: &str) -> Option<Arc<Session>> {
        self.by_sid.lock().ok()?.get(sid).cloned()
    }

    pub fn of_user(&self, user: &str) -> Vec<Arc<Session>> {
        self.by_sid.lock().map(|m| m.values().filter(|s| s.id.user == user).cloned().collect()).unwrap_or_default()
    }

    pub fn all(&self) -> Vec<Arc<Session>> {
        self.by_sid.lock().map(|m| m.values().cloned().collect()).unwrap_or_default()
    }

    pub fn insert(&self, s: Arc<Session>) {
        if let Ok(mut m) = self.by_sid.lock() {
            m.insert(s.sid.clone(), s);
        }
    }

    pub fn remove(&self, sid: &str) -> Option<Arc<Session>> {
        self.by_sid.lock().ok()?.remove(sid)
    }

    pub fn len(&self) -> usize {
        self.by_sid.lock().map(|m| m.len()).unwrap_or(0)
    }
}

pub fn new_sid() -> String {
    common::identity::fresh_hex(16)
}

pub async fn create(ctx: &Ctx, sid: String, id: Identity, pc_mode: PcMode, servers: BTreeMap<String, BTreeSet<String>>, resumed: bool) -> Arc<Session> {
    let s = Arc::new(Session::new(sid, id, pc_mode, ctx.home(), State { resumed, servers: servers.clone(), ..State::default() }));
    ctx.sessions.insert(s.clone());
    ctx.bus.declare_sess(&s.id.user, &s.sid).await;
    ctx.bus.interest_user(&s.id.user).await;
    for rooms in servers.values() {
        for r in rooms {
            ctx.bus.interest_room(r).await;
        }
    }
    tracing::info!(kind = "ev", spec = if resumed { "SES.S.bind_resume" } else { "SES.S.bind_new" }, session = %s.fp, user = %s.id.user, pc_mode = s.pc_mode.as_str(), "session up");
    s
}

pub async fn detach(ctx: &Ctx, s: &Arc<Session>) {
    let rooms: Vec<String> = {
        let mut st = s.lock();
        st.gone = true;
        st.socket = None;
        st.pending.clear();
        std::mem::take(&mut st.servers).into_values().flatten().collect()
    };
    ctx.sessions.remove(&s.sid);
    ctx.bus.undeclare_sess(&s.sid).await;
    ctx.bus.release_user(&s.id.user).await;
    for r in rooms {
        ctx.bus.release_room(&r).await;
    }
}

pub async fn destroy(ctx: &Ctx, s: &Arc<Session>, cause: GoneCause) {
    let nodes: Vec<String> = s.lock().servers.keys().cloned().collect();
    detach(ctx, s).await;
    let call = Call::SessionGone { user_id: s.id.user.clone(), session: s.fp.clone(), cause };
    for n in nodes {
        let r = ctx.bus.call(&n, &call).await;
        if let common::bus::envelope::Reply::Fail(f) = r {
            tracing::warn!(kind = "diag", session = %s.fp, node = %n, code = f.code, "session gone not delivered");
        }
    }
    tracing::info!(kind = "ev", spec = match cause { GoneCause::Leave => "SES.S.leave", GoneCause::Window => "SES.S.TWindow", GoneCause::Cut | GoneCause::Shutdown => "SES.S.drop_reclaim", GoneCause::Evict => "RULE-SES-EVICT-OLD" }, session = %s.fp, user = %s.id.user, cause = cause.as_str(), "session gone");
}

pub async fn reconcile(ctx: &Ctx, s: &Arc<Session>, node: &str, sub_rooms: &[String]) {
    let (added, removed) = {
        let mut st = s.lock();
        let now: BTreeSet<String> = sub_rooms.iter().cloned().collect();
        let before = st.servers.get(node).cloned().unwrap_or_default();
        let added: Vec<String> = now.difference(&before).cloned().collect();
        let removed: Vec<String> = before.difference(&now).cloned().collect();
        if now.is_empty() {
            st.servers.remove(node);
        } else {
            st.servers.insert(node.to_string(), now);
        }
        (added, removed)
    };
    for r in added {
        ctx.bus.interest_room(&r).await;
    }
    for r in removed {
        ctx.bus.release_room(&r).await;
    }
}

pub async fn reconcile_drop_room(ctx: &Ctx, s: &Arc<Session>, room: &str) {
    let had = {
        let mut st = s.lock();
        let mut had = false;
        st.servers.retain(|_, rs| {
            had |= rs.remove(room);
            !rs.is_empty()
        });
        had
    };
    if had {
        ctx.bus.release_room(room).await;
    }
}

pub async fn sweep_windows(ctx: &Ctx) {
    let now = common::time::wall_ms();
    let window = u64::from(ctx.policy.hub.resume_window_ms);
    for s in ctx.sessions.all() {
        let expired = {
            let st = s.lock();
            st.socket.is_none() && st.dropped_at.map(|t| now.saturating_sub(t) >= window).unwrap_or(false)
        };
        if expired {
            destroy(ctx, &s, GoneCause::Window).await;
        }
    }
}

pub fn deliver(ctx: &Ctx, to: &crate::bus::Delivery, n: &common::bus::envelope::Notice) {
    let Some(op) = oxsig::Op::from_u16(n.op) else { return };
    let body = serde_json::to_vec(&n.body).unwrap_or_default();
    let targets: Vec<Arc<Session>> = match to {
        crate::bus::Delivery::Room(r) => ctx.sessions.all().into_iter().filter(|s| s.in_room(r) || s.lock().pending.contains_key(r)).collect(),
        crate::bus::Delivery::User(u) => ctx.sessions.of_user(u),
    };
    for s in targets {
        if n.exclude.iter().any(|x| x == &s.id.user) {
            continue;
        }
        let item = Item { lane: op.lane().index(), op: n.op, needs_ack: true, bytes: body.clone() };
        let room = match to {
            crate::bus::Delivery::Room(r) => Some(r.clone()),
            crate::bus::Delivery::User(_) => n.room_id.clone(),
        };
        let mut st = s.lock();
        if let Some(r) = room
            && let Some(buf) = st.pending.get_mut(&r)
        {
            buf.push(item);
            continue;
        }
        if let Some(sock) = &st.socket {
            let _ = sock.tx.send(Out::Item(item));
        }
    }
}

pub fn affiliation_rooms(v: &serde_json::Value) -> Option<Vec<String>> {
    v.get("affiliation").unwrap_or(v).get("sub_rooms")?.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
}

pub fn count(ctx: &Ctx, code: Code) {
    ctx.counters.bump(code);
}

pub fn shutting(ctx: &Ctx) -> bool {
    ctx.shutting.load(Ordering::Relaxed)
}
