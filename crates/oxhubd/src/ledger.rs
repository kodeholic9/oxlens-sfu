// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§4-1 · §15-2 · §15-5 · 연§5-4 · model: claude-opus-5

//! 방 대장 — ★**hub 가 아는 방 전량이 곧 맵이다**(정§15-5).
//!
//! ★★**여기 없는 것 둘 — 명단과 `seq`.** 그 둘의 권위는 sfud 다(정§14-1·§15-4). hub 는
//! 대장(이름·정원·TTL)을 밀고 현황을 받아 와 ★**캐시로만** 들고 있는다. 두 곳이 명단을
//! 세면 ★**투명 참가자에서 갈린다** — hub 는 통지로만 배우는데 투명 입퇴장은 통지가 없다.

use std::collections::BTreeMap;

/// 대장 한 줄. ★**만든 사람이 정한 것만** 있다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub id: String,
    pub name: String,
    pub capacity: u32,
    /// ★`None` = 영구.
    pub unused_ttl_secs: Option<u32>,
    pub departure_ttl_secs: Option<u32>,
    pub created_at: u64,
}

#[derive(Debug, Default)]
pub struct Ledger {
    items: Vec<Record>,
    /// sfud 가 준 현황(연§5-4 목록 항목 형). ★**없으면 없는 대로 낸다** — 지어내지 않는다.
    live: BTreeMap<String, serde_json::Value>,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: &str) -> Option<&Record> {
        self.items.iter().find(|r| r.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Record> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// ★**멱등** — 같은 id 로 다시 만들면 그 방이 그대로 온다(`name` 은 무시).
    pub fn create(&mut self, r: Record) -> &Record {
        if let Some(i) = self.items.iter().position(|x| x.id == r.id) {
            return &self.items[i];
        }
        self.items.push(r);
        self.items.last().expect("방금 넣었다")
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.live.remove(id);
        let before = self.items.len();
        self.items.retain(|r| r.id != id);
        before != self.items.len()
    }

    /// sfud 가 준 현황을 받아 둔다. ★**대장에 없는 방은 버린다** — 지운 방의 잔재다.
    pub fn put_live(&mut self, id: &str, v: serde_json::Value) {
        if self.get(id).is_some() {
            self.live.insert(id.to_string(), v);
        }
    }

    /// 대장 + 현황. ★**현황이 아직 없으면 그 필드가 없다** — `0` 으로 메우지 않는다.
    ///
    /// `0` 을 메우면 *"아직 못 물어봤다"* 와 *"정말 비었다"* 가 한 값이 되어,
    /// 기동 직후의 목록이 ★**전부 빈 방**으로 보인다.
    pub fn view(&self, id: &str) -> Option<serde_json::Value> {
        let r = self.get(id)?;
        let mut o = serde_json::json!({
            "room_id": r.id,
            "name": r.name,
            "capacity": r.capacity,
            "created_at": r.created_at,
        });
        let m = o.as_object_mut().expect("방금 지은 객체다");
        if let Some(serde_json::Value::Object(live)) = self.live.get(id) {
            for (k, v) in live {
                m.insert(k.clone(), v.clone());
            }
        }
        Some(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str) -> Record {
        Record {
            id: id.into(),
            name: "첫 이름".into(),
            capacity: 10,
            unused_ttl_secs: Some(300),
            departure_ttl_secs: Some(60),
            created_at: 0,
        }
    }

    #[test]
    fn 명시_id_생성은_멱등이다() {
        let mut l = Ledger::new();
        l.create(rec("r1"));
        let again = l.create(Record { name: "다른 이름".into(), ..rec("r1") });
        assert_eq!(again.name, "첫 이름", "★name 은 무시한다");
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn 현황이_없으면_그_필드가_없다() {
        let mut l = Ledger::new();
        l.create(rec("r1"));
        let v = l.view("r1").expect("대장에 있다");
        assert!(v.get("user_count").is_none(), "★0 으로 메우지 않는다 — 모르는 것은 없는 것이다");
        l.put_live("r1", serde_json::json!({"user_count": 2, "rec": false}));
        assert_eq!(l.view("r1").expect("있다")["user_count"], 2);
    }

    #[test]
    fn 대장에_없는_방의_현황은_버린다() {
        let mut l = Ledger::new();
        l.put_live("r1", serde_json::json!({"user_count": 9}));
        l.create(rec("r1"));
        assert!(l.view("r1").expect("있다").get("user_count").is_none(), "★지운 방의 잔재를 안 받는다");
    }
}
