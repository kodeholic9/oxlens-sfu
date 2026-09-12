// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§3-1 · §3-2 · §3-3 · §3-5 · §16-1-2 · model: claude-opus-5

//! 세션 — ★**인증 두 갈래, `session_id` 가 이긴다.**
//!
//! ★**판정만 한다** — 소켓도 시계도 주인이 쥔다. 그래야 갈래 전량을 1층에서 태운다.

use oxsig::body::session::PcMode;
use oxsig::{Code, Permission};

/// 세션 하나(정§3-1). ★**토큰은 붙을 때만 본다** — 접속 중 재검사가 없다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// ★**이어받아지는 동안 불변**이다.
    pub id: String,
    pub user_id: String,
    pub participant_type: u8,
    pub hidden: bool,
    /// ★**방 입장 때 `RoomMember.permission` 의 씨앗일 뿐**이다 — 갱신은 방 단위다.
    pub permission: Permission,
    pub pc_mode: PcMode,
    /// ★**이어받은 세션인가** — `RESUME` 이 이것을 본다.
    pub resumed: bool,
    /// 소켓이 죽은 순간. 살아 있으면 `None`.
    pub dead_at: Option<u64>,
}

/// `BIND` 가 내는 답.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindOutcome {
    /// 판정 1 — ★**그 세션으로 인증. 토큰을 보지 않는다.** 같은 `session_id` 를 에코한다.
    Resumed { session_id: String },
    /// 판정 2 — 새 세션.
    Fresh { session_id: String },
    /// 판정 3 — 새 세션이고 ★**같은 신원의 산 세션을 축출**한다(옛 연결에 `LEAVE` `2010`).
    FreshEvicting { session_id: String, evicted: String },
    /// 실패 — 그 코드로 답한다.
    Rejected(Code),
}

/// 세션 창고. ★**창 안에서는 죽은 세션도 산다**(이어받기의 재료다).
#[derive(Debug, Default)]
pub struct Sessions {
    items: Vec<Session>,
    seq: u64,
    /// ★**방금 축출된 것** — 통보할 대상을 잃지 않으려고 한 걸음 들고 있는다.
    last_evicted: Vec<(String, String)>,
}

/// `BIND` 에 실려 온 것 중 판정에 쓰는 것.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindInput {
    pub session_id: Option<String>,
    /// 토큰 검증 결과 — ★**판정 1 에서는 쓰지 않는다**(토큰을 안 본다).
    pub verified: Option<VerifiedToken>,
    /// 토큰 검증이 실패했으면 그 코드.
    pub token_error: Option<Code>,
    pub pc_mode: Option<PcMode>,
    pub client_ver: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedToken {
    pub user_id: String,
    pub participant_type: u8,
    pub hidden: bool,
    pub permission: Permission,
}

