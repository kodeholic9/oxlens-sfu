// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-SES-BIND 정:IMPL-SES-HANDOVER 정:IMPL-SES-RESUME 정:IMPL-SES-DATA 정:IMPL-SES-ADMISSION 연:SES.op.BIND 연:SES.op.RESUME 연:RULE-SES-BIND-PROC 연:RULE-SES-AUTH-TWO-WAYS 연:RULE-SES-EVICT-OLD 연:RULE-SES-PC-MODE-SESSION 연:RULE-SES-RESUME-RES-TWO 연:RULE-SES-RESUME-NO-SESSION-ID 연:SES.S.bind_2002 연:SES.S.bind_2003 연:SES.S.bind_4003 · model: claude-opus-5-5
//! `BIND` 두 갈래(`session_id` 가 이긴다 · 토큰) · 다른 node 의 세션 넘겨받기 · 같은 신원 축출 · `RESUME` 스냅샷 모으기.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use oxsig::Code;
use oxsig::envelope::{Call, Reply};
use oxsig::msg::{BindReq, BindRes, MlineTable, PcMode, Publication, ResumeRes, RoomSnapshot};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{ConnHandle, Gone, Identity, Inner, SessData, Session, destroy};
use crate::ops::hub::Hub;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "q", rename_all = "snake_case")]
pub enum HubQ {
    Sess { sid: String },
    Evict { user: String },
    Cut { user: String },
    Probe { user: String },
    Users,
    Bus,
    Drops,
}

pub struct Bound {
    pub session: Arc<Session>,
    pub res: BindRes,
}

pub fn parse_bind(body: &[u8]) -> Result<BindReq, Code> {
    let v: serde_json::Value = if body.is_empty() { serde_json::json!({}) } else { serde_json::from_slice(body).map_err(|_| Code::InvalidPayload)? };
    if v.get("token").and_then(|t| t.as_str()).is_none() && v.get("session_id").and_then(|t| t.as_str()).is_none() {
        return Err(Code::MissingField);
    }
    if let Some(p) = v.get("pc_mode")
        && !matches!(p.as_str(), Some("1pc") | Some("2pc"))
    {
        return Err(Code::InvalidPayload);
    }
    let mut v = v;
    if v.get("token").is_none() {
        v["token"] = serde_json::Value::String(String::new());
    }
    let req: BindReq = serde_json::from_value(v).map_err(|_| Code::InvalidPayload)?;
    if req.client_ver.is_some_and(|c| c != 1) {
        return Err(Code::VersionMismatch);
    }
    Ok(req)
}

fn res(hub: &Hub, s: &Session) -> BindRes {
    BindRes {
        user_id: s.id.user.clone(),
        server_ver: 1,
        heartbeat_interval: u64::from(hub.cfg.policy.hub.heartbeat_interval_ms),
        session_id: s.sid.clone(),
        resume_window_ms: u64::from(hub.cfg.policy.hub.resume_window_ms),
        pc_mode: s.id.pc_mode,
    }
}

async fn attach(s: &Arc<Session>, conn: ConnHandle) -> bool {
    let old = {
        let mut g = s.inner.lock().await;
        if g.dead {
            return false;
        }
        g.gone_at = None;
        g.conn.replace(conn)
    };
    if let Some(o) = old {
        let _ = o.tx.send(super::Out::Leave(Code::DuplicateSession));
    }
    true
}

pub async fn bind(hub: &Arc<Hub>, req: BindReq, conn_id: u64, tx: tokio::sync::mpsc::UnboundedSender<super::Out>) -> Result<Bound, Code> {
    if let Some(sid) = req.session_id.as_deref().filter(|s| !s.is_empty()) {
        if let Some(s) = hub.ses.get(sid)
            && attach(&s, ConnHandle { id: conn_id, tx: tx.clone(), resumed: true }).await
        {
            common::ev!(spec = "CP-OP-BIND", session = s.fp.as_str(), user = s.id.user.as_str(), "bind resume (session_id) — token not checked");
            return Ok(Bound { res: res(hub, &s), session: s });
        }
        if let Some(s) = handover(hub, sid).await
            && attach(&s, ConnHandle { id: conn_id, tx: tx.clone(), resumed: true }).await
        {
            common::ev!(spec = "CP-OP-BIND", session = s.fp.as_str(), user = s.id.user.as_str(), "bind resume — handed over from another node");
            return Ok(Bound { res: res(hub, &s), session: s });
        }
    }
    let claims = common::jwt::verify_user(&req.token, &hub.cfg.sys)?;
    let user = claims.user().ok_or(Code::TokenInvalid)?.to_string();
    let max = hub.cfg.policy.quota.max_sessions;
    if max > 0 && hub.ses.count() as u32 >= max && hub.ses.of_user(&user).is_none() {
        return Err(Code::QuotaExceeded);
    }
    evict_user(hub, &user).await;
    let sid = common::id::session_id();
    let s = Arc::new(Session {
        fp: common::log::fingerprint(&sid),
        sid: sid.clone(),
        id: Identity {
            user: user.clone(),
            ptype: claims.participant_type,
            hidden: claims.hidden,
            metadata: claims.metadata.clone(),
            permission: claims.permission.clone(),
            pc_mode: req.pc_mode.unwrap_or(PcMode::V2pc),
        },
        inner: Mutex::new(Inner { conn: Some(ConnHandle { id: conn_id, tx, resumed: false }), ..Default::default() }),
    });
    hub.ses.by_sid.insert(sid.clone(), s.clone());
    hub.ses.by_user.insert(user.clone(), sid.clone());
    if let Err(e) = hub.bus.watch_user(&user).await {
        tracing::warn!(user = user.as_str(), "ev/user: {e}");
    }
    if let Err(e) = hub.bus.declare_sess(&user, &sid).await {
        tracing::warn!(user = user.as_str(), "sess token: {e}");
    }
    common::ev!(spec = "CP-OP-BIND", session = s.fp.as_str(), user = user.as_str(), "bind new session pc_mode={:?}", s.id.pc_mode);
    Ok(Bound { res: res(hub, &s), session: s })
}

