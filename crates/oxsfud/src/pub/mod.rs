// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-PUB-SERVER-SCOPE · 정:RULE-PUB-TWO-LAYERS · 정:RULE-PUB-STATE · 정:IMPL-PUB-ADD · 정:IMPL-PUB-REMOVE · 정:IMPL-PUB-ACCEPT · 정:IMPL-PUB-TRACK-SET · 정:IMPL-PUB-RID-SPATIAL · 정:CAND-PUB-H264-PROFILE · 정:CAND-PUB-NOT-PUB-ROOM-ORDER · 정:CAND-PUB-RID-EXTRA · 정:CAND-PUB-INGRESS-STATS-OWNER · 연:PUB.op.PUBLISH_TRACKS · 연:RULE-PUB-PUBLISH-PROC · 연:RULE-PUB-SET-PROC · 연:RULE-PUB-RID-TO-SPATIAL · 연:RULE-ROOM-PERMISSION-MAP · 연:PUB.S · model: claude-opus-5-5
//! 발행 — 논리 `Stream`(트랙 하나 · 식별 · 메타 · 구독자 목록) · 물리 `Uplink`(업링크 스트림 하나 · 캐시 · NACK · 수신 통계). 검사 차례(자격 → 발행 방 → 수 → 필수 칸 → SSRC → 코덱)는 순수 함수다.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use arc_swap::ArcSwap;
use oxsig::code::{Code, Failure};
use oxsig::types::{Duplex, Kind, Permission, PublishTrack, Source};

use crate::fwd::hot::{Downlink, Ingress};
use crate::rtcp::core::NackUp;

pub const REQ_MAX: usize = 8;
pub const ACTIVE_MAX: usize = 16;
pub const CACHE: usize = 1_024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IngressExt {
    pub mid: Option<u8>,
    pub rid: Option<u8>,
    pub repaired_rid: Option<u8>,
    pub audio_level: Option<u8>,
    pub twcc: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishState {
    Created,
    Intended,
    Active,
}

impl PublishState {
    pub fn as_str(self) -> &'static str {
        match self {
            PublishState::Created => "Created",
            PublishState::Intended => "Intended",
            PublishState::Active => "Active",
        }
    }
}

pub struct RtpCache {
    slots: Vec<Option<(u16, Vec<u8>)>>,
}

impl Default for RtpCache {
    fn default() -> RtpCache {
        RtpCache { slots: vec![None; CACHE] }
    }
}

impl RtpCache {
    pub fn put(&mut self, seq: u16, pkt: &[u8]) {
        self.slots[usize::from(seq) % CACHE] = Some((seq, pkt.to_vec()));
    }

    pub fn get(&self, seq: u16) -> Option<&[u8]> {
        match &self.slots[usize::from(seq) % CACHE] {
            Some((s, p)) if *s == seq => Some(p),
            _ => None,
        }
    }
}

pub struct Uplink {
    pub ssrc: u32,
    pub rtx: bool,
    pub layer: Option<u8>,
    pub kind: Kind,
    pub clock: u32,
    pub stream: Weak<Stream>,
    pub cache: Mutex<RtpCache>,
    pub nack: Mutex<NackUp>,
    pub ingress: Mutex<Ingress>,
    pub pt_checked: AtomicBool,
    pub last_pli_ms: AtomicU64,
    pub first_rtp_ms: AtomicU64,
    pub nacks_sent: AtomicU64,
    pub plis_sent: AtomicU64,
}

impl Uplink {
    pub fn new(ssrc: u32, rtx: bool, layer: Option<u8>, kind: Kind, stream: &Arc<Stream>) -> Uplink {
        let clock = crate::trk::clock(kind);
        Uplink {
            ssrc,
            rtx,
            layer,
            kind,
            clock,
            stream: Arc::downgrade(stream),
            cache: Mutex::new(RtpCache::default()),
            nack: Mutex::new(NackUp::new()),
            ingress: Mutex::new(Ingress::new(clock)),
            pt_checked: AtomicBool::new(false),
            last_pli_ms: AtomicU64::new(0),
            first_rtp_ms: AtomicU64::new(0),
            nacks_sent: AtomicU64::new(0),
            plis_sent: AtomicU64::new(0),
        }
    }
}

pub struct Stream {
    pub track_id: Arc<str>,
    pub user: Arc<str>,
    pub kind: Kind,
    pub mid: Arc<str>,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub codec: Arc<str>,
    pub fmtp: Option<String>,
    pub source: Option<Source>,
    pub simulcast: bool,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    duplex: AtomicU8,
    pub muted: AtomicBool,
    pub aff: Arc<crate::aff::Hot>,
    pub subs: ArcSwap<Vec<Weak<Downlink>>>,
    pub uplinks: ArcSwap<Vec<Arc<Uplink>>>,
    pub listed: AtomicBool,
    pub last_voice_ms: AtomicU64,
    state: AtomicU8,
    pub transport: Weak<crate::link::Transport>,
}

