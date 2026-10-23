// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-PUB-ADD · 정:IMPL-PUB-ACCEPT · 연:RULE-PUB-PUBLISH-PROC · 연:RULE-ROOM-MEDIA-CODECS · 연:RULE-ROOM-PERMISSION-MAP · 연:PUB.op.PUBLISH_TRACKS · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 발행 검사(순수) — 자격 → 수 → 필수 칸 → `1pc` SSRC 충돌 → 코덱, 앞에서 걸리면 뒤는 없다 · 부분 수용 없음.

use std::collections::BTreeSet;

use oxsig::err::{ErrCode, Failure};
use oxsig::r#pub::{Duplex, ExtIds, PublishTrack, TRACKS_PER_REQUEST, TRACKS_PER_USER};
use oxsig::room::{MediaCodecs, Permission};
use oxsig::ses::PcMode;
use oxsig::trk::{MediaKind, Source};
use serde_json::{json, Value};

pub const SUPPORTED_VIDEO: [&str; 2] = ["VP8", "H264"];

pub struct AddCtx<'a> {
    pub perm: Permission,
    pub active: usize,
    pub pc_mode: PcMode,
    pub downlink: &'a BTreeSet<u32>,
    pub codecs: &'a MediaCodecs,
    pub at_pub: bool,
    pub ext: &'a ExtIds,
}

fn bit_of(kind: MediaKind, source: Option<Source>) -> &'static str {
    match (kind, source) {
        (MediaKind::Audio, _) => "publish_audio",
        (MediaKind::Video, Some(Source::Screen)) => "publish_screen",
        (MediaKind::Video, _) => "publish_video",
    }
}

pub fn allowed(perm: &Permission, kind: MediaKind, source: Option<Source>) -> bool {
    match bit_of(kind, source) {
        "publish_audio" => perm.publish_audio,
        "publish_screen" => perm.publish_screen,
        _ => perm.publish_video,
    }
}

fn param<'a>(fmtp: &'a str, key: &str) -> Option<&'a str> {
    fmtp.split(';').map(str::trim).find_map(|kv| kv.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

pub fn codec_matches(kind: MediaKind, name: &str, fmtp: Option<&str>, room: &MediaCodecs) -> bool {
    let want = if kind == MediaKind::Audio { room.audio() } else { room.video() };
    if !name.eq_ignore_ascii_case(want) {
        return false;
    }
    if !name.eq_ignore_ascii_case("h264") {
        return true;
    }
    let f = fmtp.unwrap_or("");
    let pm = param(f, "packetization-mode").unwrap_or("0");
    let prof = param(f, "profile-level-id").map(|p| p.get(..4).unwrap_or(p).to_ascii_lowercase());
    pm == "1" && prof.as_deref() == Some("42e0")
}

pub fn room_fmtp(room: &MediaCodecs, kind: MediaKind) -> Option<&'static str> {
    let name = if kind == MediaKind::Audio { room.audio() } else { room.video() };
    match name.to_ascii_lowercase().as_str() {
        "h264" => Some(crate::link::table::FMTP_H264),
        "opus" => Some(crate::link::table::FMTP_OPUS),
        _ => None,
    }
}

fn shape(v: &Value) -> Result<PublishTrack, Failure> {
    for k in ["pt", "ssrc", "mid"] {
        if v.get(k).map(Value::is_null).unwrap_or(true) {
            return Err(Failure::new(ErrCode::MissingField).msg(format!("{k} 미신고")));
        }
    }
    let t: PublishTrack = serde_json::from_value(v.clone()).map_err(|e| Failure::new(ErrCode::InvalidPayload).msg(e.to_string()))?;
    if t.kind == MediaKind::Audio && t.source.is_some() {
        return Err(Failure::new(ErrCode::InvalidPayload).msg("audio 에 source"));
    }
    if t.kind == MediaKind::Audio && t.simulcast == Some(true) {
        return Err(Failure::new(ErrCode::InvalidPayload).msg("audio 시뮬캐스트"));
    }
    Ok(t)
}

