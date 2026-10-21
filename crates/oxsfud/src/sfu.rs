// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-CONC-SIX 정:IMPL-ROOM-VERSION-ISSUE 정:RULE-BUS-EVENTS 정:RULE-OPS-OBSERVE 정:RULE-LINK-TRANSPORT · model: claude-opus-5-5
//! 프로세스 전역 — 방 · Peer · 전송 색인(RCU) · 이벤트 출구 · 계수.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use arc_swap::ArcSwap;
use common::keyspace::Keys;
use common::policy::Policy;
use oxsig::Op;
use oxsig::bplane::UnitEvent;
use tokio::sync::mpsc;

use crate::peer::{Conn, Peer};
use crate::room::Room;

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn now_ms() -> u64 {
    common::ident::now_ms()
}

pub fn mono_ms() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64 + 1
}

#[derive(Default)]
pub struct Counters {
    pub codes: Mutex<HashMap<u16, u64>>,
    pub drops: Mutex<HashMap<&'static str, u64>>,
}

impl Counters {
    pub fn code(&self, c: u16) {
        lock(&self.codes).entry(c).and_modify(|v| *v += 1).or_insert(1);
    }

    pub fn drop(&self, reason: &'static str) {
        lock(&self.drops).entry(reason).and_modify(|v| *v += 1).or_insert(1);
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let codes: serde_json::Map<String, serde_json::Value> =
            lock(&self.codes).iter().map(|(k, v)| (k.to_string(), (*v).into())).collect();
        let drops: serde_json::Map<String, serde_json::Value> =
            lock(&self.drops).iter().map(|(k, v)| (k.to_string(), (*v).into())).collect();
        serde_json::json!({ "codes": codes, "drops": drops })
    }
}

pub enum Out {
    Room { room: String, ev: UnitEvent },
    User { user: String, ev: UnitEvent },
    Token { room: String, up: bool },
}

pub struct Sfu {
    pub node: String,
    pub epoch: String,
    pub build: String,
    pub policy: Policy,
    pub keys: Keys,
    pub ip: IpAddr,
    pub udp_port: u16,
    pub tcp_port: Option<u16>,
    pub cert: dtls::crypto::Certificate,
    pub fingerprint: String,
    pub udp: Arc<tokio::net::UdpSocket>,
    pub rooms: Mutex<HashMap<String, Arc<Room>>>,
    pub peers: Mutex<HashMap<String, Arc<Peer>>>,
    pub arevs: Mutex<HashMap<String, u64>>,
    pub ufrags: ArcSwap<HashMap<String, Weak<Conn>>>,
    pub addrs: ArcSwap<HashMap<SocketAddr, Weak<Conn>>>,
    pub index_lock: Mutex<()>,
    pub out_tx: mpsc::UnboundedSender<Out>,
    pub out_rx: Mutex<Option<mpsc::UnboundedReceiver<Out>>>,
    pub counters: Counters,
    pub seq: AtomicU64,
    pub beat: Arc<AtomicU64>,
    pub nodes: Mutex<HashMap<String, Vec<String>>>,
    pub bus_up: std::sync::atomic::AtomicBool,
}

fn make_cert() -> Result<(dtls::crypto::Certificate, String), String> {
    use sha2::Digest;
    let mut params = rcgen::CertificateParams::new(vec!["ox-sfu".to_string()]).map_err(|e| e.to_string())?;
    let mut dn = rcgen::DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, "ox-sfu");
    params.distinguished_name = dn;
    let key = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = params.self_signed(&key).map_err(|e| e.to_string())?;
    let der = cert.der().to_owned();
    let digest = sha2::Sha256::digest(der.as_ref());
    let hex: Vec<String> = digest.iter().map(|b| format!("{b:02X}")).collect();
    let private_key = dtls::crypto::CryptoPrivateKey::try_from(&key).map_err(|e| e.to_string())?;
    Ok((dtls::crypto::Certificate { certificate: vec![der], private_key }, format!("sha-256 {}", hex.join(":"))))
}