/// 이 규격서가 받는 세대.
pub const SERVER_VER: u32 = 1;

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_id(&mut self) -> String {
        self.seq += 1;
        format!("s-{}", self.seq)
    }

    pub fn get(&self, id: &str) -> Option<&Session> {
        self.items.iter().find(|s| s.id == id)
    }

    /// 그 사람의 세션들 — ★**unicast 통지의 대상**이다(정§17-2 ⑧).
    pub fn sessions_of(&self, user_id: &str) -> Vec<String> {
        self.items.iter().filter(|s| s.user_id == user_id).map(|s| s.id.clone()).collect()
    }

    pub fn live_of(&self, user_id: &str) -> Option<&Session> {
        self.items.iter().find(|s| s.user_id == user_id && s.dead_at.is_none())
    }

    /// 창이 지난 세션을 거둔다. ★**창 안에서는 거두지 않는다** — 그것이 이어받기의 전부다.
    pub fn sweep(&mut self, now: u64, window_ms: u64) -> Vec<String> {
        let gone: Vec<String> = self
            .items
            .iter()
            .filter(|s| s.dead_at.is_some_and(|t| now.saturating_sub(t) > window_ms))
            .map(|s| s.id.clone())
            .collect();
        self.items.retain(|s| !gone.contains(&s.id));
        gone
    }

    /// 소켓이 죽었다 — ★**세션은 창 동안 산다.**
    pub fn on_socket_dead(&mut self, id: &str, now: u64) {
        if let Some(s) = self.items.iter_mut().find(|s| s.id == id) {
            s.dead_at = Some(now);
        }
    }

    /// ★**즉시 폐기 — 창을 잡지 않는다.** 운영자 절단(`cut`)과 클라 `LEAVE` 둘뿐이다.
    ///
    /// 창을 잡아 두면 클라가 `RESUME` 으로 그대로 돌아오고, ★`RESUME` 은 토큰을 안 보므로
    /// ★**`cut` 이 아무것도 못 시킨다.**
    pub fn discard(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|s| s.id != id);
        before != self.items.len()
    }

    /// `BIND` 판정(정§3-2). ★**순서가 계약이다** — `session_id` 를 먼저 본다.
    pub fn bind(&mut self, input: &BindInput, now: u64, window_ms: u64) -> BindOutcome {
        // 5. 세대 — 미송신은 `1`.
        if input.client_ver.unwrap_or(1) != SERVER_VER {
            return BindOutcome::Rejected(Code::VersionMismatch);
        }
        // 1. ★창 안 + 서버 생존이면 그 세션으로 인증한다 — ★**토큰을 보지 않는다.**
        //    이것을 빼먹고 토큰부터 보면 종일 켠 단말의 순간 재접속이 전부 `2003` 이 된다.
        if let Some(want) = &input.session_id
            && let Some(s) = self.items.iter_mut().find(|s| &s.id == want)
            && s.dead_at.is_none_or(|t| now.saturating_sub(t) <= window_ms)
        {
            s.dead_at = None;
            s.resumed = true;
            return BindOutcome::Resumed { session_id: s.id.clone() };
        }
        // 2. 토큰 갈래.
        let v = match (&input.verified, input.token_error) {
            (Some(v), _) => v.clone(),
            (None, Some(code)) => return BindOutcome::Rejected(code),
            (None, None) => return BindOutcome::Rejected(Code::TokenInvalid),
        };
        // 4. 모르는 `pc_mode` — ★**조용한 기본값 금지.** 부재는 `2pc`.
        let pc_mode = input.pc_mode.unwrap_or_default();
        // 3. 같은 신원의 산 세션은 축출한다.
        let evicted = self.live_of(&v.user_id).map(|s| s.id.clone());
        let id = self.next_id();
        self.items.push(Session {
            id: id.clone(),
            user_id: v.user_id,
            participant_type: v.participant_type,
            hidden: v.hidden,
            permission: v.permission,
            pc_mode,
            resumed: false,
            dead_at: None,
        });
        match evicted {
            Some(old) => {
                // 옛 세션은 통보만 하고 남기지 않는다 — ★**유령으로 두면 명단에 같은 사람이 둘.**
                self.discard(&old);
                let who = self.items.iter().find(|s| s.id == id).map(|s| s.user_id.clone());
                if let Some(w) = who {
                    self.last_evicted.retain(|(u, _)| u != &w);
                    self.last_evicted.push((w, old.clone()));
                }
                BindOutcome::FreshEvicting { session_id: id, evicted: old }
            }
            None => BindOutcome::Fresh { session_id: id },
        }
    }

    /// 방금 축출된 세션 id — `bind` 가 기록해 둔 것을 ★**꺼내 온다(비운다).**
    ///
    /// ★★**장부가 아니라 한 걸음 들고 있는 것이다.** 안 비우면 옛 축출 기록이 다음 판정에
    /// 섞여, 이어받기(판정 1)가 ★**이미 죽은 옛 소켓을 닫으라고 답한다** — 그러면 지금
    /// 살아 있는 옛 소켓은 안 닫히고 `LEAVE` 도 못 받는다(실측 20260912: 한 hub 에
    /// 같은 `user_id` 가 두 번째로 붙는 순간부터 축출 통지가 사라졌다).
    pub fn take_evicted(&mut self, user_id: &str, except: &str) -> Option<String> {
        let i = self.last_evicted.iter().position(|(u, s)| u == user_id && s != except)?;
        Some(self.last_evicted.remove(i).1)
    }

    /// `RESUME` 판정(정§3-3). ★**`2008` 이 유일한 실패다** — 부분 실패 응답 형이 없다.
    pub fn may_resume(&self, id: &str) -> Result<(), Code> {
        match self.get(id) {
            // ★새 세션에 `RESUME` = 클라 버그 가드.
            Some(s) if s.resumed => Ok(()),
            _ => Err(Code::SessionNotFound),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: u64 = 60_000;

    fn tok(user: &str) -> VerifiedToken {
        VerifiedToken {
            user_id: user.into(),
            participant_type: 0,
            hidden: false,
            permission: Permission::default(),
        }
    }

    fn fresh(user: &str) -> BindInput {
        BindInput {
            session_id: None,
            verified: Some(tok(user)),
            token_error: None,
            pc_mode: None,
            client_ver: None,
        }
    }

    #[test]
    fn 순간_재접속은_토큰을_안_본다() {
        // ★1을 빼먹고 토큰부터 보면 종일 켠 단말이 전부 2003 이다.
        let mut s = Sessions::new();
        let BindOutcome::Fresh { session_id } = s.bind(&fresh("u1"), 0, WINDOW) else {
            panic!("fresh")
        };
        s.on_socket_dead(&session_id, 1_000);
        let again = BindInput {
            session_id: Some(session_id.clone()),
            verified: None,
            // ★토큰이 만료돼도 판정 1 은 그것을 보지 않는다.
            token_error: Some(Code::TokenExpired),
            pc_mode: None,
            client_ver: None,
        };
        assert_eq!(
            s.bind(&again, 2_000, WINDOW),
            BindOutcome::Resumed { session_id: session_id.clone() }
        );
        assert!(s.may_resume(&session_id).is_ok());
    }

    #[test]
    fn 창_밖이면_토큰_갈래로_간다() {
        let mut s = Sessions::new();
        let BindOutcome::Fresh { session_id } = s.bind(&fresh("u1"), 0, WINDOW) else {
            panic!("fresh")
        };
        s.on_socket_dead(&session_id, 1_000);
        let late = BindInput {
            session_id: Some(session_id),
            verified: None,
            token_error: Some(Code::TokenExpired),
            pc_mode: None,
            client_ver: None,
        };
        assert_eq!(s.bind(&late, 1_000 + WINDOW + 1, WINDOW), BindOutcome::Rejected(Code::TokenExpired));
    }

    #[test]
    fn 같은_신원은_축출된다() {
        let mut s = Sessions::new();
        let BindOutcome::Fresh { session_id: first } = s.bind(&fresh("u1"), 0, WINDOW) else {
            panic!("fresh")
        };
        let out = s.bind(&fresh("u1"), 10, WINDOW);
        let BindOutcome::FreshEvicting { evicted, .. } = out else { panic!("축출이어야 한다") };
        assert_eq!(evicted, first);
        // ★유령으로 두면 명단에 같은 사람이 둘이다.
        assert!(s.get(&first).is_none());
    }

    #[test]
    fn 새_세션에_resume_은_2008_이다() {
        let mut s = Sessions::new();
        let BindOutcome::Fresh { session_id } = s.bind(&fresh("u1"), 0, WINDOW) else {
            panic!("fresh")
        };
        assert_eq!(s.may_resume(&session_id), Err(Code::SessionNotFound));
    }

    #[test]
    fn cut_은_창을_안_잡는다() {
        // ★창을 잡으면 RESUME 으로 그대로 돌아온다 — cut 이 아무것도 못 시킨다.
        let mut s = Sessions::new();
        let BindOutcome::Fresh { session_id } = s.bind(&fresh("u1"), 0, WINDOW) else {
            panic!("fresh")
        };
        assert!(s.discard(&session_id));
        let back = BindInput {
            session_id: Some(session_id),
            verified: Some(tok("u1")),
            token_error: None,
            pc_mode: None,
            client_ver: None,
        };
        // 판정 1 이 무효 → 2 로 간다(토큰 재검사). ★그것이 `cut` 의 목적이다.
        assert!(matches!(s.bind(&back, 10, WINDOW), BindOutcome::Fresh { .. }));
    }

    #[test]
    fn 세대가_다르면_1004_다() {
        let mut s = Sessions::new();
        let mut i = fresh("u1");
        i.client_ver = Some(2);
        assert_eq!(s.bind(&i, 0, WINDOW), BindOutcome::Rejected(Code::VersionMismatch));
    }

    #[test]
    fn 창이_지나면_거둔다() {
        let mut s = Sessions::new();
        let BindOutcome::Fresh { session_id } = s.bind(&fresh("u1"), 0, WINDOW) else {
            panic!("fresh")
        };
        s.on_socket_dead(&session_id, 100);
        assert!(s.sweep(100 + WINDOW, WINDOW).is_empty(), "★창 안에서는 안 거둔다");
        assert_eq!(s.sweep(100 + WINDOW + 1, WINDOW), vec![session_id]);
    }

    #[test]
    fn pc_mode_부재는_2pc_다() {
        let mut s = Sessions::new();
        let BindOutcome::Fresh { session_id } = s.bind(&fresh("u1"), 0, WINDOW) else {
            panic!("fresh")
        };
        assert_eq!(s.get(&session_id).expect("s").pc_mode, PcMode::Two);
    }
}
