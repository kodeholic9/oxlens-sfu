// author: kodeholic (powered by Claude)
//! 수신 정리층(SDK:RULE-PLAT-SERVER 수신 정리층) — 브라우저에서는 libwebrtc 수신 쪽이 하던 일이다. 다운링크 스트림(줄 ssrc)마다:
//! 재정렬 · ★영상만 NACK(재전송은 RTX 로 와서 원래 seq 로 푼다) · 포기한 손실 뒤 키프레임까지 기다리고 PLI · 층을 바꾼 뒤 키프레임이 없으면 PLI · RR · SR 로 벽시계 잇기.
//! 무엇을 보낼지는 그 줄의 `rtcp_fb` 가 정한다(연:RULE-LINK-SDP-SUB-MLINE — 오디오에는 NACK 이 없다). 앱은 RTCP 를 짓지 않는다.
//! 전송 하나의 수신 태스크가 이것 하나를 쥔다(잠금 없음) · 시각은 부르는 쪽이 준다(1층이 시계 없이 돈다).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;

use super::depack::{self, Codec};
use super::transport::{MediaPacket, Route, Source};

/// NACK 다시 묻기 간격 · 횟수 — libwebrtc NackRequester 기본(RTT 를 모를 때 100ms · 10회)과 같다. 다 묻고도 안 오면 그 패킷을 포기한다
const NACK_INTERVAL: Duration = Duration::from_millis(100);
const NACK_TRIES: u8 = 10;
/// NACK 을 안 하는 스트림(오디오)이 순서를 기다리는 시간
const HOLD_NO_NACK: Duration = Duration::from_millis(50);
/// 이보다 큰 seq 틈은 묻지 않고 스트림을 새로 잡는다(libwebrtc 의 NACK 목록 상한과 같은 뜻)
const MAX_GAP: u64 = 1_000;
/// 키프레임을 기다리는 동안 PLI 간격 — 서버가 층마다 스로틀한다(정책 media.pli_throttle 높은 층 1000ms)
const PLI_INTERVAL: Duration = Duration::from_millis(1_000);
/// 층을 바꾼 뒤 이만큼 키프레임이 없으면 한 번 묻는다
const LAYER_KEY_WAIT: Duration = Duration::from_millis(1_000);
const RR_INTERVAL: Duration = Duration::from_millis(1_000);
/// 확장 seq 의 바닥 — 첫 패킷 앞으로 조금 늦은 것도 음수 없이 센다
const EXT_BASE: u64 = 1 << 16;
const RING: usize = 2_048;
/// NTP(1900) → 유닉스(1970)
const NTP_UNIX: u64 = 2_208_988_800;

/// 트랙마다 받는 단위(SDK:RULE-PLAT-SERVER 수신 단위 · 시각)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Unit {
    /// 복호 RTP 그대로(받은 순서 · RTX 로 메운 것은 원래 seq 로)
    #[default]
    Packet,
    /// 정리층을 지나 조립한 코덱 프레임(순서대로 · 깨진 프레임은 빼고 · 영상은 키프레임부터)
    Frame,
}

/// 조립한 코덱 프레임 하나 — H264 는 Annex-B · VP8 은 서술자를 뗀 프레임 · Opus 는 패킷 하나
#[derive(Clone, Debug)]
pub struct MediaFrame {
    pub room: String,
    pub track_id: String,
    /// 트랙 주인(개인 트랙) — 무전 슬롯은 없다
    pub user_id: Option<String>,
    pub kind: String,
    pub codec: String,
    pub mid: String,
    pub ssrc: u32,
    /// RTP 시각(그 코덱의 시계)
    pub timestamp: u32,
    pub keyframe: bool,
    pub data: Bytes,
    /// 벽시계 — `synced` 면 SR(NTP · RTP 짝)로 이은 발행 쪽 시계 · 아니면 첫 SR 전이라 받은 시각
    pub wallclock: SystemTime,
    pub synced: bool,
    pub received_at: Instant,
    /// 이 프레임의 발행 출처(조각마다 같아야 한다 — 갈리면 깨진 프레임) · 없으면 출처 없음
    pub source: Option<Source>,
}

/// 정리층이 내는 것
#[derive(Debug)]
pub(crate) enum Out {
    Packet(MediaPacket),
    Frame(MediaFrame),
    /// 정리층 출구 — 차례가 선 패킷의 원본(녹음 원시 기록 · SDK:RULE-PLAT-REC-TAP) · `record` 기능이 없으면 만들어지지 않는다
    #[cfg_attr(not(feature = "record"), allow(dead_code))]
    Raw { ssrc: u32, data: Bytes },
    /// 빈 번호 · 스트림 재설정(SDK:RULE-PLAT-REC-TAP 빈 번호) — 녹음을 켰을 때만
    #[cfg_attr(not(feature = "record"), allow(dead_code))]
    Gap(Gap),
}

/// 빈 번호 사건 — `from` · `to` 는 16비트 seq(양 끝 포함)
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "record"), allow(dead_code))]
pub(crate) struct Gap {
    pub ssrc: u32,
    pub from: u16,
    pub to: u16,
    pub count: u64,
    pub reason: &'static str,
}

/// 정리층 계수 — 모든 전송이 같이 센다
#[derive(Debug, Default)]
pub(crate) struct Counters {
    pub nack_sent: AtomicU64,
    pub rtx_recovered: AtomicU64,
    pub lost: AtomicU64,
    pub pli_sent: AtomicU64,
    pub rr_sent: AtomicU64,
    pub frames_broken: AtomicU64,
    /// 키프레임을 기다리느라 버린 프레임
    pub frames_skipped: AtomicU64,
}

impl Counters {
    fn add(c: &AtomicU64, n: u64) {
        c.fetch_add(n, Ordering::Relaxed);
    }
}

