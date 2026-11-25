// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:RULE-CHAT-KIND · 연:RULE-CHAT-REF-CHECK · 연:RULE-CHAT-DELETE · 연:RULE-CHAT-FOLD · 정:IMPL-CHAT-STORE ·정:IMPL-CHAT-MEMBERSHIP · 정:IMPL-CHAT-FILTER · 정:IMPL-CHAT-READ · 정:IMPL-CHAT-UNDELIVERED · 정:IMPL-CHAT-CATCHUP · 정:IMPL-CHAT-STORE-FAULTS · 정:IMPL-CHAT-STORE-FORMS · 정:IMPL-CFG-SYSTEM-FILE · 정:IMPL-CHAT-LIVE-BUNDLE · 정:IMPL-SES-SOCKET-LIFE · 정:RULE-CHAT-EXT-SUB · 연:RULE-CHAT-MSG-ID · 연:RULE-CHAT-CONTENT · 연:RULE-CHAT-ORDER · 연:RULE-CHAT-VISIBLE · 연:RULE-CHAT-UNREAD · 연:RULE-CHAT-HISTORY-PAGE · 연:RULE-CHAT-UNDELIVERED · 연:RULE-CHAT-CATCHUP · 연:RULE-CHAT-MEMBER-NOTICE · model: claude-opus-5-5
//! 1층 — 채팅 저장소 시험은 `[chat] db` 꼴 셋(`memory:` · `sqlite:<경로>` · `postgres://…`)에서 같은 몸으로 돈다(꼴마다 시험 하나 · PostgreSQL 은 시험마다 따로 된 스키마 · 접속 `OXLENS_TEST_PG` · 못 닿으면 「못 잼」 빨강) · 바깥 연결로 잠그는 시험은 바깥이 있는 꼴(sqlite · postgres)에서 · 질의 계획은 SQLite 엔진 꼴(memory · sqlite)에서 · 옛 파일 칸 더하기는 sqlite 꼴에서. 저장소(저장할 때 찍는 t · 같은 t 를 가르지 않는 쪽 · msg_id 멱등 · 멤버 구간 · 멤버 변경 메시지 · 보이기 시작점 · 안 읽은 수 · 미전송 기록 · 따라잡기 쪽)와 받는 게이트웨이의 멤버 캐시 · op 경계 · 저장소 실패의 원인 진단 줄 · 송신 줄의 실시간 채팅 묶기(창이 찼을 때만 · 줄 끝 · 1단 제외 · 영수증 이어 붙이기 · 바이트 상한) · 설정 배너의 비밀번호 가림 · 3rd 계정 몫 펜딩(미전송 기록과 같은 트랜잭션 · 원본 따라 걸기 · 멱등 · 시도 수 상한 · 줄을 가른 ACK · 보관 지우기).

use std::sync::Arc;

use common::extsub::{ExtSubs, Subscribe};
use common::system::{ChatDb, System, Visibility};
use oxhubd::chat::store::{Cursor, Dir, Lane, Outgoing, Page, SYS, SYS_HASH, Sent, Store, catchup_since_sql, relations_sql};
use oxhubd::chat::acks::{AckLanes, Push};
use oxhubd::chat::{BODY_MAX, Chat, Limits, outgoing, valid_msg_id};
use oxhubd::ses::conn::QUEUE_MAX;
use common::talk::Report;
use oxsig::msg::{ChatKind, ChatMessage, MessagesBody, TalkState};
use oxsig::{Code, Op};

const ID_A: &str = "e818000001a1e480b5fd7da4c5b6bc0df205";
const ID_B: &str = "e818000001a1e480b5fe0b1c2d3e4f506a7b";
const ID_C: &str = "3f0a9b1201a1e480b600aa11bb22cc33dd44";

mod pgx {
    use std::sync::mpsc;

    type Job = (String, mpsc::Sender<Result<(), String>>);

    pub struct Conn {
        tx: Option<tokio::sync::mpsc::UnboundedSender<Job>>,
        h: Option<std::thread::JoinHandle<()>>,
    }

    impl Conn {
        pub fn open(url: &str) -> Result<Conn, String> {
            let cfg = oxhubd::chat::store::pg_config(url)?;
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
            let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
            let h = std::thread::spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
                    let _ = ready_tx.send(Err("runtime".into()));
                    return;
                };
                rt.block_on(async move {
                    let (client, conn) = match cfg.connect(tokio_postgres::NoTls).await {
                        Ok(x) => x,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e.to_string()));
                            return;
                        }
                    };
                    tokio::spawn(conn);
                    let _ = ready_tx.send(Ok(()));
                    while let Some((sql, back)) = rx.recv().await {
                        let r = client.batch_execute(&sql).await.map_err(|e| e.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| e.to_string()));
                        let _ = back.send(r);
                    }
                });
            });
            ready_rx.recv().map_err(|e| e.to_string())??;
            Ok(Conn { tx: Some(tx), h: Some(h) })
        }

        pub fn exec(&self, sql: &str) -> Result<(), String> {
            let (back, got) = mpsc::channel();
            self.tx.as_ref().ok_or("closed")?.send((sql.to_string(), back)).map_err(|e| e.to_string())?;
            got.recv().map_err(|e| e.to_string())?
        }
    }

    impl Drop for Conn {
        fn drop(&mut self) {
            self.tx.take();
            if let Some(h) = self.h.take() {
                let _ = h.join();
            }
        }
    }

    pub fn run(url: &str, sql: &str) -> Result<(), String> {
        Conn::open(url)?.exec(sql)
    }
}

mod form {
    use std::sync::atomic::{AtomicU64, Ordering};

    use common::system::{ChatDb, Visibility};
    use oxhubd::chat::store::Store;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Kind {
        Memory,
        Sqlite,
        Postgres,
    }

    pub struct Db {
        pub kind: Kind,
        pub url: String,
        pub file: Option<String>,
        schema: Option<(String, String)>,
    }

    static NEXT: AtomicU64 = AtomicU64::new(0);

    pub fn pg_base() -> String {
        std::env::var("OXLENS_TEST_PG").unwrap_or_else(|_| "postgres://oxlens@localhost/oxlens_chat".into())
    }

    fn uniq() -> String {
        format!("{}_{}_{}", std::process::id(), common::now_ms(), NEXT.fetch_add(1, Ordering::Relaxed))
    }

    impl Db {
        pub fn new(kind: Kind) -> Db {
            match kind {
                Kind::Memory => Db { kind, url: "memory:".into(), file: None, schema: None },
                Kind::Sqlite => {
                    let p = std::env::temp_dir().join(format!("oxchat-{}.db", uniq())).to_string_lossy().into_owned();
                    Db { kind, url: format!("sqlite:{p}"), file: Some(p), schema: None }
                }
                Kind::Postgres => Db::pg_at(&pg_base()),
            }
        }

        pub fn pg_at(base: &str) -> Db {
            let schema = format!("t_{}", uniq());
            if let Err(e) = super::pgx::run(base, &format!("CREATE SCHEMA {schema}")) {
                panic!("S-sym-IMPL-CHAT-STORE-FORMS-2 — 못 잼: PostgreSQL {} 에 닿지 못한다(OXLENS_TEST_PG) — {e}", ChatDb::shown_of(base));
            }
            let sep = if base.contains('?') { '&' } else { '?' };
            Db { kind: Kind::Postgres, url: format!("{base}{sep}options=-c%20search_path%3D{schema}"), file: None, schema: Some((base.to_string(), schema)) }
        }

        pub fn open(&self, vis: Visibility) -> Store {
            Store::open(&self.url, vis).unwrap_or_else(|e| panic!("open {:?}: {e}", self.kind))
        }

        pub fn open_pool(&self, pool: usize, vis: Visibility) -> Store {
            Store::open_pool(&self.url, pool, vis).unwrap_or_else(|e| panic!("open {:?}: {e}", self.kind))
        }

        pub fn shared(&self) -> bool {
            self.kind != Kind::Memory
        }
    }

    impl Drop for Db {
        fn drop(&mut self) {
            if let Some(f) = &self.file {
                for x in ["", "-wal", "-shm"] {
                    let _ = std::fs::remove_file(format!("{f}{x}"));
                }
            }
            if let Some((base, s)) = &self.schema {
                let _ = super::pgx::run(base, &format!("DROP SCHEMA IF EXISTS {s} CASCADE"));
            }
        }
    }
}

enum Outside {
    Lite(rusqlite::Connection),
    Pg(pgx::Conn),
}

impl Outside {
    fn of(db: &form::Db) -> Outside {
        match db.kind {
            form::Kind::Sqlite => Outside::Lite(rusqlite::Connection::open(db.file.as_deref().expect("file")).expect("outside")),
            form::Kind::Postgres => Outside::Pg(pgx::Conn::open(&db.url).expect("outside")),
            form::Kind::Memory => panic!("memory: 은 바깥 연결이 없다"),
        }
    }

    fn exec(&self, lite: &str, pg: &str) {
        match self {
            Outside::Lite(c) => c.execute_batch(lite).expect(lite),
            Outside::Pg(c) => c.exec(pg).expect(pg),
        }
    }

    fn hold(&self) {
        self.exec("BEGIN IMMEDIATE", "BEGIN; LOCK TABLE spaces, members, messages, reads, undelivered IN EXCLUSIVE MODE");
    }

    fn release(&self) {
        self.exec("ROLLBACK", "ROLLBACK");
    }
}

macro_rules! forms {
    ($name:ident [$($f:ident = $k:ident),+] |$db:ident| $body:block) => {
        mod $name {
            use super::*;
            fn run($db: &form::Db) $body
            $(
                #[test]
                fn $f() {
                    run(&form::Db::new(form::Kind::$k))
                }
            )+
        }
    };
}

macro_rules! all_forms {
    ($name:ident |$db:ident| $body:block) => {
        forms!($name [memory = Memory, sqlite = Sqlite, postgres = Postgres] |$db| $body);
    };
}

macro_rules! shared_forms {
    ($name:ident |$db:ident| $body:block) => {
        forms!($name [sqlite = Sqlite, postgres = Postgres] |$db| $body);
    };
}

macro_rules! lite_forms {
    ($name:ident |$db:ident| $body:block) => {
        forms!($name [memory = Memory, sqlite = Sqlite] |$db| $body);
    };
}

macro_rules! async_forms {
    ($name:ident [$($f:ident = $k:ident),+] |$db:ident| $body:block) => {
        mod $name {
            use super::*;
            async fn run($db: &form::Db) $body
            $(
                #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
                async fn $f() {
                    run(&form::Db::new(form::Kind::$k)).await
                }
            )+
        }
    };
}

fn store(db: &form::Db, vis: Visibility) -> Store {
    let s = db.open(vis);
    s.create_space("S", None).expect("space");
    s
}

fn join(s: &Store, space: &str, user: &str, at: u64) {
    s.join(space, user, None, Some(at)).expect("join");
}

fn fresh_t(r: Result<Sent, Code>) -> u64 {
    match r.expect("send") {
        Sent::Fresh { msg, .. } => msg.t,
        Sent::Again { .. } => panic!("expected a fresh store"),
    }
}

fn posts(p: &Page) -> Vec<&ChatMessage> {
    p.messages.iter().filter(|m| m.kind.is_none()).collect()
}

fn id(i: usize) -> String {
    format!("{:036x}", i + 1)
}

all_forms!(t_is_the_store_time_as_is |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1000);
    let t1 = fresh_t(s.send("a", "S", ID_A, "x", Some(5000)));
    let t2 = fresh_t(s.send("a", "S", ID_B, "y", Some(5000)));
    assert_eq!((t1, t2), (5000, 5000), "S-sym-IMPL-CHAT-STORE-1 · CP-OP-MESSAGE⑦ — two stored in the same ms both keep that t");
    let t3 = fresh_t(s.send("a", "S", ID_C, "z", Some(10)));
    assert_eq!(t3, 10, "S-sym-IMPL-CHAT-STORE-1 — an earlier store time is not pushed up");
    let e = s.list("a").expect("list").remove(0);
    assert_eq!((e.total, e.last_t), (3, Some(5000)), "S-sym-IMPL-CHAT-STORE-1 — total counts posts · last t is the largest");
});

all_forms!(t_is_stamped_inside_the_store |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    let before = common::now_ms();
    let t = fresh_t(s.send("a", "S", ID_A, "x", None));
    let after = common::now_ms();
    assert!(before <= t && t <= after, "S-sym-IMPL-CHAT-STORE-1 — t is the ms taken in the store transaction ({before} ≤ {t} ≤ {after})");
});

all_forms!(same_msg_id_same_content_answers_the_first_t |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    let t1 = fresh_t(s.send("a", "S", ID_A, "hello", Some(800)));
    let before = s.pending("b").expect("pending").len();
    match s.send("a", "S", ID_A, "hello", Some(900)).expect("retry") {
        Sent::Again { msg } => assert_eq!(msg.t, t1, "S-sym-IMPL-CHAT-STORE-3 — the retry gets the first t and no new store"),
        Sent::Fresh { .. } => panic!("must not store again"),
    }
    assert_eq!(s.pending("b").expect("pending").len(), before, "S-sym-IMPL-CHAT-STORE-3 — a retry adds no undelivered record");
    assert_eq!(s.send("a", "S", ID_A, "other", Some(900)), Err(Code::MsgIdConflict), "same msg_id with other content is 3015");
    assert_eq!(s.list("a").expect("list")[0].total, 1, "one message stored");
});

#[test]
fn msg_id_is_opaque_and_only_bounded() {
    assert!(valid_msg_id("x"), "any shape is taken");
    assert!(valid_msg_id(&"a".repeat(64)), "64 bytes is taken");
    assert!(!valid_msg_id(""), "empty is refused");
    assert!(!valid_msg_id(&"a".repeat(65)), "65 bytes is refused");
}

