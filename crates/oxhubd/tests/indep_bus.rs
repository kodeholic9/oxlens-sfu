// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PLACEMENT · 정:RULE-BUS-PRESENCE · 정:RULE-CFG-THREE-PLACES · model: claude-opus-5-5
//! 맹검 독립 1층 — 버스 키 공간(방 id 가 node 앞 · {inst} 가 키에 · int/ 아래 · 금지 문자) · 배치(FNV-1a 64 → splitmix64 · HRW · 후보 = node 토큰 전량 + 자기 node 처음부터 · 소멸은 grace 동안 남김).

use common::keyspace::{Keys, Parsed, valid_piece};
use oxhubd::bus::placement::{fnv1a64, pick, score, splitmix_finish};
use oxhubd::bus::view::{SETTLE_MS, TokenEv, View};
use std::collections::BTreeSet;
use zenoh::key_expr::keyexpr;

fn o_fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn o_mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn o_score(node: &str, room: &str) -> u64 {
    let mut b = node.as_bytes().to_vec();
    b.push(0);
    b.extend_from_slice(room.as_bytes());
    o_mix(o_fnv(&b))
}

fn o_pick(nodes: &[String], room: &str) -> Option<String> {
    let mut best: Option<(u64, &String)> = None;
    for n in nodes {
        let s = o_score(n, room);
        best = match best {
            None => Some((s, n)),
            Some((bs, bn)) if s > bs || (s == bs && n < bn) => Some((s, n)),
            keep => keep,
        };
    }
    best.map(|(_, n)| n.clone())
}

fn ke(s: &str) -> &keyexpr {
    keyexpr::new(s).unwrap_or_else(|e| panic!("not a zenoh key expression {s:?}: {e}"))
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn word(&mut self) -> String {
        let alpha = b"abcdefghijklmnopqrstuvwxyz0123456789-_";
        let n = 1 + (self.next() % 12) as usize;
        (0..n).map(|_| alpha[(self.next() % alpha.len() as u64) as usize] as char).collect()
    }
}

fn feed(v: &mut View, ev: TokenEv, at: u64) -> u64 {
    v.on_token(ev, at);
    let t = at + SETTLE_MS;
    if v.settle_due(t) {
        v.settle(t);
    }
    t
}

fn rooms(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("room-{i}")).collect()
}

#[test]
fn hash_is_fnv1a64_then_splitmix64() {
    assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325, "S-sym-RULE-BUS-PLACEMENT-4 FNV-1a 64 offset basis");
    assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c, "S-sym-RULE-BUS-PLACEMENT-4 FNV-1a 64 of \"a\"");
    assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8, "S-sym-RULE-BUS-PLACEMENT-4 FNV-1a 64 of \"foobar\"");
    assert_eq!(splitmix_finish(0), 0xe220_a839_7b1d_cdaf, "S-sym-RULE-BUS-PLACEMENT-4 SplitMix64 first output from state 0");
    let mut rng = Rng(0x0f1a_5eed_0000_0001);
    for _ in 0..5_000 {
        let z = rng.next();
        assert_eq!(splitmix_finish(z), o_mix(z), "S-sym-RULE-BUS-PLACEMENT-4 splitmix finish of {z:#x}");
        let node = rng.word();
        let room = rng.word();
        assert_eq!(score(&node, &room), o_score(&node, &room), "S-sym-RULE-BUS-PLACEMENT-4 score(node_id ‖ 0x00 ‖ room_id) {node} {room}");
    }
}

#[test]
fn pick_is_hrw_with_lexicographic_tie() {
    let mut rng = Rng(0x00b0_5eed_0000_0002);
    for _ in 0..2_000 {
        let n = 1 + (rng.next() % 6) as usize;
        let nodes: Vec<String> = (0..n).map(|_| rng.word()).collect::<BTreeSet<_>>().into_iter().collect();
        let room = rng.word();
        let got = pick(nodes.iter().map(|s| s.as_str()), &room);
        assert_eq!(got, o_pick(&nodes, &room), "S-sym-RULE-BUS-PLACEMENT-4 HRW max score · {nodes:?} · {room}");
        let mut rev = nodes.clone();
        rev.reverse();
        assert_eq!(pick(rev.iter().map(|s| s.as_str()), &room), got, "S-sym-RULE-BUS-PLACEMENT-1 pick does not depend on candidate order");
    }
    assert_eq!(pick(std::iter::empty::<&str>(), "r"), None, "S-sym-RULE-BUS-PLACEMENT-6 empty candidate set has no placement");
}

