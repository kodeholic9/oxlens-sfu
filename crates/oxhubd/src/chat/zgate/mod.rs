// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! zenoh 입구(zgate) — hub 의 바깥 포트 하나(`[hook] zenoh_listen` · `tls/` 만): 3rd 의 zenoh 1.x 클라이언트를 직접 받는다(zenohd 없음 · 버스 그물에 안 들인다) · TLS 인증서 첫 CN = 계정 · InitSyn → InitAck(쿠키) → OpenSyn → OpenAck · 판 0x09 · client 만 · 확장은 되돌리지 않는다 · 연결마다 hub 가 짓는 이름(`{기동 신원}{세션 번호 hex}`) · 제 계정 키 queryable 이 서면 버스 토큰 `int/ext/{key}/{node}/{cid}` 를 내고 걷히거나 닫히면 거둔다 · lease 넘김 · KeepAlive · 연결표 · 연결에 묶음 질의를 보내고 결과(ACK · 거절 · 답 없음 · 닫힘)를 돌려받는 경계(창 · 줄 · drain 은 그 위) · 연결마다 보낼 줄(창 · 줄 · 시간 넘김 — `send`) · drain · 버림 계수 다섯(거절 · Push · 시간 넘김 · 줄 넘침 · 시도 수 포기)의 주인 · 토큰 선언이 실패하면 세션을 닫는다(3rd 가 다시 붙는다).

pub mod judge;
pub mod link;
pub mod plan;
pub mod send;
pub mod session;
pub mod wire;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::ops::AddAssign;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::keyspace::Keys;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tokio_rustls::TlsAcceptor;
use zenoh_buffers::ZSlice;
use zenoh_protocol::core::resolution::Field;
use zenoh_protocol::core::{Resolution, WhatAmI, ZenohIdProto};
use zenoh_protocol::transport::close;
use zenoh_protocol::transport::init::ext::PatchType;
use zenoh_protocol::transport::{BatchSize, Close, InitAck, KeepAlive, OpenAck, TransportBody, TransportMessage, TransportSn, batch_size};

use crate::bus::Bus;
use judge::{Accounts, Agreed, Refuse, judge_account, judge_init};
use session::{Answer, Effect, End, Opened, Session};

pub const LEASE: Duration = Duration::from_secs(10);
pub const KEEPALIVE: Duration = Duration::from_millis(2_500);
pub const HANDSHAKE: Duration = Duration::from_secs(10);
pub const COOKIE_LEN: usize = 16;
pub const SCHEME: &str = "tls/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateCfg {
    pub listen: String,
    pub cert: String,
    pub key: String,
    pub ca: String,
}

