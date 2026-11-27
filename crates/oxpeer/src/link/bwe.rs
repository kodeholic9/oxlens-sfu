// author: kodeholic (powered by Claude)
//! 송신 쪽 대역 추정(context 202611/20261108d E4) — 발행 연결마다 하나. 패킷마다 transport-cc 번호를 채워 보낸 때 · 크기를 적고,
//! 서버 TWCC 피드백(draft-holmer-rmcat-transport-wide-cc-extensions-01 · 서버 `rtcp/mod.rs` `build_twcc`)으로
//! ①손실 기반(1초 창 — 10% 넘으면 줄이고 2% 밑이면 8% 올린다 · libwebrtc 손실 제어기 꼴) ②지연 기울기(보낸 간격 대비 도착 간격의 추세 — 과사용이면 받힌 비트레이트의 85%)
//! ③서버 REMB 상한 셋의 최솟값을 ★목표 비트레이트로 낸다. ★SDK 는 인코딩하지 않는다 — 맞추는 것은 앱 몫이고 SDK 는 버리거나 늦추지 않는다(SDK:RULE-PLAT-SERVER).

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

pub(crate) const START_BPS: u64 = 300_000;
pub(crate) const MIN_BPS: u64 = 30_000;
pub(crate) const MAX_BPS: u64 = 10_000_000;
/// 보낸 기록을 쥐는 시간(피드백이 이보다 늦으면 그 번호는 버린다)
const KEEP: Duration = Duration::from_secs(2);
/// 손실을 세는 창
const LOSS_WINDOW: Duration = Duration::from_secs(1);
/// 지연 기울기 — 한 무리로 볼 보낸 간격 · 추세를 볼 무리 수 · 문턱(ms)
const GROUP_US: i64 = 5_000;
const TREND_N: usize = 20;
const OVERUSE_MS: f64 = 12.5;

/// 받은 TWCC 하나 — (전송 번호, 받은 시각 µs · None = 못 받음)
pub(crate) fn parse_twcc(p: &[u8]) -> Option<Vec<(u16, Option<i64>)>> {
    if p.len() < 20 || p[1] != 205 || p[0] & 0x1f != 15 {
        return None;
    }
    let base = u16::from_be_bytes([p[12], p[13]]);
    let count = usize::from(u16::from_be_bytes([p[14], p[15]]));
    let reft = {
        let v = (u32::from(p[16]) << 16) | (u32::from(p[17]) << 8) | u32::from(p[18]);
        let v = if v & 0x80_0000 != 0 { v as i64 - (1 << 24) } else { v as i64 };
        v * 64_000
    };
    let mut syms: Vec<u8> = Vec::with_capacity(count);
    let mut i = 20;
    while syms.len() < count {
        let c = u16::from_be_bytes([*p.get(i)?, *p.get(i + 1)?]);
        i += 2;
        if c & 0x8000 == 0 {
            let s = ((c >> 13) & 0x3) as u8;
            let n = usize::from(c & 0x1fff);
            syms.extend(std::iter::repeat_n(s, n.min(count - syms.len())));
        } else if c & 0x4000 == 0 {
            for k in (0..14).rev() {
                if syms.len() < count {
                    syms.push(((c >> k) & 1) as u8);
                }
            }
        } else {
            for k in (0..7).rev() {
                if syms.len() < count {
                    syms.push(((c >> (k * 2)) & 3) as u8);
                }
            }
        }
    }
    let mut at = reft;
    let mut out = Vec::with_capacity(count);
    for (k, s) in syms.into_iter().enumerate() {
        let seq = base.wrapping_add(k as u16);
        match s {
            1 => {
                at += i64::from(*p.get(i)?) * 250;
                i += 1;
                out.push((seq, Some(at)));
            }
            2 => {
                at += i64::from(i16::from_be_bytes([*p.get(i)?, *p.get(i + 1)?])) * 250;
                i += 2;
                out.push((seq, Some(at)));
            }
            _ => out.push((seq, None)),
        }
    }
    Some(out)
}

