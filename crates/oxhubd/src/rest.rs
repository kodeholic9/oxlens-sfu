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
use tower_http::cors::{AllowOrigin, CorsLayer};
use serde_json::{Value, json};

use crate::backend::SfuBackend;
use crate::session::SessionRegistry;

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub api_key: String,
    pub api_secret: String,
    pub user_id: String,
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default)]
    pub floor_priority: u8,
    #[serde(default)]
    pub metadata: Option<Value>,
}

fn default_role() -> String {
    auth::ROLE_USER.to_owned()
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct TokenResponse {
    pub token: String,
    pub expires_in: u64,
}

type HttpFail = (StatusCode, Failure);

/// 정§3-4 판정 — `2004`(401) · `2005`(400) · `5003`(500).
pub fn issue_token(auth_cfg: &HubAuth, ttl_secs: u64, req: &TokenRequest, now: i64) -> Result<TokenResponse, HttpFail> {
    let Some(account) = auth_cfg.api_keys.iter().find(|k| k.key == req.api_key && k.secret == req.api_secret) else {
        return Err((StatusCode::UNAUTHORIZED, Failure::new(FailCode::InvalidApiKey)));
    };
    if !auth::is_known_role(&req.role) || !account.roles.iter().any(|r| r == &req.role) {
        return Err((StatusCode::BAD_REQUEST, Failure::new(FailCode::InvalidRole).message(format!("role '{}' not allowed", req.role))));
    }
    auth::issue(&auth_cfg.jwt_secret, &req.user_id, &req.role, req.floor_priority, req.metadata.clone(), ttl_secs, now)
        .map(|i| TokenResponse { token: i.token, expires_in: i.expires_in })
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Failure::new(FailCode::InternalError).message(e.to_string())))
}

pub struct RestState {
    pub system: SystemConfig,
    pub policy: PolicyConfig,
    pub registry: Arc<SessionRegistry>,
    pub backend: Arc<SfuBackend>,
}

/// 정§16-1 `/admin/*` — 유닛 평면. 이 hub 가 ★지금 보고 있는 노드 목록이다(설정 파일이 아니라).
/// ★loopback 은 통과시킨다(XFF 를 믿지 않는다 — 리버스 프록시 뒤 배치 금지가 전제).
pub async fn admin_sfus(State(st): State<Arc<RestState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> axum::response::Response {
    if !peer.ip().is_loopback() && !is_admin(&st.system.hub.auth, &headers) {
        return respond(Err((StatusCode::UNAUTHORIZED, Failure::new(FailCode::NotAuthorized).message("admin only"))));
    }
    let sfus: Vec<Value> = st.backend.nodes.all().iter().map(|n| json!({ "sfu_id": n.id, "addr": n.addr })).collect();
    respond(Ok((StatusCode::OK, json!({ "sfus": sfus }))))
}

fn is_admin(auth_cfg: &HubAuth, headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|t| auth::verify(&auth_cfg.jwt_secret, t).ok())
        .is_some_and(|c| c.role == "admin")
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
    match issue_token(&st.system.hub.auth, u64::from(st.policy.hub.token_ttl_secs), &req, auth::now_unix()) {
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
            let env = bplane::Envelope { session_id: String::new(), user_id: String::new(), room_id: String::new(), target: String::new(), exclude: Vec::new(), wire: internal_wire(iop::ROOM_LIST, &Value::Null), pc_mode: String::new(), floor_priority: 0 };
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
                let ids = st.backend.nodes.ids();
                let chosen = crate::route::place(&ids, &room_id).ok_or_else(|| fail_of(FailCode::SfuUnavailable))?;
                st.backend.rooms.assign(&room_id, chosen)
            }
        };
        let env = bplane::Envelope { session_id: String::new(), user_id: String::new(), room_id: room_id.clone(), target: String::new(), exclude: Vec::new(), wire: internal_wire(iop::ROOM_CREATE, &body), pc_mode: String::new(), floor_priority: 0 };
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
            floor_priority: 0,
        };
        st.backend.send_to_node(&node_id, env).await.map_err(fail_of).and_then(|w| unwrap_wire(&w)).map(|v| (StatusCode::OK, v))
    }
    .await)
}

pub async fn healthz() -> &'static str {
    "ok"
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::config::ApiKey;
    use std::time::Duration;

    fn cfg() -> HubAuth {
        HubAuth { jwt_secret: "s".into(), api_keys: vec![ApiKey { key: "k".into(), secret: "p".into(), name: String::new(), roles: vec!["user".into()] }] }
    }
    fn req(key: &str, role: &str) -> TokenRequest {
        TokenRequest { api_key: key.into(), api_secret: "p".into(), user_id: "u1".into(), role: role.into(), floor_priority: 9, metadata: None }
    }

    #[test]
    fn token_judgement() {
        let now = auth::now_unix();
        let ok = issue_token(&cfg(), 3600, &req("k", "user"), now).unwrap();
        assert_eq!(ok.expires_in, 3600);
        assert_eq!(auth::verify("s", &ok.token).unwrap().floor_priority, 9);
        let (st, f) = issue_token(&cfg(), 3600, &req("x", "user"), now).unwrap_err();
        assert_eq!((st, f.code), (StatusCode::UNAUTHORIZED, 2004));
        let (st, f) = issue_token(&cfg(), 3600, &req("k", "admin"), now).unwrap_err();
        assert_eq!((st, f.code), (StatusCode::BAD_REQUEST, 2005));
    }

    #[test]
    fn two_auth_branches() {
        let reg = SessionRegistry::new("s", Duration::from_secs(60), 10_000);
        let mut h = HeaderMap::new();
        assert_eq!(authenticate("s", &reg, &h).unwrap_err().1.code, 2002);
        let t = auth::issue("s", "u1", "user", 0, None, 60, auth::now_unix()).unwrap().token;
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

    #[test]
    fn unwrap_wire_maps_status_by_code() {
        let ok = frame::encode_json(&Header { kind: Kind::Ok, reserved: 0, op: iop::ROOM_GET, pid: 0 }, &json!({"room_id":"r"}));
        assert_eq!(unwrap_wire(&ok).unwrap()["room_id"], "r");
        let nf = crate::backend::fail_frame(iop::ROOM_GET, 0, &Failure::new(FailCode::RoomNotFound));
        assert_eq!(unwrap_wire(&nf).unwrap_err().0, StatusCode::NOT_FOUND);
    }
}
