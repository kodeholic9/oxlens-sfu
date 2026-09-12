// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§11-2 · §8-1 · model: claude-opus-5

//! RTCP — ★★**서버는 relay 가 아니라 두 세션의 종단이다**(정§11-2).
//!
//! | 방향 | 누가 만드나 |
//! |---|---|
//! | 발행자에게 가는 RR | ★**서버가 자체 생성**(1초) — *"우리가 너를 이만큼 받았다"* |
//! | 구독자에게 가는 SR | ★**자체 생성 금지** — 발행자 SR 을 **번역 릴레이**한다 |
//! | 구독자가 보낸 RR | ★**서버가 소비한다** — 발행자에게 릴레이하면 발행자가 ★**남의 수신
//!   품질**로 비트레이트를 깎는다 |
//!
//! ★**NTP 시각은 번역해도 원본을 유지한다** — 서버 시계를 실으면 수신 jb 가 폭등하고
//! lip sync 가 어긋난다(Janus 선례).

/// 한 덩어리(compound) 안의 패킷 종류.
pub const PT_SR: u8 = 200;
pub const PT_RR: u8 = 201;
pub const PT_SDES: u8 = 202;
pub const PT_BYE: u8 = 203;
pub const PT_APP: u8 = 204;
/// 전송 계층 피드백 — `fmt 1` NACK · `fmt 15` TWCC.
pub const PT_RTPFB: u8 = 205;
/// 페이로드 계층 피드백 — `fmt 1` PLI · `fmt 15` REMB.
pub const PT_PSFB: u8 = 206;

const SR_MIN: usize = 28;

/// ★**RTP 와 RTCP 는 같은 대역으로 온다**(0x80~0xBF) — 가르는 것은 둘째 바이트다(RFC 5761).
pub fn is_rtcp(pkt: &[u8]) -> bool {
    matches!(pkt.get(1), Some(&pt) if (PT_SR..=PT_PSFB).contains(&pt))
}

/// ★**복호 뒤 평문에서 패킷 단위로 분해한다**(정§11-2 `1pc` 분해) — `1pc` 은 단일 5-tuple 로
/// SR(발행 축)과 RR/NACK/PLI(구독 축)가 ★**섞여 온다.**
///
/// ★**못 나눈 꼬리는 버린다** — 길이 칸이 거짓이면 그 뒤는 믿을 수 없다.
pub fn split(compound: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 4 <= compound.len() {
        let len = (u16::from_be_bytes([compound[at + 2], compound[at + 3]]) as usize + 1) * 4;
        if len == 0 || at + len > compound.len() {
            break;
        }
        out.push(&compound[at..at + len]);
        at += len;
    }
    out
}

/// 그 SR 이 누구 것이고 NTP 가 무엇인가.
pub fn sr_ntp(pkt: &[u8]) -> Option<(u32, u32, u32)> {
    if pkt.len() < SR_MIN || pkt[1] != PT_SR {
        return None;
    }
    let g = |a: usize| u32::from_be_bytes([pkt[a], pkt[a + 1], pkt[a + 2], pkt[a + 3]]);
    Some((g(4), g(8), g(12)))
}

/// 구독자에게 내보낼 SR 로 바꾼다. ★**지어내지 않고 고쳐 쓴다.**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SrPatch {
    /// 반이중이면 슬롯 SSRC — 회의면 `None`(원본).
    pub ssrc: Option<u32>,
    /// ★**RTP 전달과 같은 rewriter offset 을 쓴다** — 다르면 NTP↔RTP 사상이 어긋나
    /// AV sync 가 무너진다(정§11-2).
    pub rtp_ts: Option<u32>,
    pub packet_count: u32,
    pub octet_count: u32,
}

