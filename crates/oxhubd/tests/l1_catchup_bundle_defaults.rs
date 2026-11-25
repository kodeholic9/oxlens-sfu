// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-CHAT-CATCHUP · 정:IMPL-CFG-SYSTEM-FILE · 연:CHAT.op.MESSAGES · model: claude-opus-5-5
//! 1층 — 따라잡기 묶음 상한의 기본값: 시스템 파일 `[chat]` 에 값을 안 적으면 `catchup_bundle_max_msgs` 100 · `catchup_bundle_max_bytes` 524,288(512 KB)이고, 게이트웨이는 그 두 값을 그대로 묶음 상한으로 쓴다(context 202611/20261124b 서버 규칙 갈래).

use common::system::System;

#[test]
fn the_catchup_bundle_caps_default_to_one_hundred_messages_and_half_a_megabyte() {
    let min = "[hub]\nlisten = \"0.0.0.0:1974\"\n[bus]\nnamespace = \"ox\"\n[chat]\ndb = \"sqlite:/tmp/oxlens-l1-defaults.db\"\n";
    let s = System::parse(min).expect("stand check — a minimal system file parses");
    let c = s.chat.expect("stand check — the chat section");
    assert_eq!((c.catchup_bundle_max_msgs, c.catchup_bundle_max_bytes), (100, 524_288), "CP-OP-MESSAGES① the default bundle caps are 100 messages and 512 KB");
    let main = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs")).expect("main.rs");
    assert!(main.contains("bundle_msgs: c.catchup_bundle_max_msgs,") && main.contains("bundle_bytes: c.catchup_bundle_max_bytes,"), "CP-OP-MESSAGES① the gateway takes the bundle caps from the system file values");
    let set = System::parse(&format!("{min}catchup_bundle_max_msgs = 7\ncatchup_bundle_max_bytes = 4096\n")).expect("stand check — set values parse");
    let c = set.chat.expect("chat");
    assert_eq!((c.catchup_bundle_max_msgs, c.catchup_bundle_max_bytes), (7, 4096), "CP-OP-MESSAGES① the caps follow the setting");
}
