// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-LINK-TRANSPORT 정:IMPL-AUX-DC-SEND 정:RULE-GC-LEAVE-SEVEN 연:RULE-LINK-SDP-DC-OPENER RFC 5764 RFC 8261 RFC 8832 · model: claude-opus-5-5
//! DTLS(서버 = passive) → SRTP 키 → SCTP(DCEP `"unreliable"` 하나). 신원은 전송 객체 — DTLS 는 주소가 아니라 Conn 위에 선다.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use sctp_proto::{
    AssociationHandle, DatagramEvent, Endpoint, EndpointConfig, Event, Payload, PayloadProtocolIdentifier,
    ReliabilityType, ServerConfig, StreamEvent, TransportConfig,
};
use tokio::sync::mpsc;

use crate::peer::{Conn, DC_BUFFER};
use crate::sfu::{Sfu, lock};

const DCEP_OPEN: u8 = 0x03;
const DCEP_ACK: u8 = 0x02;

struct Pipe {
    rx: tokio::sync::Mutex<mpsc::Receiver<Bytes>>,
    conn: Weak<Conn>,
    local: SocketAddr,
}

#[async_trait]
impl webrtc_util::Conn for Pipe {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Some(b) => {
                let n = b.len().min(buf.len());
                buf[..n].copy_from_slice(&b[..n]);
                Ok(n)
            }
            None => Err(webrtc_util::Error::ErrClosedListener),
        }
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let n = self.recv(buf).await?;
        Ok((n, self.remote_addr().unwrap_or(self.local)))
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        if let Some(c) = self.conn.upgrade() {
            c.send(buf);
        }
        Ok(buf.len())
    }

    async fn send_to(&self, buf: &[u8], _target: SocketAddr) -> webrtc_util::Result<usize> {
        self.send(buf).await
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.local)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        self.conn.upgrade().and_then(|c| c.path.load().as_ref().map(|p| p.addr()))
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

pub fn on_dtls(sfu: &Arc<Sfu>, conn: &Arc<Conn>, pkt: &[u8]) {
    let tx = {
        let mut g = lock(&conn.dtls_in);
        match g.as_ref() {
            Some(tx) => tx.clone(),
            None => {
                let (tx, rx) = mpsc::channel(256);
                g.replace(tx.clone());
                let s = sfu.clone();
                let c = conn.clone();
                tokio::spawn(async move { run(s, c, rx).await });
                tx
            }
        }
    };
    if tx.try_send(Bytes::copy_from_slice(pkt)).is_err() {
        sfu.counters.drop("dtls_queue");
    }
}

async fn run(sfu: Arc<Sfu>, conn: Arc<Conn>, rx: mpsc::Receiver<Bytes>) {
    let local = SocketAddr::new(sfu.ip, sfu.udp_port);
    let pipe = Arc::new(Pipe { rx: tokio::sync::Mutex::new(rx), conn: Arc::downgrade(&conn), local });
    let cfg = dtls::config::Config {
        certificates: vec![sfu.cert.clone()],
        srtp_protection_profiles: vec![dtls::extension::extension_use_srtp::SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80],
        client_auth: dtls::config::ClientAuthType::RequireAnyClientCert,
        extended_master_secret: dtls::config::ExtendedMasterSecretType::Require,
        insecure_skip_verify: true,
        ..Default::default()
    };
    let cancel = conn.cancel.clone();
    let made = tokio::select! {
        r = dtls::conn::DTLSConn::new(pipe, cfg, false, None) => r,
        _ = cancel.cancelled() => return,
    };
    let dtls = match made {
        Ok(d) => Arc::new(d),
        Err(e) => {
            tracing::warn!(kind = "diag", user = conn.user_id.as_str(), "dtls handshake failed: {e}");
            lock(&conn.dtls_in).take();
            return;
        }
    };
    if let Err(e) = keys(&conn, &dtls).await {
        tracing::warn!(kind = "diag", user = conn.user_id.as_str(), "srtp keys failed: {e}");
        return;
    }
    conn.secured.store(true, Ordering::SeqCst);
    tracing::info!(kind = "ev", spec = "RULE-LINK-TRANSPORT", user = conn.user_id.as_str(), conn = conn.id, "dtls connected");
    sctp_loop(&sfu, &conn, &dtls).await;
    let _ = dtls.close().await;
}

