// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-FACTS · 정:RULE-BUS-PUBLIC · 정:RULE-BUS-EVENTS · model: claude-opus-5-5
//! 1층 — 방 통지 길목(`ops::dispatch` → 방 통지 가르기 → 사실 짓기)에 sfud 가 내는 꼴 그대로의 `PARTICIPANT_EVENT`(형 `ParticipantEventBody` 직렬화) 를 버스 사건으로 넣고 계정 큐에 든 사실을 본다: 방을 맡은 hub 에서 joined · left 하나가 사실 하나씩이 되고, 본문은 통지 본문 그대로(판번호 실림 · hub 가 덧붙이는 칸 0).

use std::sync::Arc;
use std::time::Duration;

use common::envelope::Notice;
use common::log::{Sender, SenderConfig, Sink};
use oxhubd::bus::{Bus, BusCfg, BusEvent};
use oxhubd::ops::Hub;
use oxhubd::ops::supervisor::{Role, SupCfg, Supervisor, UnitState};
use oxhubd::ses::token::Accounts;
use oxsig::msg::{ParticipantEventBody, ParticipantEventType, Version};

async fn hub() -> Arc<Hub> {
    let policy = common::policy::Policy::parse("").expect("policy");
    let (bus, _rx) = Bus::open("n1", "e1", 1_000, BusCfg { listen: vec!["127.0.0.1:0".into()], advertise: vec![], connect: vec![], namespace: "hd".into() }).await.expect("bus");
    bus.declare_self().await.expect("self");
    let ses = oxhubd::ses::Ctx::new(policy.clone(), Accounts { keys: Vec::new(), jwt_secret: None }, bus.clone(), 1 << 20);
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
    let (shutdown, _) = tokio::sync::mpsc::unbounded_channel();
    let hook = Arc::new(oxhubd::hook::Hook::new("e1", vec![("acme".into(), vec!["*".into()])], 100));
    Arc::new(Hub {
        node: "n1".into(),
        inst: "e1".into(),
        build: "l1".into(),
        base_path: String::new(),
        policy,
        ses,
        bus,
        sup,
        sender,
        namespace: "hd".into(),
        shutdown,
        log_files: None,
        data_root: None,
        hook,
    })
}

fn event(t: ParticipantEventType, seq: u64) -> Notice {
    let body = ParticipantEventBody {
        type_: t,
        room_id: "r1".into(),
        user_id: "u1".into(),
        role: None,
        select: None,
        participant_type: None,
        metadata: None,
        permission: None,
        version: Version { epoch: "e".into(), seq },
    };
    Notice { op: 0x0701, body: serde_json::to_value(body).expect("body"), exclude: Vec::new(), room_id: Some("r1".into()), sfu_id: Some("e1".into()) }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn participant_events_become_facts_through_the_room_notice_path() {
    let hub = hub().await;
    assert!(hub.bus.own_room("r1").await, "stand check — this hub owns r1");
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(oxhubd::ops::dispatch(hub.clone(), rx));
    let joined = event(ParticipantEventType::Joined, 3);
    let left = event(ParticipantEventType::Left, 4);
    tx.send(BusEvent::RoomNotice { room: "r1".into(), notice: joined.clone() }).expect("send joined");
    tx.send(BusEvent::RoomNotice { room: "r1".into(), notice: left.clone() }).expect("send left");
    let until = tokio::time::Instant::now() + Duration::from_secs(3);
    while hub.hook.queued("acme") < 2 && tokio::time::Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let facts = hub.hook.take("acme", 1 << 20).map(|b| b.facts).unwrap_or_default();
    let of = |k: &str| facts.iter().filter(|f| f.kind == k).cloned().collect::<Vec<_>>();
    let j = of("participant_joined");
    assert!(
        j.len() == 1 && j[0].body == joined.body && j[0].body["version"]["seq"] == 3,
        "HK-FACT-PARTICIPANT-JOINED③ — 방 통지 길목을 지난 participant_joined 의 본문은 PARTICIPANT_EVENT 그대로 · version 이 실린다 {facts:?}"
    );
    let l = of("participant_left");
    assert_eq!(l.len(), 1, "HK-FACT-PARTICIPANT-LEFT② — PARTICIPANT_EVENT left 하나는 방 통지 길목에서 participant_left 사실 하나가 된다 {facts:?}");
    assert!(
        l[0].body == left.body && l[0].body["version"]["seq"] == 4,
        "HK-FACT-PARTICIPANT-LEFT③ — 방 통지 길목을 지난 participant_left 의 본문은 PARTICIPANT_EVENT 그대로 · version 이 실린다 {facts:?}"
    );
    let keys: Vec<&String> = l[0].body.as_object().map(|o| o.keys().collect()).unwrap_or_default();
    assert!(
        ["reason", "why", "cause"].iter().all(|k| !keys.iter().any(|x| x.as_str() == *k)),
        "HK-FACT-PARTICIPANT-LEFT② — hub 는 participant_left 에 까닭 칸을 덧붙이지 않는다 {keys:?}"
    );
    assert_eq!(facts.len(), 2, "형상 — 통지 둘에 사실 둘");
}
