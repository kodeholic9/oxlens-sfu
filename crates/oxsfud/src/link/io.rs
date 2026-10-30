// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-TRANSPORT · 정:RULE-LINK-PAIR-LIVENESS · 정:IMPL-LINK-PEER-STATE · 정:IMPL-LINK-ICE-RESTART · 정:RULE-OPS-DROPS · 정:CAND-LINK-UDP-BUFFER · 정:RULE-FWD-MEDIA-WORKERS · 정:RULE-OPS-TRANSIT · model: claude-opus-5-5
//! 전송 등록부와 소켓 — UDP 포트 하나 · ICE-TCP passive(RFC 4571) · demux(첫 바이트) · STUN 검증 차례(조회 → MI 검증 → 생존 → 지명 → 응답) · DTLS 넘김. 데이터그램마다 받은 시각(μs)을 워커로 넘긴다(체류 시간의 시작). 수신 스레드가 UDP · TCP 리슨 · TCP 연결을 함께 기다리고(`mio`) 주소 색인(ufrag · 경로 주소)을 혼자 쥔다 — 제어 쪽 등록 · 회수 · ufrag 교체는 명령 채널로 넘긴다. TCP 송신은 연결마다 송신 버퍼(64KB)를 두고 막히면 수신 스레드가 쓰기 가능 사건에서 마저 비운다.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak, mpsc as smpsc};
use std::time::Duration;

use mio::net::{TcpListener as MioListener, TcpStream as MioStream, UdpSocket as MioUdp};
use mio::{Events, Interest, Poll, Token, Waker};
use tokio::sync::mpsc;

use super::ice::{self, Via};
use super::stun;
use super::tcp::{Deframer, IDENTIFY_MS, frame};
use super::transport::{PeerState, Transport};
use crate::ops::drops::{DROPS, bump};

pub const SOCKET_BUFFER: usize = 4 * 1024 * 1024;
pub const TCP_SEND_BYTES: usize = 65_536;
const T_UDP: Token = Token(0);
const T_LISTEN: Token = Token(1);
const T_WAKE: Token = Token(2);
const T_CONN0: usize = 16;
const POLL_MS: u64 = 100;
const SWEEP_MS: u64 = 1_000;

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

pub enum Cmd {
    Register { ufrags: Vec<String>, t: Weak<Transport> },
    Ufrag { ufrag: String, t: Weak<Transport> },
    Unufrag(String),
    Unregister { id: u64, ufrags: Vec<String> },
}

struct TcpOut {
    stream: Arc<MioStream>,
    pending: Mutex<Vec<u8>>,
}

pub struct Io {
    pub udp: Arc<UdpSocket>,
    senders: Vec<UdpSocket>,
    pub local: SocketAddr,
    pub tcp_port: Option<u16>,
    tcp: RwLock<HashMap<u64, Arc<TcpOut>>>,
    all: RwLock<HashMap<u64, Weak<Transport>>>,
    next_id: AtomicU64,
    ctl: OnceLock<(smpsc::Sender<Cmd>, Arc<Waker>)>,
    pub cert: dtls::crypto::Certificate,
    pub fingerprint: String,
    pub dc_events: mpsc::UnboundedSender<DcEvent>,
    pub signals: mpsc::UnboundedSender<LinkSignal>,
}

