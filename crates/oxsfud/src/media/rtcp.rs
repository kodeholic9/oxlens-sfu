// author: kodeholic (powered by Claude)
//! RTCP — 정§11-2. 서버는 relay 가 아니라 ★두 세션의 종단이다.
//! 이 파일은 wire 조각만 다룬다(RFC 3550 §6 · RFC 4585 §6): 복합 패킷 분해 · SR 읽기와 카운터 교체 ·
//! RR 조립 · 피드백(PLI·NACK) 식별. 무엇을 언제 보낼지는 `transport::udp` 와 `handlers` 가 정한다.

pub const PT_SR: u8 = 200;
pub const PT_RR: u8 = 201;
pub const PT_SDES: u8 = 202;
pub const PT_BYE: u8 = 203;
pub const PT_APP: u8 = 204;
/// RFC 4585 — 전송 계층 피드백(NACK = fmt 1 · TWCC = fmt 15).
pub const PT_RTPFB: u8 = 205;
/// RFC 4585 — 페이로드 계층 피드백(PLI = fmt 1 · REMB = fmt 15).
pub const PT_PSFB: u8 = 206;

pub const FMT_NACK: u8 = 1;
/// draft-holmer-rmcat-transport-wide-cc-extensions-01 — 전송로 단위 피드백.
pub const FMT_TWCC: u8 = 15;
/// draft-alvestrand-rmcat-remb — PSFB 안의 자리다(같은 15 지만 PT 가 다르다).
pub const FMT_REMB: u8 = 15;
pub const FMT_PLI: u8 = 1;

const HEADER_LEN: usize = 4;
/// SR 은 헤더 뒤 sender SSRC(4) + sender info(20).
const SR_INFO_LEN: usize = 24;
pub const REPORT_BLOCK_LEN: usize = 24;

/// 복합 패킷 하나를 조각으로 — ★미해소는 부르는 쪽이 계수하고 버린다(조용한 drop 금지).
pub fn packets(compound: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + HEADER_LEN <= compound.len() {
        if compound[at] >> 6 != 2 {
            break;
        }
        let words = u16::from_be_bytes([compound[at + 2], compound[at + 3]]) as usize;
        let end = at + (words + 1) * 4;
        if end > compound.len() {
            break;
        }
        out.push(&compound[at..end]);
        at = end;
    }
    out
}

pub fn payload_type(pkt: &[u8]) -> Option<u8> {
    pkt.get(1).copied()
}

/// 피드백의 fmt(하위 5비트) — PT_RTPFB·PT_PSFB 에서만 뜻이 있다.
pub fn fmt(pkt: &[u8]) -> Option<u8> {
    Some(pkt.first()? & 0x1F)
}

/// 보낸 쪽 SSRC — 모든 종류의 첫 4바이트.
pub fn sender_ssrc(pkt: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(pkt.get(4..8)?.try_into().ok()?))
}

/// 피드백이 가리키는 미디어 SSRC(RFC 4585 §6.1).
pub fn media_ssrc(pkt: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(pkt.get(8..12)?.try_into().ok()?))
}

pub fn is_pli(pkt: &[u8]) -> bool {
    payload_type(pkt) == Some(PT_PSFB) && fmt(pkt) == Some(FMT_PLI)
}

