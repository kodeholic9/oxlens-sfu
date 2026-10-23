// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-LINK-SERVER-SCOPE · 정:IMPL-LINK-PEER-STATE · 정:RULE-LINK-TRANSPORT · 정:IMPL-SUB-TABLE · 정:IMPL-GC-PEER-REAP · 정:RULE-CONC-SIX · 연:LINK.type.ServerConfig · 정:RULE-CONC-STATE-VOCAB · model: claude-opus-5-5
//! LINK — Peer(세션 × 이 서버 · 수명 = 세션) · 전송 객체(자격 한 벌 = 연결 하나) · 배정표 줄의 다운링크 스트림 · 등록부(ufrag · 주소 → 전송 객체).

pub mod core;
pub mod shell;
pub mod table;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use arc_swap::{ArcSwap, ArcSwapOption};
use dashmap::DashMap;
use oxsig::link::{DtlsInfo, IceInfo};
use oxsig::room::Permission;
use oxsig::ses::PcMode;
use oxsig::sub::MlineTable;
use oxsig::trk::MediaKind;
use serde_json::Value;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use self::core::{Creds, Pairs, PeerState, Proto};
use self::table::{Class, Table};
use crate::fwd::core::Rewriter;
use crate::r#pub::{PubStream, PubTrack};
use crate::rtcp::core::TwccRecv;

pub fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Load {
    pub track_id: String,
    pub room_id: String,
}

#[derive(Debug, Default)]
pub struct DownState {
    pub rw: Rewriter,
    pub src_user: Option<String>,
    pub src_track: Option<String>,
    pub src_ssrc: u32,
    pub src_phys: Option<Weak<PubTrack>>,
    pub rtx_seq: u16,
    pub mid_at: u64,
    pub nack_closed: bool,
    pub sel: crate::sim::Sel,
}

pub struct Down {
    pub mid: u16,
    pub kind: MediaKind,
    pub class: Class,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub pt: AtomicU8,
    pub rtx_pt: AtomicU8,
    pub since: AtomicU64,
    pub track: ArcSwapOption<Load>,
    pub peer: Weak<Peer>,
    pub st: Mutex<DownState>,
    pub sent: AtomicU64,
    pub bytes: AtomicU64,
    pub last_at: AtomicU64,
}

impl Down {
    pub fn loaded(&self) -> Option<Arc<Load>> {
        self.track.load_full()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Publish,
    Subscribe,
    Both,
}

pub struct Dc {
    pub tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
    pub open: bool,
    pub buf: VecDeque<Vec<u8>>,
    pub dropped: u64,
}

pub struct Transport {
    pub role: Role,
    pub user_id: String,
    pub peer: Weak<Peer>,
    pub creds: Mutex<Creds>,
    pub pairs: Mutex<Pairs>,
    pub state: AtomicU8,
    pub suspect_since: AtomicU64,
    pub udp: Arc<UdpSocket>,
    pub tcp: DashMap<SocketAddr, mpsc::UnboundedSender<Vec<u8>>>,
    pub dtls_tx: Mutex<Option<mpsc::Sender<bytes::Bytes>>>,
    pub dtls_from: Mutex<Option<SocketAddr>>,
    pub srtp_in: Mutex<Option<webrtc_srtp::context::Context>>,
    pub srtp_out: Mutex<Option<webrtc_srtp::context::Context>>,
    pub dc: Mutex<Dc>,
    pub ingress: DashMap<u32, Arc<PubTrack>>,
    pub rtx_in: DashMap<u32, Arc<PubTrack>>,
    pub twcc_in: Mutex<TwccRecv>,
    pub twcc_id: AtomicU8,
    pub twcc_out: AtomicU16,
    pub tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    pub closed: AtomicBool,
    pub connected_at: AtomicU64,
    pub rx_packets: AtomicU64,
    pub tx_packets: AtomicU64,
    pub sim: Mutex<crate::sim::Bwe>,
}

impl Transport {
    pub fn peer_state(&self) -> PeerState {
        if self.state.load(Ordering::Acquire) == 1 { PeerState::Suspect } else { PeerState::Alive }
    }

    pub fn set_state(&self, s: PeerState) -> PeerState {
        let v = if s == PeerState::Suspect { 1 } else { 0 };
        if self.state.swap(v, Ordering::AcqRel) == 1 { PeerState::Suspect } else { PeerState::Alive }
    }

    pub fn ufrag(&self) -> String {
        lock(&self.creds).generation().to_string()
    }

