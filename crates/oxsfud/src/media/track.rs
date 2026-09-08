// author: kodeholic (powered by Claude)
//! 발행 — 정§6-1 두 계층. ★논리 `PublisherStream` 이 식별 전부(`track_id`·`vssrc`)와 메타를 들고,
//! 물리 `PublisherTrack` 이 SSRC 하나와 구독자 목록을 든다. 물리가 바뀌어도 논리가 산다.
//! 식별 3평면을 섞지 않는다: `track_id`(지목·파싱 금지) · `vssrc`(egress 값) · 실 `ssrc`(ingress 매칭).

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use arc_swap::ArcSwap;
use dashmap::DashMap;
use oxsig::schema::{Duplex, MediaKind};

use super::nack::GapTracker;
use super::reception::Reception;
use super::subscribe::SubscriberStream;

/// 정§2-2 — 등록(의도)과 첫 RTP(실현)가 갈린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishState {
    Created,
    Intended,
    Active,
}

impl PublishState {
    fn code(self) -> u8 {
        match self {
            Self::Created => 0,
            Self::Intended => 1,
            Self::Active => 2,
        }
    }
    fn from_code(v: u8) -> Self {
        match v {
            1 => Self::Intended,
            2 => Self::Active,
            _ => Self::Created,
        }
    }
}

/// 물리 — SSRC 하나. 구독자 목록은 ★핫패스 무락(RCU)이고 방향이 역전돼 있다(발행측이 구독자를 든다).
#[derive(Debug)]
pub struct PublisherTrack {
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub rid: Option<String>,
    subscribers: ArcSwap<Vec<Weak<SubscriberStream>>>,
    pub rtp_in: AtomicU64,
    /// 정§11-2 — Ingress RR 의 재료. ★RTX 는 여기 안 든다(손실률 오염).
    pub reception: Reception,
    /// 정§11-1 상향 — 이 트랙의 결손 장부. 재전송 요구의 출처다.
    pub gaps: GapTracker,
    last_pli_ms: AtomicU64,
}

impl PublisherTrack {
    fn new(ssrc: u32, rtx_ssrc: Option<u32>, rid: Option<String>) -> Self {
        Self {
            ssrc,
            rtx_ssrc,
            rid,
            subscribers: ArcSwap::from_pointee(Vec::new()),
            rtp_in: AtomicU64::new(0),
            reception: Reception::default(),
            gaps: GapTracker::default(),
            last_pli_ms: AtomicU64::new(0),
        }
    }

    /// 정§7-1 ④ — 통째 교체(RCU). 죽은 Weak 는 이때 함께 걷는다.
    pub fn attach(&self, sub: &Arc<SubscriberStream>) {
        let mut next: Vec<Weak<SubscriberStream>> = self.subscribers.load().iter().filter(|w| w.strong_count() > 0).cloned().collect();
        next.push(Arc::downgrade(sub));
        self.subscribers.store(Arc::new(next));
    }

    pub fn detach(&self, subscriber: &str, room_id: &str) {
        let next: Vec<Weak<SubscriberStream>> = self
            .subscribers
            .load()
            .iter()
            .filter(|w| w.upgrade().is_some_and(|s| !(s.subscriber == subscriber && s.room_id == room_id)))
            .cloned()
            .collect();
        self.subscribers.store(Arc::new(next));
    }

    /// ★핫패스 — RCU 안내자를 그대로 돌려준다. 부르는 쪽이 슬라이스를 순회하므로 ★할당이 없다.
    pub fn subscribers(&self) -> arc_swap::Guard<Arc<Vec<Weak<SubscriberStream>>>> {
        self.subscribers.load()
    }

