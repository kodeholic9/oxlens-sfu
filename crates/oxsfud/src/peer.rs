// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§4-2 · §7-2 · 연§4-1 · §4-3 · model: claude-opus-5

//! Peer — ★**서버당 하나다**(연§4-2). 방이 여럿이어도 연결은 하나이고,
//! 받기 `mid` 예산·PT 표·ICE 자격이 전부 이 단위에 산다.
//!
//! ★★**그래서 같은 신원의 재입장이 "그 서버의 다른 방 멤버십까지" 지운다**(정§4-2 ②) —
//! 지우는 것이 방이 아니라 ★**Peer** 이기 때문이다.

use std::collections::BTreeMap;

use oxsig::body::session::PcMode;
use oxsig::types::{Assign, Kind};

use crate::identity::IceCreds;

/// 받기 `mid` 발급기(연§4-1 규칙 셋).
///
/// ★**`1pc` 은 `32` 부터** — 내 보내기 m-line(브라우저 배정 0~31)과 한 BUNDLE 이라 비켜 준다.
/// `2pc` 는 받기 연결에 클라 m-line 이 아예 없어 비켜 줄 상대가 없다(`0` 부터).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidPool {
    next: u16,
    /// 해제분. ★**같은 `kind` 에만 되쓴다** — m-line 의 media type 을 바꾸면
    /// 재활용되지 않는 절에서 브라우저(libwebrtc)가 거부한다(구현 근거, 연§4-1).
    free: Vec<(Kind, u16)>,
}

impl MidPool {
    pub fn new(mode: PcMode) -> Self {
        Self { next: if mode == PcMode::One { 32 } else { 0 }, free: Vec::new() }
    }

    /// ★**해제분이 있으면 그것부터 — 의무다**(연§4-1 ③, 20260911).
    ///
    /// 안 그러면 전환이 잦을수록 m-line 수가 새 값으로만 자라 ★**천장에 먼저 닿는다.**
    pub fn take(&mut self, kind: Kind) -> String {
        if let Some(i) = self.free.iter().position(|(k, _)| *k == kind) {
            let (_, mid) = self.free.remove(i);
            return mid.to_string();
        }
        let mid = self.next;
        self.next += 1;
        mid.to_string()
    }

    pub fn give(&mut self, kind: Kind, mid: &str) {
        if let Ok(v) = mid.parse::<u16>() {
            self.free.push((kind, v));
        }
    }

    /// 지금 쥐고 있는 받기 m-section 개수 — ★`4005` 는 값 폭이 아니라 이 수로 난다(연§6-2).
    pub fn in_use(&self, base: u16) -> usize {
        (self.next - base) as usize - self.free.len()
    }
}

/// 한 세션의 Peer. ★**미디어 자원 전부가 여기 매달린다.**
#[derive(Debug, Clone)]
pub struct Peer {
    pub session_id: String,
    pub user_id: String,
    pub pc_mode: PcMode,
    pub ice: IceCreds,
    /// 내가 듣는 방들 = ★**이 서버에 입장한 방들**(입장이 곧 구독).
    pub sub_rooms: Vec<String>,
    /// ★**하나뿐이라 단수다.**
    pub pub_room: Option<String>,
    pub mids: MidPool,
    /// `track_id` → 내 자리. ★**수신자별 층**이라 Peer 가 쥔다(연§4-1-1).
    pub assigns: BTreeMap<String, Assign>,
    /// ★**구독자 PT 표 — 연결마다 하나**(정§7-2-1). egress 의 PT 가 이 표의 값이다.
    pub pt: crate::pt::PtTable,
    /// `READY{tracks}` 를 받았나 — ★**게이트는 전이중 video 에만** 걸린다(정§7-4).
    pub ready: bool,
}

impl Peer {
    pub fn new(session_id: String, user_id: String, pc_mode: PcMode) -> Self {
        Self {
            session_id,
            user_id,
            pc_mode,
            ice: IceCreds::fresh(),
            sub_rooms: Vec::new(),
            pub_room: None,
            mids: MidPool::new(pc_mode),
            assigns: BTreeMap::new(),
            pt: crate::pt::PtTable::new(),
            ready: false,
        }
    }

    /// ★**내가 받는 연결의 자격** — `1pc` 은 연결이 하나라 보내기 자격 그것이다(연§9-10).
    ///
    /// 여기서 `2pc` 의 것을 `1pc` 에 쓰면 그 자격은 아무도 latch 하지 않아
    /// ★**보낼 주소가 영영 안 잡힌다** — 영상이 조용히 안 나온다.
    pub fn recv_ufrag(&self) -> &str {
        match self.pc_mode {
            PcMode::One => &self.ice.publish_ufrag,
            PcMode::Two => &self.ice.subscribe_ufrag,
        }
    }

    pub fn affiliation(&self) -> oxsig::Affiliation {
        oxsig::Affiliation { sub_rooms: self.sub_rooms.clone(), pub_room: self.pub_room.clone() }
    }
}

