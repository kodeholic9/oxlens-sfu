// author: kodeholic (powered by Claude)
//! 정§11-2 — Ingress TWCC feedback. 서버가 ★자체 생성한다.
//!
//! 발행자가 `transport-cc` 를 협상했으면 이것이 없을 때 ★송신 추정이 갱신되지 않는다 —
//! 화질이 안 올라가고 원인은 어느 계수에도 안 남는다.
//!
//! ★반대 방향도 여기 있다 — 정§10-3 v2 는 서버가 **구독 전송로에 번호를 찍고**(`Departures`)
//! 구독자가 돌려준 피드백을 읽어(`parse`) 그 전송로가 실제로 받아낸 속도를 잰다.
//!
//! draft-holmer-rmcat-transport-wide-cc-extensions-01 §3.1.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU16, AtomicU64, Ordering};

use super::rtcp::{FMT_TWCC, PT_RTPFB};

/// 참조 시각의 눈금(64ms) — 초안이 정한 값이다.
pub const REF_TICK_MS: u64 = 64;
/// 도착 간격의 눈금(0.25ms).
pub const DELTA_TICK_US: u64 = 250;
/// 한 번에 보고할 수 있는 패킷 수 상한 — 넘으면 다음 판으로 민다.
pub const MAX_REPORTED: usize = 256;

/// 한 전송로(5-tuple)의 도착 장부. ★SSRC 가 아니라 전송로 단위다 — 그래서 transport-wide 다.
#[derive(Debug, Default)]
pub struct Arrivals {
    seen: Mutex<BTreeMap<u16, u64>>,
    fb_count: AtomicU8,
}

impl Arrivals {
    /// 핫패스 — 판정하지 않고 담기만 한다.
    pub fn observe(&self, seq: u16, at_ms: u64) {
        let mut g = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() >= MAX_REPORTED * 2 {
            return;
        }
        g.insert(seq, at_ms);
    }

    /// 담긴 것을 비우고 피드백 한 장을 짓는다. 담긴 것이 없으면 아무것도 안 짓는다.
    pub fn drain(&self, sender: u32, media: u32) -> Option<Vec<u8>> {
        let taken: Vec<(u16, u64)> = {
            let mut g = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            if g.is_empty() {
                return None;
            }
            let all: Vec<(u16, u64)> = g.iter().map(|(s, t)| (*s, *t)).collect();
            g.clear();
            all
        };
        let count = self.fb_count.fetch_add(1, Ordering::Relaxed);
        Some(build(sender, media, &taken, count))
    }

    pub fn pending(&self) -> usize {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}


/// 출발 장부 한 줄 — (twcc seq, 보낸 시각 ms, 바이트, 도착 시각 ms — 구독자 시계).
type Departure = (u16, u64, u32, Option<u64>);

/// 출발 장부에 담아 두는 창 — 이보다 오래된 것은 버린다. 수신률은 1초 창으로 재므로 여유는 이만큼이면 된다.
pub const DEPART_KEEP_MS: u64 = 6_000;
/// 수신률을 재는 창(정§10-3 신선도와 같은 결 — 이 창 안에 도착이 확인된 것만 센다).
pub const RATE_WINDOW_MS: u64 = 1_000;

/// 정§10-3 v2 — 한 구독 전송로가 내보낸 것의 출발 장부.
///
/// ★번호는 **전송로가 발급한다.** 발행자 번호를 그대로 통과시키면 여러 발행자의 seq 가 한
/// 구독 전송로에 섞여 구독자 피드백이 뜻을 잃는다(정§10-3 증상).
#[derive(Debug, Default)]
pub struct Departures {
    next: AtomicU16,
    /// ★판정이 닿은 지점 — 마지막 피드백이 확인해 준 것 중 **가장 늦게 보낸** 것의 송신 시각.
    /// 이보다 뒤에 보낸 것은 ★아직 물어보지도 않은 것이라 손실로 세면 안 된다.
    judged_until_ms: AtomicU64,
    /// 송신 시각 오름차순 — 앞에서 버리고 뒤에 담는다.
    log: Mutex<VecDeque<Departure>>,
}

impl Departures {
    /// 핫패스 — 번호 하나를 발급하고 담는다. 판정하지 않는다.
    pub fn stamp(&self, now_ms: u64, bytes: usize) -> u16 {
        let seq = self.next.fetch_add(1, Ordering::Relaxed);
        let mut g = self.log.lock().unwrap_or_else(|e| e.into_inner());
        while g.front().is_some_and(|&(_, at, _, _)| now_ms.saturating_sub(at) > DEPART_KEEP_MS) {
            g.pop_front();
        }
        g.push_back((seq, now_ms, bytes as u32, None));
        seq
    }

