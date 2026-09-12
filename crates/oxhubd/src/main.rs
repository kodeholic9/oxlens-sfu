// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§16-1 · §18-1 · 운영 §3 · §8-2 · model: claude-opus-5

//! `oxhubd` — hub 프로세스.
//!
//! ★**기동 순서는 유닛의 순서 속성대로**이고 ★**정지는 그 역순**이다(정§15-6 — zenoh 는
//! sfu 보다 먼저 서고 나중에 닫힌다).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use common::{Args, Policy, System};
use oxhubd::authz::{self, Peer};
use oxhubd::boot::Resolved;
use oxhubd::healthz;
use oxhubd::session::Sessions;
use oxhubd::supervisor::{Action, Supervisor};
use oxhubd::token::{self, IssueReq};
use oxhubd::ws::{self, Conn, Reply};
use tokio::sync::Mutex;

struct Hub {
    resolved: Resolved,
    sup: Mutex<Supervisor>,
    sessions: Mutex<Sessions>,
    /// ★**살아 있는 자식들.** ★**여기 붙들고 있는 동안 부모 생존 채널이 열려 있다** —
    /// hub 가 어떤 방식으로 죽든 프로세스가 사라지면 파이프의 쓰기 끝이 닫히고
    /// 자식은 EOF 를 본다(정§15-6).
    children: Mutex<Vec<(String, std::process::Child)>>,
}

type Shared = Arc<Hub>;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match Args::parse(&argv) {
        Ok(a) => a,
        // ★모르는 인자는 기동 실패다 — 조용히 넘기면 "줬는데 왜 안 먹나" 만 남는다.
        Err(e) => {
            eprintln!("{e}");
            return std::process::ExitCode::from(2);
        }
    };
    if args.version {
        // ★**바이너리에게 직접 묻는 자리** — 하네스가 옛 바이너리의 초록을 봉쇄하는 데 쓴다.
        println!("{}", common::BuildId::new(args.build.clone()).line());
        return std::process::ExitCode::SUCCESS;
    }
    if args.help {
        eprintln!("oxhubd --id <node_id> [--system F] [--policy F] [--listen A] [--log-dir D] [--build B]");
        return std::process::ExitCode::SUCCESS;
    }
    match run(args).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::from(1)
        }
    }
}

fn read(path: &Option<String>) -> Result<String, String> {
    match path {
        Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}")),
        None => Ok(String::new()),
    }
}

