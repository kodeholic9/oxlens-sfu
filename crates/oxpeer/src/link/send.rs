// author: kodeholic (powered by Claude)
//! 보내는 쪽(20261107c 에이전트 단계) — 앱이 넣은 인코딩된 프레임을 RTP 로 쪼개고(코덱 꼴 → RTP 꼴 · 조립의 거꾸로) 확장을 싣고 기록한다.
//! 확장은 서버 선언 번호(연:RULE-LINK-EGRESS-REWRITE ② — 발행 확장 ID 는 클라 번호로 나가고 `PUBLISH_TRACKS` 의 `*_extmap_id` 로 신고) — mid(첫 패킷들) · 오디오 음량(SDK:RULE-PLAT-SERVER 음량 표시).
//! 영상만 보낸 기록을 쥐고 NACK 에 RTX(RFC 4588)로 답한다(오디오 NACK 미채택 — 연:RULE-LINK-SDP-SUB-MLINE). SR(RFC 3550 §6.4.1)은 보낸 수 · 바이트와 지금 시각의 NTP · RTP 짝.
//! 시각은 부르는 쪽이 준다(1층이 시계 없이 돈다).

use std::collections::VecDeque;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::depack::Codec;

/// libwebrtc 의 RTP 페이로드 상한 근처(경로 MTU 안 · SRTP 꼬리 · 확장 여유)
pub(crate) const MTU: usize = 1_150;
/// mid 확장을 싣는 첫 패킷 수(브라우저 실측 꼴 — 처음 몇 장만)
const MID_PACKETS: u32 = 5;
/// 보낸 기록 — 이만큼 지난 패킷은 다시 못 보낸다(NACK 이 늦었다)
const HISTORY_MS: u128 = 1_000;
const HISTORY_MAX: usize = 1_024;
const NTP_UNIX: u64 = 2_208_988_800;

/// 확장 번호 — 서버 선언(`server_config.extmap`)에서 고른 것(0 = 안 싣는다)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExtIds {
    pub mid: u8,
    pub audio_level: u8,
    /// 시뮬캐스트 층 이름(rtp-stream-id) · RTX 의 원 층 이름(repaired-rtp-stream-id) — 서버가 이것으로 업링크 층을 배운다(연:RULE-PUB-RID-TO-SPATIAL)
    pub rid: u8,
    pub rrid: u8,
    /// transport-cc — 패킷마다 전송 차례 번호(보내는 순간 `bwe` 가 채운다 · context 202611/20261108d E4)
    pub twcc: u8,
}

/// 앱이 넣는 프레임 하나 — 코덱 꼴(H264 Annex-B · VP8 프레임 · Opus 패킷)
#[derive(Clone, Debug)]
pub struct OutFrame {
    pub data: bytes::Bytes,
    /// 이 프레임이 차지하는 시간(Opus 는 보통 20ms · 영상은 프레임 간격) — 송신 시계가 이만큼 뒤에 다음 것을 낸다
    pub duration: std::time::Duration,
    pub keyframe: bool,
    /// 오디오 음량(dBov 0 ~ 127 · 0 이 가장 크다 — RFC 6464) · None 이면 말소리로 싣는다
    pub audio_level: Option<u8>,
}

/// 말소리로 볼 기본 음량(서버는 50 dBov 보다 작은 값을 「말하는 중」으로 잰다)
pub(crate) const VOICE_LEVEL: u8 = 30;

/// 코덱 꼴 → RTP 페이로드들(마지막이 marker)
pub(crate) fn packetize(codec: Codec, f: &[u8]) -> Vec<Vec<u8>> {
    match codec {
        Codec::Vp8 => f.chunks(MTU).enumerate().map(|(i, c)| [&[if i == 0 { 0x10 } else { 0x00 }][..], c].concat()).collect(),
        Codec::H264 => h264(f),
        Codec::Opus | Codec::Other => vec![f.to_vec()],
    }
}

fn annexb(b: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let mut e = starts.get(k + 1).map(|n| n - 3).unwrap_or(b.len());
        while e > s && b[e - 1] == 0 {
            e -= 1;
        }
        if e > s {
            out.push(&b[s..e]);
        }
    }
    out
}

