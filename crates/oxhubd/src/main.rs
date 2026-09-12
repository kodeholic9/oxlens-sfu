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
use oxhubd::room::{Rooms, Ttl};
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
    rooms: Mutex<Rooms>,
    /// ★**살아 있는 소켓** — 축출된 옛 연결에 `LEAVE` 를 보내려면 그 소켓을 붙들고 있어야 한다.
    ///
    /// ★**통보만 하고 유령으로 남기지 않는다**(정§3-2 #3) — 남기면 명단에 같은 사람이 둘이고
    /// 옛 소켓의 하트비트 계수가 계속 오른다.
    /// ★**값에 소켓 세대를 같이 둔다** — 세션 하나를 이어 쥐는 소켓이 갈리므로
    /// *"이 자리의 주인이 아직 나인가"* 를 묻지 못하면 옛 연결이 새 주인을 지운다.
    sockets: Mutex<std::collections::BTreeMap<String, (u64, tokio::sync::mpsc::Sender<Out>)>>,
    /// 소켓 세대 발급기 — ★**락을 잡지 않는다**(핫패스 규율 H2).
    next_conn: std::sync::atomic::AtomicU64,
}

/// 펌프에 건네는 것. ★**`Close` 가 있어야 남이 내 소켓을 닫을 수 있다** — 소켓의
/// 쓰기 끝은 펌프가 혼자 쥐고 있어서, 축출하는 쪽은 옛 연결의 루프를 직접 깨울 수 없다.
/// 채널만 끊는 것으로는 안 된다: 옛 루프가 제 송신 끝을 아직 쥐고 있어 펌프가 안 깨어난다.
enum Out {
    Frame(Vec<u8>),
    /// ★`LEAVE` 를 보낸 **뒤** 온다 — 사유 먼저, 절단 나중(연§6-1).
    Close,
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
        rooms: Mutex::new(Rooms::new()),
        sockets: Mutex::new(Default::default()),
        next_conn: std::sync::atomic::AtomicU64::new(1),
    });

    // ★접속점은 `{base}` 아래다 — 앱이 클라에 주는 그 값이다(연§5-0).
    let base = hub.resolved.system.hub.base_path.trim_end_matches('/').to_string();
    let app = Router::new()
        // ★WS 업그레이드는 무인증으로 열린다 — 인증은 `BIND` 가 한다.
        //   토큰을 질의값에 실으면 액세스 로그·프록시·리퍼러에 그대로 남는다.
        .route(&format!("{base}/ws"), get(ws_upgrade))
        .route(&format!("{base}/auth/token"), axum::routing::post(auth_token))
        .route(
            &format!("{base}/rooms"),
            get(list_rooms).post(create_room),
        )
        .route(&format!("{base}/rooms/:room_id"), get(room_detail))
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
        oxsig::Code::SessionNotFound => StatusCode::UNAUTHORIZED,
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

/// ★**클라 HTTP 는 `Authorization: Bearer` 하나**(연§5-1) — 질의값에 토큰을 싣지 않는다.
fn bearer_user(hub: &Shared, headers: &HeaderMap) -> Result<String, oxsig::Code> {
    let t = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(oxsig::Code::TokenInvalid)?;
    oxhubd::token::verify_user(&hub.resolved.system, now_ms() / 1000, t)
        .map(|c| c.sub)
        .map_err(|e| e.0)
}

#[derive(serde::Deserialize)]
struct CreateRoom {
    #[serde(default)]
    room_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    capacity: Option<u32>,
    #[serde(default)]
    unused_ttl_secs: Option<u32>,
    #[serde(default)]
    departure_ttl_secs: Option<u32>,
}

/// 방 상한 — ★**기본이자 최대다.** 초과는 조용히 자르지 않고 거절한다.
const CAPACITY_MAX: u32 = 1_000;
/// `room_id` 상한 — ★**DC TLV `0x1D` 의 `len` 1바이트가 전 규격의 상한**이다.
const ROOM_ID_MAX: usize = 255;

async fn list_rooms(
    State(hub): State<Shared>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<oxsig::Failure>)> {
    bearer_user(&hub, &headers).map_err(fail)?;
    let rooms = hub.rooms.lock().await;
    let list: Vec<serde_json::Value> = rooms
        .iter()
        .map(|r| {
            serde_json::json!({
                "room_id": r.id,
                "name": r.name,
                "capacity": r.capacity,
                // ★보이는 수다(투명 제외).
                "user_count": r.user_count(),
                "created_at": r.created_at,
                // ★녹화 사실은 감추지 않는다 — `hidden` 이어도 참이다.
                "rec": r.rec(),
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "total": list.len(), "rooms": list })))
}

