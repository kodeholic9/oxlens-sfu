// author: kodeholic (powered by Claude)
//! 기동 — 설정 두 파일을 1회 로드(정§18-1)하고 `{base}/ws`·`{base}/auth/token`·`{base}/healthz` 를 연다.
//! 배너 한 줄이 해석된 최종값 전량이다 — 인자가 권위라 파일에 안 남는다.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::routing::{get, post};
use axum::Router;
use common::config::{PolicyConfig, SystemConfig};
use oxhubd::backend::NoBackend;
use oxhubd::rest::{self, RestState};
use oxhubd::session::SessionRegistry;
use oxhubd::ws::{self, Hub};
use tracing::info;

struct Args {
    system: PathBuf,
    policy: PathBuf,
}

fn parse_args() -> Args {
    let mut a = Args { system: "system.toml".into(), policy: "policy.toml".into() };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--system" => a.system = it.next().unwrap_or_default().into(),
            "--policy" => a.policy = it.next().unwrap_or_default().into(),
            _ => {
                eprintln!("usage: oxhubd [--system system.toml] [--policy policy.toml]");
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
    info!(listen = %system.hub.listen, base = %system.hub.base_path, units = system.units.len(),
        heartbeat_ms = policy.hub.heartbeat_interval_ms, idle_ms = policy.hub.heartbeat_timeout_ms,
        resume_window_ms = policy.hub.resume_window_ms, flow_window = policy.hub.ws_flow_window,
        token_ttl_s = policy.hub.token_ttl_secs, "oxhubd up");

    let registry = Arc::new(SessionRegistry::new(
        system.hub.auth.jwt_secret.clone(),
        Duration::from_millis(u64::from(policy.hub.resume_window_ms)),
        u64::from(policy.hub.heartbeat_interval_ms),
    ));
    let hub = Arc::new(Hub::new(
        registry.clone(),
        Arc::new(NoBackend),
        usize::from(policy.hub.ws_flow_window),
        Duration::from_millis(u64::from(policy.hub.heartbeat_timeout_ms)),
    ));
    let reaper = registry.clone();
    tokio::spawn(async move {
        let mut t = tokio::time::interval(Duration::from_secs(5));
        loop {
            t.tick().await;
            let dead = reaper.reap(Instant::now());
            if !dead.is_empty() {
                info!(count = dead.len(), "sessions reaped past resume window");
            }
        }
    });

    let listen = system.hub.listen.clone();
    let base = system.hub.base_path.clone();
    let rest_state = Arc::new(RestState { system, policy });
    let inner = Router::new()
        .route("/ws", get(ws::upgrade).with_state(hub))
        .route("/auth/token", post(rest::token).with_state(rest_state))
        .route("/healthz", get(rest::healthz));
    let app = if base.is_empty() { inner } else { Router::new().nest(&base, inner) };
    let listener = tokio::net::TcpListener::bind(&listen).await.unwrap_or_else(|e| {
        eprintln!("bind {listen}: {e}");
        std::process::exit(2);
    });
    axum::serve(listener, app).await.unwrap_or_else(|e| eprintln!("serve: {e}"));
}
