// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:IMPL-LINK-PEER-STATE 정:RULE-LINK-PAIR-LIVENESS 정:RULE-LINK-TRANSPORT 정:IMPL-LINK-ICE-RESTART 정:RULE-AFF-ONE-SET 정:IMPL-SES-DATA · model: claude-opus-5-5
//! Peer(세션 × 미디어 서버) 와 전송 객체(Conn — 자격 한 벌 = 연결 하나). 신원은 객체이고 ufrag · 주소는 조회 키다.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use arc_swap::{ArcSwap, ArcSwapOption};
use bytes::Bytes;
use oxsig::bplane::Home;
use oxsig::wire::{ExtMap, PcMode, RoomJoinReqCodecs};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::media::{PubStream, PubTrack};
use crate::room::Room;
use crate::sfu::{Sfu, lock, mono_ms};
use crate::table::Table;

pub const ALIVE: u8 = 0;
pub const SUSPECT: u8 = 1;
pub const PAIR_EXPIRE_MS: u64 = 30_000;
pub const DC_BUFFER: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Publish,
    Subscribe,
    Both,
}

pub struct Creds {
    pub cur: (String, String),
    pub pend: Option<(String, String)>,
}

pub struct TcpLink {
    pub id: u64,
    pub addr: SocketAddr,
    pub tx: mpsc::Sender<Bytes>,
    pub closed: AtomicBool,
    pub cancel: CancellationToken,
    pub conn: Mutex<Weak<Conn>>,
}

#[derive(Clone)]
pub enum Path {
    Udp(SocketAddr),
    Tcp(Arc<TcpLink>),
}

impl Path {
    pub fn addr(&self) -> SocketAddr {
        match self {
            Path::Udp(a) => *a,
            Path::Tcp(l) => l.addr,
        }
    }

    pub fn key(&self) -> PairKey {
        match self {
            Path::Udp(a) => PairKey::Udp(*a),
            Path::Tcp(l) => PairKey::Tcp(l.id),
        }
    }