    pub fn send_raw(&self, data: &[u8]) -> bool {
        if self.closed.load(Ordering::Acquire) {
            return false;
        }
        let now = common::clock::mono_ms();
        let target = {
            let p = lock(&self.pairs);
            p.send_to(now).or_else(|| {
                let a = (*lock(&self.dtls_from))?;
                Some((a, p.map.get(&a).map(|x| x.proto).unwrap_or(Proto::Udp)))
            })
        };
        let Some((addr, proto)) = target else { return false };
        match proto {
            Proto::Udp => self.udp.try_send_to(data, addr).is_ok(),
            Proto::Tcp => match (self.tcp.get(&addr), self::core::frame(data)) {
                (Some(w), Some(f)) => w.send(f).is_ok(),
                _ => false,
            },
        }
    }

    pub fn send_rtp(&self, plain: &[u8]) -> bool {
        let enc = {
            let mut g = lock(&self.srtp_out);
            match g.as_mut() {
                Some(c) => c.encrypt_rtp(plain).ok(),
                None => None,
            }
        };
        match enc {
            Some(b) => {
                self.tx_packets.fetch_add(1, Ordering::Relaxed);
                self.send_raw(&b)
            }
            None => false,
        }
    }

    pub fn send_rtcp(&self, plain: &[u8]) -> bool {
        let enc = {
            let mut g = lock(&self.srtp_out);
            match g.as_mut() {
                Some(c) => c.encrypt_rtcp(plain).ok(),
                None => None,
            }
        };
        match enc {
            Some(b) => self.send_raw(&b),
            None => false,
        }
    }

    pub fn dc_send(&self, frame: Vec<u8>) -> bool {
        let mut d = lock(&self.dc);
        if d.open
            && let Some(tx) = &d.tx {
                return tx.send(frame).is_ok();
            }
        if d.buf.len() >= crate::aux::DC_BUFFER {
            d.buf.pop_front();
            d.dropped += 1;
        }
        d.buf.push_back(frame);
        false
    }

    pub fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        for h in lock(&self.tasks).drain(..) {
            h.abort();
        }
        lock(&self.dtls_tx).take();
        lock(&self.dc).tx.take();
        self.tcp.clear();
    }

