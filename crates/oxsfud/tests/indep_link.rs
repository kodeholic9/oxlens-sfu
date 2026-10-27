// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-TRANSPORT · 정:RULE-LINK-PAIR-LIVENESS · 정:IMPL-LINK-ICE-RESTART · model: claude-opus-5-5
//! 독립 1층 시험(B′ 맹검) — STUN(RFC 5769 표본) · MI 검증 후에만 처리 · 지명만 경로를 옮긴다 · 쌍 생존 · ICE 재시작 장부 · RFC 4571 프레이밍.

use oxsfud::link::ice::{check, Creds, Ledger, Pairs, Verdict, Via, PWD_LEN, UDP_LIVENESS_MS, UFRAG_LEN};
use oxsfud::link::{stun, tcp};
use std::net::SocketAddr;

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap()).collect()
}

const RFC5769_REQ: &str = "000100582112a442b7e7a701bc34d686fa87dfae802200105354554e207465737420636c69656e74002400046e0001ff80290008932ff9b151263b36000600096576746a3a68367659202020000800149aeaa70cbfd8cb56781ef2b5b2d3f249c1b571a280280004e57a3bcf";

fn addr(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

fn creds(u: &str, p: &str) -> Creds {
    Creds { ufrag: u.into(), pwd: p.into() }
}

#[test]
fn rfc5769_sample_request_verifies() {
    let raw = hex(RFC5769_REQ);
    assert!(stun::is_stun(&raw), "S-sym-RULE-LINK-TRANSPORT-3 demux 첫 바이트 0x00-03 = STUN");
    let m = stun::parse(&raw).expect("RFC 5769 2.1 parse");
    assert_eq!(m.msg_type, stun::BINDING_REQUEST, "S-sym-RULE-LINK-TRANSPORT-3 Binding 요청");
    assert_eq!(m.username(), Some("evtj:h6vY"), "S-sym-RULE-LINK-TRANSPORT-3 USERNAME");
    assert_eq!(m.local_ufrag(), Some("evtj"), "S-sym-RULE-LINK-TRANSPORT-3 서버 ufrag = USERNAME 앞 칸");
    assert!(m.verify_integrity(b"VOkJxbRl1RmTxUk/WvJxBt"), "S-sym-RULE-LINK-TRANSPORT-3 RFC 5769 MI 검증");
    assert!(!m.verify_integrity(b"VOkJxbRl1RmTxUk/WvJxBu"), "S-sym-RULE-LINK-TRANSPORT-3 다른 키 MI 거절");
    assert!(m.verify_fingerprint(), "S-sym-RULE-LINK-TRANSPORT-3 RFC 5769 FINGERPRINT");
    assert!(!m.use_candidate(), "S-sym-RULE-LINK-TRANSPORT-5 USE-CANDIDATE 없음");
    let mut bad = raw.clone();
    bad[40] ^= 1;
    let mb = stun::parse(&bad).expect("훼손 parse");
    assert!(!mb.verify_integrity(b"VOkJxbRl1RmTxUk/WvJxBt"), "S-sym-RULE-LINK-TRANSPORT-3 바이트 훼손 → MI 거절");
    assert!(!mb.verify_fingerprint(), "S-sym-RULE-LINK-TRANSPORT-3 바이트 훼손 → FINGERPRINT 거절");
}

#[test]
fn built_messages_verify_and_carry_ox_software() {
    let key = b"0123456789abcdefghijkl";
    let req = stun::binding_request(&[7; 12], "srvufrag:cliufrag", key, true);
    let m = stun::parse(&req).expect("요청 parse");
    assert!(m.verify_integrity(key) && m.verify_fingerprint(), "S-sym-RULE-LINK-TRANSPORT-3 지은 요청 MI · FP");
    assert!(m.use_candidate(), "S-sym-RULE-LINK-TRANSPORT-5 USE-CANDIDATE 실림");
    let resp = stun::binding_success(&[9; 12], addr("192.0.2.1:3478"), key);
    let r = stun::parse(&resp).expect("응답 parse");
    assert_eq!(r.msg_type, stun::BINDING_SUCCESS, "S-sym-RULE-LINK-TRANSPORT-1 Binding 성공");
    assert_eq!(r.txid, [9; 12], "S-sym-RULE-LINK-TRANSPORT-1 txid 그대로");
    assert!(r.verify_integrity(key) && r.verify_fingerprint(), "S-sym-RULE-LINK-TRANSPORT-1 응답 MI · FP");
    let xma = r.attr(stun::ATTR_XOR_MAPPED_ADDRESS).expect("XOR-MAPPED-ADDRESS");
    let port = u16::from_be_bytes([xma[2], xma[3]]) ^ 0x2112;
    let ip: Vec<u8> = xma[4..8].iter().zip([0x21u8, 0x12, 0xa4, 0x42]).map(|(a, b)| a ^ b).collect();
    assert_eq!((port, ip), (3478, vec![192, 0, 2, 1]), "S-sym-RULE-LINK-TRANSPORT-1 XOR-MAPPED-ADDRESS = 보낸 주소");
    if let Some(sw) = r.attr(stun::ATTR_SOFTWARE) {
        assert!(sw.starts_with(b"ox"), "S-sym-RULE-LINK-TRANSPORT-1 SOFTWARE 는 ox 로 시작");
    }
}

fn req_for(ufrag: &str, pwd: &str, nominate: bool) -> stun::Message {
    stun::parse(&stun::binding_request(&[3; 12], &format!("{ufrag}:remote"), pwd.as_bytes(), nominate)).unwrap()
}

#[test]
fn forged_binding_cannot_move_path() {
    let c = creds("abcdefgh", "pppppppppppppppppppppp");
    let mut l = Ledger::new(c.clone());
    let mut p = Pairs::new();
    let a = Via::Udp(addr("10.0.0.1:5000"));
    match check(&mut l, &mut p, &req_for(&c.ufrag, &c.pwd, true), a, 0) {
        Verdict::Accepted { path_moved, .. } => assert!(path_moved, "S-sym-RULE-LINK-TRANSPORT-3 진짜 지명 → 경로"),
        v => panic!("S-sym-RULE-LINK-TRANSPORT-3 진짜 검사 거절 {v:?}"),
    }
    let evil = Via::Udp(addr("203.0.113.66:6666"));
    let forged = req_for(&c.ufrag, "qqqqqqqqqqqqqqqqqqqqqq", true);
    assert_eq!(check(&mut l, &mut p, &forged, evil, 10), Verdict::Unverified, "S-sym-RULE-LINK-TRANSPORT-3 위조 Binding 은 검증 실패");
    assert_eq!(p.path(), Some(a), "S-sym-RULE-LINK-TRANSPORT-3 위조는 latch 를 못 옮긴다 — 경로 이동 0");
    assert!(!p.knows(&evil), "S-sym-RULE-LINK-TRANSPORT-3 위조 주소는 쌍으로 안 선다");
    let unknown = req_for("zzzzzzzz", &c.pwd, true);
    assert_eq!(check(&mut l, &mut p, &unknown, evil, 20), Verdict::Unverified, "S-sym-RULE-LINK-TRANSPORT-3 모르는 ufrag 거절");
    assert_eq!(p.path(), Some(a), "S-sym-RULE-LINK-TRANSPORT-3 경로 그대로");
}

#[test]
fn only_nominated_check_moves_path_udp_or_tcp() {
    let c = creds("abcdefgh", "pppppppppppppppppppppp");
    let mut l = Ledger::new(c.clone());
    let mut p = Pairs::new();
    let u = Via::Udp(addr("10.0.0.1:5000"));
    let t = Via::Tcp(7, addr("10.0.0.1:443"));
    check(&mut l, &mut p, &req_for(&c.ufrag, &c.pwd, true), u, 0);
    match check(&mut l, &mut p, &req_for(&c.ufrag, &c.pwd, false), t, 5) {
        Verdict::Accepted { path_moved, .. } => assert!(!path_moved, "S-sym-RULE-LINK-TRANSPORT-5 지명 없는 TCP 백업 검사 → 경로 이동 0"),
        v => panic!("검증된 검사 거절 {v:?}"),
    }
    assert_eq!(p.path(), Some(u), "S-sym-RULE-LINK-TRANSPORT-5 경로 UDP 그대로");
    for k in 0..40u64 {
        check(&mut l, &mut p, &req_for(&c.ufrag, &c.pwd, false), t, 1000 * k);
    }
    assert_eq!(p.path(), Some(u), "S-sym-RULE-LINK-TRANSPORT-5 서버 침묵 판정으로 경로 전환 0");
    match check(&mut l, &mut p, &req_for(&c.ufrag, &c.pwd, true), t, 50_000) {
        Verdict::Accepted { path_moved, .. } => assert!(path_moved, "S-sym-RULE-LINK-TRANSPORT-5 지명을 TCP 로 옮기면 이동 1"),
        v => panic!("{v:?}"),
    }
    assert_eq!(p.path(), Some(t), "S-sym-RULE-LINK-TRANSPORT-5 마지막 지명 쌍");
    let mut p2 = Pairs::new();
    check(&mut l, &mut p2, &req_for(&c.ufrag, &c.pwd, false), u, 0);
    assert_eq!(p2.path(), None, "S-sym-RULE-LINK-TRANSPORT-5 지명 없는 검사 하나뿐이면 경로가 없다");
}

#[test]
fn udp_pair_expires_after_30s_tcp_lives_while_connected() {
    let mut p = Pairs::new();
    let u = Via::Udp(addr("10.0.0.1:5000"));
    let t = Via::Tcp(9, addr("10.0.0.1:443"));
    p.on_verified(u, true, 1000);
    p.on_verified(t, false, 1000);
    assert!(p.alive(&u, 1000 + UDP_LIVENESS_MS - 1), "SV-STATE-PEER-STATE UDP 30s 안은 생존");
    assert!(!p.alive(&u, 1000 + UDP_LIVENESS_MS + 1), "SV-STATE-PEER-STATE UDP 30s 넘으면 만료");
    assert!(!p.path_alive(1000 + UDP_LIVENESS_MS + 1), "S-sym-RULE-LINK-PAIR-LIVENESS-1 만료된 경로 = 산 경로 아님");
    assert!(p.alive(&t, 1000 + 10 * UDP_LIVENESS_MS), "SV-STATE-PEER-STATE TCP 쌍은 연결이 있으면 생존");
    assert!(!p.path_alive(1000 + 10 * UDP_LIVENESS_MS), "SV-STATE-PEER-STATE 경로가 UDP 인 채 TCP 쌍만 살면 경로는 죽음(Suspect 유지)");
    p.on_verified(u, false, 50_000);
    assert!(p.path_alive(50_000), "SV-STATE-PEER-STATE 검사가 오면 생존 갱신");
    p.tcp_closed(9);
    assert!(!p.alive(&t, 50_001), "SV-STATE-PEER-STATE TCP 연결이 닫히면 쌍 죽음");
}

#[test]
fn ice_restart_ledger_at_most_two_and_old_until_new_verified() {
    let c0 = creds("aaaaaaaa", "0000000000000000000000");
    let c1 = creds("bbbbbbbb", "1111111111111111111111");
    let c2 = creds("cccccccc", "2222222222222222222222");
    let c3 = creds("dddddddd", "3333333333333333333333");
    let mut l = Ledger::new(c0.clone());
    assert_eq!(l.restart(c1.clone()), None, "S-sym-IMPL-LINK-ICE-RESTART-2 첫 재시작 — 대기분 없음");
    assert_eq!(l.restart(c2.clone()), Some(c1.clone()), "S-sym-IMPL-LINK-ICE-RESTART-2 대기분을 새 것으로 바꾼다");
    assert_eq!(l.restart(c3.clone()), Some(c2.clone()), "S-sym-IMPL-LINK-ICE-RESTART-2 셋째도 대기분 교체");
    assert!(l.ufrags().len() <= 2, "S-sym-IMPL-LINK-ICE-RESTART-2 자격 벌마다 ufrag ≤ 2 — {:?}", l.ufrags());
    assert_eq!(l.pwd_for(&c1.ufrag), None, "S-sym-IMPL-LINK-ICE-RESTART-2 첫 응답 자격 거절");
    assert_eq!(l.pwd_for(&c2.ufrag), None, "S-sym-IMPL-LINK-ICE-RESTART-2 둘째 응답 자격 거절");
    assert_eq!(l.pwd_for(&c0.ufrag), Some(c0.pwd.as_str()), "S-sym-IMPL-LINK-ICE-RESTART-1 새 ufrag 검증 전 옛 ufrag 통과");
    assert_eq!(l.pwd_for(&c3.ufrag), Some(c3.pwd.as_str()), "S-sym-IMPL-LINK-ICE-RESTART-1 새 ufrag 통과");
    let mut p = Pairs::new();
    let a = Via::Udp(addr("10.0.0.1:5000"));
    let b = Via::Udp(addr("10.0.0.2:5000"));
    match check(&mut l, &mut p, &req_for(&c0.ufrag, &c0.pwd, true), a, 0) {
        Verdict::Accepted { promoted, .. } => assert_eq!(promoted, None, "S-sym-IMPL-LINK-ICE-RESTART-1 옛 ufrag 검사는 승격 아님"),
        v => panic!("S-sym-IMPL-LINK-ICE-RESTART-1 옛 ufrag 검사 거절 {v:?}"),
    }
    match check(&mut l, &mut p, &req_for(&c3.ufrag, &c3.pwd, true), b, 10) {
        Verdict::Accepted { promoted, path_moved, .. } => {
            assert!(promoted.is_some(), "S-sym-IMPL-LINK-ICE-RESTART-1 새 ufrag 첫 검증 = 승격");
            assert!(path_moved, "S-sym-IMPL-LINK-ICE-RESTART-1 지명 실렸으면 경로 이동");
        }
        v => panic!("{v:?}"),
    }
    assert_eq!(l.pwd_for(&c0.ufrag), None, "S-sym-IMPL-LINK-ICE-RESTART-1 그 뒤 옛 ufrag 거절");
    assert_eq!(l.current(), &c3, "S-sym-IMPL-LINK-ICE-RESTART-1 현 자격 = 새 것");
    assert_eq!(check(&mut l, &mut p, &req_for(&c0.ufrag, &c0.pwd, true), a, 20), Verdict::Unverified, "S-sym-IMPL-LINK-ICE-RESTART-1 옛 ufrag 검사 거절");
}

#[test]
fn generated_creds_shape() {
    let a = Creds::generate();
    let b = Creds::generate();
    assert_eq!((a.ufrag.len(), a.pwd.len()), (UFRAG_LEN, PWD_LEN), "S-sym-IMPL-LINK-ICE-RESTART-2 ufrag 8 · pwd 22");
    assert_eq!((UFRAG_LEN, PWD_LEN), (8, 22), "S-sym-IMPL-LINK-ICE-RESTART-2 규격 값");
    assert_ne!(a.ufrag, b.ufrag, "S-sym-IMPL-LINK-ICE-RESTART-2 재시작마다 새 ufrag");
    let ice_char = |c: char| c.is_ascii_alphanumeric() || c == '+' || c == '/';
    assert!(a.ufrag.chars().all(ice_char) && a.pwd.chars().all(ice_char), "S-sym-IMPL-LINK-ICE-RESTART-2 ice-char(RFC 8839)");
}

#[test]
fn rfc4571_framing() {
    let f = tcp::frame(&[1, 2, 3]).unwrap();
    assert_eq!(f, vec![0, 3, 1, 2, 3], "S-sym-RULE-LINK-TRANSPORT-5 2바이트 BE 길이 접두");
    assert_eq!(tcp::frame(&[]).unwrap(), vec![0, 0], "S-sym-RULE-LINK-TRANSPORT-5 길이 0 프레임(keepalive)");
    assert!(tcp::frame(&vec![0u8; 65_535]).is_some(), "S-sym-RULE-LINK-TRANSPORT-5 최대 65,535");
    assert!(tcp::frame(&vec![0u8; 65_536]).is_none(), "S-sym-RULE-LINK-TRANSPORT-5 65,536 은 못 싣는다");
    let mut d = tcp::Deframer::new();
    let mut stream = vec![];
    stream.extend(tcp::frame(&[9; 300]).unwrap());
    stream.extend(tcp::frame(&[]).unwrap());
    stream.extend(tcp::frame(&[7, 7]).unwrap());
    stream.extend(tcp::frame(&[5; 1000]).unwrap());
    let mut out = vec![];
    for chunk in stream.chunks(7) {
        out.extend(d.push(chunk));
    }
    let nonempty: Vec<Vec<u8>> = out.into_iter().filter(|x| !x.is_empty()).collect();
    assert_eq!(nonempty, vec![vec![9; 300], vec![7, 7], vec![5; 1000]], "S-sym-RULE-LINK-TRANSPORT-5 부분 수신 보류 · 경계 그대로");
    assert!(d.push(&[0]).is_empty(), "S-sym-RULE-LINK-TRANSPORT-5 길이 반쪽은 보류");
    assert_eq!(d.push(&[2, 4, 4]), vec![vec![4, 4]], "S-sym-RULE-LINK-TRANSPORT-5 이어 오면 낸다");
}
