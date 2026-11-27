// author: kodeholic (powered by Claude)
//! 발행 mid 자리 풀 — 서버(연결)마다 하나(연:RULE-LINK-SDP-1PC 표 「클라가 자기 m-line 을 32개 미만으로」 — `mid` 는 0~31 중 비어 있는 가장 작은 값 · 비게 된 자리는 같은 kind · 구조에 되쓴다).
//! 선례는 웹 `internal/link/transport.ts` 의 `acquireSlot` · `freeMid` · `releaseSlot` 이다 — 쉬는 자리 가운데 kind · 시뮬캐스트가 같은 첫 자리를 되쓰고, 없으면 자리 장부(쉬는 자리 포함 · `1pc` 면 구독 mid 도)에 없는 가장 작은 값으로 새 자리를 낸다.
//! 쉬는 자리의 mid 는 다른 구조에 주지 않는다(웹과 같다). 32 를 다 쓰면 자리가 없다.
//! 자리를 쥔 쪽은 발행 트랙 하나다 — 트랙을 잊을 때 · 다른 서버로 옮길 때 돌려준다.

use std::collections::HashMap;

use crate::generated::ws_wire::Kind;

pub(crate) const MID_SPACE: u8 = 32;

pub(crate) fn reseat(pools: &mut HashMap<String, Pool>, from: &str, held: Option<u8>, to: &str, kind: Kind, simulcast: bool, recv: &[u8]) -> Option<u8> {
    if let (Some(mid), Some(p)) = (held, pools.get_mut(from)) {
        p.release(mid);
    }
    pools.entry(to.to_string()).or_default().acquire(kind, simulcast, recv)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    pub kind: Kind,
    pub simulcast: bool,
    pub mid: u8,
    pub owned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    Reuse(usize),
    Fresh(u8),
    Full,
}

pub(crate) fn pick(slots: &[Slot], kind: Kind, simulcast: bool, recv: &[u8]) -> Pick {
    if let Some(i) = slots.iter().position(|s| !s.owned && s.kind == kind && s.simulcast == simulcast) {
        return Pick::Reuse(i);
    }
    (0..MID_SPACE).find(|m| !slots.iter().any(|s| s.mid == *m) && !recv.contains(m)).map_or(Pick::Full, Pick::Fresh)
}

#[derive(Debug, Default)]
pub(crate) struct Pool {
    slots: Vec<Slot>,
}

impl Pool {
    pub(crate) fn acquire(&mut self, kind: Kind, simulcast: bool, recv: &[u8]) -> Option<u8> {
        match pick(&self.slots, kind, simulcast, recv) {
            Pick::Reuse(i) => self.slots.get_mut(i).map(|s| {
                s.owned = true;
                s.mid
            }),
            Pick::Fresh(mid) => {
                self.slots.push(Slot { kind, simulcast, mid, owned: true });
                Some(mid)
            }
            Pick::Full => None,
        }
    }

    pub(crate) fn release(&mut self, mid: u8) {
        if let Some(s) = self.slots.iter_mut().find(|s| s.mid == mid) {
            s.owned = false;
        }
    }

    #[cfg(test)]
    pub(crate) fn owned(&self) -> Vec<u8> {
        self.slots.iter().filter(|s| s.owned).map(|s| s.mid).collect()
    }
}

#[cfg(test)]
mod mid_pool_test {
    use super::*;

    #[test]
    fn a_new_slot_takes_the_smallest_mid_not_in_the_book() {
        let mut p = Pool::default();
        assert_eq!(p.acquire(Kind::Audio, false, &[]), Some(0), "연:RULE-LINK-SDP-1PC 0~31 중 비어 있는 가장 작은 값");
        assert_eq!(p.acquire(Kind::Video, false, &[]), Some(1));
        assert_eq!(p.acquire(Kind::Video, true, &[]), Some(2));
        assert_eq!(p.acquire(Kind::Audio, false, &[3, 4]), Some(5), "1pc 구독 mid 는 장부에 있다");
    }

