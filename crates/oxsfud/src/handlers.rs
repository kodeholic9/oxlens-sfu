// author: kodeholic (powered by Claude)
//! op 처리 — 정§4-2 입장 판정 ②~⑥ · §17-2 퇴장(②④⑦⑧) · §5-2 소속 · HTTP 대행(연§5-3~5-5) · §17-1 회수 주체 둘.
//! 응답 경로는 전부 동기·메모리 안이고 요청 하나에 응답 하나다(정§19 ②). 전송 층은 이 상태를 읽고 쓴다.

use std::collections::BTreeSet;
use std::sync::Arc;

use common::bplane::{Envelope, iop};
use oxsig::body::affiliation::{AffiliationReq, AffiliationRes, Cause};
use oxsig::body::data::{MessageRecv, MessageSend, MessageSendRes, Task, TaskPhase};
use oxsig::body::session::{ResumeReq, ResumeRes, RoomSnapshot};
use crate::media::rewriter::Rewrite;
use oxsig::body::media::{PublishAction, PublishTrack, PublishTracksReq, PublishTracksRes, PublishedTrack, ReadyReq, ReadyType, SubscribeLayerReq, TrackSetReq};
use oxsig::mbcp::{self, Msg, MsgType};
use oxsig::body::notify::{ForcedCause, ParticipantEvent, ParticipantEventType, RoomEvent, RoomEventType, TrackAction, TrackEvent, TrackState, TrackStateType};
use oxsig::body::room::{PARTICIPANT_RECORDER, RoomJoinReq, RoomJoinRes, RoomLeaveReq, RoomLeaveRes};
use oxsig::frame::{self, Header, Kind, encode_json};
use oxsig::op::Op;
use oxsig::schema::{Affiliation, CodecSpec, DtlsConfig, Duplex, Extmap, IceConfig, MediaKind, PcMode, ServerConfig, TrackEntry, Version};
use oxsig::{FailCode, Failure};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tracing::{debug, info, warn};

use crate::emit::EventBus;
use crate::media::slot::new_vssrc;
use crate::media::subscribe::{self, SubSpec, SubscribeState, entry_of, SubscriberStream};
use crate::media::{autolayer, nack, priming, rtcp, rtp};
use crate::media::track::{PublishState, PublisherStream, PublisherTrack, StreamSpec};
use crate::media::floor::{Action, Request};
use crate::media::{self, codec};
use crate::peer::{Peer, PeerMap, PeerState, judge};
use crate::room::{Created, Member, Room, RoomRegistry, RoomSpec};
use crate::transport::ServerCert;
use crate::transport::session::{ConnRole, TransportRegistry, TransportSession};

/// 정§4-2 ④ — 서버마다 따로 센다.
pub const MAX_ROOMS_PER_USER: usize = 100;
/// 정§6-3 수치 — 한 사람 활성 논리 스트림. 반복 호출 우회를 봉쇄하는 짝이다.
pub const MAX_ACTIVE_STREAMS: usize = 16;
/// 정§7-4 `T-gate` — READY 미도착 시 스스로 푸는 안전망.
pub const GATE_TIMEOUT_MS: u64 = 5_000;
/// 정§14-3 — READY 뒤 이만큼 송신 계수가 안 움직이면 정체다.
pub const STALL_WINDOW_MS: u64 = 5_000;
/// 정§11-2 — Ingress TWCC 주기. RR·REMB(1초)보다 촘촘해야 추정이 따라온다.
pub const TWCC_INTERVAL_MS: u64 = 100;
/// 정§14-3 `T-stall` — 같은 (user, 방) 재통보 쿨다운. 폭풍 방지다.
pub const T_STALL_MS: u64 = 30_000;
/// 정§11-2 — PLI 스로틀(h 300ms). 인프라 PLI 는 통과한다.
pub const PLI_MIN_GAP_MS: u64 = 300;
/// 서버가 내는 RTCP 의 보고자 SSRC — 발행자 것을 쓰면 자기 보고로 읽힌다.
pub const SERVER_RTCP_SSRC: u32 = 1;
/// 정§11-2 — Ingress RR 주기.
pub const RTCP_REPORT_INTERVAL_MS: u64 = 1_000;
/// 정§7-4 `READY{camera}` — 즉시·150ms 두 번.
const CAMERA_PLI_GAPS: [u64; 2] = [0, 150];
/// 정§9-7 — 허가 집행의 키프레임 burst.
const GRANT_PLI_GAPS: [u64; 3] = [0, 500, 1_500];

/// `server_config` 의 프로세스 고정 재료(정§4-2 emit 표).
#[derive(Debug, Clone)]
pub struct MediaParams {
    pub public_ip: String,
    pub udp_port: u16,
    pub fingerprint: String,
    pub max_bitrate_bps: u64,
    /// 정책서 §3 `bwe_mode` — 발행자 송신 추정을 어느 축으로 먹이나. 둘 다 켜지 않는다.
    pub bwe_mode: BweMode,
    /// 정책서 §3 `auto_layer` — 꺼 두면 수동(`SUBSCRIBE_LAYER`)만 산다. 두 손이 같은 값을 안 다툰다.
    pub auto_layer: AutoLayer,
}

/// 정§10-3 운영 모드 셋 — 정의는 그 절의 모듈이 갖는다. 여기서는 이름만 빌린다.
pub use crate::media::autolayer::Mode as AutoLayer;

/// 프로브 패딩 한 장의 크기 — 이더넷 MTU 안에서 한 장이 최대로 나르는 몫.
const PROBE_MTU: usize = 1_200;

/// 정§11-2 — 하나만 고른다. 둘을 같이 보내면 발행자가 어느 값을 따를지 갈린다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BweMode {
    #[default]
    Twcc,
    Remb,
}

impl BweMode {
    pub fn parse(s: &str) -> Self {
        if s == "remb" { BweMode::Remb } else { BweMode::Twcc }
    }
}

pub struct Sfu {
    pub epoch: String,
    pub rooms: RoomRegistry,
    pub peers: PeerMap,
    pub bus: EventBus,
    pub media: MediaParams,
    pub transport: TransportRegistry,
    pub cert: Arc<ServerCert>,
    /// 정§11-2 — 서버가 먼저 내는 RTCP(RR·PLI)의 출구. 기동이 바인드한 뒤 채운다.
    socket: arc_swap::ArcSwapOption<tokio::net::UdpSocket>,
    /// 정§14-3 — `(user, 방)` 마다 마지막 정체 통보 시각. 쿨다운의 자리다.
    stall_told: dashmap::DashMap<String, u64>,
}

