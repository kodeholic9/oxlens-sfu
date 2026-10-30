// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-RTCP-TERMINATE · 정:RULE-RTCP-NACK-UP · 정:RULE-RTCP-NACK-DOWN · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! RTCP 종단 — 서버가 짓는 것(Ingress RR 1000ms · TWCC 100ms · REMB 1000ms · NACK 상향 · PLI 스로틀) · 구독자 것 소비(RR · NACK → RTX · PLI · TWCC) · 발행자 SR 번역 · `1pc` 분해. sender SSRC 1.

pub mod core;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use bytes::Bytes;
use oxsig::msg::Kind;

use crate::fwd::rt::{Item, Worker, media, mono_us};
use crate::fwd::{Ctl, DownRow, parse_rtp};
use crate::link::{PathKey, Transport};
use crate::r#pub::PubTrack;
use crate::sub::ext::{ExtInput, build as ext_build};
use core::TwccRecv;

pub const SENDER_SSRC: u32 = 1;
pub const RR_MS: u64 = 1_000;
pub const TWCC_MS: u64 = 100;
pub const REMB_MS: u64 = 1_000;
pub const SIM_MS: u64 = 1_000;
pub const PROBE_US: u64 = 20_000;
pub const RTX_BUDGET: u32 = 200;
pub const RTX_WINDOW_MS: u64 = 3_000;
pub const SENT_KEEP: usize = 1 << 15;
pub const PLI_BURST_MS: [u64; 3] = [0, 500, 1_500];

#[derive(Debug, Clone, Copy, Default)]
pub struct Sr {
    pub ntp: u64,
    pub rtp: u32,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Sent {
    pub seq: u16,
    pub at_us: u64,
    pub size: usize,
    pub probe: bool,
}

#[derive(Default)]
pub struct ConnRtcp {
    pub twcc_rx: Mutex<TwccRecv>,
    pub media_ssrc: AtomicU32,
    pub next_rr: AtomicU64,
    pub next_twcc: AtomicU64,
    pub next_remb: AtomicU64,
    pub next_keepalive: AtomicU64,
    pub next_sim: AtomicU64,
    pub next_probe_us: AtomicU64,
    pub sr: Mutex<HashMap<u32, Sr>>,
    pub sent: Mutex<VecDeque<Sent>>,
    pub sim: Mutex<crate::sim::ConnSim>,
}

static BUDGET: OnceLock<Mutex<HashMap<String, (u64, u32)>>> = OnceLock::new();

fn budget_ok(user: &str, now: u64) -> bool {
    let m = BUDGET.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut g) = m.lock() else { return true };
    let e = g.entry(user.to_string()).or_insert((now, 0));
    if now.saturating_sub(e.0) >= RTX_WINDOW_MS {
        e.0 = now;
        e.1 = 0;
    }
    if e.1 >= RTX_BUDGET {
        return false;
    }
    e.1 += 1;
    true
}

pub fn forget_user(user: &str) {
    if let Some(m) = BUDGET.get()
        && let Ok(mut g) = m.lock()
    {
        g.remove(user);
    }
}

fn header(fmt: u8, pt: u8, words: usize) -> Vec<u8> {
    let mut b = Vec::with_capacity((words + 1) * 4);
    b.push(0x80 | (fmt & 0x1f));
    b.push(pt);
    b.extend_from_slice(&(words as u16).to_be_bytes());
    b
}

fn fix_len(b: &mut [u8]) {
    let words = (b.len() / 4) as u16 - 1;
    b[2..4].copy_from_slice(&words.to_be_bytes());
}

pub fn build_rr(ssrc: u32, r: &core::Report, lsr: u32, dlsr: u32) -> Vec<u8> {
    let mut b = header(1, 201, 7);
    b.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    b.extend_from_slice(&ssrc.to_be_bytes());
    let lost = r.lost.clamp(-0x80_0000, 0x7f_ffff) as u32 & 0x00ff_ffff;
    b.push(r.fraction);
    b.extend_from_slice(&lost.to_be_bytes()[1..4]);
    b.extend_from_slice(&r.extended_max.to_be_bytes());
    b.extend_from_slice(&r.jitter.to_be_bytes());
    b.extend_from_slice(&lsr.to_be_bytes());
    b.extend_from_slice(&dlsr.to_be_bytes());
    b
}

