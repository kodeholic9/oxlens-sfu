// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-PUB-TWO-LAYERS 정:RULE-PUB-STATE 정:IMPL-PUB-ADD 정:IMPL-PUB-ACCEPT 정:IMPL-PUB-REMOVE 정:IMPL-PUB-TRACK-SET 정:CAND-PUB-NOT-PUB-ROOM-ORDER 정:IMPL-PUB-RID-SPATIAL 정:CAND-PUB-RID-EXTRA 연:RULE-PUB-PUBLISH-PROC 연:RULE-PUB-REMOVE-FORM 연:RULE-ROOM-PERMISSION-MAP 연:RULE-ROOM-SOURCE-TRUSTED 연:PUB.S.add_2006 연:PUB.S.add_3011 연:PUB.S.add_4002 · model: claude-opus-5-5
//! 발행 — 논리(`track_id` · 신고 메타) 하나에 물리(업링크 스트림) 여럿 · `PUBLISH_TRACKS{add}` 검사 차례 ①자격 ①′발행 방 ②수 ③칸 ④SSRC ⑤코덱.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use oxsig::code::{Code, Failure};
use oxsig::msg::{Duplex, Kind, MediaCodecs, Permission, Publication, PublishTrack, Source};
use serde_json::{Value, json};

use crate::trk::codec;

pub const MAX_PER_REQ: usize = 8;
pub const MAX_ACTIVE: usize = 16;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExtIds {
    pub twcc: Option<u8>,
    pub rid: Option<u8>,
    pub rrid: Option<u8>,
    pub mid: Option<u8>,
    pub audio_level: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Created,
    Intended,
    Active,
}

pub struct Stream {
    pub track_id: String,
    pub user: String,
    pub room_id: String,
    pub kind: Kind,
    pub mid: String,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub codec: Option<String>,
    pub fmtp: Option<String>,
    pub source: Option<Source>,
    pub simulcast: bool,
    pub ext: ExtIds,
    duplex: AtomicU8,
    muted: AtomicBool,
    state: AtomicU8,
    pub first_rtp_ms: AtomicU64,
    pub last_loud_ms: AtomicU64,
    pub pt_checked: AtomicBool,
    pub listed: AtomicBool,
}

impl Stream {
    pub fn new(user: &str, room_id: &str, t: &PublishTrack, ext: ExtIds, track_id: String) -> Stream {
        let d = t.duplex.unwrap_or(Duplex::Full);
        let simulcast = t.simulcast.unwrap_or(false) && d == Duplex::Full;
        Stream {
            track_id,
            user: user.to_string(),
            room_id: room_id.to_string(),
            kind: t.kind,
            mid: t.mid.clone(),
            pt: t.pt,
            rtx_pt: t.rtx_pt,
            ssrc: t.ssrc,
            rtx_ssrc: t.rtx_ssrc,
            codec: t.codec.as_deref().and_then(codec::canon).map(str::to_string),
            fmtp: t.fmtp.clone(),
            source: if t.kind == Kind::Video { Some(t.source.unwrap_or(Source::Camera)) } else { None },
            simulcast,
            ext,
            duplex: AtomicU8::new(if d == Duplex::Half { 1 } else { 0 }),
            muted: AtomicBool::new(false),
            state: AtomicU8::new(1),
            first_rtp_ms: AtomicU64::new(0),
            last_loud_ms: AtomicU64::new(0),
            pt_checked: AtomicBool::new(false),
            listed: AtomicBool::new(false),
        }
    }

    pub fn duplex(&self) -> Duplex {
        if self.duplex.load(Ordering::Acquire) == 1 { Duplex::Half } else { Duplex::Full }
    }

    pub fn swap_duplex(&self, d: Duplex) -> Duplex {
        let prev = self.duplex.swap(if d == Duplex::Half { 1 } else { 0 }, Ordering::AcqRel);
        if prev == 1 { Duplex::Half } else { Duplex::Full }
    }

