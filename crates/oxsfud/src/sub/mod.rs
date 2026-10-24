// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-SUB-SERVER-SCOPE · 정:RULE-SUB-STATE · 정:IMPL-SUB-PIPELINE · 정:IMPL-SUB-READY-GATE · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:CAND-SUB-MID-EXT-ALWAYS · 연:SUB.op.READY · 연:SUB.op.SUBSCRIBE_LAYER · 연:RULE-SUB-READY-PROC · 연:RULE-SUB-READY-IDEMPOTENT · 연:RULE-SUB-TGATE · 연:RULE-SUB-LAYER-PROC · 연:SUB.S.ready · 연:SUB.S.drop · 연:RULE-ROOM-JOIN-NUMBER-TAGS · model: claude-opus-5-5
//! 영역 SUB — 구독자 확장 표(`2pc` = 서버 선언 · `1pc` = 입장 번호표) · video 게이트(방 단위 잠금 · `READY` 해제 · 시한 5s 안전망).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use oxsig::{ExtMap, PcMode};

use crate::fwd::core::ExtTable;
use crate::link::Transport;

pub const URI_MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
pub const URI_LEVEL: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
pub const URI_ABS: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
pub const URI_TWCC: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
pub const URI_RID: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
pub const URI_RRID: &str = "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id";
pub const T_GATE_MS: u64 = 5_000;

pub fn server_extmap() -> Vec<ExtMap> {
    [(1, URI_MID), (4, URI_LEVEL), (5, URI_ABS), (6, URI_TWCC), (10, URI_RID), (11, URI_RRID)].into_iter().map(|(id, uri)| ExtMap { id, uri: uri.to_string() }).collect()
}

pub fn ext_table(pc: PcMode, tags: Option<&[ExtMap]>, audio: bool) -> ExtTable {
    let server = server_extmap();
    let id_of = |uri: &str| -> Option<u8> {
        let tagged = if pc == PcMode::V1pc { tags.and_then(|t| t.iter().find(|e| e.uri == uri)).map(|e| e.id) } else { None };
        tagged.or_else(|| server.iter().find(|e| e.uri == uri).map(|e| e.id))
    };
    ExtTable { mid: id_of(URI_MID), audio_level: if audio { id_of(URI_LEVEL) } else { None }, abs_send_time: id_of(URI_ABS), twcc: id_of(URI_TWCC) }
}

pub fn gate_lock(t: &Arc<Transport>, room: &str, now_ms: u64) {
    if t.gates.load().contains_key(room) {
        return;
    }
    let cell = Arc::new(AtomicU64::new(now_ms.max(1)));
    t.gates.rcu(|m| {
        let mut m = HashMap::clone(m);
        m.entry(room.to_string()).or_insert_with(|| cell.clone());
        m
    });
}

pub fn gate_open(t: &Arc<Transport>, room: &str) -> bool {
    match t.gates.load().get(room) {
        Some(c) => c.swap(0, Ordering::AcqRel) != 0,
        None => false,
    }
}

pub fn gate_closed(t: &Transport, room: &str) -> bool {
    t.gates.load().get(room).map(|c| c.load(Ordering::Acquire) != 0).unwrap_or(false)
}

pub fn gate_drop(t: &Arc<Transport>, room: &str) {
    t.gates.rcu(|m| {
        let mut m = HashMap::clone(m);
        m.remove(room);
        m
    });
}

pub fn gate_expired(t: &Transport, now_ms: u64) -> Vec<String> {
    t.gates.load().iter().filter(|(_, c)| {
        let since = c.load(Ordering::Acquire);
        since != 0 && now_ms.saturating_sub(since) >= T_GATE_MS
    }).map(|(r, _)| r.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_tables() {
        let t2 = ext_table(PcMode::V2pc, None, true);
        assert_eq!((t2.mid, t2.audio_level, t2.abs_send_time, t2.twcc), (Some(1), Some(4), Some(5), Some(6)), "abs-send-time 은 양쪽 · 음량은 오디오");
        let tags = vec![ExtMap { id: 3, uri: URI_MID.into() }, ExtMap { id: 9, uri: URI_TWCC.into() }];
        let t1 = ext_table(PcMode::V1pc, Some(&tags), false);
        assert_eq!((t1.mid, t1.abs_send_time, t1.twcc), (Some(3), Some(5), Some(9)), "1pc 는 번호표 · 그 밖은 서버 선언 번호");
    }
}
