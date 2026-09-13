use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::demux::{classify, Packet};
use super::framing::{frame, Unframer};
use super::ice::{on_binding, Arrival, Binding, IceTable, TcpHandle};
use super::udp::now_ms;

#[derive(Debug)]
pub enum TcpIn {
    Identified { ufrag: String, from: SocketAddr },
    Frame { from: SocketAddr, bytes: Bytes },
    Closed { from: SocketAddr },
}

pub const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(10);

const READ_CHUNK: usize = 4096;
const OUTBOUND: usize = 128;

pub async fn serve(
    listener: TcpListener,
    table: Arc<IceTable>,
    identify_timeout: Duration,
    up: mpsc::Sender<TcpIn>,
) {
    loop {
        let Ok((stream, from)) = listener.accept().await else {
            continue;
        };
        let table = table.clone();
        let up = up.clone();
        tokio::spawn(async move {
            peer(stream, from, table, identify_timeout, up).await;
        });
    }
}

async fn peer(
    stream: TcpStream,
    from: SocketAddr,
    table: Arc<IceTable>,
    identify_timeout: Duration,
    up: mpsc::Sender<TcpIn>,
) {
    let _ = stream.set_nodelay(true);
    let (mut rd, mut wr) = stream.into_split();
    let (tx, mut rx) = mpsc::channel::<Bytes>(OUTBOUND);
    let handle = TcpHandle(Arc::new(tx));
    let writer = tokio::spawn(async move {
        while let Some(b) = rx.recv().await {
            if wr.write_all(&b).await.is_err() {
                break;
            }
        }
    });

    let mut un = Unframer::new();
    let mut buf = vec![0u8; READ_CHUNK];
    let mut known: Option<String> = None;

    loop {
        let read = match known {
            Some(_) => rd.read(&mut buf).await,
            None => match tokio::time::timeout(identify_timeout, rd.read(&mut buf)).await {
                Ok(r) => r,
                Err(_) => break,
            },
        };
        let Ok(n) = read else { break };
        if n == 0 {
            break;
        }
        un.push(&buf[..n]);
        let mut broken = false;
        while let Some(f) = un.next_frame() {
            if classify(&f) != Packet::Stun {
                if known.is_some() && up.send(TcpIn::Frame { from, bytes: f }).await.is_err() {
                    broken = true;
                    break;
                }
                continue;
            }
            let Binding::Respond { wire, ufrag, .. } =
                on_binding(&table, &f, from, now_ms(), Arrival::Tcp)
            else {
                continue;
            };
            if known.is_none()
                && let Some(e) = table.get(&ufrag)
            {
                e.attach_tcp(handle.clone());
                if up.send(TcpIn::Identified { ufrag: ufrag.clone(), from }).await.is_err() {
                    broken = true;
                    break;
                }
            }
            known = Some(ufrag);
            let Some(out) = frame(&wire) else { continue };
            if !handle.send(out) {
                broken = true;
                break;
            }
        }
        if broken {
            break;
        }
    }

    if let Some(ufrag) = known {
        if let Some(e) = table.get(&ufrag) {
            e.detach_tcp_if(&handle);
        }
        let _ = up.send(TcpIn::Closed { from }).await;
    }
    drop(handle);
    let _ = writer.await;
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
        let (at, mut rx) = listening_up(t, timeout).await;
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        at
    }

    async fn listening_up(
        t: Arc<IceTable>,
        timeout: Duration,
    ) -> (SocketAddr, mpsc::Receiver<TcpIn>) {
        let l = TcpListener::bind("127.0.0.1:0").await.expect("바인드");
        let at = l.local_addr().expect("주소");
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move { serve(l, t, timeout, tx).await });
        (at, rx)
    }

    async fn read_one_frame(s: &mut TcpStream) -> Option<Vec<u8>> {
        let mut un = Unframer::new();
        read_frame_with(s, &mut un).await
    }

    async fn read_frame_with(s: &mut TcpStream, un: &mut Unframer) -> Option<Vec<u8>> {
        if let Some(f) = un.next_frame() {
            return Some(f.to_vec());
        }
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
        let mut un = Unframer::new();
        assert!(read_frame_with(&mut s, &mut un).await.is_some(), "첫 답");
        assert!(read_frame_with(&mut s, &mut un).await.is_some(), "★둘째 답 — 한 세그먼트로 합쳐져도");
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
    async fn 신원을_밝히면_그_연결이_하향_경로가_된다() {
        let t = table();
        let at = listening(t.clone(), IDENTIFY_TIMEOUT).await;
        let mut s = TcpStream::connect(at).await.expect("붙는다");
        let e = t.get("srvufrag").expect("있다");
        assert!(e.tcp().is_none(), "★붙기 전에는 없다");

        s.write_all(&frame(&request("srvufrag", PWD)).expect("씌운다")).await.expect("보낸다");
        let mut un = Unframer::new();
        read_frame_with(&mut s, &mut un).await.expect("답이 온다");

        let handle = e.tcp().expect("★핸들이 걸렸다");
        assert_eq!(e.addr(), None, "★udp 는 여전히 없다");
        assert!(e.reachable(), "★그래도 보낼 수 있다");

        assert!(handle.send(frame(b"downlink").expect("씌운다")), "서버가 그 길로 보낸다");
        let got = read_frame_with(&mut s, &mut un).await.expect("클라가 받는다");
        assert_eq!(&got[..], b"downlink");
    }

    #[tokio::test]
    async fn 연결이_끊기면_하향_경로를_거둔다() {
        let t = table();
        let at = listening(t.clone(), IDENTIFY_TIMEOUT).await;
        let mut s = TcpStream::connect(at).await.expect("붙는다");
        s.write_all(&frame(&request("srvufrag", PWD)).expect("씌운다")).await.expect("보낸다");
        read_one_frame(&mut s).await.expect("답이 온다");
        let e = t.get("srvufrag").expect("있다");
        assert!(e.tcp().is_some());

        drop(s);
        for _ in 0..40 {
            if e.tcp().is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(e.tcp().is_none(), "★끊기면 거둔다 — 죽은 길로 보내지 않는다");
        assert!(!e.reachable(), "★보낼 데가 없다");
    }

    #[tokio::test]
    async fn 신원_뒤의_비_stun_프레임은_위로_올라간다() {
        let t = table();
        let (at, mut up) = listening_up(t, IDENTIFY_TIMEOUT).await;
        let mut s = TcpStream::connect(at).await.expect("붙는다");
        s.write_all(&frame(&request("srvufrag", PWD)).expect("씌운다")).await.expect("보낸다");
        match up.recv().await.expect("통지") {
            TcpIn::Identified { ufrag, .. } => assert_eq!(ufrag, "srvufrag"),
            other => panic!("★먼저 신원이다: {other:?}"),
        }
        let dtls = [0x16u8, 0xFE, 0xFD, 0x00];
        s.write_all(&frame(&dtls).expect("씌운다")).await.expect("보낸다");
        match up.recv().await.expect("프레임") {
            TcpIn::Frame { bytes, .. } => assert_eq!(&bytes[..], &dtls),
            other => panic!("★프레임이어야 한다: {other:?}"),
        }
    }

    #[tokio::test]
    async fn 신원_전의_비_stun_프레임은_안_올린다() {
        let t = table();
        let (at, mut up) = listening_up(t, Duration::from_millis(300)).await;
        let mut s = TcpStream::connect(at).await.expect("붙는다");
        s.write_all(&frame(&[0x16u8, 0xFE, 0xFD, 0x00]).expect("씌운다")).await.expect("보낸다");
        let got = tokio::time::timeout(Duration::from_millis(200), up.recv()).await;
        assert!(got.is_err(), "★자격을 밝히기 전 것은 아무것도 아니다");
    }

    #[tokio::test]
    async fn 끊기면_위에도_알린다() {
        let t = table();
        let (at, mut up) = listening_up(t, IDENTIFY_TIMEOUT).await;
        let mut s = TcpStream::connect(at).await.expect("붙는다");
        s.write_all(&frame(&request("srvufrag", PWD)).expect("씌운다")).await.expect("보낸다");
        assert!(matches!(up.recv().await, Some(TcpIn::Identified { .. })));
        drop(s);
        let ev = tokio::time::timeout(Duration::from_secs(2), up.recv()).await.expect("온다");
        assert!(matches!(ev, Some(TcpIn::Closed { .. })), "★수신 장부도 거둬야 한다");
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
