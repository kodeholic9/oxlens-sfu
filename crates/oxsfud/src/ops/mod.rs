// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-DOC-PLANES · 정:RULE-DOC-OWNERSHIP · 정:CAND-DOC-ORCHESTRATION-ASPECT · 정:RULE-CONC-SIX · 정:RULE-BUS-FATE · 정:CAND-BUS-HANDLE-SHAPE · 정:CAND-BUS-SELF-WATCH-VALUES · 정:RULE-OPS-SURFACE · 정:RULE-OPS-OBSERVE · 정:IMPL-ROOM-VERSION-ISSUE · 정:IMPL-ROOM-EMIT · model: claude-opus-5-5
//! 조율(가로지르는 것) — 미디어 서버 하나의 상태 마스터: 제어 판정은 World 락 하나 안(판번호 증가와 통지 enqueue 가 한 임계 구역) · 핫패스는 RCU 사본만 읽는다 · gRPC Handle · 기동 · 정지.

pub mod admin;
pub mod call;
pub mod pubs;
pub mod rooms;
pub mod subs;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use common::bus::envelope::{Call, Notice, Reply};
use common::bus::grpc::handle_server::{Handle, HandleServer};
use common::bus::grpc::{CallReq, CallRes, HelloReq, HelloRes};
use oxsig::Op;

use crate::aff::Peer;
use crate::bus::Bus;
use crate::cfg::Boot;
use crate::fwd::RoomHot;
use crate::link::Net;
use crate::room::Room;

pub const UDP_BUF: usize = 4 * 1024 * 1024;

pub struct World {
    pub rooms: BTreeMap<String, Room>,
    pub peers: HashMap<String, Arc<Peer>>,
}

pub struct Sfu {
    pub boot: Boot,
    pub epoch: String,
    pub node: String,
    pub net: Arc<Net>,
    pub bus: Arc<Bus>,
    pub world: tokio::sync::Mutex<World>,
    pub hot: ArcSwap<HashMap<String, Arc<RoomHot>>>,
    pub peers: ArcSwap<HashMap<String, Arc<Peer>>>,
    pub counters: crate::err::Counters,
    pub shutting: std::sync::atomic::AtomicBool,
    pub me: std::sync::OnceLock<std::sync::Weak<Sfu>>,
}

impl Sfu {
    pub fn self_weak(&self) -> std::sync::Weak<Sfu> {
        self.me.get().cloned().unwrap_or_default()
    }

    pub fn now() -> u64 {
        common::time::mono_ms()
    }

    pub fn wall() -> u64 {
        common::time::wall_ms()
    }

    pub fn v2(&self) -> bool {
        matches!(self.boot.policy.media.auto_layer, common::policy::MediaAutolayer::V2)
    }

    pub fn auto_on(&self) -> bool {
        !matches!(self.boot.policy.media.auto_layer, common::policy::MediaAutolayer::Off)
    }

    pub fn room_hot(&self, room: &str) -> Option<Arc<RoomHot>> {
        self.hot.load().get(room).cloned()
    }

    pub fn put_hot(&self, h: Arc<RoomHot>) {
        self.hot.rcu(|m| {
            let mut m = HashMap::clone(m);
            m.insert(h.id.clone(), h.clone());
            m
        });
    }

    pub fn drop_hot(&self, room: &str) {
        self.hot.rcu(|m| {
            let mut m = HashMap::clone(m);
            m.remove(room);
            m
        });
    }

    pub fn peer(&self, user: &str) -> Option<Arc<Peer>> {
        self.peers.load().get(user).cloned()
    }

    pub fn sync_peers(&self, w: &World) {
        self.peers.store(Arc::new(w.peers.clone()));
    }

    pub fn notice(&self, op: Op, body: serde_json::Value, room: Option<&str>, exclude: Vec<String>, target: Option<&str>) -> Notice {
        Notice { op: op.as_u16(), body, exclude, room_id: room.map(str::to_string), sfu_id: Some(self.epoch.clone()), target: target.map(str::to_string) }
    }

    pub fn to_room(&self, room: &str, op: Op, body: serde_json::Value, exclude: Vec<String>) {
        let n = self.notice(op, body, Some(room), exclude, None);
        self.bus.to_room(room, &n);
    }

    pub fn to_user(&self, user: &str, op: Op, body: serde_json::Value, room: Option<&str>) {
        let n = self.notice(op, body, room, Vec::new(), Some(user));
        self.bus.to_user(user, &n);
    }

    pub fn fail(&self, f: oxsig::Failure) -> Reply {
        self.counters.code(f.code);
        Reply::Fail(f)
    }
}

pub struct Svc(pub Arc<Sfu>);

#[tonic::async_trait]
impl Handle for Svc {
    async fn hello(&self, req: tonic::Request<HelloReq>) -> Result<tonic::Response<HelloRes>, tonic::Status> {
        let r = req.into_inner();
        tracing::info!(kind = "ev", spec = "CAND-BUS-HANDLE-SHAPE", node = %r.node, hub_inst = %r.hub_inst, "Hello from hub");
        Ok(tonic::Response::new(HelloRes { epoch: self.0.epoch.clone(), build: self.0.boot.build.clone() }))
    }

