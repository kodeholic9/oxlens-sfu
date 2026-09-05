// author: kodeholic (powered by Claude)
//! Peer — 정§2-1 "사용자 하나 = 이 서버에 Peer 하나". 입장 방 집합(= 듣는 방)과 `pub_room` 둘뿐(정§5-1).
//! 전송·트랙은 뒤 판. 자격은 Peer 생성 때 한 번(연§9-3 세션 동안 불변).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use dashmap::DashMap;
use oxsig::schema::{Affiliation, PcMode};

use crate::transport::IceCredentials;

#[derive(Debug)]
pub struct Peer {
    pub user_id: String,
    pub participant_type: u8,
    pub pc_mode: PcMode,
    pub publish_ice: IceCredentials,
    pub subscribe_ice: IceCredentials,
    pub created_at_ms: u64,
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
            rooms: Mutex::new(Rooms::default()),
        }
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
