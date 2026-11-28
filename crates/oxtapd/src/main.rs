// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-OPS-REC-UNIT · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-OPS-SUPERVISOR · 정:RULE-BUS-FATE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-SHAPE · 정:IMPL-FLR-TALK-REPORT · 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! 녹음 유닛 실행 파일 — 슈퍼바이저가 띄운다(`--id` · `--build` · `--bus-namespace` · `--bus-connect` · `--log-dir`? · `--log-socket`? · `--data-root` · `--rec-keep-days` · `--token` · `--backoff-ms` · `--intensity-max` · `--intensity-window-secs` · `--hub-base` · `--hub-ca`? · 유닛 명령의 `--grpc-listen`) · Handle(`Hello` 준비 신호 · `Call` 은 `/admin/drops` 와 감독자의 토큰 넘김 — 새 토큰은 방 상세 조회와 세션의 다시 BIND 에 쓴다) · 방 상세를 못 읽으면 경고 줄 · 버스로 그 node 의 방 토큰을 보고 `record` 방에 녹화 참가자로 든다 · 발언마다 파일 하나(영상 트랙이 든 발언은 mkv · 아니면 ogg)를 다 쓰고 이름을 바꿔 닫은 뒤 `rec` 알림 · 영상 키프레임 요청은 SDK 에 · 하루 한 번 보관 지우기 · 부모 생존 채널(stdin)이 닫히면 끝 · 토큰을 다시 요구받거나 세션이 끝나면 끝(슈퍼바이저가 새 토큰으로 되살린다).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use common::bus::TokenEvent;
use common::keyspace::Parsed;
use common::log::{Setup, Sink};
use common::talk::Report;
use oxpeer::record::{Pipeline, Policy, RecordConfig, Recorder};
use oxpeer::{Client, Config, Event};
use oxtapd::bus::Bus;
use oxtapd::{Hub, Layout};
use tokio::sync::mpsc;

struct Args {
    node: String,
    build: String,
    ns: String,
    connect: String,
    log_dir: Option<PathBuf>,
    log_socket: Option<PathBuf>,
    root: PathBuf,
    keep_days: u32,
    token: String,
    hub_base: String,
    hub_ca: Option<PathBuf>,
    grpc: std::net::SocketAddr,
    restart: oxtapd::Restart,
}

fn args() -> Result<Args, String> {
    let mut m: HashMap<String, String> = HashMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let Some(name) = k.strip_prefix("--") else { return Err(format!("unknown argument {k}")) };
        let v = it.next().ok_or_else(|| format!("--{name} needs a value"))?;
        m.insert(name.to_string(), v);
    }
    let need = |k: &str| m.get(k).cloned().ok_or_else(|| format!("--{k} missing"));
    for k in m.keys() {
        if !["id", "build", "bus-namespace", "bus-connect", "log-dir", "log-socket", "data-root", "rec-keep-days", "token", "hub-base", "hub-ca", "grpc-listen", "backoff-ms", "intensity-max", "intensity-window-secs"].contains(&k.as_str()) {
            return Err(format!("unknown argument --{k}"));
        }
    }
    Ok(Args {
        node: need("id")?,
        build: m.get("build").cloned().unwrap_or_default(),
        ns: need("bus-namespace")?,
        connect: need("bus-connect")?,
        log_dir: m.get("log-dir").map(PathBuf::from),
        log_socket: m.get("log-socket").map(PathBuf::from),
        root: PathBuf::from(need("data-root")?),
        keep_days: m.get("rec-keep-days").map(|v| v.parse().map_err(|_| "--rec-keep-days".to_string())).transpose()?.unwrap_or(common::system::REC_KEEP_DAYS_DEFAULT),
        token: need("token")?,
        hub_base: need("hub-base")?,
        hub_ca: m.get("hub-ca").map(PathBuf::from),
        grpc: need("grpc-listen")?.parse().map_err(|e| format!("--grpc-listen: {e}"))?,
        restart: oxtapd::Restart::parse(m.get("backoff-ms").map(String::as_str), m.get("intensity-max").map(String::as_str), m.get("intensity-window-secs").map(String::as_str))?,
    })
}