    /// 정§11-2 — PLI 스로틀. ★인프라 PLI(게이트 해제·승계·키프레임 대기)는 `force` 로 통과한다.
    pub fn claim_pli(&self, now_ms: u64, min_gap_ms: u64, force: bool) -> bool {
        let last = self.last_pli_ms.load(Ordering::Acquire);
        if !force && last != 0 && now_ms.saturating_sub(last) < min_gap_ms {
            return false;
        }
        self.last_pli_ms.store(now_ms, Ordering::Release);
        true
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.load().iter().filter(|w| w.strong_count() > 0).count()
    }
}

/// 논리 — 클라의 `MediaStream` 정합. 메타는 전부 등록 신고값 그대로다(서버는 SDP 를 안 본다).
#[derive(Debug)]
pub struct PublisherStream {
    pub track_id: String,
    pub vssrc: u32,
    pub owner: String,
    /// 등록 검사의 기준 방(연§6-3) — 발화 방은 매 패킷 발행자의 `pub_room` 이 정한다(정§7-3).
    pub room_id: String,
    pub kind: MediaKind,
    pub mid: String,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub codec: &'static str,
    pub fmtp: Option<String>,
    pub source: Option<String>,
    pub simulcast: bool,
    /// 정§16-2 — 이 스트림을 내보내지 않기로 한 사유별 계수.
    pub drops: crate::media::drops::PubDrops,
    duplex: AtomicU8,
    muted: AtomicBool,
    state: AtomicU8,
    pt_checked: AtomicBool,
    tracks: ArcSwap<Vec<Arc<PublisherTrack>>>,
}

impl PublisherStream {
    pub fn duplex(&self) -> Duplex {
        if self.duplex.load(Ordering::Acquire) == 1 { Duplex::Half } else { Duplex::Full }
    }

    /// 정§8-2 — 원자 교체 하나로 fan-out 경로가 갈린다(새 분기를 만들지 않는다).
    pub fn set_duplex(&self, duplex: Duplex) -> bool {
        let next = u8::from(duplex == Duplex::Half);
        self.duplex.swap(next, Ordering::AcqRel) != next
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Acquire)
    }

    pub fn set_muted(&self, muted: bool) -> bool {
        self.muted.swap(muted, Ordering::AcqRel) != muted
    }

    pub fn state(&self) -> PublishState {
        PublishState::from_code(self.state.load(Ordering::Acquire))
    }

    /// 전이일 때만 부수효과를 낸다(§2-3 계약 3). ★이미 그 값이면 원자 RMW 조차 하지 않는다 — 매 패킷 부르는 자리다.
    pub fn set_state(&self, next: PublishState) -> bool {
        if self.state.load(Ordering::Acquire) == next.code() {
            return false;
        }
        PublishState::from_code(self.state.swap(next.code(), Ordering::AcqRel)) != next
    }

    /// 정§6-3 — 선언 PT 와 첫 RTP PT 의 불일치는 ★표면화만 하고 고치지 않는다. 트랙당 1회 latch.
    pub fn claim_pt_report(&self) -> bool {
        !self.pt_checked.swap(true, Ordering::AcqRel)
    }

    pub fn tracks(&self) -> Vec<Arc<PublisherTrack>> {
        self.tracks.load().as_ref().clone()
    }

    /// 그 단이 이미 붙었나 — RID 학습이 같은 단을 두 번 붙이지 않게.
    pub fn track_of_rid(&self, rid: &str) -> Option<Arc<PublisherTrack>> {
        self.tracks.load().iter().find(|t| t.rid.as_deref() == Some(rid)).cloned()
    }

    pub fn track_of(&self, ssrc: u32) -> Option<Arc<PublisherTrack>> {
        self.tracks.load().iter().find(|t| t.ssrc == ssrc).cloned()
    }

    /// simulcast 는 물리를 첫 RTP 까지 미룬다(정§6-2 6) — 그때 RID 로 붙인다.
    pub fn add_track(&self, ssrc: u32, rtx_ssrc: Option<u32>, rid: Option<String>) -> Arc<PublisherTrack> {
        let track = Arc::new(PublisherTrack::new(ssrc, rtx_ssrc, rid));
        let mut next = self.tracks.load().as_ref().clone();
        next.push(track.clone());
        self.tracks.store(Arc::new(next));
        track
    }

    pub fn detach_all(&self, subscriber: &str, room_id: &str) {
        for t in self.tracks.load().iter() {
            t.detach(subscriber, room_id);
        }
    }

    pub fn rtp_in(&self) -> u64 {
        self.tracks.load().iter().map(|t| t.rtp_in.load(Ordering::Relaxed)).sum()
    }

    /// 연§4-2 — 클럭은 코덱이 정한다. opus 48k · video 90k.
    pub fn clock_rate(&self) -> u32 {
        match self.kind {
            MediaKind::Audio => 48_000,
            MediaKind::Video => 90_000,
        }
    }
}

