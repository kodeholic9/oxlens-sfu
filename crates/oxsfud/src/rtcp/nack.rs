// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-RTCP-NACK-DOWN · 정:RULE-SIM-TICK · model: claude-opus-5-5
//! 하향 NACK 장부 — 다운링크 줄마다 최근 seq 의 재전송 횟수 · 마지막으로 청한 시각 · 키프레임 기다림. 재전송 여부(키프레임 기다림 · 나이 상한 · seq 마다 상한)와 반복 NACK 셈을 한 장부가 정한다. 시각은 인자.

pub const MAX_PER_SEQ: u8 = 3;
pub const REPEAT_GAP_MS: u64 = 100;
pub const SLOTS: usize = 1024;
pub const FRESH_MS: u64 = 2_000;
pub const MAX_AGE_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    First,
    Repeat,
    Soon,
    Limit,
    Old,
    WaitKey,
}

impl Why {
    pub fn name(self) -> &'static str {
        match self {
            Why::First => "first",
            Why::Repeat => "repeat",
            Why::Soon => "soon",
            Why::Limit => "limit",
            Why::Old => "old",
            Why::WaitKey => "waitKey",
        }
    }
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
    pub wait_key: bool,
}

impl Default for Ledger {
    fn default() -> Self {
        Ledger { cells: vec![None; SLOTS], repeats: 0, wait_key: false }
    }
}

impl Ledger {
    pub fn on_key_request(&mut self) {
        self.wait_key = true;
    }

    pub fn on_key_sent(&mut self) {
        self.wait_key = false;
    }

    pub fn step(&mut self, seq: u16, now: u64, age_ms: u64) -> Step {
        if self.wait_key {
            return Step { send: false, repeated: false, why: Why::WaitKey };
        }
        if age_ms > MAX_AGE_MS {
            return Step { send: false, repeated: false, why: Why::Old };
        }
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
