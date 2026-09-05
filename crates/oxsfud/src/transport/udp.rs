// author: kodeholic (powered by Claude)
//! UDP 단일 포트 — 정§12. 소켓 하나로 STUN·DTLS·SRTP 가 들어오고 첫 바이트가 갈래를 정한다.
//! ★STUN 은 integrity 를 검증한 뒤에야 latch·응답·`last_seen` 으로 넘어간다 — 순서가 곧 방어다.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

use super::conn::DemuxConn;
use super::session::{ConnRole, TransportSession};
use super::{demux, dtls, stun};
use crate::handlers::{Sfu, now_ms};
use crate::media::{self, codec, rtcp, rtp, rtx};
use crate::peer::Peer;
use crate::media::subscribe::{SubscribeState, SubscriberStream};
use crate::media::track::{PublishState, PublisherStream, PublisherTrack};
use oxsig::schema::{Duplex, MediaKind};

const RECV_BUF: usize = 2_048;
/// egress 조립 자리 — 루프가 하나를 들고 재사용한다(구독자마다 새로 잡지 않는다).
const EGRESS_BUF: usize = 2_048;
const HANDSHAKE_TIMEOUT_MS: u64 = 10_000;

pub async fn bind(port: u16) -> std::io::Result<Arc<UdpSocket>> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    Ok(Arc::new(UdpSocket::bind(addr).await?))
}

pub async fn run(sfu: Arc<Sfu>, socket: Arc<UdpSocket>) {
    let mut buf = vec![0u8; RECV_BUF];
    let mut egress = Vec::with_capacity(EGRESS_BUF);
    loop {
        let (len, remote) = match socket.recv_from(&mut buf).await {
            Ok(r) => r,
            Err(e) => {
                debug!(error = %e, "udp recv");
                continue;
            }
        };
        let data = &buf[..len];
        match demux::classify(data) {
            demux::Packet::Stun => on_stun(&sfu, &socket, data, remote).await,
            demux::Packet::Dtls => on_dtls(&sfu, &socket, Bytes::copy_from_slice(data), remote).await,
            demux::Packet::Srtp => on_srtp(&sfu, &socket, data, remote, &mut egress).await,
            demux::Packet::Unknown => debug!(%remote, "udp packet of unknown kind"),
        }
    }
}

async fn on_stun(sfu: &Arc<Sfu>, socket: &Arc<UdpSocket>, data: &[u8], remote: SocketAddr) {
    let Some(msg) = stun::parse(data) else { return };
    if msg.msg_type != stun::BINDING_REQUEST {
        return;
    }
    let Some(ufrag) = msg.server_ufrag() else { return };
    let Some(session) = sfu.transport.by_ufrag(ufrag) else {
        debug!(%remote, ufrag, "stun for unknown credential");
        return;
    };
    if !msg.verify_integrity(&session.creds.pwd) {
        warn!(%remote, ufrag, user = %session.user_id, "stun integrity mismatch, refused before latch");
        return;
    }
    if sfu.transport.latch(&session, remote) {
        info!(%remote, ufrag, user = %session.user_id, role = session.role.as_str(), "stun latched");
    }
    sfu.observe_media(&session.user_id);
    let response = stun::binding_response(&msg.transaction_id, remote, &session.creds.pwd);
    if let Err(e) = socket.send_to(&response, remote).await {
        warn!(%remote, error = %e, "stun response send failed");
        return;
    }
    if msg.has_use_candidate() && session.claim_handshake() {
        start_handshake(sfu.clone(), socket.clone(), session);
    }
}

async fn on_dtls(sfu: &Arc<Sfu>, socket: &Arc<UdpSocket>, data: Bytes, remote: SocketAddr) {
    let Some(session) = sfu.transport.by_addr(&remote) else {
        debug!(%remote, "dtls from unlatched address");
        return;
    };
    if session.dtls_tx().is_none() && session.claim_handshake() {
        start_handshake(sfu.clone(), socket.clone(), session.clone());
    }
    let Some(tx) = session.dtls_tx() else { return };
    if tx.send(data).await.is_err() {
        debug!(%remote, user = %session.user_id, "dtls session closed");
    }
}

