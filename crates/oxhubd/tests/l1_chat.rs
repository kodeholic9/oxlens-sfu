// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:RULE-CHAT-KIND · 연:RULE-CHAT-REF-CHECK · 연:RULE-CHAT-DELETE · 연:RULE-CHAT-FOLD · 정:IMPL-CHAT-STORE ·정:IMPL-CHAT-MEMBERSHIP · 정:IMPL-CHAT-FILTER · 정:IMPL-CHAT-READ · 정:IMPL-CHAT-UNDELIVERED · 정:IMPL-CHAT-CATCHUP · 연:RULE-CHAT-MSG-ID · 연:RULE-CHAT-CONTENT · 연:RULE-CHAT-ORDER · 연:RULE-CHAT-VISIBLE · 연:RULE-CHAT-UNREAD · 연:RULE-CHAT-HISTORY-PAGE · 연:RULE-CHAT-UNDELIVERED · 연:RULE-CHAT-CATCHUP · 연:RULE-CHAT-MEMBER-NOTICE · model: claude-opus-5-5
//! 1층 — 채팅 저장소(저장할 때 찍는 t · 같은 t 를 가르지 않는 쪽 · msg_id 멱등 · 멤버 구간 · 멤버 변경 메시지 · 보이기 시작점 · 안 읽은 수 · 미전송 기록 · 따라잡기 쪽)와 받는 게이트웨이의 멤버 캐시(판이 작을 때만 다시 읽음 · 명단(t_M)) · op 경계(msg_id 64 · content 65535).

use common::system::Visibility;
use oxhubd::chat::store::{Cursor, Dir, Outgoing, Page, SYS, SYS_HASH, Sent, Store};
use oxhubd::chat::{BODY_MAX, Chat, Limits, outgoing, valid_msg_id};
use oxsig::msg::{ChatKind, ChatMessage};
use oxsig::{Code, Op};

const ID_A: &str = "e818000001a1e480b5fd7da4c5b6bc0df205";
const ID_B: &str = "e818000001a1e480b5fe0b1c2d3e4f506a7b";
const ID_C: &str = "3f0a9b1201a1e480b600aa11bb22cc33dd44";

