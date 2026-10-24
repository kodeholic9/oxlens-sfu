// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-PUB-ADD · 정:IMPL-PUB-ACCEPT · 정:IMPL-PUB-RID-SPATIAL · 정:CAND-PUB-H264-PROFILE · 정:CAND-PUB-NOT-PUB-ROOM-ORDER · 정:CAND-PUB-RID-EXTRA · 연:RULE-PUB-PUBLISH-PROC · 연:RULE-PUB-REMOVE-FORM · 연:RULE-ROOM-PERMISSION-MAP · 연:RULE-ROOM-MEDIA-CODECS · 연:RULE-PUB-RID-TO-SPATIAL · 연:PUB.S.add · 연:PUB.S.add_2006 · 연:PUB.S.add_3011 · 연:PUB.S.add_4002 · model: claude-opus-5-5
//! 발행 판정(순수) — 검사 차례(자격 → 발행 방 → 수 → 필수 칸 → 1pc SSRC → 코덱) · 사상표 비트 · 코덱 등록부 · 키프레임 판정기(VP8 · H264) · rid → spatial.

use oxsig::types::Kind;
use oxsig::{Code, Duplex, Failure, Permission, PublishTrack, Source};
use serde_json::json;

pub const TRACK_LIMIT_REQ: usize = 8;
pub const TRACK_LIMIT_ACTIVE: usize = 8;
pub const SUPPORTED_VIDEO: &[&str] = &["VP8", "H264"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bit {
    Audio,
    Video,
    Screen,
    Floor,
}

impl Bit {
    pub fn name(self) -> &'static str {
        match self {
            Bit::Audio => "publish_audio",
            Bit::Video => "publish_video",
            Bit::Screen => "publish_screen",
            Bit::Floor => "floor_request",
        }
    }

    pub fn of(p: &Permission, b: Bit) -> bool {
        match b {
            Bit::Audio => p.publish_audio,
            Bit::Video => p.publish_video,
            Bit::Screen => p.publish_screen,
            Bit::Floor => p.floor_request,
        }
    }
}

pub fn bit_for(kind: Kind, source: Option<Source>) -> Bit {
    match (kind, source) {
        (Kind::Audio, _) => Bit::Audio,
        (Kind::Video, Some(Source::Screen)) => Bit::Screen,
        (Kind::Video, _) => Bit::Video,
    }
}

pub fn video_codec(name: Option<&str>) -> Result<&'static str, Failure> {
    let n = name.unwrap_or("");
    for s in SUPPORTED_VIDEO {
        if n.eq_ignore_ascii_case(s) {
            return Ok(s);
        }
    }
    Err(Code::CodecRequired.failure_with(format!("video codec {n:?} 지원 아님"), Some(json!({"supported": SUPPORTED_VIDEO}))))
}

fn fmtp_param(fmtp: &str, name: &str) -> Option<String> {
    fmtp.split(';').map(str::trim).find_map(|kv| kv.split_once('=').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_ascii_lowercase()))
}

pub fn codec_matches(room_codec: &str, room_fmtp: Option<&str>, codec: &str, fmtp: Option<&str>) -> bool {
    if !room_codec.eq_ignore_ascii_case(codec) {
        return false;
    }
    if !codec.eq_ignore_ascii_case("H264") {
        return true;
    }
    let rf = room_fmtp.unwrap_or("packetization-mode=1;profile-level-id=42e01f");
    let f = fmtp.unwrap_or("");
    let pm = |s: &str| fmtp_param(s, "packetization-mode").unwrap_or_else(|| "0".into());
    let prof = |s: &str| fmtp_param(s, "profile-level-id").map(|p| p.chars().take(4).collect::<String>()).unwrap_or_else(|| "42e0".into());
    pm(rf) == pm(f) && prof(rf) == prof(f)
}

#[derive(Clone)]
pub struct AddCtx<'a> {
    pub permission: Permission,
    pub is_pub_room: bool,
    pub active: usize,
    pub uplink_ssrcs_taken: &'a dyn Fn(u32) -> bool,
    pub room_video: &'a str,
    pub room_video_fmtp: Option<&'a str>,
    pub negotiated: &'a dyn Fn(Kind) -> bool,
}

