// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-FWD-MEDIA-WORKERS · 정:RULE-OPS-TRANSIT · 정:RULE-LINK-TRANSPORT · 정:RULE-LINK-PAIR-LIVENESS · 정:RULE-OPS-DROPS · 정:CAND-LINK-UDP-BUFFER · model: claude-opus-5-5
//! 수신 스레드 1 · 미디어 워커 N — 주소 색인은 수신 스레드 단독 · STUN 은 수신 스레드가 곧바로 · 워커 큐는 패킷 수 한도 · 잠듦 표시가 있을 때만 깨운다 · 워커가 끝나면 프로세스를 내린다.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use crossbeam_queue::ArrayQueue;
use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Token, Waker};

use crate::link::{PathKey, PeerState, Transport, stun, tcp};

pub const RECV_RING: usize = 4_096;
pub const WORKER_RING: usize = 1_024;
pub const CTL_RING: usize = 4_096;
pub const DEFAULT_QUEUE_PACKETS: usize = 6_000;
pub const MAX_DATAGRAM: usize = 2_048;

static START: OnceLock<Instant> = OnceLock::new();
static MEDIA: OnceLock<Arc<Media>> = OnceLock::new();

pub fn mono_us() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_micros() as u64
}

pub fn media() -> Option<&'static Arc<Media>> {
    MEDIA.get()
}

pub enum Item {
    In { tr: Arc<Transport>, data: Vec<u8>, at_us: u64 },
    Out(super::Egress),
    Ctl(super::Ctl),
}

impl Item {
    fn packets(&self) -> usize {
        match self {
            Item::In { .. } => 1,
            Item::Out(e) => e.targets.len().max(1),
            Item::Ctl(c) => c.packets(),
        }
    }
}

#[repr(align(128))]
#[derive(Default)]
pub struct Transit {
    pub buckets: [AtomicU64; 8],
    pub count: AtomicU64,
    pub sum_us: AtomicU64,
}

pub const TRANSIT_NAMES: [&str; 8] = ["le_1ms", "le_5ms", "le_10ms", "le_50ms", "le_100ms", "le_500ms", "le_1000ms", "gt_1000ms"];
const TRANSIT_EDGES: [u64; 7] = [1_000, 5_000, 10_000, 50_000, 100_000, 500_000, 1_000_000];