    /// 피드백 한 장이 알려 준 도착을 ★**장부에 표시**한다.
    ///
    /// ★한 장은 100ms 남짓만 담는다 — 그것 하나로 1초 창을 재면 값이 10분의 1 로 나온다.
    /// 확인은 쌓아 두고, 속도는 창을 통째로 훑어 낸다.
    pub fn confirm(&self, received: &[(u16, u64)]) {
        if received.is_empty() {
            return;
        }
        let seen: std::collections::BTreeMap<u16, u64> = received.iter().copied().collect();
        let mut judged = 0u64;
        let mut g = self.log.lock().unwrap_or_else(|e| e.into_inner());
        for entry in g.iter_mut() {
            if let Some(&at) = seen.get(&entry.0) {
                entry.3 = Some(at);
                judged = judged.max(entry.1);
            }
        }
        drop(g);
        self.judged_until_ms.fetch_max(judged, Ordering::Relaxed);
    }

    /// 창 안에 **보낸** 바이트와 그 중 **도착이 확인된** 바이트.
    pub fn window(&self, now_ms: u64) -> (u64, u64) {
        let to = self.judged_until_ms.load(Ordering::Relaxed);
        let from = to.saturating_sub(RATE_WINDOW_MS);
        let _ = now_ms;
        let g = self.log.lock().unwrap_or_else(|e| e.into_inner());
        let mut sent = 0u64;
        let mut arrived = 0u64;
        for &(_, at, bytes, got) in g.iter() {
            if at < from || at > to {
                continue;  // ★아직 안 물어본 구간은 판정 대상이 아니다
            }
            sent += u64::from(bytes);
            if got.is_some() {
                arrived += u64::from(bytes);
            }
        }
        (sent, arrived)
    }

    /// 창 안에서 ★**편도 지연이 얼마나 자랐나**(ms).
    ///
    /// 도착 시각은 구독자 시계라 절대값을 못 쓴다 — 쓰는 것은 ★**차분**이다. 시계 오프셋은
    /// 상수라 (도착 − 송신) 의 **변화**가 곧 지연의 변화다. 큐가 자라는 구간에서 이 값이 오른다.
    /// 창 안에 확인된 것이 둘 미만이면 `None`(못 잰 것이지 0 이 아니다).
    pub fn delay_growth_ms(&self, now_ms: u64) -> Option<i64> {
        let to = self.judged_until_ms.load(Ordering::Relaxed);
        let from = to.saturating_sub(RATE_WINDOW_MS);
        let _ = now_ms;
        let g = self.log.lock().unwrap_or_else(|e| e.into_inner());
        let mut first = None;
        let mut last = None;
        for &(_, at, _, got) in g.iter() {
            let Some(arrival) = got else { continue };
            if at < from || at > to {
                continue;
            }
            let d = arrival as i64 - at as i64;
            first.get_or_insert(d);
            last = Some(d);
        }
        match (first, last) {
            (Some(a), Some(b)) if a != b => Some(b - a),
            _ => None,
        }
    }

    /// 정§10-3 v2 의 손실 축 — 창 안에 보낸 것이 ★**제때 도착하지 못한 몫**(%).
    ///
    /// ★유실과 **밀림을 한 축으로** 센다. 큐가 자라면 패킷이 사라지지 않고 **늦게** 온다 —
    /// 창 길이만큼 밀린 것은 그 창에서 통째로 빠진 것과 같으므로, 자란 지연을 창으로 나눠
    /// 같은 자에 올린다. ★지연 추세를 위한 임계를 따로 지어낼 자리가 없다.
    /// 창 안에 보낸 것이 없으면 `None`(못 잰 것이지 0 이 아니다).
    pub fn miss_pct(&self, now_ms: u64) -> Option<f64> {
        let (sent, arrived) = self.window(now_ms);
        if sent == 0 {
            return None;
        }
        let lost = (sent - arrived) as f64 * 100.0 / sent as f64;
        let late = self
            .delay_growth_ms(now_ms)
            .map_or(0.0, |ms| ms.max(0) as f64 * 100.0 / RATE_WINDOW_MS as f64);
        Some(lost.max(late))
    }