    async fn call(&self, req: tonic::Request<CallReq>) -> Result<tonic::Response<CallRes>, tonic::Status> {
        let env = req.into_inner().envelope;
        let reply = match Call::decode(&env) {
            Ok(c) => call::dispatch(&self.0, c).await,
            Err(e) => self.0.fail(oxsig::Code::InvalidPayload.failure_with(format!("envelope: {e}"), None)),
        };
        Ok(tonic::Response::new(CallRes { reply: reply.encode() }))
    }
}

pub async fn run(boot: Boot) -> i32 {
    let epoch = common::identity::epoch();
    tracing::info!(kind = "ev", spec = "RULE-CFG-INDEX", "{}", boot.banner(&epoch));
    let (cert, fingerprint) = match crate::link::shell::make_cert() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(kind = "diag", "certificate: {e}");
            return 2;
        }
    };
    let udp = match tokio::net::UdpSocket::bind(std::net::SocketAddr::new(boot.bind_ip, boot.udp_port)).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!(kind = "diag", "udp bind {}:{}: {e}", boot.bind_ip, boot.udp_port);
            return 2;
        }
    };
    let tcp = match boot.tcp_port {
        Some(p) => match tokio::net::TcpListener::bind(std::net::SocketAddr::new(boot.bind_ip, p)).await {
            Ok(l) => Some(l),
            Err(e) => {
                tracing::error!(kind = "diag", "tcp bind {}:{p}: {e}", boot.bind_ip);
                return 2;
            }
        },
        None => None,
    };
    let fd = std::os::fd::AsRawFd::as_raw_fd(udp.as_ref());
    let _ = nix::sys::socket::setsockopt(fd, nix::sys::socket::sockopt::SndBuf, &UDP_BUF);
    let _ = nix::sys::socket::setsockopt(fd, nix::sys::socket::sockopt::RcvBuf, &UDP_BUF);
    let net = Net::new(udp, boot.public_ip, boot.udp_port, boot.tcp_port, cert, fingerprint);
    let bus = match Bus::open(&boot.zenoh_connect, &boot.namespace, &boot.node_id, &epoch).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(kind = "diag", spec = "CAND-BUS-NO-ZENOH-BOOT", "zenoh connect {}: {e}", boot.zenoh_connect);
            return 2;
        }
    };
    let grpc_addr = boot.grpc_listen;
    let sfu = Arc::new(Sfu {
        node: boot.node_id.clone(),
        boot,
        epoch: epoch.clone(),
        net: net.clone(),
        bus: bus.clone(),
        world: tokio::sync::Mutex::new(World { rooms: BTreeMap::new(), peers: HashMap::new() }),
        hot: ArcSwap::from_pointee(HashMap::new()),
        peers: ArcSwap::from_pointee(HashMap::new()),
        counters: crate::err::Counters::default(),
        shutting: std::sync::atomic::AtomicBool::new(false),
        me: std::sync::OnceLock::new(),
    });
    let _ = sfu.me.set(Arc::downgrade(&sfu));
    let _ = net.up.set(Arc::new(crate::fwd::Plane(Arc::downgrade(&sfu))));
    tokio::spawn(crate::link::shell::udp_loop(net.clone()));
    if let Some(l) = tcp {
        tokio::spawn(crate::link::shell::tcp_loop(net.clone(), l));
        tokio::spawn(crate::link::shell::keepalive_loop(net.clone()));
    }
    let (home_tx, home_rx) = tokio::sync::mpsc::unbounded_channel();
    if let Err(e) = bus.watch_nodes(home_tx).await {
        tracing::error!(kind = "diag", "node watch: {e}");
        return 2;
    }
    tokio::spawn(crate::gc::home_loop(sfu.clone(), home_rx));
    tokio::spawn(crate::gc::sweep_loop(sfu.clone()));
    crate::fwd::shell::spawn_loops(&sfu);
    let beat = common::lifeline::Beat::default();
    {
        let b = beat.clone();
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_millis(common::lifeline::BEAT_MS));
            loop {
                iv.tick().await;
                b.touch();
            }
        });
    }
    common::lifeline::self_watch(beat, |age| {
        tracing::error!(kind = "diag", spec = "RULE-BUS-FATE", stalled_ms = age, "runtime stalled — exiting");
        std::process::exit(common::lifeline::EXIT_STALL);
    });
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    common::lifeline::watch_stdin(move || {
        let _ = stop_tx.send(true);
    });
    let svc = HandleServer::new(Svc(sfu.clone())).max_decoding_message_size(16 * 1024 * 1024).max_encoding_message_size(16 * 1024 * 1024);
    let server = tonic::transport::Server::builder().add_service(svc).serve_with_shutdown(grpc_addr, async move {
        let _ = stop_rx.changed().await;
    });
    tracing::info!(kind = "ev", spec = "RULE-OPS-SURFACE", grpc = %grpc_addr, udp = sfu.net.port, epoch = %sfu.epoch, "ready");
    let code = match server.await {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!(kind = "diag", "grpc: {e}");
            2
        }
    };
    sfu.shutting.store(true, std::sync::atomic::Ordering::Relaxed);
    {
        let w = sfu.world.lock().await;
        for p in w.peers.values() {
            p.close();
        }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    sfu.bus.close().await;
    tracing::info!(kind = "ev", spec = "RULE-BUS-FATE", "parent channel closed — stopped");
    code
}
