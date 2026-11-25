// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-RTCP-TERMINATE · 정:RULE-FWD-MEDIA-WORKERS · model: claude-opus-5-5
//! 1층 — PLI 요청은 발행 트랙의 표시(비트)라 몰려도 버려지지 않는다: 몇 번을 청해도 표시 하나로 합쳐지고, 청하는 함수는 큐에 넣지 않는다(넘칠 곳이 없다) · 창 안 인프라 요청은 미뤘다가 창이 열리면 한 번 나간다 · 스로틀은 보낸 차례에만 오른다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::atomic::Ordering;

use oxsfud::r#pub::PliState;
use oxsfud::rtcp::{PLI_INFRA, PLI_USER, PliGo, pli_decide, request_pli};
use oxsig::msg::{Duplex, Kind};

use support::*;

#[test]
fn a_pli_request_is_a_flag_that_never_drops() {
    let _g = serial();
    let r = room(&tag("plf"));
    let u = subscriber(&tag("plf-u"));
    enter(&u.p, &r);
    let (_, t) = published(&u.p, &r.id, Kind::Video, Duplex::Full);
    let drops0: u64 = sfu().drops.iter().map(|d| d.load(Ordering::Relaxed)).sum();
    for _ in 0..10_000 {
        request_pli(&t, PLI_USER);
        request_pli(&t, PLI_INFRA);
    }
    assert_eq!(t.pli_want.load(Ordering::Acquire), PLI_USER | PLI_INFRA, "S-sym-IMPL-RTCP-TERMINATE-9 twenty thousand requests merge into one flag word — none is lost");
    let drops1: u64 = sfu().drops.iter().map(|d| d.load(Ordering::Relaxed)).sum();
    assert_eq!(drops1, drops0, "S-sym-IMPL-RTCP-TERMINATE-9 a burst of requests drops nothing");
    let lines = product_lines();
    let body: Vec<&Src> = lines.iter().filter(|l| l.file == "rtcp/mod.rs" && l.func == "request_pli").collect();
    assert!(!body.is_empty(), "stand check — request_pli was read");
    let queued: Vec<String> = body.iter().filter(|l| l.text.contains(".send(") || l.text.contains(".push(") || l.text.contains("try_send")).map(|l| at(l)).collect();
    assert!(queued.is_empty(), "S-sym-IMPL-RTCP-TERMINATE-9 asking a PLI puts nothing on a queue — it only sets the flag: {queued:?}");
}

#[test]
fn a_held_infra_request_goes_out_once_when_the_window_opens_and_only_sending_moves_the_throttle() {
    let mut p = PliState { last_ms: 1_000, held_since: None, sent: 1 };
    let mut sends = 0;
    for (now, want) in [(1_100, PLI_INFRA), (1_200, PLI_INFRA), (1_300, 0), (1_499, 0)] {
        let g = pli_decide(&mut p, want, true, now, 500, 0, 0);
        assert_eq!(g, PliGo::Hold, "S-sym-IMPL-RTCP-TERMINATE-9 an infra request inside the window is held at {now}");
        assert_eq!((p.last_ms, p.sent), (1_000, 1), "S-sym-IMPL-RTCP-TERMINATE-9 holding does not move the throttle at {now}");
    }
    for now in [1_500, 1_600, 1_700] {
        if pli_decide(&mut p, 0, true, now, 500, 0, 0) == PliGo::Send {
            sends += 1;
        }
    }
    assert_eq!(sends, 1, "S-sym-IMPL-RTCP-TERMINATE-9 the held requests go out as one PLI when the window opens");
    assert_eq!((p.last_ms, p.sent, p.held_since), (1_500, 2, None), "S-sym-IMPL-RTCP-TERMINATE-9 the throttle moves in the step that sends");
}
