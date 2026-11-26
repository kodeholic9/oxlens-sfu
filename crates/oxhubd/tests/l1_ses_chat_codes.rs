// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:RULE-CHAT-KIND · 정:IMPL-CHAT-STORE-FAULTS · model: claude-opus-5-5
//! 1층 hub 세션 — 채팅 op 를 실제 `bind` · `handle_request` 위에서 부른다. 채팅 저장소 없는 배포의 3014 와 통지 0(대조: 저장소 있는 hub 는 같은 MESSAGE 를 같은 hub 의 다른 멤버에게 통지한다) · 저장소 장애의 응답 수(context 202611/20261124b 본게임 서버 1층 묶음 4).

use std::sync::Arc;
use std::time::Duration;

use common::system::{ApiKey, Visibility};
use oxhubd::bus::{Bus, BusCfg};
use oxhubd::chat::store::Store;
use oxhubd::chat::{Chat, Limits};
use oxhubd::ses::conn::{Conn, Held, Out, Request};
use oxhubd::ses::token::Accounts;
use oxhubd::ses::{Ctx, Session, bind, handle_request};
use oxsig::msg::TokenReq;
use oxsig::op::Op;
use oxsig::Code;
use serde_json::json;
use tokio::sync::{Notify, mpsc};

const MSG: &str = "e818000001a1e480b5fd7da4c5b6bc0df205";

fn key() -> ApiKey {
    ApiKey { key: "k".into(), secret: "s".into(), name: None, participant_types: vec![0], hidden_allowed: false, ops_allowed: false, hook_allowed: false, subscribe: None, facts: None }
}

async fn hub(ns: &str, store: Option<Store>) -> (Arc<Bus>, Arc<Ctx>) {
    let (bus, _events) = Bus::open("n1", "e1", 1_000, BusCfg { listen: vec!["127.0.0.1:0".into()], advertise: vec![], connect: vec![], namespace: ns.into() }).await.expect("bus");
    let ctx = Ctx::new(common::policy::Policy::default(), Accounts { keys: vec![key()], jwt_secret: None }, bus.clone(), 1 << 20);
    if let Some(s) = store {
        let _ = ctx.chat.set(Arc::new(Chat::new(s, Limits::default())));
    }
    (bus, ctx)
}

async fn session(ctx: &Arc<Ctx>, user: &str, id: u64) -> (Arc<Session>, Conn, mpsc::Receiver<Out>) {
    let accounts = Accounts { keys: vec![key()], jwt_secret: None };
    let tok = accounts
        .issue(&TokenReq { api_key: "k".into(), api_secret: "s".into(), user_id: user.into(), participant_type: None, hidden: None, metadata: None, permission: None }, 60, 2048)
        .expect("token")
        .token;
    let (tx, rx) = mpsc::channel(256);
    let c = Conn { id, tx, over: Arc::new(Notify::new()), held: Held::new(1 << 20) };
    let body = serde_json::to_vec(&json!({ "token": tok, "pc_mode": "2pc" })).expect("bind body");
    let s = bind(ctx, &c, Op::Bind.as_u16(), 1, 0, &body).await.expect("bound");
    (s, c, rx)
}

fn req(op: Op, pid: u32, body: serde_json::Value) -> Request {
    Request { op, op_num: op.as_u16(), pid, flags: 0, body: serde_json::to_vec(&body).expect("body") }
}

fn drain(rx: &mut mpsc::Receiver<Out>) -> Vec<Out> {
    let mut v = Vec::new();
    while let Ok(o) = rx.try_recv() {
        v.push(o);
    }
    v
}

fn replies(outs: &[Out], pid: u32) -> Vec<(bool, serde_json::Value)> {
    outs.iter()
        .filter_map(|o| match o {
            Out::Reply { flags, pid: p, body, .. } if *p == pid => Some((flags & 0b11 == 0b01, serde_json::from_slice(body).unwrap_or(serde_json::Value::Null))),
            _ => None,
        })
        .collect()
}

fn message_notices(outs: &[Out]) -> usize {
    outs.iter().filter(|o| matches!(o, Out::Notice { op: Op::Message, .. })).count()
}

