// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-LINK-SERVER-SCOPE · 정:RULE-LINK-TRANSPORT · 정:RULE-LINK-PAIR-LIVENESS · 정:IMPL-LINK-PEER-STATE · 정:IMPL-LINK-ICE-RESTART · 정:IMPL-SUB-TABLE · 정:RULE-CONC-SIX · 정:IMPL-AUX-DC-SEND · 연:LINK.type.ServerConfig · 연:RULE-LINK-RECOVERY-UNIT · 연:RULE-LINK-EVENT-CONTRACT · model: claude-opus-5-5
//! 영역 LINK — 전송 객체(자격 한 벌 = 연결 하나 · 신원) · ufrag/주소는 조회 키 · 쌍별 생존 · 경로 · SRTP 문맥 · DC · 배정표 · 다운링크 줄(재기록기).

pub mod core;
pub mod shell;
pub mod stun;
pub mod table;

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use arc_swap::ArcSwap;
use oxsig::types::Kind;
use oxsig::{PcMode};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use self::core::{Cred, Creds, Liveness, PathKey, Pairs, PeerState};
use self::table::Table;
use crate::fwd::core::{ExtTable, Rewriter};
use crate::r#pub::{Phys, PubTrack};
use crate::sim::core::Switch;

pub const DC_BACKLOG: usize = 64;

pub fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub trait Upcall: Send + Sync + 'static {
    fn media(&self, t: &Arc<Transport>, plain: &[u8], rtcp: bool);
    fn dc(&self, t: &Arc<Transport>, data: &[u8]);
    fn dc_open(&self, t: &Arc<Transport>);
    fn suspect(&self, t: &Arc<Transport>);
}

#[derive(Debug, Default)]
pub struct NetCounters {
    pub stun_integrity_fail: AtomicU64,
    pub stun_unknown_ufrag: AtomicU64,
    pub stun_malformed: AtomicU64,
    pub demux_unknown: AtomicU64,
    pub unknown_path: AtomicU64,
    pub srtp_fail: AtomicU64,
    pub tcp_unidentified: AtomicU64,
    pub tcp_bad_frame: AtomicU64,
    pub dc_dropped: AtomicU64,
    pub send_fail: AtomicU64,
}

pub struct Net {
    pub udp: Arc<UdpSocket>,
    pub ip: IpAddr,
    pub port: u16,
    pub tcp_port: Option<u16>,
    pub cert: dtls::crypto::Certificate,
    pub fingerprint: String,
    ufrags: ArcSwap<HashMap<String, Weak<Transport>>>,
    paths: ArcSwap<HashMap<PathKey, Weak<Transport>>>,
    tcp: Mutex<HashMap<u64, mpsc::Sender<Vec<u8>>>>,
    pub up: OnceLock<Arc<dyn Upcall>>,
    pub counters: NetCounters,
    next_id: AtomicU64,
    all: Mutex<Vec<Weak<Transport>>>,
}

fn random_chars(n: usize) -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut b = vec![0u8; n];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| ALPHA[(*x as usize) % ALPHA.len()] as char).collect()
}

pub fn fresh_cred() -> Cred {
    Cred { ufrag: random_chars(core::UFRAG_LEN), pwd: random_chars(core::PWD_LEN) }
}

pub fn fresh_u32() -> u32 {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    u32::from_be_bytes(b).max(1)
}