pub fn check_add(c: &AddCtx<'_>, tracks: &[PublishTrack]) -> Result<(), Failure> {
    let denied: Vec<&str> = tracks.iter().map(|t| bit_for(t.kind, t.source)).filter(|b| !Bit::of(&c.permission, *b)).map(Bit::name).collect();
    if !denied.is_empty() {
        let mut d = denied.clone();
        d.dedup();
        return Err(Code::NotAuthorized.failure_with("권한 비트가 막는다", Some(json!({"denied": d}))));
    }
    if !c.is_pub_room && tracks.iter().any(|t| t.duplex.unwrap_or(Duplex::Full) == Duplex::Full) {
        return Err(Code::NotPubRoom.failure());
    }
    if tracks.len() > TRACK_LIMIT_REQ || c.active + tracks.len() > TRACK_LIMIT_ACTIVE {
        return Err(Code::TrackLimit.failure_with("트랙 상한", Some(json!({"limit": TRACK_LIMIT_ACTIVE, "current": c.active}))));
    }
    for t in tracks {
        if t.mid.is_empty() {
            return Err(Code::MissingField.failure_with("tracks[].mid", None));
        }
        if t.kind == Kind::Video && t.codec.is_none() {
            return Err(Code::CodecRequired.failure_with("video codec 이 없다", Some(json!({"supported": SUPPORTED_VIDEO}))));
        }
        if !(c.negotiated)(t.kind) {
            return Err(Code::MissingField.failure_with("협상한 확장 번호가 없다", None));
        }
    }
    for t in tracks {
        if (c.uplink_ssrcs_taken)(t.ssrc) || t.rtx_ssrc.map(|r| (c.uplink_ssrcs_taken)(r)).unwrap_or(false) {
            return Err(Code::SsrcCollision.failure_with("다운링크 SSRC 와 겹친다", Some(json!({"ssrc": t.ssrc}))));
        }
    }
    for t in tracks.iter().filter(|t| t.kind == Kind::Video) {
        let codec = video_codec(t.codec.as_deref())?;
        if !codec_matches(c.room_video, c.room_video_fmtp, codec, t.fmtp.as_deref()) {
            return Err(Code::CodecMismatch.failure_with("방 미디어 코덱과 다르다", Some(json!({"room": c.room_video, "track": codec}))));
        }
    }
    Ok(())
}

pub fn rid_layer(rid: &str) -> Option<u8> {
    match rid {
        "l" => Some(0),
        "h" => Some(1),
        _ => None,
    }
}

pub fn vp8_keyframe(payload: &[u8]) -> bool {
    if payload.is_empty() {
        return false;
    }
    let mut i = 0;
    let x = payload[0] & 0x80 != 0;
    let s = payload[0] & 0x10 != 0;
    let pid = payload[0] & 0x07;
    i += 1;
    if x {
        let Some(&ext) = payload.get(i) else { return false };
        i += 1;
        if ext & 0x80 != 0 {
            let Some(&m) = payload.get(i) else { return false };
            i += if m & 0x80 != 0 { 2 } else { 1 };
        }
        if ext & 0x40 != 0 {
            i += 1;
        }
        if ext & 0x30 != 0 {
            i += 1;
        }
    }
    if !s || pid != 0 {
        return false;
    }
    payload.get(i).map(|b| b & 0x01 == 0).unwrap_or(false)
}

pub fn h264_keyframe(payload: &[u8]) -> bool {
    let Some(&h) = payload.first() else { return false };
    let nal = h & 0x1f;
    match nal {
        5 | 7 => true,
        24 => {
            let mut i = 1;
            while i + 2 < payload.len() {
                let len = u16::from_be_bytes([payload[i], payload[i + 1]]) as usize;
                if let Some(&b) = payload.get(i + 2)
                    && matches!(b & 0x1f, 5 | 7)
                {
                    return true;
                }
                i += 2 + len;
            }
            false
        }
        28 | 29 => payload.get(1).map(|b| b & 0x80 != 0 && matches!(b & 0x1f, 5 | 7)).unwrap_or(false),
        _ => false,
    }
}

