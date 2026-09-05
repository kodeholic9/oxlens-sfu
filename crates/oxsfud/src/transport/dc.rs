// author: kodeholic (powered by Claude)
//! SCTP/DataChannel — 정§12 SCTP·§13 부가 통신. 채널은 `"unreliable"` 하나(연§9-7), 프레임은 4B 헤더(연§3-3).
//! ★transmit 은 datagram 하나당 DTLS 레코드 하나다 — 이어붙이면 수신측이 첫 패킷만 파싱해 나머지를 삼킨다.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use ::dtls::conn::DTLSConn;
use oxsig::dc;
use sctp_proto::{
    Association, AssociationHandle, DatagramEvent, Endpoint, EndpointConfig, Event, PayloadProtocolIdentifier, ServerConfig, StreamEvent, StreamId, Transmit,
};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use webrtc_util::conn::Conn;

use super::session::TransportSession;
use crate::handlers::Sfu;

/// 송신 큐 — 준비 전 버퍼는 세션이 따로 든다(정§13).
const OUT_QUEUE: usize = 64;
const TICK_MS: u64 = 50;
const READ_BUF: usize = 65_536;

/// DTLS 가 선 뒤 이 루프가 그 연결을 마저 읽는다. 종료 조건 셋: 취소 · DTLS 닫힘 · association 상실.
pub async fn run(dtls_conn: &DTLSConn, sfu: Arc<Sfu>, session: Arc<TransportSession>) {
    let mut endpoint = Endpoint::new(Arc::new(EndpointConfig::default()), Some(Arc::new(ServerConfig::default())));
    let mut assoc: Option<(AssociationHandle, Association)> = None;
    let mut stream: Option<StreamId> = None;
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(OUT_QUEUE);
    let mut buf = vec![0u8; READ_BUF];
    let mut tick = tokio::time::interval(tokio::time::Duration::from_millis(TICK_MS));
    tick.tick().await;
    // sctp-proto 는 주소로 association 을 가른다. 우리는 DTLS 하나에 하나뿐이라 고정값이면 족하다.
    let peer: SocketAddr = "127.0.0.1:1".parse().expect("literal socket address");
    let user = session.user_id.clone();
    info!(user = %user, role = session.role.as_str(), "sctp loop up");

    loop {
        let mut lost = false;
        tokio::select! {
            _ = session.cancel.cancelled() => break,
            got = Conn::recv(dtls_conn, &mut buf) => {
                let n = match got {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let data = Bytes::copy_from_slice(&buf[..n]);
                if let Some((handle, ev)) = endpoint.handle(Instant::now(), peer, None, None, data) {
                    match ev {
                        DatagramEvent::NewAssociation(a) => {
                            info!(user = %user, "sctp association established");
                            assoc = Some((handle, a));
                        }
                        DatagramEvent::AssociationEvent(ae) => {
                            if let Some((_, a)) = assoc.as_mut() {
                                a.handle_event(ae);
                            }
                        }
                    }
                }
                if let Some((handle, a)) = assoc.as_mut() {
                    lost = poll_association(a, &mut stream, &sfu, &session, &out_tx);
                    drain(&mut endpoint, handle, a, dtls_conn).await;
                }
                drain_endpoint(&mut endpoint, dtls_conn).await;
            }
            _ = tick.tick() => {
                if let Some((handle, a)) = assoc.as_mut() {
                    a.handle_timeout(Instant::now());
                    lost = poll_association(a, &mut stream, &sfu, &session, &out_tx);
                    drain(&mut endpoint, handle, a, dtls_conn).await;
                }
                drain_endpoint(&mut endpoint, dtls_conn).await;
            }
            Some(frame) = out_rx.recv() => {
                if let (Some((handle, a)), Some(sid)) = (assoc.as_mut(), stream) {
                    write_frame(a, sid, &frame, &user);
                    drain(&mut endpoint, handle, a, dtls_conn).await;
                }
                drain_endpoint(&mut endpoint, dtls_conn).await;
            }
        }
        if lost {
            warn!(user = %user, "sctp association lost");
            break;
        }
    }

    session.dc_close();
    info!(user = %user, role = session.role.as_str(), dropped = session.dc_dropped(), "sctp loop down");
}

/// 반환: association 을 잃었나.
fn poll_association(
    assoc: &mut Association,
    stream: &mut Option<StreamId>,
    sfu: &Arc<Sfu>,
    session: &Arc<TransportSession>,
    out_tx: &mpsc::Sender<Vec<u8>>,
) -> bool {
    let mut lost = false;
    while let Some(event) = assoc.poll() {
        match event {
            Event::Connected => debug!(user = %session.user_id, "sctp connected"),
            Event::AssociationLost { .. } => lost = true,
            Event::Stream(StreamEvent::Readable { id }) => readable(assoc, id, stream, sfu, session, out_tx),
            _ => {}
        }
    }
    lost
}

fn readable(
    assoc: &mut Association,
    id: StreamId,
    stream: &mut Option<StreamId>,
    sfu: &Arc<Sfu>,
    session: &Arc<TransportSession>,
    out_tx: &mpsc::Sender<Vec<u8>>,
) {
    let Ok(mut s) = assoc.stream(id) else { return };
    let Ok(Some(chunks)) = s.read() else { return };
    let ppi = chunks.ppi;
    let mut buf = vec![0u8; READ_BUF];
    let Ok(n) = chunks.read(&mut buf) else { return };
    let data = &buf[..n];
    match ppi {
        PayloadProtocolIdentifier::Dcep => {
            let Some(super::dcep::Message::Open { label }) = super::dcep::parse(data) else { return };
            if label != dc::CHANNEL_LABEL {
                warn!(user = %session.user_id, label = %label, "datachannel label refused");
                return;
            }
            *stream = Some(id);
            if let Ok(mut s) = assoc.stream(id)
                && let Err(e) = s.write_with_ppi(&super::dcep::ack(), PayloadProtocolIdentifier::Dcep)
            {
                warn!(user = %session.user_id, error = %e, "dcep ack failed");
                return;
            }
            session.dc_open(out_tx.clone());
            info!(user = %session.user_id, stream = id, "datachannel open");
        }
        PayloadProtocolIdentifier::Binary => match dc::decode(data) {
            Some((svc, payload)) => sfu.on_dc_frame(session, svc, payload),
            None => debug!(user = %session.user_id, len = n, "dc frame dropped"),
        },
        _ => debug!(user = %session.user_id, "dc payload protocol ignored"),
    }
}

fn write_frame(assoc: &mut Association, sid: StreamId, frame: &[u8], user: &str) {
    match assoc.stream(sid) {
        Ok(mut s) => {
            if let Err(e) = s.write_with_ppi(frame, PayloadProtocolIdentifier::Binary) {
                warn!(user = %user, error = %e, "dc write failed");
            }
        }
        Err(e) => warn!(user = %user, error = %e, "dc stream gone"),
    }
}

async fn drain(endpoint: &mut Endpoint, handle: &AssociationHandle, assoc: &mut Association, dtls_conn: &DTLSConn) {
    let now = Instant::now();
    while let Some(t) = assoc.poll_transmit(now) {
        send(dtls_conn, &t).await;
    }
    while let Some(ev) = assoc.poll_endpoint_event() {
        if let Some(back) = endpoint.handle_event(*handle, ev) {
            assoc.handle_event(back);
        }
    }
    drain_endpoint(endpoint, dtls_conn).await;
}

async fn drain_endpoint(endpoint: &mut Endpoint, dtls_conn: &DTLSConn) {
    while let Some(t) = endpoint.poll_transmit() {
        send(dtls_conn, &t).await;
    }
}

/// 정§12 — datagram 하나가 레코드 하나. `RawEncode` 의 각 원소는 각자 SCTP 공통 헤더를 가진 독립 패킷이다.
async fn send(dtls_conn: &DTLSConn, transmit: &Transmit) {
    let sctp_proto::Payload::RawEncode(chunks) = &transmit.payload else { return };
    for chunk in chunks {
        if chunk.is_empty() {
            continue;
        }
        if let Err(e) = Conn::send(dtls_conn, &chunk[..]).await {
            debug!(error = %e, "sctp transmit failed");
            return;
        }
    }
}
