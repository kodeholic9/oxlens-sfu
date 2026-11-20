// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-RTCP-NACK-DOWN · 정:RULE-SIM-TICK · model: claude-opus-5-5
//! 하향 NACK 장부 — 다운링크 줄마다 최근 seq 의 재전송 횟수 · 마지막으로 청한 시각. 재전송 여부(seq 마다 상한)와 반복 NACK 셈을 한 장부가 정한다. 시각은 인자.

pub const MAX_PER_SEQ: u8 = 3;
pub const REPEAT_GAP_MS: u64 = 100;
pub const SLOTS: usize = 512;
pub const FRESH_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    First,
    Repeat,
    Soon,
    Limit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub send: bool,
    pub repeated: bool,
    pub why: Why,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cell {
    seq: u16,
    count: u8,
    at: u64,
}

#[derive(Debug, Clone)]
pub struct Ledger {
    cells: Vec<Option<Cell>>,
    pub repeats: u64,
}

impl Default for Ledger {
    fn default() -> Self {
        Ledger { cells: vec![None; SLOTS], repeats: 0 }
    }
}

impl Ledger {
    pub fn step(&mut self, seq: u16, now: u64) -> Step {
        let i = seq as usize % SLOTS;
        let fresh = self.cells[i].filter(|c| c.seq == seq && now.saturating_sub(c.at) <= FRESH_MS);
        let Some(c) = fresh else {
            self.cells[i] = Some(Cell { seq, count: 1, at: now });
            return Step { send: true, repeated: false, why: Why::First };
        };
        if now.saturating_sub(c.at) < REPEAT_GAP_MS {
            return Step { send: false, repeated: false, why: Why::Soon };
        }
        if c.count >= MAX_PER_SEQ {
            return Step { send: false, repeated: false, why: Why::Limit };
        }
        self.cells[i] = Some(Cell { seq, count: c.count + 1, at: now });
        self.repeats += 1;
        Step { send: true, repeated: true, why: Why::Repeat }
    }
}
