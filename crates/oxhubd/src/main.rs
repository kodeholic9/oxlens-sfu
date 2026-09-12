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
use oxhubd::ledger::{Ledger, Record};
use oxhubd::session::Sessions;
use oxhubd::supervisor::{Action, Supervisor};
use oxhubd::token::{self, IssueReq};
use oxhubd::ws::{self, Conn, Reply};
use oxsig::Code;
use tokio::sync::Mutex;

struct Hub {
    resolved: Resolved,
    sup: Mutex<Supervisor>,
    sessions: Mutex<Sessions>,
    /// ★**살아 있는 자식들.** ★**여기 붙들고 있는 동안 부모 생존 채널이 열려 있다** —
    /// hub 가 어떤 방식으로 죽든 프로세스가 사라지면 파이프의 쓰기 끝이 닫히고
    /// 자식은 EOF 를 본다(정§15-6).
    children: Mutex<Vec<(String, std::process::Child)>>,
    /// ★**대장이다 — 명단이 아니다.** 명단·`seq` 의 권위는 sfud 다(정§14-1).
    rooms: Mutex<Ledger>,
    /// ★**살아 있는 소켓** — 축출된 옛 연결에 `LEAVE` 를 보내려면 그 소켓을 붙들고 있어야 한다.
    ///
    /// ★**통보만 하고 유령으로 남기지 않는다**(정§3-2 #3) — 남기면 명단에 같은 사람이 둘이고
    /// 옛 소켓의 하트비트 계수가 계속 오른다.
    /// ★**값에 소켓 세대를 같이 둔다** — 세션 하나를 이어 쥐는 소켓이 갈리므로
    /// *"이 자리의 주인이 아직 나인가"* 를 묻지 못하면 옛 연결이 새 주인을 지운다.
    sockets: Mutex<std::collections::BTreeMap<String, (u64, tokio::sync::mpsc::Sender<Out>)>>,
    /// 소켓 세대 발급기 — ★**락을 잡지 않는다**(핫패스 규율 H2).
    next_conn: std::sync::atomic::AtomicU64,
    /// ★**로컬 멤버 장부** — `room_id` → 이 hub 에 붙은 세션들. 통지를 흘릴 대상이다.
    ///
    /// ★**명단이 아니다**(그것은 sfud 것이다) — 여기 있는 것은 *"내 소켓 중 누가 그 방을
    /// 듣고 있나"* 하나다. 그래서 투명 참가자도 들어온다(받을 것은 받는다, 정§4-2).
    members: Mutex<std::collections::BTreeMap<String, Vec<String>>>,
    /// 통지 `pid` 발급기 — ★**hub 것이다**(흐름 창이 hub 의 것이므로, 연§3-2).
    next_pid: std::sync::atomic::AtomicU32,
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
        rooms: Mutex::new(Ledger::new()),
        sockets: Mutex::new(Default::default()),
        next_conn: std::sync::atomic::AtomicU64::new(1),
        members: Mutex::new(Default::default()),
        next_pid: std::sync::atomic::AtomicU32::new(1),
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
                    {
                        let mut s = ticker.sup.lock().await;
                        s.on_ready(&id, epoch);
                    }
                    // ★붙은 그 자리에서 통지 스트림을 연다 — ★**관심 선언이 먼저**(정§15-4).
                    spawn_notice_pump(ticker.clone(), addr.clone()).await;
                }
            }

            // ★대장 화해 — 밀고 받는다. ★**만료 판정은 sfud 가 한다**(명단을 그쪽이 쥔다).
            reconcile_rooms(&ticker).await;

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
    // ★목록도 조회 op 이다 — 노드마다 한 번씩 묻는다(정§15-5 fan-out).
    reconcile_rooms(&hub).await;
    let rooms = hub.rooms.lock().await;
    // ★현황(`user_count`·`rec`)은 sfud 가 준 값이다 — 없으면 그 필드가 없다(지어내지 않는다).
    let ids: Vec<String> = rooms.iter().map(|r| r.id.clone()).collect();
    let list: Vec<serde_json::Value> = ids.iter().filter_map(|id| rooms.view(id)).collect();
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
    // ★멱등 — 같은 id 로 다시 부르면 그 방이 그대로 온다(`name` 은 무시).
    let r = rooms.create(Record {
        id: id.clone(),
        name,
        capacity,
        unused_ttl_secs: req.unused_ttl_secs,
        departure_ttl_secs: req.departure_ttl_secs,
        created_at: now_ms(),
    });
    let body = serde_json::json!({
        "room_id": r.id,
        "name": r.name,
        "capacity": r.capacity,
        "created_at": r.created_at,
    });
    drop(rooms);
    // ★**그 자리에서 민다** — tick 을 기다리면 갓 만든 방의 `ROOM_JOIN` 이 `3001` 을 본다.
    sync_room(&hub, &id).await;
    Ok((StatusCode::CREATED, Json(body)))
}

