// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-LINK-TRANSPORT · 정:RULE-GC-LEAVE-SEVEN · model: claude-opus-5-5
//! 1층 제어 층 — Peer 회수 100 번(반은 마지막 방 퇴장 · 반은 세션 폐기) 뒤 남는 것이 없다: 실제 입장이 지은 전송 객체마다 그것을 쥔 태스크(DTLS · 그 안의 SCTP association)가 다 끝나 약한 참조가 하나도 안 살아나고 · Peer · 배정표 장부와 열린 fd 가 기준선으로 돌아오며 · 쉬는 동안의 CPU 시간이 회수 전보다 늘지 않는다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::{Arc, Once, Weak};
use std::time::Duration;

use common::envelope::CallKind;
use oxsfud::fwd::rt::{self, StartCfg};
use oxsfud::link::Transport;
use oxsfud::ops::handle::Ctx;
use oxsfud::ops::{self};
use oxsfud::room::Room;
use oxsig::msg::PcMode;
use serde_json::json;

use support::*;

const CYCLES: usize = 100;
const FD_SLACK: usize = 4;
const IDLE_MS: u64 = 1_000;
const CPU_RISE_MAX_MS: f64 = 60.0;

static MEDIA: Once = Once::new();

fn media() {
    MEDIA.call_once(|| {
        let at = "127.0.0.1:0".parse().expect("addr");
        rt::start(StartCfg { bind: at, advertised: at, tcp_port: None, workers: 2, queue_limit: rt::DEFAULT_QUEUE_PACKETS }).expect("media");
    });
    assert!(rt::media().is_some(), "stand check — the media layer is up");
}

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    assert!(drive(ops::rooms::create(&e)).is_ok(), "stand check — create {id}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn ctx(user: &str) -> Ctx {
    Ctx { user: user.into(), sid: format!("sid-{user}"), session: format!("ses-{user}"), pc_mode: PcMode::Two, source_tag: false, home: None, claims: None, over: None }
}

fn fds() -> usize {
    std::fs::read_dir("/dev/fd").map(|d| d.count()).unwrap_or(0)
}

fn cpu_ms() -> f64 {
    use nix::sys::resource::{UsageWho, getrusage};
    let u = getrusage(UsageWho::RUSAGE_SELF).expect("getrusage");
    let t = |s: i64, us: i64| s as f64 * 1_000.0 + us as f64 / 1_000.0;
    t(u.user_time().tv_sec(), u.user_time().tv_usec() as i64) + t(u.system_time().tv_sec(), u.system_time().tv_usec() as i64)
}

fn idle_cpu() -> f64 {
    let c0 = cpu_ms();
    drive(async { tokio::time::sleep(Duration::from_millis(IDLE_MS)).await });
    cpu_ms() - c0
}

fn ledgers() -> (usize, usize) {
    let s = sfu();
    (s.peers().len(), s.tables.map.lock().map(|g| g.len()).unwrap_or(0))
}

#[test]
fn a_hundred_peer_reaps_leave_no_task_association_fd_or_cpu_behind() {
    let _g = serial();
    media();
    let r = create(&tag("reap"));
    let (l0, fd0, cpu0) = (ledgers(), fds(), idle_cpu());
    let mut weak: Vec<Weak<Transport>> = Vec::new();
    for i in 0..CYCLES {
        let user = tag(&format!("reap-{i}"));
        let c = ctx(&user);
        assert!(ops::rooms::join(&c, &json!({ "room_id": r.id })).is_ok(), "stand check — join {i}");
        let p = sfu().peer(&user).expect("stand check — the join made a Peer");
        weak.extend(p.transports().iter().map(Arc::downgrade));
        drop(p);
        drive(async { tokio::time::sleep(Duration::from_millis(2)).await });
        if i % 2 == 0 {
            assert!(ops::rooms::leave(&c, &json!({ "room_id": r.id })).is_ok(), "stand check — leaving the last room {i}");
        } else {
            assert!(ops::rooms::session_gone(&envelope(CallKind::SessionGone, Some(&user), Some(&c.sid))).is_ok(), "stand check — session gone {i}");
        }
    }
    assert_eq!(weak.len(), CYCLES * 2, "stand check — two transports per 2pc Peer");
    let alive = |w: &[Weak<Transport>]| w.iter().filter(|x| x.upgrade().is_some()).count();
    let mut left = alive(&weak);
    for _ in 0..60 {
        if left == 0 {
            break;
        }
        drive(async { tokio::time::sleep(Duration::from_millis(50)).await });
        left = alive(&weak);
    }
    assert_eq!(left, 0, "S-sym-RULE-LINK-TRANSPORT-4 after {CYCLES} reaps no transport is still held by a task (DTLS · its SCTP association) — {left} of {} alive", weak.len());
    assert_eq!(ledgers(), l0, "S-sym-RULE-LINK-TRANSPORT-4 the Peer and table ledgers are back to the baseline");
    let fd1 = fds();
    assert!(fd1 <= fd0 + FD_SLACK, "S-sym-RULE-LINK-TRANSPORT-4 open fds do not grow ({fd0} → {fd1})");
    let cpu1 = idle_cpu();
    assert!(cpu1 <= cpu0 + CPU_RISE_MAX_MS, "S-sym-RULE-LINK-TRANSPORT-4 idle CPU does not rise after the reaps ({cpu0:.1}ms → {cpu1:.1}ms per {IDLE_MS}ms)");
}
