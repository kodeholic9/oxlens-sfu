// author: kodeholic (powered by Claude)
//! 발언권 — 정§9. 방마다 제어기 하나. ★판정·상태·큐는 락 안, 집행(송신)은 락 밖이다(§2-3 계약 2)
//! — 그래서 모든 입구가 `Vec<Action>` 을 돌려주고 부르는 쪽이 내보낸다.
//! ★시계는 인자로 받는다(§2-3 계약 6) — 상태기 안에서 시각을 부르지 않으므로 시험이 결정적이다.
//! 핫패스(prefan §7-3)가 읽는 화자는 락 밖 RCU 사본이고, 정본은 락 안 상태다.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwapOption;
use oxsig::mbcp::{Msg, MsgType, field, reject, revoke};
use oxsig::timers;
use std::sync::Arc;

/// 정§9-1 — 큐 상한.
pub const QUEUE_CAP: usize = 10;
/// 정§9-2 — 타이머 판정 해상도. 상한 초과를 최대 이만큼 늦게 안다(성립 조건).
pub const TICK_MS: u64 = 2_000;
/// 연§8-4 `T2` — 정책 키의 기본값(초).
pub const T2_DEFAULT_SECS: u16 = 30;

/// 정§9-1 상태 셋. `Initialising`·`Releasing` 은 방 수명이 겸하고 `T4` 는 미채택이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FloorState {
    Idle,
    Taken { speaker: String, priority: u8, max_burst_ms: u64 },
    /// 신설 — `T3`·`T8` 이 도는 구간. ★게이트는 이 화자도 통과시킨다(말끝 보존).
    PendingRevoke { speaker: String, cause: u8, since: u64 },
}