    /// ★그 전송로가 실제로 받아낸 속도. 추정이 아니라 측정이다 —
    /// 창 안에 **보낸** 것 중 **도착이 확인된** 바이트를 창 길이로 나눈다.
    /// 지연이 자라면 창 안에 확인되는 몫이 줄어 그대로 값이 내려간다(임계를 새로 지어낼 자리가 없다).
    ///
    /// ★시간축은 **서버가 보낸 시각**이다. 피드백이 싣고 오는 도착시각은 구독자 시계라 견줄 수 없다 —
    /// 피드백에서 받는 것은 「이 번호가 도착했다」는 **사실** 하나뿐이다.
    pub fn rate_bps(&self, now_ms: u64) -> Option<u64> {
        let (sent, arrived) = self.window(now_ms);
        (sent > 0).then(|| arrived * 8 * 1_000 / RATE_WINDOW_MS)
    }
}

/// 피드백 한 장이 알려 준 것 — ★도착한 것만 담는다(빠진 번호는 없는 것이지 0 이 아니다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feedback {
    pub received: Vec<(u16, u64)>,
    /// 보고 구간에서 ★도착 안 한 것의 수 — v2 의 손실 축이다(v1 은 RR 이 그 자리를 준다).
    pub lost: usize,
}

/// 구독자가 돌려준 FMT15 한 장을 읽는다. ★묶음 세 꼴을 다 읽는다 —
/// 서버는 run-length 만 짓지만 브라우저·봇은 status-vector 를 쓴다(한 꼴만 읽으면 조용히 아무것도 못 본다).
pub fn parse(pkt: &[u8]) -> Option<Feedback> {
    if pkt.len() < 20 || pkt[0] & 0x1F != FMT_TWCC || pkt[1] != PT_RTPFB {
        return None;
    }
    let base = u16::from_be_bytes([pkt[12], pkt[13]]);
    let count = usize::from(u16::from_be_bytes([pkt[14], pkt[15]]));
    let ref_ms = (u32::from_be_bytes([0, pkt[16], pkt[17], pkt[18]]) as u64) * REF_TICK_MS;

    let mut statuses: Vec<u8> = Vec::with_capacity(count);
    let mut at = 20;
    while statuses.len() < count {
        let chunk = u16::from_be_bytes([*pkt.get(at)?, *pkt.get(at + 1)?]);
        at += 2;
        if chunk & 0x8000 == 0 {
            let symbol = ((chunk >> 13) & 0x3) as u8;
            for _ in 0..(chunk & 0x1FFF).min((count - statuses.len()) as u16) {
                statuses.push(symbol);
            }
        } else if chunk & 0x4000 == 0 {
            for slot in 0..14 {
                if statuses.len() == count {
                    break;
                }
                statuses.push(((chunk >> (13 - slot)) & 0x1) as u8);
            }
        } else {
            for slot in 0..7 {
                if statuses.len() == count {
                    break;
                }
                statuses.push(((chunk >> (12 - slot * 2)) & 0x3) as u8);
            }
        }
    }

    let mut received = Vec::new();
    let mut lost = 0usize;
    let mut us = ref_ms * 1_000;
    for (step, &sym) in statuses.iter().enumerate() {
        let ticks: i64 = match sym {
            1 => i64::from(*pkt.get(at)?),
            2 => i64::from(i16::from_be_bytes([*pkt.get(at)?, *pkt.get(at + 1)?])),
            _ => {
                lost += 1;
                continue;
            }
        };
        at += if sym == 1 { 1 } else { 2 };
        us = (us as i64 + ticks * DELTA_TICK_US as i64).max(0) as u64;
        received.push((base.wrapping_add(step as u16), us / 1_000));
    }
    Some(Feedback { received, lost })
}

