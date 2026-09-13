// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§15-0 · §15-1 · model: claude-opus-5

//! B 평면 버스 — ★**node 사이는 전부 zenoh 다**(정§15-0).
//!
//! ★**hub 는 router 를 임베드한다** — `zenohd` 를 따로 두지 않는다. `zenohd` 는 router
//! 모드 세션을 여는 실행파일일 뿐이라, hub 가 router 로 열면 ★**hub 가 곧 그 node 의
//! `zenohd`** 다. sfud 는 client 로 제 hub 하나에만 붙는다.
//!
//! ★★**`peer` 는 안 쓴다** — clique 강제라 node N 대면 세션 N(N−1)/2 로 늘고,
//! sfud 가 타 node hub 와 직결해 ★**관문 원칙이 깨진다.**
//!
//! ★★**liveliness 토큰에는 값을 실을 수 없다** — 선언 API 가 키만 받는다.
//! 그래서 ★**알려야 할 것을 전부 키에 적는다**(정§15-1).

use crate::system::Zenoh;

/// 키 한 조각이 될 수 있나 — ★**`/`·`*`·`?`·`#`·`$` 는 키 문법이다.**
///
/// ★**설정값이 키를 깨면 그 node 는 아무에게도 안 보인다** — 조용히 붙는 것이 제일 나쁘다.
pub fn is_key_segment(s: &str) -> bool {
    !s.is_empty() && !s.contains(['/', '*', '?', '#', '$'])
}

/// 키 공간(정§15-1). ★**배포 이름공간 `{c}` 가 최상위**다 — 한 버스에 여러 배포가 섞인다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keys {
    ns: String,
}

impl Keys {
    pub fn new(namespace: &str) -> Self {
        Self { ns: namespace.to_string() }
    }

    /// `ox/{c}/int/node/{node}/{inst}` — hub 가 선언하는 ★**node 존재**.
    pub fn node(&self, node: &str, inst: &str) -> String {
        format!("{}/int/node/{node}/{inst}", self.ns)
    }

    /// 전 node 의 존재를 듣는 무늬.
    pub fn node_all(&self) -> String {
        format!("{}/int/node/*/*", self.ns)
    }

    /// `ox/{c}/int/room/{R}/{node}/{inst}` — sfud 가 선언하는 ★**방 위치.**
    ///
    /// ★**sfud 세션에 묶인다** — 그 프로세스가 죽으면 토큰이 세션째 사라져
    /// 전 hub 에서 Delete 로 보인다(정§15-7).
    pub fn room(&self, room: &str, node: &str, inst: &str) -> String {
        format!("{}/int/room/{room}/{node}/{inst}", self.ns)
    }

    pub fn room_all(&self) -> String {
        format!("{}/int/room/*/*/*", self.ns)
    }

    /// `ox/{c}/int/sess/{U}/{node}/{sid}` — hub 가 선언하는 ★**WS 세션 위치.**
    ///
    /// ★**수명은 세션이다**(재개 창 포함) — 소켓이 아니다.
    pub fn sess(&self, user: &str, node: &str, sid: &str) -> String {
        format!("{}/int/sess/{user}/{node}/{sid}", self.ns)
    }

    /// 그 사람이 어느 node 에 붙었나 — ★**구독하지 않고 필요할 때 get 한다**(정§15-5).
    pub fn sess_of(&self, user: &str) -> String {
        format!("{}/int/sess/{user}/*/*", self.ns)
    }

    /// `ox/{c}/int/ev/room/{R}` — sfud 가 내는 방 broadcast.
    pub fn ev_room(&self, room: &str) -> String {
        format!("{}/int/ev/room/{room}", self.ns)
    }

    /// `ox/{c}/int/ev/user/{U}` — sfud 가 내는 unicast.
    pub fn ev_user(&self, user: &str) -> String {
        format!("{}/int/ev/user/{user}", self.ns)
    }

    /// `ox/{c}/int/ev/node/{node}` — hub 가 내는 node 간 통보.
    pub fn ev_node(&self, node: &str) -> String {
        format!("{}/int/ev/node/{node}", self.ns)
    }

    /// `ox/{c}/int/q/node/{node}/**` — node 간 요청.
    pub fn q_node(&self, node: &str, rest: &str) -> String {
        format!("{}/int/q/node/{node}/{rest}", self.ns)
    }

    /// 전 node fan-out.
    pub fn q_all(&self, rest: &str) -> String {
        format!("{}/int/q/node/*/{rest}", self.ns)
    }
}

/// liveliness 키에서 `(node, inst)` 를 되읽는다 — ★**값이 없으니 키가 곧 값이다.**
///
/// ★**모르는 모양이면 `None`** 이다 — 지어내면 없는 node 가 목록에 선다.
pub fn node_of(key: &str) -> Option<(&str, &str)> {
    let mut it = key.rsplitn(3, '/');
    let inst = it.next()?;
    let node = it.next()?;
    let head = it.next()?;
    head.ends_with("/int/node").then_some((node, inst))
}