async fn run(args: Args) -> Result<(), String> {
    let system = System::parse(&read(&args.system)?).map_err(|e| e.to_string())?;
    let policy = Policy::parse(&read(&args.policy)?).map_err(|e| e.to_string())?;
    let resolved = Resolved::new(&args, system, policy).map_err(|e| e.to_string())?;

    // ★배너가 먼저다 — 아래에서 무엇이 실패하든 무슨 값으로 떴는지는 남는다.
    eprintln!("{}", resolved.banner(&args));

    let mut sup = Supervisor::new(&resolved.system.units);
    let first = sup.start_all(now_ms());
    let hub: Shared = Arc::new(Hub {
        resolved,
        sup: Mutex::new(sup),
        sessions: Mutex::new(Sessions::new()),
        children: Mutex::new(Vec::new()),
    });

    // ★접속점은 `{base}` 아래다 — 앱이 클라에 주는 그 값이다(연§5-0).
    let base = hub.resolved.system.hub.base_path.trim_end_matches('/').to_string();
    let app = Router::new()
        // ★WS 업그레이드는 무인증으로 열린다 — 인증은 `BIND` 가 한다.
        //   토큰을 질의값에 실으면 액세스 로그·프록시·리퍼러에 그대로 남는다.
        .route(&format!("{base}/ws"), get(ws_upgrade))
        .route(&format!("{base}/auth/token"), axum::routing::post(auth_token))
        .route("/healthz/live", get(|| async { StatusCode::OK }))
        .route("/healthz/ready", get(ready))
        .route("/admin/sfus", get(admin_sfus))
        .route("/admin/snapshot", get(admin_snapshot))
        .with_state(hub.clone());

    let addr: SocketAddr = hub.resolved.listen.parse().map_err(|e| format!("listen: {e}"))?;
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| e.to_string())?;

    for a in &first {
        apply(&hub, a).await;
    }

    // backoff·회수 시계 — ★판정은 supervisor 가 하고 여기는 손만 쓴다.
    let ticker = hub.clone();
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            iv.tick().await;
            // ★`Starting` 인 유닛에 인사를 건다 — ★**답하는 순간이 `Running`** 이다(정§16-1).
            //   ★띄운 적 없는 것이 살아 있을 수 없으므로 `Starting` 에만 건다.
            let pending: Vec<(String, String)> = {
                let s = ticker.sup.lock().await;
                s.units
                    .iter()
                    .filter(|u| u.state == oxhubd::supervisor::UnitState::Starting)
                    .filter_map(|u| {
                        ticker
                            .resolved
                            .system
                            .units
                            .iter()
                            .find(|d| d.id == u.id && !d.addr.is_empty())
                            .map(|d| (u.id.clone(), d.addr.clone()))
                    })
                    .collect()
            };
            for (id, addr) in pending {
                if let Some(epoch) = hello(&ticker.resolved.node_id, &addr).await {
                    let mut s = ticker.sup.lock().await;
                    s.on_ready(&id, epoch);
                }
            }

            // ★끝난 자식을 먼저 거둔다 — 그래야 supervisor 가 `Down` 을 제때 본다.
            let done: Vec<String> = {
                let mut cs = ticker.children.lock().await;
                let mut gone = Vec::new();
                cs.retain_mut(|(id, ch)| match ch.try_wait() {
                    Ok(Some(_)) => {
                        gone.push(id.clone());
                        false
                    }
                    _ => true,
                });
                gone
            };
            let actions = {
                let mut s = ticker.sup.lock().await;
                let mut acts = Vec::new();
                for id in &done {
                    acts.push(s.on_exit(id, now_ms()));
                }
                acts.extend(s.tick(now_ms()));
                acts
            };
            for a in &actions {
                apply(&ticker, a).await;
            }
        }
    });

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .map_err(|e| e.to_string())
}

/// ★**기동 핸드셰이크** — 답하면 그 값이 그 유닛의 기동 신원이다.
///
/// ★**못 닿는 것은 실패가 아니라 아직 안 선 것**이다(`Starting` 그대로) — 지어내지 않는다.
async fn hello(node_id: &str, addr: &str) -> Option<String> {
    let url = format!("http://{addr}");
    let mut c = common::b::sfu_service_client::SfuServiceClient::connect(url).await.ok()?;
    let r = c
        .hello(common::b::HelloRequest { node_id: node_id.to_string() })
        .await
        .ok()?
        .into_inner();
    eprintln!("[b] hello → {addr} epoch={} build={}", r.epoch, r.build);
    Some(r.epoch)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// ★**판단은 supervisor 가 내고 손은 여기서 쓴다** — 콜백을 주입하지 않는다.
async fn apply(hub: &Shared, action: &Action) {
    match action {
        Action::Spawn(id) => match spawn_unit(hub, id).await {
            Ok(()) => eprintln!("[sup] spawn {id}"),
            Err(e) => {
                // ★★**못 띄운 것도 기동 시도다** — 안 세면 없는 실행파일에 무한 재시도를 돈다.
                eprintln!("[sup] spawn {id} 실패: {e}");
                let next = { hub.sup.lock().await.on_spawn_failed(id, now_ms()) };
                Box::pin(apply(hub, &next)).await;
            }
        },
        Action::Signal(id) => eprintln!("[sup] signal {id}"),
        Action::ShutdownHub(id) => {
            eprintln!("[sup] ★{id} 가 살아나지 못한다 — hub 를 내린다");
            // ★자식은 우리가 죽이지 않아도 부모 생존 채널로 스스로 끝난다(정§15-6).
            std::process::exit(1);
        }
        Action::None => {}
    }
}

/// ★**자식의 표준입력이 부모 생존 채널이다** — hub 는 거기에 아무것도 쓰지 않는다.
///
/// hub 프로세스가 사라지면 쓰기 끝이 닫히고 자식은 EOF 를 본다 —
/// ★**정상 종료·패닉·강제 종료 어느 쪽이든 성립한다.**
async fn spawn_unit(hub: &Shared, id: &str) -> Result<(), String> {
    let unit = hub
        .resolved
        .system
        .units
        .iter()
        .find(|u| u.id == id)
        .ok_or_else(|| format!("{id} 는 목록에 없다"))?
        .clone();
    if unit.cmd.is_empty() {
        return Err(format!("{id} 에 cmd 가 없다"));
    }
    let mut cmd = std::process::Command::new(&unit.cmd[0]);
    cmd.args(&unit.cmd[1..])
        .arg("--id")
        .arg(&unit.id)
        .stdin(std::process::Stdio::piped());
    if let Some(dir) = &hub.resolved.log_dir {
        // ★로그 디렉터리는 hub 가 자식에게 넘긴다 — 자식이 자기 파일을 연다(파이프가 아니다).
        cmd.arg("--log-dir").arg(dir);
    }
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    // ★`stdin` 을 `take()` 하지 않는다 — 쥐고 있는 것이 곧 생존 신호다.
    hub.children.lock().await.push((id.to_string(), child));
    Ok(())
}

fn peer_of(addr: SocketAddr, headers: &HeaderMap) -> Peer {
    Peer {
        // ★소켓이 말하는 상대만 본다 — `X-Forwarded-For` 는 읽지도 않는다.
        is_loopback: addr.ip().is_loopback(),
        bearer: headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|v| v.to_string()),
    }
}

