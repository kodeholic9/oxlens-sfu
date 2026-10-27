// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-RTCP-TERMINATE · 정:CAND-RTCP-SENDER-SSRC · 정:CAND-RTCP-RR-PER-STREAM · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! RTCP 패킷(순수) — 복합 분해(SR · RR · SDES · BYE · APP · RTPFB(NACK · TWCC) · PSFB(PLI · FIR · REMB)) · 짓기(RR 블록 하나 · REMB · NACK · PLI · SR 번역).

pub const SENDER_SSRC: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportBlock {
    pub ssrc: u32,
    pub fraction: u8,
    pub lost: i32,
    pub ext_max: u32,
    pub jitter: u32,
    pub lsr: u32,
    pub dlsr: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    Sr { ssrc: u32, ntp: u64, rtp_ts: u32, packets: u32, octets: u32, blocks: Vec<ReportBlock> },
    Rr { ssrc: u32, blocks: Vec<ReportBlock> },
    Nack { media: u32, seqs: Vec<u16> },
    Twcc { media: u32, raw: Vec<u8> },
    Pli { media: u32 },
    Fir { media: Vec<u32> },
    Remb { bitrate: u64, ssrcs: Vec<u32> },
    Ignored(u8),
}

fn blocks(b: &[u8], n: usize) -> Vec<ReportBlock> {
    let mut v = Vec::new();
    for i in 0..n {
        let o = i * 24;
        if b.len() < o + 24 {
            break;
        }
        let x = &b[o..o + 24];
        let lost_raw = u32::from_be_bytes([0, x[5], x[6], x[7]]);
        let lost = if lost_raw & 0x80_0000 != 0 { (lost_raw | 0xff00_0000) as i32 } else { lost_raw as i32 };
        v.push(ReportBlock {
            ssrc: u32::from_be_bytes([x[0], x[1], x[2], x[3]]),
            fraction: x[4],
            lost,
            ext_max: u32::from_be_bytes([x[8], x[9], x[10], x[11]]),
            jitter: u32::from_be_bytes([x[12], x[13], x[14], x[15]]),
            lsr: u32::from_be_bytes([x[16], x[17], x[18], x[19]]),
            dlsr: u32::from_be_bytes([x[20], x[21], x[22], x[23]]),
        });
    }
    v
}

fn u32at(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn parse(b: &[u8]) -> Vec<Packet> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= b.len() {
        let count = usize::from(b[i] & 0x1f);
        let pt = b[i + 1];
        let len = (usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]])) + 1) * 4;
        if b[i] >> 6 != 2 || i + len > b.len() {
            break;
        }
        let p = &b[i..i + len];
        match pt {
            200 if p.len() >= 28 => out.push(Packet::Sr {
                ssrc: u32at(p, 4),
                ntp: (u64::from(u32at(p, 8)) << 32) | u64::from(u32at(p, 12)),
                rtp_ts: u32at(p, 16),
                packets: u32at(p, 20),
                octets: u32at(p, 24),
                blocks: blocks(&p[28..], count),
            }),
            201 if p.len() >= 8 => out.push(Packet::Rr { ssrc: u32at(p, 4), blocks: blocks(&p[8..], count) }),
            205 if p.len() >= 12 => {
                let media = u32at(p, 8);
                match count {
                    1 => {
                        let mut seqs = Vec::new();
                        let mut j = 12;
                        while j + 4 <= p.len() {
                            let pid = u16::from_be_bytes([p[j], p[j + 1]]);
                            let blp = u16::from_be_bytes([p[j + 2], p[j + 3]]);
                            seqs.push(pid);
                            for k in 0..16 {
                                if blp & (1 << k) != 0 {
                                    seqs.push(pid.wrapping_add(k + 1));
                                }
                            }
                            j += 4;
                        }
                        out.push(Packet::Nack { media, seqs });
                    }
                    15 => out.push(Packet::Twcc { media, raw: p.to_vec() }),
                    _ => out.push(Packet::Ignored(pt)),
                }
            }
            206 if p.len() >= 12 => {
                let media = u32at(p, 8);
                match count {
                    1 => out.push(Packet::Pli { media }),
                    4 => {
                        let mut v = Vec::new();
                        let mut j = 12;
                        while j + 8 <= p.len() {
                            v.push(u32at(p, j));
                            j += 8;
                        }
                        out.push(Packet::Fir { media: v });
                    }
                    15 if p.len() >= 20 && &p[12..16] == b"REMB" => {
                        let n = usize::from(p[16]);
                        let exp = u32::from(p[17] >> 2);
                        let mant = (u64::from(p[17] & 0x03) << 16) | (u64::from(p[18]) << 8) | u64::from(p[19]);
                        let bitrate = mant << exp;
                        let ssrcs = (0..n).filter(|k| 20 + k * 4 + 4 <= p.len()).map(|k| u32at(p, 20 + k * 4)).collect();
                        out.push(Packet::Remb { bitrate, ssrcs });
                    }
                    _ => out.push(Packet::Ignored(pt)),
                }
            }
            _ => out.push(Packet::Ignored(pt)),
        }
        i += len;
    }
    out
}