/// Peer 하나의 발행 전부(정§2-1 소유 지도). 상한의 단위는 ★논리 스트림이다.
#[derive(Debug)]
pub struct PublishContext {
    streams: DashMap<String, Arc<PublisherStream>>,
    /// ingress 매칭 색인 — 실 SSRC 로 (논리, 물리)를 집는다. 핫패스라 조회 하나로 끝낸다.
    by_ssrc: DashMap<u32, (Arc<PublisherStream>, Arc<PublisherTrack>)>,
    /// 연§6-3 협상 결과 확장 번호 — 안 보내면 서버 선언값으로 폴백한다.
    extmap: ArcSwap<Vec<(u8, &'static str)>>,
}

/// 논리 스트림 하나를 만드는 재료 — 검사를 통과한 신고값만 들어온다.
pub struct StreamSpec {
    pub track_id: String,
    pub vssrc: u32,
    pub owner: String,
    pub room_id: String,
    pub kind: MediaKind,
    pub mid: String,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub codec: &'static str,
    pub fmtp: Option<String>,
    pub source: Option<String>,
    pub duplex: Duplex,
    pub simulcast: bool,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
}

impl PublisherStream {
    /// 정§6-2 6 — ★논리 선등록 후 물리. 뒤집으면 첫 RTP 가 소속 없는 고아 물리를 만든다.
    /// simulcast 가 아니면 물리 하나를 여기서 붙이고 `Intended` 로 올린다.
    pub fn create(spec: StreamSpec) -> Arc<Self> {
        let stream = Arc::new(PublisherStream {
            track_id: spec.track_id.clone(),
            vssrc: spec.vssrc,
            owner: spec.owner,
            room_id: spec.room_id,
            kind: spec.kind,
            mid: spec.mid,
            pt: spec.pt,
            rtx_pt: spec.rtx_pt,
            codec: spec.codec,
            fmtp: spec.fmtp,
            source: spec.source,
            simulcast: spec.simulcast,
            drops: crate::media::drops::PubDrops::default(),
            duplex: AtomicU8::new(u8::from(spec.duplex == Duplex::Half)),
            muted: AtomicBool::new(false),
            state: AtomicU8::new(PublishState::Created.code()),
            pt_checked: AtomicBool::new(false),
            tracks: ArcSwap::from_pointee(Vec::new()),
        });
        if !spec.simulcast {
            stream.add_track(spec.ssrc, spec.rtx_ssrc, None);
            stream.set_state(PublishState::Intended);
        }
        stream
    }
}

impl Default for PublishContext {
    fn default() -> Self {
        Self { streams: DashMap::new(), by_ssrc: DashMap::new(), extmap: ArcSwap::from_pointee(super::SERVER_EXTMAP.to_vec()) }
    }
}

impl PublishContext {
    pub fn insert(&self, spec: StreamSpec) -> Arc<PublisherStream> {
        let track_id = spec.track_id.clone();
        let stream = PublisherStream::create(spec);
        for t in stream.tracks() {
            self.by_ssrc.insert(t.ssrc, (stream.clone(), t));
        }
        self.streams.insert(track_id, stream.clone());
        stream
    }