#[derive(Default)]
pub struct Drops {
    pub refused: AtomicU64,
    pub push_dropped: AtomicU64,
    pub reply_timeout: AtomicU64,
    pub queue_full: AtomicU64,
    pub given_up: AtomicU64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnInfo {
    pub cid: String,
    pub key: String,
    pub open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    NoConn,
    NotOpen,
}

enum Cmd {
    Query { ticket: u64, body: Vec<u8>, tx: oneshot::Sender<Answer> },
    Forget(u64),
    Close(&'static str),
    Kick,
}

struct Handle {
    key: String,
    open: bool,
    cmd: mpsc::UnboundedSender<Cmd>,
    outbox: plan::Outbox<serde_json::Value, send::Waiter>,
    released: bool,
}

pub struct Pending {
    pub ticket: u64,
    rx: oneshot::Receiver<Answer>,
    cmd: mpsc::UnboundedSender<Cmd>,
    done: bool,
}

impl Pending {
    pub async fn answer(mut self) -> Answer {
        let a = (&mut self.rx).await.unwrap_or(Answer::Closed);
        self.done = true;
        a
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.cmd.send(Cmd::Forget(self.ticket));
        }
    }
}

pub struct Gate {
    pub addr: SocketAddr,
    listener: Mutex<Option<TcpListener>>,
    acceptor: TlsAcceptor,
    accounts: Accounts,
    keys: Keys,
    bus: Arc<Bus>,
    zid: ZenohIdProto,
    boot: String,
    seq: AtomicU64,
    ticket: AtomicU64,
    conns: Mutex<HashMap<String, Handle>>,
    seen: Mutex<HashMap<&'static str, u64>>,
    store: Arc<crate::chat::store::Store>,
    acks: Arc<crate::chat::acks::AckLanes>,
    knobs: send::GateKnobs,
    keys_sub: Vec<String>,
    drain_st: Mutex<send::DrainStates>,
    pub drops: Arc<Drops>,
}

fn zid() -> ZenohIdProto {
    let b = common::random_bytes(16);
    let mut a = [0u8; 16];
    a.copy_from_slice(&b[..16]);
    if a.iter().all(|x| *x == 0) {
        a[0] = 1;
    }
    ZenohIdProto::try_from(a).unwrap_or_default()
}

fn rand_u32() -> u32 {
    let b = common::random_bytes(4);
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn close_msg(reason: u8) -> TransportMessage {
    TransportMessage::from(Close { reason, session: true })
}

async fn send_msgs<W: AsyncWrite + Unpin>(w: &mut W, msgs: &[TransportMessage]) -> std::io::Result<()> {
    let mut parts = Vec::with_capacity(msgs.len());
    for m in msgs {
        parts.push(wire::encode(m).map_err(|e| std::io::Error::other(format!("{e:?}")))?);
    }
    link::write_batches(w, &wire::pack(parts, batch_size::UNICAST)).await
}

async fn recv_one<R: AsyncRead + Unpin>(r: &mut R) -> Result<TransportMessage, Refuse> {
    let b = tokio::time::timeout(HANDSHAKE, link::read_batch(r)).await.map_err(|_| Refuse::Protocol("handshake timeout"))?.map_err(|_| Refuse::Protocol("link closed in handshake"))?;
    let mut v = wire::decode_batch(&b).map_err(|_| Refuse::Protocol("handshake did not decode"))?;
    if v.len() != 1 {
        return Err(Refuse::Protocol("handshake batch is not one message"));
    }
    Ok(v.remove(0))
}

struct Est {
    session: Session,
    lease: Duration,
}

type Ending = (&'static str, Option<Refuse>, Option<u8>);

#[derive(Default)]
struct Applied {
    parts: Vec<Vec<u8>>,
    answers: Vec<(u64, Answer)>,
    close: bool,
}

impl Gate {
    pub async fn bind(cfg: &GateCfg, deps: send::GateDeps) -> Result<Arc<Gate>, String> {
        let send::GateDeps { accounts, bus, store, acks, keys, knobs } = deps;
        let at = cfg.listen.strip_prefix(SCHEME).ok_or_else(|| format!("hook.zenoh_listen {:?} — tls/ 꼴만", cfg.listen))?;
        let at = at.split(['#', '?']).next().unwrap_or(at);
        let acceptor = link::acceptor(&cfg.cert, &cfg.key, &cfg.ca)?;
        let listener = TcpListener::bind(at).await.map_err(|e| format!("hook.zenoh_listen {at}: {e}"))?;
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        Ok(Arc::new(Gate {
            addr,
            listener: Mutex::new(Some(listener)),
            acceptor,
            accounts,
            keys: bus.keys.clone(),
            boot: bus.inst.clone(),
            bus,
            zid: zid(),
            seq: AtomicU64::new(0),
            ticket: AtomicU64::new(0),
            conns: Mutex::new(HashMap::new()),
            seen: Mutex::new(HashMap::new()),
            store,
            acks,
            knobs,
            keys_sub: keys,
            drain_st: Mutex::new(HashMap::new()),
            drops: Arc::new(Drops::default()),
        }))
    }

    pub async fn serve(self: Arc<Self>) {
        let Some(l) = self.listener.lock().ok().and_then(|mut g| g.take()) else { return };
        tracing::info!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", addr = %self.addr, "zenoh gate listening");
        loop {
            match l.accept().await {
                Ok((tcp, peer)) => {
                    let me = self.clone();
                    tokio::spawn(async move { me.conn(tcp, peer).await });
                }
                Err(e) => {
                    tracing::warn!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", "zenoh gate accept: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    pub fn conns(&self) -> Vec<ConnInfo> {
        let mut v: Vec<ConnInfo> = self
            .conns
            .lock()
            .map(|g| g.iter().map(|(cid, h)| ConnInfo { cid: cid.clone(), key: h.key.clone(), open: h.open }).collect())
            .unwrap_or_default();
        v.sort_by(|a, b| a.cid.cmp(&b.cid));
        v
    }

    pub fn query(&self, cid: &str, body: Vec<u8>) -> Result<Pending, QueryError> {
        let g = self.conns.lock().map_err(|_| QueryError::NoConn)?;
        let h = g.get(cid).ok_or(QueryError::NoConn)?;
        if !h.open {
            return Err(QueryError::NotOpen);
        }
        let ticket = self.ticket.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        h.cmd.send(Cmd::Query { ticket, body, tx }).map_err(|_| QueryError::NoConn)?;
        Ok(Pending { ticket, rx, cmd: h.cmd.clone(), done: false })
    }

    pub fn close(&self, cid: &str, why: &'static str) -> bool {
        self.conns.lock().ok().and_then(|g| g.get(cid).map(|h| h.cmd.send(Cmd::Close(why)).is_ok())).unwrap_or(false)
    }

    fn bump(&self, why: &'static str) -> u64 {
        self.seen.lock().map(|mut g| g.entry(why).and_modify(|c| c.add_assign(1)).or_insert(1).to_owned()).unwrap_or(1)
    }

    fn refuse(&self, r: &Refuse, peer: SocketAddr) {
        self.drops.refused.fetch_add(1, Ordering::Relaxed);
        let n = self.bump(r.reason());
        if n.is_power_of_two() {
            tracing::warn!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", peer = %peer, why = r.reason(), detail = %r.detail(), times = n, "zenoh gate refused");
        }
    }

    fn sparse(&self, why: &'static str, detail: &str) {
        let n = self.bump(why);
        if n.is_power_of_two() {
            tracing::warn!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", why = why, detail = %detail, times = n, "zenoh gate");
        }
    }

    async fn conn(self: Arc<Self>, tcp: TcpStream, peer: SocketAddr) {
        let _ = tcp.set_nodelay(true);
        let tls = match tokio::time::timeout(HANDSHAKE, self.acceptor.accept(tcp)).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return self.refuse(&Refuse::Tls(e.to_string()), peer),
            Err(_) => return self.refuse(&Refuse::Tls("tls handshake timeout".into()), peer),
        };
        let cn = link::first_cn(tls.get_ref().1);
        let (mut rd, mut wr) = tokio::io::split(tls);
        let est = match self.establish(&mut rd, &mut wr, cn).await {
            Ok(e) => e,
            Err((r, said)) => {
                if said {
                    let _ = send_msgs(&mut wr, &[close_msg(close::reason::INVALID)]).await;
                }
                let _ = wr.shutdown().await;
                return self.refuse(&r, peer);
            }
        };
        let cid = format!("{:012x}{}{:x}", common::now_ms(), self.boot, self.seq.fetch_add(1, Ordering::Relaxed));
        let key = est.session.cn.clone();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        if let Ok(mut g) = self.conns.lock() {
            g.insert(cid.clone(), Handle { key: key.clone(), open: false, cmd: cmd_tx, outbox: plan::Outbox::new(self.knobs.outbox()), released: false });
        }
        tracing::info!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", key = %key, cid = %cid, peer = %peer, "ext conn open");
        let (why, refused) = self.run(&cid, est, rd, &mut wr, cmd_rx).await;
        let _ = wr.shutdown().await;
        self.bus.undeclare_ext(&cid);
        if let Ok(mut g) = self.conns.lock() {
            g.remove(&cid);
        }
        if let Some(r) = refused {
            self.refuse(&r, peer);
        }
        tracing::info!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", key = %key, cid = %cid, why = why, "ext conn closed");
    }

    async fn establish<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(&self, rd: &mut R, wr: &mut W, cn: Option<String>) -> Result<Est, (Refuse, bool)> {
        let first = recv_one(rd).await.map_err(|r| (r, false))?;
        let TransportBody::InitSyn(syn) = first.body else { return Err((Refuse::Protocol("first message is not InitSyn"), true)) };
        let Agreed { resolution, batch } = judge_init(&syn, Resolution::default(), batch_size::UNICAST).map_err(|r| (r, true))?;
        let cn = judge_account(cn.as_deref(), &self.accounts).map_err(|r| (r, true))?;
        let cookie = common::random_bytes(COOKIE_LEN);
        let ack = TransportMessage::from(InitAck {
            version: judge::VERSION,
            whatami: WhatAmI::Router,
            zid: self.zid,
            resolution,
            batch_size: batch,
            cookie: ZSlice::from(cookie.clone()),
            ext_qos: None,
            ext_qos_link: None,
            ext_auth: None,
            ext_mlink: None,
            ext_lowlatency: None,
            ext_compression: None,
            ext_patch: PatchType::NONE,
            ext_region_name: None,
        });
        send_msgs(wr, &[ack]).await.map_err(|_| (Refuse::Protocol("link closed in handshake"), false))?;
        let second = recv_one(rd).await.map_err(|r| (r, false))?;
        let open = match second.body {
            TransportBody::OpenSyn(o) => o,
            TransportBody::Close(_) => return Err((Refuse::Protocol("client closed in handshake"), false)),
            _ => return Err((Refuse::Protocol("second message is not OpenSyn"), true)),
        };
        if open.cookie.as_slice() != cookie.as_slice() {
            return Err((Refuse::Cookie, true));
        }
        let sn_mask = wire::sn_mask(&resolution);
        let tx_initial: TransportSn = rand_u32() & sn_mask;
        let oack = TransportMessage::from(OpenAck {
            lease: LEASE,
            initial_sn: tx_initial,
            ext_qos: None,
            ext_auth: None,
            ext_mlink: None,
            ext_lowlatency: None,
            ext_compression: None,
            ext_remote_bound: None,
        });
        send_msgs(wr, &[oack]).await.map_err(|_| (Refuse::Protocol("link closed in handshake"), false))?;
        let session = Session::new(Opened {
            own_key: self.keys.chat_sub(&cn),
            prefix: self.keys.chat_sub_prefix(),
            cn,
            rx_initial: open.initial_sn,
            tx_initial,
            sn_mask,
            rid_mask: resolution.get(Field::RequestID).mask() as u32,
            batch: batch as BatchSize,
        });
        Ok(Est { session, lease: open.lease })
    }

    fn set_open(&self, cid: &str, open: bool) {
        if let Ok(mut g) = self.conns.lock()
            && let Some(h) = g.get_mut(cid)
        {
            h.open = open;
        }
    }

    async fn apply(&self, cid: &str, session: &mut Session, effects: Vec<Effect>, waiting: &mut HashMap<u64, oneshot::Sender<Answer>>, out: &mut Applied) {
        for e in effects {
            match e {
                Effect::Send(n) => match wire::parts(&n, &mut session.tx, session.batch) {
                    Ok(p) => out.parts.extend(p),
                    Err(w) => self.sparse("send did not encode", &format!("{w:?}")),
                },
                Effect::Open => match self.bus.declare_ext(&session.cn, cid).await {
                    Ok(()) => {
                        self.set_open(cid, true);
                        tracing::info!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", key = %session.cn, cid = %cid, "ext queryable declared — token up");
                    }
                    Err(e) => {
                        tracing::warn!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", key = %session.cn, cid = %cid, "ext token not declared: {e} — session closed, the client connects again");
                        out.close = true;
                    }
                },
                Effect::Shut => {
                    self.bus.undeclare_ext(cid);
                    self.set_open(cid, false);
                }
                Effect::Answer { ticket, answer } => match waiting.remove(&ticket) {
                    Some(tx) => {
                        let _ = tx.send(answer);
                    }
                    None => out.answers.push((ticket, answer)),
                },
                Effect::Ignored(k) => {
                    self.drops.refused.fetch_add(1, Ordering::Relaxed);
                    self.sparse("declaration outside the allowed key", &k);
                }
                Effect::PushDropped => {
                    self.drops.push_dropped.fetch_add(1, Ordering::Relaxed);
                }
                Effect::Dropped(why) => self.sparse(why, cid),
            }
        }
    }

    async fn run<R, W>(&self, cid: &str, est: Est, rd: R, wr: &mut W, mut cmd: mpsc::UnboundedReceiver<Cmd>) -> (&'static str, Option<Refuse>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin,
    {
        let Est { mut session, lease } = est;
        let (btx, mut brx) = mpsc::channel::<std::io::Result<Vec<u8>>>(16);
        let reader = tokio::spawn(async move {
            let mut rd = rd;
            loop {
                let r = link::read_batch(&mut rd).await;
                let stop = r.is_err();
                if btx.send(r).await.is_err() || stop {
                    break;
                }
            }
        });
        let mut waiting: HashMap<u64, oneshot::Sender<Answer>> = HashMap::new();
        let mut flights: HashMap<u64, send::Flight> = HashMap::new();
        let key = session.cn.clone();
        let mut last_rx = Instant::now();
        let mut last_tx = Instant::now();
        let out = loop {
            let mut effects: Vec<Effect> = Vec::new();
            let mut ending: Option<Ending> = None;
            let mut expired: Vec<u64> = Vec::new();
            let next_deadline = flights.values().map(|f| f.deadline).min().unwrap_or_else(|| Instant::now() + LEASE);
            tokio::select! {
                b = brx.recv() => match b {
                    Some(Ok(bytes)) => {
                        last_rx = Instant::now();
                        match wire::decode_batch(&bytes) {
                            Ok(msgs) => {
                                for m in msgs {
                                    match session.on_transport(m) {
                                        Ok(e) => effects.extend(e),
                                        Err(End::Peer(_)) => { ending = Some(("peer closed", None, None)); break; }
                                        Err(End::Refused(r)) => { ending = Some(("refused", Some(r), Some(close::reason::INVALID))); break; }
                                    }
                                }
                            }
                            Err(_) => ending = Some(("batch did not decode", Some(Refuse::Protocol("batch did not decode")), Some(close::reason::INVALID))),
                        }
                    }
                    _ => ending = Some(("link closed", None, None)),
                },
                c = cmd.recv() => match c {
                    Some(Cmd::Query { ticket, body, tx }) => {
                        waiting.insert(ticket, tx);
                        effects.push(Effect::Send(Box::new(session.query(ticket, body))));
                    }
                    Some(Cmd::Forget(t)) => {
                        session.forget(t);
                        waiting.remove(&t);
                    }
                    Some(Cmd::Close(why)) => ending = Some((why, None, Some(close::reason::GENERIC))),
                    Some(Cmd::Kick) => {}
                    None => ending = Some(("gate gone", None, Some(close::reason::GENERIC))),
                },
                _ = tokio::time::sleep_until(next_deadline), if !flights.is_empty() => {
                    let now = Instant::now();
                    expired = flights.iter().filter(|(_, f)| f.deadline <= now).map(|(t, _)| *t).collect();
                }
                _ = tokio::time::sleep_until(last_rx + lease) => ending = Some(("lease expired", None, Some(close::reason::EXPIRED))),
                _ = tokio::time::sleep_until(last_tx + KEEPALIVE) => {
                    if send_msgs(wr, &[TransportMessage::from(KeepAlive)]).await.is_err() {
                        ending = Some(("link closed", None, None));
                    }
                    last_tx = Instant::now();
                }
            }
            let mut applied = Applied::default();
            self.apply(cid, &mut session, effects, &mut waiting, &mut applied).await;
            if applied.close && ending.is_none() {
                ending = Some(("ext token not declared", None, Some(close::reason::GENERIC)));
            }
            let mut ends: Vec<(send::Flight, plan::Outcome)> = Vec::new();
            for t in expired {
                session.forget(t);
                if let Some(f) = flights.remove(&t) {
                    ends.push((f, plan::Outcome::Timeout));
                }
            }
            for (t, a) in applied.answers {
                let o = match a {
                    Answer::Ack => plan::Outcome::Ack,
                    Answer::Refused(_) | Answer::NoReply => plan::Outcome::Fail,
                    Answer::Closed => continue,
                };
                if let Some(f) = flights.remove(&t) {
                    ends.push((f, o));
                }
            }
            for (f, o) in ends {
                if self.finish(cid, &key, f, o) == plan::After::Close && ending.is_none() {
                    ending = Some(("reply timeouts in a row", None, Some(close::reason::GENERIC)));
                }
            }
            if ending.is_none() {
                for it in self.pull(cid) {
                    let ticket = self.ticket.fetch_add(1, Ordering::Relaxed);
                    let body = serde_json::to_vec(&serde_json::json!({ "live": it.live, "messages": it.msgs })).unwrap_or_default();
                    let q = session.query(ticket, body);
                    flights.insert(ticket, send::Flight { ids: it.ids, deadline: Instant::now() + self.knobs.reply_timeout, waiter: it.waiter });
                    match wire::parts(&q, &mut session.tx, session.batch) {
                        Ok(p) => applied.parts.extend(p),
                        Err(w) => self.sparse("send did not encode", &format!("{w:?}")),
                    }
                }
            }
            let parts = applied.parts;
            if !parts.is_empty() {
                if link::write_batches(wr, &wire::pack(parts, session.batch)).await.is_err() && ending.is_none() {
                    ending = Some(("link closed", None, None));
                }
                last_tx = Instant::now();
            }
            if let Some((why, refused, say)) = ending {
                if let Some(reason) = say {
                    let _ = send_msgs(wr, &[close_msg(reason)]).await;
                }
                break (why, refused);
            }
        };
        reader.abort();
        for e in session.end() {
            if let Effect::Answer { ticket, answer } = e
                && let Some(tx) = waiting.remove(&ticket)
            {
                let _ = tx.send(answer);
            }
        }
        for (_, tx) in waiting.drain() {
            let _ = tx.send(Answer::Closed);
        }
        out
    }
}