pub fn build_nack(media: u32, seqs: &[u16]) -> Vec<u8> {
    let mut sorted = seqs.to_vec();
    sorted.sort_by_key(|s| *s);
    let mut fci: Vec<(u16, u16)> = Vec::new();
    for s in sorted {
        match fci.last_mut() {
            Some((pid, blp)) if s.wrapping_sub(*pid) >= 1 && s.wrapping_sub(*pid) <= 16 => {
                let bit = s.wrapping_sub(*pid) - 1;
                let _ = std::mem::replace(blp, *blp | (1 << bit));
            }
            _ => fci.push((s, 0)),
        }
    }
    let mut b = header(1, 205, 2 + fci.len());
    b.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    b.extend_from_slice(&media.to_be_bytes());
    for (pid, blp) in fci {
        b.extend_from_slice(&pid.to_be_bytes());
        b.extend_from_slice(&blp.to_be_bytes());
    }
    b
}

pub fn build_pli(media: u32) -> Vec<u8> {
    let mut b = header(1, 206, 2);
    b.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    b.extend_from_slice(&media.to_be_bytes());
    b
}

pub fn build_remb(bps: u64, ssrcs: &[u32]) -> Vec<u8> {
    let mut exp = 0u8;
    let mut mant = bps;
    while mant > 0x3ffff {
        mant >>= 1;
        exp += 1;
    }
    let mut b = header(15, 206, 4 + ssrcs.len());
    b.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    b.extend_from_slice(&0u32.to_be_bytes());
    b.extend_from_slice(b"REMB");
    b.push(ssrcs.len() as u8);
    b.push((exp << 2) | ((mant >> 16) as u8 & 0x03));
    b.extend_from_slice(&(mant as u16).to_be_bytes());
    for s in ssrcs {
        b.extend_from_slice(&s.to_be_bytes());
    }
    b
}

pub fn build_twcc(media: u32, f: &core::Feedback) -> Vec<u8> {
    let mut b = header(15, 205, 0);
    b.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    b.extend_from_slice(&media.to_be_bytes());
    b.extend_from_slice(&f.base_seq.to_be_bytes());
    b.extend_from_slice(&f.status_count.to_be_bytes());
    let rt = ((f.reference_time & 0x00ff_ffff) << 8) | f.fb_count as u32;
    b.extend_from_slice(&rt.to_be_bytes());
    let mut i = 0;
    while i < f.symbols.len() {
        let sym = f.symbols[i];
        let mut n = 1;
        while i + n < f.symbols.len() && f.symbols[i + n] == sym && n < 0x1fff {
            n += 1;
        }
        let chunk = ((sym as u16 & 0x03) << 13) | (n as u16);
        b.extend_from_slice(&chunk.to_be_bytes());
        i += n;
    }
    let mut di = 0;
    for &s in &f.symbols {
        match s {
            core::SYM_SMALL => {
                b.push(f.deltas_250us[di] as u8);
                di += 1;
            }
            core::SYM_LARGE => {
                b.extend_from_slice(&(f.deltas_250us[di] as i16).to_be_bytes());
                di += 1;
            }
            _ => {}
        }
    }
    let pad = (4 - b.len() % 4) % 4;
    if pad > 0 {
        b.extend(std::iter::repeat_n(0u8, pad - 1));
        b.push(pad as u8);
        b[0] |= 0x20;
    }
    fix_len(&mut b);
    b
}

