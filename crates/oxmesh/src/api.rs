// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-MESH-SHAPE · 정:RULE-MESH-DECL · 정:RULE-MESH-LANE · 정:RULE-MESH-REQUEST · model: claude-opus-5-5
//! 그물을 쓰는 꼴 — 열기(router · client) · 토큰 · 토큰 구독(history · 일괄 끝) · put · 구독 · 응답자 · get · 진단(info) · 닫기. 손잡이를 놓으면 철회한다.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::core::{Cmd, Core, Wiring, accept_loop};
use crate::keyexpr::{concrete, valid};
use crate::wire::DeclKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeshError {
    Closed,
    Invalid(String),
    Io(String),
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeshError::Closed => write!(f, "mesh closed"),
            MeshError::Invalid(e) => write!(f, "invalid key expression {e:?}"),
            MeshError::Io(e) => write!(f, "mesh io: {e}"),
        }
    }
}

impl std::error::Error for MeshError {}

#[derive(Debug, Clone)]
pub struct Config {
    pub node: String,
    pub router: bool,
    pub listen: Vec<String>,
    pub advertise: Vec<String>,
    pub connect: Vec<String>,
    pub lease: Duration,
}

impl Config {
    pub fn router(node: &str) -> Config {
        Config { node: node.to_string(), router: true, listen: Vec::new(), advertise: Vec::new(), connect: Vec::new(), lease: Duration::from_secs(10) }
    }

    pub fn client(node: &str, router: &str) -> Config {
        Config { node: node.to_string(), router: false, listen: Vec::new(), advertise: Vec::new(), connect: vec![router.to_string()], lease: Duration::from_secs(10) }
    }
}

