// author: kodeholic (powered by Claude)
//! 구독 배관 — 정§7-1·§7-2·§7-2-1. 구독 연결(Peer)마다 mid 풀 하나·PT 표 하나·확장표 하나.
//! `SubscribeState` 에 `Intended` 가 없는 것이 설계다 — 구독은 등록 자체가 의도이고, `Active` 전이는 `READY{tracks}` 다.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use dashmap::DashMap;
use oxsig::schema::{Duplex, Extmap, MediaKind, PcMode, TrackEntry};

use super::mid::{MidPool, to_wire};
use super::pt::PtTable;
use super::rewriter::Rewriter;
use super::track::PublisherStream;
use crate::transport::session::TransportSession;

/// 정§7-2-1 — 구독자 표에 없는 URI 는 표 밖 번호로 바꾼다. 받는 쪽이 무시하도록.
pub const EXT_ID_MAX: u8 = 14;
/// 확장 번호는 1~14(one-byte) 이므로 사상표는 16칸이면 족하다.
pub const EXT_SLOTS: usize = 16;

/// 배관 하나를 만드는 재료 — 배정이 끝난 값만 온다.
pub struct SubSpec {
    pub subscriber: String,
    pub room_id: String,
    pub mid: Option<u16>,
    pub pt: u8,
    pub transport: Option<Arc<TransportSession>>,
    pub now_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscribeState {
    Created,
    Active,
}

pub struct SubscriberStream {
    pub subscriber: String,
    pub room_id: String,
    pub track_id: String,
    pub kind: MediaKind,
    /// 정§7-2 — 고갈이면 `None` 으로 산다(존재는 알되 조립 못 함). 풀리면 재발급한다.
    mid: Mutex<Option<u16>>,
    pub vssrc: u32,
    pt: AtomicU8,
    rtx_pt: AtomicU8,
    state: AtomicU8,
    /// 발행자 번호 → 이 구독자 번호. 양쪽이 고정이라 붙일 때 한 번 굽는다(핫패스 무락).
    ext: ArcSwap<[Option<u8>; EXT_SLOTS]>,
    /// egress 를 내보내는 자리 — 직접 소유한다(패킷마다 조회하지 않는다). 회수된 뒤엔 `None` 이다.
    pub transport: Option<Arc<TransportSession>>,
    pub sent: AtomicU64,
    /// 정§11-2 — 번역 SR 의 `octets`. 페이로드가 아니라 보낸 RTP 전체 길이다(RFC 3550 §6.4.1).
    pub sent_octets: AtomicU64,
    /// `T-gate` 안전망의 기준 시각(정§7-4).
    pub created_at_ms: AtomicU64,
    /// 정§8-1·§10-2 — 출처가 바뀌어도 egress 가 이어지게 한다(슬롯은 방이 들고 개인 구독은 여기).
    pub rewriter: Rewriter,
    /// 정§10-1 — `SUBSCRIBE_LAYER` 의 `spatial` 은 ★상한이다(범위 초과는 자른다).
    spatial_cap: AtomicU8,
    paused: AtomicU8,
    priority: AtomicU8,
    /// 정§10-2 — 지금 릴레이 중인 단. 전환은 ★target 의 키프레임에서만 확정된다.
    current: Mutex<Option<String>>,
    target: Mutex<Option<String>>,
    target_since_ms: AtomicU64,
}

/// 정§10-1 — 낮은 화질부터 `0`. 이 세대의 인코딩은 두 단 고정이다(연§6-3).
pub const RID_BY_SPATIAL: [&str; 2] = ["l", "h"];
pub const SPATIAL_MAX: u8 = 1;
/// 연§4-1 — 구독자에게 알리는 값.
pub const SCALABILITY: &str = "L2T1";
/// 정§10-2 — 전환 pending 만료.
pub const SWITCH_PENDING_MS: u64 = 10_000;
/// 연§6-3 — 대역 부족 시 채움 순서의 기본값.
pub const PRIORITY_DEFAULT: u8 = 128;

impl SubscriberStream {
    pub fn mid(&self) -> Option<u16> {
        *self.mid.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_mid(&self, mid: Option<u16>) {
        *self.mid.lock().unwrap_or_else(|e| e.into_inner()) = mid;
    }

    pub fn pt(&self) -> u8 {
        self.pt.load(Ordering::Acquire)
    }

    pub fn rtx_pt(&self) -> Option<u8> {
        match self.rtx_pt.load(Ordering::Acquire) {
            0 => None,
            v => Some(v),
        }
    }

    pub fn set_pt(&self, pt: u8, rtx_pt: Option<u8>) {
        self.pt.store(pt, Ordering::Release);
        self.rtx_pt.store(rtx_pt.unwrap_or(0), Ordering::Release);
    }

    pub fn state(&self) -> SubscribeState {
        if self.state.load(Ordering::Acquire) == 1 { SubscribeState::Active } else { SubscribeState::Created }
    }

    /// `READY{tracks}` 가 그 (user, 방) 잠금을 푼다. 여러 번 보내도 안전하다.
    pub fn activate(&self) -> bool {
        self.state.swap(1, Ordering::AcqRel) == 0
    }

    // ───────── 레이어(정§10-1·§10-2) ─────────

    /// ★상한이다 — 범위를 넘으면 그 축 최대로 자른다(거절하지 않는다).
    pub fn set_spatial_cap(&self, spatial: u8) {
        self.spatial_cap.store(spatial.min(SPATIAL_MAX), Ordering::Release);
    }
    pub fn spatial_cap(&self) -> u8 {
        self.spatial_cap.load(Ordering::Acquire)
    }
    /// `paused` 는 ★별개 축이다 — 전달 자체를 멈춘다.
    pub fn set_paused(&self, paused: bool) -> bool {
        self.paused.swap(u8::from(paused), Ordering::AcqRel) != u8::from(paused)
    }
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Acquire) == 1
    }
    pub fn set_priority(&self, priority: u8) {
        self.priority.store(priority, Ordering::Release);
    }
    pub fn priority(&self) -> u8 {
        self.priority.load(Ordering::Acquire)
    }