async fn create_room(
    State(hub): State<Shared>,
    headers: HeaderMap,
    Json(req): Json<CreateRoom>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<oxsig::Failure>)> {
    bearer_user(&hub, &headers).map_err(fail)?;
    let Some(name) = req.name.filter(|n| !n.is_empty()) else {
        return Err(fail(oxsig::Code::MissingField));
    };
    let capacity = req.capacity.unwrap_or(CAPACITY_MAX);
    if capacity == 0 || capacity > CAPACITY_MAX {
        // ★조용히 자르지 않는다 — 자르면 부른 쪽이 자기가 무엇을 얻었는지 모른다.
        return Err(fail(oxsig::Code::InvalidPayload));
    }
    let id = match req.room_id.filter(|s| !s.is_empty()) {
        Some(v) if v.len() > ROOM_ID_MAX => return Err(fail(oxsig::Code::InvalidPayload)),
        Some(v) => v,
        // ★자동 id 는 hub 가 확정해 주입한다 — sfud 는 명시 id 경로 하나만 탄다.
        None => uuid::Uuid::new_v4().simple().to_string(),
    };
    let max_rooms = hub.resolved.policy.quota.max_rooms;
    let mut rooms = hub.rooms.lock().await;
    // ★`0` = 상한 없음. 있으면 ★**이 배포 전체**의 상한이다(방에 주인이 없다).
    if max_rooms > 0 && rooms.get(&id).is_none() && rooms.len() as u32 >= max_rooms {
        return Err(fail(oxsig::Code::QuotaExceeded));
    }
    let ttl = Ttl {
        unused_secs: req.unused_ttl_secs,
        departure_secs: req.departure_ttl_secs,
    };
    // ★멱등 — 같은 id 로 다시 부르면 그 방이 그대로 온다(`name` 은 무시).
    let r = rooms.create(id, name, capacity, ttl, now_ms());
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "room_id": r.id,
            "name": r.name,
            "capacity": r.capacity,
            "created_at": r.created_at,
        })),
    ))
}