pub fn is_nack(pkt: &[u8]) -> bool {
    payload_type(pkt) == Some(PT_RTPFB) && fmt(pkt) == Some(FMT_NACK)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SenderInfo {
    pub ssrc: u32,
    /// ★NTP 는 항상 원본을 유지한다 — lip sync 의 기준점이다(서버 시계를 실으면 수신 jb 가 폭등한다).
    pub ntp: u64,
    pub rtp_ts: u32,
    pub packets: u32,
    pub octets: u32,
}

pub fn sender_info(pkt: &[u8]) -> Option<SenderInfo> {
    if payload_type(pkt) != Some(PT_SR) || pkt.len() < HEADER_LEN + SR_INFO_LEN {
        return None;
    }
    let at = HEADER_LEN + 4;
    Some(SenderInfo {
        ssrc: sender_ssrc(pkt)?,
        ntp: u64::from_be_bytes(pkt.get(at..at + 8)?.try_into().ok()?),
        rtp_ts: u32::from_be_bytes(pkt.get(at + 8..at + 12)?.try_into().ok()?),
        packets: u32::from_be_bytes(pkt.get(at + 12..at + 16)?.try_into().ok()?),
        octets: u32::from_be_bytes(pkt.get(at + 16..at + 20)?.try_into().ok()?),
    })
}

/// 정§11-2 — SR 은 ★자체 생성 금지, 번역 릴레이다. SSRC·RTP ts·카운터만 egress 값으로 갈고
/// ★NTP 는 그대로 둔다. 길이 불변이라 뒤따르는 SDES 조각도 살아 있다.
pub fn translate_sr(pkt: &mut [u8], ssrc: u32, rtp_ts: u32, packets: u32, octets: u32) -> bool {
    if payload_type(pkt) != Some(PT_SR) || pkt.len() < HEADER_LEN + SR_INFO_LEN {
        return false;
    }
    pkt[4..8].copy_from_slice(&ssrc.to_be_bytes());
    let at = HEADER_LEN + 4;
    pkt[at + 8..at + 12].copy_from_slice(&rtp_ts.to_be_bytes());
    pkt[at + 12..at + 16].copy_from_slice(&packets.to_be_bytes());
    pkt[at + 16..at + 20].copy_from_slice(&octets.to_be_bytes());
    true
}

/// RFC 3550 §6.4.1 report block — 발행자가 보는 "우리 수신 품질".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportBlock {
    pub ssrc: u32,
    pub fraction_lost: u8,
    pub cumulative_lost: u32,
    pub highest_seq: u32,
    pub jitter: u32,
    pub last_sr: u32,
    pub delay_since_last_sr: u32,
}

impl ReportBlock {
    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.ssrc.to_be_bytes());
        // cumulative lost 는 24비트 부호값이라 상위 바이트를 fraction 과 나눠 쓴다.
        out.push(self.fraction_lost);
        out.extend_from_slice(&self.cumulative_lost.to_be_bytes()[1..]);
        out.extend_from_slice(&self.highest_seq.to_be_bytes());
        out.extend_from_slice(&self.jitter.to_be_bytes());
        out.extend_from_slice(&self.last_sr.to_be_bytes());
        out.extend_from_slice(&self.delay_since_last_sr.to_be_bytes());
    }
}

pub fn read_report_blocks(pkt: &[u8]) -> Vec<ReportBlock> {
    let count = usize::from(pkt.first().copied().unwrap_or(0) & 0x1F);
    let start = match payload_type(pkt) {
        Some(PT_SR) => HEADER_LEN + SR_INFO_LEN,
        Some(PT_RR) => HEADER_LEN + 4,
        _ => return Vec::new(),
    };
    (0..count)
        .filter_map(|i| {
            let at = start + i * REPORT_BLOCK_LEN;
            let b = pkt.get(at..at + REPORT_BLOCK_LEN)?;
            Some(ReportBlock {
                ssrc: u32::from_be_bytes(b[0..4].try_into().ok()?),
                fraction_lost: b[4],
                cumulative_lost: u32::from_be_bytes([0, b[5], b[6], b[7]]),
                highest_seq: u32::from_be_bytes(b[8..12].try_into().ok()?),
                jitter: u32::from_be_bytes(b[12..16].try_into().ok()?),
                last_sr: u32::from_be_bytes(b[16..20].try_into().ok()?),
                delay_since_last_sr: u32::from_be_bytes(b[20..24].try_into().ok()?),
            })
        })
        .collect()
}

/// 정§11-2 — Ingress RR 은 서버가 ★자체 생성한다(1,000ms 주기).
pub fn build_rr(sender: u32, blocks: &[ReportBlock]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + 4 + blocks.len() * REPORT_BLOCK_LEN);
    out.push(0x80 | (blocks.len() as u8 & 0x1F));
    out.push(PT_RR);
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&sender.to_be_bytes());
    for b in blocks {
        b.write(&mut out);
    }
    set_length(&mut out);
    out
}

/// RFC 4585 §6.3.1 — 키프레임 요청. 값이 없는 피드백이라 길이가 고정이다.
pub fn build_pli(sender: u32, media: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    out.push(0x80 | FMT_PLI);
    out.push(PT_PSFB);
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&sender.to_be_bytes());
    out.extend_from_slice(&media.to_be_bytes());
    set_length(&mut out);
    out
}