async fn keys(conn: &Conn, dtls: &dtls::conn::DTLSConn) -> Result<(), String> {
    let state = dtls.connection_state().await;
    let profile = webrtc_srtp::protection_profile::ProtectionProfile::Aes128CmHmacSha1_80;
    let mut cfg = webrtc_srtp::config::Config {
        keys: webrtc_srtp::config::SessionKeys {
            local_master_key: vec![],
            local_master_salt: vec![],
            remote_master_key: vec![],
            remote_master_salt: vec![],
        },
        profile,
        local_rtp_options: None,
        remote_rtp_options: None,
        local_rtcp_options: None,
        remote_rtcp_options: None,
    };
    cfg.extract_session_keys_from_dtls(state, false).await.map_err(|e| e.to_string())?;
    let out = webrtc_srtp::context::Context::new(&cfg.keys.local_master_key, &cfg.keys.local_master_salt, profile, None, None)
        .map_err(|e| e.to_string())?;
    let inn = webrtc_srtp::context::Context::new(&cfg.keys.remote_master_key, &cfg.keys.remote_master_salt, profile, None, None)
        .map_err(|e| e.to_string())?;
    lock(&conn.srtp_out).replace(out);
    lock(&conn.srtp_in).replace(inn);
    Ok(())
}

fn parse_open(b: &[u8]) -> Option<(u8, u32, String)> {
    if b.len() < 12 || b[0] != DCEP_OPEN {
        return None;
    }
    let ctype = b[1];
    let rel = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    let ll = u16::from_be_bytes([b[8], b[9]]) as usize;
    if b.len() < 12 + ll {
        return None;
    }
    Some((ctype, rel, String::from_utf8_lossy(&b[12..12 + ll]).to_string()))
}

