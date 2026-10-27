// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-TRANSPORT · 정:RULE-LINK-PAIR-LIVENESS · 정:IMPL-LINK-PEER-STATE · 정:IMPL-LINK-ICE-RESTART · 정:RULE-OPS-DROPS · 정:CAND-LINK-UDP-BUFFER · model: claude-opus-5-5
//! 전송 등록부와 소켓 — UDP 포트 하나 · ICE-TCP passive(RFC 4571) · demux(첫 바이트) · STUN 검증 차례(조회 → MI 검증 → 생존 → 지명 → 응답) · DTLS 넘김.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, Weak};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;

use super::ice::{self, Via};
use super::stun;
use super::tcp::{Deframer, IDENTIFY_MS, KEEPALIVE_MS, frame};
use super::transport::{PeerState, Transport};
use crate::ops::drops::{DROPS, bump};

pub const SOCKET_BUFFER: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Stun,
    Dtls,
    Rtp,
    Rtcp,
    Unknown,
}

pub fn classify(b: &[u8]) -> Class {
    match b.first() {
        Some(0..=3) => Class::Stun,
        Some(20..=63) => Class::Dtls,
        Some(128..=191) if b.len() >= 2 && (192..=223).contains(&b[1]) => Class::Rtcp,
        Some(128..=191) => Class::Rtp,
        _ => Class::Unknown,
    }
}

pub enum DcEvent {
    Open(Arc<Transport>),
    Message(Arc<Transport>, Vec<u8>),
}

pub enum LinkSignal {
    Alive(Arc<Transport>, u64),
    Promoted(Arc<Transport>, String),
}

pub struct Io {
    pub udp: Arc<UdpSocket>,
    pub local: SocketAddr,
    pub tcp_port: Option<u16>,
    tcp: RwLock<HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>>,
    ufrags: RwLock<HashMap<String, Weak<Transport>>>,
    by_via: RwLock<HashMap<Via, Weak<Transport>>>,
    all: RwLock<HashMap<u64, Weak<Transport>>>,
    next_id: AtomicU64,
    pub cert: dtls::crypto::Certificate,
    pub fingerprint: String,
    pub dc_events: mpsc::UnboundedSender<DcEvent>,
    pub signals: mpsc::UnboundedSender<LinkSignal>,
}

