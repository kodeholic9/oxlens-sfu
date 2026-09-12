// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§10-3 · model: claude-opus-5

//! 지연 기반 대역 추정 — ★**두 시계를 견주지 않는다.**
//!
//! 우리가 보낸 시각은 서버 시계, 도착 시각은 구독자 시계다. 절대차는 뜻이 없고
//! ★**차분끼리의 차**(도착 간격 − 송신 간격)만 뜻이 있다 — 오프셋은 거기서 상쇄된다.
//!
//! 얼개: 송신 묶음(≈5ms burst) 사이의 지연 변화를 모아 추세선을 긋고, 기울기가 적응
//! 임계를 넘으면 `Overusing`, 그 판정으로 AIMD 를 돌린다. 손실 기반 값과 ★`min` 으로
//! 합친다(둘 중 나쁜 쪽이 진실이다).
//!
//! ★**안 보내는 만큼을 못 받는 것으로 읽지 않는다**(ALR) — 정적 장면처럼 낼 것이 없어
//! 송신이 준 구간에서 추정을 따라 내리면 ★**제가 만든 하강으로 제가 강등**한다.
//! 그 구간에는 추정을 얼리고, 진짜 혼잡(`Overusing`)만 예외로 내린다.
//!
//! 출처: Carlucci et al.(ACM MMSys 2016) · draft-ietf-rmcat-gcc · libwebrtc goog_cc.

/// 정§10-3 의 수치와 그 옆 실측 상수.
pub mod v {
    /// 시작 추정 — ★**모르는 채로 크게 잡지 않는다.**
    pub const INITIAL_BPS: f64 = 300_000.0;
    /// 바닥 — 이 아래로는 안 내려간다.
    pub const MIN_BPS: f64 = 30_000.0;
    /// 송신 묶음 경계.
    pub const BURST_MS: f64 = 5.0;
    /// 추세선 창.
    pub const TRENDLINE_WINDOW: usize = 20;
    pub const TRENDLINE_SMOOTHING: f64 = 0.9;
    pub const TRENDLINE_GAIN: f64 = 4.0;
    /// 순간 스파이크를 혼잡으로 읽지 않기 위한 지속 조건.
    pub const OVERUSE_TIME_MS: f64 = 10.0;
    pub const THRESHOLD_INIT_MS: f64 = 12.5;
    pub const THRESHOLD_K_UP: f64 = 0.0087;
    pub const THRESHOLD_K_DOWN: f64 = 0.039;
    /// 감산 계수.
    pub const AIMD_BETA: f64 = 0.85;
    /// 처리율 창.
    pub const RATE_WINDOW_MS: f64 = 500.0;
    /// ALR 진입/이탈 사용률.
    pub const ALR_START_RATIO: f64 = 0.50;
    pub const ALR_STOP_RATIO: f64 = 0.65;
    /// 프로브 실측을 그대로 믿지 않고 깎는 몫.
    pub const PROBE_DISCOUNT: f64 = 0.9;
}

/// 피드백 한 건에서 복원한 패킷 하나.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    /// 우리가 보낸 시각(ms, 서버 시계).
    pub send_ms: f64,
    /// 구독자가 받았다는 시각(ms, 구독자 시계).
    pub arrival_ms: f64,
    pub size: u16,
}

/// 대역 사용 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Usage {
    #[default]
    Normal,
    Overusing,
    Underusing,
}

impl Usage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Usage::Normal => "normal",
            Usage::Overusing => "overuse",
            Usage::Underusing => "underuse",
        }
    }
}

/// 송신 묶음 하나.
#[derive(Debug, Clone, Copy)]
struct Group {
    first_send_ms: f64,
    last_send_ms: f64,
    last_arrival_ms: f64,
}

#[derive(Debug)]
pub struct Gcc {
    current_group: Option<Group>,
    prev_group: Option<Group>,

    history: Vec<(f64, f64)>,
    accumulated_delay_ms: f64,
    smoothed_delay_ms: f64,
    first_arrival_ms: Option<f64>,

    threshold_ms: f64,
    overuse_since_ms: Option<f64>,
    usage: Usage,
    last_update_ms: Option<f64>,

    delay_rate_bps: f64,
    /// ★`None` = 아직 한 번도 갱신 안 했다 — `0` 을 부재 표식으로 쓰지 않는다.
    last_rate_update_ms: Option<f64>,
    /// 마지막 감산 때의 실측 처리율 — 수렴 근처 판정에 쓴다.
    link_capacity_bps: Option<f64>,

