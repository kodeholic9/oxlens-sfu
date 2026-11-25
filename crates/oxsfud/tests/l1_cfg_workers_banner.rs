// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-FWD-MEDIA-WORKERS · 정:RULE-CFG-THREE-PLACES · model: claude-opus-5-5
//! 1층 기동 인자 — 미디어 워커 수 N · 제어 스레드 수 C 는 인자가 없으면 N = 코어 수 − 2(최소 1 · macOS 는 그 값과 2 중 작은 것) · C = 2 로 풀고, 기동 배너에 풀린 값을 싣는다(인자가 있으면 그 값)(context 202611/20261124b 서버 규칙 갈래).

use oxsfud::cfg::SfuCfg;

fn args(extra: &[&str]) -> Vec<String> {
    let policy = format!("{}/../../policy.toml", env!("CARGO_MANIFEST_DIR"));
    let mut v: Vec<String> = ["--id", "node-w", "--policy", &policy, "--grpc-listen", "127.0.0.1:0", "--udp-port", "0", "--bus-connect", "tcp/127.0.0.1:7447", "--node-grace-secs", "10"].iter().map(|s| s.to_string()).collect();
    v.extend(extra.iter().map(|s| s.to_string()));
    v
}

fn field(c: &SfuCfg, k: &str) -> Option<String> {
    c.banner_fields().into_iter().find(|(n, _)| n == k).map(|(_, v)| v)
}

#[test]
fn the_banner_carries_the_resolved_worker_and_control_counts() {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let n = cores.saturating_sub(2).max(1);
    let want_n = if cfg!(target_os = "macos") { n.min(2) } else { n };
    let c = SfuCfg::load(args(&[])).expect("stand check — sfud loads with plain values");
    assert_eq!((c.workers, c.control_threads), (want_n, 2), "S-sym-RULE-FWD-MEDIA-WORKERS-6① without arguments N = cores − 2 (macOS at most 2) · C = 2 ({cores} cores)");
    assert_eq!(field(&c, "media-workers"), Some(want_n.to_string()), "S-sym-RULE-FWD-MEDIA-WORKERS-6① the banner carries the resolved N, not the missing argument");
    assert_eq!(field(&c, "control-threads"), Some("2".to_string()), "S-sym-RULE-FWD-MEDIA-WORKERS-6① the banner carries the resolved C");
    let g = SfuCfg::load(args(&["--media-workers", "3", "--control-threads", "4"])).expect("stand check — sfud loads with worker arguments");
    assert_eq!((field(&g, "media-workers"), field(&g, "control-threads")), (Some("3".to_string()), Some("4".to_string())), "S-sym-RULE-FWD-MEDIA-WORKERS-6① an argument wins and the banner carries it");
}