    /// simulcast 는 첫 RTP 에서 RID 로 물리가 붙는다 — 그때 색인도 함께 는다(정§6-3 RID 학습).
    pub fn learn(&self, stream: &Arc<PublisherStream>, ssrc: u32, rid: Option<String>) -> Arc<PublisherTrack> {
        let track = stream.add_track(ssrc, None, rid);
        self.by_ssrc.insert(ssrc, (stream.clone(), track.clone()));
        track
    }

    /// ingress — 실 SSRC 매칭. `vssrc`·`track_id` 로는 여기 오지 않는다(식별 3평면).
    pub fn by_ssrc(&self, ssrc: u32) -> Option<(Arc<PublisherStream>, Arc<PublisherTrack>)> {
        self.by_ssrc.get(&ssrc).map(|e| e.clone())
    }

    pub fn get(&self, track_id: &str) -> Option<Arc<PublisherStream>> {
        self.streams.get(track_id).map(|s| s.clone())
    }

    /// §2-3 계약 4 — 색인 해제가 본체 제거보다 먼저다(늦으면 그 사이 도착한 패킷이 죽은 참조를 되살린다).
    pub fn remove(&self, track_id: &str) -> Option<Arc<PublisherStream>> {
        let stream = self.streams.get(track_id).map(|s| s.clone())?;
        for t in stream.tracks() {
            self.by_ssrc.remove(&t.ssrc);
        }
        self.streams.remove(track_id).map(|(_, s)| s)
    }

    /// 정§6-2 3 — 그 user 활성 스트림 수. 반복 호출 우회를 봉쇄하는 짝이다.
    pub fn active(&self) -> usize {
        self.streams.len()
    }

    pub fn all(&self) -> Vec<Arc<PublisherStream>> {
        self.streams.iter().map(|e| e.clone()).collect()
    }

    pub fn in_room(&self, room_id: &str) -> Vec<Arc<PublisherStream>> {
        self.streams.iter().filter(|e| e.room_id == room_id).map(|e| e.clone()).collect()
    }