pub fn build_sr(ssrc: u32, sr: &Sr, rtp: u32, packets: u32, octets: u32) -> Vec<u8> {
    let mut b = header(0, 200, 6);
    b.extend_from_slice(&ssrc.to_be_bytes());
    b.extend_from_slice(&sr.ntp.to_be_bytes());
    b.extend_from_slice(&rtp.to_be_bytes());
    b.extend_from_slice(&packets.to_be_bytes());
    b.extend_from_slice(&octets.to_be_bytes());
    b
}

pub struct Packet<'a> {
    pub fmt: u8,
    pub pt: u8,
    pub body: &'a [u8],
}

pub fn split(b: &[u8]) -> Vec<Packet<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= b.len() {
        if b[i] >> 6 != 2 {
            break;
        }
        let len = (u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize + 1) * 4;
        if i + len > b.len() {
            break;
        }
        out.push(Packet { fmt: b[i] & 0x1f, pt: b[i + 1], body: &b[i + 4..i + len] });
        i += len;
    }
    out
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]))
}

pub fn parse_nack(body: &[u8]) -> Option<(u32, Vec<u16>)> {
    let media = be32(body, 4)?;
    let mut seqs = Vec::new();
    let mut i = 8;
    while i + 4 <= body.len() {
        let pid = u16::from_be_bytes([body[i], body[i + 1]]);
        let blp = u16::from_be_bytes([body[i + 2], body[i + 3]]);
        seqs.push(pid);
        for k in 0..16 {
            if blp & (1 << k) != 0 {
                seqs.push(pid.wrapping_add(k + 1));
            }
        }
        i += 4;
    }
    Some((media, seqs))
}

pub fn parse_twcc(body: &[u8]) -> Option<(u16, Vec<Option<i64>>)> {
    let base = u16::from_be_bytes([*body.get(8)?, *body.get(9)?]);
    let count = u16::from_be_bytes([*body.get(10)?, *body.get(11)?]) as usize;
    let rt = be32(body, 12)? >> 8;
    let rt = if rt & 0x80_0000 != 0 { rt as i64 - 0x100_0000 } else { rt as i64 };
    let mut syms = Vec::with_capacity(count);
    let mut i = 16;
    while syms.len() < count {
        let c = u16::from_be_bytes([*body.get(i)?, *body.get(i + 1)?]);
        i += 2;
        if c & 0x8000 == 0 {
            let s = ((c >> 13) & 0x03) as u8;
            let n = (c & 0x1fff) as usize;
            for _ in 0..n.min(count - syms.len()) {
                syms.push(s);
            }
        } else if c & 0x4000 == 0 {
            for k in (0..14).rev() {
                if syms.len() < count {
                    syms.push(((c >> k) & 0x01) as u8);
                }
            }
        } else {
            for k in (0..7).rev() {
                if syms.len() < count {
                    syms.push(((c >> (k * 2)) & 0x03) as u8);
                }
            }
        }
    }
    let mut t = rt * 64_000;
    let mut out = Vec::with_capacity(count);
    for s in syms {
        match s {
            1 => {
                t += *body.get(i)? as i64 * 250;
                i += 1;
                out.push(Some(t));
            }
            2 => {
                t += i16::from_be_bytes([*body.get(i)?, *body.get(i + 1)?]) as i64 * 250;
                i += 2;
                out.push(Some(t));
            }
            _ => out.push(None),
        }
    }
    Some((base, out))
}

pub fn emit(tr: &Arc<Transport>, data: Vec<u8>) {
    let Some(m) = media() else { return };
    let Some(w) = m.workers.get(tr.worker) else { return };
    let from = m.workers.len() + 1;
    if !w.push(from, Item::Ctl(Ctl::Rtcp { tr: tr.clone(), data })) {
        crate::ops::drop_count("egress_queue_full");
    }
}

