// author: kodeholic (powered by Claude)
//! 미디어 — 정§6 발행 · §7 구독·전달 · §8 duplex. 논리/물리 두 계층(`track`), 구독 배관(`subscribe`),
//! 구독 연결마다인 두 표(`mid`·`pt`), 방 공용 슬롯(`slot`), egress 도구(`rtp`).

pub mod codec;
pub mod floor;
pub mod mid;
pub mod nack;
pub mod pt;
pub mod rewriter;
pub mod reception;
pub mod rtcp;
pub mod rtx;
pub mod rtp;
pub mod slot;
pub mod twcc;
pub mod subscribe;
pub mod track;

/// 연§4-2 확장 헤더 표 전량 — 서버 선언값. 발행자가 신고를 안 하면 이것이 답이고,
/// 구독자 기본표는 여기서 ★mid 를 뺀 것이다(정§7-2-1 — 구독자에게 mid 확장을 주지 않는다).
pub const SERVER_EXTMAP: [(u8, &str); 6] = [
    (1, "urn:ietf:params:rtp-hdrext:sdes:mid"),
    (4, "urn:ietf:params:rtp-hdrext:ssrc-audio-level"),
    (5, "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time"),
    (6, "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"),
    (10, "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id"),
    (11, "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id"),
];

pub const URI_MID: &str = SERVER_EXTMAP[0].1;
pub const URI_AUDIO_LEVEL: &str = SERVER_EXTMAP[1].1;
pub const URI_ABS_SEND_TIME: &str = SERVER_EXTMAP[2].1;
pub const URI_TWCC: &str = SERVER_EXTMAP[3].1;
pub const URI_RID: &str = SERVER_EXTMAP[4].1;
pub const URI_REPAIRED_RID: &str = SERVER_EXTMAP[5].1;

/// 정§7-2-1 — 2pc 구독자 기본표.
pub fn subscriber_extmap() -> Vec<oxsig::schema::Extmap> {
    SERVER_EXTMAP
        .iter()
        .filter(|(_, uri)| *uri != URI_MID)
        .map(|(id, uri)| oxsig::schema::Extmap { id: *id, uri: (*uri).to_owned() })
        .collect()
}