pub fn translate_sr(pkt: &[u8], p: &SrPatch) -> Option<Vec<u8>> {
    if pkt.len() < SR_MIN || pkt[1] != PT_SR {
        return None;
    }
    let mut out = pkt.to_vec();
    if let Some(s) = p.ssrc {
        out[4..8].copy_from_slice(&s.to_be_bytes());
    }
    // ★NTP(8..16)는 손대지 않는다 — lip sync 의 기준점이다.
    if let Some(t) = p.rtp_ts {
        out[16..20].copy_from_slice(&t.to_be_bytes());
    }
    out[20..24].copy_from_slice(&p.packet_count.to_be_bytes());
    out[24..28].copy_from_slice(&p.octet_count.to_be_bytes());
    Some(out)
}

/// Generic NACK(RFC 4585 RTPFB `fmt 1`)이 가리키는 seq 들.
///
/// ★★**`fmt` 로 갈린다** — RTPFB 라고 다 NACK 이 아니다: `fmt 1` 이 NACK 이고
/// ★**`fmt 15` 는 TWCC 피드백**(정§11-2). 안 가르면 TWCC 를 NACK 으로 읽어
/// 있지도 않은 손실에 재전송을 쏜다.
///
/// 한 칸은 `PID`(16) + `BLP`(16) — BLP 의 비트 `i` 가 서면 `PID + 1 + i` 도 빠졌다.
pub fn nack_seqs(pkt: &[u8]) -> Option<(u32, Vec<u16>)> {
    if pkt.len() < 12 || pkt[1] != PT_RTPFB || pkt[0] & 0x1F != 1 {
        return None;
    }
    let media_ssrc = u32::from_be_bytes([pkt[8], pkt[9], pkt[10], pkt[11]]);
    let mut out = Vec::new();
    let mut at = 12;
    while at + 4 <= pkt.len() {
        let pid = u16::from_be_bytes([pkt[at], pkt[at + 1]]);
        let blp = u16::from_be_bytes([pkt[at + 2], pkt[at + 3]]);
        out.push(pid);
        for i in 0..16 {
            if blp & (1 << i) != 0 {
                out.push(pid.wrapping_add(1 + i));
            }
        }
        at += 4;
    }
    Some((media_ssrc, out))
}

/// 머리 끝 = 본문 시작. ★**고정 12바이트가 아니다** — CSRC 와 확장이 그 뒤에 온다.
fn head_len(pkt: &[u8]) -> Option<usize> {
    if pkt.len() < 12 || pkt[0] >> 6 != 2 {
        return None;
    }
    let mut at = 12 + (pkt[0] & 0x0F) as usize * 4;
    if pkt[0] & 0x10 != 0 {
        if pkt.len() < at + 4 {
            return None;
        }
        at += 4 + u16::from_be_bytes([pkt[at + 2], pkt[at + 3]]) as usize * 4;
    }
    (at <= pkt.len()).then_some(at)
}

/// ★**RTX 한 장을 짓는다**(RFC 4588) — ★**원본 seq 를 본문 앞 2바이트(OSN)에 싣고**
/// 머리는 재전송용 `ssrc`·`pt`·제 seq 를 쓴다.
///
/// ★**본문을 다시 마샬링하지 않는다** — 머리만 새로 쓰고 원본을 통째로 붙인다.
///
/// ★★**OSN 은 머리 뒤다 — 고정 12바이트 뒤가 아니다.** 확장을 단 패킷에 12바이트만 베끼고
/// 그 자리에 OSN 을 끼우면 ★**X 비트는 선 채로 확장 시작이 2바이트 밀려** 받는 쪽이 남의
/// 바이트를 확장으로 읽는다(우리가 egress 에 transport-cc 를 찍기 시작하면서 드러났다).
pub fn build_rtx(orig: &[u8], rtx_ssrc: u32, rtx_pt: u8, rtx_seq: u16) -> Option<Vec<u8>> {
    let head = head_len(orig)?;
    let mut out = Vec::with_capacity(orig.len() + 2);
    out.extend_from_slice(&orig[..head]);
    out[1] = (orig[1] & 0x80) | (rtx_pt & 0x7F);
    out[2..4].copy_from_slice(&rtx_seq.to_be_bytes());
    out[8..12].copy_from_slice(&rtx_ssrc.to_be_bytes());
    // ★OSN — 받는 쪽은 이 둘로 원래 자리를 안다.
    out.extend_from_slice(&orig[2..4]);
    out.extend_from_slice(&orig[head..]);
    Some(out)
}

