// author: kodeholic (powered by Claude)
//! 방 귀속 op 의 뒷단 — 정§15-2. 방 결정(`route::room_of`) → 배치 맵 → 노드 gRPC. envelope 의 `user_id` 는 세션에서 주입하고
//! 응답 wire 는 그대로 통과한다(재해석 금지). 매핑 없음 `3001` · 노드 미연결 `5001` · gRPC 실패 `5002`.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use common::bplane;
use dashmap::DashMap;
use oxsig::frame::{self, Header, Kind, encode_json};
use oxsig::op::Op;
use oxsig::{FailCode, Failure};
use serde_json::Value;

use crate::nodes::NodeTable;
use crate::route::{RoomMap, room_of};

/// 토큰이 준 신원(연§5-2) — 방에 들어갈 때 한 번 넘어가 `RoomMember` 가 된다.
#[derive(Debug, Clone, Default)]
pub struct Identity {
    pub participant_type: u8,
    pub hidden: bool,
    pub metadata: Option<Value>,
}

/// envelope — `user_id` 는 세션에서 주입한다(body 의 것을 믿지 않는다).
#[derive(Debug, Clone)]
pub struct Envelope {
    pub session_id: String,
    pub user_id: String,
    /// 세션 확정값(연§6-1) — `"1pc"`/`"2pc"`.
    pub pc_mode: String,
    /// 연§4-4 토큰 신원 — ★`ROOM_JOIN` 에만 싣는다. 나머지 op 은 `None` 이다.
    pub identity: Option<Identity>,
    pub op: Op,
    pub pid: u32,
    pub body: Value,
}

#[async_trait]
pub trait Backend: Send + Sync {
    /// 응답 wire 하나(성공 또는 실패 프레임). 정§19 ②: 요청 하나에 응답은 정확히 하나.
    async fn handle(&self, env: Envelope) -> Vec<u8>;
}

pub fn ok_frame(op: Op, pid: u32, body: &Value) -> Vec<u8> {
    encode_json(&Header { kind: Kind::Ok, reserved: 0, op: op.code(), pid }, body)
}

pub fn fail_frame(op: u16, pid: u32, failure: &Failure) -> Vec<u8> {
    let body = serde_json::to_value(failure).unwrap_or(Value::Null);
    encode_json(&Header { kind: Kind::Fail, reserved: 0, op, pid }, &body)
}

/// sfud 가 아직 없는 배치 — 방 귀속 op 전부 `5001`.
pub struct NoBackend;

#[async_trait]
impl Backend for NoBackend {
    async fn handle(&self, env: Envelope) -> Vec<u8> {
        fail_frame(env.op.code(), env.pid, &Failure::new(FailCode::SfuUnavailable).message("no media server attached"))
    }
}

/// 방 → 입장 사용자 집합 — 방 broadcast 통지의 배달 대상(정§15-2 통지 배달). 정본은 sfud 명단이고 이것은 배달용 사본이다.
#[derive(Default)]
pub struct Members {
    rooms: DashMap<String, HashSet<String>>,
}

impl Members {
    pub fn join(&self, room_id: &str, user_id: &str) {
        self.rooms.entry(room_id.to_owned()).or_default().insert(user_id.to_owned());
    }
    pub fn leave(&self, room_id: &str, user_id: &str) {
        if let Some(mut set) = self.rooms.get_mut(room_id) {
            set.remove(user_id);
        }
    }
    pub fn members(&self, room_id: &str) -> Vec<String> {
        self.rooms.get(room_id).map(|s| s.iter().cloned().collect()).unwrap_or_default()
    }
    pub fn forget_room(&self, room_id: &str) -> Vec<String> {
        self.rooms.remove(room_id).map(|(_, s)| s.into_iter().collect()).unwrap_or_default()
    }
    pub fn forget_user(&self, user_id: &str) {
        for mut e in self.rooms.iter_mut() {
            e.value_mut().remove(user_id);
        }
    }
}

pub struct SfuBackend {
    pub nodes: Arc<NodeTable>,
    pub rooms: Arc<RoomMap>,
    pub members: Arc<Members>,
}

impl SfuBackend {
    /// 내부 op(HTTP 대행)도 같은 길로 — 방이 정해진 노드 하나에 `wire` 를 보내고 응답 wire 를 돌려준다.
    pub async fn send_to_node(&self, node_id: &str, env: bplane::Envelope) -> Result<Vec<u8>, FailCode> {
        let node = self.nodes.get(node_id).ok_or(FailCode::SfuUnavailable)?;
        let mut client = node.client().await.ok_or(FailCode::SfuUnavailable)?;
        match client.handle(env).await {
            Ok(resp) => Ok(resp.into_inner().wire),
            Err(_) => {
                node.drop_client().await;
                Err(FailCode::SfuError)
            }
        }
    }

