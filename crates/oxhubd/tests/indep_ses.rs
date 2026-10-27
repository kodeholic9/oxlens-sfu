// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-SES-BIND · 정:IMPL-SES-TOKEN-ISSUE · 정:RULE-OPS-TOKEN · 정:IMPL-SES-SOCKET-LIFE · model: claude-opus-5-5
//! 독립 1층 시험(B′ 맹검) — BIND 갈래(session_id 먼저) · 송신 큐 상한 · ACK 시한 · 토큰 발급 · 검증 · 운영 토큰.

use common::system::ApiKey;
use jsonwebtoken::{encode, EncodingKey, Header};
use oxhubd::ses::core::{bind_branch, new_session_id, Branch, OutQueue, Push, ACK_TIMEOUT_MS, MAX_PENDING};
use oxhubd::ses::token::{issue, secrets, verify, verify_ops};
use oxsig::code::Code;
use oxsig::msg::TokenReq;
use oxsig::op::Lane;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

fn acct(key: &str, secret: &str, ops: bool) -> ApiKey {
    ApiKey { key: key.into(), secret: secret.into(), name: None, participant_types: vec![0], hidden_allowed: false, ops_allowed: ops, hook_allowed: false }
}

fn req(key: &str, secret: &str, user: &str) -> TokenReq {
    TokenReq { api_key: key.into(), api_secret: secret.into(), user_id: user.into(), participant_type: None, hidden: None, metadata: None, permission: None }
}

#[test]
fn bind_session_id_branch_before_token() {
    assert_eq!(bind_branch(Some("s1"), true), Branch::Resume("s1".into()), "S-sym-IMPL-SES-BIND-1 session_id 갈래 먼저 — 토큰 검사 전");
    assert_eq!(bind_branch(Some("s1"), false), Branch::Elsewhere("s1".into()), "S-sym-IMPL-SES-BIND-1 이 hub 에 없으면 다른 곳을 찾는다(토큰 아님)");
    assert_eq!(bind_branch(None, true), Branch::Token, "S-sym-IMPL-SES-BIND-1 session_id 없으면 토큰 갈래");
    assert_eq!(bind_branch(Some(""), true), Branch::Token, "S-sym-IMPL-SES-BIND-1 빈 session_id 는 없음과 같다");
    let a = new_session_id();
    let b = new_session_id();
    assert!(!a.is_empty() && a != b, "S-sym-IMPL-SES-BIND-1 세션 id 는 새로 발급");
}

#[test]
fn out_queue_overflow_at_max_pending() {
    let mut q = OutQueue::new(4);
    for i in 0..MAX_PENDING {
        assert_eq!(q.push(Lane::Data, 0x0100, vec![i as u8]), Push::Queued, "SV-CODE-4006-4008 상한 전은 쌓인다");
    }
    assert_eq!(q.push(Lane::Data, 0x0100, vec![]), Push::Overflow, "SV-CODE-4006-4008 1,000 넘으면 4007");
    assert_eq!(MAX_PENDING, 1000, "SV-CODE-4006-4008 규격 값 1,000");
    assert!(q.pending() >= MAX_PENDING, "SV-CODE-4006-4008 넘친 것은 안 쌓는다");
}

#[test]
fn ack_timeout_marks_overdue_and_ack_clears() {
    let mut q = OutQueue::new(4);
    q.push(Lane::Session, 0x0200, vec![1]);
    let (pid, op, body) = q.pop_ready(1000).expect("보낼 것");
    assert_eq!((op, body), (0x0200, vec![1]), "SV-CODE-4006-4008 넣은 것이 나간다");
    assert!(!q.overdue(1000 + ACK_TIMEOUT_MS - 1, ACK_TIMEOUT_MS), "SV-CODE-4006-4008 30s 안은 시한 전");
    assert!(q.overdue(1000 + ACK_TIMEOUT_MS + 1, ACK_TIMEOUT_MS), "SV-CODE-4006-4008 ACK 30s 없으면 4006");
    assert!(q.ack(pid), "SV-CODE-4006-4008 ACK 받음");
    assert!(!q.overdue(1000 + 10 * ACK_TIMEOUT_MS, ACK_TIMEOUT_MS), "SV-CODE-4006-4008 ACK 뒤 시한 없음");
    assert!(!q.ack(pid), "SV-CODE-4006-4008 같은 ACK 두 번은 거짓");
    assert_eq!(ACK_TIMEOUT_MS, 30_000, "SV-CODE-4006-4008 규격 값 30s");
}

#[test]
fn out_queue_window_and_lane_priority() {
    let mut q = OutQueue::new(2);
    q.push(Lane::Diagnostic, 9, vec![]);
    q.push(Lane::Data, 3, vec![]);
    q.push(Lane::Recovery, 1, vec![]);
    let a = q.pop_ready(0).expect("첫째");
    assert_eq!(a.1, 1, "SV-CODE-4006-4008 Recovery 단이 먼저");
    let b = q.pop_ready(0).expect("둘째");
    assert_eq!(b.1, 3, "SV-CODE-4006-4008 Data 가 Diagnostic 보다 먼저");
    assert!(q.pop_ready(0).is_none(), "SV-CODE-4006-4008 창 2 를 넘어 안 보낸다");
    q.ack(a.0);
    assert_eq!(q.pop_ready(0).map(|x| x.1), Some(9), "SV-CODE-4006-4008 ACK 뒤 창이 열린다");
}