impl Stream {
    pub fn new(track_id: &str, user: &str, t: &PublishTrack, codec: &str, aff: Arc<crate::aff::Hot>, transport: Weak<crate::link::Transport>) -> Stream {
        let duplex = t.duplex.unwrap_or(Duplex::Full);
        Stream {
            track_id: Arc::from(track_id),
            user: Arc::from(user),
            kind: t.kind,
            mid: Arc::from(t.mid.as_str()),
            pt: t.pt,
            rtx_pt: t.rtx_pt,
            codec: Arc::from(codec),
            fmtp: t.fmtp.clone(),
            source: t.source,
            simulcast: t.simulcast.unwrap_or(false) && duplex == Duplex::Full,
            ssrc: t.ssrc,
            rtx_ssrc: t.rtx_ssrc,
            duplex: AtomicU8::new(if duplex == Duplex::Half { 1 } else { 0 }),
            muted: AtomicBool::new(false),
            aff,
            subs: ArcSwap::from_pointee(Vec::new()),
            uplinks: ArcSwap::from_pointee(Vec::new()),
            listed: AtomicBool::new(false),
            last_voice_ms: AtomicU64::new(0),
            state: AtomicU8::new(0),
            transport,
        }
    }

    pub fn duplex(&self) -> Duplex {
        if self.duplex.load(Ordering::Acquire) == 1 { Duplex::Half } else { Duplex::Full }
    }

    pub fn set_duplex(&self, d: Duplex) {
        self.duplex.store(if d == Duplex::Half { 1 } else { 0 }, Ordering::Release);
    }

    pub fn state(&self) -> PublishState {
        match self.state.load(Ordering::Acquire) {
            0 => PublishState::Created,
            1 => PublishState::Intended,
            _ => PublishState::Active,
        }
    }

    pub fn swap_state(&self, s: PublishState) -> PublishState {
        let v = match s {
            PublishState::Created => 0,
            PublishState::Intended => 1,
            PublishState::Active => 2,
        };
        match self.state.swap(v, Ordering::AcqRel) {
            0 => PublishState::Created,
            1 => PublishState::Intended,
            _ => PublishState::Active,
        }
    }

    pub fn attach(&self, d: &Arc<Downlink>) {
        let mut v: Vec<Weak<Downlink>> = self.subs.load().iter().filter(|w| w.strong_count() > 0).cloned().collect();
        if !v.iter().any(|w| w.as_ptr() == Arc::as_ptr(d)) {
            v.push(Arc::downgrade(d));
        }
        self.subs.store(Arc::new(v));
    }

    pub fn detach(&self, d: &Arc<Downlink>) {
        let v: Vec<Weak<Downlink>> = self.subs.load().iter().filter(|w| w.strong_count() > 0 && w.as_ptr() != Arc::as_ptr(d)).cloned().collect();
        self.subs.store(Arc::new(v));
    }

    pub fn add_uplink(&self, u: Arc<Uplink>) {
        let mut v: Vec<Arc<Uplink>> = self.uplinks.load().iter().cloned().collect();
        v.push(u);
        self.uplinks.store(Arc::new(v));
    }

    pub fn uplink_of_layer(&self, layer: Option<u8>) -> Option<Arc<Uplink>> {
        self.uplinks.load().iter().find(|u| !u.rtx && u.layer == layer).cloned()
    }

    pub fn primary(&self) -> Option<Arc<Uplink>> {
        let ups = self.uplinks.load();
        ups.iter().filter(|u| !u.rtx).max_by_key(|u| u.layer.unwrap_or(0)).cloned()
    }

    pub fn publication(&self, room: &str) -> oxsig::types::Publication {
        oxsig::types::Publication {
            track_id: self.track_id.to_string(),
            room_id: room.to_string(),
            kind: self.kind,
            duplex: self.duplex(),
            codec: Some(self.codec.to_string()),
            fmtp: self.fmtp.clone(),
            simulcast: self.simulcast,
        }
    }
}

pub fn rid_layer(rid: &[u8]) -> Option<u8> {
    match rid {
        b"l" => Some(0),
        b"h" => Some(1),
        _ => None,
    }
}

pub fn perm_bit(kind: Kind, source: Option<Source>) -> &'static str {
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
        _ => p.floor_request,
    }
}

