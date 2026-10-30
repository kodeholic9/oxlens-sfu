// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-TRANSPORT · 정:IMPL-LINK-PEER-STATE · 정:RULE-LINK-PAIR-LIVENESS · 정:RULE-CONC-STATE-VOCAB · 정:RULE-CONC-PERF-AXES · 정:CAND-LINK-UDP-BUFFER · 연:RULE-LINK-RECOVERY-UNIT · model: claude-opus-5-5
//! 전송 객체 — 자격 한 벌 = 연결 하나(신원). ufrag · 주소는 찾는 열쇠일 뿐이다. 쌍별 생존 · PeerState · SRTP 문맥 · DC 송신 · 업링크/다운링크 색인(RCU).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use arc_swap::ArcSwap;
use bytes::Bytes;
use tokio::sync::mpsc;
use webrtc_srtp::context::Context;

use super::ice::{Creds, Ledger, Pairs, Via};
use super::io::Io;
use crate::ops::drops::{DROPS, bump};

pub const DC_BACKLOG: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Publish,
    Subscribe,
    Both,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Publish => "publish",
            Role::Subscribe => "subscribe",
            Role::Both => "1pc",
        }
    }

    pub fn publishes(self) -> bool {
        matches!(self, Role::Publish | Role::Both)
    }

    pub fn subscribes(self) -> bool {
        matches!(self, Role::Subscribe | Role::Both)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Alive,
    Suspect,
}

impl PeerState {
    pub fn as_str(self) -> &'static str {
        match self {
            PeerState::Alive => "Alive",
            PeerState::Suspect => "Suspect",
        }
    }
}

pub struct Srtp {
    pub rx: Mutex<Context>,
    pub tx: Mutex<Context>,
}

pub struct Transport {
    pub id: u64,
    pub role: Role,
    pub user: Arc<str>,
    pub session: Arc<str>,
    pub ledger: Mutex<Ledger>,
    pub pairs: Mutex<Pairs>,
    state: AtomicU8,
    pub suspect_since: AtomicU64,
    pub alive_since: AtomicU64,
    pub created_ms: u64,
    pub dtls_tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    pub dtls_started: AtomicBool,
    pub srtp: arc_swap::ArcSwapOption<Srtp>,
    pub dc_tx: Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>,
    pub dc_open: AtomicBool,
    pub dc_backlog: Mutex<VecDeque<Vec<u8>>>,
    pub dc_dropped: AtomicU64,
    pub tasks: Mutex<Vec<tokio::task::AbortHandle>>,
    pub twcc_out: AtomicU32,
    pub io: Weak<Io>,
    pub closed: AtomicBool,
    pub uplinks: ArcSwap<HashMap<u32, Arc<crate::r#pub::Uplink>>>,
    pub downlinks: ArcSwap<HashMap<u32, Arc<crate::fwd::hot::Downlink>>>,
    pub ingress: Mutex<crate::rtcp::Ingress>,
    pub ext: ArcSwap<crate::r#pub::IngressExt>,
    pub pending_sim: ArcSwap<Vec<Arc<crate::r#pub::Stream>>>,
    pub nack_budget: Arc<crate::rtcp::Budget>,
    pub sim: Mutex<crate::sim::State>,
    pub worker: AtomicUsize,
}

