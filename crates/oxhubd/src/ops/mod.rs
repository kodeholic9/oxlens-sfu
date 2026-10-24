// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-SURFACE · 정:RULE-OPS-LOGGER · 정:RULE-BUS-FATE · 정:RULE-GC-FOUR · 정:IMPL-BUS-FAULTS · 정:RULE-CFG-THREE-PLACES · 정:CAND-DOC-ORCHESTRATION-ASPECT · 정:CAND-BUS-NO-ZENOH-BOOT · 정:CAND-OPS-SHUTDOWN-NO-RESTART · 운영 규격서 §8 · model: claude-opus-5-5
//! 영역 OPS(게이트웨이) — 기동 순서(로거 → zenoh → sfu) · 백그라운드(창 만료 sweep · 통지 배달 · node 간 질의 · 유닛 사건) · 종료 순서(클라 → 유닛 → hub).

pub mod admin;
pub mod http;
pub mod sup_core;
pub mod supervisor;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::bus::envelope::{NodeQuery, Reply};
use common::system::UnitRole;
use oxsig::Code;
use tokio::sync::{mpsc, Notify};

use crate::bus::{Bus, BusStreams, NodeAsk};
use crate::cfg::Boot;
use crate::ses::{self, Ctx, Out};
use supervisor::{Shared, Supervisor, UnitEvent};

pub const SHUTDOWN_CLIENT_WAIT: Duration = Duration::from_millis(200);
pub const SWEEP_EVERY: Duration = Duration::from_secs(1);

pub struct App {
    pub boot: Boot,
    pub ctx: Arc<Ctx>,
    pub sup: Arc<Supervisor>,
    pub stop: Arc<Notify>,
    pub stopping: AtomicBool,
}

pub fn begin_shutdown(app: Arc<App>, why: &str) {
    if app.stopping.swap(true, Ordering::SeqCst) {
        return;
    }
    tracing::info!(kind = "ev", spec = "CP-CODE-SERVER-SHUTDOWN", reason = why, "shutdown begins");
    app.stop.notify_waiters();
    app.stop.notify_one();
}

fn extra_args(boot: &Boot) -> Vec<String> {
    let mut a = vec!["--id".to_string(), boot.node_id.clone(), "--zenoh-namespace".to_string(), boot.system.zenoh.namespace.clone()];
    if let Some(l) = boot.system.zenoh.listen.first() {
        a.push("--zenoh-connect".into());
        a.push(l.clone());
    }
    if let Some(b) = &boot.label {
        a.push("--build".into());
        a.push(b.clone());
    }
    if let Some(d) = &boot.log_dir {
        a.push("--log-dir".into());
        a.push(d.display().to_string());
    }
    if let Some(s) = &boot.log_socket {
        a.push("--log-socket".into());
        a.push(s.display().to_string());
    }
    a
}

