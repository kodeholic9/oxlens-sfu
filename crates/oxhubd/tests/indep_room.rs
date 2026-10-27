// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-EVENTS · 정:RULE-BUS-PRESENCE · 정:IMPL-BUS-FAULTS · model: claude-opus-5-5
//! 독립 1층 시험(B′ 맹검) — 방 맵: Delete 는 가라앉힌 뒤 몰아 판정 · 고립이면 거두지 않음 · 한 대 형상은 고립 아님 · 자기 node 방 소멸은 평범 · 관심 참조계수.

use common::bus::keys::{KeySpace, NodeToken, RoomToken};
use oxhubd::room::{Interest, RoomMap, TokenEv};

const SETTLE: u64 = 1000;

struct M {
    k: KeySpace,
    m: RoomMap,
}

impl M {
    fn new(me: &str) -> M {
        M { k: KeySpace::new("t"), m: RoomMap::new(me) }
    }
    fn node(&mut self, n: &str, put: bool, t: u64) {
        let tok = NodeToken { node: n.into(), inst: format!("h-{n}") };
        let key = self.k.node(n, &tok.inst);
        self.m.on_event(if put { TokenEv::NodePut(tok) } else { TokenEv::NodeDel(tok) }, &key, t, t);
    }
    fn room(&mut self, r: &str, n: &str, put: bool, t: u64) {
        let tok = RoomToken { room: r.into(), node: n.into(), inst: format!("s-{n}") };
        let key = self.k.room(r, n, &tok.inst);
        self.m.on_event(if put { TokenEv::RoomPut(tok) } else { TokenEv::RoomDel(tok) }, &key, t, t);
    }
    fn settle(&mut self, t: u64) -> (Vec<String>, bool) {
        let mut out = (vec![], false);
        if self.m.settle_due(t) {
            let s = self.m.settle(t, t);
            out = (s.removed, s.isolated);
        }
        out.0.sort();
        out
    }
}

fn three_nodes() -> M {
    let mut m = M::new("a");
    m.node("a", true, 0);
    m.node("b", true, 0);
    m.node("c", true, 0);
    m.room("Ra", "a", true, 0);
    m.room("Rb1", "b", true, 0);
    m.room("Rb2", "b", true, 0);
    m.room("Rc", "c", true, 0);
    m.settle(5000);
    m
}

#[test]
fn delete_is_not_judged_on_arrival() {
    let mut m = three_nodes();
    m.room("Rb1", "b", false, 10_000);
    assert!(m.m.locate("Rb1").is_some(), "S-sym-RULE-BUS-EVENTS-7 방 토큰 Delete 는 받은 자리에서 안 지운다");
    assert!(!m.m.settle_due(10_000 + SETTLE - 1), "S-sym-RULE-BUS-EVENTS-7 가라앉기 1,000ms 전 판정 0");
    let (removed, iso) = m.settle(10_000 + SETTLE);
    assert_eq!((removed, iso), (vec!["Rb1".to_string()], false), "S-sym-RULE-BUS-EVENTS-7 가라앉은 뒤 평범한 방 소멸");
    assert!(m.m.locate("Rb1").is_none(), "S-sym-RULE-BUS-EVENTS-7 거둔 뒤 위치 없음");
}

#[test]
fn quiet_window_restarts_on_each_token_event() {
    let mut m = three_nodes();
    m.room("Rb1", "b", false, 10_000);
    m.room("Rc", "c", false, 10_900);
    assert!(!m.m.settle_due(11_000), "S-sym-RULE-BUS-EVENTS-7 마지막 사건부터 가라앉기를 센다");
    assert!(m.m.settle_due(11_900), "S-sym-RULE-BUS-EVENTS-7 마지막 사건 + 1,000ms");
}

#[test]
fn isolation_in_any_order_keeps_routing() {
    let orders: [[(&str, &str, bool); 5]; 3] = [
        [("n", "b", false), ("n", "c", false), ("r", "Rb1", false), ("r", "Rb2", false), ("r", "Rc", false)],
        [("r", "Rb1", false), ("r", "Rc", false), ("n", "b", false), ("r", "Rb2", false), ("n", "c", false)],
        [("r", "Rc", false), ("r", "Rb2", false), ("r", "Rb1", false), ("n", "c", false), ("n", "b", false)],
    ];
    for (i, ord) in orders.iter().enumerate() {
        let mut m = three_nodes();
        let mut t = 10_000;
        for (kind, name, put) in ord {
            t += 37;
            if *kind == "n" {
                m.node(name, *put, t);
            } else {
                let n = if name.starts_with("Rb") { "b" } else { "c" };
                m.room(name, n, *put, t);
            }
        }
        let (removed, iso) = m.settle(t + SETTLE);
        assert!(iso, "S-sym-RULE-BUS-EVENTS-7 타 node 토큰 일괄 소멸 = 고립 (차례 {i})");
        assert!(removed.is_empty(), "S-sym-RULE-BUS-EVENTS-7 고립이면 라우팅을 안 거둔다 — Delete 차례 없이 와도 (차례 {i}) {removed:?}");
        assert!(m.m.locate("Rb1").is_some() && m.m.locate("Rc").is_some(), "S-sym-RULE-BUS-EVENTS-7 산 방 위치 유지 (차례 {i})");
        assert!(m.m.isolated().is_some(), "S-sym-RULE-BUS-PRESENCE-5 고립 상태가 보인다");
    }
}

