// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-FWD-MEDIA-WORKERS · model: claude-opus-5-5
//! TCP 송신과 닫힘이 겹치는 판 1층 — 상대가 끊긴 연결에 다운링크 워커가 계속 써도 panic 없이 닫힘으로 표시되고, 상대가 안 읽으면 송신 버퍼는 64KB 에서 멈춘다(선례 mediasoup #2038 TCP 전송로 파괴 중 크래시 · context 202611/20261113f E).

use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use oxsfud::fwd::rt::TcpOut;
use oxsfud::link::tcp::SEND_BUF;

fn pair() -> (TcpOut, TcpStream) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let c = TcpStream::connect(l.local_addr().expect("addr")).expect("connect");
    let (s, _) = l.accept().expect("accept");
    c.set_nonblocking(true).expect("nonblocking");
    (TcpOut { stream: Mutex::new(c), pending: Mutex::new(Vec::new()), open: AtomicBool::new(true) }, s)
}

#[test]
fn peer_that_never_reads_bounds_the_send_buffer() {
    let (out, _peer) = pair();
    let pkt = vec![7u8; 1200];
    let mut refused = 0;
    for _ in 0..20_000 {
        if !out.send(&pkt) {
            refused += 1;
        }
        assert!(out.pending.lock().expect("pending").len() <= SEND_BUF);
    }
    assert!(refused > 0, "안 읽는 상대에 20,000 × 1,200B 를 다 받아 줬다");
    assert!(out.open.load(Ordering::Acquire), "느린 상대는 닫힘이 아니다");
}

#[test]
fn writing_after_the_peer_closed_marks_closed_without_panic() {
    let (out, mut peer) = pair();
    let pkt = vec![9u8; 1200];
    for _ in 0..50 {
        out.send(&pkt);
    }
    let mut sink = [0u8; 4096];
    let _ = peer.set_read_timeout(Some(Duration::from_millis(50)));
    let _ = peer.read(&mut sink);
    drop(peer);
    thread::sleep(Duration::from_millis(50));
    for _ in 0..20_000 {
        out.send(&pkt);
        if !out.open.load(Ordering::Acquire) {
            break;
        }
    }
    assert!(!out.open.load(Ordering::Acquire), "끊긴 상대에 계속 써도 닫힘이 안 됐다");
    assert!(!out.send(&pkt), "닫힌 연결에 보낸 것을 받았다고 했다");
}

#[test]
fn many_writers_racing_a_close_all_finish() {
    for _ in 0..20 {
        let (out, peer) = pair();
        let out = Arc::new(out);
        let writers: Vec<_> = (0..4)
            .map(|k| {
                let o = out.clone();
                thread::spawn(move || {
                    let pkt = vec![k as u8; 900];
                    for _ in 0..2_000 {
                        o.send(&pkt);
                    }
                })
            })
            .collect();
        let closer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(2));
            drop(peer);
        });
        closer.join().expect("closer");
        for w in writers {
            w.join().expect("writer panicked");
        }
        assert!(out.pending.lock().expect("pending").len() <= SEND_BUF);
    }
}
