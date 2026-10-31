// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-TRK-FROM-PUB · 정:IMPL-TRK-CODEC-REGISTRY · 연:RULE-TRK-PUB-TO-TRACK · 연:RULE-TRK-SLOT-LIFE · 연:RULE-ROOM-MEDIA-CODECS · 연:TRK.type.TrackEntry · 정:CAND-PUB-H264-PROFILE · 정:CAND-PUB-CODEC-REGISTRY-PLACE · 연:RULE-TRK-FIELDS-BY-TYPE · 연:RULE-TRK-FOUR-PATHS · model: claude-opus-5-5
//! 트랙 — 코덱 등록부(VP8 · H264 · 키프레임 판정기가 있는 것만) · 슬롯 트랙 둘(방과 수명) · 발행에서 파생하는 개인 트랙 목록(`muted` 포함).

use oxsig::msg::{Kind, MediaCodecs, TrackEntry, TrackEntryType};

use crate::r#pub::PubStream;

pub const SUPPORTED_VIDEO: &[&str] = &["VP8", "H264"];
pub const DEFAULT_AUDIO: &str = "opus";
pub const DEFAULT_VIDEO: &str = "H264";
pub const ROOM_H264_PROFILE: &str = "42e0";
pub const SCALABILITY: &str = "L2T1";
pub const FMTP_H264: &str = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f";
pub const FMTP_OPUS: &str = "minptime=10;useinbandfec=1";

pub fn canonical_video(name: &str) -> Option<&'static str> {
    SUPPORTED_VIDEO.iter().find(|c| c.eq_ignore_ascii_case(name)).copied()
}

pub fn supported_list() -> serde_json::Value {
    serde_json::json!(SUPPORTED_VIDEO)
}

pub fn row_fmtp(codec: &str) -> Option<String> {
    if codec.eq_ignore_ascii_case("H264") {
        Some(FMTP_H264.to_string())
    } else if codec.eq_ignore_ascii_case("opus") {
        Some(FMTP_OPUS.to_string())
    } else {
        None
    }
}

pub fn mismatch(kind: Kind, room: &MediaCodecs) -> oxsig::Failure {
    let codec = match kind {
        Kind::Audio => room.audio.clone().unwrap_or_else(|| DEFAULT_AUDIO.to_string()),
        Kind::Video => room_video(room).to_string(),
    };
    let mut d = serde_json::json!({ "codec": codec });
    if let Some(f) = row_fmtp(&codec) {
        d["fmtp"] = serde_json::json!(f);
    }
    oxsig::Code::CodecMismatch.failure().with_details(d)
}

pub fn fmtp_param(fmtp: Option<&str>, key: &str) -> Option<String> {
    fmtp?.split(';').map(|kv| kv.trim()).find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        if k.trim().eq_ignore_ascii_case(key) { Some(v.trim().to_string()) } else { None }
    })
}

pub fn h264_profile(fmtp: Option<&str>) -> Option<String> {
    fmtp_param(fmtp, "profile-level-id").filter(|p| p.len() >= 4).map(|p| p[..4].to_ascii_lowercase())
}

pub fn resolve_room_codecs(req: Option<&MediaCodecs>) -> Option<MediaCodecs> {
    let audio = match req.and_then(|m| m.audio.as_deref()) {
        None => DEFAULT_AUDIO.to_string(),
        Some(a) if a.eq_ignore_ascii_case("opus") => DEFAULT_AUDIO.to_string(),
        Some(_) => return None,
    };
    let video = match req.and_then(|m| m.video.as_deref()) {
        None => DEFAULT_VIDEO.to_string(),
        Some(v) => canonical_video(v)?.to_string(),
    };
    Some(MediaCodecs { audio: Some(audio), video: Some(video) })
}

pub fn room_video(m: &MediaCodecs) -> &str {
    m.video.as_deref().unwrap_or(DEFAULT_VIDEO)
}

pub fn matches_room(kind: Kind, codec: Option<&str>, fmtp: Option<&str>, room: &MediaCodecs) -> bool {
    match kind {
        Kind::Audio => codec.map(|c| c.eq_ignore_ascii_case("opus")).unwrap_or(true),
        Kind::Video => {
            let Some(c) = codec.and_then(canonical_video) else { return false };
            if !c.eq_ignore_ascii_case(room_video(room)) {
                return false;
            }
            if c == "H264" {
                let pm = fmtp_param(fmtp, "packetization-mode").unwrap_or_else(|| "0".into());
                return pm == "1" && h264_profile(fmtp).as_deref() == Some(ROOM_H264_PROFILE);
            }
            true
        }
    }
}

pub fn is_keyframe(codec: &str, p: &[u8]) -> bool {
    if codec.eq_ignore_ascii_case("VP8") {
        return vp8_key(p);
    }
    if codec.eq_ignore_ascii_case("H264") {
        return h264_key(p);
    }
    false
}