#[test]
fn own_room_gone_while_isolated_is_plain_removal() {
    let mut m = three_nodes();
    m.node("b", false, 10_000);
    m.node("c", false, 10_010);
    m.room("Rb1", "b", false, 10_020);
    m.room("Rb2", "b", false, 10_030);
    m.room("Rc", "c", false, 10_040);
    m.room("Ra", "a", false, 10_050);
    let (removed, iso) = m.settle(10_050 + SETTLE);
    assert!(iso, "시험 형상 — 고립");
    assert_eq!(removed, vec!["Ra".to_string()], "S-sym-RULE-BUS-PRESENCE-5 고립 중에도 자기 node 의 방 소멸은 빠진다");
}

#[test]
fn partial_loss_is_not_isolation() {
    let mut m = three_nodes();
    m.node("b", false, 10_000);
    m.room("Rb2", "b", false, 10_010);
    m.room("Rb1", "b", false, 10_020);
    let (removed, iso) = m.settle(10_020 + SETTLE);
    assert!(!iso, "S-sym-RULE-BUS-PRESENCE-5 남이 하나 남으면 고립 아님");
    assert_eq!(removed, vec!["Rb1".to_string(), "Rb2".to_string()], "S-sym-RULE-BUS-PRESENCE-5 양성 대조 — 고립 아닐 때 빠진다");
    assert!(m.m.locate("Rc").is_some(), "S-sym-RULE-BUS-PRESENCE-5 남은 node 방 그대로");
}

#[test]
fn solo_node_is_never_isolated() {
    let mut m = M::new("a");
    m.node("a", true, 0);
    m.room("R1", "a", true, 0);
    m.room("R2", "a", true, 10);
    m.settle(5000);
    m.room("R1", "a", false, 6000);
    let (removed, iso) = m.settle(6000 + SETTLE);
    assert!(!iso, "S-sym-RULE-BUS-PRESENCE-5 한 대 형상(남을 본 적 없음)은 고립이 아니다");
    assert_eq!(removed, vec!["R1".to_string()], "S-sym-RULE-BUS-PRESENCE-5 자기 방 소멸은 평범하게 빠진다");
    assert!(m.m.isolated().is_none(), "S-sym-RULE-BUS-PRESENCE-5 고립 표시 0");
}

#[test]
fn reappearing_room_within_settle_is_kept() {
    let mut m = three_nodes();
    m.room("Rb1", "b", false, 10_000);
    m.room("Rb1", "b", true, 10_200);
    let (removed, _) = m.settle(10_200 + SETTLE);
    assert!(removed.is_empty() && m.m.locate("Rb1").is_some(), "S-sym-RULE-BUS-EVENTS-7 가라앉기 안에 다시 선 방은 거두지 않는다");
}

#[test]
fn duplicate_room_on_two_nodes_is_counted() {
    let mut m = three_nodes();
    let before = m.m.duplicates;
    m.room("Rc", "b", true, 20_000);
    m.settle(20_000 + SETTLE);
    assert!(m.m.duplicates > before, "S-sym-RULE-BUS-PLACEMENT-5 room/{{R}}/* 에 node 둘 = 탐지 · 계수");
}

#[test]
fn interest_counts_in_flight_join() {
    let mut i = Interest::default();
    assert!(i.inc("R"), "S-sym-RULE-BUS-EVENTS-9 첫 참조에 관심 선언");
    assert!(!i.inc("R"), "S-sym-RULE-BUS-EVENTS-9 둘째(진행 중 입장)는 선언 안 늘린다");
    assert!(!i.dec("R"), "S-sym-RULE-BUS-EVENTS-9 마지막 멤버가 나가도 진행 중 입장이 남아 관심 유지");
    assert!(i.held("R"), "S-sym-RULE-BUS-EVENTS-9 관심이 안 걷혔다");
    assert!(i.dec("R"), "S-sym-RULE-BUS-EVENTS-9 마지막 몫을 놓으면 해제");
    assert!(!i.held("R"), "S-sym-RULE-BUS-EVENTS-9 해제 뒤 없음");
}