#[test]
fn own_node_is_a_candidate_before_its_echo() {
    let v = View::new("node-a", "hub-a-1", 10_000);
    let c = v.candidates(0);
    assert!(c.contains(&"node-a".to_string()), "S-sym-RULE-BUS-PLACEMENT-6 own node is a candidate from the start: {c:?}");
    for r in rooms(50) {
        assert_eq!(pick(c.iter().map(|s| s.as_str()), &r), Some("node-a".to_string()), "S-sym-RULE-BUS-PLACEMENT-6 single node places every room on itself before its echo");
    }

    let mut va = View::new("node-a", "hub-a-1", 10_000);
    let mut vb = View::new("node-b", "hub-b-1", 10_000);
    let t1 = feed(&mut va, TokenEv::NodePut { node: "node-b".into(), inst: "hub-b-1".into() }, 0);
    let t2 = feed(&mut vb, TokenEv::NodePut { node: "node-a".into(), inst: "hub-a-1".into() }, 0);
    let ca: BTreeSet<String> = va.candidates(t1).into_iter().collect();
    let cb: BTreeSet<String> = vb.candidates(t2).into_iter().collect();
    assert_eq!(ca, cb, "S-sym-RULE-BUS-PLACEMENT-6 two hubs without their own echo see the same candidate set");
    let want: BTreeSet<String> = ["node-a".to_string(), "node-b".to_string()].into_iter().collect();
    assert_eq!(ca, want, "S-sym-RULE-BUS-PLACEMENT-6 candidates = own node + node tokens seen");
    for r in rooms(300) {
        let pa = pick(ca.iter().map(|s| s.as_str()), &r);
        let pb = pick(cb.iter().map(|s| s.as_str()), &r);
        assert_eq!(pa, pb, "S-sym-RULE-BUS-PLACEMENT-6 both hubs place {r} on the same node");
    }

    let mut vs = View::new("node-a", "hub-a-1", 10_000);
    let t = feed(&mut vs, TokenEv::NodePut { node: "node-a".into(), inst: "hub-a-1".into() }, 0);
    let after_echo = vs.candidates(t);
    assert_eq!(after_echo.iter().filter(|n| *n == "node-a").count(), 1, "S-sym-RULE-BUS-PLACEMENT-6 own echo does not duplicate the own candidate: {after_echo:?}");
}

#[test]
fn vanished_node_stays_a_candidate_during_grace() {
    let grace = 10_000;
    let mut v = View::new("node-a", "hub-a-1", grace);
    let mut t = 0;
    for n in ["node-b", "node-c", "node-d"] {
        t = feed(&mut v, TokenEv::NodePut { node: n.into(), inst: format!("{n}-i1") }, t);
    }
    let before: BTreeSet<String> = v.candidates(t).into_iter().collect();
    let placed: Vec<Option<String>> = rooms(300).iter().map(|r| pick(before.iter().map(|s| s.as_str()), r)).collect();
    let del_at = t + 100;
    let t2 = feed(&mut v, TokenEv::NodeDel { node: "node-c".into(), inst: "node-c-i1".into() }, del_at);
    assert!(!v.node_live("node-c"), "S-sym-RULE-BUS-PLACEMENT-1 node-c is not live after its Delete");
    for now in [t2, del_at + grace / 2, del_at + grace - 1] {
        let c: BTreeSet<String> = v.candidates(now).into_iter().collect();
        assert_eq!(c, before, "S-sym-RULE-BUS-PLACEMENT-3 vanished node kept in candidates during grace · now {now}");
        let again: Vec<Option<String>> = rooms(300).iter().map(|r| pick(c.iter().map(|s| s.as_str()), r)).collect();
        assert_eq!(again, placed, "S-sym-RULE-BUS-PLACEMENT-1 placement does not move while a node is merely not live · now {now}");
    }
    let late = del_at + grace + SETTLE_MS + 1;
    if v.settle_due(late) {
        v.settle(late);
    }
    let c = v.candidates(late);
    assert!(!c.contains(&"node-c".to_string()), "S-sym-RULE-BUS-PLACEMENT-3 vanished node leaves after grace: {c:?}");
    assert!(c.contains(&"node-a".to_string()) && c.contains(&"node-b".to_string()), "S-sym-RULE-BUS-PLACEMENT-3 others stay: {c:?}");
}

