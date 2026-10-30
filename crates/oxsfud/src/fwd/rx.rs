// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-TRANSPORT · 정:RULE-FWD-MEDIA-WORKERS · 정:IMPL-RTCP-TERMINATE · model: claude-opus-5-5
//! 수신 스레드 — UDP 포트 하나에서 받아 demux 하고(STUN · DTLS 는 그 자리 처리) RTP · RTCP 는 그 전송 객체의 미디어 워커로 넘긴다. 워커는 복호한 평문을 RTP(`on_rtp`) · RTCP(`on_rtcp`)로 넘긴다.

use std::sync::Arc;

use crate::link::io::Class;
use crate::link::{Io, Transport, Via};

pub fn dispatch(t: &Arc<Transport>, class: Class, data: &[u8]) {
    let now = common::time::mono_ms();
    match class {
        Class::Rtp => {
            if let Some(p) = t.unprotect_rtp(data) {
                super::on_rtp(t, &p, now);
            }
        }
        Class::Rtcp => {
            if let Some(p) = t.unprotect_rtcp(data) {
                crate::rtcp::on_rtcp(t, &p, now);
            }
        }
        _ => {}
    }
}

pub fn udp_loop(io: &Arc<Io>) {
    let mut buf = vec![0u8; 65_536];
    loop {
        let (n, from) = match io.udp.recv_from(&mut buf) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let data = &buf[..n];
        if let Some((t, class)) = io.on_datagram(data, Via::Udp(from), common::time::mono_ms()) {
            super::work::submit_in(&t, class, data.to_vec());
        }
    }
}
