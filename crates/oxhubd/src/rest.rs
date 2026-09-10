// author: kodeholic (powered by Claude)
//! 클라 규격 안의 HTTP — 연§5-1 인증 두 갈래 · §5-2 `/auth/token`(정§3-4) · §5-3 `GET /rooms` · §5-4 `POST /rooms` · §5-5 `GET /rooms/{id}`.
//! HTTP 실패 body 는 전부 연§4-5 `Failure`. 방 요청은 B 평면 내부 op 로 sfud 에 대행한다.

use std::sync::Arc;

use std::net::SocketAddr;

use std::time::Duration;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use common::auth;
use common::bplane::{self, iop};
use common::config::{HubAuth, PolicyConfig, SystemConfig};
use oxsig::frame::{self, Header, Kind};
use oxsig::{FailCode, Failure};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tower_http::cors::{AllowOrigin, CorsLayer};
use serde_json::{Value, json};

use crate::backend::SfuBackend;
use crate::session::SessionRegistry;
use crate::supervisor::Supervisor;

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub api_key: String,
    pub api_secret: String,
    pub user_id: String,
    /// 연§5-2 — `0` 사람(기본) · `1` 녹화 · `2` 봇. 계정이 허용한 값만 서명된다.
    #[serde(default)]
    pub participant_type: u8,
    /// 명단·통지·정원에서 빠진다. ★클라가 스스로 켜는 경로는 없다 — 여기가 유일한 출처다.
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct TokenResponse {
    pub token: String,
    pub expires_in: u64,
}

type HttpFail = (StatusCode, Failure);

/// 정§3-4 판정 — `2004`(401) · `1002`/`2005`(400) · `5003`(500).
/// ★발급 경로는 이것 하나다 — 운영 토큰은 발급하지 않는다(§16-1-1).
pub fn issue_token(auth_cfg: &HubAuth, ttl_secs: u64, metadata_max: u32, req: &TokenRequest, now: i64) -> Result<TokenResponse, HttpFail> {
    let Some(account) = auth_cfg.api_keys.iter().find(|k| k.key == req.api_key && k.secret == req.api_secret) else {
        return Err((StatusCode::UNAUTHORIZED, Failure::new(FailCode::InvalidApiKey)));
    };
    if !auth::is_known_participant_type(req.participant_type) {
        return Err((StatusCode::BAD_REQUEST,
            Failure::new(FailCode::InvalidPayload).message(format!("unknown participant_type {}", req.participant_type))));
    }
    if !account.participant_types.contains(&req.participant_type) || (req.hidden && !account.hidden_allowed) {
        return Err((StatusCode::BAD_REQUEST, Failure::new(FailCode::InvalidRole).message("claim not allowed for this account")));
    }
    if let Some(m) = &req.metadata
        && metadata_max > 0
        && serde_json::to_vec(m).map(|v| v.len()).unwrap_or(usize::MAX) > metadata_max as usize
    {
        return Err((StatusCode::BAD_REQUEST,
            Failure::new(FailCode::InvalidPayload).message(format!("metadata exceeds {metadata_max} bytes"))));
    }
    auth::issue(&auth_cfg.jwt_secret, &req.user_id, req.participant_type, req.hidden, req.metadata.clone(), ttl_secs, now)
        .map(|i| TokenResponse { token: i.token, expires_in: i.expires_in })
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Failure::new(FailCode::InternalError).message(e.to_string())))
}

pub struct RestState {
    pub system: SystemConfig,
    pub policy: PolicyConfig,
    pub registry: Arc<SessionRegistry>,
    pub backend: Arc<SfuBackend>,
    /// 정§16-1 유닛 평면의 8상태 출처. supervisor 가 꺼져 있으면 비어 있다.
    pub supervisor: Arc<Mutex<Supervisor>>,
}

