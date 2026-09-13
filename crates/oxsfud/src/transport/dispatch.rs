use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::UdpSocket;

use super::framing::frame;
use super::ice::{IceEntry, Route};

#[derive(Clone)]
pub struct Dispatch {
    socket: Arc<UdpSocket>,
}

impl Dispatch {
    pub fn new(socket: Arc<UdpSocket>) -> Self {
        Self { socket }
    }

    pub async fn send(&self, to: &IceEntry, bytes: &[u8]) -> bool {
        self.send_routed(&to.route, bytes).await.is_ok()
    }

    pub async fn send_routed(&self, route: &Route, bytes: &[u8]) -> io::Result<usize> {
        let (udp, tcp) = {
            let r = route.read().expect("route 자물쇠는 패닉을 건너지 않는다");
            (r.udp, r.tcp.clone())
        };
        if let Some(dst) = udp {
            return self.socket.send_to(bytes, dst).await;
        }
        let Some(handle) = tcp else {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "경로가 없다"));
        };
        let Some(framed) = frame(bytes) else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "프레임 상한을 넘었다"));
        };
        if handle.send(framed) {
            Ok(bytes.len())
        } else {
            Err(io::Error::new(io::ErrorKind::WouldBlock, "tcp 가 밀렸다 — 버린다"))
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
    use crate::transport::ice::{RouteState, TcpHandle};
    use std::sync::RwLock;
    use tokio::sync::mpsc;

    async fn dispatch() -> Dispatch {
        let s = UdpSocket::bind("127.0.0.1:0").await.expect("바인드");
        Dispatch::new(Arc::new(s))
    }

    fn tcp_route() -> (Route, mpsc::Receiver<bytes::Bytes>, TcpHandle) {
        let (tx, rx) = mpsc::channel(4);
        let h = TcpHandle(Arc::new(tx));
        let r: Route = Arc::new(RwLock::new(RouteState { udp: None, tcp: Some(h.clone()) }));
        (r, rx, h)
    }

    #[tokio::test]
    async fn 경로가_없으면_안_보낸다() {
        let d = dispatch().await;
        let r: Route = Route::default();
        assert!(d.send_routed(&r, b"x").await.is_err());
    }

    #[tokio::test]
    async fn tcp_밖에_없으면_tcp_로_가고_프레임이_씌워진다() {
        let d = dispatch().await;
        let (r, mut rx, _h) = tcp_route();
        assert!(d.send_routed(&r, b"hello").await.is_ok());
        let got = rx.try_recv().expect("★tcp 로 갔다");
        assert_eq!(&got[..2], &[0x00, 0x05], "★길이 접두가 붙는다");
        assert_eq!(&got[2..], b"hello");
    }

    #[tokio::test]
    async fn udp_가_있으면_udp_가_이긴다() {
        let d = dispatch().await;
        let peer = UdpSocket::bind("127.0.0.1:0").await.expect("바인드");
        let at = peer.local_addr().expect("주소");
        let (tx, mut rx) = mpsc::channel(4);
        let r: Route = Arc::new(RwLock::new(RouteState {
            udp: Some(at),
            tcp: Some(TcpHandle(Arc::new(tx))),
        }));
        assert!(d.send_routed(&r, b"udp-wins").await.is_ok());
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(std::time::Duration::from_millis(500), peer.recv(&mut buf))
            .await
            .expect("온다")
            .expect("받는다");
        assert_eq!(&buf[..n], b"udp-wins", "★udp 로 갔다");
        assert!(rx.try_recv().is_err(), "★tcp 로는 안 갔다");
    }

    #[tokio::test]
    async fn tcp_가_밀리면_버리고_그렇다고_말한다() {
        let d = dispatch().await;
        let (r, _rx, _h) = tcp_route();
        for _ in 0..4 {
            assert!(d.send_routed(&r, b"fill").await.is_ok());
        }
        assert!(d.send_routed(&r, b"overflow").await.is_err(), "★막히면 버린다");
    }

    #[tokio::test]
    async fn 프레임_상한을_넘으면_안_보낸다() {
        let d = dispatch().await;
        let (r, mut rx, _h) = tcp_route();
        let big = vec![0u8; super::super::framing::MAX_FRAME + 1];
        assert!(d.send_routed(&r, &big).await.is_err());
        assert!(rx.try_recv().is_err());
    }
}
