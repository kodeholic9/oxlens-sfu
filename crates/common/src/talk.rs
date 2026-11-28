// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-FLR-TALK-REPORT · 정:IMPL-CHAT-TALK · 정:IMPL-OPS-REC-UNIT · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-SHAPE · 정:RULE-OPS-DROPS · 연:RULE-CHAT-MSG-ID · 연:RULE-CHAT-TALK · model: claude-opus-5-5
//! 발언 알림 — 같은 node 의 미디어 서버 · 녹음 유닛이 게이트웨이에 묻는 `q/node/{node}/talk` 의 요청 형 · 발언 id 짓기 · 갱신 `msg_id` · 답이 일시 실패면 백오프로 다시(상한을 넘기면 버린다).

use std::time::Duration;

use oxsig::Code;
use oxsig::msg::TalkState;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::envelope::Reply;

pub const WHAT: &str = "talk";
pub const SYS_HASH: &str = "81330e4c";
pub const RETRY_FOR: Duration = Duration::from_secs(60);
pub const ASK_TIMEOUT: Duration = Duration::from_secs(5);
const BACKOFF_FIRST: Duration = Duration::from_millis(100);
const BACKOFF_MAX: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Report {
    Start { talk_id: String, room_id: String, user_id: String, start: u64, deadline: u64, sfu_id: String },
    Regrant { talk_id: String, deadline: u64 },
    End {
        talk_id: String,
        state: TalkState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<u8>,
        duration_ms: u64,
    },
    Rec {
        talk_id: String,
        path: String,
        size: u64,
        mime: String,
        duration_ms: u64,
        #[serde(default)]
        reason: String,
    },
}

impl Report {
    pub fn talk_id(&self) -> &str {
        match self {
            Report::Start { talk_id, .. } | Report::Regrant { talk_id, .. } | Report::End { talk_id, .. } | Report::Rec { talk_id, .. } => talk_id,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Report::Start { .. } => "start",
            Report::Regrant { .. } => "regrant",
            Report::End { .. } => "end",
            Report::Rec { .. } => "rec",
        }
    }

    pub fn tail(&self) -> String {
        match self {
            Report::Start { .. } => String::new(),
            Report::Regrant { deadline, .. } => format!(":regrant:{deadline}"),
            Report::End { .. } => ":end".to_string(),
            Report::Rec { .. } => ":rec".to_string(),
        }
    }
}

pub fn talk_id(at_ms: u64) -> String {
    format!("{SYS_HASH}{:012x}{}", at_ms & 0xffff_ffff_ffff, crate::random_hex(8))
}

pub fn update_msg_id(talk_id: &str, tail: &str) -> String {
    let at = talk_id.get(8..20).unwrap_or("000000000000");
    let mut h = Sha256::new();
    h.update(talk_id.as_bytes());
    h.update(tail.as_bytes());
    let d = h.finalize();
    let hex: String = d[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("{SYS_HASH}{at}{hex}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Refused(Code),
    Dropped,
}

pub fn judge(reply: Option<&[u8]>) -> Option<Outcome> {
    let r = Reply::decode(reply?).ok()?;
    match r {
        Reply::Ok { .. } => Some(Outcome::Done),
        Reply::Fail { failure } => match Code::from_num(failure.code) {
            Some(c @ (Code::ChatDisabled | Code::SpaceNotFound)) => Some(Outcome::Refused(c)),
            _ => None,
        },
        Reply::Http { .. } => None,
    }
}

pub async fn deliver<A, F>(report: &Report, retry_for: Duration, mut ask: A) -> Outcome
where
    A: FnMut(Vec<u8>) -> F,
    F: std::future::Future<Output = Option<Vec<u8>>>,
{
    let payload = serde_json::to_vec(report).unwrap_or_default();
    let began = tokio::time::Instant::now();
    let mut wait = BACKOFF_FIRST;
    loop {
        let reply = ask(payload.clone()).await;
        if let Some(o) = judge(reply.as_deref()) {
            return o;
        }
        let left = retry_for.saturating_sub(began.elapsed());
        if left.is_zero() {
            return Outcome::Dropped;
        }
        tokio::time::sleep(wait.min(left)).await;
        wait = (wait * 2).min(BACKOFF_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn talk_id_is_sys_hash_time_random() {
        let id = talk_id(0x0190_a1b2_c3d4);
        assert_eq!(id.len(), 36);
        assert!(id.starts_with("81330e4c0190a1b2c3d4"));
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(talk_id(1), talk_id(1));
    }

    #[test]
    fn update_msg_id_is_stable_and_distinct_per_tail() {
        let id = talk_id(0x0190_a1b2_c3d4);
        let end = update_msg_id(&id, ":end");
        assert_eq!(end, update_msg_id(&id, ":end"));
        assert_ne!(end, update_msg_id(&id, ":rec"));
        assert_ne!(update_msg_id(&id, ":regrant:5"), update_msg_id(&id, ":regrant:6"));
        assert_eq!(end.len(), 36);
        assert!(end.starts_with("81330e4c0190a1b2c3d4"));
    }

    #[test]
    fn report_wire_is_tagged_by_name() {
        let r = Report::End { talk_id: "x".into(), state: TalkState::Cut, reason: Some(2), duration_ms: 7 };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, serde_json::json!({"end": {"talk_id": "x", "state": "cut", "reason": 2, "duration_ms": 7}}));
        assert_eq!(serde_json::from_value::<Report>(v).unwrap(), r);
    }

    #[test]
    fn only_chat_disabled_and_space_not_found_stop_retrying() {
        assert_eq!(judge(Some(&Reply::ok(serde_json::Value::Null).encode())), Some(Outcome::Done));
        assert_eq!(judge(Some(&Reply::fail(Code::SpaceNotFound).encode())), Some(Outcome::Refused(Code::SpaceNotFound)));
        assert_eq!(judge(Some(&Reply::fail(Code::ChatDisabled).encode())), Some(Outcome::Refused(Code::ChatDisabled)));
        assert_eq!(judge(Some(&Reply::fail(Code::InternalError).encode())), None);
        assert_eq!(judge(None), None);
    }

    #[tokio::test(start_paused = true)]
    async fn retries_with_backoff_then_drops_past_the_limit() {
        let mut n = 0;
        let o = deliver(&Report::Regrant { talk_id: "x".into(), deadline: 1 }, Duration::from_secs(60), |_| {
            n += 1;
            async { None }
        })
        .await;
        assert_eq!(o, Outcome::Dropped);
        assert!((10..40).contains(&n), "{n}");
        let mut k = 0;
        let o = deliver(&Report::Regrant { talk_id: "x".into(), deadline: 1 }, Duration::from_secs(60), |_| {
            k += 1;
            let ok = k == 3;
            async move { ok.then(|| Reply::ok(serde_json::Value::Null).encode()) }
        })
        .await;
        assert_eq!((o, k), (Outcome::Done, 3));
    }
}