/// RFC 4585 §6.2.1 Generic NACK — 묶음(PID·BLP)마다 4바이트다.
/// ★묶기는 `nack::pack` 이 한다 — 여기는 실어 나르기만 해서 두 곳이 같은 판단을 안 한다.
pub fn build_nack(sender: u32, media: u32, pairs: &[(u16, u16)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + pairs.len() * 4);
    out.push(0x80 | FMT_NACK);
    out.push(PT_RTPFB);
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&sender.to_be_bytes());
    out.extend_from_slice(&media.to_be_bytes());
    for (pid, blp) in pairs {
        out.extend_from_slice(&pid.to_be_bytes());
        out.extend_from_slice(&blp.to_be_bytes());
    }
    set_length(&mut out);
    out
}

/// 받은 Generic NACK 에서 요구된 seq 를 편다 — 하향(구독자 NACK) 응답의 입력이다.
pub fn read_nack(pkt: &[u8]) -> Vec<u16> {
    if !is_nack(pkt) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut at = 12;
    while at + 4 <= pkt.len() {
        let pid = u16::from_be_bytes([pkt[at], pkt[at + 1]]);
        let blp = u16::from_be_bytes([pkt[at + 2], pkt[at + 3]]);
        out.push(pid);
        for bit in 0..16 {
            if blp & (1 << bit) != 0 {
                out.push(pid.wrapping_add(bit + 1));
            }
        }
        at += 4;
    }
    out
}

/// draft-alvestrand-rmcat-remb — 발행자에게 "이만큼까지 보내라" 를 알린다.
/// ★값은 지수·가수로 실린다(6비트 지수 + 18비트 가수).
pub fn build_remb(sender: u32, ssrcs: &[u32], bps: u64) -> Vec<u8> {
    let (exp, mantissa) = split_bitrate(bps);
    let mut out = Vec::with_capacity(20 + ssrcs.len() * 4);
    out.push(0x80 | FMT_REMB);
    out.push(PT_PSFB);
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&sender.to_be_bytes());
    // media ssrc 는 0 이다 — 대상은 아래 목록이 정한다.
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"REMB");
    out.push(ssrcs.len() as u8);
    out.push((exp << 2) | ((mantissa >> 16) & 0x03) as u8);
    out.push(((mantissa >> 8) & 0xFF) as u8);
    out.push((mantissa & 0xFF) as u8);
    for ssrc in ssrcs {
        out.extend_from_slice(&ssrc.to_be_bytes());
    }
    set_length(&mut out);
    out
}

/// 가수가 18비트에 들어갈 때까지 지수를 올린다.
fn split_bitrate(bps: u64) -> (u8, u32) {
    let mut exp = 0u8;
    let mut mantissa = bps;
    while mantissa > 0x0003_FFFF && exp < 63 {
        mantissa >>= 1;
        exp += 1;
    }
    (exp, mantissa as u32)
}

fn set_length(out: &mut [u8]) {
    let words = (out.len() / 4 - 1) as u16;
    out[2..4].copy_from_slice(&words.to_be_bytes());
}

/// RFC 3550 — NTP 64비트 중 가운데 32비트(RR 의 `last_sr`).
pub fn ntp_middle(ntp: u64) -> u32 {
    ((ntp >> 16) & 0xFFFF_FFFF) as u32
}

