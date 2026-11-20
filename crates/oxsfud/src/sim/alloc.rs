// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SIM-TICK · 정:RULE-CONC-SANS-IO · model: claude-fable-5-2
//! 층 배분(mediasoup 3.32.2 BitrateAllocator 를 옮긴 원천의 거울) — 구독 전송로의 목표 속도 한 값을 줄마다 점(안 보냄 · 층)에 나눈다. 지금 점보다 위로 오를 때만 그 점 값의 1.2배가 든다.

pub const HYSTERESIS: f64 = 1.2;
pub const LAYER_NONE: i8 = -1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub bitrate: i64,
    pub layer: i8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: u64,
    pub weight: u8,
    pub points: Vec<Point>,
    pub current: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    Top,
    NoRoom,
    Hysteresis,
}

impl Blocked {
    pub fn name(self) -> &'static str {
        match self {
            Blocked::Top => "top",
            Blocked::NoRoom => "no_room",
            Blocked::Hysteresis => "hysteresis",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant {
    pub id: u64,
    pub point: usize,
    pub layer: i8,
    pub changed: bool,
    pub reason: Blocked,
    pub need: i64,
}

struct Entry<'a> {
    c: &'a Candidate,
    point: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Allocator {
    rounds: u64,
}

impl Allocator {
    pub fn distribute(&mut self, available: i64, candidates: &[Candidate]) -> (Vec<Grant>, i64) {
        let mut available = available;
        let mut entries: Vec<Entry> = Vec::new();
        let mut total = 0i64;
        for c in candidates {
            let Some(first) = c.points.first() else { continue };
            available -= first.bitrate;
            if c.points.len() == 1 {
                continue;
            }
            if c.weight > 0 {
                entries.push(Entry { c, point: 0 });
                total += c.weight as i64;
            }
        }
        available = available.max(0);
        if entries.is_empty() {
            return (Vec::new(), available);
        }
        entries.sort_by(|a, b| b.c.weight.cmp(&a.c.weight).then(a.c.id.cmp(&b.c.id)));
        self.rotate(&mut entries);
        for e in entries.iter_mut() {
            let climb = Self::climb(e);
            if climb <= available {
                e.point += 1;
                available -= climb;
            }
        }
        while available > 0 {
            let before = available;
            for e in entries.iter_mut() {
                available -= Self::climb_with(e, before * e.c.weight as i64 / total);
            }
            if available == before {
                for e in entries.iter_mut() {
                    available -= Self::climb_with(e, available);
                }
                if available == before {
                    break;
                }
            }
        }
        self.rounds += 1;
        let grants = entries
            .iter()
            .map(|e| {
                let top = e.point + 1 >= e.c.points.len();
                let need = if top { 0 } else { Self::climb(e) };
                let plain = if top { 0 } else { e.c.points[e.point + 1].bitrate - e.c.points[e.point].bitrate };
                let reason = if top {
                    Blocked::Top
                } else if plain <= available {
                    Blocked::Hysteresis
                } else {
                    Blocked::NoRoom
                };
                Grant { id: e.c.id, point: e.point, layer: e.c.points[e.point].layer, changed: e.c.current != Some(e.point), reason, need }
            })
            .collect();
        (grants, available)
    }

    fn climb_with(e: &mut Entry, bitrate: i64) -> i64 {
        let mut taken = 0;
        while e.point + 1 < e.c.points.len() {
            let climb = Self::climb(e);
            if climb > bitrate - taken {
                break;
            }
            taken += climb;
            e.point += 1;
        }
        taken
    }

    fn climb(e: &Entry) -> i64 {
        let held = e.c.points[e.point].bitrate;
        let next = e.c.points[e.point + 1].bitrate;
        let back = e.c.current.map(|c| e.point + 1 > c).unwrap_or(false);
        (if back { (next as f64 * HYSTERESIS).round() as i64 } else { next }) - held
    }

    fn rotate(&self, entries: &mut [Entry]) {
        let mut i = 0;
        while i < entries.len() {
            let mut j = i;
            while j < entries.len() && entries[j].c.weight == entries[i].c.weight {
                j += 1;
            }
            let k = (self.rounds % (j - i) as u64) as usize;
            entries[i..j].rotate_left(k);
            i = j;
        }
    }
}
