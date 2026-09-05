// author: kodeholic (powered by Claude)
//! version 발급 — 정§14-1. `epoch` = 프로세스 기동마다 새 값(= `sfu_id`), `seq` = 방마다 단조증가.

use std::sync::atomic::{AtomicU64, Ordering};

use oxsig::schema::Version;

pub fn new_epoch() -> String {
    format!("sfu-{}", uuid::Uuid::new_v4().simple())
}

/// 방 하나의 일련번호. 오르는 사건 = 입퇴장·발행/해제·트랙 상태·소속 변경. RTP 는 아니다.
#[derive(Debug, Default)]
pub struct Seq(AtomicU64);

impl Seq {
    pub fn bump(&self, epoch: &str) -> Version {
        Version { epoch: epoch.to_owned(), seq: self.0.fetch_add(1, Ordering::AcqRel) + 1 }
    }
    pub fn current(&self, epoch: &str) -> Version {
        Version { epoch: epoch.to_owned(), seq: self.0.load(Ordering::Acquire) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_per_room() {
        let s = Seq::default();
        assert_eq!(s.current("e").seq, 0);
        assert_eq!(s.bump("e").seq, 1);
        assert_eq!(s.bump("e").seq, 2);
        assert_eq!(s.current("e").seq, 2);
        assert_ne!(new_epoch(), new_epoch());
    }
}
