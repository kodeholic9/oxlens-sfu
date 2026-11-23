// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-RTCP-TERMINATE · 정:RULE-FWD-MEDIA-WORKERS · model: claude-opus-5-5
//! 1층 — PLI 요청은 발행 트랙의 표시만 세운다(버려질 메시지가 없다). 스로틀 · 미룸 · 보내기는 발행 전송 객체의 워커가 tick 에서 한 차례로 판단한다 — 스로틀이 오르는 것은 그 자리에서 보낼 때뿐이다(context 202611/20261112d · 옛 꼴은 부른 스레드가 스로틀을 올린 뒤 제어 링에 넣어 링이 차면 PLI 는 잃고 스로틀만 남았다). 미룬 요청은 그 층 키프레임이 오면 거둔다(202611/20261119c D10).

use oxsfud::r#pub::PliState;
use oxsfud::rtcp::{PLI_INFRA, PLI_USER, PliGo, pli_decide};

const NO_KEY: u64 = 0;

#[test]
fn the_throttle_moves_only_with_a_sent_pli() {
    let mut p = PliState::default();
    assert_eq!(pli_decide(&mut p, PLI_INFRA, true, 1_000, 500, NO_KEY, 0), PliGo::Send, "정:IMPL-RTCP-TERMINATE the first request is sent");
    assert_eq!((p.last_ms, p.sent, p.held_since), (1_000, 1, None), "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-9 the throttle moves in the same step that sends");
    assert_eq!(pli_decide(&mut p, 0, true, 1_100, 500, NO_KEY, 0), PliGo::Idle, "정:IMPL-RTCP-TERMINATE nothing asked nothing pending");
    assert_eq!((p.last_ms, p.sent), (1_000, 1), "정:IMPL-RTCP-TERMINATE an idle tick leaves the throttle");
}

#[test]
fn an_infra_request_inside_the_throttle_window_is_held_and_sent_when_it_opens() {
    let mut p = PliState { last_ms: 1_000, held_since: None, sent: 1 };
    assert_eq!(pli_decide(&mut p, PLI_INFRA, true, 1_200, 500, NO_KEY, 0), PliGo::Hold, "정:IMPL-RTCP-TERMINATE infra request inside the window is held");
    assert_eq!(p.held_since, Some(1_200), "정:IMPL-RTCP-TERMINATE held since the step that held it");
    assert_eq!(pli_decide(&mut p, PLI_INFRA, true, 1_300, 500, NO_KEY, 0), PliGo::Hold, "정:IMPL-RTCP-TERMINATE a second request joins the held one");
    assert_eq!(p.held_since, Some(1_200), "정:IMPL-RTCP-TERMINATE joined requests keep the first hold time");
    assert_eq!(pli_decide(&mut p, 0, true, 1_400, 500, NO_KEY, 0), PliGo::Hold, "정:IMPL-RTCP-TERMINATE still inside the window");
    assert_eq!(pli_decide(&mut p, 0, true, 1_500, 500, NO_KEY, 0), PliGo::Send, "정:IMPL-RTCP-TERMINATE the pending request goes out when the window opens");
    assert_eq!((p.held_since, p.sent), (None, 2), "정:IMPL-RTCP-TERMINATE one PLI for the held request");
}

#[test]
fn a_held_request_answered_by_a_keyframe_is_withdrawn() {
    let mut p = PliState { last_ms: 1_000, held_since: None, sent: 1 };
    assert_eq!(pli_decide(&mut p, PLI_INFRA, true, 1_005, 500, 900, 0), PliGo::Hold, "정:IMPL-RTCP-TERMINATE a request right after the sent one is held");
    assert_eq!(pli_decide(&mut p, 0, true, 1_150, 500, 1_150, 0), PliGo::Answered, "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-10 the layer's keyframe arrives inside the window — the held request is withdrawn");
    assert_eq!((p.held_since, p.sent, p.last_ms), (None, 1, 1_000), "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-10 nothing is sent and the throttle stays");
    assert_eq!(pli_decide(&mut p, 0, true, 1_500, 500, 1_150, 0), PliGo::Idle, "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-10 no second PLI when the window opens");
}