/// RFC 6184 packetization-mode 1 — 작은 SPS · PPS 는 STAP-A 로 묶고 · MTU 를 넘는 NAL 은 FU-A · 나머지는 단일 NAL(libwebrtc 꼴)
fn h264(f: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut stap: Vec<&[u8]> = Vec::new();
    let flush = |stap: &mut Vec<&[u8]>, out: &mut Vec<Vec<u8>>| {
        match stap.len() {
            0 => {}
            1 => out.push(stap[0].to_vec()),
            _ => {
                let nri = stap.iter().map(|n| n[0] & 0x60).max().unwrap_or(0);
                let mut p = vec![nri | 24];
                for n in stap.iter() {
                    p.extend((n.len() as u16).to_be_bytes());
                    p.extend_from_slice(n);
                }
                out.push(p);
            }
        }
        stap.clear();
    };
    for n in annexb(f) {
        let t = n[0] & 0x1f;
        if (t == 7 || t == 8) && n.len() < 200 {
            stap.push(n);
            continue;
        }
        flush(&mut stap, &mut out);
        if n.len() <= MTU {
            out.push(n.to_vec());
            continue;
        }
        let parts: Vec<&[u8]> = n[1..].chunks(MTU - 2).collect();
        for (i, c) in parts.iter().enumerate() {
            let s = if i == 0 { 0x80 } else { 0 };
            let e = if i == parts.len() - 1 { 0x40 } else { 0 };
            out.push([&[(n[0] & 0xe0) | 28, s | e | t][..], c].concat());
        }
    }
    flush(&mut stap, &mut out);
    out
}

struct Sent {
    seq: u16,
    ts: u32,
    marker: bool,
    payload: Vec<u8>,
    at: Instant,
}

/// 보내는 트랙 하나의 RTP 상태 — 송신 시계가 쥐고 · 전송의 RTCP 처리가 NACK 때 빌린다
pub(crate) struct SendTrack {
    pub ssrc: u32,
    pub pt: u8,
    pub rtx: Option<(u32, u8)>,
    pub codec: Codec,
    pub clock: u32,
    pub mid: String,
    pub ext: ExtIds,
    /// 시뮬캐스트 층 이름(`l` · `h`) — 아니면 None
    pub rid: Option<String>,
    seq: u16,
    rtx_seq: u16,
    ts_base: u32,
    sent_n: u32,
    pub packets: u32,
    pub octets: u32,
    /// 마지막으로 보낸 프레임의 (RTP 시각, 보낸 때) — SR 의 짝
    last: Option<(u32, Instant)>,
    history: VecDeque<Sent>,
}

impl SendTrack {
    pub(crate) fn new(ssrc: u32, pt: u8, rtx: Option<(u32, u8)>, codec: Codec, video: bool, mid: String, ext: ExtIds) -> SendTrack {
        let mut r = [0u8; 6];
        let _ = getrandom::fill(&mut r);
        SendTrack {
            ssrc,
            pt,
            rtx,
            codec,
            clock: codec.clock(video),
            mid,
            ext,
            rid: None,
            seq: u16::from_be_bytes([r[0], r[1]]),
            rtx_seq: u16::from_be_bytes([r[2], r[3]]),
            ts_base: u32::from(u16::from_be_bytes([r[4], r[5]])) << 8,
            sent_n: 0,
            packets: 0,
            octets: 0,
            last: None,
            history: VecDeque::new(),
        }
    }

    /// 다른 서버(발행 연결)로 옮겼다 — 그 연결의 mid 로 바꾸고 첫 패킷들에 다시 싣는다(SDK:pub.md `LocalTrack.server`)
    pub(crate) fn rebind(&mut self, mid: String) {
        self.mid = mid;
        self.sent_n = 0;
    }

    /// 송신 시계가 정한 때 `media` 의 RTP 시각
    pub(crate) fn ts_at(&self, media: std::time::Duration) -> u32 {
        self.ts_base.wrapping_add((media.as_secs_f64() * f64::from(self.clock)).round() as u32)
    }