async fn sctp_loop(sfu: &Arc<Sfu>, conn: &Arc<Conn>, dtls: &Arc<dtls::conn::DTLSConn>) {
    let transport = TransportConfig::default().with_max_data_retransmits(None).with_max_init_retransmits(None);
    let mut server = ServerConfig::new();
    server.transport = Arc::new(transport);
    let mut ep = Endpoint::new(Arc::new(EndpointConfig::new()), Some(Arc::new(server)));
    let remote: SocketAddr = SocketAddr::from(([0, 0, 0, 0], 5000));
    let mut assoc: Option<(AssociationHandle, sctp_proto::Association)> = None;
    let mut open_sid: Option<u16> = None;
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(256);
    lock(&conn.dc.tx).replace(tx);
    let cancel = conn.cancel.clone();
    let mut buf = vec![0u8; 65_536];
    loop {
        let deadline = assoc.as_ref().and_then(|(_, a)| a.poll_timeout()).unwrap_or_else(|| Instant::now() + Duration::from_secs(1));
        let wait = deadline.saturating_duration_since(Instant::now());
        tokio::select! {
            r = dtls.read(&mut buf, None) => {
                let n = match r {
                    Ok(n) => n,
                    Err(_) => break,
                };
                if let Some((h, ev)) = ep.handle(Instant::now(), remote, None, None, Bytes::copy_from_slice(&buf[..n])) {
                    match ev {
                        DatagramEvent::NewAssociation(a) => assoc = Some((h, a)),
                        DatagramEvent::AssociationEvent(e) => {
                            if let Some((_, a)) = assoc.as_mut() {
                                a.handle_event(e);
                            }
                        }
                    }
                }
            }
            m = rx.recv() => {
                let Some(m) = m else { break };
                match (assoc.as_mut(), open_sid) {
                    (Some((_, a)), Some(sid)) => {
                        let ok = a.stream(sid).and_then(|mut s| s.write_with_ppi(&m, PayloadProtocolIdentifier::Binary)).is_ok();
                        if !ok {
                            conn.dc.dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    _ => push_pending(conn, m),
                }
            }
            _ = tokio::time::sleep(wait) => {
                if let Some((_, a)) = assoc.as_mut() {
                    a.handle_timeout(Instant::now());
                }
            }
            _ = cancel.cancelled() => break,
        }
        let Some((h, a)) = assoc.as_mut() else { continue };
        while let Some(s) = a.accept_stream() {
            let _ = s.stream_identifier();
        }
        let mut readable: Vec<u16> = Vec::new();
        while let Some(ev) = a.poll() {
            if let Event::Stream(StreamEvent::Readable { id }) | Event::Stream(StreamEvent::Opened { id }) = ev {
                readable.push(id);
            }
        }
        for id in readable {
            loop {
                let chunks = match a.stream(id).and_then(|mut s| s.read_sctp()) {
                    Ok(Some(c)) => c,
                    _ => break,
                };
                let mut data = vec![0u8; chunks.len()];
                let n = chunks.read(&mut data).unwrap_or(0);
                data.truncate(n);
                match chunks.ppi {
                    PayloadProtocolIdentifier::Dcep => {
                        let Some((ctype, rel, label)) = parse_open(&data) else { continue };
                        if label != oxsig::dc::CHANNEL_LABEL {
                            tracing::warn!(kind = "diag", user = conn.user_id.as_str(), label = label.as_str(), "dc label rejected");
                            let _ = a.stream(id).and_then(|mut s| s.stop());
                            continue;
                        }
                        let unordered = ctype & 0x80 != 0;
                        let rt = match ctype & 0x7f {
                            1 => ReliabilityType::Rexmit,
                            2 => ReliabilityType::Timed,
                            _ => ReliabilityType::Reliable,
                        };
                        if let Ok(mut s) = a.stream(id) {
                            let _ = s.set_reliability_params(unordered, rt, rel);
                            let _ = s.write_with_ppi(&[DCEP_ACK], PayloadProtocolIdentifier::Dcep);
                        }
                        open_sid = Some(id);
                        tracing::info!(kind = "ev", spec = "RULE-LINK-TRANSPORT", user = conn.user_id.as_str(), sid = id, "dc open");
                        let pending: Vec<Vec<u8>> = lock(&conn.dc.pending).drain(..).collect();
                        for m in pending {
                            let _ = a.stream(id).and_then(|mut s| s.write_with_ppi(&m, PayloadProtocolIdentifier::Binary));
                        }
                    }
                    PayloadProtocolIdentifier::Binary | PayloadProtocolIdentifier::String => {
                        if Some(id) == open_sid {
                            crate::floor::on_dc(sfu, &conn.user_id, &data);
                        }
                    }
                    _ => {}
                }
            }
        }
        while let Some(e) = a.poll_endpoint_event() {
            if let Some(ae) = ep.handle_event(*h, e) {
                a.handle_event(ae);
            }
        }
        let mut out: Vec<Bytes> = Vec::new();
        while let Some(t) = a.poll_transmit(Instant::now()) {
            if let Payload::RawEncode(v) = t.payload {
                out.extend(v);
            }
        }
        for b in out {
            if dtls.write(&b, None).await.is_err() {
                break;
            }
        }
    }
    lock(&conn.dc.tx).take();
}

fn push_pending(conn: &Conn, m: Vec<u8>) {
    let mut p = lock(&conn.dc.pending);
    if p.len() >= DC_BUFFER {
        p.pop_front();
        conn.dc.dropped.fetch_add(1, Ordering::Relaxed);
    }
    p.push_back(m);
}

pub fn dc_send(conn: &Arc<Conn>, frame: Vec<u8>) {
    let tx = lock(&conn.dc.tx).clone();
    match tx {
        Some(tx) => {
            if tx.try_send(frame).is_err() {
                conn.dc.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        None => push_pending(conn, frame),
    }
}

pub fn on_srtp(sfu: &Arc<Sfu>, conn: &Arc<Conn>, pkt: &[u8]) {
    if pkt.len() < 2 {
        return;
    }
    let rtcp = (64..=95).contains(&(pkt[1] & 0x7f));
    let plain = {
        let mut g = lock(&conn.srtp_in);
        let Some(ctx) = g.as_mut() else {
            sfu.counters.drop("srtp_not_ready");
            return;
        };
        if rtcp { ctx.decrypt_rtcp(pkt) } else { ctx.decrypt_rtp(pkt) }
    };
    match plain {
        Ok(p) => {
            if rtcp {
                crate::rtcp_io::on_rtcp(sfu, conn, &p);
            } else if conn.publishes() {
                crate::media::on_rtp(sfu, conn, &p);
            } else {
                sfu.counters.drop("rtp_on_sub_conn");
            }
        }
        Err(_) => sfu.counters.drop("srtp_auth"),
    }
}