impl Transit {
    pub fn record(&self, us: u64) {
        let i = TRANSIT_EDGES.iter().position(|e| us <= *e).unwrap_or(7);
        self.buckets[i].fetch_add(1, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum_us.fetch_add(us, Ordering::Relaxed);
    }
}

#[repr(align(128))]
pub struct Line(pub AtomicUsize);

impl std::ops::Deref for Line {
    type Target = AtomicUsize;
    fn deref(&self) -> &AtomicUsize {
        &self.0
    }
}

pub struct Worker {
    pub id: usize,
    pub inbox: Vec<Arc<ArrayQueue<Item>>>,
    pub queued: Line,
    pub sleeping: AtomicBool,
    pub thread: OnceLock<std::thread::Thread>,
    pub sock: Arc<UdpSocket>,
    pub transports: Mutex<Vec<Weak<Transport>>>,
    pub transit: Transit,
    pub pubs: AtomicUsize,
    pub subs: AtomicUsize,
}

impl Worker {
    pub fn push(&self, from: usize, item: Item) -> bool {
        let m = match media() {
            Some(m) => m,
            None => return false,
        };
        let n = item.packets();
        let prev = self.queued.fetch_add(n, Ordering::AcqRel);
        if prev + n > m.queue_limit {
            self.queued.fetch_sub(n, Ordering::AcqRel);
            return false;
        }
        if self.inbox[from].push(item).is_err() {
            self.queued.fetch_sub(n, Ordering::AcqRel);
            return false;
        }
        std::sync::atomic::fence(Ordering::SeqCst);
        if self.sleeping.load(Ordering::SeqCst)
            && let Some(t) = self.thread.get()
        {
            t.unpark();
        }
        true
    }
}

pub enum RecvCmd {
    Register { ufrag: String, tr: Arc<Transport> },
    Unregister { tr: Arc<Transport> },
    DropUfrag { ufrag: String },
}

pub struct TcpOut {
    pub stream: Mutex<std::net::TcpStream>,
    pub pending: Mutex<Vec<u8>>,
    pub open: AtomicBool,
}

impl TcpOut {
    pub fn send(&self, payload: &[u8]) -> bool {
        let Some(f) = tcp::frame(payload) else { return false };
        let Ok(mut pend) = self.pending.lock() else { return false };
        if pend.len() + f.len() > tcp::SEND_BUF {
            return false;
        }
        pend.extend_from_slice(&f);
        let Ok(mut s) = self.stream.lock() else { return false };
        while !pend.is_empty() {
            match s.write(&pend) {
                Ok(0) => break,
                Ok(n) => {
                    pend.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.open.store(false, Ordering::Release);
                    return false;
                }
            }
        }
        true
    }
}

pub struct Media {
    pub workers: Vec<Arc<Worker>>,
    pub recv_cmds: ArrayQueue<RecvCmd>,
    pub waker: Waker,
    pub recv_sock: Arc<UdpSocket>,
    pub local: SocketAddr,
    pub tcp_port: Option<u16>,
    pub queue_limit: usize,
    pub tcp: Mutex<HashMap<u64, Arc<TcpOut>>>,
}

impl Media {
    pub fn pick_worker(&self, publish: bool) -> usize {
        let key = |w: &Worker| if publish { w.pubs.load(Ordering::Relaxed) } else { w.subs.load(Ordering::Relaxed) };
        let w = self.workers.iter().min_by_key(|w| key(w)).map(|w| w.id).unwrap_or(0);
        if let Some(x) = self.workers.get(w) {
            if publish { x.pubs.fetch_add(1, Ordering::Relaxed) } else { x.subs.fetch_add(1, Ordering::Relaxed) };
        }
        w
    }

    pub fn release_worker(&self, w: usize, publish: bool) {
        if let Some(x) = self.workers.get(w) {
            let c = if publish { &x.pubs } else { &x.subs };
            let _ = c.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| Some(v.saturating_sub(1)));
        }
    }

    pub fn register(&self, ufrag: &str, tr: &Arc<Transport>) {
        let _ = self.recv_cmds.push(RecvCmd::Register { ufrag: ufrag.to_string(), tr: tr.clone() });
        let _ = self.waker.wake();
    }

    pub fn unregister(&self, tr: &Arc<Transport>) {
        let _ = self.recv_cmds.push(RecvCmd::Unregister { tr: tr.clone() });
        let _ = self.waker.wake();
    }

    pub fn drop_ufrag(&self, ufrag: &str) {
        let _ = self.recv_cmds.push(RecvCmd::DropUfrag { ufrag: ufrag.to_string() });
        let _ = self.waker.wake();
    }

    pub fn tcp_open(&self, id: u64) -> bool {
        self.tcp.lock().ok().and_then(|g| g.get(&id).map(|t| t.open.load(Ordering::Acquire))).unwrap_or(false)
    }

    pub fn send_on(&self, sock: &UdpSocket, path: PathKey, data: &[u8]) -> bool {
        match path {
            PathKey::Udp(a) => {
                for _ in 0..3 {
                    match sock.send_to(data, a) {
                        Ok(_) => return true,
                        Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::yield_now(),
                        Err(_) => return false,
                    }
                }
                false
            }
            PathKey::Tcp(id) => {
                let t = self.tcp.lock().ok().and_then(|g| g.get(&id).cloned());
                t.map(|t| t.send(data)).unwrap_or(false)
            }
        }
    }
}

pub fn control_send(tr: &Transport, data: &[u8]) {
    let Some(m) = media() else { return };
    let path = tr.control_path(common::now_ms(), |id| m.tcp_open(id));
    if let Some(p) = path
        && !m.send_on(&m.recv_sock, p, data)
    {
        crate::ops::drop_count("send_fail");
    }
}