impl Net {
    pub fn new(udp: Arc<UdpSocket>, ip: IpAddr, port: u16, tcp_port: Option<u16>, cert: dtls::crypto::Certificate, fingerprint: String) -> Arc<Net> {
        Arc::new(Net {
            udp,
            ip,
            port,
            tcp_port,
            cert,
            fingerprint,
            ufrags: ArcSwap::from_pointee(HashMap::new()),
            paths: ArcSwap::from_pointee(HashMap::new()),
            tcp: Mutex::new(HashMap::new()),
            up: OnceLock::new(),
            counters: NetCounters::default(),
            next_id: AtomicU64::new(1),
            all: Mutex::new(Vec::new()),
        })
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn bind_ufrag(&self, ufrag: &str, t: &Arc<Transport>) {
        let w = Arc::downgrade(t);
        self.ufrags.rcu(|m| {
            let mut m = HashMap::clone(m);
            m.insert(ufrag.to_string(), w.clone());
            m
        });
    }

    pub fn unbind_ufrag(&self, ufrag: &str) {
        self.ufrags.rcu(|m| {
            let mut m = HashMap::clone(m);
            m.remove(ufrag);
            m
        });
    }

    pub fn by_ufrag(&self, ufrag: &str) -> Option<Arc<Transport>> {
        self.ufrags.load().get(ufrag).and_then(Weak::upgrade)
    }

    pub fn bind_path(&self, k: PathKey, t: &Arc<Transport>) {
        if self.paths.load().get(&k).and_then(Weak::upgrade).map(|x| x.id == t.id).unwrap_or(false) {
            return;
        }
        let w = Arc::downgrade(t);
        self.paths.rcu(|m| {
            let mut m = HashMap::clone(m);
            m.insert(k, w.clone());
            m
        });
    }

    pub fn unbind_paths_of(&self, id: u64) {
        self.paths.rcu(|m| {
            let mut m = HashMap::clone(m);
            m.retain(|_, w| w.upgrade().map(|t| t.id != id).unwrap_or(false));
            m
        });
    }

    pub fn by_path(&self, k: &PathKey) -> Option<Arc<Transport>> {
        self.paths.load().get(k).and_then(Weak::upgrade)
    }

    pub fn add_tcp(&self, id: u64, tx: mpsc::Sender<Vec<u8>>) {
        lock(&self.tcp).insert(id, tx);
    }

    pub fn drop_tcp(&self, id: u64) {
        lock(&self.tcp).remove(&id);
        self.paths.rcu(|m| {
            let mut m = HashMap::clone(m);
            m.remove(&PathKey::Tcp(id));
            m
        });
    }

    pub fn send_to(&self, k: PathKey, b: &[u8]) -> bool {
        let ok = match k {
            PathKey::Udp(a) => self.udp.try_send_to(b, a).is_ok(),
            PathKey::Tcp(id) => lock(&self.tcp).get(&id).map(|tx| tx.try_send(core::frame(b)).is_ok()).unwrap_or(false),
        };
        if !ok {
            self.counters.send_fail.fetch_add(1, Ordering::Relaxed);
        }
        ok
    }

    pub fn tcp_ids(&self) -> Vec<u64> {
        lock(&self.tcp).keys().copied().collect()
    }

    pub fn register(&self, t: &Arc<Transport>) {
        let mut a = lock(&self.all);
        a.retain(|w| w.strong_count() > 0);
        a.push(Arc::downgrade(t));
    }

    pub fn transports(&self) -> Vec<Arc<Transport>> {
        lock(&self.all).iter().filter_map(Weak::upgrade).collect()
    }

    pub fn up(&self) -> Option<&Arc<dyn Upcall>> {
        self.up.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Publish,
    Subscribe,
    Single,
}

impl Role {
    pub fn publishes(self) -> bool {
        matches!(self, Role::Publish | Role::Single)
    }

    pub fn subscribes(self) -> bool {
        matches!(self, Role::Subscribe | Role::Single)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Publish => "publish",
            Role::Subscribe => "subscribe",
            Role::Single => "single",
        }
    }
}

#[derive(Default)]
pub struct Ingress {
    pub by_ssrc: HashMap<u32, (Arc<PubTrack>, Arc<Phys>, bool)>,
    pub by_mid: HashMap<String, Arc<PubTrack>>,
    pub mid_id: u8,
    pub rid_id: u8,
    pub rrid_id: u8,
    pub twcc_id: u8,
    pub level_id: u8,
}

pub struct Dc {
    tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    backlog: Mutex<VecDeque<Vec<u8>>>,
    pub open: AtomicBool,
    pub dropped: AtomicU64,
}

impl Dc {
    fn new() -> Dc {
        Dc { tx: Mutex::new(None), backlog: Mutex::new(VecDeque::new()), open: AtomicBool::new(false), dropped: AtomicU64::new(0) }
    }