enum RoomCmd {
    Join(String),
    Leave(String),
}

#[tokio::main]
async fn main() {
    let a = match args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("oxtapd: {e}");
            std::process::exit(2);
        }
    };
    std::process::exit(run(a).await);
}

async fn run(a: Args) -> i32 {
    let policy = common::policy::Policy::default();
    let sink = match &a.log_socket {
        Some(p) => Sink::Socket(p.clone()),
        None => Sink::Stdout,
    };
    let epoch = common::ident::new_epoch();
    let _sender = common::log::init(Setup { node: a.node.clone(), proc_name: "peer", epoch: Some(epoch.clone()), sink, log_dir: a.log_dir.clone() }, &policy);
    tracing::info!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", build = %a.build, root = %a.root.display(), keep_days = a.keep_days, backoff_ms = ?a.restart.backoff_ms, intensity_max = a.restart.intensity_max, intensity_window_secs = a.restart.intensity_window.as_secs(), "rec unit boot");
    let ca = match &a.hub_ca {
        Some(p) => match std::fs::read(p) {
            Ok(b) => Some(b),
            Err(e) => {
                eprintln!("oxtapd: --hub-ca {}: {e}", p.display());
                return 2;
            }
        },
        None => None,
    };
    let Some(hub) = oxtapd::hub_of(&a.hub_base, ca.as_deref()) else {
        eprintln!("oxtapd: --hub-base must be http(s)://host:port/base (https needs --hub-ca)");
        return 2;
    };
    let drops = Arc::new(AtomicU64::new(0));
    let (tok_tx, tok_rx) = tokio::sync::watch::channel(a.token.clone());
    let svc = oxtapd::Svc { epoch, build: common::ident::build_id(common::ident::COMPILED, Some(&a.build)), drops: drops.clone(), token: tok_tx };
    let addr = a.grpc;
    tokio::spawn(async move {
        if let Err(e) = tonic::transport::Server::builder().add_service(common::envelope::handle::handle_server::HandleServer::new(svc)).serve(addr).await {
            eprintln!("oxtapd: --grpc-listen {addr}: {e}");
            common::log::drain(Duration::from_millis(200));
            std::process::exit(2);
        }
    });
    let layout = Arc::new(Layout::new(&a.root));
    if let Err(e) = layout.clear_staging() {
        eprintln!("oxtapd: staging {}: {e}", layout.staging().display());
        return 2;
    }
    let bus = match Bus::open(&a.node, &a.ns, &a.connect).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("oxtapd: bus: {e}");
            return 2;
        }
    };
    std::thread::spawn(|| {
        let mut sink = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::stdin(), &mut sink);
        common::log::drain(Duration::from_millis(200));
        std::process::exit(0);
    });
    let (sweep_root, keep) = (a.root.clone(), a.keep_days);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(oxtapd::DAY);
        loop {
            tick.tick().await;
            match oxtapd::sweep(&sweep_root, keep, std::time::SystemTime::now()) {
                Ok(n) if n > 0 => tracing::info!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", removed = n, "rec files past keep days removed"),
                Ok(_) => {}
                Err(e) => tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", "rec sweep: {e}"),
            }
        }
    });
    let (rep_tx, rep_rx) = mpsc::unbounded_channel::<Report>();
    let (b2, d2) = (bus.clone(), drops.clone());
    tokio::spawn(async move {
        oxtapd::report_line(
            rep_rx,
            common::talk::RETRY_FOR,
            |p| {
                let b = b2.clone();
                async move { b.ask(common::talk::WHAT, p).await }
            },
            &d2,
        )
        .await
    });
    let (room_tx, mut room_rx) = mpsc::unbounded_channel::<RoomCmd>();
    if let Err(e) = discover(&bus, hub, tok_rx.clone(), room_tx).await {
        eprintln!("oxtapd: bus watch: {e}");
        return 2;
    }
    let mut cfg = Config::new(&a.hub_base, &a.token);
    cfg.tls_pin_pem = ca;
    cfg.record = match Recorder::open(RecordConfig::live(format!("rec-{}", a.node))) {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::error!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", "rec recorder: {e}");
            return 1;
        }
    };
    let (client, mut events, mut media, _frames) = match Client::open(cfg).await {
        Ok(x) => x,
        Err(e) => {
            tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", "rec unit connect failed: {e}");
            return 1;
        }
    };
    let mut records = client.records(1 << 16);
    let mut pipe = Pipeline::new(Policy::Utterance);
    let mut talks = match oxtapd::Talks::new(Layout::new(&a.root)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("oxtapd: staging: {e}");
            return 2;
        }
    };
    let mut joined: BTreeSet<String> = BTreeSet::new();
    let mut tok = tok_rx;
    loop {
        tokio::select! {
            Ok(()) = tok.changed() => {
                let t = tok.borrow_and_update().clone();
                if let Err(e) = client.set_token(t) {
                    tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", "renewed token not given to the session: {e}");
                }
            }
            c = room_rx.recv() => match c {
                Some(RoomCmd::Join(r)) if joined.insert(r.clone()) => {
                    if let Err(e) = client.join(&r).await {
                        joined.remove(&r);
                        tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", room = %r, "rec join failed: {e}");
                    }
                }
                Some(RoomCmd::Leave(r)) if joined.remove(&r) => {
                    let _ = client.leave(&r).await;
                }
                Some(_) => {}
                None => return 1,
            },
            e = events.next() => match e {
                Some(Event::TokenRequired) | Some(Event::Closed { .. }) | None => {
                    tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", "rec unit session ended — supervisor restarts with a new token");
                    return 1;
                }
                Some(Event::Forced { room, .. }) | Some(Event::Left { room }) => {
                    joined.remove(&room);
                }
                Some(_) => {}
            },
            p = media.next() => if p.is_none() { return 1 },
            r = records.next() => {
                let Some(r) = r else { return 1 };
                for o in pipe.push(&r) {
                    for rep in talks.feed(&o) {
                        let _ = rep_tx.send(rep);
                    }
                }
                for tid in talks.keyframe_requests() {
                    let _ = client.request_keyframe(tid);
                }
            }
        }
    }
}

