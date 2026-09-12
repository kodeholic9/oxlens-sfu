// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§4-1 · §4-1-1 · §4-2 · 연§5-4 · model: claude-opus-5

//! 방 수명 — ★**빈 그릇과 자리 둘.**
//!
//! ★★**수명·`3004` 판정은 등록 기준이고 `user_count` 는 보이는 수다 — 기준이 다른 것이 계약이다.**
//! 투명 봇만 남은 방은 `user_count` 가 `0` 이지만 ★**살아 있다**(녹화 중인 방을 폭파하지 않는다).

use oxsig::Code;

/// TTL 둘. ★`None` = 영구. ★`empty` 라는 낱말은 쓰지 않는다(업계가 반대 뜻으로 쓴다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ttl {
    /// *"아무도 안 들어옴"*.
    pub unused_secs: Option<u32>,
    /// *"다 나감"*.
    pub departure_secs: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub user_id: String,
    /// ★명단·정원에서 빠지지만 ★**수명 판정에는 센다.**
    pub hidden: bool,
    /// `1` 녹화.
    pub participant_type: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub capacity: u32,
    pub created_at: u64,
    pub ttl: Ttl,
    members: Vec<Member>,
    /// ★비기 시작한 시각. 생성 직후에는 ★**생성 시각**이다(비는 사건이 따로 없다).
    ///
    /// ★★**`None` 이 "안 비었다" 이고 `0` 을 표식으로 쓰지 않는다** — 시각 0 과 겹치면
    /// 기동 직후에 만든 방이 *"방금 비었다"* 로 읽혀 한 tick 을 헛되이 쓴다.
    empty_since: Option<u64>,
    ever_joined: bool,
    /// 방 공통 스냅샷 번호.
    pub seq: u64,
}

/// tick 이 낸 판단.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    /// 아무 일 없다.
    None,
    /// ★**"방금 비었다"의 최초 관측** — 이 tick 에는 만료를 판정하지 않는다.
    JustEmptied,
    /// 만료 — 폭파한다.
    Expired,
}

impl Room {
    pub fn new(id: String, name: String, capacity: u32, ttl: Ttl, now: u64) -> Self {
        Self {
            id,
            name,
            capacity,
            created_at: now,
            ttl,
            members: Vec::new(),
            empty_since: Some(now),
            ever_joined: false,
            seq: 0,
        }
    }

    /// ★**등록 기준** — 수명·`3004` 가 보는 수. `hidden` 도 센다.
    pub fn registered(&self) -> usize {
        self.members.len()
    }

    /// ★**보이는 수** — 명단·정원이 보는 수. 투명은 빠진다.
    pub fn user_count(&self) -> usize {
        self.members.iter().filter(|m| !m.hidden).count()
    }

    /// ★**녹화 중인가는 등록에서 센다** — 방 플래그로 저장하면 봇이 조용히 나간 뒤에도 켜져 있다.
    pub fn rec(&self) -> bool {
        self.members.iter().any(|m| m.participant_type == 1)
    }

    /// 정원은 ★**보이는 수**로 본다 — 투명 봇이 자리를 먹지 않는다.
    pub fn join(&mut self, m: Member, now: u64) -> Result<(), Code> {
        if !m.hidden && self.user_count() as u32 >= self.capacity {
            return Err(Code::RoomFull);
        }
        if self.members.iter().any(|x| x.user_id == m.user_id) {
            // 같은 신원의 재입장은 축출 뒤 성공이다(정§4-2) — 여기서는 자리를 바꿔 끼운다.
            self.members.retain(|x| x.user_id != m.user_id);
        }
        self.members.push(m);
        self.ever_joined = true;
        self.empty_since = None;
        let _ = now;
        Ok(())
    }

    pub fn leave(&mut self, user_id: &str) -> bool {
        let before = self.members.len();
        self.members.retain(|m| m.user_id != user_id);
        before != self.members.len()
    }

    /// ★**한 tick 에 한 걸음** — 비는 것을 본 tick 에는 만료를 판정하지 않는다.
    pub fn tick(&mut self, now: u64) -> Sweep {
        if self.registered() > 0 {
            self.empty_since = None;
            return Sweep::None;
        }
        let Some(since) = self.empty_since else {
            // ★CAS — "방금 비었다"의 최초 관측. 이 tick 에는 만료를 판정하지 않는다.
            self.empty_since = Some(now);
            return Sweep::JustEmptied;
        };
        let ttl = if self.ever_joined { self.ttl.departure_secs } else { self.ttl.unused_secs };
        match ttl {
            // ★`None` = 영구.
            None => Sweep::None,
            Some(secs) => {
                if now.saturating_sub(since) >= secs as u64 * 1_000 {
                    Sweep::Expired
                } else {
                    Sweep::None
                }
            }
        }
    }

    /// 명시 삭제 — ★**사람이 있으면 `3004`.** 운영 `destroy` 는 이 판정을 안 본다(운영 규격서 §4-3).
    pub fn may_destroy(&self) -> Result<(), Code> {
        if self.registered() > 0 {
            return Err(Code::RoomNotEmpty);
        }
        Ok(())
    }
}

/// 방 창고. ★**명시 id 생성은 멱등이다**(있으면 그 방을 돌려준다 — `name` 은 무시).
#[derive(Debug, Default)]
pub struct Rooms {
    items: Vec<Room>,
}