    pub fn send(&self, frame: Vec<u8>) -> bool {
        if self.open.load(Ordering::Acquire)
            && let Some(tx) = lock(&self.tx).as_ref()
        {
            if tx.try_send(frame).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            return true;
        }
        let mut b = lock(&self.backlog);
        if b.len() >= DC_BACKLOG {
            b.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        b.push_back(frame);
        false
    }

    pub fn opened(&self, tx: mpsc::Sender<Vec<u8>>) {
        let pending: Vec<Vec<u8>> = lock(&self.backlog).drain(..).collect();
        for f in pending {
            let _ = tx.try_send(f);
        }
        let _ = lock(&self.tx).replace(tx);
        self.open.store(true, Ordering::Release);
    }
}

pub struct Transport {
    pub id: u64,
    pub user: String,
    pub session: String,
    pub role: Role,
    pub pc_mode: PcMode,
    pub sfu_id: String,
    pub net: Weak<Net>,
    pub creds: Mutex<Creds>,
    pub pairs: Mutex<Pairs>,
    path: ArcSwap<Option<PathKey>>,
    pub state: AtomicU8,
    pub suspect_since: AtomicU64,
    pub dtls_in: mpsc::Sender<Vec<u8>>,
    pub srtp_rx: Mutex<Option<webrtc_srtp::context::Context>>,
    pub srtp_tx: Mutex<Option<webrtc_srtp::context::Context>>,
    pub dc: Dc,
    pub table: Mutex<Table>,
    pub ack: AtomicU64,
    pub downs: ArcSwap<HashMap<u32, Arc<Down>>>,
    pub ingress: ArcSwap<Ingress>,
    pub pub_room: Arc<ArcSwap<Option<String>>>,
    pub gates: ArcSwap<HashMap<String, Arc<AtomicU64>>>,
    pub twcc_seq: AtomicU32,
    pub sent: Mutex<crate::sim::SendLog>,
    pub bwe: Mutex<crate::sim::Bwe>,
    pub nack_budget: Arc<Mutex<crate::rtcp::core::Budget>>,
    pub twcc_rx: Mutex<crate::rtcp::core::TwccRecv>,
    pub dtls_up: AtomicBool,
    pub closed: AtomicBool,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    pub stop: tokio::sync::watch::Sender<bool>,
    pub created_ms: u64,
}

impl Transport {
    #[allow(clippy::too_many_arguments)]
    pub fn new(net: &Arc<Net>, user: &str, session: &str, role: Role, pc_mode: PcMode, sfu_id: &str, pub_room: Arc<ArcSwap<Option<String>>>, nack_budget: Arc<Mutex<crate::rtcp::core::Budget>>) -> Arc<Transport> {
        let (dtls_tx, dtls_rx) = mpsc::channel(512);
        let (stop, _) = tokio::sync::watch::channel(false);
        let t = Arc::new(Transport {
            id: net.next_id(),
            user: user.to_string(),
            session: session.to_string(),
            role,
            pc_mode,
            sfu_id: sfu_id.to_string(),
            net: Arc::downgrade(net),
            creds: Mutex::new(Creds::new(fresh_cred())),
            pairs: Mutex::new(Pairs::default()),
            path: ArcSwap::from_pointee(None),
            state: AtomicU8::new(0),
            suspect_since: AtomicU64::new(0),
            dtls_in: dtls_tx,
            srtp_rx: Mutex::new(None),
            srtp_tx: Mutex::new(None),
            dc: Dc::new(),
            table: Mutex::new(Table::new(sfu_id, pc_mode)),
            ack: AtomicU64::new(0),
            downs: ArcSwap::from_pointee(HashMap::new()),
            ingress: ArcSwap::from_pointee(Ingress::default()),
            pub_room,
            gates: ArcSwap::from_pointee(HashMap::new()),
            twcc_seq: AtomicU32::new(0),
            sent: Mutex::new(crate::sim::SendLog::default()),
            bwe: Mutex::new(crate::sim::Bwe::default()),
            nack_budget,
            twcc_rx: Mutex::new(crate::rtcp::core::TwccRecv::default()),
            dtls_up: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            tasks: Mutex::new(Vec::new()),
            stop,
            created_ms: common::time::mono_ms(),
        });
        let ufrag = lock(&t.creds).current.ufrag.clone();
        net.bind_ufrag(&ufrag, &t);
        net.register(&t);
        let h = tokio::spawn(shell::dtls_task(Arc::downgrade(&t), dtls_rx, net.clone()));
        lock(&t.tasks).push(h);
        t
    }

    pub fn add_task(&self, h: tokio::task::JoinHandle<()>) {
        lock(&self.tasks).push(h);
    }

    pub fn cred(&self) -> Cred {
        lock(&self.creds).current.clone()
    }

    pub fn path(&self) -> Option<PathKey> {
        Option::clone(&self.path.load())
    }

