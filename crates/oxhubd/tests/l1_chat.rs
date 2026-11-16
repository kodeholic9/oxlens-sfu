// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-CHAT-STORE · 정:IMPL-CHAT-MEMBERSHIP · 정:IMPL-CHAT-FILTER · 정:IMPL-CHAT-READ · 연:RULE-CHAT-MSG-ID · 연:RULE-CHAT-ORDER · 연:RULE-CHAT-VISIBLE · 연:RULE-CHAT-UNREAD · 연:RULE-CHAT-HISTORY-PAGE · 연:RULE-CHAT-CATCHUP-BASIS · model: claude-opus-5-5
//! 1층 — 채팅 저장소(공간 시계 · msg_id 멱등 · 멤버 구간 · 보이기 시작점 · 쪽 · 안 읽은 수)와 받는 게이트웨이의 멤버 캐시(판이 작을 때만 다시 읽음 · 명단(t_M)).

use common::system::Visibility;
use oxhubd::chat::store::{Dir, Sent, Store};
use oxhubd::chat::{Chat, valid_msg_id};
use oxsig::Code;

const ID_A: &str = "a/6f9619ff-8b86-4011-b42d-00c04fc964ff";
const ID_B: &str = "a/7a1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d";
const ID_C: &str = "b/0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d";

fn store(vis: Visibility) -> Store {
    let s = Store::open(":memory:", vis).expect("open");
    s.create_space("S", None).expect("space");
    s
}

fn fresh_t(r: Result<Sent, Code>) -> u64 {
    match r.expect("send") {
        Sent::Fresh { msg, .. } => msg.t,
        Sent::Again { .. } => panic!("expected a fresh store"),
    }
}

#[test]
fn same_ms_gets_plus_one_and_clock_back_never_lowers_t() {
    let s = store(Visibility::Shared);
    s.join("S", "a", 1000).expect("join");
    let t1 = fresh_t(s.send("a", "S", ID_A, "x", 5000));
    let t2 = fresh_t(s.send("a", "S", ID_B, "y", 5000));
    assert_eq!(t2, t1 + 1, "S-sym-IMPL-CHAT-STORE-1 — two stores in the same ms differ by one");
    let t3 = fresh_t(s.send("a", "S", "a/1b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d", "z", 10));
    assert!(t3 > t2, "S-sym-IMPL-CHAT-STORE-1 — a clock that went back does not lower t ({t3} > {t2})");
}

#[test]
fn same_msg_id_same_content_answers_the_first_t() {
    let s = store(Visibility::Shared);
    s.join("S", "a", 1).expect("join");
    let t1 = fresh_t(s.send("a", "S", ID_A, "hello", 100));
    match s.send("a", "S", ID_A, "hello", 900).expect("again") {
        Sent::Again { msg } => assert_eq!(msg.t, t1, "S-sym-IMPL-CHAT-STORE-2 — the retry gets the first t and no new store"),
        Sent::Fresh { .. } => panic!("S-sym-IMPL-CHAT-STORE-2 — a retry must not store again"),
    }
    assert_eq!(s.send("a", "S", ID_A, "other", 900), Err(Code::MsgIdConflict), "same msg_id with other content is 3015");
    assert_eq!(s.list("a").expect("list")[0].total, 1, "one message stored");
}

#[test]
fn msg_id_prefix_must_be_the_session_user() {
    assert!(valid_msg_id(ID_A, "a"));
    assert!(!valid_msg_id(ID_A, "b"), "another user's prefix is refused");
    assert!(!valid_msg_id("a/not-a-uuid", "a"), "the rest must be a UUID");
    assert!(!valid_msg_id("a/6f9619ff-8b86-1011-b42d-00c04fc964ff", "a"), "the UUID must be v4");
    assert!(!valid_msg_id("6f9619ff-8b86-4011-b42d-00c04fc964ff", "a"), "the prefix is required");
}