#[test]
fn blip_and_restart_do_not_move_rooms() {
    let mut v = View::new("node-a", "hub-a-1", 10_000);
    let mut t = 0;
    for n in ["node-b", "node-c"] {
        t = feed(&mut v, TokenEv::NodePut { node: n.into(), inst: format!("{n}-i1") }, t);
    }
    let before: BTreeSet<String> = v.candidates(t).into_iter().collect();
    let placed: Vec<Option<String>> = rooms(300).iter().map(|r| pick(before.iter().map(|s| s.as_str()), r)).collect();
    t = feed(&mut v, TokenEv::RoomDel { room: "room-7".into(), node: "node-b".into(), inst: "sfu-b-1".into() }, t);
    t = feed(&mut v, TokenEv::NodeDel { node: "node-b".into(), inst: "node-b-i1".into() }, t);
    t = feed(&mut v, TokenEv::NodePut { node: "node-b".into(), inst: "node-b-i2".into() }, t);
    let after: BTreeSet<String> = v.candidates(t).into_iter().collect();
    assert_eq!(after, before, "S-sym-RULE-BUS-PLACEMENT-4 restart with a new inst keeps the same node_id candidate set");
    let again: Vec<Option<String>> = rooms(300).iter().map(|r| pick(after.iter().map(|s| s.as_str()), r)).collect();
    assert_eq!(again, placed, "S-sym-RULE-BUS-PLACEMENT-2 Delete blips and restarts do not re-place rooms");
    assert!(v.node_live("node-b"), "S-sym-RULE-BUS-PLACEMENT-4 node-b live again under its new inst");
}

#[test]
fn duplicate_room_holders_are_counted() {
    let mut v = View::new("node-a", "hub-a-1", 10_000);
    let mut t = 0;
    for n in ["node-b", "node-c"] {
        t = feed(&mut v, TokenEv::NodePut { node: n.into(), inst: format!("{n}-i1") }, t);
    }
    let before = v.duplicates;
    t = feed(&mut v, TokenEv::RoomPut { room: "dup".into(), node: "node-b".into(), inst: "sfu-b".into() }, t);
    feed(&mut v, TokenEv::RoomPut { room: "dup".into(), node: "node-c".into(), inst: "sfu-c".into() }, t);
    assert!(v.duplicates > before, "S-sym-RULE-BUS-PLACEMENT-5 a room held by two nodes is counted ({} → {})", before, v.duplicates);
}

#[test]
fn key_layout_is_the_spec_layout() {
    let k = Keys::new("prod");
    assert_eq!(k.node_token("n1", "h7"), "ox/prod/int/node/n1/h7", "S-sym-RULE-BUS-KEYSPACE-1 node token carries node and inst");
    assert_eq!(k.room_token("R9", "n1", "s3"), "ox/prod/int/room/R9/n1/s3", "S-sym-RULE-BUS-KEYSPACE-2 room id before node in the room token");
    assert_eq!(k.sess_token("u5", "n1", "sid2"), "ox/prod/int/sess/u5/n1/sid2", "S-sym-RULE-BUS-KEYSPACE-3 session token under int/");
    assert_eq!(k.ev_room("R9"), "ox/prod/int/ev/room/R9", "S-sym-RULE-BUS-KEYSPACE-3 room event under int/");
    assert_eq!(k.ev_user("u5"), "ox/prod/int/ev/user/u5", "S-sym-RULE-BUS-KEYSPACE-3 user event under int/");
    assert_eq!(k.ev_node("n1"), "ox/prod/int/ev/node/n1", "S-sym-RULE-BUS-KEYSPACE-3 node event under int/");
    assert!(k.q_node("n1", "x").starts_with("ox/prod/int/q/node/n1/"), "S-sym-RULE-BUS-KEYSPACE-3 node query under int/q/node/{{node}}/");
}

