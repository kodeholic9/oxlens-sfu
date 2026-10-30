// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-FATE · 정:RULE-FWD-MEDIA-WORKERS · 정:RULE-OPS-SURFACE · 운영:RULE-BOOT-ARGS · 운영:RULE-BOOT-BANNER · 정:CAND-BUS-SELF-WATCH-VALUES · 정:CAND-BUS-NO-ZENOH-BOOT · model: claude-opus-5-5
//! oxsfud — 기동(로그 → 배너 → 인증서 → 미디어 스레드 → 버스 → 주기 작업 → Handle) · 부모 생존 채널(stdin EOF 면 끝낸다) · 자가 감시(제어 런타임이 10s 멎으면 끝낸다).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use common::log::{Setup, Sink};
use oxsfud::cfg::SfuCfg;
use oxsfud::ops::{self, Sfu};

const BEAT_MS: u64 = 500;
const HANG_MS: u64 = 10_000;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "--version") {
        let label = argv.iter().position(|a| a == "--build").and_then(|i| argv.get(i + 1)).map(|s| s.as_str());
        println!("{}", common::ident::build_id(oxsfud::cfg::compiled_build(), label));
        return;
    }
    let cfg = match SfuCfg::load(argv) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("oxsfud: {e}");
            std::process::exit(2);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(cfg.control_threads).enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("oxsfud: runtime {e}");
            std::process::exit(2);
        }
    };
    let code = rt.block_on(run(cfg));
    rt.shutdown_timeout(Duration::from_millis(500));
    std::process::exit(code);
}

fn parent_watch() {
    std::thread::Builder::new()
        .name("parent-watch".into())
        .spawn(|| {
            use std::io::Read;
            let mut buf = [0u8; 64];
            let mut stdin = std::io::stdin();
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            eprintln!("oxsfud: parent channel closed — exiting");
            std::process::exit(0);
        })
        .ok();
}

fn self_watch(beat: Arc<AtomicU64>) {
    std::thread::Builder::new()
        .name("self-watch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(BEAT_MS));
                let last = beat.load(Ordering::Acquire);
                if last != 0 && common::now_ms().saturating_sub(last) > HANG_MS {
                    eprintln!("oxsfud: control runtime stalled > {HANG_MS}ms — exiting");
                    std::process::exit(70);
                }
            }
        })
        .ok();
}

async fn run(cfg: SfuCfg) -> i32 {
    let epoch = common::ident::new_epoch();
    let build = cfg.build_id();
    let sink = match &cfg.log_socket {
        Some(p) => Sink::Socket(p.clone()),
        None => Sink::Stdout,
    };
    let sender = common::log::init(Setup { node: cfg.node.clone(), proc_name: "sfu", epoch: Some(epoch.clone()), sink, log_dir: cfg.log_dir.clone() }, &cfg.policy);
    let fields = cfg.banner_fields();
    let banner = common::log::banner::Banner {
        head: &sender.head,
        epoch: Some(&epoch),
        build: &build,
        unit: Some(&cfg.node),
        msg: "sfud boot",
        fields: &fields,
        line_max: cfg.policy.logging.line_max_bytes as usize,
    };
    match banner.lines() {
        Ok(ls) => ls.into_iter().for_each(common::log::emit),
        Err(e) => {
            eprintln!("oxsfud: {e}");
            return 2;
        }
    }
    parent_watch();
    let (cert, fingerprint) = match oxsfud::link::dtls::certificate() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("oxsfud: certificate {e}");
            return 1;
        }
    };
    let start = oxsfud::fwd::rt::StartCfg {
        bind: std::net::SocketAddr::new(cfg.bind_ip, cfg.udp_port),
        advertised: std::net::SocketAddr::new(cfg.ip, cfg.udp_port),
        tcp_port: cfg.tcp_port,
        workers: cfg.workers,
        queue_limit: cfg.queue_packets,
    };
    if let Err(e) = oxsfud::fwd::rt::start(start) {
        eprintln!("oxsfud: media {e}");
        return 1;
    }
    let bus = match oxsfud::bus::Bus::open(&cfg.node, &epoch, &cfg.namespace, &cfg.connect).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("oxsfud: bus {e}");
            return 1;
        }
    };
    ops::install(Sfu {
        node: cfg.node.clone(),
        epoch: epoch.clone(),
        build: build.clone(),
        policy: cfg.policy.clone(),
        ip: cfg.ip,
        udp_port: cfg.udp_port,
        tcp_port: cfg.tcp_port,
        cert,
        fingerprint,
        node_grace_ms: cfg.grace_secs * 1000,
        peers: Default::default(),
        rooms: Default::default(),
        pubs: Default::default(),
        tables: Default::default(),
        tracks: Default::default(),
        slots: arc_swap::ArcSwap::from_pointee(Default::default()),
        next_id: AtomicU64::new(1),
        drops: ops::DROP_NAMES.iter().map(|_| AtomicU64::new(0)).collect(),
        codes: Default::default(),
        rt: tokio::runtime::Handle::current(),
    });
    ops::timers::start();
    let beat = Arc::new(AtomicU64::new(common::now_ms()));
    let b2 = beat.clone();
    tokio::spawn(async move {
        loop {
            b2.store(common::now_ms(), Ordering::Release);
            tokio::time::sleep(Duration::from_millis(BEAT_MS)).await;
        }
    });
    self_watch(beat);
    let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("oxsfud: signal {e}");
            return 1;
        }
    };
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let grpc = cfg.grpc;
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(ops::handle::server())
            .serve_with_shutdown(grpc, async {
                let _ = stop_rx.await;
            })
            .await
    });
    tracing::info!(kind = "diag", "sfud ready grpc {} udp {}:{} workers {}", grpc, cfg.ip, cfg.udp_port, cfg.workers);
    let why = tokio::select! {
        _ = term.recv() => "SIGTERM",
        _ = tokio::signal::ctrl_c() => "SIGINT",
        r = server => {
            if let Ok(Err(e)) = r {
                eprintln!("oxsfud: grpc {e}");
                return 1;
            }
            "grpc ended"
        }
    };
    tracing::info!(kind = "diag", "sfud stopping ({why})");
    let _ = stop_tx.send(());
    bus.close().await;
    0
}