fn bind_udp(addr: SocketAddr, reuse: bool) -> std::io::Result<UdpSocket> {
    let s = socket2::Socket::new(socket2::Domain::for_address(addr), socket2::Type::DGRAM, Some(socket2::Protocol::UDP))?;
    if reuse {
        s.set_reuse_address(true)?;
        s.set_reuse_port(true)?;
    }
    let _ = s.set_send_buffer_size(crate::link::SOCKET_BUF);
    let _ = s.set_recv_buffer_size(crate::link::SOCKET_BUF);
    s.bind(&addr.into())?;
    s.set_nonblocking(true)?;
    Ok(s.into())
}

pub struct StartCfg {
    pub bind: SocketAddr,
    pub advertised: SocketAddr,
    pub tcp_port: Option<u16>,
    pub workers: usize,
    pub queue_limit: usize,
}

pub fn start(c: StartCfg) -> Result<Arc<Media>, String> {
    let _ = START.get_or_init(Instant::now);
    let n = c.workers.max(1);
    let per_worker_sock = cfg!(target_os = "macos");
    let mut wsocks = Vec::new();
    if per_worker_sock {
        for _ in 0..n {
            wsocks.push(Arc::new(bind_udp(c.bind, true).map_err(|e| format!("udp {}: {e}", c.bind))?));
        }
    }
    let recv = Arc::new(bind_udp(c.bind, per_worker_sock).map_err(|e| format!("udp {}: {e}", c.bind))?);
    let poll = Poll::new().map_err(|e| e.to_string())?;
    let waker = Waker::new(poll.registry(), Token(1)).map_err(|e| e.to_string())?;
    let workers: Vec<Arc<Worker>> = (0..n)
        .map(|i| {
            Arc::new(Worker {
                id: i,
                inbox: (0..n + 2).map(|k| Arc::new(ArrayQueue::new(if k == 0 { RECV_RING } else if k == n + 1 { CTL_RING } else { WORKER_RING }))).collect(),
                queued: Line(AtomicUsize::new(0)),
                sleeping: AtomicBool::new(false),
                thread: OnceLock::new(),
                sock: if per_worker_sock { wsocks[i].clone() } else { recv.clone() },
                transports: Mutex::new(Vec::new()),
                transit: Transit::default(),
                pubs: AtomicUsize::new(0),
                subs: AtomicUsize::new(0),
            })
        })
        .collect();
    let m = Arc::new(Media {
        workers,
        recv_cmds: ArrayQueue::new(CTL_RING),
        waker,
        recv_sock: recv.clone(),
        local: c.advertised,
        tcp_port: c.tcp_port,
        queue_limit: c.queue_limit.max(1),
        tcp: Mutex::new(HashMap::new()),
    });
    MEDIA.set(m.clone()).map_err(|_| "media already started".to_string())?;
    let tcp_listener = match c.tcp_port {
        Some(p) => {
            let a = SocketAddr::new(c.bind.ip(), p);
            Some(TcpListener::bind(a).map_err(|e| format!("tcp {a}: {e}"))?)
        }
        None => None,
    };
    for w in m.workers.clone() {
        std::thread::Builder::new()
            .name(format!("media-{}", w.id))
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| super::worker_loop(&w)));
                eprintln!("oxsfud: media worker ended {r:?}");
                std::process::exit(70);
            })
            .map_err(|e| e.to_string())?;
    }
    let m2 = m.clone();
    std::thread::Builder::new()
        .name("media-recv".into())
        .spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || recv_loop(m2, poll, recv, tcp_listener)));
            eprintln!("oxsfud: receive thread ended {r:?}");
            std::process::exit(70);
        })
        .map_err(|e| e.to_string())?;
    Ok(m)
}

struct TcpIn {
    stream: TcpStream,
    de: tcp::Deframer,
    id: u64,
    opened: Instant,
    tr: Option<Arc<Transport>>,
}

struct RecvState {
    by_ufrag: HashMap<String, Arc<Transport>>,
    by_addr: HashMap<PathKey, Arc<Transport>>,
}

const UDP: Token = Token(0);
const WAKE: Token = Token(1);
const LISTEN: Token = Token(2);