fn fail(code: oxsig::Code) -> (StatusCode, Json<oxsig::Failure>) {
    let http = match code {
        oxsig::Code::TokenInvalid
        | oxsig::Code::TokenExpired
        | oxsig::Code::InvalidApiKey
        | oxsig::Code::NotAuthorized => StatusCode::UNAUTHORIZED,
        oxsig::Code::PreconditionFailed => StatusCode::CONFLICT,
        oxsig::Code::RoomNotFound => StatusCode::NOT_FOUND,
        _ => StatusCode::BAD_REQUEST,
    };
    (http, Json(oxsig::Failure::new(code)))
}

async fn ready(State(hub): State<Shared>) -> (StatusCode, Json<serde_json::Value>) {
    let sup = hub.sup.lock().await;
    let r = healthz::ready(&sup, true);
    let code = if r.ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    // ★어느 노드가 빠졌는지까지 낸다 — 503 만 주면 운영자가 다시 물어봐야 한다.
    (code, Json(serde_json::json!({ "ready": r.ok, "down": r.down, "build": hub.resolved.build.line() })))
}

async fn auth_token(
    State(hub): State<Shared>,
    Json(req): Json<IssueReq>,
) -> Result<Json<token::IssueRes>, (StatusCode, Json<oxsig::Failure>)> {
    let p = &hub.resolved.policy.hub;
    token::issue(&hub.resolved.system, p.token_ttl_secs, p.metadata_max_bytes, now_ms() / 1000, &req)
        .map(Json)
        .map_err(|e| fail(e.0))
}

async fn admin_sfus(
    State(hub): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<oxsig::Failure>)> {
    authz::admin(&hub.resolved.system, now_ms() / 1000, &peer_of(addr, &headers)).map_err(fail)?;
    let sup = hub.sup.lock().await;
    let sfus: Vec<serde_json::Value> = sup
        .units
        .iter()
        .map(|u| {
            serde_json::json!({
                "sfu_id": u.id,
                "unit_state": format!("{:?}", u.state),
                // ★유닛 축 — dial 이 아니다.
                "live": u.state.is_live(),
                // ★노드 축은 B 평면이 채운다(덩어리 4) — 지어내지 않는다.
                "node_live": serde_json::Value::Null,
                "epoch": u.epoch,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "sfus": sfus, "supervising": !sup.units.is_empty() })))
}