fn chat_ops() -> Vec<(Op, serde_json::Value)> {
    vec![
        (Op::Message, json!({ "space_id": "S", "msg_id": MSG, "content": "hi" })),
        (Op::SpaceHistory, json!({ "space_id": "S" })),
        (Op::SpaceList, json!({})),
        (Op::SpaceView, json!({ "space_id": "S" })),
        (Op::Catchup, json!({})),
    ]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hub_without_a_chat_store_refuses_every_chat_op_with_3014_and_notifies_no_one() {
    let store = Store::open("memory:", Visibility::Shared).expect("store");
    store.create_space("S", None).expect("space");
    for u in ["a", "b"] {
        store.join("S", u, None, Some(1)).expect("join");
    }
    let (on_bus, on) = hub("t271on", Some(store)).await;
    let (sa1, ca, mut ra) = session(&on, "a", 1).await;
    let (_sb, _cb, mut rb) = session(&on, "b", 2).await;
    handle_request(&on, &sa1, &ca, req(Op::Message, 2, chat_ops()[0].1.clone())).await;
    assert!(replies(&drain(&mut ra), 2).first().is_some_and(|(ok, _)| *ok), "positive control — with a chat store the MESSAGE is taken");
    let mut seen = 0;
    for _ in 0..50 {
        seen += message_notices(&drain(&mut rb));
        if seen > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(seen, 1, "positive control — with a chat store the other member on this hub gets the MESSAGE notice");
    on_bus.close().await;

    let (off_bus, off) = hub("t271off", None).await;
    let (sa, ca, mut ra) = session(&off, "a", 1).await;
    let (_sb, _cb, mut rb) = session(&off, "b", 2).await;
    for (i, (op, body)) in chat_ops().into_iter().enumerate() {
        let pid = 10 + i as u32;
        handle_request(&off, &sa, &ca, req(op, pid, body)).await;
        let got = replies(&drain(&mut ra), pid);
        assert_eq!(got.len(), 1, "CP-CODE-CHAT-DISABLED① {} answers once", op.name());
        let (ok, f) = &got[0];
        assert!(!ok, "CP-CODE-CHAT-DISABLED① {} fails", op.name());
        assert_eq!((f["code"].as_u64(), f["permanent"].as_bool()), (Some(Code::ChatDisabled.num() as u64), Some(true)), "CP-CODE-CHAT-DISABLED① {} is 3014 permanent", op.name());
    }
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert_eq!(message_notices(&drain(&mut rb)) + message_notices(&drain(&mut ra)), 0, "CP-CODE-CHAT-DISABLED③ no message notice reaches anyone");
    off_bus.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_chat_store_answers_a_request_exactly_once() {
    let path = std::env::temp_dir().join(format!("oxchat-283-{}-{}.db", std::process::id(), common::now_ms())).to_string_lossy().into_owned();
    let store = Store::open(&format!("sqlite:{path}"), Visibility::Shared).expect("store");
    store.create_space("S", None).expect("space");
    for u in ["a", "b"] {
        store.join("S", u, None, Some(1)).expect("join");
    }
    let (bus, ctx) = hub("t283", Some(store)).await;
    let (sa, ca, mut ra) = session(&ctx, "a", 1).await;
    let (_sb, _cb, mut rb) = session(&ctx, "b", 2).await;
    rusqlite::Connection::open(&path).expect("outside").execute_batch("ALTER TABLE messages RENAME TO messages_away").expect("break the store");
    handle_request(&ctx, &sa, &ca, req(Op::Message, 7, json!({ "space_id": "S", "msg_id": MSG, "content": "hi" }))).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let outs = drain(&mut ra);
    let got = replies(&outs, 7);
    assert_eq!(got.len(), 1, "CP-CODE-INTERNAL-ERROR② a store failure is answered exactly once ({} replies)", got.len());
    assert_eq!((got[0].0, got[0].1["code"].as_u64()), (false, Some(Code::InternalError.num() as u64)), "CP-CODE-INTERNAL-ERROR① the one answer is 5003");
    assert_eq!(message_notices(&drain(&mut rb)), 0, "nothing stored — no notice");
    bus.close().await;
    let _ = std::fs::remove_file(&path);
}
