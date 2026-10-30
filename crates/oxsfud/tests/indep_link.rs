// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-LINK-TRANSPORT · 정:IMPL-LINK-ICE-RESTART · 정:RULE-LINK-PAIR-LIVENESS · model: claude-opus-5-5
//! 맹검 독립 1층 — STUN 은 message-integrity 검증 뒤에만(MI 키 = USERNAME ufrag 의 pwd · RFC 5389 §15.4 를 손으로 지은 요청) · ICE 자격 벌마다 최대 둘 · 쌍별 생존.

use hmac::{Hmac, Mac};
use oxsfud::link::stun::{self, StunError};
use oxsfud::link::{Cred, Creds, PathKey, Role, Transport};
use sha1::Sha1;
use std::net::SocketAddr;
use std::sync::Arc;

const MAGIC: u32 = 0x2112_a442;

fn attr(buf: &mut Vec<u8>, t: u16, v: &[u8]) {
    buf.extend_from_slice(&t.to_be_bytes());
    buf.extend_from_slice(&(v.len() as u16).to_be_bytes());
    buf.extend_from_slice(v);
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

fn set_len(buf: &mut [u8], body_len: usize) {
    buf[2..4].copy_from_slice(&(body_len as u16).to_be_bytes());
}

fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
    let mut m = <Hmac<Sha1> as Mac>::new_from_slice(key).expect("hmac key");
    m.update(data);
    let out = m.finalize().into_bytes();
    let mut a = [0u8; 20];
    a.copy_from_slice(&out);
    a
}

struct Req<'a> {
    username: &'a str,
    sign_pwd: Option<&'a str>,
    use_candidate: bool,
    fingerprint: bool,
    priority: u32,
}

fn build_request(r: &Req<'_>, txid: [u8; 12]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&0x0001u16.to_be_bytes());
    b.extend_from_slice(&0u16.to_be_bytes());
    b.extend_from_slice(&MAGIC.to_be_bytes());
    b.extend_from_slice(&txid);
    attr(&mut b, 0x0006, r.username.as_bytes());
    attr(&mut b, 0x0024, &r.priority.to_be_bytes());
    attr(&mut b, 0x802a, &0x1122_3344_5566_7788u64.to_be_bytes());
    if r.use_candidate {
        attr(&mut b, 0x0025, &[]);
    }
    if let Some(pwd) = r.sign_pwd {
        let body_with_mi = b.len() - 20 + 24;
        set_len(&mut b, body_with_mi);
        let mac = hmac_sha1(pwd.as_bytes(), &b);
        attr(&mut b, 0x0008, &mac);
    }
    if r.fingerprint {
        let body_with_fp = b.len() - 20 + 8;
        set_len(&mut b, body_with_fp);
        let crc = crc32fast::hash(&b) ^ 0x5354_554e;
        attr(&mut b, 0x8028, &crc.to_be_bytes());
    }
    let n = b.len() - 20;
    set_len(&mut b, n);
    b
}

fn find_attr(msg: &[u8], t: u16) -> Option<(usize, Vec<u8>)> {
    let mut i = 20;
    while i + 4 <= msg.len() {
        let at = u16::from_be_bytes([msg[i], msg[i + 1]]);
        let len = u16::from_be_bytes([msg[i + 2], msg[i + 3]]) as usize;
        if i + 4 + len > msg.len() {
            return None;
        }
        if at == t {
            return Some((i, msg[i + 4..i + 4 + len].to_vec()));
        }
        i += 4 + len.div_ceil(4) * 4;
    }
    None
}

fn mi_valid(msg: &[u8], pwd: &str) -> bool {
    let Some((off, mac)) = find_attr(msg, 0x0008) else { return false };
    let mut head = msg[..off].to_vec();
    set_len(&mut head, off - 20 + 24);
    hmac_sha1(pwd.as_bytes(), &head)[..] == mac[..]
}

fn pwd_table<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<&'a str> {
    move |u: &str| pairs.iter().find(|(k, _)| *k == u).map(|(_, v)| *v)
}

const SRV: &str = "srvUfrg1";
const SRV_PWD: &str = "server-password-22chrs";

