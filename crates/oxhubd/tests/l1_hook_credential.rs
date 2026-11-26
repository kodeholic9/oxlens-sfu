// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-CREDENTIAL · 정:RULE-HOOK-NO-ROOM-TOKEN · 정:RULE-HOOK-TRANSPORT · 정:RULE-OPS-TOKEN · model: claude-opus-5-5
//! 1층 — 3rd 자격과 운영 자격은 입구부터 갈린다: hub 의 실제 HTTP 라우터(`ops::http::router`)에 loopback 이 아닌 상대로 요청을 넣는다(loopback 무토큰 통과는 운영 규격의 따로 둔 갈래라 여기서 재지 않는다) — 운영 표면(`/admin/*`)은 운영 토큰 없이 열리지 않는다(자격 없음 · 3rd 인증서 · 연동 허용 계정의 A 평면 사용자 토큰 모두 401 · 끄는 요청이 hub 에 닿지 않는다) · 운영 토큰이면 열린다(양성 대조) · HTTP 표면에 권한 제어(`ctl`) 길이 없어 운영 토큰 · 사용자 토큰으로 권한을 바꿀 자리가 없다.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use common::log::{Sender, SenderConfig, Sink};
use oxhubd::bus::{Bus, BusCfg};
use oxhubd::ops::Hub;
use oxhubd::ops::supervisor::{Role, SupCfg, Supervisor, UnitState};
use oxhubd::ses::token::Accounts;
use tower::ServiceExt;

const FAR: &str = "10.20.30.40:5555";

struct Rig {
    hub: Arc<Hub>,
    shut: tokio::sync::mpsc::UnboundedReceiver<String>,
    ops: String,
    user: String,
}

async fn rig() -> Rig {
    let sys = common::system::System::parse(concat!(
        "[hub]\nlisten = \"0.0.0.0:1974\"\n[bus]\nnamespace = \"hc\"\n",
        "[[hub.auth.api_keys]]\nkey = \"acme\"\nsecret = \"acme-secret\"\nhook_allowed = true\n",
        "[[hub.auth.api_keys]]\nkey = \"op\"\nsecret = \"op-secret\"\nops_allowed = true\n",
    ))
    .expect("system");
    let policy = common::policy::Policy::parse("").expect("policy");
    let (bus, _rx) = Bus::open("n1", "e1", 1_000, BusCfg { listen: vec!["127.0.0.1:0".into()], advertise: vec![], connect: vec![], namespace: "hc".into() }).await.expect("bus");
    let accounts = Accounts { keys: sys.hub.auth.api_keys.clone(), jwt_secret: None };
    let ops = accounts.sign_ops("op", 60).expect("ops token");
    let req = oxsig::msg::TokenReq { api_key: "acme".into(), api_secret: "acme-secret".into(), user_id: "u1".into(), participant_type: None, hidden: None, metadata: None, permission: None };
    let user = accounts.issue(&req, 60, 0).expect("user token").token;
    let ses = oxhubd::ses::Ctx::new(policy.clone(), accounts, bus.clone(), 1 << 20);
    let (fatal, _) = tokio::sync::mpsc::unbounded_channel();
    let cfg = SupCfg {
        node: "n1".into(),
        hub_inst: "e1".into(),
        build_label: None,
        backoff_ms: vec![100],
        intensity_max: 5,
        intensity_window: Duration::from_secs(60),
        hello_timeout: Duration::from_secs(1),
        extra_args: Vec::new(),
        sfu_args: Vec::new(),
        rec: None,
    };
    let sup = Supervisor::new(&[], cfg, fatal);
    sup.mark_state(Role::Bus, UnitState::Running, "l1");
    let sender = Sender::start(SenderConfig {
        node: "n1".into(),
        proc_name: "oxhubd",
        epoch: None,
        sink: Sink::Stdout,
        log_dir: None,
        line_max: 2_047,
        queue_max: 64,
        spool_max: 0,
        level: common::policy::LoggingLevel::Warn,
    });
    let (shutdown, shut) = tokio::sync::mpsc::unbounded_channel();
    let hook = Arc::new(oxhubd::hook::Hook::new("e1", Vec::new(), 10));
    let hub = Arc::new(Hub {
        node: "n1".into(),
        inst: "e1".into(),
        build: "l1".into(),
        base_path: String::new(),
        policy,
        ses,
        bus,
        sup,
        sender,
        namespace: "hc".into(),
        shutdown,
        log_files: None,
        data_root: None,
        hook,
    });
    Rig { hub, shut, ops, user }
}

