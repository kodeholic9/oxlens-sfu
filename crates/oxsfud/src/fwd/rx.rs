// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-TRANSPORT · 정:RULE-CONC-PERF-AXES · 정:IMPL-RTCP-TERMINATE · model: claude-opus-5-5
//! 수신 루프 — UDP 포트 하나에서 받아 demux 하고, 복호한 평문을 RTP(`on_rtp`) · RTCP(`on_rtcp`)로 넘긴다. 수신 루프는 하나다(알고 두는 비용).

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

pub async fn udp_loop(io: Arc<Io>) {
    let mut buf = vec![0u8; 65_536];
    loop {
        let (n, from) = match io.udp.recv_from(&mut buf).await {
            Ok(x) => x,
            Err(_) => continue,
        };
        let data = &buf[..n];
        if let Some((t, class)) = io.on_datagram(data, Via::Udp(from), common::time::mono_ms()) {
            dispatch(&t, class, data);
        }
    }
}
