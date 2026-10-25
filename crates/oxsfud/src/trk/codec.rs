// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-PUB-ACCEPT 정:IMPL-PUB-ADD 정:CAND-PUB-H264-PROFILE 정:IMPL-ROOM-SERVER-CONFIG 연:RULE-ROOM-MEDIA-CODECS 연:RULE-LINK-EGRESS-REWRITE 정:CAND-PUB-CODEC-REGISTRY-PLACE · model: claude-opus-5-5
//! 코덱 등록부 단일 출처 — 지원 video = 키프레임 판정기가 있는 것(VP8 · H264) · 방 코덱 견줌은 파라미터로(H264 = packetization-mode · profile 앞 네 자리) · 구독 줄의 고정 fmtp.

use oxsig::msg::Kind;

pub const SUPPORTED_VIDEO: &[&str] = &["VP8", "H264"];
pub const AUDIO: &str = "opus";
pub const H264_ROOM_FMTP: &str = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f";
pub const OPUS_FMTP: &str = "minptime=10;useinbandfec=1";

pub fn canon(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "opus" => Some("opus"),
        "vp8" => Some("VP8"),
        "h264" => Some("H264"),
        "vp9" => Some("VP9"),
        "av1" => Some("AV1"),
        _ => None,
    }
}

pub fn supported_video(name: &str) -> Option<&'static str> {
    canon(name).filter(|c| SUPPORTED_VIDEO.contains(c))
}

pub fn clock(codec: &str) -> u32 {
    if codec.eq_ignore_ascii_case("opus") { 48_000 } else { 90_000 }
}

pub fn row_fmtp(codec: &str) -> Option<&'static str> {
    match canon(codec) {
        Some("H264") => Some(H264_ROOM_FMTP),
        Some("opus") => Some(OPUS_FMTP),
        _ => None,
    }
}

pub fn fmtp_param<'a>(fmtp: &'a str, key: &str) -> Option<&'a str> {
    fmtp.split(';').map(str::trim).find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        k.trim().eq_ignore_ascii_case(key).then(|| v.trim())
    })
}

pub fn h264_key(fmtp: Option<&str>) -> (u8, String) {
    let f = fmtp.unwrap_or("");
    let pm = fmtp_param(f, "packetization-mode").and_then(|v| v.parse().ok()).unwrap_or(0);
    let prof = fmtp_param(f, "profile-level-id").map(|v| v.get(..4).unwrap_or(v).to_ascii_lowercase()).unwrap_or_default();
    (pm, prof)
}

pub fn opus_key(fmtp: Option<&str>) -> String {
    fmtp.and_then(|f| fmtp_param(f, "sprop-stereo")).unwrap_or("0").to_string()
}

pub fn same_as_room(kind: Kind, room_codec: &str, codec: Option<&str>, fmtp: Option<&str>) -> bool {
    let Some(rc) = canon(room_codec) else { return false };
    let got = match (kind, codec) {
        (Kind::Audio, None) => return rc == AUDIO,
        (_, Some(c)) => canon(c),
        (Kind::Video, None) => None,
    };
    if got != Some(rc) {
        return false;
    }
    match rc {
        "H264" => h264_key(fmtp) == h264_key(Some(H264_ROOM_FMTP)),
        _ => true,
    }
}

pub fn tuple_key(kind: Kind, codec: &str, fmtp: Option<&str>) -> Option<String> {
    let c = canon(codec)?;
    let k = match kind {
        Kind::Audio => "audio",
        Kind::Video => "video",
    };
    Some(match c {
        "H264" => {
            let (pm, prof) = h264_key(fmtp);
            format!("{k}/H264/90000/pm={pm}/prof={prof}")
        }
        "opus" => format!("{k}/opus/48000/2/stereo={}", opus_key(fmtp)),
        other => format!("{k}/{other}/90000"),
    })
}

pub fn is_keyframe(codec: &str, payload: &[u8]) -> bool {
    match canon(codec) {
        Some("VP8") => vp8_keyframe(payload),
        Some("H264") => h264_keyframe(payload),
        _ => false,
    }
}

pub fn vp8_keyframe(p: &[u8]) -> bool {
    if p.is_empty() {
        return false;
    }
    let b0 = p[0];
    let x = b0 & 0x80 != 0;
    let s = b0 & 0x10 != 0;
    let pid = b0 & 0x07;
    if !s || pid != 0 {
        return false;
    }
    let mut i = 1;
    if x {
        let Some(&b1) = p.get(i) else { return false };
        i += 1;
        if b1 & 0x80 != 0 {
            let Some(&pic) = p.get(i) else { return false };
            i += if pic & 0x80 != 0 { 2 } else { 1 };
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

fn nal_key(t: u8) -> bool {
    t == 5 || t == 7
}

pub fn h264_keyframe(p: &[u8]) -> bool {
    let Some(&b0) = p.first() else { return false };
    match b0 & 0x1f {
        24 => {
            let mut i = 1;
            while i + 2 < p.len() {
                let size = u16::from_be_bytes([p[i], p[i + 1]]) as usize;
                if let Some(&nal) = p.get(i + 2)
                    && nal_key(nal & 0x1f)
                {
                    return true;
                }
                i += 2 + size;
            }
            false
        }
        28 => p.get(1).is_some_and(|fu| fu & 0x80 != 0 && nal_key(fu & 0x1f)),
        t => nal_key(t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h264_compares_params_not_string() {
        let chrome = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f";
        assert!(same_as_room(Kind::Video, "H264", Some("H264"), Some(chrome)));
        assert!(same_as_room(Kind::Video, "H264", Some("h264"), Some("packetization-mode=1;profile-level-id=42e034")));
        assert!(!same_as_room(Kind::Video, "H264", Some("H264"), Some("packetization-mode=0;profile-level-id=42e01f")));
        assert!(!same_as_room(Kind::Video, "H264", Some("H264"), Some("packetization-mode=1;profile-level-id=640c1f")));
        assert!(!same_as_room(Kind::Video, "H264", Some("VP8"), None));
        assert!(same_as_room(Kind::Audio, "opus", None, None));
    }

    #[test]
    fn keyframes() {
        assert!(vp8_keyframe(&[0x10, 0x00]));
        assert!(!vp8_keyframe(&[0x10, 0x01]));
        assert!(!vp8_keyframe(&[0x00, 0x00]));
        assert!(vp8_keyframe(&[0x90, 0x80, 0x81, 0x02, 0x00]));
        assert!(h264_keyframe(&[0x65]));
        assert!(h264_keyframe(&[0x67]));
        assert!(!h264_keyframe(&[0x41]));
        assert!(h264_keyframe(&[0x18, 0x00, 0x02, 0x67, 0x00, 0x00, 0x01, 0x68]));
        assert!(h264_keyframe(&[0x7c, 0x85]));
        assert!(!h264_keyframe(&[0x7c, 0x05]));
    }

    #[test]
    fn supported_set_is_registry() {
        assert_eq!(supported_video("vp8"), Some("VP8"));
        assert_eq!(supported_video("VP9"), None);
    }
}
