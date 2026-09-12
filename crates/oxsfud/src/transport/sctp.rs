// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§12 · §13 · 연§3-3 · model: claude-opus-5

//! SCTP over DTLS — ★**DataChannel 이 서는 자리.**
//!
//! `sctp-proto` 는 Sans-I/O 다 — 바이트를 넣고 나갈 바이트를 받는다. 그래서 여기가
//! ★**DTLS 와 SCTP 사이의 손**이고, 판정은 전부 그 크레이트가 한다.
//!
//! ★★**datagram 하나당 DTLS 레코드 하나다**(정§12). `poll_transmit` 이 낸 조각들을
//! 이어붙여 한 레코드로 보내면 ★**받는 쪽이 그것을 SCTP 패킷 하나로 읽어** 두 번째부터
//! 통째로 버린다 — 유실과 같아져 T3-rtx(1초) 만료 뒤에야 재전송된다. 20260814 실측에서
//! 발언권 `GRANTED` 가 SACK 과 같은 배치에 실릴 때마다 사라져 access time 이 1,005ms 였다
//! (MCPTT KPI 300ms 의 3.4배). ★**`total 1` 인 배치만 전부 도달했다** — 결정론적이었다.

use std::net::SocketAddr;
use std::time::Instant;

use bytes::Bytes;
use dtls::conn::DTLSConn;
use sctp_proto::{
    Association, AssociationHandle, DatagramEvent, Endpoint, EndpointConfig, Event,
    PayloadProtocolIdentifier, ServerConfig, StreamEvent, StreamId, Transmit,
};
use tokio::sync::mpsc;
use webrtc_util::Conn;

use super::dc;

/// SCTP 가 쓰는 그릇 — DTLS 레코드 하나가 이보다 크면 우리 것이 아니다.
const BUF: usize = 8192;
/// 되보낼 것을 챙기는 주기.
const TIMER_MS: u64 = 50;

/// 밖에서 DC 로 흘려보낼 것. ★**준비 전에는 부르는 쪽이 버퍼를 쥔다**(정§13 — 64, 오래된 것부터).
pub type DcTx = mpsc::Sender<Vec<u8>>;

/// 루프가 밖에 알리는 것.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DcEvent {
    /// ★**채널이 섰다** — 이때부터 발언권이 흐른다(연§3-3 *"안 서면 발언권을 못 쓴다"*).
    Open,
    /// 그 `svc` 로 뭔가 왔다.
    Frame { svc: u8, payload: Vec<u8> },
    /// 채널이 갔다.
    Closed,
}

/// DTLS 하나 위에서 SCTP 를 돈다. ★**되돌아오지 않는다** — 태스크로 띄우고, 회수는 abort 다.
///
/// ★**`cancel` 을 따로 두지 않는다** — 이 태스크의 주인(`Pipe`)이 `Drop` 에서 끊는다.
/// 신호를 둘로 두면 한쪽만 지나가는 창이 생긴다(정§12 *"회수 시 태스크 종료 필수"*).
pub async fn run(conn: &DTLSConn, mut out_rx: mpsc::Receiver<Vec<u8>>, events: mpsc::Sender<DcEvent>) {
    let mut endpoint = Endpoint::new(
        std::sync::Arc::new(EndpointConfig::default()),
        Some(std::sync::Arc::new(ServerConfig::default())),
    );
    let mut assoc: Option<(AssociationHandle, Association)> = None;
    let mut channel: Option<StreamId> = None;
    // SCTP 에게 상대 주소는 뜻이 없다 — DTLS 가 이미 한 상대에 묶여 있다.
    let peer: SocketAddr = "127.0.0.1:1".parse().expect("고정값");
    let mut buf = vec![0u8; BUF];

    let mut timer = tokio::time::interval(std::time::Duration::from_millis(TIMER_MS));
    timer.tick().await;

    loop {
        let lost = tokio::select! {
            r = Conn::recv(conn, &mut buf) => {
                let Ok(n) = r else { break };
                if n == 0 {
                    break;
                }
                let data = Bytes::copy_from_slice(&buf[..n]);
                if let Some((h, ev)) = endpoint.handle(Instant::now(), peer, None, None, data) {
                    match ev {
                        DatagramEvent::NewAssociation(a) => assoc = Some((h, a)),
                        DatagramEvent::AssociationEvent(e) => {
                            if let Some((_, a)) = assoc.as_mut() {
                                a.handle_event(e);
                            }
                        }
                    }
                }
                pump(&mut endpoint, &mut assoc, &mut channel, conn, &events).await
            }
            _ = timer.tick() => {
                if let Some((_, a)) = assoc.as_mut() {
                    a.handle_timeout(Instant::now());
                }
                pump(&mut endpoint, &mut assoc, &mut channel, conn, &events).await
            }
            Some(pkt) = out_rx.recv() => {
                if let (Some((_, a)), Some(sid)) = (assoc.as_mut(), channel)
                    && let Ok(mut s) = a.stream(sid)
                    && let Err(e) = s.write_with_ppi(&pkt, PayloadProtocolIdentifier::Binary)
                {
                    // ★**계수이지 연결 종료 사유가 아니다**(정§13).
                    eprintln!("[dc] 보내기 실패: {e:?}");
                }
                pump(&mut endpoint, &mut assoc, &mut channel, conn, &events).await
            }
        };
        if lost {
            break;
        }
    }
    let _ = events.send(DcEvent::Closed).await;
}