#[derive(Debug, Clone)]
pub struct AddCtx<'a> {
    pub room: &'a str,
    pub pub_room: Option<&'a str>,
    pub perm: Permission,
    pub active: usize,
    pub downlink_ssrcs: &'a HashSet<u32>,
    pub one_pc: bool,
    pub room_audio: &'a str,
    pub room_video: &'a str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct TrackIn {
    #[serde(default)]
    pub kind: Option<Kind>,
    #[serde(default)]
    pub ssrc: Option<u32>,
    #[serde(default)]
    pub mid: Option<String>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub fmtp: Option<String>,
    #[serde(default)]
    pub pt: Option<u8>,
    #[serde(default)]
    pub rtx_pt: Option<u8>,
    #[serde(default)]
    pub rtx_ssrc: Option<u32>,
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default)]
    pub duplex: Option<Duplex>,
    #[serde(default)]
    pub simulcast: Option<bool>,
}

impl From<&PublishTrack> for TrackIn {
    fn from(t: &PublishTrack) -> TrackIn {
        TrackIn {
            kind: Some(t.kind),
            ssrc: Some(t.ssrc),
            mid: Some(t.mid.clone()),
            codec: t.codec.clone(),
            fmtp: t.fmtp.clone(),
            pt: Some(t.pt),
            rtx_pt: t.rtx_pt,
            rtx_ssrc: t.rtx_ssrc,
            source: t.source,
            duplex: t.duplex,
            simulcast: t.simulcast,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct AddIn {
    pub room_id: String,
    pub tracks: Vec<TrackIn>,
    #[serde(default)]
    pub twcc_extmap_id: Option<u8>,
    #[serde(default)]
    pub rid_extmap_id: Option<u8>,
    #[serde(default)]
    pub repair_rid_extmap_id: Option<u8>,
    #[serde(default)]
    pub mid_extmap_id: Option<u8>,
    #[serde(default)]
    pub audio_level_extmap_id: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(untagged)]
pub enum PublishIn {
    Remove { action: oxsig::msg::PublishTracksReqRemoveAction, room_id: String, track_ids: Vec<String> },
    Add(AddIn),
}

impl PublishIn {
    pub fn room_id(&self) -> &str {
        match self {
            PublishIn::Remove { room_id, .. } => room_id,
            PublishIn::Add(a) => &a.room_id,
        }
    }
}

fn fill(t: &TrackIn) -> Result<PublishTrack, Failure> {
    let missing = || Failure::of(Code::MISSING_FIELD);
    let kind = t.kind.ok_or_else(missing)?;
    let mid = t.mid.clone().filter(|m| !m.is_empty()).ok_or_else(missing)?;
    let pt = t.pt.ok_or_else(missing)?;
    let ssrc = t.ssrc.ok_or_else(missing)?;
    let sim = t.simulcast.unwrap_or(false);
    if kind == Kind::Audio && (t.source.is_some() || sim) {
        return Err(Failure::of(Code::INVALID_PAYLOAD));
    }
    if sim != (ssrc == 0) {
        return Err(missing());
    }
    Ok(PublishTrack { kind, ssrc, mid, codec: t.codec.clone(), fmtp: t.fmtp.clone(), pt, rtx_pt: t.rtx_pt, rtx_ssrc: t.rtx_ssrc, source: t.source, duplex: t.duplex, simulcast: t.simulcast })
}

pub fn check_add(c: &AddCtx<'_>, tracks: &[PublishTrack]) -> Result<Vec<String>, Failure> {
    let items: Vec<TrackIn> = tracks.iter().map(TrackIn::from).collect();
    check_add_in(c, &items).map(|(_, codecs)| codecs)
}

pub fn check_add_in(c: &AddCtx<'_>, items: &[TrackIn]) -> Result<(Vec<PublishTrack>, Vec<String>), Failure> {
    check_inner(c, items, true)
}

pub fn check_add_req(c: &AddCtx<'_>, req: &AddIn) -> Result<(Vec<PublishTrack>, Vec<String>), Failure> {
    check_inner(c, &req.tracks, req.rid_extmap_id.is_some() && req.mid_extmap_id.is_some())
}

fn check_inner(c: &AddCtx<'_>, items: &[TrackIn], sim_ext: bool) -> Result<(Vec<PublishTrack>, Vec<String>), Failure> {
    let mut denied: Vec<&str> = items.iter().filter_map(|t| t.kind.map(|k| perm_bit(k, t.source))).filter(|b| !allowed(&c.perm, b)).collect();
    if !denied.is_empty() {
        denied.sort();
        denied.dedup();
        return Err(Failure::of(Code::NOT_AUTHORIZED).details(serde_json::json!({ "denied": denied })));
    }
    if items.iter().any(|t| t.duplex.unwrap_or(Duplex::Full) == Duplex::Full) && c.pub_room != Some(c.room) {
        return Err(Failure::of(Code::NOT_PUB_ROOM));
    }
    if items.len() > REQ_MAX || c.active + items.len() > ACTIVE_MAX {
        return Err(Failure::of(Code::TRACK_LIMIT));
    }
    let tracks = items.iter().map(fill).collect::<Result<Vec<_>, _>>()?;
    if !sim_ext && tracks.iter().any(|t| t.simulcast.unwrap_or(false)) {
        return Err(Failure::of(Code::MISSING_FIELD));
    }
    if c.one_pc {
        for t in &tracks {
            for s in [Some(t.ssrc), t.rtx_ssrc].into_iter().flatten() {
                if s != 0 && c.downlink_ssrcs.contains(&s) {
                    return Err(Failure::of(Code::SSRC_COLLISION).details(serde_json::json!({ "ssrc": s })));
                }
            }
        }
    }
    let mut codecs = Vec::with_capacity(tracks.len());
    for t in &tracks {
        let canon = match (t.kind, t.codec.as_deref()) {
            (Kind::Video, name) => crate::trk::canonical_video(name.unwrap_or_default()),
            (Kind::Audio, Some(name)) => crate::trk::canonical_audio(name),
            (Kind::Audio, None) => crate::trk::canonical_audio(c.room_audio),
        };
        let Some(canon) = canon else {
            let supported: Vec<&str> = if t.kind == Kind::Video { crate::trk::SUPPORTED_VIDEO.to_vec() } else { crate::trk::SUPPORTED_AUDIO.to_vec() };
            return Err(Failure::of(Code::CODEC_REQUIRED).details(serde_json::json!({ "supported": supported })));
        };
        let room_codec = if t.kind == Kind::Video { c.room_video } else { c.room_audio };
        if !crate::trk::codec_matches(t.kind, canon, t.fmtp.as_deref(), room_codec) {
            return Err(Failure::of(Code::CODEC_MISMATCH).details(serde_json::json!({ "codec": room_codec, "fmtp": crate::trk::sub_fmtp(room_codec) })));
        }
        codecs.push(canon.to_string());
    }
    Ok((tracks, codecs))
}

pub fn move_check(streams: &BTreeMap<String, Arc<Stream>>, perm: &Permission, room_audio: &str, room_video: &str) -> (bool, bool) {
    let mut bad = false;
    let mut noperm = false;
    for s in streams.values() {
        let room_codec = if s.kind == Kind::Video { room_video } else { room_audio };
        if !crate::trk::codec_matches(s.kind, &s.codec, s.fmtp.as_deref(), room_codec) {
            bad = true;
        }
        if !allowed(perm, perm_bit(s.kind, s.source)) {
            noperm = true;
        }
    }
    (bad, noperm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(kind: Kind, codec: &str, duplex: Duplex) -> PublishTrack {
        PublishTrack { kind, ssrc: 11, mid: "0".into(), codec: Some(codec.into()), fmtp: Some("packetization-mode=1;profile-level-id=42e01f".into()), pt: 111, rtx_pt: None, rtx_ssrc: None, source: None, duplex: Some(duplex), simulcast: None }
    }

    #[test]
    fn order_permission_before_count_and_fields() {
        let none = HashSet::new();
        let c = AddCtx { room: "r", pub_room: Some("r"), perm: Permission { publish_audio: false, ..Permission::default() }, active: 0, downlink_ssrcs: &none, one_pc: false, room_audio: "opus", room_video: "H264" };
        let mut bad = tr(Kind::Audio, "opus", Duplex::Full);
        bad.mid = String::new();
        let e = check_add(&c, &[bad]).expect_err("perm");
        assert_eq!(e.code, Code::NOT_AUTHORIZED.0);
        let c2 = AddCtx { perm: Permission::default(), pub_room: None, ..c.clone() };
        assert_eq!(check_add(&c2, &[tr(Kind::Audio, "opus", Duplex::Full)]).expect_err("pub").code, Code::NOT_PUB_ROOM.0);
        assert!(check_add(&c2, &[tr(Kind::Audio, "opus", Duplex::Half)]).is_ok());
        let c3 = AddCtx { perm: Permission::default(), active: 16, ..c.clone() };
        assert_eq!(check_add(&c3, &[tr(Kind::Audio, "opus", Duplex::Full)]).expect_err("lim").code, Code::TRACK_LIMIT.0);
        let c4 = AddCtx { perm: Permission::default(), ..c.clone() };
        assert_eq!(check_add(&c4, &[tr(Kind::Video, "VP9", Duplex::Full)]).expect_err("codec").code, Code::CODEC_REQUIRED.0);
        assert_eq!(check_add(&c4, &[tr(Kind::Video, "VP8", Duplex::Full)]).expect_err("mm").code, Code::CODEC_MISMATCH.0);
        assert_eq!(rid_layer(b"q"), None);
    }
}
