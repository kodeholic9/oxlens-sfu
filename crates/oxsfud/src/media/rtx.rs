// author: kodeholic (powered by Claude)
//! 정§11-1 하향 — 구독자 NACK 에 답하는 재전송. 상향과 ★다른 기계다.
//!
//! 관문 넷을 이 순서로 지난다: ①RTX gate ②캐시 조회 ③예산 ④송신.
//! ★관문마다 사유가 남는다 — 조용한 drop 은 원인을 못 찾게 한다.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// 링버퍼 길이 — 30fps 기준 ~34초.
pub const CACHE_LEN: usize = 1024;
/// 정§11-1 — 한 구독자가 3초에 쓸 수 있는 재전송 수. 정상 10~30 이 통과하고 폭풍은 막힌다.
pub const BUDGET: u32 = 200;
pub const BUDGET_WINDOW_MS: u64 = 3_000;

/// 왜 안 보냈나. 계수의 이름이자 로그의 이름이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// ★전환 직후의 stale NACK — 가상 SSRC 공간이 이미 바뀌었다.
    Gate,
    /// 링버퍼를 지났다.
    Miss,
    /// 3초 예산 소진 — 그 구독자만 막고 남은 참가자를 보호한다.
    Budget,
}

#[derive(Debug, Clone)]
struct Cached {
    seq: u16,
    packet: Vec<u8>,
}

/// 한 구독자 스트림이 내보낸 egress 패킷의 링버퍼 + 예산.
#[derive(Debug)]
pub struct RtxCache {
    ring: Mutex<VecDeque<Cached>>,
    /// ★egress seq 공간이 바뀐 시각. 이보다 앞의 요구는 stale 이다.
    epoch_ms: AtomicU64,
    spent: AtomicU64,
    window_at_ms: AtomicU64,
}

impl Default for RtxCache {
    fn default() -> Self {
        Self {
            ring: Mutex::new(VecDeque::with_capacity(CACHE_LEN)),
            epoch_ms: AtomicU64::new(0),
            spent: AtomicU64::new(0),
            window_at_ms: AtomicU64::new(0),
        }
    }
}

impl RtxCache {
    /// egress 로 나간 패킷을 담는다. ★핫패스라 판정하지 않는다.
    pub fn keep(&self, seq: u16, packet: &[u8]) {
        let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        if ring.len() == CACHE_LEN {
            ring.pop_front();
        }
        ring.push_back(Cached { seq, packet: packet.to_vec() });
    }

    /// 정§11-1 ① — 화자·레이어 전환으로 seq 공간이 갈렸다. 이전 요구는 전부 stale 이다.
    pub fn reset(&self, now_ms: u64) {
        self.ring.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.epoch_ms.store(now_ms, Ordering::Relaxed);
    }

    /// 관문 넷을 순서대로. 통과하면 보낼 패킷, 아니면 사유다.
    pub fn take(&self, seq: u16, asked_at_ms: u64, now_ms: u64) -> Result<Vec<u8>, Refusal> {
        // ① 전환 직후의 요구는 이미 없는 공간을 가리킨다 — 보내면 수신측이 깨진다.
        if asked_at_ms < self.epoch_ms.load(Ordering::Relaxed) {
            return Err(Refusal::Gate);
        }
        // ② 캐시에 있나.
        let found = {
            let ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
            ring.iter().find(|c| c.seq == seq).map(|c| c.packet.clone())
        };
        let Some(packet) = found else { return Err(Refusal::Miss) };
        // ③ 예산 — 그 구독자만 막는다.
        if !self.spend(now_ms) {
            return Err(Refusal::Budget);
        }
        Ok(packet)
    }

    fn spend(&self, now_ms: u64) -> bool {
        let opened = self.window_at_ms.load(Ordering::Relaxed);
        if now_ms.saturating_sub(opened) >= BUDGET_WINDOW_MS {
            self.window_at_ms.store(now_ms, Ordering::Relaxed);
            self.spent.store(1, Ordering::Relaxed);
            return true;
        }
        self.spent.fetch_add(1, Ordering::Relaxed) < u64::from(BUDGET)
    }

    pub fn len(&self) -> usize {
        self.ring.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> RtxCache {
        let c = RtxCache::default();
        for seq in 0..10u16 {
            c.keep(seq, &[seq as u8; 4]);
        }
        c
    }

    #[test]
    fn a_cached_packet_comes_back_as_it_went_out() {
        assert_eq!(cache().take(3, 0, 0), Ok(vec![3u8; 4]));
    }

    #[test]
    fn what_left_the_ring_is_a_miss_not_a_silence() {
        let c = RtxCache::default();
        for seq in 0..(CACHE_LEN as u16 + 5) {
            c.keep(seq, &[1, 2, 3]);
        }
        assert_eq!(c.len(), CACHE_LEN);
        assert_eq!(c.take(0, 0, 0), Err(Refusal::Miss), "★사유가 남아야 원인을 찾는다");
        assert!(c.take(CACHE_LEN as u16, 0, 0).is_ok());
    }

    #[test]
    fn a_stale_ask_from_before_the_switch_is_refused() {
        let c = cache();
        c.reset(100);
        assert_eq!(
            c.take(3, 50, 100),
            Err(Refusal::Gate),
            "★가상 SSRC 공간이 이미 바뀐 패킷을 보내면 수신측이 깨진다"
        );
    }

    #[test]
    fn the_budget_stops_a_storm_and_reopens() {
        let c = cache();
        let mut ok = 0;
        for _ in 0..(BUDGET + 50) {
            if c.take(3, 0, 0).is_ok() {
                ok += 1;
            }
        }
        assert_eq!(ok, BUDGET, "정상 10~30 은 통과하고 폭풍만 막힌다");
        assert_eq!(c.take(3, 0, 0), Err(Refusal::Budget));
        assert!(c.take(3, 0, BUDGET_WINDOW_MS).is_ok(), "창이 지나면 다시 연다");
    }

    #[test]
    fn the_gate_is_checked_before_the_cache() {
        let c = cache();
        c.reset(100);
        // 캐시를 비웠어도 사유는 Gate 다 — 순서가 계약이다.
        assert_eq!(c.take(3, 50, 100), Err(Refusal::Gate));
    }
}
