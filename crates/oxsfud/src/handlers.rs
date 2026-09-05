// author: kodeholic (powered by Claude)
//! op 처리 — 정§4-2 입장 판정 ②~⑥ · §17-2 퇴장(이 판은 ②④⑦) · §5-2 소속 · HTTP 대행(연§5-3~5-5).
//! 전부 동기·메모리 안이다. 응답은 요청 하나에 정확히 하나(정§19 ②).

use std::sync::Arc;

use common::bplane::{Envelope, iop};
use oxsig::body::affiliation::{AffiliationReq, AffiliationRes, Cause};
use oxsig::body::notify::{ParticipantEvent, ParticipantEventType};
use oxsig::body::room::{PARTICIPANT_RECORDER, RoomJoinReq, RoomJoinRes, RoomLeaveReq, RoomLeaveRes};
use oxsig::frame::{self, Header, Kind, encode_json};
use oxsig::op::Op;
use oxsig::schema::{CodecSpec, DtlsConfig, Extmap, IceConfig, MediaKind, PcMode, ServerConfig, Version};
use oxsig::{FailCode, Failure};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::emit::EventBus;
use crate::peer::{Peer, PeerMap};
use crate::room::{Created, Member, Room, RoomRegistry, RoomSpec};

/// 정§4-2 ④ — 서버마다 따로 센다.
pub const MAX_ROOMS_PER_USER: usize = 100;

/// `server_config` 의 프로세스 고정 재료(정§4-2 emit 표).
#[derive(Debug, Clone)]
pub struct MediaParams {
    pub public_ip: String,
    pub udp_port: u16,
    pub fingerprint: String,
    pub max_bitrate_bps: u64,
}

pub struct Sfu {
    pub epoch: String,
    pub rooms: RoomRegistry,
    pub peers: PeerMap,
    pub bus: EventBus,
    pub media: MediaParams,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn ok(op: u16, pid: u32, body: &Value) -> Vec<u8> {
    encode_json(&Header { kind: Kind::Ok, reserved: 0, op, pid }, body)
}

fn fail(op: u16, pid: u32, f: &Failure) -> Vec<u8> {
    encode_json(&Header { kind: Kind::Fail, reserved: 0, op, pid }, &serde_json::to_value(f).unwrap_or(Value::Null))
}

fn parse<T: DeserializeOwned>(body: &Value) -> Result<T, Failure> {
    serde_json::from_value(body.clone()).map_err(|e| {
        let m = e.to_string();
        if m.starts_with("missing field") { Failure::new(FailCode::MissingField).message(m) } else { Failure::new(FailCode::InvalidPayload).message(m) }
    })
}

impl Sfu {
    pub fn new(epoch: String, media: MediaParams) -> Self {
        Self { epoch, rooms: RoomRegistry::default(), peers: PeerMap::default(), bus: EventBus::default(), media }
    }

    /// envelope 하나 → 응답 wire 하나.
    pub fn handle(&self, env: &Envelope) -> Vec<u8> {
        let (h, body) = match frame::decode(&env.wire) {
            Ok(x) => x,
            Err(e) => return fail(0, 0, &Failure::new(FailCode::InvalidPayload).message(e.to_string())),
        };
        let body = match frame::body_json(body) {
            Ok(v) => v,
            Err(e) => return fail(h.op, h.pid, &Failure::new(FailCode::InvalidPayload).message(e.to_string())),
        };
        let r = match h.op {
            iop::ROOM_LIST => Ok(self.room_list()),
            iop::ROOM_CREATE => self.room_create(&body),
            iop::ROOM_GET => self.room_get(&env.user_id, &body),
            code => match Op::from_code(code) {
                Some(Op::RoomJoin) => self.room_join(env, &body),
                Some(Op::RoomLeave) => self.room_leave(&env.user_id, &body),
                Some(Op::Affiliation) => self.affiliation(&env.user_id, &body),
                Some(op) => Err(Failure::new(FailCode::InternalError).message(format!("{} not implemented in this build", op.name()))),
                None => Err(Failure::new(FailCode::UnknownOp)),
            },
        };
        match r {
            Ok(v) => ok(h.op, h.pid, &v),
            Err(f) => fail(h.op, h.pid, &f),
        }
    }

