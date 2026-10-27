// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-TRANSPORT · 정:CAND-LINK-DTLS-VALUES · 정:RULE-GC-LEAVE-SEVEN · 연:RULE-LINK-SDP-DC-OPENER · 연:TRN.frame.DC · model: claude-opus-5-5
//! DTLS(passive) → SRTP 키 → SCTP 연합 → DCEP · 채널 `"unreliable"` 하나. 연합 수명 ≥ Peer 수명(데이터 재전송 한도 없음) · transmit 은 datagram 단위 레코드 · 태스크는 Peer 회수가 직접 끊는다.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use dtls::config::{ClientAuthType, Config, ExtendedMasterSecretType};
use dtls::conn::DTLSConn;
use dtls::crypto::Certificate;
use dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use sctp_proto::{Association, AssociationHandle, DatagramEvent, Endpoint, EndpointConfig, Event, Payload, PayloadProtocolIdentifier, ReliabilityType, ServerConfig, StreamEvent, TransportConfig};
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use webrtc_srtp::config::Config as SrtpConfig;
use webrtc_srtp::context::Context;
use webrtc_srtp::option::srtp_replay_protection;
use webrtc_srtp::protection_profile::ProtectionProfile;
use webrtc_util::conn::Conn;

use super::io::DcEvent;
use super::transport::{Srtp, Transport};

pub const HANDSHAKE_TIMEOUT_MS: u64 = 30_000;
pub const DTLS_MTU: usize = 1_200;
pub const CHANNEL_LABEL: &str = "unreliable";
const DCEP_OPEN: u8 = 0x03;
const DCEP_ACK: u8 = 0x02;

struct ChanConn {
    rx: AsyncMutex<mpsc::Receiver<Vec<u8>>>,
    t: Weak<Transport>,
}

#[async_trait]
impl Conn for ChanConn {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let mut r = self.rx.lock().await;
        match r.recv().await {
            Some(p) => {
                let n = p.len().min(buf.len());
                buf[..n].copy_from_slice(&p[..n]);
                Ok(n)
            }
            None => Err(webrtc_util::Error::ErrUseClosedNetworkConn),
        }
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let n = self.recv(buf).await?;
        Ok((n, self.remote_addr().unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)))))
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        if let Some(t) = self.t.upgrade() {
            t.send_raw(buf);
        }
        Ok(buf.len())
    }

    async fn send_to(&self, buf: &[u8], _target: SocketAddr) -> webrtc_util::Result<usize> {
        self.send(buf).await
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(SocketAddr::from(([0, 0, 0, 0], 0)))
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        self.t.upgrade().and_then(|t| t.path_addr())
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

fn config(cert: Certificate) -> Config {
    Config {
        certificates: vec![cert],
        srtp_protection_profiles: vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80],
        extended_master_secret: ExtendedMasterSecretType::Require,
        client_auth: ClientAuthType::RequireAnyClientCert,
        insecure_skip_verify: true,
        mtu: DTLS_MTU,
        ..Config::default()
    }
}

async fn srtp_from(conn: &DTLSConn) -> Option<Srtp> {
    let state = conn.connection_state().await;
    let mut c = SrtpConfig { profile: ProtectionProfile::Aes128CmHmacSha1_80, ..Default::default() };
    c.extract_session_keys_from_dtls(state, false).await.ok()?;
    let tx = Context::new(&c.keys.local_master_key, &c.keys.local_master_salt, c.profile, None, None).ok()?;
    let rx = Context::new(&c.keys.remote_master_key, &c.keys.remote_master_salt, c.profile, Some(srtp_replay_protection(64)), None).ok()?;
    Some(Srtp { rx: std::sync::Mutex::new(rx), tx: std::sync::Mutex::new(tx) })
}

pub async fn run(t: Arc<Transport>, rx: mpsc::Receiver<Vec<u8>>, cert: Certificate, events: mpsc::UnboundedSender<DcEvent>) {
    let conn: Arc<dyn Conn + Send + Sync> = Arc::new(ChanConn { rx: AsyncMutex::new(rx), t: Arc::downgrade(&t) });
    let hs = tokio::time::timeout(Duration::from_millis(HANDSHAKE_TIMEOUT_MS), DTLSConn::new(conn, config(cert), false, None)).await;
    let dtls = match hs {
        Ok(Ok(c)) => Arc::new(c),
        Ok(Err(e)) => {
            tracing::warn!(transport = t.id, error = %e, "DTLS 핸드셰이크 실패");
            t.dtls_started.store(false, Ordering::Release);
            return;
        }
        Err(_) => {
            tracing::warn!(transport = t.id, "DTLS 핸드셰이크 시한(30s)");
            t.dtls_started.store(false, Ordering::Release);
            return;
        }
    };
    match srtp_from(&dtls).await {
        Some(s) => t.srtp.store(Some(Arc::new(s))),
        None => tracing::warn!(transport = t.id, "SRTP 키를 못 냈다"),
    }
    tracing::info!(transport = t.id, role = t.role.as_str(), "DTLS 수립");
    if t.role.publishes() {
        sctp(t, dtls, events).await;
    } else {
        let mut buf = vec![0u8; 2_048];
        while dtls.read(&mut buf, None).await.is_ok() {}
    }
}

fn reliability(channel_type: u8) -> (bool, ReliabilityType) {
    let unordered = channel_type & 0x80 != 0;
    let rel = match channel_type & 0x7f {
        0x01 => ReliabilityType::Rexmit,
        0x02 => ReliabilityType::Timed,
        _ => ReliabilityType::Reliable,
    };
    (unordered, rel)
}