#[test]
fn token_issue_and_verify() {
    let accts = vec![acct("k1", "s1-secret-very-long-0000", false)];
    let (tok, exp) = issue(&accts, &req("k1", "s1-secret-very-long-0000", "alice"), 3600, 1024, now()).expect("발급");
    assert_eq!(exp, 3600, "SV-CODE-2001-2005 expires_in");
    let id = verify(&tok, &secrets(&accts, None)).expect("검증");
    assert_eq!(id.user_id, "alice", "SV-CODE-2001-2005 sub = 사용자");
    assert_eq!(issue(&accts, &req("nope", "x", "a"), 60, 1024, now()).unwrap_err(), Code::INVALID_API_KEY, "SV-CODE-2001-2005 모르는 계정 2004");
    assert_eq!(issue(&accts, &req("k1", "wrong", "a"), 60, 1024, now()).unwrap_err(), Code::INVALID_API_KEY, "SV-CODE-2001-2005 비밀 불일치 2004");
    let mut r = req("k1", "s1-secret-very-long-0000", "a");
    r.participant_type = Some(1);
    assert_eq!(issue(&accts, &r, 60, 1024, now()).unwrap_err(), Code::CLAIM_NOT_ALLOWED, "SV-CODE-2001-2005 허용 밖 participant_type 2005");
    let mut r = req("k1", "s1-secret-very-long-0000", "a");
    r.hidden = Some(true);
    assert_eq!(issue(&accts, &r, 60, 1024, now()).unwrap_err(), Code::CLAIM_NOT_ALLOWED, "SV-CODE-2001-2005 투명 미허용 2005");
    let mut bad = tok.clone();
    let last = bad.pop().unwrap();
    bad.push(if last == 'A' { 'B' } else { 'A' });
    assert_eq!(verify(&bad, &secrets(&accts, None)).unwrap_err(), Code::TOKEN_INVALID, "SV-CODE-2001-2005 서명 불량 2002");
    let (old, _) = issue(&accts, &req("k1", "s1-secret-very-long-0000", "bob"), 1, 1024, now() - 7200).expect("옛 발급");
    assert_eq!(verify(&old, &secrets(&accts, None)).unwrap_err(), Code::TOKEN_EXPIRED, "SV-CODE-2001-2005 만료 2003");
}

#[test]
fn user_token_is_not_ops_credential() {
    let accts = vec![acct("k1", "s1-secret-very-long-0000", true)];
    let (tok, _) = issue(&accts, &req("k1", "s1-secret-very-long-0000", "alice"), 3600, 1024, now()).expect("발급");
    let payload = tok.split('.').nth(1).unwrap();
    assert!(!payload.is_empty(), "시험 형상");
    let e = verify_ops(&tok, &accts).expect_err("S-sym-RULE-OPS-TOKEN-1 사용자 토큰으로 /admin = 401");
    assert_eq!(e.http_status(), 401, "S-sym-RULE-OPS-TOKEN-2 sub 있는 토큰은 운영 자격이 아니다 — {e:?}");
}

#[derive(serde::Serialize)]
struct Ops<'a> {
    iss: &'a str,
    ops: bool,
    iat: u64,
    exp: u64,
}

fn ops_token(iss: &str, secret: &str, ops: bool, exp: u64) -> String {
    encode(&Header::default(), &Ops { iss, ops, iat: now(), exp }, &EncodingKey::from_secret(secret.as_bytes())).unwrap()
}

#[test]
fn ops_token_verification_branches() {
    let accts = vec![acct("op", "op-secret-very-long-0000", true), acct("app", "app-secret-very-long-000", false)];
    assert_eq!(verify_ops(&ops_token("op", "op-secret-very-long-0000", true, now() + 60), &accts), Ok("op".into()), "S-sym-RULE-OPS-TOKEN-2 운영 토큰(iss + ops, 운영 허용 계정) 통과");
    assert_eq!(verify_ops(&ops_token("app", "app-secret-very-long-000", true, now() + 60), &accts).unwrap_err(), Code::NOT_AUTHORIZED, "S-sym-RULE-OPS-TOKEN-2 운영 미허용 계정 2006");
    assert_eq!(verify_ops(&ops_token("op", "op-secret-very-long-0000", false, now() + 60), &accts).unwrap_err(), Code::NOT_AUTHORIZED, "S-sym-RULE-OPS-TOKEN-2 ops 아님 2006");
    assert_eq!(verify_ops(&ops_token("ghost", "op-secret-very-long-0000", true, now() + 60), &accts).unwrap_err(), Code::INVALID_API_KEY, "S-sym-RULE-OPS-TOKEN-2 없는 iss 2004");
    assert_eq!(verify_ops(&ops_token("op", "wrong-secret-very-long-0", true, now() + 60), &accts).unwrap_err(), Code::TOKEN_INVALID, "S-sym-RULE-OPS-TOKEN-2 서명 불량 2002");
    assert_eq!(verify_ops(&ops_token("op", "op-secret-very-long-0000", true, now() - 3600), &accts).unwrap_err(), Code::TOKEN_EXPIRED, "S-sym-RULE-OPS-TOKEN-2 만료 2003");
}
