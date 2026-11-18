// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-FLR-TALK-REPORT · 정:RULE-OPS-DROPS · 연:RULE-CHAT-TALK · 연:RULE-CHAT-TALK-STATE · 연:RULE-FLR-TLV · 연:RULE-FLR-SERVER-EVENTS · 연:RULE-FLR-TIMERS · model: claude-opus-5-5
//! 발언 알리기의 판정 — 허가마다 발언 · `deadline` = 허가 + T1 + T2 + T3 · 끝난 까닭(판정 라벨 · 들어온 길)에서 상태 · 방마다 한 줄(알림 줄 — 게이트웨이에 묻는 것은 집행 쪽).

use common::talk::Report;
use oxsig::msg::TalkState;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Talk {
    pub id: String,
    pub user: String,
    pub start: u64,
    pub deadline: u64,
}

pub fn deadline(at_ms: u64, t1_ms: u64, t2_ms: u64, t3_ms: u64) -> u64 {
    at_ms.saturating_add(t1_ms).saturating_add(t2_ms).saturating_add(t3_ms)
}

const RETURNED: &[&str] = &["FLR.S.G.TAKEN.rel_speaker"];
const AWAY: &str = "FLR.S.G.TAKEN.client_release";
const CUT: &[&str] = &["FLR.S.G.PREVOKE.rel_revoked", "FLR.S.G.PREVOKE.T3", "FLR.S.ws.join_revoking"];
const T1: &[&str] = &["FLR.S.G.TAKEN.T1", "FLR.S.G.PREVOKE.T1"];

pub struct End<'a> {
    pub labels: &'a [&'static str],
    pub entry: &'a str,
    pub why: Option<&'a str>,
    pub rcause: u32,
    pub heard: bool,
    pub duration_ms: u64,
    pub min_ms: u64,
}

fn among(labels: &[&str], set: &[&str]) -> bool {
    labels.iter().any(|l| set.contains(l))
}

pub fn end_state(e: &End) -> (TalkState, Option<u8>) {
    let normal = |d: u64| if d < e.min_ms { TalkState::Short } else { TalkState::Ok };
    if among(e.labels, CUT) {
        return (TalkState::Cut, Some(e.rcause.min(255) as u8));
    }
    if among(e.labels, T1) {
        return (if e.heard { TalkState::Lost } else { TalkState::Silent }, None);
    }
    if among(e.labels, RETURNED) {
        return (normal(e.duration_ms), None);
    }
    if e.labels.contains(&AWAY) {
        let returned = match e.entry {
            "ws_pub_away" => true,
            "ws_leave" => e.why == Some("leave"),
            _ => false,
        };
        if returned {
            return (normal(e.duration_ms), None);
        }
    }
    (TalkState::Lost, None)
}

pub struct Queue {
    tx: mpsc::UnboundedSender<Report>,
    rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<Report>>>,
}

impl Queue {
    pub fn new() -> Queue {
        let (tx, rx) = mpsc::unbounded_channel();
        Queue { tx, rx: std::sync::Mutex::new(Some(rx)) }
    }

    pub fn push(&self, r: Report) {
        let _ = self.tx.send(r);
    }

    pub fn claim(&self) -> Option<mpsc::UnboundedReceiver<Report>> {
        self.rx.lock().ok().and_then(|mut g| g.take())
    }
}

impl Default for Queue {
    fn default() -> Self {
        Queue::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(labels: &[&'static str], entry: &str, why: Option<&str>, heard: bool, d: u64) -> (TalkState, Option<u8>) {
        end_state(&End { labels, entry, why, rcause: 4, heard, duration_ms: d, min_ms: 500 })
    }

    #[test]
    fn end_reasons_map_to_the_state_table() {
        assert_eq!(st(&["FLR.S.P.PERMITTED.rx_RELEASE", "FLR.S.G.TAKEN.rel_speaker", "FLR.S.G.IDLE.enter_empty"], "server_rx", None, true, 900), (TalkState::Ok, None));
        assert_eq!(st(&["FLR.S.G.TAKEN.rel_speaker"], "server_rx", None, true, 499), (TalkState::Short, None));
        assert_eq!(st(&["FLR.S.G.TAKEN.rel_speaker"], "server_rx", None, false, 0), (TalkState::Short, None));
        assert_eq!(st(&["FLR.S.ws.pub_away", "FLR.S.G.TAKEN.client_release"], "ws_pub_away", None, true, 900), (TalkState::Ok, None));
        assert_eq!(st(&["FLR.S.ws.leave", "FLR.S.G.TAKEN.client_release"], "ws_leave", Some("leave"), true, 900), (TalkState::Ok, None));
        for why in ["kick", "destroy", "evict", "session_gone", "home_lost"] {
            assert_eq!(st(&["FLR.S.ws.leave", "FLR.S.G.TAKEN.client_release"], "ws_leave", Some(why), true, 900), (TalkState::Lost, None), "{why}");
        }
        assert_eq!(st(&["FLR.S.P.PREVOKE.rx_RELEASE", "FLR.S.G.PREVOKE.rel_revoked", "FLR.S.G.IDLE.enter_succession"], "server_rx", None, true, 900), (TalkState::Cut, Some(4)));
        assert_eq!(st(&["FLR.S.G.PREVOKE.T3"], "t3", None, true, 900), (TalkState::Cut, Some(4)));
        assert_eq!(st(&["FLR.S.ws.join_revoking"], "ws_join", None, true, 900), (TalkState::Cut, Some(4)));
        assert_eq!(st(&["FLR.S.G.TAKEN.T1"], "t1", None, false, 0), (TalkState::Silent, None));
        assert_eq!(st(&["FLR.S.G.TAKEN.T1"], "t1", None, true, 3000), (TalkState::Lost, None));
        assert_eq!(st(&["FLR.S.G.PREVOKE.T1"], "t1", None, false, 0), (TalkState::Silent, None));
        assert_eq!(st(&[], "stop", None, true, 10), (TalkState::Lost, None));
    }

    #[test]
    fn deadline_is_grant_plus_t1_t2_t3() {
        assert_eq!(deadline(1_000, 7_000, 30_000, 2_000), 40_000);
    }
}
