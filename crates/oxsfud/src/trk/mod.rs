// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-TRK-FROM-PUB · 정:IMPL-TRK-CODEC-REGISTRY · 정:CAND-PUB-CODEC-REGISTRY-PLACE · 연:TRK.type.TrackEntry · 연:RULE-TRK-PUB-TO-TRACK · 연:RULE-TRK-SLOT-LIFE · 연:RULE-ROOM-MEDIA-CODECS · 연:RULE-TRK-TRACK-STATE-TO-ALL · model: claude-opus-5-5
//! 트랙 층 — 코덱 등록부 하나(지원 = 키프레임 판정기가 있는 것) · 구독 줄의 고정 fmtp · 슬롯 트랙 id · 트랙 목록 항목.

use oxsig::types::{Kind, Source, TrackEntry, TrackEntryType};

pub const AUDIO_CODEC: &str = "opus";
pub const DEFAULT_VIDEO: &str = "H264";
pub const SUPPORTED_VIDEO: &[&str] = &["VP8", "H264"];
pub const SUPPORTED_AUDIO: &[&str] = &["opus"];
pub const FMTP_H264: &str = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f";
pub const FMTP_OPUS: &str = "minptime=10;useinbandfec=1";
pub const SCALABILITY: &str = "L2T1";

pub fn canonical_video(name: &str) -> Option<&'static str> {
    SUPPORTED_VIDEO.iter().find(|c| c.eq_ignore_ascii_case(name)).copied()
}

pub fn canonical_audio(name: &str) -> Option<&'static str> {
    SUPPORTED_AUDIO.iter().find(|c| c.eq_ignore_ascii_case(name)).copied()
}

pub fn sub_fmtp(codec: &str) -> Option<&'static str> {
    if codec.eq_ignore_ascii_case("H264") {
        Some(FMTP_H264)
    } else if codec.eq_ignore_ascii_case("opus") {
        Some(FMTP_OPUS)
    } else {
        None
    }
}

pub fn slot_track_id(room: &str, kind: Kind) -> String {
    format!("ptt-{room}-{}", kind.as_str())
}

pub fn clock(kind: Kind) -> u32 {
    match kind {
        Kind::Audio => 48_000,
        Kind::Video => 90_000,
    }
}