    /// 머리 — 확장 차례: ★transport-cc 가 맨 앞(보내는 순간 번호를 채우는 자리가 정해져 있다 — `bwe::stamp`) · mid · 층 이름 · 음량
    #[allow(clippy::too_many_arguments)]
    fn header(&self, seq: u16, ts: u32, marker: bool, pt: u8, ssrc: u32, level: Option<u8>, with_mid: bool, repair: bool) -> Vec<u8> {
        let mut ext: Vec<u8> = Vec::new();
        if self.ext.twcc != 0 {
            ext.extend([(self.ext.twcc << 4) | 1, 0, 0]);
        }
        if with_mid && self.ext.mid != 0 && !self.mid.is_empty() {
            let m = self.mid.as_bytes();
            ext.push((self.ext.mid << 4) | (m.len() as u8 - 1));
            ext.extend_from_slice(m);
        }
        let rid_id = if repair { self.ext.rrid } else { self.ext.rid };
        if let Some(r) = self.rid.as_ref().filter(|r| !r.is_empty() && rid_id != 0) {
            ext.push((rid_id << 4) | (r.len() as u8 - 1));
            ext.extend_from_slice(r.as_bytes());
        }
        if let Some(l) = level
            && self.ext.audio_level != 0
        {
            // RFC 6464 — V 비트(말소리 있음)는 서버가 보지 않는다 · 음량만
            ext.push(self.ext.audio_level << 4);
            ext.push(0x80 | (l & 0x7f));
        }
        let mut h = Vec::with_capacity(16 + ext.len());
        h.push(if ext.is_empty() { 0x80 } else { 0x90 });
        h.push(if marker { 0x80 } else { 0 } | (pt & 0x7f));
        h.extend(seq.to_be_bytes());
        h.extend(ts.to_be_bytes());
        h.extend(ssrc.to_be_bytes());
        if !ext.is_empty() {
            while !ext.len().is_multiple_of(4) {
                ext.push(0);
            }
            h.extend([0xBE, 0xDE]);
            h.extend(((ext.len() / 4) as u16).to_be_bytes());
            h.extend(ext);
        }
        h
    }

    /// 프레임 하나 → 평문 RTP 패킷들(보낸 기록에 남긴다) · `spurt` = 오디오 말 덩어리의 첫 프레임(RFC 3551 §4.1 marker)
    pub(crate) fn frame(&mut self, f: &OutFrame, ts: u32, now: Instant, spurt: bool) -> Vec<Vec<u8>> {
        let video = matches!(self.codec, Codec::Vp8 | Codec::H264) || self.clock == 90_000;
        let parts = packetize(self.codec, &f.data);
        let n = parts.len();
        let mut out = Vec::with_capacity(n);
        for (i, p) in parts.into_iter().enumerate() {
            let marker = if video { i == n - 1 } else { spurt && i == 0 };
            let level = if video { None } else { Some(f.audio_level.unwrap_or(VOICE_LEVEL)) };
            let mut pkt = self.header(self.seq, ts, marker, self.pt, self.ssrc, level, self.sent_n < MID_PACKETS, false);
            pkt.extend_from_slice(&p);
            self.packets = self.packets.wrapping_add(1);
            self.octets = self.octets.wrapping_add(p.len() as u32);
            if video && self.rtx.is_some() {
                self.history.push_back(Sent { seq: self.seq, ts, marker, payload: p, at: now });
            }
            self.seq = self.seq.wrapping_add(1);
            self.sent_n += 1;
            out.push(pkt);
        }
        while self.history.len() > HISTORY_MAX || self.history.front().is_some_and(|s| now.duration_since(s.at).as_millis() > HISTORY_MS) {
            self.history.pop_front();
        }
        self.last = Some((ts, now));
        out
    }

    /// NACK 받은 seq 들 → RTX 패킷들(RFC 4588 — 앞 두 바이트 원래 seq) · 기록에 없으면 건너뛴다
    pub(crate) fn retransmit(&mut self, seqs: &[u16]) -> Vec<Vec<u8>> {
        let Some((rssrc, rpt)) = self.rtx else { return Vec::new() };
        let mut out = Vec::new();
        for s in seqs {
            let Some(p) = self.history.iter().find(|x| x.seq == *s) else { continue };
            let (ts, marker, osn, body) = (p.ts, p.marker, p.seq, p.payload.clone());
            let mut pkt = self.header(self.rtx_seq, ts, marker, rpt, rssrc, None, false, true);
            pkt.extend(osn.to_be_bytes());
            pkt.extend(body);
            self.rtx_seq = self.rtx_seq.wrapping_add(1);
            out.push(pkt);
        }
        out
    }

    /// SR(RFC 3550 §6.4.1) — 지금 벽시계 · 그에 맞는 RTP 시각(마지막 프레임에서 흐른 만큼) · 보낸 수 · 바이트
    pub(crate) fn sender_report(&self, now: Instant, wall: SystemTime) -> Option<Vec<u8>> {
        let (ts, at) = self.last?;
        let rtp = ts.wrapping_add((now.duration_since(at).as_secs_f64() * f64::from(self.clock)) as u32);
        let d = wall.duration_since(UNIX_EPOCH).ok()?;
        let ntp = ((d.as_secs() + NTP_UNIX) << 32) | ((u64::from(d.subsec_nanos()) << 32) / 1_000_000_000);
        let mut b = vec![0x80, 200, 0, 6];
        b.extend(self.ssrc.to_be_bytes());
        b.extend(ntp.to_be_bytes());
        b.extend(rtp.to_be_bytes());
        b.extend(self.packets.to_be_bytes());
        b.extend(self.octets.to_be_bytes());
        Some(b)
    }
}