impl Io {
    pub fn new(udp: Udp, tcp_port: Option<u16>, cert: dtls::crypto::Certificate, dc_events: mpsc::UnboundedSender<DcEvent>, signals: mpsc::UnboundedSender<LinkSignal>) -> Io {
        let local = udp.main.local_addr().unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        let fingerprint = cert.certificate.first().map(|c| super::cert::fingerprint(c.as_ref())).unwrap_or_default();
        Io {
            udp: Arc::new(udp.main),
            senders: udp.senders,
            local,
            tcp_port,
            tcp: RwLock::new(HashMap::new()),
            all: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            ctl: OnceLock::new(),
            cert,
            fingerprint,
            dc_events,
            signals,
        }
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn cmd(&self, c: Cmd) {
        if let Some((tx, w)) = self.ctl.get()
            && tx.send(c).is_ok()
        {
            let _ = w.wake();
        }
    }

    pub fn register(&self, t: &Arc<Transport>) {
        crate::fwd::work::adopt(t);
        let ufrags = t.ledger.lock().map(|l| l.ufrags()).unwrap_or_default();
        self.cmd(Cmd::Register { ufrags, t: Arc::downgrade(t) });
        if let Ok(mut a) = self.all.write() {
            a.insert(t.id, Arc::downgrade(t));
        }
    }

    pub fn register_ufrag(&self, ufrag: &str, t: &Arc<Transport>) {
        self.cmd(Cmd::Ufrag { ufrag: ufrag.to_string(), t: Arc::downgrade(t) });
    }

    pub fn unregister_ufrag(&self, ufrag: &str) {
        self.cmd(Cmd::Unufrag(ufrag.to_string()));
    }

    pub fn unregister(&self, t: &Transport) {
        let ufrags = t.ledger.lock().map(|l| l.ufrags()).unwrap_or_default();
        self.cmd(Cmd::Unregister { id: t.id, ufrags });
        if let Ok(mut a) = self.all.write()
            && a.remove(&t.id).is_some()
        {
            crate::fwd::work::forsake(t);
        }
    }

    pub fn transports(&self) -> Vec<Arc<Transport>> {
        self.all.read().map(|a| a.values().filter_map(Weak::upgrade).collect()).unwrap_or_default()
    }

    pub fn send_via(&self, via: Via, bytes: &[u8]) -> bool {
        match via {
            Via::Udp(a) => {
                let sock = crate::fwd::work::current().and_then(|i| self.senders.get(i)).unwrap_or(&self.udp);
                for _ in 0..3 {
                    match sock.send_to(bytes, a) {
                        Ok(_) => return true,
                        Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::yield_now(),
                        Err(_) => break,
                    }
                }
                Transport::note_send_fail();
                false
            }
            Via::Tcp(c, _) => match frame(bytes) {
                Some(f) => self.tcp_send(c, &f),
                None => false,
            },
        }
    }

    fn tcp_send(&self, conn: u64, f: &[u8]) -> bool {
        let Some(out) = self.tcp.read().ok().and_then(|m| m.get(&conn).cloned()) else { return false };
        let Ok(mut pending) = out.pending.lock() else { return false };
        if !pending.is_empty() {
            if pending.len() + f.len() > TCP_SEND_BYTES {
                Transport::note_send_fail();
                return false;
            }
            pending.extend_from_slice(f);
            return true;
        }
        match (&*out.stream).write(f) {
            Ok(n) if n == f.len() => true,
            Ok(n) => {
                pending.extend_from_slice(&f[n..]);
                true
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                pending.extend_from_slice(f);
                true
            }
            Err(_) => false,
        }
    }

    pub fn keepalive(&self, conn: u64) {
        let _ = self.tcp_send(conn, &[0, 0]);
    }

    fn flush(&self, conn: u64) {
        let Some(out) = self.tcp.read().ok().and_then(|m| m.get(&conn).cloned()) else { return };
        let Ok(mut pending) = out.pending.lock() else { return };
        while !pending.is_empty() {
            match (&*out.stream).write(&pending) {
                Ok(0) => break,
                Ok(n) => {
                    pending.drain(..n);
                }
                Err(_) => break,
            }
        }
    }

    fn feed_dtls(&self, t: &Arc<Transport>, data: &[u8]) {
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
}

struct Conn {
    stream: Arc<MioStream>,
    addr: SocketAddr,
    de: Deframer,
    born_ms: u64,
    identified: bool,
}

struct Rx {
    io: Arc<Io>,
    ufrags: HashMap<String, Weak<Transport>>,
    by_via: HashMap<Via, Weak<Transport>>,
    conns: HashMap<u64, Conn>,
}

impl Rx {
    fn apply(&mut self, c: Cmd) {
        match c {
            Cmd::Register { ufrags, t } => {
                for u in ufrags {
                    self.ufrags.insert(u, t.clone());
                }
            }
            Cmd::Ufrag { ufrag, t } => {
                self.ufrags.insert(ufrag, t);
            }
            Cmd::Unufrag(u) => {
                self.ufrags.remove(&u);
            }
            Cmd::Unregister { id, ufrags } => {
                for u in ufrags {
                    self.ufrags.remove(&u);
                }
                let mut conns = Vec::new();
                self.by_via.retain(|v, w| {
                    let mine = w.upgrade().is_none_or(|x| x.id == id);
                    if mine && let Via::Tcp(c, _) = v {
                        conns.push(*c);
                    }
                    !mine
                });
                for c in conns {
                    self.drop_conn(c);
                }
            }
        }
    }

    fn lookup(&self, via: &Via) -> Option<Arc<Transport>> {
        self.by_via.get(via).and_then(Weak::upgrade)
    }

    fn on_datagram(&mut self, data: &[u8], via: Via, at_us: u64) {
        let now_ms = at_us / 1_000;
        match classify(data) {
            Class::Stun => self.on_stun(data, via, now_ms),
            Class::Dtls => match self.lookup(&via) {
                Some(t) => self.io.feed_dtls(&t, data),
                None => bump(&DROPS.rtp_unknown_path),
            },
            c @ (Class::Rtp | Class::Rtcp) => match self.lookup(&via) {
                Some(t) => crate::fwd::work::submit_in(&t, c, data.to_vec(), at_us),
                None => bump(&DROPS.rtp_unknown_path),
            },
            Class::Unknown => bump(&DROPS.demux_unknown),
        }
    }

    fn on_stun(&mut self, data: &[u8], via: Via, now_ms: u64) {
        let Some(msg) = stun::parse(data) else {
            bump(&DROPS.stun_unverified);
            return;
        };
        if msg.msg_type != stun::BINDING_REQUEST {
            return;
        }
        let Some(t) = msg.local_ufrag().and_then(|u| self.ufrags.get(u).and_then(Weak::upgrade)) else {
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
        self.by_via.insert(via, Arc::downgrade(&t));
        if let Via::Tcp(c, _) = via
            && let Some(conn) = self.conns.get_mut(&c)
        {
            conn.identified = true;
        }
        if let Some(old) = promoted {
            self.ufrags.remove(&old.ufrag);
            let _ = self.io.signals.send(LinkSignal::Promoted(t.clone(), old.ufrag));
        }
        let alive_now = t.pairs.lock().map(|p| p.path_alive(now_ms)).unwrap_or(false);
        if msg.use_candidate() && alive_now && t.state() == PeerState::Suspect {
            let since = t.suspect_since.swap(0, Ordering::AcqRel);
            t.alive_since.store(now_ms, Ordering::Release);
            t.swap_state(PeerState::Alive);
            let _ = self.io.signals.send(LinkSignal::Alive(t.clone(), now_ms.saturating_sub(since)));
        }
        let _ = path_moved;
        let resp = stun::binding_success(&msg.txid, via.addr(), key.as_bytes());
        self.io.send_via(via, &resp);
    }

    fn drop_conn(&mut self, conn: u64) {
        self.conns.remove(&conn);
        if let Ok(mut m) = self.io.tcp.write() {
            m.remove(&conn);
        }
    }

    fn tcp_closed(&mut self, conn: u64) {
        self.drop_conn(conn);
        let mut owners = Vec::new();
        self.by_via.retain(|v, w| {
            if matches!(v, Via::Tcp(c, _) if *c == conn) {
                if let Some(t) = w.upgrade() {
                    owners.push(t);
                }
                return false;
            }
            true
        });
        for t in owners {
            if let Ok(mut p) = t.pairs.lock() {
                p.tcp_closed(conn);
            }
        }
    }

    fn read_conn(&mut self, conn: u64, buf: &mut [u8]) {
        let mut frames = Vec::new();
        let mut closed = false;
        let addr = {
            let Some(c) = self.conns.get_mut(&conn) else { return };
            loop {
                match (&*c.stream).read(buf) {
                    Ok(0) => {
                        closed = true;
                        break;
                    }
                    Ok(n) => frames.extend(c.de.push(&buf[..n])),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => {
                        closed = true;
                        break;
                    }
                }
            }
            c.addr
        };
        let at_us = common::time::mono_us();
        for f in frames {
            if f.is_empty() {
                continue;
            }
            if f.len() > crate::fwd::work::SLOT_BYTES {
                bump(&DROPS.oversize);
                continue;
            }
            self.on_datagram(&f, Via::Tcp(conn, addr), at_us);
        }
        if closed {
            self.tcp_closed(conn);
        }
    }

    fn sweep(&mut self, now_ms: u64) {
        let stale: Vec<u64> = self.conns.iter().filter(|(_, c)| !c.identified && now_ms.saturating_sub(c.born_ms) >= IDENTIFY_MS).map(|(k, _)| *k).collect();
        for c in stale {
            bump(&DROPS.tcp_unidentified);
            self.tcp_closed(c);
        }
    }
}

pub struct Reader {
    io: Arc<Io>,
    poll: Poll,
    cmds: smpsc::Receiver<Cmd>,
    udp: MioUdp,
    listener: Option<MioListener>,
}

pub fn reader(io: &Arc<Io>, listener: Option<std::net::TcpListener>) -> std::io::Result<Reader> {
    let poll = Poll::new()?;
    let waker = Arc::new(Waker::new(poll.registry(), T_WAKE)?);
    let (tx, cmds) = smpsc::channel();
    let _ = io.ctl.set((tx, waker));
    let sock = io.udp.try_clone()?;
    sock.set_nonblocking(true)?;
    let mut udp = MioUdp::from_std(sock);
    poll.registry().register(&mut udp, T_UDP, Interest::READABLE)?;
    let listener = match listener {
        Some(l) => {
            l.set_nonblocking(true)?;
            let mut l = MioListener::from_std(l);
            poll.registry().register(&mut l, T_LISTEN, Interest::READABLE)?;
            Some(l)
        }
        None => None,
    };
    Ok(Reader { io: io.clone(), poll, cmds, udp, listener })
}

impl Reader {
    fn accept(&mut self, rx: &mut Rx, now_ms: u64) {
        let Some(l) = self.listener.as_ref() else { return };
        loop {
            match l.accept() {
                Ok((mut s, addr)) => {
                    let _ = s.set_nodelay(true);
                    let id = self.io.next_id();
                    let Ok(tok) = usize::try_from(id).map(|i| T_CONN0 + i) else { continue };
                    if self.poll.registry().register(&mut s, Token(tok), Interest::READABLE | Interest::WRITABLE).is_err() {
                        continue;
                    }
                    let s = Arc::new(s);
                    if let Ok(mut m) = self.io.tcp.write() {
                        m.insert(id, Arc::new(TcpOut { stream: s.clone(), pending: Mutex::new(Vec::new()) }));
                    }
                    rx.conns.insert(id, Conn { stream: s, addr, de: Deframer::new(), born_ms: now_ms, identified: false });
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }

    pub fn run(mut self) {
        let mut rx = Rx { io: self.io.clone(), ufrags: HashMap::new(), by_via: HashMap::new(), conns: HashMap::new() };
        let mut events = Events::with_capacity(1_024);
        let mut buf = vec![0u8; 65_536];
        let mut last_sweep = common::time::mono_ms();
        loop {
            let _ = self.poll.poll(&mut events, Some(Duration::from_millis(POLL_MS)));
            while let Ok(c) = self.cmds.try_recv() {
                rx.apply(c);
            }
            let now = common::time::mono_ms();
            for ev in events.iter() {
                match ev.token() {
                    T_UDP => loop {
                        match self.udp.recv_from(&mut buf) {
                            Ok((n, from)) => rx.on_datagram(&buf[..n], Via::Udp(from), common::time::mono_us()),
                            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                            Err(_) => break,
                        }
                    },
                    T_LISTEN => self.accept(&mut rx, now),
                    T_WAKE => {}
                    Token(k) => {
                        let Ok(conn) = u64::try_from(k.saturating_sub(T_CONN0)) else { continue };
                        if ev.is_writable() {
                            self.io.flush(conn);
                        }
                        if ev.is_readable() || ev.is_read_closed() || ev.is_error() {
                            rx.read_conn(conn, &mut buf);
                        }
                    }
                }
            }
            if now.saturating_sub(last_sweep) >= SWEEP_MS {
                last_sweep = now;
                rx.sweep(now);
            }
        }
    }
}

pub struct Udp {
    pub main: UdpSocket,
    pub senders: Vec<UdpSocket>,
}

fn udp_socket(addr: SocketAddr, reuse_port: bool) -> std::io::Result<UdpSocket> {
    let s = socket2::Socket::new(socket2::Domain::for_address(addr), socket2::Type::DGRAM, Some(socket2::Protocol::UDP))?;
    if reuse_port {
        s.set_reuse_port(true)?;
    }
    let _ = s.set_recv_buffer_size(SOCKET_BUFFER);
    let _ = s.set_send_buffer_size(SOCKET_BUFFER);
    s.bind(&addr.into())?;
    Ok(s.into())
}

pub fn bind_udp(addr: SocketAddr, senders: usize) -> std::io::Result<Udp> {
    if !cfg!(target_os = "macos") || senders == 0 {
        return Ok(Udp { main: udp_socket(addr, false)?, senders: Vec::new() });
    }
    let first = udp_socket(addr, true)?;
    let bound = first.local_addr()?;
    let mut out = vec![first];
    for _ in 1..senders {
        out.push(udp_socket(bound, true)?);
    }
    Ok(Udp { main: udp_socket(bound, true)?, senders: out })
}