#[test]
fn a_keyframe_older_than_the_hold_does_not_answer_it() {
    let mut p = PliState { last_ms: 1_000, held_since: None, sent: 1 };
    assert_eq!(pli_decide(&mut p, PLI_INFRA, true, 1_200, 500, 1_100, 0), PliGo::Hold, "정:IMPL-RTCP-TERMINATE held after the last keyframe");
    assert_eq!(pli_decide(&mut p, 0, true, 1_250, 500, 1_100, 0), PliGo::Hold, "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-10 a keyframe from before the hold is not its answer");
    assert_eq!(pli_decide(&mut p, 0, true, 1_300, 500, 1_100, 0), PliGo::Send, "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-12 the unanswered request goes out once the keyframe after the last PLI and the engine floor reopen the window");
}

#[test]
fn a_keyframe_after_the_last_pli_reopens_the_window_after_the_engine_floor() {
    use oxsfud::rtcp::ENGINE_FLOOR_MS;
    let mut p = PliState { last_ms: 1_000, held_since: None, sent: 1 };
    assert_eq!(pli_decide(&mut p, PLI_INFRA, true, 1_041, 1_000, 1_040, 0), PliGo::Hold, "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-12 inside the engine floor the request waits — the publisher engine would drop it");
    assert_eq!(pli_decide(&mut p, 0, true, 1_000 + ENGINE_FLOOR_MS - 1, 1_000, 1_040, 0), PliGo::Hold, "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-12 still inside the floor");
    assert_eq!(pli_decide(&mut p, 0, true, 1_000 + ENGINE_FLOOR_MS, 1_000, 1_040, 0), PliGo::Send, "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-11 the keyframe reopened the window — no wait for the 1,000ms gap");
    let mut q = PliState { last_ms: 1_000, held_since: None, sent: 1 };
    assert_eq!(pli_decide(&mut q, PLI_INFRA, true, 1_400, 1_000, 1_000, 0), PliGo::Hold, "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-11 a keyframe at the same instant as the last PLI is not its answer");
    let mut r = PliState { last_ms: 1_000, held_since: None, sent: 1 };
    assert_eq!(pli_decide(&mut r, PLI_USER, true, 1_350, 1_000, 1_040, 0), PliGo::Throttled, "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-13 a subscriber request does not reopen the window — the keyframe interval stays at the gap");
    assert_eq!(pli_decide(&mut r, PLI_USER, true, 2_000, 1_000, 1_040, 0), PliGo::Send, "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-13 the subscriber request goes out once the gap passes");
    let mut h = PliState { last_ms: 1_000, held_since: None, sent: 1 };
    assert_eq!(pli_decide(&mut h, PLI_INFRA, true, 1_020, 1_000, 0, 0), PliGo::Hold, "정:IMPL-RTCP-TERMINATE infra request inside the gap waits");
    assert_eq!(pli_decide(&mut h, PLI_USER, true, 1_000 + ENGINE_FLOOR_MS, 1_000, 1_010, 0), PliGo::Send, "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-11 a held infra request reopens with the keyframe even when only a subscriber asks this tick");
}

