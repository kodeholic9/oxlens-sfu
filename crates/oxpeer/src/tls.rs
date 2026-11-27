// author: kodeholic (powered by Claude)
//! 전송 바깥층 — `http://` · `ws://` 는 TCP 그대로, `https://` · `wss://` 는 rustls(ring · tls12 — hub 와 같은 계열 · context 202611/20261108d E2).
//! 뿌리 인증서는 기본 webpki 뿌리 · 앱이 `Config::tls_root_pem` 을 주면 그것만 믿는다(같은 망 hub 의 사설 CA).

use std::pin::Pin;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

use crate::error::Error;

pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// 스트림 하나 — TCP 또는 TLS 위 TCP
pub(crate) type Stream = Pin<Box<dyn Io>>;

/// 믿을 뿌리 — None 이면 webpki 뿌리
pub(crate) fn client_config(root_pem: Option<&[u8]>) -> Result<Arc<ClientConfig>, Error> {
    let mut roots = RootCertStore::empty();
    match root_pem {
        Some(pem) => {
            for c in rustls_pemfile::certs(&mut std::io::BufReader::new(pem)) {
                let c = c.map_err(|e| Error::transport(format!("tls 뿌리 PEM: {e}")))?;
                roots.add(c).map_err(|e| Error::transport(format!("tls 뿌리: {e}")))?;
            }
            if roots.is_empty() {
                return Err(Error::transport("tls 뿌리 PEM 에 인증서가 없다"));
            }
        }
        None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
    }
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let cfg = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::transport(format!("tls: {e}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(cfg))
}

/// 인증서 하나 고정 — 받은 끝 인증서가 그 PEM 의 인증서와 바이트로 같을 때만 믿는다(이름 검사 없음 · 시스템 신뢰 저장소 안 씀).
/// 같은 기계의 hub 에 루프백으로 붙는 녹음 유닛 몫(정:IMPL-OPS-REC-UNIT 로컬 게이트웨이 · `--hub-ca`) — 서명 검증은 rustls 그대로 한다.
pub fn pinned_config(pem: &[u8]) -> Result<Arc<ClientConfig>, Error> {
    let cert = rustls_pemfile::certs(&mut std::io::BufReader::new(pem))
        .next()
        .ok_or_else(|| Error::transport("tls 고정 PEM 에 인증서가 없다"))?
        .map_err(|e| Error::transport(format!("tls 고정 PEM: {e}")))?;
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let cfg = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::transport(format!("tls: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned { cert, provider }))
        .with_no_client_auth();
    Ok(Arc::new(cfg))
}

#[derive(Debug)]
struct Pinned {
    cert: tokio_rustls::rustls::pki_types::CertificateDer<'static>,
    provider: Arc<tokio_rustls::rustls::crypto::CryptoProvider>,
}

impl tokio_rustls::rustls::client::danger::ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error> {
        if end_entity.as_ref() == self.cert.as_ref() {
            Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(tokio_rustls::rustls::Error::InvalidCertificate(tokio_rustls::rustls::CertificateError::UnknownIssuer))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        tokio_rustls::rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        tokio_rustls::rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// `host:port` 로 잇는다 — tls 가 있으면 그 위에 TLS(SNI = host)
pub(crate) async fn dial(auth: &str, tls: Option<&Arc<ClientConfig>>) -> Result<Stream, Error> {
    let tcp = TcpStream::connect(auth).await.map_err(|e| Error::transport(format!("{auth}: 연결 {e}")))?;
    let _ = tcp.set_nodelay(true);
    let Some(cfg) = tls else { return Ok(Box::pin(tcp)) };
    let host = auth.rsplit_once(':').map(|(h, _)| h).unwrap_or(auth).trim_start_matches('[').trim_end_matches(']');
    let name = ServerName::try_from(host.to_string()).map_err(|e| Error::transport(format!("tls 이름 {host}: {e}")))?;
    let s = TlsConnector::from(cfg.clone()).connect(name, tcp).await.map_err(|e| Error::transport(format!("{auth}: tls {e}")))?;
    Ok(Box::pin(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::TlsAcceptor;
    use tokio_rustls::rustls::ServerConfig;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio_tungstenite::tungstenite::Message;

    /// 자가 서명 `localhost` — (뿌리 PEM, 수락기)
    fn server() -> (Vec<u8>, TlsAcceptor) {
        let c = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let pem = c.cert.pem().into_bytes();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(c.key_pair.serialize_der()));
        let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
        let cfg = ServerConfig::builder_with_provider(provider).with_safe_default_protocol_versions().unwrap().with_no_client_auth().with_single_cert(vec![CertificateDer::from(c.cert.der().to_vec())], key).unwrap();
        (pem, TlsAcceptor::from(Arc::new(cfg)))
    }

    /// https 토큰 발급 하나 · wss 한 번 오감 — 사설 뿌리를 주면 붙고 · 안 주면(webpki) 거절된다
    #[tokio::test]
    async fn https_and_wss_with_private_root() {
        let (pem, acc) = server();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = l.accept().await else { return };
                let acc = acc.clone();
                tokio::spawn(async move {
                    let Ok(mut s) = acc.accept(tcp).await else { return };
                    let mut head = [0u8; 4];
                    if s.read_exact(&mut head).await.is_err() {
                        return;
                    }
                    if &head == b"POST" {
                        let mut buf = vec![0u8; 4096];
                        let _ = s.read(&mut buf).await;
                        let body = br#"{"token":"tok-1"}"#;
                        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await;
                        let _ = s.write_all(body).await;
                        let _ = s.shutdown().await;
                    } else {
                        let rest = PrefixIo { head: head.to_vec(), inner: s };
                        let Ok(mut ws) = tokio_tungstenite::accept_async(rest).await else { return };
                        if let Some(Ok(m)) = ws.next().await {
                            let _ = ws.send(m).await;
                        }
                    }
                });
            }
        });
        let base = format!("https://localhost:{port}/media");
        let req = crate::generated::ws_wire::TokenReq { api_key: "k".into(), api_secret: "s".into(), user_id: "u".into(), participant_type: None, hidden: None, metadata: None, permission: None };
        assert_eq!(crate::token::issue_token_with_root(&base, &req, Some(&pem)).await.unwrap(), "tok-1");
        assert!(crate::token::issue_token(&base, &req).await.is_err(), "webpki 뿌리는 자가 서명을 믿지 않는다");
        let b = crate::http::split_base(&base).unwrap();
        let cfg = client_config(Some(&pem)).unwrap();
        let s = dial(&b.auth, Some(&cfg)).await.unwrap();
        let (mut ws, _) = tokio_tungstenite::client_async(b.ws_url(), s).await.unwrap();
        ws.send(Message::Binary(vec![1, 2, 3])).await.unwrap();
        assert_eq!(ws.next().await.unwrap().unwrap(), Message::Binary(vec![1, 2, 3]));
    }

    /// 인증서 하나 고정 — 루프백 IP(이름이 인증서와 다름)로 붙어도 같은 인증서면 붙고 · 다른 인증서면 거절(정:IMPL-OPS-REC-UNIT `--hub-ca`)
    #[tokio::test]
    async fn pinned_cert_matches_bytes_not_names() {
        let (pem, acc) = server();
        let (other, _) = server();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = l.accept().await else { return };
                let acc = acc.clone();
                tokio::spawn(async move {
                    if let Ok(mut s) = acc.accept(tcp).await {
                        let _ = s.write_all(b"ok").await;
                        let _ = s.shutdown().await;
                    }
                });
            }
        });
        let auth = format!("127.0.0.1:{port}");
        assert!(dial(&auth, Some(&client_config(Some(&pem)).unwrap())).await.is_err(), "대조 — 뿌리 검증은 IP 이름에서 선다");
        let mut s = dial(&auth, Some(&pinned_config(&pem).unwrap())).await.expect("같은 인증서면 이름과 상관없이 붙는다");
        let mut got = Vec::new();
        s.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, b"ok");
        assert!(dial(&auth, Some(&pinned_config(&other).unwrap())).await.is_err(), "다른 인증서는 거절");
    }

    /// 앞에서 읽어 버린 네 바이트를 되돌려 주는 감싸개(시험 서버가 HTTP · WS 를 한 포트에서 가른다)
    struct PrefixIo<S> {
        head: Vec<u8>,
        inner: S,
    }

    impl<S: AsyncRead + Unpin> AsyncRead for PrefixIo<S> {
        fn poll_read(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &mut tokio::io::ReadBuf<'_>) -> std::task::Poll<std::io::Result<()>> {
            if !self.head.is_empty() {
                let n = self.head.len().min(buf.remaining());
                let h: Vec<u8> = self.head.drain(..n).collect();
                buf.put_slice(&h);
                return std::task::Poll::Ready(Ok(()));
            }
            Pin::new(&mut self.inner).poll_read(cx, buf)
        }
    }

    impl<S: AsyncWrite + Unpin> AsyncWrite for PrefixIo<S> {
        fn poll_write(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>, b: &[u8]) -> std::task::Poll<std::io::Result<usize>> {
            Pin::new(&mut self.inner).poll_write(cx, b)
        }
        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_flush(cx)
        }
        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_shutdown(cx)
        }
    }
}