/// 받은 RTP 하나(머리를 푼 것) — RTX 면 원래 seq · PT 로 풀어서 준다
#[derive(Clone, Debug)]
pub(crate) struct Rtp {
    pub seq: u16,
    pub ts: u32,
    pub marker: bool,
    pub pt: u8,
    pub payload: Bytes,
    pub recovered: bool,
    pub source: Option<Source>,
    /// 복호한 패킷 원본(녹음 원시 기록을 켰을 때만 — SDK:RULE-PLAT-REC-TAP) · RTX 는 원래 패킷으로 다시 지은 것
    pub raw: Option<Bytes>,
}

#[derive(Clone, Copy)]
struct Sr {
    ntp: u64,
    rtp: u32,
    at: Instant,
}

struct Pkt {
    rtp: Rtp,
    at: Instant,
    wall: SystemTime,
}

struct Miss {
    first: Instant,
    sent: Option<Instant>,
    tries: u8,
}

struct Asm {
    ts: u32,
    data: Vec<u8>,
    key: bool,
    broken: bool,
    source: Option<Source>,
    at: Instant,
    wall: SystemTime,
}

struct Stream {
    route: Route,
    codec: Codec,
    clock: u32,
    video: bool,
    highest: u64,
    base: u64,
    first: u64,
    buf: BTreeMap<u64, Pkt>,
    missing: BTreeMap<u64, Miss>,
    ring: Vec<u64>,
    asm: Option<Asm>,
    /// 키프레임 전에는 프레임을 내지 않는다 · `want_key` = 그래서 실제로 버렸다(PLI 를 낼 까닭) — 첫 키프레임을 기다리는 것만으로는 묻지 않는다
    need_key: bool,
    want_key: bool,
    pli_at: Option<Instant>,
    key_wait: Option<Instant>,
    layer_gen: u64,
    /// 앱 키프레임 요청 — 차례가 바뀌면 다음 차례에 PLI 한 번
    key_gen: u64,
    ask_key: bool,
    received: u64,
    expected_prior: u64,
    received_prior: u64,
    jitter: f64,
    last_transit: Option<f64>,
    last_ts: Option<u32>,
    sr: Option<Sr>,
    epoch: Instant,
}

impl Stream {
    fn new(route: &Route, seq: u16, now: Instant) -> Stream {
        let codec = Codec::of(&route.codec);
        let video = route.kind == "video";
        let ext = EXT_BASE + u64::from(seq);
        Stream {
            route: route.clone(),
            codec,
            clock: codec.clock(video),
            video,
            highest: ext,
            base: ext,
            first: ext,
            buf: BTreeMap::new(),
            missing: BTreeMap::new(),
            ring: vec![0; RING],
            asm: None,
            need_key: video,
            want_key: false,
            pli_at: None,
            key_wait: None,
            layer_gen: route.layer_gen,
            key_gen: route.key_gen,
            ask_key: false,
            received: 0,
            expected_prior: 0,
            received_prior: 0,
            jitter: 0.0,
            last_transit: None,
            last_ts: None,
            sr: None,
            epoch: now,
        }
    }

    /// 16비트 seq → 지금 가장 높은 것에 가까운 확장 seq
    fn extend(&self, seq: u16) -> u64 {
        let hi = self.highest;
        let mut c = (hi & !0xffff) | u64::from(seq);
        if c + 0x8000 < hi {
            c += 0x1_0000;
        } else if c > hi + 0x8000 && c >= 0x1_0000 {
            c -= 0x1_0000;
        }
        c
    }

    fn seen(&self, ext: u64) -> bool {
        self.ring[(ext as usize) % RING] == ext + 1
    }

    fn mark(&mut self, ext: u64) {
        self.ring[(ext as usize) % RING] = ext + 1;
    }

    fn gone(&self, m: &Miss, now: Instant) -> bool {
        if self.route.nack {
            m.tries >= NACK_TRIES && m.sent.is_none_or(|t| now.duration_since(t) >= NACK_INTERVAL)
        } else {
            now.duration_since(m.first) >= HOLD_NO_NACK
        }
    }

    /// RFC 3550 A.8 — 도착 간격 흔들림(그 코덱 시계 단위)
    fn jitter_on(&mut self, r: &Rtp, at: Instant) {
        if self.last_ts == Some(r.ts) {
            return;
        }
        let arrival = at.duration_since(self.epoch).as_secs_f64() * f64::from(self.clock);
        let transit = arrival - f64::from(r.ts);
        if let Some(prev) = self.last_transit {
            let d = (transit - prev).abs();
            if d < f64::from(self.clock) * 10.0 {
                self.jitter += (d - self.jitter) / 16.0;
            }
        }
        self.last_transit = Some(transit);
        self.last_ts = Some(r.ts);
    }

    fn wall_of(&self, ts: u32, got: SystemTime) -> (SystemTime, bool) {
        let Some(sr) = self.sr else { return (got, false) };
        let secs = (sr.ntp >> 32).saturating_sub(NTP_UNIX);
        let nanos = ((sr.ntp & 0xffff_ffff) * 1_000_000_000) >> 32;
        let base = UNIX_EPOCH + Duration::new(secs, nanos as u32);
        let d = ts.wrapping_sub(sr.rtp) as i32;
        let off = Duration::from_secs_f64(f64::from(d.unsigned_abs()) / f64::from(self.clock));
        (if d >= 0 { base + off } else { base - off }, true)
    }

    fn packet(&self, r: &Rtp, at: Instant) -> MediaPacket {
        let rt = &self.route;
        MediaPacket {
            room: rt.room.clone(),
            track_id: rt.track_id.clone(),
            user_id: rt.user_id.clone(),
            kind: rt.kind.clone(),
            codec: rt.codec.clone(),
            mid: rt.mid.clone(),
            ssrc: rt.ssrc,
            payload_type: r.pt,
            sequence: r.seq,
            timestamp: r.ts,
            marker: r.marker,
            payload: r.payload.clone(),
            received_at: at,
            source: r.source.clone(),
        }
    }

