// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-FWD-STEPS 정:IMPL-FWD-ROOM 정:IMPL-FWD-SOURCE-SWITCH 정:RULE-LINK-TRANSPORT 정:RULE-LINK-PAIR-LIVENESS 정:IMPL-PUB-ACCEPT 정:IMPL-SUB-READY-GATE 정:IMPL-FLR-RTP-OBSERVE 정:IMPL-SIM-SWITCH 정:RULE-OPS-OBSERVE 정:RULE-CONC-SIX 연:RULE-SUB-EGRESS-ACKED 연:RULE-LINK-SOURCE-SWITCH 연:RULE-PUB-RID-TO-SPATIAL · model: claude-opus-5-5
//! 수신 루프 하나(UDP) · TCP 전송로(RFC 4571) · 첫 바이트 demux · STUN(검증 뒤에만) · DTLS · SRTP · 업링크 학습(rid) · prefan · 다운링크 재기록 · 확장 다시 짓기.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::msg::Kind;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::rtp::{self, Head, Rewrite};
use super::{Conn, Egress, Ingress, RoomMedia};
use crate::link::stun;
use crate::link::tcp::{Deframer, demux_ok};
use crate::link::transport::{Path, TCP_IDENTIFY_MS};
use crate::ops::Sfu;
use crate::r#pub::Stream;
use crate::sub::table::Class;
use crate::trk::codec;

pub fn now_ms() -> u64 {
    common::log::now_ms()
}

pub fn now_us() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}

pub async fn udp_loop(sfu: Arc<Sfu>) {
    let mut buf = vec![0u8; 65_536];
    loop {
        let (n, src) = match sfu.io.udp.recv_from(&mut buf).await {
            Ok(x) => x,
            Err(_) => continue,
        };
        on_datagram(&sfu, Path::Udp(src), src, &buf[..n]);
    }
}

pub async fn tcp_listen(sfu: Arc<Sfu>, l: TcpListener) {
    loop {
        let Ok((s, addr)) = l.accept().await else { continue };
        let id = sfu.next_id();
        tokio::spawn(tcp_conn(sfu.clone(), s, addr, id));
    }
}

async fn tcp_conn(sfu: Arc<Sfu>, s: TcpStream, addr: SocketAddr, id: u64) {
    let _ = s.set_nodelay(true);
    let (mut rd, mut wr) = s.into_split();
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(1024);
    sfu.io.tcp.insert(id, tx);
    let writer = tokio::spawn(async move {
        while let Some(f) = rx.recv().await {
            if wr.write_all(&f).await.is_err() {
                break;
            }
        }
    });
    let path = Path::Tcp(id);
    let mut d = Deframer::default();
    let mut buf = vec![0u8; 65_536];
    let born = now_ms();
    let mut identified = false;
    loop {
        let wait = if identified { std::time::Duration::from_secs(3600) } else { std::time::Duration::from_millis(TCP_IDENTIFY_MS.saturating_sub(now_ms().saturating_sub(born))) };
        let r = tokio::time::timeout(wait, rd.read(&mut buf)).await;
        let n = match r {
            Err(_) if !identified => {
                sfu.drops.tcp_unidentified.fetch_add(1, Ordering::Relaxed);
                break;
            }
            Err(_) => continue,
            Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(n)) => n,
        };
        d.push(&buf[..n]);
        let mut bad = false;
        while let Some(f) = d.next_frame() {
            if f.is_empty() {
                continue;
            }
            if !demux_ok(f[0]) {
                sfu.drops.tcp_bad_frame.fetch_add(1, Ordering::Relaxed);
                bad = true;
                break;
            }
            on_datagram(&sfu, path, addr, &f);
            if !identified && sfu.plane.conn_by_path(&path).is_some() {
                identified = true;
            }
        }
        if bad {
            break;
        }
    }
    sfu.io.tcp.remove(&id);
    writer.abort();
    if let Some(c) = sfu.plane.conn_by_path(&path) {
        c.link.tcp_closed(id);
    }
    sfu.plane.unbind_path(&path);
}