impl Rooms {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: &str) -> Option<&Room> {
        self.items.iter().find(|r| r.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Room> {
        self.items.iter_mut().find(|r| r.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Room> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// ★**멱등** — 같은 id 로 다시 만들면 그 방이 그대로 온다.
    pub fn create(&mut self, id: String, name: String, capacity: u32, ttl: Ttl, now: u64) -> &Room {
        if let Some(i) = self.items.iter().position(|r| r.id == id) {
            return &self.items[i];
        }
        self.items.push(Room::new(id, name, capacity, ttl, now));
        self.items.last().expect("방금 넣었다")
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|r| r.id != id);
        before != self.items.len()
    }

    /// 만료된 방을 낸다. ★**좀비 회수 뒤에 돈다**(급사 참가자가 남아 있으면 유예 시작이 늦는다).
    pub fn sweep(&mut self, now: u64) -> Vec<String> {
        let mut expired = Vec::new();
        for r in self.items.iter_mut() {
            if r.tick(now) == Sweep::Expired {
                expired.push(r.id.clone());
            }
        }
        expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ttl(u: u32, d: u32) -> Ttl {
        Ttl { unused_secs: Some(u), departure_secs: Some(d) }
    }

    fn member(id: &str, hidden: bool, pt: u8) -> Member {
        Member { user_id: id.into(), hidden, participant_type: pt }
    }

    #[test]
    fn 명시_id_생성은_멱등이다() {
        let mut rs = Rooms::new();
        rs.create("r1".into(), "첫 이름".into(), 10, ttl(300, 60), 0);
        let again = rs.create("r1".into(), "다른 이름".into(), 99, ttl(1, 1), 100);
        assert_eq!(again.name, "첫 이름", "★name 은 무시한다");
        assert_eq!(rs.len(), 1);
    }

    #[test]
    fn 투명은_정원을_안_먹고_명단에도_없다() {
        let mut r = Room::new("r1".into(), "n".into(), 1, ttl(300, 60), 0);
        r.join(member("u1", false, 0), 0).expect("join");
        // ★정원 1 인데 투명 봇은 들어온다.
        r.join(member("bot", true, 1), 0).expect("hidden join");
        assert_eq!(r.user_count(), 1, "★보이는 수");
        assert_eq!(r.registered(), 2, "★등록 기준");
        assert_eq!(r.join(member("u2", false, 0), 0), Err(Code::RoomFull));
    }

    #[test]
    fn 녹화는_등록에서_센다() {
        // ★방 플래그로 저장하면 봇이 조용히 나간 뒤에도 '녹화 중'이 켜져 있다.
        let mut r = Room::new("r1".into(), "n".into(), 10, ttl(300, 60), 0);
        r.join(member("bot", true, 1), 0).expect("join");
        assert!(r.rec());
        r.leave("bot");
        assert!(!r.rec());
    }

    #[test]
    fn 투명_봇만_남은_방은_안_폭파된다() {
        // ★수명 판정을 user_count 로 하면 녹화 중인 방이 TTL 로 폭파된다.
        let mut r = Room::new("r1".into(), "n".into(), 10, ttl(1, 1), 0);
        r.join(member("bot", true, 1), 0).expect("join");
        assert_eq!(r.user_count(), 0);
        assert_eq!(r.tick(999_999), Sweep::None, "★등록이 남아 있으면 산다");
    }

    #[test]
    fn 비는_것을_본_tick_에는_판정하지_않는다() {
        let mut r = Room::new("r1".into(), "n".into(), 10, ttl(300, 1), 0);
        r.join(member("u1", false, 0), 0).expect("join");
        r.leave("u1");
        assert_eq!(r.tick(10_000), Sweep::JustEmptied, "★최초 관측");
        assert_eq!(r.tick(10_500), Sweep::None, "★아직 1초 전");
        assert_eq!(r.tick(11_000), Sweep::Expired);
    }

    #[test]
    fn 안_들어온_방과_다_나간_방은_다른_시계를_쓴다() {
        // unused — 기준 시각은 ★생성 시각이다(비는 사건이 따로 없다).
        let mut fresh = Room::new("r1".into(), "n".into(), 10, ttl(1, 300), 0);
        assert_eq!(fresh.tick(2_000), Sweep::Expired);

        let mut used = Room::new("r2".into(), "n".into(), 10, ttl(1, 300), 0);
        used.join(member("u1", false, 0), 0).expect("join");
        used.leave("u1");
        used.tick(1_000);
        assert_eq!(used.tick(2_000), Sweep::None, "★한 번 쓰인 방은 departure 시계다");
    }

    #[test]
    fn 영구_방은_안_거둔다() {
        let mut r = Room::new("r1".into(), "n".into(), 10, Ttl { unused_secs: None, departure_secs: None }, 0);
        r.tick(1);
        assert_eq!(r.tick(u64::MAX / 2), Sweep::None);
    }

    #[test]
    fn 사람이_있으면_삭제가_3004_다() {
        let mut r = Room::new("r1".into(), "n".into(), 10, ttl(300, 60), 0);
        r.join(member("bot", true, 1), 0).expect("join");
        // ★투명도 사람으로 센다 — 녹화 중인 방을 지우지 않기 위해서다.
        assert_eq!(r.may_destroy(), Err(Code::RoomNotEmpty));
    }

    #[test]
    fn 재입장은_자리를_바꿔_낀다() {
        let mut r = Room::new("r1".into(), "n".into(), 10, ttl(300, 60), 0);
        r.join(member("u1", false, 0), 0).expect("join");
        r.join(member("u1", false, 0), 1).expect("rejoin");
        assert_eq!(r.registered(), 1, "★같은 신원이 둘이 되지 않는다");
    }
}