    /// 순서가 선 것을 내보낸다 — 앞이 비었으면 포기할 때까지 기다린다
    fn release(&mut self, now: Instant, c: &Counters, out: &mut Vec<Out>, record: bool) {
        // 포기한 번호가 이어지면 빈 번호 사건 하나(SDK:RULE-PLAT-REC-TAP)
        let mut run: Option<(u64, u64)> = None;
        while self.base <= self.highest {
            if let Some(p) = self.buf.remove(&self.base) {
                self.base += 1;
                if let Some(g) = run.take() {
                    self.gap_out(g, out);
                }
                if record && let Some(raw) = &p.rtp.raw {
                    out.push(Out::Raw { ssrc: self.route.ssrc, data: raw.clone() });
                }
                self.feed(Some(p), c, out);
                continue;
            }
            let give_up = match self.missing.get(&self.base) {
                Some(m) => self.gone(m, now),
                None => true,
            };
            if !give_up {
                break;
            }
            self.missing.remove(&self.base);
            Counters::add(&c.lost, 1);
            if record {
                run = Some(run.map_or((self.base, self.base), |(a, _)| (a, self.base)));
            }
            self.base += 1;
            self.feed(None, c, out);
        }
        if let Some(g) = run {
            self.gap_out(g, out);
        }
    }

    fn gap_out(&self, (a, b): (u64, u64), out: &mut Vec<Out>) {
        out.push(Out::Gap(Gap { ssrc: self.route.ssrc, from: a as u16, to: b as u16, count: b - a + 1, reason: "nack_exhausted" }));
    }

    /// 조립 — None 은 포기한 틈
    fn feed(&mut self, p: Option<Pkt>, c: &Counters, out: &mut Vec<Out>) {
        let Some(p) = p else {
            if let Some(a) = self.asm.as_mut() {
                a.broken = true;
            }
            if self.video {
                self.need_key = true;
                self.want_key = true;
            }
            return;
        };
        if p.rtp.payload.is_empty() {
            return;
        }
        if self.asm.as_ref().is_some_and(|a| a.ts != p.rtp.ts)
            && let Some(a) = self.asm.take()
        {
            self.finish(a, c, out);
        }
        let piece = depack::unpack(self.codec, &p.rtp.payload);
        let a = self.asm.get_or_insert_with(|| Asm { ts: p.rtp.ts, data: Vec::new(), key: false, broken: piece.as_ref().is_some_and(|x| !x.start), source: p.rtp.source.clone(), at: p.at, wall: p.wall });
        if a.source.as_ref().map(|s| s.csrc) != p.rtp.source.as_ref().map(|s| s.csrc) {
            a.broken = true;
        }
        match piece {
            Some(x) => {
                a.data.extend_from_slice(&x.data);
                a.key |= x.key;
            }
            None => a.broken = true,
        }
        let whole = p.rtp.marker || !self.video;
        if whole && let Some(a) = self.asm.take() {
            self.finish(a, c, out);
        }
    }

    fn finish(&mut self, a: Asm, c: &Counters, out: &mut Vec<Out>) {
        if a.broken {
            Counters::add(&c.frames_broken, 1);
            if self.video {
                self.need_key = true;
                self.want_key = true;
            }
            return;
        }
        if a.key {
            self.need_key = false;
            self.want_key = false;
            self.key_wait = None;
        } else if self.need_key {
            Counters::add(&c.frames_skipped, 1);
            self.want_key = true;
            return;
        }
        if self.route.unit != Unit::Frame {
            return;
        }
        let (wallclock, synced) = self.wall_of(a.ts, a.wall);
        let rt = &self.route;
        out.push(Out::Frame(MediaFrame {
            room: rt.room.clone(),
            track_id: rt.track_id.clone(),
            user_id: rt.user_id.clone(),
            kind: rt.kind.clone(),
            codec: rt.codec.clone(),
            mid: rt.mid.clone(),
            ssrc: rt.ssrc,
            timestamp: a.ts,
            keyframe: a.key,
            data: Bytes::from(a.data),
            wallclock,
            synced,
            received_at: a.at,
            source: a.source,
        }));
    }

    /// RR 보고 블록 하나(RFC 3550 §6.4.1)
    fn report(&mut self, now: Instant) -> Option<[u8; 24]> {
        if self.received == 0 {
            return None;
        }
        let expected = self.highest - self.first + 1;
        let lost = expected as i64 - self.received as i64;
        let exp_i = expected - self.expected_prior;
        let rec_i = self.received - self.received_prior;
        self.expected_prior = expected;
        self.received_prior = self.received;
        let lost_i = exp_i as i64 - rec_i as i64;
        let fraction = if exp_i == 0 || lost_i <= 0 { 0 } else { ((lost_i << 8) / exp_i as i64).min(255) as u8 };
        let (lsr, dlsr) = match self.sr {
            Some(s) => (((s.ntp >> 16) & 0xffff_ffff) as u32, ((now.duration_since(s.at).as_micros() * 65_536 / 1_000_000) as u32).max(1)),
            None => (0, 0),
        };
        let mut b = [0u8; 24];
        b[0..4].copy_from_slice(&self.route.ssrc.to_be_bytes());
        b[4] = fraction;
        let cum = (lost.clamp(-0x80_0000, 0x7f_ffff) as u32) & 0x00ff_ffff;
        b[5..8].copy_from_slice(&cum.to_be_bytes()[1..4]);
        b[8..12].copy_from_slice(&((self.highest - EXT_BASE) as u32).to_be_bytes());
        b[12..16].copy_from_slice(&(self.jitter as u32).to_be_bytes());
        b[16..20].copy_from_slice(&lsr.to_be_bytes());
        b[20..24].copy_from_slice(&dlsr.to_be_bytes());
        Some(b)
    }
}

/// 줄 하나의 수신 계수
pub(crate) struct RowStat {
    pub ssrc: u32,
    pub room: String,
    pub track_id: String,
    pub received: u64,
    pub lost: i64,
    pub jitter_s: f64,
}

/// 전송 하나의 정리층
pub(crate) struct Recv {
    me: u32,
    streams: HashMap<u32, Stream>,
    counters: Arc<Counters>,
    rr_at: Option<Instant>,
    /// 녹음 원시 기록을 켰나 — 켜면 빈 번호 사건을 낸다
    pub(crate) record: bool,
}

