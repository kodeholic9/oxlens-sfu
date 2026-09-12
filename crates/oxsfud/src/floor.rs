// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§9-1 · §9-2 · §9-3 · §9-6 · 연§8-4 · §11-3 · §11-4 · model: claude-opus-5

//! 발언권 — ★★**전이표가 곧 시험이다.**
//!
//! ★**판정만 한다** — 시계도 소켓도 주인이 쥔다. `PendingRevoke`·`T3`·`T4` 는 미채택이라
//! 상태는 ★**둘뿐**이고, 회수는 ★**같은 임계 구역에서 `Idle` → 승계까지** 간다.

use std::collections::BTreeMap;

/// 거절·회수 사유(연§11-3). ★**번호가 곧 처방**이다.
pub mod cause {
    /// 방에 나뿐.
    pub const ALONE: u8 = 3;
    /// 청취 전용 — 반이중 발행 트랙이 없다.
    pub const RECEIVE_ONLY: u8 = 5;
    /// 상한 초과로 회수했다(`T2`). ★이 사유만 `T9` 를 건다.
    pub const MAX_DURATION: u8 = 2;
    /// 선점당했다.
    pub const PREEMPTED: u8 = 4;
    /// ★**권한 비트가 내려갔다** — 재요청이 무의미하다.
    pub const NO_PERMISSION: u8 = 102;
    /// 그 방이 내 `pub_room` 이 아니다.
    pub const NOT_PUB_ROOM: u8 = 101;
    /// `Retry-after` 가 안 끝났다.
    pub const RETRY_AFTER: u8 = 4;
}

/// 연§8-4 값. ★**정본은 그 문서 하나**이고 여기는 쓰는 자리다.
pub mod timers {
    /// RTP 가 끊긴 것으로 보는 시간.
    pub const T1_MS: u64 = 4_000;
    /// 발화 상한(정책 손잡이 `floor.t2_stop_talking_secs`).
    pub const T2_MS: u64 = 30_000;
    /// `IDLE` 재전송 주기·횟수.
    pub const T7_MS: u64 = 1_000;
    pub const C7: u8 = 10;
    /// `REVOKE` 재전송 — ★**한 번뿐**이다.
    pub const T8_MS: u64 = 1_000;
    pub const C8: u8 = 1;
    /// 큐 승계 `GRANTED` 재전송.
    pub const T20_MS: u64 = 1_000;
    pub const C20: u8 = 3;
    /// 사유 `2` 회수자만 걸리는 재요청 금지.
    pub const T9_MS: u64 = 3_000;
    /// 판정 해상도 — ★상한 초과를 최대 이만큼 늦게 안다.
    pub const TICK_MS: u64 = 2_000;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Idle,
    Taken {
        speaker: String,
        priority: u8,
        /// ★**`T2` 의 시작은 허가가 아니라 첫 RTP 다.**
        first_rtp_at: Option<u64>,
        last_rtp_at: u64,
        max_burst_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Waiting {
    user_id: String,
    priority: u8,
    enqueued_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Revoked {
    cause: u8,
    deadline: u64,
    resent: bool,
}

/// 서버가 내보낼 것. ★**값이다** — 콜백을 주입하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    /// 당사자 unicast.
    Granted { user_id: String, priority: u8, remaining_ms: u64 },
    Deny { user_id: String, cause: u8 },
    Revoke { user_id: String, cause: u8 },
    /// ★`priority` 는 ★**지금 허가된 사람의 우선순위**다(연§11-3 TLV `3` byte1) —
    /// 대기자가 *"내가 끼어들 수 있나"* 를 스스로 판단하는 재료다.
    QueueInfo { user_id: String, position: u8, size: u8, priority: u8 },
    /// 그 방 청취자 broadcast — ★**화자는 제외한다.**
    Taken { speaker: String, seq: u16 },
    /// ★`prev` 는 ★**직전에 말한 사람**이다(연§11-3 `0x1A`) — 화면이 *"방금 누가 말했나"* 를
    /// 지울지 남길지 그것으로 정한다. ★**처음부터 조용한 방이면 없다**(지어내지 않는다).
    Idle { seq: u16, prev: Option<String> },
}

/// 요청이 들고 온 것.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub user_id: String,
    /// ★**요청이 실은 값이 전부다** — 서버가 깎지 않는다.
    pub priority: u8,
    /// 요청한 발화 시간 — ★**안 실었으면 `None`** 이고, 그것은 *"서버 상한만큼"* 이다.
    ///
    /// ★★**`0` 을 부재의 표식으로 쓰지 않는다** — `0` 은 *"0초만 말하겠다"* 라는 값이고,
    /// 그렇게 두면 시간을 안 실은 요청이 ★**허가 즉시 만료되는 발언권**을 받는다(무음).
    pub want_ms: Option<u64>,
    /// 그 방이 이 사람의 `pub_room` 인가.
    pub in_pub_room: bool,
    /// 반이중 발행 트랙이 있나.
    pub has_half_track: bool,
    /// `floor_request` 비트(와 그 kind 의 발행 비트).
    pub allowed: bool,
    /// 방에 나 말고 누가 있나.
    pub others_present: bool,
}

pub const QUEUE_MAX: usize = 10;

#[derive(Debug)]
pub struct Floor {
    pub state: State,
    queue: Vec<Waiting>,
    /// ★방마다. `IDLE`·`TAKEN` 을 보낼 때마다 1 증가한다 — ★**재전송도 센다.**
    seq: u16,
    retry_after: BTreeMap<String, u64>,
    revoked: BTreeMap<String, Revoked>,
    max_burst_ms: u64,
}

impl Floor {
    pub fn new(max_burst_ms: u64) -> Self {
        Self {
            state: State::Idle,
            queue: Vec::new(),
            seq: 0,
            retry_after: BTreeMap::new(),
            revoked: BTreeMap::new(),
            max_burst_ms,
        }
    }

