// author: kodeholic (powered by Claude)
// spec: 정:IMPL-OPS-REC-UNIT · 정:RULE-OPS-REC-MKV
//! 녹음 유닛의 결과물 파일 — Ogg(발언 음성) · Matroska(영상 발언 · 개인 트랙) · 트랙 덮음 요약 · 회수 · 키프레임을 언제 요청할지.
//! oxpeer 녹음 파이프라인의 프레임 · 세그먼트 사건을 받는다 — oxpeer 는 파일을 모른다.

mod cover;
mod mkv;
mod ogg;
#[cfg(test)]
mod tests;

pub use cover::{Coverage, TrackCoverage};
pub use mkv::{MkvConfig, MkvFile, MkvSink, MkvTrack, recover_mkv};
pub use ogg::{OggConfig, OggFile, OggSink, Recovered, recover_ogg, safe as safe_name};

use std::path::Path;

pub(crate) const MIN_FREE_BYTES: u64 = 512 << 20;

fn free_bytes(dir: &Path) -> Option<u64> {
    let s = rustix::fs::statvfs(dir).ok()?;
    Some(s.f_bavail.saturating_mul(s.f_frsize))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum KeyAsk {
    #[default]
    Idle,
    Wanting,
}

pub(crate) fn key_request(st: KeyAsk, video_key: bool, elapsed: u128, target: u128, lead: u128) -> (bool, KeyAsk) {
    match st {
        KeyAsk::Idle if elapsed.saturating_add(lead) >= target => (true, KeyAsk::Wanting),
        KeyAsk::Wanting if video_key && elapsed < target => (true, KeyAsk::Wanting),
        s => (false, s),
    }
}