#[test]
fn space_and_membership_checks() {
    let s = store(Visibility::Shared);
    assert_eq!(s.send("a", "nope", ID_A, "x", 1), Err(Code::SpaceNotFound));
    assert_eq!(s.send("a", "S", ID_A, "x", 1), Err(Code::NotSpaceMember));
    s.join("S", "a", 1).expect("join");
    s.leave("S", "a", 2).expect("leave");
    assert_eq!(s.send("a", "S", ID_A, "x", 3), Err(Code::NotSpaceMember), "a member who left is not a member");
    assert_eq!(s.history("a", "S", Dir::Latest, 60), Err(Code::NotSpaceMember));
}

#[test]
fn roster_at_message_time() {
    let s = store(Visibility::Shared);
    s.join("S", "a", 10).expect("join a");
    s.join("S", "c", 10).expect("join c");
    let t = fresh_t(s.send("a", "S", ID_A, "m", 20));
    s.join("S", "b", 30).expect("join b");
    s.leave("S", "c", 40).expect("leave c");
    let r = s.roster("S").expect("roster");
    let at = r.at(t);
    assert!(!at.contains(&"b".to_string()), "S-sym-IMPL-CHAT-FILTER-1 — a member who joined after M does not get M");
    assert!(at.contains(&"c".to_string()), "S-sym-IMPL-CHAT-FILTER-1 — a member who left after M still gets M");
}

#[tokio::test]
async fn cache_reloads_only_when_older_than_the_message() {
    let s = store(Visibility::Shared);
    s.join("S", "a", 10).expect("join a");
    let v1 = s.roster("S").expect("roster").ver;
    let chat = Chat::new(s);
    assert_eq!(chat.roster_at_least("S", v1).await.expect("load").ver, v1);
    chat.store.join("S", "b", 20).expect("join b");
    let got = chat.roster_at_least("S", v1).await.expect("cached");
    assert_eq!(got.ver, v1, "S-sym-IMPL-CHAT-FILTER-2 — a cache not older than the message is not reread");
    let got = chat.roster_at_least("S", v1 + 1).await.expect("reload");
    assert_eq!(got.ver, v1 + 1, "an older cache is reread");
    assert!(got.at(25).contains(&"b".to_string()));
}

#[test]
fn history_pages_after_before_and_latest() {
    let s = store(Visibility::Shared);
    s.join("S", "a", 1).expect("join");
    let ids: Vec<String> = (0..5).map(|i| format!("a/{:08x}-4f50-4a6b-8c7d-9e0f1a2b3c4d", i + 1)).collect();
    let ts: Vec<u64> = ids.iter().map(|id| fresh_t(s.send("a", "S", id, "m", 100))).collect();
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
fn joined_visibility_hides_what_came_before_the_interval() {
    let s = Store::open(":memory:", Visibility::Joined).expect("open");
    s.create_space("S", None).expect("space");
    s.join("S", "a", 1).expect("join a");
    let t_before = fresh_t(s.send("a", "S", ID_A, "before", 10));
    s.join("S", "b", 20).expect("join b");
    let t_after = fresh_t(s.send("a", "S", ID_B, "after", 30));
    let seen: Vec<u64> = s.history("b", "S", Dir::Latest, 60).expect("history").messages.iter().map(|m| m.t).collect();
    assert_eq!(seen, vec![t_after], "joined — b does not see {t_before} sent before b joined");
    s.create_space("P", Some(Visibility::Shared)).expect("shared space");
    s.join("P", "a", 1).expect("join");
    fresh_t(s.send("a", "P", ID_C.replacen('b', "a", 1).as_str(), "old", 40));
    s.join("P", "b", 50).expect("join b");
    assert_eq!(s.history("b", "P", Dir::Latest, 60).expect("history").messages.len(), 1, "a space value overrides the default");
}

#[test]
fn unread_is_total_minus_read() {
    let s = store(Visibility::Shared);
    s.join("S", "a", 1).expect("join a");
    s.join("S", "b", 1).expect("join b");
    fresh_t(s.send("a", "S", ID_A, "1", 10));
    fresh_t(s.send("a", "S", ID_B, "2", 10));
    let e = s.list("b").expect("list").remove(0);
    assert_eq!((e.total, e.read), (2, 0));
    assert_eq!(e.last_t, Some(11));
    assert_eq!(s.view("b", "S").expect("view"), 2);
    let e = s.list("b").expect("list").remove(0);
    assert_eq!(e.total - e.read, 0, "after view nothing is unread");
}