    #[test]
    fn a_released_slot_is_reused_only_by_the_same_kind_and_structure() {
        let mut p = Pool::default();
        let a = p.acquire(Kind::Audio, false, &[]);
        let v = p.acquire(Kind::Video, false, &[]);
        assert_eq!((a, v), (Some(0), Some(1)));
        p.release(0);
        assert_eq!(p.acquire(Kind::Video, false, &[]), Some(2), "쉬는 오디오 자리는 영상에 주지 않는다");
        assert_eq!(p.acquire(Kind::Video, true, &[]), Some(3), "시뮬캐스트 ↔ 단일은 되쓰지 않는다");
        assert_eq!(p.acquire(Kind::Audio, false, &[]), Some(0), "같은 kind · 구조는 쉬는 자리를 되쓴다");
        p.release(1);
        p.release(2);
        assert_eq!(p.acquire(Kind::Video, false, &[]), Some(1), "쉬는 자리 가운데 먼저 난 자리");
    }

    #[test]
    fn thirty_three_on_off_cycles_stay_inside_0_to_31() {
        let mut p = Pool::default();
        let a = p.acquire(Kind::Audio, false, &[]);
        assert_eq!(a, Some(0));
        let mut seen = Vec::new();
        for _ in 0..33 {
            let v = p.acquire(Kind::Video, false, &[32, 33, 34]);
            seen.push(v);
            if let Some(m) = v {
                p.release(m);
            }
        }
        assert!(seen.iter().all(|m| *m == Some(1)), "켜고 끄기를 33 번 해도 한 자리를 되쓴다: {seen:?}");
        assert_eq!(p.owned(), vec![0]);
    }

    #[test]
    fn the_33rd_concurrent_publish_has_no_slot() {
        let mut p = Pool::default();
        let got: Vec<Option<u8>> = (0..32).map(|_| p.acquire(Kind::Video, false, &[])).collect();
        assert_eq!(got, (0..MID_SPACE).map(Some).collect::<Vec<_>>());
        assert_eq!(p.acquire(Kind::Video, false, &[]), None, "32 를 다 쓰면 자리가 없다");
        assert_eq!(p.acquire(Kind::Audio, false, &[]), None);
        p.release(7);
        assert_eq!(p.acquire(Kind::Audio, false, &[]), None, "쉬는 영상 자리는 오디오에 주지 않는다");
        assert_eq!(p.acquire(Kind::Video, false, &[]), Some(7));
    }

    #[test]
    fn moving_servers_returns_the_old_slot_and_takes_one_in_the_new_book() {
        let mut pools: HashMap<String, Pool> = HashMap::new();
        let a0 = reseat(&mut pools, "", None, "A", Kind::Audio, false, &[]);
        let v = reseat(&mut pools, "", None, "A", Kind::Video, false, &[]);
        assert_eq!((a0, v), (Some(0), Some(1)));
        let mut mid = v;
        for _ in 0..33 {
            mid = reseat(&mut pools, "A", mid, "B", Kind::Video, false, &[]);
            assert_eq!(mid, Some(0), "B 장부에서 가장 작은 빈 값");
            mid = reseat(&mut pools, "B", mid, "A", Kind::Video, false, &[]);
            assert_eq!(mid, Some(1), "A 로 돌아오면 떠날 때 돌려준 같은 구조의 자리");
        }
        assert_eq!(pools.get("A").map(Pool::owned), Some(vec![0, 1]));
        assert_eq!(pools.get("B").map(Pool::owned), Some(vec![]), "떠난 서버 쪽 자리는 돌려받았다");
    }

    #[test]
    fn a_full_book_leaves_the_track_without_a_slot_and_the_old_one_returned() {
        let mut pools: HashMap<String, Pool> = HashMap::new();
        for _ in 0..32 {
            reseat(&mut pools, "", None, "B", Kind::Audio, false, &[]);
        }
        let v = reseat(&mut pools, "", None, "A", Kind::Video, false, &[]);
        assert_eq!(reseat(&mut pools, "A", v, "B", Kind::Video, false, &[]), None, "옮겨 간 서버에 자리가 없다");
        assert_eq!(pools.get("A").map(Pool::owned), Some(vec![]), "옛 자리는 돌려줬다");
    }

    #[test]
    fn pick_is_pure_over_the_book() {
        let book = [Slot { kind: Kind::Audio, simulcast: false, mid: 0, owned: true }, Slot { kind: Kind::Video, simulcast: false, mid: 1, owned: false }];
        assert_eq!(pick(&book, Kind::Video, false, &[]), Pick::Reuse(1));
        assert_eq!(pick(&book, Kind::Video, true, &[]), Pick::Fresh(2));
        assert_eq!(pick(&book, Kind::Audio, false, &[2]), Pick::Fresh(3));
        assert_eq!(pick(&book, Kind::Audio, false, &(2..32).collect::<Vec<u8>>()), Pick::Full);
    }
}
