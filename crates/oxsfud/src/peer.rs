// author: kodeholic (powered by Claude)
//! Peer — 정§2-1 "사용자 하나 = 이 서버에 Peer 하나". 입장 방 집합(= 듣는 방)과 `pub_room` 둘뿐(정§5-1).
//! 전송 생존(정§2-2 PeerState)은 ★UDP 관찰 기준이다 — WS 하트비트는 이 축을 갱신하지 않는다.
//! 자격은 Peer 생성 때 한 번(연§9-3 세션 동안 불변). 트랙은 뒤 판.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use dashmap::DashMap;
use oxsig::schema::{Affiliation, PcMode};

use crate::transport::IceCredentials;

/// 정§2-2 — 전송 생존 3벌 중 Peer 의 것. ★정수 비교 금지, enum 동등성만.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Alive,
    Suspect,
    Zombie,
}

impl PeerState {
    fn code(self) -> u8 {
        match self {
            Self::Alive => 0,
            Self::Suspect => 1,
            Self::Zombie => 2,
        }
    }
    fn from_code(v: u8) -> Self {
        match v {
            1 => Self::Suspect,
            2 => Self::Zombie,
            _ => Self::Alive,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alive => "alive",
            Self::Suspect => "suspect",
            Self::Zombie => "zombie",
        }
    }
}

/// 정§17-1 회수 주기.
pub const REAPER_TICK_MS: u64 = 5_000;
/// 정§2-2 전이 수치 — 판정은 전부 초과(`>`).
pub const SUSPECT_AFTER_MS: u64 = 15_000;
pub const ZOMBIE_AFTER_MS: u64 = 20_000;

/// 정§2-2 전이표 — ★순수 함수(시계는 인자, §2-3 계약 6). `None` = 전이 없음.
/// `last_seen == 0` 은 "판정 불가"이지 "오래됨"이 아니다 — 접속 직후 즉사를 막는다.
pub fn judge(state: PeerState, last_seen: u64, now: u64) -> Option<PeerState> {
    if last_seen == 0 || state == PeerState::Zombie {
        return None;
    }
    let elapsed = now.saturating_sub(last_seen);
    let next = if elapsed > ZOMBIE_AFTER_MS {
        PeerState::Zombie
    } else if elapsed > SUSPECT_AFTER_MS {
        PeerState::Suspect
    } else {
        PeerState::Alive
    };
    (next != state).then_some(next)
}

#[derive(Debug)]
pub struct Peer {
    pub user_id: String,
    pub participant_type: u8,
    pub pc_mode: PcMode,
    pub publish_ice: IceCredentials,
    pub subscribe_ice: IceCredentials,
    pub created_at_ms: u64,
    state: AtomicU8,
    last_seen: AtomicU64,
    suspect_since: AtomicU64,
    rooms: Mutex<Rooms>,
}

#[derive(Debug, Default)]
struct Rooms {
    sub: BTreeSet<String>,
    pub_room: Option<String>,
}

impl Peer {
    pub fn new(user_id: &str, participant_type: u8, pc_mode: PcMode, now_ms: u64) -> Self {
        Self {
            user_id: user_id.to_owned(),
            participant_type,
            pc_mode,
            publish_ice: IceCredentials::generate(),
            subscribe_ice: IceCredentials::generate(),
            created_at_ms: now_ms,
            state: AtomicU8::new(PeerState::Alive.code()),
            last_seen: AtomicU64::new(0),
            suspect_since: AtomicU64::new(0),
            rooms: Mutex::new(Rooms::default()),
        }
    }

    /// 정§2-2 — 갱신원은 UDP 관찰 둘뿐(매칭된 SRTP/RTP/RTCP · STUN Binding).
    pub fn touch(&self, now_ms: u64) {
        self.last_seen.store(now_ms, Ordering::Relaxed);
    }

    pub fn last_seen(&self) -> u64 {
        self.last_seen.load(Ordering::Relaxed)
    }

    pub fn state(&self) -> PeerState {
        PeerState::from_code(self.state.load(Ordering::Acquire))
    }

