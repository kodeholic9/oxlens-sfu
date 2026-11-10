// author: kodeholic (powered by Claude)
// spec: v1.9 · 정:IMPL-FLR-MEDIA-BIND · 정:RULE-FWD-MEDIA-WORKERS · model: claude-opus-5-5
//! 1층 — 반환 뒤 PLI 는 내보내는 차례에 화자인지 본다(미뤄진 PLI 가 반환 뒤 살아남는 경합 · 2층 `ptt_video_quick_release`) · 예열 · 해제 무음 프레임의 멈춤은 워커가 그 프레임을 적용하는 차례에서 판단한다. 워커는 제어 링을 마지막에 비우므로 먼저 넣은 예열 프레임이 뒤에 온 화자 첫 RTP 보다 늦게 적용될 수 있다(2층 `peer_record_raw` 발언 갈림 · context 202611/20261110e 3-#2).

use oxsfud::fwd::SlotFan;
use std::sync::atomic::Ordering;

#[test]
fn prime_frame_queued_before_first_rtp_is_dropped_when_applied_after_it() {
    let fan = SlotFan::new(true);
    let ep = fan.episode.load(Ordering::Acquire);
    assert!(fan.frame_live(ep, true), "정:IMPL-FLR-MEDIA-BIND prime frame is live when queued");
    assert!(fan.pass_gate(1_000), "정:IMPL-FLR-MEDIA-BIND first speaker RTP passes the gate once");
    assert!(!fan.pass_gate(1_020), "정:IMPL-FLR-MEDIA-BIND only the first RTP is first");
    assert!(!fan.frame_live(ep, true), "정:IMPL-FLR-MEDIA-BIND prime frame applied after the first RTP is not sent");
    assert_eq!(fan.priming.load(Ordering::Acquire), 0, "정:IMPL-FLR-MEDIA-BIND gate stops priming");
}

#[test]
fn release_silence_is_kept_within_its_episode_and_dropped_after_rebind() {
    let fan = SlotFan::new(true);
    let ep = fan.episode.load(Ordering::Acquire);
    fan.pass_gate(1_000);
    assert!(fan.frame_live(ep, false), "정:IMPL-FLR-MEDIA-BIND release silence of the same episode is sent");
    fan.episode.fetch_add(1, Ordering::AcqRel);
    assert!(!fan.frame_live(ep, false), "정:IMPL-FLR-MEDIA-BIND release silence of an old episode is not sent after rebind");
    assert!(!fan.frame_live(ep, true), "정:IMPL-FLR-MEDIA-BIND prime frame of an old episode is not sent after rebind");
}

#[test]
fn pli_for_half_duplex_video_is_sent_only_while_bound() {
    use oxsfud::rtcp::pli_wanted;
    let fan = SlotFan::new(false);
    fan.speaker.store(Some(std::sync::Arc::new("a".to_string())));
    assert!(pli_wanted(true, Some(&fan), "a"), "정:IMPL-FLR-MEDIA-BIND bound speaker gets PLI");
    assert!(!pli_wanted(true, Some(&fan), "b"), "정:IMPL-FLR-MEDIA-BIND other user's half track gets none");
    fan.speaker.store(None);
    assert!(!pli_wanted(true, Some(&fan), "a"), "정:IMPL-FLR-MEDIA-BIND pending PLI is not sent after release");
    assert!(!pli_wanted(true, None, "a"), "정:IMPL-FLR-MEDIA-BIND no slot means no PLI for half track");
    assert!(pli_wanted(false, None, "a"), "정:IMPL-FLR-MEDIA-BIND full duplex is not gated by the slot");
}
