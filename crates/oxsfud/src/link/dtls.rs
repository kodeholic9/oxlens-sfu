// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-LINK-TRANSPORT · 연:TRN.frame.DC · 연:RULE-FLR-TRANSPORT-DC · 정:RULE-GC-LEAVE-SEVEN · 정:CAND-LINK-DTLS-VALUES · 정:RULE-DOC-FIXED-PARTS · model: claude-opus-5-5
//! DTLS(항상 passive · sha-256) → SRTP 키(AES128-CM-HMAC-SHA1-80) → SCTP/DCEP(채널 `"unreliable"` 하나 · 레코드 하나 = SCTP 패킷 하나 · 재전송 한도 없음). 회수 신호에 태스크를 끝낸다.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use bytes::Bytes;
use sctp_proto::{
    Association, AssociationHandle, DatagramEvent, Endpoint, EndpointConfig, Event, Payload, PayloadProtocolIdentifier,
    ReliabilityType, ServerConfig, StreamEvent, StreamId,
};
use tokio::sync::{Mutex as AMutex, mpsc};

use super::Transport;

pub const DC_LABEL: &str = "unreliable";
const DCEP_OPEN: u8 = 0x03;
const DCEP_ACK: u8 = 0x02;

pub struct Io {
    pub tr: Arc<Transport>,
    rx: AMutex<mpsc::UnboundedReceiver<Vec<u8>>>,
    local: SocketAddr,
}

#[async_trait::async_trait]
impl webrtc_util::Conn for Io {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        Ok(())
    }
    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Some(d) => {
                let n = d.len().min(buf.len());
                buf[..n].copy_from_slice(&d[..n]);
                Ok(n)
            }
            None => Err(webrtc_util::Error::Other("closed".into())),
        }
    }
    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let n = self.recv(buf).await?;
        Ok((n, self.local))
    }
    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        crate::fwd::rt::control_send(&self.tr, buf);
        Ok(buf.len())
    }
    async fn send_to(&self, buf: &[u8], _target: SocketAddr) -> webrtc_util::Result<usize> {
        self.send(buf).await
    }
    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.local)
    }
    fn remote_addr(&self) -> Option<SocketAddr> {
        None
    }
    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

pub fn certificate() -> Result<(dtls::crypto::Certificate, String), String> {
    let c = dtls::crypto::Certificate::generate_self_signed(vec!["oxsfud".to_string()]).map_err(|e| e.to_string())?;
    let der = c.certificate.first().map(|d| d.as_ref().to_vec()).unwrap_or_default();
    Ok((c, super::fingerprint_hex(&der)))
}

fn config(cert: dtls::crypto::Certificate) -> dtls::config::Config {
    dtls::config::Config {
        certificates: vec![cert],
        srtp_protection_profiles: vec![dtls::extension::extension_use_srtp::SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80],
        extended_master_secret: dtls::config::ExtendedMasterSecretType::Require,
        client_auth: dtls::config::ClientAuthType::RequireAnyClientCert,
        insecure_skip_verify: true,
        mtu: super::DTLS_MTU,
        ..Default::default()
    }
}

pub async fn run(tr: Arc<Transport>, cert: dtls::crypto::Certificate, local: SocketAddr) {
    let Some(rx) = tr.dtls_rx.lock().ok().and_then(|mut g| g.take()) else { return };
    let io = Arc::new(Io { tr: tr.clone(), rx: AMutex::new(rx), local });
    let mut stop = tr.stop.subscribe();
    let hs = tokio::time::timeout(Duration::from_secs(super::DTLS_HANDSHAKE_SECS), dtls::conn::DTLSConn::new(io, config(cert), false, None));
    let conn = tokio::select! {
        r = hs => match r {
            Ok(Ok(c)) => Arc::new(c),
            Ok(Err(e)) => {
                tracing::info!(kind = "diag", user = %tr.user, "dtls handshake failed: {e}");
                return;
            }
            Err(_) => {
                tracing::info!(kind = "diag", user = %tr.user, "dtls handshake timeout");
                return;
            }
        },
        _ = stop.changed() => return,
    };
    if let Err(e) = srtp_keys(&tr, &conn).await {
        tracing::warn!(kind = "diag", user = %tr.user, "srtp keys: {e}");
        return;
    }
    tr.ready.store(true, Ordering::Release);
    crate::ops::transport_ready(&tr);
    sctp_loop(tr, conn, stop).await;
}

async fn srtp_keys(tr: &Transport, conn: &dtls::conn::DTLSConn) -> Result<(), String> {
    let st = conn.connection_state().await;
    let profile = webrtc_srtp::protection_profile::ProtectionProfile::Aes128CmHmacSha1_80;
    let mut cfg = webrtc_srtp::config::Config { profile, ..Default::default() };
    cfg.extract_session_keys_from_dtls(st, false).await.map_err(|e| e.to_string())?;
    let k = &cfg.keys;
    let tx = webrtc_srtp::context::Context::new(&k.local_master_key, &k.local_master_salt, profile, None, None).map_err(|e| e.to_string())?;
    let rx = webrtc_srtp::context::Context::new(&k.remote_master_key, &k.remote_master_salt, profile, None, None).map_err(|e| e.to_string())?;
    if let Ok(mut g) = tr.srtp_tx.lock() {
        g.replace(tx);
    }
    if let Ok(mut g) = tr.srtp_rx.lock() {
        g.replace(rx);
    }
    Ok(())
}

struct Sctp {
    ep: Endpoint,
    assoc: Option<(AssociationHandle, Association)>,
    dc: Option<StreamId>,
    remote: SocketAddr,
}

