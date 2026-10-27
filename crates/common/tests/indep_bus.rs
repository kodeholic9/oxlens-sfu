// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PLACEMENT · model: claude-opus-5-5
//! 독립 1층 시험(B′ 맹검) — 키 공간 순서 · 금지 문자 · 기동 신원 · 배치 해시(FNV-1a 64 → splitmix64) · HRW · 후보 집합(자기 node · grace).

use common::bus::keys::{check_fragment, fragment_ok, KeySpace};
use common::bus::place::{hash, pick, Candidates};

fn o_hash(node: &str, room: &str) -> u64 {
    let mut z: u64 = 0xcbf2_9ce4_8422_2325;
    for b in node.as_bytes().iter().chain([0u8].iter()).chain(room.as_bytes()) {
        z ^= *b as u64;
        z = z.wrapping_mul(0x0000_0100_0000_01b3);
    }
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z ^= z >> 30;
    z = z.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z ^= z >> 27;
    z = z.wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    z
}

fn o_pick(nodes: &[&str], room: &str) -> Option<String> {
    let mut best: Option<(u64, &str)> = None;
    for n in nodes {
        let h = o_hash(n, room);
        best = match best {
            None => Some((h, n)),
            Some((bh, bn)) if h > bh || (h == bh && *n < bn) => Some((h, n)),
            b => b,
        };
    }
    best.map(|(_, n)| n.to_string())
}

#[test]
fn placement_hash_is_the_contract() {
    let rooms = ["", "r1", "room-한글", "a/b-not-allowed-but-hashable", "x".repeat(300).leak()];
    let nodes = ["n1", "node-a", "", "ノード"];
    for r in rooms {
        for n in nodes {
            assert_eq!(hash(n, r), o_hash(n, r), "S-sym-RULE-BUS-PLACEMENT-1 해시 = FNV-1a 64 ‖ 0x00 ‖ → splitmix64 ({n:?},{r:?})");
        }
    }
    assert_ne!(hash("ab", "c"), hash("a", "bc"), "S-sym-RULE-BUS-PLACEMENT-1 0x00 구분자가 경계를 가른다");
}

#[test]
fn hrw_pick_matches_oracle_and_moves_one_over_n() {
    let nodes = ["n1", "n2", "n3", "n4"];
    let mut moved = 0;
    for i in 0..2000 {
        let room = format!("room-{i}");
        let a = pick(nodes.iter().copied(), &room);
        assert_eq!(a, o_pick(&nodes, &room), "S-sym-RULE-BUS-PLACEMENT-1 HRW 최대 · 동점 사전순 — {room}");
        let rev: Vec<&str> = nodes.iter().rev().copied().collect();
        assert_eq!(pick(rev.iter().copied(), &room), a, "S-sym-RULE-BUS-PLACEMENT-1 후보 순서 무관(순수 함수)");
        let b = pick(nodes[..3].iter().copied(), &room);
        if a.as_deref() != Some("n4") {
            assert_eq!(b, a, "S-sym-RULE-BUS-PLACEMENT-1 노드 하나 빠져도 그 노드 밖 방은 안 옮는다 — {room}");
        } else {
            moved += 1;
        }
    }
    assert!((300..700).contains(&moved), "S-sym-RULE-BUS-PLACEMENT-1 이동 ≈ 1/N — {moved}/2000");
    assert_eq!(pick(std::iter::empty::<&str>(), "r"), None, "S-sym-RULE-BUS-PLACEMENT-6 후보 0 이면 없음");
}

#[test]
fn candidates_include_self_before_own_token_echo() {
    let c = Candidates::new("me");
    assert!(c.set().iter().any(|n| n == "me"), "S-sym-RULE-BUS-PLACEMENT-6 후보에 자기 node 가 처음부터 든다");
    assert_eq!(c.place("any-room").as_deref(), Some("me"), "S-sym-RULE-BUS-PLACEMENT-6 한 대 형상이면 배치가 선다 — SFU_UNAVAILABLE 0");
    let mut c = Candidates::new("me");
    c.token_put("other");
    let both = ["me", "other"];
    for i in 0..200 {
        let r = format!("r{i}");
        assert_eq!(c.place(&r), o_pick(&both, &r), "S-sym-RULE-BUS-PLACEMENT-6 두 hub 가 같은 후보로 계산 — {r}");
    }
}

