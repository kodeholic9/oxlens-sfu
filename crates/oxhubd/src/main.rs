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
    for a in sup.start_all(now_ms()) {
        apply(&a);
    }
    let hub: Shared = Arc::new(Hub { resolved, sup: Mutex::new(sup), sessions: Mutex::new(Sessions::new()) });

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

    // backoff 시계 — ★판정은 supervisor 가 하고 여기는 손만 쓴다.
    let ticker = hub.clone();
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            iv.tick().await;
            let mut s = ticker.sup.lock().await;
            for a in s.tick(now_ms()) {
                apply(&a);
            }
        }
    });

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .map_err(|e| e.to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// ★**판단은 supervisor 가 내고 손은 여기서 쓴다** — 콜백을 주입하지 않는다.
///
/// ★**부모 생존 채널은 아직 없다**(정§15-6) — 자식 쪽 끝이 `oxsfud` 라 덩어리 4 와 한 쌍이다.
/// 그때까지 `kill_on_drop` 은 ★**강제 종료를 못 덮는다**는 것을 알고 쓴다.
fn apply(action: &Action) {
    match action {
        Action::Spawn(id) => eprintln!("[sup] spawn {id}"),
        Action::Signal(id) => eprintln!("[sup] signal {id}"),
        Action::ShutdownHub(id) => eprintln!("[sup] ★{id} 가 살아나지 못한다 — hub 를 내린다"),
        Action::None => {}
    }
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
    (code, Json(serde_json::json!({ "ready": r.ok, "down": r.down, "build": hub.resolved.build.as_str() })))
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
        "build": hub.resolved.build.as_str(),
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
