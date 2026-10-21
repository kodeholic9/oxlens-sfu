// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-LINK-TRANSPORT 정:RULE-LINK-PAIR-LIVENESS 정:IMPL-LINK-PEER-STATE 정:IMPL-LINK-ICE-RESTART RFC 4571 RFC 7983 · model: claude-opus-5-5
//! 전송로 — UDP 하나 · TCP passive(RFC 4571). 첫 바이트로 가른다. STUN 은 MI 검증 뒤에만 처리하고, 경로는 클라가 지명한 쌍이다.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use bytes::Bytes;
use oxsig::Op;
use oxsig::wire::{LinkEventBody, LinkEventBodyType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::peer::{ALIVE, Conn, Pair, Path, SUSPECT, TcpLink};
use crate::sfu::{Sfu, lock, mono_ms};
use crate::stun;

const TCP_IDENT_MS: u64 = 10_000;
pub const TCP_KEEPALIVE_MS: u64 = 20_000;

fn links() -> &'static Mutex<HashMap<u64, Arc<TcpLink>>> {
    static L: OnceLock<Mutex<HashMap<u64, Arc<TcpLink>>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn spawn_udp(sfu: &Arc<Sfu>) {
    let s = sfu.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 2048];
        loop {
            match s.udp.recv_from(&mut buf).await {
                Ok((n, from)) => demux(&s, Path::Udp(from), &buf[..n]),
                Err(e) => {
                    tracing::warn!(kind = "diag", "udp recv: {e}");
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    });
}

fn demux(sfu: &Arc<Sfu>, path: Path, b: &[u8]) {
    let Some(&first) = b.first() else { return };
    match first {
        0..=3 => on_stun(sfu, path, b),
        20..=63 => match conn_of(sfu, &path) {
            Some(c) => crate::secure::on_dtls(sfu, &c, b),
            None => sfu.counters.drop("dtls_unknown_addr"),
        },
        128..=191 => match conn_of(sfu, &path) {
            Some(c) => crate::secure::on_srtp(sfu, &c, b),
            None => sfu.counters.drop("rtp_unknown_addr"),
        },
        _ => sfu.counters.drop("demux"),
    }
}

fn conn_of(sfu: &Sfu, path: &Path) -> Option<Arc<Conn>> {
    match path {
        Path::Udp(a) => sfu.conn_by_addr(a),
        Path::Tcp(l) => lock(&l.conn).upgrade(),
    }
}

fn on_stun(sfu: &Arc<Sfu>, path: Path, b: &[u8]) {
    let Some(req) = stun::parse(b) else {
        sfu.counters.drop("stun_parse");
        return;
    };
    let Some(ufrag) = req.local_ufrag().map(str::to_string) else {
        sfu.counters.drop("stun_no_user");
        return;
    };
    let Some(conn) = sfu.conn_by_ufrag(&ufrag) else {
        sfu.counters.drop("stun_unknown_ufrag");
        return;
    };
    let Some(pwd) = conn.pwd_for(&ufrag) else {
        sfu.counters.drop("stun_unknown_ufrag");
        return;
    };
    if !req.verify(&pwd) {
        sfu.counters.drop("stun_integrity");
        let res = stun::unauthorized(&req.txid);
        conn.send_on(&path, &res);
        return;
    }
    let now = mono_ms();
    lock(&conn.pairs).insert(path.key(), Pair { path: path.clone(), last_ok: now });
    match &path {
        Path::Udp(a) => sfu.index_addr(*a, &conn),
        Path::Tcp(l) => {
            let mut g = lock(&l.conn);
            if g.upgrade().is_none() {
                let w: Weak<Conn> = Arc::downgrade(&conn);
                let _ = std::mem::replace(&mut *g, w);
            }
        }
    }
    if conn.promote(sfu, &ufrag) {
        tracing::info!(kind = "ev", spec = "IMPL-LINK-ICE-RESTART", user = conn.user_id.as_str(), ufrag = ufrag.as_str(), "new ufrag in use");
    }
    let first = conn.path.load().is_none();
    if req.use_candidate || first {
        conn.path.store(Some(Arc::new(path.clone())));
    }
    if req.use_candidate && conn.state.swap(ALIVE, Ordering::SeqCst) == SUSPECT {
        let since = conn.suspect_since.swap(0, Ordering::SeqCst);
        conn.state_since.store(now, Ordering::Relaxed);
        tracing::info!(
            kind = "ev",
            spec = "IMPL-LINK-PEER-STATE",
            user = conn.user_id.as_str(),
            conn = conn.id,
            addr = path.addr().to_string().as_str(),
            suspect_ms = if since == 0 { 0 } else { now.saturating_sub(since) },
            "alive"
        );
    }
    let res = stun::success(&req.txid, path.addr(), &pwd);
    conn.send_on(&path, &res);
}

pub fn liveness(sfu: &Arc<Sfu>) {
    let now = mono_ms();
    let peers: Vec<Arc<crate::peer::Peer>> = lock(&sfu.peers).values().cloned().collect();
    for p in peers {
        for c in p.conns() {
            check_conn(sfu, &p, &c, now);
        }
    }
}

fn check_conn(sfu: &Arc<Sfu>, peer: &crate::peer::Peer, c: &Arc<Conn>, now: u64) {
    let path = c.path.load_full();
    let alive_path = {
        let mut pairs = lock(&c.pairs);
        pairs.retain(|_, pr| pr.path.alive(now, pr.last_ok));
        match path.as_ref() {
            Some(p) => pairs.contains_key(&p.key()),
            None => false,
        }
    };
    if path.is_some() && !alive_path && c.state.load(Ordering::SeqCst) == ALIVE {
        let _ = c.suspect_since.compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
        if c.state.swap(SUSPECT, Ordering::SeqCst) == ALIVE {
            c.state_since.store(now, Ordering::Relaxed);
            let body = LinkEventBody { type_: LinkEventBodyType::Suspect, sfu_id: sfu.epoch.clone(), ufrag: c.ufrag() };
            sfu.emit_user(&peer.user_id, Op::LinkEvent, &body);
            tracing::info!(kind = "ev", spec = "IMPL-LINK-PEER-STATE", user = peer.user_id.as_str(), conn = c.id, "suspect");
        }
    }
}

pub fn tcp_keepalive(sfu: &Sfu) {
    let all: Vec<Arc<TcpLink>> = lock(links()).values().cloned().collect();
    for l in all {
        if l.closed.load(Ordering::Relaxed) || lock(&l.conn).upgrade().is_none() {
            continue;
        }
        if l.tx.try_send(Bytes::from_static(&[0, 0])).is_err() {
            sfu.counters.drop("tcp_keepalive");
        }
    }
}

pub fn close_links_of(conn: &Arc<Conn>) {
    let all: Vec<Arc<TcpLink>> = lock(links()).values().cloned().collect();
    for l in all {
        if lock(&l.conn).upgrade().is_some_and(|c| Arc::ptr_eq(&c, conn)) {
            l.closed.store(true, Ordering::Relaxed);
            l.cancel.cancel();
        }
    }
}

pub fn spawn_tcp(sfu: &Arc<Sfu>, listener: tokio::net::TcpListener) {
    let s = sfu.clone();
    tokio::spawn(async move {
        loop {
            let (stream, addr) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(kind = "diag", "tcp accept: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            let _ = stream.set_nodelay(true);
            let s2 = s.clone();
            tokio::spawn(async move { tcp_conn(s2, stream, addr).await });
        }
    });
}

async fn tcp_conn(sfu: Arc<Sfu>, stream: tokio::net::TcpStream, addr: SocketAddr) {
    let (mut rd, mut wr) = stream.into_split();
    let (tx, mut rx) = mpsc::channel::<Bytes>(1024);
    let cancel = CancellationToken::new();
    let link = Arc::new(TcpLink {
        id: sfu.next_id(),
        addr,
        tx,
        closed: std::sync::atomic::AtomicBool::new(false),
        cancel: cancel.clone(),
        conn: Mutex::new(Weak::new()),
    });
    lock(links()).insert(link.id, link.clone());
    let wcancel = cancel.clone();
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                m = rx.recv() => {
                    let Some(m) = m else { break };
                    if wr.write_all(&m).await.is_err() {
                        break;
                    }
                }
                _ = wcancel.cancelled() => break,
            }
        }
        let _ = wr.shutdown().await;
    });
    let started = mono_ms();
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = vec![0u8; 65_536];
    loop {
        let ident = lock(&link.conn).upgrade().is_some();
        let wait = if ident { Duration::from_secs(3600) } else { Duration::from_millis(TCP_IDENT_MS.saturating_sub(mono_ms() - started).max(1)) };
        let n = tokio::select! {
            r = rd.read(&mut chunk) => match r {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            },
            _ = tokio::time::sleep(wait) => {
                if lock(&link.conn).upgrade().is_none() {
                    sfu.counters.drop("tcp_ident_timeout");
                    break;
                }
                continue;
            }
            _ = cancel.cancelled() => break,
        };
        buf.extend_from_slice(&chunk[..n]);
        while buf.len() >= 2 {
            let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
            if buf.len() < 2 + len {
                break;
            }
            let frame: Vec<u8> = buf[2..2 + len].to_vec();
            buf.drain(..2 + len);
            if frame.is_empty() {
                continue;
            }
            demux(&sfu, Path::Tcp(link.clone()), &frame);
        }
    }
    link.closed.store(true, Ordering::Relaxed);
    cancel.cancel();
    let _ = writer.await;
    lock(links()).remove(&link.id);
}