pub fn now_ms() -> u64 {
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
    pub fn new(epoch: String, media: MediaParams, cert: Arc<ServerCert>) -> Self {
        Self { epoch, rooms: RoomRegistry::default(), peers: PeerMap::default(), bus: EventBus::default(), media, transport: TransportRegistry::default(), cert, socket: arc_swap::ArcSwapOption::empty(), stall_told: dashmap::DashMap::new() }
    }

    /// envelope 하나 → 응답 wire 하나. `Arc` 로 받는 것은 ★키프레임 요청처럼 응답을 안 붙잡는
    /// 곁가지가 있어서다(정§7-4) — 그 자리만 태스크로 떨어져 나간다.
    pub fn handle(self: &Arc<Self>, env: &Envelope) -> Vec<u8> {
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
                Some(Op::PublishTracks) => self.publish_tracks(&env.user_id, &body),
                Some(Op::Ready) => self.ready(&env.user_id, &body),
                Some(Op::TrackSet) => self.track_set(&env.user_id, &body),
                Some(Op::SubscribeLayer) => self.subscribe_layer(&env.user_id, &body),
                Some(Op::Resume) => self.resume(&env.user_id, &body),
                Some(Op::Message) => self.message(&env.user_id, &body),
                Some(Op::Task) => self.task(&env.user_id, &body),
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
            // 정§14-4 — `mid` 는 그 세션에 배정된 값이라 요청자가 입장 중일 때만 채운다.
            let entries = self
                .peers
                .get(user_id)
                .filter(|_| !user_id.is_empty() && room.is_member(user_id))
                .map(|peer| {
                    peer.subscribe
                        .in_room(&room.id)
                        .iter()
                        .filter_map(|sub| self.stream_of(&room.id, &sub.track_id).map(|s| entry_of(&s, sub)))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            v["tracks"] = serde_json::to_value(entries).unwrap_or(Value::Null);
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

    // ───────── 입장·퇴장 ─────────

    fn room_join(self: &Arc<Self>, env: &Envelope, body: &Value) -> Result<Value, Failure> {
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
        // 정§12 — `pc_mode` 는 세션 확정값이다. 모르는 값을 조용히 갈음하지 않는다.
        let pc_mode: PcMode = serde_json::from_value(Value::String(env.pc_mode.clone()))
            .map_err(|_| Failure::new(FailCode::InvalidPayload).message(format!("pc_mode {}", env.pc_mode)))?;
        let peer = match peer {
            Some(p) => p,
            None => {
                let priority = u8::try_from(env.floor_priority).unwrap_or(u8::MAX);
                let p = Arc::new(Peer::new(user_id, req.participant_type, pc_mode, now_ms()).with_floor_priority(priority));
                self.peers.insert(p.clone());
                self.transport.register(&p);
                p
            }
        };
        // ⑤ 구독 배관 — mid 가 모자라면 이 입장을 `4005` 로 거절한다(정§7-2 고갈 규칙).
        let tracks = self.subscribe_room(&peer, &room)?;
        // ⑥ 명단 등록 · select 에코 · pub_room · seq++ · emit — 한 임계 구역.
        let version = {
            let _g = room.gate.lock().unwrap_or_else(|e| e.into_inner());
            if !room.insert(user_id, Member { role: req.role, select: req.select, participant_type: req.participant_type, joined_at_ms: now_ms() }) {
                return Err(Failure::new(FailCode::InternalError).message("member already present after eviction"));
            }
            peer.join_room(&room, req.select);
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
            tracks,
            version,
        };
        Ok(serde_json::to_value(res).unwrap_or(Value::Null))
    }

    /// 연§6-1 · 정§3-3 — 시그널만 끊겼다 다시 붙었다. ★**방 단위로 판정한다**.
    ///
    /// 방 열 개 중 하나가 어긋났다고 열 개를 다시 만들지 않는다 — 다방 청취가 기본이라
    /// 방 하나 때문에 나머지 아홉을 끊으면 무전이 통째로 멎는다.
    /// ★발행 트랙은 방과 독립으로 판정한다 — 트랙은 세션 것이지 방에 못박히지 않는다(연§6-4).
    fn resume(&self, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let req: ResumeReq = parse(body)?;
        let peer = self.peers.get(user_id).ok_or_else(|| Failure::new(FailCode::SessionNotFound))?;
        let mut res = ResumeRes::default();

        for room_id in &req.rooms {
            let Some(room) = self.rooms.get(room_id) else {
                res.failed.push(room_id.clone());
                res.reason.insert(room_id.clone(), FailCode::RoomNotFound.name().to_owned());
                continue;
            };
            if !room.is_member(user_id) {
                res.failed.push(room_id.clone());
                res.reason.insert(room_id.clone(), FailCode::NotInRoom.name().to_owned());
                continue;
            }
            // ★스냅샷이 끊겨 있는 동안 사라진 통지를 대신한다. 없으면 그 사이 트랙이 영영 안 붙는다.
            res.snapshot.insert(room_id.clone(), RoomSnapshot {
                participants: room.participants(),
                tracks: self.tracks_for(&peer, &room),
                version: room.version(&self.epoch),
            });
            res.resumed.push(room_id.clone());
        }

        // ★신고가 곧 발행 상태다 — 신고에서 빠진 트랙은 서버가 지운다(remove 와 같은 경로).
        let reported: BTreeSet<&str> = req.publish.iter().map(|t| t.track_id.as_str()).collect();
        let mine: Vec<String> = peer.publish.all().iter().map(|s| s.track_id.clone()).collect();
        for track_id in &mine {
            if !reported.contains(track_id.as_str()) {
                self.retire_track(&peer, track_id);
            }
        }
        for want in &req.publish {
            if !mine.contains(&want.track_id) {
                res.publish_failed.push(want.track_id.clone());
            }
        }
        Ok(serde_json::to_value(res).unwrap_or(Value::Null))
    }

    /// 연§6-1 — 신고에서 빠진 발행 트랙을 지운다. `remove` 와 같은 경로를 지나야
    /// 배관·슬롯 정리가 한 곳에서만 일어난다.
    fn retire_track(&self, peer: &Arc<Peer>, track_id: &str) {
        let Some(stream) = peer.publish.get(track_id) else { return };
        let Some(room) = self.rooms.get(&stream.room_id) else { return };
        let req = PublishTracksReq {
            action: PublishAction::Remove,
            room_id: room.id.clone(),
            tracks: Vec::new(),
            track_ids: vec![track_id.to_owned()],
            twcc_extmap_id: None,
            rid_extmap_id: None,
            repair_rid_extmap_id: None,
            mid_extmap_id: None,
            audio_level_extmap_id: None,
        };
        if self.publish_remove(peer, &room, &req).is_err() {
            warn!(user = %peer.user_id, track = %track_id, "resume retire failed");
        }
    }

    /// 그 방에서 이 사람이 받는 트랙 전량 — 스냅샷의 재료다(연§4-1 보관본과 같은 형).
    fn tracks_for(&self, peer: &Arc<Peer>, room: &Arc<Room>) -> Vec<TrackEntry> {
        peer.subscribe
            .in_room(&room.id)
            .iter()
            .filter_map(|sub| self.stream_of(&room.id, &sub.track_id).map(|s| entry_of(&s, sub)))
            .collect()
    }

    /// 연§6-5 · 정§13 — 방 broadcast 중계. ★발신자는 뺀다(자기 것은 응답의 `pid` 로 안다).
    /// ★`user_id` 는 세션에서 넣는다 — body 의 것을 믿으면 아무나 남을 사칭한다.
    fn message(&self, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let req: MessageSend = parse(body)?;
        let room = self.rooms.get(&req.room_id).ok_or_else(|| Failure::new(FailCode::NotInRoom))?;
        if !room.is_member(user_id) {
            return Err(Failure::new(FailCode::NotInRoom));
        }
        let recv = MessageRecv { room_id: room.id.clone(), user_id: user_id.to_owned(), content: req.content };
        self.bus.room(&room.id, Op::Message, &recv, std::slice::from_ref(&user_id.to_owned()));
        let res = MessageSendRes { msg_id: uuid::Uuid::new_v4().simple().to_string() };
        Ok(serde_json::to_value(res).unwrap_or(Value::Null))
    }

    /// 연§6-6 — 서버가 발제하고 클라가 `report` 를 새 프레임으로 보낸다.
    /// ★모르는 `type` 은 조용히 버리지 않는다 — 버리면 발제자가 타임아웃까지 기다린다.
    fn task(&self, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let task: Task = parse(body)?;
        if !task.is_known_type() {
            return Err(Failure::new(FailCode::InvalidPayload).message("unknown task type"));
        }
        // 이 자리에 오는 것은 클라의 보고뿐이다 — 발제는 서버가 낸다.
        if task.phase != TaskPhase::Report {
            return Err(Failure::new(FailCode::InvalidPayload).message("phase"));
        }
        info!(user = %user_id, req_id = task.req_id, task = %task.task_type, "task report");
        Ok(Value::Null)
    }

    fn room_leave(self: &Arc<Self>, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let req: RoomLeaveReq = parse(body)?;
        let room = self.rooms.get(&req.room_id).ok_or_else(|| Failure::new(FailCode::RoomNotFound))?;
        let peer = self.peers.get(user_id).ok_or_else(|| Failure::new(FailCode::NotInRoom))?;
        let version = self.leave_one(&peer, &room).ok_or_else(|| Failure::new(FailCode::NotInRoom))?;
        let res = RoomLeaveRes { room_id: room.id.clone(), affiliation: peer.affiliation(), version };
        Ok(serde_json::to_value(res).unwrap_or(Value::Null))
    }

    /// 정§17-2 — ② 명단 제거 · ④ peer.leave_room(마지막 방이면 Peer 제거) · ⑦ left broadcast. 없으면 `None`(거짓 성공 금지).
    fn leave_one(self: &Arc<Self>, peer: &Arc<Peer>, room: &Arc<Room>) -> Option<Version> {
        // 정§17-2 ① — 발언권 정리가 먼저다(화자면 회수·승계, 큐에서도 제거).
        if !room.is_member(&peer.user_id) {
            return None;
        }
        self.floor_leave(room, &peer.user_id);
        let version = {
            let _g = room.gate.lock().unwrap_or_else(|e| e.into_inner());
            let member = room.remove(&peer.user_id)?;
            self.teardown_in_room(peer, room);
            let out = peer.leave_room(&room.id);
            if !out.was_member {
                warn!(user = %peer.user_id, room = %room.id, "index mismatch: member without peer room");
            }
            if out.last_room {
                self.peers.remove(&peer.user_id);
                self.transport.unregister(&peer.user_id);
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


    /// 정§17-2 ③⑥ — 내가 받던 배관을 걷고, 내가 내던 것을 남은 구독자에게서 지운다.
    /// ★방 `gate` 를 쥔 채로 부른다(퇴장 통지와 같은 임계 구역).
    fn teardown_in_room(&self, peer: &Arc<Peer>, room: &Room) {
        for sub in peer.subscribe.in_room(&room.id) {
            if let Some(stream) = self.stream_of(&room.id, &sub.track_id) {
                stream.detach_all(&peer.user_id, &room.id);
            }
            peer.subscribe.remove(&peer.user_id, &room.id, &sub.track_id);
        }
        let mine = peer.publish.in_room(&room.id);
        for s in &mine {
            peer.publish.remove(&s.track_id);
        }
        let personal: Vec<Arc<PublisherStream>> = mine.iter().filter(|s| s.duplex() == Duplex::Full).cloned().collect();
        self.withdraw(room, &personal);
        if mine.iter().any(|s| s.duplex() == Duplex::Half && s.kind == MediaKind::Video) {
            self.retire_video_slot(room);
        }
    }

    /// 정§17-3 — 축출의 단위는 Peer: 모든 방에 ①~⑦(방마다 `left`), ⑧ 은 없다.
    fn evict(self: &Arc<Self>, peer: &Arc<Peer>) {
        for id in peer.rooms() {
            match self.rooms.get(&id) {
                Some(room) => {
                    self.leave_one(peer, &room);
                }
                None => warn!(user = %peer.user_id, room = %id, "index mismatch: peer room without room"),
            }
        }
        self.peers.remove(&peer.user_id);
        self.transport.unregister(&peer.user_id);
    }

    // ───────── 전송 층이 부르는 자리 ─────────

    /// 정§2-2 — `last_seen` 의 갱신원은 UDP 관찰 둘뿐이다. WS 하트비트는 여기 오지 않는다.
    pub fn observe_media(&self, user_id: &str) {
        if let Some(peer) = self.peers.get(user_id) {
            peer.touch(now_ms());
        }
    }

    /// 정§13 — `svc` 별 입구. 발언권만 이 문서가 정하고(연§11-6), 나머지는 계수하고 버린다.
    pub fn on_dc_frame(self: &Arc<Self>, session: &Arc<TransportSession>, svc: u8, payload: &[u8]) {
        self.observe_media(&session.user_id);
        if svc != oxsig::dc::SVC_MBCP {
            debug!(user = %session.user_id, svc, len = payload.len(), "dc frame dropped");
            return;
        }
        let Some(msg) = Msg::decode(payload) else {
            debug!(user = %session.user_id, len = payload.len(), "mbcp decode");
            return;
        };
        for action in self.floor_message(&session.user_id, &msg) {
            self.dispatch_floor(&action);
        }
    }

    /// 정§9-6 입구 관문 — ★`0x1D` 없음/미지 = 계수 후 무응답(돌려줄 방이 없다) ·
    /// 미입장 = `DENY(255, "not_in_room")`(원문에 자리가 없는 우리 쪽 오류). ★조용히 첫 방을 고르지 않는다.
    fn floor_message(self: &Arc<Self>, user_id: &str, msg: &Msg) -> Vec<Action> {
        let Some(room_id) = msg.room_id() else {
            debug!(user = user_id, msg = msg.msg_type.name(), "mbcp without room");
            return Vec::new();
        };
        let Some(room) = self.rooms.get(room_id) else {
            debug!(user = user_id, room = room_id, "mbcp for unknown room");
            return Vec::new();
        };
        let Some(peer) = self.peers.get(user_id) else { return Vec::new() };
        if !peer.is_in(&room.id) {
            let deny = Msg::new(MsgType::Deny)
                .ack(true)
                .u8(mbcp::field::CAUSE, mbcp::reject::OTHER)
                .str(mbcp::field::CAUSE_TEXT, "not_in_room")
                .room(&room.id);
            return vec![Action::Unicast { to: user_id.to_owned(), msg: deny }];
        }
        let now = now_ms();
        let blocked = self.blocked_in(&room);
        let actions = match msg.msg_type {
            MsgType::Request => {
                let req = Request {
                    user: user_id.to_owned(),
                    // 정§9-5 — 권위는 토큰 클레임이다. 클라 선언값으로 선점을 결정하지 않는다.
                    eff_priority: msg.get_u8(mbcp::field::PRIORITY).unwrap_or(0).min(peer.floor_priority),
                    duration_secs: msg.get_u16(mbcp::field::DURATION),
                    has_half_track: self.has_half_track(&peer, &room),
                    alone: room.user_count() <= 1,
                };
                room.floor.request(&req, &blocked, now)
            }
            MsgType::Release => room.floor.release(user_id, &blocked, now),
            MsgType::QueuePosRequest => room.floor.queue_position(user_id),
            // 정§9-4 — `FLOOR_ACK` 는 도달 관측이지 재전송 사유가 아니다.
            MsgType::Ack => {
                debug!(user = user_id, room = %room.id, acked = msg.get_u8(mbcp::field::ACK_TYPE), "floor ack");
                Vec::new()
            }
            other => {
                debug!(user = user_id, msg = other.name(), "mbcp message is server to client only");
                Vec::new()
            }
        };
        self.settle_floor(&room, actions)
    }

    /// 정§9-2 — cross-room 검사(Peer 축). 큐 후보와 요청자만 보므로 최대 열한 번 읽는다.
    fn blocked_in(&self, room: &Arc<Room>) -> std::collections::BTreeSet<String> {
        room.floor
            .candidates()
            .into_iter()
            .chain(room.member_ids())
            .filter(|u| self.peers.get(u).is_some_and(|p| p.holds_floor_elsewhere(&room.id)))
            .collect()
    }

    /// 정§9-6 관문 ② — 그 user 에 이 방 반이중 발행 트랙이 있나.
    fn has_half_track(&self, peer: &Peer, room: &Room) -> bool {
        peer.publish.in_room(&room.id).iter().any(|s| s.duplex() == Duplex::Half)
    }

    /// 상태기가 낸 액션을 집행하기 전에 Peer 축(§9-2 cross-room)을 맞춘다.
    /// 정§9-7 — 허가가 섰으면 그 방 video 슬롯에 키프레임 burst 를 건다(결합은 흐름의 재료다).
    fn settle_floor(self: &Arc<Self>, room: &Arc<Room>, actions: Vec<Action>) -> Vec<Action> {
        let granted = actions.iter().any(|a| a.msg().msg_type == MsgType::Granted);
        if granted && let Some(slot) = room.slots.video() {
            self.spawn_keyframe_burst(vec![slot.vssrc], &GRANT_PLI_GAPS);
        }
        // 정§11-1 ① — 허가가 서는 ★그 순간 egress seq 공간이 갈린다. 앞의 재전송 요구는 전부
        // stale 이므로 슬롯 구독자의 캐시를 여기서 비운다. ★새 화자의 첫 패킷을 기다리면 안 된다 —
        // 옛 화자의 마지막 패킷과 새 화자의 첫 패킷 사이에 온 NACK 이 옛 공간을 그대로 되받는다.
        if granted {
            let now = now_ms();
            for slot in room.slots.all() {
                for t in slot.tracks().iter() {
                    for weak in t.subscribers().iter() {
                        if let Some(sub) = weak.upgrade() {
                            sub.rtx.reset(now);
                        }
                    }
                }
            }
        }
        // 정§9-7 — 허가 직후 슬롯을 데운다. 조용하던 슬롯에 바로 말하면 첫 음절이 잘린다.
        if granted && let Some(speaker) = room.floor.speaker() {
            self.spawn_priming(room, speaker.to_string(), priming::MAX_FRAMES);
        }
        let holder = room.floor.state();
        for u in room.member_ids() {
            let Some(peer) = self.peers.get(&u) else { continue };
            let mine = holder.speaker() == Some(u.as_str()) && room.floor.holds(&u);
            if mine {
                peer.set_floor_room(Some(&room.id));
            } else if peer.floor_room_is(&room.id) {
                peer.set_floor_room(None);
            }
        }
        actions
    }

    /// 정§9 — 락 밖 집행. DC 는 그 사람의 발행 연결 위에 있다(연§9-6).
    fn dispatch_floor(&self, action: &Action) {
        let Ok(payload) = action.msg().encode() else { return };
        let Ok(frame) = oxsig::dc::encode(oxsig::dc::SVC_MBCP, &payload) else { return };
        match action {
            Action::Unicast { to, .. } => self.send_dc(to, &frame),
            Action::Broadcast { msg, exclude } => {
                let Some(room_id) = msg.room_id().and_then(|r| self.rooms.get(r)) else { return };
                for u in room_id.member_ids() {
                    if exclude.as_deref() == Some(u.as_str()) {
                        continue;
                    }
                    self.send_dc(&u, &frame);
                }
            }
        }
    }

    fn send_dc(&self, user_id: &str, frame: &[u8]) {
        if let Some(session) = self.transport.session_for(user_id, ConnRole::Publish) {
            session.dc_send(frame.to_vec());
        }
    }

    /// 정§17-1 — 발언권 타이머 주기(2,000ms). 회수 tick 에 편승한다.
    fn floor_tick(self: &Arc<Self>, now_ms: u64) {
        for room in self.rooms.all() {
            let blocked = self.blocked_in(&room);
            let actions = room.floor.tick(&blocked, now_ms);
            if actions.is_empty() {
                continue;
            }
            for action in self.settle_floor(&room, actions) {
                self.dispatch_floor(&action);
            }
        }
    }

    /// 정§5-2·§17-2 ① — 그 방에서 화자·대기자였으면 반환·제거한다.
    fn floor_leave(self: &Arc<Self>, room: &Arc<Room>, user_id: &str) {
        let blocked = self.blocked_in(room);
        let actions = room.floor.on_leave(user_id, &blocked, now_ms());
        for action in self.settle_floor(room, actions) {
            self.dispatch_floor(&action);
        }
    }


    // ───────── RTCP(정§11-2) ─────────

    pub fn attach_socket(&self, socket: Arc<tokio::net::UdpSocket>) {
        self.socket.store(Some(socket));
    }

    /// 서버가 먼저 내는 RTCP 하나 — 그 사용자의 발행 연결로.
    async fn send_rtcp(&self, user_id: &str, plain: &[u8]) -> bool {
        let (Some(socket), Some(session)) = (self.socket.load_full(), self.transport.session_for(user_id, ConnRole::Publish)) else {
            return false;
        };
        let (Some(addr), Ok(sealed)) = (session.addr.get(), session.encrypt_rtcp(plain)) else {
            return false;
        };
        socket.send_to(&sealed, addr).await.is_ok()
    }

    /// 정§11-2 — Ingress RR 은 서버가 ★자체 생성한다(1,000ms). 발행자가 보는 "우리 수신 품질"이다.
    /// ★조립 함수는 여기 하나다 — 두 곳에서 부르면 구간 델타가 갈려 값이 틀린다.
    pub async fn emit_receiver_reports(&self, now_ms: u64) -> usize {
        let mut sent = 0;
        for peer in self.peers.snapshot() {
            let blocks: Vec<rtcp::ReportBlock> = peer
                .publish
                .all()
                .iter()
                .flat_map(|st| st.tracks())
                .filter_map(|t| t.reception.report(t.ssrc, now_ms))
                .collect();
            if blocks.is_empty() {
                continue;
            }
            // 보고자 SSRC 는 서버 자신이다 — 발행자의 것을 쓰면 자기 보고로 읽힌다.
            if self.send_rtcp(&peer.user_id, &rtcp::build_rr(SERVER_RTCP_SSRC, &blocks)).await {
                sent += 1;
            }
        }
        sent
    }

    /// 정§10-3 — 레이어 자동 판단 한 눈금. ★판정은 순수 함수가 하고 여기는 신호를 모아 집행만 한다.
    /// 정책이 `off` 면 수동(`SUBSCRIBE_LAYER`)만 산다 — 두 손이 같은 값을 다투지 않게.
    pub fn auto_layer_tick(self: &Arc<Self>, now_ms: u64) -> usize {
        if !self.media.auto_layer.on() {
            return 0;
        }
        let mut moved = 0;
        for peer in self.peers.snapshot() {
            for sub in peer.subscribe.all() {
                if sub.kind != MediaKind::Video {
                    continue;
                }
                let signals = self.signals_for(&sub, now_ms);
                let mut st = sub.auto.lock().unwrap_or_else(|e| e.into_inner());
                // 정§16-2 — ★상태가 아니라 **판단의 입력**을 낸다. "왜 안 올라가나" 를 밖에서 보게.
                let decision = autolayer::policy_tick(&mut st, now_ms, &signals, self.media.auto_layer);
                debug!(user = %peer.user_id, track = %sub.track_id, layer = ?st.layer, ?decision,
                    send_side = ?signals.send_side, send_loss = ?signals.send_loss, remb = ?signals.remb,
                    loss = ?signals.loss, probe = ?signals.probe, nack = signals.nack_per_sec, drops = signals.drops,
                    "auto layer tick");
                match decision {
                    autolayer::Decision::Hold => continue,
                    autolayer::Decision::Demote(cause) => {
                        sub.set_spatial_cap(0);
                        info!(user = %peer.user_id, track = %sub.track_id, ?cause, "layer down");
                    }
                    autolayer::Decision::Promote => {
                        sub.set_spatial_cap(1);
                        info!(user = %peer.user_id, track = %sub.track_id, "layer up");
                    }
                    // ★올릴 조건은 다 섰는데 실측이 없다 — 지어내지 않고 쏴 본다.
                    autolayer::Decision::Probe => {
                        drop(st);
                        self.spawn_probe(&sub, now_ms);
                        continue;
                    }
                }
                moved += 1;
            }
        }
        moved
    }

    /// 정§10-3 v2 — RTX 패딩으로 「이만큼은 지나가나」를 물어본다.
    ///
    /// ★패딩은 구독자가 버린다(모르는 원본의 RTX 다). 우리가 얻는 것은 ★**TWCC 로 되돌아오는
    /// 도착 사실** 하나뿐이고, 그것이 승격의 유일한 근거다. 겹쳐 쏘면 두 판이 서로의 측정을
    /// 오염시키므로 한 번에 한 판만 연다.
    fn spawn_probe(self: &Arc<Self>, sub: &Arc<SubscriberStream>, at_ms: u64) {
        // ★못 쏘는 사유는 남긴다 — 조용히 안 쏘면 "승격이 안 온다" 의 원인을 밖에서 못 본다(정§16-2).
        let (Some(pt), Some(ssrc)) = (sub.rtx_pt(), sub.rtx_vssrc) else {
            debug!(user = %sub.subscriber, track = %sub.track_id, "probe skipped: no rtx pt/ssrc");
            return;
        };
        let Some(transport) = sub.transport.clone() else {
            debug!(user = %sub.subscriber, track = %sub.track_id, "probe skipped: no transport");
            return;
        };
        let Some(socket) = self.socket.load_full() else { return };
        // 지금 받아내는 값의 배율만큼 겨눈다 — 아직 못 쟀으면 지금 층이 기준이다.
        let base = sub.send_side().map_or_else(|| sub.auto.lock().unwrap_or_else(|e| e.into_inner()).layer.bps(), |(b, _)| b);
        let target = autolayer::probe_target_bps(base);
        let until = at_ms + autolayer::PROBE_HOLD_MS;
        if !sub.open_probe(at_ms, until, target) {
            return;
        }
        info!(user = %sub.subscriber, track = %sub.track_id, target, base, "probe up");
        let chunk_bytes = (target / 8 * autolayer::PROBE_CHUNK_MS / 1_000).max(1) as usize;
        let sub = sub.clone();
        tokio::spawn(async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(autolayer::PROBE_HOLD_MS);
            while std::time::Instant::now() < deadline {
                let Some(addr) = transport.addr.get() else { break };
                let mut left = chunk_bytes;
                while left > 0 {
                    let bytes = left.min(PROBE_MTU);
                    left -= bytes;
                    let mut pkt = rtp::probe_padding(pt, ssrc, sub.next_rtx_seq(), bytes);
                    if let Some(id) = sub.twcc_id() {
                        let seq = transport.departures.stamp(now_ms(), pkt.len());
                        rtp::upsert_twcc(&mut pkt, id, seq);
                    }
                    let Ok(sealed) = transport.encrypt_rtp(&pkt) else { break };
                    if socket.send_to(&sealed, addr).await.is_err() {
                        return;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(autolayer::PROBE_CHUNK_MS)).await;
            }
        });
    }

    /// 판정에 쓰는 사실을 모은다. ★못 잰 것은 `None` 이다 — 0 으로 채우면 "쟀는데 0" 과 못 가른다.
    fn signals_for(&self, sub: &Arc<SubscriberStream>, now_ms: u64) -> autolayer::Signals {
        let mut s = autolayer::Signals::default();
        // v1 — REMB 는 서버가 발행자에게 내는 값이 아니라 구독자가 준 추정이다.
        if let Some((bps, at)) = sub.remb_estimate() {
            s.remb = Some((bps, at));
        }
        if let Some((pct, at)) = sub.reported_loss() {
            s.loss = Some((pct, at));
        }
        // v2 — 실측 수신률과 프로브 실증치. ★못 잰 것은 `None` 으로 남긴다.
        s.send_side = sub.send_side();
        s.send_loss = sub.twcc_loss();
        s.probe = sub.probe_result();
        let _ = now_ms;
        s
    }

    /// 정§9-7 — 허가 직후 슬롯 오디오를 무음으로 데운다. ★화자 자신은 뺀다(self-echo).
    /// 화자의 진짜 RTP 가 오면 그 자리에서 멈춘다 — 데우기와 실제 음성이 겹칠 이유가 없다.
    ///
    /// ★해제 뒤에는 흘리지 않는다. 정§9-7 의 "해제가 돌려준 silence 프레임" 은 놓으면서 나오는
    /// 것을 내보내라는 뜻이고, 새로 지어 넣으라는 뜻이 아니다 — 화자 없는 구간에 슬롯이 흐르면
    /// 그것이 곧 게이트 누수다(정§7-3 이 강제 권위인 자리).
    fn spawn_priming(self: &Arc<Self>, room: &Arc<Room>, speaker: String, frames: u32) {
        let sfu = self.clone();
        let room = room.clone();
        tokio::spawn(async move {
            let slot = room.slots.audio.clone();
            for _ in 0..frames {
                // 화자가 말을 시작했다 — 데울 이유가 사라졌다.
                if !speaker.is_empty() && room.floor.heard_media() {
                    return;
                }
                let (seq, ts) = room.slots.next_priming();
                let mut pkt = priming::silence(slot.pt, seq, ts, slot.vssrc);
                if room.slots.rewriter(MediaKind::Audio).rewrite(&mut pkt, priming::SOURCE, slot.vssrc) == Rewrite::Skip {
                    return;
                }
                sfu.broadcast_slot(&slot, &speaker, &pkt).await;
                tokio::time::sleep(std::time::Duration::from_millis(priming::FRAME_INTERVAL_MS)).await;
            }
        });
    }

    /// 슬롯 구독자 전원에게 한 장 — 이름이 비면 아무도 안 뺀다(해제 무음이 그 경우다).
    async fn broadcast_slot(&self, slot: &Arc<PublisherStream>, exclude: &str, packet: &[u8]) {
        let Some(socket) = self.socket.load_full() else { return };
        let Some(track) = slot.tracks().first().cloned() else { return };
        for weak in track.subscribers().iter() {
            let Some(sub) = weak.upgrade() else { continue };
            if sub.subscriber == exclude {
                continue;
            }
            let Some(transport) = sub.transport.as_ref() else { continue };
            let Some(addr) = transport.addr.get() else { continue };
            let mut egress = packet.to_vec();
            rtp::set_payload_type(&mut egress, sub.pt());
            if let Ok(sealed) = transport.encrypt_rtp(&egress) {
                let _ = socket.send_to(&sealed, addr).await;
            }
        }
    }

    /// 정§11-2 — Ingress TWCC. 발행자가 `transport-cc` 를 협상했을 때만 나간다.
    /// ★없으면 발행자 송신 추정이 갱신되지 않는다 — 화질이 안 올라가고 어느 계수에도 안 남는다.
    pub async fn emit_transport_feedback(&self) -> usize {
        if self.media.bwe_mode != BweMode::Twcc {
            return 0;
        }
        let mut sent = 0;
        for peer in self.peers.snapshot() {
            let Some(session) = self.transport.by_ufrag(&peer.publish_ice.ufrag) else { continue };
            let Some(first) = peer.publish.all().iter().flat_map(|st| st.tracks()).map(|t| t.ssrc).next() else {
                continue;
            };
            let Some(pkt) = session.arrivals.drain(SERVER_RTCP_SSRC, first) else { continue };
            if self.send_rtcp(&peer.user_id, &pkt).await {
                sent += 1;
            }
        }
        sent
    }

    /// 정§11-2 — Ingress REMB. 값은 `min(수신측 추정, max_bitrate_bps)` 인데
    /// ★수신측 추정은 자동 레이어 축(정§10-3)이 세운다 — 그것이 서기 전에는 상한이 곧 값이다.
    pub async fn emit_remb(&self) -> usize {
        let mut sent = 0;
        for (user, pkt) in self.remb_targets() {
            if self.send_rtcp(&user, &pkt).await {
                sent += 1;
            }
        }
        sent
    }

    /// 판정만 — 누구에게 무엇을 보낼지. ★송신과 갈라야 소켓 없이도 축 선택을 잰다.
    fn remb_targets(&self) -> Vec<(String, Vec<u8>)> {
        if self.media.bwe_mode != BweMode::Remb {
            return Vec::new();
        }
        self.peers
            .snapshot()
            .into_iter()
            .filter_map(|peer| {
                let ssrcs: Vec<u32> =
                    peer.publish.all().iter().flat_map(|st| st.tracks()).map(|t| t.ssrc).collect();
                (!ssrcs.is_empty()).then(|| {
                    (peer.user_id.clone(), rtcp::build_remb(SERVER_RTCP_SSRC, &ssrcs, self.media.max_bitrate_bps))
                })
            })
            .collect()
    }

    /// 정§11-1 상향 — 결손 장부에서 지금 물을 것을 꺼내 발행자에게 보낸다.
    /// ★소비자는 이 타이머 하나다 — 두 곳에서 꺼내면 같은 결손을 두 번 묻는다.
    pub async fn emit_nacks(&self, now_ms: u64) -> usize {
        let mut sent = 0;
        for peer in self.peers.snapshot() {
            for track in peer.publish.all().iter().flat_map(|st| st.tracks()) {
                let due = track.gaps.due(now_ms);
                if due.is_empty() {
                    continue;
                }
                let pairs = nack::pack(&due);
                if self.send_rtcp(&peer.user_id, &rtcp::build_nack(SERVER_RTCP_SSRC, track.ssrc, &pairs)).await {
                    sent += 1;
                }
            }
        }
        sent
    }

    /// 정§11-2 — 키프레임 요청. `egress_ssrc` 는 구독자가 본 값이라 발행 트랙으로 되짚는다.
    /// `force` 는 인프라 PLI(게이트 해제·승계·`READY`)로 스로틀을 통과한다.
    pub async fn request_keyframe(&self, egress_ssrc: u32, now_ms: u64, force: bool) -> bool {
        let Some((owner, track)) = self.publisher_of_egress(egress_ssrc) else { return false };
        if !track.claim_pli(now_ms, PLI_MIN_GAP_MS, force) {
            return false;
        }
        self.send_rtcp(&owner, &rtcp::build_pli(SERVER_RTCP_SSRC, track.ssrc)).await
    }

    /// 구독자가 보는 SSRC → 발행 물리 트랙. 전이중 non-sim 은 원본이고 반이중은 방 슬롯 값이다(정§8-1).
    fn publisher_of_egress(&self, egress_ssrc: u32) -> Option<(String, Arc<PublisherTrack>)> {
        for peer in self.peers.snapshot() {
            if let Some((_, track)) = peer.publish.by_ssrc(egress_ssrc) {
                return Some((peer.user_id.clone(), track));
            }
        }
        // 슬롯 값이면 지금 그 방에서 말하는 사람의 트랙을 가리킨다.
        // ★찾다 만 방은 건너뛴다 — 여기서 함수를 빠져나가면 첫 방에 없다는 이유로 나머지를 안 본다.
        for room in self.rooms.all() {
            let Some(slot) = room.slots.all().into_iter().find(|s| s.vssrc == egress_ssrc) else { continue };
            let Some(speaker) = room.floor.speaker() else { continue };
            let Some(peer) = self.peers.get(&speaker) else { continue };
            let track = peer
                .publish
                .in_room(&room.id)
                .iter()
                .find(|s| s.kind == slot.kind && s.duplex() == Duplex::Half)
                .and_then(|s| s.tracks().first().cloned());
            if let Some(track) = track {
                return Some((speaker.to_string(), track));
            }
        }
        None
    }

    /// 응답을 붙잡지 않고 낸다 — 키프레임은 요청이지 왕복이 아니다.
    fn spawn_keyframe_burst(self: &Arc<Self>, ssrcs: Vec<u32>, gaps_ms: &'static [u64]) {
        if ssrcs.is_empty() {
            return;
        }
        let sfu = Arc::clone(self);
        tokio::spawn(async move { sfu.keyframe_burst(ssrcs, gaps_ms).await });
    }

    /// 정§7-4 — `READY{tracks}` 는 그 방에 ★한 번, `{camera}` 는 그 트랙에 두 번(즉시·150ms).
    /// 정§9-7 — 허가 집행의 burst 는 `[0, 500, 1500]` 이다.
    async fn keyframe_burst(&self, ssrcs: Vec<u32>, gaps_ms: &[u64]) {
        for (i, gap) in gaps_ms.iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(*gap)).await;
            }
            let now = now_ms();
            for ssrc in &ssrcs {
                self.request_keyframe(*ssrc, now, true).await;
            }
        }
    }

    // ───────── 회수(정§17-1) ─────────

    /// 정§17-1 — 좀비 회수가 먼저, 빈 방 sweep 은 그 뒤에(급사 참가자가 남으면 유예 시작이 늦는다).
    pub fn tick(self: &Arc<Self>) {
        let now = now_ms();
        self.reap(now);
        self.sweep_gates(now);
        self.floor_tick(now);
        self.sweep_stalls(now);
        for id in self.rooms.sweep(now) {
            self.destroy_room(&id);
        }
    }

    /// 정§14-3 — 전달 정체 감지. 구독자가 받아야 하는데 서버 송신이 멎은 자리를 당사자에게 알린다.
    ///
    /// ★"안 흐르는 게 정상" 인 창을 전부 건너뛴다. 그 목록이 전량이고, 빠뜨리면 오탐이 난다.
    /// ★못 보는 것 — 클라 하향이 죽은 경우는 서버 송신 계수가 계속 오르므로 여기 안 걸린다.
    ///   그 축은 UDP 관찰 → zombie → `media_lost` 다(연§2-6).
    pub fn sweep_stalls(&self, now_ms: u64) -> usize {
        let mut told = 0;
        for peer in self.peers.snapshot() {
            // ★enum 으로 견준다 — 정수 비교는 의미가 반전된다.
            if peer.state() != PeerState::Alive {
                continue;
            }
            for sub in peer.subscribe.all() {
                if !self.stalled_now(&sub, now_ms) {
                    continue;
                }
                if self.notify_stall(&peer.user_id, &sub.room_id, now_ms) {
                    warn!(user = %peer.user_id, room = %sub.room_id, track = %sub.track_id, "forwarding stalled");
                    told += 1;
                }
            }
        }
        told
    }

    /// 판정 하나 — 흐를 자리인데 안 흐르는가. 안 흐르는 게 정상이면 기준만 옮기고 거짓을 돌려준다.
    fn stalled_now(&self, sub: &Arc<SubscriberStream>, now_ms: u64) -> bool {
        // 게이트가 안 열렸으면 아직 흐를 자리가 아니다(정§7-4).
        if sub.state() != SubscribeState::Active || sub.paused() {
            sub.rebase_probe(now_ms);
            return false;
        }
        let Some(stream) = self.stream_of(&sub.room_id, &sub.track_id) else {
            // 발행 자체가 없다 — 슬롯 자리만 남은 것이다.
            sub.rebase_probe(now_ms);
            return false;
        };
        if stream.muted() {
            sub.rebase_probe(now_ms);
            return false;
        }
        // 반이중은 화자가 있을 때만 흐른다 — 조용한 무전은 정상이다.
        if stream.duplex() == Duplex::Half && !self.someone_holds(&sub.room_id) {
            sub.rebase_probe(now_ms);
            return false;
        }
        sub.stalled(now_ms, STALL_WINDOW_MS)
    }

    fn someone_holds(&self, room_id: &str) -> bool {
        self.rooms.get(room_id).is_some_and(|r| r.floor.speaker().is_some())
    }

    /// 정§14-3 — 같은 (user, 방) 재통보는 `T-stall` 쿨다운을 둔다. 폭풍 방지다.
    fn notify_stall(&self, user_id: &str, room_id: &str, now_ms: u64) -> bool {
        let key = format!("{user_id}\u{0}{room_id}");
        if self.stall_told.get(&key).is_some_and(|last| now_ms.saturating_sub(*last) < T_STALL_MS) {
            return false;
        }
        let Some(room) = self.rooms.get(room_id) else { return false };
        self.stall_told.insert(key, now_ms);
        let ev = RoomEvent {
            event_type: RoomEventType::SyncRequired,
            room_id: room.id.clone(),
            version: room.version(&self.epoch),
            affiliation: None,
            cause: None,
            reason: Some("no_media_flow".to_owned()),
        };
        self.bus.user(&room.id, user_id, Op::RoomEvent, &ev);
        true
    }

    /// 정§7-4 `T-gate` — READY 가 안 와도 스스로 푸는 안전망.
    /// ★이 경로엔 키프레임 요청이 없어 영구 검은 화면이 가능하다 — 그래서 원샷 관측을 반드시 남긴다.
    pub fn sweep_gates(&self, now_ms: u64) -> usize {
        let mut opened = 0;
        for peer in self.peers.snapshot() {
            for sub in peer.subscribe.all() {
                let waited = now_ms.saturating_sub(sub.created_at_ms.load(std::sync::atomic::Ordering::Relaxed));
                if sub.kind != MediaKind::Video || sub.state() != SubscribeState::Created || waited <= GATE_TIMEOUT_MS {
                    continue;
                }
                if sub.activate() {
                    warn!(user = %peer.user_id, room = %sub.room_id, track = %sub.track_id, waited, "gate opened by timeout, READY not received");
                    opened += 1;
                }
            }
        }
        opened
    }

    /// 정§2-2 PeerState — 전이 주체는 이 tick 단일. 반환: 회수한 Peer 수.
    pub fn reap(self: &Arc<Self>, now_ms: u64) -> usize {
        let mut reaped = 0;
        // §2-3 계약 4 — 순회와 삭제를 겹치지 않는다.
        for peer in self.peers.snapshot() {
            let Some(next) = judge(peer.state(), peer.last_seen(), now_ms) else { continue };
            let (changed, dwell) = peer.transition(next, now_ms);
            if !changed {
                continue;
            }
            match next {
                PeerState::Suspect => warn!(user = %peer.user_id, "peer suspect"),
                PeerState::Alive => info!(user = %peer.user_id, "peer media resumed"),
                PeerState::Zombie => {
                    self.reclaim(&peer, dwell);
                    reaped += 1;
                }
            }
        }
        reaped
    }

    /// 정§17-2 — 좀비 경로. ①~⑦ 은 `leave_one` 이 맡고, ⑧ `media_lost` unicast 가 이 경로에만 붙는다.
    fn reclaim(self: &Arc<Self>, peer: &Arc<Peer>, dwell_ms: u64) {
        info!(user = %peer.user_id, suspect_dwell_ms = dwell_ms, rooms = peer.room_count(), "zombie reclaimed");
        for id in peer.rooms() {
            let Some(room) = self.rooms.get(&id) else {
                warn!(user = %peer.user_id, room = %id, "index mismatch: peer room without room");
                continue;
            };
            let Some(version) = self.leave_one(peer, &room) else { continue };
            let ev = RoomEvent {
                event_type: RoomEventType::Affiliation,
                room_id: id.clone(),
                version,
                affiliation: Some(Affiliation { sub_rooms: Vec::new(), pub_room: None }),
                cause: Some(ForcedCause::MediaLost),
                reason: None,
            };
            self.bus.user(&id, &peer.user_id, Op::RoomEvent, &ev);
        }
        self.peers.remove(&peer.user_id);
        self.transport.unregister(&peer.user_id);
    }

    // ───────── 소속 ─────────

    fn affiliation(self: &Arc<Self>, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let req: AffiliationReq = parse(body)?;
        if req.pub_select.is_none() && req.pub_deselect.is_none() {
            return Err(Failure::new(FailCode::MissingField).message("pub_select or pub_deselect"));
        }
        let peer = self.peers.get(user_id).ok_or_else(|| Failure::new(FailCode::NotInRoom))?;
        let select = req.pub_select.as_deref().and_then(|id| self.rooms.get(id));
        // 정§5-2 — ★새 방의 무전 코덱 재검사. 등록 때만 보면 H264 방 트랙을 VP8 방으로 끌고 가는 구멍이 생긴다.
        if let Some(room) = select.as_ref() {
            self.check_slot_codec(&peer, room)?;
        }
        let changed = peer.apply_affiliation(req.pub_deselect.as_deref(), select.as_ref());
        // 정§5-2 — 발행 방에서 손을 떼면 그 방 발언권도 반환한다(`RELEASE` 와 같은 경로).
        for id in &changed {
            if peer.pub_room_id().as_deref() == Some(id.as_str()) {
                continue;
            }
            if let Some(room) = self.rooms.get(id) {
                self.floor_leave(&room, user_id);
            }
        }
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


    // ───────── 구독 배관(정§7-1·§7-2) ─────────

    /// 정§12 — 받기 egress 를 내보내는 연결. 1pc 는 그 하나가 겸한다.
    /// 정§10-3 v2 — egress 에 TWCC 를 찍을 자리. ★v2 가 아니면 안 찍는다(스탬핑은 v2 의 축이다).
    fn egress_twcc_id(&self, peer: &Peer) -> Option<u8> {
        self.media.auto_layer.is_v2().then(|| peer.subscribe.twcc_id()).flatten()
    }

    fn subscribe_transport(&self, peer: &Peer) -> Option<Arc<TransportSession>> {
        let role = match peer.pc_mode {
            PcMode::OnePc => ConnRole::Publish,
            PcMode::TwoPc => ConnRole::Subscribe,
        };
        self.transport.session_for(&peer.user_id, role)
    }

    /// 발행자가 쓴 확장 번호표 — 슬롯은 주인이 없으므로 서버 선언표다.
    fn publisher_extmap(&self, stream: &PublisherStream) -> Vec<(u8, &'static str)> {
        self.peers.get(&stream.owner).map_or_else(|| media::SERVER_EXTMAP.to_vec(), |p| p.publish.extmap())
    }

    /// 정§7-1 — 배관 하나. mid·PT 배정 → SubscriberStream → ★발행측 구독자 목록에 attach.
    /// `None` = PT 표 고갈(관측하고 건너뛴다) 또는 이미 배관이 있다.
    fn plumb(&self, peer: &Arc<Peer>, room_id: &str, stream: &Arc<PublisherStream>) -> Option<TrackEntry> {
        if peer.subscribe.get(room_id, &stream.track_id).is_some() {
            return None;
        }
        let want_rtx = stream.kind == MediaKind::Video;
        let Some((pt, rtx_pt)) = peer.subscribe.assign_pt(stream.codec, stream.fmtp.as_deref(), want_rtx) else {
            warn!(user = %peer.user_id, track = %stream.track_id, "subscriber pt table exhausted");
            return None;
        };
        let mid = peer.subscribe.alloc_mid(stream.kind);
        if mid.is_none() {
            warn!(user = %peer.user_id, track = %stream.track_id, "mid pool exhausted, entry goes without mid");
        }
        let transport = self.subscribe_transport(peer);
        let sub = peer.subscribe.insert(stream, SubSpec { subscriber: peer.user_id.clone(), room_id: room_id.to_owned(), mid, pt, transport, now_ms: now_ms() });
        sub.set_pt(pt, rtx_pt);
        sub.set_ext(peer.subscribe.ext_table(&self.publisher_extmap(stream)), self.egress_twcc_id(peer));
        for t in stream.tracks() {
            t.attach(&sub);
        }
        Some(entry_of(stream, &sub))
    }

    /// 정§7-1 ① — 그 구독자가 그 방에서 받아야 할 것 전량(슬롯 포함, 자기 개인 트랙 제외).
    fn room_streams_for(&self, room: &Room, subscriber: &str) -> Vec<Arc<PublisherStream>> {
        let mut out = room.slots.all();
        for member in room.member_ids() {
            if member == subscriber {
                continue;
            }
            let Some(p) = self.peers.get(&member) else { continue };
            out.extend(p.publish.in_room(&room.id).into_iter().filter(|s| !s.tracks().is_empty()));
        }
        out
    }

    /// 정§4-2 입장 조립 — 배관 전량. mid 가 모자라면 되돌리고 `4005`(조용한 부분 등록 금지).
    fn subscribe_room(&self, peer: &Arc<Peer>, room: &Arc<Room>) -> Result<Vec<TrackEntry>, Failure> {
        let streams = self.room_streams_for(room, &peer.user_id);
        let mut entries = Vec::new();
        for stream in &streams {
            if let Some(e) = self.plumb(peer, &room.id, stream) {
                entries.push(e);
            }
        }
        if entries.iter().any(|e| e.mid.is_none()) {
            for stream in &streams {
                self.unplumb(peer, &room.id, stream);
            }
            return Err(Failure::new(FailCode::MidLimit).details(json!({ "room_id": room.id })));
        }
        Ok(entries)
    }

    /// 정§7-2 회수 — 발행측 색인 해제가 먼저, mid 반환이 나중.
    fn unplumb(&self, peer: &Arc<Peer>, room_id: &str, stream: &Arc<PublisherStream>) {
        stream.detach_all(&peer.user_id, room_id);
        peer.subscribe.remove(&peer.user_id, room_id, &stream.track_id);
    }

    /// 정§14-2 — 방 단위. 항목은 구독자마다 다르므로(mid·PT) 메시지도 구독자마다다.
    /// ★방 `gate` 를 쥔 채로 부른다 — `seq` 증가와 enqueue 가 한 임계 구역이어야 한다(정§14-1).
    fn emit_track_event(&self, room: &Room, action: TrackAction, targets: Vec<(String, Vec<TrackEntry>)>) {
        let targets: Vec<_> = targets.into_iter().filter(|(_, t)| !t.is_empty()).collect();
        if targets.is_empty() {
            return;
        }
        let version = room.seq.bump(&self.epoch);
        for (user, tracks) in targets {
            let ev = TrackEvent { action, room_id: room.id.clone(), tracks, version: version.clone() };
            self.bus.user(&room.id, &user, Op::TrackEvent, &ev);
        }
    }

    /// 발행 스트림을 그 방 구독자 전원에게 배관하고 `add` 를 낸다. 슬롯은 화자도 받는다(N:1).
    fn announce(&self, room: &Room, streams: &[Arc<PublisherStream>]) {
        let mut targets: Vec<(String, Vec<TrackEntry>)> = Vec::new();
        for member in room.member_ids() {
            let Some(peer) = self.peers.get(&member) else { continue };
            let mut mine = Vec::new();
            for stream in streams {
                if stream.owner == member {
                    continue;
                }
                if let Some(e) = self.plumb(&peer, &room.id, stream) {
                    mine.push(e);
                }
            }
            targets.push((member, mine));
        }
        self.emit_track_event(room, TrackAction::Add, targets);
    }

    /// 발행 스트림 해제 — 구독자마다 `remove` + mid 회수, 그리고 ★고갈로 밀렸던 구독의 재발급(정§7-2).
    fn withdraw(&self, room: &Room, streams: &[Arc<PublisherStream>]) {
        let mut removed: Vec<(String, Vec<TrackEntry>)> = Vec::new();
        let mut refilled: Vec<(String, Vec<TrackEntry>)> = Vec::new();
        for member in room.member_ids() {
            let Some(peer) = self.peers.get(&member) else { continue };
            let mut mine = Vec::new();
            for stream in streams {
                let Some(sub) = peer.subscribe.get(&room.id, &stream.track_id) else { continue };
                mine.push(entry_of(stream, &sub));
                self.unplumb(&peer, &room.id, stream);
            }
            if mine.is_empty() {
                continue;
            }
            removed.push((member.clone(), mine));
            let back: Vec<TrackEntry> = peer
                .subscribe
                .refill_mids()
                .iter()
                .filter_map(|sub| self.stream_of(&sub.room_id, &sub.track_id).map(|s| entry_of(&s, sub)))
                .collect();
            refilled.push((member, back));
        }
        self.emit_track_event(room, TrackAction::Remove, removed);
        self.emit_track_event(room, TrackAction::Add, refilled);
    }

    /// 그 방에 실제로 있는 발행 스트림(슬롯 포함) — 보관본 조립이 쓴다.
    fn stream_of(&self, room_id: &str, track_id: &str) -> Option<Arc<PublisherStream>> {
        let room = self.rooms.get(room_id)?;
        room.slots
            .all()
            .into_iter()
            .find(|s| s.track_id == track_id)
            .or_else(|| room.member_ids().iter().filter_map(|m| self.peers.get(m)).find_map(|p| p.publish.get(track_id)))
    }


    /// 정§6-3 RID 학습 — simulcast 는 물리를 ★첫 RTP 까지 미룬다. 그 순간 배관·통지가 선다.
    /// ★rid 확장을 못 읽으면 붙이지 않는다 — 어느 단인지 모르는 물리는 전환을 못 한다.
    pub fn learn_simulcast(&self, peer: &Arc<Peer>, packet: &[u8], ssrc: u32) -> Option<(Arc<PublisherStream>, Arc<PublisherTrack>)> {
        let rid_id = peer.publish.extmap().iter().find(|(_, uri)| *uri == media::URI_RID).map(|(id, _)| *id)?;
        let rid = std::str::from_utf8(rtp::extension_value(packet, rid_id)?).ok()?.to_owned();
        if !subscribe::RID_BY_SPATIAL.contains(&rid.as_str()) {
            return None;
        }
        // 아직 물리가 없는 simulcast 논리 스트림 — 같은 kind 로 하나만 고른다.
        let stream = peer.publish.all().into_iter().find(|s| s.simulcast && s.kind == MediaKind::Video && s.track_of_rid(&rid).is_none())?;
        let track = peer.publish.learn(&stream, ssrc, Some(rid.clone()));
        let first = stream.tracks().len() == 1;
        info!(user = %peer.user_id, track = %stream.track_id, rid = %rid, ssrc, first, "simulcast layer learned");
        if first {
            stream.set_state(PublishState::Intended);
            if let Some(room) = self.rooms.get(&stream.room_id) {
                let _g = room.gate.lock().unwrap_or_else(|e| e.into_inner());
                self.announce(&room, std::slice::from_ref(&stream));
            }
        } else {
            // 둘째 단 — 이미 붙은 구독자들이 이 물리도 봐야 전환이 성립한다.
            self.attach_existing(&stream, &track);
        }
        Some((stream, track))
    }

    /// 논리 스트림에 이미 붙은 구독자를 새 물리에도 건다(정§7-1 ④ 와 같은 방향 역전).
    fn attach_existing(&self, stream: &Arc<PublisherStream>, track: &Arc<PublisherTrack>) {
        let Some(room) = self.rooms.get(&stream.room_id) else { return };
        for member in room.member_ids() {
            let Some(peer) = self.peers.get(&member) else { continue };
            if let Some(sub) = peer.subscribe.get(&room.id, &stream.track_id) {
                track.attach(&sub);
            }
        }
    }

    // ───────── 트랙 발행(정§6) ─────────

    fn publish_tracks(&self, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let req: PublishTracksReq = parse(body)?;
        let room = self.rooms.get(&req.room_id).ok_or_else(|| Failure::new(FailCode::RoomNotFound))?;
        let peer = self.peers.get(user_id).filter(|p| p.is_in(&room.id)).ok_or_else(|| Failure::new(FailCode::NotInRoom))?;
        req.validate().map_err(Failure::new)?;
        match req.action {
            PublishAction::Add => self.publish_add(&peer, &room, &req),
            PublishAction::Remove => self.publish_remove(&peer, &room, &req),
        }
    }

    /// 정§6-2 — 검사 전량이 먼저, 등록은 그 뒤. ★부분 수용 없음.
    fn publish_add(&self, peer: &Arc<Peer>, room: &Arc<Room>, req: &PublishTracksReq) -> Result<Value, Failure> {
        // ① 협상 확장 번호 수용 — 요청에 있는 것만 교체(원자값).
        if let Some(declared) = declared_extmap(req) {
            peer.publish.set_extmap(declared);
        }
        // ③ 그 user 활성 스트림 + 요청 > 16.
        let after = peer.publish.active() + req.tracks.len();
        if after > MAX_ACTIVE_STREAMS {
            return Err(Failure::new(FailCode::TrackLimit).details(json!({ "limit": MAX_ACTIVE_STREAMS, "current": peer.publish.active() })));
        }
        // ③-1 ~ ⑤ — 하나라도 걸리면 전체 거절. 슬롯 코덱은 이 요청 안에서 첫 half video 가 정한다.
        let mut pending_slot = room.slots.video_codec();
        let mut specs = Vec::with_capacity(req.tracks.len());
        for t in &req.tracks {
            if t.mid.is_empty() {
                return Err(Failure::new(FailCode::MissingField).message("mid"));
            }
            let Some(canonical) = codec::canonical(t.kind, t.codec.as_deref()) else {
                return Err(Failure::new(FailCode::CodecRequired).details(json!({ "supported": codec::supported_list() })));
            };
            let half = t.duplex == Some(Duplex::Half);
            if half && t.kind == MediaKind::Video {
                match &pending_slot {
                    Some((slot_codec, slot_fmtp)) if (*slot_codec, slot_fmtp.as_deref()) != (canonical, t.fmtp.as_deref()) => {
                        return Err(Failure::new(FailCode::CodecMismatch)
                            .details(json!({ "codec": slot_codec, "fmtp": slot_fmtp })));
                    }
                    Some(_) => {}
                    None => pending_slot = Some((canonical, t.fmtp.clone())),
                }
            }
            specs.push(spec_of(peer, room, t, canonical));
        }
        // ⑥ 등록 — 논리 선등록 후 물리는 `PublisherStream::create` 가 지킨다.
        let mut fresh = Vec::new();
        let mut published = Vec::new();
        for (spec, t) in specs.into_iter().zip(&req.tracks) {
            published.push(PublishedTrack { mid: t.mid.clone(), track_id: spec.track_id.clone() });
            fresh.push(peer.publish.insert(spec));
        }
        // 갈래별 후속 — 반이중 video 는 슬롯이 비어 있었으면 슬롯 항목이 생긴다. simulcast 는 첫 RTP 로 미룬다.
        let mut announce: Vec<Arc<PublisherStream>> = Vec::new();
        for s in &fresh {
            if s.duplex() == Duplex::Half {
                if s.kind == MediaKind::Video {
                    let (slot, created) = room.slots.ensure_video(&room.id, s.codec, s.fmtp.clone());
                    if created {
                        announce.push(slot);
                    }
                }
                continue;
            }
            if s.simulcast {
                info!(user = %peer.user_id, track = %s.track_id, "simulcast registered, physical waits for the first rtp");
                continue;
            }
            announce.push(s.clone());
        }
        {
            let _g = room.gate.lock().unwrap_or_else(|e| e.into_inner());
            self.announce(room, &announce);
        }
        info!(user = %peer.user_id, room = %room.id, added = fresh.len(), active = peer.publish.active(), "tracks published");
        let res = PublishTracksRes { intent: true, action: PublishAction::Add, tracks: Some(published) };
        Ok(serde_json::to_value(res).unwrap_or(Value::Null))
    }

    /// 정§6-2-1 — 남의 `track_id` 가 하나라도 있으면 전체 `3005`(add 와 같은 원자성).
    fn publish_remove(&self, peer: &Arc<Peer>, room: &Arc<Room>, req: &PublishTracksReq) -> Result<Value, Failure> {
        let mut targets = Vec::with_capacity(req.track_ids.len());
        for id in &req.track_ids {
            let Some(s) = peer.publish.get(id) else {
                return Err(Failure::new(FailCode::TrackNotFound).message(id.clone()));
            };
            targets.push(s);
        }
        for s in &targets {
            peer.publish.remove(&s.track_id);
        }
        let personal: Vec<Arc<PublisherStream>> = targets.iter().filter(|s| s.duplex() == Duplex::Full).cloned().collect();
        let retire = targets.iter().any(|s| s.duplex() == Duplex::Half && s.kind == MediaKind::Video);
        {
            let _g = room.gate.lock().unwrap_or_else(|e| e.into_inner());
            self.withdraw(room, &personal);
            // 정§17-2 ⑥ — 반이중 video 보유자가 전원 빠졌으면 슬롯 코덱을 리셋하고 항목을 지운다.
            if retire {
                self.retire_video_slot(room);
            }
        }
        info!(user = %peer.user_id, room = %room.id, removed = targets.len(), active = peer.publish.active(), "tracks removed");
        let res = PublishTracksRes { intent: true, action: PublishAction::Remove, tracks: None };
        Ok(serde_json::to_value(res).unwrap_or(Value::Null))
    }

    /// 정§5-2·§6-2 #5 — 그 사람의 반이중 video 가 새 방의 무전 코덱과 어긋나면 `1006` 으로 요청을 거절한다.
    /// ★조용한 건너뛰기가 아니다 — 클라가 코덱을 맞춰 재등록해야 풀리는 실패라 원인을 알려야 한다.
    fn check_slot_codec(&self, peer: &Peer, room: &Room) -> Result<(), Failure> {
        let Some((slot_codec, slot_fmtp)) = room.slots.video_codec() else { return Ok(()) };
        let mine = peer.publish.all();
        let mismatch = mine
            .iter()
            .filter(|s| s.duplex() == Duplex::Half && s.kind == MediaKind::Video)
            .any(|s| (s.codec, s.fmtp.as_deref()) != (slot_codec, slot_fmtp.as_deref()));
        if mismatch {
            return Err(Failure::new(FailCode::CodecMismatch).details(json!({ "codec": slot_codec, "fmtp": slot_fmtp })));
        }
        Ok(())
    }

    /// 그 방에 반이중 video 보유자가 남아 있으면 슬롯을 그대로 둔다(어기면 잔존 구독자 영상이 영구 미표시).
    fn retire_video_slot(&self, room: &Room) {
        let still = room.member_ids().iter().filter_map(|m| self.peers.get(m)).any(|p| {
            p.publish.in_room(&room.id).iter().any(|s| s.duplex() == Duplex::Half && s.kind == MediaKind::Video)
        });
        if still {
            return;
        }
        let Some(slot) = room.slots.reset_video() else { return };
        self.withdraw(room, &[slot]);
    }


    // ───────── duplex·muted 전환(정§8-2) ─────────

    /// 연§6-3 `TRACK_SET` — `muted` 와 `duplex` 는 배타(`1007`). 식별은 `track_id` 우선, `ssrc` 폴백.
    fn track_set(self: &Arc<Self>, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let req: TrackSetReq = parse(body)?;
        req.validate().map_err(Failure::new)?;
        let room = self.rooms.get(&req.room_id).ok_or_else(|| Failure::new(FailCode::RoomNotFound))?;
        let peer = self.peers.get(user_id).filter(|p| p.is_in(&room.id)).ok_or_else(|| Failure::new(FailCode::NotInRoom))?;
        let stream = req
            .track_id
            .as_deref()
            .and_then(|t| peer.publish.get(t))
            .or_else(|| req.ssrc.and_then(|s| peer.publish.by_ssrc(s)).map(|(st, _)| st))
            .ok_or_else(|| Failure::new(FailCode::TrackNotFound))?;
        match (req.muted, req.duplex) {
            (Some(muted), _) => Ok(self.set_muted(&room, &stream, muted)),
            (_, Some(duplex)) => self.set_duplex(&peer, &room, &stream, duplex),
            _ => Err(Failure::new(FailCode::FieldConflict)),
        }
    }

    /// 정§8-2 — ★논리 Stream 단위다(그 안 모든 물리에 적용 — sim 단 비대칭 방지).
    fn set_muted(&self, room: &Arc<Room>, stream: &Arc<PublisherStream>, muted: bool) -> Value {
        if !stream.set_muted(muted) {
            return json!({ "ssrc": stream.vssrc, "muted": muted, "noop": true });
        }
        self.emit_track_state(room, stream, TrackStateType::Muted, Some(muted), None, None);
        info!(user = %stream.owner, room = %room.id, track = %stream.track_id, muted, "track muted");
        json!({ "ssrc": stream.vssrc, "muted": muted })
    }

    /// 정§8-2 — ★duplex 원자 교체 하나로 fan-out 경로가 갈린다(새 분기를 만들지 않는다).
    /// ★개인 구독 배관(mid)은 지우지 않는다 — 복귀 대비 보존이고, 잔존은 `active:false` 로 실린다.
    fn set_duplex(self: &Arc<Self>, peer: &Arc<Peer>, room: &Arc<Room>, stream: &Arc<PublisherStream>, duplex: Duplex) -> Result<Value, Failure> {
        // 정§8-1 — `half` 는 simulcast 강제 off. 두 재기록기가 같은 vssrc 를 다툰다.
        if stream.simulcast {
            return Err(Failure::new(FailCode::TrackOpUnsupported).message("simulcast track cannot change duplex"));
        }
        if stream.duplex() == duplex {
            return Ok(json!({ "ssrc": stream.vssrc, "duplex": duplex, "noop": true }));
        }
        if duplex == Duplex::Half {
            // 정§6-2 #5 와 같은 검사 — 첫 화자면 이 값이 방의 무전 코덱이 된다.
            if stream.kind == MediaKind::Video {
                if let Some((slot_codec, slot_fmtp)) = room.slots.video_codec()
                    && (slot_codec, slot_fmtp.as_deref()) != (stream.codec, stream.fmtp.as_deref())
                {
                    return Err(Failure::new(FailCode::CodecMismatch).details(json!({ "codec": slot_codec, "fmtp": slot_fmtp })));
                }
                let (slot, created) = room.slots.ensure_video(&room.id, stream.codec, stream.fmtp.clone());
                if created {
                    let _g = room.gate.lock().unwrap_or_else(|e| e.into_inner());
                    self.announce(room, &[slot]);
                }
            }
            stream.set_duplex(Duplex::Half);
        } else {
            // 정§8-2 — ★그 트랙 발언권을 먼저 해제한다. 반이중이 아닌 트랙이 화자로 남으면 안 된다.
            if let Some(pub_room) = peer.pub_room() {
                self.floor_leave(&pub_room, &peer.user_id);
            }
            stream.set_duplex(Duplex::Full);
            if stream.kind == MediaKind::Video {
                let _g = room.gate.lock().unwrap_or_else(|e| e.into_inner());
                self.retire_video_slot(room);
            }
            // 복귀한 영상은 키프레임이 있어야 보인다.
            self.spawn_keyframe_burst(stream.tracks().iter().map(|t| t.ssrc).collect(), &CAMERA_PLI_GAPS);
        }
        let active = duplex == Duplex::Full;
        self.emit_track_state(room, stream, TrackStateType::Duplex, None, Some(duplex), Some(active));
        info!(user = %peer.user_id, room = %room.id, track = %stream.track_id, ?duplex, "track duplex");
        Ok(json!({ "ssrc": stream.vssrc, "duplex": duplex }))
    }

    /// 연§6-7 `TRACK_STATE` — ★배관을 가진 구독자에게만 간다(없는 트랙의 속성을 알릴 이유가 없다).
    /// `seq` 증가와 enqueue 는 한 임계 구역이다(정§14-1).
    fn emit_track_state(&self, room: &Arc<Room>, stream: &Arc<PublisherStream>, state_type: TrackStateType, muted: Option<bool>, duplex: Option<Duplex>, active: Option<bool>) {
        let targets: Vec<String> = room
            .member_ids()
            .into_iter()
            .filter(|m| self.peers.get(m).is_some_and(|p| p.subscribe.get(&room.id, &stream.track_id).is_some()))
            .collect();
        if targets.is_empty() {
            return;
        }
        let _g = room.gate.lock().unwrap_or_else(|e| e.into_inner());
        let version = room.seq.bump(&self.epoch);
        for user in targets {
            let ev = TrackState {
                state_type,
                user_id: stream.owner.clone(),
                track_id: stream.track_id.clone(),
                ssrc: stream.vssrc,
                kind: stream.kind,
                room_id: room.id.clone(),
                version: version.clone(),
                muted,
                duplex,
                active,
                source: stream.source.clone(),
            };
            self.bus.user(&room.id, &user, Op::TrackState, &ev);
        }
    }


    /// 연§6-3 `SUBSCRIBE_LAYER` — 요청은 "지정"이 아니라 ★**상한**이다. 생략한 필드는 안 바꾸고
    /// 범위 초과는 그 축 최대로 자른다(거절하지 않는다). ★대상별 실패는 조용히 건너뛰고 응답은 성공이다.
    fn subscribe_layer(self: &Arc<Self>, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let req: SubscribeLayerReq = parse(body)?;
        let room = self.rooms.get(&req.room_id).ok_or_else(|| Failure::new(FailCode::RoomNotFound))?;
        let peer = self.peers.get(user_id).filter(|p| p.is_in(&room.id)).ok_or_else(|| Failure::new(FailCode::NotInRoom))?;
        let mut resumed = Vec::new();
        for t in &req.targets {
            let Some(sub) = peer.subscribe.get(&room.id, &t.track_id) else { continue };
            if let Some(spatial) = t.spatial {
                sub.set_spatial_cap(spatial);
            }
            if let Some(priority) = t.priority {
                sub.set_priority(priority);
            }
            if let Some(paused) = t.paused
                && sub.set_paused(paused)
                && !paused
            {
                // 정§10-1 — 멈춰 있던 동안의 I-frame 이 없다. 풀 때 서버가 키프레임을 청한다.
                resumed.push(sub.vssrc);
            }
        }
        info!(user = user_id, room = %room.id, targets = req.targets.len(), "subscribe layer");
        self.spawn_keyframe_burst(resumed, &[0]);
        Ok(json!({}))
    }

    // ───────── READY(정§7-4) ─────────

    fn ready(self: &Arc<Self>, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let req: ReadyReq = parse(body)?;
        req.validate().map_err(Failure::new)?;
        let room = self.rooms.get(&req.room_id).ok_or_else(|| Failure::new(FailCode::RoomNotFound))?;
        let peer = self.peers.get(user_id).filter(|p| p.is_in(&room.id)).ok_or_else(|| Failure::new(FailCode::NotInRoom))?;
        match req.ready_type {
            ReadyType::Tracks => {
                let opened = peer.subscribe.in_room(&room.id).iter().filter(|s| s.activate()).count();
                // 정§7-4 — 키프레임은 ★그 방에 한 번이다(스트림마다 발동하면 과발동).
                let video: Vec<u32> = peer.subscribe.in_room(&room.id).iter().filter(|s| s.kind == MediaKind::Video).map(|s| s.vssrc).collect();
                self.spawn_keyframe_burst(video, &[0]);
                // 정§9-6 처음 알리기 — 그 방에 화자가 있으면 그 사람에게만 `TAKEN` unicast.
                for action in room.floor.announce_speaker(user_id) {
                    self.dispatch_floor(&action);
                }
                info!(user = user_id, room = %room.id, opened, "ready tracks");
            }
            ReadyType::Transport => self.ready_transport(&peer, &room, &req)?,
            ReadyType::Camera => self.ready_camera(&peer, &req)?,
        }
        Ok(json!({}))
    }

    /// 정§7-2-1 — 1pc 전용. 신고표로 확장·PT 를 교체하고, 어긋난 배정은 재배정해 `add` 를 ★응답보다 먼저 낸다.
    fn ready_transport(&self, peer: &Arc<Peer>, room: &Arc<Room>, req: &ReadyReq) -> Result<(), Failure> {
        if peer.pc_mode != PcMode::OnePc {
            return Err(Failure::new(FailCode::InvalidPayload).message("transport ready is 1pc only"));
        }
        let (Some(extmap), Some(codecs)) = (req.extmap.clone(), req.codecs.as_ref()) else {
            return Err(Failure::new(FailCode::MissingField).message("extmap, codecs"));
        };
        peer.subscribe.set_extmap(extmap);
        peer.subscribe.seed_pt(codecs);
        let mut changed = Vec::new();
        for sub in peer.subscribe.all() {
            let Some(stream) = self.stream_of(&sub.room_id, &sub.track_id) else { continue };
            sub.set_ext(peer.subscribe.ext_table(&self.publisher_extmap(&stream)), self.egress_twcc_id(peer));
            let want_rtx = stream.kind == MediaKind::Video;
            let Some((pt, rtx_pt)) = peer.subscribe.assign_pt(stream.codec, stream.fmtp.as_deref(), want_rtx) else { continue };
            if (pt, rtx_pt) != (sub.pt(), sub.rtx_pt()) {
                sub.set_pt(pt, rtx_pt);
                changed.push(entry_of(&stream, &sub));
            }
        }
        info!(user = %peer.user_id, room = %room.id, reassigned = changed.len(), "ready transport");
        let _g = room.gate.lock().unwrap_or_else(|e| e.into_inner());
        self.emit_track_event(room, TrackAction::Add, vec![(peer.user_id.clone(), changed)]);
        Ok(())
    }

    /// 연§6-3 — `track_id` 의 그 트랙이 흐르는 방(`pub_room`)에 `TRACK_STATE{live}`. 발행 방이 없으면 통지 없음.
    fn ready_camera(self: &Arc<Self>, peer: &Arc<Peer>, req: &ReadyReq) -> Result<(), Failure> {
        let track_id = req.track_id.as_deref().unwrap_or_default();
        let stream = peer.publish.get(track_id).ok_or_else(|| Failure::new(FailCode::TrackNotFound).message(track_id.to_owned()))?;
        stream.set_state(PublishState::Active);
        // 정§7-4 — 카메라가 프레임을 내기 시작했다. 그 트랙에 키프레임 두 번.
        self.spawn_keyframe_burst(stream.tracks().iter().map(|t| t.ssrc).collect(), &CAMERA_PLI_GAPS);
        let Some(pub_room) = peer.pub_room() else {
            info!(user = %peer.user_id, track = track_id, "ready camera without a publishing room");
            return Ok(());
        };
        let _g = pub_room.gate.lock().unwrap_or_else(|e| e.into_inner());
        let version = pub_room.seq.bump(&self.epoch);
        let ev = TrackState {
            state_type: TrackStateType::Live,
            user_id: peer.user_id.clone(),
            track_id: stream.track_id.clone(),
            ssrc: stream.vssrc,
            kind: stream.kind,
            room_id: pub_room.id.clone(),
            version,
            muted: None,
            duplex: None,
            active: Some(true),
            source: stream.source.clone(),
        };
        self.bus.room(&pub_room.id, Op::TrackState, &ev, std::slice::from_ref(&peer.user_id));
        Ok(())
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
            extmap: media::SERVER_EXTMAP.iter().map(|(id, uri)| Extmap { id: *id, uri: (*uri).to_owned() }).collect(),
            max_bitrate_bps: self.media.max_bitrate_bps,
        }
    }
}


/// 연§6-3 — 요청이 신고한 확장 번호만 서버 선언표 위에 얹는다.
fn declared_extmap(req: &PublishTracksReq) -> Option<Vec<(u8, &'static str)>> {
    let declared = [
        (req.mid_extmap_id, media::URI_MID),
        (req.audio_level_extmap_id, media::URI_AUDIO_LEVEL),
        (req.twcc_extmap_id, media::URI_TWCC),
        (req.rid_extmap_id, media::URI_RID),
        (req.repair_rid_extmap_id, media::URI_REPAIRED_RID),
    ];
    if declared.iter().all(|(id, _)| id.is_none()) {
        return None;
    }
    let mut table: Vec<(u8, &'static str)> = media::SERVER_EXTMAP
        .iter()
        .filter(|(_, uri)| !declared.iter().any(|(id, d)| id.is_some() && d == uri))
        .map(|(id, uri)| (*id, *uri))
        .collect();
    table.extend(declared.iter().filter_map(|(id, uri)| id.map(|i| (i, *uri))));
    Some(table)
}

/// 검사를 통과한 신고값만 이 자리에 온다 — 서버는 SDP 를 보지 않는다(정§6-1).
fn spec_of(peer: &Peer, room: &Room, t: &PublishTrack, canonical: &'static str) -> StreamSpec {
    // 정§8-1 — egress SSRC 는 전이중 non-sim 이면 ★원본이다. 가상값이 필요한 것은
    // 방 슬롯(반이중)과 단이 갈리는 simulcast 뿐이고, `vssrc` 는 그 "egress 값"의 이름이다.
    let simulcast = t.simulcast_effective();
    StreamSpec {
        track_id: format!("tr-{}", uuid::Uuid::new_v4().simple()),
        vssrc: if simulcast { new_vssrc() } else { t.ssrc },
        owner: peer.user_id.clone(),
        room_id: room.id.clone(),
        kind: t.kind,
        mid: t.mid.clone(),
        pt: t.pt,
        rtx_pt: t.rtx_pt,
        codec: canonical,
        fmtp: t.fmtp.clone(),
        source: t.source.clone(),
        duplex: t.duplex.unwrap_or(Duplex::Full),
        simulcast,
        ssrc: t.ssrc,
        rtx_ssrc: t.rtx_ssrc,
    }
}

fn video_fb() -> Vec<String> {
    ["nack", "nack pli", "ccm fir", "transport-cc"].iter().map(|s| (*s).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxsig::frame::Kind;

    fn sfu() -> Arc<Sfu> {
        sfu_with(BweMode::Twcc)
    }
    fn sfu_with(bwe_mode: BweMode) -> Arc<Sfu> {
        let cert = Arc::new(ServerCert::generate().unwrap());
        Arc::new(Sfu::new("sfu-test".into(), MediaParams { public_ip: "10.0.0.1".into(), udp_port: 20000, bwe_mode, auto_layer: AutoLayer::V1, fingerprint: cert.fingerprint.clone(), max_bitrate_bps: 800_000 }, cert))
    }
    fn env(user: &str, op: u16, body: Value) -> Envelope {
        Envelope { session_id: format!("s-{user}"), user_id: user.into(), room_id: String::new(), target: String::new(), exclude: Vec::new(), wire: encode_json(&Header::msg(op, 7), &body), pc_mode: "2pc".into(), floor_priority: 0 }
    }
    fn call(s: &Arc<Sfu>, user: &str, op: u16, body: Value) -> (Kind, Value) {
        let w = s.handle(&env(user, op, body));
        let (h, b) = frame::decode(&w).unwrap();
        assert_eq!(h.pid, 7);
        (h.kind, frame::body_json(b).unwrap())
    }
    fn create(s: &Arc<Sfu>, id: &str, cap: u64) {
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

        // 정§5-2 — 새 방의 무전 코덱과 어긋나는 반이중 video 를 끌고 갈 수 없다.
        let half_h264 = json!({"kind": "video", "ssrc": 7, "mid": "0", "pt": 96, "codec": "H264", "duplex": "half"});
        let half_vp8 = json!({"kind": "video", "ssrc": 8, "mid": "0", "pt": 96, "codec": "VP8", "duplex": "half"});
        call(&s, "u", Op::PublishTracks.code(), json!({"room_id": "b", "tracks": [half_h264]}));
        call(&s, "other", Op::RoomJoin.code(), json!({"room_id": "a"}));
        call(&s, "other", Op::PublishTracks.code(), json!({"room_id": "a", "tracks": [half_vp8]}));
        let (k, b) = call(&s, "u", Op::Affiliation.code(), json!({"pub_select": "a"}));
        assert_eq!((k, b["code"].as_u64(), b["details"]["codec"].as_str()), (Kind::Fail, Some(1006), Some("VP8")));
        assert_eq!(s.peers.get("u").unwrap().pub_room_id().as_deref(), Some("b"), "거절이면 아무것도 안 바뀐다");
        let (k, b) = call(&s, "u", 0x0601, json!({}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(1003)), "필수 필드가 없으면 1003 이다");
        let (k, b) = call(&s, "u", Op::PublishTracks.code(), json!({}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(1003)));
        let (k, b) = call(&s, "u", 0x0FFF, json!({}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(1001)));
    }


    fn audio(mid: &str, ssrc: u32) -> Value {
        json!({"kind": "audio", "ssrc": ssrc, "mid": mid, "pt": 111})
    }
    fn video(mid: &str, ssrc: u32, codec: &str) -> Value {
        json!({"kind": "video", "ssrc": ssrc, "mid": mid, "pt": 96, "codec": codec, "simulcast": false})
    }
    fn drain_raw(rx: &mut tokio::sync::broadcast::Receiver<Envelope>) -> Vec<Envelope> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }
    fn drain(rx: &mut tokio::sync::broadcast::Receiver<Envelope>) -> Vec<(u16, String, Value)> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            let (h, body) = frame::decode(&e.wire).unwrap();
            out.push((h.op, e.target.clone(), frame::body_json(body).unwrap()));
        }
        out
    }
    fn track_events(rx: &mut tokio::sync::broadcast::Receiver<Envelope>, action: &str) -> Vec<(String, Value)> {
        drain(rx)
            .into_iter()
            .filter(|(op, _, b)| *op == Op::TrackEvent.code() && b["action"] == action)
            .map(|(_, target, b)| (target, b))
            .collect()
    }

    #[test]
    fn publish_add_plumbs_every_other_member_and_leaves_the_room_consistent() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r", "select": false}));
        let mut rx = s.bus.subscribe();

        let (k, b) = call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [audio("0", 1111), video("1", 2222, "VP8")]}));
        assert_eq!((k, b["intent"].as_bool(), b["action"].as_str()), (Kind::Ok, Some(true), Some("add")));
        let published = b["tracks"].as_array().unwrap();
        assert_eq!(published.len(), 2);
        assert_eq!(published[0]["mid"].as_str(), Some("0"), "응답의 mid 는 발행자 자신의 신고값이다");
        let track_id = published[0]["track_id"].as_str().unwrap().to_owned();
        assert!(track_id.starts_with("tr-") && published[1]["track_id"] != published[0]["track_id"]);

        let events = track_events(&mut rx, "add");
        assert_eq!(events.len(), 1, "발행자 본인은 자기 트랙을 구독하지 않는다");
        let (target, ev) = &events[0];
        assert_eq!((target.as_str(), ev["room_id"].as_str(), ev["version"]["seq"].as_u64()), ("u2", Some("r"), Some(3)));
        let tracks = ev["tracks"].as_array().unwrap();
        assert_eq!(tracks.len(), 2);
        let a = tracks.iter().find(|t| t["kind"] == "audio").unwrap();
        let v = tracks.iter().find(|t| t["kind"] == "video").unwrap();
        assert_eq!((a["user_id"].as_str(), a["pt"].as_u64(), a["mid"].as_str()), (Some("u1"), Some(111), Some("1")));
        assert_eq!((v["pt"].as_u64(), v["rtx_pt"].as_u64(), v["codec"].as_str(), v["mid"].as_str()), (Some(96), Some(97), Some("VP8"), Some("2")));
        // 정§8-1 — 전이중 non-sim 의 egress SSRC 는 원본이다. 항목의 `ssrc` 가 곧 그것이라야
        // 클라가 지은 받기 m-line 의 `a=ssrc` 와 실제 도착 패킷이 맞는다.
        assert_eq!((a["ssrc"].as_u64(), v["ssrc"].as_u64()), (Some(1111), Some(2222)));

        // 정§14-4 — 요청자가 입장 중일 때만 mid 가 찬다.
        let (_, g) = call(&s, "u2", iop::ROOM_GET, json!({"room_id": "r", "tracks": true}));
        assert_eq!(g["tracks"].as_array().map(Vec::len), Some(3), "audio 슬롯 + 개인 둘");
        let (_, g0) = call(&s, "nobody", iop::ROOM_GET, json!({"room_id": "r", "tracks": true}));
        assert_eq!(g0["tracks"].as_array().map(Vec::len), Some(0), "남의 mid 는 없다");

        // 해제 — 구독자에게 remove, mid 회수.
        let (k, b) = call(&s, "u1", Op::PublishTracks.code(), json!({"action": "remove", "room_id": "r", "track_ids": [track_id]}));
        assert_eq!((k, b["action"].as_str(), b.get("tracks")), (Kind::Ok, Some("remove"), None));
        let removed = track_events(&mut rx, "remove");
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].1["tracks"][0]["mid"].as_str(), Some("1"), "지울 m-line 을 알려 준다");
        let u2 = s.peers.get("u2").unwrap();
        assert_eq!(u2.subscribe.alloc_mid(MediaKind::Audio), Some(1), "회수분이 같은 kind 풀로 돌아왔다");
    }

    #[test]
    fn publish_checks_run_before_any_registration() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        let peer = s.peers.get("u1").unwrap();

        let nine: Vec<Value> = (0..9).map(|i| audio(&i.to_string(), 100 + i)).collect();
        let (k, b) = call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": nine}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(4002)), "요청당 8");

        // ★부분 수용 없음 — 뒤엣것이 틀리면 앞엣것도 안 남는다.
        for (bad, code) in [
            (json!([audio("0", 1), video("1", 2, "VP9")]), 1005),
            (json!([audio("0", 1), json!({"kind": "video", "ssrc": 2, "mid": "1", "pt": 96, "simulcast": false})]), 1005),
            (json!([audio("0", 1), json!({"kind": "audio", "ssrc": 0, "mid": "1", "pt": 111})]), 1003),
            (json!([audio("0", 1), json!({"kind": "audio", "ssrc": 2, "mid": "", "pt": 111})]), 1003),
        ] {
            let (k, b) = call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": bad}));
            assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(code)), "{b}");
            assert_eq!(peer.publish.active(), 0, "앞엣것도 등록되지 않는다");
        }
        let (_, b) = call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [video("1", 2, "VP9")]}));
        assert_eq!(b["details"]["supported"], json!(["VP8", "H264"]), "지원 전량을 알린다");

        // ③ 활성 16 — 반복 호출 우회 봉쇄(8+8 뒤 1 추가 = 거절).
        for round in 0..2 {
            let eight: Vec<Value> = (0..8).map(|i| audio(&format!("{round}-{i}"), 1000 + round * 8 + i)).collect();
            assert_eq!(call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": eight})).0, Kind::Ok);
        }
        let (k, b) = call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [audio("x", 9999)]}));
        assert_eq!((k, b["code"].as_u64(), b["details"]["current"].as_u64()), (Kind::Fail, Some(4002), Some(16)));
        assert_eq!(peer.publish.active(), 16);

        let (k, b) = call(&s, "u1", Op::PublishTracks.code(), json!({"action": "remove", "room_id": "r", "track_ids": ["not-mine"]}));
        assert_eq!((k, b["code"].as_u64(), peer.publish.active()), (Kind::Fail, Some(3005), 16), "남의 것 하나면 전체 거절");
        let (k, b) = call(&s, "ghost", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [audio("0", 1)]}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(3002)));
    }

    /// 정§14-3 — "안 흐르는 게 정상" 인 창을 전부 건너뛴다. 이 목록이 전량이고 빠뜨리면 오탐이 난다.
    #[test]
    fn a_stall_is_only_reported_where_media_was_supposed_to_flow() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r"}));
        let full = json!({"kind": "audio", "ssrc": 7, "mid": "0", "pt": 111});
        assert_eq!(call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [full]})).0, Kind::Ok);
        call(&s, "u2", Op::Ready.code(), json!({"room_id": "r", "type": "tracks"}));

        let t0 = now_ms();
        // 첫 판은 기준만 놓는다 — 한 점으로는 흐르는지 멎었는지 알 수 없다.
        assert_eq!(s.sweep_stalls(t0), 0, "첫 관측은 판정이 아니다");
        assert_eq!(s.sweep_stalls(t0 + STALL_WINDOW_MS - 1), 0, "창이 안 찼다");

        let mut rx = s.bus.subscribe();
        assert_eq!(s.sweep_stalls(t0 + STALL_WINDOW_MS), 1, "흐를 자리인데 안 흐른다");
        let ev = drain(&mut rx).into_iter().find(|(op, _, _)| *op == Op::RoomEvent.code()).expect("당사자에게 간다");
        assert_eq!((ev.1.as_str(), ev.2["type"].as_str(), ev.2["reason"].as_str()), ("u2", Some("sync_required"), Some("no_media_flow")));

        // 쿨다운 — 같은 (user, 방) 재통보는 T-stall 안에서 막힌다.
        assert_eq!(s.sweep_stalls(t0 + STALL_WINDOW_MS * 2), 0, "폭풍 방지");
        assert_eq!(s.sweep_stalls(t0 + T_STALL_MS + STALL_WINDOW_MS), 1, "쿨다운이 지나면 다시 알린다");
    }

    /// 나머지 오탐 창 셋 — 게이트 전 · muted · Peer 비Alive. 하나만 빠져도 헛 재동기가 돈다.
    #[test]
    fn the_other_quiet_windows_are_not_stalls() {
        let full = |ssrc: u32| json!({"kind": "audio", "ssrc": ssrc, "mid": "0", "pt": 111});

        // ① 게이트가 안 열렸다 — READY 전이라 아직 흐를 자리가 아니다(정§7-4).
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r"}));
        assert_eq!(call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [full(7)]})).0, Kind::Ok);
        let t0 = now_ms();
        s.sweep_stalls(t0);
        assert_eq!(s.sweep_stalls(t0 + STALL_WINDOW_MS * 3), 0, "READY 전은 정체가 아니다");

        // ② muted — 안 보내는 것이 정상이다.
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r"}));
        let (_, body) = call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [full(8)]}));
        let track_id = body["tracks"][0]["track_id"].as_str().unwrap().to_owned();
        call(&s, "u2", Op::Ready.code(), json!({"room_id": "r", "type": "tracks"}));
        assert_eq!(call(&s, "u1", Op::TrackSet.code(), json!({"room_id": "r", "track_id": track_id, "muted": true})).0, Kind::Ok);
        let t0 = now_ms();
        s.sweep_stalls(t0);
        assert_eq!(s.sweep_stalls(t0 + STALL_WINDOW_MS * 3), 0, "muted 는 정체가 아니다");

        // ③ Peer 가 Alive 가 아니다 — 회수 축이 따로 맡는다(연§2-6).
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r"}));
        assert_eq!(call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [full(9)]})).0, Kind::Ok);
        call(&s, "u2", Op::Ready.code(), json!({"room_id": "r", "type": "tracks"}));
        let t0 = now_ms();
        s.sweep_stalls(t0);
        s.peers.get("u2").unwrap().transition(PeerState::Suspect, t0);
        assert_eq!(s.sweep_stalls(t0 + STALL_WINDOW_MS * 3), 0, "★enum 으로 견딘다 — 정수 비교는 의미가 반전된다");
    }

    /// 정§11-2 — 발행자 송신 추정은 한 축으로만 먹인다. 둘을 같이 보내면 어느 값을 따를지 갈린다.
    #[tokio::test]
    async fn only_one_bandwidth_axis_speaks() {
        assert_eq!(BweMode::parse("remb"), BweMode::Remb);
        assert_eq!(BweMode::parse("twcc"), BweMode::Twcc);
        assert_eq!(BweMode::parse("무엇이든"), BweMode::Twcc, "모르는 값은 기본 축이다");

        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        let full = json!({"kind": "audio", "ssrc": 7, "mid": "0", "pt": 111});
        assert_eq!(call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [full]})).0, Kind::Ok);

        // ★판정을 직접 본다 — 소켓 없이 송신 계수를 보면 어느 축이든 0 이라 판정이 가려진다.
        assert!(s.remb_targets().is_empty(), "★twcc 인데 REMB 를 같이 보내면 발행자가 갈린다");

        let r = sfu_with(BweMode::Remb);
        create(&r, "r", 5);
        call(&r, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        assert_eq!(call(&r, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [full]})).0, Kind::Ok);
        let targets = r.remb_targets();
        assert_eq!(targets.len(), 1, "remb 축이면 나간다");
        assert_eq!(&targets[0].1[12..16], b"REMB");
    }

    /// 연§6-5 · 정§13 — 문자는 방 broadcast 이고 신원은 세션이 준다.
    #[test]
    fn a_message_carries_the_session_identity_not_the_body() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r"}));
        let mut rx = s.bus.subscribe();

        // body 에 남의 이름을 실어 보낸다 — 서버는 그것을 믿지 않는다.
        let (k, res) = call(&s, "u1", Op::Message.code(), json!({"room_id": "r", "content": "여기 u1", "user_id": "u2"}));
        assert_eq!(k, Kind::Ok);
        assert!(res["msg_id"].as_str().is_some_and(|m| !m.is_empty()), "응답이 msg_id 를 준다");

        let sent: Vec<Envelope> = drain_raw(&mut rx).into_iter().filter(|e| {
            frame::decode(&e.wire).map(|(h, _)| h.op) == Ok(Op::Message.code())
        }).collect();
        assert_eq!(sent.len(), 1, "방 하나에 한 장이다");
        assert_eq!(sent[0].exclude, vec!["u1".to_owned()],
            "★보낸 사람에게는 에코하지 않는다 — 자기 것은 응답으로 안다");
        let body = frame::body_json(frame::decode(&sent[0].wire).unwrap().1).unwrap();
        assert_eq!(body["user_id"], "u1", "★신원은 세션이 준다 — body 의 것을 믿으면 아무나 사칭한다");
        assert_eq!(body["content"], "여기 u1");
    }

    #[test]
    fn a_message_to_a_room_i_am_not_in_is_refused() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        assert_eq!(call(&s, "zz", Op::Message.code(), json!({"room_id": "r", "content": "x"})).1["code"], 3002);
        assert_eq!(call(&s, "u1", Op::Message.code(), json!({"room_id": "none", "content": "x"})).1["code"], 3002);
    }

    /// 연§6-6 — 모르는 type 을 버리면 발제자가 타임아웃까지 기다린다.
    #[test]
    fn an_unknown_task_is_answered_not_dropped() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        let (k, b) = call(&s, "u1", Op::Task.code(), json!({"phase": "report", "req_id": 1, "type": "무엇이든", "result": {}}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(1002)));

        // 발제는 서버가 낸다 — 클라가 request 를 올리는 자리가 아니다.
        assert_eq!(call(&s, "u1", Op::Task.code(), json!({"phase": "request", "req_id": 2, "type": "probe"})).0, Kind::Fail);
        assert_eq!(call(&s, "u1", Op::Task.code(), json!({"phase": "report", "req_id": 2, "type": "probe", "result": {}})).0, Kind::Ok);
    }

    /// 정§10-3 — 정책이 꺼져 있으면 수동만 산다. 두 손이 같은 값을 다투면 앱이 정한 상한이 흔들린다.
    #[test]
    fn auto_layer_stays_out_of_the_way_when_the_policy_is_off() {
        let off = {
            let cert = Arc::new(ServerCert::generate().unwrap());
            Arc::new(Sfu::new("sfu-test".into(), MediaParams { public_ip: "10.0.0.1".into(), udp_port: 20000, bwe_mode: BweMode::Twcc, auto_layer: AutoLayer::Off, fingerprint: cert.fingerprint.clone(), max_bitrate_bps: 800_000 }, cert))
        };
        create(&off, "r", 5);
        call(&off, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        assert_eq!(off.auto_layer_tick(now_ms()), 0, "★꺼 두면 아무 값도 안 움직인다");
    }

    /// 연§6-1 · 정§3-3 — ★방 열 개 중 하나가 어긋났다고 열 개를 다시 만들지 않는다.
    #[test]
    fn resume_judges_room_by_room_not_all_or_nothing() {
        let s = sfu();
        create(&s, "a", 5);
        create(&s, "b", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "a"}));
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "b", "select": false}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "a"}));

        let (k, b) = call(&s, "u1", Op::Resume.code(), json!({"rooms": ["a", "b", "없는방"], "publish": []}));
        assert_eq!(k, Kind::Ok);
        assert_eq!(b["resumed"], json!(["a", "b"]), "★하나가 어긋나도 나머지는 이어받는다");
        assert_eq!(b["failed"], json!(["없는방"]));
        assert!(b["reason"]["없는방"].as_str().is_some(), "사유는 로그용이지만 비어 있으면 안 된다");

        // ★스냅샷이 끊겨 있는 동안 사라진 통지를 대신한다 — 없으면 그 사이 들어온 사람이 명단에 없다.
        let snap = &b["snapshot"]["a"];
        let members: Vec<&str> = snap["participants"].as_array().unwrap().iter().map(|m| m["user_id"].as_str().unwrap()).collect();
        assert_eq!(members, vec!["u1", "u2"]);
        assert!(snap["version"]["epoch"].as_str().is_some(), "스냅샷과 뒤 통지 사이에 틈이 없다는 근거다");
        assert!(b["snapshot"]["없는방"].is_null(), "이어받지 못한 방엔 스냅샷이 없다");
    }

    /// 연§6-1 — 트랙은 세션 것이라 방에 못박히지 않는다. ★신고가 곧 발행 상태다.
    #[test]
    fn resume_reconciles_publish_independently_of_rooms() {
        let s = sfu();
        create(&s, "a", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "a"}));
        let track = json!({"kind": "audio", "ssrc": 5, "mid": "0", "pt": 111});
        let (_, res) = call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "a", "tracks": [track]}));
        let live = res["tracks"][0]["track_id"].as_str().unwrap().to_owned();

        // 서버가 모르는 것을 신고했다 — publish_failed 다.
        let (_, b) = call(&s, "u1", Op::Resume.code(), json!({
            "rooms": ["a"],
            "publish": [{"track_id": live, "kind": "audio"}, {"track_id": "유령", "kind": "audio"}]
        }));
        assert_eq!(b["publish_failed"], json!(["유령"]));
        assert_eq!(s.peers.get("u1").unwrap().publish.active(), 1, "신고한 것은 그대로 산다");

        // ★신고에서 빠뜨렸다 — 서버가 지운다. 신고가 곧 발행 상태다.
        let (_, b) = call(&s, "u1", Op::Resume.code(), json!({"rooms": ["a"], "publish": []}));
        assert_eq!(b["publish_failed"], json!([]));
        assert_eq!(s.peers.get("u1").unwrap().publish.active(), 0, "★신고에서 빠진 트랙은 서버가 지운다");
    }

    #[test]
    fn a_quiet_walkie_talkie_is_not_a_stall() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r"}));
        let half = json!({"kind": "audio", "ssrc": 9, "mid": "0", "pt": 111, "duplex": "half"});
        assert_eq!(call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [half]})).0, Kind::Ok);
        call(&s, "u2", Op::Ready.code(), json!({"room_id": "r", "type": "tracks"}));

        let t0 = now_ms();
        s.sweep_stalls(t0);
        assert_eq!(
            s.sweep_stalls(t0 + STALL_WINDOW_MS * 3),
            0,
            "★아무도 말하지 않는 무전은 조용한 것이 정상이다 — 여기서 알리면 온 방이 재동기를 돈다"
        );
    }

    #[test]
    fn half_video_slot_is_decided_by_the_first_speaker_and_retired_with_the_last() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r"}));
        let room = s.rooms.get("r").unwrap();
        assert!(room.slots.video().is_none() && room.slots.audio.track_id == "ptt-r-audio");
        let mut rx = s.bus.subscribe();

        let half = |mid: &str, ssrc: u32, codec: &str, fmtp: Value| {
            json!({"kind": "video", "ssrc": ssrc, "mid": mid, "pt": 96, "codec": codec, "duplex": "half", "fmtp": fmtp})
        };
        let (k, _) = call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [half("0", 1, "H264", json!("profile-level-id=42e01f"))]}));
        assert_eq!(k, Kind::Ok);
        assert_eq!(room.slots.video_codec(), Some(("H264", Some("profile-level-id=42e01f".to_owned()))), "첫 화자가 방의 무전 코덱을 정한다");
        let added = track_events(&mut rx, "add");
        assert_eq!(added.len(), 2, "슬롯은 화자 본인도 받는다(N:1)");
        let slot_entry = &added.iter().find(|(t, _)| t == "u2").unwrap().1["tracks"][0];
        assert_eq!(
            (slot_entry["track_id"].as_str(), slot_entry["user_id"].is_null(), slot_entry["active"].is_null()),
            (Some("ptt-r-video"), true, true),
            "슬롯은 잔존이 아니라 배관이라 active 를 안 싣는다(연§4-1)"
        );

        let (k, b) = call(&s, "u2", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [half("0", 2, "VP8", Value::Null)]}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(1006)));
        assert_eq!((b["details"]["codec"].as_str(), b["details"]["fmtp"].as_str()), (Some("H264"), Some("profile-level-id=42e01f")), "거절 사유의 구체화 — 왕복 하나로 끝난다");
        let (k, _) = call(&s, "u2", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [half("0", 2, "H264", json!("profile-level-id=42e01f"))]}));
        assert_eq!(k, Kind::Ok);
        assert!(track_events(&mut rx, "add").is_empty(), "슬롯이 이미 있으면 후속이 없다");

        // 정§17-2 ⑥ — 같은 kind 의 half 보유자가 남아 있으면 슬롯을 지우지 않는다.
        let mine: Vec<String> = s.peers.get("u1").unwrap().publish.all().iter().map(|t| t.track_id.clone()).collect();
        call(&s, "u1", Op::PublishTracks.code(), json!({"action": "remove", "room_id": "r", "track_ids": mine}));
        assert!(room.slots.video().is_some(), "u2 가 남아 있다");
        assert!(track_events(&mut rx, "remove").is_empty());
        let theirs: Vec<String> = s.peers.get("u2").unwrap().publish.all().iter().map(|t| t.track_id.clone()).collect();
        call(&s, "u2", Op::PublishTracks.code(), json!({"action": "remove", "room_id": "r", "track_ids": theirs}));
        assert!(room.slots.video().is_none(), "전원 빠지면 다음 화자가 새로 정한다");
        assert_eq!(track_events(&mut rx, "remove").len(), 2, "슬롯 항목이 구독자 둘에게서 지워진다");
    }

    // 키프레임 요청은 응답을 안 붙잡고 태스크로 떨어지므로(정§7-4) 런타임 안에서 돈다.
    #[tokio::test]
    async fn ready_opens_the_gate_and_transport_report_is_one_pc_only() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        let one = s.handle(&Envelope { pc_mode: "1pc".into(), ..env("u2", Op::RoomJoin.code(), json!({"room_id": "r", "select": false})) });
        assert_eq!(frame::decode(&one).unwrap().0.kind, Kind::Ok);
        call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [video("1", 2222, "VP8")]}));
        let u2 = s.peers.get("u2").unwrap();
        let sub = u2.subscribe.in_room("r").into_iter().find(|x| x.kind == MediaKind::Video).unwrap();
        assert_eq!((sub.state(), sub.pt(), sub.rtx_pt()), (SubscribeState::Created, 96, Some(97)));

        let (k, _) = call(&s, "u2", Op::Ready.code(), json!({"room_id": "r", "type": "tracks"}));
        assert_eq!((k, sub.state()), (Kind::Ok, SubscribeState::Active));

        // 1pc 신고 — 씨앗이 먼저 들어가 어긋난 배정이 재배정되고 `add` 가 응답보다 먼저 나간다.
        let mut rx = s.bus.subscribe();
        let report = json!({"room_id": "r", "type": "transport",
            "extmap": [{"id": 3, "uri": media::URI_TWCC}],
            "codecs": [{"pt": 100, "name": "VP8", "rtx_pt": 101}]});
        let w = s.handle(&Envelope { pc_mode: "1pc".into(), ..env("u2", Op::Ready.code(), report.clone()) });
        assert_eq!(frame::decode(&w).unwrap().0.kind, Kind::Ok);
        assert_eq!((sub.pt(), sub.rtx_pt()), (100, Some(101)));
        let moved = track_events(&mut rx, "add");
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].1["tracks"][0]["pt"].as_u64(), Some(100));
        assert_eq!(sub.ext_of(6), Some(3), "발행자 twcc 번호가 구독자 번호로 바뀐다");
        assert_eq!(sub.ext_of(1), Some(14), "구독자 표에 없는 mid 는 표 밖 번호로");

        let (k, b) = call(&s, "u1", Op::Ready.code(), json!({"room_id": "r", "type": "transport", "extmap": [], "codecs": []}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(1002)), "2pc 에서 오면 거절");
        let (k, b) = call(&s, "u1", Op::Ready.code(), json!({"room_id": "r", "type": "camera"}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(1003)));
        let (k, b) = call(&s, "u1", Op::Ready.code(), json!({"room_id": "r", "type": "camera", "track_id": "nope"}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(3005)));

        let mine = s.peers.get("u1").unwrap().publish.all()[0].track_id.clone();
        let mut rx = s.bus.subscribe();
        let (k, _) = call(&s, "u1", Op::Ready.code(), json!({"room_id": "r", "type": "camera", "track_id": mine}));
        assert_eq!(k, Kind::Ok);
        let live: Vec<_> = drain(&mut rx).into_iter().filter(|(op, ..)| *op == Op::TrackState.code()).collect();
        assert_eq!(live.len(), 1);
        assert_eq!((live[0].2["type"].as_str(), live[0].2["active"].as_bool()), (Some("live"), Some(true)));
    }


    #[tokio::test]
    async fn duplex_and_mute_transitions_follow_the_atomic_axis() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r", "select": false}));
        call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [video("1", 0xB1, "VP8")]}));
        let room = s.rooms.get("r").unwrap();
        let sub = s.peers.get("u2").unwrap().subscribe.in_room("r").into_iter().find(|x| x.kind == MediaKind::Video).unwrap();
        let mid = sub.mid();
        let mut rx = s.bus.subscribe();

        // muted — 논리 Stream 단위. 같은 값이면 noop.
        let (k, b) = call(&s, "u1", Op::TrackSet.code(), json!({"room_id": "r", "ssrc": 0xB1, "muted": true}));
        assert_eq!((k, b["muted"].as_bool(), b["ssrc"].as_u64(), b.get("noop")), (Kind::Ok, Some(true), Some(0xB1), None));
        let states = drain(&mut rx).into_iter().filter(|(op, ..)| *op == Op::TrackState.code()).collect::<Vec<_>>();
        assert_eq!(states.len(), 1, "배관을 가진 구독자에게만");
        assert_eq!((states[0].1.as_str(), states[0].2["type"].as_str(), states[0].2["muted"].as_bool()), ("u2", Some("muted"), Some(true)));
        let (_, again) = call(&s, "u1", Op::TrackSet.code(), json!({"room_id": "r", "ssrc": 0xB1, "muted": true}));
        assert_eq!(again["noop"].as_bool(), Some(true));

        // full→half — 슬롯이 생기고, 개인 배관은 ★보존된다(복귀 대비).
        let (k, b) = call(&s, "u1", Op::TrackSet.code(), json!({"room_id": "r", "track_id": s.peers.get("u1").unwrap().publish.all().iter().find(|t| t.kind == MediaKind::Video).unwrap().track_id.clone(), "duplex": "half"}));
        assert_eq!((k, b["duplex"].as_str()), (Kind::Ok, Some("half")));
        assert!(room.slots.video().is_some(), "첫 화자가 방의 무전 코덱을 정한다");
        assert_eq!(s.peers.get("u2").unwrap().subscribe.get("r", &sub.track_id).map(|x| x.mid()), Some(mid), "개인 mid 를 지우지 않는다");
        let half = drain(&mut rx).into_iter().filter(|(op, ..)| *op == Op::TrackState.code()).collect::<Vec<_>>();
        let duplex_ev: Vec<_> = half.iter().filter(|(_, _, b)| b["type"] == "duplex").collect();
        assert_eq!(duplex_ev.len(), 1);
        assert_eq!((duplex_ev[0].1.as_str(), duplex_ev[0].2["active"].as_bool()), ("u2", Some(false)), "잔존은 active:false");

        // half→full — 슬롯이 걷히고 active:true 가 간다.
        let (k, b) = call(&s, "u1", Op::TrackSet.code(), json!({"room_id": "r", "ssrc": 0xB1, "duplex": "full"}));
        assert_eq!((k, b["duplex"].as_str()), (Kind::Ok, Some("full")));
        assert!(room.slots.video().is_none(), "마지막 반이중 video 보유자가 빠지면 슬롯을 리셋한다");
        let back: Vec<_> = drain(&mut rx).into_iter().filter(|(op, _, b)| *op == Op::TrackState.code() && b["type"] == "duplex").collect();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].2["active"].as_bool(), Some(true));

        // 판정 셋 — 배타·미지 트랙·simulcast.
        let (k, b) = call(&s, "u1", Op::TrackSet.code(), json!({"room_id": "r", "ssrc": 0xB1, "muted": true, "duplex": "half"}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(1007)));
        let (k, b) = call(&s, "u1", Op::TrackSet.code(), json!({"room_id": "r", "ssrc": 999, "duplex": "half"}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(3005)));
        call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [{"kind": "video", "ssrc": 0, "mid": "2", "pt": 96, "codec": "VP8", "simulcast": true}]}));
        let sim = s.peers.get("u1").unwrap().publish.all().iter().find(|t| t.simulcast).unwrap().track_id.clone();
        let (k, b) = call(&s, "u1", Op::TrackSet.code(), json!({"room_id": "r", "track_id": sim, "duplex": "half"}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(3006)), "★half 는 simulcast 강제 off");
    }


    /// rid 확장을 실은 RTP 하나 — 서버 선언 번호 10(연§4-2)을 쓴다.
    fn rid_packet(ssrc: u32, rid: &str, keyframe: bool) -> Vec<u8> {
        let mut p = vec![0x90, 0x60, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0];
        rtp::set_ssrc(&mut p, ssrc);
        let mut ext = vec![(10u8 << 4) | (rid.len() as u8 - 1)];
        ext.extend_from_slice(rid.as_bytes());
        while !ext.len().is_multiple_of(4) {
            ext.push(0);
        }
        p.extend_from_slice(&[0xBE, 0xDE]);
        p.extend_from_slice(&((ext.len() / 4) as u16).to_be_bytes());
        p.extend_from_slice(&ext);
        // VP8 descriptor(S=1·PID=0) + payload header 의 P 비트.
        p.extend_from_slice(&[0x10, if keyframe { 0x00 } else { 0x01 }, 0, 0]);
        p
    }

    #[test]
    fn simulcast_layers_are_learned_from_the_first_rtp_and_announced_once() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r", "select": false}));
        let (k, _) = call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [
            {"kind": "video", "ssrc": 0, "mid": "1", "pt": 96, "codec": "VP8", "simulcast": true}]}));
        assert_eq!(k, Kind::Ok);
        let peer = s.peers.get("u1").unwrap();
        let stream = peer.publish.all().into_iter().find(|t| t.simulcast).unwrap();
        assert!(stream.tracks().is_empty(), "물리는 첫 RTP 까지 미룬다");
        let mut rx = s.bus.subscribe();

        // 첫 단 — 배관과 통지가 이때 선다.
        assert!(s.learn_simulcast(&peer, &rid_packet(0xC1, "h", true), 0xC1).is_some());
        let added = track_events(&mut rx, "add");
        assert_eq!(added.len(), 1, "발행자 본인은 빼고 한 번만");
        let entry = &added[0].1["tracks"][0];
        assert_eq!((entry["simulcast"].as_bool(), entry["scalability"].as_str()), (Some(true), Some("L2T1")));
        assert_eq!(entry["ssrc"].as_u64(), Some(u64::from(stream.vssrc)), "★egress 는 vssrc 하나로 합쳐진다");

        // 둘째 단 — 통지는 다시 안 나가고, 이미 붙은 구독자가 새 물리에도 걸린다.
        assert!(s.learn_simulcast(&peer, &rid_packet(0xC2, "l", true), 0xC2).is_some());
        assert!(track_events(&mut rx, "add").is_empty(), "단이 늘어도 항목은 하나다");
        assert_eq!(stream.tracks().len(), 2);
        for t in stream.tracks() {
            assert_eq!(t.subscriber_count(), 1, "rid={:?} 도 구독자를 봐야 전환이 성립한다", t.rid);
        }
        // 같은 단이 다시 와도 물리를 두 번 붙이지 않는다.
        assert!(s.learn_simulcast(&peer, &rid_packet(0xC3, "h", true), 0xC3).is_none());
        assert!(s.learn_simulcast(&peer, &rid_packet(0xC4, "m", true), 0xC4).is_none(), "모르는 rid 는 안 붙인다");
        assert_eq!(stream.tracks().len(), 2);
    }

    // `paused:false` 는 키프레임을 청하므로(정§10-1) 런타임 안에서 돈다.
    #[tokio::test]
    async fn layer_request_is_a_cap_and_pause_is_a_separate_axis() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r", "select": false}));
        call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [
            {"kind": "video", "ssrc": 0, "mid": "1", "pt": 96, "codec": "VP8", "simulcast": true}]}));
        let peer = s.peers.get("u1").unwrap();
        s.learn_simulcast(&peer, &rid_packet(0xC1, "h", true), 0xC1);
        let stream = peer.publish.all().into_iter().find(|t| t.simulcast).unwrap();
        let sub = s.peers.get("u2").unwrap().subscribe.get("r", &stream.track_id).unwrap();
        assert_eq!((sub.spatial_cap(), sub.wanted_rid(), sub.paused(), sub.priority()), (1, "h", false, 128));

        let ask = |spatial: Value, paused: Value| {
            json!({"room_id": "r", "targets": [{"track_id": stream.track_id, "spatial": spatial, "paused": paused, "priority": 200}]})
        };
        let (k, _) = call(&s, "u2", Op::SubscribeLayer.code(), ask(json!(0), Value::Null));
        assert_eq!((k, sub.spatial_cap(), sub.wanted_rid()), (Kind::Ok, 0, "l"));
        assert_eq!(sub.priority(), 200);
        // ★상한이라 범위를 넘으면 자른다 — 거절하지 않는다.
        call(&s, "u2", Op::SubscribeLayer.code(), ask(json!(9), Value::Null));
        assert_eq!((sub.spatial_cap(), sub.wanted_rid()), (1, "h"));
        // 생략한 필드는 안 바꾼다 · 대상별 실패는 조용히 건너뛰고 응답은 성공이다.
        let (k, _) = call(&s, "u2", Op::SubscribeLayer.code(), json!({"room_id": "r", "targets": [{"track_id": "nope", "spatial": 0}]}));
        assert_eq!((k, sub.spatial_cap()), (Kind::Ok, 1));

        // `paused` 는 별개 축이다.
        call(&s, "u2", Op::SubscribeLayer.code(), ask(json!(1), json!(true)));
        assert!(sub.paused() && sub.spatial_cap() == 1);
        call(&s, "u2", Op::SubscribeLayer.code(), ask(json!(1), json!(false)));
        assert!(!sub.paused());

        // 정§10-2 — 목표는 세워지되 전환은 키프레임에서만 확정된다.
        assert!(sub.current_rid().is_none());
        assert!(sub.aim("h", 1_000) && !sub.aim("h", 1_500), "pending 안에서는 다시 안 세운다");
        sub.switch_to("h");
        assert_eq!(sub.current_rid().as_deref(), Some("h"));
    }

    #[test]
    fn gate_timeout_opens_video_only_and_is_observed() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r", "select": false}));
        call(&s, "u1", Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [audio("0", 11), video("1", 22, "VP8")]}));
        let u2 = s.peers.get("u2").unwrap();
        let subs = u2.subscribe.in_room("r");
        assert_eq!(subs.len(), 3, "audio 슬롯 + 개인 둘");
        assert_eq!(s.sweep_gates(now_ms()), 0, "아직 창 안");
        assert_eq!(s.sweep_gates(now_ms() + GATE_TIMEOUT_MS + 1), 1, "video 하나만 — audio 는 gate 로 죽지 않는다");
        assert_eq!(s.sweep_gates(now_ms() + GATE_TIMEOUT_MS + 2), 0, "원샷");
    }


    fn half_audio(mid: &str, ssrc: u32) -> Value {
        json!({"kind": "audio", "ssrc": ssrc, "mid": mid, "pt": 111, "duplex": "half"})
    }
    fn mbcp_frame(msg: &Msg) -> Vec<u8> {
        oxsig::dc::encode(oxsig::dc::SVC_MBCP, &msg.encode().unwrap()).unwrap()
    }
    /// DC 는 실제 세션이 있어야 나가므로, 시험은 상태기가 낸 액션을 직접 본다.
    fn floor_in(s: &Arc<Sfu>, user: &str, msg: Msg) -> Vec<(MsgType, Option<String>, Option<u8>)> {
        s.floor_message(user, &msg)
            .iter()
            .map(|a| {
                let m = a.msg();
                let key = match m.msg_type {
                    MsgType::Deny | MsgType::Revoke => m.get_u8(mbcp::field::CAUSE),
                    MsgType::QueueInfo => m.get(mbcp::field::QUEUE_INFO).and_then(|v| v.first().copied()),
                    _ => None,
                };
                (m.msg_type, a.target().map(str::to_owned), key)
            })
            .collect()
    }
    fn request(room: &str) -> Msg {
        Msg::new(MsgType::Request).u8(mbcp::field::PRIORITY, 200).room(room)
    }

    #[test]
    fn floor_entry_gates_are_the_contract_not_silence() {
        let s = sfu();
        create(&s, "r", 5);
        create(&s, "other", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r"}));

        // 정§9-6 ① — 방이 없거나 모르는 방이면 계수 후 무응답(돌려줄 방이 없다).
        assert!(floor_in(&s, "u1", Msg::new(MsgType::Request)).is_empty(), "0x1D 없음");
        assert!(floor_in(&s, "u1", request("__nope__")).is_empty(), "모르는 방");
        // 미입장은 우리 쪽 오류라 사유 255 + 설명으로 돌려준다.
        let out = s.floor_message("u1", &request("other"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].msg().get_u8(mbcp::field::CAUSE), Some(mbcp::reject::OTHER));
        assert_eq!(out[0].msg().get_str(mbcp::field::CAUSE_TEXT), Some("not_in_room"));

        // 정§9-6 ② — 반이중 발행 트랙이 없으면 무시가 아니라 사유 5 다.
        assert_eq!(
            floor_in(&s, "u1", request("r")),
            vec![(MsgType::Deny, Some("u1".to_owned()), Some(mbcp::reject::RECEIVE_ONLY))]
        );
    }

    // 정§9-7 — 허가가 데우기 태스크를 띄우므로 런타임 안에서 돈다.
    #[tokio::test]
    async fn floor_grant_gate_and_succession_run_through_the_wire() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r"}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r"}));
        for u in ["u1", "u2"] {
            let (k, _) = call(&s, u, Op::PublishTracks.code(), json!({"room_id": "r", "tracks": [half_audio("0", if u == "u1" { 11 } else { 22 })]}));
            assert_eq!(k, Kind::Ok);
        }
        let room = s.rooms.get("r").unwrap();

        // 정§9-5 — 권위는 토큰 클레임이다. 요청 TLV 0 이 200 이어도 상한이 자른다.
        s.peers.get("u1").unwrap().set_floor_room(None);
        let granted = floor_in(&s, "u1", request("r"));
        assert_eq!(granted, vec![(MsgType::Granted, Some("u1".to_owned()), None), (MsgType::Taken, None, None)]);
        assert!(room.floor.is_speaker("u1"));
        assert!(s.peers.get("u1").unwrap().holds_floor_elsewhere("zzz"), "cross-room 축이 Peer 에 선다");
        assert!(!s.peers.get("u1").unwrap().holds_floor_elsewhere("r"));

        // 두 번째 요청은 큐로. 우선순위가 같으면 선점이 아니다.
        assert_eq!(
            floor_in(&s, "u2", request("r")),
            vec![(MsgType::QueueInfo, Some("u2".to_owned()), Some(1))]
        );

        // RELEASE → 큐가 있으므로 IDLE 없이 곧바로 승계.
        let after = floor_in(&s, "u1", Msg::new(MsgType::Release).room("r"));
        assert_eq!(after, vec![(MsgType::Granted, Some("u2".to_owned()), None), (MsgType::Taken, None, None)]);
        assert!(room.floor.is_speaker("u2") && !s.peers.get("u1").unwrap().holds_floor_elsewhere("zzz"));

        // 정§17-2 ① — 퇴장이 발언권을 먼저 정리한다.
        call(&s, "u2", Op::RoomLeave.code(), json!({"room_id": "r"}));
        assert_eq!(room.floor.state(), crate::media::floor::FloorState::Idle);
        assert!(room.floor.speaker().is_none());
    }

    #[test]
    fn frames_round_trip_through_the_datachannel_codec() {
        let msg = Msg::new(MsgType::Granted).ack(true).u8(mbcp::field::PRIORITY, 7).room("r1");
        let frame = mbcp_frame(&msg);
        let (svc, payload) = oxsig::dc::decode(&frame).unwrap();
        assert_eq!(svc, oxsig::dc::SVC_MBCP);
        assert_eq!(Msg::decode(payload), Some(msg), "DC 4B 헤더 안에서 그대로 산다");
    }

    #[test]
    fn transport_registered_per_mode_and_released_with_peer() {
        let s = sfu();
        create(&s, "r", 5);
        call(&s, "u2pc", Op::RoomJoin.code(), json!({"room_id": "r"}));
        assert_eq!(s.transport.len(), 2, "2pc 는 자격 두 벌이 다 산다");
        let w = s.handle(&Envelope { pc_mode: "1pc".into(), ..env("u1pc", Op::RoomJoin.code(), json!({"room_id": "r", "select": false})) });
        assert_eq!(frame::decode(&w).unwrap().0.kind, Kind::Ok);
        assert_eq!(s.transport.len(), 3, "1pc 는 publish 하나");
        let pub_ufrag = s.peers.get("u1pc").unwrap().publish_ice.ufrag.clone();
        assert_eq!(s.transport.by_ufrag(&pub_ufrag).unwrap().user_id, "u1pc");
        let w = s.handle(&Envelope { pc_mode: "3pc".into(), ..env("ux", Op::RoomJoin.code(), json!({"room_id": "r"})) });
        let (h, b) = frame::decode(&w).unwrap();
        assert_eq!((h.kind, frame::body_json(b).unwrap()["code"].as_u64()), (Kind::Fail, Some(1002)), "미지 pc_mode 는 갈음하지 않는다");
        call(&s, "u1pc", Op::RoomLeave.code(), json!({"room_id": "r"}));
        assert_eq!(s.transport.len(), 2, "마지막 방을 나가면 전송도 회수");
        assert!(s.transport.by_ufrag(&pub_ufrag).is_none());
    }

    #[test]
    fn zombie_reclaim_is_the_full_teardown_plus_media_lost() {
        let s = sfu();
        create(&s, "r1", 5);
        create(&s, "r2", 5);
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r1"}));
        call(&s, "u1", Op::RoomJoin.code(), json!({"room_id": "r2", "select": false}));
        call(&s, "u2", Op::RoomJoin.code(), json!({"room_id": "r1", "select": false}));
        assert_eq!((s.transport.len(), s.peers.len()), (4, 2));
        assert_eq!(s.reap(10_000_000), 0, "UDP 미관찰이면 판정 자체를 건너뛴다");

        let peer = s.peers.get("u1").unwrap();
        s.observe_media("u1");
        peer.touch(1_000);
        assert_eq!(s.reap(1_000 + crate::peer::SUSPECT_AFTER_MS + 1), 0);
        assert_eq!(peer.state(), PeerState::Suspect);
        let mut rx = s.bus.subscribe();
        assert_eq!(s.reap(1_000 + crate::peer::ZOMBIE_AFTER_MS + 1), 1);

        let mut seen = Vec::new();
        while let Ok(e) = rx.try_recv() {
            let (h, body) = frame::decode(&e.wire).unwrap();
            seen.push((h.op, e.room_id.clone(), e.target.clone(), frame::body_json(body).unwrap()));
        }
        let lefts: Vec<_> = seen.iter().filter(|(op, ..)| *op == Op::ParticipantEvent.code()).collect();
        let losts: Vec<_> = seen.iter().filter(|(op, ..)| *op == Op::RoomEvent.code()).collect();
        assert_eq!((lefts.len(), losts.len()), (2, 2), "방마다 ⑦ left broadcast + ⑧ media_lost unicast");
        for (_, room_id, target, body) in &losts {
            assert_eq!(target, "u1", "⑧ 은 당사자에게만");
            let ev: RoomEvent = serde_json::from_value(body.clone()).unwrap();
            assert_eq!((ev.event_type, ev.cause, ev.room_id.as_str()), (RoomEventType::Affiliation, Some(ForcedCause::MediaLost), room_id.as_str()));
            assert_eq!(ev.affiliation, Some(Affiliation { sub_rooms: Vec::new(), pub_room: None }));
            assert!(!ev.still_member());
        }
        assert!(s.peers.get("u1").is_none() && s.transport.len() == 2, "Peer 와 그 전송만 회수");
        assert!(!s.rooms.get("r1").unwrap().is_member("u1") && s.rooms.get("r2").unwrap().user_count() == 0);
        assert!(s.rooms.get("r1").unwrap().is_member("u2"), "남의 방 자리는 건드리지 않는다");
        assert_eq!(s.reap(1_000 + crate::peer::ZOMBIE_AFTER_MS + 2), 0, "회수는 한 번뿐");
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
        assert_eq!((g["participants"][0]["select"].as_bool(), g["version"]["seq"].as_u64(), g["tracks"].as_array().map(Vec::len)), (Some(false), Some(1), Some(1)));
        // 정§8-1 — audio 슬롯은 방과 수명이 같아 입장 즉시 배관이 있다(주인 없음 = N:1).
        let slot = &g["tracks"][0];
        assert_eq!((slot["track_id"].as_str(), slot["kind"].as_str(), slot["mid"].as_str()), (Some("ptt-r-audio"), Some("audio"), Some("0")));
        assert_eq!((slot["duplex"].as_str(), slot["pt"].as_u64(), slot["user_id"].is_null()), (Some("half"), Some(111), true));
        let (_, g0) = call(&s, "", iop::ROOM_GET, json!({"room_id": "r"}));
        assert!(g0.get("tracks").is_none());
        assert!(!s.destroy_room("r"), "occupied → not destroyed");
    }
}