async fn call(r: &Rig, method: &str, path: &str, auth: Option<String>) -> StatusCode {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(a) = auth {
        b = b.header(header::AUTHORIZATION, a);
    }
    let mut req = b.body(Body::empty()).expect("request");
    req.extensions_mut().insert(ConnectInfo(FAR.parse::<SocketAddr>().expect("peer")));
    oxhubd::ops::http::router(r.hub.clone()).oneshot(req).await.expect("response").status()
}

fn bearer(s: &str) -> Option<String> {
    Some(format!("Bearer {s}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_3rd_credential_does_not_open_the_operations_surface() {
    let mut r = rig().await;
    let shutdown = "/admin/supervisor/shutdown?confirm=n1";
    assert_eq!(call(&r, "GET", "/admin/supervisor/status", None).await, StatusCode::UNAUTHORIZED, "S-rule-RULE-HOOK-CREDENTIAL-5② — 자격 없이는 운영 표면(/admin/*)이 열리지 않는다(401)");
    assert_eq!(call(&r, "POST", shutdown, None).await, StatusCode::UNAUTHORIZED, "S-rule-RULE-HOOK-CREDENTIAL-5② — 자격 없이는 끄는 요청이 거절된다(401)");
    let user = r.user.clone();
    assert_eq!(call(&r, "GET", "/admin/supervisor/status", bearer(&user)).await, StatusCode::UNAUTHORIZED, "S-rule-RULE-HOOK-CREDENTIAL-5② — 연동 허용 계정의 A 평면 사용자 토큰도 운영 표면의 자격이 아니다(401)");
    assert_eq!(call(&r, "POST", shutdown, bearer(&user)).await, StatusCode::UNAUTHORIZED, "S-rule-RULE-HOOK-CREDENTIAL-5② — 연동 허용 계정의 사용자 토큰으로는 서버를 끄지 못한다(401)");
    assert!(r.shut.try_recv().is_err(), "형상 — 거절된 끄는 요청은 hub 에 닿지 않았다(처리기가 응답 전에 동기로 보내므로 기다림이 필요 없다)");
    let ops = r.ops.clone();
    assert_eq!(call(&r, "GET", "/admin/supervisor/status", bearer(&ops)).await, StatusCode::OK, "positive control — 운영 토큰이면 운영 표면이 열린다");
    assert_eq!(call(&r, "POST", shutdown, bearer(&ops)).await, StatusCode::OK, "positive control — 운영 토큰이면 끄는 요청이 받아들여진다");
    assert_eq!(r.shut.try_recv().ok().as_deref(), Some("admin shutdown"), "positive control — 받아들인 끄는 요청은 hub 에 닿는다");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_http_surface_has_no_permission_control() {
    let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ops/http.rs")).expect("http.rs");
    let body = src.split("pub fn router(").nth(1).and_then(|t| t.split("\n}\n").next()).expect("stand check — the router function");
    let routes: Vec<(String, String)> = body
        .split(".route(\"")
        .skip(1)
        .filter_map(|t| t.split_once("\", ").map(|(p, rest)| (p.to_string(), rest.split(')').next().unwrap_or("").to_string())))
        .collect();
    assert!(routes.len() >= 20 && routes.iter().any(|(p, _)| p == "/admin/supervisor/shutdown"), "stand check — the route table is read whole ({} routes)", routes.len());
    let r = rig().await;
    let ops = r.ops.clone();
    assert_eq!(call(&r, "GET", "/admin/supervisor/status", bearer(&ops)).await, StatusCode::OK, "stand check — the table is the live router's (a listed route answers)");
    for (path, handler) in &routes {
        assert!(
            !path.contains("permission") && !path.contains("ctl") && !handler.contains("permission"),
            "S-rule-RULE-HOOK-CREDENTIAL-5① · S-rule-RULE-HOOK-CREDENTIAL-4① · S-sym-RULE-HOOK-NO-ROOM-TOKEN-2① — HTTP 라우터 표 전량에 권한 제어(ctl) 길이 없다 — 운영 토큰 · 사용자 토큰이 실릴 HTTP 길로는 권한을 바꿀 수 없다 {path} → {handler}"
        );
    }
    let admin = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ops/admin.rs")).expect("admin.rs");
    assert!(
        [&src, &admin].iter().all(|t| !t.contains("CallKind::Permission") && !t.contains("permission_env") && !t.contains("query::permission")),
        "S-rule-RULE-HOOK-CREDENTIAL-5① · S-rule-RULE-HOOK-CREDENTIAL-4① · S-sym-RULE-HOOK-NO-ROOM-TOKEN-2① — HTTP 처리기(http.rs · admin.rs) 어디에서도 권한 제어 Call 을 짓지 않는다"
    );
}