async fn admin_snapshot(
    State(hub): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<oxsig::Failure>)> {
    authz::admin(&hub.resolved.system, now_ms() / 1000, &peer_of(addr, &headers)).map_err(fail)?;
    let sup = hub.sup.lock().await;
    let r = healthz::ready(&sup, true);
    Ok(Json(serde_json::json!({
        // ★`shutdown` 확인값의 출처다(운영 §5).
        "hub_id": hub.resolved.node_id,
        "ready": r.ok,
        "build": hub.resolved.build.line(),
        "supervising": !sup.units.is_empty(),
        "rooms": 0,
        "users": 0,
    })))
}

async fn ws_upgrade(
    State(hub): State<Shared>,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> axum::response::Response {
    // ★서브프로토콜을 쓰지 않는다 — `Sec-WebSocket-Protocol` 을 보지도 않는다.
    upgrade.on_upgrade(move |socket| serve_ws(hub, socket))
}

async fn serve_ws(hub: Shared, mut socket: axum::extract::ws::WebSocket) {
    use axum::extract::ws::Message;
    use oxsig::frame;

    let mut conn = Conn::Unbound { opened_at: now_ms() };
    let window_ms = hub.resolved.policy.hub.resume_window_ms as u64;
    // ★그릇을 하나 잡아 재사용한다 — 프레임마다 새로 잡지 않는다(핫패스 규율 H3).
    let mut out = Vec::with_capacity(frame::HEADER_LEN + 256);

    loop {
        let msg = tokio::select! {
            m = socket.recv() => m,
            // ★`BIND` 가 제때 안 오면 끊는다 — 무인증 소켓을 열어 두지 않는다.
            () = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                if let Some(n) = ws::bind_overdue(&conn, now_ms()) {
                    send_leave(&mut socket, &mut out, &n).await;
                    return;
                }
                continue;
            }
        };
        let Some(Ok(Message::Binary(buf))) = msg else { return };
        // ★text 프레임은 받지 않는다(연§3-1 — binary 고정). 위 패턴이 그것을 거른다.
        let (header, body) = match frame::decode(&buf) {
            Ok(v) => v,
            Err(e) => {
                if let Reply::Close(n) = ws::on_decode_error(e) {
                    send_leave(&mut socket, &mut out, &n).await;
                }
                return;
            }
        };
        let reply = {
            let sys = &hub.resolved.system;
            let verify = |t: &str| {
                oxhubd::token::verify_user(sys, now_ms() / 1000, t)
                    .map(|c| oxhubd::session::VerifiedToken {
                        user_id: c.sub.clone(),
                        participant_type: c.participant_type,
                        hidden: c.hidden,
                        permission: c.permission(),
                    })
                    .map_err(|e| e.0)
            };
            let mut sessions = hub.sessions.lock().await;
            ws::dispatch(&mut conn, &mut sessions, &verify, window_ms, now_ms(), header, body)
        };
        match reply {
            Reply::Ok { header, body } | Reply::Fail { header, body } => {
                frame::encode(&mut out, header, &body);
                if socket.send(Message::Binary(out.clone())).await.is_err() {
                    return;
                }
            }
            Reply::Silent => {}
            Reply::Close(n) => {
                send_leave(&mut socket, &mut out, &n).await;
                return;
            }
        }
    }
}

/// ★**사유는 `LEAVE` 가 나른다** — WS Close 에 싣지 않는다(전송을 바꿔도 이 op 은 그대로 선다).
async fn send_leave(
    socket: &mut axum::extract::ws::WebSocket,
    out: &mut Vec<u8>,
    notice: &oxsig::body::session::LeaveNotice,
) {
    use axum::extract::ws::Message;
    use oxsig::frame::{self, Header, Kind};
    let body = serde_json::to_vec(notice).unwrap_or_default();
    frame::encode(out, Header::new(Kind::Request, oxsig::Op::Leave, 0), &body);
    let _ = socket.send(Message::Binary(out.clone())).await;
    // ★보내고 닫는다 — 응답을 기다리면 닫지도 못하고 굳는다.
    let _ = socket.send(Message::Close(None)).await;
}