/// 받은 RTCP 묶음에서 발행자가 볼 것 — NACK(미디어 ssrc · seq 들) · PLI(미디어 ssrc) · REMB(bps)
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Feedback {
    pub nacks: Vec<(u32, Vec<u16>)>,
    pub plis: Vec<u32>,
    pub remb: Option<u64>,
    /// TWCC — 전송 번호마다 받은 시각(µs · None 못 받음)
    pub twcc: Vec<Vec<(u16, Option<i64>)>>,
}

pub(crate) fn feedback(b: &[u8]) -> Feedback {
    let mut f = Feedback::default();
    let mut i = 0;
    while i + 4 <= b.len() && b[i] >> 6 == 2 {
        let len = (usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]])) + 1) * 4;
        let Some(p) = b.get(i..i + len) else { break };
        let (fmt, pt) = (p[0] & 0x1f, p[1]);
        let media = p.get(8..12).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]));
        match (pt, fmt) {
            (205, 15) => {
                if let Some(r) = super::bwe::parse_twcc(p) {
                    f.twcc.push(r);
                }
            }
            (205, 1) => {
                let mut seqs = Vec::new();
                let mut k = 12;
                while k + 4 <= p.len() {
                    let pid = u16::from_be_bytes([p[k], p[k + 1]]);
                    let blp = u16::from_be_bytes([p[k + 2], p[k + 3]]);
                    seqs.push(pid);
                    for j in 0..16 {
                        if blp & (1 << j) != 0 {
                            seqs.push(pid.wrapping_add(j + 1));
                        }
                    }
                    k += 4;
                }
                if let Some(m) = media {
                    f.nacks.push((m, seqs));
                }
            }
            (206, 1) | (206, 4) => {
                let m = if fmt == 4 { p.get(12..16).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]])) } else { media };
                if let Some(m) = m {
                    f.plis.push(m);
                }
            }
            (206, 15) if p.get(12..16) == Some(b"REMB") && p.len() >= 20 => {
                let exp = p[17] >> 2;
                let mant = (u64::from(p[17] & 0x03) << 16) | (u64::from(p[18]) << 8) | u64::from(p[19]);
                f.remb = Some(mant << exp);
            }
            _ => {}
        }
        i += len;
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::depack;
    use crate::link::transport::rtp_header;
    use std::time::Duration;

    fn of(data: &[u8], key: bool) -> OutFrame {
        OutFrame { data: bytes::Bytes::copy_from_slice(data), duration: Duration::from_millis(20), keyframe: key, audio_level: None }
    }

    #[test]
    fn opus_packet_has_mid_then_level_only() {
        let mut t = SendTrack::new(0xA1, 111, None, Codec::Opus, false, "0".into(), ExtIds { mid: 1, audio_level: 4, ..Default::default() });
        let now = Instant::now();
        let mut first = Vec::new();
        for i in 0..7 {
            let p = t.frame(&of(&[0xf8, i], true), 960 * u32::from(i), now, i == 0).remove(0);
            first.push(p);
        }
        let h = rtp_header(&first[0]).unwrap();
        assert_eq!((h.pt, h.marker), (111, true));
        assert!(!rtp_header(&first[1]).unwrap().marker);
        // 확장 머리 BEDE · mid(1) "0" · 음량(4) 30
        assert_eq!(&first[0][12..14], &[0xBE, 0xDE]);
        assert!(first[0][16..20].windows(2).any(|w| w == [0x10, b'0']));
        assert!(first[0][16..20].windows(2).any(|w| w == [0x40, 0x80 | 30]));
        // 다섯 장 뒤엔 mid 를 안 싣는다
        assert!(!first[6][16..20].windows(2).any(|w| w == [0x10, b'0']));
        assert_eq!(&first[6][first[6].len() - 2..], &[0xf8, 6]);
        assert_eq!(t.packets, 7);
    }

    #[test]
    fn video_round_trips_through_depack_and_rtx() {
        let mut t = SendTrack::new(0xB1, 96, Some((0xB2, 97)), Codec::Vp8, true, "1".into(), ExtIds { mid: 1, audio_level: 4, ..Default::default() });
        let big: Vec<u8> = (0..3000u32).map(|x| x as u8).collect();
        let mut f = vec![0x10];
        f.extend(&big);
        let now = Instant::now();
        let pkts = t.frame(&of(&f, true), 0, now, false);
        assert_eq!(pkts.len(), 3);
        let mut joined = Vec::new();
        for (i, p) in pkts.iter().enumerate() {
            let h = rtp_header(p).unwrap();
            assert_eq!(h.marker, i == 2);
            let piece = depack::unpack(Codec::Vp8, &p[h.start..h.end]).unwrap();
            assert_eq!(piece.start, i == 0);
            joined.extend(piece.data);
        }
        assert_eq!(joined, f);
        let s1 = rtp_header(&pkts[1]).unwrap().seq;
        let r = t.retransmit(&[s1, s1.wrapping_add(100)]);
        assert_eq!(r.len(), 1);
        let h = rtp_header(&r[0]).unwrap();
        assert_eq!((h.ssrc, h.pt), (0xB2, 97));
        assert_eq!(&r[0][h.start..h.start + 2], &s1.to_be_bytes());
        assert_eq!(&r[0][h.start + 2..h.end], &pkts[1][rtp_header(&pkts[1]).unwrap().start..]);
    }

    #[test]
    fn h264_stap_and_fua_depack_back() {
        let mut t = SendTrack::new(0xC1, 102, Some((0xC2, 103)), Codec::H264, true, "1".into(), ExtIds::default());
        let idr: Vec<u8> = std::iter::once(0x65).chain((0..2500u32).map(|x| (x % 251) as u8 | 1)).collect();
        let mut au = vec![0, 0, 0, 1, 0x67, 0x42, 0xe0, 0x1f, 0, 0, 0, 1, 0x68, 0xce, 0, 0, 0, 1];
        au.extend(&idr);
        let pkts = t.frame(&of(&au, true), 0, Instant::now(), false);
        let kinds: Vec<u8> = pkts.iter().map(|p| p[rtp_header(p).unwrap().start] & 0x1f).collect();
        assert_eq!(kinds, vec![24, 28, 28, 28]);
        let mut out = Vec::new();
        for p in &pkts {
            let h = rtp_header(p).unwrap();
            out.extend(depack::unpack(Codec::H264, &p[h.start..h.end]).unwrap().data);
        }
        let want: Vec<u8> = [&[0, 0, 0, 1, 0x67, 0x42, 0xe0, 0x1f, 0, 0, 0, 1, 0x68, 0xce, 0, 0, 0, 1][..], &idr].concat();
        assert_eq!(out, want);
    }

    #[test]
    fn sr_and_feedback() {
        let mut t = SendTrack::new(0xA1, 111, None, Codec::Opus, false, "0".into(), ExtIds::default());
        let t0 = Instant::now();
        assert!(t.sender_report(t0, SystemTime::now()).is_none());
        t.frame(&of(&[0xf8], true), 48_000, t0, true);
        let sr = t.sender_report(t0 + Duration::from_millis(500), UNIX_EPOCH + Duration::from_secs(1)).unwrap();
        assert_eq!(&sr[4..8], &0xA1u32.to_be_bytes());
        assert_eq!(&sr[8..12], &((1 + NTP_UNIX) as u32).to_be_bytes());
        assert_eq!(u32::from_be_bytes([sr[16], sr[17], sr[18], sr[19]]), 48_000 + 24_000);
        // NACK(0xB1: 10, 12) · PLI(0xB1) · REMB 300k
        let mut fb = vec![0x81, 205, 0, 3, 0, 0, 0, 9];
        fb.extend(0xB1u32.to_be_bytes());
        fb.extend([0, 10, 0, 2]);
        fb.extend([0x81, 206, 0, 2, 0, 0, 0, 9, 0, 0, 0, 0xB1]);
        let mut remb = vec![0x8f, 206, 0, 5, 0, 0, 0, 9, 0, 0, 0, 0, b'R', b'E', b'M', b'B', 1];
        let (exp, mant) = (2u8, 75_000u32);
        remb.extend([(exp << 2) | (mant >> 16) as u8, (mant >> 8) as u8, mant as u8]);
        remb.extend(0xB1u32.to_be_bytes());
        fb.extend(remb);
        let f = feedback(&fb);
        assert_eq!(f.nacks, vec![(0xB1, vec![10, 12])]);
        assert_eq!(f.plis, vec![0xB1]);
        assert_eq!(f.remb, Some(300_000));
    }
}
