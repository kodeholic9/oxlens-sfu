// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SIM-SWITCH · 정:RULE-FWD-MEDIA-WORKERS · model: claude-opus-5-5
//! 1층 — 줄의 층 상태(층 · 목표 · 대기 시작)의 주인은 그 줄의 워커 하나다. 길목은 받는 워커의 보내기 판단(줄 세대 확인 뒤 · 재기록 앞) · 제어와 자동 판단은 제 입력 칸만 · 재배정 리셋은 줄 명령으로(context 202611/20261112d R11 · 모형 exec layer · layer_fields · layer_stale).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use oxsfud::fwd::{DownRow, LAYER_PAUSE, apply_row_cmd, row_same};
use oxsfud::link::{Role, Transport};
use oxsfud::sim::{LayerState, PENDING_MS, Pli, judge, layer_state, step, want};
use oxsig::msg::Kind;

fn row() -> DownRow {
    let tr = Transport::new(9, "s", "", Role::Both, 0, false);
    let r = DownRow::create(tr, "1".into(), Kind::Video, false, 1, None, Arc::new(AtomicU64::new(9)));
    r.since.store(1, Ordering::Release);
    apply_row_cmd(&r, 1, true);
    r
}

fn on(row: &DownRow, s: LayerState) {
    row.lstate.store(s.pack(), Ordering::Release);
}

#[test]
fn layer_state_round_trips_through_one_word() {
    for s in [
        LayerState { layer: LAYER_PAUSE, target: 1, since: 0 },
        LayerState { layer: 0, target: LAYER_PAUSE, since: 1_762_900_000_123 },
        LayerState { layer: 1, target: 0, since: 4_000_000_000_000 },
    ] {
        assert_eq!(LayerState::unpack(s.pack()), s, "정:IMPL-SIM-SWITCH readers see the layer, the target and the pending start as one value");
    }
}

#[test]
fn a_layer_request_writes_only_the_input_and_the_row_worker_adopts_it_in_one_step() {
    let r = row();
    on(&r, LayerState { layer: 1, target: 1, since: 0 });
    r.cap.store(0, Ordering::Release);
    assert_eq!(layer_state(&r), LayerState { layer: 1, target: 1, since: 0 }, "정:IMPL-SIM-SWITCH S-sym-IMPL-SIM-SWITCH-2 the control's request touches only its input cell");
    let (st, adopted) = judge(&r, 1, false, 50_000, false, |_| false);
    assert_eq!(adopted, Some(0), "정:IMPL-SIM-SWITCH the row worker adopts min(cap, auto) as the target");
    assert!(st.fwd, "정:IMPL-SIM-SWITCH the current layer is relayed while pending");
    let s = layer_state(&r);
    assert_eq!((s.target, s.since), (0, 50_000), "정:IMPL-SIM-SWITCH S-sym-IMPL-SIM-SWITCH-2 target and pending start move in one step of the owner");
    let (st, adopted) = judge(&r, 1, false, 50_001, false, |_| false);
    assert!(st.fwd && adopted.is_none() && layer_state(&r).target == 0, "정:IMPL-SIM-SWITCH S-sym-IMPL-SIM-SWITCH-2 the next packet does not read the request as expired");
    let (st, _) = judge(&r, 0, true, 50_002, false, |_| false);
    assert!(st.fwd && layer_state(&r).layer == 0, "정:IMPL-SIM-SWITCH the target keyframe confirms the switch");
}

#[test]
fn reading_a_new_target_with_the_old_pending_start_would_drop_the_request() {
    let half = LayerState { layer: 1, target: 0, since: 0 };
    let st = step(half, 1, false, PENDING_MS + 50_000, false, false);
    assert_eq!(st.next.map(|n| n.target), Some(1), "정:IMPL-SIM-SWITCH S-sym-IMPL-SIM-SWITCH-2 the half-written state is read as an expired switch and the request is undone — one owner never leaves it half written");
}

#[test]
fn a_reassign_resets_the_layer_by_row_command_before_any_new_source_packet() {
    let r = row();
    on(&r, LayerState { layer: 1, target: 1, since: 0 });
    r.since.store(2, Ordering::Release);
    assert_eq!(layer_state(&r).layer, 1, "정:IMPL-SIM-SWITCH the reassign itself does not write the layer state");
    assert!(!row_same(&r, 2), "정:RULE-FWD-MEDIA-WORKERS a new-source packet waits for the row command");
    assert!(apply_row_cmd(&r, 2, true), "정:RULE-FWD-MEDIA-WORKERS the row worker applies the command");
    let s = layer_state(&r);
    assert_eq!((s.layer, s.target), (LAYER_PAUSE, want(&r)), "정:IMPL-SIM-SWITCH S-sym-IMPL-SIM-SWITCH-3 the row command resets the layer state");
    assert!(!row_same(&r, 1), "정:IMPL-SIM-SWITCH S-sym-IMPL-SIM-SWITCH-3 a late packet of the publisher before the reassign never reaches the layer decision");
    assert!(row_same(&r, 2), "정:RULE-FWD-MEDIA-WORKERS the new source flows after the reset");
}

#[test]
fn switch_machine_steps_are_unchanged() {
    let pending = LayerState { layer: 1, target: 0, since: 10_000 };
    assert_eq!(step(pending, 0, true, 10_001, false, false).next.map(|n| n.layer), Some(0), "정:IMPL-SIM-SWITCH target keyframe confirms the switch");
    assert_eq!(step(pending, 1, false, 10_001, true, false).pli, Pli::Retry(0), "정:IMPL-SIM-SWITCH a missing target keyframe asks the target layer again");
    assert_eq!(step(pending, 1, false, 10_000 + PENDING_MS + 1, false, false).next.map(|n| n.target), Some(1), "정:IMPL-SIM-SWITCH pending expires after 10s");
    let steady = LayerState { layer: 1, target: 1, since: 0 };
    let demote = step(steady, 0, false, 5_000, false, true);
    assert_eq!((demote.next.map(|n| (n.target, n.since)), demote.fwd, demote.pli), (Some((0, 5_000)), false, Pli::Here), "정:IMPL-SIM-SWITCH in-band demote when h lags");
    let paused = LayerState { layer: LAYER_PAUSE, target: 1, since: 0 };
    assert!(!step(paused, 0, false, 1, false, false).fwd, "정:IMPL-SIM-SWITCH pause drops");
}

#[test]
fn only_the_row_worker_paths_write_the_layer_state() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let owner_paths = [root.join("sim"), root.join("fwd").join("mod.rs")];
    let writers = ["lstate.store(", "lstate.compare_exchange", "lstate.fetch_update", "lstate.swap("];
    let owner_calls = ["sim::adopt", "sim::judge", "sim::reset", "sim::admit", "sim::adopt_state"];
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
            let in_sim = p.starts_with(&owner_paths[0]);
            let owner = in_sim || p == owner_paths[1];
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            for (n, line) in text.lines().enumerate() {
                let write = !in_sim && writers.iter().any(|w| line.contains(w));
                let call = !owner && owner_calls.iter().any(|c| line.contains(c));
                if write || call {
                    bad.push(format!("{}:{}", p.display(), n + 1));
                }
            }
        }
    }
    assert!(bad.is_empty(), "정:IMPL-SIM-SWITCH 정:RULE-FWD-MEDIA-WORKERS only the row worker (delivery and row commands in fwd · the auto layer tick in sim) writes the layer state: {bad:?}");
}