pub fn on_datagram(sfu: &Arc<Sfu>, path: Path, src: SocketAddr, b: &[u8]) {
    let Some(&first) = b.first() else { return };
    match first {
        0..=3 => on_stun(sfu, path, src, b),
        20..=63 => on_dtls(sfu, path, b),
        128..=191 => on_srtp(sfu, path, b),
        _ => {
            sfu.drops.demux_unknown.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn on_stun(sfu: &Arc<Sfu>, path: Path, src: SocketAddr, b: &[u8]) {
    let Some(req) = stun::parse(b) else {
        sfu.drops.stun_malformed.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(uf) = req.local_ufrag().map(str::to_string) else {
        sfu.drops.stun_malformed.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(conn) = sfu.plane.conn_by_ufrag(&uf) else {
        sfu.drops.stun_unknown_ufrag.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(pwd) = conn.link.pwd_of(&uf) else {
        sfu.drops.stun_unknown_ufrag.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let ok = req.mi_at.is_some_and(|at| stun::integrity_ok(b, at, &pwd));
    if !ok {
        sfu.drops.stun_integrity_fail.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let now = now_ms();
    let out = conn.link.on_check(path, &uf, req.use_candidate, now);
    sfu.plane.bind_path(path, &conn);
    if let Some(old) = &out.promoted {
        sfu.plane.unbind_ufrag(old);
        common::log::transition(&common::log::Step {
            area: "LINK",
            key: &conn.user,
            from: "restart",
            to: Some("current"),
            cause: "checked_new",
            label: Some("LINK.S.checked_new"),
            ins: &[("ufrag", uf.clone())],
            room: None,
            user: Some(&conn.user),
        });
    }
    if let Some(dwell) = out.alive_again {
        common::log::transition(&common::log::Step {
            area: "LINK",
            key: &conn.user,
            from: "Suspect",
            to: Some("Alive"),
            cause: "nominated",
            label: Some("SV-STATE-PEER-STATE"),
            ins: &[("dwell_ms", dwell.to_string()), ("path", path.to_string())],
            room: None,
            user: Some(&conn.user),
        });
    }
    let resp = stun::success(&req.txid, src, &pwd);
    sfu.io.send(&path, &resp);
}

fn on_dtls(sfu: &Arc<Sfu>, path: Path, b: &[u8]) {
    let Some(conn) = sfu.plane.conn_by_path(&path) else {
        sfu.drops.dtls_unknown_path.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if !conn.link.dtls_started.swap(true, Ordering::AcqRel) {
        let link = conn.link.clone();
        let cert = sfu.cert.clone();
        let first = b.to_vec();
        let user = conn.user.clone();
        tokio::spawn(async move {
            let end = crate::link::dtls::run(link, cert, first).await;
            tracing::info!(user = user.as_str(), "dtls ended {}", end.name());
        });
        return;
    }
    let tx = conn.link.dtls_in.lock().ok().and_then(|g| g.clone());
    match tx {
        Some(tx) => {
            if tx.try_send(b.to_vec()).is_err() {
                sfu.drops.dtls_queue_full.fetch_add(1, Ordering::Relaxed);
            }
        }
        None => {
            sfu.drops.dtls_not_ready.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn on_srtp(sfu: &Arc<Sfu>, path: Path, b: &[u8]) {
    let Some(conn) = sfu.plane.conn_by_path(&path) else {
        sfu.drops.rtp_unknown_path.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if !conn.link.srtp_ready.load(Ordering::Acquire) {
        sfu.drops.srtp_not_ready.fetch_add(1, Ordering::Relaxed);
        return;
    }
    conn.link.rx_bytes.fetch_add(b.len() as u64, Ordering::Relaxed);
    if rtp::is_rtcp(b) {
        match conn.link.unprotect_rtcp(b) {
            Some(p) => crate::rtcp::shell::on_rtcp(sfu, &conn, &p),
            None => {
                sfu.drops.srtcp_fail.fetch_add(1, Ordering::Relaxed);
            }
        }
        return;
    }
    match conn.link.unprotect_rtp(b) {
        Some(p) => on_rtp(sfu, &conn, &p),
        None => {
            sfu.drops.srtp_fail.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn learn(sfu: &Arc<Sfu>, conn: &Arc<Conn>, b: &[u8], h: &Head) -> Option<Arc<Ingress>> {
    let peer = conn.peer.upgrade()?;
    let streams = peer.streams();
    let sim = streams.iter().find(|s| s.simulcast && conn_has_mid(b, h, s))?;
    let rid_id = sim.ext.rid?;
    if let Some(rr) = sim.ext.rrid
        && let Some(v) = rtp::ext_value(b, h, rr)
        && let Some(layer) = std::str::from_utf8(v).ok().and_then(crate::r#pub::rid_layer)
    {
        let primary = conn.ingress.load().values().find(|i| i.stream.track_id == sim.track_id && i.layer == layer).map(|i| i.ssrc)?;
        conn.add_rtx(h.ssrc, primary);
        return None;
    }
    let rid = rtp::ext_value(b, h, rid_id).and_then(|v| std::str::from_utf8(v).ok())?;
    let Some(layer) = crate::r#pub::rid_layer(rid) else {
        sfu.drops.rid_unknown.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    if let Some(dup) = conn.ingress.load().values().find(|i| i.stream.track_id == sim.track_id && i.layer == layer) {
        return Some(dup.clone());
    }
    let ing = Arc::new(Ingress::new(h.ssrc, layer, sim.clone(), Arc::downgrade(conn)));
    conn.add_ingress(ing.clone());
    let s2 = sfu.clone();
    let p2 = peer.clone();
    let track = sim.track_id.clone();
    tokio::spawn(async move { crate::ops::publish::sim_attached(&s2, &p2, &track).await });
    Some(ing)
}

fn conn_has_mid(b: &[u8], h: &Head, s: &Stream) -> bool {
    match s.ext.mid {
        Some(id) => rtp::ext_value(b, h, id).is_some_and(|v| v == s.mid.as_bytes()),
        None => true,
    }
}

pub fn on_rtp(sfu: &Arc<Sfu>, conn: &Arc<Conn>, plain: &[u8]) {
    let Some(h) = rtp::parse(plain) else {
        sfu.drops.rtp_malformed.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let now = now_ms();
    let tw = conn.twcc_id.load(Ordering::Relaxed);
    if tw != 0
        && let Some(v) = rtp::ext_value(plain, &h, tw)
        && v.len() >= 2
        && let Ok(mut t) = conn.twcc_in.lock()
    {
        t.record(u16::from_be_bytes([v[0], v[1]]), now_us());
    }
    if let Some(primary) = conn.rtx_map.load().get(&h.ssrc).copied() {
        let Some(ing) = conn.ingress_of(primary) else { return };
        let pl = rtp::payload(plain, &h);
        if pl.len() < 2 {
            return;
        }
        let osn = u16::from_be_bytes([pl[0], pl[1]]);
        let mut orig = Vec::with_capacity(plain.len());
        orig.extend_from_slice(&plain[..h.payload]);
        orig[1] = (orig[1] & 0x80) | (ing.stream.pt & 0x7f);
        orig[2..4].copy_from_slice(&osn.to_be_bytes());
        orig[8..12].copy_from_slice(&primary.to_be_bytes());
        orig.extend_from_slice(&pl[2..]);
        if orig[0] & 0x20 != 0 {
            orig[0] &= !0x20;
        }
        let Some(oh) = rtp::parse(&orig) else { return };
        ingest(sfu, conn, &ing, &orig, &oh, now, true);
        return;
    }
    let ing = match conn.ingress_of(h.ssrc) {
        Some(i) => i,
        None => match learn(sfu, conn, plain, &h) {
            Some(i) => i,
            None => {
                conn.unknown_drop.fetch_add(1, Ordering::Relaxed);
                sfu.drops.rtp_unknown_ssrc.fetch_add(1, Ordering::Relaxed);
                return;
            }
        },
    };
    ingest(sfu, conn, &ing, plain, &h, now, false);
}

fn ingest(sfu: &Arc<Sfu>, conn: &Arc<Conn>, ing: &Arc<Ingress>, pkt: &[u8], h: &Head, now: u64, retransmit: bool) {
    ing.last_rx_ms.store(now, Ordering::Relaxed);
    ing.rx_pkts.fetch_add(1, Ordering::Relaxed);
    if conn.media_ssrc.load(Ordering::Relaxed) == 0 {
        conn.media_ssrc.store(ing.ssrc, Ordering::Relaxed);
    }
    let s = &ing.stream;
    if let Ok(mut st) = ing.st.lock() {
        if !retransmit {
            st.stats.update(h.seq, h.ts, now, pkt.len());
        }
        let gap = st.nack.on_packet(h.seq, now);
        if matches!(gap, crate::rtcp::core::Gap::TooLarge) && s.kind == Kind::Video {
            drop(st);
            crate::rtcp::shell::pli(sfu, ing, now, true);
        } else {
            st.cache.put(h.seq, pkt);
        }
    }
    if s.first_rtp(now) {
        tracing::info!(user = s.user.as_str(), "uplink first rtp track={} ssrc={} layer={}", s.track_id, ing.ssrc, ing.layer);
    }
    if !s.pt_checked.swap(true, Ordering::AcqRel) && h.pt != s.pt {
        sfu.drops.pt_mismatch.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(user = s.user.as_str(), "[PT:MISMATCH] track={} declared={} first_rtp={}", s.track_id, s.pt, h.pt);
    }
    if let Some(al) = s.ext.audio_level
        && let Some(v) = rtp::ext_value(pkt, h, al)
        && let Some(x) = v.first()
        && (x & 0x7f) < 50
    {
        s.last_loud_ms.store(now, Ordering::Relaxed);
    }
    let Some(peer) = conn.peer.upgrade() else { return };
    let Some(room) = peer.aff.pub_room() else {
        sfu.drops.no_pub_room.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if s.half() {
        prefan(sfu, ing, &room, pkt, h, now);
    } else {
        fan_full(sfu, ing, pkt, h, now);
    }
}

fn prefan(sfu: &Arc<Sfu>, ing: &Arc<Ingress>, room: &str, pkt: &[u8], h: &Head, now: u64) {
    let s = &ing.stream;
    let Some(rm) = sfu.plane.room(room) else { return };
    let Some(r) = sfu.rooms.get(room).map(|x| x.clone()) else { return };
    let slot = rm.slot(s.kind);
    let bound = slot.bound.load_full();
    let pass = bound.as_ref().is_some_and(|b| b.user == s.user);
    if let Some(step) = r.floor.observe_rtp_gated(&s.user, now, pass) {
        let s2 = sfu.clone();
        let rr = r.clone();
        tokio::spawn(async move { crate::ops::floor::exec(&s2, &rr, step).await });
    }
    let Some(b) = bound.filter(|_| pass) else {
        sfu.drops.prefan_drop.fetch_add(1, Ordering::Relaxed);
        return;
    };
    b.primed.store(true, Ordering::Release);
    let key = (b.episode << 33) | u64::from(ing.ssrc);
    let codec = match s.kind {
        Kind::Audio => rm.audio_codec.as_str(),
        Kind::Video => rm.video_codec.as_str(),
    };
    for w in slot.subs.load().iter() {
        let Some(e) = w.upgrade() else { continue };
        if e.user == s.user {
            continue;
        }
        if !e.acked() {
            sfu.drops.ack_gate.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        send_on(sfu, &e, Some(ing), key, pkt, h, now, codec);
    }
}

fn fan_full(sfu: &Arc<Sfu>, ing: &Arc<Ingress>, pkt: &[u8], h: &Head, now: u64) {
    let s = &ing.stream;
    let codec = s.codec_name().to_string();
    for w in ing.subs.load().iter() {
        let Some(e) = w.upgrade() else { continue };
        let loaded = e.load.load();
        let Some(l) = loaded.as_ref() else { continue };
        if l.track_id != s.track_id {
            continue;
        }
        if s.kind == Kind::Video
            && let Some(g) = e.gate.load().as_ref()
            && !g.open()
        {
            sfu.drops.ready_gate.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if !e.acked() {
            sfu.drops.ack_gate.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if s.simulcast && !layer_ok(sfu, &e, ing, pkt, h, now, &codec) {
            continue;
        }
        let key = (u64::from(ing.layer) << 32) | u64::from(ing.ssrc);
        send_on(sfu, &e, Some(ing), key, pkt, h, now, &codec);
    }
}

fn layer_ok(sfu: &Arc<Sfu>, e: &Arc<Egress>, ing: &Arc<Ingress>, pkt: &[u8], h: &Head, now: u64, codec: &str) -> bool {
    let paused = e.prefs.load().as_ref().is_some_and(|p| p.paused.load(Ordering::Relaxed));
    if paused {
        return false;
    }
    let cap = e.prefs.load().as_ref().map(|p| p.spatial.load(Ordering::Relaxed)).unwrap_or(crate::sub::MAX_SPATIAL);
    let auto = crate::sim::auto_layer(sfu, e);
    let mut target = cap.min(auto);
    let Ok(mut rw) = e.rw.lock() else { return false };
    if target == 1 {
        let h_alive = sibling_alive(ing, 1, now);
        if !h_alive {
            target = 0;
        }
    }
    if rw.target_layer != target {
        rw.target_layer = target;
        rw.pending_since = now;
        drop(rw);
        if let Some(t) = sibling(ing, target) {
            crate::rtcp::shell::pli(sfu, &t, now, false);
        }
        let Ok(r) = e.rw.lock() else { return false };
        rw = r;
    }
    match rw.cur_layer {
        Some(c) if c == ing.layer && c == rw.target_layer => true,
        Some(c) if c == ing.layer => {
            if rw.pending_since != 0 && now.saturating_sub(rw.pending_since) > crate::sim::core::PENDING_EXPIRE_MS {
                rw.target_layer = c;
                rw.pending_since = 0;
                tracing::info!(user = e.user.as_str(), "[SIM:PENDING] expired mid={}", e.mid);
            }
            true
        }
        _ if ing.layer == rw.target_layer && codec::is_keyframe(codec, rtp::payload(pkt, h)) => {
            rw.cur_layer = Some(ing.layer);
            rw.pending_since = 0;
            rw.await_key = false;
            true
        }
        None if ing.layer == rw.target_layer => {
            drop(rw);
            crate::rtcp::shell::pli(sfu, ing, now, false);
            false
        }
        _ => false,
    }
}

fn sibling(ing: &Arc<Ingress>, layer: u8) -> Option<Arc<Ingress>> {
    let c = ing.conn.upgrade()?;
    c.ingress.load().values().find(|i| i.stream.track_id == ing.stream.track_id && i.layer == layer).cloned()
}

fn sibling_alive(ing: &Arc<Ingress>, layer: u8, now: u64) -> bool {
    sibling(ing, layer).is_some_and(|i| now.saturating_sub(i.last_rx_ms.load(Ordering::Relaxed)) < 1000)
}

#[allow(clippy::too_many_arguments)]
pub fn send_on(sfu: &Arc<Sfu>, e: &Arc<Egress>, src: Option<&Arc<Ingress>>, key: u64, pkt: &[u8], h: &Head, now: u64, codec: &str) -> bool {
    let Some(conn) = e.conn.upgrade() else { return false };
    let clock = codec::clock(codec);
    let Ok(mut rw) = e.rw.lock() else { return false };
    if rw.src != key {
        if e.kind == Kind::Video && !codec::is_keyframe(codec, rtp::payload(pkt, h)) {
            drop(rw);
            sfu.drops.await_keyframe.fetch_add(1, Ordering::Relaxed);
            if let Some(i) = src {
                crate::rtcp::shell::pli(sfu, i, now, false);
            }
            return false;
        }
        rw.src = key;
        e.rtx_block.store(false, Ordering::Release);
    }
    let (seq, ts) = rw.rewriter.map(key, h.seq, h.ts, now, clock);
    let ext = crate::sub::table::SubExt::of(e.kind, &conn.ext.load());
    let level = if e.kind == Kind::Audio {
        src.and_then(|i| i.stream.ext.audio_level).and_then(|id| rtp::ext_value(pkt, h, id)).and_then(|v| v.first().copied())
    } else {
        None
    };
    let tw = conn.twcc_out.fetch_add(1, Ordering::Relaxed);
    let t_us = now_us();
    let Rw2 { ext_buf, out_buf } = take_bufs(&mut rw);
    let mut ext_buf = ext_buf;
    let mut out_buf = out_buf;
    ext_buf.clear();
    if !ext.is_empty() {
        crate::sub::ext::build(&ext, Some(&e.mid_str), level, t_us, tw, &mut ext_buf);
    }
    let pt = e.pt.load(Ordering::Relaxed);
    rtp::rebuild(pkt, h, &Rewrite { pt, seq, ts, ssrc: e.ssrc, ext: &ext_buf }, &mut out_buf);
    let ok = conn.link.send_rtp(&out_buf);
    if ok {
        if let Ok(mut b) = conn.bwe.lock() {
            b.on_sent(tw as u16, t_us, out_buf.len(), false);
        }
        e.sent.fetch_add(1, Ordering::Relaxed);
        e.last_sent_ms.store(now, Ordering::Relaxed);
    }
    rw.ext_buf = ext_buf;
    rw.out_buf = out_buf;
    ok
}

struct Rw2 {
    ext_buf: Vec<u8>,
    out_buf: Vec<u8>,
}

fn take_bufs(rw: &mut super::Rw) -> Rw2 {
    Rw2 { ext_buf: std::mem::take(&mut rw.ext_buf), out_buf: std::mem::take(&mut rw.out_buf) }
}

pub struct Synth<'a> {
    pub kind: Kind,
    pub key: u64,
    pub seq: u16,
    pub ts: u32,
    pub payload: &'a [u8],
    pub skip: Option<&'a str>,
}

pub fn send_synth(sfu: &Arc<Sfu>, rm: &RoomMedia, x: &Synth<'_>) {
    let (kind, skip) = (x.kind, x.skip);
    let slot = rm.slot(kind);
    let codec = match kind {
        Kind::Audio => rm.audio_codec.as_str(),
        Kind::Video => rm.video_codec.as_str(),
    };
    let key = x.key;
    let pkt = rtp::synth(0, x.seq, x.ts, 0, false, x.payload);
    let Some(h) = rtp::parse(&pkt) else { return };
    let now = now_ms();
    for w in slot.subs.load().iter() {
        let Some(e) = w.upgrade() else { continue };
        if Some(e.user.as_str()) == skip || e.class != Class::Slot || !e.acked() {
            continue;
        }
        send_on(sfu, &e, None, key, &pkt, &h, now, codec);
    }
}
