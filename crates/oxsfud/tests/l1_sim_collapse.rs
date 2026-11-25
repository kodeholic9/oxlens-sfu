// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-DOC-COND-INDEX-SERVER · 정:RULE-SIM-TICK · model: claude-opus-5-5
//! 1층 v1 층 판단 — 승격 뒤 재강등 창(5,000ms) 안의 재강등만 「무너짐」이라 backoff 를 배로 늘리고, 창 밖의 재강등은 backoff 를 그대로 둔다(context 202611/20261124b 서버 규칙 갈래).

use oxsfud::sim::core::{P_BACKOFF_MS, P_COLLAPSE_MS, Policy, Signals};

fn sig(now: u64, loss: f64) -> Signals {
    Signals { now, loss, sent: 100, repeats: 0, drops: 0 }
}

fn run(p: &mut Policy, from: u64, to: u64, loss: f64) -> Vec<(u64, u8)> {
    let mut moves = Vec::new();
    let mut t = from;
    while t <= to {
        if let Some(l) = p.tick(&sig(t, loss)).to {
            moves.push((t, l));
        }
        t += 1_000;
    }
    moves
}

fn promoted_at(p: &mut Policy) -> u64 {
    assert_eq!(run(p, 100_000, 101_000, 0.1), vec![(101_000, 0)], "stand check — two lossy ticks demote");
    assert_eq!(p.backoff, P_BACKOFF_MS, "stand check — the first demote is not a collapse");
    let up = run(p, 102_000, 116_000, 0.0);
    assert_eq!(up, vec![(116_000, 1)], "stand check — clean for the backoff promotes");
    116_000
}

#[test]
fn only_a_re_demote_inside_the_window_doubles_the_backoff() {
    assert_eq!(P_COLLAPSE_MS, 5_000, "S-cond-RULE-DOC-COND-INDEX-SERVER-6① the re-demote window is 5,000ms");
    let mut inside = Policy::default();
    let up = promoted_at(&mut inside);
    assert_eq!(run(&mut inside, up + 1_000, up + 2_000, 0.1), vec![(up + 2_000, 0)], "stand check — re-demoted 2s after the promotion");
    assert_eq!(inside.backoff, P_BACKOFF_MS * 2, "S-cond-RULE-DOC-COND-INDEX-SERVER-6② a re-demote inside the window is a collapse — the backoff doubles");
    let mut outside = Policy::default();
    let up = promoted_at(&mut outside);
    assert!(run(&mut outside, up + 1_000, up + 5_000, 0.0).is_empty(), "stand check — clean past the window");
    assert_eq!(run(&mut outside, up + 6_000, up + 7_000, 0.1), vec![(up + 7_000, 0)], "stand check — re-demoted 7s after the promotion");
    assert_eq!(outside.backoff, P_BACKOFF_MS, "S-cond-RULE-DOC-COND-INDEX-SERVER-6② a re-demote outside the window is not a collapse — the backoff stays");
}