/// PLI 한 장(RFC 4585 PSFB `fmt 1`) — 12바이트 고정.
pub fn build_pli(sender_ssrc: u32, media_ssrc: u32) -> [u8; 12] {
    let mut b = [0u8; 12];
    b[0] = 0x80 | 1; // V=2 · fmt=1
    b[1] = PT_PSFB;
    b[2..4].copy_from_slice(&2u16.to_be_bytes());
    b[4..8].copy_from_slice(&sender_ssrc.to_be_bytes());
    b[8..12].copy_from_slice(&media_ssrc.to_be_bytes());
    b
}

/// REMB 한 장(PSFB `fmt 15`, `draft-alvestrand-rmcat-remb`).
///
/// ★★**발행자에게 「이만큼까지 올려도 된다」고 말하는 자리**다(정§11-2) — 이것이 없으면
/// 발행자의 송신 추정이 갱신되지 않아 ★**화질이 안 올라간다.**
///
/// ★**값은 지수·가수로 실린다** — 18비트 가수로 못 담으면 지수를 올려 담는다.
/// ★`ssrcs` 는 그 값이 걸리는 스트림들이다(비어도 형식은 선다).
pub fn build_remb(sender_ssrc: u32, bitrate_bps: u64, ssrcs: &[u32]) -> Vec<u8> {
    let n = ssrcs.len().min(255);
    let total = 20 + 4 * n;
    let mut b = vec![0u8; total];
    b[0] = 0x80 | 15;
    b[1] = PT_PSFB;
    b[2..4].copy_from_slice(&((total / 4 - 1) as u16).to_be_bytes());
    b[4..8].copy_from_slice(&sender_ssrc.to_be_bytes());
    // ★media ssrc 는 `0` 이다 — 대상은 아래 목록이 말한다(RFC 4585 §6.1 의 REMB 관례).
    b[12..16].copy_from_slice(b"REMB");
    // 18비트에 들어갈 때까지 지수를 올린다.
    let (mut mantissa, mut exp) = (bitrate_bps, 0u32);
    while mantissa > 0x0003_FFFF {
        mantissa >>= 1;
        exp += 1;
    }
    b[16] = n as u8;
    b[17] = ((exp as u8) << 2) | ((mantissa >> 16) as u8 & 0x03);
    b[18] = (mantissa >> 8) as u8;
    b[19] = mantissa as u8;
    for (i, s) in ssrcs.iter().take(n).enumerate() {
        let o = 20 + i * 4;
        b[o..o + 4].copy_from_slice(&s.to_be_bytes());
    }
    b
}

/// RR 한 칸(RFC 3550 §6.4.1) — 24바이트.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReportBlock {
    pub ssrc: u32,
    pub fraction_lost: u8,
    pub cumulative_lost: u32,
    pub extended_highest_seq: u32,
    pub jitter: u32,
    pub last_sr: u32,
    pub delay_since_last_sr: u32,
}

