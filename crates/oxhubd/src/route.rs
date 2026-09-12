// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§15-2 · §15-3 · §15-5 · §16-1 · model: claude-opus-5

//! 배치와 라우팅 — ★**방 단위 · 순수 함수.**
//!
//! ★**hub 상태를 읽지 않는다** — 그래서 재시작·다중화에도 결론이 같다.

use oxsig::Code;

/// 후보 노드 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// ★**안정값**(설정) — 배치 키다. `sfu_id`(기동마다 새 값)가 아니다.
    pub node_id: String,
    /// ★**노드 `live`** — 그 노드의 `node` 토큰이 서 있나(B 평면 축).
    pub live: bool,
    /// 토큰이 사라진 시각 — ★**grace 동안 후보에 남긴다.**
    pub gone_at: Option<u64>,
}

/// ★소멸한 노드를 후보에서 빼기까지 기다리는 시간.
pub const GRACE_MS: u64 = 10_000;

/// FNV-1a — ★**순서가 아니라 값이 배치를 정한다.** 구현이 달라도 같은 답이어야 하므로
/// 표준 해시(`DefaultHasher`)를 쓰지 않는다(그것은 판마다 값이 바뀔 수 있다).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn score(node_id: &str, room_id: &str) -> u64 {
    let mut buf = Vec::with_capacity(node_id.len() + 1 + room_id.len());
    buf.extend_from_slice(node_id.as_bytes());
    buf.push(0x1f);
    buf.extend_from_slice(room_id.as_bytes());
    fnv1a(&buf)
}

/// ★**HRW(rendezvous)** — `hash(node_id‖room_id)` 최대, 동점은 사전순.
///
/// ★**후보는 `node` 토큰이 든 노드 전량이다 — 살아 있는 것만으로 좁히지 않는다.**
/// 좁히면 노드 하나가 죽는 창에 그 방이 다른 노드로 옮겨 붙어 *"한 방은 한 sfud"* 가 창마다 흔들린다.
pub fn place<'a>(nodes: &'a [Node], room_id: &str, now: u64) -> Option<&'a Node> {
    nodes
        .iter()
        // ★소멸은 grace 동안 집합에 남긴다 — 망이 잠깐 흔들릴 때마다 방이 옮겨 다니지 않게.
        .filter(|n| n.gone_at.is_none_or(|t| now.saturating_sub(t) <= GRACE_MS))
        .max_by(|a, b| {
            let (sa, sb) = (score(&a.node_id, room_id), score(&b.node_id, room_id));
            sa.cmp(&sb).then_with(|| a.node_id.cmp(&b.node_id))
        })
}

/// 방 생성 때의 판정. ★**뽑힌 노드가 `노드 live` 가 아니면 `5001`** 이다(정§16-1).
///
/// ★**배치 함수 자체는 그대로다** — 후보를 좁히지 않고, 거절만 한다.
/// 토큰이 다시 서면 ★**같은 방이 같은 노드로** 간다.
pub fn place_for_create<'a>(nodes: &'a [Node], room_id: &str, now: u64) -> Result<&'a Node, Code> {
    match place(nodes, room_id, now) {
        None => Err(Code::SfuUnavailable),
        Some(n) if !n.live => Err(Code::SfuUnavailable),
        Some(n) => Ok(n),
    }
}

/// 방 → node 맵. ★**hub 가 아는 방 전량이 곧 맵이다.**
#[derive(Debug, Default)]
pub struct RoomMap {
    items: Vec<(String, String)>,
}

