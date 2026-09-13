use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::UdpSocket;

use super::ice::{IceEntry, Latch};

#[derive(Clone)]
pub struct Dispatch {
    socket: Arc<UdpSocket>,
}

impl Dispatch {
    pub fn new(socket: Arc<UdpSocket>) -> Self {
        Self { socket }
    }

    pub async fn send(&self, to: &IceEntry, bytes: &[u8]) -> bool {
        self.send_latched(&to.addr, bytes).await.is_ok()
    }

    pub async fn send_latched(&self, addr: &Latch, bytes: &[u8]) -> io::Result<usize> {
        let latched = *addr.read().expect("latch 자물쇠는 패닉을 건너지 않는다");
        let Some(dst) = latched else {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "latch 전이다"));
        };
        self.socket.send_to(bytes, dst).await
    }

    pub async fn reply_to(&self, dst: SocketAddr, bytes: &[u8]) -> bool {
        self.socket.send_to(bytes, dst).await.is_ok()
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}
