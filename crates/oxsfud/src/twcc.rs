// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§11-2 · §10-2 · model: claude-opus-5

//! TWCC — ★★**시간축은 서버가 보낸 시각이다.**
//!
//! 피드백이 싣고 오는 도착시각은 ★**구독자 시계**라 우리 시계와 견줄 수 없다 —
//! ★**쓰는 것은 차분(도착 간격)뿐**이고, 절대 시각을 섞으면 ★**시계 오프셋이 그대로 추세가 된다.**
//!
//! 그래서 egress twcc seq 는 ★**서버가 교체 스탬핑**한다 — 발행자 값을 통과시키면
//! 여러 발행자의 seq 가 한 구독 전송로에 섞여 ★**구독자 피드백이 무의미**해진다.

use std::collections::BTreeMap;

/// 한 전송로의 송신 장부. ★**우리 시각만 담는다.**
#[derive(Debug, Default)]
pub struct SendLedger {
    /// egress twcc seq → 우리가 보낸 시각.
    sent: BTreeMap<u16, u64>,
    next_seq: u16,
}

/// 피드백 한 줄 — 구독자가 말한 것.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub seq: u16,
    /// ★**구독자 시계의 도착시각.** 우리 시각과 견주지 않는다.
    pub arrived_at: Option<u64>,
}

/// 한 묶음에서 얻는 것.
#[derive(Debug, Clone, PartialEq)]
pub struct Delay {
    /// ★**차분끼리의 차**(도착 간격 − 송신 간격). 양수면 밀리는 중이다.
    pub trend_us: Vec<i64>,
    pub lost: u32,
    pub received: u32,
}

impl SendLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// ★**서버가 번호를 매긴다** — 발행자 값을 통과시키지 않는다.
    pub fn stamp(&mut self, now: u64) -> u16 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.sent.insert(seq, now);
        // 장부가 무한히 자라지 않게 오래된 것을 걷는다 — 핫패스 밖(피드백 주기)에서 해도 된다.
        while self.sent.len() > 4_096 {
            let Some((&k, _)) = self.sent.iter().next() else { break };
            self.sent.remove(&k);
        }
        seq
    }

    /// ★**차분만 쓴다.** 두 시계를 섞지 않는다.
    pub fn ingest(&mut self, reports: &[Report]) -> Delay {
        let mut trend = Vec::new();
        let mut lost = 0;
        let mut received = 0;
        let mut prev: Option<(u64, u64)> = None; // (우리 송신, 구독자 도착)
        for r in reports {
            let Some(arrived) = r.arrived_at else {
                lost += 1;
                continue;
            };
            received += 1;
            let Some(&sent_at) = self.sent.get(&r.seq) else { continue };
            if let Some((psent, parr)) = prev {
                let d_send = sent_at as i64 - psent as i64;
                let d_arr = arrived as i64 - parr as i64;
                // ★오프셋은 여기서 상쇄된다 — 그래서 두 시계를 견주지 않아도 추세가 나온다.
                trend.push(d_arr - d_send);
            }
            prev = Some((sent_at, arrived));
        }
        for r in reports {
            self.sent.remove(&r.seq);
        }
        Delay { trend_us: trend, lost, received }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 서버가_번호를_매긴다() {
        // ★발행자 값을 통과시키면 여러 발행자의 seq 가 한 전송로에 섞인다.
        let mut a = SendLedger::new();
        let mut b = SendLedger::new();
        assert_eq!(a.stamp(0), 0);
        assert_eq!(a.stamp(1), 1);
        assert_eq!(b.stamp(0), 0, "★전송로마다 제 공간이다");
    }

    #[test]
    fn 시계_오프셋은_추세를_안_흔든다() {
        // ★절대 시각을 견주면 오프셋이 그대로 추세가 된다 — 차분만 쓰면 상쇄된다.
        let mut l = SendLedger::new();
        let seqs: Vec<u16> = (0..4).map(|i| l.stamp(1_000 + i * 20)).collect();
        // 구독자 시계가 1억만큼 앞선다 — 간격은 같다(밀림 없음).
        let reports: Vec<Report> = seqs
            .iter()
            .enumerate()
            .map(|(i, s)| Report { seq: *s, arrived_at: Some(100_000_000 + i as u64 * 20) })
            .collect();
        let d = l.ingest(&reports);
        assert!(d.trend_us.iter().all(|t| *t == 0), "★오프셋이 상쇄되어야 한다: {:?}", d.trend_us);
        assert_eq!(d.received, 4);
    }

    #[test]
    fn 밀리면_추세가_양수다() {
        let mut l = SendLedger::new();
        let seqs: Vec<u16> = (0..3).map(|i| l.stamp(i * 20)).collect();
        // 도착 간격이 송신 간격보다 크다.
        let reports: Vec<Report> = seqs
            .iter()
            .enumerate()
            .map(|(i, s)| Report { seq: *s, arrived_at: Some(i as u64 * 35) })
            .collect();
        let d = l.ingest(&reports);
        assert!(d.trend_us.iter().all(|t| *t > 0), "{:?}", d.trend_us);
    }

    #[test]
    fn 못_받은_것은_따로_센다() {
        let mut l = SendLedger::new();
        let s0 = l.stamp(0);
        let s1 = l.stamp(20);
        let d = l.ingest(&[
            Report { seq: s0, arrived_at: Some(100) },
            Report { seq: s1, arrived_at: None },
        ]);
        assert_eq!(d.lost, 1);
        assert_eq!(d.received, 1);
    }

    #[test]
    fn 장부가_무한히_자라지_않는다() {
        let mut l = SendLedger::new();
        for i in 0..5_000u64 {
            l.stamp(i);
        }
        assert!(l.sent.len() <= 4_096);
    }
}