    // ───────── 방(HTTP 대행) ─────────

    fn room_list(&self) -> Value {
        json!({ "rooms": self.rooms.list() })
    }

    fn room_create(&self, body: &Value) -> Result<Value, Failure> {
        let room_id = body["room_id"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| Failure::new(FailCode::MissingField).message("room_id"))?;
        let name = body["name"].as_str().ok_or_else(|| Failure::new(FailCode::MissingField).message("name"))?;
        let capacity = body["capacity"].as_u64().unwrap_or(1000);
        if !(1..=1000).contains(&capacity) {
            return Err(Failure::new(FailCode::InvalidPayload).message("capacity 1~1000"));
        }
        let spec = RoomSpec {
            room_id: room_id.to_owned(),
            name: name.to_owned(),
            capacity: capacity as u32,
            unused_ttl_secs: body["unused_ttl_secs"].as_u64(),
            departure_ttl_secs: body["departure_ttl_secs"].as_u64(),
        };
        let room = match self.rooms.create(spec, now_ms()) {
            Created::New(r) => {
                info!(room = %r.id, capacity = r.capacity, unused_ttl = ?r.unused_ttl_secs, departure_ttl = ?r.departure_ttl_secs, "room created");
                self.bus.lifecycle(&r.id, true);
                r
            }
            Created::Existing(r) => r,
        };
        Ok(json!({ "room_id": room.id, "name": room.name, "capacity": room.capacity, "created_at": room.created_at_ms }))
    }

    /// 연§5-5 — `mid` 는 요청자가 입장 중일 때만(정§14-4). 트랙은 이 판에 없어 `[]`.
    fn room_get(&self, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let room_id = body["room_id"].as_str().ok_or_else(|| Failure::new(FailCode::MissingField).message("room_id"))?;
        let room = self.rooms.get(room_id).ok_or_else(|| Failure::new(FailCode::RoomNotFound))?;
        let mut v = json!({
            "room_id": room.id, "name": room.name, "capacity": room.capacity, "user_count": room.user_count(),
            "participants": room.participants(), "created_at": room.created_at_ms, "version": room.version(&self.epoch),
        });
        if body["tracks"].as_bool().unwrap_or(false) {
            let _with_mid = !user_id.is_empty() && room.is_member(user_id);
            v["tracks"] = json!([]);
        }
        Ok(v)
    }

    /// 정§4-1 만료·명시 삭제 — 폭파와 통보는 한 쌍.
    pub fn destroy_room(&self, room_id: &str) -> bool {
        let Some(r) = self.rooms.remove_if_unoccupied(room_id) else { return false };
        info!(room = %r.id, "room destroyed");
        self.bus.lifecycle(room_id, false);
        true
    }

    pub fn sweep(&self) {
        for id in self.rooms.sweep(now_ms()) {
            self.destroy_room(&id);
        }
    }

    // ───────── 입장·퇴장 ─────────

