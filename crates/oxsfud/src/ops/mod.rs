// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-OPS-SURFACE · 정:CAND-OPS-SERVER-TRANSITION-SPEC · 정:RULE-OPS-OBSERVE · 정:RULE-OPS-DROPS · 정:RULE-BUS-SHAPE · 정:RULE-BUS-FATE · 정:CAND-BUS-HANDLE-SHAPE · 정:CAND-BUS-SELF-WATCH-VALUES · 정:RULE-CONC-SIX · 정:CAND-DOC-ORCHESTRATION-ASPECT · 정:IMPL-LINK-PEER-STATE · 정:RULE-GC-FOUR · 운영:RULE-LOG-TRANSITION · 운영:RULE-LOG-CONTEXT · model: claude-opus-5-5
//! 운영 — 미디어 서버 한 프로세스의 조율. 상태(방 · Peer)는 락 하나가 지키고(판정은 락 안) · 통지는 락 안에서 버스 한 줄에 줄 세우고 · 송신 · 미디어 결합은 락 밖에서 한다. gRPC Handle(`Hello` · `Call`) · 주기(RTCP · 자동 판단 · 쌍별 생존 · 방 sweep · 정체 · stat 줄) · 부모 생존 채널 · 자가 감시.

pub mod admin;
pub mod call;
pub mod drops;
pub mod floor;
pub mod peer;

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use common::bus::env::{Call, Notice, Reply};
use oxsig::op::Op;
use tokio::sync::{Notify, mpsc};

use crate::bus::Bus;
use crate::cfg::SfuCfg;
use crate::link::{DcEvent, Io, LinkSignal, PeerState};
use crate::r#pub::{Stream, Uplink};
use crate::trk::Room;

pub use peer::Peer;

pub struct State {
    pub rooms: BTreeMap<String, Room>,
    pub peers: BTreeMap<String, Peer>,
    pub arev: BTreeMap<String, u64>,
}

pub struct Sfu {
    pub cfg: SfuCfg,
    pub epoch: String,
    pub io: Arc<Io>,
    pub bus: Arc<Bus>,
    pub state: Mutex<State>,
    pub floor_wake: Mutex<HashMap<String, Arc<Notify>>>,
    pub udp_addr: SocketAddr,
}

pub enum Signal {
    FirstRtp(Arc<Stream>, Arc<Uplink>),
}

static SIG: OnceLock<mpsc::UnboundedSender<Signal>> = OnceLock::new();

pub fn first_rtp(s: &Arc<Stream>, u: &Arc<Uplink>) {
    if let Some(tx) = SIG.get() {
        let _ = tx.send(Signal::FirstRtp(s.clone(), u.clone()));
    }
}

pub fn now_ms() -> u64 {
    common::time::mono_ms()
}

impl Sfu {
    pub fn notice_room(&self, room: &str, op: Op, body: serde_json::Value, exclude: Vec<String>) {
        let n = Notice { op: op.code(), body, exclude, room_id: Some(room.to_string()), sfu_id: Some(self.epoch.clone()) };
        self.bus.put_room(room, n.encode());
    }

    pub fn notice_user(&self, user: &str, op: Op, body: serde_json::Value) {
        let n = Notice { op: op.code(), body, exclude: Vec::new(), room_id: None, sfu_id: Some(self.epoch.clone()) };
        self.bus.put_user(user, n.encode());
    }

    pub fn mline_event(&self, p: &Peer) {
        let body = serde_json::to_value(oxsig::msg::MlineEventBody { table: p.table.wire() }).unwrap_or_default();
        common::ev!(spec = "CP-OP-MLINE-EVENT", user = %p.user, arev = p.table.arev, "배정표");
        self.notice_user(&p.user, Op::MlineEvent, body);
    }
}

pub struct Handle {
    pub sfu: Arc<Sfu>,
}

#[tonic::async_trait]
impl common::grpc::Handle for Handle {
    async fn hello(&self, req: tonic::Request<common::grpc::HelloReq>) -> Result<tonic::Response<common::grpc::HelloRes>, tonic::Status> {
        let r = req.into_inner();
        tracing::info!(peer_node = %r.node, hub_inst = %r.hub_inst, "Hello");
        Ok(tonic::Response::new(common::grpc::HelloRes { epoch: self.sfu.epoch.clone(), build: self.sfu.cfg.build.clone() }))
    }

