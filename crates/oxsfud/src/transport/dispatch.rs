use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::UdpSocket;

use super::framing::frame;
use super::ice::{IceEntry, Path};
use super::udp::now_ms;

#[derive(Clone)]
pub struct Dispatch {
    socket: Arc<UdpSocket>,
}

impl Dispatch {
    pub fn new(socket: Arc<UdpSocket>) -> Self {
        Self { socket }
    }

    pub async fn send(&self, to: &IceEntry, bytes: &[u8]) -> bool {
        self.send_picked(to, bytes).await.is_ok()
    }

    pub async fn send_picked(&self, to: &IceEntry, bytes: &[u8]) -> io::Result<usize> {
        match to.pick(now_ms()) {
            Path::Udp(dst) => self.socket.send_to(bytes, dst).await,
            Path::Tcp(handle) => {
                let Some(framed) = frame(bytes) else {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, "프레임 상한을 넘었다"));
                };
                if handle.send(framed) {
                    Ok(bytes.len())
                } else {
                    Err(io::Error::new(io::ErrorKind::WouldBlock, "tcp 가 밀렸다 — 버린다"))
                }
            }
            Path::Nowhere => Err(io::Error::new(io::ErrorKind::NotConnected, "경로가 없다")),
        }
    }

    pub async fn reply_to(&self, dst: SocketAddr, bytes: &[u8]) -> bool {
        self.socket.send_to(bytes, dst).await.is_ok()
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ice::{IceRole, IceTable, TcpHandle};
    use tokio::sync::mpsc;

    const PWD: &str = "zxcvbnmasdfghjklqwerty";

    async fn dispatch() -> Dispatch {
        let s = UdpSocket::bind("127.0.0.1:0").await.expect("바인드");
        Dispatch::new(Arc::new(s))
    }

    fn entry() -> Arc<IceEntry> {
        let t = IceTable::new();
        t.insert("srvufrag", PWD, "sess-1", IceRole::Publish)
    }

    fn attach(e: &IceEntry) -> mpsc::Receiver<bytes::Bytes> {
        let (tx, rx) = mpsc::channel(4);
        e.attach_tcp(TcpHandle(Arc::new(tx)));
        rx
    }

    #[tokio::test]
    async fn 경로가_없으면_안_보낸다() {
        let d = dispatch().await;
        assert!(d.send_picked(&entry(), b"x").await.is_err());
    }

    #[tokio::test]
    async fn tcp_밖에_없으면_tcp_로_가고_프레임이_씌워진다() {
        let d = dispatch().await;
        let e = entry();
        let mut rx = attach(&e);
        assert!(d.send_picked(&e, b"hello").await.is_ok());
        let got = rx.try_recv().expect("★tcp 로 갔다");
        assert_eq!(&got[..2], &[0x00, 0x05], "★길이 접두가 붙는다");
        assert_eq!(&got[2..], b"hello");
    }

    #[tokio::test]
    async fn tcp_가_밀리면_버리고_그렇다고_말한다() {
        let d = dispatch().await;
        let e = entry();
        let _rx = attach(&e);
        for _ in 0..4 {
            assert!(d.send_picked(&e, b"fill").await.is_ok());
        }
        assert!(d.send_picked(&e, b"overflow").await.is_err(), "★막히면 버린다");
    }

    #[tokio::test]
    async fn 프레임_상한을_넘으면_안_보낸다() {
        let d = dispatch().await;
        let e = entry();
        let mut rx = attach(&e);
        let big = vec![0u8; super::super::framing::MAX_FRAME + 1];
        assert!(d.send_picked(&e, &big).await.is_err());
        assert!(rx.try_recv().is_err());
    }
}