fn recv_loop(m: Arc<Media>, mut poll: Poll, recv: Arc<UdpSocket>, listener: Option<TcpListener>) {
    let mut events = Events::with_capacity(1024);
    let mut st = RecvState { by_ufrag: HashMap::new(), by_addr: HashMap::new() };
    let mut msock = match recv.try_clone() {
        Ok(s) => mio::net::UdpSocket::from_std(s),
        Err(_) => return,
    };
    if poll.registry().register(&mut msock, UDP, Interest::READABLE).is_err() {
        return;
    }
    let mut listener = listener;
    if let Some(l) = listener.as_mut() {
        let _ = poll.registry().register(l, LISTEN, Interest::READABLE);
    }
    let mut conns: HashMap<Token, TcpIn> = HashMap::new();
    let mut next_tok = 16usize;
    let mut buf = vec![0u8; 65_536];
    loop {
        let _ = poll.poll(&mut events, Some(Duration::from_millis(100)));
        for ev in events.iter() {
            match ev.token() {
                UDP => {
                    while let Ok((n, from)) = msock.recv_from(&mut buf) {
                        let at = mono_us();
                        datagram(&m, &mut st, &buf[..n], PathKey::Udp(from), at, None);
                    }
                }
                WAKE => {}
                LISTEN => {
                    if let Some(l) = listener.as_ref() {
                        while let Ok((s, _)) = l.accept() {
                            let tok = Token(next_tok);
                            next_tok += 1;
                            let id = tok.0 as u64;
                            let std_stream = match tcp_split(s) {
                                Some(x) => x,
                                None => continue,
                            };
                            let (mut rs, ws) = std_stream;
                            if poll.registry().register(&mut rs, tok, Interest::READABLE).is_err() {
                                continue;
                            }
                            if let Ok(mut g) = m.tcp.lock() {
                                g.insert(id, Arc::new(TcpOut { stream: Mutex::new(ws), pending: Mutex::new(Vec::new()), open: AtomicBool::new(true) }));
                            }
                            conns.insert(tok, TcpIn { stream: rs, de: tcp::Deframer::default(), id, opened: Instant::now(), tr: None });
                        }
                    }
                }
                tok => {
                    let mut closed = false;
                    if let Some(c) = conns.get_mut(&tok) {
                        loop {
                            match c.stream.read(&mut buf) {
                                Ok(0) => {
                                    closed = true;
                                    break;
                                }
                                Ok(n) => c.de.push(&buf[..n]),
                                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                                Err(_) => {
                                    closed = true;
                                    break;
                                }
                            }
                        }
                        let id = c.id;
                        while let Some(f) = c.de.next_frame() {
                            let at = mono_us();
                            if f.len() > MAX_DATAGRAM {
                                crate::ops::drop_count("oversize");
                                continue;
                            }
                            if let Some(tr) = datagram(&m, &mut st, &f, PathKey::Tcp(id), at, c.tr.as_ref()) {
                                c.tr = Some(tr);
                            }
                        }
                    }
                    if closed && let Some(mut c) = conns.remove(&tok) {
                        let _ = poll.registry().deregister(&mut c.stream);
                        tcp_closed(&m, &mut st, c.id);
                    }
                }
            }
        }
        while let Some(cmd) = m.recv_cmds.pop() {
            match cmd {
                RecvCmd::Register { ufrag, tr } => {
                    st.by_ufrag.insert(ufrag, tr);
                }
                RecvCmd::DropUfrag { ufrag } => {
                    st.by_ufrag.remove(&ufrag);
                }
                RecvCmd::Unregister { tr } => {
                    st.by_ufrag.retain(|_, t| !Arc::ptr_eq(t, &tr));
                    st.by_addr.retain(|_, t| !Arc::ptr_eq(t, &tr));
                }
            }
        }
        let stale: Vec<Token> = conns
            .iter()
            .filter(|(_, c)| c.tr.is_none() && c.opened.elapsed() > Duration::from_millis(tcp::IDENTIFY_MS))
            .map(|(t, _)| *t)
            .collect();
        for t in stale {
            if let Some(mut c) = conns.remove(&t) {
                let _ = poll.registry().deregister(&mut c.stream);
                crate::ops::drop_count("tcp_unidentified");
                tcp_closed(&m, &mut st, c.id);
            }
        }
    }
}

