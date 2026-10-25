// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-LINK-TRANSPORT 정:IMPL-LINK-PEER-STATE 정:RULE-LINK-PAIR-LIVENESS 정:IMPL-LINK-ICE-RESTART 정:RULE-CONC-SIX 정:RULE-CONC-STATE-VOCAB 정:CAND-LINK-UDP-BUFFER 연:LINK.S.suspect 연:LINK.S.checked_new 연:RULE-LINK-EVENT-CONTRACT · model: claude-opus-5-5
//! 전송 객체 — 신원은 이것이다(ufrag · 주소는 찾는 키) · 쌍별 생존 · 지명된 경로 · PeerState(Alive · Suspect) · SRTP 문맥 · DC 통로.

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use arc_swap::ArcSwapOption;
use dashmap::DashMap;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::ice::{Checked, Cred, CredSet};

pub const UDP_PAIR_TTL_MS: u64 = 30_000;
pub const TCP_IDENTIFY_MS: u64 = 10_000;
pub const TCP_KEEPALIVE_MS: u64 = 20_000;
pub const SOCKET_BUF: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Path {
    Udp(SocketAddr),
    Tcp(u64),
}

impl Path {
    pub fn is_tcp(&self) -> bool {
        matches!(self, Path::Tcp(_))
    }
}

impl std::fmt::Display for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Path::Udp(a) => write!(f, "udp:{a}"),
            Path::Tcp(i) => write!(f, "tcp:{i}"),
        }
    }
}

pub struct Io {
    pub udp: Arc<UdpSocket>,
    pub tcp: DashMap<u64, mpsc::Sender<Vec<u8>>>,
    pub send_fail: AtomicU64,
    pub tcp_drop: AtomicU64,
}

impl Io {
    pub fn new(udp: Arc<UdpSocket>) -> Io {
        Io { udp, tcp: DashMap::new(), send_fail: AtomicU64::new(0), tcp_drop: AtomicU64::new(0) }
    }