impl Recv {
    pub(crate) fn new(me: u32, counters: Arc<Counters>) -> Recv {
        Recv { me, streams: HashMap::new(), counters, rr_at: None, record: false }
    }

    /// 받은 패킷 하나 — `route` 는 그 줄 ssrc 의 경로(RTX 면 원래 줄의 것)
    pub(crate) fn on_rtp(&mut self, route: &Route, r: Rtp, now: Instant, wall: SystemTime) -> Vec<Out> {
        let mut out = Vec::new();
        let fresh = self.streams.get(&route.ssrc).is_none_or(|s| s.route.track_id != route.track_id || s.route.codec != route.codec);
        if fresh {
            if r.recovered {
                return out;
            }
            if self.record && self.streams.contains_key(&route.ssrc) {
                out.push(Out::Gap(Gap { ssrc: route.ssrc, from: r.seq, to: r.seq, count: 0, reason: "stream_reset" }));
            }
            self.streams.insert(route.ssrc, Stream::new(route, r.seq, now));
        }
        let c = self.counters.clone();
        let Some(s) = self.streams.get_mut(&route.ssrc) else { return out };
        s.route = route.clone();
        if s.layer_gen != route.layer_gen {
            s.layer_gen = route.layer_gen;
            if s.video {
                s.key_wait = Some(now + LAYER_KEY_WAIT);
            }
        }
        if s.key_gen != route.key_gen {
            s.key_gen = route.key_gen;
            s.ask_key = s.video;
        }
        let ext = s.extend(r.seq);
        if ext > s.highest {
            let gap = ext - s.highest - 1;
            if gap > MAX_GAP {
                if self.record {
                    out.push(Out::Gap(Gap { ssrc: route.ssrc, from: (s.highest + 1) as u16, to: (ext - 1) as u16, count: gap, reason: "stream_reset" }));
                }
                s.buf.clear();
                s.missing.clear();
                s.asm = None;
                s.base = ext;
                s.first = ext;
                s.expected_prior = 0;
                s.received = 0;
                s.received_prior = 0;
                if s.video {
                    s.need_key = true;
                    s.want_key = true;
                }
            } else {
                for k in s.highest + 1..ext {
                    s.missing.insert(k, Miss { first: now, sent: None, tries: 0 });
                }
            }
            s.highest = ext;
        }
        if s.seen(ext) || s.buf.contains_key(&ext) {
            return out;
        }
        s.mark(ext);
        if r.recovered {
            Counters::add(&c.rtx_recovered, 1);
        } else {
            s.received += 1;
            s.jitter_on(&r, now);
        }
        if s.route.unit == Unit::Packet {
            out.push(Out::Packet(s.packet(&r, now)));
        }
        if ext < s.base {
            return out;
        }
        s.missing.remove(&ext);
        s.buf.insert(ext, Pkt { rtp: r, at: now, wall });
        s.release(now, &c, &mut out, self.record);
        out
    }

    /// 받은 SR(RFC 3550 §6.4.1) — 그 줄 ssrc 의 NTP · RTP 짝
    pub(crate) fn on_sr(&mut self, ssrc: u32, ntp: u64, rtp: u32, now: Instant) {
        if let Some(s) = self.streams.get_mut(&ssrc) {
            s.sr = Some(Sr { ntp, rtp, at: now });
        }
    }

    /// 받는 줄마다 수신 계수(운영 로그 `stat` 수신 줄 — W3C `packetsReceived` · `packetsLost` · `jitter`(초)) — 줄 번호 차례
    pub(crate) fn stats(&self) -> Vec<RowStat> {
        let mut v: Vec<RowStat> = self
            .streams
            .iter()
            .map(|(k, s)| {
                let expected = s.highest.saturating_sub(s.first) + 1;
                RowStat { ssrc: *k, room: s.route.room.clone(), track_id: s.route.track_id.clone(), received: s.received, lost: expected as i64 - s.received as i64, jitter_s: s.jitter / f64::from(s.clock.max(1)) }
            })
            .collect();
        v.sort_by_key(|x| x.ssrc);
        v
    }

    /// 경로표에서 빠진 줄 · 다른 트랙이 실린 줄의 상태를 버린다
    pub(crate) fn retain(&mut self, live: impl Fn(u32, &str) -> bool) {
        self.streams.retain(|k, s| live(*k, &s.route.track_id));
    }

    /// 시각이 지나 할 일 — 포기한 틈 넘기기 · NACK · PLI · RR. 낼 프레임과 보낼 RTCP(평문 · 묶음 하나)를 돌려준다
    pub(crate) fn tick(&mut self, now: Instant) -> (Vec<Out>, Option<Vec<u8>>) {
        let mut out = Vec::new();
        let c = self.counters.clone();
        let mut fb: Vec<u8> = Vec::new();
        let mut keys: Vec<&u32> = self.streams.keys().collect();
        keys.sort();
        let keys: Vec<u32> = keys.into_iter().copied().collect();
        let record = self.record;
        for k in &keys {
            let Some(s) = self.streams.get_mut(k) else { continue };
            s.release(now, &c, &mut out, record);
            if s.route.nack {
                let mut seqs = Vec::new();
                for (e, m) in s.missing.iter_mut() {
                    if m.tries < NACK_TRIES && m.sent.is_none_or(|t| now.duration_since(t) >= NACK_INTERVAL) {
                        m.sent = Some(now);
                        m.tries += 1;
                        seqs.push(*e as u16);
                    }
                }
                if !seqs.is_empty() {
                    Counters::add(&c.nack_sent, 1);
                    fb.extend(nack(self.me, s.route.ssrc, &seqs));
                }
            }
            if s.video && s.route.pli {
                let late = s.key_wait.is_some_and(|t| now >= t);
                if (s.want_key || late || s.ask_key) && s.pli_at.is_none_or(|t| now.duration_since(t) >= PLI_INTERVAL) {
                    s.pli_at = Some(now);
                    s.ask_key = false;
                    if late {
                        s.key_wait = None;
                    }
                    Counters::add(&c.pli_sent, 1);
                    fb.extend(pli(self.me, s.route.ssrc));
                }
            }
        }
        let rr_due = self.rr_at.is_none_or(|t| now.duration_since(t) >= RR_INTERVAL);
        let mut blocks = Vec::new();
        if rr_due {
            self.rr_at = Some(now);
            for k in &keys {
                if let Some(b) = self.streams.get_mut(k).and_then(|s| s.report(now)) {
                    blocks.push(b);
                }
            }
        }
        if fb.is_empty() && blocks.is_empty() {
            return (out, None);
        }
        // 묶음은 RR 로 시작한다(RFC 3550 §6.1) — 보고할 것이 없으면 빈 RR
        let mut pkt = Vec::new();
        let mut chunks = blocks.chunks(31).peekable();
        if chunks.peek().is_none() {
            pkt.extend(rr(self.me, &[]));
        }
        for ch in chunks {
            pkt.extend(rr(self.me, ch));
        }
        if !blocks.is_empty() {
            Counters::add(&c.rr_sent, 1);
        }
        pkt.extend(fb);
        (out, Some(pkt))
    }
}

