// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§5-2 · 정§3-4 · §16-1-1 · hook §3 · model: claude-opus-5

//! 토큰 셋 — ★**자격이 셋이고 평면이 셋이다.**
//!
//! | | 서명 비밀 | 실리는 것 | 여는 것 |
//! |---|---|---|---|
//! | A 평면 사용자 | ★시스템 파일의 `jwt_secret`(hub 가 발급) | `sub`(사람) · 종류 · 투명 · 권한 | WS·클라 HTTP |
//! | 운영(`ops`) | ★**계정의 `api_secret`**(운영자가 직접 서명) | `iss`(계정) — ★**`sub` 가 없다** | `/admin/*` |
//! | 연동(`hook`) | ★**계정의 `api_secret`**(3rd 가 직접 서명) | 같음 | 대외 `ctl`·사실 |
//!
//! ★★**hub 에 발급 경로가 있는 것은 A 평면 하나다.** 운영·연동에 발급 경로를 두면
//! ★**그 경로를 부를 수 있는 쪽이 전부 운영자가 된다.**

use common::system::{ApiKey, System};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use oxsig::{Code, Permission};
use serde::{Deserialize, Serialize};

/// A 평면 사용자 토큰의 claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserClaims {
    /// 사람의 신원.
    pub sub: String,
    /// ★**무엇인가** — `0` 사람 · `1` 녹화 · `2` 봇.
    #[serde(default)]
    pub participant_type: u8,
    /// ★**명단에 보이나** — 종류와 ★**다른 축이다**(녹화가 곧 투명이 아니다).
    #[serde(default)]
    pub hidden: bool,
    /// ★**입장 초기값일 뿐이다** — 그 뒤는 방이 기억한다(연§4-4-1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
    /// 사람의 신원 — 서명되어 명단에 그대로 실린다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub iat: u64,
    pub exp: u64,
}

impl UserClaims {
    /// ★부재를 기본값으로 편다 — 소비처마다 다시 펴면 그때마다 갈린다.
    pub fn permission(&self) -> Permission {
        self.permission.unwrap_or_default()
    }
}

/// 운영·연동 토큰의 claim. ★**`sub` 가 없다** — 사람의 신원이 아니라 **계정 자격**이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountClaims {
    pub iss: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ops: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hook: bool,
    pub iat: u64,
    pub exp: u64,
}

/// 계정 자격 두 갈래.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountScope {
    /// `/admin/*` — 서버를 끄고 유닛을 죽인다.
    Ops,
    /// 대외 `ctl`·사실 — ★**연동사에게 줄 물건이고 운영 자격과 다르다.**
    Hook,
}

/// 발급·검증 실패. ★**번호는 연§10-2 한 공간이다.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenError(pub Code, pub String);

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.1, self.0.name())
    }
}

impl std::error::Error for TokenError {}

fn err<T>(code: Code, msg: impl Into<String>) -> Result<T, TokenError> {
    Err(TokenError(code, msg.into()))
}

/// `POST /auth/token` 요청 — ★**앱 백엔드가 부른다.** 클라가 직접 부르지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IssueReq {
    pub api_key: String,
    pub api_secret: String,
    pub user_id: String,
    #[serde(default)]
    pub participant_type: u8,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub permission: Option<Permission>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueRes {
    pub token: String,
    /// 초.
    pub expires_in: u32,
}

fn account<'a>(sys: &'a System, key: &str) -> Option<&'a ApiKey> {
    sys.hub.auth.api_keys.iter().find(|a| a.key == key)
}