/// 받은 것들로 한 장을 짓는다. ★입력은 seq 오름차순이어야 한다(BTreeMap 이 그것을 준다).
///
/// 상태 묶음은 ★run-length 만 쓴다 — 같은 상태가 이어지는 구간을 한 묶음으로 접는다.
/// vector 묶음을 섞으면 인코더가 두 갈래가 되고, 그 이득은 상태가 잦게 바뀔 때뿐이다.
pub fn build(sender: u32, media: u32, arrivals: &[(u16, u64)], fb_count: u8) -> Vec<u8> {
    let base = arrivals[0].0;
    let last = arrivals[arrivals.len() - 1].0;
    let span = usize::from(last.wrapping_sub(base)) + 1;
    let span = span.min(MAX_REPORTED);
    let known: BTreeMap<u16, u64> = arrivals.iter().copied().collect();

    // 기준 시각은 64ms 눈금으로 내림한다 — 첫 도착이 음수 델타가 되지 않게.
    let ref_ms = (arrivals[0].1 / REF_TICK_MS) * REF_TICK_MS;
    let mut statuses: Vec<u8> = Vec::with_capacity(span);
    let mut deltas: Vec<u8> = Vec::new();
    let mut prev_us = ref_ms * 1_000;

    for step in 0..span {
        let seq = base.wrapping_add(step as u16);
        let Some(&at_ms) = known.get(&seq) else {
            statuses.push(0);
            continue;
        };
        let at_us = at_ms * 1_000;
        let ticks = (at_us as i64 - prev_us as i64) / DELTA_TICK_US as i64;
        prev_us = (prev_us as i64 + ticks * DELTA_TICK_US as i64) as u64;
        if (0..=255).contains(&ticks) {
            statuses.push(1);
            deltas.push(ticks as u8);
        } else {
            statuses.push(2);
            deltas.extend_from_slice(&(ticks.clamp(-32768, 32767) as i16).to_be_bytes());
        }
    }

    let mut out = Vec::with_capacity(20 + span);
    out.push(0x80 | FMT_TWCC);
    out.push(PT_RTPFB);
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&sender.to_be_bytes());
    out.extend_from_slice(&media.to_be_bytes());
    out.extend_from_slice(&base.to_be_bytes());
    out.extend_from_slice(&(statuses.len() as u16).to_be_bytes());
    let ref_ticks = ((ref_ms / REF_TICK_MS) & 0x00FF_FFFF) as u32;
    out.extend_from_slice(&ref_ticks.to_be_bytes()[1..]);
    out.push(fb_count);

    for (symbol, run) in runs(&statuses) {
        // run-length 묶음: bit15=0 · 상태 2비트 · 길이 13비트.
        let chunk = (u16::from(symbol) << 13) | (run & 0x1FFF);
        out.extend_from_slice(&chunk.to_be_bytes());
    }
    out.extend_from_slice(&deltas);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    let words = (out.len() / 4 - 1) as u16;
    out[2..4].copy_from_slice(&words.to_be_bytes());
    out
}

/// 같은 상태가 이어지는 구간으로 접는다. 길이는 13비트라 8191 에서 끊는다.
fn runs(statuses: &[u8]) -> Vec<(u8, u16)> {
    let mut out: Vec<(u8, u16)> = Vec::new();
    for &s in statuses {
        match out.last_mut() {
            Some((sym, run)) if *sym == s && *run < 0x1FFF => *run += 1,
            _ => out.push((s, 1)),
        }
    }
    out
}

#[cfg(test)]
mod v2_tests {
    use super::*;

    /// 서버가 지은 것을 서버가 읽는다 — 왕복이 서야 「무엇이 도착했나」가 사실이 된다.
    #[test]
    fn a_feedback_round_trips_through_run_length_chunks() {
        let arrivals = [(100u16, 1_000u64), (101, 1_020), (102, 1_040), (104, 1_100)];
        let fb = build(1, 2, &arrivals, 0);
        let got = parse(&fb).expect("읽힌다");
        let seqs: Vec<u16> = got.received.iter().map(|&(s, _)| s).collect();
        assert_eq!(seqs, vec![100, 101, 102, 104], "★빠진 103 은 없는 것이지 0 이 아니다");
        assert_eq!(got.lost, 1, "빠진 하나는 손실로 센다 — v2 의 손실 축이다");
        for (&(s, want), &(_, got_ms)) in arrivals.iter().zip(got.received.iter()) {
            assert!(got_ms.abs_diff(want) <= 1, "seq={s} {want} vs {got_ms}");
        }
    }