fn head(fmt: u8, pt: u8, words: usize) -> Vec<u8> {
    let mut b = Vec::with_capacity((words + 1) * 4);
    b.push(0x80 | (fmt & 0x1f));
    b.push(pt);
    b.extend_from_slice(&(words as u16).to_be_bytes());
    b
}

fn rr(me: u32, blocks: &[[u8; 24]]) -> Vec<u8> {
    let mut b = head(blocks.len() as u8, 201, 1 + 6 * blocks.len());
    b.extend_from_slice(&me.to_be_bytes());
    for x in blocks {
        b.extend_from_slice(x);
    }
    b
}

/// RFC 4585 §6.2.1 일반 NACK — PID + 뒤 16개 비트
fn nack(me: u32, media: u32, seqs: &[u16]) -> Vec<u8> {
    let mut fci: Vec<(u16, u16)> = Vec::new();
    for &s in seqs {
        match fci.last_mut() {
            Some((pid, blp)) if (1..=16).contains(&s.wrapping_sub(*pid)) => *blp |= 1 << (s.wrapping_sub(*pid) - 1),
            _ => fci.push((s, 0)),
        }
    }
    let mut b = head(1, 205, 2 + fci.len());
    b.extend_from_slice(&me.to_be_bytes());
    b.extend_from_slice(&media.to_be_bytes());
    for (pid, blp) in fci {
        b.extend_from_slice(&pid.to_be_bytes());
        b.extend_from_slice(&blp.to_be_bytes());
    }
    b
}

/// RFC 4585 §6.3.1 PLI
fn pli(me: u32, media: u32) -> Vec<u8> {
    let mut b = head(1, 206, 2);
    b.extend_from_slice(&me.to_be_bytes());
    b.extend_from_slice(&media.to_be_bytes());
    b
}

