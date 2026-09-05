// author: kodeholic (powered by Claude)
//! 배치·라우팅 — 정§15-1(HRW 순수 함수, 키는 `node_id`)·§15-2(방 귀속 op 의 방 결정).

use dashmap::DashMap;
use oxsig::body::affiliation::AffiliationReq;
use oxsig::op::Op;
use oxsig::schema::Version;
use oxsig::FailCode;
use serde_json::Value;

/// FNV-1a 64 + splitmix64 finalizer — 의존성 없이 결정적. 접두가 같은 짧은 키(`sfu-1`/`sfu-2`)에서
/// FNV 만으로는 상위 비트가 몰려 HRW 의 max 가 편중된다 — finalizer 가 그것을 편다.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let h = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
    let mut z = h.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// 정§15-1 — `hash(node_id‖room_id)` 최대, 동점은 사전순. hub 상태를 읽지 않는다.
pub fn place<'a>(node_ids: &[&'a str], room_id: &str) -> Option<&'a str> {
    node_ids
        .iter()
        .copied()
        .max_by(|a, b| {
            let ha = fnv1a64(format!("{a}\u{0}{room_id}").as_bytes());
            let hb = fnv1a64(format!("{b}\u{0}{room_id}").as_bytes());
            ha.cmp(&hb).then_with(|| b.cmp(a))
        })
}

/// 방 → 노드 배치 맵 + 방별 version 커서(급사 통지용 통과값 — 상태 그림자가 아니다).
#[derive(Default)]
pub struct RoomMap {
    rooms: DashMap<String, String>,
    cursor: DashMap<String, Version>,
}

impl RoomMap {
    pub fn node_of(&self, room_id: &str) -> Option<String> {
        self.rooms.get(room_id).map(|n| n.clone())
    }

    /// 명시 배치(생성 요청) — 원자적 결정+기록. 이미 있으면 그 노드(멱등).
    pub fn assign(&self, room_id: &str, node_id: &str) -> String {
        self.rooms.entry(room_id.to_owned()).or_insert_with(|| node_id.to_owned()).clone()
    }

    /// 생성 통보로 배운다. 다른 노드가 이미 쥐고 있으면 그것을 돌려준다(충돌 관측).
    pub fn learn(&self, room_id: &str, node_id: &str) -> Option<String> {
        let prev = self.rooms.entry(room_id.to_owned()).or_insert_with(|| node_id.to_owned()).clone();
        (prev != node_id).then_some(prev)
    }

    pub fn unbind(&self, room_id: &str) -> Option<String> {
        self.cursor.remove(room_id);
        self.rooms.remove(room_id).map(|(_, n)| n)
    }

    /// 급사(정§15-1) — 그 노드의 배치 전량을 풀고 방 목록을 돌려준다.
    pub fn unbind_node(&self, node_id: &str) -> Vec<(String, Option<Version>)> {
        let rooms: Vec<String> = self.rooms.iter().filter(|e| e.value() == node_id).map(|e| e.key().clone()).collect();
        rooms
            .into_iter()
            .map(|r| {
                self.rooms.remove(&r);
                let v = self.cursor.remove(&r).map(|(_, v)| v);
                (r, v)
            })
            .collect()
    }

    /// 방 상태 통지를 통과시키며 마지막 version 을 기억한다.
    pub fn touch(&self, room_id: &str, v: Version) {
        self.cursor.insert(room_id.to_owned(), v);
    }

    pub fn len(&self) -> usize {
        self.rooms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
    }
}

/// 정§15-2 — 방 귀속 op 의 방. `AFFILIATION` 은 `pub_select` → `pub_deselect`, 둘 다 없으면 `1003`.
pub fn room_of(op: Op, body: &Value) -> Result<String, FailCode> {
    if op == Op::Affiliation {
        let req: AffiliationReq = serde_json::from_value(body.clone()).map_err(|_| FailCode::InvalidPayload)?;
        return req.pub_select.or(req.pub_deselect).ok_or(FailCode::MissingField);
    }
    body.get("room_id").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned).ok_or(FailCode::MissingField)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hrw_is_deterministic_and_moves_about_one_over_n() {
        let three = ["sfu-1", "sfu-2", "sfu-3"];
        let two = ["sfu-1", "sfu-2"];
        let rooms: Vec<String> = (0..300).map(|i| format!("room-{i}")).collect();
        let mut moved = 0;
        for r in &rooms {
            let a = place(&three, r).unwrap();
            assert_eq!(a, place(&["sfu-3", "sfu-1", "sfu-2"], r).unwrap(), "입력 순서 무관");
            if a != "sfu-3" {
                assert_eq!(a, place(&two, r).unwrap(), "sfu-3 제거는 sfu-3 방만 옮긴다");
            } else {
                moved += 1;
            }
        }
        assert!((60..=140).contains(&moved), "moved={moved}");
        assert_eq!(place(&[], "r"), None);
    }

    #[test]
    fn room_map_and_node_down() {
        let m = RoomMap::default();
        assert_eq!(m.assign("r1", "a"), "a");
        assert_eq!(m.assign("r1", "b"), "a", "멱등");
        assert_eq!(m.learn("r2", "b"), None);
        assert_eq!(m.learn("r2", "c"), Some("b".into()), "충돌 관측");
        m.touch("r1", Version { epoch: "e".into(), seq: 5 });
        let down = m.unbind_node("a");
        assert_eq!(down.len(), 1);
        assert_eq!(down[0].1.as_ref().map(|v| v.seq), Some(5));
        assert_eq!(m.node_of("r1"), None);
        assert_eq!(m.unbind("r2"), Some("b".into()));
        assert!(m.is_empty());
    }

    #[test]
    fn route_rules() {
        assert_eq!(room_of(Op::RoomJoin, &json!({"room_id":"r"})), Ok("r".into()));
        assert_eq!(room_of(Op::RoomJoin, &json!({})), Err(FailCode::MissingField));
        assert_eq!(room_of(Op::Affiliation, &json!({"pub_deselect":"a","pub_select":"b"})), Ok("b".into()));
        assert_eq!(room_of(Op::Affiliation, &json!({"change_id":"x"})), Err(FailCode::MissingField));
    }
}