    fn room_join(&self, env: &Envelope, body: &Value) -> Result<Value, Failure> {
        let req: RoomJoinReq = parse(body)?;
        let room = self.rooms.get(&req.room_id).ok_or_else(|| Failure::new(FailCode::RoomNotFound))?;
        let user_id = env.user_id.as_str();
        // ② 재입장 = 그 방에 이미 있다 → 옛 Peer 를 통째로 축출하고 새 Peer(새 자격).
        let existing = self.peers.get(user_id);
        let peer = match existing {
            Some(p) if p.is_in(&room.id) => {
                warn!(user = user_id, room = %room.id, rooms = p.room_count(), "re-entry: evicting old peer");
                self.evict(&p);
                None
            }
            other => other,
        };
        // ③ 정원 — 축출 뒤 값으로 센다.
        if room.is_full_for(req.participant_type) {
            return Err(Failure::new(FailCode::RoomFull).message(format!("capacity {}", room.capacity)));
        }
        // ④ 입장 방 수.
        let current = peer.as_ref().map_or(0, |p| p.room_count());
        if current >= MAX_ROOMS_PER_USER {
            return Err(Failure::new(FailCode::ListenLimit).details(json!({ "limit": MAX_ROOMS_PER_USER, "current": current })));
        }
        let pc_mode: PcMode = serde_json::from_value(Value::String(env.pc_mode.clone())).unwrap_or(PcMode::TwoPc);
        let peer = peer.unwrap_or_else(|| {
            let p = Arc::new(Peer::new(user_id, req.participant_type, pc_mode, now_ms()));
            self.peers.insert(p.clone());
            p
        });
        // ⑥ 명단 등록 · select 에코 · pub_room · seq++ · emit — 한 임계 구역.
        let version = {
            let _g = room.gate.lock().unwrap_or_else(|e| e.into_inner());
            if !room.insert(user_id, Member { role: req.role, select: req.select, participant_type: req.participant_type, joined_at_ms: now_ms() }) {
                return Err(Failure::new(FailCode::InternalError).message("member already present after eviction"));
            }
            peer.join_room(&room.id, req.select);
            let v = room.seq.bump(&self.epoch);
            if req.participant_type != PARTICIPANT_RECORDER {
                let ev = ParticipantEvent { event_type: ParticipantEventType::Joined, room_id: room.id.clone(), user_id: user_id.to_owned(), role: Some(req.role), select: Some(req.select), version: v.clone() };
                self.bus.room(&room.id, Op::ParticipantEvent, &ev, &[user_id.to_owned()]);
            }
            v
        };
        info!(user = user_id, room = %room.id, select = req.select, seq = version.seq, "joined");
        let res = RoomJoinRes {
            room_id: room.id.clone(),
            participants: room.participants(),
            affiliation: peer.affiliation(),
            server_config: self.server_config(&peer),
            tracks: Vec::new(),
            version,
        };
        Ok(serde_json::to_value(res).unwrap_or(Value::Null))
    }

    fn room_leave(&self, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let req: RoomLeaveReq = parse(body)?;
        let room = self.rooms.get(&req.room_id).ok_or_else(|| Failure::new(FailCode::RoomNotFound))?;
        let peer = self.peers.get(user_id).ok_or_else(|| Failure::new(FailCode::NotInRoom))?;
        let version = self.leave_one(&peer, &room).ok_or_else(|| Failure::new(FailCode::NotInRoom))?;
        let res = RoomLeaveRes { room_id: room.id.clone(), affiliation: peer.affiliation(), version };
        Ok(serde_json::to_value(res).unwrap_or(Value::Null))
    }

    /// 정§17-2 — ② 명단 제거 · ④ peer.leave_room(마지막 방이면 Peer 제거) · ⑦ left broadcast. 없으면 `None`(거짓 성공 금지).
    fn leave_one(&self, peer: &Arc<Peer>, room: &Arc<Room>) -> Option<Version> {
        let version = {
            let _g = room.gate.lock().unwrap_or_else(|e| e.into_inner());
            let member = room.remove(&peer.user_id)?;
            let out = peer.leave_room(&room.id);
            if !out.was_member {
                warn!(user = %peer.user_id, room = %room.id, "index mismatch: member without peer room");
            }
            if out.last_room {
                self.peers.remove(&peer.user_id);
            }
            let v = room.seq.bump(&self.epoch);
            if !member.is_recorder() {
                let ev = ParticipantEvent { event_type: ParticipantEventType::Left, room_id: room.id.clone(), user_id: peer.user_id.clone(), role: None, select: None, version: v.clone() };
                self.bus.room(&room.id, Op::ParticipantEvent, &ev, std::slice::from_ref(&peer.user_id));
            }
            v
        };
        info!(user = %peer.user_id, room = %room.id, seq = version.seq, "left");
        Some(version)
    }

