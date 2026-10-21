// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-DOC-PLANES 정:RULE-CFG-THREE-PLACES 정:RULE-BUS-FATE 정:RULE-OPS-SURFACE 운영 규격서 §8 · 연:RULE-TRN-ENDPOINTS · model: claude-opus-5-5
//! oxhubd — 게이트웨이. 세션 · 배치 · op 라우팅 · 운영 표면 · supervisor · 그 node 의 zenoh router.

mod admin;
mod auth;
mod bus;
mod conn;
mod http;
mod route;
mod session;
mod state;
mod supervisor;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Router;
use axum::routing::{get, post};
use common::policy::Policy;
use common::system::System;

use crate::state::Hub;
use crate::supervisor::{StartCtx, UnitState};

#[derive(Default)]
struct Args {
    system: Option<String>,
    policy: Option<String>,
    id: Option<String>,
    build: Option<String>,
    version: bool,
    listen: Option<String>,
    log_dir: Option<String>,
    log_socket: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{k} 에 값이 없다"));
        match k.as_str() {
            "--system" => a.system = Some(val()?),
            "--policy" => a.policy = Some(val()?),
            "--id" => a.id = Some(val()?),
            "--build" => a.build = Some(val()?),
            "--listen" => a.listen = Some(val()?),
            "--log-dir" => a.log_dir = Some(val()?),
            "--log-socket" => a.log_socket = Some(val()?),
            "--version" => a.version = true,
            other => return Err(format!("모르는 인자: {other}")),
        }
    }
    Ok(a)
}

fn identity(label: Option<&str>) -> String {
    common::ident::build_identity(env!("OX_GIT_SHA"), env!("OX_BUILD_TIME"), label)
}

pub async fn shutdown(hub: &Arc<Hub>, code: i32) {
    if hub.shutting_down.swap(true, Ordering::SeqCst) {
        return;
    }
    tracing::info!(kind = "ev", spec = "CP-CODE-SERVER-SHUTDOWN", "shutdown: clients → units → hub");
    session::shutdown_all(hub).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    hub.units.shutdown_units().await;
    if let Some(b) = hub.bus() {
        let _ = b.close().await;
    }
    std::process::exit(code);
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("oxhubd: {e}");
            std::process::exit(2);
        }
    };
    if args.version {
        println!("{}", identity(args.build.as_deref()));
        return;
    }
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("oxhubd: runtime: {e}");
            std::process::exit(1);
        }
    };
    let code = rt.block_on(run(args));
    std::process::exit(code);
}