    loss_rate_bps: f64,

    /// (도착 ms, 크기) — 구독자 시계.
    acked_window: Vec<(f64, u16)>,
    /// (송신 ms, 크기) — 서버 시계.
    sent_window: Vec<(f64, u16)>,
    pub alr_active: bool,
}

impl Default for Gcc {
    fn default() -> Self {
        Self::new()
    }
}

impl Gcc {
    pub fn new() -> Self {
        Self {
            current_group: None,
            prev_group: None,
            history: Vec::new(),
            accumulated_delay_ms: 0.0,
            smoothed_delay_ms: 0.0,
            first_arrival_ms: None,
            threshold_ms: v::THRESHOLD_INIT_MS,
            overuse_since_ms: None,
            usage: Usage::Normal,
            last_update_ms: None,
            delay_rate_bps: v::INITIAL_BPS,
            last_rate_update_ms: None,
            link_capacity_bps: None,
            loss_rate_bps: v::INITIAL_BPS,
            acked_window: Vec::new(),
            sent_window: Vec::new(),
            alr_active: false,
        }
    }

    /// ★**둘 중 나쁜 쪽이 진실이다** — 지연 기반과 손실 기반의 `min`.
    pub fn estimate_bps(&self) -> u64 {
        self.delay_rate_bps.min(self.loss_rate_bps).max(v::MIN_BPS) as u64
    }

    pub fn state(&self) -> Usage {
        self.usage
    }

    /// 보낸 것을 적어 둔다 — ALR 사용률의 분모다.
    pub fn on_sent(&mut self, send_ms: f64, size: u16) {
        self.sent_window.push((send_ms, size));
    }

    /// 피드백 한 건. `lost` = 그 묶음에서 못 받았다고 한 수.
    pub fn on_feedback(&mut self, samples: &[Sample], lost: usize, now_ms: f64) {
        // ★손실 기반(draft-gcc §6) — 2% 아래면 올리고, 10% 위면 깎고, 사이는 유지.
        let received = samples.len();
        if received + lost > 0 {
            let p = lost as f64 / (received + lost) as f64;
            if p > 0.10 {
                self.loss_rate_bps *= 1.0 - 0.5 * p;
            } else if p < 0.02 {
                // ★지연 기반 위로 폭주하지 않게 묶는다.
                self.loss_rate_bps = (self.loss_rate_bps * 1.05).min(self.delay_rate_bps * 1.5);
            }
            self.loss_rate_bps = self.loss_rate_bps.max(v::MIN_BPS);
        }

        for s in samples {
            self.acked_window.push((s.arrival_ms, s.size));
            self.process_packet(*s);
        }
        self.prune(now_ms);
        // ★ALR 판정을 속도 갱신 **앞**에 — 얼릴지를 먼저 정해야 app-limited 하강을 막는다.
        self.update_alr(now_ms);
        self.update_rate(now_ms);
    }

    /// 프로브가 완주한 뒤의 실측 반영 — ★**올리기만 한다.**
    ///
    /// ★**혼잡 중에는 무반영**이다(프로브가 멎은 것 자체가 *"수요를 못 받는다"* 는 측정이다).
    pub fn apply_probe(&mut self) -> Option<f64> {
        if self.usage == Usage::Overusing {
            return None;
        }
        let acked = self.max_windowed_acked_bps()?;
        let boosted = acked * v::PROBE_DISCOUNT;
        if boosted > self.delay_rate_bps {
            self.delay_rate_bps = boosted;
            // ★`min` 합성이 프로브 실측을 가리지 않게 손실 기반도 같이 올린다 —
            //   손실이 있었다면 다음 피드백이 정직하게 도로 끌어내린다.
            self.loss_rate_bps = self.loss_rate_bps.max(boosted);
        }
        Some(acked)
    }

    /// ★**창 안 최대 처리율**로 잰다 — *"마지막 도착 기준"* 으로 재면 프로브가 끝난 뒤
    /// 들어온 저율 미디어 꼬리가 창을 밀어 측정이 희석된다(실측 2.3M → 1.5M).
    fn max_windowed_acked_bps(&self) -> Option<f64> {
        if self.acked_window.is_empty() {
            return None;
        }
        let mut w = self.acked_window.clone();
        w.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (mut best, mut sum, mut j) = (0u64, 0u64, 0usize);
        for i in 0..w.len() {
            sum += w[i].1 as u64;
            while w[i].0 - w[j].0 > v::RATE_WINDOW_MS {
                sum -= w[j].1 as u64;
                j += 1;
            }
            best = best.max(sum);
        }
        if best == 0 {
            return None;
        }
        Some(best as f64 * 8.0 * 1000.0 / v::RATE_WINDOW_MS)
    }