async fn discover(bus: &Bus, hub: Hub, token: tokio::sync::watch::Receiver<String>, tx: mpsc::UnboundedSender<RoomCmd>) -> Result<(), String> {
    let mut sub = bus.rooms().await?;
    let keys = bus.keys.clone();
    let hub = Arc::new(hub);
    tokio::spawn(async move {
        let mut live: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        while let Some(ev) = sub.recv().await {
            let (k, put) = match ev {
                TokenEvent::Put(k) => (k, true),
                TokenEvent::Delete(k) => (k, false),
                TokenEvent::BatchEnd => continue,
            };
            let Some(Parsed::Room { room, inst, .. }) = keys.parse(&k) else { continue };
            let insts = live.entry(room.clone()).or_default();
            let was = !insts.is_empty();
            if put {
                insts.insert(inst);
            } else {
                insts.remove(&inst);
            }
            let now = !insts.is_empty();
            if !now {
                live.remove(&room);
            }
            match (was, now) {
                (false, true) => {
                    let (hub, token, tx) = (hub.clone(), token.borrow().clone(), tx.clone());
                    tokio::spawn(async move {
                        match oxtapd::get_json(&hub, &format!("/rooms/{room}"), &token).await {
                            Some((200, body)) if oxtapd::recorded(&body) => {
                                let _ = tx.send(RoomCmd::Join(room));
                            }
                            Some((200, _)) => {}
                            Some((status, _)) => tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", room = %room, status, "room detail refused by the local gateway — the room is not recorded"),
                            None => tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", room = %room, "room detail not read from the local gateway"),
                        }
                    });
                }
                (true, false) => {
                    let _ = tx.send(RoomCmd::Leave(room));
                }
                _ => {}
            }
        }
    });
    Ok(())
}
