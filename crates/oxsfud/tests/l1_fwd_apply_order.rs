// author: kodeholic (powered by Claude)
// spec: v1.10 · 정:RULE-FWD-MEDIA-WORKERS · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FWD-SOURCE-SWITCH · model: claude-opus-5-5
//! 1층 — 워커 사이로 넘긴 묶음은 받는 워커가 적용하는 차례에 다시 판단한다. 슬롯 묶음은 게이트를 지난 일화가 그대로인지 · 줄은 판단한 뒤 다시 실리지 않았는지 본다(링 사이 차례가 없어 앞 화자의 묶음이 새 화자 패킷 뒤에 적용되면 재기록기가 앞 출처로 되돌아간다). 결합은 첫 RTP 기록 · 재전송 문 · 예열을 먼저 놓고 화자를 마지막에 보인다(context 202611/20261111a).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use oxsfud::fwd::core::Rewriter;
use oxsfud::fwd::{DownRow, SlotFan, row_same};
use oxsfud::link::{Role, Transport};
use oxsig::msg::Kind;

fn who(u: &str) -> Option<Arc<String>> {
    Some(Arc::new(u.to_string()))
}

#[test]
fn slot_packet_admitted_before_release_is_dropped_when_applied_after_it() {
    let fan = SlotFan::new(true);
    fan.bind(who("a"), false);
    let ep = fan.admit("a").expect("speaker passes the gate");
    assert!(fan.frame_live(ep, false), "정:RULE-FWD-MEDIA-WORKERS packet of the current episode is sent");
    fan.release();
    assert!(fan.admit("a").is_none(), "정:IMPL-FLR-MEDIA-BIND released speaker does not pass the gate");
    assert!(!fan.frame_live(ep, false), "정:RULE-FWD-MEDIA-WORKERS S-sym-IMPL-FLR-MEDIA-BIND-7 packet admitted before release is not sent when applied after it");
}

#[test]
fn old_speaker_packet_is_dropped_after_succession_and_new_speaker_packet_is_kept() {
    let fan = SlotFan::new(true);
    fan.bind(who("a"), false);
    let old = fan.admit("a").expect("a passes");
    fan.bind(who("b"), false);
    let new = fan.admit("b").expect("b passes");
    assert!(fan.admit("a").is_none(), "정:IMPL-FLR-MEDIA-BIND only the new speaker passes after succession");
    assert!(!fan.frame_live(old, false), "정:RULE-FWD-MEDIA-WORKERS S-sym-IMPL-FLR-MEDIA-BIND-7 old speaker packet applied after succession is not sent");
    assert!(fan.frame_live(new, false), "정:RULE-FWD-MEDIA-WORKERS new speaker packet is sent");
}

#[test]
fn rewriter_goes_back_to_the_old_source_when_its_late_packet_is_applied() {
    let (a, b) = (11u64, 22u64);
    let mut rw = Rewriter::new(48_000);
    for i in 0..5u16 {
        rw.map(a, 100 + i, 9_600 * i as u32, 1_000 + 20 * i as u64, 960).unwrap();
    }
    let b1 = rw.map(b, 7_000, 500_000, 1_200, 960).unwrap();
    assert!(b1.switched, "정:IMPL-FWD-SOURCE-SWITCH new speaker opens a switch");
    let late = rw.map(a, 105, 48_000, 1_200, 960).unwrap();
    assert!(late.switched, "정:IMPL-FWD-SOURCE-SWITCH a late old-source packet switches back — the slot carries the old speaker again");
    let b2 = rw.map(b, 7_001, 500_960, 1_220, 960).unwrap();
    assert!(b2.switched, "정:IMPL-FWD-SOURCE-SWITCH and the new speaker switches once more");
    assert_eq!(rw.switches, 3, "정:IMPL-FWD-SOURCE-SWITCH S-sym-IMPL-FLR-MEDIA-BIND-7 one floor change became three source switches");
    assert_eq!(b2.ts.wrapping_sub(b1.ts), 1_920, "정:IMPL-FWD-SOURCE-SWITCH the new speaker's audio is pushed 20ms later than it was sent");
}

#[test]
fn row_reloaded_after_the_decision_is_not_sent() {
    let tr = Transport::new(9, "s", "", Role::Both, 0, false);
    let acked = Arc::new(AtomicU64::new(3));
    let row = DownRow::create(tr, "0".into(), Kind::Audio, false, 1, None, acked.clone());
    row.since.store(3, Ordering::Release);
    let rev = row.rev();
    assert!(row.open() && row_same(&row, rev), "정:RULE-FWD-MEDIA-WORKERS row open at the decision");
    row.since.store(4, Ordering::Release);
    assert!(!row_same(&row, rev), "정:RULE-FWD-MEDIA-WORKERS row reloaded after the decision is not sent");
    acked.store(4, Ordering::Release);
    assert!(!row_same(&row, rev), "정:RULE-FWD-MEDIA-WORKERS even after the new load is acked the old decision stays stale");
}

#[test]
fn bind_shows_the_speaker_only_after_the_first_rtp_record_is_cleared() {
    let fan = Arc::new(SlotFan::new(true));
    let stop = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(AtomicU64::new(0));
    let reader = {
        let (fan, stop, seen) = (fan.clone(), stop.clone(), seen.clone());
        std::thread::spawn(move || {
            let mut bad = 0u64;
            while !stop.load(Ordering::Acquire) {
                let Some(sp) = fan.speaker.load_full() else { continue };
                let j: u64 = sp.parse().unwrap_or(0);
                if fan.first_rtp_us.load(Ordering::Acquire) == j {
                    bad += 1;
                }
                seen.fetch_add(1, Ordering::Relaxed);
            }
            bad
        })
    };
    for i in 1..=200_000u64 {
        fan.first_rtp_us.store(i, Ordering::Release);
        fan.bind(Some(Arc::new(i.to_string())), false);
    }
    stop.store(true, Ordering::Release);
    let bad = reader.join().unwrap();
    assert!(seen.load(Ordering::Relaxed) > 0, "reader observed the slot");
    assert_eq!(bad, 0, "정:IMPL-FLR-MEDIA-BIND S-sym-IMPL-FLR-MEDIA-BIND-6 the new speaker is never visible with the previous episode's first-RTP record");
}