/// 정§2-2 전송 생존 관찰 + 정§7-3 전달. RTCP 종단은 §11 의 몫이라 여기선 생존만 센다.
async fn on_srtp(sfu: &Arc<Sfu>, socket: &Arc<UdpSocket>, data: &[u8], remote: SocketAddr, egress: &mut Vec<u8>) {
    let Some(session) = sfu.transport.by_addr(&remote) else { return };
    // 세션이 주인을 직접 들고 있어 등록부를 다시 뒤지지 않는다.
    let Some(peer) = session.peer() else { return };
    peer.touch(now_ms());
    if rtp::is_rtcp(data) {
        // 정§11-2 — RTCP 는 릴레이가 아니라 종단이다. 1pc 는 한 5-tuple 로 섞여 오므로
        // ★복호 후 평문에서 패킷 단위로 분해한다(미해소는 계수하고 버린다).
        match session.decrypt_rtcp(data) {
            Ok(plain) => on_rtcp(sfu, socket, &peer, &session, &plain, egress).await,
            Err(e) => debug!(user = %session.user_id, error = %e, "srtcp decrypt"),
        }
        return;
    }
    if session.role != ConnRole::Publish {
        return;
    }
    let plain = match session.decrypt_rtp(data) {
        Ok(p) => p,
        Err(e) => {
            debug!(user = %session.user_id, error = %e, "srtp decrypt");
            return;
        }
    };
    fan_out(sfu, socket, &peer, &session, &plain, egress).await;
}

/// 정§11-2 — 조각마다 축이 다르다: SR 은 발행 축(번역 릴레이) · RR/PLI/NACK 은 구독 축(서버가 소비).
/// ★구독자 RR 을 발행자에게 릴레이하지 않는다 — 발행자가 남의 수신 품질로 비트레이트를 깎는다.
async fn on_rtcp(sfu: &Arc<Sfu>, socket: &Arc<UdpSocket>, peer: &Arc<Peer>, session: &Arc<TransportSession>, compound: &[u8], egress: &mut Vec<u8>) {
    let now = now_ms();
    for part in rtcp::packets(compound) {
        match rtcp::payload_type(part) {
            Some(rtcp::PT_SR) => relay_sender_report(socket, peer, part, egress).await,
            // 구독자 RR 은 여기서 끝난다(소비). 자동 레이어가 설 때 이 값을 읽는다(정§10).
            Some(rtcp::PT_RR) => debug!(user = %session.user_id, blocks = rtcp::read_report_blocks(part).len(), "subscriber rr"),
            Some(rtcp::PT_SDES | rtcp::PT_BYE | rtcp::PT_APP) => {}
            Some(rtcp::PT_PSFB) if rtcp::is_pli(part) => {
                if let Some(media) = rtcp::media_ssrc(part) {
                    sfu.request_keyframe(media, now, false).await;
                }
            }
            Some(rtcp::PT_RTPFB) if rtcp::is_nack(part) => serve_nack(socket, peer, part, now).await,
            // TWCC·REMB 는 대역 축(정§10)의 몫이다. 조용히 버리지 않는다.
            _ => debug!(user = %session.user_id, pt = ?rtcp::payload_type(part), fmt = ?rtcp::fmt(part), "rtcp not terminated here"),
        }
    }
}

/// 정§11-2 — SR 은 ★자체 생성 금지, 번역 릴레이다. 카운터만 egress 기준으로 갈고 NTP 는 원본을 지킨다.
async fn relay_sender_report(socket: &Arc<UdpSocket>, peer: &Arc<Peer>, sr: &[u8], egress: &mut Vec<u8>) {
    let Some(info) = rtcp::sender_info(sr) else { return };
    let Some((stream, track)) = peer.publish.by_ssrc(info.ssrc) else { return };
    track.reception.on_sender_report(info.ntp, now_ms());
    if stream.duplex() == Duplex::Half || stream.simulcast {
        return;
    }
    for weak in track.subscribers().iter() {
        let Some(sub) = weak.upgrade() else { continue };
        let Some(transport) = sub.transport.as_ref() else { continue };
        let Some(addr) = transport.addr.get() else { continue };
        egress.clear();
        egress.extend_from_slice(sr);
        let sent = u32::try_from(sub.sent.load(Ordering::Relaxed)).unwrap_or(u32::MAX);
        let octets = u32::try_from(sub.sent_octets.load(Ordering::Relaxed)).unwrap_or(u32::MAX);
        if !rtcp::translate_sr(egress, sub.vssrc, info.rtp_ts, sent, octets) {
            continue;
        }
        let Ok(sealed) = transport.encrypt_rtcp(egress) else { continue };
        let _ = socket.send_to(&sealed, addr).await;
    }
}