    fn process_packet(&mut self, s: Sample) {
        let Some(g) = self.current_group.as_mut() else {
            self.current_group = Some(Group {
                first_send_ms: s.send_ms,
                last_send_ms: s.send_ms,
                last_arrival_ms: s.arrival_ms,
            });
            return;
        };
        if s.send_ms - g.first_send_ms < v::BURST_MS {
            g.last_send_ms = s.send_ms;
            // ★재정렬 방어 — 도착은 최대값을 쥔다.
            if s.arrival_ms > g.last_arrival_ms {
                g.last_arrival_ms = s.arrival_ms;
            }
            return;
        }
        let done = *g;
        if let Some(prev) = self.prev_group {
            let d_send = done.last_send_ms - prev.last_send_ms;
            let d_arr = done.last_arrival_ms - prev.last_arrival_ms;
            self.add_delay_sample(done.last_arrival_ms, d_arr - d_send);
        }
        self.prev_group = Some(done);
        self.current_group = Some(Group {
            first_send_ms: s.send_ms,
            last_send_ms: s.send_ms,
            last_arrival_ms: s.arrival_ms,
        });
    }

    fn add_delay_sample(&mut self, arrival_ms: f64, variation_ms: f64) {
        let first = *self.first_arrival_ms.get_or_insert(arrival_ms);
        self.accumulated_delay_ms += variation_ms;
        self.smoothed_delay_ms = v::TRENDLINE_SMOOTHING * self.smoothed_delay_ms
            + (1.0 - v::TRENDLINE_SMOOTHING) * self.accumulated_delay_ms;
        self.history.push((arrival_ms - first, self.smoothed_delay_ms));
        if self.history.len() > v::TRENDLINE_WINDOW {
            self.history.remove(0);
        }
        if self.history.len() == v::TRENDLINE_WINDOW
            && let Some(slope) = slope(&self.history)
        {
            self.detect(slope, arrival_ms);
        }
    }

    fn detect(&mut self, slope: f64, now_arrival_ms: f64) {
        let modified = slope * v::TRENDLINE_GAIN * (v::TRENDLINE_WINDOW as f64);
        let dt = self.last_update_ms.map_or(0.0, |t| (now_arrival_ms - t).clamp(0.0, 100.0));
        self.last_update_ms = Some(now_arrival_ms);

        if modified > self.threshold_ms {
            let since = *self.overuse_since_ms.get_or_insert(now_arrival_ms);
            if now_arrival_ms - since >= v::OVERUSE_TIME_MS {
                self.usage = Usage::Overusing;
            }
        } else if modified < -self.threshold_ms {
            self.overuse_since_ms = None;
            self.usage = Usage::Underusing;
        } else {
            self.overuse_since_ms = None;
            self.usage = Usage::Normal;
        }

        // ★임계도 따라간다 — 멀면 천천히, 가까우면 빨리.
        let k = if modified.abs() < self.threshold_ms { v::THRESHOLD_K_DOWN } else { v::THRESHOLD_K_UP };
        self.threshold_ms += k * (modified.abs() - self.threshold_ms) * dt;
        self.threshold_ms = self.threshold_ms.clamp(6.0, 600.0);
    }

    fn acked_bps(&self, now_arrival_ms: f64) -> Option<f64> {
        let from = now_arrival_ms - v::RATE_WINDOW_MS;
        let bytes: u64 =
            self.acked_window.iter().filter(|(t, _)| *t >= from).map(|(_, s)| *s as u64).sum();
        if bytes == 0 {
            return None;
        }
        Some(bytes as f64 * 8.0 * 1000.0 / v::RATE_WINDOW_MS)
    }