    async fn call(&self, req: tonic::Request<common::grpc::CallReq>) -> Result<tonic::Response<common::grpc::CallRes>, tonic::Status> {
        let env = req.into_inner().envelope;
        let reply = match Call::decode(&env) {
            Ok(c) => call::handle(&self.sfu, c).await,
            Err(_) => Reply::fail(oxsig::code::Code::INVALID_PAYLOAD),
        };
        if let Reply::Fail { failure } = &reply {
            drops::DROPS.code(failure.code);
        }
        Ok(tonic::Response::new(common::grpc::CallRes { reply: reply.encode() }))
    }
}

async fn on_dc(sfu: Arc<Sfu>, mut rx: mpsc::UnboundedReceiver<DcEvent>) {
    while let Some(ev) = rx.recv().await {
        match ev {
            DcEvent::Open(t) => {
                t.dc_opened();
                tracing::info!(user = &*t.user, transport = t.id, "DC 열림");
            }
            DcEvent::Message(t, b) => floor::on_dc_frame(&sfu, &t, &b).await,
        }
    }
}

async fn on_link(sfu: Arc<Sfu>, mut rx: mpsc::UnboundedReceiver<LinkSignal>) {
    while let Some(s) = rx.recv().await {
        match s {
            LinkSignal::Alive(t, dwell) => {
                common::ev!(spec = common::log::SPEC_PEER_STATE, area = "LINK", key = t.id, from = "Suspect", to = "Alive", cause = "USE-CANDIDATE", user = &*t.user, dwell_ms = dwell);
            }
            LinkSignal::Promoted(t, old) => {
                common::ev!(spec = "LINK.S.checked_new", area = "LINK", key = t.id, from = "pending", to = "use", cause = "STUN", user = &*t.user, in.g = %old);
            }
        }
        let _ = &sfu;
    }
}

async fn on_signal(sfu: Arc<Sfu>, mut rx: mpsc::UnboundedReceiver<Signal>) {
    while let Some(s) = rx.recv().await {
        match s {
            Signal::FirstRtp(stream, up) => call::on_first_rtp(&sfu, &stream, &up),
        }
    }
}

fn liveness(sfu: &Sfu, now: u64) {
    for t in sfu.io.transports() {
        let (had, alive) = t.pairs.lock().map(|p| (p.path().is_some(), p.path_alive(now))).unwrap_or((false, true));
        if had && !alive && t.state() == PeerState::Alive {
            if t.suspect_since.compare_exchange(0, now.max(1), Ordering::AcqRel, Ordering::Acquire).is_err() {
                continue;
            }
            if t.swap_state(PeerState::Suspect) == PeerState::Suspect {
                continue;
            }
            let ufrag = t.ufrag();
            common::ev!(spec = common::log::SPEC_PEER_STATE, area = "LINK", key = t.id, from = "Alive", to = "Suspect", cause = "pair_expired", user = &*t.user);
            common::ev!(spec = "LINK.S.suspect", area = "LINK", key = t.id, from = "Alive", to = "Suspect", cause = "SSuspect", user = &*t.user);
            let body = serde_json::to_value(oxsig::msg::LinkEventBody { type_: oxsig::msg::LinkEventBodyType::Suspect, sfu_id: sfu.epoch.clone(), ufrag }).unwrap_or_default();
            sfu.notice_user(&t.user, Op::LinkEvent, body);
        }
    }
}

fn stat_lines(sfu: &Sfu) {
    let st = match sfu.state.lock() {
        Ok(s) => s,
        Err(_) => return,
    };
    for p in st.peers.values() {
        for s in p.streams.values() {
            for u in s.uplinks.load().iter().filter(|u| !u.rtx) {
                let (pk, by, lost, jit) = u.ingress.lock().map(|i| (i.packets, i.bytes, i.stats.lost(), i.stats.jitter_secs())).unwrap_or((0, 0, 0, 0.0));
                let nack = u.nacks_sent.load(Ordering::Relaxed);
                let pli = u.plis_sent.load(Ordering::Relaxed);
                match u.layer {
                    Some(l) => common::stat!(user = %p.user, track = &*s.track_id, ssrc = u.ssrc, packets_received = pk, packets_lost = lost, jitter = jit, bytes_received = by, nack_count = nack, pli_count = pli, rid = if l == 0 { "l" } else { "h" }),
                    None => common::stat!(user = %p.user, track = &*s.track_id, ssrc = u.ssrc, packets_received = pk, packets_lost = lost, jitter = jit, bytes_received = by, nack_count = nack, pli_count = pli),
                }
            }
        }
        let (mut pk, mut by, mut rtx, mut nack, mut pli) = (0u64, 0u64, 0u64, 0u64, 0u64);
        for r in p.table.rows() {
            pk += r.dl.egress_pkts.load(Ordering::Relaxed);
            by += r.dl.egress_bytes.load(Ordering::Relaxed);
            rtx += r.dl.rtx_sent.load(Ordering::Relaxed);
            nack += r.dl.nacks_recv.load(Ordering::Relaxed);
            pli += r.dl.plis_recv.load(Ordering::Relaxed);
        }
        if !p.table.rows().is_empty() {
            common::stat!(user = %p.user, packets_sent = pk, bytes_sent = by, retransmitted_packets_sent = rtx, nack_count = nack, pli_count = pli);
        }
    }
}