async_forms!(message_op_refuses_only_that_request_on_bounds [memory = Memory, sqlite = Sqlite, postgres = Postgres] |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    let chat = Chat::new(s, Limits::default());
    let ask = |msg_id: String, content: String| serde_json::json!({ "space_id": "S", "msg_id": msg_id, "content": content });
    let f = chat.handle("a", "s1", Op::Message, ask("a".repeat(65), "x".into()), 100).await.err().expect("refused");
    assert_eq!(f.code, Code::InvalidPayload.num(), "CP-OP-MESSAGE④ — msg_id over 64 bytes is 1002");
    let f = chat.handle("a", "s1", Op::Message, ask(ID_A.into(), "가".repeat(21_846)), 70_000).await.err().expect("refused");
    assert_eq!(f.code, Code::InvalidPayload.num(), "CP-OP-MESSAGE ⑧ — content over 65535 bytes is 1002");
    assert!(chat.handle("a", "s1", Op::Message, ask("not-hex-at-all".into(), "가".repeat(21_845)), 70_000).await.is_ok(), "CP-OP-MESSAGE④ — the shape is not checked · 65535 bytes is taken");
    let f = chat.handle("a", "s1", Op::Message, ask(ID_B.into(), "x".into()), BODY_MAX + 1).await.err().expect("refused");
    assert_eq!(f.code, Code::InvalidPayload.num(), "연:RULE-CHAT-CONTENT — a body over 131072 received bytes is 1002");
});

async fn call(chat: &Chat, user: &str, op: Op, body: serde_json::Value) -> Result<(serde_json::Value, bool), u16> {
    chat.handle(user, "s1", op, body, 100).await.map(|a| (a.body, a.deliver.is_some())).map_err(|f| f.code)
}

fn say(space: &str, msg_id: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut b = serde_json::json!({ "space_id": space, "msg_id": msg_id });
    if let (Some(o), Some(e)) = (b.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    b
}

async fn seen_by(chat: &Chat, user: &str, other: &str) -> (serde_json::Value, Vec<String>) {
    let (page, _) = call(chat, user, Op::SpaceHistory, serde_json::json!({ "space_id": "S", "limit": 100 })).await.expect("history");
    (page, chat.store.pending(other).expect("pending"))
}

async_forms!(space_list_and_view_answer_as_the_store_stands [memory = Memory, sqlite = Sqlite, postgres = Postgres] |db| {
    let s = store(db, Visibility::Shared);
    s.create_space("E", None).expect("empty space");
    for sp in ["S", "E"] {
        join(&s, sp, "a", 1);
        join(&s, sp, "b", 1);
    }
    let chat = Chat::new(s, Limits::default());
    call(&chat, "a", Op::Message, say("S", ID_A, serde_json::json!({ "content": "hi" }))).await.expect("post");
    let (list, _) = call(&chat, "b", Op::SpaceList, serde_json::json!({})).await.expect("list");
    let entry = |sp: &str| list["spaces"].as_array().and_then(|v| v.iter().find(|e| e["space_id"] == sp).cloned()).expect("entry");
    assert!(entry("S").get("last_t").is_some_and(|t| t.is_u64()), "positive control — a space with a post carries last_t");
    assert!(entry("E").get("last_t").is_none(), "CP-OP-SPACE-LIST③ a space without messages has no last_t {}", entry("E"));
    let (v1, _) = call(&chat, "b", Op::SpaceView, serde_json::json!({ "space_id": "S" })).await.expect("view");
    let (v2, _) = call(&chat, "b", Op::SpaceView, serde_json::json!({ "space_id": "S" })).await.expect("view again");
    assert_eq!((v1["read"].as_u64(), v2["read"].as_u64()), (Some(1), Some(1)), "CP-OP-SPACE-VIEW③ sending it again gives the same read");
    let (after, _) = call(&chat, "b", Op::SpaceList, serde_json::json!({})).await.expect("list");
    let s_after = after["spaces"].as_array().and_then(|v| v.iter().find(|e| e["space_id"] == "S").cloned()).expect("S");
    assert_eq!((s_after["total"].as_u64(), s_after["read"].as_u64()), (Some(1), Some(1)), "CP-OP-SPACE-VIEW③ the second view leaves the read where the first put it");
});

async_forms!(refused_messages_store_nothing_and_notify_no_one [memory = Memory, sqlite = Sqlite, postgres = Postgres] |db| {
    let s = store(db, Visibility::Shared);
    s.create_space("T", None).expect("space T");
    for u in ["a", "b"] {
        join(&s, "S", u, 1);
    }
    join(&s, "T", "a", 1);
    let chat = Chat::new(s, Limits::default());
    let (_, fanned) = call(&chat, "a", Op::Message, say("S", &id(1), serde_json::json!({ "content": "hello" }))).await.expect("post");
    assert!(fanned, "positive control — a stored message is handed to the fan-out");
    call(&chat, "a", Op::Message, say("T", &id(2), serde_json::json!({ "content": "there" }))).await.expect("post in T");
    call(&chat, "b", Op::Message, say("S", &id(3), serde_json::json!({ "kind": "react", "ref": id(1), "keys": ["+1"] }))).await.expect("react");
    call(&chat, "a", Op::Message, say("S", &id(4), serde_json::json!({ "content": "gone" }))).await.expect("post");
    call(&chat, "a", Op::Message, say("S", &id(5), serde_json::json!({ "kind": "delete", "ref": id(4) }))).await.expect("delete");
    let before = seen_by(&chat, "a", "b").await;
    let first_t = before.0["messages"].as_array().and_then(|v| v.iter().find(|m| m["msg_id"] == id(1).as_str()).and_then(|m| m["t"].as_u64())).expect("first t");
    let cases: Vec<(&str, u16, serde_json::Value, &str)> = vec![
        ("a", Code::MsgIdConflict.num(), say("S", &id(1), serde_json::json!({ "content": "other" })), "CP-CODE-MSG-ID-CONFLICT②"),
        ("a", Code::RefNotFound.num(), say("S", &id(10), serde_json::json!({ "kind": "edit", "ref": "missing", "content": "x" })), "CP-CODE-REF-NOT-FOUND② unknown msg_id"),
        ("a", Code::RefNotFound.num(), say("S", &id(11), serde_json::json!({ "kind": "edit", "ref": id(2), "content": "x" })), "CP-CODE-REF-NOT-FOUND② a post of another space"),
        ("a", Code::RefNotFound.num(), say("S", &id(12), serde_json::json!({ "kind": "react", "ref": id(3), "keys": ["x"] })), "CP-CODE-REF-NOT-FOUND② a relation"),
        ("a", Code::RefDeleted.num(), say("S", &id(13), serde_json::json!({ "kind": "edit", "ref": id(4), "content": "back" })), "CP-CODE-REF-DELETED② edit"),
        ("b", Code::RefDeleted.num(), say("S", &id(14), serde_json::json!({ "kind": "react", "ref": id(4), "keys": ["x"] })), "CP-CODE-REF-DELETED② react"),
        ("a", Code::RefDeleted.num(), say("S", &id(15), serde_json::json!({ "kind": "delete", "ref": id(4) })), "CP-CODE-REF-DELETED② delete"),
        ("b", Code::NotRefSender.num(), say("S", &id(16), serde_json::json!({ "kind": "edit", "ref": id(1), "content": "mine" })), "CP-CODE-NOT-REF-SENDER② edit"),
        ("b", Code::NotRefSender.num(), say("S", &id(17), serde_json::json!({ "kind": "delete", "ref": id(1) })), "CP-CODE-NOT-REF-SENDER② delete"),
    ];
    for (user, code, body, what) in cases {
        let mid = body["msg_id"].as_str().unwrap_or_default().to_string();
        let before = seen_by(&chat, "a", "b").await;
        assert_eq!(call(&chat, user, Op::Message, body).await.err(), Some(code), "{what} — the code");
        assert_eq!(seen_by(&chat, "a", "b").await, before, "{what} — nothing stored · no undelivered record · no fan-out (the refusal carries no delivery) · CP-CODE-REF-NOT-FOUND② · CP-CODE-REF-DELETED② · CP-CODE-NOT-REF-SENDER② · CP-CODE-MSG-ID-CONFLICT②");
        if code != Code::MsgIdConflict.num() {
            let probe = call(&chat, user, Op::Message, say("S", &mid, serde_json::json!({ "content": "probe" }))).await;
            assert!(matches!(probe, Ok((_, true))), "{what} — nothing was stored under its msg_id (a post with the same msg_id is stored fresh) {probe:?}");
        }
    }
    let after = seen_by(&chat, "a", "b").await;
    let orig = after.0["messages"].as_array().and_then(|v| v.iter().find(|m| m["msg_id"] == id(1).as_str()).cloned()).expect("first");
    assert_eq!((orig["content"].as_str(), orig["t"].as_u64()), (Some("hello"), Some(first_t)), "CP-CODE-MSG-ID-CONFLICT② the message stored first stays as it was");
});

all_forms!(space_and_membership_checks |db| {
    let s = store(db, Visibility::Shared);
    assert_eq!(s.send("a", "nope", ID_A, "x", Some(1)), Err(Code::SpaceNotFound));
    assert_eq!(s.send("a", "S", ID_A, "x", Some(1)), Err(Code::NotSpaceMember), "CP-OP-MESSAGE⑤ — a non-member is 3013");
    join(&s, "S", "a", 1);
    s.leave("S", "a", None, Some(2)).expect("leave");
    assert_eq!(s.send("a", "S", ID_A, "x", Some(3)), Err(Code::NotSpaceMember), "a member who left is not a member");
    assert_eq!(s.history("a", "S", Dir::Latest, 60), Err(Code::NotSpaceMember));
    assert_eq!(s.catchup_check("a", "S"), Err(Code::NotSpaceMember), "CP-OP-CATCHUP④ — not a member is 3013");
    assert_eq!(s.catchup_check("a", "nope"), Err(Code::SpaceNotFound), "CP-OP-CATCHUP④ — no such space is 3012");
});

all_forms!(roster_at_message_time |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 10);
    join(&s, "S", "c", 10);
    let t = fresh_t(s.send("a", "S", ID_A, "m", Some(20)));
    join(&s, "S", "b", 30);
    s.leave("S", "c", None, Some(40)).expect("leave c");
    let at = s.roster("S").expect("roster").at(t);
    assert!(!at.contains(&"b".to_string()), "S-sym-IMPL-CHAT-FILTER-1 — a member who joined after M does not get M");
    assert!(at.contains(&"c".to_string()), "S-sym-IMPL-CHAT-FILTER-1 — a member who left after M still gets M");
});

async_forms!(cache_reloads_only_when_older_than_the_message [memory = Memory, sqlite = Sqlite, postgres = Postgres] |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 10);
    let v1 = s.roster("S").expect("roster").ver;
    let chat = Chat::new(s, Limits::default());
    assert_eq!(chat.roster_at_least("S", v1).await.expect("load").ver, v1);
    chat.store.join("S", "b", None, Some(20)).expect("join b");
    let got = chat.roster_at_least("S", v1).await.expect("cached");
    assert_eq!(got.ver, v1, "S-sym-IMPL-CHAT-FILTER-2 — a cache not older than the message is not reread");
    let got = chat.roster_at_least("S", v1 + 1).await.expect("reload");
    assert_eq!(got.ver, v1 + 1, "an older cache is reread");
    assert!(got.at(25).contains(&"b".to_string()));
});

all_forms!(history_pages_after_before_and_latest |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    let ids: Vec<String> = (0..5).map(id).collect();
    let ts: Vec<u64> = ids.iter().zip(100u64..).map(|(m, at)| fresh_t(s.send("a", "S", m, "m", Some(at)))).collect();
    let p = s.history("a", "S", Dir::Latest, 2).expect("latest");
    assert_eq!(p.messages.iter().map(|m| m.t).collect::<Vec<_>>(), ts[3..].to_vec(), "latest page in t order");
    assert!(p.more);
    let p = s.history("a", "S", Dir::Before(ts[3]), 2).expect("before");
    assert_eq!(p.messages.iter().map(|m| m.t).collect::<Vec<_>>(), ts[1..3].to_vec(), "before goes back from the nearest");
    assert!(p.more);
    let p = s.history("a", "S", Dir::After(ts[1]), 10).expect("after");
    assert_eq!(p.messages.iter().map(|m| m.t).collect::<Vec<_>>(), ts[2..].to_vec(), "after is ascending with no gap");
    assert!(!p.more);
});

all_forms!(a_page_never_splits_one_t |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    for (i, at) in [100u64, 200, 200, 200, 300].iter().enumerate() {
        fresh_t(s.send("a", "S", &id(i), "m", Some(*at)));
    }
    let ts = |p: &Page| p.messages.iter().map(|m| m.t).collect::<Vec<_>>();
    let p = s.history("a", "S", Dir::Latest, 2).expect("latest");
    assert_eq!(ts(&p), vec![200, 200, 200, 300], "CP-OP-SPACE-HISTORY⑤ — the page takes every message of its edge t");
    assert!(p.more);
    let same: Vec<&str> = p.messages[..3].iter().map(|m| m.msg_id.as_str()).collect();
    let mut sorted = same.clone();
    sorted.sort();
    assert_eq!(same, sorted, "CP-OP-SPACE-HISTORY① — same t is in msg_id order");
    let p = s.history("a", "S", Dir::Before(200), 2).expect("before");
    assert_eq!((ts(&p), p.more), (vec![1, 100], false), "the join message at t 1 and the post at 100");
    let p = s.history("a", "S", Dir::After(100), 1).expect("after");
    assert_eq!((ts(&p), p.more), (vec![200, 200, 200], true), "CP-OP-SPACE-HISTORY⑤ — after keeps one t whole too");
    let p = s.history("a", "S", Dir::After(200), 1).expect("after");
    assert_eq!((ts(&p), p.more), (vec![300], false), "continuing from the edge t loses nothing and repeats nothing");
});

all_forms!(joined_visibility_hides_what_came_before_the_interval |db| {
    let s = store(db, Visibility::Joined);
    join(&s, "S", "a", 1);
    let t_before = fresh_t(s.send("a", "S", ID_A, "before", Some(10)));
    join(&s, "S", "b", 20);
    let t_after = fresh_t(s.send("a", "S", ID_B, "after", Some(30)));
    let p = s.history("b", "S", Dir::Latest, 60).expect("history");
    assert_eq!(posts(&p).iter().map(|m| m.t).collect::<Vec<_>>(), vec![t_after], "joined — b does not see {t_before} sent before b joined");
    s.create_space("P", Some(Visibility::Shared)).expect("shared space");
    join(&s, "P", "a", 1);
    fresh_t(s.send("a", "P", ID_C, "old", Some(40)));
    join(&s, "P", "b", 50);
    assert_eq!(posts(&s.history("b", "P", Dir::Latest, 60).expect("history")).len(), 1, "a space value overrides the default");
});