    /// 요청에 있는 번호만 교체한다(정§6-2 1 — 원자값).
    pub fn set_extmap(&self, declared: Vec<(u8, &'static str)>) {
        self.extmap.store(Arc::new(declared));
    }

    /// 발행자가 쓴 번호 → URI 표. 신고가 없으면 서버 선언표가 답이다.
    pub fn extmap(&self) -> Vec<(u8, &'static str)> {
        self.extmap.load().as_ref().clone()
    }

    pub fn uri_of(&self, id: u8) -> Option<&'static str> {
        self.extmap.load().iter().find(|(i, _)| *i == id).map(|(_, uri)| *uri)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::subscribe::{SubSpec, SubscribeContext};

    fn spec(track_id: &str, kind: MediaKind, ssrc: u32) -> StreamSpec {
        StreamSpec {
            track_id: track_id.into(),
            vssrc: 0xF000_0000 | ssrc,
            owner: "u1".into(),
            room_id: "r1".into(),
            kind,
            mid: "0".into(),
            pt: 111,
            rtx_pt: None,
            codec: "opus",
            fmtp: None,
            source: None,
            duplex: Duplex::Full,
            simulcast: false,
            ssrc,
            rtx_ssrc: None,
        }
    }

    #[test]
    fn logical_registers_before_physical_and_survives_it() {
        let ctx = PublishContext::default();
        let s = ctx.insert(spec("t1", MediaKind::Audio, 1234));
        assert_eq!((s.tracks().len(), s.state(), ctx.active()), (1, PublishState::Intended, 1));
        let sim = ctx.insert(StreamSpec { simulcast: true, ssrc: 0, kind: MediaKind::Video, codec: "VP8", ..spec("t2", MediaKind::Video, 0) });
        assert_eq!((sim.tracks().len(), sim.state()), (0, PublishState::Created), "simulcast 는 물리를 첫 RTP 까지 미룬다");
        let low = sim.add_track(10, None, Some("l".into()));
        sim.add_track(11, None, Some("h".into()));
        assert_eq!((sim.tracks().len(), sim.track_of(10).map(|t| t.ssrc)), (2, Some(10)));
        assert_eq!((low.rid.as_deref(), sim.track_id.as_str(), sim.vssrc), (Some("l"), "t2", 0xF000_0000));
        assert_eq!(ctx.in_room("r1").len(), 2);
        assert!(ctx.in_room("other").is_empty());
        assert_eq!(ctx.by_ssrc(1234).map(|(s, t)| (s.track_id.clone(), t.ssrc)), Some(("t1".to_owned(), 1234)));
        assert!(ctx.by_ssrc(10).is_none(), "add_track 만으로는 색인에 안 든다 — learn 이 짝이다");
        ctx.learn(&sim, 12, Some("h".into()));
        assert_eq!(ctx.by_ssrc(12).map(|(s, _)| s.track_id.clone()), Some("t2".to_owned()));
        assert!(ctx.remove("t1").is_some() && ctx.get("t1").is_none() && ctx.active() == 1 && ctx.by_ssrc(1234).is_none());
    }

    #[test]
    fn subscriber_list_is_rcu_and_drops_dead_weaks() {
        let ctx = PublishContext::default();
        let s = ctx.insert(spec("t1", MediaKind::Audio, 7));
        let track = s.tracks().remove(0);
        let subs = SubscribeContext::new();
        let a = subs.insert(&s, SubSpec { subscriber: "u2".into(), room_id: "r1".into(), mid: Some(0), pt: 111, transport: None, now_ms: 0 });
        {
            let b = subs.insert(&s, SubSpec { subscriber: "u3".into(), room_id: "r1".into(), mid: Some(1), pt: 111, transport: None, now_ms: 0 });
            track.attach(&a);
            track.attach(&b);
            assert_eq!((track.subscriber_count(), track.subscribers().len()), (2, 2));
            subs.remove("u3", "r1", "t1");
        }
        assert_eq!(track.subscriber_count(), 1, "Arc 가 사라지면 목록에서 없는 것과 같다");
        let alive: Vec<String> = track.subscribers().iter().filter_map(Weak::upgrade).map(|s| s.subscriber.clone()).collect();
        assert_eq!(alive, vec!["u2".to_owned()]);
        s.detach_all("u2", "r1");
        assert_eq!(track.subscriber_count(), 0);
        // ★핫패스 계약 — 순회는 RCU 안내자를 그대로 쓴다(목록을 복사하지 않는다).
        let (g1, g2) = (track.subscribers(), track.subscribers());
        assert_eq!(g1.as_ptr(), g2.as_ptr());
    }

    #[test]
    fn atomic_axes_fire_once_each() {
        let ctx = PublishContext::default();
        let s = ctx.insert(spec("t1", MediaKind::Audio, 7));
        assert_eq!((s.duplex(), s.muted()), (Duplex::Full, false));
        assert!(s.set_duplex(Duplex::Half) && !s.set_duplex(Duplex::Half));
        assert!(s.set_muted(true) && !s.set_muted(true));
        assert!(s.claim_pt_report() && !s.claim_pt_report(), "PT 불일치 관측은 트랙당 1회");
        assert!(s.set_state(PublishState::Active) && !s.set_state(PublishState::Active));
        assert_eq!(ctx.uri_of(1), Some(crate::media::URI_MID), "신고 전엔 서버 선언표가 답이다");
        ctx.set_extmap(vec![(3, "urn:x")]);
        assert_eq!((ctx.uri_of(3), ctx.uri_of(9), ctx.extmap().len()), (Some("urn:x"), None, 1));
    }
}