impl Sfu {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        node: String,
        build: String,
        policy: Policy,
        namespace: &str,
        ip: IpAddr,
        udp_port: u16,
        tcp_port: Option<u16>,
        udp: Arc<tokio::net::UdpSocket>,
    ) -> Result<Arc<Sfu>, String> {
        let (cert, fingerprint) = make_cert()?;
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        Ok(Arc::new(Sfu {
            node,
            epoch: common::ident::new_epoch(),
            build,
            policy,
            keys: Keys::new(namespace),
            ip,
            udp_port,
            tcp_port,
            cert,
            fingerprint,
            udp,
            rooms: Mutex::new(HashMap::new()),
            peers: Mutex::new(HashMap::new()),
            arevs: Mutex::new(HashMap::new()),
            ufrags: ArcSwap::from_pointee(HashMap::new()),
            addrs: ArcSwap::from_pointee(HashMap::new()),
            index_lock: Mutex::new(()),
            out_tx,
            out_rx: Mutex::new(Some(out_rx)),
            counters: Counters::default(),
            seq: AtomicU64::new(1),
            beat: Arc::new(AtomicU64::new(0)),
            nodes: Mutex::new(HashMap::new()),
            bus_up: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    pub fn next_id(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    pub fn room(&self, id: &str) -> Option<Arc<Room>> {
        lock(&self.rooms).get(id).cloned()
    }

    pub fn peer(&self, sid: &str) -> Option<Arc<Peer>> {
        lock(&self.peers).get(sid).cloned()
    }

    pub fn peer_of_user(&self, user: &str) -> Vec<Arc<Peer>> {
        lock(&self.peers).values().filter(|p| p.user_id == user).cloned().collect()
    }

    pub fn emit_user(&self, user: &str, op: Op, body: &impl serde::Serialize) {
        let ev = UnitEvent { op: op.as_u16(), body: serde_json::to_string(body).unwrap_or_default(), exclude: None };
        let _ = self.out_tx.send(Out::User { user: user.to_string(), ev });
    }

    pub fn room_token(&self, room: &str, up: bool) {
        let _ = self.out_tx.send(Out::Token { room: room.to_string(), up });
    }

    pub fn index_ufrag(&self, ufrag: &str, conn: &Arc<Conn>) {
        let _g = lock(&self.index_lock);
        let mut m = (**self.ufrags.load()).clone();
        m.insert(ufrag.to_string(), Arc::downgrade(conn));
        self.ufrags.store(Arc::new(m));
    }

    pub fn unindex_ufrag(&self, ufrag: &str) {
        let _g = lock(&self.index_lock);
        let mut m = (**self.ufrags.load()).clone();
        if m.remove(ufrag).is_some() {
            self.ufrags.store(Arc::new(m));
        }
    }

    pub fn index_addr(&self, addr: SocketAddr, conn: &Arc<Conn>) {
        let cur = self.addrs.load();
        if let Some(w) = cur.get(&addr)
            && w.upgrade().is_some_and(|c| Arc::ptr_eq(&c, conn))
        {
            return;
        }
        let _g = lock(&self.index_lock);
        let mut m = (**self.addrs.load()).clone();
        m.insert(addr, Arc::downgrade(conn));
        self.addrs.store(Arc::new(m));
    }

    pub fn unindex_conn(&self, conn: &Arc<Conn>) {
        let _g = lock(&self.index_lock);
        let mut a = (**self.addrs.load()).clone();
        a.retain(|_, w| w.upgrade().is_some_and(|c| !Arc::ptr_eq(&c, conn)));
        self.addrs.store(Arc::new(a));
        let mut u = (**self.ufrags.load()).clone();
        u.retain(|_, w| w.upgrade().is_some_and(|c| !Arc::ptr_eq(&c, conn)));
        self.ufrags.store(Arc::new(u));
    }

    pub fn conn_by_addr(&self, addr: &SocketAddr) -> Option<Arc<Conn>> {
        self.addrs.load().get(addr).and_then(Weak::upgrade)
    }

    pub fn conn_by_ufrag(&self, ufrag: &str) -> Option<Arc<Conn>> {
        self.ufrags.load().get(ufrag).and_then(Weak::upgrade)
    }
}