    /// 상한 안에서 받고 싶은 단의 rid. 자동 판단(정§10-3)은 이 판에 없으므로 상한이 곧 목표다.
    pub fn wanted_rid(&self) -> &'static str {
        RID_BY_SPATIAL[usize::from(self.spatial_cap())]
    }

    pub fn current_rid(&self) -> Option<String> {
        self.current.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 정§10-2 — 목표를 세운다. 반환: 새로 세웠나(그때 target-rid PLI 를 청한다).
    pub fn aim(&self, rid: &str, now_ms: u64) -> bool {
        let mut t = self.target.lock().unwrap_or_else(|e| e.into_inner());
        if t.as_deref() == Some(rid) && now_ms.saturating_sub(self.target_since_ms.load(Ordering::Acquire)) < SWITCH_PENDING_MS {
            return false;
        }
        *t = Some(rid.to_owned());
        self.target_since_ms.store(now_ms, Ordering::Release);
        true
    }

    /// 정§10-2 — ★전환은 target 레이어의 키프레임 도착에서만 확정된다.
    pub fn switch_to(&self, rid: &str) {
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = Some(rid.to_owned());
        *self.target.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn ext_of(&self, publisher_id: u8) -> Option<u8> {
        *self.ext.load().get(usize::from(publisher_id))?
    }

    pub fn set_ext(&self, table: [Option<u8>; EXT_SLOTS]) {
        self.ext.store(Arc::new(table));
    }
}

/// Peer 하나의 구독 전부. 키는 (방, 발행 `track_id`) — 같은 트랙을 두 방에서 받는 일은 없다.
pub struct SubscribeContext {
    streams: DashMap<(String, String), Arc<SubscriberStream>>,
    mids: Mutex<MidPool>,
    pts: Mutex<PtTable>,
    /// 2pc 는 `server_config.extmap`(mid 제외), 1pc 는 `READY{transport}` 신고표.
    extmap: ArcSwap<Vec<Extmap>>,
}

impl SubscribeContext {
    pub fn new(pc_mode: PcMode) -> Self {
        Self {
            streams: DashMap::new(),
            mids: Mutex::new(MidPool::new(pc_mode)),
            pts: Mutex::new(PtTable::default()),
            extmap: ArcSwap::from_pointee(Vec::new()),
        }
    }

    pub fn set_extmap(&self, table: Vec<Extmap>) {
        self.extmap.store(Arc::new(table));
    }

    pub fn extmap(&self) -> Arc<Vec<Extmap>> {
        self.extmap.load_full()
    }

    /// 정§7-2-1 — 발행자 번호 → URI → 구독자 번호. 표에 없는 URI 는 ★표 밖 번호(1~14 중 없는 최대값)로.
    pub fn ext_table(&self, publisher: &[(u8, &'static str)]) -> [Option<u8>; EXT_SLOTS] {
        let mine = self.extmap();
        let spare = (1..=EXT_ID_MAX).rev().find(|id| !mine.iter().any(|e| e.id == *id));
        let mut table = [None; EXT_SLOTS];
        for (id, uri) in publisher {
            let Some(slot) = table.get_mut(usize::from(*id)) else { continue };
            *slot = mine.iter().find(|e| e.uri == *uri).map(|e| e.id).or(spare);
        }
        table
    }

    pub fn assign_pt(&self, codec: &str, fmtp: Option<&str>, want_rtx: bool) -> Option<(u8, Option<u8>)> {
        self.pts.lock().unwrap_or_else(|e| e.into_inner()).assign(codec, fmtp, want_rtx)
    }

    /// 1pc — 신고 `codecs` 를 씨앗으로 먼저. 반환: 앞서 배정한 값과 어긋나 재배정된 튜플 키들.
    pub fn seed_pt(&self, codecs: &[oxsig::body::media::CodecLine]) -> Vec<String> {
        self.pts.lock().unwrap_or_else(|e| e.into_inner()).seed_from_ready(codecs)
    }

    pub fn alloc_mid(&self, kind: MediaKind) -> Option<u16> {
        self.mids.lock().unwrap_or_else(|e| e.into_inner()).alloc(kind)
    }

    pub fn release_mid(&self, kind: MediaKind, mid: u16) {
        self.mids.lock().unwrap_or_else(|e| e.into_inner()).release(kind, mid);
    }

    pub fn insert(&self, stream: &Arc<PublisherStream>, spec: SubSpec) -> Arc<SubscriberStream> {
        let SubSpec { subscriber, room_id, mid, pt, transport, now_ms } = spec;
        let sub = Arc::new(SubscriberStream {
            subscriber: subscriber.clone(),
            room_id: room_id.clone(),
            track_id: stream.track_id.clone(),
            kind: stream.kind,
            mid: Mutex::new(mid),
            vssrc: stream.vssrc,
            pt: AtomicU8::new(pt),
            rtx_pt: AtomicU8::new(0),
            state: AtomicU8::new(0),
            ext: ArcSwap::from_pointee([None; EXT_SLOTS]),
            transport,
            sent: AtomicU64::new(0),
            sent_octets: AtomicU64::new(0),
            created_at_ms: AtomicU64::new(now_ms),
            rewriter: Rewriter::default(),
            spatial_cap: AtomicU8::new(SPATIAL_MAX),
            paused: AtomicU8::new(0),
            priority: AtomicU8::new(PRIORITY_DEFAULT),
            current: Mutex::new(None),
            target: Mutex::new(None),
            target_since_ms: AtomicU64::new(0),
        });
        self.streams.insert((room_id, stream.track_id.clone()), sub.clone());
        sub
    }

    pub fn get(&self, room_id: &str, track_id: &str) -> Option<Arc<SubscriberStream>> {
        self.streams.get(&(room_id.to_owned(), track_id.to_owned())).map(|s| s.clone())
    }

    /// 정§7-2 회수 순서 — mid_map → 색인 → mid_pool. pool 반환이 "다음 발급 가능" 신호이므로 색인이 먼저 깨끗해야 한다.
    pub fn remove(&self, subscriber: &str, room_id: &str, track_id: &str) -> Option<Arc<SubscriberStream>> {
        let (_, sub) = self.streams.remove(&(room_id.to_owned(), track_id.to_owned()))?;
        debug_assert_eq!(sub.subscriber, subscriber);
        let mid = sub.mid();
        sub.set_mid(None);
        if let Some(m) = mid {
            self.release_mid(sub.kind, m);
        }
        Some(sub)
    }

    pub fn in_room(&self, room_id: &str) -> Vec<Arc<SubscriberStream>> {
        self.streams.iter().filter(|e| e.key().0 == room_id).map(|e| e.clone()).collect()
    }

    pub fn all(&self) -> Vec<Arc<SubscriberStream>> {
        self.streams.iter().map(|e| e.clone()).collect()
    }

    /// 정§7-2 재발급 — mid 없는 구독 중 오래된 것부터. 반환: 다시 발급된 것들.
    pub fn refill_mids(&self) -> Vec<Arc<SubscriberStream>> {
        let mut waiting: Vec<Arc<SubscriberStream>> = self.streams.iter().filter(|e| e.mid().is_none()).map(|e| e.clone()).collect();
        waiting.sort_by(|a, b| (&a.room_id, &a.track_id).cmp(&(&b.room_id, &b.track_id)));
        let mut filled = Vec::new();
        for sub in waiting {
            let Some(m) = self.alloc_mid(sub.kind) else { break };
            sub.set_mid(Some(m));
            filled.push(sub);
        }
        filled
    }
}

/// 연§4-1 — 구독자에게 나가는 보관본 한 항목. 잔존(`active:false`)에도 pt·codec·fmtp 를 반드시 싣는다(정§7-1 ⑤).
pub fn entry_of(stream: &PublisherStream, sub: &SubscriberStream) -> TrackEntry {
    let half = stream.duplex() == Duplex::Half;
    TrackEntry {
        room_id: sub.room_id.clone(),
        user_id: (!stream.owner.is_empty()).then(|| stream.owner.clone()),
        kind: stream.kind,
        ssrc: sub.vssrc,
        track_id: stream.track_id.clone(),
        mid: sub.mid().map(to_wire),
        duplex: Some(stream.duplex()),
        // 정§8-2 — half 로 전환된 개인 트랙은 `active:false` 로 잔존한다(자리 지킴 m-line 의 근거).
        active: Some(!half),
        source: stream.source.clone(),
        rtx_ssrc: None,
        pt: Some(sub.pt()),
        rtx_pt: sub.rtx_pt(),
        codec: (stream.kind == MediaKind::Video).then(|| stream.codec.to_owned()),
        fmtp: stream.fmtp.clone(),
        simulcast: stream.simulcast.then_some(true),
        scalability: stream.simulcast.then(|| SCALABILITY.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::track::{PublishContext, StreamSpec};

    fn publisher(kind: MediaKind, codec: &'static str) -> (PublishContext, Arc<PublisherStream>) {
        let ctx = PublishContext::default();
        let s = ctx.insert(StreamSpec {
            track_id: "t1".into(),
            vssrc: 0xABCD,
            owner: "u1".into(),
            room_id: "r1".into(),
            kind,
            mid: "0".into(),
            pt: 96,
            rtx_pt: None,
            codec,
            fmtp: Some("x=1".into()),
            source: Some("camera".into()),
            duplex: Duplex::Full,
            simulcast: false,
            ssrc: 5,
            rtx_ssrc: None,
        });
        (ctx, s)
    }

    #[test]
    fn mid_release_returns_to_the_pool_after_the_index_is_clean() {
        let (_pub, s) = publisher(MediaKind::Video, "VP8");
        let ctx = SubscribeContext::new(PcMode::TwoPc);
        let mid = ctx.alloc_mid(MediaKind::Video).unwrap();
        let sub = ctx.insert(&s, SubSpec { subscriber: "u2".into(), room_id: "r1".into(), mid: Some(mid), pt: 96, transport: None, now_ms: 0 });
        assert_eq!((sub.mid(), sub.state()), (Some(0), SubscribeState::Created));
        assert!(sub.activate() && !sub.activate(), "READY 는 여러 번 와도 안전하다");
        assert!(ctx.get("r1", "t1").is_some() && ctx.in_room("r1").len() == 1);
        ctx.remove("u2", "r1", "t1");
        assert!(ctx.get("r1", "t1").is_none() && sub.mid().is_none());
        assert_eq!(ctx.alloc_mid(MediaKind::Video), Some(0), "회수분이 같은 kind 로 돌아왔다");
    }

    #[test]
    fn exhausted_mid_is_refilled_oldest_first() {
        let (_pub, s) = publisher(MediaKind::Video, "VP8");
        let ctx = SubscribeContext::new(PcMode::TwoPc);
        let a = ctx.insert(&s, SubSpec { subscriber: "u2".into(), room_id: "r1".into(), mid: None, pt: 96, transport: None, now_ms: 0 });
        let b = ctx.insert(&s, SubSpec { subscriber: "u2".into(), room_id: "r2".into(), mid: None, pt: 96, transport: None, now_ms: 0 });
        assert!(ctx.refill_mids().len() == 2, "풀리면 오래된 것부터 다시 발급");
        assert_eq!((a.mid(), b.mid()), (Some(0), Some(1)));
        assert!(ctx.refill_mids().is_empty());
    }

    #[test]
    fn entry_carries_everything_the_assembler_needs() {
        let (_pub, s) = publisher(MediaKind::Video, "H264");
        let ctx = SubscribeContext::new(PcMode::OnePc);
        let sub = ctx.insert(&s, SubSpec { subscriber: "u2".into(), room_id: "r1".into(), mid: ctx.alloc_mid(MediaKind::Video), pt: 102, transport: None, now_ms: 0 });
        sub.set_pt(102, Some(103));
        let e = entry_of(&s, &sub);
        assert_eq!((e.mid.as_deref(), e.pt, e.rtx_pt, e.codec.as_deref()), (Some("32"), Some(102), Some(103), Some("H264")));
        assert_eq!((e.fmtp.as_deref(), e.ssrc, e.user_id.as_deref(), e.duplex), (Some("x=1"), 0xABCD, Some("u1"), Some(Duplex::Full)));
        assert!(e.is_reachable() && !e.is_slot() && e.simulcast.is_none());
    }

    #[test]
    fn extension_table_maps_by_uri_and_parks_the_unknown() {
        let ctx = SubscribeContext::new(PcMode::TwoPc);
        ctx.set_extmap(vec![
            Extmap { id: 4, uri: "urn:level".into() },
            Extmap { id: 5, uri: "urn:abs".into() },
            Extmap { id: 6, uri: "urn:twcc".into() },
        ]);
        let t = ctx.ext_table(&[(1, "urn:mid"), (2, "urn:abs"), (3, "urn:twcc")]);
        assert_eq!((t[2], t[3]), (Some(5), Some(6)), "URI 로 이어 번호만 바꾼다");
        assert_eq!(t[1], Some(14), "구독자 표에 없는 mid 는 표 밖 번호로 — 받는 쪽이 버린다");
        assert_eq!(t[7], None, "발행자가 안 쓰는 번호는 손대지 않는다");
    }
}