/// A 평면 사용자 토큰을 낸다.
///
/// ★**계정이 낼 수 있는 것만 서명된다** — 허용 밖이면 `2005` 다.
/// ★**클라가 스스로 투명해지는 경로는 없다**(그 자리가 `ROOM_JOIN` 에 없다).
pub fn issue(
    sys: &System,
    ttl_secs: u32,
    metadata_max_bytes: u32,
    now: u64,
    req: &IssueReq,
) -> Result<IssueRes, TokenError> {
    let acct = match account(sys, &req.api_key) {
        Some(a) if a.secret == req.api_secret => a,
        // ★없는 키와 틀린 비밀을 가르지 않는다 — 가르면 키 목록을 훑을 수 있다.
        _ => return err(Code::InvalidApiKey, "api_key·api_secret 이 맞지 않는다"),
    };
    // ★**형이 먼저다** — 모르는 값은 닫힌 집합 밖이라 `1002` 이고, 계정 허용 밖은 `2005` 다.
    //   합치면 *"오타"* 와 *"권한 없음"* 이 같은 답을 받아 발급자가 무엇을 고칠지 모른다.
    if !oxsig::is_participant_type(req.participant_type) {
        return err(
            Code::InvalidPayload,
            format!("participant_type {} 는 닫힌 집합(0·1·2) 밖", req.participant_type),
        );
    }
    if !acct.participant_types.contains(&req.participant_type) {
        return err(
            Code::ClaimNotAllowed,
            format!("계정 {} 는 participant_type {} 를 못 낸다", acct.key, req.participant_type),
        );
    }
    if req.hidden && !acct.hidden_allowed {
        return err(Code::ClaimNotAllowed, format!("계정 {} 는 투명을 못 낸다", acct.key));
    }
    if let Some(m) = &req.metadata {
        let n = serde_json::to_string(m).map(|s| s.len()).unwrap_or(usize::MAX);
        if n > metadata_max_bytes as usize {
            // ★여기서 거절한다 — 발급된 토큰이 방 안에서 걸리면 고칠 자리가 없다.
            return err(Code::InvalidPayload, format!("metadata {n} 바이트 > 상한 {metadata_max_bytes}"));
        }
    }
    let claims = UserClaims {
        sub: req.user_id.clone(),
        participant_type: req.participant_type,
        hidden: req.hidden,
        permission: req.permission,
        metadata: req.metadata.clone(),
        iat: now,
        exp: now + ttl_secs as u64,
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(sys.hub.auth.jwt_secret.as_bytes()),
    )
    .map_err(|e| TokenError(Code::InternalError, e.to_string()))?;
    Ok(IssueRes { token, expires_in: ttl_secs })
}

/// ★★**만료 판정을 라이브러리에 맡기지 않는다.**
///
/// 맡기면 그것이 **제 시계**(벽시계)를 보므로 주입한 `now` 가 무시되고,
/// ★**시험이 시계에 묶여** 만료 갈래를 못 태운다. 서명만 라이브러리가 보고 `exp` 는 우리가 본다.
fn validation() -> Validation {
    let mut v = Validation::new(Algorithm::HS256);
    v.required_spec_claims.clear();
    v.validate_exp = false;
    v.leeway = 0;
    v
}

fn map_jwt(e: jsonwebtoken::errors::Error) -> TokenError {
    use jsonwebtoken::errors::ErrorKind;
    match e.kind() {
        ErrorKind::ExpiredSignature => TokenError(Code::TokenExpired, "만료".into()),
        _ => TokenError(Code::TokenInvalid, "서명·형식 불량".into()),
    }
}

/// A 평면 사용자 토큰을 검증한다. ★**시스템 파일의 `jwt_secret` 으로만** 본다.
pub fn verify_user(sys: &System, now: u64, token: &str) -> Result<UserClaims, TokenError> {
    let data = decode::<UserClaims>(
        token,
        &DecodingKey::from_secret(sys.hub.auth.jwt_secret.as_bytes()),
        &validation(),
    )
    .map_err(map_jwt)?;
    if data.claims.exp <= now {
        return err(Code::TokenExpired, "만료");
    }
    Ok(data.claims)
}