impl Transport {
    #[allow(clippy::too_many_arguments)]
    pub fn new(id: u64, role: Role, user: &str, session: &str, creds: Creds, io: &Arc<Io>, budget: Arc<crate::rtcp::Budget>, now_ms: u64) -> Transport {
        Transport {
            id,
            role,
            user: Arc::from(user),
            session: Arc::from(session),
            ledger: Mutex::new(Ledger::new(creds)),
            pairs: Mutex::new(Pairs::new()),
            state: AtomicU8::new(0),
            suspect_since: AtomicU64::new(0),
            alive_since: AtomicU64::new(now_ms),
            created_ms: now_ms,
            dtls_tx: Mutex::new(None),
            dtls_started: AtomicBool::new(false),
            srtp: arc_swap::ArcSwapOption::empty(),
            dc_tx: Mutex::new(None),
            dc_open: AtomicBool::new(false),
            dc_backlog: Mutex::new(VecDeque::new()),
            dc_dropped: AtomicU64::new(0),
            tasks: Mutex::new(Vec::new()),
            twcc_out: AtomicU32::new(0),
            io: Arc::downgrade(io),
            closed: AtomicBool::new(false),
            uplinks: ArcSwap::from_pointee(HashMap::new()),
            downlinks: ArcSwap::from_pointee(HashMap::new()),
            ingress: Mutex::new(crate::rtcp::Ingress::default()),
            ext: ArcSwap::from_pointee(crate::r#pub::IngressExt::default()),
            pending_sim: ArcSwap::from_pointee(Vec::new()),
            nack_budget: budget,
            sim: Mutex::new(crate::sim::State::new(now_ms)),
            worker: AtomicUsize::new(0),
        }
    }

    pub fn state(&self) -> PeerState {
        if self.state.load(Ordering::Acquire) == 0 { PeerState::Alive } else { PeerState::Suspect }
    }

    pub fn swap_state(&self, next: PeerState) -> PeerState {
        let prev = self.state.swap(if next == PeerState::Alive { 0 } else { 1 }, Ordering::AcqRel);
        if prev == 0 { PeerState::Alive } else { PeerState::Suspect }
    }

    pub fn ufrag(&self) -> String {
        self.ledger.lock().map(|l| l.current().ufrag.clone()).unwrap_or_default()
    }

    pub fn creds(&self) -> Option<Creds> {
        self.ledger.lock().ok().map(|l| l.current().clone())
    }

    pub fn path(&self) -> Option<Via> {
        self.pairs.lock().ok().and_then(|p| p.path())
    }

    pub fn path_addr(&self) -> Option<SocketAddr> {
        self.path().map(|v| v.addr())
    }

    pub fn send_raw(&self, bytes: &[u8]) -> bool {
        let now = common::time::mono_ms();
        let via = match self.pairs.lock() {
            Ok(p) if p.path_alive(now) => p.path(),
            _ => None,
        };
        let (Some(via), Some(io)) = (via, self.io.upgrade()) else { return false };
        io.send_via(via, bytes)
    }

    pub fn protect_rtp(&self, plain: &[u8]) -> Option<Bytes> {
        let s = self.srtp.load_full()?;
        let mut tx = s.tx.lock().ok()?;
        tx.encrypt_rtp(plain).ok()
    }

    pub fn protect_rtcp(&self, plain: &[u8]) -> Option<Bytes> {
        let s = self.srtp.load_full()?;
        let mut tx = s.tx.lock().ok()?;
        tx.encrypt_rtcp(plain).ok()
    }

    pub fn send_rtp(&self, plain: &[u8]) -> bool {
        match self.protect_rtp(plain) {
            Some(b) => self.send_raw(&b),
            None => false,
        }
    }

    pub fn send_rtcp(&self, plain: &[u8]) -> bool {
        match self.protect_rtcp(plain) {
            Some(b) => self.send_raw(&b),
            None => false,
        }
    }

    pub fn unprotect_rtp(&self, cipher: &[u8]) -> Option<Bytes> {
        let Some(s) = self.srtp.load_full() else {
            bump(&DROPS.srtp_not_ready);
            return None;
        };
        let r = s.rx.lock().ok()?.decrypt_rtp(cipher);
        match r {
            Ok(b) => Some(b),
            Err(_) => {
                bump(&DROPS.srtp_fail);
                None
            }
        }
    }

    pub fn unprotect_rtcp(&self, cipher: &[u8]) -> Option<Bytes> {
        let Some(s) = self.srtp.load_full() else {
            bump(&DROPS.srtp_not_ready);
            return None;
        };
        let r = s.rx.lock().ok()?.decrypt_rtcp(cipher);
        match r {
            Ok(b) => Some(b),
            Err(_) => {
                bump(&DROPS.srtcp_fail);
                None
            }
        }
    }

    pub fn dc_send(&self, frame: Vec<u8>) {
        if self.dc_open.load(Ordering::Acquire)
            && let Ok(g) = self.dc_tx.lock()
            && let Some(tx) = g.as_ref()
        {
            if tx.send(frame).is_err() {
                self.dc_dropped.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
        if let Ok(mut b) = self.dc_backlog.lock() {
            if b.len() >= DC_BACKLOG {
                b.pop_front();
                self.dc_dropped.fetch_add(1, Ordering::Relaxed);
            }
            b.push_back(frame);
        }
    }

    pub fn dc_opened(&self) {
        let backlog: Vec<Vec<u8>> = self.dc_backlog.lock().map(|mut b| b.drain(..).collect()).unwrap_or_default();
        self.dc_open.store(true, Ordering::Release);
        if let Ok(g) = self.dc_tx.lock()
            && let Some(tx) = g.as_ref()
        {
            for f in backlog {
                let _ = tx.send(f);
            }
        }
    }

    pub fn add_task(&self, h: tokio::task::AbortHandle) {
        if let Ok(mut t) = self.tasks.lock() {
            t.push(h);
        }
    }

    pub fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Ok(mut d) = self.dtls_tx.lock() {
            d.take();
        }
        if let Ok(mut d) = self.dc_tx.lock() {
            d.take();
        }
        if let Ok(mut t) = self.tasks.lock() {
            for h in t.drain(..) {
                h.abort();
            }
        }
    }

    pub fn next_twcc(&self) -> u32 {
        self.twcc_out.fetch_add(1, Ordering::Relaxed)
    }

    pub fn note_send_fail() {
        bump(&DROPS.send_fail);
    }
}