    /// 정§2-3 계약 3 — `swap` 으로 이전 값을 원자 회수해 실제로 바뀐 경우에만 부수효과를 낸다.
    /// 반환: (바뀌었나, Suspect 체류 시간) — 회수 로그가 이 값을 동봉한다(정§17-3).
    pub fn transition(&self, next: PeerState, now_ms: u64) -> (bool, u64) {
        let prev = PeerState::from_code(self.state.swap(next.code(), Ordering::AcqRel));
        if prev == next {
            return (false, 0);
        }
        let dwell = match next {
            PeerState::Suspect => {
                let _ = self.suspect_since.compare_exchange(0, now_ms, Ordering::AcqRel, Ordering::Relaxed);
                0
            }
            PeerState::Alive => {
                self.suspect_since.swap(0, Ordering::AcqRel);
                0
            }
            PeerState::Zombie => match self.suspect_since.load(Ordering::Acquire) {
                0 => 0,
                since => now_ms.saturating_sub(since),
            },
        };
        (true, dwell)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Rooms> {
        self.rooms.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn affiliation(&self) -> Affiliation {
        let g = self.lock();
        Affiliation { sub_rooms: g.sub.iter().cloned().collect(), pub_room: g.pub_room.clone() }
    }
    pub fn room_count(&self) -> usize {
        self.lock().sub.len()
    }
    pub fn rooms(&self) -> Vec<String> {
        self.lock().sub.iter().cloned().collect()
    }
    pub fn is_in(&self, room_id: &str) -> bool {
        self.lock().sub.contains(room_id)
    }
    pub fn pub_room(&self) -> Option<String> {
        self.lock().pub_room.clone()
    }

    pub fn join_room(&self, room_id: &str, select: bool) {
        let mut g = self.lock();
        g.sub.insert(room_id.to_owned());
        if select {
            g.pub_room = Some(room_id.to_owned());
        }
    }

    /// 정§17-2 ④ + §5-2 cascade — 발행 방이면 `pub_room=null`. 반환: (있었나, 마지막 방이었나, 발행 방을 뺐나).
    pub fn leave_room(&self, room_id: &str) -> LeaveOutcome {
        let mut g = self.lock();
        let was_member = g.sub.remove(room_id);
        let pub_cleared = g.pub_room.as_deref() == Some(room_id);
        if pub_cleared {
            g.pub_room = None;
        }
        LeaveOutcome { was_member, last_room: g.sub.is_empty(), pub_cleared }
    }

    /// 정§5-2 — `pub_deselect` 는 현재 값과 일치할 때만(멱등), `pub_select` 는 입장 방일 때만(아니면 skip). 반환: 바뀐 방들.
    pub fn apply_affiliation(&self, pub_deselect: Option<&str>, pub_select: Option<&str>) -> Vec<String> {
        let mut g = self.lock();
        let mut changed = Vec::new();
        if let Some(d) = pub_deselect
            && g.pub_room.as_deref() == Some(d)
        {
            g.pub_room = None;
            changed.push(d.to_owned());
        }
        if let Some(s) = pub_select
            && g.sub.contains(s)
            && g.pub_room.as_deref() != Some(s)
        {
            if let Some(prev) = g.pub_room.replace(s.to_owned())
                && !changed.contains(&prev)
            {
                changed.push(prev);
            }
            changed.push(s.to_owned());
        }
        changed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaveOutcome {
    pub was_member: bool,
    pub last_room: bool,
    pub pub_cleared: bool,
}

/// user_id → Peer. 정§4-2 ② 재입장 판정의 근거.
#[derive(Default)]
pub struct PeerMap {
    peers: DashMap<String, Arc<Peer>>,
}

impl PeerMap {
    pub fn get(&self, user_id: &str) -> Option<Arc<Peer>> {
        self.peers.get(user_id).map(|p| p.clone())
    }
    pub fn insert(&self, peer: Arc<Peer>) -> Option<Arc<Peer>> {
        self.peers.insert(peer.user_id.clone(), peer)
    }
    pub fn remove(&self, user_id: &str) -> Option<Arc<Peer>> {
        self.peers.remove(user_id).map(|(_, p)| p)
    }
    /// §2-3 계약 4 — 회수 tick 이 쓰는 스냅샷. 순회와 삭제를 겹치지 않는다.
    pub fn snapshot(&self) -> Vec<Arc<Peer>> {
        self.peers.iter().map(|e| e.clone()).collect()
    }
    pub fn len(&self) -> usize {
        self.peers.len()
    }
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> Peer {
        Peer::new("u", 0, PcMode::TwoPc, 0)
    }

    #[test]
    fn peer_state_is_judged_on_udp_observation_only() {
        assert_eq!(judge(PeerState::Alive, 0, 60_000), None, "미관찰은 판정 불가");
        assert_eq!(judge(PeerState::Alive, 50_000, 60_000), None, "여유 안");
        assert_eq!(judge(PeerState::Alive, 44_000, 60_000), Some(PeerState::Suspect));
        assert_eq!(judge(PeerState::Alive, 45_000, 60_000), None, "15,000 은 초과가 아니다");
        assert_eq!(judge(PeerState::Suspect, 39_000, 60_000), Some(PeerState::Zombie));
        assert_eq!(judge(PeerState::Suspect, 40_000, 60_000), None, "20,000 도 초과가 아니다");
        assert_eq!(judge(PeerState::Alive, 10, 60_000), Some(PeerState::Zombie), "tick 을 건너뛰어도 좀비가 먼저");
        assert_eq!(judge(PeerState::Suspect, 59_000, 60_000), Some(PeerState::Alive), "패킷 재개");
        assert_eq!(judge(PeerState::Zombie, 10, 60_000), None, "좀비는 종착이 아니라 삭제다");
        assert_eq!(judge(PeerState::Alive, 90_000, 60_000), None, "시계 역행 방어");
    }

    #[test]
    fn transition_fires_once_and_reports_dwell() {
        let p = peer();
        assert_eq!(p.state(), PeerState::Alive);
        assert_eq!((p.last_seen(), judge(p.state(), p.last_seen(), 1)), (0, None));
        p.touch(1_000);
        assert_eq!(p.last_seen(), 1_000);
        assert_eq!(p.transition(PeerState::Suspect, 20_000), (true, 0));
        assert_eq!(p.transition(PeerState::Suspect, 25_000), (false, 0), "같은 값으로는 부수효과 없음");
        assert_eq!(p.transition(PeerState::Zombie, 25_000), (true, 5_000), "suspect 체류 5초 = 정상 경로");
        let q = peer();
        q.touch(1);
        assert_eq!(q.transition(PeerState::Zombie, 30_000), (true, 0), "체류 0 = 급사");
        let r = peer();
        r.transition(PeerState::Suspect, 10_000);
        r.transition(PeerState::Alive, 12_000);
        assert_eq!(r.transition(PeerState::Zombie, 40_000), (true, 0), "돌아왔다 다시 죽으면 체류는 새로 센다");
    }

    #[test]
    fn join_leave_cascade() {
        let p = peer();
        p.join_room("a", false);
        p.join_room("b", true);
        assert_eq!(p.affiliation(), Affiliation { sub_rooms: vec!["a".into(), "b".into()], pub_room: Some("b".into()) });
        assert_eq!(p.leave_room("b"), LeaveOutcome { was_member: true, last_room: false, pub_cleared: true });
        assert_eq!(p.leave_room("b"), LeaveOutcome { was_member: false, last_room: false, pub_cleared: false });
        assert_eq!(p.leave_room("a"), LeaveOutcome { was_member: true, last_room: true, pub_cleared: false });
        assert!(p.affiliation().is_consistent());
    }

    #[test]
    fn affiliation_rules() {
        let p = peer();
        p.join_room("a", true);
        p.join_room("b", false);
        assert!(p.apply_affiliation(None, Some("zzz")).is_empty(), "not a member → skip");
        assert!(p.apply_affiliation(Some("b"), None).is_empty(), "mismatch deselect → ignored");
        assert_eq!(p.pub_room().as_deref(), Some("a"));
        let mut ch = p.apply_affiliation(Some("a"), Some("b"));
        ch.sort();
        assert_eq!(ch, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(p.pub_room().as_deref(), Some("b"));
        let mut ch = p.apply_affiliation(None, Some("a"));
        ch.sort();
        assert_eq!(ch, vec!["a".to_owned(), "b".to_owned()], "switch without deselect bumps both");
        assert!(p.apply_affiliation(None, Some("a")).is_empty(), "no-op select");
        assert_ne!(p.publish_ice, p.subscribe_ice);
    }
}
