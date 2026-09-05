// author: kodeholic (powered by Claude)
//! DTLS 가 쓰는 가상 연결 — 하나뿐인 UDP 소켓(정§12 demux)을 `Conn` 으로 감싼다.
//! 수신은 recv 루프가 넣어 주는 채널, 송신은 latch 된 주소로. 주소는 공유 셀이라 STUN re-latch 가 그대로 반영된다.

use std::any::Any;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, mpsc};
use webrtc_util::conn::Conn;

use super::session::AddrCell;

pub type PacketTx = mpsc::Sender<Bytes>;

pub struct DemuxConn {
    socket: Arc<UdpSocket>,
    addr: AddrCell,
    rx: Mutex<mpsc::Receiver<Bytes>>,
}

impl DemuxConn {
    pub fn new(socket: Arc<UdpSocket>, addr: AddrCell) -> (Self, PacketTx) {
        let (tx, rx) = mpsc::channel(128);
        (Self { socket, addr, rx: Mutex::new(rx) }, tx)
    }

    fn target(&self) -> webrtc_util::Result<SocketAddr> {
        self.addr.get().ok_or_else(|| webrtc_util::Error::Other("no latched address".to_owned()))
    }
}

#[async_trait]
impl Conn for DemuxConn {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let data = self.rx.lock().await.recv().await.ok_or_else(|| webrtc_util::Error::Other("transport closed".to_owned()))?;
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok(n)
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let n = self.recv(buf).await?;
        Ok((n, self.target()?))
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        self.socket.send_to(buf, self.target()?).await.map_err(|e| webrtc_util::Error::Other(e.to_string()))
    }

    async fn send_to(&self, buf: &[u8], _target: SocketAddr) -> webrtc_util::Result<usize> {
        self.send(buf).await
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        self.socket.local_addr().map_err(|e| webrtc_util::Error::Other(e.to_string()))
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        self.addr.get()
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }
}