/// 발행 연결 하나의 추정기
pub(crate) struct Bwe {
    /// transport-cc 확장 번호(0 = 끔 — 번호를 채우지 않는다)
    pub ext: u8,
    next: u16,
    sent: HashMap<u16, (Instant, usize)>,
    order: VecDeque<(u16, Instant)>,
    target: u64,
    remb: Option<u64>,
    reported: u64,
    /// 손실 창 — (받음, 잃음) · 창 시작
    win: (u64, u64),
    win_at: Option<Instant>,
    pub loss: f64,
    /// 지연 — 앞 무리(보낸 µs · 받은 µs) · 쌓은 지연 · 매끈한 값 · (받은 ms, 매끈한 값) 이력
    group: Option<(i64, i64)>,
    acc: f64,
    smooth: f64,
    hist: VecDeque<(f64, f64)>,
    pub overuse: bool,
    /// 받힌 바이트 — (받은 µs, 크기)
    acked: VecDeque<(i64, usize)>,
    t0: Instant,
}

impl Default for Bwe {
    fn default() -> Self {
        Bwe { ext: 0, next: 1, sent: HashMap::new(), order: VecDeque::new(), target: START_BPS, remb: None, reported: START_BPS, win: (0, 0), win_at: None, loss: 0.0, group: None, acc: 0.0, smooth: 0.0, hist: VecDeque::new(), overuse: false, acked: VecDeque::new(), t0: Instant::now() }
    }
}

impl Bwe {
    pub(crate) fn target(&self) -> u64 {
        self.remb.map_or(self.target, |r| self.target.min(r.max(MIN_BPS)))
    }

    /// 평문 RTP 의 transport-cc 자리에 번호를 채우고 보낸 것을 적는다 — 머리 맨 앞 확장이 우리 번호 · 길이 2 일 때만(`SendTrack::header` 가 그렇게 짓는다)
    pub(crate) fn stamp(&mut self, pkt: &mut [u8], now: Instant) {
        if self.ext == 0 || pkt.len() < 20 || pkt[0] & 0x10 == 0 || pkt[0] & 0x0f != 0 || pkt[12..14] != [0xBE, 0xDE] || pkt[16] != (self.ext << 4) | 1 {
            return;
        }
        let seq = self.next;
        self.next = self.next.wrapping_add(1);
        pkt[17..19].copy_from_slice(&seq.to_be_bytes());
        self.sent.insert(seq, (now, pkt.len()));
        self.order.push_back((seq, now));
        while self.order.front().is_some_and(|(_, at)| now.duration_since(*at) > KEEP) {
            if let Some((s, _)) = self.order.pop_front() {
                self.sent.remove(&s);
            }
        }
    }

    pub(crate) fn on_remb(&mut self, bps: u64) -> Option<u64> {
        self.remb = Some(bps);
        self.report()
    }

    /// TWCC 피드백 하나 — 바뀐 목표가 앞서 알린 값과 5% 넘게 다르면 돌려준다
    pub(crate) fn on_feedback(&mut self, fb: &[(u16, Option<i64>)], now: Instant) -> Option<u64> {
        for (seq, rx) in fb {
            let Some(&(sent_at, size)) = self.sent.get(seq) else { continue };
            match rx {
                None => self.win.1 += 1,
                Some(r) => {
                    self.win.0 += 1;
                    self.acked.push_back((*r, size));
                    self.delay(sent_at.duration_since(self.t0).as_micros() as i64, *r);
                }
            }
        }
        if let Some(&(last, _)) = self.acked.back() {
            while self.acked.front().is_some_and(|(r, _)| last - r > 1_000_000) {
                self.acked.pop_front();
            }
        }
        let acked_bps = {
            let span = match (self.acked.front(), self.acked.back()) {
                (Some(a), Some(b)) if b.0 > a.0 => (b.0 - a.0) as f64 / 1e6,
                _ => 0.0,
            };
            if span > 0.2 { (self.acked.iter().map(|(_, s)| *s).sum::<usize>() as f64 * 8.0 / span) as u64 } else { 0 }
        };
        if self.overuse && acked_bps > 0 {
            self.target = self.target.min((acked_bps as f64 * 0.85) as u64);
        }
        let start = *self.win_at.get_or_insert(now);
        if now.duration_since(start) >= LOSS_WINDOW {
            let (got, lost) = std::mem::take(&mut self.win);
            self.win_at = Some(now);
            let total = got + lost;
            if total > 0 {
                self.loss = lost as f64 / total as f64;
                if self.loss > 0.10 {
                    self.target = (self.target as f64 * (1.0 - 0.5 * self.loss)) as u64;
                } else if self.loss < 0.02 && !self.overuse {
                    self.target = (self.target as f64 * 1.08) as u64;
                }
            }
        }
        self.target = self.target.clamp(MIN_BPS, MAX_BPS);
        self.report()
    }