impl Sctp {
    fn input(&mut self, data: Vec<u8>) {
        let now = Instant::now();
        if let Some((h, ev)) = self.ep.handle(now, self.remote, None, None, Bytes::from(data)) {
            match ev {
                DatagramEvent::NewAssociation(a) => self.assoc = Some((h, a)),
                DatagramEvent::AssociationEvent(e) => {
                    if let Some((_, a)) = self.assoc.as_mut() {
                        a.handle_event(e);
                    }
                }
            }
        }
    }

    fn transmits(&mut self) -> Vec<Bytes> {
        let mut out = Vec::new();
        let now = Instant::now();
        if let Some((h, a)) = self.assoc.as_mut() {
            while let Some(t) = a.poll_transmit(now) {
                if let Payload::RawEncode(v) = t.payload {
                    out.extend(v);
                }
            }
            while let Some(e) = a.poll_endpoint_event() {
                let _ = self.ep.handle_event(*h, e);
            }
        }
        out
    }

    fn events(&mut self, tr: &Arc<Transport>) {
        let Some((_, a)) = self.assoc.as_mut() else { return };
        while let Some(ev) = a.poll() {
            if let Event::Stream(StreamEvent::Opened { .. }) | Event::Stream(StreamEvent::Readable { .. }) = ev {}
        }
        while let Some(mut s) = a.accept_stream() {
            let _ = s.set_reliability_params(true, ReliabilityType::Rexmit, 0);
        }
        let ids = a.stream_ids();
        for id in ids {
            let Ok(mut s) = a.stream(id) else { continue };
            while let Ok(Some(chunks)) = s.read_sctp() {
                let mut buf = vec![0u8; chunks.len()];
                let n = chunks.read(&mut buf).unwrap_or(0);
                buf.truncate(n);
                match chunks.ppi {
                    PayloadProtocolIdentifier::Dcep => {
                        if buf.first() == Some(&DCEP_OPEN) {
                            let label = dcep_label(&buf);
                            if label.as_deref() == Some(DC_LABEL) {
                                let (unordered, rel, val) = dcep_reliability(&buf);
                                let _ = s.set_reliability_params(unordered, rel, val);
                                let _ = s.write_with_ppi(&[DCEP_ACK], PayloadProtocolIdentifier::Dcep);
                                self.dc = Some(id);
                                tr.dc_open.store(true, Ordering::Release);
                                crate::ops::dc_opened(tr);
                            } else {
                                tracing::info!(kind = "diag", user = %tr.user, "dc label refused {label:?}");
                            }
                        }
                    }
                    PayloadProtocolIdentifier::Binary | PayloadProtocolIdentifier::String => {
                        if self.dc == Some(id) {
                            crate::ops::dc_in(tr, &buf);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn send_dc(&mut self, data: &[u8]) -> bool {
        let (Some((_, a)), Some(id)) = (self.assoc.as_mut(), self.dc) else { return false };
        match a.stream(id) {
            Ok(mut s) => s.write_with_ppi(data, PayloadProtocolIdentifier::Binary).is_ok(),
            Err(_) => false,
        }
    }

    fn timeout(&mut self) -> Option<Instant> {
        self.assoc.as_ref().and_then(|(_, a)| a.poll_timeout())
    }

    fn on_timeout(&mut self) {
        if let Some((_, a)) = self.assoc.as_mut() {
            a.handle_timeout(Instant::now());
        }
    }
}

fn dcep_label(b: &[u8]) -> Option<String> {
    if b.len() < 12 {
        return None;
    }
    let ll = u16::from_be_bytes([b[8], b[9]]) as usize;
    b.get(12..12 + ll).map(|l| String::from_utf8_lossy(l).into_owned())
}

fn dcep_reliability(b: &[u8]) -> (bool, ReliabilityType, u32) {
    let ct = b.get(1).copied().unwrap_or(0);
    let val = if b.len() >= 8 { u32::from_be_bytes([b[4], b[5], b[6], b[7]]) } else { 0 };
    let unordered = ct & 0x80 != 0;
    let rel = match ct & 0x7f {
        0x01 => ReliabilityType::Rexmit,
        0x02 => ReliabilityType::Timed,
        _ => ReliabilityType::Reliable,
    };
    (unordered, rel, val)
}

async fn sctp_loop(tr: Arc<Transport>, conn: Arc<dtls::conn::DTLSConn>, mut stop: tokio::sync::watch::Receiver<bool>) {
    let remote: SocketAddr = ([127, 0, 0, 1], 5000).into();
    let mut sc = Sctp {
        ep: Endpoint::new(Arc::new(EndpointConfig::new()), Some(Arc::new(ServerConfig::new()))),
        assoc: None,
        dc: None,
        remote,
    };
    let Some(mut dc_rx) = tr.dc_rx.lock().ok().and_then(|mut g| g.take()) else { return };
    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let reader = conn.clone();
    let rtask = tokio::spawn(async move {
        let mut buf = vec![0u8; 65_536];
        while let Ok(n) = reader.read(&mut buf, None).await {
            if in_tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    loop {
        let wake = sc.timeout().map(|t| t.saturating_duration_since(Instant::now())).unwrap_or(Duration::from_secs(1));
        tokio::select! {
            d = in_rx.recv() => match d {
                Some(d) => sc.input(d),
                None => break,
            },
            m = dc_rx.recv() => match m {
                Some(m) => { sc.send_dc(&m); }
                None => break,
            },
            _ = tokio::time::sleep(wake) => sc.on_timeout(),
            _ = stop.changed() => break,
        }
        sc.events(&tr);
        for p in sc.transmits() {
            let _ = conn.write(&p, None).await;
        }
    }
    rtask.abort();
    let _ = conn.close().await;
    tr.dc_open.store(false, Ordering::Release);
}