pub fn send_rtcp(w: &Worker, tr: &Transport, data: &[u8]) {
    let Some(m) = media() else { return };
    if tr.closed.load(Ordering::Acquire) || !tr.ready.load(Ordering::Acquire) {
        return;
    }
    let Some(path) = tr.send_ok(common::now_ms(), |id| m.tcp_open(id)) else { return };
    let enc = match tr.srtp_tx.lock() {
        Ok(mut g) => match g.as_mut() {
            Some(c) => c.encrypt_rtcp(data),
            None => return,
        },
        Err(_) => return,
    };
    if let Ok(e) = enc
        && !m.send_on(&w.sock, path, &e)
    {
        crate::ops::drop_count("send_fail");
    }
}

fn throttle_ms(t: &PubTrack) -> u64 {
    let p = crate::ops::policy();
    if t.rid == Some(0) { p.media.pli_throttle_low_ms as u64 } else { p.media.pli_throttle_high_ms as u64 }
}

pub fn request_pli(t: &Arc<PubTrack>, infra: bool) {
    if t.stream.kind != Kind::Video {
        return;
    }
    let now = common::now_ms();
    let gap = throttle_ms(t);
    let fire = {
        let Ok(mut p) = t.pli.lock() else { return };
        if p.last_ms == 0 || now.saturating_sub(p.last_ms) >= gap {
            p.last_ms = now;
            p.pending = false;
            p.sent += 1;
            true
        } else {
            if infra {
                p.pending = true;
            } else {
                crate::ops::drop_count("pli_throttled");
            }
            false
        }
    };
    if fire && let Some(tr) = t.transport.upgrade() {
        emit(&tr, build_pli(t.ssrc));
    }
}

pub fn pli_burst(t: &Arc<PubTrack>) {
    for d in PLI_BURST_MS {
        let t = t.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(d)).await;
            request_pli(&t, true);
        });
    }
}

pub fn twcc_in(tr: &Transport, seq: u16, at_us: i64) {
    if let Ok(mut g) = tr.rtc.twcc_rx.lock() {
        g.record(seq, at_us);
    }
}

pub fn sent_twcc(conn: &Transport, seq: u16, size: usize, now_us: u64, probe: bool) {
    if let Ok(mut g) = conn.rtc.sent.lock() {
        g.push_back(Sent { seq, at_us: now_us, size, probe });
        while g.len() > SENT_KEEP {
            g.pop_front();
        }
    }
}

fn uplink_tracks(tr: &Transport) -> Vec<Arc<PubTrack>> {
    let ing = tr.ingress.load();
    let mut v: Vec<Arc<PubTrack>> = ing.iter().filter(|(k, t)| **k == t.ssrc).map(|(_, t)| t.clone()).collect();
    v.sort_by_key(|t| t.ssrc);
    v
}

fn due(slot: &AtomicU64, now: u64, every: u64) -> bool {
    let n = slot.load(Ordering::Relaxed);
    if now >= n {
        slot.store(now + every, Ordering::Relaxed);
        return true;
    }
    false
}

pub fn tick(w: &Arc<Worker>, now_us: u64) {
    let trs: Vec<Arc<Transport>> = match w.transports.lock() {
        Ok(mut g) => {
            g.retain(|t| t.strong_count() > 0);
            g.iter().filter_map(|t| t.upgrade()).collect()
        }
        Err(_) => return,
    };
    let now = common::now_ms();
    for tr in trs {
        if tr.closed.load(Ordering::Acquire) {
            continue;
        }
        if due(&tr.rtc.next_keepalive, now, crate::link::tcp::KEEPALIVE_MS) {
            keepalive(&tr);
        }
        if !tr.ready.load(Ordering::Acquire) {
            continue;
        }
        let ups = uplink_tracks(&tr);
        if !ups.is_empty() {
            uplink(w, &tr, &ups, now, now_us);
        }
        if crate::ops::auto_layer_on() {
            if due(&tr.rtc.next_sim, now, SIM_MS) {
                crate::sim::tick(&tr, now);
            }
            if now_us >= tr.rtc.next_probe_us.load(Ordering::Relaxed) {
                tr.rtc.next_probe_us.store(now_us + PROBE_US, Ordering::Relaxed);
                crate::sim::probe(w, &tr, now_us);
            }
        }
    }
}