#[test]
fn room_lookup_is_one_wildcard_chunk() {
    let mut rng = Rng(0x6b65_7973_7061_6365);
    let k = Keys::new("dep1");
    for _ in 0..500 {
        let (r, r2, n, n2, i) = (rng.word(), rng.word(), rng.word(), rng.word(), rng.word());
        let tok = k.room_token(&r, &n, &i);
        let chunks: Vec<&str> = tok.split('/').collect();
        assert_eq!(chunks.get(4).copied(), Some(r.as_str()), "S-sym-RULE-BUS-KEYSPACE-2 room id is the chunk right after room/ · {tok}");
        assert_eq!(chunks.get(5).copied(), Some(n.as_str()), "S-sym-RULE-BUS-KEYSPACE-2 node follows the room id · {tok}");
        let w = k.room_where(&r);
        assert!(ke(&w).includes(ke(&tok)), "S-sym-RULE-BUS-KEYSPACE-2 room_where({r}) covers its token {tok} · {w}");
        let wc: Vec<&str> = w.split('/').collect();
        assert_eq!(wc.get(4).copied(), Some(r.as_str()), "S-sym-RULE-BUS-KEYSPACE-2 \"where is this room\" names the room literally, no scan over nodes · {w}");
        assert!(!wc[..5].iter().any(|c| c.contains('*')), "S-sym-RULE-BUS-KEYSPACE-2 no wildcard before the room id · {w}");
        if r2 != r {
            assert!(!ke(&w).intersects(ke(&k.room_token(&r2, &n, &i))), "S-sym-RULE-BUS-KEYSPACE-2 room_where({r}) does not match room {r2}");
        }
        let by_node = k.rooms_of_node(&n);
        assert!(ke(&by_node).includes(ke(&tok)), "S-sym-RULE-BUS-KEYSPACE-2 rooms_of_node({n}) covers {tok} · {by_node}");
        let bc: Vec<&str> = by_node.split('/').collect();
        assert_eq!((bc.get(4).copied(), bc.get(5).copied()), (Some("*"), Some(n.as_str())), "S-sym-RULE-BUS-KEYSPACE-2 rooms of a node = one * chunk then the node · {by_node}");
        if n2 != n {
            assert!(!ke(&by_node).intersects(ke(&k.room_token(&r, &n2, &i))), "S-sym-RULE-BUS-KEYSPACE-2 rooms_of_node({n}) does not match node {n2}");
        }
        assert!(ke(&k.room_all()).includes(ke(&tok)), "S-sym-RULE-BUS-KEYSPACE-2 room_all covers every room token");
    }
}

