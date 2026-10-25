// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-DOC-OWNERSHIP 정:IMPL-BUS-HOME-LOST 정:IMPL-SES-HANDOVER 정:RULE-RTCP-NACK-DOWN 정:IMPL-AUX-DC-SEND 정:RULE-BUS-REQUEST 연:RULE-LINK-PER-SERVER-UNITS · model: claude-opus-5-5
//! Peer — user × sfud · 홈(node, inst) · 전송 객체(1pc 하나 · 2pc 둘) · 소속 · 발행 문맥 · 연결의 배정표 · (user, 방) 게이트 · 레이어 상한 · Peer 단위 단일 큐.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use arc_swap::ArcSwap;
use oxsig::envelope::{Ctx, Home};
use oxsig::msg::{PcMode, Permission};
use serde_json::Value;

use crate::aff::Aff;
use crate::fwd::{Conn, Ingress};
use crate::r#pub::{Ctx as PubCtx, Stream};
use crate::sub::table::Table;
use crate::sub::{Gate, Prefs};

pub const DC_BUFFER: usize = 64;

pub struct PeerSt {
    pub pubs: PubCtx,
    pub table: Table,
    pub gates: HashMap<String, Arc<Gate>>,
    pub prefs: HashMap<(String, String), Arc<Prefs>>,
}

struct Budget {
    start: u64,
    used: u32,
}

pub struct Peer {
    pub user: String,
    pub session: std::sync::Mutex<String>,
    pub home: ArcSwap<Home>,
    pub pc_mode: PcMode,
    pub ptype: u8,
    pub hidden: bool,
    pub metadata: Option<Value>,
    pub token_perm: Option<Permission>,
    pub aff: Aff,
    pub op: tokio::sync::Mutex<()>,
    pub st: std::sync::Mutex<PeerSt>,
    pub conns: Vec<Arc<Conn>>,
    pub created_at: u64,
    budget: std::sync::Mutex<Budget>,
    pub dc_buf: std::sync::Mutex<VecDeque<Vec<u8>>>,
    pub gone: AtomicBool,
    pub dc_drop: AtomicU64,
    pub sync_lock: std::sync::Mutex<()>,
}

impl Peer {
    pub fn new(ctx: &Ctx, st: PeerSt, conns: Vec<Arc<Conn>>, now: u64) -> Peer {
        Peer {
            user: ctx.user_id.clone(),
            session: std::sync::Mutex::new(ctx.session.clone()),
            home: ArcSwap::from_pointee(ctx.home.clone()),
            pc_mode: ctx.pc_mode,
            ptype: ctx.participant_type,
            hidden: ctx.hidden,
            metadata: ctx.metadata.clone(),
            token_perm: ctx.permission.clone(),
            aff: Aff::default(),
            op: tokio::sync::Mutex::new(()),
            st: std::sync::Mutex::new(st),
            conns,
            created_at: now,
            budget: std::sync::Mutex::new(Budget { start: now, used: 0 }),
            dc_buf: std::sync::Mutex::new(VecDeque::new()),
            gone: AtomicBool::new(false),
            dc_drop: AtomicU64::new(0),
            sync_lock: std::sync::Mutex::new(()),
        }
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, PeerSt> {
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn session(&self) -> String {
        self.session.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn pub_conn(&self) -> &Arc<Conn> {
        &self.conns[0]
    }

    pub fn sub_conn(&self) -> &Arc<Conn> {
        &self.conns[self.conns.len() - 1]
    }

    pub fn follow_home(&self, h: &Home) {
        if **self.home.load() != *h {
            self.home.store(Arc::new(h.clone()));
        }
    }

    pub fn streams(&self) -> Vec<Arc<Stream>> {
        self.lock().pubs.streams.clone()
    }

    pub fn ingress_by_ssrc(&self, ssrc: u32) -> Option<Arc<Ingress>> {
        self.pub_conn().ingress_of(ssrc)
    }

    pub fn ingress_of_track(&self, track: &str) -> Vec<Arc<Ingress>> {
        self.pub_conn().ingress.load().values().filter(|i| i.stream.track_id == track).cloned().collect()
    }

    pub fn rtx_budget(&self, now: u64) -> bool {
        let Ok(mut b) = self.budget.lock() else { return false };
        if now.saturating_sub(b.start) >= crate::rtcp::shell::RTX_WINDOW_MS {
            b.start = now;
            b.used = 0;
        }
        if b.used >= crate::rtcp::shell::RTX_BUDGET {
            return false;
        }
        b.used += 1;
        true
    }

    pub fn dc_push(&self, frame: Vec<u8>) {
        let link = &self.pub_conn().link;
        if link.dc_open.load(Ordering::Acquire) && self.dc_buf.lock().map(|b| b.is_empty()).unwrap_or(true) {
            if !link.dc_send(frame) {
                self.dc_drop.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
        if let Ok(mut b) = self.dc_buf.lock() {
            if b.len() >= DC_BUFFER {
                b.pop_front();
                self.dc_drop.fetch_add(1, Ordering::Relaxed);
            }
            b.push_back(frame);
        }
    }

    pub fn dc_flush(&self) {
        let link = &self.pub_conn().link;
        if !link.dc_open.load(Ordering::Acquire) {
            return;
        }
        let frames: Vec<Vec<u8>> = self.dc_buf.lock().map(|mut b| b.drain(..).collect()).unwrap_or_default();
        for f in frames {
            if !link.dc_send(f) {
                self.dc_drop.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn gate(&self, room: &str) -> Option<Arc<Gate>> {
        self.lock().gates.get(room).cloned()
    }
}