/// `ensure` 가 낸 것. ★**튜플로 두면 부르는 쪽마다 자리를 헷갈린다.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ensured {
    pub idx: usize,
    /// 걷힌 Peer 가 듣던 방들 — 부르는 쪽이 방마다 `left` 를 내야 한다(정§4-2 ②).
    pub orphaned: Vec<String>,
    /// 걷힌 Peer 의 세션 — ★그 세션의 전송 자격도 같이 내린다.
    pub evicted: Option<String>,
    /// ★**이번에 세웠나** — 세웠으면 ICE 자격을 장부에 올려야 한다.
    pub created: bool,
}

/// 이 유닛의 Peer 들. ★**키는 세션이다** — 같은 사람의 새 세션은 새 Peer 다.
#[derive(Debug, Default)]
pub struct Peers {
    items: Vec<Peer>,
}

impl Peers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, session_id: &str) -> Option<&Peer> {
        self.items.iter().find(|p| p.session_id == session_id)
    }

    pub fn get_mut(&mut self, session_id: &str) -> Option<&mut Peer> {
        self.items.iter_mut().find(|p| p.session_id == session_id)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 그 세션의 Peer 를 낸다. ★**없으면 세운다** — 그 자리에서 옛 Peer 도 걷어낸다.
    ///
    /// 걷힌 Peer 의 방 목록을 돌려준다 — 부르는 쪽이 그 방들에서 ★**퇴장시켜야 한다**
    /// (정§4-2 ② *"옛 Peer 를 통째로 축출"* · 방마다 `left`).
    pub fn ensure(&mut self, session_id: &str, user_id: &str, mode: PcMode) -> Ensured {
        if let Some(i) = self.items.iter().position(|p| p.session_id == session_id) {
            return Ensured { idx: i, orphaned: Vec::new(), evicted: None, created: false };
        }
        // ★같은 신원의 옛 Peer — 통째로 걷는다(방이 아니라 Peer 가 단위다).
        let mut orphaned = Vec::new();
        let mut evicted = None;
        if let Some(i) = self.items.iter().position(|p| p.user_id == user_id) {
            orphaned = self.items[i].sub_rooms.clone();
            evicted = Some(self.items[i].session_id.clone());
            self.items.remove(i);
        }
        self.items.push(Peer::new(session_id.into(), user_id.into(), mode));
        Ensured { idx: self.items.len() - 1, orphaned, evicted, created: true }
    }

    pub fn at(&self, i: usize) -> &Peer {
        &self.items[i]
    }

    pub fn at_mut(&mut self, i: usize) -> &mut Peer {
        &mut self.items[i]
    }

    /// 세션이 갔다 — Peer 가 쥐고 있던 방들을 돌려준다.
    pub fn drop_session(&mut self, session_id: &str) -> Vec<String> {
        let Some(i) = self.items.iter().position(|p| p.session_id == session_id) else {
            return Vec::new();
        };
        let rooms = self.items[i].sub_rooms.clone();
        self.items.remove(i);
        rooms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 받기_mid_시작은_모드가_정한다() {
        assert_eq!(MidPool::new(PcMode::One).take(Kind::Audio), "32");
        assert_eq!(MidPool::new(PcMode::Two).take(Kind::Audio), "0");
    }

    #[test]
    fn 해제분이_있으면_그것부터_쓴다() {
        let mut p = MidPool::new(PcMode::Two);
        let a = p.take(Kind::Audio);
        let b = p.take(Kind::Audio);
        assert_eq!((a.as_str(), b.as_str()), ("0", "1"));
        p.give(Kind::Audio, "0");
        // ★새 값(2)이 아니라 해제분(0)이다 — 의무다(연§4-1 ③).
        assert_eq!(p.take(Kind::Audio), "0");
        // ★같은 kind 의 해제분이 없으면 새 값이다 — media type 을 바꾸지 않는다.
        p.give(Kind::Audio, "1");
        assert_eq!(p.take(Kind::Video), "2");
    }

    #[test]
    fn 같은_신원의_새_세션은_옛_peer_를_통째로_걷는다() {
        let mut ps = Peers::new();
        let e = ps.ensure("s-1", "u1", PcMode::One);
        assert!(e.orphaned.is_empty() && e.created && e.evicted.is_none());
        ps.at_mut(e.idx).sub_rooms = vec!["r1".into(), "r2".into()];
        let e = ps.ensure("s-2", "u1", PcMode::Two);
        // ★방 하나가 아니라 ★**그 Peer 의 방 전부**가 딸려 나간다(정§4-2 ②).
        assert_eq!(e.orphaned, vec!["r1".to_string(), "r2".to_string()]);
        assert_eq!(e.evicted.as_deref(), Some("s-1"));
        assert_eq!(ps.len(), 1, "옛 Peer 는 남지 않는다");

        // ★같은 세션이 또 들어오면 세우지 않는다 — 자격을 다시 발급하면 전송이 끊긴다.
        let again = ps.ensure("s-2", "u1", PcMode::Two);
        assert!(!again.created && again.evicted.is_none());
    }
}
