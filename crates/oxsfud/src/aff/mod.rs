// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-AFF-SERVER-SCOPE · 정:RULE-AFF-ONE-SET · 정:IMPL-AFF-PUB-ROOM · 정:IMPL-BUS-HOME-LOST · 정:IMPL-GC-PEER-REAP · 연:AFF.type.Affiliation · 연:RULE-AFF-PROC · 연:RULE-AFF-RES-FORM · 연:RULE-AFF-CAUSE-USER · 연:RULE-AFF-SINGLE-SPEAK-ROOM · 연:AFF.S.select · 연:AFF.S.select_moved · 연:AFF.S.away · 연:AFF.S.same · 연:AFF.S.join_select · 연:AFF.S.join_select_moved · 연:AFF.S.join_listen · 연:AFF.S.left · 연:AFF.S.left_other · model: claude-opus-5-5
//! 영역 AFF — 그 서버가 아는 그 세션(Peer): 관계 둘뿐(입장 방 집합 = 구독 집합 · `pub_room` RCU 값) · 홈(node, inst) · 전송 객체 · 발행 트랙(세션 것).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use common::bus::envelope::Home;
use oxsig::{Affiliation, PcMode, Permission};

use crate::link::{lock, Transport};
use crate::r#pub::PubTrack;
use crate::rtcp::core::Budget;

pub struct Peer {
    pub user: String,
    pub session: String,
    pub home: Mutex<Home>,
    pub pc_mode: PcMode,
    pub participant_type: u8,
    pub hidden: bool,
    pub metadata: Option<serde_json::Value>,
    pub token_permission: Option<Permission>,
    pub rooms: Mutex<BTreeSet<String>>,
    pub pub_room: Arc<ArcSwap<Option<String>>>,
    pub pubt: Arc<Transport>,
    pub subt: Arc<Transport>,
    pub tracks: ArcSwap<Vec<Arc<PubTrack>>>,
    pub budget: Arc<Mutex<Budget>>,
    pub ext_tags: Mutex<Option<Vec<oxsig::ExtMap>>>,
    pub created_ms: u64,
}

impl Peer {
    pub fn affiliation(&self) -> Affiliation {
        Affiliation { sub_rooms: self.rooms().into_iter().collect(), pub_room: self.pub_room() }
    }

    pub fn rooms(&self) -> BTreeSet<String> {
        lock(&self.rooms).clone()
    }

    pub fn in_room(&self, r: &str) -> bool {
        lock(&self.rooms).contains(r)
    }

    pub fn pub_room(&self) -> Option<String> {
        Option::clone(&self.pub_room.load())
    }

    pub fn set_pub_room(&self, r: Option<String>) {
        self.pub_room.store(Arc::new(r));
    }

    pub fn transports(&self) -> Vec<Arc<Transport>> {
        if Arc::ptr_eq(&self.pubt, &self.subt) { vec![self.pubt.clone()] } else { vec![self.pubt.clone(), self.subt.clone()] }
    }

    pub fn tracks(&self) -> Vec<Arc<PubTrack>> {
        Vec::clone(&self.tracks.load())
    }

    pub fn has_half(&self) -> bool {
        self.tracks.load().iter().any(|t| t.half.load(std::sync::atomic::Ordering::Relaxed))
    }

    pub fn track(&self, id: &str) -> Option<Arc<PubTrack>> {
        self.tracks.load().iter().find(|t| t.track_id == id).cloned()
    }

    pub fn track_by_ssrc(&self, ssrc: u32) -> Option<Arc<PubTrack>> {
        self.tracks.load().iter().find(|t| t.decl_ssrc == ssrc || t.phys.load().iter().any(|p| p.ssrc == ssrc)).cloned()
    }

    pub fn add_track(&self, t: Arc<PubTrack>) {
        self.tracks.rcu(|v| {
            let mut v = Vec::clone(v);
            v.push(t.clone());
            v
        });
    }

    pub fn drop_track(&self, id: &str) -> Option<Arc<PubTrack>> {
        let found = self.track(id);
        self.tracks.rcu(|v| {
            let mut v = Vec::clone(v);
            v.retain(|t| t.track_id != id);
            v
        });
        found
    }

    pub fn home(&self) -> Home {
        lock(&self.home).clone()
    }

    pub fn set_home(&self, h: &Home) {
        let slot: &mut Home = &mut lock(&self.home);
        if slot != h {
            let _ = std::mem::replace(slot, h.clone());
        }
    }

    pub fn close(&self) {
        for t in self.transports() {
            t.close();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Same,
    Select,
    Moved,
    Away,
}

impl Move {
    pub fn label(self, join: bool) -> &'static str {
        match (self, join) {
            (Move::Same, false) => "AFF.S.same",
            (Move::Select, false) => "AFF.S.select",
            (Move::Moved, false) => "AFF.S.select_moved",
            (Move::Away, _) => "AFF.S.away",
            (Move::Same, true) => "AFF.S.join_listen",
            (Move::Select, true) => "AFF.S.join_select",
            (Move::Moved, true) => "AFF.S.join_select_moved",
        }
    }
}

pub fn judge(current: Option<&str>, select: Option<&str>, deselect: Option<&str>) -> (Move, Option<String>) {
    match (select, deselect) {
        (Some(s), _) if current.is_none() => (Move::Select, Some(s.to_string())),
        (Some(s), _) if current != Some(s) => (Move::Moved, Some(s.to_string())),
        (None, Some(d)) if current == Some(d) => (Move::Away, None),
        _ => (Move::Same, current.map(str::to_string)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judge_orders_deselect_then_select() {
        assert_eq!(judge(None, Some("r1"), None), (Move::Select, Some("r1".into())));
        assert_eq!(judge(Some("r1"), Some("r3"), Some("r1")), (Move::Moved, Some("r3".into())), "S-sym-IMPL-AFF-PUB-ROOM-1 pub_select 단독도 옛 방을 묵시로 내린다");
        assert_eq!(judge(Some("r1"), None, Some("r1")), (Move::Away, None));
        assert_eq!(judge(Some("r1"), None, Some("r9")), (Move::Same, Some("r1".into())), "S-sym-IMPL-AFF-PUB-ROOM-2 모르는 방의 해제는 건너뛴다");
        assert_eq!(judge(Some("r1"), Some("r1"), None), (Move::Same, Some("r1".into())));
    }
}
