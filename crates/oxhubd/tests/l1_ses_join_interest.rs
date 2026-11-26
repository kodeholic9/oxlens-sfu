// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-EVENTS · model: claude-opus-5-5
//! 1층 hub 세션 — 입장(ROOM_JOIN)이 노드에서 도는 사이 같은 hub 의 마지막 멤버가 나가도 그 방 관심(방 통지 구독)이 걷히지 않는다. 실제 버스(루프백) · 실제 `bind` · `handle_request` 위에 가짜 노드(gRPC Handle)를 두고 ROOM_JOIN 응답을 시험이 붙잡아 경합 창을 결정적으로 만든다(context 202611/20261124b 본게임 차례 1 · 제품 코드에 손잡이 없음).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use common::envelope::handle::handle_client::HandleClient;
use common::envelope::handle::handle_server::{Handle, HandleServer};
use common::envelope::handle::{CallReq, CallRes, HelloReq, HelloRes};
use common::envelope::{Envelope, Reply};
use common::system::ApiKey;
use oxhubd::bus::view::TokenEv;
use oxhubd::bus::{Bus, BusCfg, Local};
use oxhubd::ses::conn::{Conn, Held, Out, Request};
use oxhubd::ses::token::Accounts;
use oxhubd::ses::{Ctx, Session, bind, handle_request};
use oxsig::op::Op;
use oxsig::msg::TokenReq;
use serde_json::json;
use tokio::sync::{Notify, mpsc};

const ROOM: &str = "r591";

struct Node {
    hold_user: String,
    arrived: Arc<Notify>,
    release: Arc<Notify>,
}

fn join_res(room: &str, user: &str) -> serde_json::Value {
    let me = json!({ "user_id": user, "select": false, "participant_type": 0 });
    json!({
        "room_id": room, "join_id": format!("j-{user}"), "participants": [me.clone()], "me": me,
        "affiliation": { "sub_rooms": [room], "pub_room": null },
        "server_config": {
            "sfu_id": "e1", "pc_mode": "2pc",
            "ice": { "ip": "127.0.0.1", "port": 7000, "publish_ufrag": "pu", "publish_pwd": "pp", "subscribe_ufrag": "su", "subscribe_pwd": "sp" },
            "dtls": { "fingerprint": "sha-256 00", "setup": "passive" },
            "codecs": [{ "kind": "audio", "name": "opus", "rtcp_fb": [] }], "extmap": [], "max_bitrate_bps": 800000
        },
        "tracks": [], "version": { "epoch": "e1", "seq": 1 }, "media_codecs": { "audio": "opus", "video": "H264" },
        "mline_table": { "sfu_id": "e1", "arev": 1, "mlines": [] }
    })
}

#[tonic::async_trait]
impl Handle for Node {
    async fn hello(&self, _req: tonic::Request<HelloReq>) -> Result<tonic::Response<HelloRes>, tonic::Status> {
        Ok(tonic::Response::new(HelloRes { epoch: "e1".into(), build: "t".into() }))
    }

    async fn call(&self, req: tonic::Request<CallReq>) -> Result<tonic::Response<CallRes>, tonic::Status> {
        let env = Envelope::decode(&req.into_inner().envelope).map_err(|e| tonic::Status::invalid_argument(e.to_string()))?;
        let user = env.user_id.clone().unwrap_or_default();
        let reply = match env.op {
            Some(op) if op == Op::RoomJoin.as_u16() => {
                if user == self.hold_user {
                    self.arrived.notify_one();
                    self.release.notified().await;
                }
                Reply::Ok { body: join_res(ROOM, &user) }
            }
            Some(op) if op == Op::RoomLeave.as_u16() => Reply::Ok { body: json!({ "room_id": ROOM, "affiliation": { "sub_rooms": [], "pub_room": null } }) },
            _ => Reply::Ok { body: json!({}) },
        };
        Ok(tonic::Response::new(CallRes { reply: reply.encode() }))
    }
}

fn conn(id: u64) -> (Conn, mpsc::Receiver<Out>) {
    let (tx, rx) = mpsc::channel(256);
    (Conn { id, tx, over: Arc::new(Notify::new()), held: Held::new(1 << 20) }, rx)
}

fn req(op: Op, pid: u32, body: serde_json::Value) -> Request {
    Request { op, op_num: op.as_u16(), pid, flags: 0, body: serde_json::to_vec(&body).expect("body") }
}

fn replied_ok(rx: &mut mpsc::Receiver<Out>, pid: u32) -> bool {
    while let Ok(o) = rx.try_recv() {
        if let Out::Reply { flags, pid: p, .. } = o
            && p == pid
        {
            return flags & 0b11 == 0b01;
        }
    }
    false
}

