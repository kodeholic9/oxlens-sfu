// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-REQUEST · 정:IMPL-ERR-ISSUERS · 정:CAND-BUS-HANDLE-SHAPE · model: claude-opus-5-5
//! node 안 Handle — `Hello`(node · hub_inst → epoch · build) · `Call`(봉투 → 답). `user_id` 는 봉투에서만 · 실패는 코드별로 센다.

use common::envelope::handle::handle_server::{Handle, HandleServer};
use common::envelope::handle::{CallReq, CallRes, HelloReq, HelloRes};
use common::envelope::{CallKind, Claims, Envelope, Home, Reply};
use oxsig::msg::PcMode;
use oxsig::{Code, Failure, Op};
use serde_json::Value;

pub struct Ctx {
    pub user: String,
    pub sid: String,
    pub session: String,
    pub pc_mode: PcMode,
    pub source_tag: bool,
    pub home: Option<Home>,
    pub claims: Option<Claims>,
    pub over: Option<u64>,
}

impl Ctx {
    pub fn from(env: &Envelope) -> Result<Ctx, Failure> {
        let user = env.user_id.clone().ok_or_else(|| Code::NotBound.failure())?;
        Ok(Ctx {
            user,
            sid: env.sid.clone().unwrap_or_default(),
            session: env.session.clone().unwrap_or_default(),
            pc_mode: env.pc_mode.unwrap_or(PcMode::Two),
            source_tag: env.source_tag.unwrap_or(false),
            home: env.home.clone(),
            claims: env.claims.clone(),
            over: env.admit_retry_after_ms,
        })
    }

    pub fn hidden(&self) -> bool {
        self.claims.as_ref().map(|c| c.hidden).unwrap_or(false)
    }
}

pub struct Svc;

static HELLO_HUB: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);
static REFUSED_HUB: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);

pub fn new_caller(last: &std::sync::Mutex<Option<(String, String)>>, node: &str, hub_inst: &str) -> bool {
    let Ok(mut g) = last.lock() else { return true };
    if g.as_ref().is_some_and(|(n, i)| n == node && i == hub_inst) {
        return false;
    }
    let _ = g.replace((node.to_string(), hub_inst.to_string()));
    true
}

#[tonic::async_trait]
impl Handle for Svc {
    async fn hello(&self, req: tonic::Request<HelloReq>) -> Result<tonic::Response<HelloRes>, tonic::Status> {
        let r = req.into_inner();
        let s = super::sfu().ok_or_else(|| tonic::Status::unavailable("starting"))?;
        if r.node != s.node {
            if new_caller(&REFUSED_HUB, &r.node, &r.hub_inst) {
                tracing::warn!(kind = "diag", spec = "SV-RULE-BUS-SHAPE", "in.hub_node" = %r.node, "hello refused — the hub of node {} is not this sfud's node {}", r.node, s.node);
            }
            return Err(tonic::Status::permission_denied(format!("node {} is not this sfud's node", r.node)));
        }
        if new_caller(&HELLO_HUB, &r.node, &r.hub_inst) {
            tracing::info!(kind = "diag", spec = "SV-RULE-OPS-SURFACE", "hello from hub {} ({})", r.node, r.hub_inst);
        }
        Ok(tonic::Response::new(HelloRes { epoch: s.epoch.clone(), build: s.build.clone() }))
    }

    async fn call(&self, req: tonic::Request<CallReq>) -> Result<tonic::Response<CallRes>, tonic::Status> {
        let env = Envelope::decode(&req.into_inner().envelope).map_err(|e| tonic::Status::invalid_argument(e.to_string()))?;
        let reply = dispatch(&env).await;
        Ok(tonic::Response::new(CallRes { reply: reply.encode() }))
    }
}

pub fn server() -> HandleServer<Svc> {
    HandleServer::new(Svc)
}

fn reply(r: Result<Value, Failure>) -> Reply {
    match r {
        Ok(v) => Reply::ok(v),
        Err(f) => {
            super::code_count(f.code);
            Reply::Fail { failure: f }
        }
    }
}

fn follow_home(env: &Envelope) {
    if let (Some(s), Some(u), Some(h)) = (super::sfu(), env.user_id.as_deref(), env.home.as_ref())
        && let Some(p) = s.peer(u)
    {
        p.set_home(h);
    }
}

pub async fn dispatch(env: &Envelope) -> Reply {
    if matches!(env.kind, CallKind::Op | CallKind::Resume) {
        follow_home(env);
    }
    match env.kind {
        CallKind::Op => reply(op(env)),
        CallKind::SessionGone => reply(super::rooms::session_gone(env)),
        CallKind::Resume => reply(super::rooms::resume(env)),
        CallKind::CreateRoom => reply(super::rooms::create(env).await),
        CallKind::GetRoom => reply(super::rooms::get(env)),
        CallKind::ListRooms => reply(super::rooms::list()),
        CallKind::Admin => reply(super::admin::handle(env)),
        CallKind::LogLevel => reply(super::admin::log_level(env)),
        CallKind::Permission => reply(Err(Code::UnknownOp.failure())),
    }
}

fn op(env: &Envelope) -> Result<Value, Failure> {
    let c = Ctx::from(env)?;
    let op = env.op.and_then(Op::from_u16).ok_or_else(|| Code::UnknownOp.failure())?;
    let b = &env.body;
    match op {
        Op::RoomJoin => super::rooms::join(&c, b),
        Op::RoomLeave => super::rooms::leave(&c, b),
        Op::PublishTracks => super::media::publish(&c, b),
        Op::Ready => super::media::ready(&c, b),
        Op::SubscribeLayer => super::media::subscribe_layer(&c, b),
        Op::TrackSet => super::media::track_set(&c, b),
        Op::IceRestart => super::media::ice_restart(&c, b),
        Op::Affiliation => super::media::affiliation(&c, b),
        _ => Err(Code::UnknownOp.failure()),
    }
}

#[cfg(test)]
mod hello_line_tests {
    use super::new_caller;

    #[test]
    fn the_hello_line_is_written_once_per_calling_hub() {
        let last = std::sync::Mutex::new(None);
        assert!(new_caller(&last, "n1", "i1"), "정:RULE-BUS-SHAPE the first hello from a hub is written");
        for _ in 0..3 {
            assert!(!new_caller(&last, "n1", "i1"), "정:RULE-BUS-SHAPE the remote check's repeated hello from the same hub writes no line");
        }
        assert!(new_caller(&last, "n1", "i2"), "정:RULE-BUS-SHAPE a hub that came back with a new boot identity is written again");
        assert!(new_caller(&last, "n2", "i2"), "정:RULE-BUS-SHAPE another node's hub is written again");
        assert!(!new_caller(&last, "n2", "i2"), "stand check — and then quiet again");
    }
}