    /// 정§17-3 — 축출의 단위는 Peer: 모든 방에 ①~⑦(방마다 `left`), ⑧ 은 없다.
    fn evict(&self, peer: &Arc<Peer>) {
        for id in peer.rooms() {
            match self.rooms.get(&id) {
                Some(room) => {
                    self.leave_one(peer, &room);
                }
                None => warn!(user = %peer.user_id, room = %id, "index mismatch: peer room without room"),
            }
        }
        self.peers.remove(&peer.user_id);
    }

    // ───────── 소속 ─────────

    fn affiliation(&self, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let req: AffiliationReq = parse(body)?;
        if req.pub_select.is_none() && req.pub_deselect.is_none() {
            return Err(Failure::new(FailCode::MissingField).message("pub_select or pub_deselect"));
        }
        let peer = self.peers.get(user_id).ok_or_else(|| Failure::new(FailCode::NotInRoom))?;
        let changed = peer.apply_affiliation(req.pub_deselect.as_deref(), req.pub_select.as_deref());
        let mut versions = std::collections::BTreeMap::new();
        for id in changed {
            if let Some(room) = self.rooms.get(&id) {
                let _g = room.gate.lock().unwrap_or_else(|e| e.into_inner());
                versions.insert(id, room.seq.bump(&self.epoch));
            }
        }
        let a = peer.affiliation();
        info!(user = user_id, pub_room = ?a.pub_room, changed = versions.len(), "affiliation");
        let res = AffiliationRes { sub_rooms: a.sub_rooms, pub_room: a.pub_room, cause: Cause::User, change_id: req.change_id, versions };
        Ok(serde_json::to_value(res).unwrap_or(Value::Null))
    }

    // ───────── server_config ─────────

