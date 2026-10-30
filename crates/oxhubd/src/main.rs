// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-FATE · 정:RULE-OPS-SURFACE · 정:RULE-OPS-LOGGER · 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! oxhubd — 기동(로거 → zenoh → sfu) · 정지(클라 `LEAVE 5004` → 유닛 → zenoh → hub).

use std::sync::Arc;
use std::time::Duration;

use common::log::{Setup, Sink};
use oxhubd::bus::{Bus, ZenohCfg};
use oxhubd::cfg::HubCfg;
use oxhubd::ops::supervisor::{Role, SupCfg, Supervisor, UnitState};
use oxhubd::ops::{self, Hub};
use oxhubd::ses::{self, token::Accounts};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "--version") {
        let label = argv.iter().position(|a| a == "--build").and_then(|i| argv.get(i + 1)).map(|s| s.as_str());
        println!("{}", common::ident::build_id(oxhubd::cfg::compiled_build(), label));
        return;
    }
    let cfg = match HubCfg::load(argv) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("oxhubd: {e}");
            std::process::exit(2);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("oxhubd: runtime {e}");
            std::process::exit(2);
        }
    };
    let code = rt.block_on(run(cfg));
    rt.shutdown_timeout(Duration::from_millis(500));
    std::process::exit(code);
}

async fn run(cfg: HubCfg) -> i32 {
    let inst = common::ident::new_epoch();
    let build = cfg.build_id();
    let sink = if cfg.log_socket.is_some() { Sink::Inproc } else { Sink::Stdout };
    let sender = common::log::init(
        Setup { node: cfg.node.clone(), proc_name: "hub", epoch: None, sink, log_dir: cfg.log_dir.clone() },
        &cfg.policy,
    );
    let fields = cfg.banner_fields();
    let banner = common::log::banner::Banner {
        head: &sender.head,
        epoch: None,
        build: &build,
        unit: None,
        msg: "hub boot",
        fields: &fields,
        line_max: cfg.policy.logging.line_max_bytes as usize,
    };
    match banner.lines() {
        Ok(ls) => ls.into_iter().for_each(common::log::emit),
        Err(e) => {
            eprintln!("oxhubd: {e}");
            return 2;
        }
    }
    let (fatal_tx, mut fatal_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let sup = Supervisor::new(
        &cfg.system.unit,
        SupCfg {
            node: cfg.node.clone(),
            hub_inst: inst.clone(),
            build_label: cfg.build_label.clone(),
            backoff_ms: cfg.system.supervisor.backoff_ms.clone(),
            intensity_max: cfg.system.supervisor.intensity_max,
            intensity_window: Duration::from_secs(cfg.system.supervisor.intensity_window_secs),
            hello_timeout: Duration::from_millis(cfg.system.supervisor.hello_timeout_ms),
            extra_args: cfg.child_args(),
        },
        fatal_tx.clone(),
    );
    match (&cfg.log_socket, &cfg.log_dir) {
        (Some(sock), dir) => {
            sup.mark_state(Role::Log, UnitState::Starting, "start");
            let lc = ops::logger::LoggerCfg {
                socket: sock.clone(),
                dir: dir.clone().unwrap_or_else(|| std::path::PathBuf::from(".")),
                policy: cfg.policy.logging.clone(),
            };
            match ops::logger::start(lc, &sender) {
                Ok(()) => sup.mark_state(Role::Log, UnitState::Running, "bound"),
                Err(e) => {
                    tracing::warn!(kind = "diag", "logger: {e}");
                    sup.mark_state(Role::Log, UnitState::Down, "bind failed");
                }
            }
        }
        (None, _) => sup.mark_state(Role::Log, UnitState::Disabled, "no log_socket"),
    }
    sup.mark_state(Role::Zenoh, UnitState::Starting, "start");
    let z = &cfg.system.zenoh;
    let mut tries = 0u32;
    let (bus, bus_rx) = loop {
        let zc = ZenohCfg {
            listen: z.listen.clone(),
            connect: z.connect.clone(),
            multicast_scouting: z.multicast_scouting,
            namespace: z.namespace.clone(),
            base_config: z.base_config.clone(),
        };
        match Bus::open(&cfg.node, &inst, cfg.system.supervisor.node_grace_secs * 1000, zc).await {
            Ok(b) => break b,
            Err(e) => {
                tries += 1;
                tracing::warn!(kind = "diag", unit = "zenoh", "zenoh open failed: {e}");
                sup.mark_state(Role::Zenoh, UnitState::Down, "open failed");
                if tries > cfg.system.supervisor.intensity_max {
                    sup.mark_state(Role::Zenoh, UnitState::Blocked, "intensity");
                    return 1;
                }
                let b = cfg.system.supervisor.backoff_ms.get(tries as usize - 1).copied().unwrap_or(1000);
                tokio::time::sleep(Duration::from_millis(b)).await;
                sup.mark_state(Role::Zenoh, UnitState::Starting, "backoff");
            }
        }
    };
    sup.mark_state(Role::Zenoh, UnitState::Running, "open");
    let accounts = Accounts { keys: cfg.system.hub.auth.api_keys.clone(), jwt_secret: cfg.system.hub.auth.jwt_secret.clone() };
    let ctx = ses::Ctx::new(cfg.policy.clone(), accounts, bus.clone());
    let (shut_tx, mut shut_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let hub = Arc::new(Hub {
        node: cfg.node.clone(),
        inst: inst.clone(),
        build: build.clone(),
        base_path: cfg.system.hub.base_path.clone(),
        policy: cfg.policy.clone(),
        ses: ctx.clone(),
        bus: bus.clone(),
        sup: sup.clone(),
        sender: sender.clone(),
        mode: z.mode.clone(),
        namespace: z.namespace.clone(),
        shutdown: shut_tx,
    });
    tokio::spawn(ops::dispatch(hub.clone(), bus_rx));
    tokio::spawn(ops::sweeper(hub.clone()));
    let tls = cfg.system.hub.tls.as_ref().map(|t| (t.cert.clone(), t.key.clone()));
    if let Err(e) = ops::http::serve(hub.clone(), &cfg.listen, tls).await {
        tracing::warn!(kind = "diag", "{e}");
        return 2;
    }
    sup.start_sfu(bus.clone());
    let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(s) => s,
        Err(_) => return 2,
    };
    let reason = tokio::select! {
        _ = term.recv() => "SIGTERM".to_string(),
        _ = tokio::signal::ctrl_c() => "SIGINT".to_string(),
        r = shut_rx.recv() => r.unwrap_or_default(),
        r = fatal_rx.recv() => r.unwrap_or_default(),
    };
    common::log::emit(common::log::Line::ev("CP-CODE-SERVER-SHUTDOWN").msg(format!("hub shutdown — {reason}")));
    let n = ses::shutdown_all(&ctx);
    if n > 0 {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    sup.stop_all().await;
    sup.mark_state(Role::Zenoh, UnitState::Stopping, "shutdown");
    bus.close().await;
    sup.mark_state(Role::Zenoh, UnitState::Stopped, "shutdown");
    sup.mark_state(Role::Log, UnitState::Stopped, "shutdown");
    tokio::time::sleep(Duration::from_millis(50)).await;
    sender.detach_inproc();
    if reason.starts_with("zenoh") { 1 } else { 0 }
}