/// 1/65536 초 단위 — RR 의 `delay_since_last_sr`.
pub fn delay_units(ms: u64) -> u32 {
    u32::try_from(ms.saturating_mul(65_536) / 1_000).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sr(ssrc: u32, packets: u32) -> Vec<u8> {
        let mut p = vec![0x80, PT_SR, 0, 0];
        p.extend_from_slice(&ssrc.to_be_bytes());
        p.extend_from_slice(&0x1234_5678_9ABC_DEF0u64.to_be_bytes());
        p.extend_from_slice(&1_000u32.to_be_bytes());
        p.extend_from_slice(&packets.to_be_bytes());
        p.extend_from_slice(&(packets * 160).to_be_bytes());
        set_length(&mut p);
        p
    }

    #[test]
    fn compound_packets_split_and_a_truncated_tail_is_dropped() {
        let mut compound = sr(0xAAAA, 10);
        compound.extend_from_slice(&build_pli(1, 2));
        compound.extend_from_slice(&[0x81, PT_BYE, 0, 1, 0, 0, 0, 9]);
        let parts = packets(&compound);
        assert_eq!(parts.iter().filter_map(|p| payload_type(p)).collect::<Vec<_>>(), vec![PT_SR, PT_PSFB, PT_BYE]);
        compound.truncate(compound.len() - 2);
        assert_eq!(packets(&compound).len(), 2, "잘린 꼬리는 버린다");
        assert!(packets(&[0x00, 0, 0, 0]).is_empty(), "버전이 2 가 아니면 RTCP 가 아니다");
    }

    #[test]
    fn sender_report_translation_keeps_the_ntp_and_the_length() {
        let mut p = sr(0xAAAA, 10);
        let before = (p.len(), sender_info(&p).unwrap());
        assert!(translate_sr(&mut p, 0xBBBB, 777, 5, 800));
        let after = sender_info(&p).unwrap();
        assert_eq!(p.len(), before.0, "길이 불변 — 뒤따르는 조각이 살아야 한다");
        assert_eq!((after.ssrc, after.rtp_ts, after.packets, after.octets), (0xBBBB, 777, 5, 800));
        assert_eq!(after.ntp, before.1.ntp, "★NTP 는 원본 — 서버 시계를 실으면 수신 jb 가 폭등한다");
        assert!(!translate_sr(&mut build_pli(1, 2), 1, 1, 1, 1), "SR 이 아니면 손대지 않는다");
    }

    #[test]
    fn receiver_report_round_trips() {
        let blocks = [
            ReportBlock { ssrc: 0x1111, fraction_lost: 12, cumulative_lost: 0x0012_3456, highest_seq: 70_000, jitter: 33, last_sr: 0xDEAD, delay_since_last_sr: 65_536 },
            ReportBlock { ssrc: 0x2222, fraction_lost: 0, cumulative_lost: 0, highest_seq: 5, jitter: 0, last_sr: 0, delay_since_last_sr: 0 },
        ];
        let rr = build_rr(0x9999, &blocks);
        assert_eq!((payload_type(&rr), sender_ssrc(&rr), rr[0] & 0x1F), (Some(PT_RR), Some(0x9999), 2));
        assert_eq!(rr.len() % 4, 0);
        assert_eq!(u16::from_be_bytes([rr[2], rr[3]]) as usize, rr.len() / 4 - 1);
        assert_eq!(read_report_blocks(&rr), blocks.to_vec());
    }

    #[test]
    fn feedback_is_identified_by_type_and_format() {
        let pli = build_pli(0x9999, 0x1111);
        assert!(is_pli(&pli) && !is_nack(&pli));
        assert_eq!((sender_ssrc(&pli), media_ssrc(&pli), pli.len()), (Some(0x9999), Some(0x1111), 12));
        let nack = [0x81, PT_RTPFB, 0, 3, 0, 0, 0, 9, 0, 0, 0, 1, 0, 10, 0, 0];
        assert!(is_nack(&nack) && !is_pli(&nack));
        let built = build_nack(1, 42, &[(12, 0b11), (99, 0)]);
        assert!(is_nack(&built) && media_ssrc(&built) == Some(42) && sender_ssrc(&built) == Some(1));
        assert_eq!(read_nack(&built), vec![12, 13, 14, 99], "실은 것과 편 것이 같다");
        assert_eq!(read_nack(&build_pli(1, 42)), Vec::<u16>::new(), "PLI 는 NACK 이 아니다");

        let remb = build_remb(1, &[7, 9], 800_000);
        assert_eq!((payload_type(&remb), fmt(&remb)), (Some(PT_PSFB), Some(FMT_REMB)));
        assert_eq!(&remb[12..16], b"REMB");
        assert_eq!(remb[16], 2, "대상 ssrc 개수");
        let exp = u32::from(remb[17] >> 2);
        let mantissa = (u32::from(remb[17] & 0x03) << 16) | (u32::from(remb[18]) << 8) | u32::from(remb[19]);
        assert_eq!((mantissa as u64) << exp, 800_000, "지수·가수가 원래 값을 낸다");
        assert_eq!(&remb[20..24], &7u32.to_be_bytes());
        assert_eq!(remb.len() % 4, 0);
        assert_eq!(media_ssrc(&nack), Some(1));
        assert_eq!(ntp_middle(0x1234_5678_9ABC_DEF0), 0x5678_9ABC);
        assert_eq!((delay_units(1_000), delay_units(0)), (65_536, 0));
    }
}
