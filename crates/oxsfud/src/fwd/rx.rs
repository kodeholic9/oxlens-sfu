// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-TRANSPORT · 정:RULE-FWD-MEDIA-WORKERS · 정:IMPL-RTCP-TERMINATE · model: claude-opus-5-5
//! 워커의 받기 입구 — 수신 스레드가 넘긴 SRTP · SRTCP 를 그 전송 객체의 워커가 복호해 RTP(`on_rtp`) · RTCP(`on_rtcp`)로 넘긴다.

use std::sync::Arc;

use crate::link::Transport;
use crate::link::io::Class;

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
