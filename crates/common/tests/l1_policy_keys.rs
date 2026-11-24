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
fn hook_ext_window_zero_fails_the_boot() {
    assert!(boots("[hook]\next_window = 1\n"), "positive control — 1 boots");
    assert!(!boots("[hook]\next_window = 0\n"), "PL-KEY-HOOK-EXT-WINDOW② 0 is out of range — the boot fails");
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

const RANGED: &[(&str, &str, Option<u64>, Option<u64>)] = &[
    ("hub", "token_ttl_secs", Some(60), None),
    ("hub", "resume_window_ms", Some(52000), None),
    ("hub", "ws_flow_window", Some(1), Some(10)),
    ("hub", "room_default_capacity", Some(1), None),
    ("hub", "metadata_max_bytes", Some(256), None),
    ("hub", "join_retry_after_ms", Some(100), None),
    ("logging", "rotate_max_bytes", Some(1048576), None),
    ("logging", "queue_max_lines", Some(256), None),
    ("logging", "spool_max_bytes", Some(1048576), None),
    ("logging", "socket_rcvbuf_bytes", Some(65536), None),
    ("logging", "history_max_lines", Some(100), None),
    ("logging", "history_window_secs", Some(60), None),
    ("logging", "stat_interval_ms", Some(200), Some(60000)),
    ("media", "pli_throttle_high_ms", Some(300), Some(10000)),
    ("media", "pli_throttle_low_ms", Some(300), Some(10000)),
    ("media", "pli_reopen_margin_ms", Some(0), Some(1000)),
    ("floor", "queue_max", Some(1), Some(255)),
    ("floor", "t2_stop_talking_secs", Some(5), Some(600)),
    ("floor", "talk_min_ms", Some(0), Some(10000)),
    ("hook", "queue_max", Some(1), None),
    ("hook", "ext_window", Some(1), None),
    ("hook", "ext_reply_timeout_ms", Some(1), None),
    ("hook", "ext_timeout_close", Some(1), None),
    ("hook", "ext_max_tries", Some(1), None),
    ("hook", "ext_sweep_secs", Some(1), None),
];

fn with(group: &str, key: &str, v: u64) -> String {
    format!("[{group}]\n{key} = {v}\n")
}

#[test]
fn every_ranged_key_boots_on_its_edges_and_fails_one_past() {
    for (g, k, min, max) in RANGED {
        if let Some(lo) = min {
            assert!(boots(&with(g, k, *lo)), "positive control — {g}.{k} = {lo} (the book minimum) boots");
            if *lo > 0 {
                assert!(!boots(&with(g, k, lo - 1)), "{g}.{k} = {} one below the book minimum fails the boot", lo - 1);
            }
        }
        if let Some(hi) = max {
            assert!(boots(&with(g, k, *hi)), "positive control — {g}.{k} = {hi} (the book maximum) boots");
            assert!(!boots(&with(g, k, hi + 1)), "{g}.{k} = {} one above the book maximum fails the boot", hi + 1);
        }
    }
}

#[test]
fn every_default_is_the_book_value() {
    let p = Policy::default();
    assert_eq!((p.logging.level, p.logging.rotate_max_bytes, p.logging.retention_days, p.logging.line_max_bytes, p.logging.queue_max_lines), (LoggingLevel::Info, 268_435_456, 14, 1_024, 8_192));
    assert_eq!((p.logging.spool_max_bytes, p.logging.socket_rcvbuf_bytes, p.logging.history_max_lines, p.logging.history_window_secs, p.logging.stat_interval_ms), (67_108_864, 4_194_304, 5_000, 3_600, 5_000));
    assert_eq!((p.media.slot_mid_reserve, p.media.max_bitrate_bps, p.media.ingress_remb_max_bps), (40, 2_500_000, 5_000_000));
    assert_eq!((p.media.pli_throttle_high_ms, p.media.pli_throttle_low_ms, p.media.pli_reopen_margin_ms), (1_000, 500, 100));
    assert_eq!((p.floor.queue_max, p.floor.t2_stop_talking_secs, p.floor.talk_min_ms), (10, 30, 500));
    assert_eq!((p.hub.token_ttl_secs, p.hub.heartbeat_interval_ms, p.hub.heartbeat_timeout_ms, p.hub.resume_window_ms, p.hub.ws_flow_window), (3_600, 10_000, 30_000, 60_000, 8));
    assert_eq!((p.hub.room_unused_ttl_secs, p.hub.room_departure_ttl_secs, p.hub.room_default_capacity, p.hub.metadata_max_bytes), (0, 0, 1_000, 2_048));
    assert_eq!((p.hub.join_admit_per_sec, p.hub.join_retry_after_ms, p.hub.allowed_origins.clone()), (0, 5_000, vec!["*".to_string()]));
    assert_eq!((p.quota.max_sessions, p.quota.max_rooms), (0, 0));
    assert_eq!((p.hook.retry_backoff_ms.clone(), p.hook.retry_max, p.hook.queue_max), (vec![1_000, 2_000, 4_000, 8_000, 16_000], 5, 10_000));
    assert_eq!((p.hook.ext_window, p.hook.ext_reply_timeout_ms, p.hook.ext_timeout_close, p.hook.ext_max_tries, p.hook.ext_sweep_secs), (8, 10_000, 3, 10, 30));
}

#[test]
fn the_cited_ranges_fail_one_past_the_book_edge() {
    assert!(boots("[hub]\nresume_window_ms = 52000\n"), "positive control — 52000 boots");
    assert!(!boots("[hub]\nresume_window_ms = 51999\n"), "PL-KEY-HUB-RESUME-WINDOW-MS① — below the backoff total (52000) fails the boot");
    assert!(boots("[hub]\nmetadata_max_bytes = 256\n"), "positive control — 256 boots");
    assert!(!boots("[hub]\nmetadata_max_bytes = 255\n"), "PL-KEY-HUB-METADATA-MAX-BYTES② — below 256 fails the boot");
    assert!(boots("[logging]\nqueue_max_lines = 256\n"), "positive control — 256 boots");
    assert!(!boots("[logging]\nqueue_max_lines = 255\n"), "PL-KEY-LOGGING-QUEUE-MAX-LINES① 255 is out of range — the boot fails");
    for key in ["pli_throttle_high_ms", "pli_throttle_low_ms"] {
        assert!(boots(&format!("[media]\n{key} = 300\n")), "positive control — {key} = 300 boots");
    }
    assert!(!boots("[media]\npli_throttle_high_ms = 299\n"), "PL-KEY-MEDIA-PLI-THROTTLE-HIGH-MS① 299 is out of range — the boot fails");
    assert!(!boots("[media]\npli_throttle_low_ms = 299\n"), "PL-KEY-MEDIA-PLI-THROTTLE-LOW-MS① 299 is out of range — the boot fails");
}
