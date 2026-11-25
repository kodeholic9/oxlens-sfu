// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-CHAT-EXT-SUB · model: claude-opus-5-5
//! 1층 — 3rd 연결 둘(같은 계정)에 실시간 글을 나눠 줄 때 시작 연결이 글마다 돈다: 계정별 계수기(`rr_next`)가 한 칸씩 오르고 그 값으로 시작하는 고리(`ring`)의 첫 연결이 번갈아 둘 다 된다 · 넘기는 곳(`ses::ext_fanout`)은 그 계수기 값으로 고리를 지어 그 차례대로 넘긴다(context 202611/20261124b 서버 규칙 갈래).

use std::collections::BTreeSet;

use common::system::Visibility;
use oxhubd::chat::store::Store;
use oxhubd::chat::zgate::plan::ring;
use oxhubd::chat::{Chat, Limits};

#[test]
fn live_messages_take_turns_over_both_connections_of_one_account() {
    let chat = Chat::new(Store::open("memory:", Visibility::Shared).expect("store"), Limits::default());
    let firsts: Vec<usize> = (0..6).map(|_| ring(2, chat.rr_next("acme"))[0]).collect();
    assert_eq!(firsts.iter().copied().collect::<BTreeSet<_>>(), BTreeSet::from([0, 1]), "S-sym-RULE-CHAT-EXT-SUB-4② successive messages start on both connections: {firsts:?}");
    assert!(firsts.windows(2).all(|w| w[0] != w[1]), "S-sym-RULE-CHAT-EXT-SUB-4② the starting connection alternates message by message: {firsts:?}");
    assert_eq!(ring(2, 7), vec![1, 0], "S-sym-RULE-CHAT-EXT-SUB-4② a message falls over to the other connection after its first — the ring holds both");
    assert_eq!(ring(2, chat.rr_next("other"))[0], 0, "stand check — the counter is per account (another account starts from its own zero)");
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ses/mod.rs")).expect("ses/mod.rs");
    let start = text.find("async fn ext_fanout(").expect("stand check — ext_fanout");
    let body = &text[start..start + text[start..].find("\n}\n").expect("end")];
    assert!(body.contains("let start = chat.rr_next(key);"), "S-sym-RULE-CHAT-EXT-SUB-4② the fan-out takes the account counter for its start");
    assert!(body.contains("for i in crate::chat::zgate::plan::ring(list.len(), start) {"), "S-sym-RULE-CHAT-EXT-SUB-4② the fan-out offers the connections in ring order from that start");
}
