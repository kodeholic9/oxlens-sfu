// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SIM-SWITCH · 정:RULE-FWD-MEDIA-WORKERS · model: claude-opus-5-5
//! 1층 — 줄의 층 상태(층 · 목표 · 대기 시작 · 줄 세대)는 한 묶음이다. 쓰는 쪽 셋(제어 층 요청 · 구독 워커 자동 층 · 업링크 워커 전환)은 읽은 묶음이 그대로이고 그 세대가 제 세대일 때만 통째로 바꾼다(context 202611/20261112d R11 · 모형 exec layer · layer_fields · layer_stale).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use oxsfud::fwd::{DownRow, LAYER_PAUSE};
use oxsfud::link::{Role, Transport};
use oxsfud::sim::{LayerState, PENDING_MS, Pli, advance, layer_state, reset, set_target, step};
use oxsig::msg::Kind;

fn row() -> DownRow {
    let tr = Transport::new(9, "s", "", Role::Both, 0, false);
    DownRow::create(tr, "1".into(), Kind::Video, false, 1, None, Arc::new(AtomicU64::new(0)))
}

fn on(row: &DownRow, s: LayerState) {
    row.lstate.store(s.pack(), Ordering::Release);
}

#[test]
fn layer_state_round_trips_through_one_word() {
    for s in [
        LayerState { layer: LAYER_PAUSE, target: 1, since: 0, rev: 0 },
        LayerState { layer: 0, target: LAYER_PAUSE, since: 1_762_900_000_123, rev: 262_143 },
        LayerState { layer: 1, target: 0, since: 4_000_000_000_000, rev: 7 },
    ] {
        assert_eq!(LayerState::unpack(s.pack()), s, "정:IMPL-SIM-SWITCH one word carries the layer, the target, the pending start and the row revision");
    }
}

#[test]
fn a_target_request_moves_the_pending_start_with_it() {
    let r = row();
    on(&r, LayerState { layer: 1, target: 1, since: 0, rev: 5 });
    assert!(set_target(&r, 0, 50_000), "정:IMPL-SIM-SWITCH the request changes the target");
    let s = layer_state(&r);
    assert_eq!((s.target, s.since), (0, 50_000), "정:IMPL-SIM-SWITCH S-sym-IMPL-SIM-SWITCH-2 target and pending start are one write");
    let st = advance(&r, 5, |s| step(s, 1, false, 50_001, false, false)).expect("owned");
    assert!(st.fwd, "정:IMPL-SIM-SWITCH the current layer is relayed while pending");
    assert_eq!(layer_state(&r).target, 0, "정:IMPL-SIM-SWITCH S-sym-IMPL-SIM-SWITCH-2 a packet right after the request does not read it as expired");
}

#[test]
fn reading_a_new_target_with_the_old_pending_start_would_drop_the_request() {
    let half = LayerState { layer: 1, target: 0, since: 0, rev: 5 };
    let st = step(half, 1, false, PENDING_MS + 50_000, false, false);
    assert_eq!(st.next.map(|n| n.target), Some(1), "정:IMPL-SIM-SWITCH S-sym-IMPL-SIM-SWITCH-2 the half-written state is read as an expired switch and the request is undone — the one word makes it unrepresentable");
}

#[test]
fn a_worker_of_the_row_before_reassign_never_writes_its_layer() {
    let r = row();
    reset(&r, 1, true, 1_000);
    let st = advance(&r, 1, |s| step(s, 1, true, 1_001, false, false)).expect("owned");
    assert!(st.fwd && layer_state(&r).layer == 1, "정:IMPL-SIM-SWITCH the target keyframe confirms the switch");
    reset(&r, 2, true, 2_000);
    assert!(advance(&r, 1, |s| step(s, 1, true, 2_001, false, false)).is_none(), "정:IMPL-SIM-SWITCH S-sym-IMPL-SIM-SWITCH-3 a packet decided under the old row revision is not forwarded");
    let s = layer_state(&r);
    assert_eq!((s.layer, s.rev), (LAYER_PAUSE, 2), "정:IMPL-SIM-SWITCH S-sym-IMPL-SIM-SWITCH-3 the old publisher's worker does not write the reassigned row's layer");
    assert!(advance(&r, 2, |s| step(s, 1, true, 2_002, false, false)).is_some_and(|st| st.fwd), "정:IMPL-SIM-SWITCH the new publisher's worker confirms on its own keyframe");
}

#[test]
fn a_late_reset_from_an_older_table_change_is_skipped() {
    let r = row();
    reset(&r, 3, true, 3_000);
    reset(&r, 2, true, 2_000);
    let s = layer_state(&r);
    assert_eq!((s.rev, s.since), (3, 3_000), "정:IMPL-SIM-SWITCH S-sym-IMPL-SIM-SWITCH-3 a reset from an older table change arriving late does not take the row back");
}

#[test]
fn switch_machine_steps_are_unchanged() {
    let pending = LayerState { layer: 1, target: 0, since: 10_000, rev: 1 };
    assert_eq!(step(pending, 0, true, 10_001, false, false).next.map(|n| n.layer), Some(0), "정:IMPL-SIM-SWITCH target keyframe confirms the switch");
    assert_eq!(step(pending, 1, false, 10_001, true, false).pli, Pli::Retry(0), "정:IMPL-SIM-SWITCH a missing target keyframe asks the target layer again");
    assert_eq!(step(pending, 1, false, 10_000 + PENDING_MS + 1, false, false).next.map(|n| n.target), Some(1), "정:IMPL-SIM-SWITCH pending expires after 10s");
    let steady = LayerState { layer: 1, target: 1, since: 0, rev: 1 };
    let demote = step(steady, 0, false, 5_000, false, true);
    assert_eq!((demote.next.map(|n| (n.target, n.since)), demote.fwd, demote.pli), (Some((0, 5_000)), false, Pli::Here), "정:IMPL-SIM-SWITCH in-band demote when h lags");
    let paused = LayerState { layer: LAYER_PAUSE, target: 1, since: 0, rev: 1 };
    assert!(!step(paused, 0, false, 1, false, false).fwd, "정:IMPL-SIM-SWITCH pause drops");
}

#[test]
fn only_the_simulcast_module_writes_the_layer_word() {
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
            if p.starts_with(root.join("sim")) {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            for (n, line) in text.lines().enumerate() {
                if line.contains("lstate.store(") || line.contains("lstate.compare_exchange") || line.contains("lstate.fetch_update") || line.contains("lstate.swap(") {
                    bad.push(format!("{}:{}", p.display(), n + 1));
                }
            }
        }
    }
    assert!(bad.is_empty(), "정:IMPL-SIM-SWITCH the layer word is written only through the simulcast module: {bad:?}");
}