/// 사건을 비우고 나갈 것을 내보낸다. ★**association 이 죽었으면 참**을 낸다.
async fn pump(
    endpoint: &mut Endpoint,
    assoc: &mut Option<(AssociationHandle, Association)>,
    channel: &mut Option<StreamId>,
    conn: &DTLSConn,
    events: &mpsc::Sender<DcEvent>,
) -> bool {
    let mut lost = false;
    if let Some((handle, a)) = assoc.as_mut() {
        while let Some(ev) = a.poll() {
            match ev {
                Event::Stream(se) => on_stream(a, se, channel, events).await,
                Event::AssociationLost { reason, .. } => {
                    eprintln!("[dc] association 끊김: {reason:?}");
                    lost = true;
                }
                _ => {}
            }
        }
        let now = Instant::now();
        while let Some(t) = a.poll_transmit(now) {
            send(conn, &t).await;
        }
        while let Some(e) = a.poll_endpoint_event() {
            if let Some(ae) = endpoint.handle_event(*handle, e) {
                a.handle_event(ae);
            }
        }
    }
    while let Some(t) = endpoint.poll_transmit() {
        send(conn, &t).await;
    }
    lost
}

async fn on_stream(
    a: &mut Association,
    ev: StreamEvent,
    channel: &mut Option<StreamId>,
    events: &mpsc::Sender<DcEvent>,
) {
    let StreamEvent::Readable { id } = ev else { return };
    let Ok(mut stream) = a.stream(id) else { return };
    let Ok(Some(chunks)) = stream.read() else { return };
    let ppi = chunks.ppi;
    let mut buf = vec![0u8; BUF];
    let Ok(n) = chunks.read(&mut buf) else { return };
    let data = &buf[..n];

    if ppi == PayloadProtocolIdentifier::Dcep {
        let Some(dc::Dcep::Open { label }) = dc::parse_dcep(data) else { return };
        if label != dc::LABEL {
            // ★**거부는 침묵이다** — DCEP 에 거부 메시지가 없다(연§3-3). ACK 을 안 보내는 것이 거부다.
            eprintln!("[dc] 모르는 채널 이름 {label:?} — 열지 않는다");
            return;
        }
        *channel = Some(id);
        if let Ok(mut s) = a.stream(id)
            && let Err(e) = s.write_with_ppi(&dc::ack(), PayloadProtocolIdentifier::Dcep)
        {
            eprintln!("[dc] ACK 실패: {e:?}");
            return;
        }
        let _ = events.send(DcEvent::Open).await;
        return;
    }
    // ★**잘린 프레임은 조용히 버린다**(연§3-3).
    if let Some((svc, payload)) = dc::parse(data) {
        let _ = events.send(DcEvent::Frame { svc, payload: payload.to_vec() }).await;
    }
}

/// ★**조각 하나가 레코드 하나다** — 이어붙이면 받는 쪽이 첫 것만 읽고 나머지를 삼킨다.
async fn send(conn: &DTLSConn, t: &Transmit) {
    let sctp_proto::Payload::RawEncode(chunks) = &t.payload else { return };
    for c in chunks {
        if let Err(e) = Conn::send(conn, c).await {
            eprintln!("[dc] DTLS 로 못 보냈다: {e}");
            return;
        }
    }
}