all_forms!(unread_is_total_minus_read |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    fresh_t(s.send("a", "S", ID_A, "1", Some(10)));
    fresh_t(s.send("a", "S", ID_B, "2", Some(10)));
    join(&s, "S", "c", 20);
    let e = s.list("b").expect("list").remove(0);
    assert_eq!((e.total, e.read), (2, 0), "연:RULE-CHAT-UNREAD — member change messages are not counted");
    assert_eq!(e.last_t, Some(10));
    assert_eq!(s.view("b", "S").expect("view"), 2);
    let e = s.list("b").expect("list").remove(0);
    assert_eq!(e.total - e.read, 0, "after view nothing is unread");
    let a = s.list("a").expect("list").remove(0);
    assert_eq!(a.total - a.read, 0, "the sender has read up to what it sent");
});

all_forms!(undelivered_is_written_with_the_message_and_cleared_by_ack |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    join(&s, "S", "c", 1);
    for u in ["a", "b", "c"] {
        let ids = s.pending(u).expect("pending");
        s.ack(u, &ids).expect("ack");
    }
    fresh_t(s.send("a", "S", ID_A, "m", Some(10)));
    assert_eq!(s.pending("a").expect("a"), Vec::<String>::new(), "S-sym-IMPL-CHAT-STORE-2 — no record for the sender");
    assert_eq!(s.pending("b").expect("b"), vec![ID_A.to_string()], "S-sym-IMPL-CHAT-STORE-2 — every other member has a record");
    assert_eq!(s.pending("c").expect("c"), vec![ID_A.to_string()]);
    s.ack("b", &[ID_A.to_string()]).expect("ack");
    assert!(s.pending("b").expect("b").is_empty(), "CP-OP-MESSAGE⑨ — an ACK clears that member's record");
    assert_eq!(s.pending("c").expect("c"), vec![ID_A.to_string()], "another member's record stays");
    s.purge(11).expect("purge");
    assert!(s.pending("c").expect("c").is_empty(), "records older than the keep window are removed");
});

all_forms!(member_change_is_a_message |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    let (m, _) = s.join("S", "b", Some(SYS), Some(5)).expect("add").expect("new member");
    assert_eq!((m.kind, m.user_id.as_str(), m.target.as_deref(), m.content.as_str(), m.t), (Some(ChatKind::Add), SYS, Some("b"), "", 5));
    assert!(m.msg_id.starts_with(SYS_HASH) && m.msg_id.len() == 36, "연:RULE-CHAT-MSG-ID — the server's own msg_id {}", m.msg_id);
    assert!(s.pending("b").expect("b").contains(&m.msg_id), "연:RULE-CHAT-MEMBER-NOTICE — the added member gets its add message");
    let (l, _) = s.leave("S", "a", None, Some(9)).expect("leave");
    assert_eq!((l.kind, l.user_id.as_str(), l.target.as_deref()), (Some(ChatKind::Leave), "a", None));
    assert!(!s.pending("a").expect("a").contains(&l.msg_id), "the one who left is not a member at that t");
    assert!(s.pending("b").expect("b").contains(&l.msg_id));
    assert_eq!(s.list("b").expect("list")[0].total, 0, "member change messages are not counted");
});

all_forms!(catchup_merges_records_and_since_in_t_order |db| {
    let s = store(db, Visibility::Shared);
    s.create_space("T", None).expect("space T");
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    join(&s, "T", "a", 1);
    join(&s, "T", "b", 1);
    let ids = s.pending("b").expect("b");
    s.ack("b", &ids).expect("ack joins");
    for (i, (sp, at)) in [("S", 100u64), ("T", 100), ("S", 200), ("T", 300)].iter().enumerate() {
        fresh_t(s.send("a", sp, &id(i), "m", Some(*at)));
    }
    s.ack("b", &[id(2)]).expect("b got the S post at 200 live");
    let page = s.catchup("b", None, None, &Cursor::default(), 100, 1 << 20).expect("records only");
    assert_eq!(page.iter().map(|m| (m.t, m.msg_id.clone())).collect::<Vec<_>>(), vec![(100, id(0)), (100, id(1)), (300, id(3))], "CP-OP-CATCHUP② — records in (t, msg_id) order");
    let page = s.catchup("b", None, Some(200), &Cursor::default(), 100, 1 << 20).expect("since");
    assert_eq!(page.iter().map(|m| m.msg_id.clone()).collect::<Vec<_>>(), vec![id(0), id(1), id(2), id(3)], "CP-OP-CATCHUP② — t ≥ 200 joins the records once each");
    let page = s.catchup("b", Some("S"), None, &Cursor::default(), 100, 1 << 20).expect("one space");
    assert_eq!(page.iter().map(|m| m.msg_id.clone()).collect::<Vec<_>>(), vec![id(0)], "CP-OP-CATCHUP③ — one space only");
    let first = s.catchup("b", None, None, &Cursor::default(), 2, 1 << 20).expect("bundle");
    assert_eq!(first.len(), 2, "CP-OP-MESSAGES① — at most the bundle count");
    let last = first.last().expect("last");
    let next = s.catchup("b", None, None, &Cursor { t: last.t, msg_id: last.msg_id.clone() }, 2, 1 << 20).expect("next");
    assert_eq!(next.iter().map(|m| m.msg_id.clone()).collect::<Vec<_>>(), vec![id(3)], "the cursor continues without a gap or a repeat");
    let one = s.catchup("b", None, None, &Cursor::default(), 100, 1).expect("bytes");
    assert_eq!(one.len(), 1, "CP-OP-MESSAGES① — a message over the byte cap goes alone");
    s.leave("S", "b", None, Some(400)).expect("leave S");
    let page = s.catchup("b", None, Some(0), &Cursor::default(), 100, 1 << 20).expect("since after leaving");
    assert!(page.iter().filter(|m| m.space_id == "S").all(|m| m.msg_id == id(0)), "a space left is not read by t — only its records come");
});

shared_forms!(two_stores_on_one_file_count_every_message |db| {
    let a = db.open(Visibility::Shared);
    a.create_space("S", None).expect("space");
    join(&a, "S", "a", 1);
    let b = db.open(Visibility::Shared);
    let run = |st: Store, k: u32| {
        std::thread::spawn(move || {
            (0..40u32)
                .map(|i| match st.send("a", "S", &format!("{k:04x}{i:04x}"), "m", Some(5000)).expect("send") {
                    Sent::Fresh { msg, .. } => msg.t,
                    Sent::Again { .. } => panic!("fresh ids"),
                })
                .collect::<Vec<u64>>()
        })
    };
    let (ha, hb) = (run(a, 1), run(b, 2));
    let ts: Vec<u64> = ha.join().expect("a").into_iter().chain(hb.join().expect("b")).collect();
    assert!(ts.iter().all(|t| *t == 5000), "every message keeps its store t");
    let c = db.open(Visibility::Shared);
    let e = c.list("a").expect("list").remove(0);
    assert_eq!((e.total, e.last_t), (80, Some(5000)), "two gateways on one store — total counts every message");
});

shared_forms!(visibility_is_fixed_at_join |db| {
    let s = db.open(Visibility::Joined);
    s.create_space("S", None).expect("space");
    join(&s, "S", "a", 1);
    fresh_t(s.send("a", "S", ID_A, "before", Some(10)));
    join(&s, "S", "b", 20);
    drop(s);
    let s = db.open(Visibility::Shared);
    assert!(posts(&s.history("b", "S", Dir::Latest, 60).expect("history")).is_empty(), "a later policy change does not reach a member who already joined");
    join(&s, "S", "c", 30);
    assert_eq!(posts(&s.history("c", "S", Dir::Latest, 60).expect("history")).len(), 1, "a new member gets the policy of its own join");
});

all_forms!(a_retry_after_leaving_still_gets_the_first_t |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    let t = fresh_t(s.send("a", "S", ID_A, "x", Some(10)));
    s.leave("S", "a", None, Some(20)).expect("leave");
    match s.send("a", "S", ID_A, "x", Some(30)).expect("retry") {
        Sent::Again { msg } => assert_eq!(msg.t, t, "the stored message answers the retry even after leaving"),
        Sent::Fresh { .. } => panic!("must not store again"),
    }
});

all_forms!(same_msg_id_in_another_space_conflicts |db| {
    let s = store(db, Visibility::Shared);
    s.create_space("T", None).expect("space T");
    join(&s, "S", "a", 1);
    join(&s, "T", "a", 1);
    fresh_t(s.send("a", "S", ID_A, "x", Some(10)));
    assert_eq!(s.send("a", "T", ID_A, "x", Some(10)), Err(Code::MsgIdConflict), "a msg_id is global — another space is a conflict");
});