#[test]
fn inst_separates_restarts() {
    let k = Keys::new("ox1");
    let a = k.room_token("R", "n", "inst-1");
    let b = k.room_token("R", "n", "inst-2");
    assert_ne!(a, b, "S-sym-RULE-BUS-KEYSPACE-1 restart inst changes the room token key");
    assert_eq!(k.parse(&a), Some(Parsed::Room { room: "R".into(), node: "n".into(), inst: "inst-1".into() }), "S-sym-RULE-BUS-KEYSPACE-1 room token parses back with its inst");
    assert_eq!(k.parse(&b), Some(Parsed::Room { room: "R".into(), node: "n".into(), inst: "inst-2".into() }), "S-sym-RULE-BUS-KEYSPACE-1 new inst parses back");
    let na = k.node_token("n", "hub-1");
    let nb = k.node_token("n", "hub-2");
    assert_ne!(na, nb, "S-sym-RULE-BUS-KEYSPACE-1 node token key differs per inst");
    assert_eq!(k.parse(&na), Some(Parsed::Node { node: "n".into(), inst: "hub-1".into() }), "S-sym-RULE-BUS-KEYSPACE-1 node token parses back");
    assert_eq!(k.parse(&k.sess_token("u", "n", "s")), Some(Parsed::Sess { user: "u".into(), node: "n".into(), sid: "s".into() }), "S-sym-RULE-BUS-KEYSPACE-3 sess token parses back");
    assert!(ke(&k.node_all()).includes(ke(&na)), "S-sym-RULE-BUS-KEYSPACE-1 node_all covers node tokens");
    assert!(!ke(&k.node_all()).intersects(ke(&a)), "S-sym-RULE-BUS-KEYSPACE-3 node_all does not catch room tokens");
    assert!(!ke(&k.room_all()).intersects(ke(&na)), "S-sym-RULE-BUS-KEYSPACE-3 room_all does not catch node tokens");
    let other = Keys::new("ox2");
    assert!(!ke(&k.room_all()).intersects(ke(&other.room_token("R", "n", "inst-1"))), "S-sym-RULE-BUS-KEYSPACE-6 deployment namespace separates deployments");
    assert_eq!(k.parse(&other.room_token("R", "n", "i")), None, "S-sym-RULE-BUS-KEYSPACE-6 another namespace is not ours");
}

#[test]
fn forbidden_pieces_are_rejected() {
    for bad in ["", "a*b", "a$b", "a?b", "a#b", "a/b", "*", "**", "/", "$*"] {
        assert!(!valid_piece(bad), "S-sym-RULE-BUS-KEYSPACE-4 {bad:?} cannot be a key piece");
    }
    for good in ["room-1", "회의실", "a.b_c", "R9", "0"] {
        assert!(valid_piece(good), "S-sym-RULE-BUS-KEYSPACE-4 {good:?} is a valid key piece");
        let k = Keys::new("ox");
        assert!(keyexpr::new(k.room_token(good, "n", "i").as_str()).is_ok(), "S-sym-RULE-BUS-KEYSPACE-4 a valid room id makes a valid zenoh key");
    }
}

fn system_text(ns: &str) -> String {
    let base = include_str!("../../../system.toml");
    let mut out = String::new();
    for line in base.lines() {
        if line.trim_start().starts_with("namespace") {
            out.push_str(&format!("namespace = {ns:?}\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn unusable_namespace_fails_boot() {
    let ok = common::system::System::parse(&system_text("ox")).and_then(|s| s.validate().map(|_| s));
    assert!(ok.is_ok(), "S-sym-RULE-BUS-KEYSPACE-6 control: a plain namespace loads · {:?}", ok.err());
    for bad in ["", "a/b", "a*", "x$y", "q?", "h#"] {
        let r = common::system::System::parse(&system_text(bad)).and_then(|s| s.validate().map(|_| s));
        assert!(r.is_err(), "S-sym-RULE-BUS-KEYSPACE-6 namespace {bad:?} must fail at boot");
    }
}

fn repo_file(name: &str) -> String {
    format!("{}/../../{name}", env!("CARGO_MANIFEST_DIR"))
}

fn hub_args(id: &str) -> Vec<String> {
    ["--system", &repo_file("system.toml"), "--policy", &repo_file("policy.toml"), "--id", id].iter().map(|s| s.to_string()).collect()
}

#[test]
fn unusable_node_id_fails_hub_boot() {
    let ok = oxhubd::cfg::HubCfg::load(hub_args("node-a"));
    assert!(ok.is_ok(), "S-sym-RULE-BUS-KEYSPACE-6 control: hub loads with a plain node id · {:?}", ok.err());
    for bad in ["", "n/1", "n*", "n$", "n?", "n#"] {
        let r = oxhubd::cfg::HubCfg::load(hub_args(bad));
        assert!(r.is_err(), "S-sym-RULE-BUS-KEYSPACE-6 hub with node id {bad:?} must fail at boot");
    }
}