impl RoomMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn learn(&mut self, room_id: &str, node_id: &str) {
        if let Some(e) = self.items.iter_mut().find(|(r, _)| r == room_id) {
            e.1 = node_id.to_string();
            return;
        }
        self.items.push((room_id.to_string(), node_id.to_string()));
    }

    pub fn forget(&mut self, room_id: &str) {
        self.items.retain(|(r, _)| r != room_id);
    }

    /// ★**맵에 없으면 `3001`** — 없는 방은 방이 없는 것이지 서버 사정이 아니다.
    /// ★**default 폴백을 두지 않는다**(두면 두 노드가 같은 방을 쥔다).
    pub fn route(&self, room_id: &str) -> Result<&str, Code> {
        self.items
            .iter()
            .find(|(r, _)| r == room_id)
            .map(|(_, n)| n.as_str())
            .ok_or(Code::RoomNotFound)
    }

    /// ★**같은 방을 둘이 쥐었나** — 1차는 탐지·계수·경보다(자동 수선은 실측 뒤).
    pub fn duplicates(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        let mut dup: Vec<&str> = Vec::new();
        for (r, _) in &self.items {
            if seen.contains(&r.as_str()) {
                dup.push(r.as_str());
            } else {
                seen.push(r.as_str());
            }
        }
        dup
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes(ids: &[&str]) -> Vec<Node> {
        ids.iter().map(|i| Node { node_id: (*i).into(), live: true, gone_at: None }).collect()
    }

    #[test]
    fn 배치는_순수_함수다() {
        let ns = nodes(&["a", "b", "c"]);
        let first = place(&ns, "r1", 0).expect("place").node_id.clone();
        for _ in 0..5 {
            assert_eq!(place(&ns, "r1", 999).expect("place").node_id, first, "★hub 상태를 안 읽는다");
        }
        // 순서를 바꿔도 같은 답이어야 한다.
        let mut rev = ns.clone();
        rev.reverse();
        assert_eq!(place(&rev, "r1", 0).expect("place").node_id, first);
    }

    #[test]
    fn 죽은_노드도_후보로_남는다() {
        // ★살아 있는 것만으로 좁히면 그 창에 방이 다른 노드로 옮겨 붙는다.
        let mut ns = nodes(&["a", "b", "c"]);
        let target = place(&ns, "r1", 0).expect("place").node_id.clone();
        for n in ns.iter_mut() {
            if n.node_id == target {
                n.live = false;
            }
        }
        assert_eq!(place(&ns, "r1", 0).expect("place").node_id, target, "★짝이 안 바뀐다");
    }

    #[test]
    fn 안_선_노드로는_방을_안_만든다() {
        let mut ns = nodes(&["a", "b"]);
        let target = place(&ns, "r1", 0).expect("place").node_id.clone();
        for n in ns.iter_mut() {
            if n.node_id == target {
                n.live = false;
            }
        }
        assert_eq!(place_for_create(&ns, "r1", 0), Err(Code::SfuUnavailable));
        // ★토큰이 다시 서면 같은 방이 같은 노드로 간다.
        for n in ns.iter_mut() {
            n.live = true;
        }
        assert_eq!(place_for_create(&ns, "r1", 0).expect("place").node_id, target);
    }

    #[test]
    fn grace_가_지나야_후보에서_빠진다() {
        let mut ns = nodes(&["a", "b", "c"]);
        let target = place(&ns, "r1", 0).expect("place").node_id.clone();
        for n in ns.iter_mut() {
            if n.node_id == target {
                n.gone_at = Some(1_000);
            }
        }
        assert_eq!(place(&ns, "r1", 1_000 + GRACE_MS).expect("p").node_id, target, "★grace 안");
        assert_ne!(place(&ns, "r1", 1_000 + GRACE_MS + 1).expect("p").node_id, target);
    }

    #[test]
    fn 노드가_늘면_이동은_일부다() {
        // ★안정 해싱 — 노드 증감 시 이동 1/N.
        let before = nodes(&["a", "b", "c"]);
        let after = nodes(&["a", "b", "c", "d"]);
        let rooms: Vec<String> = (0..200).map(|i| format!("r{i}")).collect();
        let moved = rooms
            .iter()
            .filter(|r| {
                place(&before, r, 0).expect("p").node_id != place(&after, r, 0).expect("p").node_id
            })
            .count();
        assert!(moved < rooms.len() / 2, "옮긴 방 {moved}/200 — 전 방 재배치면 안정 해싱이 죽은 것이다");
    }

    #[test]
    fn 맵에_없으면_3001_이다() {
        let mut m = RoomMap::new();
        m.learn("r1", "a");
        assert_eq!(m.route("r1"), Ok("a"));
        // ★기본 노드 폴백을 두면 두 노드가 같은 방을 쥔다.
        assert_eq!(m.route("없는방"), Err(Code::RoomNotFound));
    }

    #[test]
    fn 배치가_바뀌면_맵도_바뀐다() {
        let mut m = RoomMap::new();
        m.learn("r1", "a");
        m.learn("r1", "b");
        assert_eq!(m.route("r1"), Ok("b"));
        assert!(m.duplicates().is_empty(), "★한 방에 한 줄이다");
    }
}