async fn session(ctx: &Arc<Ctx>, accounts: &Accounts, user: &str, id: u64) -> (Arc<Session>, Conn, mpsc::Receiver<Out>) {
    let tok = accounts
        .issue(&TokenReq { api_key: "k".into(), api_secret: "s".into(), user_id: user.into(), participant_type: None, hidden: None, metadata: None, permission: None }, 60, 2048)
        .expect("token")
        .token;
    let (c, rx) = conn(id);
    let body = serde_json::to_vec(&json!({ "token": tok, "pc_mode": "2pc" })).expect("bind body");
    let s = bind(ctx, &c, Op::Bind.as_u16(), 1, 0, &body).await.expect("bound");
    (s, c, rx)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_last_member_leaving_while_a_join_is_in_flight_does_not_drop_the_room_interest() {
    let (bus, _events) = Bus::open("n1", "e1", 1_000, BusCfg { listen: vec!["127.0.0.1:0".into()], advertise: vec![], connect: vec![], namespace: "t591".into() }).await.expect("bus");
    let now = common::now_ms();
    if let Ok(mut v) = bus.view.lock() {
        v.on_token(TokenEv::NodePut { node: "n1".into(), inst: "e1".into() }, now);
        v.on_token(TokenEv::RoomPut { room: ROOM.into(), node: "n1".into(), inst: "e1".into() }, now);
    }
    assert!(bus.room_route(ROOM).is_some(), "positive control — the room is placed on this node");

    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("port");
    let addr = probe.local_addr().expect("addr");
    drop(probe);
    let arrived = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let node = Node { hold_user: "b591".into(), arrived: arrived.clone(), release: release.clone() };
    tokio::spawn(tonic::transport::Server::builder().add_service(HandleServer::new(node)).serve(addr));
    let mut client = None;
    for _ in 0..50 {
        if let Ok(x) = HandleClient::connect(format!("http://{addr}")).await {
            client = Some(x);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bus.set_local(Some(Local { client: client.expect("node listens"), epoch: "e1".into(), unit: "sfu".into() }));

    let key = ApiKey { key: "k".into(), secret: "s".into(), name: None, participant_types: vec![0], hidden_allowed: false, ops_allowed: false, hook_allowed: false, subscribe: None, facts: None };
    let accounts = Accounts { keys: vec![key.clone()], jwt_secret: None };
    let ctx = Ctx::new(common::policy::Policy::default(), Accounts { keys: vec![key], jwt_secret: None }, bus.clone(), 1 << 20);
    let (sa, ca, mut ra) = session(&ctx, &accounts, "a591", 1).await;
    let (sb, cb, mut rb) = session(&ctx, &accounts, "b591", 2).await;

    handle_request(&ctx, &sa, &ca, req(Op::RoomJoin, 2, json!({ "room_id": ROOM, "select": false }))).await;
    assert!(replied_ok(&mut ra, 2) && sa.in_room(ROOM), "positive control — A joined");
    assert_eq!(bus.interested(ROOM), 1, "positive control — one member holds the room interest");

    let seen = Arc::new(AtomicU64::new(u64::MAX));
    let joining = {
        let (ctx, sb, cb) = (ctx.clone(), sb.clone(), cb.clone());
        tokio::spawn(async move { handle_request(&ctx, &sb, &cb, req(Op::RoomJoin, 2, json!({ "room_id": ROOM, "select": false }))).await })
    };
    tokio::time::timeout(Duration::from_secs(5), arrived.notified()).await.expect("B's ROOM_JOIN reached the node");

    handle_request(&ctx, &sa, &ca, req(Op::RoomLeave, 3, json!({ "room_id": ROOM }))).await;
    assert!(replied_ok(&mut ra, 3) && !sa.in_room(ROOM), "positive control — A, the last member here, left while B's join is in flight");
    seen.store(bus.interested(ROOM) as u64, Ordering::Relaxed);
    assert!(seen.load(Ordering::Relaxed) >= 1, "S-sym-RULE-BUS-EVENTS-9 the in-flight join holds the room interest — the last member leaving does not withdraw it ({})", seen.load(Ordering::Relaxed));

    release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), joining).await.expect("join finishes").expect("join task");
    assert!(replied_ok(&mut rb, 2) && sb.in_room(ROOM), "positive control — B joined");
    assert_eq!(bus.interested(ROOM), 1, "S-sym-RULE-BUS-EVENTS-9 after the join the newcomer keeps the room interest — later room notices still reach this hub");
    bus.close().await;
}