/// 운영·연동 토큰을 검증한다.
///
/// ★**`iss` 로 계정을 찾아 그 계정의 비밀로 본다** — A 평면 비밀을 쓰지 않는다.
/// 그래서 ★**앱 사용자 토큰으로는 `/admin` 도 `ctl` 도 열리지 않는다.**
pub fn verify_account(
    sys: &System,
    scope: AccountScope,
    now: u64,
    token: &str,
) -> Result<AccountClaims, TokenError> {
    // `iss` 를 알아야 비밀을 고른다 — 서명 검증 전에 읽되 ★**그 값으로 아무것도 허락하지 않는다.**
    let unverified = {
        let mut v = Validation::new(Algorithm::HS256);
        v.insecure_disable_signature_validation();
        v.required_spec_claims.clear();
        v.validate_exp = false;
        decode::<AccountClaims>(token, &DecodingKey::from_secret(b""), &v).map_err(map_jwt)?
    };
    let acct = match account(sys, &unverified.claims.iss) {
        Some(a) => a,
        None => return err(Code::InvalidApiKey, "없는 iss"),
    };
    let data = decode::<AccountClaims>(
        token,
        &DecodingKey::from_secret(acct.secret.as_bytes()),
        &validation(),
    )
    .map_err(map_jwt)?;
    let c = data.claims;
    if c.exp <= now {
        return err(Code::TokenExpired, "만료");
    }
    // ★claim 과 계정 허용이 **둘 다** 서야 한다 — 하나만 보면 계정을 내려도 옛 토큰이 산다.
    let ok = match scope {
        AccountScope::Ops => c.ops && acct.ops_allowed,
        AccountScope::Hook => c.hook && acct.hook_allowed,
    };
    if !ok {
        return err(Code::NotAuthorized, "그 자격이 아니다");
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::system::System;

    fn sys() -> System {
        System::parse(
            r#"
[hub]
listen = "127.0.0.1:1974"
[hub.auth]
jwt_secret = "app-secret"
[[hub.auth.api_keys]]
key = "ox_k_demo"
secret = "ox_s_demo"
participant_types = [0, 1]
hidden_allowed = true
ops_allowed = true
[[hub.auth.api_keys]]
key = "ox_k_limited"
secret = "ox_s_limited"
participant_types = [0]
"#,
        )
        .expect("system")
    }

    fn req() -> IssueReq {
        IssueReq {
            api_key: "ox_k_demo".into(),
            api_secret: "ox_s_demo".into(),
            user_id: "u1".into(),
            participant_type: 0,
            hidden: false,
            permission: None,
            metadata: None,
        }
    }

    fn sign_account(secret: &str, c: &AccountClaims) -> String {
        encode(&Header::new(Algorithm::HS256), c, &EncodingKey::from_secret(secret.as_bytes()))
            .expect("sign")
    }

    #[test]
    fn 발급과_검증이_왕복한다() {
        let s = sys();
        let r = issue(&s, 3600, 2048, 1_000, &req()).expect("issue");
        assert_eq!(r.expires_in, 3600);
        let c = verify_user(&s, 1_100, &r.token).expect("verify");
        assert_eq!(c.sub, "u1");
        assert_eq!(c.permission(), Permission::default(), "★부재는 전부 허용");
    }

    #[test]
    fn 계정이_못_내는_종류는_2005_다() {
        let s = sys();
        let mut q = req();
        q.api_key = "ox_k_limited".into();
        q.api_secret = "ox_s_limited".into();
        q.participant_type = 1;
        assert_eq!(issue(&s, 3600, 2048, 0, &q).unwrap_err().0, Code::ClaimNotAllowed);
    }

    #[test]
    fn 모르는_종류는_형이_아니다() {
        // ★`1002`(형) 와 `2005`(허용 밖)를 가른다.
        let s = sys();
        let mut q = req();
        q.participant_type = 9;
        assert_eq!(issue(&s, 3600, 2048, 0, &q).unwrap_err().0, Code::InvalidPayload);
        q.participant_type = 2; // 닫힌 집합 안이지만 이 계정은 못 낸다
        assert_eq!(issue(&s, 3600, 2048, 0, &q).unwrap_err().0, Code::ClaimNotAllowed);
    }

    #[test]
    fn 투명은_허용된_계정만_낸다() {
        let s = sys();
        let mut q = req();
        q.api_key = "ox_k_limited".into();
        q.api_secret = "ox_s_limited".into();
        q.hidden = true;
        assert_eq!(issue(&s, 3600, 2048, 0, &q).unwrap_err().0, Code::ClaimNotAllowed);
    }

    #[test]
    fn metadata_상한은_발급에서_막는다() {
        let s = sys();
        let mut q = req();
        q.metadata = Some(serde_json::json!({ "name": "가".repeat(4000) }));
        // ★방 안에서 걸리면 고칠 자리가 없다 — 발급이 유일한 검사점이다.
        assert_eq!(issue(&s, 3600, 2048, 0, &q).unwrap_err().0, Code::InvalidPayload);
    }

    #[test]
    fn 없는_키와_틀린_비밀은_같은_답이다() {
        let s = sys();
        let mut a = req();
        a.api_key = "없는키".into();
        let mut b = req();
        b.api_secret = "틀림".into();
        assert_eq!(issue(&s, 3600, 2048, 0, &a).unwrap_err().0, Code::InvalidApiKey);
        assert_eq!(issue(&s, 3600, 2048, 0, &b).unwrap_err().0, Code::InvalidApiKey);
    }

    #[test]
    fn 만료는_2003_이다() {
        let s = sys();
        let r = issue(&s, 10, 2048, 1_000, &req()).expect("issue");
        assert_eq!(verify_user(&s, 2_000, &r.token).unwrap_err().0, Code::TokenExpired);
    }

    #[test]
    fn 앱_토큰으로는_운영이_안_열린다() {
        // ★평면이 갈리는 자리 — A 평면 비밀로 서명된 것은 계정 비밀 검증을 통과할 수 없다.
        let s = sys();
        let r = issue(&s, 3600, 2048, 0, &req()).expect("issue");
        let e = verify_account(&s, AccountScope::Ops, 10, &r.token).unwrap_err();
        assert!(matches!(e.0, Code::InvalidApiKey | Code::TokenInvalid), "{:?}", e.0);
    }

    #[test]
    fn 운영_토큰은_계정_비밀로_선다() {
        let s = sys();
        let c = AccountClaims { iss: "ox_k_demo".into(), ops: true, hook: false, iat: 0, exp: 9_999 };
        let t = sign_account("ox_s_demo", &c);
        assert!(verify_account(&s, AccountScope::Ops, 10, &t).is_ok());
        // ★`ops` 토큰으로 `hook` 자리를 열지 못한다.
        assert_eq!(
            verify_account(&s, AccountScope::Hook, 10, &t).unwrap_err().0,
            Code::NotAuthorized
        );
    }

    #[test]
    fn 계정_허용을_내리면_옛_토큰이_막힌다() {
        // ★claim 만 보면 계정을 내려도 옛 토큰이 산다 — 둘 다 봐야 회수가 성립한다.
        let s = sys();
        let c = AccountClaims { iss: "ox_k_limited".into(), ops: true, hook: false, iat: 0, exp: 9_999 };
        let t = sign_account("ox_s_limited", &c);
        assert_eq!(
            verify_account(&s, AccountScope::Ops, 10, &t).unwrap_err().0,
            Code::NotAuthorized
        );
    }

    #[test]
    fn 없는_iss_는_2004_다() {
        let s = sys();
        let c = AccountClaims { iss: "없음".into(), ops: true, hook: false, iat: 0, exp: 9_999 };
        let t = sign_account("아무거나", &c);
        assert_eq!(verify_account(&s, AccountScope::Ops, 10, &t).unwrap_err().0, Code::InvalidApiKey);
    }

    #[test]
    fn 계정_토큰에는_sub_가_없다() {
        let c = AccountClaims { iss: "k".into(), ops: true, hook: false, iat: 0, exp: 1 };
        let json = serde_json::to_string(&c).expect("ser");
        assert!(!json.contains("sub"), "★사람의 신원이 아니라 계정 자격이다");
        assert!(!json.contains("hook"), "★안 선 자격은 안 싣는다");
    }
}