fn keepalive(tr: &Transport) {
    let Some(m) = media() else { return };
    let ids: Vec<u64> = tr
        .pairs
        .lock()
        .map(|p| p.iter().filter_map(|x| if let PathKey::Tcp(id) = x.key { Some(id) } else { None }).collect())
        .unwrap_or_default();
    for id in ids {
        let t = m.tcp.lock().ok().and_then(|g| g.get(&id).cloned());
        if let Some(t) = t
            && !t.send(&[])
        {
            crate::ops::drop_count("send_fail");
        }
    }
}

fn uplink(w: &Worker, tr: &Arc<Transport>, ups: &[Arc<PubTrack>], now: u64, now_us: u64) {
    for t in ups {
        if t.stream.kind == Kind::Video {
            let (seqs, gave_up) = match t.nack.lock() {
                Ok(mut n) => n.due(now),
                Err(_) => (Vec::new(), false),
            };
            if !seqs.is_empty() {
                send_rtcp(w, tr, &build_nack(t.ssrc, &seqs));
            }
            if gave_up {
                request_pli(t, true);
            }
            let flush = t.pli.lock().map(|p| p.pending && now.saturating_sub(p.last_ms) >= throttle_ms(t)).unwrap_or(false);
            if flush {
                request_pli(t, true);
            }
        }
    }
    if due(&tr.rtc.next_rr, now, RR_MS) {
        let srs = tr.rtc.sr.lock().map(|g| g.clone()).unwrap_or_default();
        for t in ups {
            let r = match t.recv.lock() {
                Ok(mut s) if s.packets > 0 => s.report(),
                _ => continue,
            };
            let (lsr, dlsr) = match srs.get(&t.ssrc) {
                Some(s) => (((s.ntp >> 16) & 0xffff_ffff) as u32, ((now.saturating_sub(s.at_ms)) * 65_536 / 1000) as u32),
                None => (0, 0),
            };
            send_rtcp(w, tr, &build_rr(t.ssrc, &r, lsr, dlsr));
        }
    }
    if due(&tr.rtc.next_remb, now, REMB_MS) {
        let max = crate::ops::policy().media.max_bitrate_bps as u64;
        let ssrcs: Vec<u32> = ups.iter().map(|t| t.ssrc).collect();
        send_rtcp(w, tr, &build_remb(max, &ssrcs));
    }
    if tr.ext_ids.load().twcc != 0 && due(&tr.rtc.next_twcc, now, TWCC_MS) {
        let media_ssrc = ups.first().map(|t| t.ssrc).unwrap_or(0);
        loop {
            let fb = match tr.rtc.twcc_rx.lock() {
                Ok(mut g) if g.pending() => g.build(),
                _ => None,
            };
            match fb {
                Some(f) => send_rtcp(w, tr, &build_twcc(media_ssrc, &f)),
                None => break,
            }
        }
    }
    let _ = now_us;
}

pub fn on_rtcp(w: &Arc<Worker>, tr: &Arc<Transport>, plain: &[u8]) {
    let table = crate::ops::table_of(tr.id);
    for p in split(plain) {
        match (p.pt, p.fmt) {
            (200, _) => on_sr(tr, p.body),
            (201, _) => {
                let Some(t) = &table else {
                    crate::ops::drop_count("rtcp_unresolved");
                    continue;
                };
                on_rr(t, p.body);
            }
            (205, 1) => {
                let Some((media_ssrc, seqs)) = parse_nack(p.body) else { continue };
                match table.as_ref().and_then(|t| t.row_by_ssrc(media_ssrc)) {
                    Some((row, _)) => {
                        row.nacks.fetch_add(seqs.len() as u64, Ordering::Relaxed);
                        tr.tx.nack_in.fetch_add(1, Ordering::Relaxed);
                        if row.kind == Kind::Audio {
                            crate::ops::drop_count("nack_audio");
                        } else {
                            send_rtx(w, &row, &seqs);
                        }
                    }
                    None => crate::ops::drop_count("rtcp_unresolved"),
                }
            }
            (205, 15) => {
                if let Some(t) = &table {
                    crate::sim::on_twcc(tr, t, p.body);
                } else {
                    crate::ops::drop_count("rtcp_unresolved");
                }
            }
            (206, 1) | (206, 4) => {
                let media_ssrc = if p.fmt == 4 { be32(p.body, 8) } else { be32(p.body, 4) };
                let Some(ms) = media_ssrc else { continue };
                match table.as_ref().and_then(|t| t.row_by_ssrc(ms)) {
                    Some((row, _)) => {
                        tr.tx.pli_in.fetch_add(1, Ordering::Relaxed);
                        if let Some(src) = row.src_track.load_full() {
                            request_pli(&src, false);
                        }
                    }
                    None => crate::ops::drop_count("rtcp_unresolved"),
                }
            }
            (206, 15) => {}
            (202..=204, _) => {}
            _ => {}
        }
    }
}

