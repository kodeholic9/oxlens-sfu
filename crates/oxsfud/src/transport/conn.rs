// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§12 · model: claude-opus-5

//! DTLS 가 쓰는 가짜 연결 — ★**소켓은 하나인데 세션은 여럿이기 때문에** 있다.
//!
//! ```text
//!   진짜 UDP 소켓(우리 것)
//!       ├── STUN  → ICE 장부(바로)
//!       ├── DTLS  → 이 갈래 → 그 세션의 DTLSConn
//!       └── SRTP  → 미디어
//! ```
//!
//! ★**보낼 주소는 latch 가 들고 있다** — 여기서 주소를 복사해 두면 망이 바뀐 뒤에도
//! 옛 주소로 계속 쏜다(정§12 *"주소 변경은 ICE 재시작 없이 latch 가 흡수한다"*).

use std::any::Any;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex};
use webrtc_util::conn::Conn;

use super::ice::Latch;

/// ★**밀린 DTLS 조각의 상한** — 핸드셰이크는 몇 장이면 끝난다. 넘치면 그 세션이 못 서고,
/// 그것은 조용한 성공보다 낫다.
const INBOUND: usize = 128;

/// 한 자격(연결 하나)의 DTLS 통로.
pub struct DemuxConn {
    socket: Arc<UdpSocket>,
    /// ★**읽을 때마다 본다** — 붙들어 두지 않는다.
    addr: Latch,
    rx: Mutex<mpsc::Receiver<Bytes>>,
}

impl DemuxConn {
    /// 통로와 그 입구를 낸다. 입구는 UDP 수신 루프가 쥔다.
    pub fn new(socket: Arc<UdpSocket>, addr: Latch) -> (Self, mpsc::Sender<Bytes>) {
        let (tx, rx) = mpsc::channel(INBOUND);
        (Self { socket, addr, rx: Mutex::new(rx) }, tx)
    }

    fn peer(&self) -> Option<SocketAddr> {
        *self.addr.read().expect("latch 자물쇠는 패닉을 건너지 않는다")
    }
}

#[async_trait]
impl Conn for DemuxConn {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        // ★우리는 이미 붙어 있다 — 붙이는 것은 STUN 이 했다.
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Some(data) => {
                // ★**자르지 않는다** — DTLS 는 레코드 경계가 계약이라 잘린 레코드는 버린다.
                if data.len() > buf.len() {
                    return Err(webrtc_util::Error::Other("dtls 레코드가 그릇보다 크다".into()));
                }
                buf[..data.len()].copy_from_slice(&data);
                Ok(data.len())
            }
            // ★세션이 회수됐다 — DTLS 태스크도 여기서 끝난다(정§12 *"회수 시 태스크 종료 필수"*).
            None => Err(webrtc_util::Error::Other("dtls 통로가 닫혔다".into())),
        }
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let n = self.recv(buf).await?;
        let addr = self.peer().ok_or_else(|| webrtc_util::Error::Other("latch 전이다".into()))?;
        Ok((n, addr))
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        // ★**그때그때 읽는다** — 망이 바뀌면 다음 장부터 새 주소로 간다.
        let addr = self.peer().ok_or_else(|| webrtc_util::Error::Other("latch 전이다".into()))?;
        self.socket.send_to(buf, addr).await.map_err(|e| webrtc_util::Error::Other(e.to_string()))
    }

    async fn send_to(&self, buf: &[u8], _target: SocketAddr) -> webrtc_util::Result<usize> {
        // ★목적지는 latch 하나다 — 부르는 쪽이 고른 주소를 받지 않는다.
        self.send(buf).await
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        self.socket.local_addr().map_err(|e| webrtc_util::Error::Other(e.to_string()))
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        self.peer()
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }
}
