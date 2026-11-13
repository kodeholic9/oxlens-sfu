// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-AUX-DC-SEND · 정:IMPL-AUX-MESSAGE · 연:TRN.frame.DC · 연:AUX.op.MESSAGE · 연:RULE-AUX-MESSAGE-FORM · 연:RULE-AUX-MESSAGE-PROC · model: claude-opus-5-5
//! 보조 — DC 송신(준비 전 통지는 버퍼 64 · 넘치면 오래된 것부터 버리고 센다 · 실패는 계수이지 종료 사유가 아니다) · `MESSAGE` 중계(`user_id` 는 세션에서 · `msg_id` uuid).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::link::{Peer, Transport};

pub const DC_BUFFER: usize = 64;

static PENDING: OnceLock<Mutex<HashMap<String, VecDeque<Vec<u8>>>>> = OnceLock::new();
pub static DC_SEND_FAIL: AtomicU64 = AtomicU64::new(0);

fn pending() -> &'static Mutex<HashMap<String, VecDeque<Vec<u8>>>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn pending_users() -> usize {
    pending().lock().map(|g| g.len()).unwrap_or(0)
}

fn dc_transport(p: &Peer) -> Option<Arc<Transport>> {
    p.transports().into_iter().find(|t| t.dc_open.load(Ordering::Acquire))
}

pub fn dc_send(p: &Peer, frame: Vec<u8>) {
    match dc_transport(p) {
        Some(t) => dc_put(&t, frame),
        None => {
            if let Ok(mut g) = pending().lock() {
                let q = g.entry(p.user.clone()).or_default();
                q.push_back(frame);
                while q.len() > DC_BUFFER {
                    q.pop_front();
                    crate::ops::drop_count("dc_backlog_full");
                }
            }
        }
    }
}

pub fn flush(tr: &Arc<Transport>) {
    let q = pending().lock().ok().and_then(|mut g| g.remove(&tr.user));
    for f in q.into_iter().flatten() {
        dc_put(tr, f);
    }
}

fn dc_put(t: &Transport, frame: Vec<u8>) {
    match t.dc_out.try_send(frame) {
        Ok(()) => {}
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => crate::ops::drop_count("dc_out_full"),
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            DC_SEND_FAIL.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub fn forget(user: &str) {
    if let Ok(mut g) = pending().lock() {
        g.remove(user);
    }
}

pub fn message_body(room: &str, user: &str, content: &str) -> oxsig::msg::MessageBody {
    oxsig::msg::MessageBody { room_id: room.to_string(), user_id: user.to_string(), content: content.to_string() }
}

pub fn msg_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::{DC_OUT_MAX, DTLS_IN_MAX, Role};

    #[test]
    fn outside_fed_channels_are_bounded() {
        let t = Transport::new(1, "u", "", Role::Both, 0, false);
        for i in 0..DC_OUT_MAX + 10 {
            dc_put(&t, vec![i as u8]);
        }
        assert_eq!(t.dc_out.capacity(), 0, "dc_out 은 {DC_OUT_MAX} 에서 멈춘다");
        let mut over = 0;
        for i in 0..DTLS_IN_MAX + 10 {
            if t.dtls_in.try_send(vec![i as u8]).is_err() {
                over += 1;
            }
        }
        assert_eq!(over, 10, "dtls_in 은 {DTLS_IN_MAX} 넘게 담지 않는다");
    }
}