#[test]
fn deleted_node_stays_through_grace() {
    let mut c = Candidates::new("me");
    c.token_put("b");
    let room = (0..500).map(|i| format!("r{i}")).find(|r| c.place(r).as_deref() == Some("b")).expect("b 로 가는 방");
    c.token_delete("b", 1000);
    assert!(c.set().iter().any(|n| n == "b"), "S-sym-RULE-BUS-PLACEMENT-1 소멸은 grace 동안 집합에 남긴다");
    assert_eq!(c.place(&room).as_deref(), Some("b"), "S-sym-RULE-BUS-PLACEMENT-1 죽는 창에 방이 옮겨 붙지 않는다");
    assert!(!c.live("b"), "S-sym-RULE-BUS-PLACEMENT-1 산 것 판정은 따로(배치에 안 쓴다)");
    let gone = c.expire(1000 + 5_000, 10_000);
    assert!(gone.is_empty() && c.set().iter().any(|n| n == "b"), "S-sym-RULE-BUS-PLACEMENT-3 grace 안 만료 0");
    let gone = c.expire(1000 + 10_001, 10_000);
    assert_eq!(gone, vec!["b".to_string()], "S-sym-RULE-BUS-PLACEMENT-3 grace 뒤 집합에서 뺀다");
    c.token_put("b");
    assert!(c.live("b"), "S-sym-RULE-BUS-PLACEMENT-2 다시 선언되면 산다");
    let mut c = Candidates::new("me");
    c.token_put("b");
    c.token_delete("b", 0);
    c.token_put("b");
    assert!(c.expire(100_000, 10_000).is_empty(), "S-sym-RULE-BUS-PLACEMENT-2 grace 안 재선언이면 소멸 취소");
}

#[test]
fn keyspace_room_before_node_and_inst_in_key() {
    let k = KeySpace::new("dep1");
    let key = k.room("R1", "N1", "I1");
    assert_eq!(key, "ox/dep1/int/room/R1/N1/I1", "S-sym-RULE-BUS-KEYSPACE-2 room/{{R}}/{{node}}/{{inst}}");
    let w = k.room_where("R1");
    assert!(w.starts_with("ox/dep1/int/room/R1/"), "S-sym-RULE-BUS-KEYSPACE-2 이 방 어디 = 방 id 고정 — {w}");
    let of = k.rooms_of_node("N1");
    let segs: Vec<&str> = of.split('/').collect();
    assert_eq!(&segs[..6], &["ox", "dep1", "int", "room", "*", "N1"], "S-sym-RULE-BUS-KEYSPACE-2 그 node 의 방 전부 = * 한 칸 — {of}");
    assert_ne!(k.room("R1", "N1", "I1"), k.room("R1", "N1", "I2"), "S-sym-RULE-BUS-KEYSPACE-1 {{inst}} 가 키를 가른다");
    let t = k.parse_room(&key).expect("parse_room");
    assert_eq!((t.room.as_str(), t.node.as_str(), t.inst.as_str()), ("R1", "N1", "I1"), "S-sym-RULE-BUS-KEYSPACE-2 왕복");
    assert!(k.parse_room("ox/dep2/int/room/R1/N1/I1").is_none(), "S-sym-RULE-BUS-KEYSPACE-2 다른 이름공간 키는 내 것 아님");
    assert_eq!(k.node("N1", "H1"), "ox/dep1/int/node/N1/H1", "S-sym-RULE-BUS-KEYSPACE-5 node/{{node}}/{{inst}}");
    let n = k.parse_node(&k.node("N1", "H1")).expect("parse_node");
    assert_eq!((n.node.as_str(), n.inst.as_str()), ("N1", "H1"), "S-sym-RULE-BUS-KEYSPACE-5 왕복");
    assert_eq!(k.sess("U", "N1", "S1"), "ox/dep1/int/sess/U/N1/S1", "S-sym-RULE-BUS-KEYSPACE-2 sess 키");
    assert_eq!(k.ev_room("R1"), "ox/dep1/int/ev/room/R1", "S-sym-RULE-BUS-KEYSPACE-3 내부 키는 int/ 아래");
    assert!(k.q_node("N1", "x").starts_with("ox/dep1/int/q/node/N1/"), "S-sym-RULE-BUS-KEYSPACE-3 q/node/{{node}}/**");
}

#[test]
fn forbidden_characters_and_empty_rejected() {
    for bad in ["a/b", "a*", "$x", "q?", "#h", "", "/"] {
        assert!(check_fragment("room_id", bad).is_err(), "S-sym-RULE-BUS-KEYSPACE-4 금지 문자 · 빈 조각 거절 — {bad:?}");
        assert!(!fragment_ok(bad), "S-sym-RULE-BUS-KEYSPACE-6 금지 값 — {bad:?}");
    }
    for good in ["room-1", "방_이름", "a.b-c", "UPPER", "x".repeat(200).as_str()] {
        assert!(check_fragment("room_id", good).is_ok(), "S-sym-RULE-BUS-KEYSPACE-4 양성 대조 — {good:?}");
    }
}