async fn run(args: Args) -> i32 {
    let Some(sys_path) = args.system.clone() else {
        eprintln!("oxhubd: --system 이 필요하다");
        return 2;
    };
    let mut system = match System::load(std::path::Path::new(&sys_path)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("oxhubd: {e}");
            return 2;
        }
    };
    let policy = match args.policy.as_deref() {
        Some(p) => match Policy::load(std::path::Path::new(p)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("oxhubd: {e}");
                return 2;
            }
        },
        None => Policy::default(),
    };
    if let Some(l) = &args.listen {
        system.hub.listen = l.clone();
    }
    if let Some(d) = &args.log_dir {
        system.hub.log_dir = Some(d.clone());
    }
    if let Some(s) = &args.log_socket {
        system.hub.log_socket = Some(s.clone());
    }
    let node = match args.id.clone().or_else(|| system.first_sfu().map(|u| u.id.clone())) {
        Some(n) if common::keyspace::valid_piece(&n) => n,
        _ => {
            eprintln!("oxhubd: --id(node_id) 가 없거나 키 금지 문자를 담았다");
            return 2;
        }
    };
    let build = identity(args.build.as_deref());
    common::log::init(common::log::LogConfig {
        node: &node,
        proc_name: "hub",
        logging: &policy.logging,
        log_dir: system.hub.log_dir.as_deref(),
        log_socket: system.hub.log_socket.as_deref(),
    });
    tracing::info!(
        kind = "diag",
        "banner node={node} build={build} listen={} base={} units={} zenoh.listen={:?} zenoh.connect={:?} namespace={} policy={:?}",
        system.hub.listen,
        system.hub.base_path,
        system.unit.iter().map(|u| u.id.as_str()).collect::<Vec<_>>().join(","),
        system.zenoh.listen,
        system.zenoh.connect,
        system.zenoh.namespace,
        policy
    );
    let hub = Arc::new(Hub::new(node.clone(), build, system, policy));
    if let Err(e) = bus::open(&hub).await {
        tracing::warn!(kind = "ev", spec = "SV-STATE-BLOCKED", "zenoh unit failed: {e}");
        hub.units.set_state(supervisor::ZENOH_UNIT, UnitState::Blocked);
        return 1;
    }
    hub.units.set_state(supervisor::ZENOH_UNIT, UnitState::Running);
    let ctx = Arc::new(StartCtx {
        node: node.clone(),
        build_label: args.build.clone(),
        zenoh_namespace: hub.system.zenoh.namespace.clone(),
        zenoh_connect: hub.system.zenoh.listen.first().cloned(),
        log_dir: hub.system.hub.log_dir.clone(),
        log_socket: hub.system.hub.log_socket.clone(),
    });
    let hub_cb = hub.clone();
    let on_running: Arc<dyn Fn(String, String) + Send + Sync> = Arc::new(move |id: String, epoch: String| {
        let hub = hub_cb.clone();
        tokio::spawn(async move {
            if hub.units.sfu_id().as_deref() == Some(id.as_str()) {
                bus::declare_node(&hub, &epoch).await;
            }
        });
    });
    hub.units.start(ctx, on_running);
    {
        let hub = hub.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                common::log::restore_if_due();
                if hub.is_shutting_down() {
                    break;
                }
            }
        });
    }
    let base = hub.system.hub.base_path.trim_end_matches('/').to_string();
    let app = Router::new()
        .route(&format!("{base}/ws"), get(http::ws))
        .route(&format!("{base}/auth/token"), post(http::token))
        .route(&format!("{base}/rooms"), get(http::list_rooms).post(http::create_room).options(http::preflight))
        .route(&format!("{base}/rooms/:room_id"), get(http::get_room).options(http::preflight))
        .route("/healthz/live", get(http::live))
        .route("/healthz/ready", get(http::ready))
        .route("/admin/sfus", get(admin::sfus))
        .route("/admin/sfus/:sfu_id/rooms", get(admin::sfu_rooms))
        .route("/admin/rooms", get(admin::rooms))
        .route("/admin/rooms/:room_id/snapshot", get(admin::room_snapshot))
        .route("/admin/rooms/:room_id/reap/:user_id", post(admin::reap))
        .route("/admin/rooms/:room_id/destroy", post(admin::destroy))
        .route("/admin/users", get(admin::users))
        .route("/admin/users/:user_id/cut", post(admin::user_cut))
        .route("/admin/snapshot", get(admin::snapshot))
        .route("/admin/drops", get(admin::drops))
        .route("/admin/bus", get(admin::bus))
        .route("/admin/supervisor/status", get(admin::sup_status))
        .route("/admin/supervisor/load/:id", post(admin::sup_load))
        .route("/admin/supervisor/stop/:id", post(admin::sup_stop))
        .route("/admin/supervisor/kill/:id", post(admin::sup_kill))
        .route("/admin/supervisor/shutdown", post(admin::sup_shutdown))
        .route("/admin/log-level", post(admin::log_level))
        .with_state(hub.clone());
    let listener = match tokio::net::TcpListener::bind(&hub.system.hub.listen).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(kind = "diag", "listen {} failed: {e}", hub.system.hub.listen);
            return 1;
        }
    };
    {
        let hub = hub.clone();
        tokio::spawn(async move {
            let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(_) => return,
            };
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            shutdown(&hub, 0).await;
        });
    }
    let svc = app.into_make_service_with_connect_info::<SocketAddr>();
    if let Err(e) = axum::serve(listener, svc).await {
        tracing::warn!(kind = "diag", "serve failed: {e}");
        return 1;
    }
    0
}
