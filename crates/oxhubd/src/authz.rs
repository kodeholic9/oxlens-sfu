// author: kodeholic (powered by Claude)
// spec: v1.1 · 운영 §1 · 정§16-1 · §16-1-1 · model: claude-opus-5

//! C 평면 자격 — ★**세 갈래.**
//!
//! ★★**운영자는 사용자가 아니다.** A 평면 토큰으로는 `/admin` 이 열리지 않는다 —
//! 열리면 ★**녹화 봇이 서버를 끌 수 있다**(그 봇은 고객 앱이 발급한 것이다).

use common::System;
use oxsig::Code;

use crate::token::{verify_account, AccountScope};

/// 요청이 들고 온 것. ★**`X-Forwarded-For` 는 여기 없다** — 믿지 않기로 했으므로 자리도 안 둔다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// ★**소켓이 말하는 상대**다. 헤더가 아니다.
    pub is_loopback: bool,
    pub bearer: Option<String>,
}

/// 자격 판정 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    /// loopback — 같은 기계라 통과.
    Loopback,
    /// 운영 토큰으로 통과. 어느 계정인가.
    Ops(String),
}

/// `/admin/*` 자격. ★**전부 401 이고 body 는 `Failure` 형이다.**
pub fn admin(sys: &System, now: u64, peer: &Peer) -> Result<Grant, Code> {
    // ★리버스 프록시 뒤 배치 금지가 전제다 — 그래서 소켓만 본다.
    if peer.is_loopback {
        return Ok(Grant::Loopback);
    }
    let token = peer.bearer.as_deref().ok_or(Code::NotAuthorized)?;
    match verify_account(sys, AccountScope::Ops, now, token) {
        Ok(c) => Ok(Grant::Ops(c.iss)),
        Err(e) => Err(e.0),
    }
}

/// 되돌릴 수 없는 조작의 확인값(운영 §5). ★**대상이 쥔 판 값**이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm<'a> {
    /// 방 — `?if_version={epoch}:{seq}`.
    Version { epoch: &'a str, seq: u64 },
    /// 유닛 — `?if_epoch={sfu_id}`.
    Epoch(&'a str),
    /// hub — `?confirm={hub id}`.
    Name(&'a str),
}

/// 확인값을 견준다. ★**없으면 `1003`, 안 맞으면 `3009`.**
///
/// ★**도구가 사람에게 다시 묻는 것으로 대신하지 않는다** — 스크립트로 부르면 그 물음이 사라진다.
pub fn check_confirm(given: Option<&str>, want: &Confirm<'_>) -> Result<(), Code> {
    let Some(g) = given else { return Err(Code::MissingField) };
    let ok = match want {
        Confirm::Version { epoch, seq } => g == format!("{epoch}:{seq}"),
        Confirm::Epoch(e) => g == *e,
        Confirm::Name(n) => g == *n,
    };
    if ok {
        Ok(())
    } else {
        // ★형식은 맞고 값이 낡았다 — `1002`(형식)도 `1003`(부재)도 아니다.
        Err(Code::PreconditionFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::{AccountClaims, IssueReq};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    fn sys() -> System {
        System::parse(
            r#"
[hub]
listen = "127.0.0.1:1974"
[hub.auth]
jwt_secret = "app-secret"
[[hub.auth.api_keys]]
key = "ox_k_ops"
secret = "ox_s_ops"
participant_types = [0]
ops_allowed = true
"#,
        )
        .expect("system")
    }

    fn ops_token() -> String {
        let c = AccountClaims {
            iss: "ox_k_ops".into(),
            ops: true,
            hook: false,
            iat: 0,
            exp: 9_999,
        };
        encode(&Header::new(Algorithm::HS256), &c, &EncodingKey::from_secret(b"ox_s_ops"))
            .expect("sign")
    }

    #[test]
    fn loopback_은_토큰_없이_통과한다() {
        let p = Peer { is_loopback: true, bearer: None };
        assert_eq!(admin(&sys(), 0, &p), Ok(Grant::Loopback));
    }

    #[test]
    fn 원격은_운영_토큰이_있어야_한다() {
        let s = sys();
        let none = Peer { is_loopback: false, bearer: None };
        assert_eq!(admin(&s, 0, &none), Err(Code::NotAuthorized));
        let ok = Peer { is_loopback: false, bearer: Some(ops_token()) };
        assert_eq!(admin(&s, 10, &ok), Ok(Grant::Ops("ox_k_ops".into())));
    }

    #[test]
    fn 앱_사용자_토큰으로는_안_열린다() {
        // ★열리면 녹화 봇이 서버를 끌 수 있다 — 그 봇은 고객 앱이 발급한 것이다.
        let s = sys();
        let app = crate::token::issue(
            &s,
            3600,
            2048,
            0,
            &IssueReq {
                api_key: "ox_k_ops".into(),
                api_secret: "ox_s_ops".into(),
                user_id: "u1".into(),
                participant_type: 0,
                hidden: false,
                permission: None,
                metadata: None,
            },
        )
        .expect("issue");
        let p = Peer { is_loopback: false, bearer: Some(app.token) };
        assert!(admin(&s, 10, &p).is_err());
    }

    #[test]
    fn 확인값이_없으면_1003_안_맞으면_3009_다() {
        let want = Confirm::Version { epoch: "sfu-1", seq: 10 };
        assert_eq!(check_confirm(None, &want), Err(Code::MissingField));
        assert_eq!(check_confirm(Some("sfu-1:9"), &want), Err(Code::PreconditionFailed));
        assert_eq!(check_confirm(Some("sfu-1:10"), &want), Ok(()));
    }

    #[test]
    fn 유닛은_기동_신원으로_짚는다() {
        // ★이름은 재기동해도 같아 "내가 본 그 프로세스"를 못 지목한다.
        let want = Confirm::Epoch("sfu-7f3a");
        assert_eq!(check_confirm(Some("sfu-1"), &want), Err(Code::PreconditionFailed));
        assert_eq!(check_confirm(Some("sfu-7f3a"), &want), Ok(()));
    }
}
