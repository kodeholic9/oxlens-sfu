// author: kodeholic (powered by Claude)
//! 무전 슬롯 — 정§8-1. 반이중은 방 공용 m-line 하나를 화자들이 돌려쓴다(N:1 — 화자 교대에 재협상이 없다).
//! 슬롯도 배관은 개인 트랙과 같은 자료를 쓴다(`owner` 가 빈 값이라 `TrackEntry.user_id` 가 없다 — 연§4-1).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use oxsig::schema::{Duplex, MediaKind};

use super::codec::AUDIO_CODEC;
use super::rewriter::Rewriter;
use super::track::{PublisherStream, StreamSpec};

/// 연§4-1 wire 값 — 슬롯 `track_id` 는 고정이다(클라는 파싱하지 않고 키로만 쓴다).
pub fn slot_track_id(room_id: &str, kind: MediaKind) -> String {
    let suffix = match kind {
        MediaKind::Audio => "audio",
        MediaKind::Video => "video",
    };
    format!("ptt-{room_id}-{suffix}")
}

/// 방 슬롯·simulcast 의 egress SSRC — ingress 매칭에 쓰지 않는다(식별 3평면, 정§6-1).
/// 전이중 non-sim 은 원본을 그대로 내보내므로 이 값을 쓰지 않는다(정§8-1).
pub fn new_vssrc() -> u32 {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("getrandom");
    u32::from_be_bytes(b) | 0x8000_0000
}

fn slot_stream(room_id: &str, kind: MediaKind, codec: &'static str, fmtp: Option<String>) -> Arc<PublisherStream> {
    let vssrc = new_vssrc();
    PublisherStream::create(StreamSpec {
        track_id: slot_track_id(room_id, kind),
        vssrc,
        owner: String::new(),
        room_id: room_id.to_owned(),
        kind,
        mid: String::new(),
        pt: 0,
        rtx_pt: None,
        codec,
        fmtp,
        source: None,
        duplex: Duplex::Half,
        simulcast: false,
        ssrc: vssrc,
        rtx_ssrc: None,
    })
}

/// 정§7-3·§9-7 — 이 발행 스트림이 지금 그 방 슬롯에 **결합되는가**. 결합되면 그 슬롯을 준다.
///
/// ★핫패스(`prefan`)와 정체 판정(정§14-3)이 **이 함수 하나**를 쓴다. 조건을 두 곳에 적으면
/// 반드시 어긋난다 — 정§8-1 이 *"같은 정보를 두 곳에서 판정하면 시점 race 가 난다"* 고 정한 그것이다.
/// 실제로 어긋나 있었다: 정체 쪽이 *"그 방에 화자가 아무나 있나"* 만 보고 결합을 안 봐서,
/// 코덱 불일치로 미결합된 video 슬롯에 재동기 지시를 보냈다.
///
/// ★화자 여부는 부르는 쪽이 정한다(`floor.is_speaker`) — 발언권 권위는 하나다.
pub fn bound_slot(slots: &SlotSet, stream: &PublisherStream) -> Option<Arc<PublisherStream>> {
    if stream.duplex() != Duplex::Half {
        return None;
    }
    let slot = match stream.kind {
        MediaKind::Audio => slots.audio.clone(),
        MediaKind::Video => slots.video()?,
    };
    // 정§9-7 — video 는 화자 코덱 == 슬롯 코덱일 때만 결합한다(다르면 조용한 검은 화면이 된다).
    if stream.kind == MediaKind::Video
        && (slot.codec, slot.fmtp.as_deref()) != (stream.codec, stream.fmtp.as_deref())
    {
        return None;
    }
    Some(slot)
}

/// 방마다 둘. audio 는 방과 수명이 같고(opus 고정 — 맞출 것이 없다), video 는 첫 화자가 코덱을 정한다.
pub struct SlotSet {
    pub audio: Arc<PublisherStream>,
    video: Mutex<Option<Arc<PublisherStream>>>,
    /// 정§8-1 — 화자가 교대해도 슬롯의 `seq`·`ts` 는 이어져야 한다. kind 마다 하나.
    audio_rewriter: Rewriter,
    video_rewriter: Rewriter,
    /// 정§9-7 — 데우기의 입력 번호. ★판마다 0 으로 되돌리면 rewriter 가 출처 교대를 못 보고
    /// egress seq 가 역행한다(그 자리가 곧 무전 음성 드롭아웃이다). 방과 수명이 같다.
    priming_seq: AtomicU32,
}

impl SlotSet {
    pub fn new(room_id: &str) -> Self {
        debug_assert_eq!(AUDIO_CODEC, "opus");
        Self {
            audio: slot_stream(room_id, MediaKind::Audio, AUDIO_CODEC, None),
            video: Mutex::new(None),
            audio_rewriter: Rewriter::default(),
            video_rewriter: Rewriter::default(),
            priming_seq: AtomicU32::new(0),
        }
    }

    /// 데우기 한 장이 쓸 입력 (seq, ts). ★이어지는 번호라 rewriter 의 offset 이 그대로 산다.
    pub fn next_priming(&self) -> (u16, u32) {
        let n = self.priming_seq.fetch_add(1, Ordering::Relaxed);
        (n as u16, n.wrapping_mul(crate::media::priming::FRAME_TS_STEP))
    }

    pub fn rewriter(&self, kind: MediaKind) -> &Rewriter {
        match kind {
            MediaKind::Audio => &self.audio_rewriter,
            MediaKind::Video => &self.video_rewriter,
        }
    }