    fn report(&mut self) -> Option<u64> {
        let t = self.target();
        let d = (t as f64 - self.reported as f64).abs() / self.reported.max(1) as f64;
        if d > 0.05 {
            self.reported = t;
            Some(t)
        } else {
            None
        }
    }

    /// 지연 기울기(trendline) — 보낸 간격 5ms 안을 한 무리로 보고 무리마다 (도착 간격 − 보낸 간격)을 쌓아 추세를 본다
    fn delay(&mut self, sent_us: i64, recv_us: i64) {
        let Some((gs, gr)) = self.group else {
            self.group = Some((sent_us, recv_us));
            return;
        };
        if sent_us - gs < GROUP_US {
            self.group = Some((gs, gr.max(recv_us)));
            return;
        }
        let d = ((recv_us - gr) - (sent_us - gs)) as f64 / 1_000.0;
        self.group = Some((sent_us, recv_us));
        self.acc += d;
        self.smooth = 0.9 * self.smooth + 0.1 * self.acc;
        self.hist.push_back((recv_us as f64 / 1_000.0, self.smooth));
        if self.hist.len() > TREND_N {
            self.hist.pop_front();
        }
        if self.hist.len() < TREND_N {
            return;
        }
        let n = self.hist.len() as f64;
        let (mx, my) = (self.hist.iter().map(|h| h.0).sum::<f64>() / n, self.hist.iter().map(|h| h.1).sum::<f64>() / n);
        let (num, den) = self.hist.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + (x - mx) * (y - my), b + (x - mx) * (x - mx)));
        let slope = if den > 0.0 { num / den } else { 0.0 };
        self.overuse = slope * n * 4.0 > OVERUSE_MS;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 서버 `build_twcc` 와 같은 꼴 — 두 비트 상태 벡터만(7 개씩) · 받은 것은 작은 차(1 바이트)
    fn twcc(base: u16, rx: &[Option<i64>]) -> Vec<u8> {
        let mut b = vec![0x8f, 205, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2];
        b.extend(base.to_be_bytes());
        b.extend((rx.len() as u16).to_be_bytes());
        b.extend([0, 0, 0, 0]);
        for c in rx.chunks(7) {
            let mut w: u16 = 0xC000;
            for (k, r) in c.iter().enumerate() {
                w |= u16::from(if r.is_some() { 1u8 } else { 0 }) << (12 - k * 2);
            }
            b.extend(w.to_be_bytes());
        }
        let mut last = 0;
        for r in rx.iter().flatten() {
            b.push(((r - last) / 250) as u8);
            last = *r;
        }
        while !b.len().is_multiple_of(4) {
            b.push(0);
        }
        let l = (b.len() / 4 - 1) as u16;
        b[2..4].copy_from_slice(&l.to_be_bytes());
        b
    }

    #[test]
    fn parse_vector_and_run() {
        let p = twcc(100, &[Some(1_000), None, Some(2_000)]);
        assert_eq!(parse_twcc(&p).unwrap(), vec![(100, Some(1_000)), (101, None), (102, Some(2_000))]);
        // 달리기 꼴 — 받음 300 개 같은 상태(작은 차 0)
        let mut b = vec![0x8f, 205, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 5, 1, 44, 0, 0, 1, 0];
        b.extend((0x2000u16 | 300).to_be_bytes());
        b.extend(std::iter::repeat_n(0u8, 300));
        let r = parse_twcc(&b).unwrap();
        assert_eq!(r.len(), 300);
        assert_eq!(r[0], (5, Some(64_000)));
        assert_eq!(r[299], (5u16.wrapping_add(299), Some(64_000)));
    }

    fn pkt(ext: u8) -> Vec<u8> {
        let mut p = vec![0x90, 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 9, 0xBE, 0xDE, 0, 1, (ext << 4) | 1, 0, 0, 0];
        p.extend([0u8; 1000]);
        p
    }

    /// 손실 20% 가 이어지면 목표가 내려가고 · 손실 0 이 이어지면 올라간다 — 앞서 알린 값과 5% 넘게 다를 때만 알린다
    #[test]
    fn loss_lowers_and_clean_raises() {
        for (loss_every, down) in [(5u16, true), (0, false)] {
            let mut b = Bwe { ext: 6, ..Bwe::default() };
            let t0 = Instant::now();
            let mut reports = Vec::new();
            for sec in 0..6u64 {
                let mut fb = Vec::new();
                for k in 0..100u64 {
                    let now = t0 + Duration::from_millis(sec * 1_000 + k * 10);
                    let mut p = pkt(6);
                    b.stamp(&mut p, now);
                    let seq = u16::from_be_bytes([p[17], p[18]]);
                    let rx = if loss_every > 0 && seq.is_multiple_of(loss_every) { None } else { Some(now.duration_since(b.t0).as_micros() as i64 + 20_000) };
                    fb.push((seq, rx));
                }
                if let Some(t) = b.on_feedback(&fb, t0 + Duration::from_millis(sec * 1_000 + 1_000)) {
                    reports.push(t);
                }
            }
            if down {
                // 1초 창마다 (1 − 0.5 × 0.2) = 0.9 배 — 다섯 창이면 0.59 배
                assert!(b.target() < START_BPS * 6 / 10, "손실 20% 6초 — {} {reports:?}", b.target());
                assert!((b.loss - 0.2).abs() < 0.02);
            } else {
                assert!(b.target() > START_BPS, "손실 0 — {} {reports:?}", b.target());
            }
            assert!(!reports.is_empty());
        }
    }

    /// 도착 간격이 보낸 간격보다 계속 길어지면(줄이 찬다) 과사용 — 목표가 받힌 비트레이트의 85% 아래로(목표 2Mbps 에서 시작 · 받힌 것은 약 680kbps)
    #[test]
    fn growing_delay_is_overuse() {
        let mut b = Bwe { ext: 6, target: 2_000_000, reported: 2_000_000, ..Bwe::default() };
        let t0 = b.t0;
        for k in 0..200u64 {
            let now = t0 + Duration::from_millis(k * 10);
            let mut p = pkt(6);
            b.stamp(&mut p, now);
            let seq = u16::from_be_bytes([p[17], p[18]]);
            // 보낸 간격 10ms · 도착 간격 12ms
            b.on_feedback(&[(seq, Some((k * 12_000) as i64))], now);
        }
        assert!(b.overuse);
        assert!(b.target() < 600_000, "{}", b.target());
    }

    #[test]
    fn remb_caps_target() {
        let mut b = Bwe::default();
        assert_eq!(b.on_remb(100_000), Some(100_000));
        assert_eq!(b.target(), 100_000);
    }

    #[test]
    fn stamp_only_our_slot() {
        let mut b = Bwe { ext: 6, ..Bwe::default() };
        let mut p = pkt(6);
        b.stamp(&mut p, Instant::now());
        assert_eq!(&p[17..19], &[0, 1]);
        let mut q = pkt(5);
        b.stamp(&mut q, Instant::now());
        assert_eq!(&q[17..19], &[0, 0]);
    }
}