    pub fn half(&self) -> bool {
        self.duplex() == Duplex::Half
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Acquire)
    }

    pub fn swap_muted(&self, m: bool) -> bool {
        self.muted.swap(m, Ordering::AcqRel)
    }

    pub fn state(&self) -> State {
        match self.state.load(Ordering::Acquire) {
            0 => State::Created,
            1 => State::Intended,
            _ => State::Active,
        }
    }

    pub fn first_rtp(&self, now: u64) -> bool {
        self.state.store(2, Ordering::Release);
        self.first_rtp_ms.compare_exchange(0, now.max(1), Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    pub fn codec_name(&self) -> &str {
        self.codec.as_deref().unwrap_or(match self.kind {
            Kind::Audio => codec::AUDIO,
            Kind::Video => "",
        })
    }

    pub fn publication(&self) -> Publication {
        Publication {
            track_id: self.track_id.clone(),
            room_id: self.room_id.clone(),
            kind: self.kind,
            duplex: self.duplex(),
            codec: if self.kind == Kind::Video { self.codec.clone() } else { None },
            fmtp: self.fmtp.clone(),
            simulcast: self.simulcast,
        }
    }
}

#[derive(Default)]
pub struct Ctx {
    pub streams: Vec<Arc<Stream>>,
}

impl Ctx {
    pub fn by_id(&self, track_id: &str) -> Option<Arc<Stream>> {
        self.streams.iter().find(|s| s.track_id == track_id).cloned()
    }

    pub fn by_ssrc(&self, ssrc: u32) -> Option<Arc<Stream>> {
        self.streams.iter().find(|s| s.ssrc == ssrc && ssrc != 0).cloned()
    }

    pub fn remove(&mut self, track_id: &str) -> Option<Arc<Stream>> {
        let i = self.streams.iter().position(|s| s.track_id == track_id)?;
        Some(self.streams.remove(i))
    }

    pub fn half_kinds(&self) -> Vec<Kind> {
        let mut v: Vec<Kind> = self.streams.iter().filter(|s| s.half()).map(|s| s.kind).collect();
        v.dedup();
        v
    }

    pub fn has_half(&self) -> bool {
        self.streams.iter().any(|s| s.half())
    }

    pub fn uplink_ssrcs(&self) -> HashSet<u32> {
        let mut h = HashSet::new();
        for s in &self.streams {
            if s.ssrc != 0 {
                h.insert(s.ssrc);
            }
            if let Some(r) = s.rtx_ssrc {
                h.insert(r);
            }
        }
        h
    }
}

pub fn bit_of(kind: Kind, source: Option<Source>) -> &'static str {
    match (kind, source) {
        (Kind::Audio, _) => "publish_audio",
        (Kind::Video, Some(Source::Screen)) => "publish_screen",
        (Kind::Video, _) => "publish_video",
    }
}

pub fn allowed(p: &Permission, bit: &str) -> bool {
    match bit {
        "publish_audio" => p.publish_audio,
        "publish_video" => p.publish_video,
        "publish_screen" => p.publish_screen,
        "floor_request" => p.floor_request,
        _ => false,
    }
}

fn kind_of(v: &Value) -> Result<Kind, Failure> {
    match v.get("kind") {
        None | Some(Value::Null) => Err(Code::MissingField.failure_msg("kind")),
        Some(k) => serde_json::from_value(k.clone()).map_err(|_| Code::InvalidPayload.failure_msg("kind")),
    }
}

fn source_of(v: &Value, kind: Kind) -> Result<Option<Source>, Failure> {
    match v.get("source") {
        None | Some(Value::Null) => Ok(None),
        Some(s) => {
            if kind == Kind::Audio {
                return Err(Code::InvalidPayload.failure_msg("audio 항목의 source"));
            }
            serde_json::from_value::<Source>(s.clone()).map(Some).map_err(|_| Code::InvalidPayload.failure_msg("모르는 source"))
        }
    }
}

fn duplex_of(v: &Value) -> Result<Duplex, Failure> {
    match v.get("duplex") {
        None | Some(Value::Null) => Ok(Duplex::Full),
        Some(d) => serde_json::from_value(d.clone()).map_err(|_| Code::InvalidPayload.failure_msg("duplex")),
    }
}

pub struct AddInput<'a> {
    pub items: &'a [Value],
    pub perm: &'a Permission,
    pub is_pub_room: bool,
    pub active: usize,
    pub ext: ExtIds,
    pub room_codecs: &'a MediaCodecs,
    pub downlink_ssrcs: Option<&'a HashSet<u32>>,
}