/// RR 한 장(RFC 3550 §6.4.2). ★**칸이 없어도 보낸다** — *"받고 있다"* 는 것 자체가 신호다.
pub fn build_rr(sender_ssrc: u32, blocks: &[ReportBlock]) -> Vec<u8> {
    let rc = blocks.len().min(31);
    let total = 8 + 24 * rc;
    let mut b = vec![0u8; total];
    b[0] = 0x80 | rc as u8;
    b[1] = PT_RR;
    b[2..4].copy_from_slice(&((total / 4 - 1) as u16).to_be_bytes());
    b[4..8].copy_from_slice(&sender_ssrc.to_be_bytes());
    for (i, r) in blocks.iter().take(rc).enumerate() {
        let o = 8 + i * 24;
        b[o..o + 4].copy_from_slice(&r.ssrc.to_be_bytes());
        b[o + 4] = r.fraction_lost;
        let c = r.cumulative_lost & 0x00FF_FFFF;
        b[o + 5..o + 8].copy_from_slice(&c.to_be_bytes()[1..]);
        b[o + 8..o + 12].copy_from_slice(&r.extended_highest_seq.to_be_bytes());
        b[o + 12..o + 16].copy_from_slice(&r.jitter.to_be_bytes());
        b[o + 16..o + 20].copy_from_slice(&r.last_sr.to_be_bytes());
        b[o + 20..o + 24].copy_from_slice(&r.delay_since_last_sr.to_be_bytes());
    }
    b
}

/// ★RFC 3550 A.1 의 값 — 이 셋이 *"새 출처인가 심한 reorder 인가"* 를 가른다.
const MAX_DROPOUT: u16 = 3_000;
const MAX_MISORDER: u16 = 100;
const MIN_SEQUENTIAL: u32 = 2;

/// 한 발행 스트림의 수신 통계(RFC 3550 A.1·A.3·A.8).
///
/// ★**핫패스에서 갱신하고 1초 타이머가 소비한다** — 구간 델타를 내는 자리를 둘로 두면
/// 구간이 갈려 값이 틀린다(정§11-2 *"소비자는 타이머 하나"*).
#[derive(Debug, Clone)]
pub struct RecvStats {
    pub ssrc: u32,
    clock_rate: u32,
    max_seq: u16,
    cycles: u32,
    base_seq: u32,
    bad_seq: u32,
    received: u32,
    expected_prior: u32,
    received_prior: u32,
    jitter: f64,
    last_transit: Option<i64>,
    last_sr_middle: u32,
    last_sr_at: Option<u64>,
    started: bool,
    probation: u32,
}

impl RecvStats {
    pub fn new(ssrc: u32, clock_rate: u32) -> Self {
        Self {
            ssrc,
            clock_rate,
            max_seq: 0,
            cycles: 0,
            base_seq: 0,
            bad_seq: 0,
            received: 0,
            expected_prior: 0,
            received_prior: 0,
            jitter: 0.0,
            last_transit: None,
            last_sr_middle: 0,
            last_sr_at: None,
            started: false,
            probation: MIN_SEQUENTIAL,
        }
    }

    fn init_seq(&mut self, seq: u16) {
        self.base_seq = u32::from(seq);
        self.max_seq = seq;
        self.bad_seq = u32::from(seq).wrapping_add(1);
        self.cycles = 0;
        self.received = 0;
        self.received_prior = 0;
        self.expected_prior = 0;
    }

    /// 패킷 하나를 셌다. ★**핫패스다** — 할당이 없다.
    pub fn on_rtp(&mut self, seq: u16, rtp_ts: u32, now_ms: u64) {
        if !self.started {
            self.init_seq(seq);
            self.started = true;
            self.probation = MIN_SEQUENTIAL - 1;
            self.received += 1;
            self.update_jitter(rtp_ts, now_ms);
            return;
        }
        if self.probation > 0 {
            if seq == self.max_seq.wrapping_add(1) {
                self.probation -= 1;
                self.max_seq = seq;
                if self.probation == 0 {
                    self.init_seq(seq);
                    self.received += 1;
                    self.update_jitter(rtp_ts, now_ms);
                    return;
                }
            } else {
                self.probation = MIN_SEQUENTIAL - 1;
                self.max_seq = seq;
            }
            self.received += 1;
            return;
        }
        let delta = seq.wrapping_sub(self.max_seq);
        if delta < MAX_DROPOUT {
            if seq < self.max_seq {
                // ★감겼다 — 여기서 안 세면 확장 seq 가 65,536 마다 뒤로 간다.
                self.cycles += 65_536;
            }
            self.max_seq = seq;
        } else if delta <= 65_535 - MAX_MISORDER {
            // 큰 점프 — 새 출처이거나 심한 reorder.
            if u32::from(seq) == self.bad_seq {
                // 같은 자리에서 두 번 — 출처가 다시 시작한 것으로 본다.
                self.init_seq(seq);
            } else {
                self.bad_seq = u32::from(seq).wrapping_add(1);
                // ★이 패킷은 안 센다 — 세면 확장 seq 가 튀어 손실률이 거짓이 된다.
                return;
            }
        }
        self.received += 1;
        self.update_jitter(rtp_ts, now_ms);
    }