/// `GET /rooms/{room_id}` — ★**방 상세.** 없으면 `3001`(그 방이 없는 것이지 서버 사정이 아니다).
///
/// ★**미입장 방도 조회를 허용한다** — 막으면 사용자가 채널을 눈감고 고른다(연§5-5).
async fn room_detail(
    State(hub): State<Shared>,
    axum::extract::Path(room_id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<oxsig::Failure>)> {
    // ★인증 두 갈래 — `Bearer` 또는 세션 헤더(새 비밀을 만들지 않는다, 정§14-4).
    if bearer_user(&hub, &headers).is_err() {
        let sid = headers
            .get("x-oxlens-session")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        match sid {
            // ★**세션 갈래의 실패는 `2008`** 이다 — 토큰이 틀린 것(`2002`)과 다른 축이다.
            //   합치면 *"세션이 만료됐다"* 와 *"토큰이 위조됐다"* 가 같은 답을 받아 처방이 갈린다.
            Some(s) if hub.sessions.lock().await.get(&s).is_none() => {
                return Err(fail(oxsig::Code::SessionNotFound));
            }
            Some(_) => {}
            None => return Err(fail(oxsig::Code::TokenInvalid)),
        }
    }
    let rooms = hub.rooms.lock().await;
    let Some(r) = rooms.get(&room_id) else {
        return Err(fail(oxsig::Code::RoomNotFound));
    };
    Ok(Json(serde_json::json!({
        "room_id": r.id,
        "name": r.name,
        "capacity": r.capacity,
        "user_count": r.user_count(),
        "created_at": r.created_at,
        "rec": r.rec(),
        // 명단·트랙·`version` 은 sfud 정본이라 결선 뒤다 — ★지어내지 않는다.
        "participants": [],
    })))
}

async fn ws_upgrade(
    State(hub): State<Shared>,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> axum::response::Response {
    // ★서브프로토콜을 쓰지 않는다 — `Sec-WebSocket-Protocol` 을 보지도 않는다.
    upgrade.on_upgrade(move |socket| serve_ws(hub, socket))
}

async fn serve_ws(hub: Shared, socket: axum::extract::ws::WebSocket) {
    use axum::extract::ws::Message;
    use futures_util::{SinkExt, StreamExt};
    use oxsig::frame;

    // ★내보내기를 한 갈래로 모은다 — 축출 통지가 남의 소켓으로 가야 하므로
    //   *"이 루프만 쓴다"* 가 성립하지 않는다.
    let (mut tx_sock, mut rx_sock) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Out>(64);
    let pump = tokio::spawn(async move {
        while let Some(o) = rx.recv().await {
            let Out::Frame(b) = o else { break };
            if tx_sock.send(Message::Binary(b)).await.is_err() {
                break;
            }
        }
        // ★WS Close 에 코드를 싣지 않는다 — 사유는 이미 `LEAVE` 가 날랐다(연§6-1).
        let _ = tx_sock.send(Message::Close(None)).await;
    });

    let conn_gen = hub.next_conn.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut conn = Conn::Unbound { opened_at: now_ms() };
    let window_ms = hub.resolved.policy.hub.resume_window_ms as u64;
    // ★그릇을 하나 잡아 재사용한다 — 프레임마다 새로 잡지 않는다(핫패스 규율 H3).
    let mut out = Vec::with_capacity(frame::HEADER_LEN + 256);

    loop {
        let msg = tokio::select! {
            m = rx_sock.next() => m,
            // ★`BIND` 가 제때 안 오면 끊는다 — 무인증 소켓을 열어 두지 않는다.
            () = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                if let Some(n) = ws::bind_overdue(&conn, now_ms()) {
                    send_leave(&tx, &mut out, &n).await;
                    break;
                }
                continue;
            }
        };
        let Some(Ok(Message::Binary(buf))) = msg else { break };
        // ★text 프레임은 받지 않는다(연§3-1 — binary 고정). 위 패턴이 그것을 거른다.
        let (header, body) = match frame::decode(&buf) {
            Ok(v) => v,
            // ★**모르는 op 은 끊지 않는다** — `op`·`pid` 가 멀쩡하니 `1001` **응답**으로 답한다.
            Err(oxsig::frame::DecodeError::UnknownOp { op, pid }) => {
                let body = serde_json::to_vec(&oxsig::Failure::new(oxsig::Code::UnknownOp))
                    .unwrap_or_default();
                // 카탈로그 밖 번호를 그대로 되돌리려면 헤더를 손으로 짓는다.
                out.clear();
                out.push(oxsig::frame::VER);
                out.push(0b10);
                out.extend_from_slice(&op.to_be_bytes());
                out.extend_from_slice(&pid.to_be_bytes());
                out.extend_from_slice(&body);
                if tx.send(Out::Frame(out.clone())).await.is_err() {
                    break;
                }
                continue;
            }
            Err(e) => {
                if let Reply::Close(n) = ws::on_decode_error(e) {
                    send_leave(&tx, &mut out, &n).await;
                }
                break;
            }
        };
        let (reply, evicted) = {
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
            let before = conn.clone();
            let r = ws::dispatch(&mut conn, &mut sessions, &verify, window_ms, now_ms(), header, body);
            let ev = ws::evicted_by(&before, &conn, &sessions);
            (r, ev)
        };
        // ★옛 연결에 `LEAVE` `2010` 을 보내고 **소켓을 실제로 닫는다** — 한 세션에 소켓 하나다.
        if let Some(ev) = evicted {
            let key = match &ev {
                ws::Evicted::Session(id) | ws::Evicted::Socket(id) => id.clone(),
            };
            let victim = hub.sockets.lock().await.remove(&key).map(|(_, v)| v);
            if let Some(v) = victim {
                let mut buf = Vec::new();
                let n = oxsig::body::session::LeaveNotice::new(oxsig::Code::DuplicateSession);
                send_leave(&v, &mut buf, &n).await;
            }
        }
        // 붙었으면 이 소켓을 장부에 올린다 — 다음 축출의 대상이 된다.
        if let Conn::Bound { session_id } = &conn {
            let mut socks = hub.sockets.lock().await;
            socks.insert(session_id.clone(), (conn_gen, tx.clone()));
        }
        match reply {
            Reply::Ok { header, body } | Reply::Fail { header, body } => {
                frame::encode(&mut out, header, &body);
                if tx.send(Out::Frame(out.clone())).await.is_err() {
                    break;
                }
            }
            Reply::Silent => {}
            Reply::Close(n) => {
                send_leave(&tx, &mut out, &n).await;
                break;
            }
        }
    }
    if let Conn::Bound { session_id } = &conn {
        let mut socks = hub.sockets.lock().await;
        // ★★**내가 아직 그 세션의 소켓일 때만 거둔다.** 축출된 옛 연결도 `Bound` 인 채로
        //   여기까지 온다 — 세대를 안 보면 그것이 **새 주인의 자리를 지우고 산 세션을
        //   죽은 것으로 표시한다**(다음 `BIND` 가 축출할 상대를 못 찾는다).
        if socks.get(session_id).is_some_and(|(g, _)| *g == conn_gen) {
            socks.remove(session_id);
            drop(socks);
            // ★소켓이 죽었다 — 세션은 창 동안 산다(정§3-1).
            hub.sessions.lock().await.on_socket_dead(session_id, now_ms());
        }
    }
    drop(tx);
    let _ = pump.await;
}

/// ★**사유는 `LEAVE` 가 나른다** — WS Close 에 싣지 않는다(전송을 바꿔도 이 op 은 그대로 선다).
async fn send_leave(
    tx: &tokio::sync::mpsc::Sender<Out>,
    out: &mut Vec<u8>,
    notice: &oxsig::body::session::LeaveNotice,
) {
    use oxsig::frame::{self, Header, Kind};
    let body = serde_json::to_vec(notice).unwrap_or_default();
    frame::encode(out, Header::new(Kind::Request, oxsig::Op::Leave, 0), &body);
    // ★보내고 닫는다 — 응답을 기다리지 않는다(연§6-1 절차). 두 줄의 순서가 계약이다.
    let _ = tx.send(Out::Frame(out.clone())).await;
    let _ = tx.send(Out::Close).await;
}