pub fn check_add(i: &AddInput<'_>) -> Result<Vec<PublishTrack>, Failure> {
    let mut denied: Vec<&'static str> = Vec::new();
    let mut duplexes = Vec::with_capacity(i.items.len());
    for v in i.items {
        let kind = kind_of(v)?;
        let src = source_of(v, kind)?;
        let bit = bit_of(kind, src);
        if !allowed(i.perm, bit) && !denied.contains(&bit) {
            denied.push(bit);
        }
        duplexes.push(duplex_of(v)?);
    }
    if !denied.is_empty() {
        return Err(Code::NotAuthorized.failure().with_details(json!({ "denied": denied })));
    }
    if duplexes.contains(&Duplex::Full) && !i.is_pub_room {
        return Err(Code::NotPubRoom.failure());
    }
    if i.items.len() > MAX_PER_REQ {
        return Err(Code::TrackLimit.failure().with_details(json!({ "limit": MAX_PER_REQ, "current": i.items.len() })));
    }
    if i.active + i.items.len() > MAX_ACTIVE {
        return Err(Code::TrackLimit.failure().with_details(json!({ "limit": MAX_ACTIVE, "current": i.active })));
    }
    let mut tracks = Vec::with_capacity(i.items.len());
    for v in i.items {
        for f in ["ssrc", "mid", "pt"] {
            if v.get(f).is_none_or(Value::is_null) {
                return Err(Code::MissingField.failure_msg(format!("tracks[].{f}")));
            }
        }
        let t: PublishTrack = serde_json::from_value(v.clone()).map_err(|e| Code::InvalidPayload.failure_msg(e.to_string()))?;
        let sim = t.simulcast.unwrap_or(false);
        if sim && t.kind == Kind::Audio {
            return Err(Code::InvalidPayload.failure_msg("audio 시뮬캐스트"));
        }
        if sim != (t.ssrc == 0) {
            return Err(Code::MissingField.failure_msg("simulcast 와 ssrc 가 어긋난다"));
        }
        if sim && i.ext.rid.is_none() {
            return Err(Code::MissingField.failure_msg("rid_extmap_id"));
        }
        tracks.push(t);
    }
    if let Some(down) = i.downlink_ssrcs {
        for t in &tracks {
            for s in [Some(t.ssrc), t.rtx_ssrc].into_iter().flatten() {
                if s != 0 && down.contains(&s) {
                    return Err(Code::SsrcCollision.failure().with_details(json!({ "ssrc": s })));
                }
            }
        }
    }
    for t in &tracks {
        let room = match t.kind {
            Kind::Audio => i.room_codecs.audio.as_deref().unwrap_or(codec::AUDIO),
            Kind::Video => i.room_codecs.video.as_deref().unwrap_or("H264"),
        };
        if t.kind == Kind::Video && t.codec.as_deref().and_then(codec::supported_video).is_none() {
            return Err(Code::CodecRequired.failure().with_details(json!({ "supported": codec::SUPPORTED_VIDEO })));
        }
        if !codec::same_as_room(t.kind, room, t.codec.as_deref(), t.fmtp.as_deref()) {
            return Err(Code::CodecMismatch.failure().with_details(json!({ "codec": room, "fmtp": codec::row_fmtp(room) })));
        }
    }
    Ok(tracks)
}

pub fn rid_layer(rid: &str) -> Option<u8> {
    match rid {
        "l" => Some(0),
        "h" => Some(1),
        _ => None,
    }
}

pub const SCALABILITY: &str = "L2T1";

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Permission {
        Permission { publish_audio: true, publish_video: true, publish_screen: true, floor_request: true }
    }

    fn codecs() -> MediaCodecs {
        MediaCodecs { audio: Some("opus".into()), video: Some("H264".into()) }
    }

    #[test]
    fn order_permission_before_fields() {
        let mut p = all();
        p.publish_audio = false;
        let items = vec![json!({"kind": "audio"})];
        let c = codecs();
        let i = AddInput { items: &items, perm: &p, is_pub_room: true, active: 0, ext: ExtIds::default(), room_codecs: &c, downlink_ssrcs: None };
        let e = check_add(&i).expect_err("deny");
        assert_eq!(e.code, 2006, "S-sym-IMPL-PUB-ADD-3 정:IMPL-PUB-ADD ① 자격이 ③ 필수 칸보다 먼저");
        assert_eq!(e.details, Some(json!({"denied": ["publish_audio"]})));
    }

    #[test]
    fn not_pub_room_after_perm_before_count() {
        let items: Vec<Value> = (0..9).map(|_| json!({"kind": "audio"})).collect();
        let c = codecs();
        let p = all();
        let i = AddInput { items: &items, perm: &p, is_pub_room: false, active: 0, ext: ExtIds::default(), room_codecs: &c, downlink_ssrcs: None };
        assert_eq!(check_add(&i).expect_err("3011").code, 3011);
        let i2 = AddInput { is_pub_room: true, ..i };
        assert_eq!(check_add(&i2).expect_err("4002").code, 4002);
    }

    #[test]
    fn active_count_blocks_repeat_calls() {
        let items = vec![json!({"kind": "audio", "ssrc": 1, "mid": "0", "pt": 111})];
        let c = codecs();
        let p = all();
        let i = AddInput { items: &items, perm: &p, is_pub_room: true, active: 16, ext: ExtIds::default(), room_codecs: &c, downlink_ssrcs: None };
        assert_eq!(check_add(&i).expect_err("limit").code, 4002);
    }

    #[test]
    fn codec_checks() {
        let c = codecs();
        let p = all();
        let vp9 = vec![json!({"kind": "video", "ssrc": 1, "mid": "1", "pt": 98, "codec": "VP9"})];
        let i = AddInput { items: &vp9, perm: &p, is_pub_room: true, active: 0, ext: ExtIds::default(), room_codecs: &c, downlink_ssrcs: None };
        assert_eq!(check_add(&i).expect_err("1005").code, 1005);
        let vp8 = vec![json!({"kind": "video", "ssrc": 1, "mid": "1", "pt": 96, "codec": "VP8"})];
        let i = AddInput { items: &vp8, ..i };
        let e = check_add(&i).expect_err("1006");
        assert_eq!(e.code, 1006);
        assert_eq!(e.details.as_ref().and_then(|d| d.get("codec")).and_then(Value::as_str), Some("H264"));
    }

    #[test]
    fn rid_only_l_h() {
        assert_eq!(rid_layer("l"), Some(0));
        assert_eq!(rid_layer("h"), Some(1));
        assert_eq!(rid_layer("q"), None, "정:CAND-PUB-RID-EXTRA");
    }
}
