// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-LINK-PEER-STATE · 정:RULE-LINK-PAIR-LIVENESS · 정:RULE-LINK-TRANSPORT · 정:IMPL-LINK-ICE-RESTART · 정:RULE-DOC-OWNERSHIP · 정:RULE-LINK-SERVER-SCOPE · 정:RULE-CONC-STATE-VOCAB · model: claude-opus-5-5
//! 연결 — Peer(사람 × 미디어 서버) · 전송 객체(연결마다 하나 · 자격 · 경로 · 쌍별 생존 · PeerState · SRTP 문맥) · 신원은 전송 객체(ufrag · 주소는 조회 키).

pub mod creds;
pub mod dtls;
pub mod stun;
pub mod tcp;

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwapOption;
use common::envelope::{Claims, Home};
use oxsig::msg::PcMode;

pub use creds::{Cred, Creds};

pub const UDP_PAIR_EXPIRE_MS: u64 = 30_000;
pub const DTLS_HANDSHAKE_SECS: u64 = 30;
pub const DTLS_MTU: usize = 1_200;
pub const SOCKET_BUF: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PathKey {
    Udp(SocketAddr),
    Tcp(u64),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Publish,
    Subscribe,
    Both,
}

#[derive(Debug, Clone, Copy)]
pub struct Pair {
    pub key: PathKey,
    pub last_ms: u64,
}

#[derive(Default)]
pub struct TxStats {
    pub packets: AtomicU64,
    pub bytes: AtomicU64,
    pub retransmitted: AtomicU64,
    pub nack_in: AtomicU64,
    pub pli_in: AtomicU64,
}

pub struct Transport {
    pub id: u64,
    pub user: String,
    pub role: Role,
    pub worker: usize,
    pub creds: Mutex<Creds>,
    pub path: ArcSwapOption<PathKey>,
    pub pairs: Mutex<Vec<Pair>>,
    state: AtomicU8,
    pub suspect_since: AtomicU64,
    pub state_since: AtomicU64,
    pub srtp_rx: Mutex<Option<webrtc_srtp::context::Context>>,
    pub srtp_tx: Mutex<Option<webrtc_srtp::context::Context>>,
    pub ready: AtomicBool,
    pub closed: AtomicBool,
    pub twcc_seq: AtomicU16,
    pub tx: TxStats,
    pub dtls_in: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    pub dtls_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>>>,
    pub dc_out: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    pub dc_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>>>,
    pub dc_open: AtomicBool,
    pub stop: tokio::sync::watch::Sender<bool>,
    pub ingress: arc_swap::ArcSwap<std::collections::HashMap<u32, Arc<crate::r#pub::PubTrack>>>,
    pub ext_ids: arc_swap::ArcSwap<ExtIds>,
    pub pub_room: ArcSwapOption<String>,
    pub rtc: crate::rtcp::ConnRtcp,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExtIds {
    pub mid: u8,
    pub rid: u8,
    pub repair_rid: u8,
    pub twcc: u8,
    pub audio_level: u8,
}

impl Transport {
    pub fn new(id: u64, user: &str, role: Role, worker: usize) -> Arc<Transport> {
        let (dtls_in, dtls_rx) = tokio::sync::mpsc::unbounded_channel();
        let (dc_out, dc_rx) = tokio::sync::mpsc::unbounded_channel();
        let (stop, _) = tokio::sync::watch::channel(false);
        let now = common::now_ms();
        Arc::new(Transport {
            id,
            user: user.to_string(),
            role,
            worker,
            creds: Mutex::new(Creds::new()),
            path: ArcSwapOption::empty(),
            pairs: Mutex::new(Vec::new()),
            state: AtomicU8::new(0),
            suspect_since: AtomicU64::new(0),
            state_since: AtomicU64::new(now),
            srtp_rx: Mutex::new(None),
            srtp_tx: Mutex::new(None),
            ready: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            twcc_seq: AtomicU16::new(0),
            tx: TxStats::default(),
            dtls_in,
            dtls_rx: Mutex::new(Some(dtls_rx)),
            dc_out,
            dc_rx: Mutex::new(Some(dc_rx)),
            dc_open: AtomicBool::new(false),
            stop,
            ingress: arc_swap::ArcSwap::from_pointee(std::collections::HashMap::new()),
            ext_ids: arc_swap::ArcSwap::from_pointee(ExtIds::default()),
            pub_room: ArcSwapOption::empty(),
            rtc: crate::rtcp::ConnRtcp::default(),
        })
    }