/// `GET /rooms/{room_id}` — ★**방 상세.** 없으면 `3001`(그 방이 없는 것이지 서버 사정이 아니다).
///
/// ★**미입장 방도 조회를 허용한다** — 막으면 사용자가 채널을 눈감고 고른다(연§5-5).
async fn room_detail(
    State(hub): State<Shared>,
    axum::extract::Path(room_id): axum::extract::Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
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
    // ★**그 자리에서 묻는다** — 명단·`seq` 의 권위는 sfud 다(정§15-5).
    if hub.rooms.lock().await.get(&room_id).is_some() {
        sync_room(&hub, &room_id).await;
    }
    let rooms = hub.rooms.lock().await;
    let Some(mut r) = rooms.view(&room_id) else {
        return Err(fail(oxsig::Code::RoomNotFound));
    };
    // ★**트랙은 물어봤을 때만 나간다**(연§5-5 `?tracks=1`) — 채널 고르기에 쓰는 미리보기가
    //   방마다 트랙 목록을 끌고 다니지 않게 한다.
    if q.get("tracks").map(String::as_str) != Some("1")
        && let Some(o) = r.as_object_mut()
    {
        o.remove("tracks");
    }
    // ★대장 + sfud 현황 그대로 — 명단·`version` 도 그 안에 있다(없으면 없는 대로).
    Ok(Json(r))
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
        // ★들어온 `flags=01` 은 통지의 **ACK** 다 — 요청이 아니라 답이다(연§3-2). 삼킨다.
        if header.kind == frame::Kind::Ok {
            continue;
        }
        // ★**방 축은 그 방의 sfud 가 답한다** — hub 는 재해석하지 않는다(정§15-5).
        if routed(header.op) {
            let wire = match &conn {
                Conn::Bound { session_id } => {
                    route_frame(&hub, &session_id.clone(), header, body).await
                }
                // ★`BIND` 전의 다른 op 은 받지 않는다 — 인증 전 프레임 큐를 두지 않는다.
                Conn::Unbound { .. } => fail_wire(header, Code::NotBound),
            };
            if tx.send(Out::Frame(wire)).await.is_err() {
                break;
            }
            continue;
        }
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
        // ★소켓이 갔으면 흘릴 곳이 없다 — 로컬 멤버 장부에서 뺀다(sfud 의 명단은 그대로다:
        //   세션은 창 동안 살고, 이어받으면 다시 붙는다, 정§3-1).
        let mut m = hub.members.lock().await;
        for v in m.values_mut() {
            v.retain(|s| s != session_id);
        }
        drop(m);
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

// ─── B 평면 결선 — ★**wire 는 그대로 통과한다**(정§15-5) ──────────────────────

/// 배치 후보. ★**`node` 토큰이 아직 없어 유닛 축으로 선다** — 정§16-1 이 가른 두 축 중
/// 노드 축(zenoh)은 다음 덩어리다. 그때까지 *"이 hub 가 띄운 유닛이 `Running` 인가"* 로 읽는다.
async fn sfu_nodes(hub: &Shared) -> Vec<oxhubd::route::Node> {
    let sup = hub.sup.lock().await;
    hub.resolved
        .system
        .units
        .iter()
        .filter(|u| u.role == "sfu" && u.enabled && !u.addr.is_empty())
        .map(|u| oxhubd::route::Node {
            node_id: u.id.clone(),
            live: sup
                .units
                .iter()
                .any(|x| x.id == u.id && x.state == oxhubd::supervisor::UnitState::Running),
            gone_at: None,
        })
        .collect()
}

/// 그 방을 맡은 유닛의 gRPC 주소. ★**맵에 없으면 없는 것이다** — 기본 노드 폴백 금지(정§15-5).
async fn addr_for_room(hub: &Shared, room_id: &str) -> Option<String> {
    let nodes = sfu_nodes(hub).await;
    let n = oxhubd::route::place(&nodes, room_id, now_ms())?;
    hub.resolved
        .system
        .units
        .iter()
        .find(|u| u.id == n.node_id)
        .map(|u| u.addr.clone())
}

type BClient = common::b::sfu_service_client::SfuServiceClient<tonic::transport::Channel>;

async fn b_client(addr: &str) -> Option<BClient> {
    BClient::connect(format!("http://{addr}")).await.ok()
}

/// 대장 한 줄을 그 방의 node 로 밀고 현황을 받아 온다 — ★**한 왕복에 둘 다.**
///
/// ★**조회는 그때 묻는다**(정§15-5 조회 op) — tick 캐시로 답하면 방금 들어온 사람이
/// 목록에 없고(`user_count` 0 · `seq` 0), *"아직 못 물어봤다"* 와 *"정말 비었다"* 가 한 값이 된다.
/// ★생성 직후에도 민다 — 안 그러면 갓 만든 방의 `ROOM_JOIN` 이 `3001` 을 본다.
async fn sync_room(hub: &Shared, id: &str) {
    let rec = {
        let l = hub.rooms.lock().await;
        l.get(id).map(|r| common::b::RoomRecord {
            room_id: r.id.clone(),
            name: r.name.clone(),
            capacity: r.capacity,
            unused_ttl_secs: r.unused_ttl_secs,
            departure_ttl_secs: r.departure_ttl_secs,
        })
    };
    let (Some(rec), Some(addr)) = (rec, addr_for_room(hub, id).await) else {
        return;
    };
    let Some(mut c) = b_client(&addr).await else { return };
    let led = common::b::RoomLedger {
        node_id: hub.resolved.node_id.clone(),
        put: vec![rec],
        drop: Vec::new(),
    };
    if let Ok(v) = c.rooms(led).await {
        absorb_view(hub, v.into_inner()).await;
    }
}

/// sfud 가 준 현황을 대장에 덧씌운다. ★**만료는 sfud 가 판정한다** — hub 는 지우기만 한다.
async fn absorb_view(hub: &Shared, v: common::b::RoomView) {
    let mut l = hub.rooms.lock().await;
    for line in &v.rooms {
        if let Ok(serde_json::Value::Object(o)) = serde_json::from_str::<serde_json::Value>(line)
            && let Some(serde_json::Value::String(id)) = o.get("room_id")
        {
            let id = id.clone();
            l.put_live(&id, serde_json::Value::Object(o));
        }
    }
    for id in &v.expired {
        l.remove(id);
    }
}

/// 프레임 하나를 그 방의 sfud 에 넘기고 응답 wire 를 그대로 받아 온다.
async fn to_sfu(hub: &Shared, room_id: &str, env: common::b::Envelope) -> Result<Vec<u8>, Code> {
    let addr = addr_for_room(hub, room_id).await.ok_or(Code::SfuUnavailable)?;
    let mut c = b_client(&addr).await.ok_or(Code::SfuUnavailable)?;
    // ★**타임아웃의 뜻은 "살아 있는데 느림"** 하나다 — 생존은 위에서 이미 갈렸다(정§15-5).
    let r = c.handle(env).await.map_err(|_| Code::SfuUnavailable)?;
    Ok(r.into_inner().wire)
}

/// sfud 통지 스트림을 받아 ★**내 로컬 멤버에게만** 흘린다(정§15-4).
async fn spawn_notice_pump(hub: Shared, addr: String) {
    tokio::spawn(async move {
        let Some(mut c) = b_client(&addr).await else { return };
        let req = common::b::SubscribeRequest { hub_id: hub.resolved.node_id.clone() };
        let Ok(stream) = c.subscribe(req).await else { return };
        let mut stream = stream.into_inner();
        loop {
            use tokio_stream::StreamExt;
            let Some(Ok(env)) = stream.next().await else { break };
            deliver(&hub, &env).await;
        }
        eprintln!("[b] 통지 스트림 끊김 ← {addr}");
    });
}

/// 통지 한 장을 그 방의 로컬 멤버에게. ★**`exclude` 는 받는 쪽이 적용한다**(정§15-4).
///
/// ★**`target` 이 있으면 그 사람에게만** 간다(정§15-4 가름) — 방 명단을 거치지 않는다.
/// 회수 통지(`media_lost`)가 그 길이다: 그 사람은 ★**이미 명단에서 빠진 뒤**라 방으로
/// 흘리면 닿지 않는다.
async fn deliver(hub: &Shared, env: &common::b::Envelope) {
    if !env.target.is_empty() {
        deliver_to_user(hub, env).await;
        return;
    }
    let targets: Vec<String> = {
        let m = hub.members.lock().await;
        m.get(&env.room_id).cloned().unwrap_or_default()
    };
    if targets.is_empty() {
        return;
    }
    // ★`exclude` 는 사용자 축이다 — 세션을 사용자로 되짚어 거른다.
    let excluded: Vec<String> = {
        let s = hub.sessions.lock().await;
        targets
            .iter()
            .filter(|sid| {
                s.get(sid).is_some_and(|x| env.exclude.iter().any(|u| u == &x.user_id))
            })
            .cloned()
            .collect()
    };
    // ★**`pid` 는 여기서 매긴다** — sfud 가 매기면 두 곳이 번호를 내어 창이 어긋난다.
    let Ok((h, body)) = oxsig::frame::decode(&env.wire) else { return };
    let body = body.to_vec();
    let socks = hub.sockets.lock().await;
    for sid in targets.iter().filter(|s| !excluded.contains(s)) {
        let Some((_, tx)) = socks.get(sid) else { continue };
        let pid = hub.next_pid.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut out = Vec::with_capacity(oxsig::frame::HEADER_LEN + body.len());
        oxsig::frame::encode(&mut out, oxsig::frame::Header::new(h.kind, h.op, pid), &body);
        let _ = tx.send(Out::Frame(out)).await;
    }
}

/// 대장을 그 방의 node 마다 몰아 밀고 현황을 받아 온다. ★**만료는 sfud 가 판정한다.**
async fn reconcile_rooms(hub: &Shared) {
    use std::collections::BTreeMap;
    let ids: Vec<String> = { hub.rooms.lock().await.iter().map(|r| r.id.clone()).collect() };
    let mut by_addr: BTreeMap<String, Vec<common::b::RoomRecord>> = BTreeMap::new();
    for id in &ids {
        let Some(addr) = addr_for_room(hub, id).await else { continue };
        let l = hub.rooms.lock().await;
        if let Some(r) = l.get(id) {
            by_addr.entry(addr).or_default().push(common::b::RoomRecord {
                room_id: r.id.clone(),
                name: r.name.clone(),
                capacity: r.capacity,
                unused_ttl_secs: r.unused_ttl_secs,
                departure_ttl_secs: r.departure_ttl_secs,
            });
        }
    }
    for (addr, put) in by_addr {
        let Some(mut c) = b_client(&addr).await else { continue };
        let led = common::b::RoomLedger {
            node_id: hub.resolved.node_id.clone(),
            put,
            drop: Vec::new(),
        };
        if let Ok(v) = c.rooms(led).await {
            absorb_view(hub, v.into_inner()).await;
        }
    }
}

/// 그 op 이 sfud 로 가는가 — ★**방 축은 전부 간다**(정§15-5 *"wire 는 그대로 통과"*).
fn routed(op: oxsig::Op) -> bool {
    matches!(op, oxsig::Op::RoomJoin | oxsig::Op::RoomLeave | oxsig::Op::Affiliation)
}

/// 그 프레임이 가리키는 방. ★**body 를 한 번만 읽는다** — 두 번 읽으면 갈린다.
fn room_of(op: oxsig::Op, body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let pick = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    match op {
        oxsig::Op::RoomJoin | oxsig::Op::RoomLeave => pick("room_id"),
        // ★한 요청이 두 방을 바꿀 수 있다 — 라우팅은 `pub_select` 가 먼저다(연§6-4).
        oxsig::Op::Affiliation => pick("pub_select").or_else(|| pick("pub_deselect")),
        _ => None,
    }
}

/// 그 사람의 소켓에만. ★**생존을 판정하지 않는다**(정§17-2 ⑧) — 붙어 있으면 닿고,
/// 아니면 사라진다. 사라진 것은 재접속의 `RESUME` 스냅샷이 답한다.
async fn deliver_to_user(hub: &Shared, env: &common::b::Envelope) {
    let sessions: Vec<String> = {
        hub.sessions.lock().await.sessions_of(&env.target)
    };
    if sessions.is_empty() {
        return;
    }
    // ★그 사람은 그 방에서 이미 빠졌다 — 로컬 장부도 따라간다(안 따라가면 유령이 남는다).
    if !env.room_id.is_empty() {
        let mut m = hub.members.lock().await;
        if let Some(v) = m.get_mut(&env.room_id) {
            v.retain(|s| !sessions.contains(s));
        }
    }
    let Ok((h, body)) = oxsig::frame::decode(&env.wire) else { return };
    let body = body.to_vec();
    let socks = hub.sockets.lock().await;
    for sid in &sessions {
        let Some((_, tx)) = socks.get(sid) else { continue };
        let pid = hub.next_pid.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut out = Vec::with_capacity(oxsig::frame::HEADER_LEN + body.len());
        oxsig::frame::encode(&mut out, oxsig::frame::Header::new(h.kind, h.op, pid), &body);
        let _ = tx.send(Out::Frame(out)).await;
    }
}

fn fail_wire(header: oxsig::frame::Header, code: Code) -> Vec<u8> {
    use oxsig::frame::{self, Header, Kind};
    let body = serde_json::to_vec(&oxsig::Failure::new(code)).unwrap_or_default();
    let mut out = Vec::with_capacity(frame::HEADER_LEN + body.len());
    frame::encode(&mut out, Header { kind: Kind::Fail, ..header }, &body);
    out
}

/// 프레임 하나를 그 방의 sfud 로 넘긴다. ★**신원은 hub 세션이 주입한다** — body 를 믿지 않는다.
async fn route_frame(
    hub: &Shared,
    session_id: &str,
    header: oxsig::frame::Header,
    body: &[u8],
) -> Vec<u8> {
    use oxsig::frame::{self, Kind};
    let Some(room_id) = room_of(header.op, body) else {
        return fail_wire(header, Code::MissingField);
    };
    // ★**hub 가 아는 방 전량이 곧 맵이다** — 없으면 방이 없는 것이다(정§15-5).
    if hub.rooms.lock().await.get(&room_id).is_none() {
        return fail_wire(header, Code::RoomNotFound);
    }
    let Some(sess) = hub.sessions.lock().await.get(session_id).cloned() else {
        return fail_wire(header, Code::SessionNotFound);
    };
    let mut wire = Vec::with_capacity(frame::HEADER_LEN + body.len());
    frame::encode(&mut wire, header, body);
    let env = common::b::Envelope {
        session_id: session_id.to_string(),
        // ★**impersonation 방어** — 이 값이 body 에서 오면 남을 사칭할 수 있다(정§15-5).
        user_id: sess.user_id.clone(),
        room_id: room_id.clone(),
        pc_mode: match sess.pc_mode {
            oxsig::body::session::PcMode::One => "1pc".into(),
            oxsig::body::session::PcMode::Two => "2pc".into(),
        },
        participant_type: sess.participant_type as u32,
        hidden: sess.hidden,
        ..Default::default()
    };
    let out = match to_sfu(hub, &room_id, common::b::Envelope { wire, ..env }).await {
        Ok(v) => v,
        Err(c) => return fail_wire(header, c),
    };
    // ★**로컬 멤버 장부는 성공한 것만 따라간다** — 실패를 따라가면 흘릴 곳이 어긋난다.
    if let Ok((h, _)) = frame::decode(&out)
        && h.kind == Kind::Ok
    {
        let mut m = hub.members.lock().await;
        match header.op {
            oxsig::Op::RoomJoin => {
                let v = m.entry(room_id).or_default();
                if !v.iter().any(|s| s == session_id) {
                    v.push(session_id.to_string());
                }
            }
            oxsig::Op::RoomLeave => {
                if let Some(v) = m.get_mut(&room_id) {
                    v.retain(|s| s != session_id);
                }
            }
            _ => {}
        }
    }
    out
}
