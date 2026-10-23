// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-DOC-PLANES · 정:RULE-DOC-OWNERSHIP · 정:RULE-BUS-FATE · 정:RULE-OPS-SURFACE · 정:RULE-DOC-CODE-MIRROR · model: claude-opus-5-5
//! `oxhubd` — 게이트웨이: 세션 · 배치 · op 라우팅 · supervisor · zenoh router. 영역마다 한 디렉터리(bus · ses · room · ops · aux · cfg).

mod aux;
mod bus;
mod cfg;
mod ops;
mod room;
mod ses;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use common::args::Args;

fn exit(msg: &str) -> ! {
    eprintln!("oxhubd: {msg}");
    std::process::exit(2);
}

fn main() {
    let args = Args::parse(std::env::args().skip(1), &cfg::VALUED, &cfg::FLAGS).unwrap_or_else(|e| exit(&e));
    let raw = env!("OX_BUILD_ID");
    if args.flag("version") {
        println!("{}", common::ident::build_identity(raw, args.get("build")));
        return;
    }
    let boot = cfg::Boot::load(&args, raw).unwrap_or_else(|e| exit(&e));
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap_or_else(|e| exit(&e.to_string()));
    rt.block_on(run(boot));
}

async fn run(boot: cfg::Boot) {
    let _ = common::clock::start();
    common::log::init(common::log::LogConf {
        node: boot.node.clone(),
        proc_name: "hub".to_string(),
        socket: boot.log_socket.as_ref().map(PathBuf::from),
        log_dir: boot.log_dir.as_ref().map(PathBuf::from),
        level: format!("{:?}", boot.policy.logging.level).to_lowercase(),
        line_max: boot.policy.logging.line_max_bytes as usize,
        queue_max: boot.policy.logging.queue_max_lines as usize,
        spool_max: boot.policy.logging.spool_max_bytes,
        sink: None,
    });
    if let Some(d) = &boot.log_dir {
        let _ = std::fs::create_dir_all(d);
    }
    tracing::info!(kind = "diag", build = %boot.build, banner = %boot.banner(), "hub boot");
    let grace = boot.sys.supervisor.node_grace_secs * 1000;
    let z = &boot.sys.zenoh;
    let bus_ = bus::Bus::open(&boot.node, &z.namespace, z.listen.clone(), z.connect.clone(), z.multicast_scouting, grace).await;
    let bus_ = match bus_ {
        Ok(b) => b,
        Err(e) => exit(&e),
    };
    let ses_ = ses::Ses::new(bus_.clone(), boot.sys.clone(), boot.policy.clone(), &boot.node);
    let s2 = ses_.clone();
    bus_.set_sink(Arc::new(move |d| s2.deliver(d)));
    let s3 = ses_.clone();
    bus_.set_serve(Arc::new(move |what, payload| {
        let s = s3.clone();
        Box::pin(async move { s.serve(&what, &payload).await })
    }));
    ses_.spawn_sweeper();
    let sup = ops::supervisor::Supervisor::new(boot.sys.clone(), bus_.clone(), boot.child_args());
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let admin = ops::admin::Admin { bus: bus_.clone(), ses: ses_.clone(), sup: sup.clone(), build: boot.build.clone(), node: boot.node.clone(), shutdown: shutdown.clone() };
    let s4 = ses_.clone();
    let rooms = room::Rooms {
        bus: bus_.clone(),
        policy: boot.policy.clone(),
        ident: Arc::new(move |h| ses::http::identify(&s4, h).map(|i| (i.user_id, i.session_id))),
        status: Arc::new(ses::http::status_of),
    };
    let base = boot.sys.hub.base_path.trim_end_matches('/').to_string();
    let a_plane = axum::Router::new().merge(ses::http::router(ses_.clone())).merge(room::router(rooms));
    let app = if base.is_empty() { axum::Router::new().merge(a_plane) } else { axum::Router::new().nest(&base, a_plane) };
    let app = app.merge(ops::admin::router(admin));
    let listener = match tokio::net::TcpListener::bind(&boot.listen).await {
        Ok(l) => l,
        Err(e) => exit(&format!("listen {}: {e}", boot.listen)),
    };
    tracing::info!(kind = "diag", listen = %boot.listen, "hub listening");
    let stop_srv = Arc::new(tokio::sync::Notify::new());
    let server = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown({
        let sd = stop_srv.clone();
        async move { sd.notified().await }
    });
    let sup2 = sup.clone();
    tokio::spawn(async move { sup2.start_all().await });
    let sd = shutdown.clone();
    tokio::spawn(async move {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => return,
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        sd.notify_one();
    });
    let srv = tokio::spawn(async move {
        let _ = server.await;
    });
    shutdown.notified().await;
    tracing::info!(kind = "diag", "shutdown — clients · units · hub");
    sup.begin_close();
    ses_.shutdown().await;
    sup.stop_all().await;
    bus_.close().await;
    stop_srv.notify_one();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), srv).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    std::process::exit(0);
}
