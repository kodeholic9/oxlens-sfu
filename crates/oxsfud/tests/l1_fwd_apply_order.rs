// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-FWD-MEDIA-WORKERS · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FWD-SOURCE-SWITCH · model: claude-opus-5-5
//! 1층 — 워커 사이로 넘긴 묶음은 받는 워커가 적용하는 차례에 다시 판단한다. 슬롯 묶음은 게이트를 지난 일화가 그대로인지 · 줄은 판단한 뒤 다시 실리지 않았는지 본다(링 사이 차례가 없어 앞 화자의 묶음이 새 화자 패킷 뒤에 적용되면 재기록기가 앞 출처로 되돌아간다). 슬롯 상태는 일화 묶음 하나를 통째로 바꿔 끼운다 · 재전송 문은 줄이 마지막으로 보낸 일화와 지금 일화로 유도한다 · 줄의 재기록기 출처 · 실린 트랙은 그 워커가 줄 명령으로만 바꾸고 묶음은 명령을 적용한 세대와 견준다(층 ③)(같은 사람 재허가도 새 일화 첫 패킷에서 열린다 · R16) · 반이중 영상 PLI 는 보내는 차례에 화자인지 본다(context 202611/20261111a · 20261111c 층 ①).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use oxsfud::fwd::core::Rewriter;
use oxsfud::fwd::rt::{CTL_RING, DEFAULT_QUEUE_PACKETS};
use oxsfud::fwd::{DownRow, RowCmd, RowCmds, SlotFan, apply_row_cmd, row_same, rtx_closed};
use oxsfud::link::{Role, Transport};
use oxsig::msg::Kind;

fn who(u: &str) -> Option<Arc<String>> {
    Some(Arc::new(u.to_string()))
}

#[test]
fn slot_packet_admitted_before_release_is_dropped_when_applied_after_it() {
    let fan = SlotFan::new(true);
    fan.bind(who("a"), 1);
    let ep = fan.admit("a").expect("speaker passes the gate").id;
    assert!(fan.live(ep), "정:RULE-FWD-MEDIA-WORKERS packet of the current episode is sent");
    fan.release();
    assert!(fan.admit("a").is_none(), "정:IMPL-FLR-MEDIA-BIND released speaker does not pass the gate");
    assert!(!fan.live(ep), "정:RULE-FWD-MEDIA-WORKERS S-sym-IMPL-FLR-MEDIA-BIND-7 packet admitted before release is not sent when applied after it");
}

#[test]
fn old_speaker_packet_is_dropped_after_succession_and_new_speaker_packet_is_kept() {
    let fan = SlotFan::new(true);
    fan.bind(who("a"), 2);
    let old = fan.admit("a").expect("a passes").id;
    fan.bind(who("b"), 3);
    let new = fan.admit("b").expect("b passes").id;
    assert!(fan.admit("a").is_none(), "정:IMPL-FLR-MEDIA-BIND only the new speaker passes after succession");
    assert!(!fan.live(old), "정:RULE-FWD-MEDIA-WORKERS S-sym-IMPL-FLR-MEDIA-BIND-7 old speaker packet applied after succession is not sent");
    assert!(fan.live(new), "정:RULE-FWD-MEDIA-WORKERS new speaker packet is sent");
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
    apply_row_cmd(&row, 3, false);
    let rev = row.rev();
    assert!(row.open() && row_same(&row, rev), "정:RULE-FWD-MEDIA-WORKERS row open at the decision");
    row.since.store(4, Ordering::Release);
    apply_row_cmd(&row, 4, true);
    assert!(!row_same(&row, rev), "정:RULE-FWD-MEDIA-WORKERS row reloaded after the decision is not sent");
    acked.store(4, Ordering::Release);
    assert!(!row_same(&row, rev), "정:RULE-FWD-MEDIA-WORKERS even after the new load is acked the old decision stays stale");
}

fn row_with_source(src: u64) -> DownRow {
    let tr = Transport::new(9, "s", "", Role::Both, 0, false);
    let row = DownRow::create(tr, "0".into(), Kind::Audio, false, 1, None, Arc::new(AtomicU64::new(1)));
    row.since.store(1, Ordering::Release);
    apply_row_cmd(&row, 1, false);
    let mut rw = row.rw.lock().unwrap();
    for i in 0..3u16 {
        rw.map(src, 100 + i, 960 * i as u32, 1_000 + 20 * i as u64, 960).unwrap();
    }
    drop(rw);
    row
}