#[test]
fn verified_binding_is_accepted() {
    let table = [(SRV, SRV_PWD)];
    for fp in [true, false] {
        let raw = build_request(&Req { username: "srvUfrg1:cliA", sign_pwd: Some(SRV_PWD), use_candidate: true, fingerprint: fp, priority: 7 }, [7; 12]);
        let b = stun::check(&raw, pwd_table(&table)).unwrap_or_else(|e| panic!("S-sym-RULE-LINK-TRANSPORT-3 valid request rejected: {e:?} · fingerprint {fp}"));
        assert_eq!(b.local_ufrag, SRV, "S-sym-RULE-LINK-TRANSPORT-3 USERNAME first half is the server ufrag");
        assert_eq!(b.remote_ufrag, "cliA", "S-sym-RULE-LINK-TRANSPORT-3 USERNAME second half is the client ufrag");
        assert!(b.use_candidate, "S-sym-RULE-LINK-TRANSPORT-3 USE-CANDIDATE seen after verification");
        assert_eq!(b.txid, [7; 12], "S-sym-RULE-LINK-TRANSPORT-3 txid carried");
    }
}

#[test]
fn forged_binding_is_rejected_before_processing() {
    let table = [(SRV, SRV_PWD)];
    let forged = build_request(&Req { username: "srvUfrg1:evil", sign_pwd: Some("attacker-guess-password"), use_candidate: true, fingerprint: true, priority: 1 }, [1; 12]);
    let r = stun::check(&forged, pwd_table(&table));
    assert!(r.is_err(), "S-sym-RULE-LINK-TRANSPORT-3 binding signed with a wrong pwd must not pass: {r:?}");
    assert_eq!(r.err(), Some(StunError::BadIntegrity), "S-sym-RULE-LINK-TRANSPORT-3 wrong key → BadIntegrity");

    let unsigned = build_request(&Req { username: "srvUfrg1:evil", sign_pwd: None, use_candidate: true, fingerprint: true, priority: 1 }, [2; 12]);
    let r = stun::check(&unsigned, pwd_table(&table));
    assert!(r.is_err(), "S-sym-RULE-LINK-TRANSPORT-3 binding without MESSAGE-INTEGRITY must not pass: {r:?}");

    let unknown = build_request(&Req { username: "zzzzzzzz:evil", sign_pwd: Some(SRV_PWD), use_candidate: true, fingerprint: true, priority: 1 }, [3; 12]);
    let r = stun::check(&unknown, pwd_table(&table));
    assert!(r.is_err(), "S-sym-RULE-LINK-TRANSPORT-3 unknown ufrag must not pass: {r:?}");

    let mut tampered = build_request(&Req { username: "srvUfrg1:cliA", sign_pwd: Some(SRV_PWD), use_candidate: false, fingerprint: false, priority: 5 }, [4; 12]);
    let (off, _) = find_attr(&tampered, 0x0024).expect("priority");
    tampered[off + 7] ^= 0x01;
    let r = stun::check(&tampered, pwd_table(&table));
    assert_eq!(r.err(), Some(StunError::BadIntegrity), "S-sym-RULE-LINK-TRANSPORT-3 a byte changed after signing → BadIntegrity");

    let mut uc_injected = build_request(&Req { username: "srvUfrg1:cliA", sign_pwd: Some(SRV_PWD), use_candidate: false, fingerprint: false, priority: 5 }, [5; 12]);
    let (off, _) = find_attr(&uc_injected, 0x0024).expect("priority");
    uc_injected[off..off + 2].copy_from_slice(&0x0025u16.to_be_bytes());
    uc_injected[off + 2..off + 4].copy_from_slice(&4u16.to_be_bytes());
    let r = stun::check(&uc_injected, pwd_table(&table));
    assert!(r.is_err(), "S-sym-RULE-LINK-TRANSPORT-3 USE-CANDIDATE spliced in after signing must not pass: {r:?}");
}