fn on_sr(tr: &Arc<Transport>, body: &[u8]) {
    let Some(ssrc) = be32(body, 0) else { return };
    let ing = tr.ingress.load();
    let Some(track) = ing.get(&ssrc).filter(|t| t.ssrc == ssrc).cloned() else {
        crate::ops::drop_count("rtcp_unresolved");
        return;
    };
    let (Some(hi), Some(lo), Some(rtp)) = (be32(body, 4), be32(body, 8), be32(body, 12)) else { return };
    let sr = Sr { ntp: ((hi as u64) << 32) | lo as u64, rtp, at_ms: common::now_ms() };
    if let Ok(mut g) = tr.rtc.sr.lock() {
        g.insert(ssrc, sr);
    }
    for row in crate::ops::rows_of_source(&track) {
        let translated = match row.rw.lock() {
            Ok(rw) if rw.src == Some(track.src_key) => rw.translate_sr_ts(rtp),
            _ => continue,
        };
        let pk = build_sr(row.ssrc, &sr, translated, row.sent.load(Ordering::Relaxed) as u32, row.bytes.load(Ordering::Relaxed) as u32);
        emit(&row.conn, pk);
    }
}

fn on_rr(t: &crate::sub::Table, body: &[u8]) {
    let rc = body.len().saturating_sub(4) / 24;
    for k in 0..rc {
        let o = 4 + k * 24;
        let Some(ssrc) = be32(body, o) else { break };
        if let Some((row, false)) = t.row_by_ssrc(ssrc) {
            row.rr_loss.store(body[o + 4], Ordering::Relaxed);
            row.rr_at.store(common::now_ms(), Ordering::Relaxed);
        }
    }
}