    pub fn video(&self) -> Option<Arc<PublisherStream>> {
        self.video.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 정§6-2 #5 — 그 방의 무전 코덱. 비어 있으면 첫 화자가 정한다.
    pub fn video_codec(&self) -> Option<(&'static str, Option<String>)> {
        self.video().map(|v| (v.codec, v.fmtp.clone()))
    }

    /// 첫 화자면 슬롯을 만들고 그 값이 방의 무전 코덱이 된다. 반환: 새로 생겼나(= `TRACK_EVENT{add}` 를 낼 때).
    pub fn ensure_video(&self, room_id: &str, codec: &'static str, fmtp: Option<String>) -> (Arc<PublisherStream>, bool) {
        let mut g = self.video.lock().unwrap_or_else(|e| e.into_inner());
        match g.clone() {
            Some(v) => (v, false),
            None => {
                let v = slot_stream(room_id, MediaKind::Video, codec, fmtp);
                *g = Some(v.clone());
                (v, true)
            }
        }
    }

    /// 정§17-2 ⑥ — 반이중 video 보유자가 전원 빠지면 슬롯 코덱을 리셋하고 항목을 지운다.
    /// 다음 화자가 새로 정하며 그때 `add` 로 다시 생긴다. 반환: 지운 슬롯.
    pub fn reset_video(&self) -> Option<Arc<PublisherStream>> {
        self.video_rewriter.reset();
        self.video.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    pub fn all(&self) -> Vec<Arc<PublisherStream>> {
        let mut out = vec![self.audio.clone()];
        out.extend(self.video());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_lives_with_the_room_video_is_decided_by_the_first_speaker() {
        let s = SlotSet::new("r1");
        assert_eq!(s.audio.track_id, "ptt-r1-audio");
        assert_eq!((s.audio.codec, s.audio.duplex(), s.audio.owner.as_str()), ("opus", Duplex::Half, ""));
        assert!(s.audio.vssrc >= 0x8000_0000 && s.video().is_none() && s.video_codec().is_none());
        assert_eq!(s.all().len(), 1);

        let (v, fresh) = s.ensure_video("r1", "H264", Some("profile-level-id=42e01f".into()));
        assert!(fresh && v.track_id == "ptt-r1-video");
        assert_eq!(s.video_codec(), Some(("H264", Some("profile-level-id=42e01f".to_owned()))));
        let (again, fresh) = s.ensure_video("r1", "VP8", None);
        assert!(!fresh && again.codec == "H264", "뒤에 오는 화자가 첫 화자를 따른다");
        assert_eq!(s.all().len(), 2);

        assert!(s.reset_video().is_some() && s.video().is_none(), "전원 빠지면 다음 화자가 새로 정한다");
        let (v2, fresh) = s.ensure_video("r1", "VP8", None);
        assert!(fresh && v2.codec == "VP8" && v2.vssrc != v.vssrc);
    }

    fn half(kind: MediaKind, codec: &'static str, fmtp: Option<&str>) -> Arc<PublisherStream> {
        PublisherStream::create(StreamSpec {
            track_id: "t".into(), vssrc: 1, owner: "u1".into(), room_id: "r1".into(), kind,
            mid: "0".into(), pt: 96, rtx_pt: None, codec, fmtp: fmtp.map(str::to_owned),
            source: None, duplex: Duplex::Half, simulcast: false, ssrc: 1, rtx_ssrc: None,
        })
    }

    /// 정§9-7 — video 는 화자 코덱 == 슬롯 코덱일 때만 결합한다. 미결합이면 흘릴 소스가 없다.
    ///
    /// ★이 판정을 정체 감지(정§14-3)가 안 봐서, 코덱 불일치로 거절된 화자가 발언하는 동안
    /// 청취자에게 "재동기하라" 가 갔다. 이제 두 곳이 이 함수 하나를 쓴다.
    #[test]
    fn an_unbound_slot_has_nothing_to_flow() {
        let s = SlotSet::new("r1");
        assert!(bound_slot(&s, &half(MediaKind::Audio, "opus", None)).is_some(), "audio 는 방과 수명이 같다");
        assert!(bound_slot(&s, &half(MediaKind::Video, "VP8", None)).is_none(), "video 슬롯이 아직 없다");

        s.ensure_video("r1", "H264", Some("profile-level-id=42e01f".into()));
        assert!(bound_slot(&s, &half(MediaKind::Video, "H264", Some("profile-level-id=42e01f"))).is_some());
        assert!(bound_slot(&s, &half(MediaKind::Video, "VP8", None)).is_none(), "★코덱이 다르면 미결합");
        assert!(bound_slot(&s, &half(MediaKind::Video, "H264", None)).is_none(), "★fmtp 까지가 계약이다");

        let full = PublisherStream::create(StreamSpec {
            track_id: "t2".into(), vssrc: 2, owner: "u1".into(), room_id: "r1".into(),
            kind: MediaKind::Audio, mid: "1".into(), pt: 111, rtx_pt: None, codec: "opus",
            fmtp: None, source: None, duplex: Duplex::Full, simulcast: false, ssrc: 2, rtx_ssrc: None,
        });
        assert!(bound_slot(&s, &full).is_none(), "전이중은 슬롯을 안 탄다");
    }

    #[test]
    fn slot_entries_carry_no_owner() {
        assert_eq!(slot_track_id("room-9", MediaKind::Video), "ptt-room-9-video");
        let a = new_vssrc();
        assert!(a >= 0x8000_0000 && a != new_vssrc());
    }
}