pub async fn evict_user(hub: &Arc<Hub>, user: &str) {
    if let Some(old) = hub.ses.of_user(user) {
        destroy(hub, &old, Gone::Evicted, Some(Code::DuplicateSession)).await;
    }
    for (node, _) in hub.bus.sess_of_user(user).await {
        if hub.is_local(&node) {
            continue;
        }
        if let Ok(b) = serde_json::to_vec(&HubQ::Evict { user: user.to_string() }) {
            let _ = hub.bus.ask(&node, "hub", b, common::bus::NODE_CALL_TIMEOUT).await;
        }
    }
}

async fn handover(hub: &Arc<Hub>, sid: &str) -> Option<Arc<Session>> {
    let (user, node) = hub.bus.sess_by_sid(sid).await?;
    if hub.is_local(&node) {
        return None;
    }
    let body = serde_json::to_vec(&HubQ::Sess { sid: sid.to_string() }).ok()?;
    let bytes = hub.bus.ask(&node, "hub", body, common::bus::NODE_CALL_TIMEOUT).await.ok()?;
    let d: SessData = serde_json::from_slice::<Option<SessData>>(&bytes).ok()??;
    if d.id.user != user {
        return None;
    }
    let s = Arc::new(Session { fp: common::log::fingerprint(&d.sid), sid: d.sid.clone(), id: d.id.clone(), inner: Mutex::new(Inner::default()) });
    hub.ses.by_sid.insert(d.sid.clone(), s.clone());
    hub.ses.by_user.insert(d.id.user.clone(), d.sid.clone());
    let _ = hub.bus.watch_user(&d.id.user).await;
    let _ = hub.bus.declare_sess(&d.id.user, &d.sid).await;
    for (n, rs) in d.rooms {
        super::set_rooms(hub, &s, &n, rs).await;
    }
    Some(s)
}

pub async fn hand_out(hub: &Arc<Hub>, sid: &str) -> Option<SessData> {
    let s = hub.ses.get(sid)?;
    let (rooms, conn) = {
        let mut g = s.inner.lock().await;
        if g.dead {
            return None;
        }
        g.dead = true;
        (std::mem::take(&mut g.rooms), g.conn.take())
    };
    if let Some(c) = conn {
        let _ = c.tx.send(super::Out::Leave(Code::DuplicateSession));
    }
    hub.ses.by_sid.remove(sid);
    hub.ses.by_user.remove_if(&s.id.user, |_, v| v == sid);
    for rs in rooms.values() {
        for r in rs {
            if let Some(mut e) = hub.ses.by_room.get_mut(r) {
                e.remove(sid);
            }
            hub.ses.by_room.remove_if(r, |_, v| v.is_empty());
            let _ = hub.bus.room_interest(r, -1).await;
        }
    }
    hub.bus.undeclare_sess(sid).await;
    if hub.ses.of_user(&s.id.user).is_none() {
        hub.bus.unwatch_user(&s.id.user).await;
    }
    common::ev!(spec = "CP-OP-BIND", session = s.fp.as_str(), user = s.id.user.as_str(), "session handed over to another node");
    Some(SessData { sid: s.sid.clone(), id: s.id.clone(), rooms })
}

pub async fn resume(hub: &Arc<Hub>, s: &Arc<Session>) -> Result<ResumeRes, Code> {
    let resumed = s.inner.lock().await.conn.as_ref().is_some_and(|c| c.resumed);
    if !resumed {
        return Err(Code::SessionNotFound);
    }
    let mut snapshot: BTreeMap<String, RoomSnapshot> = BTreeMap::new();
    let mut publications: Vec<Publication> = Vec::new();
    let mut mline_tables: Vec<MlineTable> = Vec::new();
    for node in s.nodes().await {
        let call = Call::Resume { ctx: s.ctx(hub) };
        match hub.call_node(&node, None, &call).await {
            Reply::Ok { body } => {
                if let Ok(r) = serde_json::from_str::<ResumeRes>(&body) {
                    let keys: BTreeSet<String> = r.snapshot.keys().cloned().collect();
                    super::set_rooms(hub, s, &node, keys).await;
                    snapshot.extend(r.snapshot);
                    publications.extend(r.publications);
                    mline_tables.extend(r.mline_tables);
                }
            }
            Reply::Fail { failure } => {
                tracing::warn!(session = s.fp.as_str(), "resume: node {node} failed {}", failure.code);
                if failure.code == Code::PeerNotFound.num() {
                    super::set_rooms(hub, s, &node, BTreeSet::new()).await;
                }
            }
            Reply::Http { .. } => {}
        }
    }
    Ok(ResumeRes { snapshot, publications, mline_tables })
}