pub fn check_add(raw: &[Value], c: &AddCtx<'_>) -> Result<Vec<PublishTrack>, Failure> {
    let mut items = Vec::with_capacity(raw.len());
    for v in raw {
        items.push(shape(v)?);
    }
    let denied: BTreeSet<&str> = items.iter().filter(|t| !allowed(&c.perm, t.kind, t.source)).map(|t| bit_of(t.kind, t.source)).collect();
    if !denied.is_empty() {
        return Err(Failure::new(ErrCode::NotAuthorized).details(json!({"denied": denied})));
    }
    if !c.at_pub && items.iter().any(|t| t.duplex.unwrap_or(Duplex::Full) == Duplex::Full) {
        return Err(Failure::new(ErrCode::NotPubRoom));
    }
    if items.len() > TRACKS_PER_REQUEST || c.active + items.len() > TRACKS_PER_USER {
        return Err(Failure::new(ErrCode::TrackLimit).details(json!({"limit": TRACKS_PER_USER, "current": c.active})));
    }
    for t in &items {
        let sim = t.simulcast.unwrap_or(false);
        if sim == (t.ssrc != 0) {
            return Err(Failure::new(ErrCode::MissingField).msg("simulcast · ssrc 불일치"));
        }
        if sim && c.ext.rid.is_none() {
            return Err(Failure::new(ErrCode::MissingField).msg("rid_extmap_id 미신고"));
        }
        if sim && t.duplex == Some(Duplex::Half) {
            return Err(Failure::new(ErrCode::TrackOpUnsupported).msg("반이중 시뮬캐스트"));
        }
    }
    if c.pc_mode == PcMode::One {
        for t in &items {
            for s in [Some(t.ssrc), t.rtx_ssrc].into_iter().flatten().filter(|s| *s != 0) {
                if c.downlink.contains(&s) {
                    return Err(Failure::new(ErrCode::SsrcCollision).details(json!({"ssrc": s})));
                }
            }
        }
    }
    for t in &items {
        let name = match (&t.codec, t.kind) {
            (Some(n), _) => n.clone(),
            (None, MediaKind::Audio) => c.codecs.audio().to_string(),
            (None, MediaKind::Video) => return Err(Failure::new(ErrCode::CodecRequired).details(json!({"supported": SUPPORTED_VIDEO}))),
        };
        if t.kind == MediaKind::Video && !SUPPORTED_VIDEO.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
            return Err(Failure::new(ErrCode::CodecRequired).details(json!({"supported": SUPPORTED_VIDEO})));
        }
        if !codec_matches(t.kind, &name, t.fmtp.as_deref(), c.codecs) {
            let codec = if t.kind == MediaKind::Audio { c.codecs.audio() } else { c.codecs.video() };
            return Err(Failure::new(ErrCode::CodecMismatch).details(json!({"codec": codec, "fmtp": room_fmtp(c.codecs, t.kind)})));
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(perm: Permission, d: &'a BTreeSet<u32>, mc: &'a MediaCodecs, ext: &'a ExtIds) -> AddCtx<'a> {
        AddCtx { perm, active: 0, pc_mode: PcMode::One, downlink: d, codecs: mc, at_pub: true, ext }
    }

    #[test]
    fn permission_before_codec() {
        let d = BTreeSet::new();
        let mc = MediaCodecs::default().filled();
        let ext = ExtIds::default();
        let perm = Permission { publish_video: false, ..Permission::default() };
        let r = check_add(&[json!({"kind":"video","ssrc":5,"mid":"1","pt":96,"codec":"AV1"})], &ctx(perm, &d, &mc, &ext));
        assert_eq!(r.map_err(|f| f.code), Err(2006), "정:IMPL-PUB-ADD ①");
    }

    #[test]
    fn codec_rules() {
        let d: BTreeSet<u32> = [7].into_iter().collect();
        let mc = MediaCodecs::default().filled();
        let ext = ExtIds::default();
        let c = ctx(Permission::default(), &d, &mc, &ext);
        let bad = check_add(&[json!({"kind":"video","ssrc":5,"mid":"1","pt":96,"codec":"VP8"})], &c);
        assert_eq!(bad.map_err(|f| f.code), Err(1006));
        let ok = check_add(&[json!({"kind":"video","ssrc":5,"mid":"1","pt":102,"codec":"H264","fmtp":"level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"})], &c);
        assert!(ok.is_ok());
        let col = check_add(&[json!({"kind":"audio","ssrc":7,"mid":"0","pt":111})], &c);
        assert_eq!(col.map_err(|f| f.code), Err(3008));
        let miss = check_add(&[json!({"kind":"audio","ssrc":8,"mid":"0"})], &c);
        assert_eq!(miss.map_err(|f| f.code), Err(1003));
    }
}