fn head(count: u8, pt: u8, words: usize) -> Vec<u8> {
    let mut v = vec![0x80 | (count & 0x1f), pt];
    v.extend_from_slice(&((words as u16).to_be_bytes()));
    v
}

pub fn rr(media: &ReportBlock) -> Vec<u8> {
    let mut v = head(1, 201, 7);
    v.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    v.extend_from_slice(&media.ssrc.to_be_bytes());
    let lost = (media.lost.clamp(-0x80_0000, 0x7f_ffff) as u32) & 0x00ff_ffff;
    v.push(media.fraction);
    v.extend_from_slice(&lost.to_be_bytes()[1..]);
    v.extend_from_slice(&media.ext_max.to_be_bytes());
    v.extend_from_slice(&media.jitter.to_be_bytes());
    v.extend_from_slice(&media.lsr.to_be_bytes());
    v.extend_from_slice(&media.dlsr.to_be_bytes());
    v
}

pub fn remb(bitrate: u64, ssrcs: &[u32]) -> Vec<u8> {
    let mut exp = 0u8;
    let mut mant = bitrate;
    while mant > 0x3_ffff {
        mant >>= 1;
        exp += 1;
    }
    let words = 4 + ssrcs.len();
    let mut v = head(15, 206, words);
    v.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(b"REMB");
    v.push(ssrcs.len() as u8);
    v.push((exp << 2) | ((mant >> 16) as u8 & 0x03));
    v.push((mant >> 8) as u8);
    v.push(mant as u8);
    for s in ssrcs {
        v.extend_from_slice(&s.to_be_bytes());
    }
    v
}

pub fn pli(media: u32) -> Vec<u8> {
    let mut v = head(1, 206, 2);
    v.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    v.extend_from_slice(&media.to_be_bytes());
    v
}

pub fn nack(media: u32, seqs: &[u16]) -> Vec<u8> {
    let mut pairs: Vec<(u16, u16)> = Vec::new();
    let mut sorted = seqs.to_vec();
    sorted.sort_unstable();
    for s in sorted {
        match pairs.last_mut() {
            Some((pid, blp)) if s.wrapping_sub(*pid) >= 1 && s.wrapping_sub(*pid) <= 16 => {
                let k = s.wrapping_sub(*pid) - 1;
                let b = (*blp) | (1 << k);
                let p = *pid;
                pairs.pop();
                pairs.push((p, b));
            }
            _ => pairs.push((s, 0)),
        }
    }
    let mut v = head(1, 205, 2 + pairs.len());
    v.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    v.extend_from_slice(&media.to_be_bytes());
    for (p, b) in pairs {
        v.extend_from_slice(&p.to_be_bytes());
        v.extend_from_slice(&b.to_be_bytes());
    }
    v
}

pub fn sr(ssrc: u32, ntp: u64, rtp_ts: u32, packets: u32, octets: u32) -> Vec<u8> {
    let mut v = head(0, 200, 6);
    v.extend_from_slice(&ssrc.to_be_bytes());
    v.extend_from_slice(&ntp.to_be_bytes());
    v.extend_from_slice(&rtp_ts.to_be_bytes());
    v.extend_from_slice(&packets.to_be_bytes());
    v.extend_from_slice(&octets.to_be_bytes());
    v
}

pub fn ntp_mid(ntp: u64) -> u32 {
    ((ntp >> 16) & 0xffff_ffff) as u32
}

pub fn dlsr(held_ms: u64) -> u32 {
    dlsr_us(held_ms.saturating_mul(1_000))
}

pub fn dlsr_us(held_us: u64) -> u32 {
    ((held_us.saturating_mul(65_536)) / 1_000_000).min(u64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let b = ReportBlock { ssrc: 9, fraction: 3, lost: -2, ext_max: 100, jitter: 5, lsr: 7, dlsr: 65_536 };
        let p = parse(&rr(&b));
        assert_eq!(p, vec![Packet::Rr { ssrc: SENDER_SSRC, blocks: vec![b] }]);
        assert_eq!(parse(&pli(5)), vec![Packet::Pli { media: 5 }]);
        let n = parse(&nack(7, &[10, 11, 13, 40]));
        assert_eq!(n, vec![Packet::Nack { media: 7, seqs: vec![10, 11, 13, 40] }]);
        let r = parse(&remb(800_000, &[3]));
        assert!(matches!(&r[0], Packet::Remb { bitrate, .. } if *bitrate <= 800_000 && *bitrate > 790_000));
        assert_eq!(dlsr(1_000), 65_536);
        let mut compound = sr(1, 2, 3, 4, 5);
        compound.extend_from_slice(&pli(6));
        assert_eq!(parse(&compound).len(), 2);
    }
}