    /// RFC 3550 A.8 — `J += (|D| − J) / 16`.
    fn update_jitter(&mut self, rtp_ts: u32, now_ms: u64) {
        let arrival = (now_ms as i64) * i64::from(self.clock_rate) / 1_000;
        let transit = arrival - i64::from(rtp_ts);
        if let Some(prev) = self.last_transit {
            let d = (transit - prev).abs() as f64;
            self.jitter += (d - self.jitter) / 16.0;
        }
        // ★첫 패킷에는 견줄 것이 없다 — `0` 을 이전 값으로 쓰면 첫 지터가 통째로 거짓이 된다.
        self.last_transit = Some(transit);
    }

    /// 발행자 SR 을 받았다 — `LSR`·`DLSR` 의 재료다.
    pub fn on_sr(&mut self, ntp_hi: u32, ntp_lo: u32, now_ms: u64) {
        self.last_sr_middle = ((ntp_hi & 0xFFFF) << 16) | (ntp_lo >> 16);
        self.last_sr_at = Some(now_ms);
    }

    /// 지금까지를 한 칸으로. ★**부르면 구간이 닫힌다**(다음 구간이 시작된다).
    pub fn report(&mut self, now_ms: u64) -> ReportBlock {
        let extended = self.cycles + u32::from(self.max_seq);
        let expected = extended.wrapping_sub(self.base_seq).wrapping_add(1);
        let lost = expected.saturating_sub(self.received);
        let exp_iv = expected.wrapping_sub(self.expected_prior);
        let rcv_iv = self.received.wrapping_sub(self.received_prior);
        let lost_iv = exp_iv.saturating_sub(rcv_iv);
        let fraction =
            if exp_iv > 0 { ((u64::from(lost_iv) * 256) / u64::from(exp_iv)) as u8 } else { 0 };
        self.expected_prior = expected;
        self.received_prior = self.received;
        ReportBlock {
            ssrc: self.ssrc,
            fraction_lost: fraction,
            // ★24비트다 — 넘치면 자른다(음수로 감기면 발행자가 손실을 거꾸로 읽는다).
            cumulative_lost: lost.min(0x7F_FFFF),
            extended_highest_seq: extended,
            jitter: self.jitter as u32,
            last_sr: self.last_sr_middle,
            // 1/65536초 단위.
            delay_since_last_sr: self
                .last_sr_at
                .map(|t| ((now_ms.saturating_sub(t)) * 65_536 / 1_000) as u32)
                .unwrap_or(0),
        }
    }