impl FloorState {
    pub fn speaker(&self) -> Option<&str> {
        match self {
            Self::Idle => None,
            Self::Taken { speaker, .. } | Self::PendingRevoke { speaker, .. } => Some(speaker),
        }
    }
    /// 정§9-2 — cross-room 검사에서 `PendingRevoke` 는 제외한다(이미 마이크를 껐다).
    fn holder(&self) -> Option<&str> {
        match self {
            Self::Taken { speaker, .. } => Some(speaker),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiter {
    pub user: String,
    pub priority: u8,
    pub enqueued_at: u64,
}

/// 락 밖에서 집행한다. 방 broadcast 는 `exclude` 를 뺀 그 방 청취자 전원.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Unicast { to: String, msg: Msg },
    Broadcast { msg: Msg, exclude: Option<String> },
}

impl Action {
    pub fn msg(&self) -> &Msg {
        match self {
            Self::Unicast { msg, .. } | Self::Broadcast { msg, .. } => msg,
        }
    }
    pub fn target(&self) -> Option<&str> {
        match self {
            Self::Unicast { to, .. } => Some(to),
            Self::Broadcast { .. } => None,
        }
    }
}

/// 요청 하나의 판정 재료 — ★전부 부르는 쪽이 미리 구한 값이다(상태기는 바깥을 조회하지 않는다).
#[derive(Debug, Clone)]
pub struct Request {
    pub user: String,
    /// 정§9-5 — 유효 우선순위 `min(요청 TLV 0, 토큰 floor_priority)`. 원값으로 판정하면 255 만 실으면 뺏는다.
    pub eff_priority: u8,
    pub duration_secs: Option<u16>,
    /// 정§9-6 관문 ② — 반이중 발행 트랙이 없으면 청취 전용이다.
    pub has_half_track: bool,
    /// 정§9-2 — 방에 나뿐.
    pub alone: bool,
}

/// 재전송 하나 — 정§9-4 의 셋(`T7`·`T8`·`T20`)만 쓴다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Retx {
    due_ms: u64,
    left: u8,
}

impl Retx {
    fn new(now: u64, period: u64, count: u8) -> Self {
        Self { due_ms: now + period, left: count }
    }
    /// 만료했으면 다음 기한을 걸고 `true`. 상한을 소진하면 `None` 이 되도록 부르는 쪽이 지운다.
    fn fire(&mut self, now: u64, period: u64) -> bool {
        if now < self.due_ms || self.left == 0 {
            return false;
        }
        self.left -= 1;
        self.due_ms = now + period;
        true
    }
    fn spent(&self) -> bool {
        self.left == 0
    }
}

#[derive(Debug)]
struct Floor {
    state: FloorState,
    /// 우선순위 DESC → 먼저 온 순. 선점자만 맨 앞에 꽂는다(정§9-2).
    queue: Vec<Waiter>,
    /// 연§11-3 `8` — 방마다. `IDLE`·`TAKEN` 을 보낼 때마다(재전송 포함) 1 증가.
    seq: u16,
    prev_speaker: Option<String>,
    /// 정§9-1 `T9` — 사유 2 회수자만.
    retry_after: BTreeMap<String, u64>,
    idle_retx: Option<Retx>,
    revoke_retx: Option<Retx>,
    grant_retx: Option<Retx>,
    /// 마지막 허가가 실린 발화 상한(승계·재허가의 남은 시간 계산).
    granted_at: u64,
}

pub struct FloorController {
    room_id: String,
    max_burst_ms: u64,
    inner: Mutex<Floor>,
    /// ★핫패스 전용 파생값(정§7-3 prefan). 정본은 `inner.state` 이고, 갱신은 상태 교체와 같은 임계 구역이다.
    speaker: ArcSwapOption<String>,
    /// prefan 게이트를 ★통과한 RTP 만 센다(정§9-2) — 막힌 RTP 는 발화가 아니다.
    first_rtp_at: AtomicU64,
    last_rtp_at: AtomicU64,
}

impl FloorController {
    pub fn new(room_id: &str, t2_secs: u16) -> Self {
        Self {
            room_id: room_id.to_owned(),
            max_burst_ms: u64::from(t2_secs) * 1_000,
            inner: Mutex::new(Floor {
                state: FloorState::Idle,
                queue: Vec::new(),
                seq: 0,
                prev_speaker: None,
                retry_after: BTreeMap::new(),
                idle_retx: None,
                revoke_retx: None,
                grant_retx: None,
                granted_at: 0,
            }),
            speaker: ArcSwapOption::empty(),
            first_rtp_at: AtomicU64::new(0),
            last_rtp_at: AtomicU64::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Floor> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// ★핫패스 — 락 없이 읽는다. `Taken` 과 `PendingRevoke` 둘 다 참이다(말끝 보존, 정§9-2).
    pub fn is_speaker(&self, user: &str) -> bool {
        self.speaker.load().as_deref().map(String::as_str) == Some(user)
    }

    pub fn speaker(&self) -> Option<Arc<String>> {
        self.speaker.load_full()
    }

    /// ★핫패스 — prefan 게이트를 통과한 순간. 원자값만 만진다.
    /// `T2` 의 시작은 허가가 아니라 첫 RTP 이고(연§8-4), `T1` 은 받을 때마다 다시 걸린다.
    /// 정§9-7 — 이 허가에서 화자의 RTP 를 이미 받았나. 데우기를 멈출 자리다.
    pub fn heard_media(&self) -> bool {
        self.first_rtp_at.load(Ordering::Acquire) != 0
    }

    pub fn on_media(&self, now_ms: u64) {
        let _ = self.first_rtp_at.compare_exchange(0, now_ms, Ordering::AcqRel, Ordering::Relaxed);
        self.last_rtp_at.store(now_ms, Ordering::Release);
    }

    pub fn state(&self) -> FloorState {
        self.lock().state.clone()
    }

    /// cross-room 검사(정§9-2 — Peer 축)의 대상. 부르는 쪽이 이 목록만 확인해 `blocked` 를 만든다.
    pub fn candidates(&self) -> Vec<String> {
        self.lock().queue.iter().map(|w| w.user.clone()).collect()
    }

    pub fn queue_len(&self) -> usize {
        self.lock().queue.len()
    }

    /// 정§9-2 — 다른 방이 이 사람을 쥐고 있는지 볼 때 쓰는 값(`Taken` 만).
    pub fn holds(&self, user: &str) -> bool {
        self.lock().state.holder() == Some(user)
    }

    // ───────── 입구 ─────────

    /// 정§9-2 `REQUEST` 행 전량. `blocked` = 다른 방에서 `Taken` 인 사람들(부르는 쪽이 Peer 축에서 구한다).
    pub fn request(&self, req: &Request, blocked: &BTreeSet<String>, now: u64) -> Vec<Action> {
        let mut f = self.lock();
        if !req.has_half_track {
            return vec![self.deny(&req.user, reject::RECEIVE_ONLY)];
        }
        if f.retry_after.get(&req.user).is_some_and(|until| now < *until) {
            return vec![self.deny(&req.user, reject::RETRY_AFTER_NOT_EXPIRED)];
        }
        if blocked.contains(&req.user) {
            return vec![self.deny(&req.user, reject::HELD_ELSEWHERE)];
        }
        if req.alone {
            return vec![self.deny(&req.user, reject::ONLY_ONE_PARTICIPANT)];
        }
        match f.state.clone() {
            FloorState::Idle => {
                let burst = self.burst_of(req.duration_secs);
                self.grant(&mut f, &req.user, req.eff_priority, burst, now, false)
            }
            // 같은 u 재요청 — 멱등 재응답. 시간엔 `T2` 남은 값을 싣고 발화 시각은 갱신하지 않는다.
            FloorState::Taken { ref speaker, priority, max_burst_ms } if speaker == &req.user => {
                let spent = self.first_rtp_at.load(Ordering::Acquire);
                let left = if spent == 0 { max_burst_ms } else { max_burst_ms.saturating_sub(now.saturating_sub(spent)) };
                vec![self.granted(&req.user, priority, left)]
            }
            FloorState::Taken { ref speaker, priority, .. } if req.eff_priority > priority => {
                self.preempt(&mut f, speaker.clone(), &req.user, req.eff_priority, now)
            }
            // 회수가 도는 중이면 우선순위와 무관하게 큐 규칙으로만 받는다(재선점 없음).
            FloorState::Taken { .. } | FloorState::PendingRevoke { .. } => self.enqueue(&mut f, &req.user, req.eff_priority, now),
        }
    }

    /// 정§9-2 `RELEASE` 행 — 화자면 승계로, 아니면 대기 취소(멱등).
    pub fn release(&self, user: &str, blocked: &BTreeSet<String>, now: u64) -> Vec<Action> {
        let mut f = self.lock();
        match f.state.clone() {
            FloorState::Taken { ref speaker, .. } | FloorState::PendingRevoke { ref speaker, .. } if speaker == user => {
                f.revoke_retx = None;
                self.step_down(&mut f, blocked, now)
            }
            _ => self.cancel_wait(&mut f, user),
        }
    }

    /// 정§17-2 ① — 퇴장·`pub_deselect`. 화자면 `RELEASE` 와 같고, 큐에서도 뺀다.
    pub fn on_leave(&self, user: &str, blocked: &BTreeSet<String>, now: u64) -> Vec<Action> {
        let mut actions = self.release(user, blocked, now);
        let mut f = self.lock();
        actions.extend(self.cancel_wait(&mut f, user));
        actions
    }

    /// 정§9-2 — 큐에 있으면 순번, 없으면 계수 후 무응답(`DENY` 를 주면 없는 요청의 상태를 되감는다).
    pub fn queue_position(&self, user: &str) -> Vec<Action> {
        let f = self.lock();
        match f.queue.iter().position(|w| w.user == user) {
            Some(at) => vec![self.queue_info(&f, at)],
            None => Vec::new(),
        }
    }

    /// 정§9-6 처음 알리기 — `READY{tracks}` 때 화자가 있으면 그 사람에게만 `TAKEN` unicast.
    pub fn announce_speaker(&self, to: &str) -> Vec<Action> {
        let mut f = self.lock();
        let Some(speaker) = f.state.speaker().map(str::to_owned) else { return Vec::new() };
        let msg = self.taken(&mut f, &speaker);
        vec![Action::Unicast { to: to.to_owned(), msg }]
    }

    /// 정§9-2 타이머 행 전량 — `T1`·`T2`·`T3`·`T7`·`T8`·`T20` + `T9` 청소. tick 은 2,000ms.
    pub fn tick(&self, blocked: &BTreeSet<String>, now: u64) -> Vec<Action> {
        let mut f = self.lock();
        f.retry_after.retain(|_, until| now < *until);
        match f.state.clone() {
            FloorState::Taken { ref speaker, .. } => {
                let first = self.first_rtp_at.load(Ordering::Acquire);
                let last = self.last_rtp_at.load(Ordering::Acquire);
                // `T2` — 첫 RTP 부터 상한. 회수 사유 2 는 `T9` 를 건다.
                if first != 0 && now.saturating_sub(first) > self.burst_now(&f) {
                    let u = speaker.clone();
                    f.retry_after.insert(u.clone(), now + timers::T9_MS);
                    return self.begin_revoke(&mut f, u, revoke::BURST_TOO_LONG, now);
                }
                // `T1` — RTP 가 끊겼다. 허가가 끝난 것으로 본다(u 에게 따로 보내지 않는다).
                let since = if last == 0 { f.granted_at } else { last };
                if now.saturating_sub(since) > timers::T1_MS {
                    return self.step_down(&mut f, blocked, now);
                }
                self.retransmit_grant(&mut f, now)
            }
            FloorState::PendingRevoke { ref speaker, since, .. } => {
                if now.saturating_sub(since) > timers::T3_MS {
                    f.revoke_retx = None;
                    return self.step_down(&mut f, blocked, now);
                }
                let speaker = speaker.clone();
                let cause = match f.state {
                    FloorState::PendingRevoke { cause, .. } => cause,
                    _ => revoke::OTHER,
                };
                let fire = f.revoke_retx.as_mut().is_some_and(|r| r.fire(now, timers::T8_MS));
                if fire { vec![self.revoke_msg(&speaker, cause)] } else { Vec::new() }
            }
            FloorState::Idle => {
                let fire = f.idle_retx.as_mut().is_some_and(|r| r.fire(now, timers::T7_MS));
                if !fire {
                    if f.idle_retx.as_ref().is_some_and(Retx::spent) {
                        f.idle_retx = None;
                    }
                    return Vec::new();
                }
                let msg = self.idle_msg(&mut f);
                vec![Action::Broadcast { msg, exclude: None }]
            }
        }
    }

    // ───────── 집행 조각 ─────────

    fn burst_of(&self, requested: Option<u16>) -> u64 {
        // 정§9-5 — 요청 duration 을 수용하되 상한으로 자른다(자리가 있는데 무시하면 유령 필드다).
        requested.map_or(self.max_burst_ms, |s| (u64::from(s) * 1_000).min(self.max_burst_ms))
    }

    fn burst_now(&self, f: &Floor) -> u64 {
        match f.state {
            FloorState::Taken { max_burst_ms, .. } => max_burst_ms,
            _ => self.max_burst_ms,
        }
    }

    /// 허가 — 직접이든 승계든 같다. `TAKEN` broadcast(화자 제외)가 같은 임계 구역에서 나간다.
    fn grant(&self, f: &mut Floor, user: &str, priority: u8, burst: u64, now: u64, succession: bool) -> Vec<Action> {
        f.state = FloorState::Taken { speaker: user.to_owned(), priority, max_burst_ms: burst };
        f.granted_at = now;
        f.idle_retx = None;
        f.grant_retx = succession.then(|| Retx::new(now, timers::T20_MS, timers::C20));
        self.speaker.store(Some(Arc::new(user.to_owned())));
        self.first_rtp_at.store(0, Ordering::Release);
        self.last_rtp_at.store(0, Ordering::Release);
        let taken = self.taken(f, user);
        vec![self.granted(user, priority, burst), Action::Broadcast { msg: taken, exclude: Some(user.to_owned()) }]
    }

    /// 정§9-2 선점 3걸음 — ①`REVOKE`+`T3`·`T8` ②선점자를 큐에 + `QUEUE_INFO` ③뒤에 승계.
    /// ★뺏긴 사람은 큐에 넣지 않는다. ★`GRANTED` 를 바로 주지 않는다.
    ///
    /// ★선점자는 화자보다 높은 우선순위라 정렬 규칙만으로 맨 앞에 선다 — 자리를 못박지 않는다.
    /// 못박으면 나중에 더 높은 우선순위가 와도 뒤에 서게 되어, 큐의 규칙이 원소마다 갈린다.
    fn preempt(&self, f: &mut Floor, victim: String, taker: &str, priority: u8, now: u64) -> Vec<Action> {
        let mut actions = self.begin_revoke(f, victim, revoke::PREEMPTED, now);
        f.queue.retain(|w| w.user != taker);
        actions.extend(self.enqueue(f, taker, priority, now));
        actions
    }

    fn begin_revoke(&self, f: &mut Floor, speaker: String, cause: u8, now: u64) -> Vec<Action> {
        f.state = FloorState::PendingRevoke { speaker: speaker.clone(), cause, since: now };
        f.revoke_retx = Some(Retx::new(now, timers::T8_MS, u8::MAX));
        f.grant_retx = None;
        vec![self.revoke_msg(&speaker, cause)]
    }

    /// 정§9-2 — 큐가 비었으면 `IDLE`(seq++·`T7`), 있으면 ★`IDLE` 없이 곧바로 승계(깜빡임 금지).
    fn step_down(&self, f: &mut Floor, blocked: &BTreeSet<String>, now: u64) -> Vec<Action> {
        f.prev_speaker = f.state.speaker().map(str::to_owned);
        f.state = FloorState::Idle;
        f.grant_retx = None;
        self.speaker.store(None);
        self.first_rtp_at.store(0, Ordering::Release);
        self.last_rtp_at.store(0, Ordering::Release);

        let mut actions = Vec::new();
        while !f.queue.is_empty() {
            let next = f.queue.remove(0);
            // 정§9-3 — cross-room 검사를 승계에도. 쥔 채로 방을 옮기지 않는다.
            if blocked.contains(&next.user) {
                actions.push(self.deny(&next.user, reject::HELD_ELSEWHERE));
                continue;
            }
            actions.extend(self.grant(f, &next.user, next.priority, self.max_burst_ms, now, true));
            actions.extend(self.refresh_queue(f));
            return actions;
        }
        let msg = self.idle_msg(f);
        f.idle_retx = Some(Retx::new(now, timers::T7_MS, timers::C7));
        actions.push(Action::Broadcast { msg, exclude: None });
        actions
    }

    fn enqueue(&self, f: &mut Floor, user: &str, priority: u8, now: u64) -> Vec<Action> {
        if let Some(at) = f.queue.iter().position(|w| w.user == user) {
            return vec![self.queue_info(f, at)];
        }
        if f.queue.len() >= QUEUE_CAP {
            // 큐는 항상 켜므로 사유 1(점유)은 내지 않는다 — 만석이 진짜 사유다.
            return vec![self.deny(user, reject::QUEUE_FULL)];
        }
        let at = f.queue.iter().position(|o| o.priority < priority).unwrap_or(f.queue.len());
        f.queue.insert(at, Waiter { user: user.to_owned(), priority, enqueued_at: now });
        let mut actions = vec![self.queue_info(f, at)];
        actions.extend(self.refresh_queue(f).into_iter().filter(|a| a.target() != Some(user)));
        actions
    }

    fn cancel_wait(&self, f: &mut Floor, user: &str) -> Vec<Action> {
        let before = f.queue.len();
        f.queue.retain(|w| w.user != user);
        if f.queue.len() == before {
            return Vec::new();
        }
        self.refresh_queue(f)
    }

    /// 남은 대기자 전원에게 순번 갱신 unicast.
    fn refresh_queue(&self, f: &Floor) -> Vec<Action> {
        (0..f.queue.len()).map(|at| self.queue_info(f, at)).collect()
    }

    fn retransmit_grant(&self, f: &mut Floor, now: u64) -> Vec<Action> {
        // 정§9-4 — 승계 `GRANTED` 재전송의 정지는 ★그 사람의 RTP 수신이다.
        if self.first_rtp_at.load(Ordering::Acquire) != 0 {
            f.grant_retx = None;
            return Vec::new();
        }
        let (speaker, priority, burst) = match f.state {
            FloorState::Taken { ref speaker, priority, max_burst_ms } => (speaker.clone(), priority, max_burst_ms),
            _ => return Vec::new(),
        };
        let fire = f.grant_retx.as_mut().is_some_and(|r| r.fire(now, timers::T20_MS));
        if f.grant_retx.as_ref().is_some_and(Retx::spent) {
            f.grant_retx = None;
        }
        if fire { vec![self.granted(&speaker, priority, burst)] } else { Vec::new() }
    }

    // ───────── 메시지 조립(정§9-6 — S→C 전부에 `0x1D`) ─────────

    fn granted(&self, to: &str, priority: u8, burst_ms: u64) -> Action {
        let msg = Msg::new(MsgType::Granted)
            .ack(true)
            .u8(field::PRIORITY, priority)
            .u16(field::DURATION, (burst_ms / 1_000) as u16)
            .room(&self.room_id);
        Action::Unicast { to: to.to_owned(), msg }
    }

    fn deny(&self, to: &str, cause: u8) -> Action {
        let msg = Msg::new(MsgType::Deny).ack(true).u8(field::CAUSE, cause).room(&self.room_id);
        Action::Unicast { to: to.to_owned(), msg }
    }

    fn revoke_msg(&self, to: &str, cause: u8) -> Action {
        let msg = Msg::new(MsgType::Revoke).u8(field::CAUSE, cause).room(&self.room_id);
        Action::Unicast { to: to.to_owned(), msg }
    }

    fn taken(&self, f: &mut Floor, speaker: &str) -> Msg {
        f.seq = f.seq.wrapping_add(1);
        Msg::new(MsgType::Taken).str(field::GRANTED_PARTY, speaker).u16(field::SEQ, f.seq).room(&self.room_id)
    }

    fn idle_msg(&self, f: &mut Floor) -> Msg {
        f.seq = f.seq.wrapping_add(1);
        let mut msg = Msg::new(MsgType::Idle).u16(field::SEQ, f.seq).room(&self.room_id);
        if let Some(prev) = f.prev_speaker.clone() {
            msg = msg.str(field::PREV_SPEAKER, &prev);
        }
        msg
    }

    fn queue_info(&self, f: &Floor, at: usize) -> Action {
        let w = &f.queue[at];
        let msg = Msg::new(MsgType::QueueInfo)
            .tlv(field::QUEUE_INFO, [(at + 1) as u8, w.priority])
            .u8(field::QUEUE_SIZE, f.queue.len() as u8)
            .room(&self.room_id);
        Action::Unicast { to: w.user.clone(), msg }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T2_MS: u64 = 30_000;

    fn floor() -> FloorController {
        FloorController::new("r1", T2_DEFAULT_SECS)
    }
    fn none() -> BTreeSet<String> {
        BTreeSet::new()
    }
    fn req(user: &str, priority: u8) -> Request {
        Request { user: user.into(), eff_priority: priority, duration_secs: None, has_half_track: true, alone: false }
    }
    /// (타입, 수신자 또는 `None`=broadcast, 사유·순번 같은 판정값)
    fn shape(a: &Action) -> (MsgType, Option<String>, Option<u8>) {
        let m = a.msg();
        let key = match m.msg_type {
            MsgType::Deny | MsgType::Revoke => m.get_u8(field::CAUSE),
            MsgType::QueueInfo => m.get(field::QUEUE_INFO).and_then(|v| v.first().copied()),
            MsgType::Granted => m.get_u8(field::PRIORITY),
            _ => None,
        };
        (m.msg_type, a.target().map(str::to_owned), key)
    }
    fn shapes(actions: &[Action]) -> Vec<(MsgType, Option<String>, Option<u8>)> {
        actions.iter().map(shape).collect()
    }
    fn to(user: &str) -> Option<String> {
        Some(user.to_owned())
    }

    #[test]
    fn idle_request_grants_and_tells_the_room() {
        let f = floor();
        let a = f.request(&req("u1", 10), &none(), 1_000);
        assert_eq!(shapes(&a), vec![(MsgType::Granted, to("u1"), Some(10)), (MsgType::Taken, None, None)]);
        assert!(matches!(&a[1], Action::Broadcast { exclude, .. } if exclude.as_deref() == Some("u1")), "화자는 제 TAKEN 을 안 받는다");
        assert_eq!(a[0].msg().get_u16(field::DURATION), Some(T2_DEFAULT_SECS), "요청이 값을 안 실으면 서버 상한");
        assert_eq!(a[1].msg().seq(), Some(1), "TAKEN 은 방별 일련번호 필수");
        assert_eq!(a[1].msg().get_str(field::GRANTED_PARTY), Some("u1"));
        assert!(a.iter().all(|x| x.msg().room_id() == Some("r1")), "S→C 전부에 방을 싣는다");
        assert!(a[0].msg().ack_req && !a[1].msg().ack_req, "A 는 GRANTED·DENY 둘뿐");
        assert!(f.is_speaker("u1") && !f.is_speaker("u2") && f.holds("u1"));

        f.on_media(2_000);
        let again = f.request(&req("u1", 10), &none(), 12_000);
        assert_eq!(shapes(&again), vec![(MsgType::Granted, to("u1"), Some(10))]);
        assert_eq!(again[0].msg().get_u16(field::DURATION), Some(20), "재요청엔 T2 남은 값을 싣는다");
    }

    #[test]
    fn every_refusal_row_has_its_own_cause() {
        let f = floor();
        let listener = Request { has_half_track: false, ..req("u9", 10) };
        assert_eq!(shapes(&f.request(&listener, &none(), 0)), vec![(MsgType::Deny, to("u9"), Some(reject::RECEIVE_ONLY))]);
        let alone = Request { alone: true, ..req("u1", 10) };
        assert_eq!(shapes(&f.request(&alone, &none(), 0)), vec![(MsgType::Deny, to("u1"), Some(reject::ONLY_ONE_PARTICIPANT))]);
        let elsewhere: BTreeSet<String> = ["u1".to_owned()].into();
        assert_eq!(shapes(&f.request(&req("u1", 10), &elsewhere, 0)), vec![(MsgType::Deny, to("u1"), Some(reject::HELD_ELSEWHERE))]);

        f.request(&req("hold", 200), &none(), 0);
        for i in 0..QUEUE_CAP {
            let a = f.request(&req(&format!("w{i}"), 10), &none(), 0);
            assert_eq!(shape(&a[0]).0, MsgType::QueueInfo, "{i}");
        }
        assert_eq!(shapes(&f.request(&req("late", 10), &none(), 0)), vec![(MsgType::Deny, to("late"), Some(reject::QUEUE_FULL))]);
    }

    #[test]
    fn preemption_is_three_steps_and_never_grants_at_once() {
        let f = floor();
        f.request(&req("a", 10), &none(), 0);
        let a = f.request(&req("b", 200), &none(), 1_000);
        assert_eq!(
            shapes(&a),
            vec![(MsgType::Revoke, to("a"), Some(revoke::PREEMPTED)), (MsgType::QueueInfo, to("b"), Some(1))],
            "①REVOKE ②큐 맨 앞 — GRANTED 는 아직 없다"
        );
        assert!(matches!(f.state(), FloorState::PendingRevoke { ref speaker, cause, .. } if speaker == "a" && cause == revoke::PREEMPTED));
        assert!(f.is_speaker("a"), "게이트는 회수 중 화자도 통과시킨다 — 말끝 보존");
        assert!(!f.holds("a"), "cross-room 검사에선 빠진다 — 뺏긴 사람을 더 벌주지 않는다");
        assert_eq!(f.queue_len(), 1, "뺏긴 a 는 큐에 넣지 않는다");

        // 회수 중 요청은 재선점을 부르지 않고 큐 규칙으로만 받는다. 규칙은 하나라 더 높은 우선순위가 앞에 선다.
        let c = f.request(&req("c", 255), &none(), 1_100);
        assert_eq!(shape(&c[0]).0, MsgType::QueueInfo);
        assert!(matches!(f.state(), FloorState::PendingRevoke { .. }), "재선점은 없다");
        assert_eq!(f.candidates(), vec!["c".to_owned(), "b".to_owned()], "선점자 자리를 못박지 않는다 — 규칙 하나");

        assert_eq!(shapes(&f.tick(&none(), 1_000 + timers::T8_MS + 1)), vec![(MsgType::Revoke, to("a"), Some(revoke::PREEMPTED))]);
        let done = f.release("a", &none(), 3_000);
        assert_eq!(
            shapes(&done),
            vec![(MsgType::Granted, to("c"), Some(255)), (MsgType::Taken, None, None), (MsgType::QueueInfo, to("b"), Some(1))],
            "RELEASE 가 T3·T8 을 멈추고 곧바로 승계"
        );
        assert_eq!(done[0].msg().get_u16(field::DURATION), Some(T2_DEFAULT_SECS), "승계 시간은 서버 기본이다");
        assert!(f.is_speaker("c"));
    }

    #[test]
    fn release_and_leave_take_the_same_path() {
        let f = floor();
        f.request(&req("a", 10), &none(), 0);
        f.request(&req("b", 5), &none(), 10);
        assert!(f.release("b", &none(), 20).is_empty(), "화자가 아닌 RELEASE 는 대기 취소");
        assert_eq!(f.queue_len(), 0);
        assert!(f.release("zz", &none(), 20).is_empty(), "없는 사람은 조용히 무시 — 멱등");

        let idle = f.release("a", &none(), 30);
        assert_eq!(shapes(&idle), vec![(MsgType::Idle, None, None)]);
        assert_eq!(idle[0].msg().get_str(field::PREV_SPEAKER), Some("a"));
        assert_eq!(idle[0].msg().seq(), Some(2), "TAKEN 1 → IDLE 2");
        assert!(!f.is_speaker("a") && f.state() == FloorState::Idle);

        let mut seqs = Vec::new();
        for i in 1..=u64::from(timers::C7) + 2 {
            if let Some(a) = f.tick(&none(), 30 + timers::T7_MS * i + 1).first() {
                seqs.push(a.msg().seq().unwrap());
            }
        }
        assert_eq!(seqs, (3..3 + u16::from(timers::C7)).collect::<Vec<_>>(), "C7 상한까지, 재전송도 seq 를 올린다");

        f.request(&req("a", 10), &none(), 100_000);
        f.request(&req("b", 5), &none(), 100_001);
        assert_eq!(f.on_leave("b", &none(), 100_002), Vec::new(), "대기자 퇴장은 큐에서만 뺀다");
        assert_eq!(shape(&f.on_leave("a", &none(), 100_003)[0]).0, MsgType::Idle, "화자 퇴장은 RELEASE 와 같다");
    }

    #[test]
    fn burst_limit_revokes_with_retry_after_and_silence_steps_down() {
        let f = floor();
        f.request(&req("a", 10), &none(), 0);
        assert!(f.tick(&none(), 1_000).is_empty(), "첫 RTP 전엔 T2 가 시작하지 않는다");
        f.on_media(1_000);
        let gone = f.tick(&none(), 1_000 + timers::T1_MS + 1);
        assert_eq!(shapes(&gone), vec![(MsgType::Idle, None, None)], "T1 — RTP 가 끊기면 허가가 끝난 것으로 본다");

        f.request(&req("a", 10), &none(), 200_000);
        f.on_media(200_000);
        f.on_media(200_000 + T2_MS);
        let over = f.tick(&none(), 200_001 + T2_MS);
        assert_eq!(shapes(&over), vec![(MsgType::Revoke, to("a"), Some(revoke::BURST_TOO_LONG))]);
        f.release("a", &none(), 200_002 + T2_MS);
        let blocked = f.request(&req("a", 10), &none(), 200_003 + T2_MS);
        assert_eq!(shapes(&blocked), vec![(MsgType::Deny, to("a"), Some(reject::RETRY_AFTER_NOT_EXPIRED))]);
        let ok = f.request(&req("a", 10), &none(), 200_003 + T2_MS + timers::T9_MS);
        assert_eq!(shape(&ok[0]).0, MsgType::Granted, "T9 가 지나면 풀린다");

        let f2 = floor();
        f2.request(&req("x", 10), &none(), 0);
        f2.request(&req("y", 99), &none(), 100);
        let expired = f2.tick(&none(), 100 + timers::T3_MS + 1);
        assert_eq!(shape(&expired[0]).0, MsgType::Granted, "T3 만료 → RELEASE 없이도 승계");
        assert!(f2.is_speaker("y"));
    }

    #[test]
    fn succession_grant_is_retransmitted_until_the_media_arrives() {
        let f = floor();
        f.request(&req("a", 10), &none(), 0);
        f.request(&req("b", 5), &none(), 10);
        f.release("a", &none(), 20);
        assert!(f.is_speaker("b"));

        // `T20` 창은 `T1`(4초) 안에서 끝난다 — 상한을 소진해도 조용하고, 그 뒤엔 T1 이 회수한다.
        let mut sent = 0;
        for i in 1..=u64::from(timers::C20) {
            if !f.tick(&none(), 20 + timers::T20_MS * i + 1).is_empty() {
                sent += 1;
            }
        }
        assert_eq!(sent, usize::from(timers::C20), "C20 상한까지");
        assert!(f.tick(&none(), 20 + timers::T20_MS * u64::from(timers::C20) + 500).is_empty(), "소진 뒤엔 조용하다");
        assert_eq!(shape(&f.tick(&none(), 20 + timers::T1_MS + 1)[0]).0, MsgType::Idle, "마이크를 안 켜면 T1 이 회수한다");

        let f2 = floor();
        f2.request(&req("a", 10), &none(), 0);
        f2.request(&req("b", 5), &none(), 10);
        f2.release("a", &none(), 20);
        f2.on_media(21);
        assert!(f2.tick(&none(), 20 + timers::T20_MS + 1).is_empty(), "그 사람의 RTP 수신이 정지 조건이다");
    }

    #[test]
    fn queue_order_and_position_queries() {
        let f = floor();
        f.request(&req("hold", 250), &none(), 0);
        f.request(&req("low", 1), &none(), 10);
        f.request(&req("high", 9), &none(), 20);
        f.request(&req("mid", 5), &none(), 30);
        assert_eq!(f.candidates(), vec!["high".to_owned(), "mid".to_owned(), "low".to_owned()], "우선순위 DESC");

        assert_eq!(shapes(&f.queue_position("mid")), vec![(MsgType::QueueInfo, to("mid"), Some(2))]);
        assert!(f.queue_position("nobody").is_empty(), "대기 중이 아니면 계수 후 무응답 — DENY 는 없는 요청을 되감는다");
        let info = f.queue_position("high")[0].msg().clone();
        assert_eq!((info.get_u8(field::QUEUE_SIZE), info.get(field::QUEUE_INFO)), (Some(3), Some(&[1u8, 9][..])));

        let dup = f.request(&req("mid", 5), &none(), 40);
        assert_eq!((shapes(&dup), f.queue_len()), (vec![(MsgType::QueueInfo, to("mid"), Some(2))], 3), "중복 삽입 없음");

        let blocked: BTreeSet<String> = ["high".to_owned()].into();
        let a = f.release("hold", &blocked, 50);
        assert_eq!(
            shapes(&a),
            vec![
                (MsgType::Deny, to("high"), Some(reject::HELD_ELSEWHERE)),
                (MsgType::Granted, to("mid"), Some(5)),
                (MsgType::Taken, None, None),
                (MsgType::QueueInfo, to("low"), Some(1)),
            ],
            "승계에도 cross-room 검사 — 걸린 사람은 빼고 다음으로"
        );
    }

    #[test]
    fn first_notice_only_when_someone_is_speaking() {
        let f = floor();
        assert!(f.announce_speaker("late").is_empty(), "화자가 없으면 아무것도 안 보낸다");
        f.request(&req("a", 10), &none(), 0);
        let notice = f.announce_speaker("late");
        assert_eq!(shapes(&notice), vec![(MsgType::Taken, to("late"), None)]);
        assert_eq!(notice[0].msg().get_str(field::GRANTED_PARTY), Some("a"));
        assert_eq!(notice[0].msg().seq(), Some(2), "보낼 때마다 오른다");
    }

    #[test]
    fn duration_request_is_accepted_but_clipped() {
        let f = floor();
        let short = Request { duration_secs: Some(5), ..req("a", 10) };
        let a = f.request(&short, &none(), 0);
        assert_eq!(a[0].msg().get_u16(field::DURATION), Some(5), "요청 값을 수용한다 — 유령 필드가 아니다");
        f.release("a", &none(), 10);
        let greedy = Request { duration_secs: Some(9_999), ..req("b", 10) };
        let b = f.request(&greedy, &none(), 20);
        assert_eq!(b[0].msg().get_u16(field::DURATION), Some(T2_DEFAULT_SECS), "상한으로 자른다");
    }
}
