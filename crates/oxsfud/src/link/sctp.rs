// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-LINK-TRANSPORT 정:IMPL-AUX-DC-SEND 정:RULE-DOC-FIXED-PARTS 연:RULE-LINK-SDP-DC-OPENER 연:TRN.frame.DC · model: claude-opus-5-5
//! SCTP(sctp-proto · sans-IO) 위 DCEP — 채널 `"unreliable"` 하나(다른 라벨 거부) · 종류는 DCEP 가 실어 온 대로 · SCTP 패킷 하나 = DTLS 레코드 하나 · 데이터 재전송 한도 없음.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use bytes::Bytes;
use dtls::conn::DTLSConn;
use sctp_proto::{
    Association, AssociationHandle, DatagramEvent, Endpoint, EndpointConfig, Event, Payload, PayloadProtocolIdentifier, ReliabilityType, ServerConfig, StreamEvent, StreamId,
    TransportConfig,
};
use tokio::sync::mpsc;

use super::transport::Transport;

pub const LABEL: &str = "unreliable";
const DCEP_OPEN: u8 = 0x03;
const DCEP_ACK: u8 = 0x02;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    pub channel_type: u8,
    pub reliability: u32,
    pub label: String,
}

pub fn parse_open(b: &[u8]) -> Option<Open> {
    if b.len() < 12 || b[0] != DCEP_OPEN {
        return None;
    }
    let channel_type = b[1];
    let reliability = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    let ll = u16::from_be_bytes([b[8], b[9]]) as usize;
    let label = b.get(12..12 + ll).and_then(|x| std::str::from_utf8(x).ok())?.to_string();
    Some(Open { channel_type, reliability, label })
}

pub fn reliability_of(channel_type: u8) -> (bool, ReliabilityType) {
    let unordered = channel_type & 0x80 != 0;
    let t = match channel_type & 0x7f {
        0x01 => ReliabilityType::Rexmit,
        0x02 => ReliabilityType::Timed,
        _ => ReliabilityType::Reliable,
    };
    (unordered, t)
}

fn remote() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 5000))
}

pub async fn run(t: Arc<Transport>, conn: Arc<DTLSConn>, mut out: mpsc::Receiver<Vec<u8>>) {
    let tc = TransportConfig::default().with_max_init_retransmits(None).with_max_data_retransmits(None);
    let mut sc = ServerConfig::new();
    sc.transport = Arc::new(tc);
    let mut ep = Endpoint::new(Arc::new(EndpointConfig::new()), Some(Arc::new(sc)));
    let mut assoc: Option<(AssociationHandle, Association)> = None;
    let mut chan: Option<StreamId> = None;
    let mut buf = vec![0u8; 65_536];
    loop {
        let deadline = assoc.as_ref().and_then(|(_, a)| a.poll_timeout());
        let sleep_for = deadline.map(|d| d.saturating_duration_since(Instant::now())).unwrap_or(std::time::Duration::from_secs(3600));
        tokio::select! {
            _ = t.cancel.cancelled() => break,
            r = conn.read(&mut buf, None) => {
                let Ok(n) = r else { break };
                let data = Bytes::copy_from_slice(&buf[..n]);
                match ep.handle(Instant::now(), remote(), None, None, data) {
                    Some((h, DatagramEvent::NewAssociation(a))) => assoc = Some((h, a)),
                    Some((_, DatagramEvent::AssociationEvent(ev))) => {
                        if let Some((_, a)) = assoc.as_mut() {
                            a.handle_event(ev);
                        }
                    }
                    None => {}
                }
            }
            m = out.recv() => {
                let Some(m) = m else { break };
                if let (Some((_, a)), Some(id)) = (assoc.as_mut(), chan)
                    && let Ok(mut s) = a.stream(id)
                {
                    let ppi = if m.is_empty() { PayloadProtocolIdentifier::BinaryEmpty } else { PayloadProtocolIdentifier::Binary };
                    let body = if m.is_empty() { Bytes::from_static(&[0]) } else { Bytes::from(m) };
                    let _ = s.write_sctp(&body, ppi);
                }
            }
            _ = tokio::time::sleep(sleep_for) => {
                if let Some((_, a)) = assoc.as_mut() {
                    a.handle_timeout(Instant::now());
                }
            }
        }
        let Some((h, a)) = assoc.as_mut() else { continue };
        while let Some(ev) = a.poll_endpoint_event() {
            let _ = ep.handle_event(*h, ev);
        }
        while a.accept_stream().is_some() {}
        while let Some(ev) = a.poll() {
            if let Event::Stream(StreamEvent::Readable { id }) = ev {
                read_stream(&t, a, id, &mut chan);
            }
        }
        let now = Instant::now();
        while let Some(tx) = a.poll_transmit(now) {
            if let Payload::RawEncode(pkts) = tx.payload {
                for p in pkts {
                    let _ = conn.write(&p, None).await;
                }
            }
        }
        if a.is_closed() {
            break;
        }
    }
    t.dc_open.store(false, Ordering::Release);
}

fn read_stream(t: &Transport, a: &mut Association, id: StreamId, chan: &mut Option<StreamId>) {
    let Ok(mut s) = a.stream(id) else { return };
    while let Ok(Some(chunks)) = s.read_sctp() {
        let mut data = vec![0u8; chunks.len()];
        let n = chunks.read(&mut data).unwrap_or(0);
        data.truncate(n);
        match chunks.ppi {
            PayloadProtocolIdentifier::Dcep => {
                let Some(open) = parse_open(&data) else { continue };
                if open.label != LABEL {
                    let _ = s.stop();
                    continue;
                }
                let (unordered, ty) = reliability_of(open.channel_type);
                let _ = s.set_reliability_params(unordered, ty, open.reliability);
                let _ = s.write_sctp(&Bytes::from_static(&[DCEP_ACK]), PayloadProtocolIdentifier::Dcep);
                let _ = chan.replace(id);
                t.dc_open.store(true, Ordering::Release);
            }
            PayloadProtocolIdentifier::Binary | PayloadProtocolIdentifier::String => {
                let _ = t.dc_in_tx.try_send(data);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dcep_open_parse() {
        let mut b = vec![DCEP_OPEN, 0x81, 0, 0, 0, 0, 0, 0, 0, 10, 0, 0];
        b.extend_from_slice(b"unreliable");
        let o = parse_open(&b).expect("open");
        assert_eq!(o.label, LABEL);
        let (u, t) = reliability_of(o.channel_type);
        assert!(u);
        assert_eq!(t, ReliabilityType::Rexmit);
    }
}