    pub fn set_path(&self, p: Option<PathKey>) {
        self.path.store(Arc::new(p));
    }

    pub fn peer_state(&self) -> PeerState {
        if self.state.load(Ordering::Relaxed) == 1 { PeerState::Suspect } else { PeerState::Alive }
    }

    pub fn send_raw(&self, b: &[u8]) -> bool {
        let (Some(p), Some(net)) = (self.path(), self.net.upgrade()) else { return false };
        net.send_to(p, b)
    }

    pub fn send_rtp(&self, plain: &[u8]) -> bool {
        let enc = {
            let mut g = lock(&self.srtp_tx);
            let Some(ctx) = g.as_mut() else { return false };
            match ctx.encrypt_rtp(plain) {
                Ok(b) => b,
                Err(_) => return false,
            }
        };
        self.send_raw(&enc)
    }

    pub fn send_rtcp(&self, plain: &[u8]) -> bool {
        let enc = {
            let mut g = lock(&self.srtp_tx);
            let Some(ctx) = g.as_mut() else { return false };
            match ctx.encrypt_rtcp(plain) {
                Ok(b) => b,
                Err(_) => return false,
            }
        };
        self.send_raw(&enc)
    }

    pub fn ice_restart(self: &Arc<Self>) -> Cred {
        let fresh = fresh_cred();
        let retired = lock(&self.creds).restart(fresh.clone());
        if let Some(net) = self.net.upgrade() {
            if let Some(r) = retired {
                net.unbind_ufrag(&r);
            }
            net.bind_ufrag(&fresh.ufrag, self);
        }
        fresh
    }

    pub fn on_check(self: &Arc<Self>, net: &Arc<Net>, k: PathKey, ufrag: &str, nominated: bool, now_ms: u64) {
        let retired = lock(&self.creds).checked(ufrag);
        if let Some(r) = &retired {
            net.unbind_ufrag(r);
            tracing::info!(kind = "ev", spec = "LINK.S.checked_new", user = %self.user, session = %self.session, ufrag = %ufrag, retired = %r, "new ICE credentials in use");
        }
        net.bind_path(k, self);
        let live = lock(&self.pairs).checked(k, nominated, now_ms);
        if nominated {
            let changed = self.path() != Some(k);
            self.set_path(Some(k));
            if changed {
                tracing::info!(kind = "ev", spec = "RULE-LINK-TRANSPORT", user = %self.user, session = %self.session, role = self.role.as_str(), path = ?k, "path follows nomination");
            }
        }
        if live == Liveness::BecameAlive || (nominated && self.state.load(Ordering::Relaxed) == 1) {
            let since = self.suspect_since.swap(0, Ordering::AcqRel);
            self.state.store(0, Ordering::Relaxed);
            tracing::info!(kind = "ev", spec = "IMPL-LINK-PEER-STATE", user = %self.user, session = %self.session, dwell_ms = now_ms.saturating_sub(since), "suspect → alive");
        }
    }

    pub fn tick(self: &Arc<Self>, now_ms: u64) -> bool {
        let live = lock(&self.pairs).tick(now_ms);
        if live == Liveness::BecameSuspect && self.suspect_since.compare_exchange(0, now_ms.max(1), Ordering::AcqRel, Ordering::Relaxed).is_ok() {
            self.state.store(1, Ordering::Relaxed);
            self.set_path(None);
            return true;
        }
        false
    }

    pub fn ufrags(&self) -> Vec<String> {
        lock(&self.creds).ufrags()
    }

    pub fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.stop.send(true);
        for h in lock(&self.tasks).drain(..) {
            h.abort();
        }
        if let Some(net) = self.net.upgrade() {
            for u in self.ufrags() {
                net.unbind_ufrag(&u);
            }
            net.unbind_paths_of(self.id);
        }
        self.downs.store(Arc::new(HashMap::new()));
        self.ingress.store(Arc::new(Ingress::default()));
    }

    pub fn set_ingress(&self, f: impl Fn(&mut Ingress)) {
        let cur = self.ingress.load();
        let mut next = Ingress {
            by_ssrc: cur.by_ssrc.clone(),
            by_mid: cur.by_mid.clone(),
            mid_id: cur.mid_id,
            rid_id: cur.rid_id,
            rrid_id: cur.rrid_id,
            twcc_id: cur.twcc_id,
            level_id: cur.level_id,
        };
        f(&mut next);
        self.ingress.store(Arc::new(next));
    }

