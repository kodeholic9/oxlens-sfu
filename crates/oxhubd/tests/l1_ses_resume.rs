// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SES-RES · model: claude-opus-5-5
//! 1층 hub 세션 — 이어받을 것이 없는 세션(새로 BIND 한 세션)의 RESUME 은 2008 응답 하나로 끝난다. 실제 버스 · `bind` · `handle_request` 위에서 본다(context 202611/20261124b 본게임 서버 1층 묶음 5).

use std::sync::Arc;
use std::time::Duration;

use common::system::ApiKey;
use oxhubd::bus::{Bus, BusCfg};
use oxhubd::ses::conn::{Conn, Held, Out, Request};
use oxhubd::ses::token::Accounts;
use oxhubd::ses::{Ctx, bind, handle_request};
use oxsig::Code;
use oxsig::msg::TokenReq;
use oxsig::op::Op;
use serde_json::json;
use tokio::sync::{Notify, mpsc};

fn key() -> ApiKey {
    ApiKey { key: "k".into(), secret: "s".into(), name: None, participant_types: vec![0], hidden_allowed: false, ops_allowed: false, hook_allowed: false, subscribe: None }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_on_a_fresh_session_answers_2008_once() {
    let (bus, _events) = Bus::open("n1", "e1", 1_000, BusCfg { listen: vec!["127.0.0.1:0".into()], advertise: vec![], connect: vec![], namespace: "t258".into() }).await.expect("bus");
    let ctx = Ctx::new(common::policy::Policy::default(), Accounts { keys: vec![key()], jwt_secret: None }, bus.clone(), 1 << 20);
    let accounts = Accounts { keys: vec![key()], jwt_secret: None };
    let tok = accounts
        .issue(&TokenReq { api_key: "k".into(), api_secret: "s".into(), user_id: "a258".into(), participant_type: None, hidden: None, metadata: None, permission: None }, 60, 2048)
        .expect("token")
        .token;
    let (tx, mut rx) = mpsc::channel(256);
    let c = Conn { id: 1, tx, over: Arc::new(Notify::new()), held: Held::new(1 << 20) };
    let body = serde_json::to_vec(&json!({ "token": tok, "pc_mode": "2pc" })).expect("bind body");
    let s = bind(&ctx, &c, Op::Bind.as_u16(), 1, 0, &body).await.expect("bound");
    while rx.try_recv().is_ok() {}
    handle_request(&ctx, &s, &c, Request { op: Op::Resume, op_num: Op::Resume.as_u16(), pid: 2, flags: 0, body: b"{}".to_vec() }).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut got = Vec::new();
    while let Ok(o) = rx.try_recv() {
        if let Out::Reply { flags, pid: 2, body, .. } = o {
            got.push((flags & 0b11 == 0b01, serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_default()));
        }
    }
    assert_eq!(got.len(), 1, "CP-CODE-SESSION-NOT-FOUND① a RESUME with nothing to take over is answered once ({got:?})");
    assert_eq!((got[0].0, got[0].1["code"].as_u64()), (false, Some(Code::SessionNotFound.num() as u64)), "CP-CODE-SESSION-NOT-FOUND① the one answer is 2008 — no partial failure code");
    bus.close().await;
}
