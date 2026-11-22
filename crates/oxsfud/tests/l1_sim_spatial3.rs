// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-SIM-SWITCH · 정:IMPL-PUB-RID-SPATIAL · 정:RULE-SIM-TICK · model: claude-opus-5-5
//! 1층 — 공간 층 셋(rid 고정 번호 l=0 · m=1 · h=2). 없는 층 맞추기 · in-band demote 가 살아 있는 가장 높은 아래 층으로 · v1 단 둘 · 층 상태 묶음(context 202611/20261122a S3 · 시험 단위 S-sym-IMPL-SIM-SWITCH-8 · 9).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use oxsfud::fwd::{DownRow, LAYER_PAUSE, LAYER_TOP, apply_row_cmd};
use oxsfud::link::{Role, Transport};
use oxsfud::sim::{LayerState, Pli, fall, fit, judge, layer_name, layer_state, live_layers, step, v1_layer};
use oxsig::msg::Kind;

const L: u8 = 1 << 0;
const M: u8 = 1 << 1;
const H: u8 = 1 << 2;

fn row() -> DownRow {
    let tr = Transport::new(9, "s", "", Role::Both, 0, false);
    let r = DownRow::create(tr, "1".into(), Kind::Video, false, 1, None, Arc::new(AtomicU64::new(9)));
    r.since.store(1, Ordering::Release);
    apply_row_cmd(&r, 1, true);
    r
}

#[test]
fn the_rid_table_is_fixed_and_the_top_is_h() {
    assert_eq!((layer_name(0), layer_name(1), layer_name(LAYER_TOP), layer_name(LAYER_PAUSE)), ("l", "m", "h", "pause"), "정:IMPL-PUB-RID-SPATIAL l=0 · m=1 · h=2");
    assert_eq!(oxsfud::trk::SCALABILITY, "L3T3", "연:RULE-PUB-RID-TO-SPATIAL the address space is three spatial and three temporal layers");
    let r = row();
    assert_eq!((r.cap.load(Ordering::Acquire), r.auto_cap.load(Ordering::Acquire)), (LAYER_TOP, LAYER_TOP), "정:IMPL-SIM-SWITCH a new row asks the top layer");
}

#[test]
fn layer_state_packs_three_layers_and_pause() {
    for layer in [0, 1, LAYER_TOP, LAYER_PAUSE] {
        for target in [0, 1, LAYER_TOP, LAYER_PAUSE] {
            let s = LayerState { layer, target, since: 1_762_900_000_123 };
            assert_eq!(LayerState::unpack(s.pack()), s, "정:IMPL-SIM-SWITCH layer {layer} target {target} survive the one-word state — h no longer folds onto l");
        }
    }
}

#[test]
fn a_missing_layer_fits_to_the_highest_live_below() {
    assert_eq!(fit(1, L | H), 0, "S-sym-IMPL-SIM-SWITCH-8① two-layer publisher · m asked → l");
    assert_eq!(fit(1, L | M | H), 1, "S-sym-IMPL-SIM-SWITCH-8② three layers keep m");
    assert_eq!(fit(LAYER_TOP, L | M), 1, "S-sym-IMPL-SIM-SWITCH-8⑥ the top is fitted too — a publisher sending l · m only gives m");
    assert_eq!(fit(LAYER_TOP, L), 0, "S-sym-IMPL-SIM-SWITCH-8⑥");
    assert_eq!(fit(0, M | H), 1, "S-sym-IMPL-SIM-SWITCH-8④ nothing live below — the lowest live above");
    assert_eq!(fit(1, H), LAYER_TOP, "S-sym-IMPL-SIM-SWITCH-8④");
    assert_eq!(fit(LAYER_PAUSE, L | H), LAYER_PAUSE, "S-sym-IMPL-SIM-SWITCH-8⑤ pause stays pause");
    for w in [0, 1, LAYER_TOP] {
        assert_eq!(fit(w, 0), w, "S-sym-IMPL-SIM-SWITCH-8③ nothing known (start-up window) — nothing to fit");
    }
}

#[test]
fn live_layers_are_the_ones_heard_within_a_second_after_the_start_up_window() {
    let at = [10_200, 10_500, 0];
    assert_eq!(live_layers(&at, 0, 11_000), 0, "S-sym-IMPL-SIM-SWITCH-8③ no RTP yet — no knowledge");
    assert_eq!(live_layers(&at, 10_000, 11_000), 0, "S-sym-IMPL-SIM-SWITCH-8③ within a second of the first RTP h may still come — no fitting (start waits for h as before)");
    assert_eq!(live_layers(&at, 10_000, 11_001), L | M, "S-sym-IMPL-SIM-SWITCH-8③ after the window a never-heard h is not live");
    assert_eq!(live_layers(&at, 10_000, 11_400), M, "S-sym-IMPL-SIM-SWITCH-8⑥ a layer silent for 1s+ is not live");
    assert_eq!(live_layers(&[1, 1, 1], 1, 9_999), 0, "정:IMPL-SIM-SWITCH all silent — no knowledge, keep the wish");
}