#[test]
fn integrity_key_is_pwd_of_username_ufrag_during_restart() {
    let mut creds = Creds::new();
    let old = creds.active.clone();
    let next = Cred { ufrag: "NeWuFrAg".into(), pwd: "new-password-22-chars!".into() };
    creds.restart(next.clone());
    let pwd_of = |u: &str| creds.pwd(u);
    let ok_new = build_request(&Req { username: &format!("{}:c", next.ufrag), sign_pwd: Some(&next.pwd), use_candidate: true, fingerprint: true, priority: 9 }, [9; 12]);
    assert!(stun::check(&ok_new, pwd_of).is_ok(), "S-sym-RULE-LINK-TRANSPORT-3 new ufrag signed with new pwd passes");
    let ok_old = build_request(&Req { username: &format!("{}:c", old.ufrag), sign_pwd: Some(&old.pwd), use_candidate: false, fingerprint: true, priority: 9 }, [9; 12]);
    assert!(stun::check(&ok_old, pwd_of).is_ok(), "S-sym-RULE-LINK-TRANSPORT-3 old ufrag signed with old pwd passes during restart");
    let cross1 = build_request(&Req { username: &format!("{}:c", next.ufrag), sign_pwd: Some(&old.pwd), use_candidate: true, fingerprint: true, priority: 9 }, [9; 12]);
    assert_eq!(stun::check(&cross1, pwd_of).err(), Some(StunError::BadIntegrity), "S-sym-RULE-LINK-TRANSPORT-3 new ufrag signed with old pwd fails");
    let cross2 = build_request(&Req { username: &format!("{}:c", old.ufrag), sign_pwd: Some(&next.pwd), use_candidate: true, fingerprint: true, priority: 9 }, [9; 12]);
    assert_eq!(stun::check(&cross2, pwd_of).err(), Some(StunError::BadIntegrity), "S-sym-RULE-LINK-TRANSPORT-3 old ufrag signed with new pwd fails");
}

#[test]
fn product_request_and_verify_agree_with_rfc5389() {
    let raw = stun::request(SRV, "cliZ", SRV_PWD, [0x42; 12], true);
    assert!(mi_valid(&raw, SRV_PWD), "S-sym-RULE-LINK-TRANSPORT-3 product-built request carries an RFC 5389 §15.4 MI under the given pwd");
    assert!(!mi_valid(&raw, "other"), "S-sym-RULE-LINK-TRANSPORT-3 MI does not verify under another pwd");
    let b = stun::parse(&raw).expect("parse own request");
    assert!(stun::verify(&raw, &b, SRV_PWD), "S-sym-RULE-LINK-TRANSPORT-3 verify accepts the right pwd");
    assert!(!stun::verify(&raw, &b, "wrong-password-xxxxxxx"), "S-sym-RULE-LINK-TRANSPORT-3 verify rejects the wrong pwd");
    let mine = build_request(&Req { username: "srvUfrg1:cliB", sign_pwd: Some(SRV_PWD), use_candidate: false, fingerprint: true, priority: 3 }, [8; 12]);
    let b = stun::parse(&mine).expect("parse hand-built request");
    assert!(stun::verify(&mine, &b, SRV_PWD), "S-sym-RULE-LINK-TRANSPORT-3 verify accepts a hand-built RFC 5389 request");
    assert!(!stun::verify(&mine, &b, "nope"), "S-sym-RULE-LINK-TRANSPORT-3 verify rejects it under another key");
}

#[test]
fn restart_keeps_at_most_two_per_set() {
    let mut creds = Creds::new();
    let first = creds.active.clone();
    let mut issued = Vec::new();
    for i in 0..50 {
        let c = Cred { ufrag: format!("u{i:07}"), pwd: format!("p{i:021}") };
        creds.restart(c.clone());
        issued.push(c);
        assert!(creds.ufrags().len() <= 2, "S-sym-IMPL-LINK-ICE-RESTART-2 at most two ufrags per set after {} restarts: {:?}", i + 1, creds.ufrags());
    }
    let last = issued.last().expect("issued").clone();
    assert_eq!(creds.pwd(&first.ufrag), Some(first.pwd.as_str()), "S-sym-IMPL-LINK-ICE-RESTART-1 the one in use is kept until the new one is checked");
    assert_eq!(creds.pwd(&last.ufrag), Some(last.pwd.as_str()), "S-sym-IMPL-LINK-ICE-RESTART-2 the newest pending passes");
    for c in &issued[..issued.len() - 1] {
        assert_eq!(creds.pwd(&c.ufrag), None, "S-sym-IMPL-LINK-ICE-RESTART-2 a superseded pending ufrag no longer passes: {}", c.ufrag);
    }
    let late = build_request(&Req { username: &format!("{}:c", issued[10].ufrag), sign_pwd: Some(&issued[10].pwd), use_candidate: true, fingerprint: true, priority: 1 }, [1; 12]);
    assert!(stun::check(&late, |u: &str| creds.pwd(u)).is_err(), "S-sym-IMPL-LINK-ICE-RESTART-2 credentials from a late older response do not authenticate");
}

