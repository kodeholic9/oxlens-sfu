// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-SIM-TICK · 정:IMPL-SIM-SWITCH · 정책 media.layer_strategy · model: claude-opus-5-5
//! 1층 정책 배선 — `media.layer_strategy` 를 기본과 다른 값으로 둔 서버를 세우면 미디어 서버의 전략 접근자가 그 값을 내고, 배분 점 · 기다리는 동안의 시간 층 두 판단 자리가 그 접근자를 부른다(context 202611/20261124b 넷째 갈래). 정책은 프로세스에 한 번 서므로 이 파일은 시험 하나다.

mod support;

use common::policy::{MediaLayerStrategy, Policy};
use support::*;

#[test]
fn the_layer_strategy_key_reaches_both_decisions() {
    let mut p = Policy::default();
    assert_eq!(p.media.layer_strategy, MediaLayerStrategy::LowestTemporal, "stand check — the default is lowest_temporal, so all_points is a different value");
    p.media.layer_strategy = MediaLayerStrategy::AllPoints;
    sfu_with(p);
    assert_eq!(oxsfud::sim::layer_strategy(), MediaLayerStrategy::AllPoints, "PL-KEY-MEDIA-LAYER-STRATEGY② the media server's strategy is the policy key's value");
    let src = product_lines();
    assert!(src.len() > 1_000, "stand check — the product sources were read ({})", src.len());
    let sim: Vec<&Src> = src.iter().filter(|l| l.file == "sim/mod.rs").collect();
    let near = |func: &str, call: &str| -> bool {
        let body: Vec<&str> = sim.iter().filter(|l| l.func == func).map(|l| l.text.trim()).collect();
        body.windows(2).any(|w| w.join(" ").contains(call))
    };
    assert!(near("candidates","strategy::layer_points(layer_strategy(),"), "PL-KEY-MEDIA-LAYER-STRATEGY② the allocation points take the policy strategy");
    assert!(near("temporal_keep", "strategy::wait_temporal( layer_strategy(),"), "PL-KEY-MEDIA-LAYER-STRATEGY② the temporal layer while waiting takes the policy strategy");
}