#[test]
fn the_row_worker_adopts_the_fitted_target() {
    let r = row();
    r.lstate.store(LayerState { layer: 0, target: 0, since: 0 }.pack(), Ordering::Release);
    r.cap.store(1, Ordering::Release);
    let (st, adopted) = judge(&r, 0, false, 50_000, true, L | H, |_| false);
    assert_eq!(adopted, None, "S-sym-IMPL-SIM-SWITCH-8① m on a two-layer publisher is l — nothing to adopt");
    assert!(st.fwd && st.pli == Pli::None, "S-sym-IMPL-SIM-SWITCH-8① no keyframe is asked for a layer that is never coming");
    let (_, adopted) = judge(&r, 0, false, 50_001, true, L | M | H, |_| false);
    assert_eq!(adopted, Some(1), "S-sym-IMPL-SIM-SWITCH-8② once m is live the target is m");
    assert_eq!(layer_state(&r).target, 1);
}

#[test]
fn layers_stalling_one_after_another_step_down_and_come_back() {
    let r = row();
    r.lstate.store(LayerState { layer: LAYER_TOP, target: LAYER_TOP, since: 0 }.pack(), Ordering::Release);
    let (st, adopted) = judge(&r, 1, false, 50_000, true, L | M, |_| false);
    assert_eq!(adopted, Some(1), "S-sym-IMPL-SIM-SWITCH-8⑥ h silent → the target is m, not the dead h");
    assert!(!st.fwd);
    let (st, _) = judge(&r, 1, true, 50_100, true, L | M, |_| false);
    assert!(st.fwd && layer_state(&r) == LayerState { layer: 1, target: 1, since: 50_000 }, "S-sym-IMPL-SIM-SWITCH-8⑥ an m keyframe confirms m");
    let (_, adopted) = judge(&r, 1, false, 50_200, true, L | M, |_| false);
    assert_eq!(adopted, None, "S-sym-IMPL-SIM-SWITCH-8⑥ the target is not pulled back to the silent h");
    let (st, adopted) = judge(&r, 0, false, 52_000, true, L, |_| false);
    assert_eq!(adopted, Some(0), "S-sym-IMPL-SIM-SWITCH-8⑥ m silent too → l (Fable review 20261122: the second stall used to stay stuck on h)");
    assert!(!st.fwd);
    let (st, _) = judge(&r, 0, true, 52_100, true, L, |_| false);
    assert!(st.fwd && layer_state(&r).layer == 0, "S-sym-IMPL-SIM-SWITCH-8⑥ l flows again");
    let (_, adopted) = judge(&r, 0, false, 60_000, true, L | M | H, |_| false);
    assert_eq!(adopted, Some(LAYER_TOP), "S-sym-IMPL-SIM-SWITCH-8⑥ h back → the target climbs to h");
    let (st, _) = judge(&r, LAYER_TOP, true, 60_100, true, L | M | H, |_| false);
    assert!(st.fwd && layer_state(&r).layer == LAYER_TOP);
}

#[test]
fn in_band_demote_goes_to_the_highest_live_layer_below() {
    let live = |x: u8| Some(x == 2);
    assert!(fall(1, LAYER_TOP, live), "S-sym-IMPL-SIM-SWITCH-9① h stalls · m live → m");
    assert!(!fall(0, LAYER_TOP, live), "S-sym-IMPL-SIM-SWITCH-9③ a live m blocks the fall to l");
    let both = |x: u8| Some(x >= 1);
    assert!(fall(0, LAYER_TOP, both), "S-sym-IMPL-SIM-SWITCH-9② h and m stall → l");
    let two = |x: u8| if x == 1 { None } else { Some(x == 2) };
    assert!(fall(0, LAYER_TOP, two), "정:IMPL-SIM-SWITCH a two-layer publisher falls from h to l");
    assert!(!fall(0, LAYER_TOP, |x| if x == 2 { None } else { Some(false) }), "정:IMPL-SIM-SWITCH a current layer never learned is not a stall");
    assert!(!fall(0, LAYER_TOP, |_| Some(false)), "정:IMPL-SIM-SWITCH nothing stalls");
    let steady = LayerState { layer: LAYER_TOP, target: LAYER_TOP, since: 0 };
    let st = step(steady, 1, false, 5_000, false, true);
    assert_eq!((st.next.map(|n| (n.layer, n.target, n.since)), st.fwd, st.pli), (Some((LAYER_TOP, 1, 5_000)), false, Pli::Here), "S-sym-IMPL-SIM-SWITCH-9① demote target is the packet's layer");
    let st = step(steady, 1, true, 5_000, false, true);
    assert_eq!(st.next, Some(LayerState { layer: 1, target: 1, since: 5_000 }), "정:IMPL-SIM-SWITCH a keyframe of that layer confirms at once");
    assert!(st.fwd);
}

#[test]
fn v1_moves_between_the_bottom_and_the_top() {
    assert_eq!((v1_layer(0), v1_layer(1)), (0, LAYER_TOP), "정:RULE-SIM-TICK v1 promotes to the top layer — the middle layer is v2 only");
}