    pub fn received(&self) -> u32 {
        self.received
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flowing(n: u16) -> RecvStats {
        let mut s = RecvStats::new(0x1234, 48_000);
        for i in 0..n {
            s.on_rtp(i, u32::from(i) * 960, u64::from(i) * 20);
        }
        s
    }

    #[test]
    fn rtp_와_rtcp_는_둘째_바이트가_가른다() {
        // RTP — PT 111(opus).
        assert!(!is_rtcp(&[0x80, 111]));
        assert!(is_rtcp(&[0x80, PT_SR]));
        assert!(is_rtcp(&[0x81, PT_RR]));
        assert!(is_rtcp(&[0x8F, PT_PSFB]));
        // ★경계 밖 — 207 은 우리가 안 읽는다(SDES 뒤의 확장).
        assert!(!is_rtcp(&[0x80, 207]));
        assert!(!is_rtcp(&[0x80]));
    }

    #[test]
    fn 덩어리를_패킷으로_나눈다() {
        let mut c = build_rr(1, &[ReportBlock::default()]);
        c.extend_from_slice(&build_pli(1, 2));
        let parts = split(&c);
        assert_eq!(parts.len(), 2);
        assert_eq!((parts[0][1], parts[1][1]), (PT_RR, PT_PSFB));
        // ★길이 칸이 거짓이면 그 뒤는 안 읽는다.
        let mut bad = c.clone();
        bad[3] = 0xFF;
        assert!(split(&bad).is_empty());
    }

    #[test]
    fn nack_은_fmt_로_갈린다() {
        let mut n = vec![0x81, PT_RTPFB, 0, 3];
        n.extend_from_slice(&1u32.to_be_bytes());
        n.extend_from_slice(&0xAAAAu32.to_be_bytes());
        n.extend_from_slice(&43u16.to_be_bytes());
        // BLP 비트 0·2 → 44·46 도 빠졌다.
        n.extend_from_slice(&0b101u16.to_be_bytes());
        assert_eq!(nack_seqs(&n), Some((0xAAAA, vec![43, 44, 46])));
        // ★`fmt 15` 는 TWCC 다 — NACK 으로 읽으면 없는 손실에 재전송을 쏜다.
        let mut twcc = n.clone();
        twcc[0] = 0x80 | 15;
        assert_eq!(nack_seqs(&twcc), None);
    }

    #[test]
    fn rtx_는_osn_을_앞에_싣고_본문을_안_건드린다() {
        let mut orig = vec![0x80, 96, 0, 43, 0, 0, 0, 9];
        orig.extend_from_slice(&0x1111_1111u32.to_be_bytes());
        orig.extend_from_slice(b"frame");
        let out = build_rtx(&orig, 0x2222_2222, 97, 5).expect("짓는다");
        assert_eq!(out[1] & 0x7F, 97);
        assert_eq!(u16::from_be_bytes([out[2], out[3]]), 5, "제 seq 를 쓴다");
        assert_eq!(&out[8..12], &0x2222_2222u32.to_be_bytes());
        // ★OSN = 원본 seq 43.
        assert_eq!(u16::from_be_bytes([out[12], out[13]]), 43);
        assert_eq!(&out[14..], b"frame", "★본문 무접촉");
        assert_eq!(out.len(), orig.len() + 2, "★2바이트만 는다");
    }

    #[test]
    fn rtx_는_확장을_단_것도_자리를_안_흔든다() {
        // ★egress 에 transport-cc 를 찍기 시작하면서 드러난 자리다 — 12바이트 뒤에 OSN 을
        //   끼우면 X 비트는 선 채로 확장 시작이 2바이트 밀린다.
        let mut orig = vec![0x90, 96, 0, 43, 0, 0, 0, 9];
        orig.extend_from_slice(&0x1111_1111u32.to_be_bytes());
        orig.extend_from_slice(&0xBEDEu16.to_be_bytes());
        orig.extend_from_slice(&1u16.to_be_bytes());
        orig.extend_from_slice(&[(6 << 4) | 1, 0x12, 0x34, 0]);
        orig.extend_from_slice(b"frame");
        let out = build_rtx(&orig, 0x2222_2222, 97, 5).expect("짓는다");
        // ★확장은 제자리 그대로다.
        assert_eq!(&out[12..20], &orig[12..20]);
        assert_eq!(crate::rtpext::get(&out, 6), Some(&[0x12, 0x34][..]));
        // ★OSN 은 머리(확장 포함) 뒤다.
        assert_eq!(u16::from_be_bytes([out[20], out[21]]), 43);
        assert_eq!(&out[22..], b"frame", "★본문 무접촉");
        assert_eq!(out.len(), orig.len() + 2);
    }

    #[test]
    fn 머리가_거짓이면_안_짓는다() {
        // ★확장 길이가 버퍼를 넘으면 자리를 셀 수 없다 — 지어내지 않는다.
        let bad = vec![0x90, 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 7, 0xBE, 0xDE, 0xFF, 0xFF];
        assert!(build_rtx(&bad, 1, 97, 1).is_none());
        assert!(build_rtx(&[0u8; 8], 1, 97, 1).is_none());
    }

    /// REMB 의 값을 도로 읽는다 — 시험이 제 형식을 스스로 견준다.
    fn remb_bps(p: &[u8]) -> u64 {
        let exp = (p[17] >> 2) as u32;
        let mantissa = (((p[17] & 0x03) as u64) << 16) | ((p[18] as u64) << 8) | p[19] as u64;
        mantissa << exp
    }

    #[test]
    fn remb_는_지수_가수로_담는다() {
        let w = build_remb(1, 300_000, &[0xAABB_CCDD]);
        assert_eq!((w[0] & 0x1F, w[1]), (15, PT_PSFB));
        assert_eq!(&w[12..16], b"REMB");
        assert_eq!(w[16], 1, "대상 수");
        assert_eq!(u32::from_be_bytes([w[20], w[21], w[22], w[23]]), 0xAABB_CCDD);
        assert_eq!(remb_bps(&w), 300_000, "★18비트에 그대로 들어간다");
        assert_eq!(u16::from_be_bytes([w[2], w[3]]) as usize, w.len() / 4 - 1);
    }

    #[test]
    fn remb_는_큰_값도_담는다() {
        // ★18비트(262,143)를 넘으면 지수를 올린다 — 안 올리면 값이 잘린다.
        let w = build_remb(1, 5_000_000, &[]);
        let got = remb_bps(&w);
        assert!(got > 4_900_000 && got <= 5_000_000, "{got}");
        assert_eq!(w[16], 0, "대상이 없어도 형식은 선다");
        assert_eq!(w.len(), 20);
    }

    #[test]
    fn rr_은_칸이_없어도_선다() {
        let w = build_rr(7, &[]);
        assert_eq!((w[0] & 0x1F, w[1], w.len()), (0, PT_RR, 8));
        assert_eq!(u16::from_be_bytes([w[2], w[3]]) as usize, w.len() / 4 - 1);
    }

    /// ★**끊김 없이 오면 끝까지 손실 0 이어야 한다** — probation 구간을 포함해서다.
    /// 2층 `rr_uplink_clean` 이 보는 것이 바로 이 값이고, ★**누적은 한 번 오르면 안 내려간다.**
    #[test]
    fn 끊김이_없으면_누적_손실이_0_이다() {
        let mut s = RecvStats::new(1, 48_000);
        for i in 0..500u16 {
            s.on_rtp(i, u32::from(i) * 960, u64::from(i) * 20);
            if i.is_multiple_of(50) {
                // 1초 주기로 RR 을 낸다 — 구간이 닫혀도 누적이 오르면 안 된다.
                let b = s.report(u64::from(i) * 20);
                assert_eq!((b.fraction_lost, b.cumulative_lost), (0, 0), "seq={i} {b:?}");
            }
        }
        let b = s.report(10_000);
        assert_eq!((b.fraction_lost, b.cumulative_lost), (0, 0), "{b:?}");
    }

    /// ★**같은 `ssrc` 로 다시 시작해도 손실이 안 쌓인다**(재발행 형상).
    ///
    /// RFC 3550 A.1 은 큰 점프를 두 번 연속 보면 출처 재시작으로 보고 `init_seq` 한다 —
    /// ★**그 자리가 없으면 재발행 한 번에 누적 손실이 수백으로 뛰고 영영 안 내려간다.**
    #[test]
    fn 같은_ssrc_로_다시_시작해도_손실이_안_쌓인다() {
        let mut s = RecvStats::new(1, 48_000);
        for i in 0..500u16 {
            s.on_rtp(i, u32::from(i) * 960, u64::from(i) * 20);
        }
        s.report(10_000);
        // 재발행 — 같은 ssrc, seq 는 0 부터 다시.
        for i in 0..200u16 {
            s.on_rtp(i, u32::from(i) * 960, 10_000 + u64::from(i) * 20);
        }
        let b = s.report(20_000);
        assert_eq!((b.fraction_lost, b.cumulative_lost), (0, 0), "{b:?}");
    }

    #[test]
    fn 손실은_구간마다_다시_센다() {
        let mut s = flowing(100);
        let a = s.report(2_000);
        assert_eq!(a.fraction_lost, 0, "다 받았다");
        // 한 장 건너뛴다 — 다음 구간에만 잡혀야 한다.
        s.on_rtp(101, 101 * 960, 2_020);
        let b = s.report(2_040);
        assert!(b.fraction_lost > 0, "{b:?}");
        let c = s.report(2_060);
        assert_eq!(c.fraction_lost, 0, "★구간이 닫혔다 — 같은 손실을 두 번 세지 않는다");
        assert_eq!(c.cumulative_lost, 1, "누적은 남는다");
    }

    #[test]
    fn seq_가_감기면_확장값이_이어진다() {
        let mut s = RecvStats::new(1, 48_000);
        for q in [65_530u16, 65_531, 65_532] {
            s.on_rtp(q, 0, 0);
        }
        let before = s.report(0).extended_highest_seq;
        for q in [65_533u16, 65_534, 65_535, 0, 1] {
            s.on_rtp(q, 0, 0);
        }
        let after = s.report(0).extended_highest_seq;
        // ★안 세면 여기서 뒤로 간다 — 발행자가 손실을 거꾸로 읽는다.
        assert!(after > before, "{before} → {after}");
    }

    #[test]
    fn sr_은_ntp_를_안_건드리고_수를_갈아_끼운다() {
        let mut sr = vec![0x80, PT_SR, 0, 6];
        sr.extend_from_slice(&0xAAAA_AAAAu32.to_be_bytes()); // ssrc
        sr.extend_from_slice(&0x1111_1111u32.to_be_bytes()); // ntp hi
        sr.extend_from_slice(&0x2222_2222u32.to_be_bytes()); // ntp lo
        sr.extend_from_slice(&0x3333_3333u32.to_be_bytes()); // rtp ts
        sr.extend_from_slice(&10u32.to_be_bytes()); // packets
        sr.extend_from_slice(&20u32.to_be_bytes()); // octets

        assert_eq!(sr_ntp(&sr), Some((0xAAAA_AAAA, 0x1111_1111, 0x2222_2222)));
        let out = translate_sr(
            &sr,
            &SrPatch {
                ssrc: Some(0xBBBB_BBBB),
                rtp_ts: Some(0x4444_4444),
                packet_count: 99,
                octet_count: 88,
            },
        )
        .expect("번역");
        assert_eq!(&out[4..8], &0xBBBB_BBBBu32.to_be_bytes());
        // ★**NTP 는 그대로다** — 서버 시계를 실으면 수신 jb 가 폭등한다.
        assert_eq!(&out[8..16], &sr[8..16]);
        assert_eq!(&out[16..20], &0x4444_4444u32.to_be_bytes());
        assert_eq!(&out[20..24], &99u32.to_be_bytes());
        assert_eq!(out.len(), sr.len(), "★길이 불변");
    }

    #[test]
    fn 첫_패킷에는_지터가_없다() {
        let mut s = RecvStats::new(1, 48_000);
        s.on_rtp(0, 0, 1_000_000);
        // ★이전 transit 을 `0` 으로 두면 여기서 거대한 지터가 나온다.
        assert_eq!(s.report(0).jitter, 0);
    }
}