#[test]
fn control_never_writes_the_row_rewriter_or_its_source_track() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut bad = Vec::new();
    for dir in ["ops", "sub", "flr", "gc", "room"] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        for e in rd.flatten() {
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            for (n, line) in text.lines().enumerate() {
                if line.contains("rw.src =") || line.contains("src_track.store(") {
                    bad.push(format!("{}:{}", e.path().display(), n + 1));
                }
            }
        }
    }
    assert!(bad.is_empty(), "정:RULE-FWD-MEDIA-WORKERS S-sym-RULE-FWD-MEDIA-WORKERS-16 only the row's worker writes the rewriter source and the source track: {bad:?}");
}

#[test]
fn reassign_leaves_the_flowing_source_until_the_row_worker_applies_the_command() {
    let row = row_with_source(11);
    let decided = row.rev();
    row.since.store(2, Ordering::Release);
    assert_eq!(row.rw.lock().unwrap().src, Some(11), "정:RULE-FWD-MEDIA-WORKERS S-sym-RULE-FWD-MEDIA-WORKERS-16 closing the row does not touch the rewriter");
    assert!(row_same(&row, decided), "정:RULE-FWD-MEDIA-WORKERS a bundle decided before the command still goes out on the source it was decided for");
    assert!(!row_same(&row, 2), "정:RULE-FWD-MEDIA-WORKERS S-sym-RULE-FWD-MEDIA-WORKERS-18 a new-generation bundle that arrives before the row command is row_stale");
    assert!(apply_row_cmd(&row, 2, true));
    assert_eq!(row.rw.lock().unwrap().src, None, "정:RULE-FWD-MEDIA-WORKERS the row worker opens the source when it applies the command");
    assert!(row_same(&row, 2) && !row_same(&row, decided), "정:RULE-FWD-MEDIA-WORKERS S-sym-RULE-FWD-MEDIA-WORKERS-18 after the command only the new generation goes out");
}

#[test]
fn row_commands_beyond_the_packet_limit_and_the_ring_are_never_dropped() {
    let row = Arc::new(row_with_source(11));
    let q = RowCmds::default();
    let n = (DEFAULT_QUEUE_PACKETS + CTL_RING) as u64;
    for rev in 2..2 + n {
        q.push(RowCmd { row: row.clone(), rev, reset: true });
    }
    assert_eq!(q.drain() as u64, n, "정:RULE-FWD-MEDIA-WORKERS S-sym-RULE-FWD-MEDIA-WORKERS-17 every row command is applied");
    assert!(q.is_empty() && row_same(&row, 1 + n), "정:RULE-FWD-MEDIA-WORKERS S-sym-RULE-FWD-MEDIA-WORKERS-17 the last generation is the applied one");
}

#[test]
fn an_empty_row_command_queue_is_skipped_and_a_posted_command_is_applied_on_the_next_drain() {
    let row = Arc::new(row_with_source(11));
    let q = RowCmds::default();
    assert!(q.is_empty() && q.drain() == 0, "정:RULE-FWD-MEDIA-WORKERS an empty row command queue costs the worker loop nothing");
    q.push(RowCmd { row: row.clone(), rev: 2, reset: true });
    assert!(!q.is_empty(), "정:RULE-FWD-MEDIA-WORKERS a posted command keeps the worker from parking");
    assert_eq!(q.drain(), 1, "정:RULE-FWD-MEDIA-WORKERS S-sym-RULE-FWD-MEDIA-WORKERS-17 the posted command is applied on the next drain");
    assert!(q.is_empty() && row_same(&row, 2), "정:RULE-FWD-MEDIA-WORKERS the queue is empty again after the drain");
}

#[test]
fn an_older_row_command_arriving_late_is_skipped() {
    let row = row_with_source(11);
    assert!(apply_row_cmd(&row, 5, true));
    row.rw.lock().unwrap().map(22, 7_000, 0, 2_000, 960).unwrap();
    assert!(!apply_row_cmd(&row, 4, true), "정:IMPL-SUB-TABLE S-sym-IMPL-SUB-TABLE-9 an older command is skipped");
    assert!(row_same(&row, 5), "정:IMPL-SUB-TABLE S-sym-IMPL-SUB-TABLE-9 the applied generation never goes down");
    assert_eq!(row.rw.lock().unwrap().src, Some(22), "정:IMPL-SUB-TABLE S-sym-IMPL-SUB-TABLE-9 the skipped command does not clear the flowing source");
}