fn tcp_split(s: TcpStream) -> Option<(TcpStream, std::net::TcpStream)> {
    let _ = s.set_nodelay(true);
    let sock = socket2::SockRef::from(&s);
    let dup = sock.try_clone().ok()?;
    let ws: std::net::TcpStream = dup.into();
    ws.set_nonblocking(true).ok()?;
    Some((s, ws))
}

fn tcp_closed(m: &Media, st: &mut RecvState, id: u64) {
    if let Ok(mut g) = m.tcp.lock()
        && let Some(t) = g.remove(&id)
    {
        t.open.store(false, Ordering::Release);
    }
    st.by_addr.remove(&PathKey::Tcp(id));
}

fn reply(m: &Media, key: PathKey, data: &[u8]) {
    if !m.send_on(&m.recv_sock, key, data) {
        crate::ops::drop_count("send_fail");
    }
}

fn datagram(m: &Media, st: &mut RecvState, data: &[u8], key: PathKey, at_us: u64, known: Option<&Arc<Transport>>) -> Option<Arc<Transport>> {
    let &b0 = data.first()?;
    match b0 {
        0..=3 => {
            let lookup = |u: &str| st.by_ufrag.get(u).cloned();
            let Ok(parsed) = stun::parse(data) else {
                crate::ops::drop_count("stun_unverified");
                return None;
            };
            let Some(tr) = lookup(&parsed.local_ufrag) else {
                crate::ops::drop_count("stun_unverified");
                return None;
            };
            let pwd = tr.creds.lock().ok().and_then(|c| c.pwd(&parsed.local_ufrag).map(|p| p.to_string()));
            let Some(pwd) = pwd else {
                crate::ops::drop_count("stun_unverified");
                return None;
            };
            if !stun::verify(data, &parsed, &pwd) {
                crate::ops::drop_count("stun_unverified");
                return None;
            }
            let now = common::now_ms();
            tr.touch_pair(key, now);
            st.by_addr.insert(key, tr.clone());
            let promoted = tr.creds.lock().ok().map(|mut c| c.checked(&parsed.local_ufrag));
            if let Some(crate::link::creds::Checked::Promoted { retired }) = promoted {
                st.by_ufrag.remove(&retired);
                crate::ops::ufrag_promoted(&tr, &parsed.local_ufrag);
            }
            if parsed.use_candidate {
                let changed = tr.path.load_full().map(|p| *p != key).unwrap_or(true);
                tr.path.store(Some(Arc::new(key)));
                if tr.set_state(PeerState::Alive, now) == Some(PeerState::Suspect) || changed {
                    crate::ops::peer_state_changed(&tr, PeerState::Alive, changed);
                }
            }
            let from = match key {
                PathKey::Udp(a) => a,
                PathKey::Tcp(_) => m.local,
            };
            reply(m, key, &stun::success(&parsed.txid, from, &pwd));
            Some(tr)
        }
        20..=63 => {
            let tr = st.by_addr.get(&key).cloned().or_else(|| known.cloned());
            match tr {
                Some(t) => {
                    let _ = t.dtls_in.send(data.to_vec());
                    Some(t)
                }
                None => {
                    crate::ops::drop_count("rtp_unknown_path");
                    None
                }
            }
        }
        128..=191 => {
            let Some(tr) = st.by_addr.get(&key).cloned() else {
                crate::ops::drop_count("rtp_unknown_path");
                return None;
            };
            if data.len() > MAX_DATAGRAM {
                crate::ops::drop_count("oversize");
                return Some(tr);
            }
            if let Some(w) = m.workers.get(tr.worker)
                && !w.push(0, Item::In { tr: tr.clone(), data: data.to_vec(), at_us })
            {
                crate::ops::drop_count("ingress_queue_full");
            }
            Some(tr)
        }
        _ => {
            crate::ops::drop_count("demux_unknown");
            None
        }
    }
}
