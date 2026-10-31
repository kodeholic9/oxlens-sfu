// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · model: claude-opus-5-5
//! 1층 — wire 왕복 · 조각 이어 붙이기 · 키 식 겹침 · 포함(202610/20261031e 단계 B).

use oxmesh::keyexpr::{includes, intersects, valid};
use oxmesh::wire::{Assembled, CloseReason, DeclKind, FRAME_MAX, Hello, Lane, Msg, Reassembler, Role, TokenEvtKind};

fn all() -> Vec<Msg> {
    vec![
        Msg::Hello(Hello {
            proto: 1,
            role: Role::Router,
            node: "n1".into(),
            boot: 1_730_000_000_000_000,
            sno: 7,
            lease_ms: 10_000,
            listen: vec!["127.0.0.1:7447".into()],
            peers: vec![("n2".into(), "10.0.0.2:7447".into())],
        }),
        Msg::Close(CloseReason::Replaced),
        Msg::KeepAlive,
        Msg::Declare { kind: DeclKind::Qabl, id: u64::MAX, expr: "ox/c/int/q/node/n1/**".into() },
        Msg::Undeclare { kind: DeclKind::Sub, id: 300 },
        Msg::DeclareDone,
        Msg::TokenSub { id: 9, expr: "ox/c/int/node/*/*".into(), history: true },
        Msg::TokenUnsub { id: 9 },
        Msg::TokenEvt { id: 9, kind: TokenEvtKind::BatchEnd, key: String::new() },
        Msg::Peers(vec![]),
        Msg::Put { key: "ox/c/int/ev/room/r1".into(), payload: vec![0, 1, 2, 255] },
        Msg::Request { qid: 1 << 40, expr: "ox/c/int/q/node/*/call".into(), payload: b"{}".to_vec(), timeout_ms: 10_000 },
        Msg::Reply { qid: 3, key: "ox/c/int/q/node/n2/call".into(), ok: false, payload: vec![] },
        Msg::Final { qid: 3 },
    ]
}

#[test]
fn every_message_round_trips() {
    for m in all() {
        assert_eq!(Msg::decode(&m.encode()), Some(m.clone()), "{m:?}");
    }
}

#[test]
fn truncated_or_trailing_bytes_are_rejected() {
    for m in all() {
        let b = m.encode();
        if b.len() > 1 {
            assert_eq!(Msg::decode(&b[..b.len() - 1]), None, "{m:?}");
        }
        let mut long = b.clone();
        long.push(0);
        assert_eq!(Msg::decode(&long), None, "{m:?}");
    }
}

#[test]
fn lanes_follow_the_four_kinds() {
    assert_eq!(Msg::KeepAlive.lane(), Lane::Control);
    assert_eq!(Msg::DeclareDone.lane(), Lane::Declare);
    assert_eq!(Msg::Final { qid: 1 }.lane(), Lane::Request);
    assert_eq!(Msg::Put { key: "a".into(), payload: vec![] }.lane(), Lane::Data);
}

#[test]
fn big_message_is_cut_under_frame_max_and_reassembled() {
    let m = Msg::Reply { qid: 1, key: "k".into(), ok: true, payload: (0..200_000u32).map(|i| i as u8).collect() };
    let (lane, frames) = m.frames();
    assert_eq!(lane, Lane::Request);
    assert!(frames.len() > 10);
    let mut asm = Reassembler::default();
    let mut done = None;
    for (i, f) in frames.iter().enumerate() {
        assert!(f.len() <= FRAME_MAX);
        let n = u32::from_be_bytes([f[0], f[1], f[2], f[3]]) as usize;
        assert_eq!(n + 4, f.len());
        assert_eq!(f[4], Lane::Request as u8);
        match asm.push(lane, f[5], &f[6..]) {
            Assembled::Partial => assert!(i + 1 < frames.len()),
            Assembled::Done(b) => done = Some(b),
            Assembled::TooBig => panic!("too big"),
        }
    }
    assert_eq!(Msg::decode(&done.unwrap_or_default()), Some(m));
}

#[test]
fn lanes_reassemble_independently() {
    let big = Msg::Put { key: "k".into(), payload: vec![7; 40_000] };
    let small = Msg::KeepAlive;
    let (_, bf) = big.frames();
    let (_, sf) = small.frames();
    let mut asm = Reassembler::default();
    assert_eq!(asm.push(Lane::Data, bf[0][5], &bf[0][6..]), Assembled::Partial);
    assert_eq!(asm.push(Lane::Control, sf[0][5], &sf[0][6..]), Assembled::Done(small.encode()));
    let mut last = Assembled::Partial;
    for f in &bf[1..] {
        last = asm.push(Lane::Data, f[5], &f[6..]);
    }
    assert_eq!(last, Assembled::Done(big.encode()));
}

#[test]
fn key_expressions() {
    assert!(valid("ox/c/int/room/r1/n1/i1"));
    assert!(valid("ox/c/int/q/node/n1/**"));
    assert!(!valid("ox//room"));
    assert!(!valid("ox/r*/x"));
    assert!(!valid("ox/**/**"));
    assert!(!valid("ox/$x"));
    assert!(intersects("ox/c/int/room/*/*/*", "ox/c/int/room/r1/n1/i1"));
    assert!(!intersects("ox/c/int/room/*/*", "ox/c/int/room/r1/n1/i1"));
    assert!(intersects("ox/c/int/q/node/*/call", "ox/c/int/q/node/n1/**"));
    assert!(intersects("ox/c/int/sess/u1/**", "ox/c/int/sess/u1/n2/s9"));
    assert!(!intersects("ox/c/int/sess/u1/**", "ox/c/int/sess/u2/n2/s9"));
    assert!(intersects("ox/**", "ox"));
    assert!(intersects("a/**/c", "a/b/x/c"));
    assert!(!intersects("a/**/c", "a/b/x/d"));
    assert!(includes("ox/c/int/q/node/n1/**", "ox/c/int/q/node/n1/call"));
    assert!(!includes("ox/c/int/q/node/n1/call", "ox/c/int/q/node/n1/**"));
    assert!(includes("a/*/c", "a/b/c"));
    assert!(!includes("a/b/c", "a/*/c"));
    assert!(includes("a/**", "a/*/c"));
}
