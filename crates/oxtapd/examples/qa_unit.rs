// author: kodeholic (powered by Claude)
// spec: v1.14 · 정:IMPL-OPS-REC-UNIT · model: claude-opus-5-5
//! 시험 전용 녹음 유닛 실행 파일 — 기본 등록부에 독 소비자 하나를 `Registry::register` 로 더해(같은 이름이면 바꿔) `oxtapd::unit` 을 띄운다. 제품 실행 파일에는 시험 손잡이가 없다. 환경변수: `QA_POISON_KIND`(등록 이름 · 기본 `qa_poison`) · `QA_POISON_AT`(`open` = 방 작업이 소비자를 지을 때 패닉 — 방 격리에 센다 · `frame` = 첫 프레임에서 패닉 — 그 종류만 · 기본 `frame`) · `QA_POISON_TIMES`(패닉 횟수 상한 · 기본 끝없음) · `QA_POISON_PREFIX`(이 접두의 방만 · 기본 전부) · `QA_POISON_STATE`(횟수를 프로세스 되살림 너머로 세는 파일 · 없으면 프로세스 안에서만).

use std::sync::atomic::{AtomicU64, Ordering};

use oxpeer::record::Frame;
use oxtapd::{RecConsumer, Registry, RoomCtx};

static FIRED: AtomicU64 = AtomicU64::new(0);

struct Poison {
    armed: bool,
    room: String,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn fired() -> u64 {
    match env("QA_POISON_STATE") {
        Some(p) => std::fs::read_to_string(p).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0),
        None => FIRED.load(Ordering::SeqCst),
    }
}

fn fire(room: &str, at: &str) -> ! {
    let n = fired() + 1;
    match env("QA_POISON_STATE") {
        Some(p) => {
            let _ = std::fs::write(p, n.to_string());
        }
        None => FIRED.store(n, Ordering::SeqCst),
    }
    tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", room = %room, rec_kind = "qa_poison", at = %at, fired = n, "qa poison fired");
    panic!("qa poison {at} #{n} in {room}");
}

fn armed(room: &str) -> bool {
    let times: u64 = env("QA_POISON_TIMES").and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
    room.starts_with(env("QA_POISON_PREFIX").as_deref().unwrap_or("")) && fired() < times
}

impl Poison {
    fn open(ctx: &RoomCtx) -> std::io::Result<Poison> {
        let at_open = env("QA_POISON_AT").as_deref() == Some("open");
        if at_open && armed(&ctx.room) {
            fire(&ctx.room, "open");
        }
        Ok(Poison { armed: !at_open && armed(&ctx.room), room: ctx.room.clone() })
    }
}

impl RecConsumer for Poison {
    fn frame(&mut self, _f: &Frame) -> Result<(), String> {
        if self.armed {
            self.armed = false;
            fire(&self.room, "frame");
        }
        Ok(())
    }

    fn finish(&mut self, _reason: &str) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let mut r = Registry::defaults();
    let name = env("QA_POISON_KIND").unwrap_or_else(|| "qa_poison".into());
    r.register::<Poison>(&name, Poison::open);
    oxtapd::unit::main_with(r).await
}
