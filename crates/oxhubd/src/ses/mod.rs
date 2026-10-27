// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-SES-GATEWAY-SCOPE · 정:IMPL-SES-DATA · 정:IMPL-SES-BIND · 정:IMPL-SES-HANDOVER · 정:IMPL-SES-SOCKET-LIFE · 정:IMPL-SES-ADMISSION · 정:CAND-SES-SUBROOMS-RECONCILE · 연:SES.S.bind_new · 연:SES.S.bind_resume · 연:SES.S.TWindow · model: claude-opus-5-5
//! 세션 — 게이트웨이가 드는 세션 자료(신원 · 토큰값 넷 · 홈 · 서버별 입장 방) · 표 · 소켓 물림 · 창 만료.

pub mod core;
pub mod token;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use oxsig::types::{PcMode, Permission};
use serde::{Deserialize, Serialize};

pub use core::{Branch, OutQueue, bind_branch, new_session_id};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ServerRooms {
    pub node: String,
    pub rooms: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionData {
    pub sid: String,
    pub user_id: String,
    pub participant_type: u8,
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub permission: Permission,
    pub pc_mode: PcMode,
    pub home_node: String,
    pub home_inst: String,
    #[serde(default)]
    pub servers: BTreeMap<String, ServerRooms>,
}

impl SessionData {
    pub fn fingerprint(&self) -> String {
        common::log::fingerprint(&self.sid)
    }

    pub fn ctx(&self) -> common::bus::env::SessionCtx {
        common::bus::env::SessionCtx {
            user_id: self.user_id.clone(),
            session: self.fingerprint(),
            pc_mode: self.pc_mode,
            home_node: self.home_node.clone(),
            home_inst: self.home_inst.clone(),
            participant_type: self.participant_type,
            hidden: self.hidden,
            metadata: self.metadata.clone(),
            permission: self.permission,
        }
    }

    pub fn all_rooms(&self) -> BTreeSet<String> {
        self.servers.values().flat_map(|s| s.rooms.iter().cloned()).collect()
    }

    pub fn set_server_rooms(&mut self, sfu_id: &str, node: &str, rooms: BTreeSet<String>) {
        if rooms.is_empty() {
            self.servers.remove(sfu_id);
        } else {
            self.servers.insert(sfu_id.to_string(), ServerRooms { node: node.to_string(), rooms });
        }
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub data: SessionData,
    pub sock: Option<u64>,
    pub gone_at_ms: Option<u64>,
}

#[derive(Debug, Default)]
pub struct SessionTable {
    by_sid: HashMap<String, Session>,
    by_user: HashMap<String, String>,
    by_sock: HashMap<u64, String>,
}

impl SessionTable {
    pub fn insert(&mut self, data: SessionData, sock: u64) -> Option<Session> {
        let old = self.by_user.get(&data.user_id).cloned().and_then(|sid| self.remove(&sid));
        self.by_user.insert(data.user_id.clone(), data.sid.clone());
        self.by_sock.insert(sock, data.sid.clone());
        self.by_sid.insert(data.sid.clone(), Session { data, sock: Some(sock), gone_at_ms: None });
        old
    }

    pub fn get(&self, sid: &str) -> Option<&Session> {
        self.by_sid.get(sid)
    }

    pub fn get_mut(&mut self, sid: &str) -> Option<&mut Session> {
        self.by_sid.get_mut(sid)
    }

    pub fn of_user(&self, user: &str) -> Option<&Session> {
        self.by_user.get(user).and_then(|s| self.by_sid.get(s))
    }



    pub fn attach(&mut self, sid: &str, sock: u64) -> Option<u64> {
        let s = self.by_sid.get_mut(sid)?;
        let prev = s.sock.replace(sock);
        s.gone_at_ms = None;
        if let Some(p) = prev {
            self.by_sock.remove(&p);
        }
        self.by_sock.insert(sock, sid.to_string());
        prev
    }

    pub fn detach(&mut self, sock: u64, now_ms: u64) -> Option<String> {
        let sid = self.by_sock.remove(&sock)?;
        if let Some(s) = self.by_sid.get_mut(&sid)
            && s.sock == Some(sock)
        {
            s.sock = None;
            s.gone_at_ms = Some(now_ms);
        }
        Some(sid)
    }

    pub fn remove(&mut self, sid: &str) -> Option<Session> {
        let s = self.by_sid.remove(sid)?;
        if self.by_user.get(&s.data.user_id).is_some_and(|x| x == sid) {
            self.by_user.remove(&s.data.user_id);
        }
        if let Some(sock) = s.sock {
            self.by_sock.remove(&sock);
        }
        Some(s)
    }

    pub fn expired(&self, now_ms: u64, window_ms: u64) -> Vec<String> {
        self.by_sid.values().filter(|s| s.gone_at_ms.is_some_and(|t| now_ms.saturating_sub(t) >= window_ms)).map(|s| s.data.sid.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.by_sid.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_sid.is_empty()
    }


    pub fn all(&self) -> impl Iterator<Item = &Session> {
        self.by_sid.values()
    }


    pub fn room_members(&self, room: &str) -> Vec<String> {
        self.by_sid.values().filter(|s| s.data.servers.values().any(|r| r.rooms.contains(room))).map(|s| s.data.user_id.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(sid: &str, user: &str) -> SessionData {
        SessionData {
            sid: sid.into(),
            user_id: user.into(),
            participant_type: 0,
            hidden: false,
            metadata: None,
            permission: Permission::default(),
            pc_mode: PcMode::V2pc,
            home_node: "n".into(),
            home_inst: "i".into(),
            servers: BTreeMap::new(),
        }
    }

    #[test]
    fn new_session_of_same_user_evicts_old() {
        let mut t = SessionTable::default();
        assert!(t.insert(data("s1", "u"), 1).is_none());
        let old = t.insert(data("s2", "u"), 2).expect("old");
        assert_eq!(old.data.sid, "s1");
        assert_eq!(t.of_user("u").map(|s| s.data.sid.as_str()), Some("s2"));
        assert!(t.detach(1, 0).is_none());
    }

    #[test]
    fn window_runs_from_socket_loss() {
        let mut t = SessionTable::default();
        t.insert(data("s1", "u"), 1);
        assert_eq!(t.detach(1, 1_000).as_deref(), Some("s1"));
        assert!(t.expired(60_999, 60_000).is_empty());
        assert_eq!(t.expired(61_000, 60_000), vec!["s1".to_string()]);
        assert_eq!(t.attach("s1", 5), None);
        assert!(t.expired(200_000, 60_000).is_empty());
    }

    #[test]
    fn server_rooms_are_replaced_whole() {
        let mut d = data("s", "u");
        d.set_server_rooms("e1", "n1", ["r1".to_string(), "r2".to_string()].into());
        d.set_server_rooms("e1", "n1", ["r2".to_string()].into());
        assert_eq!(d.all_rooms(), ["r2".to_string()].into());
        d.set_server_rooms("e1", "n1", BTreeSet::new());
        assert!(d.servers.is_empty());
    }
}
