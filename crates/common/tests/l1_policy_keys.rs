// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-POLICY · model: claude-opus-5-5
//! 1층 정책 키 — 기동 길(`oxhubd` · `oxsfud` `cfg` 가 부르는 `Policy::load` → `Policy::parse`)로 값마다 기동 성패와 부재 값을 본다. 경계 안쪽 값이 서는 것을 양성 대조로 같이 둔다(context 202611/20261124b 본게임 서버 1층 묶음 4).

use common::policy::{LoggingLevel, MediaAutolayer, MediaLayerStrategy, Policy};

fn boots(toml: &str) -> bool {
    Policy::parse(toml).is_ok()
}

#[test]
fn logging_level_takes_its_five_values_and_a_typo_fails_the_boot() {
    for v in ["trace", "debug", "info", "warn", "error"] {
        assert!(boots(&format!("[logging]\nlevel = \"{v}\"\n")), "PL-KEY-LOGGING-LEVEL {v} boots");
    }
    for v in ["inf", "Info", "warning", ""] {
        assert!(!boots(&format!("[logging]\nlevel = \"{v}\"\n")), "PL-KEY-LOGGING-LEVEL a typo ({v:?}) fails the boot");
    }
    assert_eq!(Policy::parse("").expect("empty").logging.level, LoggingLevel::Info);
}

#[test]
fn bwe_mode_is_twcc_alone() {
    assert!(boots("[media]\nbwe_mode = \"twcc\"\n"), "PL-KEY-MEDIA-BWE-MODE twcc boots");
    for v in ["remb", "TWCC", "gcc"] {
        assert!(!boots(&format!("[media]\nbwe_mode = \"{v}\"\n")), "PL-KEY-MEDIA-BWE-MODE any other value ({v}) fails the boot");
    }
}

#[test]
fn absent_layer_keys_take_their_defaults() {
    let p = Policy::parse("[media]\nmax_bitrate_bps = 2500000\n").expect("media without the keys");
    assert_eq!(p.media.auto_layer, MediaAutolayer::Off, "PL-KEY-MEDIA-AUTO-LAYER absent = off");
    assert_eq!(p.media.layer_strategy, MediaLayerStrategy::LowestTemporal, "PL-KEY-MEDIA-LAYER-STRATEGY① absent = lowest_temporal");
    let set = Policy::parse("[media]\nauto_layer = \"v1\"\nlayer_strategy = \"halve_wait\"\n").expect("set");
    assert_eq!((set.media.auto_layer, set.media.layer_strategy), (MediaAutolayer::V1, MediaLayerStrategy::HalveWait), "positive control — a given value is taken");
}

#[test]
fn heartbeat_timeout_below_twice_the_interval_fails_the_boot() {
    assert!(boots("[hub]\nheartbeat_interval_ms = 5000\nheartbeat_timeout_ms = 10000\n"), "positive control — exactly twice the interval boots");
    assert!(!boots("[hub]\nheartbeat_interval_ms = 5000\nheartbeat_timeout_ms = 9999\n"), "PL-KEY-HUB-HEARTBEAT-TIMEOUT-MS below 2 × interval fails the boot");
    assert!(!boots("[hub]\nheartbeat_interval_ms = 1000\nheartbeat_timeout_ms = 1999\n"), "PL-KEY-HUB-HEARTBEAT-TIMEOUT-MS below 2000 fails the boot");
}

#[test]
fn ws_flow_window_outside_one_to_ten_fails_the_boot() {
    for v in [1, 10] {
        assert!(boots(&format!("[hub]\nws_flow_window = {v}\n")), "positive control — {v} boots");
    }
    for v in [0, 11] {
        assert!(!boots(&format!("[hub]\nws_flow_window = {v}\n")), "PL-KEY-HUB-WS-FLOW-WINDOW {v} is out of range — the boot fails");
    }
}

#[test]
fn logging_ranges_fail_the_boot_outside() {
    assert!(boots("[logging]\nline_max_bytes = 2047\n"), "positive control — 2047 boots");
    assert!(!boots("[logging]\nline_max_bytes = 2048\n"), "PL-KEY-LOGGING-LINE-MAX-BYTES① 2048 is out of range — the boot fails");
    assert!(boots("[logging]\nsocket_rcvbuf_bytes = 65536\n"), "positive control — 65536 boots");
    assert!(!boots("[logging]\nsocket_rcvbuf_bytes = 65535\n"), "PL-KEY-LOGGING-SOCKET-RCVBUF-BYTES① 65535 is out of range — the boot fails");
    assert!(boots("[logging]\nstat_interval_ms = 200\n"), "positive control — 200 boots");
    assert!(!boots("[logging]\nstat_interval_ms = 199\n"), "PL-KEY-LOGGING-STAT-INTERVAL-MS① 199 is out of range — the boot fails");
}
