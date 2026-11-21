// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:IMPL-CFG-SYSTEM-FILE · model: claude-opus-5-5
//! zenoh 입구의 링크 — TLS 받기(클라이언트 인증서 필수 · `[hook] zenoh_ca` 로 검증 · 서버 인증서 `zenoh_cert` · `zenoh_key`) · 받은 인증서 첫 장의 첫 CN = 계정 키 · 스트림 링크라 배치마다 앞에 길이 2 바이트(LE).

use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::server::WebPkiClientVerifier;
use tokio_rustls::rustls::{RootCertStore, ServerConfig, ServerConnection};

fn pem_certs(path: &str) -> Result<Vec<tokio_rustls::rustls::pki_types::CertificateDer<'static>>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let v: Vec<_> = rustls_pemfile::certs(&mut std::io::BufReader::new(f)).filter_map(|c| c.ok()).collect();
    if v.is_empty() {
        return Err(format!("{path}: no certificate"));
    }
    Ok(v)
}

pub fn acceptor(cert: &str, key: &str, ca: &str) -> Result<TlsAcceptor, String> {
    let certs = pem_certs(cert)?;
    let f = std::fs::File::open(key).map_err(|e| format!("{key}: {e}"))?;
    let key = rustls_pemfile::private_key(&mut std::io::BufReader::new(f)).map_err(|e| format!("{key}: {e}"))?.ok_or_else(|| format!("{key}: no key"))?;
    let mut roots = RootCertStore::empty();
    for c in pem_certs(ca)? {
        roots.add(c).map_err(|e| format!("{ca}: {e}"))?;
    }
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots)).build().map_err(|e| format!("client verifier: {e}"))?;
    let cfg = ServerConfig::builder().with_client_cert_verifier(verifier).with_single_cert(certs, key).map_err(|e| format!("zenoh tls: {e}"))?;
    Ok(TlsAcceptor::from(Arc::new(cfg)))
}

pub fn first_cn(conn: &ServerConnection) -> Option<String> {
    let der = conn.peer_certificates()?.first()?;
    let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).ok()?;
    cert.subject().iter_common_name().next().and_then(|cn| cn.as_str().ok()).map(str::to_string)
}

pub async fn read_batch<R: AsyncRead + Unpin>(r: &mut R) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 2];
    r.read_exact(&mut len).await?;
    let mut buf = vec![0u8; u16::from_le_bytes(len) as usize];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

pub async fn write_batches<W: AsyncWrite + Unpin>(w: &mut W, batches: &[Vec<u8>]) -> std::io::Result<()> {
    for b in batches {
        let len = u16::try_from(b.len()).map_err(|_| std::io::Error::other("batch over 65535"))?;
        w.write_all(&len.to_le_bytes()).await?;
        w.write_all(b).await?;
    }
    w.flush().await
}
