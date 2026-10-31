// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-SES-DATA · 정:IMPL-SES-BIND · 정:IMPL-SES-HANDOVER · 정:IMPL-SES-RESUME · 정:IMPL-SES-ADMISSION · 정:RULE-BUS-REQUEST · 정:RULE-BUS-EVENTS · 연:SES.S.bind_new · 연:SES.S.bind_resume · 연:SES.S.resume_ok · 연:SES.S.leave · 연:SES.S.drop · 연:SES.S.TWindow · 정:RULE-SES-GATEWAY-SCOPE · 정:CAND-AFF-SPLIT-ROUTE · 정:CAND-SES-SUBROOMS-RECONCILE · 연:RULE-ERR-4009-SUPPRESS · 연:RULE-SES-SOCKET-CLOSE-NORMAL · 연:RULE-SES-SERVER-POLICY · model: claude-opus-5-5
//! 게이트웨이의 세션 — 세션 자료 · 인증 두 갈래 · 축출 · 넘겨받기 · RESUME 모으기 · op 라우팅 · 세션 폐기 한 길 · 통지 배달.

pub mod conn;
pub mod token;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use common::envelope::{CallKind, Claims, Envelope, Home, Notice, Reply};
use common::log::{Transition, bool_fields_state};
use common::policy::Policy;
use oxsig::msg::{
    Affiliation, AffiliationRes, BindReq, BindRes, MlineTable, PcMode, Publication, ResumeRes, RoomEventBody, RoomJoinRes,
    RoomLeaveRes, RoomSnapshot,
};
use oxsig::{Code, Failure, Op};
use serde::{Deserialize, Serialize};

use crate::bus::Bus;
pub use conn::{Conn, Out, Request};
use token::Accounts;

pub const SERVER_VER: u32 = 1;

#[derive(Default)]
pub struct SesState {
    pub bound: bool,
    pub resumed: bool,
    pub live: bool,
    pub conn: Option<Conn>,
    pub dropped_at: Option<u64>,
    pub servers: BTreeMap<String, BTreeSet<String>>,
    pub epochs: BTreeMap<String, String>,
    pub touched: BTreeSet<String>,
}

pub struct Session {
    pub sid: String,
    pub fp: String,
    pub user_id: String,
    pub claims: Claims,
    pub pc_mode: PcMode,
    pub created_at: u64,
    pub st: Mutex<SesState>,
}

impl Session {
    fn state_name(&self, full: bool) -> String {
        let (live, bound, rooms, resumed) = match self.st.lock() {
            Ok(s) => (s.live, s.bound, s.servers.values().any(|r| !r.is_empty()), s.resumed),
            Err(_) => (false, false, false, false),
        };
        bool_fields_state(&[
            ("hidden", self.claims.hidden),
            ("live", live),
            ("bound", bound),
            ("rooms", rooms),
            ("full", full),
            ("resumed", resumed),
        ])
    }

    pub fn conn(&self) -> Option<Conn> {
        self.st.lock().ok().and_then(|s| s.conn.clone())
    }

    pub fn rooms(&self) -> Vec<(String, String)> {
        self.st
            .lock()
            .map(|s| s.servers.iter().flat_map(|(n, rs)| rs.iter().map(move |r| (n.clone(), r.clone()))).collect())
            .unwrap_or_default()
    }

    pub fn in_room(&self, room: &str) -> bool {
        self.st.lock().map(|s| s.servers.values().any(|rs| rs.contains(room))).unwrap_or(false)
    }

