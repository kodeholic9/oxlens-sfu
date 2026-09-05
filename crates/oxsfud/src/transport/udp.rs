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
use crate::handlers::Sfu;
use crate::media::rtp;
use crate::media::subscribe::SubscribeState;
use crate::media::track::PublishState;
use oxsig::schema::{Duplex, MediaKind};

const RECV_BUF: usize = 2_048;
const HANDSHAKE_TIMEOUT_MS: u64 = 10_000;

pub async fn bind(port: u16) -> std::io::Result<Arc<UdpSocket>> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    Ok(Arc::new(UdpSocket::bind(addr).await?))
}

pub async fn run(sfu: Arc<Sfu>, socket: Arc<UdpSocket>) {
    let mut buf = vec![0u8; RECV_BUF];
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
            demux::Packet::Srtp => on_srtp(&sfu, &socket, data, remote).await,
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
async fn on_srtp(sfu: &Arc<Sfu>, socket: &Arc<UdpSocket>, data: &[u8], remote: SocketAddr) {
    let Some(session) = sfu.transport.by_addr(&remote) else { return };
    sfu.observe_media(&session.user_id);
    if rtp::is_rtcp(data) || session.role != ConnRole::Publish {
        return;
    }
    let plain = match session.decrypt_rtp(data) {
        Ok(p) => p,
        Err(e) => {
            debug!(user = %session.user_id, error = %e, "srtp decrypt");
            return;
        }
    };
    fan_out(sfu, socket, &session.user_id, &plain).await;
}

/// 정§7-3 — 순서가 계약이다. 방 결정은 ★매 패킷 그 발행자의 `pub_room`(RCU 읽기)이고,
/// egress PT 는 ★구독자 표의 값이며 확장 번호는 원소마다 다시 쓴다(정§7-2-1).
async fn fan_out(sfu: &Arc<Sfu>, socket: &Arc<UdpSocket>, publisher: &str, packet: &[u8]) {
    let Some(peer) = sfu.peers.get(publisher) else { return };
    let Some(ssrc) = rtp::ssrc(packet) else { return };
    let Some((stream, track)) = peer.publish.by_ssrc(ssrc) else { return };
    track.rtp_in.fetch_add(1, Ordering::Relaxed);
    stream.set_state(PublishState::Active);
    // 정§6-3 — 선언 PT 와 첫 RTP PT 의 불일치는 표면화만 한다. 고치면 검은 화면이 침묵한다.
    if let Some(pt) = rtp::payload_type(packet)
        && pt != stream.pt
        && stream.claim_pt_report()
    {
        warn!(user = publisher, track = %stream.track_id, declared = stream.pt, arrived = pt, "pt mismatch");
    }
    let Some(room_id) = peer.pub_room() else { return };
    if stream.muted() {
        return;
    }
    // 반이중은 prefan(정§9)이 권위이고 simulcast 는 레이어 선택(정§10)이 있어야 나간다.
    if stream.duplex() == Duplex::Half || stream.simulcast {
        return;
    }
    for sub in track.subscribers() {
        if sub.room_id != room_id {
            continue;
        }
        // 게이트는 ★전이중 video 에만 — audio 는 어떤 경우에도 gate 로 죽지 않는다.
        if sub.kind == MediaKind::Video && sub.state() != SubscribeState::Active {
            continue;
        }
        let (Some(transport), Some(addr)) = (sub.transport.as_ref(), sub.transport.as_ref().and_then(|t| t.addr.get())) else {
            continue;
        };
        let mut out = packet.to_vec();
        rtp::set_payload_type(&mut out, sub.pt());
        rtp::rewrite_extension_ids(&mut out, |id| sub.ext_of(id));
        let Ok(sealed) = transport.encrypt_rtp(&out) else { continue };
        if socket.send_to(&sealed, addr).await.is_ok() {
            sub.sent.fetch_add(1, Ordering::Relaxed);
        }
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
