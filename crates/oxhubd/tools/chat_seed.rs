// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-CHAT-MEMBERSHIP · 연:RULE-CHAT-MEMBER-NOTICE · 연:CAND-CHAT-SPACE-ADMIN · 정:CAND-CHAT-STORE-SCHEMA · model: claude-opus-5-5
//! 개발 · 시험 도구 — 채팅 저장소에 공간을 만들고 멤버를 넣고 뺀다(행위자는 사용자가 아니라 `$sys` — 멤버 변경 메시지는 추가 · 뺌). 공간 관리 입구는 아직 정하지 않았다(연:CAND-CHAT-SPACE-ADMIN) — 그때까지 2층 · 3층이 이것으로 공간을 준비한다. 쓰기는 게이트웨이와 같은 저장소 코드를 지난다.
//!
//! chat_seed <db> space <space_id> [shared|joined]
//! chat_seed <db> join <space_id> <user_id>...
//! chat_seed <db> leave <space_id> <user_id>...

use common::system::Visibility;
use oxhubd::chat::store::Store;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let usage = || -> ! {
        eprintln!("chat_seed <db> space <space_id> [shared|joined] | join <space_id> <user_id>... | leave <space_id> <user_id>...");
        std::process::exit(2)
    };
    if a.len() < 3 {
        usage();
    }
    let st = match Store::open(&a[0], Visibility::Shared) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("chat_seed: {e}");
            std::process::exit(2)
        }
    };
    let by = Some(oxhubd::chat::store::SYS);
    let r = match a[1].as_str() {
        "space" => {
            let vis = match a.get(3).map(|s| s.as_str()) {
                None => None,
                Some("shared") => Some(Visibility::Shared),
                Some("joined") => Some(Visibility::Joined),
                Some(_) => usage(),
            };
            st.create_space(&a[2], vis)
        }
        "join" if a.len() > 3 => a[3..].iter().try_for_each(|u| st.join(&a[2], u, by, None).map(|_| ())),
        "leave" if a.len() > 3 => a[3..].iter().try_for_each(|u| st.leave(&a[2], u, by, None).map(|_| ())),
        _ => usage(),
    };
    if let Err(c) = r {
        eprintln!("chat_seed: {} {}", c.num(), c.name());
        std::process::exit(1);
    }
}