    fn learn_membership(&self, env: &Envelope, room_id: &str, wire: &[u8]) {
        let Ok((h, _)) = frame::decode(wire) else { return };
        match (env.op, h.kind) {
            (Op::RoomJoin, Kind::Ok) => self.members.join(room_id, &env.user_id),
            (Op::RoomLeave, Kind::Ok) => self.members.leave(room_id, &env.user_id),
            (Op::RoomLeave, Kind::Fail) => {
                // 연§7-5-5 — `3001`·`3002` 는 성공과 같이 다룬다.
                if let Ok(b) = frame::body_json(&wire[frame::HEADER_LEN..])
                    && matches!(b["code"].as_u64(), Some(3001 | 3002))
                {
                    self.members.leave(room_id, &env.user_id);
                }
            }
            _ => {}
        }
    }
}

#[async_trait]
impl Backend for SfuBackend {
    async fn handle(&self, env: Envelope) -> Vec<u8> {
        let op = env.op.code();
        let room_id = match room_of(env.op, &env.body) {
            Ok(r) => r,
            Err(code) => return fail_frame(op, env.pid, &Failure::new(code)),
        };
        let Some(node_id) = self.rooms.node_of(&room_id) else {
            return fail_frame(op, env.pid, &Failure::new(FailCode::RoomNotFound));
        };
        let wire = encode_json(&Header::msg(op, env.pid), &env.body);
        // 신원은 방에 들어갈 때 한 번만 넘어간다(정§15-2 — 프로필이 매 op 을 타지 않는다).
        let id = env.identity.as_ref().filter(|_| env.op == Op::RoomJoin);
        let out = bplane::Envelope {
            session_id: env.session_id.clone(),
            user_id: env.user_id.clone(),
            room_id: room_id.clone(),
            target: String::new(),
            exclude: Vec::new(),
            wire,
            pc_mode: env.pc_mode.clone(),
            participant_type: u32::from(id.map_or(0, |i| i.participant_type)),
            hidden: id.is_some_and(|i| i.hidden),
            metadata: id.and_then(|i| i.metadata.as_ref()).map(|m| m.to_string()).unwrap_or_default(),
        };
        match self.send_to_node(&node_id, out).await {
            Ok(resp) => {
                self.learn_membership(&env, &room_id, &resp);
                resp
            }
            Err(code) => fail_frame(op, env.pid, &Failure::new(code)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn unmapped_room_is_3001_and_unreachable_node_is_5001() {
        let be = SfuBackend { nodes: Arc::new(NodeTable::new([("sfu-1".to_owned(), "127.0.0.1:1".to_owned())])), rooms: Arc::new(RoomMap::default()), members: Arc::new(Members::default()) };
        let env = |body: Value| Envelope { session_id: "s".into(), user_id: "u".into(), pc_mode: "2pc".into(), identity: None, op: Op::RoomJoin, pid: 1, body };
        let w = be.handle(env(json!({"room_id":"r"}))).await;
        assert_eq!(frame::body_json(&w[frame::HEADER_LEN..]).unwrap()["code"], 3001);
        be.rooms.assign("r", "sfu-1");
        let w = be.handle(env(json!({"room_id":"r"}))).await;
        assert_eq!(frame::body_json(&w[frame::HEADER_LEN..]).unwrap()["code"], 5001);
        let w = be.handle(env(json!({}))).await;
        assert_eq!(frame::body_json(&w[frame::HEADER_LEN..]).unwrap()["code"], 1003);
    }

    #[test]
    fn membership_learns_from_join_and_leave_including_3002() {
        let be = SfuBackend { nodes: Arc::new(NodeTable::new([])), rooms: Arc::new(RoomMap::default()), members: Arc::new(Members::default()) };
        let env = |op| Envelope { session_id: "s".into(), user_id: "u".into(), pc_mode: "2pc".into(), identity: None, op, pid: 1, body: Value::Null };
        be.learn_membership(&env(Op::RoomJoin), "r", &ok_frame(Op::RoomJoin, 1, &Value::Null));
        assert_eq!(be.members.members("r"), vec!["u".to_owned()]);
        be.learn_membership(&env(Op::RoomLeave), "r", &fail_frame(Op::RoomLeave.code(), 1, &Failure::new(FailCode::NotInRoom)));
        assert!(be.members.members("r").is_empty());
    }
}