#[test]
fn restart_replaces_pending_and_reports_it() {
    let mut creds = Creds::new();
    let a = Cred { ufrag: "aaaaaaaa".into(), pwd: "a".repeat(22) };
    let b = Cred { ufrag: "bbbbbbbb".into(), pwd: "b".repeat(22) };
    let r1 = creds.restart(a.clone());
    assert_eq!(r1, None, "S-sym-IMPL-LINK-ICE-RESTART-2 first restart replaces no pending");
    let r2 = creds.restart(b.clone());
    assert_eq!(r2, Some(a.ufrag.clone()), "S-sym-IMPL-LINK-ICE-RESTART-2 second restart hands back the replaced pending ufrag to unregister");
    let mut set = creds.ufrags();
    set.sort();
    let mut want = vec![creds.active.ufrag.clone(), b.ufrag.clone()];
    want.sort();
    assert_eq!(set, want, "S-sym-IMPL-LINK-ICE-RESTART-2 ledger is exactly active + newest pending");
}

#[test]
fn old_ufrag_kept_until_new_one_checked() {
    let mut creds = Creds::new();
    let old = creds.active.clone();
    let next = Cred { ufrag: "nnnnnnnn".into(), pwd: "n".repeat(22) };
    creds.restart(next.clone());
    assert!(creds.pwd(&old.ufrag).is_some(), "S-sym-IMPL-LINK-ICE-RESTART-1 old ufrag still accepted right after issuing new credentials");
    assert_eq!(creds.checked(&old.ufrag), oxsfud::link::creds::Checked::Current, "S-sym-IMPL-LINK-ICE-RESTART-1 a check on the old ufrag keeps it current");
    assert!(creds.pwd(&old.ufrag).is_some() && creds.pwd(&next.ufrag).is_some(), "S-sym-IMPL-LINK-ICE-RESTART-1 both pass before the new one is used");
    let c = creds.checked(&next.ufrag);
    assert_eq!(c, oxsfud::link::creds::Checked::Promoted { retired: old.ufrag.clone() }, "S-sym-IMPL-LINK-ICE-RESTART-1 first verified check on the new ufrag retires the old");
    assert_eq!(creds.pwd(&old.ufrag), None, "S-sym-IMPL-LINK-ICE-RESTART-1 old ufrag dropped after promotion");
    assert_eq!(creds.active, next, "S-sym-IMPL-LINK-ICE-RESTART-1 new credentials active");
    assert_eq!(creds.ufrags(), vec![next.ufrag.clone()], "S-sym-IMPL-LINK-ICE-RESTART-2 one set left after promotion");
}

#[test]
fn expired_udp_pair_is_not_used() {
    let tr: Arc<Transport> = Transport::new(1, "u1", Role::Both, 0);
    let addr: SocketAddr = "198.51.100.7:40000".parse().expect("addr");
    let key = PathKey::Udp(addr);
    tr.touch_pair(key, 1_000);
    tr.path.store(Some(Arc::new(key)));
    let never_tcp = |_: u64| false;
    assert_eq!(tr.send_ok(1_000, never_tcp), Some(key), "S-sym-RULE-LINK-PAIR-LIVENESS-1 fresh path is usable");
    assert_eq!(tr.send_ok(30_999, never_tcp), Some(key), "S-sym-RULE-LINK-PAIR-LIVENESS-1 path within 30s is usable");
    assert_eq!(tr.send_ok(31_001, never_tcp), None, "S-sym-RULE-LINK-PAIR-LIVENESS-1 path past 30s without a verified check must not be sent to");
    assert!(!tr.path_alive(31_001, never_tcp), "S-sym-RULE-LINK-PAIR-LIVENESS-1 expired path is not alive");
    tr.touch_pair(key, 40_000);
    assert_eq!(tr.send_ok(40_001, never_tcp), Some(key), "S-sym-RULE-LINK-PAIR-LIVENESS-1 a new verified check revives the pair");
}

#[test]
fn tcp_pair_lives_while_connected() {
    let tr: Arc<Transport> = Transport::new(2, "u2", Role::Both, 0);
    let key = PathKey::Tcp(77);
    tr.touch_pair(key, 0);
    tr.path.store(Some(Arc::new(key)));
    assert_eq!(tr.send_ok(10_000_000, |id| id == 77), Some(key), "S-sym-RULE-LINK-PAIR-LIVENESS-1 TCP pair lives while the connection exists");
    assert_eq!(tr.send_ok(1, |_| false), None, "S-sym-RULE-LINK-PAIR-LIVENESS-1 closed TCP connection is not sent to");
}