    pub fn epochs(&self) -> BTreeMap<String, String> {
        self.st.lock().map(|s| s.epochs.clone()).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionData {
    pub sid: String,
    pub user_id: String,
    pub claims: Claims,
    pub pc_mode: PcMode,
    pub servers: BTreeMap<String, BTreeSet<String>>,
    pub epochs: BTreeMap<String, String>,
    pub touched: BTreeSet<String>,
}

#[derive(Default)]
pub struct Counters {
    pub notice_no_socket: AtomicU64,
    pub codes: Mutex<BTreeMap<u16, u64>>,
}

impl Counters {
    pub fn code(&self, c: u16) {
        if let Ok(mut g) = self.codes.lock() {
            g.entry(c).and_modify(|x| *x += 1).or_insert(1);
        }
    }
}

pub struct Ctx {
    pub policy: Policy,
    pub accounts: Accounts,
    pub bus: Arc<Bus>,
    pub home: Home,
    pub by_sid: Mutex<HashMap<String, Arc<Session>>>,
    pub by_user: Mutex<HashMap<String, String>>,
    pub counters: Counters,
    pub full: AtomicBool,
    pub admit: Mutex<(u64, u32)>,
    pub probes: crate::aux::Probes,
    pub shutting_down: AtomicBool,
}

impl Ctx {
    pub fn new(policy: Policy, accounts: Accounts, bus: Arc<Bus>) -> Arc<Ctx> {
        let home = Home { node: bus.node.clone(), inst: bus.inst.clone() };
        Arc::new(Ctx {
            policy,
            accounts,
            bus,
            home,
            by_sid: Mutex::new(HashMap::new()),
            by_user: Mutex::new(HashMap::new()),
            counters: Counters::default(),
            full: AtomicBool::new(false),
            admit: Mutex::new((0, 0)),
            probes: crate::aux::Probes::default(),
            shutting_down: AtomicBool::new(false),
        })
    }

    pub fn sessions(&self) -> Vec<Arc<Session>> {
        self.by_sid.lock().map(|g| g.values().cloned().collect()).unwrap_or_default()
    }

    pub fn session(&self, sid: &str) -> Option<Arc<Session>> {
        self.by_sid.lock().ok().and_then(|g| g.get(sid).cloned())
    }

    pub fn session_of_user(&self, user: &str) -> Option<Arc<Session>> {
        let sid = self.by_user.lock().ok().and_then(|g| g.get(user).cloned())?;
        self.session(&sid)
    }

    fn full_flag(&self) -> bool {
        self.full.load(Ordering::Relaxed)
    }

    pub fn envelope(&self, s: &Session, kind: CallKind) -> Envelope {
        let mut e = Envelope::new(kind);
        e.user_id = Some(s.user_id.clone());
        e.session = Some(s.fp.clone());
        e.sid = Some(s.sid.clone());
        e.pc_mode = Some(s.pc_mode);
        e.home = Some(self.home.clone());
        e.claims = Some(s.claims.clone());
        e
    }
}

fn transition(s: &Session, spec: Option<&str>, from: &str, to: Option<&str>, cause: &str, inputs: &[(&str, String)]) {
    let t = Transition { area: "SES", key: &s.fp, spec, from, to, cause, inputs };
    common::log::emit(t.line().key("session", &s.fp).key("user", &s.user_id));
}

pub fn pre_bind_other(_ctx: &Ctx, op: Option<Op>) {
    let cause = op.map(|o| o.name()).unwrap_or("UNKNOWN");
    let t = Transition { area: "SES", key: "-", spec: Some("SES.S.pre_bind_other"), from: "-", to: Some("-"), cause, inputs: &[] };
    common::log::emit(t.line());
}

pub fn bind_bound(ctx: &Ctx, s: &Session) {
    let st = s.state_name(ctx.full_flag());
    transition(s, Some("SES.S.bind_bound"), &st, Some(&st), "BIND", &[]);
}

fn fail_bind(ctx: &Ctx, conn: &Conn, op: u16, pid: u32, flags: u8, f: Failure, label: Option<&str>) {
    ctx.counters.code(f.code);
    if let Some(l) = label {
        let t = Transition { area: "SES", key: "-", spec: Some(l), from: "-", to: Some("-"), cause: "BIND", inputs: &[] };
        common::log::emit(t.line());
    }
    conn.reply_fail(op, pid, flags, &f);
}

pub async fn bind(ctx: &Arc<Ctx>, conn: &Conn, op: u16, pid: u32, flags: u8, body: &[u8]) -> Option<Arc<Session>> {
    let req: BindReq = match oxsig::frame::parse_body(body) {
        Ok(r) => r,
        Err(_) => {
            fail_bind(ctx, conn, op, pid, flags, Code::InvalidPayload.failure(), None);
            return None;
        }
    };
    if req.client_ver.unwrap_or(1) != 1 {
        fail_bind(ctx, conn, op, pid, flags, Code::VersionMismatch.failure(), None);
        return None;
    }
    if let Some(sid) = req.session_id.as_deref().filter(|s| !s.is_empty())
        && let Some(s) = resume_session(ctx, sid).await
    {
        let from = s.state_name(ctx.full_flag());
        let old = {
            let Ok(mut st) = s.st.lock() else { return None };
            let old = st.conn.replace(conn.clone());
            st.bound = true;
            st.resumed = true;
            st.live = true;
            st.dropped_at = None;
            old
        };
        if let Some(o) = old
            && o.id != conn.id
        {
            o.leave(Code::DuplicateSession);
        }
        let to = s.state_name(ctx.full_flag());
        transition(&s, Some("SES.S.bind_resume"), &from, Some(&to), "BIND", &[]);
        reply_bind(ctx, conn, op, pid, flags, &s);
        return Some(s);
    }
    let Some(token) = req.token.as_deref() else {
        fail_bind(ctx, conn, op, pid, flags, Code::MissingField.failure().with_message("token"), None);
        return None;
    };
    let id = match ctx.accounts.verify_user(token) {
        Ok(i) => i,
        Err(c) => {
            let label = if c == Code::TokenExpired { "SES.S.bind_2003" } else { "SES.S.bind_2002" };
            fail_bind(ctx, conn, op, pid, flags, c.failure(), Some(label));
            return None;
        }
    };
    if ctx.policy.quota.max_sessions > 0 {
        let local = ctx.by_sid.lock().map(|g| g.values().filter(|s| s.user_id != id.user_id).count()).unwrap_or(0) as u32;
        let deploy = ctx.bus.session_users().await.into_iter().filter(|u| u != &id.user_id).count() as u32;
        let n = local.max(deploy);
        ctx.full.store(n >= ctx.policy.quota.max_sessions, Ordering::Relaxed);
        if ctx.full_flag() {
            fail_bind(ctx, conn, op, pid, flags, Code::QuotaExceeded.failure(), Some("SES.S.bind_4003"));
            return None;
        }
    }
    if let Some(old) = ctx.session_of_user(&id.user_id) {
        if let Some(c) = old.conn() {
            c.leave(Code::DuplicateSession);
        }
        dispose(ctx, &old, "SES.S.bind_new", "evict").await;
    }
    for (node, sid) in ctx.bus.sessions_of_user(&id.user_id).await {
        if node != ctx.home.node {
            let _ = ctx.bus.query(&node, "evict", serde_json::to_vec(&serde_json::json!({ "sid": sid })).unwrap_or_default()).await;
        }
    }
    let sid = common::random_hex(16);
    let s = Arc::new(Session {
        fp: common::log::fingerprint(&sid),
        sid: sid.clone(),
        user_id: id.user_id.clone(),
        claims: id.claims,
        pc_mode: req.pc_mode.unwrap_or(PcMode::Two),
        created_at: common::now_ms(),
        st: Mutex::new(SesState { bound: true, live: true, conn: Some(conn.clone()), ..Default::default() }),
    });
    register(ctx, &s).await;
    let to = s.state_name(ctx.full_flag());
    transition(&s, Some("SES.S.bind_new"), "-", Some(&to), "BIND", &[]);
    reply_bind(ctx, conn, op, pid, flags, &s);
    Some(s)
}

async fn register(ctx: &Ctx, s: &Arc<Session>) {
    ctx.bus.interest_user(&s.user_id, 1).await;
    ctx.bus.declare_sess(&s.user_id, &s.sid).await;
    if let Ok(mut g) = ctx.by_sid.lock() {
        g.insert(s.sid.clone(), s.clone());
    }
    if let Ok(mut g) = ctx.by_user.lock() {
        g.insert(s.user_id.clone(), s.sid.clone());
    }
}

fn reply_bind(ctx: &Ctx, conn: &Conn, op: u16, pid: u32, flags: u8, s: &Session) {
    let res = BindRes {
        user_id: s.user_id.clone(),
        server_ver: SERVER_VER,
        heartbeat_interval: ctx.policy.hub.heartbeat_interval_ms as u64,
        session_id: s.sid.clone(),
        resume_window_ms: ctx.policy.hub.resume_window_ms as u64,
        pc_mode: s.pc_mode,
    };
    common::log::emit(common::log::Line::ev("CP-OP-BIND").key("session", &s.fp).key("user", &s.user_id).msg("bound"));
    conn.reply_ok(op, pid, flags, serde_json::to_vec(&res).unwrap_or_default());
}

async fn resume_session(ctx: &Arc<Ctx>, sid: &str) -> Option<Arc<Session>> {
    if let Some(s) = ctx.session(sid) {
        return Some(s);
    }
    let (_, node) = ctx.bus.find_sess(sid).await?;
    if node == ctx.home.node {
        return None;
    }
    let bytes = ctx.bus.query(&node, "sess", serde_json::to_vec(&serde_json::json!({ "sid": sid })).ok()?).await.ok()?;
    let d: SessionData = serde_json::from_slice(&bytes).ok()?;
    let s = Arc::new(Session {
        fp: common::log::fingerprint(&d.sid),
        sid: d.sid,
        user_id: d.user_id,
        claims: d.claims,
        pc_mode: d.pc_mode,
        created_at: common::now_ms(),
        st: Mutex::new(SesState { servers: d.servers.clone(), epochs: d.epochs, touched: d.touched, ..Default::default() }),
    });
    register(ctx, &s).await;
    for rooms in d.servers.values() {
        for r in rooms {
            ctx.bus.interest_room(r, 1).await;
        }
    }
    Some(s)
}

pub async fn hand_over(ctx: &Arc<Ctx>, sid: &str) -> Option<SessionData> {
    let s = ctx.session(sid)?;
    if let Some(c) = s.conn() {
        c.leave(Code::DuplicateSession);
    }
    let d = s.st.lock().ok().map(|st| SessionData {
        sid: s.sid.clone(),
        user_id: s.user_id.clone(),
        claims: s.claims.clone(),
        pc_mode: s.pc_mode,
        servers: st.servers.clone(),
        epochs: st.epochs.clone(),
        touched: st.touched.clone(),
    })?;
    if let Ok(mut st) = s.st.lock() {
        st.conn = None;
    }
    unregister(ctx, &s).await;
    Some(d)
}

async fn unregister(ctx: &Ctx, s: &Session) {
    let removed = ctx.by_sid.lock().map(|mut g| g.remove(&s.sid).is_some()).unwrap_or(false);
    if !removed {
        return;
    }
    if let Ok(mut g) = ctx.by_user.lock()
        && g.get(&s.user_id) == Some(&s.sid)
    {
        g.remove(&s.user_id);
    }
    ctx.bus.undeclare_sess(&s.sid);
    ctx.bus.interest_user(&s.user_id, -1).await;
    let rooms: Vec<String> = s.rooms().into_iter().map(|(_, r)| r).collect();
    for r in rooms {
        ctx.bus.interest_room(&r, -1).await;
    }
}

pub async fn dispose(ctx: &Arc<Ctx>, s: &Arc<Session>, label: &str, cause: &str) {
    let from = s.state_name(ctx.full_flag());
    let nodes: Vec<String> = s.st.lock().map(|st| st.touched.iter().cloned().collect()).unwrap_or_default();
    unregister(ctx, s).await;
    if let Ok(mut st) = s.st.lock() {
        st.live = false;
        st.bound = false;
        st.resumed = false;
        st.conn = None;
        st.servers.clear();
    }
    if label != "SES.S.bind_new" {
        transition(s, Some(label), &from, Some("-"), cause, &[]);
    }
    let env = ctx.envelope(s, CallKind::SessionGone);
    for n in nodes {
        let bus = ctx.bus.clone();
        let env = env.clone();
        tokio::spawn(async move {
            let _ = bus.call(&n, &env).await;
        });
    }
}

pub async fn client_leave(ctx: &Arc<Ctx>, s: &Arc<Session>, _conn: &Conn) {
    dispose(ctx, s, "SES.S.leave", "LEAVE").await;
}

pub async fn socket_dropped(ctx: &Arc<Ctx>, s: &Arc<Session>, conn_id: u64, code: Option<Code>) {
    let mine = s.st.lock().map(|st| st.conn.as_ref().map(|c| c.id == conn_id).unwrap_or(false)).unwrap_or(false);
    if !mine || ctx.session(&s.sid).is_none() {
        return;
    }
    let c = code.map(|c| c.num()).unwrap_or(0);
    if c == 2009 || c == 5004 {
        dispose(ctx, s, "SES.S.drop_reclaim", "drop").await;
        return;
    }
    let from = s.state_name(ctx.full_flag());
    if let Ok(mut st) = s.st.lock() {
        st.bound = false;
        st.conn = None;
        st.dropped_at = Some(common::now_ms());
    }
    let to = s.state_name(ctx.full_flag());
    transition(s, Some("SES.S.drop"), &from, Some(&to), "drop", &[("c", c.to_string())]);
}

pub async fn sweep(ctx: &Arc<Ctx>) {
    let now = common::now_ms();
    let win = ctx.policy.hub.resume_window_ms as u64;
    let due: Vec<Arc<Session>> = ctx
        .sessions()
        .into_iter()
        .filter(|s| s.st.lock().map(|st| !st.bound && st.dropped_at.map(|d| now >= d + win).unwrap_or(false)).unwrap_or(false))
        .collect();
    for s in due {
        dispose(ctx, &s, "SES.S.TWindow", "TWindow").await;
    }
}

pub async fn evict_sid(ctx: &Arc<Ctx>, sid: &str) {
    if let Some(s) = ctx.session(sid) {
        if let Some(c) = s.conn() {
            c.leave(Code::DuplicateSession);
        }
        dispose(ctx, &s, "SES.S.bind_new", "evict").await;
    }
}

pub async fn cut(ctx: &Arc<Ctx>, user: &str) -> Vec<String> {
    let Some(s) = ctx.session_of_user(user) else { return Vec::new() };
    if let Some(c) = s.conn() {
        c.leave(Code::SessionRevoked);
    }
    dispose(ctx, &s, "SES.S.drop_reclaim", "cut").await;
    vec![s.fp.clone()]
}

pub fn shutdown_all(ctx: &Arc<Ctx>) -> usize {
    ctx.shutting_down.store(true, Ordering::Relaxed);
    let mut n = 0;
    for s in ctx.sessions() {
        if let Some(c) = s.conn() {
            c.leave(Code::ServerShutdown);
            n += 1;
        }
    }
    n
}

fn room_of(body: &serde_json::Value) -> Result<String, Failure> {
    match body.get("room_id") {
        None | Some(serde_json::Value::Null) => Err(Code::MissingField.failure().with_message("room_id")),
        Some(serde_json::Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(Code::InvalidPayload.failure().with_message("room_id")),
    }
}

pub fn valid_room_id(r: &str) -> bool {
    crate::room::valid_room_id(r)
}

async fn set_rooms(ctx: &Ctx, s: &Session, node: &str, rooms: &[String]) {
    let new: BTreeSet<String> = rooms.iter().cloned().collect();
    let old = s.st.lock().map(|mut st| st.servers.insert(node.to_string(), new.clone()).unwrap_or_default()).unwrap_or_default();
    for r in new.difference(&old) {
        ctx.bus.interest_room(r, 1).await;
    }
    for r in old.difference(&new) {
        ctx.bus.interest_room(r, -1).await;
    }
}

fn admit(ctx: &Ctx) -> Result<Option<u64>, Failure> {
    let lim = ctx.policy.hub.join_admit_per_sec;
    if lim == 0 {
        return Ok(None);
    }
    let sec = common::now_ms() / 1000;
    let mut g = ctx.admit.lock().map_err(|_| Code::InternalError.failure())?;
    if g.0 != sec {
        g.0 = sec;
        g.1 = 0;
    }
    g.1 += 1;
    if g.1 <= lim {
        return Ok(None);
    }
    let base = ctx.policy.hub.join_retry_after_ms as u64;
    let jitter = u64::from_be_bytes(common::random_bytes(8).try_into().unwrap_or([0; 8])) % base.max(1);
    Ok(Some(base + jitter))
}

pub async fn handle_request(ctx: &Arc<Ctx>, s: &Arc<Session>, conn: &Conn, r: Request) {
    let body = match oxsig::frame::body_value(&r.body) {
        Ok(b) => b,
        Err(_) => {
            conn.reply_fail(r.op_num, r.pid, r.flags, &Code::InvalidPayload.failure());
            return;
        }
    };
    match route(ctx, s, r.op, body).await {
        Ok(v) => {
            common::log::emit(
                common::log::Line::ev(&format!("CP-OP-{}", r.op.name().replace('_', "-")))
                    .key("session", &s.fp)
                    .key("user", &s.user_id)
                    .msg("ok"),
            );
            let bytes = if v.is_null() { Vec::new() } else { serde_json::to_vec(&v).unwrap_or_default() };
            conn.reply_ok(r.op_num, r.pid, r.flags, bytes);
        }
        Err(f) => {
            ctx.counters.code(f.code);
            common::log::emit(
                common::log::Line::ev(&format!("CP-CODE-{}", f.name.replace('_', "-")))
                    .key("session", &s.fp)
                    .key("user", &s.user_id)
                    .rec("op", r.op.name())
                    .msg(f.message.clone().unwrap_or_default()),
            );
            conn.reply_fail(r.op_num, r.pid, r.flags, &f);
        }
    }
}

async fn call_node(ctx: &Ctx, s: &Session, node: &str, op: Op, body: serde_json::Value) -> Result<serde_json::Value, Failure> {
    call_node_over(ctx, s, node, op, body, None).await
}

async fn call_node_over(
    ctx: &Ctx,
    s: &Session,
    node: &str,
    op: Op,
    body: serde_json::Value,
    over: Option<u64>,
) -> Result<serde_json::Value, Failure> {
    if let Ok(mut st) = s.st.lock() {
        st.touched.insert(node.to_string());
    }
    let mut env = ctx.envelope(s, CallKind::Op);
    env.op = Some(op.as_u16());
    env.body = body;
    env.admit_retry_after_ms = over;
    match ctx.bus.call(node, &env).await {
        Reply::Ok { body } => Ok(body),
        Reply::Fail { failure } => Err(failure),
        Reply::Http { .. } => Err(Code::SfuError.failure()),
    }
}

async fn route(ctx: &Arc<Ctx>, s: &Arc<Session>, op: Op, body: serde_json::Value) -> Result<serde_json::Value, Failure> {
    match op {
        Op::Resume => resume(ctx, s).await,
        Op::Task => crate::aux::task_from_client(&ctx.probes, &s.fp, &body),
        Op::IceRestart => {
            let sfu: String = oxsig::code::need(&body, "sfu_id")?;
            let node = ctx.bus.node_of_epoch(&sfu).ok_or_else(|| Code::PeerNotFound.failure())?;
            call_node(ctx, s, &node, op, body).await
        }
        Op::Affiliation => {
            let sel: Option<String> = oxsig::code::field(&body, "pub_select")?;
            let de: Option<String> = oxsig::code::field(&body, "pub_deselect")?;
            let room = sel.or(de).ok_or_else(|| Code::MissingField.failure().with_message("pub_select/pub_deselect"))?;
            if !valid_room_id(&room) {
                return Err(Code::InvalidPayload.failure().with_message("room_id"));
            }
            let loc = ctx.bus.room_route(&room).ok_or_else(|| Code::RoomNotFound.failure())?;
            let v = call_node(ctx, s, &loc.node, op, body).await?;
            if let Ok(a) = serde_json::from_value::<AffiliationRes>(v.clone()) {
                set_rooms(ctx, s, &loc.node, &a.sub_rooms).await;
            }
            Ok(v)
        }
        Op::RoomJoin => {
            let room = room_of(&body)?;
            if !valid_room_id(&room) {
                return Err(Code::InvalidPayload.failure().with_message("room_id"));
            }
            let has = |k: &str| body.get(k).map(|v| !v.is_null()).unwrap_or(false);
            match s.pc_mode {
                PcMode::One if !has("extmap") => return Err(Code::MissingField.failure().with_message("extmap")),
                PcMode::One if !has("codecs") => return Err(Code::MissingField.failure().with_message("codecs")),
                PcMode::Two if has("extmap") || has("codecs") => return Err(Code::InvalidPayload.failure().with_message("extmap/codecs in 2pc")),
                _ => {}
            }
            let over = admit(ctx)?;
            let loc = ctx.bus.room_route(&room).ok_or_else(|| Code::RoomNotFound.failure())?;
            ctx.bus.interest_room(&room, 1).await;
            let r = call_node_over(ctx, s, &loc.node, op, body, over).await;
            if let Ok(v) = &r
                && let Ok(j) = serde_json::from_value::<RoomJoinRes>(v.clone())
            {
                if let Ok(mut st) = s.st.lock() {
                    st.epochs.insert(loc.node.clone(), j.server_config.sfu_id.clone());
                }
                set_rooms(ctx, s, &loc.node, &j.affiliation.sub_rooms).await;
            }
            ctx.bus.interest_room(&room, -1).await;
            r
        }
        Op::RoomLeave => {
            let room = room_of(&body)?;
            if !valid_room_id(&room) {
                return Err(Code::InvalidPayload.failure().with_message("room_id"));
            }
            let loc = ctx.bus.room_route(&room).ok_or_else(|| Code::RoomNotFound.failure())?;
            let v = call_node(ctx, s, &loc.node, op, body).await?;
            if let Ok(l) = serde_json::from_value::<RoomLeaveRes>(v.clone()) {
                set_rooms(ctx, s, &loc.node, &l.affiliation.sub_rooms).await;
            }
            Ok(v)
        }
        Op::PublishTracks | Op::Ready | Op::SubscribeLayer | Op::TrackSet | Op::Message => {
            let room = room_of(&body)?;
            if !valid_room_id(&room) {
                return Err(Code::InvalidPayload.failure().with_message("room_id"));
            }
            let loc = ctx.bus.room_route(&room).ok_or_else(|| Code::RoomNotFound.failure())?;
            call_node(ctx, s, &loc.node, op, body).await
        }
        _ => Err(Code::UnknownOp.failure()),
    }
}

#[derive(Deserialize)]
struct ResumePart {
    snapshot: BTreeMap<String, RoomSnapshot>,
    publications: Vec<Publication>,
    #[serde(default)]
    mline_table: Option<MlineTable>,
}

async fn resume(ctx: &Arc<Ctx>, s: &Arc<Session>) -> Result<serde_json::Value, Failure> {
    let from = s.state_name(ctx.full_flag());
    let ok = s.st.lock().map(|st| st.bound && st.live && st.resumed).unwrap_or(false);
    if !ok {
        transition(s, Some("SES.S.resume_2008"), &from, Some(&from), "RESUME", &[]);
        return Err(Code::SessionNotFound.failure());
    }
    let nodes: Vec<String> = s
        .st
        .lock()
        .map(|st| st.servers.iter().filter(|(_, r)| !r.is_empty()).map(|(n, _)| n.clone()).collect())
        .unwrap_or_default();
    let mut out = ResumeRes { snapshot: BTreeMap::new(), publications: Vec::new(), mline_tables: Vec::new() };
    for n in nodes {
        let env = ctx.envelope(s, CallKind::Resume);
        match ctx.bus.call(&n, &env).await {
            Reply::Ok { body } => {
                let p: ResumePart = serde_json::from_value(body).map_err(|_| Code::SfuError.failure())?;
                let rooms: Vec<String> = p.snapshot.keys().cloned().collect();
                set_rooms(ctx, s, &n, &rooms).await;
                out.snapshot.extend(p.snapshot);
                out.publications.extend(p.publications);
                if let Some(t) = p.mline_table {
                    out.mline_tables.push(t);
                }
            }
            _ => return Err(Code::SfuError.failure()),
        }
    }
    transition(s, Some("SES.S.resume_ok"), &from, Some(&from), "RESUME", &[]);
    serde_json::to_value(out).map_err(|_| Code::InternalError.failure())
}

pub fn deliver_room(ctx: &Ctx, room: &str, n: &Notice) {
    for s in ctx.sessions() {
        if !s.in_room(room) || n.exclude.iter().any(|u| u == &s.user_id) {
            continue;
        }
        push_notice(ctx, &s, n);
    }
}

pub async fn deliver_user(ctx: &Ctx, user: &str, n: &Notice) {
    let Some(s) = ctx.session_of_user(user) else { return };
    if n.op == Op::RoomEvent.as_u16()
        && let Ok(RoomEventBody::Affiliation { affiliation, .. }) = serde_json::from_value::<RoomEventBody>(n.body.clone())
    {
        apply_affiliation(ctx, &s, n.sfu_id.as_deref(), n.room_id.as_deref(), &affiliation).await;
    }
    push_notice(ctx, &s, n);
}

async fn apply_affiliation(ctx: &Ctx, s: &Session, sfu: Option<&str>, room: Option<&str>, a: &Affiliation) {
    let by_epoch = sfu.and_then(|e| s.epochs().into_iter().find(|(_, v)| v == e).map(|(k, _)| k));
    let node = by_epoch.or_else(|| sfu.and_then(|e| ctx.bus.node_of_epoch(e))).or_else(|| {
        room.and_then(|r| s.rooms().into_iter().find(|(_, rr)| rr == r).map(|(n, _)| n))
    });
    if let Some(n) = node {
        set_rooms(ctx, s, &n, &a.sub_rooms).await;
    }
}

fn push_notice(ctx: &Ctx, s: &Session, n: &Notice) {
    let Some(op) = Op::from_u16(n.op) else { return };
    match s.conn() {
        Some(c) => {
            let _ = c.send(Out::Notice { op, body: serde_json::to_vec(&n.body).unwrap_or_default() });
        }
        None => {
            ctx.counters.notice_no_socket.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub async fn room_dropped(ctx: &Ctx, room: &str) {
    for s in ctx.sessions() {
        let had = s.st.lock().map(|mut st| st.servers.values_mut().fold(false, |acc, rs| rs.remove(room) || acc)).unwrap_or(false);
        if had {
            ctx.bus.interest_room(room, -1).await;
        }
    }
}
