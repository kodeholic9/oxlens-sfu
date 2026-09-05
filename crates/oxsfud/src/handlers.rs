// author: kodeholic (powered by Claude)
//! op 처리 — 정§4-2 입장 판정 ②~⑥ · §17-2 퇴장(②④⑦⑧) · §5-2 소속 · HTTP 대행(연§5-3~5-5) · §17-1 회수 주체 둘.
//! 응답 경로는 전부 동기·메모리 안이고 요청 하나에 응답 하나다(정§19 ②). 전송 층은 이 상태를 읽고 쓴다.

use std::sync::Arc;

use common::bplane::{Envelope, iop};
use oxsig::body::affiliation::{AffiliationReq, AffiliationRes, Cause};
use oxsig::body::media::{PublishAction, PublishTrack, PublishTracksReq, PublishTracksRes, PublishedTrack, ReadyReq, ReadyType};
use oxsig::body::notify::{ForcedCause, ParticipantEvent, ParticipantEventType, RoomEvent, RoomEventType, TrackAction, TrackEvent, TrackState, TrackStateType};
use oxsig::body::room::{PARTICIPANT_RECORDER, RoomJoinReq, RoomJoinRes, RoomLeaveReq, RoomLeaveRes};
use oxsig::frame::{self, Header, Kind, encode_json};
use oxsig::op::Op;
use oxsig::schema::{Affiliation, CodecSpec, DtlsConfig, Duplex, Extmap, IceConfig, MediaKind, PcMode, ServerConfig, TrackEntry, Version};
use oxsig::{FailCode, Failure};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::emit::EventBus;
use crate::media::slot::new_vssrc;
use crate::media::subscribe::{SubSpec, SubscribeState, entry_of};
use crate::media::track::{PublishState, PublisherStream, StreamSpec};
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
    pub transport: TransportRegistry,
    pub cert: Arc<ServerCert>,
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
    pub fn new(epoch: String, media: MediaParams, cert: Arc<ServerCert>) -> Self {
        Self { epoch, rooms: RoomRegistry::default(), peers: PeerMap::default(), bus: EventBus::default(), media, transport: TransportRegistry::default(), cert }
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
                Some(Op::PublishTracks) => self.publish_tracks(&env.user_id, &body),
                Some(Op::Ready) => self.ready(&env.user_id, &body),
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
        // 정§12 — `pc_mode` 는 세션 확정값이다. 모르는 값을 조용히 갈음하지 않는다.
        let pc_mode: PcMode = serde_json::from_value(Value::String(env.pc_mode.clone()))
            .map_err(|_| Failure::new(FailCode::InvalidPayload).message(format!("pc_mode {}", env.pc_mode)))?;
        let peer = match peer {
            Some(p) => p,
            None => {
                let p = Arc::new(Peer::new(user_id, req.participant_type, pc_mode, now_ms()));
                self.peers.insert(p.clone());
                self.transport.register(user_id, pc_mode, &p.publish_ice, &p.subscribe_ice);
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
            tracks,
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
        self.transport.unregister(&peer.user_id);
    }

    // ───────── 전송 층이 부르는 자리 ─────────

    /// 정§2-2 — `last_seen` 의 갱신원은 UDP 관찰 둘뿐이다. WS 하트비트는 여기 오지 않는다.
    pub fn observe_media(&self, user_id: &str) {
        if let Some(peer) = self.peers.get(user_id) {
            peer.touch(now_ms());
        }
    }

    /// 정§13 — `svc` 별 입구. 발언권(0x01)의 상태기는 정§9 의 몫이라 이 판은 관찰만 한다.
    pub fn on_dc_frame(&self, session: &Arc<TransportSession>, svc: u8, payload: &[u8]) {
        self.observe_media(&session.user_id);
        let name = match svc {
            oxsig::dc::SVC_MBCP => "mbcp",
            oxsig::dc::SVC_VOICE_ACTIVITY => "voice_activity",
            oxsig::dc::SVC_APP_MIN.. => "app",
            _ => "unknown",
        };
        info!(user = %session.user_id, svc = name, len = payload.len(), "dc frame in");
    }

    // ───────── 회수(정§17-1) ─────────

    /// 정§17-1 — 좀비 회수가 먼저, 빈 방 sweep 은 그 뒤에(급사 참가자가 남으면 유예 시작이 늦는다).
    pub fn tick(&self) {
        let now = now_ms();
        self.reap(now);
        self.sweep_gates(now);
        for id in self.rooms.sweep(now) {
            self.destroy_room(&id);
        }
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
    pub fn reap(&self, now_ms: u64) -> usize {
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
    fn reclaim(&self, peer: &Arc<Peer>, dwell_ms: u64) {
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


    // ───────── 구독 배관(정§7-1·§7-2) ─────────

    /// 정§12 — 받기 egress 를 내보내는 연결. 1pc 는 그 하나가 겸한다.
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
        sub.set_ext(peer.subscribe.ext_table(&self.publisher_extmap(stream)));
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

    // ───────── READY(정§7-4) ─────────

    fn ready(&self, user_id: &str, body: &Value) -> Result<Value, Failure> {
        let req: ReadyReq = parse(body)?;
        req.validate().map_err(Failure::new)?;
        let room = self.rooms.get(&req.room_id).ok_or_else(|| Failure::new(FailCode::RoomNotFound))?;
        let peer = self.peers.get(user_id).filter(|p| p.is_in(&room.id)).ok_or_else(|| Failure::new(FailCode::NotInRoom))?;
        match req.ready_type {
            ReadyType::Tracks => {
                let opened = peer.subscribe.in_room(&room.id).iter().filter(|s| s.activate()).count();
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
            sub.set_ext(peer.subscribe.ext_table(&self.publisher_extmap(&stream)));
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
    fn ready_camera(&self, peer: &Arc<Peer>, req: &ReadyReq) -> Result<(), Failure> {
        let track_id = req.track_id.as_deref().unwrap_or_default();
        let stream = peer.publish.get(track_id).ok_or_else(|| Failure::new(FailCode::TrackNotFound).message(track_id.to_owned()))?;
        stream.set_state(PublishState::Active);
        let Some(pub_room) = peer.pub_room().and_then(|id| self.rooms.get(&id)) else {
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

    fn sfu() -> Sfu {
        let cert = Arc::new(ServerCert::generate().unwrap());
        Sfu::new("sfu-test".into(), MediaParams { public_ip: "10.0.0.1".into(), udp_port: 20000, fingerprint: cert.fingerprint.clone(), max_bitrate_bps: 800_000 }, cert)
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
        let (k, b) = call(&s, "u", Op::TrackSet.code(), json!({}));
        assert_eq!((k, b["code"].as_u64()), (Kind::Fail, Some(5003)), "아직 안 옮긴 op 는 시험이 잡는다");
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
        assert_eq!((slot_entry["track_id"].as_str(), slot_entry["user_id"].is_null(), slot_entry["active"].as_bool()), (Some("ptt-r-video"), true, Some(false)));

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

    #[test]
    fn ready_opens_the_gate_and_transport_report_is_one_pc_only() {
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
