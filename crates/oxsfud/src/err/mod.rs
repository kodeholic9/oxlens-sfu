// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-ERR-ISSUERS · 정:RULE-OPS-OBSERVE · 연:RULE-ERR-UNKNOWN-BY-LEAD · model: claude-opus-5-5
//! 영역 ERR — 미디어 서버가 내는 실패 코드별 계수 · 사유별 버림 계수(조용한 drop 금지 — `/admin/drops`).

use std::collections::BTreeMap;
use std::sync::Mutex;

use oxsig::Code;

#[derive(Default)]
pub struct Counters {
    codes: Mutex<BTreeMap<u16, u64>>,
    drops: Mutex<BTreeMap<&'static str, u64>>,
}

impl Counters {
    pub fn code(&self, c: u16) {
        let mut g = self.codes.lock().unwrap_or_else(|p| p.into_inner());
        let cur = g.get(&c).copied().unwrap_or(0);
        g.insert(c, cur + 1);
    }

    pub fn drop(&self, why: &'static str) {
        self.add(why, 1);
    }

    pub fn add(&self, why: &'static str, n: u64) {
        if n == 0 {
            return;
        }
        let mut g = self.drops.lock().unwrap_or_else(|p| p.into_inner());
        let cur = g.get(why).copied().unwrap_or(0);
        g.insert(why, cur + n);
    }

    pub fn snapshot(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut m = serde_json::Map::new();
        for (k, v) in self.drops.lock().unwrap_or_else(|p| p.into_inner()).iter() {
            m.insert(k.to_string(), serde_json::Value::from(*v));
        }
        for (k, v) in self.codes.lock().unwrap_or_else(|p| p.into_inner()).iter() {
            let name = Code::from_number(*k).map(|c| c.name()).unwrap_or("?");
            m.insert(format!("code_{k}_{name}"), serde_json::Value::from(*v));
        }
        m
    }
}