    pub fn peer_state(&self) -> PeerState {
        if self.state.load(Ordering::Acquire) == 0 { PeerState::Alive } else { PeerState::Suspect }
    }

    pub fn set_state(&self, s: PeerState, now: u64) -> Option<PeerState> {
        let v = if s == PeerState::Alive { 0 } else { 1 };
        let prev = self.state.swap(v, Ordering::AcqRel);
        if prev == v {
            return None;
        }
        self.state_since.store(now, Ordering::Release);
        if s == PeerState::Suspect {
            let _ = self.suspect_since.compare_exchange(0, now, Ordering::AcqRel, Ordering::Acquire);
        } else {
            self.suspect_since.store(0, Ordering::Release);
        }
        Some(if prev == 0 { PeerState::Alive } else { PeerState::Suspect })
    }

    pub fn ufrag(&self) -> String {
        self.creds.lock().map(|c| c.use_ufrag().to_string()).unwrap_or_default()
    }

    pub fn cred(&self) -> Cred {
        self.creds.lock().map(|c| c.active.clone()).unwrap_or_else(|_| Cred::fresh())
    }

    pub fn touch_pair(&self, key: PathKey, now: u64) {
        if let Ok(mut p) = self.pairs.lock() {
            match p.iter_mut().find(|x| x.key == key) {
                Some(x) => x.last_ms = now,
                None => p.push(Pair { key, last_ms: now }),
            }
        }
    }

    pub fn path_alive(&self, now: u64, tcp_open: impl Fn(u64) -> bool) -> bool {
        let Some(path) = self.path.load_full() else { return true };
        match *path {
            PathKey::Tcp(id) => tcp_open(id),
            PathKey::Udp(_) => self
                .pairs
                .lock()
                .map(|p| p.iter().find(|x| x.key == *path).map(|x| now.saturating_sub(x.last_ms) <= UDP_PAIR_EXPIRE_MS).unwrap_or(false))
                .unwrap_or(false),
        }
    }

    pub fn send_ok(&self, now: u64, tcp_open: impl Fn(u64) -> bool) -> Option<PathKey> {
        let p = self.path.load_full()?;
        if self.path_alive(now, tcp_open) { Some(*p) } else { None }
    }
}

pub struct Peer {
    pub user: String,
    pub sid: String,
    pub session: String,
    pub home: Mutex<Home>,
    pub pc_mode: PcMode,
    pub claims: Claims,
    pub publish: Arc<Transport>,
    pub subscribe: Arc<Transport>,
    pub rooms: Mutex<BTreeSet<String>>,
    pub created_at: u64,
    pub ever_sent_join: AtomicBool,
}

impl Peer {
    pub fn transports(&self) -> Vec<Arc<Transport>> {
        if Arc::ptr_eq(&self.publish, &self.subscribe) {
            vec![self.publish.clone()]
        } else {
            vec![self.publish.clone(), self.subscribe.clone()]
        }
    }

    pub fn rooms(&self) -> Vec<String> {
        self.rooms.lock().map(|r| r.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn in_room(&self, room: &str) -> bool {
        self.rooms.lock().map(|r| r.contains(room)).unwrap_or(false)
    }

    pub fn home(&self) -> Home {
        self.home.lock().map(|h| h.clone()).unwrap_or(Home { node: String::new(), inst: String::new() })
    }

    pub fn set_home(&self, h: &Home) {
        if let Ok(mut g) = self.home.lock()
            && (g.node != h.node || g.inst != h.inst)
        {
            g.clone_from(h);
        }
    }
}

pub fn fingerprint_hex(der: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(der);
    let hex: Vec<String> = d.iter().map(|b| format!("{b:02X}")).collect();
    format!("sha-256 {}", hex.join(":"))
}