    /// 정§4-2 emit 표 — 1pc 도 자격 넷 다 싣는다. `rtcp_fb` 는 연§9-9 예시 그대로.
    pub fn server_config(&self, peer: &Peer) -> ServerConfig {
        ServerConfig {
            sfu_id: self.epoch.clone(),
            pc_mode: peer.pc_mode,
            ice: IceConfig {
                ip: self.media.public_ip.clone(),
                port: self.media.udp_port,
                publish_ufrag: peer.publish_ice.ufrag.clone(),
                publish_pwd: peer.publish_ice.pwd.clone(),
                subscribe_ufrag: peer.subscribe_ice.ufrag.clone(),
                subscribe_pwd: peer.subscribe_ice.pwd.clone(),
            },
            dtls: DtlsConfig { fingerprint: self.media.fingerprint.clone(), setup: "passive".to_owned() },
            codecs: vec![
                CodecSpec { kind: MediaKind::Audio, name: "opus".into(), rtcp_fb: vec!["transport-cc".into()] },
                CodecSpec { kind: MediaKind::Video, name: "VP8".into(), rtcp_fb: video_fb() },
                CodecSpec { kind: MediaKind::Video, name: "H264".into(), rtcp_fb: video_fb() },
            ],
            codecs_sub: None,
            extmap: EXTMAP.iter().map(|(id, uri)| Extmap { id: *id, uri: (*uri).to_owned() }).collect(),
            max_bitrate_bps: self.media.max_bitrate_bps,
        }
    }
}

fn video_fb() -> Vec<String> {
    ["nack", "nack pli", "ccm fir", "transport-cc"].iter().map(|s| (*s).to_owned()).collect()
}

/// 연§4-2 확장 헤더 표 전량.
pub const EXTMAP: [(u8, &str); 6] = [
    (1, "urn:ietf:params:rtp-hdrext:sdes:mid"),
    (4, "urn:ietf:params:rtp-hdrext:ssrc-audio-level"),
    (5, "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time"),
    (6, "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"),
    (10, "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id"),
    (11, "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use oxsig::frame::Kind;

    fn sfu() -> Sfu {
        Sfu::new("sfu-test".into(), MediaParams { public_ip: "10.0.0.1".into(), udp_port: 20000, fingerprint: "sha-256 AA".into(), max_bitrate_bps: 800_000 })
    }
    fn env(user: &str, op: u16, body: Value) -> Envelope {
        Envelope { session_id: format!("s-{user}"), user_id: user.into(), room_id: String::new(), target: String::new(), exclude: Vec::new(), wire: encode_json(&Header::msg(op, 7), &body), pc_mode: "2pc".into() }
    }
    fn call(s: &Sfu, user: &str, op: u16, body: Value) -> (Kind, Value) {
        let w = s.handle(&env(user, op, body));
        let (h, b) = frame::decode(&w).unwrap();
        assert_eq!(h.pid, 7);
        (h.kind, frame::body_json(b).unwrap())
    }
    fn create(s: &Sfu, id: &str, cap: u64) {
        let (k, _) = call(s, "", iop::ROOM_CREATE, json!({"room_id": id, "name": "n", "capacity": cap}));
        assert_eq!(k, Kind::Ok);
    }

    #[test]
    fn join_response_shape_and_events() {
        let s = sfu();
        let mut rx = s.bus.subscribe();
        create(&s, "r1", 10);
        assert_eq!(frame::decode(&rx.try_recv().unwrap().wire).unwrap().0.op, iop::ROOM_LIFECYCLE);
        let (k, b) = call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r1"}));
        assert_eq!(k, Kind::Ok);
        let res: RoomJoinRes = serde_json::from_value(b).unwrap();
        assert_eq!((res.version.seq, res.participants.len(), res.affiliation.pub_room.as_deref()), (1, 1, Some("r1")));
        assert_eq!((res.server_config.sfu_id.as_str(), res.server_config.ice.publish_ufrag.len(), res.server_config.extmap.len()), ("sfu-test", 8, 6));
        let ev = rx.try_recv().unwrap();
        assert_eq!((ev.room_id.as_str(), ev.exclude.as_slice()), ("r1", &["u1".to_owned()][..]));
        let (h, body) = frame::decode(&ev.wire).unwrap();
        let pe: ParticipantEvent = serde_json::from_value(frame::body_json(body).unwrap()).unwrap();
        assert_eq!((h.op, pe.event_type, pe.select), (Op::ParticipantEvent.code(), ParticipantEventType::Joined, Some(true)));
        let (k, b) = call(&s, "u1", Op::RoomLeave.code(), json!({"room_id": "r1"}));
        assert_eq!((k, b["version"]["seq"].as_u64(), b["affiliation"]["pub_room"].is_null()), (Kind::Ok, Some(2), true));
        assert!(s.peers.is_empty(), "last room removes the peer");
        let (k, b) = call(&s, "u1", Op::RoomLeave.code(), json!({"room_id": "r1"}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(3002)));
    }

    #[test]
    fn judgement_order_capacity_reentry_limit() {
        let s = sfu();
        create(&s, "full", 1);
        create(&s, "other", 5);
        assert_eq!(call(&s, "a", Op::RoomJoin.code(), json!({"room_id": "full"})).0, Kind::Ok);
        assert_eq!(call(&s, "a", Op::RoomJoin.code(), json!({"room_id": "other", "select": false})).0, Kind::Ok);
        let (k, b) = call(&s, "b", Op::RoomJoin.code(), json!({"room_id": "full"}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(4001)));
        let old_creds = s.peers.get("a").unwrap().publish_ice.clone();
        let mut rx = s.bus.subscribe();
        // 재입장 take-over — 옛 Peer 의 모든 방에서 left, 정원이 찬 방에서도 성공, 새 자격.
        let (k, b) = call(&s, "a", Op::RoomJoin.code(), json!({"room_id": "full"}));
        assert_eq!(k, Kind::Ok);
        assert_eq!(b["affiliation"]["sub_rooms"], json!(["full"]));
        assert_ne!(s.peers.get("a").unwrap().publish_ice, old_creds);
        let mut lefts = 0;
        while let Ok(e) = rx.try_recv() {
            let (_, body) = frame::decode(&e.wire).unwrap();
            if frame::body_json(body).unwrap()["type"] == "left" { lefts += 1; }
        }
        assert_eq!(lefts, 2);
        assert!(!s.rooms.get("other").unwrap().is_member("a"));
        let (k, b) = call(&s, "", iop::ROOM_GET, json!({"room_id": "nope"}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(3001)));
        let (k, b) = call(&s, "zz", Op::RoomLeave.code(), json!({"room_id": "nope"}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(3001)));
    }

    #[test]
    fn affiliation_bumps_both_rooms_and_keeps_invariant() {
        let s = sfu();
        create(&s, "a", 5);
        create(&s, "b", 5);
        call(&s, "u", Op::RoomJoin.code(), json!({"room_id": "a"}));
        call(&s, "u", Op::RoomJoin.code(), json!({"room_id": "b", "select": false}));
        let (k, b) = call(&s, "u", Op::Affiliation.code(), json!({"pub_select": "b", "change_id": "c1"}));
        let res: AffiliationRes = serde_json::from_value(b).unwrap();
        assert_eq!((k, res.pub_room.as_deref(), res.change_id.as_deref(), res.versions.len()), (Kind::Ok, Some("b"), Some("c1"), 2));
        assert!(res.is_consistent());
        let (k, b) = call(&s, "u", Op::Affiliation.code(), json!({"pub_select": "zzz"}));
        assert_eq!((k, b["pub_room"].as_str(), b["versions"].as_object().unwrap().len()), (Kind::Ok, Some("b"), 0));
        let (k, b) = call(&s, "u", Op::Affiliation.code(), json!({}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(1003)));
        let (k, b) = call(&s, "ghost", Op::Affiliation.code(), json!({"pub_select": "a"}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(3002)));
        let (k, b) = call(&s, "u", Op::PublishTracks.code(), json!({}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(5003)));
        let (k, b) = call(&s, "u", 0x0FFF, json!({}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(1001)));
    }

    #[test]
    fn http_proxy_ops() {
        let s = sfu();
        let (k, b) = call(&s, "", iop::ROOM_CREATE, json!({"room_id": "r", "name": "n", "capacity": 1001}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(1002)));
        create(&s, "r", 3);
        let (_, again) = call(&s, "", iop::ROOM_CREATE, json!({"room_id": "r", "name": "other"}));
        assert_eq!(again["name"], "n");
        call(&s, "u", Op::RoomJoin.code(), json!({"room_id": "r", "select": false}));
        let (_, l) = call(&s, "", iop::ROOM_LIST, Value::Null);
        assert_eq!((l["rooms"][0]["user_count"].as_u64(), l["rooms"][0]["rec"].as_bool()), (Some(1), Some(false)));
        let (_, g) = call(&s, "u", iop::ROOM_GET, json!({"room_id": "r", "tracks": true}));
        assert_eq!((g["participants"][0]["select"].as_bool(), g["version"]["seq"].as_u64(), g["tracks"].as_array().map(Vec::len)), (Some(false), Some(1), Some(0)));
        let (_, g0) = call(&s, "", iop::ROOM_GET, json!({"room_id": "r"}));
        assert!(g0.get("tracks").is_none());
        assert!(!s.destroy_room("r"), "occupied → not destroyed");
    }
}