impl Io {
    pub fn new(udp: Arc<UdpSocket>, tcp_port: Option<u16>, cert: dtls::crypto::Certificate, dc_events: mpsc::UnboundedSender<DcEvent>, signals: mpsc::UnboundedSender<LinkSignal>) -> Io {
        let local = udp.local_addr().unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        let fingerprint = cert.certificate.first().map(|c| super::cert::fingerprint(c.as_ref())).unwrap_or_default();
        Io {
            udp,
            local,
            tcp_port,
            tcp: RwLock::new(HashMap::new()),
            ufrags: RwLock::new(HashMap::new()),
            by_via: RwLock::new(HashMap::new()),
            all: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            cert,
            fingerprint,
            dc_events,
            signals,
        }
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn register(&self, t: &Arc<Transport>) {
        let ufrags = t.ledger.lock().map(|l| l.ufrags()).unwrap_or_default();
        if let Ok(mut m) = self.ufrags.write() {
            for u in ufrags {
                m.insert(u, Arc::downgrade(t));
            }
        }
        if let Ok(mut a) = self.all.write() {
            a.insert(t.id, Arc::downgrade(t));
        }
    }

    pub fn register_ufrag(&self, ufrag: &str, t: &Arc<Transport>) {
        if let Ok(mut m) = self.ufrags.write() {
            m.insert(ufrag.to_string(), Arc::downgrade(t));
        }
    }

    pub fn unregister_ufrag(&self, ufrag: &str) {
        if let Ok(mut m) = self.ufrags.write() {
            m.remove(ufrag);
        }
    }

    pub fn unregister(&self, t: &Transport) {
        let ufrags = t.ledger.lock().map(|l| l.ufrags()).unwrap_or_default();
        if let Ok(mut m) = self.ufrags.write() {
            for u in ufrags {
                m.remove(&u);
            }
        }
        let mut conns = Vec::new();
        if let Ok(mut m) = self.by_via.write() {
            m.retain(|v, w| {
                let mine = w.upgrade().is_some_and(|x| x.id == t.id) || w.strong_count() == 0;
                if mine && let Via::Tcp(c, _) = v {
                    conns.push(*c);
                }
                !mine
            });
        }
        if let Ok(mut m) = self.tcp.write() {
            for c in conns {
                m.remove(&c);
            }
        }
        if let Ok(mut a) = self.all.write() {
            a.remove(&t.id);
        }
    }

    pub fn transports(&self) -> Vec<Arc<Transport>> {
        self.all.read().map(|a| a.values().filter_map(Weak::upgrade).collect()).unwrap_or_default()
    }

    pub fn by_via(&self, via: &Via) -> Option<Arc<Transport>> {
        self.by_via.read().ok().and_then(|m| m.get(via).and_then(Weak::upgrade))
    }

    pub fn send_via(&self, via: Via, bytes: &[u8]) -> bool {
        match via {
            Via::Udp(a) => {
                for _ in 0..3 {
                    match self.udp.try_send_to(bytes, a) {
                        Ok(_) => return true,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::yield_now(),
                        Err(_) => break,
                    }
                }
                Transport::note_send_fail();
                false
            }
            Via::Tcp(c, _) => {
                let Some(f) = frame(bytes) else { return false };
                let tx = self.tcp.read().ok().and_then(|m| m.get(&c).cloned());
                match tx {
                    Some(tx) => tx.send(f).is_ok(),
                    None => false,
                }
            }
        }
    }

    pub fn on_datagram(self: &Arc<Self>, data: &[u8], via: Via, now_ms: u64) -> Option<(Arc<Transport>, Class)> {
        match classify(data) {
            Class::Stun => {
                self.on_stun(data, via, now_ms);
                None
            }
            Class::Dtls => {
                let Some(t) = self.by_via(&via) else {
                    bump(&DROPS.rtp_unknown_path);
                    return None;
                };
                self.feed_dtls(&t, data);
                None
            }
            c @ (Class::Rtp | Class::Rtcp) => match self.by_via(&via) {
                Some(t) => Some((t, c)),
                None => {
                    bump(&DROPS.rtp_unknown_path);
                    None
                }
            },
            Class::Unknown => {
                bump(&DROPS.demux_unknown);
                None
            }
        }
    }

    fn on_stun(self: &Arc<Self>, data: &[u8], via: Via, now_ms: u64) {
        let Some(msg) = stun::parse(data) else {
            bump(&DROPS.stun_unverified);
            return;
        };
        if msg.msg_type != stun::BINDING_REQUEST {
            return;
        }
        let t = msg.local_ufrag().and_then(|u| self.ufrags.read().ok().and_then(|m| m.get(u).and_then(Weak::upgrade)));
        let Some(t) = t else {
            bump(&DROPS.stun_unverified);
            return;
        };
        let verdict = {
            let (Ok(mut l), Ok(mut p)) = (t.ledger.lock(), t.pairs.lock()) else { return };
            ice::check(&mut l, &mut p, &msg, via, now_ms)
        };
        let ice::Verdict::Accepted { key, path_moved, promoted } = verdict else {
            bump(&DROPS.stun_unverified);
            return;
        };
        if let Ok(mut m) = self.by_via.write() {
            m.insert(via, Arc::downgrade(&t));
        }
        if let Some(old) = promoted {
            self.unregister_ufrag(&old.ufrag);
            let _ = self.signals.send(LinkSignal::Promoted(t.clone(), old.ufrag));
        }
        let alive_now = t.pairs.lock().map(|p| p.path_alive(now_ms)).unwrap_or(false);
        if msg.use_candidate() && alive_now && t.state() == PeerState::Suspect {
            let since = t.suspect_since.swap(0, Ordering::AcqRel);
            t.swap_state(PeerState::Alive);
            let _ = self.signals.send(LinkSignal::Alive(t.clone(), now_ms.saturating_sub(since)));
        }
        let _ = path_moved;
        let resp = stun::binding_success(&msg.txid, via.addr(), key.as_bytes());
        self.send_via(via, &resp);
    }

    fn feed_dtls(self: &Arc<Self>, t: &Arc<Transport>, data: &[u8]) {
        if !t.dtls_started.swap(true, Ordering::AcqRel) {
            let (tx, rx) = mpsc::channel::<Vec<u8>>(256);
            if let Ok(mut g) = t.dtls_tx.lock() {
                g.replace(tx);
            }
            let h = tokio::spawn(super::dtls::run(t.clone(), rx, self.cert.clone(), self.dc_events.clone()));
            t.add_task(h.abort_handle());
        }
        let tx = t.dtls_tx.lock().ok().and_then(|g| g.clone());
        if let Some(tx) = tx {
            let _ = tx.try_send(data.to_vec());
        }
    }

    pub fn keepalive_tcp(&self) {
        let conns: Vec<u64> = self.transports().iter().flat_map(|t| t.pairs.lock().map(|p| p.tcp_conns()).unwrap_or_default()).collect();
        if let Ok(m) = self.tcp.read() {
            for c in conns {
                if let Some(tx) = m.get(&c) {
                    let _ = tx.send(vec![0, 0]);
                }
            }
        }
    }

    fn tcp_closed(&self, conn: u64) {
        if let Ok(mut m) = self.tcp.write() {
            m.remove(&conn);
        }
        let mut owners = Vec::new();
        if let Ok(mut m) = self.by_via.write() {
            m.retain(|v, w| {
                if matches!(v, Via::Tcp(c, _) if *c == conn) {
                    if let Some(t) = w.upgrade() {
                        owners.push(t);
                    }
                    return false;
                }
                true
            });
        }
        for t in owners {
            if let Ok(mut p) = t.pairs.lock() {
                p.tcp_closed(conn);
            }
        }
    }

    fn identified(&self, conn: u64) -> bool {
        self.by_via.read().map(|m| m.keys().any(|v| matches!(v, Via::Tcp(c, _) if *c == conn))).unwrap_or(false)
    }
}

pub fn bind_udp(addr: SocketAddr) -> std::io::Result<UdpSocket> {
    let std_sock = std::net::UdpSocket::bind(addr)?;
    std_sock.set_nonblocking(true)?;
    let fd = std::os::fd::AsRawFd::as_raw_fd(&std_sock);
    let _ = nix::sys::socket::setsockopt(fd, nix::sys::socket::sockopt::RcvBuf, &SOCKET_BUFFER);
    let _ = nix::sys::socket::setsockopt(fd, nix::sys::socket::sockopt::SndBuf, &SOCKET_BUFFER);
    UdpSocket::from_std(std_sock)
}

pub async fn tcp_serve(io: Arc<Io>, listener: TcpListener) {
    loop {
        let Ok((sock, addr)) = listener.accept().await else { continue };
        let _ = sock.set_nodelay(true);
        let conn = io.next_id();
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        if let Ok(mut m) = io.tcp.write() {
            m.insert(conn, tx);
        }
        let (mut rd, mut wr) = sock.into_split();
        let writer = tokio::spawn(async move {
            while let Some(f) = rx.recv().await {
                if wr.write_all(&f).await.is_err() {
                    break;
                }
            }
        });
        let io2 = io.clone();
        tokio::spawn(async move {
            let mut d = Deframer::new();
            let mut buf = vec![0u8; 65_536];
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(IDENTIFY_MS);
            loop {
                let read = if io2.identified(conn) {
                    rd.read(&mut buf).await
                } else {
                    match tokio::time::timeout_at(deadline, rd.read(&mut buf)).await {
                        Ok(r) => r,
                        Err(_) => {
                            bump(&DROPS.tcp_unidentified);
                            break;
                        }
                    }
                };
                let n = match read {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                for f in d.push(&buf[..n]) {
                    if f.is_empty() {
                        continue;
                    }
                    let via = Via::Tcp(conn, addr);
                    if let Some((t, class)) = io2.on_datagram(&f, via, common::time::mono_ms()) {
                        crate::fwd::rx::dispatch(&t, class, &f);
                    }
                }
            }
            io2.tcp_closed(conn);
            writer.abort();
        });
    }
}

pub async fn tcp_keepalive(io: Arc<Io>) {
    let mut iv = tokio::time::interval(std::time::Duration::from_millis(KEEPALIVE_MS));
    loop {
        iv.tick().await;
        io.keepalive_tcp();
    }
}
