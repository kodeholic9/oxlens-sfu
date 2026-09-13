use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::demux::{classify, Packet};
use super::framing::{frame, Unframer};
use super::ice::{on_binding, Arrival, Binding, IceTable};
use super::udp::now_ms;

pub const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(10);

const READ_CHUNK: usize = 4096;

pub async fn serve(listener: TcpListener, table: Arc<IceTable>, identify_timeout: Duration) {
    loop {
        let Ok((stream, from)) = listener.accept().await else {
            continue;
        };
        let table = table.clone();
        tokio::spawn(async move {
            peer(stream, from, table, identify_timeout).await;
        });
    }
}

async fn peer(
    mut stream: TcpStream,
    from: SocketAddr,
    table: Arc<IceTable>,
    identify_timeout: Duration,
) {
    let _ = stream.set_nodelay(true);
    let mut un = Unframer::new();
    let mut buf = vec![0u8; READ_CHUNK];
    let mut known: Option<String> = None;

    loop {
        let read = match known {
            Some(_) => stream.read(&mut buf).await,
            None => match tokio::time::timeout(identify_timeout, stream.read(&mut buf)).await {
                Ok(r) => r,
                Err(_) => return,
            },
        };
        let Ok(n) = read else { return };
        if n == 0 {
            return;
        }
        un.push(&buf[..n]);
        while let Some(f) = un.next_frame() {
            if classify(&f) != Packet::Stun {
                continue;
            }
            let Binding::Respond { wire, ufrag, .. } =
                on_binding(&table, &f, from, now_ms(), Arrival::Tcp)
            else {
                continue;
            };
            known = Some(ufrag);
            let Some(out) = frame(&wire) else { continue };
            if stream.write_all(&out).await.is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ice::IceRole;

    const PWD: &str = "zxcvbnmasdfghjklqwerty";

    fn table() -> Arc<IceTable> {
        let t = IceTable::new();
        t.insert("srvufrag", PWD, "sess-1", IceRole::Publish);
        Arc::new(t)
    }

    fn request(ufrag: &str, pwd: &str) -> Vec<u8> {
        crate::transport::stun::binding_request(&[7u8; 12], &format!("{ufrag}:bot"), pwd, true)
    }

    async fn listening(t: Arc<IceTable>, timeout: Duration) -> SocketAddr {
        let l = TcpListener::bind("127.0.0.1:0").await.expect("바인드");
        let at = l.local_addr().expect("주소");
        tokio::spawn(async move { serve(l, t, timeout).await });
        at
    }

    async fn read_one_frame(s: &mut TcpStream) -> Option<Vec<u8>> {
        let mut un = Unframer::new();
        let mut buf = [0u8; 1024];
        for _ in 0..8 {
            let n = tokio::time::timeout(Duration::from_millis(500), s.read(&mut buf))
                .await
                .ok()?
                .ok()?;
            if n == 0 {
                return None;
            }
            un.push(&buf[..n]);
            if let Some(f) = un.next_frame() {
                return Some(f.to_vec());
            }
        }
        None
    }

    #[tokio::test]
    async fn 프레임_씌운_stun_에_프레임_씌워_답한다() {
        let at = listening(table(), IDENTIFY_TIMEOUT).await;
        let mut s = TcpStream::connect(at).await.expect("붙는다");
        s.write_all(&frame(&request("srvufrag", PWD)).expect("씌운다")).await.expect("보낸다");
        let got = read_one_frame(&mut s).await.expect("답이 온다");
        assert_eq!(classify(&got), Packet::Stun, "★답도 STUN 이다");
    }

    #[tokio::test]
    async fn 한_연결에_두_요청을_이어_보내도_각각_답한다() {
        let at = listening(table(), IDENTIFY_TIMEOUT).await;
        let mut s = TcpStream::connect(at).await.expect("붙는다");
        let one = frame(&request("srvufrag", PWD)).expect("씌운다");
        let mut both = one.to_vec();
        both.extend_from_slice(&one);
        s.write_all(&both).await.expect("보낸다");
        assert!(read_one_frame(&mut s).await.is_some(), "첫 답");
        assert!(read_one_frame(&mut s).await.is_some(), "둘째 답");
    }

    #[tokio::test]
    async fn 모르는_ufrag_에는_답하지_않는다() {
        let at = listening(table(), Duration::from_millis(200)).await;
        let mut s = TcpStream::connect(at).await.expect("붙는다");
        s.write_all(&frame(&request("nope", PWD)).expect("씌운다")).await.expect("보낸다");
        assert_eq!(read_one_frame(&mut s).await, None, "★조용히 버린다");
    }

    #[tokio::test]
    async fn 위조는_답을_못_받는다() {
        let at = listening(table(), Duration::from_millis(200)).await;
        let mut s = TcpStream::connect(at).await.expect("붙는다");
        let forged = request("srvufrag", "wrong-password-000000");
        s.write_all(&frame(&forged).expect("씌운다")).await.expect("보낸다");
        assert_eq!(read_one_frame(&mut s).await, None);
    }

    #[tokio::test]
    async fn 신원을_안_밝히면_끊는다() {
        let at = listening(table(), Duration::from_millis(150)).await;
        let mut s = TcpStream::connect(at).await.expect("붙는다");
        let mut buf = [0u8; 16];
        let n = tokio::time::timeout(Duration::from_secs(2), s.read(&mut buf))
            .await
            .expect("★상한 안에 끊긴다")
            .expect("읽기");
        assert_eq!(n, 0, "★서버가 닫았다");
    }

    #[tokio::test]
    async fn 신원을_밝히면_끊지_않는다() {
        let at = listening(table(), Duration::from_millis(150)).await;
        let mut s = TcpStream::connect(at).await.expect("붙는다");
        s.write_all(&frame(&request("srvufrag", PWD)).expect("씌운다")).await.expect("보낸다");
        assert!(read_one_frame(&mut s).await.is_some(), "답을 받았다");
        let mut buf = [0u8; 16];
        let still = tokio::time::timeout(Duration::from_millis(600), s.read(&mut buf)).await;
        assert!(still.is_err(), "★상한이 지나도 살아 있다");
    }

    #[tokio::test]
    async fn 길이_0_프레임은_흘려보낸다() {
        let at = listening(table(), Duration::from_millis(400)).await;
        let mut s = TcpStream::connect(at).await.expect("붙는다");
        s.write_all(&frame(b"").expect("빈 것도 프레임")).await.expect("보낸다");
        s.write_all(&frame(&request("srvufrag", PWD)).expect("씌운다")).await.expect("보낸다");
        assert!(read_one_frame(&mut s).await.is_some(), "★빈 프레임이 뒤를 막지 않는다");
    }
}
