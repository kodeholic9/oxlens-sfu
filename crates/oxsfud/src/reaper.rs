// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§2-2 · §17-2 · model: claude-opus-5

//! 전송 생존 판정 — ★**UDP 관찰 하나만 본다.**
//!
//! ★★**WS `HEARTBEAT` 는 이 축을 갱신하지 않는다**(정§2-2) — 시그널이 살아도 미디어가
//! 죽었으면 회수한다. 제어와 미디어를 ★**독립으로 관찰하는 것이 의도**다.
//!
//! ★**한 값으로 합치지 않는다** — 전송 생존과 미디어 흐름 단계는 다른 축이고, 합쳤던
//! 구 5단계 모형이 ★**비교 반전으로 감지 전멸**을 냈다(20260816). 그래서 `enum` 동등성만 쓴다.

/// reaper 가 도는 주기.
pub const TICK_MS: u64 = 5_000;
/// 이만큼 조용하면 의심.
pub const SUSPECT_MS: u64 = 15_000;
/// 이만큼 조용하면 ★**회수**다 — `Zombie` 는 종착이 아니라 삭제다(정§2-2).
pub const ZOMBIE_MS: u64 = 20_000;

const _: () = assert!(SUSPECT_MS < ZOMBIE_MS, "★의심이 회수보다 늦으면 Suspect 를 아무도 못 본다");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PeerState {
    #[default]
    Alive,
    Suspect,
    Zombie,
}

/// 그 Peer 의 판정 상태 — ★**의심에 처음 들어간 시각을 붙들고 있는다.**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Health {
    pub state: PeerState,
    /// ★**최초 진입 시각을 고정**한다 — 매 tick 갱신하면 체류 시간이 늘 0 이 된다.
    pub suspect_since: Option<u64>,
}

impl Health {
    /// 한 tick 의 판정. ★**판정은 전부 초과(`>`)이고 zombie 를 먼저 본다**(정§2-2).
    ///
    /// `last_seen == 0` 은 ★**"판정 불가"이지 "오래됨"이 아니다** — UDP 를 아직 한 번도
    /// 못 본 것이라 접속 직후 즉사를 막는다.
    pub fn tick(self, last_seen: u64, now: u64) -> Self {
        if last_seen == 0 {
            return self;
        }
        // ★시계 역행 방어 — 빼기가 음수로 감기면 그 순간 전원이 좀비가 된다.
        let elapsed = now.saturating_sub(last_seen);
        if elapsed > ZOMBIE_MS {
            return Self { state: PeerState::Zombie, suspect_since: self.suspect_since };
        }
        if elapsed > SUSPECT_MS {
            return Self {
                state: PeerState::Suspect,
                // ★**CAS 자리** — 이미 있으면 그대로 둔다.
                suspect_since: self.suspect_since.or(Some(now)),
            };
        }
        // 패킷이 재개됐다 — 의심을 푼다.
        Self { state: PeerState::Alive, suspect_since: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 관찰_전에는_판정하지_않는다() {
        // ★접속 직후 즉사 방지 — `0` 은 오래된 것이 아니다.
        assert_eq!(Health::default().tick(0, 10_000_000), Health::default());
    }

    #[test]
    fn 조용해지면_의심에서_회수로_간다() {
        let h = Health::default();
        let h = h.tick(1_000, 1_000 + SUSPECT_MS);
        assert_eq!(h.state, PeerState::Alive, "★경계는 초과다 — 같으면 아직이다");

        let h = h.tick(1_000, 1_000 + SUSPECT_MS + 1);
        assert_eq!((h.state, h.suspect_since), (PeerState::Suspect, Some(1_000 + SUSPECT_MS + 1)));

        // ★최초 진입 시각은 고정된다 — 매 tick 갱신하면 체류 시간이 늘 0 이다.
        let h2 = h.tick(1_000, 1_000 + SUSPECT_MS + 5_000);
        assert_eq!(h2.suspect_since, h.suspect_since);

        assert_eq!(h.tick(1_000, 1_000 + ZOMBIE_MS).state, PeerState::Suspect);
        assert_eq!(h.tick(1_000, 1_000 + ZOMBIE_MS + 1).state, PeerState::Zombie);
    }

    #[test]
    fn 패킷이_재개되면_의심이_풀린다() {
        let h = Health::default().tick(1_000, 1_000 + SUSPECT_MS + 1);
        assert_eq!(h.state, PeerState::Suspect);
        let h = h.tick(50_000, 50_100);
        assert_eq!((h.state, h.suspect_since), (PeerState::Alive, None));
    }

    #[test]
    fn 시계가_뒤로_가도_좀비가_안_된다() {
        // ★빼기가 감기면 그 순간 전원이 좀비가 된다.
        let h = Health::default().tick(10_000, 5_000);
        assert_eq!(h.state, PeerState::Alive);
    }
}
