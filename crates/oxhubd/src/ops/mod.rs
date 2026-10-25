// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-OPS-SURFACE 정:RULE-OPS-LOGGER 정:RULE-BUS-FATE 정:RULE-BUS-SHAPE 정:IMPL-BUS-FAULTS 정:RULE-CFG-THREE-PLACES 정:CAND-BUS-NO-ZENOH-BOOT 정:CAND-DOC-ORCHESTRATION-ASPECT 운영:RULE-BOOT-ARGS 운영:RULE-AUTH-THREE 연:DEV-TRN-WS-ORIGIN · model: claude-opus-5-5
//! hub 기동과 종료 — 로거 → zenoh → sfu 차례로 세우고 · 버스 사건 · node 사이 질의를 나눠 주고 · 종료는 ①클라 `LEAVE 5004` ②유닛 ③hub.

pub mod admin;
pub mod hub;
pub mod route;
pub mod shape;
pub mod supervisor;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::extract::{State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use common::log::{LogOpts, Sink};
use common::system::UnitRole;
use oxsig::Code;
use tokio::sync::{Mutex, Notify};

use crate::bus::{Bus, QueryIn};
use crate::cfg::Config;
use crate::ses::bind::HubQ;
use hub::{Hub, RemoteCall};
use supervisor::Supervisor;

static SHUTDOWN: std::sync::OnceLock<Arc<Notify>> = std::sync::OnceLock::new();

pub fn begin_shutdown(hub: &Hub) {
    hub.shutting_down.store(true, Ordering::Relaxed);
    if let Some(n) = SHUTDOWN.get() {
        n.notify_waiters();
        n.notify_one();
    }
}

async fn ws(State(hub): State<Arc<Hub>>, h: HeaderMap, up: WebSocketUpgrade) -> Response {
    let origin = h.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok());
    if !crate::ses::conn::origin_ok(&hub, origin, host) {
        return (StatusCode::FORBIDDEN, "origin").into_response();
    }
    if hub.stopping() {
        return (StatusCode::SERVICE_UNAVAILABLE, "stopping").into_response();
    }
    up.max_message_size(oxsig::frame::WS_MAX_FRAME + 1).on_upgrade(move |sock| crate::ses::conn::run(hub, sock))
}

async fn dispatch(hub: Arc<Hub>, q: QueryIn) {
    let reply: Vec<u8> = match q.what.as_str() {
        "call" => match serde_json::from_slice::<RemoteCall>(&q.body) {
            Ok(rc) => {
                let r = match hub.sup.sfu_client(rc.inst.as_deref()).await {
                    Some(mut c) => common::handle::call(&mut c, &rc.call).await,
                    None => oxsig::envelope::Reply::fail(Code::SfuUnavailable),
                };
                r.encode()
            }
            Err(e) => oxsig::envelope::Reply::failure(Code::InvalidPayload.failure_msg(e.to_string())).encode(),
        },
        "hub" => match serde_json::from_slice::<HubQ>(&q.body) {
            Ok(HubQ::Sess { sid }) => serde_json::to_vec(&crate::ses::bind::hand_out(&hub, &sid).await).unwrap_or_default(),
            Ok(HubQ::Evict { user }) => {
                if let Some(s) = hub.ses.of_user(&user) {
                    crate::ses::destroy(&hub, &s, crate::ses::Gone::Evicted, Some(Code::DuplicateSession)).await;
                }
                b"{}".to_vec()
            }
            Ok(HubQ::Cut { user }) => serde_json::to_vec(&admin::local_cut(&hub, &user).await).unwrap_or_default(),
            Ok(HubQ::Probe { user }) => serde_json::to_vec(&admin::local_probe(&hub, &user).await).unwrap_or_default(),
            Ok(HubQ::Users) => serde_json::to_vec(&admin::local_users(&hub).await).unwrap_or_default(),
            Ok(HubQ::Bus) => serde_json::to_vec(&admin::bus_body(&hub).await).unwrap_or_default(),
            Ok(HubQ::Drops) => serde_json::to_vec(&admin::hub_drops(&hub).await).unwrap_or_default(),
            Err(_) => b"null".to_vec(),
        },
        _ => b"null".to_vec(),
    };
    let key = q.query.key_expr().clone();
    if let Err(e) = q.query.reply(key, reply).await {
        tracing::warn!("query reply: {e}");
    }
}

async fn serve(hub: Arc<Hub>, app: Router) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(&hub.cfg.listen).await.map_err(|e| format!("listen {}: {e}", hub.cfg.listen))?;
    match hub.cfg.sys.hub.tls.clone() {
        None => {
            tokio::spawn(async move {
                let _ = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await;
            });
        }
        Some(tls) => {
            let acceptor = tls_acceptor(&tls.cert, &tls.key)?;
            tokio::spawn(async move {
                loop {
                    let Ok((tcp, peer)) = listener.accept().await else {
                        continue;
                    };
                    let acc = acceptor.clone();
                    let svc = app.clone().layer(axum::Extension(axum::extract::ConnectInfo(peer)));
                    tokio::spawn(async move {
                        let Ok(stream) = acc.accept(tcp).await else {
                            return;
                        };
                        let hs = hyper_util::service::TowerToHyperService::new(svc);
                        let _ = hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                            .serve_connection_with_upgrades(hyper_util::rt::TokioIo::new(stream), hs)
                            .await;
                    });
                }
            });
        }
    }
    Ok(())
}