fn rel(kind: ChatKind, to: &str) -> Outgoing {
    Outgoing { kind: Some(kind), r#ref: Some(to.into()), ..Outgoing::default() }
}

fn edit(to: &str, content: &str) -> Outgoing {
    Outgoing { content: content.into(), ext: Some(serde_json::json!({ "app": 1 })), ..rel(ChatKind::Edit, to) }
}

fn react(to: &str, keys: &[&str]) -> Outgoing {
    Outgoing { keys: Some(keys.iter().map(|k| k.to_string()).collect()), ..rel(ChatKind::React, to) }
}

all_forms!(relations_are_checked_on_the_pointer_only |db| {
    let s = store(db, Visibility::Shared);
    s.create_space("T", None).expect("space T");
    for u in ["a", "b"] {
        join(&s, "S", u, 1);
    }
    join(&s, "T", "a", 1);
    fresh_t(s.send("a", "S", &id(1), "hello", Some(10)));
    fresh_t(s.send("a", "T", &id(2), "there", Some(10)));
    assert_eq!(s.post("b", "S", &id(3), &edit(&id(1), "x"), Some(11)), Err(Code::NotRefSender), "연:RULE-CHAT-REF-CHECK — someone else's post cannot be edited");
    assert_eq!(s.post("b", "S", &id(3), &rel(ChatKind::Delete, &id(1)), Some(11)), Err(Code::NotRefSender), "nor deleted");
    assert!(matches!(s.post("b", "S", &id(4), &react(&id(1), &["+1"]), Some(12)), Ok(Sent::Fresh { .. })), "CP-CODE-NOT-REF-SENDER③ a react on someone else's post is taken");
    assert_eq!(s.post("a", "S", &id(5), &edit("missing", "x"), Some(13)), Err(Code::RefNotFound), "an unknown msg_id");
    assert_eq!(s.post("a", "S", &id(5), &edit(&id(2), "x"), Some(13)), Err(Code::RefNotFound), "a post of another space");
    assert_eq!(s.post("a", "S", &id(5), &react(&id(4), &["x"]), Some(13)), Err(Code::RefNotFound), "a relation is not a post");
    let reply = Outgoing { r#ref: Some(id(1)), ..Outgoing::text("reply") };
    fresh_t(s.post("b", "S", &id(6), &reply, Some(14)));
    assert_eq!(s.list("a").expect("list").iter().find(|e| e.space_id == "S").expect("S").total, 2, "연:RULE-CHAT-UNREAD — posts only (the post and the reply)");
});

all_forms!(delete_clears_the_post_and_what_points_at_it |db| {
    let s = store(db, Visibility::Shared);
    for u in ["a", "b"] {
        join(&s, "S", u, 1);
    }
    fresh_t(s.send("a", "S", &id(1), "secret", Some(10)));
    fresh_t(s.post("a", "S", &id(2), &edit(&id(1), "secret v2"), Some(11)));
    fresh_t(s.post("b", "S", &id(3), &react(&id(1), &["+1"]), Some(12)));
    fresh_t(s.post("a", "S", &id(4), &rel(ChatKind::Delete, &id(1)), Some(13)));
    assert_eq!(s.post("a", "S", &id(5), &edit(&id(1), "again"), Some(14)), Err(Code::RefDeleted), "연:RULE-CHAT-DELETE — no edit after delete");
    assert_eq!(s.post("b", "S", &id(5), &react(&id(1), &["x"]), Some(14)), Err(Code::RefDeleted), "no reaction after delete");
    assert_eq!(s.post("a", "S", &id(5), &rel(ChatKind::Delete, &id(1)), Some(14)), Err(Code::RefDeleted), "no second delete");
    assert!(matches!(s.post("b", "S", &id(6), &Outgoing { r#ref: Some(id(1)), ..Outgoing::text("why?") }, Some(15)), Ok(Sent::Fresh { .. })), "CP-CODE-REF-DELETED③ a reply (a post's ref) to the deleted post is taken");
    let page = s.history("b", "S", Dir::Latest, 50).expect("history");
    let orig = page.messages.iter().find(|m| m.msg_id == id(1)).expect("the envelope stays");
    assert_eq!((orig.content.as_str(), orig.ext.is_none()), ("", true), "S-sym-IMPL-CHAT-STORE-5① — the post is emptied");
    let ed = page.relations.iter().find(|m| m.kind == Some(ChatKind::Edit)).expect("the edit envelope stays");
    assert_eq!((ed.content.as_str(), ed.ext.is_none()), ("", true), "S-sym-IMPL-CHAT-STORE-5② — the edited text is gone too");
    let re = page.relations.iter().find(|m| m.kind == Some(ChatKind::React)).expect("the reaction envelope stays");
    assert_eq!(re.keys.as_deref(), Some(&[][..]), "S-sym-IMPL-CHAT-STORE-5③ — reaction keys are emptied");
    assert!(page.relations.iter().any(|m| m.kind == Some(ChatKind::Delete)), "the delete rides with the page");
    assert!(page.messages.iter().all(|m| !m.content.contains("secret")) && page.relations.iter().all(|m| !m.content.contains("secret")));
});

all_forms!(a_page_counts_visible_messages_and_folds_relations |db| {
    let s = store(db, Visibility::Shared);
    for u in ["a", "b"] {
        join(&s, "S", u, 1);
    }
    fresh_t(s.send("a", "S", &id(1), "p", Some(10)));
    fresh_t(s.post("a", "S", &id(2), &edit(&id(1), "p1"), Some(20)));
    fresh_t(s.post("a", "S", &id(3), &edit(&id(1), "p2"), Some(30)));
    fresh_t(s.post("b", "S", &id(4), &react(&id(1), &["x"]), Some(21)));
    fresh_t(s.post("b", "S", &id(5), &react(&id(1), &["y", "z"]), Some(31)));
    fresh_t(s.post("a", "S", &id(6), &react(&id(1), &["x"]), Some(32)));
    let page = s.history("a", "S", Dir::Latest, 1).expect("history");
    assert_eq!(page.messages.iter().map(|m| m.msg_id.clone()).collect::<Vec<_>>(), vec![id(1)], "연:RULE-CHAT-HISTORY-PAGE — relations are not counted in the page");
    let ed: Vec<&ChatMessage> = page.relations.iter().filter(|m| m.kind == Some(ChatKind::Edit)).collect();
    assert_eq!((ed.len(), ed[0].content.as_str()), (1, "p2"), "the last edit by (t, msg_id) only");
    let mut rx: Vec<(String, Vec<String>)> = page.relations.iter().filter(|m| m.kind == Some(ChatKind::React)).map(|m| (m.user_id.clone(), m.keys.clone().unwrap_or_default())).collect();
    rx.sort();
    assert_eq!(rx, vec![("a".into(), vec!["x".to_string()]), ("b".into(), vec!["y".to_string(), "z".to_string()])], "the last reaction per sender");
    let tie = s.history("a", "S", Dir::Latest, 5).expect("again");
    assert!(posts(&tie).len() == 1 && !tie.more, "no more pages — only one visible post");
});

all_forms!(a_tie_on_t_is_broken_by_msg_id |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    fresh_t(s.send("a", "S", &id(1), "p", Some(10)));
    fresh_t(s.post("a", "S", &id(9), &edit(&id(1), "later id"), Some(20)));
    fresh_t(s.post("a", "S", &id(8), &edit(&id(1), "earlier id"), Some(20)));
    let page = s.history("a", "S", Dir::Latest, 10).expect("history");
    let ed: Vec<&ChatMessage> = page.relations.iter().filter(|m| m.kind == Some(ChatKind::Edit)).collect();
    assert_eq!(ed[0].content, "later id", "연:RULE-CHAT-FOLD — same t, the larger msg_id wins");
});

fn side(db: &form::Db, a: &Arc<Store>) -> Arc<Store> {
    if db.shared() { Arc::new(db.open(Visibility::Shared)) } else { a.clone() }
}

all_forms!(delete_and_edit_racing_on_one_post_leave_no_text |db| {
    let a = Arc::new(db.open(Visibility::Shared));
    a.create_space("S", None).expect("space");
    a.join("S", "u", None, Some(1)).expect("join");
    let (s1, s2) = (side(db, &a), side(db, &a));
    for round in 0..50usize {
        let orig = id(1000 + round);
        fresh_t(a.send("u", "S", &orig, "text", None));
        let gate = Arc::new(std::sync::Barrier::new(2));
        let (st, g, o) = (s1.clone(), gate.clone(), orig.clone());
        let del = std::thread::spawn(move || {
            g.wait();
            st.post("u", "S", &format!("d{round:035}"), &rel(ChatKind::Delete, &o), None)
        });
        let (st, g, o) = (s2.clone(), gate.clone(), orig.clone());
        let ed = std::thread::spawn(move || {
            g.wait();
            st.post("u", "S", &format!("e{round:035}"), &edit(&o, "revived"), None)
        });
        let (d, e) = (del.join().expect("del"), ed.join().expect("edit"));
        assert!(d.is_ok(), "the delete goes through {d:?}");
        assert!(e.is_ok() || e == Err(Code::RefDeleted), "S-sym-IMPL-CHAT-STORE-4① — the edit either went first or is refused: {e:?}");
    }
    let page = a.history("u", "S", Dir::Latest, 200).expect("history");
    assert!(page.relations.iter().all(|m| m.content.is_empty()), "S-sym-IMPL-CHAT-STORE-4② — no edit text survives a delete");
});

all_forms!(the_same_msg_id_on_two_connections_is_one_message |db| {
    let a = Arc::new(store(db, Visibility::Shared));
    join(&a, "S", "u", 1);
    join(&a, "S", "v", 1);
    let (s1, s2) = (side(db, &a), side(db, &a));
    let rounds = 50usize;
    for round in 0..rounds {
        let mid = id(5000 + round);
        let gate = Arc::new(std::sync::Barrier::new(2));
        let go = |st: Arc<Store>, g: Arc<std::sync::Barrier>, m: String| {
            std::thread::spawn(move || {
                g.wait();
                st.send("u", "S", &m, "same", None)
            })
        };
        let (h1, h2) = (go(s1.clone(), gate.clone(), mid.clone()), go(s2.clone(), gate.clone(), mid.clone()));
        let (r1, r2) = (h1.join().expect("one"), h2.join().expect("two"));
        let t = |r: &Result<Sent, Code>| match r {
            Ok(Sent::Fresh { msg, .. }) | Ok(Sent::Again { msg }) => Some(msg.t),
            Err(_) => None,
        };
        assert!(r1.is_ok() && r2.is_ok(), "S-sym-IMPL-CHAT-STORE-FORMS-3 — 둘 다 성공 · 원인 없는 INTERNAL_ERROR 0 (round {round}: {r1:?} {r2:?})");
        assert_eq!(t(&r1), t(&r2), "S-sym-IMPL-CHAT-STORE-FORMS-3 — 같은 t");
        let fresh = [&r1, &r2].iter().filter(|r| matches!(r, Ok(Sent::Fresh { .. }))).count();
        assert_eq!(fresh, 1, "S-sym-IMPL-CHAT-STORE-FORMS-3 — 받는 사람 통지 1(새로 저장은 하나)");
    }
    assert_eq!(a.list("u").expect("list")[0].total, rounds as u64, "S-sym-IMPL-CHAT-STORE-FORMS-3 — 메시지 행 1 씩");
    let mids: Vec<String> = (0..rounds).map(|r| id(5000 + r)).collect();
    let pend = a.pending("v").expect("v");
    assert_eq!(pend.iter().filter(|m| mids.contains(m)).count(), rounds, "S-sym-IMPL-CHAT-STORE-FORMS-3 — 미전송 기록도 한 건씩");
});

all_forms!(a_retry_after_delete_still_answers_the_first_t |db| {
    let s = store(db, Visibility::Shared);
    for u in ["a", "b"] {
        join(&s, "S", u, 1);
    }
    let t0 = fresh_t(s.send("a", "S", &id(1), "post", Some(10)));
    let tr = fresh_t(s.post("b", "S", &id(2), &react(&id(1), &["+1"]), Some(11)));
    let te = fresh_t(s.post("a", "S", &id(3), &edit(&id(1), "post v2"), Some(12)));
    fresh_t(s.post("a", "S", &id(4), &rel(ChatKind::Delete, &id(1)), Some(13)));
    for (who, mid, out, t) in [("b", id(2), react(&id(1), &["+1"]), tr), ("a", id(3), edit(&id(1), "post v2"), te), ("a", id(1), Outgoing::text("post"), t0)] {
        match s.post(who, "S", &mid, &out, Some(20)).expect("retry after the delete emptied it") {
            Sent::Again { msg } => assert_eq!(msg.t, t, "연:RULE-CHAT-MSG-ID — an emptied message answers its first t"),
            Sent::Fresh { .. } => panic!("must not store again"),
        }
    }
    assert!(matches!(s.post("b", "S", &id(2), &react(&id(1), &["x"]), Some(20)), Ok(Sent::Again { .. })), "once emptied only the envelope decides — the stored body is gone");
    assert_eq!(s.post("a", "S", &id(2), &react(&id(1), &["+1"]), Some(20)), Err(Code::MsgIdConflict), "another sender is still a conflict");
});

all_forms!(relations_follow_the_visibility_start |db| {
    let s = store(db, Visibility::Joined);
    join(&s, "S", "a", 1);
    join(&s, "S", "u", 2);
    fresh_t(s.send("a", "S", &id(1), "p", Some(10)));
    s.leave("S", "u", None, Some(20)).expect("leave");
    fresh_t(s.post("a", "S", &id(2), &edit(&id(1), "while away"), Some(25)));
    s.join("S", "u", None, Some(30)).expect("rejoin");
    fresh_t(s.post("a", "S", &id(3), &react(&id(1), &["+1"]), Some(35)));
    let page = s.history("u", "S", Dir::After(0), 50).expect("history");
    assert!(page.messages.iter().any(|m| m.msg_id == id(1)), "the post from the first interval is visible");
    assert!(!page.relations.iter().any(|m| m.msg_id == id(2)), "연:RULE-CHAT-VISIBLE — an edit from the gap is not shown");
    assert!(page.relations.iter().any(|m| m.msg_id == id(3)), "a reaction inside the interval is shown");
});

all_forms!(a_page_stays_within_the_byte_budget |db| {
    let s = store(db, Visibility::Shared);
    for u in ["a", "b"] {
        join(&s, "S", u, 1);
    }
    let big = "x".repeat(10_000);
    for i in 0..10u64 {
        fresh_t(s.send("a", "S", &id(i as usize + 1), &big, Some(100 + i)));
    }
    fresh_t(s.post("b", "S", &id(50), &react(&id(10), &["+1"]), Some(200)));
    let page = s.history_within("b", "S", Dir::Latest, 60, 35_000).expect("latest");
    let ids: Vec<String> = page.messages.iter().map(|m| m.msg_id.clone()).collect();
    assert_eq!(ids, vec![id(8), id(9), id(10)], "연:RULE-CHAT-HISTORY-PAGE — the latest side is kept within the budget");
    assert!(page.more, "more after a byte cut");
    assert!(page.relations.iter().any(|m| m.msg_id == id(50)), "relations of kept posts ride along");
    let first = s.history_within("b", "S", Dir::After(0), 60, 35_000).expect("after");
    assert_eq!(posts(&first).iter().map(|m| m.msg_id.clone()).collect::<Vec<_>>(), vec![id(1), id(2), id(3)], "after keeps the near side (member changes at t=1 ride first)");
    let one = s.history_within("b", "S", Dir::Latest, 60, 10).expect("tiny");
    assert_eq!(one.messages.len(), 1, "one message over the budget goes alone");
});

#[test]
fn message_shapes_follow_the_kind() {
    let ok = |v: serde_json::Value| outgoing(&v).is_ok();
    assert!(ok(serde_json::json!({ "content": "x" })), "a post");
    assert!(ok(serde_json::json!({ "content": "x", "ref": "m", "ext": { "k": [1] } })), "a reply with app data");
    assert!(ok(serde_json::json!({ "kind": "edit", "ref": "m", "content": "y" })));
    assert!(ok(serde_json::json!({ "kind": "react", "ref": "m", "keys": [] })), "an empty set removes all");
    assert!(ok(serde_json::json!({ "kind": "delete", "ref": "m" })));
    for bad in [
        serde_json::json!({ "kind": "join", "content": "" }),
        serde_json::json!({ "kind": "nope", "content": "" }),
        serde_json::json!({ "keys": ["x"], "content": "x" }),
        serde_json::json!({ "kind": "react", "ref": "m", "keys": ["x"], "content": "x" }),
        serde_json::json!({ "kind": "delete", "ref": "m", "keys": [] }),
        serde_json::json!({ "content": "x", "ext": "not an object" }),
        serde_json::json!({ "content": "x", "target": "u" }),
    ] {
        assert_eq!(outgoing(&bad).err().map(|f| f.code), Some(Code::InvalidPayload.num()), "연:RULE-CHAT-KIND — a field the kind does not take is 1002: {bad}");
    }
    for missing in [
        serde_json::json!({ "kind": "edit", "ref": "m" }),
        serde_json::json!({ "kind": "react", "keys": ["x"] }),
        serde_json::json!({ "kind": "react", "ref": "m" }),
        serde_json::json!({}),
    ] {
        assert_eq!(outgoing(&missing).err().map(|f| f.code), Some(Code::MissingField.num()), "연:RULE-ERR-ONE-NUMBER-ONE-MEANING — a field the kind needs is 1003: {missing}");
    }
}

lite_forms!(relations_ride_the_ref_index |db| {
    let s = store(db, Visibility::Shared);
    for joined in [false, true] {
        let p = s.plan(&relations_sql(joined, 3)).expect("plan");
        assert!(p.contains("messages_ref"), "정:IMPL-CHAT-READ — 관계는 ref 색인으로 찾는다(공간 전체를 훑지 않는다 · joined={joined}):\n{p}");
        assert!(!p.contains("messages_space_t_id"), "정:IMPL-CHAT-READ — 관계 질의가 공간 색인으로 공간 전체를 훑는다(joined={joined}):\n{p}");
    }
});

lite_forms!(catchup_since_starts_from_the_members_with_one_floor |db| {
    let s = store(db, Visibility::Shared);
    let all = s.plan(&catchup_since_sql(false)).expect("plan");
    assert!(all.lines().next().is_some_and(|l| l.starts_with("SEARCH mm USING INDEX members_user")), "정:IMPL-CHAT-CATCHUP — 공간 없는 따라잡기는 그 사람의 멤버 행에서 출발한다(파일 전체를 훑지 않는다):\n{all}");
    for (space, p) in [(false, all), (true, s.plan(&catchup_since_sql(true)).expect("plan"))] {
        assert!(p.contains("messages_space_t_id (space_id=? AND (t,msg_id)>(?,?))"), "정:IMPL-CHAT-CATCHUP — t 이상 질의의 범위는 커서 하나에서 시작한다(space={space}):\n{p}");
    }
});

all_forms!(catchup_since_floor_is_the_later_of_since_and_cursor |db| {
    let s = store(db, Visibility::Shared);
    s.create_space("T", None).expect("space T");
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    join(&s, "T", "a", 1);
    for (i, at) in [100u64, 200, 200, 300].iter().enumerate() {
        fresh_t(s.send("a", "S", &id(i), "m", Some(*at)));
    }
    fresh_t(s.send("a", "T", &id(9), "not b's", Some(250)));
    let ids = s.pending("b").expect("b");
    s.ack("b", &ids).expect("ack all");
    let got = |since: u64, after: Cursor| s.catchup("b", None, Some(since), &after, 100, 1 << 20).expect("since").iter().map(|m| m.msg_id.clone()).collect::<Vec<_>>();
    assert_eq!(got(200, Cursor::default()), vec![id(1), id(2), id(3)], "연:RULE-CHAT-CATCHUP — t 는 이상(≥) · 멤버 아닌 공간은 안 온다");
    assert_eq!(got(200, Cursor { t: 200, msg_id: id(1) }), vec![id(2), id(3)], "커서가 since 뒤면 커서 다음부터");
    assert_eq!(got(300, Cursor { t: 100, msg_id: id(0) }), vec![id(3)], "since 가 커서 뒤면 since 부터");
});

mod diag {
    use std::fmt::Debug;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata};

    #[derive(Default)]
    struct Lines {
        got: Mutex<Vec<(String, String)>>,
        next: AtomicU64,
    }

    #[derive(Default)]
    struct Seen {
        kind: String,
        spec: String,
        all: String,
    }

    impl Visit for Seen {
        fn record_str(&mut self, f: &Field, v: &str) {
            self.put(f.name(), v.to_string());
        }
        fn record_debug(&mut self, f: &Field, v: &dyn Debug) {
            self.put(f.name(), format!("{v:?}"));
        }
    }

    impl Seen {
        fn put(&mut self, name: &str, v: String) {
            match name {
                "kind" => self.kind = v,
                "spec" => self.spec = v,
                _ => self.all.push_str(&format!("{name}={v} ")),
            }
        }
    }

    impl tracing::Subscriber for Lines {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(self.next.fetch_add(1, Ordering::Relaxed) + 1)
        }
        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn event(&self, e: &Event<'_>) {
            let mut seen = Seen::default();
            e.record(&mut seen);
            if seen.kind == "diag"
                && let Ok(mut g) = self.got.lock()
            {
                g.push((seen.spec, seen.all));
            }
        }
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
    }

    pub fn capture<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
        let lines = std::sync::Arc::new(Lines::default());
        let out = tracing::subscriber::with_default(lines.clone(), f);
        let got = lines.got.lock().map(|g| g.iter().filter(|(s, _)| s == "SV-IMPL-CHAT-STORE-FAULTS").map(|(_, a)| a.clone()).collect()).unwrap_or_default();
        (out, got)
    }
}

shared_forms!(a_locked_store_fails_with_its_cause_on_a_diag_line |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    let other = Outside::of(db);
    other.hold();
    let (r, lines) = diag::capture(|| s.view("a", "S"));
    let want = if db.kind == form::Kind::Postgres { "cause=55P03" } else { "cause=DatabaseBusy" };
    assert_eq!(r, Err(Code::InternalError), "S-sym-IMPL-CHAT-STORE-FAULTS-1 — 클라에는 코드 하나");
    assert_eq!(lines.len(), 1, "S-sym-IMPL-CHAT-STORE-FAULTS-1 — 원인을 든 진단 줄 1 {lines:?}");
    assert!(lines[0].contains(want), "S-sym-IMPL-CHAT-STORE-FAULTS-1 — 원인(잠김 {want})이 줄에 있다 {lines:?}");
    other.release();
    assert!(s.view("a", "S").is_ok(), "잠금이 풀리면 다시 된다");
});

all_forms!(acks_in_one_drain_are_one_transaction |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    for i in 0..40 {
        fresh_t(s.send("a", "S", &id(i), "m", Some(100 + i as u64)));
    }
    let ids = s.pending("b").expect("b");
    assert!(ids.len() >= 40, "형상 — b 몫 기록 {}", ids.len());
    let (tx, rx) = std::sync::mpsc::sync_channel(ids.len());
    for m in &ids[1..] {
        tx.send(("b".to_string(), vec![m.clone()])).expect("queue");
    }
    let before = s.ack_transactions();
    let rows = s.ack_drain(Lane::Member, ("b".to_string(), vec![ids[0].clone()]), || rx.try_recv().ok(), oxhubd::chat::ACK_BATCH).expect("drain");
    assert_eq!(rows, ids.len(), "형상 — 줄 선 ACK 를 한 번에 거뒀다");
    assert!(s.pending("b").expect("b").is_empty(), "S-sym-IMPL-CHAT-UNDELIVERED-2① — ACK {} 건이 기록을 모두 지운다", ids.len());
    assert_eq!(s.ack_transactions() - before, 1, "S-sym-IMPL-CHAT-UNDELIVERED-2② — 묶음 하나 = 트랜잭션 하나(ACK {} 건)", ids.len());
});