fn spawn_timers(sfu: &Arc<Sfu>) {
    let s = sfu.clone();
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(crate::sim::core::TICK_MS));
        loop {
            iv.tick().await;
            liveness(&s, now_ms());
        }
    });
    let s = sfu.clone();
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(crate::room::SWEEP_MS));
        loop {
            iv.tick().await;
            crate::gc::sweep_rooms(&s).await;
            crate::gc::stall(&s, now_ms());
        }
    });
    let s = sfu.clone();
    tokio::spawn(async move {
        let ms = u64::from(s.cfg.policy.logging.stat_interval_ms.max(100));
        let mut iv = tokio::time::interval(Duration::from_millis(ms));
        loop {
            iv.tick().await;
            stat_lines(&s);
        }
    });
}

pub async fn run(cfg: SfuCfg, epoch: String) -> Result<(), String> {
    let cert = crate::link::cert::generate()?;
    let udp_addr = SocketAddr::new(cfg.bind_ip, cfg.udp_port);
    let udp = crate::link::io::bind_udp(udp_addr, cfg.media_workers).map_err(|e| format!("UDP {udp_addr}: {e}"))?;
    let tcp = match cfg.tcp_port {
        Some(p) => Some(std::net::TcpListener::bind(SocketAddr::new(cfg.bind_ip, p)).map_err(|e| format!("TCP {p}: {e}"))?),
        None => None,
    };
    let (dc_tx, dc_rx) = mpsc::unbounded_channel();
    let (link_tx, link_rx) = mpsc::unbounded_channel();
    let (sig_tx, sig_rx) = mpsc::unbounded_channel();
    let _ = SIG.set(sig_tx);
    let io = Arc::new(Io::new(udp, cfg.tcp_port, cert, dc_tx, link_tx));
    let bus = Bus::open(&cfg.zenoh_connect, &cfg.namespace, &cfg.node_id, &epoch).await?;
    let grpc: SocketAddr = cfg.grpc_listen.parse().map_err(|e| format!("--grpc-listen {}: {e}", cfg.grpc_listen))?;
    let sfu = Arc::new(Sfu { cfg, epoch, io: io.clone(), bus, state: Mutex::new(State { rooms: BTreeMap::new(), peers: BTreeMap::new(), arev: BTreeMap::new() }), floor_wake: Mutex::new(HashMap::new()), udp_addr });
    let rt = tokio::runtime::Handle::current();
    let tick = crate::fwd::work::TickCfg { max_bitrate_bps: u64::from(sfu.cfg.policy.media.max_bitrate_bps), auto_layer: sfu.cfg.policy.media.auto_layer };
    crate::fwd::work::start(sfu.cfg.media_workers, tick, rt.clone())?;
    let reader = crate::link::io::reader(&io, tcp).map_err(|e| format!("수신 스레드 준비: {e}"))?;
    crate::fwd::work::spawn_reader(reader, rt)?;
    tokio::spawn(on_dc(sfu.clone(), dc_rx));
    tokio::spawn(on_link(sfu.clone(), link_rx));
    tokio::spawn(on_signal(sfu.clone(), sig_rx));
    spawn_timers(&sfu);
    tokio::spawn(crate::gc::watch_home(sfu.clone()));
    let _beat = common::proc::self_watch("oxsfud");
    let mut life = common::proc::lifeline();
    let svc = common::grpc::HandleServer::new(Handle { sfu: sfu.clone() });
    let server = tonic::transport::Server::builder().add_service(svc).serve_with_shutdown(grpc, async move {
        loop {
            if life.changed().await.is_err() || *life.borrow() {
                break;
            }
        }
    });
    common::log::boot_lines(&sfu.cfg.build, &sfu.epoch, &sfu.cfg.banner())?;
    let r = server.await.map_err(|e| format!("gRPC {grpc}: {e}"));
    tracing::info!("부모 생존 채널 닫힘 — 끝낸다");
    sfu.bus.close().await;
    r
}

pub fn reply_ok<T: serde::Serialize>(v: &T) -> Reply {
    Reply::ok(v)
}

pub fn version_str(v: &oxsig::types::Version) -> String {
    format!("{}:{}", v.epoch, v.seq)
}