pub fn keyframe(codec: &str, payload: &[u8]) -> bool {
    if codec.eq_ignore_ascii_case("VP8") {
        vp8_keyframe(payload)
    } else if codec.eq_ignore_ascii_case("H264") {
        h264_keyframe(payload)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(kind: Kind, source: Option<Source>, codec: Option<&str>, duplex: Duplex) -> PublishTrack {
        PublishTrack { kind, ssrc: 11, mid: "0".into(), codec: codec.map(str::to_string), fmtp: Some("packetization-mode=1;profile-level-id=42e01f".into()), pt: 102, rtx_pt: None, rtx_ssrc: None, source, duplex: Some(duplex), simulcast: None }
    }

    #[test]
    fn order_is_authority_then_count_then_fields_then_codec() {
        let no_video = Permission { publish_video: false, ..Permission::ALL };
        let taken = |_: u32| false;
        let neg = |_: Kind| true;
        let ctx = AddCtx { permission: no_video, is_pub_room: true, active: 8, uplink_ssrcs_taken: &taken, room_video: "H264", room_video_fmtp: None, negotiated: &neg };
        let bad = vec![t(Kind::Video, Some(Source::Camera), None, Duplex::Full)];
        assert_eq!(check_add(&ctx, &bad).map_err(|f| f.code), Err(Code::NotAuthorized.number()), "S-sym-IMPL-PUB-ADD-1 자격이 먼저 — 코덱을 보기 전에");
        let ctx2 = AddCtx { permission: Permission::ALL, ..ctx.clone() };
        assert_eq!(check_add(&ctx2, &bad).map_err(|f| f.code), Err(Code::TrackLimit.number()), "S-sym-IMPL-PUB-ADD-2 활성 수 8+1 은 TRACK_LIMIT");
        let ctx3 = AddCtx { active: 0, ..ctx2.clone() };
        assert_eq!(check_add(&ctx3, &bad).map_err(|f| f.code), Err(Code::CodecRequired.number()));
        let vp9 = vec![t(Kind::Video, Some(Source::Camera), Some("VP9"), Duplex::Full)];
        assert_eq!(check_add(&ctx3, &vp9).map_err(|f| f.code), Err(Code::CodecRequired.number()), "S-sym-IMPL-PUB-ACCEPT-1 키프레임 판정기 없는 코덱은 지원이 아니다");
        let vp8 = vec![t(Kind::Video, Some(Source::Camera), Some("VP8"), Duplex::Full)];
        assert_eq!(check_add(&ctx3, &vp8).map_err(|f| f.code), Err(Code::CodecMismatch.number()));
        let off = AddCtx { is_pub_room: false, ..ctx3.clone() };
        assert_eq!(check_add(&off, &vp8).map_err(|f| f.code), Err(Code::NotPubRoom.number()), "S-sym-IMPL-PUB-ADD-3 전이중은 발행 방에서만");
        assert!(check_add(&off, &[t(Kind::Audio, None, None, Duplex::Half)]).is_ok());
    }

    #[test]
    fn keyframes() {
        assert!(vp8_keyframe(&[0x10, 0x00]));
        assert!(!vp8_keyframe(&[0x10, 0x01]));
        assert!(vp8_keyframe(&[0x90, 0x80, 0x81, 0x23, 0x00]));
        assert!(h264_keyframe(&[0x65]));
        assert!(h264_keyframe(&[0x7c, 0x85]));
        assert!(!h264_keyframe(&[0x7c, 0x05]));
        assert!(h264_keyframe(&[0x78, 0, 2, 0x67, 0x42]));
        assert!(!h264_keyframe(&[0x41]));
    }

    #[test]
    fn rid_and_codecs() {
        assert_eq!(rid_layer("l"), Some(0));
        assert_eq!(rid_layer("q"), None, "표 밖 rid 는 받지 않는다");
        assert!(codec_matches("H264", Some("packetization-mode=1;profile-level-id=42e01f"), "h264", Some("profile-level-id=42e034;packetization-mode=1")));
        assert!(!codec_matches("H264", None, "H264", Some("packetization-mode=0;profile-level-id=42e01f")));
    }
}