fn store(vis: Visibility) -> Store {
    let s = Store::open(":memory:", vis).expect("open");
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

#[test]
fn t_is_the_store_time_as_is() {
    let s = store(Visibility::Shared);
    join(&s, "S", "a", 1000);
    let t1 = fresh_t(s.send("a", "S", ID_A, "x", Some(5000)));
    let t2 = fresh_t(s.send("a", "S", ID_B, "y", Some(5000)));
    assert_eq!((t1, t2), (5000, 5000), "S-sym-IMPL-CHAT-STORE-1 — two stored in the same ms both keep that t");
    let t3 = fresh_t(s.send("a", "S", ID_C, "z", Some(10)));
    assert_eq!(t3, 10, "S-sym-IMPL-CHAT-STORE-1 — an earlier store time is not pushed up");
    let e = s.list("a").expect("list").remove(0);
    assert_eq!((e.total, e.last_t), (3, Some(5000)), "S-sym-IMPL-CHAT-STORE-1 — total counts posts · last t is the largest");
}

#[test]
fn t_is_stamped_inside_the_store() {
    let s = store(Visibility::Shared);
    join(&s, "S", "a", 1);
    let before = common::now_ms();
    let t = fresh_t(s.send("a", "S", ID_A, "x", None));
    let after = common::now_ms();
    assert!(before <= t && t <= after, "S-sym-IMPL-CHAT-STORE-1 — t is the ms taken in the store transaction ({before} ≤ {t} ≤ {after})");
}

#[test]
fn same_msg_id_same_content_answers_the_first_t() {
    let s = store(Visibility::Shared);
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
}

#[test]
fn msg_id_is_opaque_and_only_bounded() {
    assert!(valid_msg_id("x"), "any shape is taken");
    assert!(valid_msg_id(&"a".repeat(64)), "64 bytes is taken");
    assert!(!valid_msg_id(""), "empty is refused");
    assert!(!valid_msg_id(&"a".repeat(65)), "65 bytes is refused");
}

#[tokio::test]
async fn message_op_refuses_only_that_request_on_bounds() {
    let s = store(Visibility::Shared);
    join(&s, "S", "a", 1);
    let chat = Chat::new(s, Limits::default());
    let ask = |msg_id: String, content: String| serde_json::json!({ "space_id": "S", "msg_id": msg_id, "content": content });
    let f = chat.handle("a", "s1", Op::Message, ask("a".repeat(65), "x".into()), 100).await.err().expect("refused");
    assert_eq!(f.code, Code::InvalidPayload.num(), "CP-OP-MESSAGE ④ — msg_id over 64 bytes is 1002");
    let f = chat.handle("a", "s1", Op::Message, ask(ID_A.into(), "가".repeat(21_846)), 70_000).await.err().expect("refused");
    assert_eq!(f.code, Code::InvalidPayload.num(), "CP-OP-MESSAGE ⑧ — content over 65535 bytes is 1002");
    assert!(chat.handle("a", "s1", Op::Message, ask("not-hex-at-all".into(), "가".repeat(21_845)), 70_000).await.is_ok(), "CP-OP-MESSAGE ④ — the shape is not checked · 65535 bytes is taken");
    let f = chat.handle("a", "s1", Op::Message, ask(ID_B.into(), "x".into()), BODY_MAX + 1).await.err().expect("refused");
    assert_eq!(f.code, Code::InvalidPayload.num(), "연:RULE-CHAT-CONTENT — a body over 131072 received bytes is 1002");
}

#[test]
fn space_and_membership_checks() {
    let s = store(Visibility::Shared);
    assert_eq!(s.send("a", "nope", ID_A, "x", Some(1)), Err(Code::SpaceNotFound));
    assert_eq!(s.send("a", "S", ID_A, "x", Some(1)), Err(Code::NotSpaceMember));
    join(&s, "S", "a", 1);
    s.leave("S", "a", None, Some(2)).expect("leave");
    assert_eq!(s.send("a", "S", ID_A, "x", Some(3)), Err(Code::NotSpaceMember), "a member who left is not a member");
    assert_eq!(s.history("a", "S", Dir::Latest, 60), Err(Code::NotSpaceMember));
    assert_eq!(s.catchup_check("a", "S"), Err(Code::NotSpaceMember), "CP-OP-CATCHUP ④ — not a member is 3013");
    assert_eq!(s.catchup_check("a", "nope"), Err(Code::SpaceNotFound), "CP-OP-CATCHUP ④ — no such space is 3012");
}

#[test]
fn roster_at_message_time() {
    let s = store(Visibility::Shared);
    join(&s, "S", "a", 10);
    join(&s, "S", "c", 10);
    let t = fresh_t(s.send("a", "S", ID_A, "m", Some(20)));
    join(&s, "S", "b", 30);
    s.leave("S", "c", None, Some(40)).expect("leave c");
    let at = s.roster("S").expect("roster").at(t);
    assert!(!at.contains(&"b".to_string()), "S-sym-IMPL-CHAT-FILTER-1 — a member who joined after M does not get M");
    assert!(at.contains(&"c".to_string()), "S-sym-IMPL-CHAT-FILTER-1 — a member who left after M still gets M");
}

#[tokio::test]
async fn cache_reloads_only_when_older_than_the_message() {
    let s = store(Visibility::Shared);
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
}

#[test]
fn history_pages_after_before_and_latest() {
    let s = store(Visibility::Shared);
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
}

#[test]
fn a_page_never_splits_one_t() {
    let s = store(Visibility::Shared);
    join(&s, "S", "a", 1);
    for (i, at) in [100u64, 200, 200, 200, 300].iter().enumerate() {
        fresh_t(s.send("a", "S", &id(i), "m", Some(*at)));
    }
    let ts = |p: &Page| p.messages.iter().map(|m| m.t).collect::<Vec<_>>();
    let p = s.history("a", "S", Dir::Latest, 2).expect("latest");
    assert_eq!(ts(&p), vec![200, 200, 200, 300], "CP-OP-SPACE-HISTORY ⑤ — the page takes every message of its edge t");
    assert!(p.more);
    let same: Vec<&str> = p.messages[..3].iter().map(|m| m.msg_id.as_str()).collect();
    let mut sorted = same.clone();
    sorted.sort();
    assert_eq!(same, sorted, "CP-OP-SPACE-HISTORY ① — same t is in msg_id order");
    let p = s.history("a", "S", Dir::Before(200), 2).expect("before");
    assert_eq!((ts(&p), p.more), (vec![1, 100], false), "the join message at t 1 and the post at 100");
    let p = s.history("a", "S", Dir::After(100), 1).expect("after");
    assert_eq!((ts(&p), p.more), (vec![200, 200, 200], true), "CP-OP-SPACE-HISTORY ⑤ — after keeps one t whole too");
    let p = s.history("a", "S", Dir::After(200), 1).expect("after");
    assert_eq!((ts(&p), p.more), (vec![300], false), "continuing from the edge t loses nothing and repeats nothing");
}

#[test]
fn joined_visibility_hides_what_came_before_the_interval() {
    let s = Store::open(":memory:", Visibility::Joined).expect("open");
    s.create_space("S", None).expect("space");
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
}

#[test]
fn unread_is_total_minus_read() {
    let s = store(Visibility::Shared);
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
}

#[test]
fn undelivered_is_written_with_the_message_and_cleared_by_ack() {
    let s = store(Visibility::Shared);
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
    assert!(s.pending("b").expect("b").is_empty(), "CP-OP-MESSAGE ⑨ — an ACK clears that member's record");
    assert_eq!(s.pending("c").expect("c"), vec![ID_A.to_string()], "another member's record stays");
    s.purge(11).expect("purge");
    assert!(s.pending("c").expect("c").is_empty(), "records older than the keep window are removed");
}

#[test]
fn member_change_is_a_message() {
    let s = store(Visibility::Shared);
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
}

#[test]
fn catchup_merges_records_and_since_in_t_order() {
    let s = store(Visibility::Shared);
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
    assert_eq!(page.iter().map(|m| (m.t, m.msg_id.clone())).collect::<Vec<_>>(), vec![(100, id(0)), (100, id(1)), (300, id(3))], "CP-OP-CATCHUP ② — records in (t, msg_id) order");
    let page = s.catchup("b", None, Some(200), &Cursor::default(), 100, 1 << 20).expect("since");
    assert_eq!(page.iter().map(|m| m.msg_id.clone()).collect::<Vec<_>>(), vec![id(0), id(1), id(2), id(3)], "CP-OP-CATCHUP ② — t ≥ 200 joins the records once each");
    let page = s.catchup("b", Some("S"), None, &Cursor::default(), 100, 1 << 20).expect("one space");
    assert_eq!(page.iter().map(|m| m.msg_id.clone()).collect::<Vec<_>>(), vec![id(0)], "CP-OP-CATCHUP ③ — one space only");
    let first = s.catchup("b", None, None, &Cursor::default(), 2, 1 << 20).expect("bundle");
    assert_eq!(first.len(), 2, "CP-OP-MESSAGES ① — at most the bundle count");
    let last = first.last().expect("last");
    let next = s.catchup("b", None, None, &Cursor { t: last.t, msg_id: last.msg_id.clone() }, 2, 1 << 20).expect("next");
    assert_eq!(next.iter().map(|m| m.msg_id.clone()).collect::<Vec<_>>(), vec![id(3)], "the cursor continues without a gap or a repeat");
    let one = s.catchup("b", None, None, &Cursor::default(), 100, 1).expect("bytes");
    assert_eq!(one.len(), 1, "CP-OP-MESSAGES ① — a message over the byte cap goes alone");
    s.leave("S", "b", None, Some(400)).expect("leave S");
    let page = s.catchup("b", None, Some(0), &Cursor::default(), 100, 1 << 20).expect("since after leaving");
    assert!(page.iter().filter(|m| m.space_id == "S").all(|m| m.msg_id == id(0)), "a space left is not read by t — only its records come");
}

fn temp_db(tag: &str) -> String {
    let p = std::env::temp_dir().join(format!("oxchat-{tag}-{}-{}.db", std::process::id(), common::now_ms()));
    p.to_string_lossy().into_owned()
}

#[test]
fn two_stores_on_one_file_count_every_message() {
    let path = temp_db("two");
    let a = Store::open(&path, Visibility::Shared).expect("open a");
    a.create_space("S", None).expect("space");
    join(&a, "S", "a", 1);
    let b = Store::open(&path, Visibility::Shared).expect("open b");
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
    let c = Store::open(&path, Visibility::Shared).expect("open c");
    let e = c.list("a").expect("list").remove(0);
    assert_eq!((e.total, e.last_t), (80, Some(5000)), "two gateways on one file — total counts every message");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn visibility_is_fixed_at_join() {
    let path = temp_db("vis");
    let s = Store::open(&path, Visibility::Joined).expect("open");
    s.create_space("S", None).expect("space");
    join(&s, "S", "a", 1);
    fresh_t(s.send("a", "S", ID_A, "before", Some(10)));
    join(&s, "S", "b", 20);
    drop(s);
    let s = Store::open(&path, Visibility::Shared).expect("reopen with another default");
    assert!(posts(&s.history("b", "S", Dir::Latest, 60).expect("history")).is_empty(), "a later policy change does not reach a member who already joined");
    join(&s, "S", "c", 30);
    assert_eq!(posts(&s.history("c", "S", Dir::Latest, 60).expect("history")).len(), 1, "a new member gets the policy of its own join");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_retry_after_leaving_still_gets_the_first_t() {
    let s = store(Visibility::Shared);
    join(&s, "S", "a", 1);
    let t = fresh_t(s.send("a", "S", ID_A, "x", Some(10)));
    s.leave("S", "a", None, Some(20)).expect("leave");
    match s.send("a", "S", ID_A, "x", Some(30)).expect("retry") {
        Sent::Again { msg } => assert_eq!(msg.t, t, "the stored message answers the retry even after leaving"),
        Sent::Fresh { .. } => panic!("must not store again"),
    }
}

#[test]
fn same_msg_id_in_another_space_conflicts() {
    let s = store(Visibility::Shared);
    s.create_space("T", None).expect("space T");
    join(&s, "S", "a", 1);
    join(&s, "T", "a", 1);
    fresh_t(s.send("a", "S", ID_A, "x", Some(10)));
    assert_eq!(s.send("a", "T", ID_A, "x", Some(10)), Err(Code::MsgIdConflict), "a msg_id is global — another space is a conflict");
}

fn rel(kind: ChatKind, to: &str) -> Outgoing {
    Outgoing { kind: Some(kind), r#ref: Some(to.into()), ..Outgoing::default() }
}

fn edit(to: &str, content: &str) -> Outgoing {
    Outgoing { content: content.into(), ext: Some(serde_json::json!({ "app": 1 })), ..rel(ChatKind::Edit, to) }
}

fn react(to: &str, keys: &[&str]) -> Outgoing {
    Outgoing { keys: Some(keys.iter().map(|k| k.to_string()).collect()), ..rel(ChatKind::React, to) }
}

#[test]
fn relations_are_checked_on_the_pointer_only() {
    let s = store(Visibility::Shared);
    s.create_space("T", None).expect("space T");
    for u in ["a", "b"] {
        join(&s, "S", u, 1);
    }
    join(&s, "T", "a", 1);
    fresh_t(s.send("a", "S", &id(1), "hello", Some(10)));
    fresh_t(s.send("a", "T", &id(2), "there", Some(10)));
    assert_eq!(s.post("b", "S", &id(3), &edit(&id(1), "x"), Some(11)), Err(Code::NotRefSender), "연:RULE-CHAT-REF-CHECK — someone else's post cannot be edited");
    assert_eq!(s.post("b", "S", &id(3), &rel(ChatKind::Delete, &id(1)), Some(11)), Err(Code::NotRefSender), "nor deleted");
    fresh_t(s.post("b", "S", &id(4), &react(&id(1), &["+1"]), Some(12)));
    assert_eq!(s.post("a", "S", &id(5), &edit("missing", "x"), Some(13)), Err(Code::RefNotFound), "an unknown msg_id");
    assert_eq!(s.post("a", "S", &id(5), &edit(&id(2), "x"), Some(13)), Err(Code::RefNotFound), "a post of another space");
    assert_eq!(s.post("a", "S", &id(5), &react(&id(4), &["x"]), Some(13)), Err(Code::RefNotFound), "a relation is not a post");
    let reply = Outgoing { r#ref: Some(id(1)), ..Outgoing::text("reply") };
    fresh_t(s.post("b", "S", &id(6), &reply, Some(14)));
    assert_eq!(s.list("a").expect("list").iter().find(|e| e.space_id == "S").expect("S").total, 2, "연:RULE-CHAT-UNREAD — posts only (the post and the reply)");
}

#[test]
fn delete_clears_the_post_and_what_points_at_it() {
    let s = store(Visibility::Shared);
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
    fresh_t(s.post("b", "S", &id(6), &Outgoing { r#ref: Some(id(1)), ..Outgoing::text("why?") }, Some(15)));
    let page = s.history("b", "S", Dir::Latest, 50).expect("history");
    let orig = page.messages.iter().find(|m| m.msg_id == id(1)).expect("the envelope stays");
    assert_eq!((orig.content.as_str(), orig.ext.is_none()), ("", true), "S-sym-IMPL-CHAT-STORE-5① — the post is emptied");
    let ed = page.relations.iter().find(|m| m.kind == Some(ChatKind::Edit)).expect("the edit envelope stays");
    assert_eq!((ed.content.as_str(), ed.ext.is_none()), ("", true), "S-sym-IMPL-CHAT-STORE-5② — the edited text is gone too");
    let re = page.relations.iter().find(|m| m.kind == Some(ChatKind::React)).expect("the reaction envelope stays");
    assert_eq!(re.keys.as_deref(), Some(&[][..]), "S-sym-IMPL-CHAT-STORE-5③ — reaction keys are emptied");
    assert!(page.relations.iter().any(|m| m.kind == Some(ChatKind::Delete)), "the delete rides with the page");
    assert!(page.messages.iter().all(|m| !m.content.contains("secret")) && page.relations.iter().all(|m| !m.content.contains("secret")));
}

#[test]
fn a_page_counts_visible_messages_and_folds_relations() {
    let s = store(Visibility::Shared);
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
}

#[test]
fn a_tie_on_t_is_broken_by_msg_id() {
    let s = store(Visibility::Shared);
    join(&s, "S", "a", 1);
    fresh_t(s.send("a", "S", &id(1), "p", Some(10)));
    fresh_t(s.post("a", "S", &id(9), &edit(&id(1), "later id"), Some(20)));
    fresh_t(s.post("a", "S", &id(8), &edit(&id(1), "earlier id"), Some(20)));
    let page = s.history("a", "S", Dir::Latest, 10).expect("history");
    let ed: Vec<&ChatMessage> = page.relations.iter().filter(|m| m.kind == Some(ChatKind::Edit)).collect();
    assert_eq!(ed[0].content, "later id", "연:RULE-CHAT-FOLD — same t, the larger msg_id wins");
}

#[test]
fn delete_and_edit_racing_on_one_post_leave_no_text() {
    let path = temp_db("race");
    let a = Store::open(&path, Visibility::Shared).expect("a");
    a.create_space("S", None).expect("space");
    a.join("S", "u", None, Some(1)).expect("join");
    for round in 0..20usize {
        let orig = id(1000 + round);
        fresh_t(a.send("u", "S", &orig, "text", None));
        let p1 = path.clone();
        let o1 = orig.clone();
        let del = std::thread::spawn(move || {
            let s = Store::open(&p1, Visibility::Shared).expect("s1");
            s.post("u", "S", &format!("d{round:035}"), &rel(ChatKind::Delete, &o1), None)
        });
        let p2 = path.clone();
        let o2 = orig.clone();
        let ed = std::thread::spawn(move || {
            let s = Store::open(&p2, Visibility::Shared).expect("s2");
            s.post("u", "S", &format!("e{round:035}"), &edit(&o2, "revived"), None)
        });
        let (d, e) = (del.join().expect("del"), ed.join().expect("edit"));
        assert!(d.is_ok(), "the delete goes through");
        assert!(e.is_ok() || e == Err(Code::RefDeleted), "S-sym-IMPL-CHAT-STORE-4① — the edit either went first or is refused: {e:?}");
    }
    let page = a.history("u", "S", Dir::Latest, 200).expect("history");
    assert!(page.relations.iter().all(|m| m.content.is_empty()), "S-sym-IMPL-CHAT-STORE-4② — no edit text survives a delete");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_retry_after_delete_still_answers_the_first_t() {
    let s = store(Visibility::Shared);
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
}

#[test]
fn relations_follow_the_visibility_start() {
    let s = store(Visibility::Joined);
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
}

#[test]
fn a_page_stays_within_the_byte_budget() {
    let s = store(Visibility::Shared);
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
}

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