#[test]
fn the_reopen_margin_waits_past_the_engine_floor() {
    use oxsfud::rtcp::ENGINE_FLOOR_MS;
    let margin = common::policy::Policy::default().media.pli_reopen_margin_ms as u64;
    let mut p = PliState { last_ms: 1_000, held_since: None, sent: 1 };
    assert_eq!(pli_decide(&mut p, PLI_INFRA, true, 1_000 + ENGINE_FLOOR_MS, 1_000, 1_040, margin), PliGo::Hold, "정:IMPL-RTCP-TERMINATE PL-KEY-MEDIA-PLI-REOPEN-MARGIN-MS past the engine floor but inside the margin the request waits");
    assert_eq!(pli_decide(&mut p, 0, true, 1_000 + ENGINE_FLOOR_MS + margin - 1, 1_000, 1_040, margin), PliGo::Hold, "정:IMPL-RTCP-TERMINATE PL-KEY-MEDIA-PLI-REOPEN-MARGIN-MS still inside the margin");
    assert_eq!(pli_decide(&mut p, 0, true, 1_000 + ENGINE_FLOOR_MS + margin, 1_000, 1_040, margin), PliGo::Send, "정:IMPL-RTCP-TERMINATE PL-KEY-MEDIA-PLI-REOPEN-MARGIN-MS S-sym-IMPL-RTCP-TERMINATE-11 floor plus margin reopens the window");
}

#[test]
fn a_new_request_in_the_answering_step_is_judged_on_its_own() {
    let mut p = PliState { last_ms: 1_000, held_since: Some(1_010), sent: 1 };
    assert_eq!(pli_decide(&mut p, PLI_INFRA, true, 1_200, 500, 1_150, 0), PliGo::Hold, "정:IMPL-RTCP-TERMINATE the old hold is answered and the new request inside the window is held anew");
    assert_eq!(p.held_since, Some(1_200), "정:IMPL-RTCP-TERMINATE the new hold starts at this step");
}

#[test]
fn a_subscriber_request_inside_the_window_is_dropped_and_counted() {
    let mut p = PliState { last_ms: 1_000, held_since: None, sent: 1 };
    assert_eq!(pli_decide(&mut p, PLI_USER, true, 1_200, 500, NO_KEY, 0), PliGo::Throttled, "정:IMPL-RTCP-TERMINATE subscriber PLI inside the window is dropped");
    assert_eq!(p.held_since, None, "정:IMPL-RTCP-TERMINATE a dropped subscriber request leaves nothing pending");
    assert_eq!(pli_decide(&mut p, PLI_USER | PLI_INFRA, true, 1_300, 500, NO_KEY, 0), PliGo::Hold, "정:IMPL-RTCP-TERMINATE with an infra request in the same tick it is held");
}

#[test]
fn an_unbound_half_duplex_track_gets_no_pli_and_drops_what_was_pending() {
    let mut p = PliState { last_ms: 1_000, held_since: Some(1_100), sent: 1 };
    assert_eq!(pli_decide(&mut p, PLI_INFRA, false, 5_000, 500, NO_KEY, 0), PliGo::Unbound, "정:IMPL-FLR-MEDIA-BIND a released speaker's track gets no PLI at the sending step");
    assert_eq!(p.held_since, None, "정:IMPL-FLR-MEDIA-BIND what was pending is dropped");
    assert_eq!(pli_decide(&mut p, 0, true, 6_000, 500, NO_KEY, 0), PliGo::Idle, "정:IMPL-FLR-MEDIA-BIND nothing comes back later");
}

#[test]
fn only_the_publisher_worker_tick_writes_the_pli_state() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut bad = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            for (n, line) in text.lines().enumerate() {
                if line.contains("Ctl::Pli") || (line.contains("mut") && line.contains("pli.lock()") && !p.ends_with("rtcp/mod.rs")) {
                    bad.push(format!("{}:{}", p.display(), n + 1));
                }
            }
            if p.ends_with("rtcp/mod.rs") {
                let writers = text.lines().filter(|l| l.contains("mut") && l.contains("pli.lock()")).count();
                if writers != 1 {
                    bad.push(format!("{} has {writers} writers of the PLI state", p.display()));
                }
            }
        }
    }
    assert!(bad.is_empty(), "정:RULE-FWD-MEDIA-WORKERS S-sym-IMPL-RTCP-TERMINATE-9 the PLI throttle state has one writer — the publisher worker's tick: {bad:?}");
}