/// 받은 RTCP 묶음에서 SR 만 — (ssrc, NTP, RTP)
pub(crate) fn sender_reports(b: &[u8]) -> Vec<(u32, u64, u32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= b.len() && b[i] >> 6 == 2 {
        let len = (usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]])) + 1) * 4;
        let Some(p) = b.get(i..i + len) else { break };
        if p[1] == 200 && p.len() >= 24 {
            let ssrc = u32::from_be_bytes([p[4], p[5], p[6], p[7]]);
            let ntp = u64::from_be_bytes([p[8], p[9], p[10], p[11], p[12], p[13], p[14], p[15]]);
            let rtp = u32::from_be_bytes([p[16], p[17], p[18], p[19]]);
            out.push((ssrc, ntp, rtp));
        }
        i += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(kind: &str, codec: &str, unit: Unit) -> Route {
        let video = kind == "video";
        Route { room: "r".into(), track_id: "t".into(), user_id: Some("u".into()), kind: kind.into(), codec: codec.into(), mid: "1".into(), ssrc: 1111, pt: 96, nack: video, pli: video, unit, layer_gen: 0, key_gen: 0 }
    }

    fn rtp(seq: u16, ts: u32, marker: bool, payload: &[u8]) -> Rtp {
        Rtp { seq, ts, marker, pt: 96, payload: Bytes::copy_from_slice(payload), recovered: false, source: None, raw: None }
    }

    fn frames(o: &[Out]) -> Vec<(u32, bool, Vec<u8>)> {
        o.iter()
            .filter_map(|x| match x {
                Out::Frame(f) => Some((f.timestamp, f.keyframe, f.data.to_vec())),
                _ => None,
            })
            .collect()
    }

    /// RTCP 묶음을 (pt, fmt, 본문) 으로
    fn split(b: &[u8]) -> Vec<(u8, u8, Vec<u8>)> {
        let mut v = Vec::new();
        let mut i = 0;
        while i + 4 <= b.len() {
            let len = (usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]])) + 1) * 4;
            v.push((b[i + 1], b[i] & 0x1f, b[i + 4..i + len].to_vec()));
            i += len;
        }
        v
    }

    const VP8_KEY_A: [u8; 3] = [0x10, 0x00, 0xaa];
    const VP8_CONT: [u8; 2] = [0x00, 0xbb];
    const VP8_DELTA: [u8; 2] = [0x10, 0x01];

    #[test]
    fn reorders_and_assembles_vp8() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        let rt = route("video", "VP8", Unit::Frame);
        let t0 = Instant::now();
        let w = SystemTime::now();
        // 키프레임 두 조각이 뒤집혀 온다 → 둘째가 먼저 와도 첫째가 오면 순서대로 한 프레임
        assert!(frames(&r.on_rtp(&rt, rtp(10, 900, false, &VP8_KEY_A), t0, w)).is_empty());
        let o = r.on_rtp(&rt, rtp(12, 3900, true, &VP8_DELTA), t0, w);
        assert!(frames(&o).is_empty());
        let o = r.on_rtp(&rt, rtp(11, 900, true, &VP8_CONT), t0, w);
        assert_eq!(frames(&o), vec![(900, true, vec![0x00, 0xaa, 0xbb]), (3900, false, vec![0x01])]);
        assert_eq!(c.lost.load(Ordering::Relaxed), 0);
    }

    fn tagged(seq: u16, ts: u32, marker: bool, payload: &[u8], csrc: u32) -> Rtp {
        Rtp { source: Some(Source { csrc, user_id: Some(format!("u{csrc}")), track_id: None }), ..rtp(seq, ts, marker, payload) }
    }

    #[test]
    fn frame_carries_source_and_breaks_on_mixed_source() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        let rt = route("video", "VP8", Unit::Frame);
        let (t0, w) = (Instant::now(), SystemTime::now());
        r.on_rtp(&rt, tagged(10, 900, false, &VP8_KEY_A, 3), t0, w);
        let o = r.on_rtp(&rt, tagged(11, 900, true, &VP8_CONT, 3), t0, w);
        let src: Vec<_> = o.iter().filter_map(|x| if let Out::Frame(f) = x { Some(f.source.as_ref().map(|s| s.csrc)) } else { None }).collect();
        assert_eq!(src, vec![Some(3)]);
        r.on_rtp(&rt, tagged(12, 1800, false, &VP8_KEY_A, 3), t0, w);
        let o = r.on_rtp(&rt, tagged(13, 1800, true, &VP8_CONT, 4), t0, w);
        assert!(frames(&o).is_empty(), "조각마다 출처가 갈리면 프레임을 내지 않는다");
        assert_eq!(c.frames_broken.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn nack_then_rtx_fills_gap() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        let rt = route("video", "VP8", Unit::Frame);
        let t0 = Instant::now();
        let w = SystemTime::now();
        r.on_rtp(&rt, rtp(100, 0, true, &[0x10, 0x00]), t0, w);
        r.on_rtp(&rt, rtp(103, 9000, true, &VP8_DELTA), t0, w);
        let (_, fb) = r.tick(t0);
        let fb = split(&fb.unwrap());
        // RR(블록 하나) 다음 NACK 하나 — PID 101 · BLP 비트 0(=102)
        assert_eq!(fb[0].0, 201);
        let n = fb.iter().find(|x| x.0 == 205 && x.1 == 1).unwrap();
        assert_eq!(&n.2[4..8], &1111u32.to_be_bytes());
        assert_eq!(&n.2[8..12], &[0, 101, 0, 1]);
        // 100ms 안에는 다시 묻지 않는다
        let (_, fb) = r.tick(t0 + Duration::from_millis(50));
        assert!(fb.is_none());
        let mut x = rtp(101, 3000, true, &VP8_DELTA);
        x.recovered = true;
        let a = r.on_rtp(&rt, x, t0 + Duration::from_millis(60), w);
        let mut y = rtp(102, 6000, true, &VP8_DELTA);
        y.recovered = true;
        let b = r.on_rtp(&rt, y, t0 + Duration::from_millis(60), w);
        let ts: Vec<u32> = frames(&a).into_iter().chain(frames(&b)).map(|f| f.0).collect();
        assert_eq!(ts, vec![3000, 6000, 9000]);
        assert_eq!(c.rtx_recovered.load(Ordering::Relaxed), 2);
        assert_eq!(c.lost.load(Ordering::Relaxed), 0);
    }

    fn raw(seq: u16, ts: u32, marker: bool, payload: &[u8]) -> Rtp {
        let mut v = vec![0x80, 0x60, 0, 0, 0, 0, 0, 0, 0, 0, 0x04, 0x57];
        v[2..4].copy_from_slice(&seq.to_be_bytes());
        v.extend_from_slice(payload);
        Rtp { raw: Some(Bytes::from(v)), ..rtp(seq, ts, marker, payload) }
    }

    fn raw_seqs(o: &[Out]) -> Vec<u16> {
        o.iter().filter_map(|x| if let Out::Raw { data, .. } = x { Some(u16::from_be_bytes([data[2], data[3]])) } else { None }).collect()
    }

    fn gaps(o: &[Out]) -> Vec<Gap> {
        o.iter().filter_map(|x| if let Out::Gap(g) = x { Some(g.clone()) } else { None }).collect()
    }

    #[test]
    fn record_exit_is_in_order_with_gap_for_given_up_run() {
        // SDK:RULE-PLAT-REC-TAP — 출구는 차례가 선 뒤 · 포기한 번호가 이어지면 빈 번호 사건 하나
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        r.record = true;
        let rt = route("video", "VP8", Unit::Packet);
        let t0 = Instant::now();
        let w = SystemTime::now();
        let mut o = r.on_rtp(&rt, raw(1, 0, true, &[0x10, 0x00]), t0, w);
        o.extend(r.on_rtp(&rt, raw(5, 3000, true, &VP8_DELTA), t0, w));
        o.extend(r.on_rtp(&rt, raw(3, 1500, true, &VP8_DELTA), t0, w));
        assert_eq!(raw_seqs(&o), vec![1], "도착 차례(1 · 5 · 3)가 아니라 출구 차례 — 2 가 비어 3 · 5 는 아직 못 나간다");
        let mut t = t0;
        for _ in 0..15 {
            let (x, _) = r.tick(t);
            o.extend(x);
            t += NACK_INTERVAL;
        }
        assert_eq!(raw_seqs(&o), vec![1, 3, 5]);
        assert_eq!(gaps(&o), vec![Gap { ssrc: 1111, from: 2, to: 2, count: 1, reason: "nack_exhausted" }, Gap { ssrc: 1111, from: 4, to: 4, count: 1, reason: "nack_exhausted" }]);
        assert_eq!(c.lost.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn record_off_has_no_raw_or_gap() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c);
        let rt = route("audio", "opus", Unit::Packet);
        let t0 = Instant::now();
        let mut o = r.on_rtp(&rt, raw(1, 0, true, &[1]), t0, SystemTime::now());
        o.extend(r.on_rtp(&rt, raw(3, 960, true, &[1]), t0, SystemTime::now()));
        o.extend(r.tick(t0 + HOLD_NO_NACK * 2).0);
        assert!(raw_seqs(&o).is_empty() && gaps(&o).is_empty());
    }

    #[test]
    fn record_huge_jump_is_stream_reset() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c);
        r.record = true;
        let rt = route("audio", "opus", Unit::Packet);
        let t0 = Instant::now();
        let mut o = r.on_rtp(&rt, raw(10, 0, true, &[1]), t0, SystemTime::now());
        o.extend(r.on_rtp(&rt, raw(5_000, 960, true, &[1]), t0, SystemTime::now()));
        assert_eq!(gaps(&o), vec![Gap { ssrc: 1111, from: 11, to: 4_999, count: 4_989, reason: "stream_reset" }]);
        assert_eq!(raw_seqs(&o), vec![10, 5_000]);
    }

    #[test]
    fn gives_up_then_waits_key_and_plis() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        let rt = route("video", "VP8", Unit::Frame);
        let t0 = Instant::now();
        let w = SystemTime::now();
        r.on_rtp(&rt, rtp(1, 0, true, &[0x10, 0x00]), t0, w);
        r.on_rtp(&rt, rtp(3, 6000, true, &VP8_DELTA), t0, w);
        let mut t = t0;
        let (mut nacks, mut plis) = (0, 0);
        for _ in 0..15 {
            let (_, fb) = r.tick(t);
            let fb = fb.map(|b| split(&b)).unwrap_or_default();
            nacks += fb.iter().filter(|x| x.0 == 205).count();
            plis += fb.iter().filter(|x| x.0 == 206 && x.1 == 1).count();
            t += NACK_INTERVAL;
        }
        // 열 번 묻고 포기 → 그 뒤 델타는 버리고 PLI 한 번(간격 1s 안에는 다시 안 낸다)
        assert_eq!(nacks, usize::from(NACK_TRIES));
        assert_eq!(c.lost.load(Ordering::Relaxed), 1);
        assert_eq!(c.frames_skipped.load(Ordering::Relaxed), 1);
        assert_eq!(plis, 1);
        let (_, fb) = r.tick(t0 + NACK_INTERVAL * 10 + PLI_INTERVAL);
        assert!(split(&fb.unwrap()).iter().any(|x| x.0 == 206));
        // 키프레임이 오면 다시 낸다
        let o = r.on_rtp(&rt, rtp(4, 9000, true, &[0x10, 0x00]), t, w);
        assert_eq!(frames(&o), vec![(9000, true, vec![0x00])]);
    }

    #[test]
    fn starts_with_pli_when_first_frame_is_delta() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        let rt = route("video", "VP8", Unit::Frame);
        let t0 = Instant::now();
        let o = r.on_rtp(&rt, rtp(50, 0, true, &VP8_DELTA), t0, SystemTime::now());
        assert!(frames(&o).is_empty());
        let (_, fb) = r.tick(t0);
        assert!(split(&fb.unwrap()).iter().any(|x| x.0 == 206));
    }

    #[test]
    fn no_pli_while_first_key_is_assembling() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        let rt = route("video", "VP8", Unit::Frame);
        let t0 = Instant::now();
        let w = SystemTime::now();
        r.on_rtp(&rt, rtp(1, 0, false, &VP8_KEY_A), t0, w);
        let (_, fb) = r.tick(t0);
        assert!(fb.is_none_or(|b| !split(&b).iter().any(|x| x.0 == 206)));
        let o = r.on_rtp(&rt, rtp(2, 0, true, &VP8_CONT), t0, w);
        assert_eq!(frames(&o).len(), 1);
        assert_eq!(c.pli_sent.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn layer_change_without_key_asks_once() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        let mut rt = route("video", "VP8", Unit::Frame);
        let t0 = Instant::now();
        let w = SystemTime::now();
        r.on_rtp(&rt, rtp(1, 0, true, &[0x10, 0x00]), t0, w);
        r.tick(t0);
        rt.layer_gen = 1;
        r.on_rtp(&rt, rtp(2, 3000, true, &VP8_DELTA), t0, w);
        let (_, fb) = r.tick(t0 + Duration::from_millis(500));
        assert!(fb.is_none_or(|b| !split(&b).iter().any(|x| x.0 == 206)));
        let (_, fb) = r.tick(t0 + LAYER_KEY_WAIT);
        assert!(split(&fb.unwrap()).iter().any(|x| x.0 == 206));
        assert!(r.tick(t0 + LAYER_KEY_WAIT * 3).1.is_none_or(|b| !split(&b).iter().any(|x| x.0 == 206)));
    }

    #[test]
    fn app_keyframe_request_sends_one_pli_respecting_interval() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        let mut rt = route("video", "VP8", Unit::Frame);
        let t0 = Instant::now();
        let w = SystemTime::now();
        r.on_rtp(&rt, rtp(1, 0, true, &[0x10, 0x00]), t0, w);
        let (_, fb) = r.tick(t0);
        assert!(fb.is_none_or(|b| !split(&b).iter().any(|x| x.0 == 206)), "키프레임을 받았으니 묻지 않는다");
        rt.key_gen = 1;
        r.on_rtp(&rt, rtp(2, 3000, true, &VP8_DELTA), t0, w);
        let (_, fb) = r.tick(t0);
        assert!(split(&fb.unwrap()).iter().any(|x| x.0 == 206), "앱이 요청하면 다음 차례에 PLI");
        assert!(r.tick(t0 + Duration::from_millis(100)).1.is_none_or(|b| !split(&b).iter().any(|x| x.0 == 206)), "한 번만");
        rt.key_gen = 2;
        r.on_rtp(&rt, rtp(3, 6000, true, &VP8_DELTA), t0, w);
        assert!(r.tick(t0 + Duration::from_millis(200)).1.is_none_or(|b| !split(&b).iter().any(|x| x.0 == 206)), "간격(1s) 안에는 미룬다");
        assert!(split(&r.tick(t0 + PLI_INTERVAL).1.unwrap()).iter().any(|x| x.0 == 206), "간격이 지나면 낸다");
    }

    #[test]
    fn audio_no_nack_short_hold_and_packet_unit_is_arrival_order() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        let rt = route("audio", "opus", Unit::Packet);
        let t0 = Instant::now();
        let w = SystemTime::now();
        let seqs: Vec<u16> = [5u16, 7, 6]
            .iter()
            .flat_map(|&s| {
                r.on_rtp(&rt, rtp(s, u32::from(s) * 960, false, &[0xf8]), t0, w)
                    .into_iter()
                    .filter_map(|o| match o {
                        Out::Packet(p) => Some(p.sequence),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(seqs, vec![5, 7, 6]);
        r.on_rtp(&rt, rtp(9, 9 * 960, false, &[0xf8]), t0, w);
        let (_, fb) = r.tick(t0 + HOLD_NO_NACK);
        assert!(!split(&fb.unwrap()).iter().any(|x| x.0 == 205 || x.0 == 206));
        assert_eq!(c.lost.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn sr_maps_rtp_to_wallclock() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c);
        let rt = route("audio", "opus", Unit::Frame);
        let t0 = Instant::now();
        let got = UNIX_EPOCH + Duration::from_secs(5);
        let o = r.on_rtp(&rt, rtp(1, 48_000, false, &[0xf8]), t0, got);
        let Out::Frame(f) = &o[0] else { panic!() };
        assert!(!f.synced);
        assert_eq!(f.wallclock, got);
        // NTP 1_700_000_000 + 0.5 초 ↔ RTP 0 → RTP 48000(1초 뒤)는 +1s
        let ntp = ((1_700_000_000 + NTP_UNIX) << 32) | 0x8000_0000;
        r.on_sr(1111, ntp, 0, t0);
        let o = r.on_rtp(&rt, rtp(2, 48_000, false, &[0xf8]), t0, got);
        let Out::Frame(f) = &o[0] else { panic!() };
        assert!(f.synced);
        assert_eq!(f.wallclock, UNIX_EPOCH + Duration::from_millis(1_700_000_001_500));
    }

    #[test]
    fn rr_block_counts_loss_but_not_recovery() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c);
        let rt = route("video", "VP8", Unit::Packet);
        let t0 = Instant::now();
        let w = SystemTime::now();
        for s in [0u16, 1, 3] {
            r.on_rtp(&rt, rtp(s, u32::from(s) * 3000, true, &[0x10, 0x00]), t0, w);
        }
        let mut x = rtp(2, 6000, true, &VP8_DELTA);
        x.recovered = true;
        r.on_rtp(&rt, x, t0, w);
        let (_, fb) = r.tick(t0);
        let fb = split(&fb.unwrap());
        let rrb = &fb[0];
        assert_eq!((rrb.0, rrb.1), (201, 1));
        let b = &rrb.2[4..28];
        assert_eq!(&b[0..4], &1111u32.to_be_bytes());
        // 4 기대 · 3 받음(복구는 망 손실로 센다) → 1/4 = 64/256 · 누적 1 · 최고 seq 3
        assert_eq!(b[4], 64);
        assert_eq!(&b[5..8], &[0, 0, 1]);
        assert_eq!(&b[8..12], &3u32.to_be_bytes());
    }

    #[test]
    fn huge_gap_resets_without_nack_storm() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        let rt = route("video", "VP8", Unit::Frame);
        let t0 = Instant::now();
        let w = SystemTime::now();
        r.on_rtp(&rt, rtp(1, 0, true, &[0x10, 0x00]), t0, w);
        let o = r.on_rtp(&rt, rtp(5000, 90_000, true, &[0x10, 0x00]), t0, w);
        assert_eq!(frames(&o).len(), 1);
        let (_, fb) = r.tick(t0);
        assert!(!split(&fb.unwrap()).iter().any(|x| x.0 == 205));
    }

    #[test]
    fn seq_wraps() {
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        let rt = route("audio", "opus", Unit::Frame);
        let t0 = Instant::now();
        let w = SystemTime::now();
        let mut n = 0;
        for s in [65534u16, 65535, 0, 1] {
            n += frames(&r.on_rtp(&rt, rtp(s, 0, false, &[0xf8]), t0, w)).len();
        }
        assert_eq!(n, 4);
        assert_eq!(c.lost.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn record_exit_and_gap_cross_seq_wrap() {
        // 설계 20261109b §10-3 「seq 넘침도 같은 방식」(202611/20261110d T2) — 65534 · (65535 · 0 잃음) · 1 → 출구 차례 65534 · 1 · 빈 번호 사건 하나가 넘침을 가로지른다
        let c = Arc::new(Counters::default());
        let mut r = Recv::new(9, c.clone());
        r.record = true;
        let rt = route("video", "VP8", Unit::Packet);
        let t0 = Instant::now();
        let w = SystemTime::now();
        let mut o = r.on_rtp(&rt, raw(65534, 0, true, &[0x10, 0x00]), t0, w);
        o.extend(r.on_rtp(&rt, raw(1, 9000, true, &VP8_DELTA), t0, w));
        let mut t = t0;
        for _ in 0..15 {
            o.extend(r.tick(t).0);
            t += NACK_INTERVAL;
        }
        assert_eq!(raw_seqs(&o), vec![65534, 1]);
        assert_eq!(gaps(&o), vec![Gap { ssrc: 1111, from: 65535, to: 0, count: 2, reason: "nack_exhausted" }], "넘침을 가로지른 빈 번호는 범위 하나(스트림 재설정이 아니다)");
        assert_eq!(c.lost.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn parses_sender_report() {
        let mut b = head(0, 200, 6);
        b.extend_from_slice(&1111u32.to_be_bytes());
        b.extend_from_slice(&0x0102_0304_0506_0708u64.to_be_bytes());
        b.extend_from_slice(&77u32.to_be_bytes());
        b.extend_from_slice(&[0; 8]);
        b.extend(rr(5, &[]));
        assert_eq!(sender_reports(&b), vec![(1111, 0x0102_0304_0506_0708, 77)]);
    }
}