/// `room` 토큰에서 `(room, node, inst)`.
pub fn room_of(key: &str) -> Option<(&str, &str, &str)> {
    let mut it = key.rsplitn(4, '/');
    let inst = it.next()?;
    let node = it.next()?;
    let room = it.next()?;
    let head = it.next()?;
    head.ends_with("/int/room").then_some((room, node, inst))
}

/// `sess` 토큰에서 `(user, node, sid)`.
pub fn sess_of(key: &str) -> Option<(&str, &str, &str)> {
    let mut it = key.rsplitn(4, '/');
    let sid = it.next()?;
    let node = it.next()?;
    let user = it.next()?;
    let head = it.next()?;
    head.ends_with("/int/sess").then_some((user, node, sid))
}

/// 설정을 zenoh 설정으로 편다. ★**성립 조건 둘을 명시로 적는다**(정§15-0).
///
/// ★`qos.enabled` 와 `lowlatency` 는 기본값이지만 ★**기본값이라 안 적으면 성능 튜닝
/// 중에 꺼진다.** 꺼지면 우선순위 큐가 하나로 합쳐져 §15-4 의 레인 분리가 사라지고,
/// ★**이벤트가 몰린 순간 토큰이 함께 죽는다.**
pub fn config_json(z: &Zenoh) -> Vec<(&'static str, String)> {
    let arr = |v: &[String]| {
        let items: Vec<String> = v.iter().map(|s| format!("\"{s}\"")).collect();
        format!("[{}]", items.join(","))
    };
    vec![
        ("mode", format!("\"{}\"", z.mode)),
        ("listen/endpoints", arr(&z.listen)),
        ("connect/endpoints", arr(&z.connect)),
        ("scouting/multicast/enabled", z.multicast_scouting.to_string()),
        ("transport/unicast/qos/enabled", z.qos_enabled.to_string()),
        ("transport/unicast/lowlatency", z.lowlatency.to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k() -> Keys {
        Keys::new("ox")
    }

    #[test]
    fn 키는_규격_그대로다() {
        assert_eq!(k().node("n1", "i1"), "ox/int/node/n1/i1");
        assert_eq!(k().room("r1", "n1", "i1"), "ox/int/room/r1/n1/i1");
        assert_eq!(k().sess("u1", "n1", "s-1"), "ox/int/sess/u1/n1/s-1");
        assert_eq!(k().ev_room("r1"), "ox/int/ev/room/r1");
        assert_eq!(k().ev_user("u1"), "ox/int/ev/user/u1");
        assert_eq!(k().q_node("n1", "cut"), "ox/int/q/node/n1/cut");
        assert_eq!(k().q_all("cut"), "ox/int/q/node/*/cut");
    }

    #[test]
    fn 이름공간이_갈린다() {
        // ★한 버스에 여러 배포가 섞인다 — 최상위 조각이 그것을 가른다.
        assert_ne!(Keys::new("a").node("n", "i"), Keys::new("b").node("n", "i"));
    }

    #[test]
    fn 키를_되읽는다() {
        // ★값이 없으니 키가 곧 값이다.
        assert_eq!(node_of("ox/int/node/n1/i1"), Some(("n1", "i1")));
        assert_eq!(room_of("ox/int/room/r1/n1/i1"), Some(("r1", "n1", "i1")));
        assert_eq!(sess_of("ox/int/sess/u1/n1/s-9"), Some(("u1", "n1", "s-9")));
    }

    #[test]
    fn 모르는_모양은_안_읽는다() {
        // ★지어내면 없는 node 가 목록에 선다.
        assert_eq!(node_of("ox/int/room/r1/n1/i1"), None);
        assert_eq!(node_of("ox/int/node/n1"), None);
        assert_eq!(room_of("ox/int/sess/u1/n1/s-1"), None);
        assert_eq!(sess_of(""), None);
    }

    #[test]
    fn 키를_깨는_값은_거른다() {
        // ★설정값이 키를 깨면 그 node 는 아무에게도 안 보인다 — 조용히 붙는 것이 제일 나쁘다.
        assert!(is_key_segment("sfu-1"));
        assert!(!is_key_segment(""));
        for bad in ["a/b", "a*", "a?", "a#", "a$"] {
            assert!(!is_key_segment(bad), "{bad}");
        }
    }

    #[test]
    fn 성립_조건_둘을_명시로_적는다() {
        // ★기본값이라 안 적으면 성능 튜닝 중에 꺼진다.
        let z = Zenoh::default();
        let got = config_json(&z);
        let find = |k: &str| got.iter().find(|(x, _)| *x == k).map(|(_, v)| v.clone());
        assert_eq!(find("transport/unicast/qos/enabled"), Some("true".into()));
        assert_eq!(find("transport/unicast/lowlatency"), Some("false".into()));
        assert_eq!(find("mode"), Some("\"router\"".into()));
    }
}
