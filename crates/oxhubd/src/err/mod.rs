// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-ERR-ISSUERS · 정:RULE-OPS-OBSERVE · model: claude-opus-5-5
//! 코드별 계수기 — 실패 코드마다 하나(어떤 거절이 늘고 있나를 밖에서 본다).

use std::collections::BTreeMap;
use std::sync::Mutex;

use oxsig::Code;

#[derive(Default)]
pub struct Counters {
    by_code: Mutex<BTreeMap<u16, u64>>,
}

impl Counters {
    pub fn bump(&self, c: Code) {
        if let Ok(mut m) = self.by_code.lock() {
            m.entry(c.number()).and_modify(|v| *v += 1).or_insert(1);
        }
    }

    pub fn snapshot(&self) -> BTreeMap<String, u64> {
        self.by_code.lock().map(|m| m.iter().map(|(k, v)| (format!("code_{k}"), v.to_owned())).collect()).unwrap_or_default()
    }
}