    fn bump(&mut self) -> u16 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    pub fn speaker(&self) -> Option<&str> {
        match &self.state {
            State::Taken { speaker, .. } => Some(speaker.as_str()),
            State::Idle => None,
        }
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    fn sort_queue(&mut self) {
        // ★우선순위 DESC → 넣은 순서 ASC. 같은 단 안에서는 먼저 온 사람이 먼저다.
        self.queue.sort_by(|a, b| {
            b.priority.cmp(&a.priority).then_with(|| a.enqueued_at.cmp(&b.enqueued_at))
        });
    }

    fn queue_notices(&self) -> Vec<Out> {
        // ★**한 바이트에 담긴다**(연§11-3) — 큐 상한이 10 이라 넘칠 자리가 없다.
        let size = self.queue.len().min(u8::MAX as usize) as u8;
        let granted = match &self.state {
            State::Taken { priority, .. } => *priority,
            _ => 0,
        };
        self.queue
            .iter()
            .enumerate()
            .map(|(i, w)| Out::QueueInfo {
                user_id: w.user_id.clone(),
                // ★순번은 1부터다.
                position: (i + 1).min(u8::MAX as usize) as u8,
                size,
                priority: granted,
            })
            .collect()
    }

    fn grant(&mut self, user_id: String, priority: u8, want_ms: Option<u64>, now: u64) -> Vec<Out> {
        // ★**상한과 견준다** — 부재는 상한 그것이다(서버가 깎는 자리는 여기 하나).
        let burst = want_ms.unwrap_or(self.max_burst_ms).min(self.max_burst_ms);
        self.state = State::Taken {
            speaker: user_id.clone(),
            priority,
            first_rtp_at: None,
            last_rtp_at: now,
            max_burst_ms: burst,
        };
        let seq = self.bump();
        vec![
            Out::Granted { user_id: user_id.clone(), priority, remaining_ms: burst },
            // ★화자 제외 broadcast — 안 빼면 자기 목소리가 되돌아온 것처럼 보인다.
            Out::Taken { speaker: user_id, seq },
        ]
    }

    /// `Idle` 로 가면서 ★**큐가 있으면 `IDLE` 을 보내지 않고 곧바로 승계**한다.
    ///
    /// 보내면 화면이 *"아무도 안 말함"* 으로 깜빡였다가 다음 화자로 바뀐다.
    fn idle_or_succeed(&mut self, now: u64) -> Vec<Out> {
        self.sort_queue();
        if self.queue.is_empty() {
            // ★**비우기 전에 읽는다** — 지운 뒤에 물으면 직전 화자를 영영 모른다.
            let prev = self.speaker().map(str::to_string);
            self.state = State::Idle;
            let seq = self.bump();
            return vec![Out::Idle { seq, prev }];
        }
        let w = self.queue.remove(0);
        // ★대기에서 올라오는 사람은 ★**상한만큼** 받는다 — 대기표가 처음 요청의 시간을
        //   들고 있지 않다(들게 하면 오래 기다린 요청의 값이 낡는다).
        let mut out = self.grant(w.user_id, w.priority, None, now);
        out.extend(self.queue_notices());
        out
    }

    /// `FLOOR_REQUEST`.
    pub fn on_request(&mut self, req: &Request, now: u64) -> Vec<Out> {
        let deny = |c: u8| vec![Out::Deny { user_id: req.user_id.clone(), cause: c }];
        // ★관문 순서가 계약이다 — 자격이 먼저고 상태는 그 뒤다.
        if !req.allowed {
            // ★긴급도 같다 — 우회 경로가 없다.
            return deny(cause::NO_PERMISSION);
        }
        if !req.in_pub_room {
            return deny(cause::NOT_PUB_ROOM);
        }
        if !req.has_half_track {
            return deny(cause::RECEIVE_ONLY);
        }
        if !req.others_present {
            return deny(cause::ALONE);
        }
        if self.retry_after.get(&req.user_id).is_some_and(|t| now < *t) {
            return deny(cause::RETRY_AFTER);
        }
        match self.state.clone() {
            State::Idle => self.grant(req.user_id.clone(), req.priority, req.want_ms, now),
            State::Taken { speaker, priority, first_rtp_at, max_burst_ms, .. }
                if speaker == req.user_id =>
            {
                // ★같은 사람의 재요청 — ★**시간엔 `T2` 남은 값**을 싣고 RTP 시각은 갱신하지 않는다.
                let used = first_rtp_at.map(|t| now.saturating_sub(t)).unwrap_or(0);
                vec![Out::Granted {
                    user_id: req.user_id.clone(),
                    priority,
                    remaining_ms: max_burst_ms.saturating_sub(used),
                }]
            }
            State::Taken { speaker, priority, .. } if req.priority > priority => {
                // ★★**선점은 한 걸음이다** — 회수·허가·통지가 한 임계 구역에서 난다.
                //   선점자는 ★**큐를 거치지 않는다**(`QUEUE_INFO` 가 가면 안 된다).
                let mut out = vec![Out::Revoke { user_id: speaker.clone(), cause: cause::PREEMPTED }];
                self.revoked.insert(
                    speaker,
                    Revoked { cause: cause::PREEMPTED, deadline: now + timers::T8_MS, resent: false },
                );
                out.extend(self.grant(req.user_id.clone(), req.priority, req.want_ms, now));
                out
            }
            State::Taken { .. } => {
                if self.queue.iter().any(|w| w.user_id == req.user_id) {
                    // ★이미 있으면 현 순번만 다시 알린다 — 중복 등록하지 않는다.
                    self.sort_queue();
                    return self
                        .queue_notices()
                        .into_iter()
                        .filter(|o| matches!(o, Out::QueueInfo { user_id, .. } if user_id == &req.user_id))
                        .collect();
                }
                if self.queue.len() >= QUEUE_MAX {
                    return deny(7);
                }
                self.queue.push(Waiting {
                    user_id: req.user_id.clone(),
                    priority: req.priority,
                    enqueued_at: now,
                });
                self.sort_queue();
                self.queue_notices()
            }
        }
    }

    /// `FLOOR_RELEASE`. ★**멱등이다** — 화자가 아니면 대기 취소이고, 그도 아니면 무시다.
    pub fn on_release(&mut self, user_id: &str, now: u64) -> Vec<Out> {
        // ★회수된 사람의 `RELEASE` 는 `T8` 을 멈춘다(상태는 안 바뀐다 — 이미 넘어갔다).
        self.revoked.remove(user_id);
        match &self.state {
            State::Taken { speaker, .. } if speaker == user_id => self.idle_or_succeed(now),
            _ => {
                let had = self.queue.iter().any(|w| w.user_id == user_id);
                self.queue.retain(|w| w.user_id != user_id);
                if had { self.queue_notices() } else { Vec::new() }
            }
        }
    }

    /// `QUEUE_POS_REQUEST`. ★**큐에 없으면 계수 후 무응답** — 없는 자리를 지어 답하지 않는다.
    pub fn on_queue_pos(&mut self, user_id: &str) -> Vec<Out> {
        self.sort_queue();
        self.queue_notices()
            .into_iter()
            .filter(|o| matches!(o, Out::QueueInfo { user_id: u, .. } if u == user_id))
            .collect()
    }

    /// ★**prefan 게이트를 통과한 RTP 만 센다** — 막힌 RTP 는 발화가 아니다.
    pub fn on_rtp(&mut self, user_id: &str, now: u64) {
        if let State::Taken { speaker, first_rtp_at, last_rtp_at, .. } = &mut self.state
            && speaker == user_id
        {
            if first_rtp_at.is_none() {
                *first_rtp_at = Some(now);
            }
            *last_rtp_at = now;
        }
    }

    /// 권한 비트가 내려갔다(정§6-4 ③). ★**즉시·소급**이다.
    pub fn on_permission_lost(&mut self, user_id: &str, now: u64) -> Vec<Out> {
        match &self.state {
            State::Taken { speaker, .. } if speaker == user_id => {
                // ★화자는 `REVOKE(3)` — ★`T9` 는 걸지 않는다(사유 `2` 만 건다).
                let mut out =
                    vec![Out::Revoke { user_id: user_id.to_string(), cause: 3 }];
                self.revoked.insert(
                    user_id.to_string(),
                    Revoked { cause: 3, deadline: now + timers::T8_MS, resent: false },
                );
                out.extend(self.idle_or_succeed(now));
                out
            }
            _ if self.queue.iter().any(|w| w.user_id == user_id) => {
                // ★대기자는 큐에서 빠지고 `DENY(102)` 를 받는다 — 무통지면 `T104` 소진으로만 안다.
                self.queue.retain(|w| w.user_id != user_id);
                let mut out = vec![Out::Deny {
                    user_id: user_id.to_string(),
                    cause: cause::NO_PERMISSION,
                }];
                out.extend(self.queue_notices());
                out
            }
            // ★그 밖은 아무것도 안 한다 — 다음 요청이 `102` 다.
            _ => Vec::new(),
        }
    }

    /// 퇴장·`pub_deselect` — ★**화자든 대기자든 빠진다.**
    pub fn on_gone(&mut self, user_id: &str, now: u64) -> Vec<Out> {
        self.revoked.remove(user_id);
        self.on_release(user_id, now)
    }

    /// 시계. ★**판정 해상도는 2초**라 상한 초과를 최대 그만큼 늦게 안다.
    pub fn tick(&mut self, now: u64) -> Vec<Out> {
        let mut out = Vec::new();
        // `T8` — ★**한 번만 더** 보낸다(`C8`=1). 죽은 DC 에 영원히 재송하지 않는다.
        let due: Vec<String> = self
            .revoked
            .iter()
            .filter(|(_, r)| !r.resent && now >= r.deadline)
            .map(|(u, _)| u.clone())
            .collect();
        for u in due {
            if let Some(r) = self.revoked.remove(&u) {
                out.push(Out::Revoke { user_id: u, cause: r.cause });
            }
        }
        match self.state.clone() {
            State::Taken { speaker, first_rtp_at, last_rtp_at, max_burst_ms, .. } => {
                if let Some(start) = first_rtp_at
                    && now.saturating_sub(start) > max_burst_ms
                {
                    // ★`T2` — 상한 초과. ★**이 사유만 `T9` 를 건다.**
                    out.push(Out::Revoke {
                        user_id: speaker.clone(),
                        cause: cause::MAX_DURATION,
                    });
                    self.revoked.insert(
                        speaker.clone(),
                        Revoked {
                            cause: cause::MAX_DURATION,
                            deadline: now + timers::T8_MS,
                            resent: false,
                        },
                    );
                    self.retry_after.insert(speaker, now + timers::T9_MS);
                    out.extend(self.idle_or_succeed(now));
                } else if now.saturating_sub(last_rtp_at) > timers::T1_MS {
                    // ★`T1` — 그 허가가 끝난 것으로 본다. ★**회수 통지가 없다**(`RELEASE` 와 같다).
                    out.extend(self.idle_or_succeed(now));
                }
            }
            State::Idle => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(u: &str, p: u8) -> Request {
        Request {
            user_id: u.into(),
            priority: p,
            want_ms: Some(30_000),
            in_pub_room: true,
            has_half_track: true,
            allowed: true,
            others_present: true,
        }
    }

    fn floor() -> Floor {
        Floor::new(timers::T2_MS)
    }

    #[test]
    fn idle_은_직전_화자를_싣는다() {
        let mut f = floor();
        f.on_request(&req("a", 0), 0);
        let out = f.on_release("a", 1_000);
        // ★화면이 *"방금 누가 말했나"* 를 지울지 남길지 이 값으로 정한다.
        assert!(matches!(&out[0], Out::Idle { prev, .. } if prev.as_deref() == Some("a")));
    }

    #[test]
    fn 시간을_안_실으면_상한만큼이다() {
        let mut f = Floor::new(timers::T2_MS);
        let out = f.on_request(&Request { want_ms: None, ..req("a", 0) }, 0);
        // ★`0` 이 아니라 상한이다 — `0` 이면 허가 즉시 만료되는 발언권이 된다.
        assert!(matches!(out.first(), Some(Out::Granted { remaining_ms, .. }) if *remaining_ms == timers::T2_MS));
    }

    #[test]
    fn 요청이_상한보다_길면_깎는다() {
        let mut f = Floor::new(timers::T2_MS);
        let out = f.on_request(&Request { want_ms: Some(timers::T2_MS * 10), ..req("a", 0) }, 0);
        assert!(matches!(out.first(), Some(Out::Granted { remaining_ms, .. }) if *remaining_ms == timers::T2_MS));
    }

    #[test]
    fn 허가는_요청_우선순위를_깎지_않는다() {
        let mut f = floor();
        let out = f.on_request(&req("u1", 200), 0);
        assert!(matches!(&out[0], Out::Granted { priority: 200, .. }), "{out:?}");
        assert!(matches!(&out[1], Out::Taken { speaker, .. } if speaker == "u1"));
    }

    #[test]
    fn 선점은_한_걸음이고_큐를_안_거친다() {
        let mut f = floor();
        f.on_request(&req("a", 10), 0);
        let out = f.on_request(&req("b", 200), 10);
        assert!(matches!(&out[0], Out::Revoke { user_id, cause } if user_id == "a" && *cause == cause::PREEMPTED));
        assert!(matches!(&out[1], Out::Granted { user_id, .. } if user_id == "b"));
        assert!(matches!(&out[2], Out::Taken { speaker, .. } if speaker == "b"));
        // ★선점자에게 `QUEUE_INFO` 가 가면 안 된다.
        assert!(!out.iter().any(|o| matches!(o, Out::QueueInfo { .. })));
        assert_eq!(f.queue_len(), 0);
    }

    #[test]
    fn 같은_우선순위는_선점이_아니라_큐다() {
        let mut f = floor();
        f.on_request(&req("a", 100), 0);
        let out = f.on_request(&req("b", 100), 10);
        assert_eq!(f.speaker(), Some("a"));
        // ★`priority` 는 **지금 허가된 사람**의 값이다 — 대기자 자기 값이 아니다.
        assert!(matches!(&out[0], Out::QueueInfo { user_id, position: 1, size: 1, .. } if user_id == "b"));
    }

    #[test]
    fn 큐가_있으면_idle_을_안_보낸다() {
        // ★보내면 화면이 "아무도 안 말함"으로 깜빡였다가 다음 화자로 바뀐다.
        let mut f = floor();
        f.on_request(&req("a", 10), 0);
        f.on_request(&req("b", 10), 1);
        let out = f.on_release("a", 100);
        assert!(!out.iter().any(|o| matches!(o, Out::Idle { .. })), "{out:?}");
        assert_eq!(f.speaker(), Some("b"));
    }

    #[test]
    fn 큐가_비면_idle_이_나간다() {
        let mut f = floor();
        f.on_request(&req("a", 10), 0);
        let out = f.on_release("a", 100);
        assert!(matches!(out[0], Out::Idle { .. }));
        assert_eq!(f.speaker(), None);
    }

    #[test]
    fn t2_는_첫_rtp_부터_센다() {
        // ★허가 시각부터 세면 큐에서 늦게 켠 사람이 남보다 짧게 말한다.
        let mut f = Floor::new(1_000);
        f.on_request(&req("u1", 10), 0);
        // 허가만 받고 아직 안 말한다 — ★상한이 안 돈다(`T1` 창 안에서 본다).
        assert!(f.tick(2_000).iter().all(|o| !matches!(o, Out::Revoke { cause: 2, .. })));
        f.on_rtp("u1", 2_500);
        assert!(f.tick(3_000).is_empty());
        let out = f.tick(4_000);
        assert!(matches!(&out[0], Out::Revoke { cause, .. } if *cause == cause::MAX_DURATION), "{out:?}");
    }

    #[test]
    fn 상한_회수만_재요청을_막는다() {
        let mut f = Floor::new(1_000);
        f.on_request(&req("u1", 10), 0);
        f.on_rtp("u1", 0);
        f.tick(2_000);
        // ★`T9` 안에는 `DENY(4)`.
        let out = f.on_request(&req("u1", 10), 2_100);
        assert!(matches!(&out[0], Out::Deny { cause, .. } if *cause == cause::RETRY_AFTER));
        // 지나면 선다.
        let out = f.on_request(&req("u1", 10), 2_000 + timers::T9_MS);
        assert!(matches!(out[0], Out::Granted { .. }));
    }

    #[test]
    fn t1_은_회수_통지가_없다() {
        // ★`RELEASE` 와 같다 — 그 허가가 끝난 것으로 본다.
        let mut f = floor();
        f.on_request(&req("u1", 10), 0);
        f.on_rtp("u1", 0);
        let out = f.tick(timers::T1_MS + 1);
        assert!(!out.iter().any(|o| matches!(o, Out::Revoke { .. })), "{out:?}");
        assert!(out.iter().any(|o| matches!(o, Out::Idle { .. })));
    }

    #[test]
    fn 권한을_잃은_화자는_삼번으로_회수된다() {
        let mut f = floor();
        f.on_request(&req("u1", 10), 0);
        let out = f.on_permission_lost("u1", 10);
        assert!(matches!(&out[0], Out::Revoke { user_id, cause: 3 } if user_id == "u1"), "{out:?}");
        // ★`T9` 는 사유 `2` 만 건다 — 비트가 올라오면 곧바로 말할 수 있어야 한다.
        let back = f.on_request(&req("u1", 10), 20);
        assert!(matches!(back[0], Out::Granted { .. }), "{back:?}");
    }

    #[test]
    fn 권한을_잃은_대기자는_큐에서_빠지고_백이를_받는다() {
        // ★무통지면 `T104` 소진("클라 버그" 경로)으로만 알게 된다.
        let mut f = floor();
        f.on_request(&req("a", 10), 0);
        f.on_request(&req("b", 10), 1);
        let out = f.on_permission_lost("b", 2);
        assert!(matches!(&out[0], Out::Deny { user_id, cause } if user_id == "b" && *cause == cause::NO_PERMISSION));
        assert_eq!(f.queue_len(), 0);
    }

    #[test]
    fn 자격_관문이_상태보다_먼저다() {
        let mut f = floor();
        let mut r = req("u1", 10);
        r.allowed = false;
        assert!(matches!(&f.on_request(&r, 0)[0], Out::Deny { cause, .. } if *cause == cause::NO_PERMISSION));
        let mut r = req("u1", 10);
        r.in_pub_room = false;
        assert!(matches!(&f.on_request(&r, 0)[0], Out::Deny { cause, .. } if *cause == cause::NOT_PUB_ROOM));
        let mut r = req("u1", 10);
        r.has_half_track = false;
        assert!(matches!(&f.on_request(&r, 0)[0], Out::Deny { cause, .. } if *cause == cause::RECEIVE_ONLY));
        let mut r = req("u1", 10);
        r.others_present = false;
        assert!(matches!(&f.on_request(&r, 0)[0], Out::Deny { cause, .. } if *cause == cause::ALONE));
    }

    #[test]
    fn 같은_사람_재요청은_남은_시간을_싣는다() {
        let mut f = Floor::new(10_000);
        f.on_request(&req("u1", 10), 0);
        f.on_rtp("u1", 1_000);
        let out = f.on_request(&req("u1", 10), 4_000);
        let Out::Granted { remaining_ms, .. } = out[0].clone() else { panic!("{out:?}") };
        assert_eq!(remaining_ms, 7_000, "★T2 남은 값이다");
        // ★RTP 시각을 갱신하지 않는다 — 재요청으로 상한을 늘릴 수 없다.
        assert_eq!(out.len(), 1, "★TAKEN 을 다시 뿌리지 않는다");
    }

    #[test]
    fn seq_는_재전송도_센다() {
        let mut f = floor();
        f.on_request(&req("a", 10), 0);
        let s1 = f.seq;
        f.on_release("a", 1);
        assert_eq!(f.seq, s1 + 1, "★IDLE 도 센다");
    }

    #[test]
    fn 회수_뒤_release_는_t8_을_멈춘다() {
        let mut f = Floor::new(1_000);
        f.on_request(&req("u1", 10), 0);
        f.on_rtp("u1", 0);
        f.tick(2_000);
        f.on_release("u1", 2_100);
        // ★`RELEASE` 를 받았으니 한 번 더 보내지 않는다.
        assert!(f.tick(10_000).iter().all(|o| !matches!(o, Out::Revoke { .. })));
    }

    #[test]
    fn 회수가_유실되면_한_번만_더_보낸다() {
        let mut f = Floor::new(1_000);
        f.on_request(&req("u1", 10), 0);
        f.on_rtp("u1", 0);
        f.tick(2_000);
        let again = f.tick(2_000 + timers::T8_MS);
        assert_eq!(again.len(), 1, "★C8=1 — 딱 한 번");
        assert!(f.tick(99_000).is_empty(), "★그 뒤는 0(죽은 DC 에 영원 재송 금지)");
    }

    #[test]
    fn 큐에_없으면_순번을_안_답한다() {
        let mut f = floor();
        f.on_request(&req("a", 10), 0);
        assert!(f.on_queue_pos("없는사람").is_empty(), "★계수 후 무응답");
    }

    #[test]
    fn 큐는_우선순위_뒤_선착순이다() {
        let mut f = floor();
        // ★화자를 제일 높게 둔다 — 아니면 vip 가 큐가 아니라 선점으로 간다.
        f.on_request(&req("a", 250), 0);
        f.on_request(&req("b", 10), 1);
        f.on_request(&req("c", 10), 2);
        f.on_request(&req("vip", 200), 3);
        assert_eq!(f.queue_len(), 3);
        let out = f.on_release("a", 10);
        // ★우선순위가 높은 vip 가 먼저, 같은 단은 먼저 온 b.
        assert!(matches!(&out[0], Out::Granted { user_id, .. } if user_id == "vip"), "{out:?}");
        let out = f.on_release("vip", 20);
        assert!(matches!(&out[0], Out::Granted { user_id, .. } if user_id == "b"), "{out:?}");
    }
}
