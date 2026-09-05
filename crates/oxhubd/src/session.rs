// author: kodeholic (powered by Claude)
//! 세션 — 정§3-1·§3-2. `BIND` 인증 두 갈래(`session_id` 가 이긴다) · 축출(4005) · `resume_window_ms` 생존 창.

use std::time::{Duration, Instant};

use common::auth::{self, VerifyError};
use dashmap::DashMap;
use oxsig::body::session::{BindReq, BindRes, CLIENT_VER};
use oxsig::schema::PcMode;
use oxsig::{CloseCode, FailCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attach {
    Attached { conn_id: u64 },
    Detached { since: Instant },
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub role: String,
    pub floor_priority: u8,
    pub pc_mode: PcMode,
    pub attach: Attach,
    /// 재접속 `BIND` 가 이어받았다 — 다음 `RESUME` 하나가 유효하다(정§3-3 1).
    pub resume_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindOutcome {
    pub res: BindRes,
    pub session_id: String,
    pub resumed: bool,
    /// 축출(4005)하거나 대체(4003)할 옛 연결.
    pub close_old: Option<(u64, CloseCode)>,
}

pub struct SessionRegistry {
    by_id: DashMap<String, Session>,
    by_user: DashMap<String, String>,
    secret: String,
    window: Duration,
    heartbeat_interval_ms: u64,
}

impl SessionRegistry {
    pub fn new(secret: impl Into<String>, window: Duration, heartbeat_interval_ms: u64) -> Self {
        Self { by_id: DashMap::new(), by_user: DashMap::new(), secret: secret.into(), window, heartbeat_interval_ms }
    }

    pub fn window(&self) -> Duration {
        self.window
    }

    fn alive(&self, s: &Session, now: Instant) -> bool {
        match s.attach {
            Attach::Attached { .. } => true,
            Attach::Detached { since } => now.duration_since(since) <= self.window,
        }
    }

    /// 정§3-2 판정 순서 1~5.
    pub fn bind(&self, req: &BindReq, conn_id: u64, now: Instant) -> Result<BindOutcome, FailCode> {
        if req.client_ver != CLIENT_VER {
            return Err(FailCode::VersionMismatch);
        }
        // 1. session_id 유효 → 그 세션. 토큰은 보지 않는다.
        if let Some(sid) = req.session_id.as_deref()
            && let Some(mut s) = self.by_id.get_mut(sid)
            && self.alive(&s, now)
        {
            let old = match s.attach {
                Attach::Attached { conn_id: old } if old != conn_id => Some((old, CloseCode::HeartbeatTimeout)),
                _ => None,
            };
            s.attach = Attach::Attached { conn_id };
            s.resume_pending = true;
            return Ok(BindOutcome { res: self.res_of(&s), session_id: s.id.clone(), resumed: true, close_old: old });
        }
        // 2. 토큰 검증 → 새 세션.
        let claims = auth::verify(&self.secret, &req.token).map_err(|e| match e {
            VerifyError::Expired => FailCode::TokenExpired,
            VerifyError::Invalid => FailCode::TokenInvalid,
        })?;
        if !auth::is_known_role(&claims.role) {
            return Err(FailCode::InvalidRole);
        }
        // 3. 같은 신원의 산 세션 → 축출(실패가 아니다).
        let mut close_old = None;
        if let Some(old_sid) = self.by_user.get(&claims.sub).map(|r| r.clone())
            && let Some((_, old)) = self.by_id.remove(&old_sid)
            && let Attach::Attached { conn_id: old_conn } = old.attach
        {
            close_old = Some((old_conn, CloseCode::DuplicateSession));
        }
        let session = Session {
            id: uuid::Uuid::new_v4().simple().to_string(),
            user_id: claims.sub.clone(),
            role: claims.role.clone(),
            floor_priority: claims.floor_priority,
            pc_mode: req.pc_mode,
            attach: Attach::Attached { conn_id },
            resume_pending: false,
        };
        let res = self.res_of(&session);
        let sid = session.id.clone();
        self.by_user.insert(session.user_id.clone(), sid.clone());
        self.by_id.insert(sid.clone(), session);
        Ok(BindOutcome { res, session_id: sid, resumed: false, close_old })
    }

    fn res_of(&self, s: &Session) -> BindRes {
        BindRes {
            user_id: s.user_id.clone(),
            role: s.role.clone(),
            server_ver: CLIENT_VER,
            heartbeat_interval: self.heartbeat_interval_ms,
            session_id: s.id.clone(),
            resume_window_ms: u64::try_from(self.window.as_millis()).unwrap_or(u64::MAX),
            pc_mode: s.pc_mode,
        }
    }

    /// 정§3-3 1 — 이어받은 세션이 아니면 `2008`. 한 번 쓰면 닫힌다.
    pub fn take_resume(&self, session_id: &str) -> Result<(), FailCode> {
        let mut s = self.by_id.get_mut(session_id).ok_or(FailCode::SessionNotFound)?;
        if !s.resume_pending {
            return Err(FailCode::SessionNotFound);
        }
        s.resume_pending = false;
        Ok(())
    }

    pub fn get(&self, session_id: &str) -> Option<Session> {
        self.by_id.get(session_id).map(|s| s.clone())
    }

    /// 소켓이 죽었다 — 세션은 창 동안 산다. 다른 연결이 이미 이어받았으면 건드리지 않는다.
    pub fn detach(&self, session_id: &str, conn_id: u64, now: Instant) {
        if let Some(mut s) = self.by_id.get_mut(session_id)
            && s.attach == (Attach::Attached { conn_id })
        {
            s.attach = Attach::Detached { since: now };
            s.resume_pending = false;
        }
    }

    /// 창을 넘긴 세션을 지운다 — 지운 뒤의 `RESUME` 은 `2008`.
    pub fn reap(&self, now: Instant) -> Vec<Session> {
        let dead: Vec<Session> = self.by_id.iter().filter(|s| !self.alive(s, now)).map(|s| s.clone()).collect();
        for s in &dead {
            self.by_id.remove(&s.id);
            self.by_user.remove_if(&s.user_id, |_, sid| sid == &s.id);
        }
        dead
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> SessionRegistry {
        SessionRegistry::new("s", Duration::from_secs(60), 10_000)
    }
    fn token(user: &str, ttl: u64, back: i64) -> String {
        auth::issue("s", user, auth::ROLE_USER, 3, None, ttl, auth::now_unix() - back).unwrap().token
    }
    fn req(token: &str, sid: Option<&str>) -> BindReq {
        BindReq { token: token.to_owned(), session_id: sid.map(str::to_owned), client_ver: 1, pc_mode: PcMode::OnePc }
    }

    #[test]
    fn two_branches_and_eviction() {
        let r = reg();
        let now = Instant::now();
        let first = r.bind(&req(&token("u1", 60, 0), None), 1, now).unwrap();
        assert!(!first.resumed && first.close_old.is_none());
        assert_eq!(first.res.pc_mode, PcMode::OnePc);
        assert_eq!(r.get(&first.session_id).unwrap().floor_priority, 3);
        // 같은 신원 새 BIND(session_id 없음) → 성공 + 옛 연결 4005
        let dup = r.bind(&req(&token("u1", 60, 0), None), 2, now).unwrap();
        assert_eq!(dup.close_old, Some((1, CloseCode::DuplicateSession)));
        assert_ne!(dup.session_id, first.session_id);
        assert_eq!(r.len(), 1);
        // 끊긴 뒤 창 안 재접속 — 만료 토큰이어도 session_id 가 이긴다
        r.detach(&dup.session_id, 2, now);
        let expired = token("u1", 1, 600);
        let back = r.bind(&req(&expired, Some(&dup.session_id)), 3, now + Duration::from_secs(30)).unwrap();
        assert!(back.resumed);
        assert_eq!(back.session_id, dup.session_id);
        assert_eq!(r.take_resume(&dup.session_id), Ok(()));
        assert_eq!(r.take_resume(&dup.session_id), Err(FailCode::SessionNotFound));
        // 창을 넘기면 session_id 무효 → 토큰으로 → 만료면 2003
        r.detach(&dup.session_id, 3, now);
        let late = now + Duration::from_secs(61);
        assert_eq!(r.bind(&req(&expired, Some(&dup.session_id)), 4, late).unwrap_err(), FailCode::TokenExpired);
        assert_eq!(r.reap(late).len(), 1);
        assert!(r.is_empty());
    }

    #[test]
    fn invalid_inputs() {
        let r = reg();
        let now = Instant::now();
        assert_eq!(r.bind(&req("garbage", None), 1, now).unwrap_err(), FailCode::TokenInvalid);
        let mut v = req(&token("u1", 60, 0), None);
        v.client_ver = 2;
        assert_eq!(r.bind(&v, 1, now).unwrap_err(), FailCode::VersionMismatch);
        assert_eq!(r.take_resume("nope"), Err(FailCode::SessionNotFound));
    }
}