fn vp8_key(p: &[u8]) -> bool {
    let Some(&b0) = p.first() else { return false };
    if b0 & 0x10 == 0 || b0 & 0x07 != 0 {
        return false;
    }
    let mut i = 1;
    if b0 & 0x80 != 0 {
        let Some(&x) = p.get(1) else { return false };
        i = 2;
        if x & 0x80 != 0 {
            let Some(&pid) = p.get(i) else { return false };
            i += if pid & 0x80 != 0 { 2 } else { 1 };
        }
        if x & 0x40 != 0 {
            i += 1;
        }
        if x & 0x30 != 0 {
            i += 1;
        }
    }
    p.get(i).map(|h| h & 0x01 == 0).unwrap_or(false)
}

fn h264_nal_key(t: u8) -> bool {
    t == 5 || t == 7
}

fn h264_key(p: &[u8]) -> bool {
    let Some(&b0) = p.first() else { return false };
    match b0 & 0x1f {
        24 => {
            let mut i = 1;
            while i + 2 < p.len() {
                let n = u16::from_be_bytes([p[i], p[i + 1]]) as usize;
                if let Some(&h) = p.get(i + 2)
                    && h264_nal_key(h & 0x1f)
                {
                    return true;
                }
                i += 2 + n;
            }
            false
        }
        28 => p.get(1).map(|h| h & 0x80 != 0 && h264_nal_key(h & 0x1f)).unwrap_or(false),
        t => h264_nal_key(t),
    }
}

pub fn slot_track_id(room: &str, kind: Kind) -> String {
    format!("ptt-{room}-{}", kind.as_str())
}

pub fn slot_entries(room: &str, codecs: &MediaCodecs) -> Vec<TrackEntry> {
    [Kind::Audio, Kind::Video]
        .into_iter()
        .map(|k| {
            let codec = match k {
                Kind::Audio => None,
                Kind::Video => Some(room_video(codecs).to_string()),
            };
            TrackEntry {
                type_: TrackEntryType::Slot,
                room_id: room.to_string(),
                track_id: slot_track_id(room, k),
                kind: k,
                fmtp: codec.as_deref().and_then(row_fmtp),
                codec,
                user_id: None,
                source: None,
                active: None,
                muted: None,
                simulcast: None,
                scalability: None,
            }
        })
        .collect()
}

pub fn entry(s: &PubStream) -> TrackEntry {
    TrackEntry {
        type_: TrackEntryType::Individual,
        room_id: s.room.load().as_str().to_string(),
        track_id: s.track_id.clone(),
        kind: s.kind,
        codec: if s.kind == Kind::Video { Some(s.codec_name()) } else { None },
        fmtp: s.fmtp.clone(),
        user_id: Some(s.user.clone()),
        source: s.source,
        active: Some(!s.is_half()),
        muted: Some(s.muted.load(std::sync::atomic::Ordering::Acquire)),
        simulcast: Some(s.simulcast),
        scalability: if s.simulcast { Some(SCALABILITY.to_string()) } else { None },
    }
}

pub fn listed(s: &PubStream, room: &str) -> bool {
    s.listed.load(std::sync::atomic::Ordering::Acquire) && s.room.load().as_str() == room
}

pub fn room_entries(room: &str, codecs: &MediaCodecs, streams: &[std::sync::Arc<PubStream>]) -> Vec<TrackEntry> {
    let mut v = slot_entries(room, codecs);
    let mut ind: Vec<TrackEntry> = streams.iter().filter(|s| listed(s, room)).map(|s| entry(s)).collect();
    ind.sort_by(|a, b| a.track_id.cmp(&b.track_id));
    v.extend(ind);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyframes() {
        assert!(is_keyframe("VP8", &[0x10, 0x00]));
        assert!(!is_keyframe("VP8", &[0x10, 0x01]));
        assert!(is_keyframe("VP8", &[0x90, 0x80, 0x12, 0x00]));
        assert!(is_keyframe("H264", &[0x65]));
        assert!(is_keyframe("H264", &[0x7c, 0x85]));
        assert!(!is_keyframe("H264", &[0x7c, 0x05]));
        assert!(is_keyframe("H264", &[0x18, 0x00, 0x02, 0x67, 0x42]));
        assert!(!is_keyframe("H264", &[0x41]));
    }

    #[test]
    fn room_match() {
        let r = MediaCodecs { audio: Some("opus".into()), video: Some("H264".into()) };
        assert!(matches_room(Kind::Video, Some("H264"), Some("level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"), &r));
        assert!(!matches_room(Kind::Video, Some("H264"), Some("packetization-mode=1;profile-level-id=42001f"), &r));
        assert!(!matches_room(Kind::Video, Some("VP8"), None, &r));
        assert!(resolve_room_codecs(Some(&MediaCodecs { audio: None, video: Some("vp9".into()) })).is_none());
    }
}