    /// ★브라우저·봇은 status-vector 꼴을 쓴다 — 한 꼴만 읽으면 조용히 아무것도 못 본다.
    #[test]
    fn a_two_bit_status_vector_chunk_is_read_too() {
        let mut pkt = vec![0x80 | FMT_TWCC, PT_RTPFB, 0, 0];
        pkt.extend_from_slice(&1u32.to_be_bytes());
        pkt.extend_from_slice(&2u32.to_be_bytes());
        pkt.extend_from_slice(&7u16.to_be_bytes());   // base seq
        pkt.extend_from_slice(&3u16.to_be_bytes());   // 3개 보고
        pkt.extend_from_slice(&[0, 0, 10]);           // ref = 10 tick = 640ms
        pkt.push(0);                                  // fb count
        // 2비트 vector: [small, lost, small, ...] → 0xC000 | 1<<12 | 0<<10 | 1<<8
        let chunk: u16 = 0xC000 | (1 << 12) | (1 << 8);
        pkt.extend_from_slice(&chunk.to_be_bytes());
        pkt.extend_from_slice(&[4, 8]);               // delta 4tick(1ms) · 8tick(2ms)
        let got = parse(&pkt).expect("읽힌다");
        assert_eq!(got.received.iter().map(|&(s, _)| s).collect::<Vec<_>>(), vec![7, 9]);
        assert_eq!(got.received[0].1, 641);
        assert_eq!(got.received[1].1, 643);
    }

    /// 수신률은 ★창 안에 **보낸** 것 중 **도착이 확인된** 몫이다.
    /// 확인은 피드백 여러 장에 걸쳐 쌓인다 — 한 장으로 창을 재면 값이 10분의 1 로 나온다.
    #[test]
    fn the_rate_accumulates_confirmations_across_feedbacks() {
        let d = Departures::default();
        let seqs: Vec<u16> = (0..20u64).map(|i| d.stamp(10_000 + i * 10, 1_000)).collect();
        let now = 10_200;
        assert_eq!(d.rate_bps(now), None, "★아무것도 안 물어봤다 — 잴 것이 없다(0 이 아니다)");

        // 피드백 두 장이 나눠서 확인해 준다.
        d.confirm(&seqs[..10].iter().map(|&s| (s, 999_999)).collect::<Vec<_>>());
        let half = d.rate_bps(now).expect("잰다");
        d.confirm(&seqs[10..].iter().map(|&s| (s, 999_999)).collect::<Vec<_>>());
        let full = d.rate_bps(now).expect("잰다");
        assert_eq!(full, 20 * 1_000 * 8, "20패킷 × 1,000B → 160kbps");
        assert_eq!(half, full / 2, "★확인이 쌓인다 — 한 장으로 끝내지 않는다");

        // 밀림은 미도착으로 센다 — 지연 추세를 위한 임계를 따로 지어내지 않는다.
        let late = Departures::default();
        let ls: Vec<u16> = (0..10u64).map(|i| late.stamp(10_000 + i, 100)).collect();
        assert_eq!(late.miss_pct(10_100), None, "★아무것도 안 물어봤다 — 잴 것이 없다");
        // 0..7 은 왔고 8·9 는 안 왔다. 판정은 마지막 확인분(7)까지만 — 8·9 는 아직 안 물어본 것이다.
        late.confirm(&ls[..8].iter().map(|&s| (s, 0)).collect::<Vec<_>>());
        assert_eq!(late.miss_pct(10_100), Some(0.0), "★안 물어본 꼬리를 손실로 세지 않는다");
        // 9 까지 물었는데 8 이 빠졌다 — 그때 비로소 손실이다.
        late.confirm(&[(ls[9], 0)]);
        assert!((late.miss_pct(10_100).unwrap() - 10.0).abs() < 0.01, "10 중 하나가 빠졌다 = 10%");

        // 밀림 — 다 도착했는데 지연이 창의 20% 만큼 자랐다. 같은 자에 오른다.
        let creep = Departures::default();
        let cs: Vec<u16> = (0..10u64).map(|i| creep.stamp(10_000 + i * 10, 100)).collect();
        creep.confirm(&cs.iter().enumerate().map(|(i, &s)| (s, 50_000 + i as u64 * 30)).collect::<Vec<_>>());
        assert_eq!(creep.delay_growth_ms(10_100), Some(9 * 30 - 9 * 10));
        assert!((creep.miss_pct(10_100).unwrap() - 18.0).abs() < 0.01, "자란 지연 180ms / 창 1,000ms = 18%");

        // ★창은 **판정이 닿은 지점**에서 뒤로 1초다 — 그보다 오래 전에 보낸 것은 안 센다.
        let win = Departures::default();
        let ancient = win.stamp(1_000, 9_999);
        let recent = win.stamp(10_000, 1_000);
        win.confirm(&[(ancient, 0), (recent, 0)]);
        assert_eq!(win.rate_bps(10_000), Some(1_000 * 8), "옛 것은 창 밖이라 안 센다");
        assert_eq!(Departures::default().miss_pct(10_000), None, "★못 잰 것은 None 이다 — 0% 와 다르다");
    }