fn tls_acceptor(cert: &str, key: &str) -> Result<tokio_rustls::TlsAcceptor, String> {
    let mut cr = std::io::BufReader::new(std::fs::File::open(cert).map_err(|e| format!("tls cert {cert}: {e}"))?);
    let certs: Vec<_> = rustls_pemfile::certs(&mut cr).filter_map(Result::ok).collect();
    let mut kr = std::io::BufReader::new(std::fs::File::open(key).map_err(|e| format!("tls key {key}: {e}"))?);
    let k = rustls_pemfile::private_key(&mut kr).map_err(|e| format!("tls key {key}: {e}"))?.ok_or_else(|| format!("tls key {key}: 없음"))?;
    let sc = tokio_rustls::rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(certs, k).map_err(|e| format!("tls: {e}"))?;
    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(sc)))
}

pub async fn run(cfg: Config) -> Result<(), String> {
    let inst = common::id::instance();
    let sink = match (&cfg.log_socket, &cfg.log_dir) {
        (Some(s), Some(d)) => {
            Sink::Socket { path: s.into(), spool: common::log::spool_path(d, &cfg.node_id, "hub"), spool_max: cfg.policy.logging.spool_max_bytes }
        }
        (Some(s), None) => {
            Sink::Socket { path: s.into(), spool: common::log::spool_path("/tmp", &cfg.node_id, "hub"), spool_max: cfg.policy.logging.spool_max_bytes }
        }
        _ => Sink::Stdout,
    };
    let log = common::log::init(LogOpts {
        node: cfg.node_id.clone(),
        proc_: "hub",
        level: common::log::env_level(cfg.policy.logging.level),
        line_max: cfg.policy.logging.line_max_bytes as usize,
        queue_max: cfg.policy.logging.queue_max_lines as usize,
        sink,
    });
    common::ev!(spec = "SV-STATE-UNIT-STATE", build = cfg.build.as_str(), "{} inst={inst} level={}", cfg.banner(), common::log::level_name(log.level()));
    let shutdown = SHUTDOWN.get_or_init(|| Arc::new(Notify::new())).clone();
    let sup = Supervisor::new(&cfg, &inst);
    sup.start_role(Some(UnitRole::Log)).await;
    let grace = Duration::from_secs(cfg.sys.supervisor.node_grace_secs);
    let opened = Bus::open(&cfg.sys.zenoh, &cfg.node_id, &inst, grace).await?;
    sup.zenoh_running().await;
    sup.set_bus(opened.bus.clone());
    let hub = Arc::new(Hub {
        cfg: cfg.clone(),
        inst: inst.clone(),
        log,
        bus: opened.bus.clone(),
        ses: Default::default(),
        sup: sup.clone(),
        aux: Default::default(),
        codes: Mutex::new(Default::default()),
        shutting_down: Default::default(),
        joins: Mutex::new((0, 0)),
        ev_lost: Default::default(),
    });
    let mut events = opened.events;
    let h = hub.clone();
    tokio::spawn(async move {
        while let Some(e) = events.recv().await {
            crate::ses::deliver(&h, e).await;
        }
    });
    let mut queries = opened.queries;
    let h = hub.clone();
    tokio::spawn(async move {
        while let Some(q) = queries.recv().await {
            tokio::spawn(dispatch(h.clone(), q));
        }
    });
    tokio::spawn(crate::ses::sweep(hub.clone()));
    sup.start_role(Some(UnitRole::Sfu)).await;
    let base = cfg.sys.hub.base_path.trim_end_matches('/').to_string();
    let app = Router::new().merge(crate::room::routes(&base)).route(&format!("{base}/ws"), get(ws)).merge(admin::routes(hub.clone())).with_state(hub.clone());
    serve(hub.clone(), app).await?;
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|e| e.to_string())?;
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
        _ = shutdown.notified() => {}
        _ = sup.zenoh_down_wait() => {
            common::ev!(spec = "SV-STATE-UNIT-STATE", "zenoh unit down — the whole node goes down");
        }
    }
    hub.shutting_down.store(true, Ordering::Relaxed);
    common::ev!(spec = "CP-CODE-SERVER-SHUTDOWN", "shutdown ① clients LEAVE 5004");
    for s in hub.ses.all() {
        s.send(crate::ses::Out::Leave(Code::ServerShutdown)).await;
    }
    tokio::time::sleep(common::bus::LEAVE_FLUSH).await;
    for s in hub.ses.all() {
        crate::ses::destroy(&hub, &s, crate::ses::Gone::Shutdown, None).await;
    }
    common::ev!(spec = "SV-STATE-UNIT-STATE", "shutdown ② units");
    sup.shutdown_units().await;
    hub.bus.close().await;
    common::ev!(spec = "SV-STATE-UNIT-STATE", "shutdown ③ hub — logger last");
    tokio::time::sleep(Duration::from_millis(100)).await;
    sup.stop_logger().await;
    Ok(())
}
