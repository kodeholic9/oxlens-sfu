// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-SIM-MODES · 정:IMPL-SIM-SWITCH · 정:RULE-SIM-TICK · 정:IMPL-PUB-RID-SPATIAL · model: claude-opus-5-5
//! SIM — 다운링크 스트림마다 레이어 선택(현재 · 목표 · pending 만료 · 수동 상한과 `min` 결합) · 구독 전송로마다 자동 판단 상태(v1 REMB+RR · v2 TWCC GCC).

pub mod core;

use std::collections::VecDeque;

use self::core::{Gcc, Layer, Policy, PENDING_MS};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sel {
    pub current: Option<u8>,
    pub target: Option<u8>,
    pub pending_at: u64,
    pub cap: Option<u8>,
    pub paused: bool,
}

impl Sel {
    pub fn aim(&mut self, auto: Option<Layer>, now: u64) -> bool {
        let want = match auto.unwrap_or(Layer::High) {
            Layer::Pause => None,
            l => {
                let idx = l.rid_index().unwrap_or(1);
                Some(self.cap.map(|c| c.min(idx)).unwrap_or(idx))
            }
        };
        let want = if self.paused { None } else { want };
        if want == self.target {
            return false;
        }
        self.target = want;
        self.pending_at = now;
        want.is_some() && want != self.current
    }

    pub fn expire(&mut self, now: u64) -> bool {
        if self.target != self.current && self.target.is_some() && self.current.is_some() && now.saturating_sub(self.pending_at) > PENDING_MS {
            self.target = self.current;
            return true;
        }
        false
    }

    pub fn accept(&mut self, layer: u8, key: bool) -> bool {
        if self.paused {
            return false;
        }
        if self.target == Some(layer) && self.current != Some(layer) && key {
            self.current = Some(layer);
            return true;
        }
        self.current == Some(layer) && self.target.is_some()
    }
}

#[derive(Debug, Default)]
pub struct Bwe {
    pub policy: Policy,
    pub gcc: Gcc,
    pub sent: VecDeque<(u16, u64, usize)>,
    pub acked: VecDeque<(u64, usize)>,
    pub remb: Option<(f64, u64)>,
    pub loss: Option<(f64, u64)>,
    pub nacks: u32,
    pub nack_at: u64,
    pub nack_rate: f64,
    pub drops: u64,
    pub est_at: u64,
}

impl Bwe {
    pub fn sent_packet(&mut self, seq: u16, at_us: u64, size: usize) {
        self.sent.push_back((seq, at_us, size));
        if self.sent.len() > 4096 {
            self.sent.pop_front();
        }
    }

    pub fn feedback(&mut self, acks: &[(u16, i64)], now: u64) {
        for (seq, arr_us) in acks {
            let Some(&(_, send_us, size)) = self.sent.iter().rev().find(|(s, _, _)| s == seq) else { continue };
            self.gcc.on_packet(send_us as f64 / 1000.0, *arr_us as f64 / 1000.0);
            self.acked.push_back((now, size));
        }
        while self.acked.front().map(|(t, _)| now.saturating_sub(*t) > 1000).unwrap_or(false) {
            self.acked.pop_front();
        }
        let bits: f64 = self.acked.iter().map(|(_, s)| *s as f64 * 8.0).sum();
        self.gcc.update(now, bits);
        self.est_at = now;
    }

    pub fn nack(&mut self, n: u32, now: u64) {
        if now.saturating_sub(self.nack_at) >= 1000 {
            self.nack_rate = self.nacks as f64 * 1000.0 / now.saturating_sub(self.nack_at).max(1) as f64;
            self.nacks = 0;
            self.nack_at = now;
        }
        self.nacks += n;
    }
}