    #[test]
    fn a_transport_numbers_its_own_packets() {
        let d = Departures::default();
        assert_eq!((d.stamp(0, 10), d.stamp(0, 10), d.stamp(0, 10)), (0, 1, 2), "전송로가 발급한다");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::rtcp;

    #[test]
    fn a_feedback_names_its_base_and_counts_what_it_covers() {
        let pkt = build(1, 42, &[(100, 0), (101, 5), (102, 10)], 7);
        assert_eq!(rtcp::payload_type(&pkt), Some(PT_RTPFB));
        assert_eq!(rtcp::fmt(&pkt), Some(FMT_TWCC));
        assert_eq!((rtcp::sender_ssrc(&pkt), rtcp::media_ssrc(&pkt)), (Some(1), Some(42)));
        assert_eq!(u16::from_be_bytes([pkt[12], pkt[13]]), 100, "base seq");
        assert_eq!(u16::from_be_bytes([pkt[14], pkt[15]]), 3, "덮은 개수");
        assert_eq!(pkt[19], 7, "피드백 일련번호");
        assert_eq!(pkt.len() % 4, 0, "RTCP 는 4바이트 정렬이다");
    }

    #[test]
    fn a_hole_is_reported_as_not_received() {
        // 100·102 만 받았다 — 101 은 상태 0 이고 델타가 없다.
        let pkt = build(1, 42, &[(100, 0), (102, 4)], 0);
        assert_eq!(u16::from_be_bytes([pkt[14], pkt[15]]), 3, "빠진 것도 덮는 범위다");
        let chunks = &pkt[20..];
        // 상태 1(받음) 1개 · 0(못 받음) 1개 · 1(받음) 1개 = 묶음 셋.
        assert_eq!(u16::from_be_bytes([chunks[0], chunks[1]]) >> 13, 1);
        assert_eq!(u16::from_be_bytes([chunks[2], chunks[3]]) >> 13, 0);
        assert_eq!(u16::from_be_bytes([chunks[4], chunks[5]]) >> 13, 1);
    }

    #[test]
    fn a_run_of_the_same_status_folds_into_one_chunk() {
        assert_eq!(runs(&[1, 1, 1]), vec![(1, 3)]);
        assert_eq!(runs(&[1, 0, 0, 1]), vec![(1, 1), (0, 2), (1, 1)]);
        assert_eq!(runs(&[]), vec![]);
    }

    #[test]
    fn the_ledger_empties_when_drained() {
        let a = Arrivals::default();
        assert!(a.drain(1, 2).is_none(), "담긴 것이 없으면 안 짓는다");
        a.observe(5, 0);
        a.observe(6, 5);
        assert_eq!(a.pending(), 2);
        assert!(a.drain(1, 2).is_some());
        assert_eq!(a.pending(), 0, "★비우지 않으면 같은 도착을 매 판 다시 보고한다");
    }

    #[test]
    fn the_feedback_counter_moves_each_time() {
        let a = Arrivals::default();
        a.observe(1, 0);
        let first = a.drain(1, 2).unwrap();
        a.observe(2, 1);
        let second = a.drain(1, 2).unwrap();
        assert_eq!((first[19], second[19]), (0, 1), "발행자가 유실을 알아채는 값이다");
    }

    #[test]
    fn the_ledger_does_not_grow_without_bound() {
        let a = Arrivals::default();
        for seq in 0..(MAX_REPORTED as u16 * 3) {
            a.observe(seq, u64::from(seq));
        }
        assert!(a.pending() <= MAX_REPORTED * 2);
    }
}