pub async fn run(boot: Boot) -> i32 {
    let hub_inst = format!("hub-{}", common::identity::fresh_hex(6));
    tracing::info!(kind = "ev", spec = "RULE-CFG-THREE-PLACES", epoch = %hub_inst, build = %boot.build, "{}", boot.banner());
    let shutting = Arc::new(AtomicBool::new(false));
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let sv = &boot.system.supervisor;
    let shared = Shared {
        node: boot.node_id.clone(),
        hub_inst: hub_inst.clone(),
        extra: extra_args(&boot),
        limits: sup_core::Limits { backoff_ms: sv.backoff_ms.clone(), intensity_max: sv.intensity_max, window_ms: sv.intensity_window_secs * 1000 },
        hello_timeout: Duration::from_millis(sv.hello_timeout_ms),
        shutting: shutting.clone(),
        events: ev_tx,
    };
    let sup = Arc::new(Supervisor::new(&boot.system.unit, shared));
    sup.boot(UnitRole::Log);
    sup.wait_running(UnitRole::Log, Duration::from_millis(sv.hello_timeout_ms)).await;
    sup.set_zenoh(sup_core::UnitState::Starting);
    let (bus, streams) = match Bus::open(&boot.node_id, &boot.system.zenoh, sv.node_grace_secs).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(kind = "ev", spec = "RULE-BUS-FATE", "zenoh router failed to open: {e}");
            sup.set_zenoh(sup_core::UnitState::Blocked);
            sup.stop_role(UnitRole::Log).await;
            return 3;
        }
    };
    sup.set_zenoh(sup_core::UnitState::Running);
    tracing::info!(kind = "ev", spec = "RULE-BUS-SHAPE", node = %boot.node_id, namespace = %boot.system.zenoh.namespace, "zenoh router open");
    let mut secrets: Vec<String> = boot.system.hub.auth.api_keys.iter().map(|a| a.secret.clone()).collect();
    if let Some(j) = &boot.system.hub.auth.jwt_secret {
        secrets.push(j.clone());
    }
    let policy = Arc::new(boot.policy.clone());
    let ctx = Arc::new(Ctx {
        node: boot.node_id.clone(),
        bus: bus.clone(),
        policy: policy.clone(),
        secrets,
        sessions: Arc::new(ses::Sessions::default()),
        shutting: shutting.clone(),
        admission: Arc::new(crate::room::Admission::new(policy.hub.join_admit_per_sec, policy.hub.join_retry_after_ms)),
        counters: Arc::new(crate::err::Counters::default()),
        generation: AtomicU64::new(0),
    });
    let app = Arc::new(App { boot, ctx: ctx.clone(), sup: sup.clone(), stop: Arc::new(Notify::new()), stopping: AtomicBool::new(false) });
    spawn_background(app.clone(), streams, ev_rx);
    sup.boot(UnitRole::Sfu);
    let listen: SocketAddr = match app.boot.listen.parse() {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(kind = "diag", "listen {}: {e}", app.boot.listen);
            return 2;
        }
    };
    let server = tokio::spawn(serve_http(app.clone(), listen));
    let a2 = app.clone();
    tokio::spawn(async move {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => return,
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        begin_shutdown(a2, "signal");
    });
    app.stop.notified().await;
    shutdown(&app).await;
    server.abort();
    0
}

async fn serve_http(app: Arc<App>, listen: SocketAddr) {
    let router = http::routes(app.clone());
    let listener = match tokio::net::TcpListener::bind(listen).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(kind = "diag", "bind {listen}: {e}");
            begin_shutdown(app, "listen failed");
            return;
        }
    };
    match &app.boot.system.hub.tls {
        None => {
            let _ = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await;
        }
        Some(t) => serve_tls(listener, router, &t.cert, &t.key).await,
    }
}

async fn serve_tls(listener: tokio::net::TcpListener, router: axum::Router, cert: &str, key: &str) {
    use tokio_rustls::rustls;
    let certs: Vec<_> = match std::fs::File::open(cert) {
        Ok(f) => rustls_pemfile::certs(&mut std::io::BufReader::new(f)).filter_map(Result::ok).collect(),
        Err(e) => {
            tracing::error!(kind = "diag", "tls cert {cert}: {e}");
            return;
        }
    };
    let key = match std::fs::File::open(key).ok().and_then(|f| rustls_pemfile::private_key(&mut std::io::BufReader::new(f)).ok().flatten()) {
        Some(k) => k,
        None => {
            tracing::error!(kind = "diag", "tls key {key}: unreadable");
            return;
        }
    };
    let cfg = match rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(certs, key) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(kind = "diag", "tls config: {e}");
            return;
        }
    };
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    loop {
        let Ok((tcp, peer)) = listener.accept().await else { continue };
        let acceptor = acceptor.clone();
        let svc = router.clone().layer(axum::Extension(axum::extract::ConnectInfo(peer)));
        tokio::spawn(async move {
            let Ok(tls) = acceptor.accept(tcp).await else { return };
            let io = hyper_util::rt::TokioIo::new(tls);
            let hyper_svc = hyper_util::service::TowerToHyperService::new(svc);
            let _ = hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new()).serve_connection_with_upgrades(io, hyper_svc).await;
        });
    }
}