    pub fn has_dtls(&self) -> bool {
        lock(&self.srtp_out).is_some()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubExt {
    pub mid: Option<u8>,
    pub audio_level: Option<u8>,
    pub abs_send_time: Option<u8>,
    pub twcc: Option<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gate {
    pub open: bool,
    pub gated_at: u64,
    pub first_ready_at: u64,
    pub warned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerPref {
    pub spatial: Option<u8>,
    pub temporal: Option<u8>,
    pub paused: bool,
    pub priority: u8,
}

impl Default for LayerPref {
    fn default() -> Self {
        LayerPref { spatial: None, temporal: None, paused: false, priority: oxsig::sub::PRIORITY_DEFAULT }
    }
}

pub struct Ident {
    pub participant_type: u8,
    pub hidden: bool,
    pub metadata: Option<Value>,
    pub permission: Permission,
}

pub struct Peer {
    pub user_id: String,
    pub session_id: Mutex<String>,
    pub pc_mode: PcMode,
    pub ident: Ident,
    pub home: Mutex<(String, String)>,
    pub pub_t: Arc<Transport>,
    pub sub_t: Option<Arc<Transport>>,
    pub table: Mutex<Table>,
    pub downs: Mutex<BTreeMap<u16, Arc<Down>>>,
    pub ack: AtomicU64,
    pub ext: SubExt,
    pub rooms: Mutex<BTreeSet<String>>,
    pub pub_room: ArcSwapOption<String>,
    pub pubs: Mutex<Vec<Arc<PubStream>>>,
    pub gates: Mutex<BTreeMap<String, Gate>>,
    pub gate_closed: ArcSwap<BTreeMap<String, u64>>,
    pub layers: Mutex<BTreeMap<String, LayerPref>>,
    pub created_at: u64,
    pub reaped: AtomicBool,
    pub sfu_id: String,
    pub nack_budget: Mutex<(u64, u32)>,
}

impl Peer {
    pub fn sub_transport(&self) -> &Arc<Transport> {
        self.sub_t.as_ref().unwrap_or(&self.pub_t)
    }

    pub fn transports(&self) -> Vec<Arc<Transport>> {
        let mut v = vec![self.pub_t.clone()];
        if let Some(s) = &self.sub_t {
            v.push(s.clone());
        }
        v
    }

    pub fn pub_room(&self) -> Option<String> {
        self.pub_room.load_full().map(|s| s.as_ref().clone())
    }

    pub fn in_room(&self, room: &str) -> bool {
        lock(&self.rooms).contains(room)
    }

    pub fn room_list(&self) -> Vec<String> {
        lock(&self.rooms).iter().cloned().collect()
    }

    pub fn table_wire(&self) -> MlineTable {
        lock(&self.table).wire(&self.sfu_id)
    }

    pub fn session(&self) -> String {
        lock(&self.session_id).clone()
    }

    pub fn down(&self, mid: u16) -> Option<Arc<Down>> {
        lock(&self.downs).get(&mid).cloned()
    }

    pub fn ice(&self, ip: IpAddr, port: u16, tcp_port: Option<u16>) -> IceInfo {
        let p = lock(&self.pub_t.creds).cur.clone();
        let s = self.sub_t.as_ref().map(|t| lock(&t.creds).cur.clone()).unwrap_or_else(|| p.clone());
        IceInfo { ip: ip.to_string(), port, publish_ufrag: p.ufrag, publish_pwd: p.pwd, subscribe_ufrag: s.ufrag, subscribe_pwd: s.pwd, tcp_port }
    }

    pub fn active_pubs(&self) -> usize {
        lock(&self.pubs).iter().filter(|p| !p.gone.load(Ordering::Acquire)).count()
    }

    pub fn pub_by_id(&self, track_id: &str) -> Option<Arc<PubStream>> {
        lock(&self.pubs).iter().find(|p| p.track_id == track_id && !p.gone.load(Ordering::Acquire)).cloned()
    }

    pub fn ensure_down(self: &Arc<Self>, mid: u16) -> Option<Arc<Down>> {
        if let Some(d) = self.down(mid) {
            return Some(d);
        }
        let t = lock(&self.table);
        let r = t.rows.iter().find(|r| r.mid == mid)?;
        let d = Arc::new(Down {
            mid,
            kind: r.kind,
            class: r.class,
            ssrc: r.ssrc,
            rtx_ssrc: r.rtx_ssrc,
            pt: AtomicU8::new(r.pt),
            rtx_pt: AtomicU8::new(r.rtx_pt.unwrap_or(0)),
            since: AtomicU64::new(r.since),
            track: ArcSwapOption::empty(),
            peer: Arc::downgrade(self),
            st: Mutex::new(DownState::default()),
            sent: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            last_at: AtomicU64::new(0),
        });
        drop(t);
        lock(&self.downs).insert(mid, d.clone());
        Some(d)
    }
}

pub struct Cert {
    pub dtls: dtls::crypto::Certificate,
    pub fingerprint: String,
}

pub struct Links {
    pub peers: DashMap<String, Arc<Peer>>,
    pub ufrags: DashMap<String, Weak<Transport>>,
    pub addrs: DashMap<SocketAddr, Weak<Transport>>,
    pub arevs: DashMap<String, u64>,
    pub udp: Arc<UdpSocket>,
    pub cert: Cert,
    pub ip: IpAddr,
    pub udp_port: u16,
    pub tcp_port: Option<u16>,
    pub slot_reserve: usize,
    pub sfu_id: String,
    pub stun_bad: AtomicU64,
    pub unknown_addr: AtomicU64,
}

pub struct NewPeer {
    pub user_id: String,
    pub session_id: String,
    pub pc_mode: PcMode,
    pub ident: Ident,
    pub home: (String, String),
    pub ext: SubExt,
}

impl Links {
    pub fn peer(&self, user: &str) -> Option<Arc<Peer>> {
        self.peers.get(user).map(|p| p.clone())
    }

    pub fn dtls_info(&self) -> DtlsInfo {
        DtlsInfo { fingerprint: self.cert.fingerprint.clone(), setup: "passive".to_string() }
    }

    fn transport(&self, role: Role, user: &str, peer: &Weak<Peer>) -> Arc<Transport> {
        let t = Arc::new(Transport {
            role,
            user_id: user.to_string(),
            peer: peer.clone(),
            creds: Mutex::new(Creds::new()),
            pairs: Mutex::new(Pairs::default()),
            state: AtomicU8::new(0),
            suspect_since: AtomicU64::new(0),
            udp: self.udp.clone(),
            tcp: DashMap::new(),
            dtls_tx: Mutex::new(None),
            dtls_from: Mutex::new(None),
            srtp_in: Mutex::new(None),
            srtp_out: Mutex::new(None),
            dc: Mutex::new(Dc { tx: None, open: false, buf: VecDeque::new(), dropped: 0 }),
            ingress: DashMap::new(),
            rtx_in: DashMap::new(),
            twcc_in: Mutex::new(TwccRecv::default()),
            twcc_id: AtomicU8::new(0),
            twcc_out: AtomicU16::new(0),
            tasks: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
            connected_at: AtomicU64::new(0),
            rx_packets: AtomicU64::new(0),
            tx_packets: AtomicU64::new(0),
            sim: Mutex::new(crate::sim::Bwe::default()),
        });
        self.ufrags.insert(t.ufrag(), Arc::downgrade(&t));
        t
    }

    pub fn create(&self, n: NewPeer) -> Arc<Peer> {
        let arev = self.arevs.get(&n.session_id).map(|a| *a).unwrap_or(0);
        let peer = Arc::new_cyclic(|w: &Weak<Peer>| {
            let (pub_t, sub_t) = match n.pc_mode {
                PcMode::One => (self.transport(Role::Both, &n.user_id, w), None),
                PcMode::Two => (self.transport(Role::Publish, &n.user_id, w), Some(self.transport(Role::Subscribe, &n.user_id, w))),
            };
            let mut table = Table::new(n.pc_mode, self.slot_reserve);
            table.arev = arev;
            table.ack = arev;
            Peer {
                user_id: n.user_id.clone(),
                session_id: Mutex::new(n.session_id.clone()),
                pc_mode: n.pc_mode,
                ident: n.ident,
                home: Mutex::new(n.home),
                pub_t,
                sub_t,
                table: Mutex::new(table),
                downs: Mutex::new(BTreeMap::new()),
                ack: AtomicU64::new(arev),
                ext: n.ext,
                rooms: Mutex::new(BTreeSet::new()),
                pub_room: ArcSwapOption::empty(),
                pubs: Mutex::new(Vec::new()),
                gates: Mutex::new(BTreeMap::new()),
                gate_closed: ArcSwap::from_pointee(BTreeMap::new()),
                layers: Mutex::new(BTreeMap::new()),
                created_at: common::clock::wall_ms(),
                reaped: AtomicBool::new(false),
                sfu_id: self.sfu_id.clone(),
                nack_budget: Mutex::new((0, 0)),
            }
        });
        self.peers.insert(n.user_id, peer.clone());
        peer
    }

    pub fn reap(&self, p: &Arc<Peer>) -> bool {
        if p.reaped.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.arevs.insert(p.session(), lock(&p.table).arev);
        for t in p.transports() {
            let ufrags: Vec<String> = {
                let c = lock(&t.creds);
                let mut v = vec![c.cur.ufrag.clone()];
                if let Some(x) = &c.pend {
                    v.push(x.ufrag.clone());
                }
                v
            };
            for u in ufrags {
                self.ufrags.remove(&u);
            }
            let addrs: Vec<SocketAddr> = lock(&t.pairs).map.keys().cloned().collect();
            for a in addrs {
                self.addrs.remove_if(&a, |_, w| w.upgrade().map(|x| Arc::ptr_eq(&x, &t)).unwrap_or(true));
            }
            t.close();
        }
        self.peers.remove_if(&p.user_id, |_, x| Arc::ptr_eq(x, p));
        true
    }

    pub fn restart(&self, p: &Arc<Peer>) -> Vec<(Role, self::core::Cred)> {
        let mut out = Vec::new();
        for t in p.transports() {
            let (n, dropped) = lock(&t.creds).restart();
            if let Some(d) = dropped {
                self.ufrags.remove(&d);
            }
            self.ufrags.insert(n.ufrag.clone(), Arc::downgrade(&t));
            out.push((t.role, n));
        }
        out
    }

    pub fn by_ufrag(&self, u: &str) -> Option<Arc<Transport>> {
        self.ufrags.get(u).and_then(|w| w.upgrade())
    }

    pub fn by_addr(&self, a: &SocketAddr) -> Option<Arc<Transport>> {
        self.addrs.get(a).and_then(|w| w.upgrade())
    }

    pub fn all(&self) -> Vec<Arc<Peer>> {
        self.peers.iter().map(|p| p.clone()).collect()
    }

    pub fn downlink_ssrcs(p: &Peer) -> BTreeSet<u32> {
        lock(&p.table).ssrcs.clone()
    }
}

pub type Rows = ArcSwap<Vec<Weak<Down>>>;
