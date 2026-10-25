// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-BUS-REQUEST 정:IMPL-ROOM-JOIN 정:IMPL-SES-ADMISSION 정:IMPL-ERR-ISSUERS 정:RULE-BUS-EVENTS 정:IMPL-AUX-MESSAGE 정:IMPL-AUX-TASK 정:IMPL-LINK-ICE-RESTART 정:CAND-DOC-ORCHESTRATION-ASPECT 정:CAND-AUX-TASK-CLIENT-REQUEST 정:IMPL-AFF-PUB-ROOM 정:CAND-AFF-SPLIT-ROUTE 연:RULE-ROOM-JOIN-NUMBER-TAGS 연:RULE-ROOM-CREATE-FORM 연:RULE-ERR-FAILURE-FORM 연:DEV-ROOM-JOIN-FAIL-4009 연:RULE-AUX-TASK-PROC · model: claude-opus-5-5
//! op 길 — 클라 요청 하나를 게이트웨이 검사(①) 뒤 그 방 · 그 서버의 미디어 서버로 넘기고 응답이 나르는 진실로 서버별 입장 방 집합을 갈아 끼운다.

use std::collections::BTreeSet;
use std::sync::Arc;

use oxsig::envelope::{Call, Reply};
use oxsig::msg::{PcMode, RoomJoinReq, TaskReq};
use oxsig::{Code, Failure, Op};

use crate::ops::hub::Hub;
use crate::ses::{self, Session};

fn json(body: &[u8]) -> Result<serde_json::Value, Failure> {
    if body.is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_slice(body).map_err(|_| Code::InvalidPayload.failure())
}

fn room_field(v: &serde_json::Value, k: &str) -> Result<String, Failure> {
    match v.get(k) {
        None | Some(serde_json::Value::Null) => Err(Code::MissingField.failure_msg(format!("{k} 없음"))),
        Some(serde_json::Value::String(s)) => {
            if !common::bus::room_id_ok(s) || s.len() > 255 {
                return Err(Code::InvalidPayload.failure_msg(format!("{k} 형식")));
            }
            Ok(s.clone())
        }
        Some(_) => Err(Code::InvalidPayload.failure_msg(format!("{k} 형"))),
    }
}

async fn admit(hub: &Hub) -> Result<(), Failure> {
    let per = hub.cfg.policy.hub.join_admit_per_sec;
    if per == 0 {
        return Ok(());
    }
    let sec = common::log::now_ms() / 1000;
    let mut g = hub.joins.lock().await;
    if g.0 != sec {
        g.0 = sec;
        g.1 = 0;
    }
    g.1 += 1;
    if g.1 > u64::from(per) {
        let base = u64::from(hub.cfg.policy.hub.join_retry_after_ms);
        let r = u64::from_str_radix(&common::id::random_hex(4), 16).unwrap_or(0) % base.max(1);
        return Err(Code::Overloaded.failure().with_details(serde_json::json!({ "retry_after_ms": base + r })));
    }
    Ok(())
}

fn reply_into(r: Reply) -> Result<Vec<u8>, Failure> {
    match r {
        Reply::Ok { body } => Ok(body.into_bytes()),
        Reply::Fail { failure } => Err(failure),
        Reply::Http { body, .. } => Ok(body.into_bytes()),
    }
}

async fn forward(hub: &Arc<Hub>, s: &Arc<Session>, node: &str, inst: Option<&str>, op: Op, body: Vec<u8>) -> Result<Vec<u8>, Failure> {
    let call = Call::Op { ctx: s.ctx(hub.home()), op: op.as_u16(), body: String::from_utf8_lossy(&body).into_owned() };
    reply_into(hub.call_node(node, inst, &call).await)
}

async fn reconcile(hub: &Arc<Hub>, s: &Arc<Session>, node: &str, op: Op, res: &[u8]) {
    if let Some(rooms) = ses::parse_sub_rooms(op, res) {
        super::sess::set_rooms(hub, s, node, rooms).await;
    }
}

