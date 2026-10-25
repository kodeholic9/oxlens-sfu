// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-SES-DATA 정:IMPL-SES-BIND 정:IMPL-SES-HANDOVER 정:IMPL-SES-RESUME 정:IMPL-SES-SOCKET-LIFE 정:RULE-SES-READ-AND-PROCESS 정:IMPL-SES-ADMISSION 정:RULE-GC-FOUR 정:CAND-SES-SUBROOMS-RECONCILE 정:CAND-SES-PREBIND-2001 연:RULE-SES-AUTH-TWO-WAYS 연:RULE-SES-EVICT-OLD 연:RULE-SES-PC-MODE-SESSION 연:RULE-SES-RESUME-BY-SESSION-ID 연:SES.S.bind_resume 연:SES.S.bind_new 연:SES.S.leave 연:SES.S.TWindow 연:SES.S.drop_reclaim 연:SES.S.resume_ok 연:SES.S.resume_2008 · model: claude-opus-5-5
//! 세션 — 신원 · `session_id` · 토큰값 넷 · 홈 · 서버별 입장 방 집합 · 소켓 하나 · 재개 창 · 폐기 경로 하나.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use oxsig::envelope::{Ctx, Home};
use oxsig::msg::{PcMode, Permission};
use oxsig::{Code, Op};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc};

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
    pub fn ctx(&self, home: Home) -> Ctx {
        Ctx {
            user_id: self.id.user.clone(),
            session: self.fp.clone(),
            pc_mode: self.id.pc_mode,
            home,
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
    pub fn name(self) -> &'static str {
        match self {
            Gone::Leave => "leave",
            Gone::Window => "window",
            Gone::Evicted => "evicted",
            Gone::Cut => "cut",
            Gone::Shutdown => "shutdown",
        }
    }
}