    fn update_rate(&mut self, now_ms: f64) {
        // ★app-limited 구간은 증감 둘 다 건너뛴다 — 안 보낸 것을 못 받은 것으로 읽지 않는다.
        //   진짜 혼잡만 예외로 내린다.
        if self.alr_active && self.usage != Usage::Overusing {
            return;
        }
        let dt_s = match self.last_rate_update_ms {
            None => 0.1,
            Some(t) => ((now_ms - t) / 1000.0).clamp(0.001, 1.0),
        };
        self.last_rate_update_ms = Some(now_ms);

        let arrival_now = self.history.last().map(|(t, _)| *t).unwrap_or(0.0)
            + self.first_arrival_ms.unwrap_or(0.0);
        let acked = self.acked_bps(arrival_now);

        match self.usage {
            Usage::Overusing => {
                let base = acked.unwrap_or(self.delay_rate_bps);
                let target = v::AIMD_BETA * base;
                if target < self.delay_rate_bps {
                    self.delay_rate_bps = target;
                }
                self.link_capacity_bps = acked;
            }
            Usage::Normal => {
                // 수렴 근처면 가산, 멀면 승산.
                let near = self
                    .link_capacity_bps
                    .map(|c| self.delay_rate_bps > 0.9 * c && self.delay_rate_bps < 1.1 * c)
                    .unwrap_or(false);
                if near {
                    self.delay_rate_bps += 96_000.0 * dt_s;
                } else {
                    self.delay_rate_bps *= 1.0 + 0.08 * dt_s;
                }
                // ★안 보낸 대역을 상상으로 올리지 않는다 — 실측의 1.5배가 천장이다.
                if let Some(a) = acked {
                    self.delay_rate_bps = self.delay_rate_bps.min(1.5 * a + 10_000.0);
                }
            }
            // 큐가 비는 중 — 올리지도 내리지도 않는다.
            Usage::Underusing => {}
        }
        self.delay_rate_bps = self.delay_rate_bps.max(v::MIN_BPS);
    }

    fn update_alr(&mut self, now_ms: f64) {
        let from = now_ms - v::RATE_WINDOW_MS;
        let sent: u64 =
            self.sent_window.iter().filter(|(t, _)| *t >= from).map(|(_, s)| *s as u64).sum();
        let sent_bps = sent as f64 * 8.0 * 1000.0 / v::RATE_WINDOW_MS;
        let ratio = sent_bps / self.estimate_bps().max(1) as f64;

        if self.alr_active {
            if ratio > v::ALR_STOP_RATIO {
                self.alr_active = false;
            }
        } else if ratio < v::ALR_START_RATIO && sent > 0 {
            self.alr_active = true;
        }
    }

    fn prune(&mut self, now_ms: f64) {
        // ★두 창은 시계가 다르다 — 각자 제 시계로 걷는다.
        let keep = 2.0 * v::RATE_WINDOW_MS;
        if let Some(&(last, _)) = self.acked_window.last() {
            self.acked_window.retain(|(t, _)| last - *t <= keep);
        }
        self.sent_window.retain(|(t, _)| now_ms - *t <= keep);
    }
}