fn spawn_background(app: Arc<App>, streams: BusStreams, mut events: mpsc::UnboundedReceiver<UnitEvent>) {
    let BusStreams { mut inbound, mut asks, mut room_changes, mut node_changes } = streams;
    let a = app.clone();
    tokio::spawn(async move {
        let mut t = tokio::time::interval(SWEEP_EVERY);
        loop {
            t.tick().await;
            ses::sweep_windows(&a.ctx).await;
        }
    });
    let a = app.clone();
    tokio::spawn(async move {
        while let Some(m) = inbound.recv().await {
            ses::deliver(&a.ctx, &m.to, &m.notice);
            if let crate::bus::Delivery::User(u) = &m.to {
                ses::route::on_room_event(&a.ctx, u, &m.notice).await;
            }
        }
    });
    let a = app.clone();
    tokio::spawn(async move {
        while let Some(ask) = asks.recv().await {
            let a = a.clone();
            tokio::spawn(async move { answer(&a, ask).await });
        }
    });
    tokio::spawn(async move { while room_changes.recv().await.is_some() {} });
    tokio::spawn(async move {
        while let Some((node, inst, up)) = node_changes.recv().await {
            tracing::info!(kind = "ev", spec = "RULE-BUS-PRESENCE", node = %node, inst = %inst, up, "node token");
        }
    });
    let a = app.clone();
    tokio::spawn(async move {
        while let Some(e) = events.recv().await {
            match e {
                UnitEvent::Ready { id, role: UnitRole::Sfu, epoch, client } => {
                    if a.sup.of_role(UnitRole::Sfu).first().map(|u| u.cfg.id == id).unwrap_or(false) {
                        a.ctx.bus.set_local(client.map(|b| *b));
                        a.ctx.bus.set_node_inst(Some(&epoch)).await;
                    }
                }
                UnitEvent::Down { id, role: UnitRole::Sfu } => {
                    if a.sup.of_role(UnitRole::Sfu).first().map(|u| u.cfg.id == id).unwrap_or(false) {
                        a.ctx.bus.set_local(None);
                    }
                }
                _ => {}
            }
        }
    });
}

async fn answer(app: &Arc<App>, ask: NodeAsk) {
    let bytes = match ask.query {
        NodeQuery::Call { call, .. } => app.ctx.bus.call_local(&call).await.encode(),
        NodeQuery::Session { sid } => match ses::route::hand_over(&app.ctx, &sid).await {
            Some(w) => serde_json::to_vec(&w).unwrap_or_default(),
            None => Reply::fail(Code::SessionNotFound).encode(),
        },
        NodeQuery::Evict { user_id, keep_sid } => serde_json::to_vec(&ses::route::evict_local(&app.ctx, &user_id, &keep_sid).await).unwrap_or_default(),
        NodeQuery::Cut { user_id } => serde_json::to_vec(&ses::route::cut_local(&app.ctx, &user_id).await).unwrap_or_default(),
        NodeQuery::Bus => serde_json::to_vec(&app.ctx.bus.info().await).unwrap_or_default(),
        NodeQuery::Rooms => app.ctx.bus.call_local(&common::bus::envelope::Call::Admin { what: common::bus::envelope::Admin::Rooms }).await.encode(),
    };
    let _ = ask.reply.send(bytes);
}

async fn shutdown(app: &Arc<App>) {
    app.ctx.shutting.store(true, Ordering::SeqCst);
    supervisor::shutting_mark(&app.sup);
    for s in app.ctx.sessions.all() {
        s.send(Out::Leave(Code::ServerShutdown));
        ses::destroy(&app.ctx, &s, common::bus::envelope::GoneCause::Shutdown).await;
    }
    tokio::time::sleep(SHUTDOWN_CLIENT_WAIT).await;
    app.sup.stop_role(UnitRole::Sfu).await;
    app.ctx.bus.close().await;
    app.sup.set_zenoh(sup_core::UnitState::Stopped);
    app.ctx.bus.shutdown_session().await;
    app.sup.stop_role(UnitRole::Log).await;
    tracing::info!(kind = "ev", spec = "RULE-BUS-FATE", "hub exits");
    tokio::time::sleep(Duration::from_millis(100)).await;
}
