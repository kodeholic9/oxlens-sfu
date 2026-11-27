// author: kodeholic (powered by Claude)
//! 연결 모드(SDK:RULE-PLAT-PC-MODE · SDK:RULE-PLAT-SERVER `pc_mode`) — oxpeer 는 늘 `1pc` 다(앱 설정 없음 · 능력표 밖). 서버마다 전송 하나 · BIND 에 늘 `1pc` · `ROOM_JOIN` 에 늘 번호표.
//! 번호표는 oxpeer 의 고정 표다(SDK:RULE-PLAT-SERVER 번호표 — 확장은 연:LINK.fact.ServerExtMap 전량 그 번호 · PT 는 opus 111 · VP8 96/97 · H264 102/103).
//! 발행 PT · 확장 번호도 이 표에서 읽는다(표는 여기 하나).

use crate::generated::ws_wire::{ExtMap, Kind, RoomJoinReqCodecs};
use crate::link::depack::Codec;

pub(crate) const URI_MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
pub(crate) const URI_AUDIO_LEVEL: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
pub(crate) const URI_ABS_SEND_TIME: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
pub(crate) const URI_TWCC: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
pub(crate) const URI_RID: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
pub(crate) const URI_REPAIR_RID: &str = "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id";
pub(crate) const URI_DD: &str = "https://aomediacodec.github.io/av1-rtp-spec/#dependency-descriptor-rtp-header-extension";

pub(crate) const EXTMAP: [(u8, &str); 7] = [(1, URI_MID), (4, URI_AUDIO_LEVEL), (5, URI_ABS_SEND_TIME), (6, URI_TWCC), (10, URI_RID), (11, URI_REPAIR_RID), (12, URI_DD)];

pub(crate) fn ext_id(uri: &str) -> Option<u8> {
    EXTMAP.iter().find(|(_, u)| *u == uri).map(|(id, _)| *id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PtRow {
    pub kind: Kind,
    pub codec: Codec,
    pub name: &'static str,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub fmtp: Option<&'static str>,
}

pub(crate) const CODECS: [PtRow; 3] = [
    PtRow { kind: Kind::Audio, codec: Codec::Opus, name: "opus", pt: 111, rtx_pt: None, fmtp: None },
    PtRow { kind: Kind::Video, codec: Codec::Vp8, name: "VP8", pt: 96, rtx_pt: Some(97), fmtp: None },
    PtRow { kind: Kind::Video, codec: Codec::H264, name: "H264", pt: 102, rtx_pt: Some(103), fmtp: Some("level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f") },
];

pub(crate) fn pt_row(codec: Codec) -> Option<&'static PtRow> {
    CODECS.iter().find(|r| r.codec == codec)
}

pub(crate) fn join_tags() -> (Vec<ExtMap>, Vec<RoomJoinReqCodecs>) {
    (
        EXTMAP.iter().map(|(id, uri)| ExtMap { id: *id, uri: (*uri).to_string() }).collect(),
        CODECS.iter().map(|r| RoomJoinReqCodecs { kind: r.kind, pt: r.pt, name: r.name.to_string(), fmtp: r.fmtp.map(str::to_string), rtx_pt: r.rtx_pt }).collect(),
    )
}

#[cfg(test)]
mod pc_mode_test {
    use super::*;

    #[test]
    fn number_tags_and_publish_pts_come_from_the_same_table() {
        let (ext, codecs) = join_tags();
        let ext: Vec<(u8, &str)> = ext.iter().map(|e| (e.id, e.uri.as_str())).collect();
        assert_eq!(ext, vec![(1, URI_MID), (4, URI_AUDIO_LEVEL), (5, URI_ABS_SEND_TIME), (6, URI_TWCC), (10, URI_RID), (11, URI_REPAIR_RID), (12, URI_DD)], "SDK:RULE-PLAT-SERVER 번호표 = 연:LINK.fact.ServerExtMap 전량 그 번호 그대로");
        let rows: Vec<(Kind, &str, u8, Option<u8>)> = codecs.iter().map(|c| (c.kind, c.name.as_str(), c.pt, c.rtx_pt)).collect();
        assert_eq!(rows, vec![(Kind::Audio, "opus", 111, None), (Kind::Video, "VP8", 96, Some(97)), (Kind::Video, "H264", 102, Some(103))]);
        assert_eq!(codecs[2].fmtp.as_deref(), Some("level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"));
        for c in [Codec::Opus, Codec::Vp8, Codec::H264] {
            let r = pt_row(c).expect("발행 코덱은 표에 있다");
            assert!(codecs.iter().any(|x| x.pt == r.pt && x.rtx_pt == r.rtx_pt && x.name == r.name), "발행 PT 와 번호표가 한 표");
        }
        assert!(pt_row(Codec::Other).is_none());
        let (extmap, codecs) = join_tags();
        let body = serde_json::to_value(crate::generated::ws_wire::RoomJoinReq { room_id: "r".into(), role: None, select: Some(false), extmap: Some(extmap), codecs: Some(codecs), join_id: None }).unwrap_or_default();
        assert_eq!(body["codecs"][0]["kind"], "audio", "코덱 줄은 kind 를 싣는다(CAND-ROOM-JOIN-CODEC-KIND)");
        assert_eq!(body["extmap"][0]["id"], 1);
    }

    #[test]
    fn publish_extension_ids_come_from_the_tags() {
        assert_eq!(ext_id(URI_MID), Some(1), "SDK:RULE-PLAT-SERVER 발행은 번호표 번호");
        assert_eq!(ext_id(URI_AUDIO_LEVEL), Some(4));
        assert_eq!(ext_id(URI_TWCC), Some(6));
        assert_eq!(ext_id(URI_RID), Some(10));
        assert_eq!(ext_id(URI_REPAIR_RID), Some(11));
        assert_eq!(ext_id("urn:x"), None);
    }
}