pub fn endpoint(e: &str) -> &str {
    e.strip_prefix("tcp/").unwrap_or(e)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub key: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenEvent {
    Put(String),
    Delete(String),
    BatchEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub key: String,
    pub result: Result<Vec<u8>, Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct SessInfo {
    pub peer: String,
    pub router: bool,
    pub dialer: String,
    pub boot: u64,
    pub sno: u64,
    pub addr: String,
    pub decls: usize,
    pub depths: [usize; 4],
    pub dropped: u64,
    pub full: u64,
    pub age_ms: u64,
}

#[derive(Debug, Clone)]
pub struct Info {
    pub node: String,
    pub router: bool,
    pub boot: u64,
    pub tokens: usize,
    pub sessions: Vec<SessInfo>,
}

#[derive(Clone)]
pub struct Mesh {
    cmd: mpsc::UnboundedSender<Cmd>,
    node: String,
    bound: Vec<SocketAddr>,
    dropped: Arc<AtomicU64>,
}

fn check(e: &str) -> Result<(), MeshError> {
    if valid(e) { Ok(()) } else { Err(MeshError::Invalid(e.to_string())) }
}

impl Mesh {
    pub async fn open(cfg: Config) -> Result<Mesh, MeshError> {
        if !crate::keyexpr::valid(&cfg.node) || cfg.node.contains(['/', '*']) {
            return Err(MeshError::Invalid(cfg.node));
        }
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let (sess_tx, sess_rx) = mpsc::unbounded_channel();
        let mut bound = Vec::new();
        let mut listeners = Vec::new();
        if cfg.router {
            for l in &cfg.listen {
                let li = tokio::net::TcpListener::bind(endpoint(l)).await.map_err(|e| MeshError::Io(format!("listen {l}: {e}")))?;
                let a = li.local_addr().map_err(|e| MeshError::Io(e.to_string()))?;
                bound.push(a);
                listeners.push(li);
            }
        }
        let adv: Vec<String> =
            if cfg.advertise.is_empty() { bound.iter().map(|a| a.to_string()).collect() } else { cfg.advertise.iter().map(|a| endpoint(a).to_string()).collect() };
        let seeds: Vec<String> = cfg.connect.iter().map(|c| endpoint(c).to_string()).collect();
        let dropped = Arc::new(AtomicU64::new(0));
        let accepts = listeners.into_iter().map(|li| tokio::spawn(accept_loop(li, ev_tx.clone())).abort_handle()).collect();
        let core = Core::new(cfg.node.clone(), cfg.router, cfg.lease, adv, seeds, Wiring { cmd_tx: cmd_tx.clone(), ev_tx, sess_tx, accepts, dropped: dropped.clone() });
        tokio::spawn(core.run(cmd_rx, ev_rx, sess_rx));
        let listen = bound.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(",");
        let some = |s: String| (!s.is_empty()).then_some(s);
        let (listen, advertise, connect) = (some(listen), some(cfg.advertise.join(",")), some(cfg.connect.join(",")));
        tracing::info!(
            kind = "diag",
            spec = "SV-RULE-MESH-SHAPE",
            me = %cfg.node,
            router = cfg.router,
            listen = listen.as_deref(),
            advertise = advertise.as_deref(),
            connect = connect.as_deref(),
            lease_ms = cfg.lease.as_millis() as u64,
            "mesh open"
        );
        Ok(Mesh { cmd: cmd_tx, node: cfg.node, bound, dropped })
    }

    pub fn node(&self) -> &str {
        &self.node
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn bound(&self) -> &[SocketAddr] {
        &self.bound
    }

    fn send(&self, c: Cmd) -> Result<(), MeshError> {
        self.cmd.send(c).map_err(|_| MeshError::Closed)
    }

    pub async fn declare_token(&self, key: &str) -> Result<Token, MeshError> {
        check(key)?;
        let (ack, rx) = oneshot::channel();
        self.send(Cmd::DeclareToken { key: key.to_string(), ack })?;
        let id = rx.await.map_err(|_| MeshError::Closed)?;
        Ok(Token { id, key: key.to_string(), cmd: self.cmd.clone() })
    }

    pub async fn token_sub(&self, expr: &str, history: bool) -> Result<TokenSub, MeshError> {
        check(expr)?;
        let (tx, rx) = mpsc::unbounded_channel();
        let (ack, arx) = oneshot::channel();
        self.send(Cmd::TokenSub { expr: expr.to_string(), history, tx, ack })?;
        let id = arx.await.map_err(|_| MeshError::Closed)?;
        Ok(TokenSub { id, rx, cmd: self.cmd.clone() })
    }

    pub async fn tokens(&self, expr: &str, timeout: Duration) -> Result<Vec<String>, MeshError> {
        let mut sub = self.token_sub(expr, true).await?;
        let mut out = Vec::new();
        let until = tokio::time::Instant::now() + timeout;
        loop {
            match tokio::time::timeout_at(until, sub.recv()).await {
                Ok(Some(TokenEvent::Put(k))) => out.push(k),
                Ok(Some(TokenEvent::Delete(k))) => out.retain(|x| *x != k),
                Ok(Some(TokenEvent::BatchEnd)) | Ok(None) | Err(_) => break,
            }
        }
        Ok(out)
    }

    pub fn put(&self, key: &str, payload: Vec<u8>) -> Result<(), MeshError> {
        if !valid(key) || !concrete(key) {
            return Err(MeshError::Invalid(key.to_string()));
        }
        self.send(Cmd::Put { key: key.to_string(), payload })
    }

    pub async fn subscribe(&self, expr: &str) -> Result<Subscriber, MeshError> {
        check(expr)?;
        let (tx, rx) = mpsc::unbounded_channel();
        let (ack, arx) = oneshot::channel();
        self.send(Cmd::Subscribe { expr: expr.to_string(), tx, ack })?;
        let id = arx.await.map_err(|_| MeshError::Closed)?;
        Ok(Subscriber { id, rx, cmd: self.cmd.clone() })
    }

    pub async fn queryable(&self, expr: &str) -> Result<Queryable, MeshError> {
        check(expr)?;
        let (tx, rx) = mpsc::unbounded_channel();
        let (ack, arx) = oneshot::channel();
        self.send(Cmd::Queryable { expr: expr.to_string(), tx, ack })?;
        let id = arx.await.map_err(|_| MeshError::Closed)?;
        Ok(Queryable { id, rx, cmd: self.cmd.clone() })
    }

    pub fn get(&self, expr: &str, payload: Vec<u8>, timeout: Duration) -> Result<Replies, MeshError> {
        check(expr)?;
        let (tx, rx) = mpsc::unbounded_channel();
        self.send(Cmd::Get { expr: expr.to_string(), payload, timeout, tx })?;
        Ok(Replies { rx })
    }

    pub async fn info(&self) -> Result<Info, MeshError> {
        let (ack, rx) = oneshot::channel();
        self.send(Cmd::Info { ack })?;
        rx.await.map_err(|_| MeshError::Closed)
    }

    pub async fn close(&self) {
        let (ack, rx) = oneshot::channel();
        if self.cmd.send(Cmd::Close { ack }).is_ok() {
            let _ = rx.await;
        }
    }
}

pub struct Token {
    id: u64,
    key: String,
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl Token {
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl Drop for Token {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Undeclare { kind: DeclKind::Token, id: self.id });
    }
}

pub struct TokenSub {
    id: u64,
    rx: mpsc::UnboundedReceiver<TokenEvent>,
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl TokenSub {
    pub async fn recv(&mut self) -> Option<TokenEvent> {
        self.rx.recv().await
    }
}

impl Drop for TokenSub {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::TokenUnsub { id: self.id });
    }
}

pub struct Subscriber {
    id: u64,
    rx: mpsc::UnboundedReceiver<Sample>,
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl Subscriber {
    pub async fn recv(&mut self) -> Option<Sample> {
        self.rx.recv().await
    }
}

impl Drop for Subscriber {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Undeclare { kind: DeclKind::Sub, id: self.id });
    }
}

pub struct Queryable {
    id: u64,
    rx: mpsc::UnboundedReceiver<Query>,
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl Queryable {
    pub async fn recv(&mut self) -> Option<Query> {
        self.rx.recv().await
    }
}

impl Drop for Queryable {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Undeclare { kind: DeclKind::Qabl, id: self.id });
    }
}

pub struct Query {
    qid: u64,
    qabl: u64,
    key_expr: String,
    payload: Vec<u8>,
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl Query {
    pub(crate) fn new(qid: u64, qabl: u64, key_expr: String, payload: Vec<u8>, cmd: mpsc::UnboundedSender<Cmd>) -> Query {
        Query { qid, qabl, key_expr, payload, cmd }
    }

    pub fn key_expr(&self) -> &str {
        &self.key_expr
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn reply(&self, key: &str, payload: Vec<u8>) {
        let _ = self.cmd.send(Cmd::LocalReply { qid: self.qid, qabl: self.qabl, key: key.to_string(), ok: true, payload });
    }

    pub fn reply_err(&self, key: &str, payload: Vec<u8>) {
        let _ = self.cmd.send(Cmd::LocalReply { qid: self.qid, qabl: self.qabl, key: key.to_string(), ok: false, payload });
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::LocalFinal { qid: self.qid, qabl: self.qabl });
    }
}

pub struct Replies {
    rx: mpsc::UnboundedReceiver<Reply>,
}

impl Replies {
    pub async fn recv(&mut self) -> Option<Reply> {
        self.rx.recv().await
    }
}