/// 정§7-3 — 순서가 계약이다. egress PT 는 ★구독자 표의 값이고 확장 번호는 원소마다 다시 쓴다(정§7-2-1).
///
/// ★**전이중의 발화 방은 등록 방이다**(갈래 ②) — 배관이 그 방에서만 만들어지므로 목록 자체가 방이고,
/// 매 패킷 방을 견줄 것이 없다. `pub_room` 은 방 공용 슬롯을 쓰는 반이중 축 전용이다(정§8-1).
///
/// ★이 함수는 힙을 잡지 않는다 — 구독자 목록은 RCU 안내자를 그대로 훑고, 조립은 넘겨받은 버퍼를 재사용한다.
async fn fan_out(sfu: &Arc<Sfu>, socket: &Arc<UdpSocket>, peer: &Arc<Peer>, session: &Arc<TransportSession>, packet: &[u8], egress: &mut Vec<u8>) {
    let Some(ssrc) = rtp::ssrc(packet) else { return };
    let Some((stream, track)) = peer.publish.by_ssrc(ssrc).or_else(|| sfu.learn_simulcast(peer, packet, ssrc)) else { return };
    // 정§11-2 — 발행자가 신고한 번호로 읽는다. 서버 선언값으로 읽으면 협상 결과와 어긋난다.
    if let Some(id) = peer.publish.extmap().iter().find(|(_, uri)| *uri == media::URI_TWCC).map(|(id, _)| *id)
        && let Some(value) = rtp::extension_value(packet, id)
        && value.len() >= 2
    {
        session.arrivals.observe(u16::from_be_bytes([value[0], value[1]]), now_ms());
    }
    track.rtp_in.fetch_add(1, Ordering::Relaxed);
    // 정§11-2 — 발행자에게 돌려줄 "우리 수신 품질". RTP 헤더만 보고 원자값으로 센다.
    if let (Some(seq), Some(ts)) = (rtp::sequence(packet), rtp::timestamp(packet)) {
        let seen_at = now_ms();
        track.reception.observe(seq, ts, seen_at, stream.clock_rate());
        // 정§11-1 상향 — 판정만 한다. 무엇을 언제 보낼지는 타이머 하나가 정한다.
        track.gaps.observe(seq, seen_at);
    }
    stream.set_state(PublishState::Active);
    // 정§6-3 — 선언 PT 와 첫 RTP PT 의 불일치는 표면화만 한다. 고치면 검은 화면이 침묵한다.
    if let Some(pt) = rtp::payload_type(packet)
        && pt != stream.pt
        && stream.claim_pt_report()
    {
        warn!(user = %peer.user_id, track = %stream.track_id, declared = stream.pt, arrived = pt, "pt mismatch");
    }
    if stream.muted() {
        return;
    }
    if stream.duplex() == Duplex::Half {
        prefan(socket, peer, &stream, packet, egress).await;
        return;
    }
    if stream.simulcast {
        fan_out_simulcast(socket, &stream, &track, packet, egress).await;
        return;
    }
    for weak in track.subscribers().iter() {
        let Some(sub) = weak.upgrade() else { continue };
        // 게이트는 ★전이중 video 에만 — audio 는 어떤 경우에도 gate 로 죽지 않는다.
        if sub.kind == MediaKind::Video && sub.state() != SubscribeState::Active {
            continue;
        }
        let Some(transport) = sub.transport.as_ref() else { continue };
        let Some(addr) = transport.addr.get() else { continue };
        assemble(egress, packet, &sub);
        // 정§11-1 하향 — 재전송에 대비해 나간 것을 담는다. ★평문이다(암호는 그때 다시 건다).
        if let Some(seq) = rtp::sequence(egress) {
            sub.rtx.keep(seq, egress);
        }
        let Ok(sealed) = transport.encrypt_rtp(egress) else { continue };
        if socket.send_to(&sealed, addr).await.is_ok() {
            sub.sent.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// 정§11-1 하향 — 구독자가 요구한 재전송. 관문 넷을 지나고 사유는 계수로 남는다.
async fn serve_nack(socket: &Arc<UdpSocket>, peer: &Arc<Peer>, pkt: &[u8], now: u64) {
    let Some(media) = rtcp::media_ssrc(pkt) else { return };
    let Some(sub) = peer.subscribe.all().into_iter().find(|s| s.vssrc == media) else { return };
    // 오디오 NACK 은 미채택이다 — 협상하지 않았으므로 와도 답하지 않는다.
    let (Some(rtx_pt), Some(rtx_ssrc)) = (sub.rtx_pt(), sub.rtx_vssrc) else { return };
    let Some(transport) = sub.transport.as_ref() else { return };
    let Some(addr) = transport.addr.get() else { return };

    let (mut sent, mut gate, mut miss, mut budget) = (0u32, 0u32, 0u32, 0u32);
    for seq in rtcp::read_nack(pkt) {
        match sub.rtx.take(seq, now, now) {
            Ok(original) => {
                let Some(retx) = rtp::to_rtx(&original, rtx_pt, rtx_ssrc, sub.next_rtx_seq()) else { continue };
                let Ok(sealed) = transport.encrypt_rtp(&retx) else { continue };
                if socket.send_to(&sealed, addr).await.is_ok() {
                    sent += 1;
                }
            }
            Err(rtx::Refusal::Gate) => gate += 1,
            Err(rtx::Refusal::Miss) => miss += 1,
            Err(rtx::Refusal::Budget) => budget += 1,
        }
    }
    if gate + miss + budget > 0 {
        debug!(user = %peer.user_id, track = %sub.track_id, sent, gate, miss, budget, "rtx refused");
    }
}

fn start_handshake(sfu: Arc<Sfu>, socket: Arc<UdpSocket>, session: Arc<TransportSession>) {
    let (adapter, tx) = DemuxConn::new(socket, session.addr.clone());
    session.set_dtls_tx(tx);
    tokio::spawn(async move {
        let user = session.user_id.clone();
        let config = dtls::server_config(&sfu.cert);
        let deadline = tokio::time::Duration::from_millis(HANDSHAKE_TIMEOUT_MS);
        let accepted = tokio::select! {
            _ = session.cancel.cancelled() => return,
            r = tokio::time::timeout(deadline, dtls::accept(Arc::new(adapter), config)) => r,
        };
        let conn = match accepted {
            Ok(Ok(c)) => c,
            Ok(Err(e)) => {
                warn!(user = %user, role = session.role.as_str(), error = %e, "dtls handshake failed");
                return;
            }
            Err(_) => {
                warn!(user = %user, role = session.role.as_str(), "dtls handshake timeout");
                return;
            }
        };
        match dtls::export_srtp_keys(&conn).await {
            Ok(keys) => match session.install_srtp(&keys) {
                Ok(()) => info!(user = %user, role = session.role.as_str(), "dtls up, srtp keyed"),
                Err(e) => {
                    warn!(user = %user, error = %e, "srtp key install failed");
                    return;
                }
            },
            Err(e) => {
                warn!(user = %user, error = %e, "srtp key export failed");
                return;
            }
        }
        match session.role {
            ConnRole::Publish => super::dc::run(&conn, sfu, session).await,
            ConnRole::Subscribe => hold(&conn, &session).await,
        }
    });
}

/// 정§10-1·§10-2 — simulcast 는 단이 여럿이라 ★구독자마다 하나를 고른다.
/// 전환은 ★**target 레이어의 키프레임 도착에서만** 확정되고(중간에 바꾸면 디코더가 깨진다),
/// egress 는 `vssrc` 하나로 합쳐지므로 재기록기가 `seq`·`ts` 를 이어 붙인다.
async fn fan_out_simulcast(
    socket: &Arc<UdpSocket>,
    stream: &Arc<PublisherStream>,
    track: &Arc<PublisherTrack>,
    packet: &[u8],
    egress: &mut Vec<u8>,
) {
    let Some(rid) = track.rid.clone() else { return };
    let keyframe = rtp::payload(packet).is_some_and(|p| codec::is_keyframe(stream.codec, p));
    let now = now_ms();
    for weak in track.subscribers().iter() {
        let Some(sub) = weak.upgrade() else { continue };
        if sub.state() != SubscribeState::Active || sub.paused() {
            continue;
        }
        let wanted = sub.wanted_rid();
        let current = sub.current_rid();
        if current.as_deref() != Some(rid.as_str()) {
            // 아직 이 단이 아니다 — 목표면 키프레임을 기다리고, 아니면 흘리지 않는다.
            if wanted != rid {
                continue;
            }
            if !keyframe {
                if sub.aim(&rid, now) {
                    debug!(user = %sub.subscriber, track = %stream.track_id, rid = %rid, "layer target set, waiting for a keyframe");
                }
                continue;
            }
            sub.switch_to(&rid);
        } else if wanted != rid {
            // 상한이 내려갔다 — 다음 목표의 키프레임이 올 때까지 지금 단을 계속 흘린다.
            sub.aim(wanted, now);
        }
        let Some(transport) = sub.transport.as_ref() else { continue };
        let Some(addr) = transport.addr.get() else { continue };
        egress.clear();
        egress.extend_from_slice(packet);
        if !sub.rewriter.rewrite(egress, &rid, sub.vssrc) {
            continue;
        }
        rtp::set_payload_type(egress, sub.pt());
        rtp::rewrite_extension_ids(egress, |id| sub.ext_of(id));
        let Ok(sealed) = transport.encrypt_rtp(egress) else { continue };
        if socket.send_to(&sealed, addr).await.is_ok() {
            sub.sent.fetch_add(1, Ordering::Relaxed);
            sub.sent_octets.fetch_add(egress.len() as u64, Ordering::Relaxed);
        }
    }
}

/// 정§7-3 — ★**발언권의 강제 권위는 여기다.** 통지(DC)가 늦거나 유실돼도 미디어는 새지 않는다.
/// 산출 조건은 둘: 방 = 그 순간의 `pub_room`(RCU) · 그 방 발언권 화자 == 발행자.
/// 통과한 것만 `T1`·`T2` 의 "RTP 수신"으로 센다(정§9-2) — 막힌 RTP 는 발화가 아니다.
async fn prefan(socket: &Arc<UdpSocket>, peer: &Arc<Peer>, stream: &Arc<PublisherStream>, packet: &[u8], egress: &mut Vec<u8>) {
    let Some(room) = peer.pub_room() else { return };
    if !room.floor.is_speaker(&peer.user_id) {
        return;
    }
    let Some(slot) = (match stream.kind {
        MediaKind::Audio => Some(room.slots.audio.clone()),
        MediaKind::Video => room.slots.video(),
    }) else {
        return;
    };
    // 정§9-7 — video 는 화자 코덱 == 슬롯 코덱일 때만 결합한다(다르면 조용한 검은 화면이 된다).
    if stream.kind == MediaKind::Video && (slot.codec, slot.fmtp.as_deref()) != (stream.codec, stream.fmtp.as_deref()) {
        return;
    }
    room.floor.on_media(now_ms());
    egress.clear();
    egress.extend_from_slice(packet);
    let rewriter = room.slots.rewriter(stream.kind);
    if !rewriter.rewrite(egress, &peer.user_id, slot.vssrc) {
        return;
    }
    let base = std::mem::take(egress);
    for weak in slot.tracks().first().map(|t| t.subscribers()).as_deref().into_iter().flat_map(|g| g.iter()) {
        let Some(sub) = weak.upgrade() else { continue };
        // ★슬롯 fan-out 은 화자 본인을 뺀다 — N:1 슬롯을 화자도 구독하므로 안 빼면 제 목소리가 되돌아온다.
        if sub.subscriber == peer.user_id {
            continue;
        }
        let Some(transport) = sub.transport.as_ref() else { continue };
        let Some(addr) = transport.addr.get() else { continue };
        egress.clear();
        egress.extend_from_slice(&base);
        rtp::set_payload_type(egress, sub.pt());
        rtp::rewrite_extension_ids(egress, |id| sub.ext_of(id));
        let Ok(sealed) = transport.encrypt_rtp(egress) else { continue };
        if socket.send_to(&sealed, addr).await.is_ok() {
            sub.sent.fetch_add(1, Ordering::Relaxed);
            sub.sent_octets.fetch_add(egress.len() as u64, Ordering::Relaxed);
        }
    }
}

/// 구독자 하나 몫의 egress 조립 — PT 는 구독자 표 값으로, 확장 번호는 그 구독자 표로(정§7-2-1).
/// ★버퍼를 넘겨받아 재사용한다 — 구독자마다 새로 잡으면 방 하나에 사람이 늘수록 할당이 곱으로 는다.
fn assemble(egress: &mut Vec<u8>, packet: &[u8], sub: &SubscriberStream) {
    egress.clear();
    egress.extend_from_slice(packet);
    rtp::set_payload_type(egress, sub.pt());
    rtp::rewrite_extension_ids(egress, |id| sub.ext_of(id));
}

/// 받기 연결에는 DC 가 없다(연§9-6) — 회수 신호가 올 때까지 DTLS 만 살려 둔다.
/// UDP 는 상대가 사라져도 recv 가 영영 pending 이라 취소 팔이 없으면 태스크가 남는다(정§17-2 ④).
async fn hold(conn: &dtls::DtlsConn, session: &Arc<TransportSession>) {
    let mut buf = vec![0u8; RECV_BUF];
    loop {
        tokio::select! {
            _ = session.cancel.cancelled() => break,
            r = webrtc_util::conn::Conn::recv(conn, &mut buf) => match r {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    }
    debug!(user = %session.user_id, "subscribe transport closed");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::subscribe::{SubSpec, SubscribeContext};
    use crate::media::track::{PublishContext, StreamSpec};
    use oxsig::schema::{Extmap, PcMode};

    fn packet() -> Vec<u8> {
        // marker=1 · PT=96 — 조립이 PT 만 바꾸고 marker 를 남기는지 함께 본다.
        let mut p = vec![0x90, 0xE0, 0x00, 0x0A, 0, 0, 0x30, 0x39, 0x12, 0x34, 0x56, 0x78];
        // 서버 선언 번호 그대로 온 확장 둘 — 5=abs-send-time · 6=twcc.
        p.extend_from_slice(&[0xBE, 0xDE, 0x00, 0x01, 0x50, 0x77, 0x60, 0x11]);
        p.extend_from_slice(&[9; 40]);
        p
    }

    /// ★핫패스 계약 — 구독자마다 버퍼를 새로 잡지 않고, 길이는 변하지 않는다(SRTP 태그 계산의 전제).
    #[test]
    fn egress_assembly_reuses_one_buffer_and_keeps_the_length() {
        let publish = PublishContext::default();
        let stream = publish.insert(StreamSpec {
            track_id: "t".into(), vssrc: 0x1234_5678, owner: "u1".into(), room_id: "r".into(),
            kind: MediaKind::Audio, mid: "0".into(), pt: 111, rtx_pt: None, codec: "opus", fmtp: None,
            source: None, duplex: Duplex::Full, simulcast: false, ssrc: 0x1234_5678, rtx_ssrc: None,
        });
        let subs = SubscribeContext::new(PcMode::TwoPc);
        // 이 구독자는 abs-send-time 을 3 번으로 협상했고 twcc 는 아예 없다.
        subs.set_extmap(vec![Extmap { id: 3, uri: crate::media::URI_ABS_SEND_TIME.into() }]);
        let sub = subs.insert(&stream, SubSpec { subscriber: "u2".into(), room_id: "r".into(), mid: Some(0), pt: 111, transport: None, now_ms: 0 });
        sub.set_ext(subs.ext_table(&publish.extmap()));

        let src = packet();
        let mut egress = Vec::with_capacity(EGRESS_BUF);
        let cap = egress.capacity();
        for _ in 0..1_000 {
            assemble(&mut egress, &src, &sub);
        }
        assert_eq!(egress.capacity(), cap, "구독자·패킷이 늘어도 힙을 다시 잡지 않는다");
        assert_eq!(egress.len(), src.len(), "길이 불변 — 바꾸면 SRTP 재암호가 어긋난다");
        assert_eq!((rtp::payload_type(&egress), rtp::payload_type(&src)), (Some(111), Some(96)));
        assert_eq!(egress[1] & 0x80, 0x80, "marker 보존");
        assert_eq!(rtp::extension_ids(&src), vec![5, 6]);
        assert_eq!(rtp::extension_ids(&egress), vec![3, 14], "abs-send-time 은 구독자 번호로, 표에 없는 twcc 는 표 밖 번호로");
        assert_eq!(&egress[20..], &src[20..], "본문 무접촉");
    }
}
