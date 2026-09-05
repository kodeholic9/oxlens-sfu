// author: kodeholic (powered by Claude)
//! 기동 — 설정 두 파일을 1회 로드(정§18-1)하고 `{base}/ws`·`{base}/auth/token`·`{base}/rooms`·`{base}/healthz` 를 연다.
//! 배너 한 줄이 해석된 최종값 전량이다 — 인자가 권위라 파일에 안 남는다.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::routing::{get, post};
use axum::Router;
use common::config::{PolicyConfig, SystemConfig};
use oxhubd::backend::{Members, SfuBackend};
use oxhubd::events::{self, EventCtx};
use oxhubd::nodes::NodeTable;
use oxhubd::rest::{self, RestState};
use oxhubd::route::RoomMap;
use oxhubd::session::SessionRegistry;
use oxhubd::ws::{self, Hub};
use tracing::info;

struct Args {
    system: PathBuf,
    policy: PathBuf,
    id: String,
}

fn parse_args() -> Args {
    let mut a = Args { system: "system.toml".into(), policy: "policy.toml".into(), id: "hub-1".into() };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--system" => a.system = it.next().unwrap_or_default().into(),
            "--policy" => a.policy = it.next().unwrap_or_default().into(),
            "--id" => a.id = it.next().unwrap_or_default(),
            _ => {
                eprintln!("usage: oxhubd [--system system.toml] [--policy policy.toml] [--id hub-1]");
                std::process::exit(2);
            }
        }
    }
    a
}

#[tokio::main]
async fn main() {
    let args = parse_args();
    let system = SystemConfig::load(&args.system).unwrap_or_else(|e| {
        eprintln!("system config: {e}");
        std::process::exit(2);
    });
    let policy = PolicyConfig::load(&args.policy).unwrap_or_else(|e| {
        eprintln!("policy config: {e}");
        std::process::exit(2);
    });
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::new(policy.logging.level.clone())).init();
    let sfu_units: Vec<(String, String)> = system.units.iter().filter(|u| u.role == "sfu").map(|u| (u.id.clone(), u.addr.clone())).collect();
    info!(id = %args.id, listen = %system.hub.listen, base = %system.hub.base_path, sfu_nodes = ?sfu_units,
        heartbeat_ms = policy.hub.heartbeat_interval_ms, idle_ms = policy.hub.heartbeat_timeout_ms,
        resume_window_ms = policy.hub.resume_window_ms, flow_window = policy.hub.ws_flow_window,
        token_ttl_s = policy.hub.token_ttl_secs, capacity = policy.hub.room_default_capacity, "oxhubd up");

    let registry = Arc::new(SessionRegistry::new(
        system.hub.auth.jwt_secret.clone(),
        Duration::from_millis(u64::from(policy.hub.resume_window_ms)),
        u64::from(policy.hub.heartbeat_interval_ms),
    ));
    let nodes = Arc::new(NodeTable::new(sfu_units));
    let rooms = Arc::new(RoomMap::default());
    let members = Arc::new(Members::default());
    let backend = Arc::new(SfuBackend { nodes: nodes.clone(), rooms: rooms.clone(), members: members.clone() });
    let hub = Arc::new(Hub::new(
        registry.clone(),
        backend.clone(),
        usize::from(policy.hub.ws_flow_window),
        Duration::from_millis(u64::from(policy.hub.heartbeat_timeout_ms)),
    ));
    let ctx = Arc::new(EventCtx { hub: hub.clone(), rooms, members: members.clone(), hub_id: args.id.clone() });
    for node in nodes.all() {
        tokio::spawn(events::run_consumer(ctx.clone(), node.clone()));
    }
    let reaper = registry.clone();
    let reaper_members = members;
    tokio::spawn(async move {
        let mut t = tokio::time::interval(Duration::from_secs(5));
        loop {
            t.tick().await;
            let dead = reaper.reap(Instant::now());
            for s in &dead {
                reaper_members.forget_user(&s.user_id);
            }
            if !dead.is_empty() {
                info!(count = dead.len(), "sessions reaped past resume window");
            }
        }
    });

    let listen = system.hub.listen.clone();
    let base = system.hub.base_path.clone();
    let rest_state = Arc::new(RestState { system, policy, registry, backend });
    let inner = Router::new()
        .route("/ws", get(ws::upgrade).with_state(hub))
        .route("/auth/token", post(rest::token))
        .route("/rooms", get(rest::list_rooms).post(rest::create_room))
        .route("/rooms/:room_id", get(rest::get_room))
        .route("/healthz", get(rest::healthz))
        .with_state(rest_state);
    let app = if base.is_empty() { inner } else { Router::new().nest(&base, inner) };
    let listener = tokio::net::TcpListener::bind(&listen).await.unwrap_or_else(|e| {
        eprintln!("bind {listen}: {e}");
        std::process::exit(2);
    });
    axum::serve(listener, app).await.unwrap_or_else(|e| eprintln!("serve: {e}"));
}
