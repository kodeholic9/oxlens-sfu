// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-BUS-FATE · 정:RULE-OPS-SURFACE · 정:CAND-BUS-SELF-WATCH-VALUES · 정:CAND-OPS-SHUTDOWN-NO-RESTART · model: claude-opus-5-5
//! 부모 생존 채널 · 자가 감시 — 유닛은 stdin 이 닫히거나 부모가 바뀌면 스스로 끝나고, 런타임이 멎으면 런타임 밖 감시자가 진단을 남기고 끝낸다.

use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const BEAT_MS: u64 = 500;
pub const STALL_MS: u64 = 10_000;
pub const EXIT_STALL: i32 = 70;

pub fn watch_stdin(on_close: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name("ox-lifeline".into()).spawn(move || {
        let mut buf = [0u8; 256];
        let mut stdin = std::io::stdin();
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }
        on_close();
    });
}

pub fn parent_pid() -> i32 {
    std::os::unix::process::parent_id() as i32
}

#[derive(Clone, Default)]
pub struct Beat(Arc<AtomicU64>);

impl Beat {
    pub fn touch(&self) {
        self.0.store(crate::time::mono_ms(), Ordering::Relaxed);
    }

    pub fn age_ms(&self) -> u64 {
        crate::time::mono_ms().saturating_sub(self.0.load(Ordering::Relaxed))
    }
}

pub fn self_watch(beat: Beat, on_stall: impl Fn(u64) + Send + 'static) {
    let parent = parent_pid();
    beat.touch();
    let _ = std::thread::Builder::new().name("ox-selfwatch".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_millis(BEAT_MS));
        let age = beat.age_ms();
        if age > STALL_MS {
            on_stall(age);
            std::process::exit(EXIT_STALL);
        }
        if parent > 1 && parent_pid() != parent {
            on_stall(0);
            std::process::exit(0);
        }
    });
}

pub async fn beat_task(beat: Beat) {
    let mut t = tokio::time::interval(Duration::from_millis(BEAT_MS));
    loop {
        t.tick().await;
        beat.touch();
    }
}