    pub fn alive(&self, now: u64, last_ok: u64) -> bool {
        match self {
            Path::Udp(_) => now.saturating_sub(last_ok) < PAIR_EXPIRE_MS,
            Path::Tcp(l) => !l.closed.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PairKey {
    Udp(SocketAddr),
    Tcp(u64),
}

pub struct Pair {
    pub path: Path,
    pub last_ok: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ExtIds {
    pub mid: Option<u8>,
    pub audio_level: Option<u8>,
    pub abs_send_time: Option<u8>,
    pub twcc: Option<u8>,
}

pub const URI_MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
pub const URI_AUDIO_LEVEL: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
pub const URI_ABS_SEND_TIME: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
pub const URI_TWCC: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
pub const URI_RID: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
pub const URI_RRID: &str = "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id";

pub fn server_extmap() -> Vec<ExtMap> {
    [(1, URI_MID), (4, URI_AUDIO_LEVEL), (5, URI_ABS_SEND_TIME), (6, URI_TWCC), (10, URI_RID), (11, URI_RRID)]
        .iter()
        .map(|(id, uri)| ExtMap { id: *id, uri: (*uri).to_string() })
        .collect()
}

impl ExtIds {
    pub fn from_table(map: &[ExtMap]) -> ExtIds {
        let find = |uri: &str| map.iter().find(|e| e.uri == uri).map(|e| e.id);
        let server = server_extmap();
        let pick = |uri: &str| find(uri).or_else(|| server.iter().find(|e| e.uri == uri).map(|e| e.id));
        ExtIds { mid: pick(URI_MID), audio_level: pick(URI_AUDIO_LEVEL), abs_send_time: pick(URI_ABS_SEND_TIME), twcc: pick(URI_TWCC) }
    }
}

pub struct Dc {
    pub tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    pub pending: Mutex<VecDeque<Vec<u8>>>,
    pub dropped: AtomicU64,
}

pub struct Conn {
    pub id: u64,
    pub role: Role,
    pub user_id: String,
    pub peer: OnceLock<Weak<Peer>>,
    pub sfu: Weak<Sfu>,
    pub creds: Mutex<Creds>,
    pub path: ArcSwapOption<Path>,
    pub pairs: Mutex<HashMap<PairKey, Pair>>,
    pub state: AtomicU8,
    pub suspect_since: AtomicU64,
    pub state_since: AtomicU64,
    pub dtls_in: Mutex<Option<mpsc::Sender<Bytes>>>,
    pub srtp_in: Mutex<Option<webrtc_srtp::context::Context>>,
    pub srtp_out: Mutex<Option<webrtc_srtp::context::Context>>,
    pub secured: AtomicBool,
    pub dc: Dc,
    pub cancel: CancellationToken,
    pub table: Mutex<Table>,
    pub ack: AtomicU64,
    pub twcc_out: AtomicU32,
    pub sub_ext: ArcSwap<ExtIds>,
    pub rtx_budget: Mutex<(u64, u32)>,
    pub pubs: ArcSwap<HashMap<u32, Arc<PubTrack>>>,
    pub rtx_of: ArcSwap<HashMap<u32, Arc<PubTrack>>>,
    pub sims: ArcSwap<Vec<Arc<PubStream>>>,
    pub twcc_in: Mutex<crate::rtcp_io::TwccIn>,
    pub bwe: Mutex<crate::layer::Bwe>,
    pub auto_layer: AtomicU8,
}

impl Conn {
    pub fn new(sfu: &Arc<Sfu>, role: Role, user_id: &str, mid_start: u8) -> Arc<Conn> {
        let ufrag = common::ident::random_hex(4);
        let pwd: String = common::ident::random_hex(11);
        let c = Arc::new(Conn {
            id: sfu.next_id(),
            role,
            user_id: user_id.to_string(),
            peer: OnceLock::new(),
            sfu: Arc::downgrade(sfu),
            creds: Mutex::new(Creds { cur: (ufrag.clone(), pwd), pend: None }),
            path: ArcSwapOption::empty(),
            pairs: Mutex::new(HashMap::new()),
            state: AtomicU8::new(SUSPECT),
            suspect_since: AtomicU64::new(0),
            state_since: AtomicU64::new(mono_ms()),
            dtls_in: Mutex::new(None),
            srtp_in: Mutex::new(None),
            srtp_out: Mutex::new(None),
            secured: AtomicBool::new(false),
            dc: Dc { tx: Mutex::new(None), pending: Mutex::new(VecDeque::new()), dropped: AtomicU64::new(0) },
            cancel: CancellationToken::new(),
            table: Mutex::new(Table::new(mid_start)),
            ack: AtomicU64::new(0),
            twcc_out: AtomicU32::new(1),
            sub_ext: ArcSwap::from_pointee(ExtIds::from_table(&server_extmap())),
            rtx_budget: Mutex::new((0, 0)),
            pubs: ArcSwap::from_pointee(HashMap::new()),
            rtx_of: ArcSwap::from_pointee(HashMap::new()),
            sims: ArcSwap::from_pointee(Vec::new()),
            twcc_in: Mutex::new(crate::rtcp_io::TwccIn::default()),
            bwe: Mutex::new(crate::layer::Bwe::default()),
            auto_layer: AtomicU8::new(0),
        });
        sfu.index_ufrag(&ufrag, &c);
        c
    }

    pub fn peer(&self) -> Option<Arc<Peer>> {
        self.peer.get().and_then(Weak::upgrade)
    }

    pub fn ufrag(&self) -> String {
        lock(&self.creds).cur.0.clone()
    }

    pub fn pwd_for(&self, ufrag: &str) -> Option<String> {
        let c = lock(&self.creds);
        if c.cur.0 == ufrag {
            return Some(c.cur.1.clone());
        }
        c.pend.as_ref().filter(|p| p.0 == ufrag).map(|p| p.1.clone())
    }

    pub fn restart(&self, sfu: &Arc<Sfu>, me: &Arc<Conn>) -> (String, String) {
        let ufrag = common::ident::random_hex(4);
        let pwd = common::ident::random_hex(11);
        let old = {
            let mut c = lock(&self.creds);
            c.pend.replace((ufrag.clone(), pwd.clone()))
        };
        if let Some((u, _)) = old {
            sfu.unindex_ufrag(&u);
        }
        sfu.index_ufrag(&ufrag, me);
        (ufrag, pwd)
    }

    pub fn promote(&self, sfu: &Sfu, ufrag: &str) -> bool {
        let old = {
            let mut c = lock(&self.creds);
            match c.pend.take() {
                Some(p) if p.0 == ufrag => Some(std::mem::replace(&mut c.cur, p)),
                other => {
                    c.pend = other;
                    None
                }
            }
        };
        match old {
            Some((u, _)) => {
                sfu.unindex_ufrag(&u);
                true
            }
            None => false,
        }
    }

    pub fn send(&self, b: &[u8]) -> bool {
        let Some(path) = self.path.load_full() else {
            return false;
        };
        self.send_on(&path, b)
    }

    pub fn send_on(&self, path: &Path, b: &[u8]) -> bool {
        match path {
            Path::Udp(a) => match self.sfu.upgrade() {
                Some(s) => s.udp.try_send_to(b, *a).is_ok(),
                None => false,
            },
            Path::Tcp(l) => {
                if b.len() > 65_535 || l.closed.load(Ordering::Relaxed) {
                    return false;
                }
                let mut f = Vec::with_capacity(b.len() + 2);
                f.extend_from_slice(&(b.len() as u16).to_be_bytes());
                f.extend_from_slice(b);
                l.tx.try_send(Bytes::from(f)).is_ok()
            }
        }
    }

    pub fn send_rtp(&self, plain: &[u8]) -> bool {
        let enc = {
            let mut g = lock(&self.srtp_out);
            match g.as_mut() {
                Some(ctx) => ctx.encrypt_rtp(plain).ok(),
                None => None,
            }
        };
        match enc {
            Some(e) => self.send(&e),
            None => false,
        }
    }

    pub fn send_rtcp(&self, plain: &[u8]) -> bool {
        let enc = {
            let mut g = lock(&self.srtp_out);
            match g.as_mut() {
                Some(ctx) => ctx.encrypt_rtcp(plain).ok(),
                None => None,
            }
        };
        match enc {
            Some(e) => self.send(&e),
            None => false,
        }
    }

    pub fn is_alive(&self) -> bool {
        self.state.load(Ordering::Relaxed) == ALIVE
    }

    pub fn state_name(&self) -> &'static str {
        if self.is_alive() { "alive" } else { "suspect" }
    }

    pub fn publishes(&self) -> bool {
        matches!(self.role, Role::Publish | Role::Both)
    }

    pub fn rtx_allow(&self, now: u64) -> bool {
        let mut b = lock(&self.rtx_budget);
        if now.saturating_sub(b.0) >= 3_000 {
            b.0 = now;
            b.1 = 0;
        }
        if b.1 >= 200 {
            return false;
        }
        b.1 += 1;
        true
    }
}

#[derive(Debug, Clone, Default)]
pub struct JoinTags {
    pub extmap: Vec<ExtMap>,
    pub codecs: Vec<RoomJoinReqCodecs>,
}

#[derive(Debug, Clone, Default)]
pub struct Ready {
    pub first_at: u64,
    pub open: bool,
}

pub struct Peer {
    pub session_id: String,
    pub user_id: String,
    pub pc_mode: PcMode,
    pub home: Home,
    pub pub_conn: Arc<Conn>,
    pub sub_conn: Arc<Conn>,
    pub rooms: Mutex<BTreeSet<String>>,
    pub pub_room: ArcSwapOption<Room>,
    pub streams: Mutex<Vec<Arc<PubStream>>>,
    pub ready: Mutex<HashMap<String, Ready>>,
    pub alive: AtomicBool,
}

impl Peer {
    pub fn conns(&self) -> Vec<Arc<Conn>> {
        if Arc::ptr_eq(&self.pub_conn, &self.sub_conn) {
            vec![self.pub_conn.clone()]
        } else {
            vec![self.pub_conn.clone(), self.sub_conn.clone()]
        }
    }

    pub fn room_ids(&self) -> Vec<String> {
        lock(&self.rooms).iter().cloned().collect()
    }

    pub fn in_room(&self, r: &str) -> bool {
        lock(&self.rooms).contains(r)
    }

    pub fn pub_room_id(&self) -> Option<String> {
        self.pub_room.load().as_ref().map(|r| r.id.clone())
    }

    pub fn affiliation(&self) -> oxsig::wire::Affiliation {
        oxsig::wire::Affiliation { sub_rooms: self.room_ids(), pub_room: self.pub_room_id() }
    }

    pub fn stream(&self, track_id: &str) -> Option<Arc<PubStream>> {
        lock(&self.streams).iter().find(|s| s.track_id == track_id).cloned()
    }

    pub fn stream_by_ssrc(&self, ssrc: u32) -> Option<Arc<PubStream>> {
        lock(&self.streams).iter().find(|s| s.ssrc == ssrc && ssrc != 0).cloned()
    }

    pub fn active_count(&self) -> usize {
        lock(&self.streams).len()
    }
}