    pub fn send(&self, path: &Path, b: &[u8]) -> bool {
        match path {
            Path::Udp(a) => {
                for _ in 0..3 {
                    match self.udp.try_send_to(b, *a) {
                        Ok(_) => return true,
                        Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::yield_now(),
                        Err(_) => break,
                    }
                }
                self.send_fail.fetch_add(1, Ordering::Relaxed);
                false
            }
            Path::Tcp(id) => {
                let Some(framed) = super::tcp::frame(b) else { return false };
                match self.tcp.get(id) {
                    Some(tx) => {
                        if tx.try_send(framed).is_ok() {
                            true
                        } else {
                            self.tcp_drop.fetch_add(1, Ordering::Relaxed);
                            false
                        }
                    }
                    None => false,
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Alive,
    Suspect,
}

impl PeerState {
    pub fn name(self) -> &'static str {
        match self {
            PeerState::Alive => "Alive",
            PeerState::Suspect => "Suspect",
        }
    }
}

#[derive(Debug, Clone)]
struct Pair {
    path: Path,
    last_ok_ms: u64,
    closed: bool,
}

impl Pair {
    fn live(&self, now: u64) -> bool {
        match self.path {
            Path::Udp(_) => now.saturating_sub(self.last_ok_ms) <= UDP_PAIR_TTL_MS,
            Path::Tcp(_) => !self.closed,
        }
    }
}

#[derive(Debug)]
struct Link {
    pairs: Vec<Pair>,
    nominated: Option<Path>,
    state: PeerState,
    since_ms: u64,
}

pub struct CheckOutcome {
    pub promoted: Option<String>,
    pub path_changed: bool,
    pub alive_again: Option<u64>,
}

pub struct SuspectEvent {
    pub ufrag: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Both,
    Publish,
    Subscribe,
}

pub struct Transport {
    pub id: u64,
    pub role: Role,
    pub io: Arc<Io>,
    creds: std::sync::Mutex<CredSet>,
    link: std::sync::Mutex<Link>,
    path: ArcSwapOption<Path>,
    pub suspect_since: AtomicU64,
    pub srtp_rx: std::sync::Mutex<Option<webrtc_srtp::context::Context>>,
    pub srtp_tx: std::sync::Mutex<Option<webrtc_srtp::context::Context>>,
    pub srtp_ready: AtomicBool,
    pub dtls_in: std::sync::Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    pub dtls_started: AtomicBool,
    pub dc_open: AtomicBool,
    pub dc_out: mpsc::Sender<Vec<u8>>,
    pub dc_out_rx: std::sync::Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
    pub dc_in_tx: mpsc::Sender<Vec<u8>>,
    pub dc_in: std::sync::Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
    pub cancel: CancellationToken,
    pub rx_bytes: AtomicU64,
    pub tx_bytes: AtomicU64,
}

pub const DC_QUEUE: usize = 1024;

impl Transport {
    pub fn new(id: u64, role: Role, io: Arc<Io>, now: u64) -> Transport {
        let (dc_out, dc_out_rx) = mpsc::channel(DC_QUEUE);
        let (dc_in_tx, dc_in) = mpsc::channel(DC_QUEUE);
        Transport {
            id,
            role,
            io,
            creds: std::sync::Mutex::new(CredSet::new()),
            link: std::sync::Mutex::new(Link { pairs: Vec::new(), nominated: None, state: PeerState::Alive, since_ms: now }),
            path: ArcSwapOption::empty(),
            suspect_since: AtomicU64::new(0),
            srtp_rx: std::sync::Mutex::new(None),
            srtp_tx: std::sync::Mutex::new(None),
            srtp_ready: AtomicBool::new(false),
            dtls_in: std::sync::Mutex::new(None),
            dtls_started: AtomicBool::new(false),
            dc_open: AtomicBool::new(false),
            dc_out,
            dc_out_rx: std::sync::Mutex::new(Some(dc_out_rx)),
            dc_in_tx,
            dc_in: std::sync::Mutex::new(Some(dc_in)),
            cancel: CancellationToken::new(),
            rx_bytes: AtomicU64::new(0),
            tx_bytes: AtomicU64::new(0),
        }
    }

    pub fn cred(&self) -> Cred {
        self.creds.lock().map(|c| c.current.clone()).unwrap_or_else(|p| p.into_inner().current.clone())
    }

    pub fn ufrag(&self) -> String {
        self.cred().ufrag
    }

    pub fn ufrags(&self) -> Vec<String> {
        self.creds.lock().map(|c| c.ufrags()).unwrap_or_default()
    }

    pub fn pwd_of(&self, ufrag: &str) -> Option<String> {
        self.creds.lock().ok().and_then(|c| c.pwd_of(ufrag).map(str::to_string))
    }

    pub fn restart(&self) -> (Cred, Option<String>) {
        match self.creds.lock() {
            Ok(mut c) => c.restart(),
            Err(p) => p.into_inner().restart(),
        }
    }

    pub fn on_check(&self, path: Path, ufrag: &str, nominate: bool, now: u64) -> CheckOutcome {
        let promoted = match self.creds.lock() {
            Ok(mut c) => match c.checked(ufrag) {
                Checked::Promoted { dropped } => Some(dropped),
                _ => None,
            },
            Err(_) => None,
        };
        let mut out = CheckOutcome { promoted: promoted.clone(), path_changed: false, alive_again: None };
        let Ok(mut l) = self.link.lock() else { return out };
        match l.pairs.iter_mut().find(|p| p.path == path) {
            Some(p) => {
                p.last_ok_ms = now;
                p.closed = false;
            }
            None => l.pairs.push(Pair { path, last_ok_ms: now, closed: false }),
        }
        if nominate {
            if l.nominated != Some(path) {
                l.nominated = Some(path);
                self.path.store(Some(Arc::new(path)));
                out.path_changed = true;
            }
            if l.state == PeerState::Suspect {
                let since = self.suspect_since.swap(0, Ordering::AcqRel);
                l.state = PeerState::Alive;
                l.since_ms = now;
                out.alive_again = Some(now.saturating_sub(since));
            }
        } else if l.nominated.is_none() {
            l.nominated = Some(path);
            self.path.store(Some(Arc::new(path)));
            out.path_changed = true;
        }
        let keep = l.nominated;
        l.pairs.retain(|p| p.live(now) || Some(p.path) == keep);
        out
    }

    pub fn tcp_closed(&self, id: u64) {
        if let Ok(mut l) = self.link.lock() {
            for p in l.pairs.iter_mut() {
                if p.path == Path::Tcp(id) {
                    p.closed = true;
                }
            }
        }
    }

    pub fn liveness(&self, now: u64) -> Option<SuspectEvent> {
        let mut l = self.link.lock().ok()?;
        let nominated = l.nominated?;
        let live = l.pairs.iter().any(|p| p.path == nominated && p.live(now));
        if live || l.state == PeerState::Suspect {
            return None;
        }
        let _ = self.suspect_since.compare_exchange(0, now.max(1), Ordering::AcqRel, Ordering::Acquire);
        l.state = PeerState::Suspect;
        l.since_ms = now;
        drop(l);
        Some(SuspectEvent { ufrag: self.ufrag() })
    }

    pub fn state(&self) -> (PeerState, u64) {
        self.link.lock().map(|l| (l.state, l.since_ms)).unwrap_or((PeerState::Suspect, 0))
    }

    pub fn path(&self) -> Option<Path> {
        self.path.load().as_deref().copied()
    }

    pub fn live_tcp_pairs(&self, now: u64) -> Vec<u64> {
        self.link
            .lock()
            .map(|l| {
                l.pairs
                    .iter()
                    .filter(|p| p.live(now))
                    .filter_map(|p| match p.path {
                        Path::Tcp(i) => Some(i),
                        Path::Udp(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn known_paths(&self) -> Vec<Path> {
        self.link.lock().map(|l| l.pairs.iter().map(|p| p.path).collect()).unwrap_or_default()
    }

    pub fn send_raw(&self, b: &[u8]) -> bool {
        let Some(p) = self.path() else { return false };
        if self.io.send(&p, b) {
            self.tx_bytes.fetch_add(b.len() as u64, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    pub fn protect_rtp(&self, plain: &[u8]) -> Option<bytes::Bytes> {
        let mut g = self.srtp_tx.lock().ok()?;
        g.as_mut()?.encrypt_rtp(plain).ok()
    }

    pub fn protect_rtcp(&self, plain: &[u8]) -> Option<bytes::Bytes> {
        let mut g = self.srtp_tx.lock().ok()?;
        g.as_mut()?.encrypt_rtcp(plain).ok()
    }

    pub fn unprotect_rtp(&self, enc: &[u8]) -> Option<bytes::Bytes> {
        let mut g = self.srtp_rx.lock().ok()?;
        g.as_mut()?.decrypt_rtp(enc).ok()
    }

    pub fn unprotect_rtcp(&self, enc: &[u8]) -> Option<bytes::Bytes> {
        let mut g = self.srtp_rx.lock().ok()?;
        g.as_mut()?.decrypt_rtcp(enc).ok()
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

    pub fn dc_send(&self, msg: Vec<u8>) -> bool {
        self.dc_out.try_send(msg).is_ok()
    }

    pub fn close(&self) {
        self.cancel.cancel();
        if let Ok(mut g) = self.dtls_in.lock() {
            g.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn t() -> Transport {
        let s = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        Transport::new(1, Role::Both, Arc::new(Io::new(Arc::new(s))), 0)
    }

    #[tokio::test]
    async fn nominated_pair_drives_state() {
        let t = t().await;
        let u = t.ufrag();
        let a: SocketAddr = "127.0.0.1:4000".parse().expect("a");
        t.on_check(Path::Udp(a), &u, true, 1000);
        assert_eq!(t.path(), Some(Path::Udp(a)));
        assert!(t.liveness(20_000).is_none());
        let ev = t.liveness(31_001 + 1000).expect("suspect");
        assert_eq!(ev.ufrag, u);
        assert!(t.liveness(40_000).is_none(), "통지를 되풀이하지 않는다");
        let b: SocketAddr = "127.0.0.1:4001".parse().expect("b");
        let o = t.on_check(Path::Udp(b), &u, true, 41_000);
        assert!(o.path_changed && o.alive_again.is_some());
        assert_eq!(t.state().0, PeerState::Alive);
    }

    #[tokio::test]
    async fn backup_check_does_not_move_path() {
        let t = t().await;
        let u = t.ufrag();
        let a: SocketAddr = "127.0.0.1:4000".parse().expect("a");
        let b: SocketAddr = "127.0.0.1:4001".parse().expect("b");
        t.on_check(Path::Udp(a), &u, true, 1);
        t.on_check(Path::Udp(b), &u, false, 2);
        assert_eq!(t.path(), Some(Path::Udp(a)));
    }
}