async_forms!(acks_are_cleared_in_fewer_transactions_than_acks [sqlite = Sqlite, postgres = Postgres] |db| {
    let chat = Chat::new(db.open(Visibility::Shared), Limits::default());
    let s = chat.store.clone();
    s.create_space("S", None).expect("space");
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    for i in 0..40 {
        fresh_t(s.send("a", "S", &id(i), "m", Some(100 + i as u64)));
    }
    let ids = s.pending("b").expect("b");
    assert!(ids.len() >= 40, "형상 — b 몫 기록 {}", ids.len());
    let before = s.ack_transactions();
    let other = Outside::of(db);
    other.hold();
    for m in &ids {
        chat.acked("b", vec![m.clone()]);
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    other.release();
    let mut left = ids.len();
    for _ in 0..100 {
        left = s.pending("b").expect("b").len();
        if left == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let tx = s.ack_transactions() - before;
    assert_eq!(left, 0, "S-sym-IMPL-CHAT-UNDELIVERED-2① — ACK {} 건이 기록을 모두 지운다", ids.len());
    assert!(tx >= 1 && tx < ids.len() as u64, "S-sym-IMPL-CHAT-UNDELIVERED-2② — 지운 트랜잭션 {tx} < ACK {}(묶음마다 하나)", ids.len());
});

async_forms!(ack_deletes_give_way_to_a_queued_store_call [sqlite = Sqlite, postgres = Postgres] |db| {
    let chat = Chat::new(db.open_pool(1, Visibility::Shared), Limits::default());
    let s = chat.store.clone();
    s.create_space("S", None).expect("space");
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    let full = oxhubd::chat::ACK_BATCH;
    for i in 0..full + 10 {
        fresh_t(s.send("a", "S", &id(i), "m", Some(100 + i as u64)));
    }
    let ids: Vec<String> = (0..full + 10).map(id).collect();
    let (first, second) = ids.split_at(full);
    s.ack("b", &s.pending("b").expect("b").into_iter().filter(|m| !ids.contains(m)).collect::<Vec<_>>()).expect("member notices");
    let other = Outside::of(db);
    other.hold();
    chat.acked("b", first.to_vec());
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let queued = {
        let s = s.clone();
        std::thread::spawn(move || s.pending("b"))
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    for m in second {
        chat.acked("b", vec![m.clone()]);
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    other.release();
    let seen = queued.join().expect("queued").expect("pending");
    assert_eq!(seen, second.to_vec(), "S-sym-IMPL-CHAT-UNDELIVERED-3① — 첫 묶음 뒤 줄 선 일이 다음 묶음보다 먼저 잠금을 얻는다(본 기록 = 둘째 묶음 지우기 전)");
    let mut left = ids.len();
    for _ in 0..100 {
        left = s.pending("b").expect("b").len();
        if left == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(left, 0, "S-sym-IMPL-CHAT-UNDELIVERED-3② — 그 뒤 기록이 모두 지워진다");
});

forms!(add_column_swallows_only_the_existing_column [sqlite = Sqlite] |db| {
    let path = db.file.clone().expect("file");
    drop(db.open(Visibility::Shared));
    let old = rusqlite::Connection::open(&path).expect("old");
    old.pragma_update(None, "journal_mode", "DELETE").expect("rollback journal");
    old.execute_batch("ALTER TABLE messages DROP COLUMN ext").expect("an older store without ext");
    drop(old);
    let ro = |mode: u32| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    };
    ro(0o444);
    let (r, lines) = diag::capture(|| Store::open(&db.url, Visibility::Shared).map(|_| ()));
    ro(0o644);
    assert!(r.is_ok(), "S-sym-IMPL-CHAT-STORE-FAULTS-2 — 기동은 계속한다 {r:?}");
    let alter: Vec<&String> = lines.iter().filter(|l| l.contains("column=")).collect();
    assert_eq!(alter.len(), 1, "S-sym-IMPL-CHAT-STORE-FAULTS-2 — 「이미 있음」 다섯은 넘기고 ext 하나만 진단 줄 {lines:?}");
    assert!(alter[0].contains("column=ext") && alter[0].contains("cause=ReadOnly"), "S-sym-IMPL-CHAT-STORE-FAULTS-2 — 칸 · 원인 {alter:?}");
    assert_eq!(lines.iter().filter(|l| l.contains("WAL")).count(), 1, "정:IMPL-CHAT-STORE-FAULTS — WAL 이 안 들면 기동 때 진단 줄 한 줄 {lines:?}");
});

shared_forms!(a_failed_purge_leaves_a_diag_line_and_the_next_cycle_purges |db| {
    let chat = Chat::new(db.open(Visibility::Shared), Limits::default());
    let other = Outside::of(db);
    other.exec("ALTER TABLE undelivered RENAME TO undelivered_away", "ALTER TABLE undelivered RENAME TO undelivered_away");
    let (r, lines) = diag::capture(|| chat.purge_once(30 * 86_400_000));
    assert_eq!(r, Err(Code::InternalError), "S-sym-IMPL-CHAT-STORE-FAULTS-3 — 보관 지우기 실패");
    assert!(lines.iter().any(|l| l.contains("purge")), "S-sym-IMPL-CHAT-STORE-FAULTS-3 — 진단 줄 1 {lines:?}");
    assert!(lines.iter().any(|l| l.contains("cause=")), "S-sym-IMPL-CHAT-STORE-FAULTS-3 — 원인 {lines:?}");
    let back = "ALTER TABLE undelivered_away RENAME TO undelivered; INSERT INTO undelivered (user_id, msg_id, space_id, t) VALUES ('b', 'old', 'S', 1)";
    other.exec(back, back);
    assert_eq!(chat.purge_once(30 * 86_400_000), Ok(1), "S-sym-IMPL-CHAT-STORE-FAULTS-3 — 다음 주기에 다시 지운다");
});

#[test]
fn chat_store_tests_run_on_every_form_they_can_reach() {
    let src = include_str!("l1_chat.rs");
    let mut single = Vec::new();
    let mut spread = 0;
    for l in src.lines() {
        let Some(r) = ["forms!(", "all_forms!(", "shared_forms!(", "lite_forms!(", "async_forms!("].iter().find_map(|p| l.strip_prefix(p)) else { continue };
        spread += 1;
        let name = r.split_whitespace().next().unwrap_or_default();
        let forms = match r.split_once('[').and_then(|(_, x)| x.split_once(']')) {
            Some((list, _)) => list.matches('=').count(),
            None if l.starts_with("all_forms!(") => 3,
            None => 2,
        };
        if forms < 2 {
            single.push(name);
        }
    }
    let plain: Vec<&str> = src
        .split("\n#[test]\nfn ")
        .skip(1)
        .filter(|b| !b.starts_with("chat_store_tests_run_on_every_form_they_can_reach("))
        .filter_map(|b| {
            let body = b.split("\n}\n").next().unwrap_or_default();
            (body.contains("Store::open") || body.contains("store(db") || body.contains("db.open")).then(|| body.split('(').next().unwrap_or_default())
        })
        .collect();
    assert!(spread >= 30, "형상 — 꼴로 펼친 시험 {spread}");
    assert!(plain.is_empty(), "S-sym-IMPL-CHAT-STORE-FORMS-1 — 꼴을 고르지 않고 저장소를 여는 시험 0: {plain:?}");
    assert_eq!(single, vec!["add_column_swallows_only_the_existing_column"], "S-sym-IMPL-CHAT-STORE-FORMS-1 — 꼴 하나에서만 도는 채팅 시험 0 — 하나뿐인 것은 옛 SQLite 파일의 칸 더하기(그 꼴만의 이행 · 계약 시험 아님)");
}

#[test]
fn a_missing_postgres_is_red_not_skipped() {
    let r = std::panic::catch_unwind(|| form::Db::pg_at("postgres://oxlens@127.0.0.1:1/none"));
    let why = match r {
        Ok(_) => panic!("S-sym-IMPL-CHAT-STORE-FORMS-2 — 닿지 않는 PostgreSQL 에서 시험 준비가 지나갔다"),
        Err(e) => e.downcast_ref::<String>().cloned().unwrap_or_default(),
    };
    assert!(why.contains("못 잼"), "S-sym-IMPL-CHAT-STORE-FORMS-2 — PostgreSQL 이 없으면 「못 잼」 빨강(조용히 건너뛴 초록 0): {why}");
}

#[test]
fn the_banner_hides_the_password_in_chat_db() {
    let min = "[hub]\nlisten = \"0.0.0.0:1974\"\n[bus]\nnamespace = \"ox\"\n";
    let s = System::parse(&format!("{min}[chat]\ndb = \"postgres://oxlens:s3cret@db.example:5433/oxlens_chat\"\n")).expect("parse");
    let shown = s.banner_fields().into_iter().find(|(k, _)| k == "chat.db").map(|(_, v)| v).expect("chat.db");
    assert_eq!(shown, "postgres://oxlens:***@db.example:5433/oxlens_chat", "S-sym-IMPL-CHAT-STORE-FORMS-4 — 비밀번호만 *** · 꼴 · 사용자 · 호스트 · DB 이름은 그대로");
    assert!(s.banner_fields().iter().all(|(_, v)| !v.contains("s3cret")), "S-sym-IMPL-CHAT-STORE-FORMS-4 — 배너 어디에도 비밀번호 0");
    let plain = System::parse(&format!("{min}[chat]\ndb = \"postgres://oxlens@localhost/oxlens_chat\"\n")).expect("parse");
    assert!(plain.banner_fields().iter().any(|(k, v)| k == "chat.db" && v == "postgres://oxlens@localhost/oxlens_chat"), "비밀번호 없는 값은 그대로");
    for bad in ["/tmp/chat.db", ":memory:", "memory", "sqlite:", "mysql://x"] {
        assert!(System::parse(&format!("{min}[chat]\ndb = \"{bad}\"\n")).is_err(), "정:IMPL-CHAT-STORE-FORMS — 그 밖의 꼴은 기동 실패: {bad}");
    }
    assert_eq!(ChatDb::parse("memory:"), Ok(ChatDb::Memory));
    assert_eq!(ChatDb::parse("sqlite:/x/y.db"), Ok(ChatDb::Sqlite("/x/y.db".into())));
}

mod lane {
    use std::sync::Arc;

    use oxhubd::ses::conn::{Conn, Held, Out, QUEUE_MAX, Receipt, Writer};
    use oxsig::msg::{ChatMessage, MessagesBody};
    use oxsig::{Code, Op};
    use tokio::sync::mpsc;

    pub const WINDOW: usize = 8;

    pub struct Pipe {
        pub conn: Conn,
        rx: mpsc::Receiver<Out>,
        pub w: Writer,
    }

    pub struct Frame {
        pub pid: u32,
        pub op: Op,
        pub body: Vec<u8>,
    }

    impl Frame {
        pub fn messages(&self) -> Vec<ChatMessage> {
            match self.op {
                Op::Message => vec![serde_json::from_slice(&self.body).expect("MESSAGE body")],
                Op::Messages => serde_json::from_slice::<MessagesBody>(&self.body).expect("MESSAGES body").messages,
                _ => Vec::new(),
            }
        }

        pub fn live(&self) -> Option<bool> {
            match self.op {
                Op::Messages => serde_json::from_slice::<MessagesBody>(&self.body).expect("MESSAGES body").live,
                _ => None,
            }
        }
    }

    impl Pipe {
        pub fn new(bundle_max: usize, queue_bytes: usize) -> Pipe {
            let (tx, rx) = mpsc::channel(QUEUE_MAX);
            let held = Held::new(queue_bytes);
            let conn = Conn { id: 1, tx, over: Arc::new(tokio::sync::Notify::new()), held: held.clone() };
            Pipe { conn, rx, w: Writer::new(WINDOW, bundle_max, held) }
        }

        pub fn put(&mut self, op: Op, body: Vec<u8>, receipt: Option<Receipt>) -> Result<(), Code> {
            if !self.conn.send(Out::Notice { op, body, receipt }) {
                return Err(Code::FlowOverflow);
            }
            match self.rx.try_recv() {
                Ok(Out::Notice { op, body, receipt }) => self.w.push(op, body, receipt),
                _ => panic!("the notice must be in the channel"),
            }
        }

        pub fn live(&mut self, m: &ChatMessage) -> Result<(), Code> {
            let r = Receipt { user: "b".into(), msg_ids: vec![m.msg_id.clone()], done: None, slot: None };
            self.put(Op::Message, serde_json::to_vec(m).expect("json"), Some(r))
        }

        pub fn out(&mut self) -> Vec<Frame> {
            let mut v = Vec::new();
            while let Some((pid, op, body)) = self.w.pop() {
                v.push(Frame { pid, op, body });
            }
            v
        }

        pub fn ack_all(&mut self, mut sent: Vec<Frame>) -> Vec<Frame> {
            let mut got = Vec::new();
            while !sent.is_empty() {
                let f = sent.remove(0);
                let _ = self.w.acked(f.pid);
                got.push(f);
                sent.extend(self.out());
            }
            got
        }
    }

    pub fn msg(i: usize, t: u64, content: &str) -> ChatMessage {
        ChatMessage { space_id: "S".into(), msg_id: super::id(i), user_id: "a".into(), t, content: content.into(), kind: None, r#ref: None, keys: None, ext: None, target: None, talk: None }
    }
}

#[test]
fn a_full_window_bundles_live_notices_instead_of_overflowing() {
    let run = |bundle_max: usize| {
        let mut p = lane::Pipe::new(bundle_max, 4_194_304);
        let mut first = Vec::new();
        let mut cut = None;
        for i in 0..2_000 {
            if let Err(c) = p.live(&lane::msg(i, 1_000 + i as u64, "hello there")) {
                cut = Some((i, c));
                break;
            }
            first.extend(p.out());
        }
        (p, first, cut)
    };
    let (mut p, first, cut) = run(65_536);
    assert_eq!(cut, None, "S-sym-IMPL-CHAT-LIVE-BUNDLE-1① — ACK 가 늦어 윈도우가 찬 동안 실시간 2,000 건이 FLOW_OVERFLOW 0");
    assert_eq!(first.len(), lane::WINDOW, "형상 — 윈도우 {} 칸이 찬 뒤로는 줄에 머문다", lane::WINDOW);
    assert!(p.w.queued() < 100, "S-sym-IMPL-CHAT-LIVE-BUNDLE-1① — 줄에 머문 것은 묶음 {} 개", p.w.queued());
    let got = p.ack_all(first);
    let bundles: Vec<&lane::Frame> = got.iter().filter(|f| f.op == Op::Messages).collect();
    assert!(!bundles.is_empty() && bundles.iter().all(|f| f.live() == Some(true)), "S-sym-IMPL-CHAT-LIVE-BUNDLE-1① — `MESSAGES{{live:true}}` 로 묶여 온다");
    let all: Vec<String> = got.iter().flat_map(|f| f.messages()).map(|m| m.msg_id).collect();
    assert_eq!(all, (0..2_000).map(id).collect::<Vec<_>>(), "S-sym-IMPL-CHAT-LIVE-BUNDLE-1① — 2,000 건이 빠짐없이 넣은 차례로 간다");
    let (_, _, cut) = run(0);
    assert_eq!(cut.map(|(_, c)| c), Some(Code::FlowOverflow), "S-sym-IMPL-CHAT-LIVE-BUNDLE-1② — 양성 대조: 합치기를 끄면 4007");
    assert!(cut.is_some_and(|(i, _)| i > QUEUE_MAX), "S-sym-IMPL-CHAT-LIVE-BUNDLE-1② — 끊긴 자리 = 밀린 프레임 {QUEUE_MAX} 초과 {cut:?}");
}

#[test]
fn an_open_window_sends_live_notices_one_by_one_at_once() {
    let mut p = lane::Pipe::new(65_536, 4_194_304);
    for i in 0..50 {
        let m = lane::msg(i, 1_000 + i as u64, "hi");
        p.live(&m).expect("put");
        let sent = p.out();
        assert_eq!(sent.len(), 1, "S-sym-IMPL-CHAT-LIVE-BUNDLE-2① — 넣자마자 나간다(타이머 기다림 0)");
        assert_eq!(sent[0].op, Op::Message, "S-sym-IMPL-CHAT-LIVE-BUNDLE-2① — 빈 칸이 있으면 `MESSAGE` 한 건");
        assert_eq!(sent[0].body, serde_json::to_vec(&m).expect("json"), "S-sym-IMPL-CHAT-LIVE-BUNDLE-2② — 몸이 합치기 전과 같다");
        assert!(p.w.acked(sent[0].pid).is_some(), "영수증이 돌아온다");
    }
    assert_eq!(p.w.queued(), 0, "S-sym-IMPL-CHAT-LIVE-BUNDLE-2① — 줄에 남은 것 0");
}

#[test]
fn bundling_stays_at_the_data_lane_end() {
    let mut p = lane::Pipe::new(65_536, 4_194_304);
    let mut first = Vec::new();
    for i in 0..lane::WINDOW {
        p.live(&lane::msg(i, 10 + i as u64, "w")).expect("put");
        first.extend(p.out());
    }
    assert_eq!(first.len(), lane::WINDOW, "형상 — 윈도우가 찼다");
    let catchup = |a: usize, b: usize| serde_json::to_vec(&MessagesBody { messages: (a..b).map(|i| lane::msg(i, 1 + i as u64, "c")).collect(), live: None }).expect("json");
    let c1 = catchup(100, 103);
    let c2 = catchup(200, 202);
    p.put(Op::Messages, c1.clone(), None).expect("catchup 1");
    p.live(&lane::msg(300, 50, "m1")).expect("m1");
    p.put(Op::RoomEvent, b"{\"room_id\":\"r\"}".to_vec(), None).expect("room");
    p.live(&lane::msg(301, 51, "m2")).expect("m2");
    p.put(Op::TrackEvent, b"{\"room_id\":\"r\"}".to_vec(), None).expect("track");
    p.put(Op::Messages, c2.clone(), None).expect("catchup 2");
    p.live(&lane::msg(302, 52, "m3")).expect("m3");
    let got = p.ack_all(first);
    let rest: Vec<&lane::Frame> = got[lane::WINDOW..].iter().collect();
    let ops: Vec<Op> = rest.iter().map(|f| f.op).collect();
    assert_eq!(ops, vec![Op::RoomEvent, Op::TrackEvent, Op::Messages, Op::Messages, Op::Messages, Op::Message], "S-sym-IMPL-CHAT-LIVE-BUNDLE-3① — 1단 통지가 채팅 묶음보다 먼저 · 데이터 단은 넣은 차례");
    assert_eq!(rest[0].body, b"{\"room_id\":\"r\"}".to_vec(), "S-sym-IMPL-CHAT-LIVE-BUNDLE-3① — 1단 통지는 묶음에 들지 않는다");
    assert_eq!((rest[2].body.clone(), rest[2].live()), (c1, None), "S-sym-IMPL-CHAT-LIVE-BUNDLE-3② — 따라잡기 묶음에 실시간이 섞이지 않는다");
    assert_eq!(rest[3].messages().iter().map(|m| m.msg_id.clone()).collect::<Vec<_>>(), vec![id(300), id(301)], "S-sym-IMPL-CHAT-LIVE-BUNDLE-3② — 줄 끝의 실시간끼리만 합친다");
    assert_eq!(rest[3].live(), Some(true), "S-sym-IMPL-CHAT-LIVE-BUNDLE-3② — 실시간 묶음");
    assert_eq!((rest[4].body.clone(), rest[4].live()), (c2, None), "S-sym-IMPL-CHAT-LIVE-BUNDLE-3② — 따라잡기 뒤의 실시간은 새 항목(줄 중간을 찾지 않는다)");
    assert_eq!(rest[5].messages()[0].msg_id, id(302), "S-sym-IMPL-CHAT-LIVE-BUNDLE-3② — 차례 그대로");
}

async_forms!(one_ack_for_a_live_bundle_clears_every_record_in_it [memory = Memory, sqlite = Sqlite, postgres = Postgres] |db| {
    let chat = Chat::new(db.open(Visibility::Shared), Limits::default());
    let s = chat.store.clone();
    s.create_space("S", None).expect("space");
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    let mut sent = Vec::new();
    for i in 0..40 {
        match s.send("a", "S", &id(i), "m", Some(100 + i as u64)).expect("send") {
            Sent::Fresh { msg, .. } => sent.push(msg),
            Sent::Again { .. } => panic!("fresh"),
        }
    }
    let (acked, refused) = sent.split_at(20);
    let mut p = lane::Pipe::new(Limits::default().live_bundle_bytes, 4_194_304);
    for _ in 0..lane::WINDOW {
        p.put(Op::RoomEvent, b"{}".to_vec(), None).expect("fill");
    }
    let first = p.out();
    for m in acked {
        p.live(m).expect("live");
    }
    assert!(p.w.acked(first[0].pid).is_none(), "형상 — 1단 통지에는 영수증 없음");
    let bundle = p.out();
    assert_eq!((bundle.len(), bundle[0].op), (1, Op::Messages), "형상 — 20 건이 묶음 하나");
    let r = p.w.acked(bundle[0].pid).expect("receipt");
    assert_eq!(r.msg_ids.len(), 20, "S-sym-IMPL-CHAT-LIVE-BUNDLE-4① — 영수증이 묶음 몫 전부를 든다");
    chat.acked(&r.user, r.msg_ids);
    for m in refused {
        p.live(m).expect("live");
    }
    assert!(p.w.acked(first[1].pid).is_none(), "형상");
    let second = p.out();
    assert!(p.w.refused(second[0].pid), "실패 답");
    let ids = |v: &[ChatMessage]| v.iter().map(|m| m.msg_id.clone()).collect::<Vec<_>>();
    let mut left = ids(acked);
    for _ in 0..100 {
        let pend = s.pending("b").expect("pending");
        left.retain(|m| pend.contains(m));
        if left.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(left.is_empty(), "S-sym-IMPL-CHAT-LIVE-BUNDLE-4① — ACK 하나로 묶음 몫 기록이 모두 0 · 남은 {left:?}");
    let page = chat.catchup_page("b", &oxsig::msg::CatchupReq::default(), &Cursor::default()).await.expect("catchup");
    let back = ids(&page);
    assert!(ids(acked).iter().all(|m| !back.contains(m)), "S-sym-IMPL-CHAT-LIVE-BUNDLE-4① — 뒤의 CATCHUP{{}} 에 그 몫 0");
    let pend = s.pending("b").expect("pending");
    assert!(ids(refused).iter().all(|m| pend.contains(m)), "S-sym-IMPL-CHAT-LIVE-BUNDLE-4② — 양성 대조: 실패로 답한 묶음 몫은 모두 남는다");
    assert!(ids(refused).iter().all(|m| back.contains(m)), "S-sym-IMPL-CHAT-LIVE-BUNDLE-4② — 남은 몫은 따라잡기로 간다");
});

#[test]
fn live_bundles_stay_under_their_byte_cap_and_carry_live() {
    let cap = 2_000;
    let mut p = lane::Pipe::new(cap, 4_194_304);
    let mut first = Vec::new();
    for i in 0..lane::WINDOW {
        p.live(&lane::msg(i, 10 + i as u64, "w")).expect("put");
        first.extend(p.out());
    }
    assert_eq!(first.len(), lane::WINDOW, "형상 — 윈도우가 찼다");
    let mut want = Vec::new();
    for i in 100..160 {
        let content = if i == 130 { "x".repeat(3_000) } else { "y".repeat(150) };
        p.live(&lane::msg(i, 1_000 + i as u64, &content)).expect("put");
        want.push(id(i));
    }
    let got = p.ack_all(first);
    let rest = &got[lane::WINDOW..];
    for f in rest {
        match f.op {
            Op::Messages => {
                assert_eq!(f.live(), Some(true), "S-sym-IMPL-CHAT-LIVE-BUNDLE-5① — 실시간 묶음에만 live:true");
                assert!(f.body.len() <= cap, "S-sym-IMPL-CHAT-LIVE-BUNDLE-5② — 묶음 몸 {} ≤ live_bundle_max_bytes {cap}", f.body.len());
            }
            Op::Message => {}
            o => panic!("unexpected {o:?}"),
        }
    }
    let alone: Vec<&lane::Frame> = rest.iter().filter(|f| f.messages().iter().any(|m| m.msg_id == id(130))).collect();
    assert_eq!((alone.len(), alone[0].op, alone[0].messages().len()), (1, Op::Message, 1), "S-sym-IMPL-CHAT-LIVE-BUNDLE-5② — 상한을 넘는 한 건은 혼자 간다");
    assert_eq!(rest.iter().flat_map(|f| f.messages()).map(|m| m.msg_id).collect::<Vec<_>>(), want, "S-sym-IMPL-CHAT-LIVE-BUNDLE-5② — 빠짐 · 순서 바뀜 없음");
    assert!(rest.iter().filter(|f| f.op == Op::Messages).count() >= 2, "S-sym-IMPL-CHAT-LIVE-BUNDLE-5② — 묶음이 상한에서 나뉜다");
    let c = serde_json::to_vec(&MessagesBody { messages: vec![lane::msg(1, 1, "c")], live: None }).expect("json");
    assert!(!String::from_utf8_lossy(&c).contains("\"live\""), "S-sym-IMPL-CHAT-LIVE-BUNDLE-5① — 따라잡기 묶음에는 live 칸 없음");
    assert_eq!(p.conn.held.bytes(), 0, "다 나가면 쥔 바이트 0 — 합칠 때 늘린 만큼 나갈 때 덜었다");
}

#[test]
fn the_send_lane_closes_on_bytes_before_the_frame_count() {
    let max = 4_194_304;
    let mut p = lane::Pipe::new(65_536, max);
    let mut cut = (0, None);
    for i in 0..2 * QUEUE_MAX {
        let m = lane::msg(i, 1 + i as u64, &"가".repeat(21_845));
        if let Err(c) = p.put(Op::Messages, serde_json::to_vec(&MessagesBody { messages: vec![m], live: None }).expect("json"), None) {
            cut = (i, Some(c));
            break;
        }
    }
    let frame = 65_535 + 200;
    assert_eq!(cut.1, Some(Code::FlowOverflow), "S-sym-IMPL-SES-SOCKET-LIFE-1① — 바이트 상한을 넘으면 FLOW_OVERFLOW");
    assert!(cut.0 < QUEUE_MAX, "S-sym-IMPL-SES-SOCKET-LIFE-1① — 건수 {} 는 {QUEUE_MAX} 아래", cut.0);
    assert!(p.conn.held.bytes() <= max + frame, "S-sym-IMPL-SES-SOCKET-LIFE-1② — 쥔 바이트 {} ≤ 상한 + 프레임 하나", p.conn.held.bytes());
    let mut q = lane::Pipe::new(65_536, max);
    let mut n = 0;
    let cut = loop {
        if let Err(c) = q.put(Op::RoomEvent, b"{}".to_vec(), None) {
            break c;
        }
        n += 1;
        assert!(n <= QUEUE_MAX + lane::WINDOW + 1, "형상 — 건수 상한에서 끊겨야 한다");
    };
    assert_eq!((cut, n), (Code::FlowOverflow, QUEUE_MAX), "S-sym-IMPL-SES-SOCKET-LIFE-1③ SV-CODE-4006-4008 — 양성 대조: 건수 {QUEUE_MAX} 초과도 같은 사유");
}

fn talk_id(i: usize) -> String {
    format!("{}{:012x}{:016x}", SYS_HASH, 0x0190_0000_0000u64 + i as u64, i)
}

fn talk_start(id: &str, room: &str, user: &str) -> Report {
    Report::Start { talk_id: id.into(), room_id: room.into(), user_id: user.into(), start: 1_000, deadline: 38_000, sfu_id: "e1".into() }
}

fn talk_end(id: &str, state: TalkState, reason: Option<u8>, d: u64) -> Report {
    Report::End { talk_id: id.into(), state, reason, duration_ms: d }
}

fn talk_rec(id: &str, path: &str) -> Report {
    Report::Rec { talk_id: id.into(), path: path.into(), size: 123, mime: "audio/ogg".into(), duration_ms: 900 }
}

fn sent_msg(r: Result<Sent, Code>) -> (bool, ChatMessage) {
    match r.expect("talk") {
        Sent::Fresh { msg, .. } => (true, msg),
        Sent::Again { msg } => (false, msg),
    }
}

all_forms!(a_talk_is_a_message_by_the_speaker_for_everyone |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    let id = talk_id(1);
    let (fresh, m) = sent_msg(s.talk(&talk_start(&id, "S", "a"), Some(2_000)));
    assert!(fresh, "stand check — the first start stores");
    assert_eq!((m.kind, m.user_id.as_str(), m.content.as_str(), m.msg_id.as_str()), (Some(ChatKind::Talk), "a", "", id.as_str()), "연:RULE-CHAT-TALK — kind talk · user = speaker · empty content · msg_id = talk id");
    let info = m.talk.clone().expect("talk info");
    assert_eq!((info.state, info.room_id.as_str(), info.start, info.deadline), (TalkState::Talking, "S", 1_000, 38_000));
    for u in ["a", "b"] {
        assert!(s.pending(u).expect("pending").contains(&id), "정:IMPL-CHAT-TALK — the speaker too gets the talk ({u})");
    }
    let e = s.list("a").expect("list").remove(0);
    assert_eq!((e.total, e.read), (1, 1), "연:RULE-CHAT-UNREAD — total counts the talk · the speaker has read it");
    assert_eq!(s.list("b").expect("list").remove(0).read, 0, "the listener has not");
    let (again, m2) = sent_msg(s.talk(&talk_start(&id, "S", "a"), Some(9_000)));
    assert!(!again && m2.t == m.t, "S-sym-IMPL-CHAT-TALK-3 연:RULE-CHAT-MSG-ID — a resent start answers the first t without comparing");
    assert_eq!(s.talk(&talk_start(&talk_id(2), "nope", "a"), None), Err(Code::SpaceNotFound), "정:IMPL-CHAT-TALK 1 — no space = no record");
    let (fresh, m3) = sent_msg(s.talk(&talk_start(&talk_id(3), "S", "outsider"), Some(3_000)));
    assert!(fresh && m3.user_id == "outsider", "연:RULE-CHAT-TALK — a speaker who is not a member still leaves a talk");
    assert_eq!(s.list("a").expect("list")[0].total, 2);
});

all_forms!(a_talk_update_carries_the_whole_state |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    let id = talk_id(10);
    sent_msg(s.talk(&talk_start(&id, "S", "a"), Some(2_000)));
    let path = format!("rec/S/S-{id}-a.ogg");
    let (fresh, r) = sent_msg(s.talk(&talk_rec(&id, &path), Some(2_500)));
    assert!(fresh, "a recording that closes before the end still updates");
    assert_eq!((r.kind, r.r#ref.as_deref(), r.user_id.as_str()), (Some(ChatKind::TalkUpdate), Some(id.as_str()), "a"));
    assert_eq!(r.msg_id, common::talk::update_msg_id(&id, ":rec"), "정:IMPL-CHAT-TALK 2 — the update msg_id is fixed by the talk id and the tail");
    let ri = r.talk.clone().expect("info");
    assert_eq!((ri.state, ri.rec.as_ref().map(|x| x.path.as_str())), (TalkState::Talking, Some(path.as_str())));
    let (_, e) = sent_msg(s.talk(&talk_end(&id, TalkState::Cut, Some(2), 4_000), Some(3_000)));
    let ei = e.talk.clone().expect("info");
    assert_eq!((ei.state, ei.reason, ei.duration_ms), (TalkState::Cut, Some(2), Some(4_000)), "연:RULE-CHAT-TALK-STATE");
    assert_eq!(ei.rec.as_ref().map(|x| x.path.as_str()), Some(path.as_str()), "S-sym-IMPL-CHAT-TALK-1 — the end keeps the earlier recording (whole state, not an append)");
    let (again, e2) = sent_msg(s.talk(&talk_end(&id, TalkState::Ok, None, 1), Some(9_000)));
    assert!(!again && e2.t == e.t && e2.talk == e.talk, "S-sym-IMPL-CHAT-TALK-3 연:RULE-CHAT-MSG-ID — a resent end is the first one, body not compared");
    assert_eq!(s.list("a").expect("list")[0].total, 1, "연:RULE-CHAT-UNREAD — updates do not count");
    assert!(s.pending("a").expect("pending").contains(&e.msg_id), "the update goes to everyone");
    let page = s.history("a", "S", Dir::Latest, 60).expect("history");
    assert!(page.messages.iter().any(|m| m.msg_id == id), "연:RULE-CHAT-HISTORY-PAGE — the talk is a visible message");
    assert!(page.messages.iter().all(|m| m.kind != Some(ChatKind::TalkUpdate)), "updates are not page messages");
    let ups: Vec<_> = page.relations.iter().filter(|m| m.kind == Some(ChatKind::TalkUpdate)).collect();
    assert_eq!(ups.len(), 1, "연:RULE-CHAT-HISTORY-PAGE — one update per talk in relations");
    assert_eq!(ups[0].msg_id, e.msg_id, "the last by (t, msg_id)");
    assert_eq!(s.talk(&talk_end(&talk_id(11), TalkState::Ok, None, 1), None), Err(Code::InternalError), "정:IMPL-CHAT-TALK 2 — no talk yet is a passing failure (5003 · not permanent)");
    assert_eq!(common::talk::judge(Some(&common::envelope::Reply::fail(Code::InternalError).encode())), None, "정:IMPL-FLR-TALK-REPORT 4 — the caller sends it again");
    let other = talk_id(12);
    sent_msg(s.talk(&talk_start(&other, "S", "a"), Some(2_000)));
    assert_eq!(s.talk(&talk_rec(&other, "rec/T/x.ogg"), None), Err(Code::InvalidPayload), "정:IMPL-CHAT-TALK 2 — a path outside rec/{{space}}/ is refused");
    assert_eq!(s.talk(&talk_rec(&other, "rec/S/../x.ogg"), None), Err(Code::InvalidPayload));
    let r2 = Report::Regrant { talk_id: id.clone(), deadline: 99_000 };
    let (_, g) = sent_msg(s.talk(&r2, Some(3_500)));
    assert_eq!(g.talk.expect("info").deadline, 99_000, "a regrant moves the deadline");
});

all_forms!(a_talk_is_an_origin_only_for_reply_and_react |db| {
    let s = store(db, Visibility::Shared);
    join(&s, "S", "a", 1);
    let tid = talk_id(20);
    sent_msg(s.talk(&talk_start(&tid, "S", "a"), Some(2_000)));
    let reply = Outgoing { content: "copy".into(), r#ref: Some(tid.clone()), ..Outgoing::default() };
    assert!(s.post("a", "S", ID_A, &reply, Some(2_100)).is_ok(), "연:RULE-CHAT-REF-CHECK CP-CODE-REF-NOT-FOUND③ — a reply may point at a talk");
    assert!(s.post("a", "S", ID_B, &react(&tid, &["👍"]), Some(2_200)).is_ok(), "CP-CODE-REF-NOT-FOUND③ a react may point at a talk");
    assert_eq!(s.post("a", "S", ID_C, &edit(&tid, "x"), Some(2_300)), Err(Code::RefNotFound), "연:RULE-CHAT-REF-CHECK — a talk is not edited");
    assert_eq!(s.post("a", "S", &id(30), &rel(ChatKind::Delete, &tid), Some(2_400)), Err(Code::RefNotFound), "a talk is not deleted");
    let up = common::talk::update_msg_id(&tid, ":end");
    sent_msg(s.talk(&talk_end(&tid, TalkState::Ok, None, 900), Some(2_500)));
    assert_eq!(s.post("a", "S", &id(31), &react(&up, &["👍"]), Some(2_600)), Err(Code::RefNotFound), "a talk update is not an origin");
});

all_forms!(a_recording_is_seen_like_the_history |db| {
    let s = db.open(Visibility::Joined);
    s.create_space("S", None).expect("space");
    join(&s, "S", "a", 1);
    let id = talk_id(40);
    sent_msg(s.talk(&talk_start(&id, "S", "a"), Some(2_000)));
    join(&s, "S", "late", 3_000);
    assert_eq!(s.rec_seen("a", "S", &id), Ok(()), "연:RULE-CHAT-REC-FILE — a member who could see the talk");
    assert_eq!(s.rec_seen("late", "S", &id), Err(Code::NotSpaceMember), "joined visibility — a talk before the join is not seen");
    assert_eq!(s.rec_seen("x", "S", &id), Err(Code::NotSpaceMember), "not a member");
    assert_eq!(s.rec_seen("a", "S", &talk_id(41)), Err(Code::RecNotFound), "no such talk");
});

all_forms!(end_and_rec_racing_on_one_talk_keep_both |db| {
    let a = Arc::new(store(db, Visibility::Shared));
    join(&a, "S", "u", 1);
    let (s1, s2) = (side(db, &a), side(db, &a));
    for round in 0..40usize {
        let id = talk_id(100 + round);
        sent_msg(a.talk(&talk_start(&id, "S", "u"), None));
        let gate = Arc::new(std::sync::Barrier::new(2));
        let go = |st: Arc<Store>, g: Arc<std::sync::Barrier>, r: Report| {
            std::thread::spawn(move || {
                g.wait();
                st.talk(&r, None)
            })
        };
        let path = format!("rec/S/S-{id}-u.ogg");
        let h1 = go(s1.clone(), gate.clone(), talk_end(&id, TalkState::Ok, None, 900));
        let h2 = go(s2.clone(), gate.clone(), talk_rec(&id, &path));
        let (r1, r2) = (h1.join().expect("end"), h2.join().expect("rec"));
        let (_, m1) = sent_msg(r1);
        let (_, m2) = sent_msg(r2);
        let full = |m: &ChatMessage| m.talk.as_ref().is_some_and(|i| i.state == TalkState::Ok && i.rec.is_some());
        assert!(full(&m1) || full(&m2), "S-sym-IMPL-CHAT-TALK-2 — the later of end · rec holds both (round {round}: {:?} / {:?})", m1.talk, m2.talk);
    }
});

#[test]
fn a_client_cannot_send_talk_kinds() {
    for k in ["talk", "talk_update"] {
        let f = outgoing(&serde_json::json!({ "kind": k, "ref": ID_A, "content": "" })).expect_err("refused");
        assert_eq!(f.code, Code::InvalidPayload.num(), "연:RULE-CHAT-TALK — a client {k} is 1002");
    }
    let f = outgoing(&serde_json::json!({ "content": "x", "talk": { "state": "ok" } })).expect_err("refused");
    assert_eq!(f.code, Code::InvalidPayload.num(), "연:RULE-CHAT-KIND — a talk field on a client message is 1002");
}

#[test]
fn rec_file_names_and_ranges() {
    use oxhubd::ops::http::{byte_range, rec_talk_id};
    let id = talk_id(7);
    assert_eq!(rec_talk_id("ch-1", &format!("ch-1-{id}-bot-a.ogg")), Some(id.as_str()), "정:IMPL-CHAT-TALK 3 — the file name gives the talk id even when the space has '-'");
    assert_eq!(rec_talk_id("ch-1", "other-x-y.ogg"), None);
    assert!(oxhubd::chat::store::rec_path_ok("S", &format!("rec/S/S-{id}-a.ogg")));
    assert!(!oxhubd::chat::store::rec_path_ok("S", "rec/S/a/b.ogg"));
    assert!(!oxhubd::chat::store::rec_file_ok(".."));
    assert_eq!(byte_range("bytes=0-9", 100), Some((0, 9)), "연:RULE-CHAT-REC-FILE Range");
    assert_eq!(byte_range("bytes=90-", 100), Some((90, 99)));
    assert_eq!(byte_range("bytes=-10", 100), Some((90, 99)));
    assert_eq!(byte_range("bytes=50-500", 100), Some((50, 99)));
    assert_eq!(byte_range("bytes=100-", 100), None, "416 past the end");
    assert_eq!(byte_range("bytes=0-1,5-6", 100), None, "one range only");
}

fn subs(accounts: &[(&str, &[&str])]) -> ExtSubs {
    ExtSubs::new(
        accounts
            .iter()
            .map(|(k, items)| ((*k).to_string(), Subscribe::try_from(items.iter().map(|s| (*s).to_string()).collect::<Vec<_>>()).expect("subscribe")))
            .collect(),
    )
}

fn ext_store(db: &form::Db) -> Store {
    let s = db.open(Visibility::Shared).subscribed(subs(&[("texts", &[""]), ("talks", &["talk"]), ("all", &["*"]), ("none", &["com.acme.*"])]));
    s.create_space("S", None).expect("space");
    s
}

fn ext_ids(s: &Store, key: &str) -> Vec<String> {
    s.ext_rows(key).expect("ext rows").into_iter().map(|r| r.msg_id).collect()
}

all_forms!(ext_pending_is_written_in_the_store_transaction_per_matching_account |db| {
    let s = ext_store(db);
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    let member_rows = ext_ids(&s, "all");
    assert_eq!(member_rows.len(), 2, "정:RULE-CHAT-EXT-SUB 2 — 멤버 변경 메시지(정:IMPL-CHAT-MEMBERSHIP)도 같은 길목 · \"*\" 계정 몫 {member_rows:?}");
    assert!(ext_ids(&s, "texts").is_empty(), "정:RULE-CHAT-EXT-SUB 1 — 글 구독에 멤버 변경은 안 걸림");
    let post = id(1);
    fresh_t(s.send("a", "S", &post, "hi", Some(10)));
    let ed = id(2);
    fresh_t(s.post("a", "S", &ed, &edit(&post, "hi!"), Some(11)));
    let re = id(3);
    fresh_t(s.post("b", "S", &re, &react(&post, &["+1"]), Some(12)));
    let de = id(4);
    fresh_t(s.post("a", "S", &de, &rel(ChatKind::Delete, &post), Some(13)));
    let texts = ext_ids(&s, "texts");
    assert!(texts.contains(&ed) && texts.contains(&de), "S-sym-RULE-CHAT-EXT-SUB-8① — 글 구독: 고침 · 지움도 원본 따라 걸린다 {texts:?}");
    assert!(!texts.contains(&re), "S-sym-RULE-CHAT-EXT-SUB-8② — 반응은 안 걸린다 {texts:?}");
    assert_eq!(texts, vec![post.clone(), ed.clone(), de.clone()], "정:RULE-CHAT-EXT-SUB 2 — 글 · 고침 · 지움 차례");
    assert!(ext_ids(&s, "none").is_empty(), "정:RULE-CHAT-EXT-SUB 1 — 안 걸린 계정은 0 행");
    let talk = talk_id(7);
    sent_msg(s.talk(&talk_start(&talk, "S", "a"), Some(20)));
    let (_, upd) = sent_msg(s.talk(&talk_end(&talk, TalkState::Ok, None, 900), Some(21)));
    assert_eq!(ext_ids(&s, "talks"), vec![talk.clone(), upd.msg_id.clone()], "S-sym-RULE-CHAT-EXT-SUB-9① — talk 구독: 발언 기록과 그 갱신(원본 따라 걸기)");
    assert!(!ext_ids(&s, "texts").contains(&upd.msg_id), "S-sym-RULE-CHAT-EXT-SUB-9② — 글 구독에 발언 기록 갱신은 안 걸림");
    assert_eq!(ext_ids(&s, "all").len(), 2 + 4 + 2, "정:RULE-CHAT-EXT-SUB 1 — \"*\" 는 모든 종류");
    assert!(s.ext_rows("texts").expect("rows").iter().all(|r| r.tries == 0), "형상 — 새 행의 시도 수 0");
});

all_forms!(a_star_account_gets_every_space_outside_membership_and_the_visible_start |db| {
    let s = db.open(Visibility::Joined).subscribed(subs(&[("all", &["*"]), ("texts", &[""])]));
    s.create_space("S", None).expect("space");
    s.create_space("S2", None).expect("space two");
    join(&s, "S", "a", 1);
    join(&s, "S2", "c", 1);
    let early = id(1);
    fresh_t(s.send("a", "S", &early, "before b joined", Some(10)));
    join(&s, "S", "b", 20);
    let other = id(2);
    fresh_t(s.send("c", "S2", &other, "another space", Some(30)));
    let seen_by_b: Vec<String> = s.history("b", "S", Dir::Latest, 60).expect("history").messages.into_iter().map(|m| m.msg_id).collect();
    assert!(!seen_by_b.contains(&early), "positive control — in the joined form the member b does not see what came before b joined {seen_by_b:?}");
    assert_eq!(s.history("b", "S2", Dir::Latest, 60), Err(Code::NotSpaceMember), "positive control — b is no member of the other space");
    let all = ext_ids(&s, "all");
    assert!(all.contains(&early), "H-rule-7-3-13 — the \"*\" account gets the message from before the visible start of a member {all:?}");
    assert!(all.contains(&other), "H-rule-7-3-13 — the \"*\" account gets the message of a space it has no membership in either {all:?}");
    assert_eq!(all.iter().filter(|i| **i == early || **i == other).count(), 2, "H-rule-7-3-13 — one row each");
});

all_forms!(a_resent_msg_id_writes_no_second_ext_row |db| {
    let s = ext_store(db);
    join(&s, "S", "a", 1);
    let m = id(1);
    let t1 = fresh_t(s.send("a", "S", &m, "x", Some(10)));
    s.ext_failed("texts", std::slice::from_ref(&m), 10).expect("one try");
    match s.send("a", "S", &m, "x", Some(20)).expect("again") {
        Sent::Again { msg } => assert_eq!(msg.t, t1),
        Sent::Fresh { .. } => panic!("형상 — 같은 msg_id 는 다시 저장하지 않는다"),
    }
    let rows = s.ext_rows("texts").expect("rows");
    assert_eq!((rows.len(), rows[0].tries), (1, 1), "정:RULE-CHAT-EXT-SUB 2 — 다시 받은 msg_id 는 펜딩을 다시 쓰지 않는다(행 하나 · 시도 수 그대로)");
});

all_forms!(ext_tries_reach_the_limit_and_the_row_is_given_up |db| {
    let s = ext_store(db);
    join(&s, "S", "a", 1);
    let (bad, good) = (id(1), id(2));
    fresh_t(s.send("a", "S", &bad, "x", Some(10)));
    fresh_t(s.send("a", "S", &good, "y", Some(11)));
    for n in 1..3 {
        assert!(s.ext_failed("texts", std::slice::from_ref(&bad), 3).expect("try").is_empty(), "형상 — {n} 번째는 상한 아래");
    }
    assert_eq!(s.ext_failed("texts", &[bad.clone(), "missing".into()], 3).expect("try"), vec![bad.clone()], "S-sym-RULE-CHAT-EXT-SUB-7① — 상한에 닿은 행을 지우고 돌려준다(없는 msg_id 는 무시 · 세는 것은 부른 쪽)");
    let rows = s.ext_rows("texts").expect("rows");
    assert_eq!((rows.len(), rows[0].msg_id.as_str(), rows[0].tries), (1, good.as_str(), 0), "정:RULE-CHAT-EXT-SUB 2 — 다른 행은 그대로");
    assert!(ext_ids(&s, "all").contains(&bad), "형상 — 다른 계정 몫은 따로 센다");
});

all_forms!(ext_acks_clear_only_ext_rows_in_one_transaction |db| {
    let s = ext_store(db);
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    for i in 0..10 {
        fresh_t(s.send("a", "S", &id(i), "m", Some(100 + i as u64)));
    }
    let ids = ext_ids(&s, "texts");
    assert_eq!(ids.len(), 10, "형상");
    let member = s.pending("b").expect("b").len();
    let mut rest: std::collections::VecDeque<(String, Vec<String>)> = ids[1..].iter().map(|m| ("texts".to_string(), vec![m.clone()])).collect();
    let before = s.ack_transactions();
    let rows = s.ack_drain(Lane::Ext, ("texts".to_string(), vec![ids[0].clone()]), || rest.pop_front(), oxhubd::chat::ACK_BATCH).expect("drain");
    assert_eq!(rows, 10, "형상");
    assert!(ext_ids(&s, "texts").is_empty(), "정:RULE-CHAT-EXT-SUB 2 — 3rd ACK 묶음이 그 계정 펜딩을 지운다");
    assert_eq!(s.ack_transactions() - before, 1, "정:RULE-CHAT-EXT-SUB 2 — 묶음 하나 = 트랜잭션 하나");
    assert_eq!(s.pending("b").expect("b").len(), member, "정:RULE-CHAT-EXT-SUB 2 — 멤버 미전송 기록은 건드리지 않는다");
    assert_eq!(ext_ids(&s, "all").len(), 2 + 10, "형상 — 다른 계정 몫은 그대로");
});

all_forms!(the_retention_purge_clears_old_ext_rows_too |db| {
    let s = ext_store(db);
    join(&s, "S", "a", 1);
    fresh_t(s.send("a", "S", &id(1), "old", Some(1_000)));
    fresh_t(s.send("a", "S", &id(2), "new", Some(9_000)));
    s.purge(5_000).expect("purge");
    assert_eq!(ext_ids(&s, "texts"), vec![id(2)], "정:RULE-CHAT-EXT-SUB 2 · 정:IMPL-CHAT-UNDELIVERED 보관 — 같은 보관 지우기가 넘긴 3rd 몫 행도 지운다");
});

shared_forms!(an_ext_write_failure_rolls_the_whole_store_back |db| {
    let s = ext_store(db);
    join(&s, "S", "a", 1);
    join(&s, "S", "b", 1);
    let other = Outside::of(db);
    other.exec("ALTER TABLE ext_pending RENAME TO ext_pending_away", "ALTER TABLE ext_pending RENAME TO ext_pending_away");
    let (r, _) = diag::capture(|| s.send("a", "S", &id(1), "x", Some(10)));
    other.exec("ALTER TABLE ext_pending_away RENAME TO ext_pending", "ALTER TABLE ext_pending_away RENAME TO ext_pending");
    assert_eq!(r.err(), Some(Code::InternalError), "S-sym-RULE-CHAT-EXT-SUB-1① — 펜딩을 못 쓰면 저장이 실패한다");
    assert!(!s.pending("b").expect("b").contains(&id(1)), "S-sym-RULE-CHAT-EXT-SUB-1② — 메시지 · 미전송 기록 · 펜딩은 한 트랜잭션(미전송 기록이 없다)");
    assert!(posts(&s.history("a", "S", Dir::Latest, 10).expect("history")).is_empty(), "S-sym-RULE-CHAT-EXT-SUB-1② — 메시지 행도 없다");
});

#[test]
fn a_full_ext_ack_lane_does_not_drop_member_acks() {
    let lanes = AckLanes::new(2, 1);
    assert_eq!(lanes.push(Lane::Ext, ("k".into(), vec!["e1".into()])), Push::Queued);
    assert_eq!(lanes.push(Lane::Ext, ("k".into(), vec!["e2".into()])), Push::Full, "형상 — 3rd 줄 상한");
    assert_eq!(lanes.push(Lane::Member, ("u".into(), vec!["m1".into()])), Push::Queued, "S-sym-RULE-CHAT-EXT-SUB-3① — 3rd 줄이 차도 멤버 ACK 는 들어간다");
    assert_eq!(lanes.push(Lane::Member, ("u".into(), vec!["m2".into()])), Push::Queued);
    assert_eq!(lanes.push(Lane::Member, ("u".into(), vec!["m3".into()])), Push::Full, "형상 — 멤버 줄은 제 상한");
    let order: Vec<Lane> = std::iter::from_fn(|| (lanes.len(Lane::Member) + lanes.len(Lane::Ext) > 0).then(|| lanes.next().map(|(l, _)| l)).flatten()).collect();
    assert_eq!(order, vec![Lane::Member, Lane::Ext, Lane::Member], "S-sym-RULE-CHAT-EXT-SUB-3② — 지우는 주인 하나가 둘 다 있으면 번갈아 거둔다(한쪽이 다른 쪽을 굶기지 않는다)");
    lanes.close();
    assert_eq!(lanes.push(Lane::Member, ("u".into(), vec!["m4".into()])), Push::Closed);
    assert!(lanes.next().is_none(), "형상 — 닫히고 비면 끝");
}