struct Dc {
    assoc: Option<(AssociationHandle, Association)>,
    stream: Option<u16>,
}

async fn flush(endpoint: &mut Endpoint, dc: &mut Dc, dtls: &DTLSConn) {
    while let Some(tx) = endpoint.poll_transmit() {
        write_payload(dtls, tx.payload).await;
    }
    let Some((h, a)) = dc.assoc.as_mut() else { return };
    let now = Instant::now();
    while let Some(tx) = a.poll_transmit(now) {
        write_payload(dtls, tx.payload).await;
    }
    while let Some(ev) = a.poll_endpoint_event() {
        if let Some(back) = endpoint.handle_event(*h, ev) {
            a.handle_event(back);
        }
    }
}

async fn write_payload(dtls: &DTLSConn, p: Payload) {
    if let Payload::RawEncode(chunks) = p {
        for c in chunks {
            let _ = dtls.write(&c, None).await;
        }
    }
}

fn drain_events(t: &Arc<Transport>, dc: &mut Dc, events: &mpsc::UnboundedSender<DcEvent>) {
    let Some((_, a)) = dc.assoc.as_mut() else { return };
    while let Some(ev) = a.poll() {
        let id = match ev {
            Event::Stream(StreamEvent::Readable { id }) | Event::Stream(StreamEvent::Opened { id }) => id,
            _ => continue,
        };
        while let Some(s) = a.accept_stream() {
            let _ = s.stream_identifier();
        }
        let Ok(mut s) = a.stream(id) else { continue };
        loop {
            let chunks = match s.read_sctp() {
                Ok(Some(c)) => c,
                _ => break,
            };
            let mut buf = vec![0u8; chunks.len()];
            let Ok(n) = chunks.read(&mut buf) else { continue };
            buf.truncate(n);
            match chunks.ppi {
                PayloadProtocolIdentifier::Dcep => {
                    if buf.first() == Some(&DCEP_OPEN) && buf.len() >= 12 {
                        let channel_type = buf[1];
                        let rel_val = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
                        let llen = usize::from(u16::from_be_bytes([buf[8], buf[9]]));
                        let label = buf.get(12..12 + llen).map(|l| String::from_utf8_lossy(l).to_string()).unwrap_or_default();
                        if label != CHANNEL_LABEL {
                            tracing::warn!(transport = t.id, label = %label, "DC 라벨 거부");
                            let _ = s.stop();
                            continue;
                        }
                        let (unordered, rel) = reliability(channel_type);
                        let _ = s.set_reliability_params(unordered, rel, rel_val);
                        let _ = s.write_sctp(&Bytes::from_static(&[DCEP_ACK]), PayloadProtocolIdentifier::Dcep);
                        dc.stream = Some(id);
                        let _ = events.send(DcEvent::Open(t.clone()));
                    }
                }
                PayloadProtocolIdentifier::Binary | PayloadProtocolIdentifier::String => {
                    if dc.stream.is_none() {
                        dc.stream = Some(id);
                    }
                    let _ = events.send(DcEvent::Message(t.clone(), buf));
                }
                _ => {}
            }
        }
    }
}

async fn sctp(t: Arc<Transport>, dtls: Arc<DTLSConn>, events: mpsc::UnboundedSender<DcEvent>) {
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    if let Ok(mut g) = t.dc_tx.lock() {
        g.replace(out_tx);
    }
    let (pkt_tx, mut pkt_rx) = mpsc::channel::<Vec<u8>>(512);
    let reader = dtls.clone();
    let rh = tokio::spawn(async move {
        let mut buf = vec![0u8; 65_536];
        while let Ok(n) = reader.read(&mut buf, None).await {
            if pkt_tx.send(buf[..n].to_vec()).await.is_err() {
                break;
            }
        }
    });
    t.add_task(rh.abort_handle());
    let mut sc = ServerConfig::new();
    sc.transport = Arc::new(TransportConfig::default().with_max_data_retransmits(None).with_max_init_retransmits(None));
    let mut endpoint = Endpoint::new(Arc::new(EndpointConfig::new()), Some(Arc::new(sc)));
    let mut dc = Dc { assoc: None, stream: None };
    let remote = SocketAddr::from(([127, 0, 0, 1], 5_000));
    loop {
        let deadline = dc.assoc.as_ref().and_then(|(_, a)| a.poll_timeout());
        let sleep = async {
            match deadline {
                Some(d) => tokio::time::sleep_until(tokio::time::Instant::from_std(d)).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            p = pkt_rx.recv() => {
                let Some(p) = p else { break };
                if let Some((h, ev)) = endpoint.handle(Instant::now(), remote, None, None, Bytes::from(p)) {
                    match ev {
                        DatagramEvent::NewAssociation(a) => dc.assoc = Some((h, a)),
                        DatagramEvent::AssociationEvent(e) => {
                            if let Some((_, a)) = dc.assoc.as_mut() {
                                a.handle_event(e);
                            }
                        }
                    }
                }
            }
            m = out_rx.recv() => {
                let Some(m) = m else { break };
                if let (Some((_, a)), Some(sid)) = (dc.assoc.as_mut(), dc.stream)
                    && let Ok(mut s) = a.stream(sid)
                {
                    let _ = s.write_sctp(&Bytes::from(m), PayloadProtocolIdentifier::Binary);
                }
            }
            _ = sleep => {
                if let Some((_, a)) = dc.assoc.as_mut() {
                    a.handle_timeout(Instant::now());
                }
            }
        }
        drain_events(&t, &mut dc, &events);
        flush(&mut endpoint, &mut dc, &dtls).await;
    }
    rh.abort();
}
