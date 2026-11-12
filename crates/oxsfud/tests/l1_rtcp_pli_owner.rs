// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-RTCP-TERMINATE · 정:RULE-FWD-MEDIA-WORKERS · model: claude-opus-5-5
//! 1층 — PLI 요청은 발행 트랙의 표시만 세운다(버려질 메시지가 없다). 스로틀 · 미룸 · 보내기는 발행 전송 객체의 워커가 tick 에서 한 차례로 판단한다 — 스로틀이 오르는 것은 그 자리에서 보낼 때뿐이다(context 202611/20261112d · 옛 꼴은 부른 스레드가 스로틀을 올린 뒤 제어 링에 넣어 링이 차면 PLI 는 잃고 스로틀만 남았다).

use oxsfud::r#pub::PliState;
use oxsfud::rtcp::{PLI_INFRA, PLI_USER, PliGo, pli_decide};

#[test]
fn the_throttle_moves_only_with_a_sent_pli() {
    let mut p = PliState::default();
    assert_eq!(pli_decide(&mut p, PLI_INFRA, true, 1_000, 500), PliGo::Send, "정:IMPL-RTCP-TERMINATE the first request is sent");
    assert_eq!((p.last_ms, p.sent, p.pending), (1_000, 1, false), "정:IMPL-RTCP-TERMINATE S-sym-IMPL-RTCP-TERMINATE-9 the throttle moves in the same step that sends");
    assert_eq!(pli_decide(&mut p, 0, true, 1_100, 500), PliGo::Idle, "정:IMPL-RTCP-TERMINATE nothing asked nothing pending");
    assert_eq!((p.last_ms, p.sent), (1_000, 1), "정:IMPL-RTCP-TERMINATE an idle tick leaves the throttle");
}

#[test]
fn an_infra_request_inside_the_throttle_window_is_held_and_sent_when_it_opens() {
    let mut p = PliState { last_ms: 1_000, pending: false, sent: 1 };
    assert_eq!(pli_decide(&mut p, PLI_INFRA, true, 1_200, 500), PliGo::Hold, "정:IMPL-RTCP-TERMINATE infra request inside the window is held");
    assert!(p.pending, "정:IMPL-RTCP-TERMINATE held as pending");
    assert_eq!(pli_decide(&mut p, 0, true, 1_400, 500), PliGo::Hold, "정:IMPL-RTCP-TERMINATE still inside the window");
    assert_eq!(pli_decide(&mut p, 0, true, 1_500, 500), PliGo::Send, "정:IMPL-RTCP-TERMINATE the pending request goes out when the window opens");
    assert_eq!((p.pending, p.sent), (false, 2), "정:IMPL-RTCP-TERMINATE one PLI for the held request");
}

#[test]
fn a_subscriber_request_inside_the_window_is_dropped_and_counted() {
    let mut p = PliState { last_ms: 1_000, pending: false, sent: 1 };
    assert_eq!(pli_decide(&mut p, PLI_USER, true, 1_200, 500), PliGo::Throttled, "정:IMPL-RTCP-TERMINATE subscriber PLI inside the window is dropped");
    assert!(!p.pending, "정:IMPL-RTCP-TERMINATE a dropped subscriber request leaves nothing pending");
    assert_eq!(pli_decide(&mut p, PLI_USER | PLI_INFRA, true, 1_300, 500), PliGo::Hold, "정:IMPL-RTCP-TERMINATE with an infra request in the same tick it is held");
}

#[test]
fn an_unbound_half_duplex_track_gets_no_pli_and_drops_what_was_pending() {
    let mut p = PliState { last_ms: 1_000, pending: true, sent: 1 };
    assert_eq!(pli_decide(&mut p, PLI_INFRA, false, 5_000, 500), PliGo::Unbound, "정:IMPL-FLR-MEDIA-BIND a released speaker's track gets no PLI at the sending step");
    assert!(!p.pending, "정:IMPL-FLR-MEDIA-BIND what was pending is dropped");
    assert_eq!(pli_decide(&mut p, 0, true, 6_000, 500), PliGo::Idle, "정:IMPL-FLR-MEDIA-BIND nothing comes back later");
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
