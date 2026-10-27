// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-FATE · 정:CAND-BUS-SELF-WATCH-VALUES · 정:RULE-OPS-SUPERVISOR · 정:CAND-OPS-SHUTDOWN-NO-RESTART · model: claude-opus-5-5
//! 프로세스 수명 — 부모 생존 채널(stdin 닫힘 · 고아) · 자가 감시(런타임 밖 스레드가 박동을 본다 · 멎으면 진단을 남기고 끝낸다).

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub const BEAT_MS: u64 = 500;
pub const STALL_MS: u64 = 10_000;
pub const LIFELINE_ENV: &str = "OX_LIFELINE";
pub const STALL_EXIT: i32 = 70;

pub fn lifeline() -> tokio::sync::watch::Receiver<bool> {
    let (tx, rx) = tokio::sync::watch::channel(false);
    let tx = Arc::new(tx);
    if std::env::var(LIFELINE_ENV).ok().as_deref() == Some("stdin") {
        let t = tx.clone();
        let _ = std::thread::Builder::new().name("ox-lifeline".into()).spawn(move || {
            let mut buf = [0u8; 256];
            let mut stdin = std::io::stdin();
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let _ = t.send(true);
        });
    }
    let parent = std::os::unix::process::parent_id();
    let t = tx;
    let _ = std::thread::Builder::new().name("ox-orphan".into()).spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(BEAT_MS));
            if std::os::unix::process::parent_id() != parent {
                let _ = t.send(true);
                return;
            }
            if t.is_closed() {
                return;
            }
        }
    });
    rx
}

#[derive(Debug, Clone, Default)]
pub struct Beat(Arc<AtomicU64>);

impl Beat {
    pub fn tick(&self) {
        self.0.store(crate::time::mono_ms(), Ordering::Relaxed);
    }

    pub fn last(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

pub fn stalled(last_ms: u64, now_ms: u64, stall_ms: u64) -> bool {
    now_ms.saturating_sub(last_ms) >= stall_ms
}

pub fn self_watch(what: &'static str) -> Beat {
    let beat = Beat::default();
    beat.tick();
    let b = beat.clone();
    let _ = std::thread::Builder::new().name("ox-selfwatch".into()).spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(BEAT_MS));
            let now = crate::time::mono_ms();
            if stalled(b.last(), now, STALL_MS) {
                tracing::error!(what, stalled_ms = now.saturating_sub(b.last()), "런타임 멎음 — 끝낸다");
                std::thread::sleep(Duration::from_millis(200));
                std::process::exit(STALL_EXIT);
            }
        }
    });
    let t = beat.clone();
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(BEAT_MS));
        loop {
            iv.tick().await;
            t.tick();
        }
    });
    beat
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stall_threshold_is_inclusive() {
        assert!(!stalled(1_000, 10_999, STALL_MS));
        assert!(stalled(1_000, 11_000, STALL_MS));
        assert!(!stalled(5_000, 1_000, STALL_MS));
    }
}
