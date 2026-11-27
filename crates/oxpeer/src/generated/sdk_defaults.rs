// author: kodeholic (powered by Claude)
// ★생성 파일 — context/spec/policy/keys.ts SDK_DEFAULTS 가 정본(gen/policy.mts) · sdk_defaults.ts 의 Rust 판. 값이 연 원장 · SDK 정의서에 있는 것(canon)은 여기 없다. 앱이 안 준 칸은 이 값으로 채운다.
#![allow(dead_code)]
/// 무발화 뒤 cold 진입
pub const MIC_COLD_AFTER_MS: i64 = 30000;
/// getUserMedia 대기 상한(프롬프트 방치 포함)
pub const DEVICE_ACQUIRE_TIMEOUT_MS: i64 = 30000;
/// 1pc 에서 PeerLink 단위로 굳는 opus fmtp 선호
pub mod opus_fmtp_default {
    pub const USEDTX: i64 = 1;
    pub const USEINBANDFEC: i64 = 1;
    pub const STEREO: i64 = 0;
}
/// 카메라 시뮬캐스트 세 단 — sendEncodings(낮은 품질부터 l → m → h · 층마다 시간 층 셋)
pub const SIMULCAST_LAYERS_JSON: &str = "[{\"rid\":\"l\",\"scaleResolutionDownBy\":4,\"maxBitrate\":160000,\"maxFramerate\":20,\"scalabilityMode\":\"L1T3\"},{\"rid\":\"m\",\"scaleResolutionDownBy\":2,\"maxBitrate\":450000,\"maxFramerate\":20,\"scalabilityMode\":\"L1T3\"},{\"rid\":\"h\",\"scaleResolutionDownBy\":1,\"maxBitrate\":1650000,\"maxFramerate\":30,\"scalabilityMode\":\"L1T3\"}]";
/// 카메라 획득 제약 기본
pub mod camera_capture {
    pub const WIDTH: i64 = 1280;
    pub const HEIGHT: i64 = 720;
    pub const FRAME_RATE: i64 = 30;
}
/// 화면공유 발행 기본
pub mod screen_share {
    pub const SIMULCAST: bool = false;
    pub const CONTENT_HINT: &str = "detail";
    pub const DEGRADATION_PREFERENCE: &str = "maintain-resolution";
    pub const MAX_BITRATE: i64 = 1500000;
    pub const MAX_FRAMERATE: i64 = 15;
}
/// 수신 지터 버퍼 목표
pub const PLAYOUT_DELAY_MS: Option<i64> = None;
/// silence 판정 창
pub const SILENCE_WINDOW_MS: i64 = 1000;
/// adaptiveStream 바뀜 묶음
pub const ADAPTIVE_STREAM_SETTLE_MS: i64 = 500;
/// DC 만 끊겼을 때 같은 PeerLink 위 재개설
pub mod dc_reopen {
    pub const TRIES: i64 = 3;
    pub const INTERVAL_MS: i64 = 1000;
}
/// ICE disconnected 뒤 재시작 전 기다림
pub const WOBBLY_GRACE_MS: i64 = 2000;
/// 재시작 적용 뒤 connected 대기
pub const ICE_RESTART_TIMEOUT_MS: i64 = 5000;
/// WS 점검 시한
pub const WS_PROBE_TIMEOUT_MS: i64 = 2000;
/// 재구축 칸 첫 ROOM_JOIN 분산 창
pub const MASS_JITTER_MS: i64 = 5000;
/// 경로 불가 표시 문턱(회)
pub const UNREACHABLE_AFTER: i64 = 3;
/// 깨어남 판정 문턱
pub const WAKE_DETECT_MS: i64 = 2000;
/// diagnostics.stats 주기
pub const STATS_INTERVAL_MS: i64 = 5000;
/// 4단 문턱
pub mod quality {
    pub mod excellent {
        pub const LOSS_PCT: i64 = 2;
        pub const RTT_MS: i64 = 150;
    }
    pub mod good {
        pub const LOSS_PCT: i64 = 5;
        pub const RTT_MS: i64 = 400;
    }
}
/// 타일 크기 문턱 — 엘리먼트의 짧은 변 쪽 길이(가로 영상은 높이 · 세로 영상은 폭 · 물리 픽셀 = CSS × DPR)
pub mod adaptive_stream {
    pub const TOLERANCE: f64 = 0.9;
    pub const SHORT_SIDES_JSON: &str = "[180,360]";
}