/// 정§16-1 `/admin/*` — 유닛 평면. 이 hub 가 ★지금 보고 있는 노드 목록이다(설정 파일이 아니라).
/// ★loopback 은 통과시킨다(XFF 를 믿지 않는다 — 리버스 프록시 뒤 배치 금지가 전제).
pub async fn admin_sfus(State(st): State<Arc<RestState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> axum::response::Response {
    if let Some(deny) = guard(&st, &peer, &headers) {
        return deny;
    }
    let (sfus, supervising) = sfu_rows(&st).await;
    respond(Ok((StatusCode::OK, json!({ "sfus": sfus, "supervising": supervising }))))
}

/// ★유닛 한 줄의 정본 — 유닛 평면과 스냅샷 평면이 같은 것을 봐야 한다(두 곳에 적으면 갈린다).
/// ★`live` 는 **배치 가능**과 같은 값이다(정§16-1) — dial 이 아니라 이벤트 스트림이 섰는가다.
/// supervisor 가 무엇으로 적었느냐는 다른 물음이라 둘 다 낸다.
async fn sfu_rows(st: &Arc<RestState>) -> (Vec<Value>, bool) {
    let states = st.supervisor.lock().await.states();
    let mut sfus: Vec<Value> = Vec::new();
    for node in st.backend.nodes.all() {
        let mut row = json!({ "sfu_id": node.id, "addr": node.addr, "live": node.is_live() });
        // supervisor 가 안 쥔 유닛은 상태 칸 자체가 없다 — 모르는 것을 지어내지 않는다(연§6-6).
        if let Some((_, state)) = states.iter().find(|(id, _)| id == &node.id)
            && let Some(map) = row.as_object_mut()
        {
            map.insert("unit_state".into(), Value::String(format!("{state:?}")));
        }
        sfus.push(row);
    }
    (sfus, !states.is_empty())
}

/// 정§16-1 방 평면 — 배치와 배달 명단. ★정본은 sfud 이고 이것은 hub 가 **보고 있는 것**이다.
pub async fn admin_rooms(State(st): State<Arc<RestState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> axum::response::Response {
    if let Some(deny) = guard(&st, &peer, &headers) {
        return deny;
    }
    let rooms: Vec<Value> = st.backend.rooms.placements().into_iter()
        .map(|(room_id, node_id)| {
            let members = st.backend.members.members(&room_id);
            json!({ "room_id": room_id, "sfu_id": node_id, "members": members.len(), "member_ids": members })
        })
        .collect();
    respond(Ok((StatusCode::OK, json!({ "rooms": rooms, "total": rooms.len() }))))
}

/// 정§16-2 관측 평면 — 사유별 drop 계수. ★유닛마다 따로 낸다(합치면 어느 유닛인지 잃는다).
/// 실패 노드는 그 자리에 `error` 를 적는다 — 빠뜨리면 "0 건"으로 읽힌다.
pub async fn admin_drops(State(st): State<Arc<RestState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> axum::response::Response {
    if let Some(deny) = guard(&st, &peer, &headers) {
        return deny;
    }
    let mut units: Vec<Value> = Vec::new();
    for node in st.backend.nodes.all() {
        let env = bplane::Envelope { session_id: String::new(), user_id: String::new(), room_id: String::new(), target: String::new(), exclude: Vec::new(), wire: internal_wire(iop::SFU_STATS, &Value::Null), pc_mode: String::new(), participant_type: 0, hidden: false, metadata: String::new() };
        let row = match st.backend.send_to_node(&node.id, env).await.map_err(fail_of).and_then(|w| unwrap_wire(&w)) {
            Ok(mut v) => {
                if let Some(map) = v.as_object_mut() {
                    map.insert("sfu_id".into(), Value::String(node.id.clone()));
                }
                v
            }
            Err((_, f)) => json!({ "sfu_id": node.id, "error": f.code }),
        };
        units.push(row);
    }
    respond(Ok((StatusCode::OK, json!({ "units": units }))))
}

/// 정§16-1 사용자 평면 — 붙어 있는 세션. ★토큰·시크릿은 안 낸다.
pub async fn admin_users(State(st): State<Arc<RestState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> axum::response::Response {
    if let Some(deny) = guard(&st, &peer, &headers) {
        return deny;
    }
    let users: Vec<Value> = st.registry.snapshot().into_iter()
        .map(|(session_id, user_id, participant_type, hidden)|
            json!({ "session_id": session_id, "user_id": user_id, "participant_type": participant_type, "hidden": hidden }))
        .collect();
    respond(Ok((StatusCode::OK, json!({ "users": users, "total": users.len() }))))
}

/// 정§16-1 스냅샷 평면 — 위 셋을 한 번에. ★운영자가 세 번 묻지 않게 하는 자리다.
pub async fn admin_snapshot(State(st): State<Arc<RestState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> axum::response::Response {
    if let Some(deny) = guard(&st, &peer, &headers) {
        return deny;
    }
    let (sfus, supervising) = sfu_rows(&st).await;
    let ready = sfus.iter().all(|r| r["live"] == Value::Bool(true));
    respond(Ok((StatusCode::OK, json!({
        "ready": ready,
        "build": common::build::stamp(),
        "supervising": supervising,
        "sfus": sfus,
        "rooms": st.backend.rooms.placements().len(),
        "users": st.registry.snapshot().len(),
    }))))
}

/// 정§16-1 — 토큰 role="admin" ★또는 loopback. ★XFF 를 믿지 않는다(리버스 프록시 뒤 배치 금지가 전제).
fn guard(st: &Arc<RestState>, peer: &SocketAddr, headers: &HeaderMap) -> Option<axum::response::Response> {
    if admin_ok(&st.system.hub.auth, peer, headers) {
        return None;
    }
    Some(respond(Err((StatusCode::UNAUTHORIZED, Failure::new(FailCode::NotAuthorized).message("admin only")))))
}

/// ★판정만 하는 자리 — 응답을 안 만들기 때문에 1층이 경계 자체를 시험할 수 있다.
fn admin_ok(auth: &HubAuth, peer: &SocketAddr, headers: &HeaderMap) -> bool {
    peer.ip().is_loopback() || is_admin(auth, headers)
}

/// 정§16-1-1 운영 토큰 — 운영자가 자기 `api_secret` 으로 서명한 것을 검증만 한다.
/// ★A 평면 사용자 토큰은 여기를 열지 못한다(그 토큰은 `jwt_secret` 으로 서명되고 `ops` 도 없다).
fn is_admin(auth_cfg: &HubAuth, headers: &HeaderMap) -> bool {
    let Some(token) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    let Some(iss) = auth::ops_issuer(token) else { return false };
    let Some(account) = auth_cfg.api_keys.iter().find(|k| k.key == iss) else { return false };
    account.ops_allowed && auth::verify_ops(&account.secret, token).is_ok_and(|c| c.ops)
}

/// 연§5-1 — 브라우저 클라가 부르는 세 자리에만 붙인다.
///
/// ★`X-OxLens-Session` 이 안전목록 밖이라 매 요청 앞에 preflight 가 붙는다 — 헤더를 허용하고
/// `max_age` 로 왕복을 줄인다. 쿠키를 안 쓰므로(Bearer·세션 헤더) 자격증명은 허용하지 않는다:
/// 자격증명 없는 요청은 ambient 권한이 없어 `*` 로도 남의 페이지가 읽을 것이 없다.
///
/// 빈 목록이면 `None` — 헤더를 아예 안 낸다(같은 origin 배포 전제).
pub fn cors(origins: &[String]) -> Option<CorsLayer> {
    if origins.is_empty() {
        return None;
    }
    let allow = if origins.iter().any(|o| o == "*") {
        AllowOrigin::any()
    } else {
        let list: Vec<HeaderValue> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        if list.is_empty() {
            return None;
        }
        AllowOrigin::list(list)
    };
    Some(
        CorsLayer::new()
            .allow_origin(allow)
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([AUTHORIZATION, CONTENT_TYPE, HeaderName::from_static(SESSION_HEADER)])
            .max_age(Duration::from_secs(CORS_MAX_AGE_SECS)),
    )
}

/// preflight 를 얼마나 재사용하나 — 재동기가 갭마다 도는 자리라 왕복이 값지다.
const CORS_MAX_AGE_SECS: u64 = 600;

/// 연§5-1 세션 헤더. 이 이름이 안전목록 밖이라 preflight 를 부른다.
const SESSION_HEADER: &str = "x-oxlens-session";

/// 연§5-1 — `Authorization: Bearer` 또는 `X-OxLens-Session`. 둘 중 하나면 통과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub user_id: String,
    pub via_session: Option<String>,
}

pub fn authenticate(secret: &str, registry: &SessionRegistry, headers: &HeaderMap) -> Result<Identity, HttpFail> {
    if let Some(sid) = headers.get(SESSION_HEADER).and_then(|v| v.to_str().ok()) {
        return registry
            .get(sid)
            .map(|s| Identity { user_id: s.user_id, via_session: Some(s.id) })
            .ok_or((StatusCode::UNAUTHORIZED, Failure::new(FailCode::SessionNotFound)));
    }
    let bearer = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    let Some(token) = bearer else {
        return Err((StatusCode::UNAUTHORIZED, Failure::new(FailCode::TokenInvalid).message("no credential")));
    };
    auth::verify(secret, token)
        .map(|c| Identity { user_id: c.sub, via_session: None })
        .map_err(|e| {
            let code = match e {
                auth::VerifyError::Expired => FailCode::TokenExpired,
                auth::VerifyError::Invalid => FailCode::TokenInvalid,
            };
            (StatusCode::UNAUTHORIZED, Failure::new(code))
        })
}

#[derive(Debug, Deserialize)]
pub struct CreateRoomReq {
    #[serde(default)]
    pub room_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub capacity: Option<u32>,
    #[serde(default)]
    pub unused_ttl_secs: Option<u64>,
    #[serde(default)]
    pub departure_ttl_secs: Option<u64>,
}

/// 연§5-4 형상 검사 + 기본값 채움(정책서 `hub.room_*`). 결과 body 가 sfud 로 간다.
pub fn create_room_body(req: &CreateRoomReq, policy: &PolicyConfig) -> Result<(String, Value), HttpFail> {
    let name = req.name.as_deref().filter(|n| !n.is_empty()).ok_or((StatusCode::BAD_REQUEST, Failure::new(FailCode::MissingField).message("name")))?;
    let room_id = match req.room_id.as_deref().filter(|r| !r.is_empty()) {
        Some(r) if r.len() > oxsig::mbcp::MAX_TLV_VALUE_LEN => {
            return Err((StatusCode::BAD_REQUEST, Failure::new(FailCode::InvalidPayload).message("room_id over 255 bytes")));
        }
        Some(r) => r.to_owned(),
        None => uuid::Uuid::new_v4().to_string(),
    };
    let capacity = req.capacity.unwrap_or(policy.hub.room_default_capacity);
    if !(1..=1_000).contains(&capacity) {
        return Err((StatusCode::BAD_REQUEST, Failure::new(FailCode::InvalidPayload).message("capacity 1~1000")));
    }
    let body = json!({
        "room_id": room_id, "name": name, "capacity": capacity,
        "unused_ttl_secs": req.unused_ttl_secs.or(policy.hub.room_unused_ttl_secs.map(u64::from)),
        "departure_ttl_secs": req.departure_ttl_secs.or(policy.hub.room_departure_ttl_secs.map(u64::from)),
    });
    Ok((room_id, body))
}

fn internal_wire(op: u16, body: &Value) -> Vec<u8> {
    frame::encode_json(&Header::msg(op, 0), body)
}

/// sfud 응답 wire → (성공 body) 또는 HTTP 실패. 실패 코드는 앞자리로 상태 코드를 고른다(판단은 `code`).
fn unwrap_wire(wire: &[u8]) -> Result<Value, HttpFail> {
    let (h, body) = frame::decode(wire).map_err(|_| (StatusCode::BAD_GATEWAY, Failure::new(FailCode::SfuError)))?;
    let v = frame::body_json(body).map_err(|_| (StatusCode::BAD_GATEWAY, Failure::new(FailCode::SfuError)))?;
    if h.kind == Kind::Ok {
        return Ok(v);
    }
    let failure: Failure = serde_json::from_value(v).unwrap_or_else(|_| Failure::new(FailCode::SfuError));
    let status = match failure.code {
        3001 => StatusCode::NOT_FOUND,
        1000..=1999 => StatusCode::BAD_REQUEST,
        2000..=2999 => StatusCode::UNAUTHORIZED,
        3000..=3999 => StatusCode::CONFLICT,
        4000..=4999 => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::BAD_GATEWAY,
    };
    Err((status, failure))
}

fn fail_of(code: FailCode) -> HttpFail {
    let status = match code {
        FailCode::RoomNotFound => StatusCode::NOT_FOUND,
        FailCode::SfuUnavailable | FailCode::SfuError => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Failure::new(code))
}

fn respond(r: Result<(StatusCode, Value), HttpFail>) -> axum::response::Response {
    match r {
        Ok((status, v)) => (status, Json(v)).into_response(),
        Err((status, f)) => (status, Json(f)).into_response(),
    }
}

pub async fn token(State(st): State<Arc<RestState>>, Json(req): Json<TokenRequest>) -> axum::response::Response {
    match issue_token(&st.system.hub.auth, u64::from(st.policy.hub.token_ttl_secs), st.policy.hub.metadata_max_bytes, &req, auth::now_unix()) {
        Ok(res) => (StatusCode::OK, Json(res)).into_response(),
        Err((status, failure)) => (status, Json(failure)).into_response(),
    }
}

/// 연§5-3 — 전 노드 fan-out 병합. 실패 노드는 경고를 남기고 부분 병합(정§15-2).
pub async fn list_rooms(State(st): State<Arc<RestState>>, headers: HeaderMap) -> axum::response::Response {
    respond(async {
        authenticate(&st.system.hub.auth.jwt_secret, &st.registry, &headers)?;
        let mut rooms: Vec<Value> = Vec::new();
        for node in st.backend.nodes.all() {
            let env = bplane::Envelope { session_id: String::new(), user_id: String::new(), room_id: String::new(), target: String::new(), exclude: Vec::new(), wire: internal_wire(iop::ROOM_LIST, &Value::Null), pc_mode: String::new(), participant_type: 0, hidden: false, metadata: String::new() };
            match st.backend.send_to_node(&node.id, env).await.map_err(fail_of).and_then(|w| unwrap_wire(&w)) {
                Ok(v) => rooms.extend(v["rooms"].as_array().cloned().unwrap_or_default()),
                Err((_, f)) => tracing::warn!(node = %node.id, code = f.code, "ROOM_LIST fan-out: partial merge"),
            }
        }
        let total = rooms.len();
        Ok((StatusCode::OK, json!({ "rooms": rooms, "total": total })))
    }
    .await)
}

/// 연§5-4 — 배치는 hub 가 결정·기록(정§15-1)하고 sfud 는 명시 id 경로 하나만 탄다.
pub async fn create_room(State(st): State<Arc<RestState>>, headers: HeaderMap, Json(req): Json<CreateRoomReq>) -> axum::response::Response {
    respond(async {
        authenticate(&st.system.hub.auth.jwt_secret, &st.registry, &headers)?;
        let (room_id, body) = create_room_body(&req, &st.policy)?;
        let node_id = match st.backend.rooms.node_of(&room_id) {
            Some(n) => n,
            None => {
                // 정§15-1 — 배치는 **전 노드**를 후보로 하는 순수 함수다(HRW). 살아 있는 것만
                // 후보로 좁히면 노드 하나가 죽는 창에 그 방이 다른 노드로 옮겨 붙어, 정§15-1 의
                // "한 방은 한 sfud" 안정성이 창마다 흔들린다.
                let ids = st.backend.nodes.ids();
                let chosen = crate::route::place(&ids, &room_id).ok_or_else(|| fail_of(FailCode::SfuUnavailable))?;
                // 정§16-1 — ★그 노드의 이벤트 스트림이 서 있을 때만 배치한다. 스트림이 없으면
                // 방은 살고 통지는 갈 곳이 없어 그 창의 입퇴장·트랙 통지가 통째로 버려진다.
                // "매핑은 있는데 그 sfud 에 못 닿는다"(정§15-2)와 같은 자리라 `5001` 이다.
                if !st.backend.nodes.get(chosen).is_some_and(|n| n.is_live()) {
                    return Err(fail_of(FailCode::SfuUnavailable));
                }
                st.backend.rooms.assign(&room_id, chosen)
            }
        };
        let env = bplane::Envelope { session_id: String::new(), user_id: String::new(), room_id: room_id.clone(), target: String::new(), exclude: Vec::new(), wire: internal_wire(iop::ROOM_CREATE, &body), pc_mode: String::new(), participant_type: 0, hidden: false, metadata: String::new() };
        let out = st.backend.send_to_node(&node_id, env).await.map_err(fail_of).and_then(|w| unwrap_wire(&w));
        if out.is_err() {
            st.backend.rooms.unbind(&room_id);
        }
        out.map(|v| (StatusCode::CREATED, v))
    }
    .await)
}

#[derive(Debug, Deserialize)]
pub struct GetRoomQuery {
    #[serde(default)]
    pub tracks: u8,
}

/// 연§5-5 — 세션 헤더로 왔을 때만 그 사람이 입장한 방에 `mid` 가 채워진다(sfud 판정).
pub async fn get_room(State(st): State<Arc<RestState>>, headers: HeaderMap, Path(room_id): Path<String>, Query(q): Query<GetRoomQuery>) -> axum::response::Response {
    respond(async {
        let id = authenticate(&st.system.hub.auth.jwt_secret, &st.registry, &headers)?;
        let node_id = st.backend.rooms.node_of(&room_id).ok_or_else(|| fail_of(FailCode::RoomNotFound))?;
        let env = bplane::Envelope {
            session_id: id.via_session.clone().unwrap_or_default(),
            user_id: if id.via_session.is_some() { id.user_id.clone() } else { String::new() },
            room_id: room_id.clone(),
            target: String::new(),
            exclude: Vec::new(),
            wire: internal_wire(iop::ROOM_GET, &json!({ "room_id": room_id, "tracks": q.tracks == 1 })),
            pc_mode: String::new(),
            participant_type: 0,
            hidden: false,
            metadata: String::new(),
        };
        st.backend.send_to_node(&node_id, env).await.map_err(fail_of).and_then(|w| unwrap_wire(&w)).map(|v| (StatusCode::OK, v))
    }
    .await)
}

/// 정§16-1 — ★**프로세스 생존**. 항상 200 이고 무인증이다(probe 가 부른다).
///
/// ★`ready` 와 가르는 것이 요점이다: 이쪽이 200 인데 저쪽이 503 이면 *"떠는 있는데 일을 못 한다"*
/// 이고, 이쪽이 안 뜨면 *"죽었다"* 다. 하나로 두면 재기동해야 할 때와 기다려야 할 때가 안 갈린다.
pub async fn healthz_live() -> &'static str {
    "ok"
}

/// 정§16-1 — hub 정상 ∧ ★**enabled sfud 전부 Live** 면 200, 아니면 503.
///
/// ★설정에 있는 것이 아니라 **지금 닿는가**를 본다 — 노드 하나가 죽으면 그 방들이 안 서므로
/// 트래픽을 받으면 안 된다. 무인증이다(probe).
pub async fn healthz_ready(State(st): State<Arc<RestState>>) -> axum::response::Response {
    // ★`Live` 는 배치 가능과 같은 값이다(정§16-1) — 스트림이 안 선 노드가 있으면 트래픽을 받지 않는다.
    // dial 만 보면 재기동 창에 ready 가 200 이고 그 창에 배치된 방은 통지가 없다.
    let down: Vec<&str> = st.backend.nodes.all().iter().filter(|n| !n.is_live()).map(|n| n.id.as_str()).collect();
    let code = if down.is_empty() { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    // ★어느 노드가 빠졌는지까지 낸다 — 503 만 주면 운영자가 다시 물어봐야 한다.
    // ★어느 빌드가 돌고 있는지 같이 낸다 — 회귀가 옛 바이너리를 상대로 도는 것을 봉쇄한다.
    respond(Ok((code, json!({ "ready": down.is_empty(), "down": down, "build": common::build::stamp() }))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::config::ApiKey;
    use std::time::Duration;

    fn account(pt: Vec<u8>, hidden: bool, ops: bool) -> ApiKey {
        ApiKey { key: "k".into(), secret: "p".into(), name: String::new(), participant_types: pt, hidden_allowed: hidden, ops_allowed: ops }
    }
    fn cfg() -> HubAuth {
        HubAuth { jwt_secret: "s".into(), api_keys: vec![account(vec![auth::PT_USER, auth::PT_RECORDER], false, false)] }
    }
    fn req(key: &str, pt: u8, hidden: bool) -> TokenRequest {
        TokenRequest { api_key: key.into(), api_secret: "p".into(), user_id: "u1".into(), participant_type: pt, hidden, metadata: None }
    }

    #[test]
    fn token_judgement() {
        let now = auth::now_unix();
        let ok = issue_token(&cfg(), 3600, 2048, &req("k", auth::PT_RECORDER, false), now).unwrap();
        assert_eq!(ok.expires_in, 3600);
        let c = auth::verify("s", &ok.token).unwrap();
        assert_eq!((c.participant_type, c.hidden), (auth::PT_RECORDER, false));
        let (st, f) = issue_token(&cfg(), 3600, 2048, &req("x", auth::PT_USER, false), now).unwrap_err();
        assert_eq!((st, f.code), (StatusCode::UNAUTHORIZED, 2004));
        // 계정 허용 밖 — 종류(봇)와 투명 둘 다 2005.
        let (st, f) = issue_token(&cfg(), 3600, 2048, &req("k", auth::PT_BOT, false), now).unwrap_err();
        assert_eq!((st, f.code), (StatusCode::BAD_REQUEST, 2005));
        let (st, f) = issue_token(&cfg(), 3600, 2048, &req("k", auth::PT_USER, true), now).unwrap_err();
        assert_eq!((st, f.code), (StatusCode::BAD_REQUEST, 2005));
        // 모르는 종류는 형이 아니다 — 1002.
        let (st, f) = issue_token(&cfg(), 3600, 2048, &req("k", 9, false), now).unwrap_err();
        assert_eq!((st, f.code), (StatusCode::BAD_REQUEST, 1002));
    }

    #[test]
    fn metadata_ceiling_rejects_at_issue() {
        let now = auth::now_unix();
        let mut r = req("k", auth::PT_USER, false);
        r.metadata = Some(json!({ "name": "x".repeat(64) }));
        assert!(issue_token(&cfg(), 3600, 2048, &r, now).is_ok());
        let (st, f) = issue_token(&cfg(), 3600, 16, &r, now).unwrap_err();
        assert_eq!((st, f.code), (StatusCode::BAD_REQUEST, 1002));
        // `0` = 무제한(정책서 §2 규약).
        assert!(issue_token(&cfg(), 3600, 0, &r, now).is_ok());
    }

    /// 정§16-1-1 — A 평면 토큰은 `/admin` 을 못 연다. 운영 토큰만, 그것도 운영 허용 계정만.
    #[test]
    fn ops_token_only_opens_admin() {
        let now = auth::now_unix();
        let mut h = HeaderMap::new();
        let bearer = |h: &mut HeaderMap, t: &str| { h.insert("authorization", format!("Bearer {t}").parse().unwrap()); };

        let user_token = issue_token(&cfg(), 3600, 2048, &req("k", auth::PT_USER, false), now).unwrap().token;
        bearer(&mut h, &user_token);
        assert!(!is_admin(&cfg(), &h), "사용자 토큰이 운영을 열면 안 된다");

        let ops = |secret: &str| auth::sign_ops(secret, "k", 60, now).unwrap();
        bearer(&mut h, &ops("p"));
        assert!(!is_admin(&cfg(), &h), "운영 허용이 없는 계정은 못 연다");
        let allowed = HubAuth { jwt_secret: "s".into(), api_keys: vec![account(vec![auth::PT_USER], false, true)] };
        assert!(is_admin(&allowed, &h));
        // 계정 비밀이 아닌 것으로 서명한 것은 통과하지 못한다.
        bearer(&mut h, &ops("wrong"));
        assert!(!is_admin(&allowed, &h));
    }

    #[test]
    fn two_auth_branches() {
        let reg = SessionRegistry::new("s", Duration::from_secs(60), 10_000);
        let mut h = HeaderMap::new();
        assert_eq!(authenticate("s", &reg, &h).unwrap_err().1.code, 2002);
        let t = auth::issue("s", "u1", auth::PT_USER, false, None, 60, auth::now_unix()).unwrap().token;
        h.insert("authorization", format!("Bearer {t}").parse().unwrap());
        assert_eq!(authenticate("s", &reg, &h).unwrap().user_id, "u1");
        let mut h2 = HeaderMap::new();
        h2.insert("x-oxlens-session", "nope".parse().unwrap());
        assert_eq!(authenticate("s", &reg, &h2).unwrap_err().1.code, 2008);
    }

    #[test]
    fn create_room_shape() {
        let p = PolicyConfig::default();
        let bad = CreateRoomReq { room_id: None, name: None, capacity: None, unused_ttl_secs: None, departure_ttl_secs: None };
        assert_eq!(create_room_body(&bad, &p).unwrap_err().1.code, 1003);
        let big = CreateRoomReq { room_id: Some("x".repeat(256)), name: Some("n".into()), capacity: None, unused_ttl_secs: None, departure_ttl_secs: None };
        assert_eq!(create_room_body(&big, &p).unwrap_err().1.code, 1002);
        let over = CreateRoomReq { room_id: None, name: Some("n".into()), capacity: Some(1001), unused_ttl_secs: None, departure_ttl_secs: None };
        assert_eq!(create_room_body(&over, &p).unwrap_err().1.code, 1002);
        let ok = CreateRoomReq { room_id: Some("r1".into()), name: Some("n".into()), capacity: None, unused_ttl_secs: Some(0), departure_ttl_secs: None };
        let (id, body) = create_room_body(&ok, &p).unwrap();
        assert_eq!((id.as_str(), body["capacity"].as_u64(), body["departure_ttl_secs"].is_null()), ("r1", Some(1000), true));
    }

    #[test]
    fn cors_is_off_when_no_origin_is_allowed() {
        assert!(cors(&[]).is_none(), "빈 목록은 같은 origin 배포 전제 — 헤더를 안 낸다");
        assert!(cors(&["not a header value\n".to_owned()]).is_none(), "못 읽는 값으로 열지 않는다");
    }

    #[test]
    fn cors_is_on_for_any_and_for_a_list() {
        assert!(cors(&["*".to_owned()]).is_some());
        assert!(cors(&["https://app.example".to_owned()]).is_some());
    }

    #[test]
    fn default_policy_lets_a_browser_on_another_origin_in() {
        let p = PolicyConfig::default();
        assert_eq!(p.hub.allowed_origins, vec!["*".to_owned()],
            "웹 SDK 는 고객 앱에 들어가는 물건이라 다른 origin 이 기본이다");
        assert!(cors(&p.hub.allowed_origins).is_some());
    }

    /// A 평면 사용자 토큰 — ★운영을 열지 못한다(§16-1-1). 경계 시험에서 "안 열린다" 쪽으로 쓴다.
    fn bearer(_role: &str) -> HeaderMap {
        let t = auth::issue("s", "u1", auth::PT_USER, false, None, 60, auth::now_unix()).unwrap().token;
        let mut h = HeaderMap::new();
        h.insert("authorization", format!("Bearer {t}").parse().unwrap());
        h
    }
    fn peer(s: &str) -> SocketAddr { s.parse().unwrap() }

    fn ops_headers(secret: &str) -> HeaderMap {
        let t = auth::sign_ops(secret, "k", 60, auth::now_unix()).unwrap();
        let mut h = HeaderMap::new();
        h.insert("authorization", format!("Bearer {t}").parse().unwrap());
        h
    }
    fn ops_cfg() -> HubAuth {
        HubAuth { jwt_secret: "s".into(), api_keys: vec![account(vec![auth::PT_USER], false, true)] }
    }

    #[test]
    fn admin_plane_takes_loopback_or_an_ops_token() {
        assert!(admin_ok(&cfg(), &peer("127.0.0.1:9"), &HeaderMap::new()), "같은 기계에서 온 것은 토큰 없이 본다");
        assert!(admin_ok(&cfg(), &peer("[::1]:9"), &HeaderMap::new()), "v6 loopback 도 같은 기계다");
        assert!(admin_ok(&ops_cfg(), &peer("10.0.0.5:9"), &ops_headers("p")));
    }

    #[test]
    fn admin_plane_turns_the_rest_away() {
        assert!(!admin_ok(&ops_cfg(), &peer("10.0.0.5:9"), &HeaderMap::new()));
        assert!(!admin_ok(&ops_cfg(), &peer("10.0.0.5:9"), &bearer("user")), "붙어 있는 사용자라고 관리 평면을 못 본다");
        let mut forged = HeaderMap::new();
        forged.insert("x-forwarded-for", "127.0.0.1".parse().unwrap());
        assert!(!admin_ok(&ops_cfg(), &peer("10.0.0.5:9"), &forged), "XFF 는 아무나 적는다 — 안 믿는다");
        assert!(!admin_ok(&cfg(), &peer("10.0.0.5:9"), &ops_headers("p")), "운영 허용이 없는 계정의 서명은 통하지 않는다");
        assert!(!admin_ok(&ops_cfg(), &peer("10.0.0.5:9"), &ops_headers("다른 비밀")), "그 계정 비밀로 서명한 것이 아니면 아니다");
    }

    #[test]
    fn unwrap_wire_maps_status_by_code() {
        let ok = frame::encode_json(&Header { kind: Kind::Ok, reserved: 0, op: iop::ROOM_GET, pid: 0 }, &json!({"room_id":"r"}));
        assert_eq!(unwrap_wire(&ok).unwrap()["room_id"], "r");
        let nf = crate::backend::fail_frame(iop::ROOM_GET, 0, &Failure::new(FailCode::RoomNotFound));
        assert_eq!(unwrap_wire(&nf).unwrap_err().0, StatusCode::NOT_FOUND);
    }
}