/// 최소자승 기울기.
fn slope(points: &[(f64, f64)]) -> Option<f64> {
    let n = points.len() as f64;
    if n < 2.0 {
        return None;
    }
    let sx: f64 = points.iter().map(|(x, _)| x).sum();
    let sy: f64 = points.iter().map(|(_, y)| y).sum();
    let sxx: f64 = points.iter().map(|(x, _)| x * x).sum();
    let sxy: f64 = points.iter().map(|(x, y)| x * y).sum();
    let denom = n * sxx - sx * sx;
    if denom.abs() < 1e-9 {
        return None;
    }
    Some((n * sxy - sx * sy) / denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 피드백 한 건 분량 — `t0` 부터 `gap` 간격 20개, 도착 = 송신 + owd + 큐(i).
    fn fb(t0: f64, gap: f64, size: u16, queue: f64, owd: f64) -> Vec<Sample> {
        (0..20)
            .map(|i| {
                let send = t0 + i as f64 * gap;
                Sample { send_ms: send, arrival_ms: send + owd + queue * i as f64, size }
            })
            .collect()
    }

    /// `n` 건을 100ms 간격으로 넣는다.
    fn run(
        g: &mut Gcc,
        start: f64,
        n: usize,
        size: u16,
        queue: impl Fn(usize) -> f64,
        lost: usize,
    ) {
        let mut owd = 10.0;
        for k in 0..n {
            let now = start + k as f64 * 100.0;
            let q = queue(k);
            let samples = fb(now, 5.0, size, q, owd);
            owd += q * 20.0;
            for s in &samples {
                g.on_sent(s.send_ms, s.size);
            }
            g.on_feedback(&samples, lost, now + 100.0);
        }
    }

    #[test]
    fn 깨끗한_길에서는_추정이_큰다() {
        let mut g = Gcc::new();
        run(&mut g, 0.0, 200, 1000, |_| 0.0, 0);
        assert_eq!(g.state(), Usage::Normal);
        let e = g.estimate_bps();
        assert!(e > 1_000_000, "{e}");
        // ★실측의 1.5배가 천장이다 — 상상으로 더 올리지 않는다.
        assert!(e <= 2_600_000, "{e}");
    }

    #[test]
    fn 큐가_쌓이면_혼잡으로_읽고_깎는다() {
        let mut g = Gcc::new();
        run(&mut g, 0.0, 200, 1000, |_| 0.0, 0);
        let before = g.estimate_bps();
        run(&mut g, 21_000.0, 30, 1000, |_| 2.0, 0);
        assert_eq!(g.state(), Usage::Overusing, "지연 기울기가 혼잡이다");
        assert!(g.estimate_bps() < before, "{before} → {}", g.estimate_bps());
    }

    #[test]
    fn 큐가_풀리면_다시_큰다() {
        let mut g = Gcc::new();
        run(&mut g, 0.0, 30, 1000, |_| 0.0, 0);
        run(&mut g, 3_000.0, 30, 1000, |_| 2.0, 0);
        let low = g.estimate_bps();
        run(&mut g, 6_000.0, 10, 1000, |_| -1.5, 0);
        run(&mut g, 7_000.0, 40, 1000, |_| 0.0, 0);
        assert!(g.estimate_bps() > low, "{low} → {}", g.estimate_bps());
        assert_eq!(g.state(), Usage::Normal);
    }

    #[test]
    fn 안_보내는_구간에서는_얼린다() {
        // ★여기가 안 얼면 제가 만든 하강으로 제가 강등한다.
        let mut g = Gcc::new();
        run(&mut g, 0.0, 50, 1000, |_| 0.0, 0);
        let before = g.estimate_bps();
        run(&mut g, 5_000.0, 50, 100, |_| 0.0, 0);
        assert!(g.alr_active, "저사용률이면 ALR 이다");
        let after = g.estimate_bps();
        assert!(after as f64 >= before as f64 * 0.95, "{before} → {after}");
    }

    #[test]
    fn 손실이_크면_깎는다() {
        let mut g = Gcc::new();
        run(&mut g, 0.0, 30, 1000, |_| 0.0, 0);
        let before = g.estimate_bps();
        run(&mut g, 3_000.0, 20, 1000, |_| 0.0, 5);
        assert!(g.estimate_bps() < before, "{before} → {}", g.estimate_bps());
    }

    #[test]
    fn 프로브는_올리기만_하고_혼잡_중엔_무반영이다() {
        let mut g = Gcc::new();
        run(&mut g, 0.0, 10, 1000, |_| 0.0, 0);
        let before = g.estimate_bps();
        run(&mut g, 1_000.0, 5, 1500, |_| 0.0, 0);
        let acked = g.apply_probe().expect("실측");
        assert!(acked > 1_500_000.0, "{acked}");
        assert!(g.estimate_bps() >= before, "★올리기만 한다");
        assert!(g.estimate_bps() > 1_500_000, "{}", g.estimate_bps());
        run(&mut g, 2_000.0, 30, 1500, |_| 2.0, 0);
        assert_eq!(g.state(), Usage::Overusing);
        assert!(g.apply_probe().is_none(), "★혼잡 중엔 무반영");
    }

    #[test]
    fn 프로브_뒤_미디어_꼬리가_측정을_희석하지_않는다() {
        // ★"마지막 도착 기준 창"으로 재면 여기서 1.0M 로 희석된다(실측 회귀 봉인).
        let mut g = Gcc::new();
        run(&mut g, 0.0, 10, 1000, |_| 0.0, 0);
        run(&mut g, 1_000.0, 5, 1500, |_| 0.0, 0);
        run(&mut g, 1_500.0, 3, 100, |_| 0.0, 0);
        let acked = g.apply_probe().expect("실측");
        assert!(acked > 2_000_000.0, "{acked}");
    }

    #[test]
    fn 기울기_셈이_맞다() {
        let up: Vec<(f64, f64)> = (0..20).map(|i| (i as f64, i as f64)).collect();
        assert!((slope(&up).unwrap() - 1.0).abs() < 1e-9);
        let flat: Vec<(f64, f64)> = (0..20).map(|i| (i as f64, 5.0)).collect();
        assert!(slope(&flat).unwrap().abs() < 1e-9);
    }
}