fn fmtp_param<'a>(fmtp: &'a str, key: &str) -> Option<&'a str> {
    fmtp.split(';').map(str::trim).find_map(|kv| kv.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

pub fn h264_profile_matches(publisher_fmtp: Option<&str>, room_fmtp: &str) -> bool {
    let Some(p) = publisher_fmtp else { return false };
    let pm = |f: &str| fmtp_param(f, "packetization-mode").unwrap_or("0").to_string();
    let prof = |f: &str| fmtp_param(f, "profile-level-id").map(|v| v.get(..4).unwrap_or(v).to_ascii_lowercase());
    pm(p) == pm(room_fmtp) && prof(p).is_some() && prof(p) == prof(room_fmtp)
}

pub fn codec_matches(kind: Kind, codec: &str, fmtp: Option<&str>, room_codec: &str) -> bool {
    if !codec.eq_ignore_ascii_case(room_codec) {
        return false;
    }
    if kind == Kind::Video && room_codec.eq_ignore_ascii_case("H264") {
        return h264_profile_matches(fmtp, FMTP_H264);
    }
    true
}

pub fn is_keyframe(codec: &str, payload: &[u8]) -> bool {
    if codec.eq_ignore_ascii_case("VP8") {
        return vp8_keyframe(payload);
    }
    if codec.eq_ignore_ascii_case("H264") {
        return h264_keyframe(payload);
    }
    false
}

fn vp8_keyframe(p: &[u8]) -> bool {
    let Some(&b0) = p.first() else { return false };
    let x = b0 & 0x80 != 0;
    let s = b0 & 0x10 != 0;
    let pid = b0 & 0x07;
    if !s || pid != 0 {
        return false;
    }
    let mut i = 1;
    if x {
        let Some(&b1) = p.get(1) else { return false };
        i = 2;
        if b1 & 0x80 != 0 {
            let Some(&m) = p.get(i) else { return false };
            i += if m & 0x80 != 0 { 2 } else { 1 };
        }
        if b1 & 0x40 != 0 {
            i += 1;
        }
        if b1 & 0x30 != 0 {
            i += 1;
        }
    }
    p.get(i).is_some_and(|h| h & 0x01 == 0)
}

fn h264_nal_key(t: u8) -> bool {
    t == 5 || t == 7
}

fn h264_keyframe(p: &[u8]) -> bool {
    let Some(&b0) = p.first() else { return false };
    let t = b0 & 0x1f;
    match t {
        1..=23 => h264_nal_key(t),
        24 => {
            let mut i = 1;
            while i + 2 < p.len() {
                let len = usize::from(u16::from_be_bytes([p[i], p[i + 1]]));
                if let Some(&h) = p.get(i + 2)
                    && h264_nal_key(h & 0x1f)
                {
                    return true;
                }
                i += 2 + len;
            }
            false
        }
        28 => p.get(1).is_some_and(|fu| fu & 0x80 != 0 && h264_nal_key(fu & 0x1f)),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub track_id: String,
    pub user_id: String,
    pub kind: Kind,
    pub codec: String,
    pub fmtp: Option<String>,
    pub source: Option<Source>,
    pub active: bool,
    pub muted: bool,
    pub simulcast: bool,
}

impl Item {
    pub fn entry(&self, room: &str) -> TrackEntry {
        let video = self.kind == Kind::Video;
        TrackEntry {
            type_: TrackEntryType::Individual,
            room_id: room.to_string(),
            track_id: self.track_id.clone(),
            kind: self.kind,
            codec: video.then(|| self.codec.clone()),
            fmtp: if video { self.fmtp.clone() } else { None },
            user_id: Some(self.user_id.clone()),
            source: if video { self.source.or(Some(Source::Camera)) } else { None },
            active: Some(self.active),
            muted: Some(self.muted),
            simulcast: video.then_some(self.simulcast),
            scalability: (video && self.simulcast).then(|| SCALABILITY.to_string()),
        }
    }

    pub fn removed(&self, room: &str) -> TrackEntry {
        TrackEntry { type_: TrackEntryType::Individual, room_id: room.to_string(), track_id: self.track_id.clone(), kind: self.kind, codec: None, fmtp: None, user_id: None, source: None, active: None, muted: None, simulcast: None, scalability: None }
    }
}

impl crate::room::TrackLayer for Item {
    fn entry(&self, room: &str) -> TrackEntry {
        Item::entry(self, room)
    }

    fn slot(room: &str, kind: Kind, video_codec: &str) -> TrackEntry {
        slot_entry(room, kind, video_codec)
    }
}

pub type Room = crate::room::Room<Item>;

pub fn slot_entry(room: &str, kind: Kind, video_codec: &str) -> TrackEntry {
    let video = kind == Kind::Video;
    TrackEntry {
        type_: TrackEntryType::Slot,
        room_id: room.to_string(),
        track_id: slot_track_id(room, kind),
        kind,
        codec: video.then(|| video_codec.to_string()),
        fmtp: if video { sub_fmtp(video_codec).map(str::to_string) } else { None },
        user_id: None,
        source: None,
        active: None,
        muted: None,
        simulcast: None,
        scalability: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_and_profile() {
        assert_eq!(canonical_video("vp8"), Some("VP8"));
        assert_eq!(canonical_video("VP9"), None);
        assert!(h264_profile_matches(Some("level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"), FMTP_H264));
        assert!(h264_profile_matches(Some("packetization-mode=1;profile-level-id=42e034"), FMTP_H264));
        assert!(!h264_profile_matches(Some("packetization-mode=1;profile-level-id=42001f"), FMTP_H264));
        assert!(!h264_profile_matches(Some("packetization-mode=0;profile-level-id=42e01f"), FMTP_H264));
        assert_eq!(slot_track_id("r1", Kind::Audio), "ptt-r1-audio");
    }

    #[test]
    fn keyframes() {
        assert!(is_keyframe("H264", &[0x65, 0]));
        assert!(is_keyframe("H264", &[0x7c, 0x85]));
        assert!(!is_keyframe("H264", &[0x7c, 0x01]));
        assert!(is_keyframe("H264", &[0x18, 0, 2, 0x67, 0]));
        assert!(is_keyframe("VP8", &[0x10, 0x00]));
        assert!(!is_keyframe("VP8", &[0x10, 0x01]));
        assert!(is_keyframe("VP8", &[0x90, 0x80, 0x01, 0x00]));
    }
}