pub fn send_rtx(w: &Worker, row: &DownRow, seqs: &[u16]) {
    let Some(m) = media() else { return };
    let now = common::now_ms();
    let rtx_pt = row.rtx_pt.load(Ordering::Relaxed);
    let Some(rtx_ssrc) = row.rtx_ssrc else { return };
    for &out_seq in seqs {
        if row.rtx_gate_until_new.load(Ordering::Acquire) {
            crate::ops::drop_count("rtx_gate");
            continue;
        }
        let (src_seq, ts_off) = match row.rw.lock() {
            Ok(rw) => (rw.unmap_nack(out_seq), rw.ts_off),
            Err(_) => continue,
        };
        let Some(src_seq) = src_seq else {
            crate::ops::drop_count("rtx_gate");
            continue;
        };
        let Some(track) = row.src_track.load_full() else {
            crate::ops::drop_count("rtx_cache_miss");
            continue;
        };
        let Some(orig) = track.cache.lock().ok().and_then(|c| c.get(src_seq)) else {
            crate::ops::drop_count("rtx_cache_miss");
            continue;
        };
        if !budget_ok(&row.user, now) {
            crate::ops::drop_count("rtx_budget");
            continue;
        }
        let Some(rtp) = parse_rtp(&orig) else { continue };
        let rseq = row.rtx_seq.fetch_add(1, Ordering::AcqRel);
        let mut payload = Vec::with_capacity(orig.len() - rtp.hlen + 2);
        payload.extend_from_slice(&out_seq.to_be_bytes());
        payload.extend_from_slice(&orig[rtp.hlen..]);
        let b = Built { pt: rtx_pt, seq: rseq, ts: rtp.ts.wrapping_add(ts_off), ssrc: rtx_ssrc, marker: rtp.marker, payload: &payload, probe: false, padding: false };
        if send_built(w, m, row, b) {
            row.conn.tx.retransmitted.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub struct Built<'a> {
    pub pt: u8,
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
    pub marker: bool,
    pub payload: &'a [u8],
    pub probe: bool,
    pub padding: bool,
}

pub fn send_built(w: &Worker, m: &crate::fwd::rt::Media, row: &DownRow, b: Built<'_>) -> bool {
    let Built { pt, seq, ts, ssrc, marker, payload, probe, padding } = b;
    let conn = &row.conn;
    if conn.closed.load(Ordering::Acquire) || !conn.ready.load(Ordering::Acquire) {
        return false;
    }
    let Some(path) = conn.send_ok(common::now_ms(), |id| m.tcp_open(id)) else { return false };
    let twcc = conn.twcc_seq.fetch_add(1, Ordering::AcqRel);
    let mut out = Vec::with_capacity(payload.len() + 40);
    out.push(if padding { 0xa0 } else { 0x80 });
    out.push(if marker { 0x80 | (pt & 0x7f) } else { pt & 0x7f });
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&ts.to_be_bytes());
    out.extend_from_slice(&ssrc.to_be_bytes());
    let table = row.ext.load();
    let inp = ExtInput { mid: row.mid.as_bytes(), audio_level: None, t_us: mono_us(), twcc_seq: Some(twcc) };
    if ext_build(&table, &inp, &mut out) > 0 {
        out[0] |= 0x10;
    }
    out.extend_from_slice(payload);
    let enc = match conn.srtp_tx.lock() {
        Ok(mut g) => match g.as_mut() {
            Some(c) => c.encrypt_rtp(&out),
            None => return false,
        },
        Err(_) => return false,
    };
    let Ok(enc): Result<Bytes, _> = enc else { return false };
    if !m.send_on(&w.sock, path, &enc) {
        crate::ops::drop_count("send_fail");
        return false;
    }
    sent_twcc(conn, twcc, enc.len(), mono_us(), probe);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nack_roundtrip() {
        let b = build_nack(7, &[10, 11, 13, 40]);
        let p = split(&b);
        assert_eq!(p.len(), 1);
        let (m, s) = parse_nack(p[0].body).unwrap();
        assert_eq!(m, 7);
        assert_eq!(s, vec![10, 11, 13, 40]);
    }

    #[test]
    fn twcc_roundtrip() {
        let mut t = TwccRecv::default();
        for (s, a) in [(10u16, 1_000_000i64), (11, 1_020_000), (13, 1_090_000)] {
            t.record(s, a);
        }
        let f = t.build().unwrap();
        let b = build_twcc(9, &f);
        assert_eq!(b.len() % 4, 0);
        let p = split(&b);
        let (base, arr) = parse_twcc(p[0].body).unwrap();
        assert_eq!(base, 10);
        assert_eq!(arr.len(), 4);
        assert!(arr[2].is_none());
        assert_eq!(arr[1].unwrap() - arr[0].unwrap(), 20_000);
    }

    #[test]
    fn remb_mantissa() {
        let b = build_remb(2_000_000, &[5]);
        assert_eq!(&b[12..16], b"REMB");
        let exp = b[17] >> 2;
        let mant = (((b[17] & 3) as u64) << 16) | u16::from_be_bytes([b[18], b[19]]) as u64;
        assert_eq!(mant << exp, 2_000_000 >> exp << exp);
    }
}