pub async fn handle(hub: &Arc<Hub>, s: &Arc<Session>, op_num: u16, body: Vec<u8>) -> Result<Vec<u8>, Failure> {
    let Some(op) = Op::from_u16(op_num) else {
        return Err(Code::UnknownOp.failure());
    };
    match op {
        Op::Resume => {
            let r = super::bind::resume(hub, s).await.map_err(|c| c.failure())?;
            common::ev!(spec = "CP-OP-RESUME", session = s.fp.as_str(), user = s.id.user.as_str(), "resume rooms={}", r.snapshot.len());
            serde_json::to_vec(&r).map_err(|e| Code::InternalError.failure_msg(e.to_string()))
        }
        Op::Task => task(hub, &body).await,
        Op::RoomJoin => {
            let v = json(&body)?;
            let room = room_field(&v, "room_id")?;
            let req: RoomJoinReq = serde_json::from_value(v).map_err(|_| Code::InvalidPayload.failure())?;
            match s.id.pc_mode {
                PcMode::V1pc if req.extmap.is_none() || req.codecs.is_none() => {
                    return Err(Code::MissingField.failure_msg("1pc 번호표 없음"));
                }
                PcMode::V2pc if req.extmap.is_some() || req.codecs.is_some() => {
                    return Err(Code::InvalidPayload.failure_msg("2pc 에 번호표"));
                }
                _ => {}
            }
            let Some((node, inst)) = hub.bus.room_where(&room) else {
                return Err(Code::RoomNotFound.failure());
            };
            admit(hub).await?;
            let _ = hub.bus.room_interest(&room, 1).await;
            let r = forward(hub, s, &node, Some(&inst), op, body).await;
            if let Ok(b) = &r {
                reconcile(hub, s, &node, op, b).await;
            }
            let _ = hub.bus.room_interest(&room, -1).await;
            r
        }
        Op::RoomLeave | Op::PublishTracks | Op::Ready | Op::SubscribeLayer | Op::TrackSet | Op::Message => {
            let v = json(&body)?;
            let room = room_field(&v, "room_id")?;
            let Some((node, inst)) = hub.bus.room_where(&room) else {
                return Err(Code::RoomNotFound.failure());
            };
            let r = forward(hub, s, &node, Some(&inst), op, body).await;
            if op == Op::RoomLeave {
                match &r {
                    Ok(b) => reconcile(hub, s, &node, op, b).await,
                    Err(f) if f.code == Code::NotInRoom.num() || f.code == Code::RoomNotFound.num() => {
                        let mut cur: BTreeSet<String> = s.inner.lock().await.rooms.get(&node).cloned().unwrap_or_default();
                        cur.remove(&room);
                        super::sess::set_rooms(hub, s, &node, cur).await;
                    }
                    Err(_) => {}
                }
            }
            r
        }
        Op::Affiliation => {
            let v = json(&body)?;
            let sel = v.get("pub_select").and_then(|x| x.as_str()).map(str::to_string);
            let desel = v.get("pub_deselect").and_then(|x| x.as_str()).map(str::to_string);
            for r in [&sel, &desel].into_iter().flatten() {
                if !common::bus::room_id_ok(r) || r.len() > 255 {
                    return Err(Code::InvalidPayload.failure_msg("room_id 형식"));
                }
            }
            let target = match (&sel, &desel) {
                (None, None) => {
                    return Err(Code::MissingField.failure_msg("pub_select · pub_deselect 둘 다 없음"));
                }
                (Some(a), Some(b)) => hub.bus.room_where(a).or_else(|| hub.bus.room_where(b)).ok_or_else(|| Code::RoomNotFound.failure())?,
                (Some(a), None) | (None, Some(a)) => hub.bus.room_where(a).ok_or_else(|| Code::RoomNotFound.failure())?,
            };
            let r = forward(hub, s, &target.0, Some(&target.1), op, body).await;
            if let Ok(b) = &r {
                reconcile(hub, s, &target.0, op, b).await;
            }
            r
        }
        Op::IceRestart => {
            let v = json(&body)?;
            let Some(sfu) = v.get("sfu_id").and_then(|x| x.as_str()) else {
                return Err(Code::MissingField.failure_msg("sfu_id 없음"));
            };
            let Some(node) = hub.node_of_inst(sfu) else {
                return Err(Code::PeerNotFound.failure());
            };
            let r = forward(hub, s, &node, Some(sfu), op, body).await;
            if let Err(f) = &r
                && f.code == Code::PeerNotFound.num()
            {
                super::sess::set_rooms(hub, s, &node, BTreeSet::new()).await;
            }
            r
        }
        _ => Err(Code::UnknownOp.failure()),
    }
}

async fn task(hub: &Arc<Hub>, body: &[u8]) -> Result<Vec<u8>, Failure> {
    let v = json(body)?;
    if v.get("req_id").is_none() || v.get("type").is_none() || v.get("phase").is_none() {
        return Err(Code::MissingField.failure_msg("phase · req_id · type"));
    }
    let t: TaskReq = serde_json::from_value(v).map_err(|_| Code::InvalidPayload.failure())?;
    match t {
        TaskReq::Request { .. } => Ok(Vec::new()),
        TaskReq::Report { req_id, result, .. } => {
            hub.aux.report(req_id, result).await;
            Ok(Vec::new())
        }
    }
}