#[test]
fn gate_never_sees_a_new_speaker_with_the_previous_episode_first_rtp_record() {
    let fan = Arc::new(SlotFan::new(true));
    let stop = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(AtomicU64::new(0));
    let reader = {
        let (fan, stop, seen) = (fan.clone(), stop.clone(), seen.clone());
        std::thread::spawn(move || {
            let mut bad = 0u64;
            while !stop.load(Ordering::Acquire) {
                let ep = fan.ep.load_full();
                let Some(sp) = ep.speaker.as_deref() else { continue };
                let j: u64 = sp.parse().unwrap_or(0);
                if ep.first_rtp_us.load(Ordering::Acquire) == j {
                    bad += 1;
                }
                seen.fetch_add(1, Ordering::Relaxed);
            }
            bad
        })
    };
    for i in 1..=200_000u64 {
        fan.ep.load().first_rtp_us.store(i, Ordering::Release);
        fan.bind(Some(Arc::new(i.to_string())), i);
    }
    stop.store(true, Ordering::Release);
    let bad = reader.join().unwrap();
    assert!(seen.load(Ordering::Relaxed) > 0, "reader observed the slot");
    assert_eq!(bad, 0, "정:IMPL-FLR-MEDIA-BIND S-sym-IMPL-FLR-MEDIA-BIND-6 the gate reads one episode bundle — a new speaker never comes with the previous episode's first-RTP record");
}

#[test]
fn regrant_of_the_same_speaker_opens_the_rtx_gate_with_its_first_packet() {
    let fan = SlotFan::new(false);
    let mut rw = Rewriter::new(90_000);
    let a = 7u64;
    let e1 = fan.bind(who("a"), 4);
    let first = rw.map(a, 100, 0, 1_000, 1).unwrap();
    let mut last = e1;
    assert!(!rtx_closed(last, fan.episode()), "정:RULE-RTCP-NACK-DOWN gate open after the speaker's first packet");
    fan.release();
    assert!(rtx_closed(last, fan.episode()), "정:RULE-RTCP-NACK-DOWN gate closed after release");
    let e3 = fan.bind(who("a"), 5);
    assert!(rtx_closed(last, fan.episode()), "정:RULE-RTCP-NACK-DOWN gate closed until the regranted speaker's first packet");
    let again = rw.map(a, 101, 3_000, 4_000, 1).unwrap();
    let _ = first;
    assert!(!again.switched, "정:IMPL-FWD-SOURCE-SWITCH S-sym-IMPL-FLR-MEDIA-BIND-8 the same speaker is not a source switch — the old gate (opened only on a switch) stayed closed for the whole talk");
    last = e3;
    assert!(!rtx_closed(last, fan.episode()), "정:RULE-RTCP-NACK-DOWN S-sym-IMPL-FLR-MEDIA-BIND-8 S-sym-RULE-RTCP-NACK-DOWN-1 the regranted speaker's first packet opens the gate — it is a new episode");
}

#[test]
fn pli_for_half_duplex_video_is_sent_only_while_bound() {
    use oxsfud::rtcp::pli_wanted;
    let fan = SlotFan::new(false);
    fan.bind(who("a"), 6);
    assert!(pli_wanted(true, Some(&fan), "a"), "정:IMPL-FLR-MEDIA-BIND bound speaker gets PLI");
    assert!(!pli_wanted(true, Some(&fan), "b"), "정:IMPL-FLR-MEDIA-BIND other user's half track gets none");
    fan.release();
    assert!(!pli_wanted(true, Some(&fan), "a"), "정:IMPL-FLR-MEDIA-BIND pending PLI is not sent after release");
    assert!(!pli_wanted(true, None, "a"), "정:IMPL-FLR-MEDIA-BIND no slot means no PLI for half track");
    assert!(pli_wanted(false, None, "a"), "정:IMPL-FLR-MEDIA-BIND full duplex is not gated by the slot");
}

#[test]
fn rtp_observation_is_recorded_only_under_the_grant_that_admitted_it() {
    use oxsfud::ops::floor::obs_recorded;
    assert!(obs_recorded(2, 2), "정:IMPL-FLR-RTP-OBSERVE packet admitted under the current grant feeds its clock");
    assert!(!obs_recorded(1, 2), "정:IMPL-FLR-RTP-OBSERVE S-sym-IMPL-FLR-RTP-OBSERVE-4 the previous speaker's packet between the judgment and its execution does not feed the new grant's clock");
    assert!(!obs_recorded(0, 0), "정:IMPL-FLR-RTP-OBSERVE a release episode carries no grant");
    let fan = SlotFan::new(true);
    fan.bind(who("a"), 7);
    let ep = fan.admit("a").expect("a passes");
    assert_eq!(ep.grant, 7, "정:IMPL-FLR-RTP-OBSERVE the episode carries the grant number of its bind");
    fan.release();
    assert_eq!(fan.ep.load().grant, 0, "정:IMPL-FLR-RTP-OBSERVE release episode has no grant");
}
