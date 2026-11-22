// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SIM-SWITCH · 정:IMPL-PUB-RID-SPATIAL · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 시간 층 고르기 — VP8 L1T3 표지(TID · Y · 15비트 picture ID) 읽기 · 프레임 단위 판단(내리기는 다음 프레임 · 올리기는 Y 프레임 · 키프레임은 목표로) · 줄 상태 묶음. 원천 server/areas/sim/behavior/temporal.ts.

use crate::fwd::core::after15;

pub const TL_TOP: u8 = 2;
pub const TL_NONE: u8 = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vp8Tl {
    pub tid: u8,
    pub y: bool,
    pub pid: u16,
}

pub fn vp8_tl(p: &[u8]) -> Option<Vp8Tl> {
    let b0 = *p.first()?;
    if b0 & 0x80 == 0 {
        return None;
    }
    let x = *p.get(1)?;
    if x & 0x80 == 0 || x & 0x40 == 0 || x & 0x20 == 0 {
        return None;
    }
    let hi = *p.get(2)?;
    if hi & 0x80 == 0 {
        return None;
    }
    let pid = (((hi & 0x7f) as u16) << 8) | *p.get(3)? as u16;
    let t = *p.get(5)?;
    Some(Vp8Tl { tid: t >> 6, y: t & 0x20 != 0, pid })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlWhy {
    Key,
    Down,
    UpSync,
}

impl TlWhy {
    pub fn name(self) -> &'static str {
        match self {
            TlWhy::Key => "key",
            TlWhy::Down => "down",
            TlWhy::UpSync => "up_sync",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporalSel {
    pub current: u8,
    pub pid: Option<u16>,
    pub keep: bool,
}

impl Default for TemporalSel {
    fn default() -> TemporalSel {
        TemporalSel { current: TL_TOP, pid: None, keep: true }
    }
}

impl TemporalSel {
    pub fn step(&mut self, tl: Vp8Tl, key: bool, target: u8) -> (bool, Option<TlWhy>) {
        let goal = target.min(TL_TOP);
        if self.pid == Some(tl.pid) {
            return (self.keep, None);
        }
        if let Some(p) = self.pid
            && after15(p, tl.pid)
        {
            return (tl.tid <= self.current, None);
        }
        let mut why = None;
        if key {
            if self.current != goal {
                why = Some(TlWhy::Key);
            }
            self.current = goal;
        } else if goal < self.current {
            self.current = goal;
            why = Some(TlWhy::Down);
        } else if tl.tid > self.current && tl.tid <= goal && tl.y {
            self.current = tl.tid;
            why = Some(TlWhy::UpSync);
        }
        self.pid = Some(tl.pid);
        self.keep = key || tl.tid <= self.current;
        (self.keep, why)
    }

    pub fn pack(self) -> u64 {
        let pid = self.pid.map(|p| (1u64 << 15) | p as u64).unwrap_or(0);
        (self.current as u64) | ((self.keep as u64) << 8) | (pid << 16)
    }

    pub fn unpack(v: u64) -> TemporalSel {
        let p = (v >> 16) & 0xffff;
        TemporalSel { current: (v & 0xff) as u8, keep: (v >> 8) & 1 == 1, pid: (p >> 15 == 1).then_some((p & 0x7fff) as u16) }
    }
}

pub fn goal(manual: u8, auto: u8) -> u8 {
    let m = if manual == TL_NONE { TL_TOP } else { manual };
    m.min(auto).min(TL_TOP)
}
