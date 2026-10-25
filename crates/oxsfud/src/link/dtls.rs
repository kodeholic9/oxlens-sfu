// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-LINK-TRANSPORT 정:CAND-LINK-DTLS-VALUES 정:RULE-GC-LEAVE-SEVEN 정:RULE-DOC-FIXED-PARTS · model: claude-opus-5-5
//! DTLS(서버 = 늘 passive · sha-256 · mtu 1,200 · 핸드셰이크 30s) → SRTP 키(EXTRACTOR-dtls_srtp · AES128_CM_HMAC_SHA1_80) → SCTP. 통로를 닫는 것은 종료가 아니다 — 읽기 고리를 직접 끝낸다.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use dtls::config::{ClientAuthType, Config, ExtendedMasterSecretType};
use dtls::conn::DTLSConn;
use dtls::crypto::{Certificate, CryptoPrivateKey};
use dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use webrtc_srtp::context::Context;
use webrtc_srtp::protection_profile::ProtectionProfile;

use super::transport::Transport;

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
pub const DTLS_MTU: usize = 1200;
pub const CERT_NAME: &str = "oxsfud";

pub struct Cert {
    pub dtls: Certificate,
    pub fingerprint: String,
}

pub fn make_cert() -> Result<Cert, String> {
    let mut params = rcgen::CertificateParams::new(vec![CERT_NAME.to_string()]).map_err(|e| e.to_string())?;
    let mut dn = rcgen::DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, CERT_NAME);
    params.distinguished_name = dn;
    let kp = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = params.self_signed(&kp).map_err(|e| e.to_string())?;
    let der = cert.der().to_owned();
    let digest = Sha256::digest(der.as_ref());
    let hex: Vec<String> = digest.iter().map(|b| format!("{b:02X}")).collect();
    let private_key = CryptoPrivateKey::try_from(&kp).map_err(|e| e.to_string())?;
    Ok(Cert { dtls: Certificate { certificate: vec![der], private_key }, fingerprint: format!("sha-256 {}", hex.join(":")) })
}

fn placeholder() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 0))
}

const FATAL_ALERT: [u8; 15] = [21, 0xfe, 0xfd, 0, 0, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 2, 2, 10];

struct Pipe {
    rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    t: Weak<Transport>,
    stop: CancellationToken,
    alerted: AtomicBool,
}

#[async_trait]
impl webrtc_util::Conn for Pipe {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let mut rx = self.rx.lock().await;
        let got = tokio::select! {
            m = rx.recv() => m,
            _ = self.stop.cancelled() => None,
        };
        match got {
            Some(m) => {
                let n = m.len().min(buf.len());
                buf[..n].copy_from_slice(&m[..n]);
                Ok(n)
            }
            None => {
                if !self.alerted.swap(true, Ordering::AcqRel) {
                    let n = FATAL_ALERT.len().min(buf.len());
                    buf[..n].copy_from_slice(&FATAL_ALERT[..n]);
                    return Ok(n);
                }
                std::future::pending::<()>().await;
                Ok(0)
            }
        }
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let n = self.recv(buf).await?;
        Ok((n, placeholder()))
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
        Ok(placeholder())
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

fn config(cert: &Cert) -> Config {
    Config {
        certificates: vec![cert.dtls.clone()],
        srtp_protection_profiles: vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80],
        client_auth: ClientAuthType::RequireAnyClientCert,
        extended_master_secret: ExtendedMasterSecretType::Request,
        insecure_skip_verify: true,
        mtu: DTLS_MTU,
        ..Default::default()
    }
}

pub enum Ended {
    HandshakeTimeout,
    HandshakeFailed(String),
    KeyExport(String),
    Closed,
}

impl Ended {
    pub fn name(&self) -> String {
        match self {
            Ended::HandshakeTimeout => "handshake_timeout".into(),
            Ended::HandshakeFailed(e) => format!("handshake_failed:{e}"),
            Ended::KeyExport(e) => format!("key_export:{e}"),
            Ended::Closed => "closed".into(),
        }
    }
}

pub async fn run(t: Arc<Transport>, cert: Arc<Cert>, first: Vec<u8>) -> Ended {
    let (tx, rx) = mpsc::channel::<Vec<u8>>(256);
    let _ = tx.try_send(first);
    if let Ok(mut g) = t.dtls_in.lock() {
        let _ = g.replace(tx);
    }
    let stop = t.cancel.child_token();
    let pipe = Arc::new(Pipe { rx: Mutex::new(rx), t: Arc::downgrade(&t), stop: stop.clone(), alerted: AtomicBool::new(false) });
    let made = tokio::select! {
        r = tokio::time::timeout(HANDSHAKE_TIMEOUT, DTLSConn::new(pipe.clone(), config(&cert), false, None)) => r,
        _ = t.cancel.cancelled() => {
            stop.cancel();
            return Ended::Closed;
        }
    };
    let conn = match made {
        Err(_) => {
            stop.cancel();
            reset(&t);
            return Ended::HandshakeTimeout;
        }
        Ok(Err(e)) => {
            stop.cancel();
            reset(&t);
            return Ended::HandshakeFailed(e.to_string());
        }
        Ok(Ok(c)) => Arc::new(c),
    };
    if let Err(e) = install_srtp(&t, &conn).await {
        let _ = conn.close().await;
        stop.cancel();
        reset(&t);
        return Ended::KeyExport(e);
    }
    let dc_out = t.dc_out_rx.lock().ok().and_then(|mut g| g.take());
    if let Some(rx) = dc_out {
        super::sctp::run(t.clone(), conn.clone(), rx).await;
    } else {
        t.cancel.cancelled().await;
    }
    let _ = conn.close().await;
    stop.cancel();
    Ended::Closed
}

fn reset(t: &Transport) {
    if let Ok(mut g) = t.dtls_in.lock() {
        g.take();
    }
    t.dtls_started.store(false, Ordering::Release);
}

async fn install_srtp(t: &Transport, conn: &DTLSConn) -> Result<(), String> {
    let state = conn.connection_state().await;
    let mut c = webrtc_srtp::config::Config { profile: ProtectionProfile::Aes128CmHmacSha1_80, ..Default::default() };
    c.extract_session_keys_from_dtls(state, false).await.map_err(|e| e.to_string())?;
    let p = ProtectionProfile::Aes128CmHmacSha1_80;
    let rx = Context::new(&c.keys.remote_master_key, &c.keys.remote_master_salt, p, None, None).map_err(|e| e.to_string())?;
    let tx = Context::new(&c.keys.local_master_key, &c.keys.local_master_salt, p, None, None).map_err(|e| e.to_string())?;
    if let Ok(mut g) = t.srtp_rx.lock() {
        let _ = g.replace(rx);
    }
    if let Ok(mut g) = t.srtp_tx.lock() {
        let _ = g.replace(tx);
    }
    t.srtp_ready.store(true, Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cert_name_and_fingerprint() {
        let c = make_cert().expect("cert");
        assert!(c.fingerprint.starts_with("sha-256 "));
        assert_eq!(c.fingerprint.len(), "sha-256 ".len() + 32 * 3 - 1);
    }
}