    pub fn uplink_ssrcs(&self) -> Vec<u32> {
        self.ingress.load().by_ssrc.keys().copied().collect()
    }

    pub fn down(&self, ssrc: u32) -> Option<Arc<Down>> {
        self.downs.load().get(&ssrc).cloned()
    }

    pub fn put_down(&self, d: Arc<Down>) {
        self.downs.rcu(|m| {
            let mut m = HashMap::clone(m);
            m.insert(d.ssrc, d.clone());
            if let Some(r) = d.rtx_ssrc {
                m.insert(r, d.clone());
            }
            m
        });
    }

    pub fn all_downs(&self) -> Vec<Arc<Down>> {
        let mut v: Vec<Arc<Down>> = self.downs.load().iter().filter(|(k, d)| **k == d.ssrc).map(|(_, d)| d.clone()).collect();
        v.sort_by_key(|d| d.mid);
        v
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Prefs {
    pub spatial: Option<u8>,
    pub paused: bool,
    pub priority: u8,
}

pub struct Down {
    pub transport: Weak<Transport>,
    pub user: String,
    pub mid: u16,
    pub kind: Kind,
    pub slot: bool,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub pt: AtomicU8,
    pub rtx_pt: AtomicU8,
    pub clock: u32,
    pub ext: ArcSwap<ExtTable>,
    pub since: AtomicU64,
    pub carry: ArcSwap<Option<(String, String)>>,
    pub track: ArcSwap<Option<Arc<PubTrack>>>,
    pub cur: ArcSwap<Option<Arc<Phys>>>,
    pub rw: Mutex<Rewriter>,
    pub sel: Mutex<Switch>,
    pub prefs: Mutex<Prefs>,
    pub rtx_seq: AtomicU16,
    pub egress: AtomicU64,
    pub egress_bytes: AtomicU64,
    pub gate_drops: AtomicU64,
    pub rtx_sent: AtomicU64,
    pub rtx_gate: AtomicBool,
    pub stall_mark: AtomicU64,
    pub need_kf: AtomicBool,
    pub last_egress_ms: AtomicU64,
}

impl Down {
    pub fn new(t: &Arc<Transport>, row: &table::Row, pt: u8, rtx_pt: Option<u8>, ext: ExtTable) -> Arc<Down> {
        let clock = table::clock_of(&row.tuple.codec);
        Arc::new(Down {
            transport: Arc::downgrade(t),
            user: t.user.clone(),
            mid: row.mid,
            kind: row.kind,
            slot: row.class == table::RowClass::Slot,
            ssrc: row.ssrc,
            rtx_ssrc: row.rtx_ssrc,
            pt: AtomicU8::new(pt),
            rtx_pt: AtomicU8::new(rtx_pt.unwrap_or(0)),
            clock,
            ext: ArcSwap::from_pointee(ext),
            since: AtomicU64::new(row.since),
            carry: ArcSwap::from_pointee(row.track.clone()),
            track: ArcSwap::from_pointee(None),
            cur: ArcSwap::from_pointee(None),
            rw: Mutex::new(Rewriter::new(clock)),
            sel: Mutex::new(Switch::new(crate::sim::core::Layer::High)),
            prefs: Mutex::new(Prefs::default()),
            rtx_seq: AtomicU16::new((fresh_u32() & 0x7fff) as u16),
            egress: AtomicU64::new(0),
            egress_bytes: AtomicU64::new(0),
            gate_drops: AtomicU64::new(0),
            rtx_sent: AtomicU64::new(0),
            rtx_gate: AtomicBool::new(false),
            stall_mark: AtomicU64::new(0),
            need_kf: AtomicBool::new(row.kind == Kind::Video),
            last_egress_ms: AtomicU64::new(0),
        })
    }

    pub fn acked(&self) -> bool {
        let Some(t) = self.transport.upgrade() else { return false };
        self.since.load(Ordering::Acquire) <= t.ack.load(Ordering::Acquire)
    }

    pub fn room(&self) -> Option<String> {
        self.carry.load().as_ref().as_ref().map(|(_, r)| r.clone())
    }

    pub fn track_id(&self) -> Option<String> {
        self.carry.load().as_ref().as_ref().map(|(t, _)| t.clone())
    }
}

pub fn to_addr(ip: IpAddr, port: u16) -> SocketAddr {
    SocketAddr::new(ip, port)
}
