// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§4-2 · §6-2 · §7-2 · §14-1 · 연§6-2 · §6-4 · model: claude-opus-5

//! 클라 프레임 처리 — ★**여기가 방·명단·`seq` 의 권위다**(정§14-1·§15-4).
//!
//! ★**hub 는 재해석하지 않는다** — 프레임을 그대로 넘기고 응답 프레임을 그대로 돌려준다.
//! 그래서 이 모듈은 ★**소켓도 시계도 모른다**: 판정만 하고 나갈 것을 낸다(hub 의 `ws` 와 같은 규율).

use oxsig::body::data::{AffiliationCause, AffiliationReq, AffiliationRes};
use oxsig::body::notify::{ParticipantChange, ParticipantEvent};
use oxsig::body::room::{RoomJoinReq, RoomJoinRes, RoomLeaveReq, RoomLeaveRes, ServerConfig};
use oxsig::body::session::PcMode;
use oxsig::frame::{self, Header, Kind as FrameKind};
use oxsig::types::{Assign, Kind, StreamType, TrackEntry};
use oxsig::{Code, Failure, Op};

use crate::identity::{self, Dtls};
use crate::peer::Peers;
use crate::room::{Member, Rooms};

/// hub 가 봉투에 실어 준 신원. ★**body 를 믿지 않는다** — `user_id` 는 hub 세션이 주입한다.
#[derive(Debug, Clone)]
pub struct Ingress {
    pub session_id: String,
    pub user_id: String,
    pub participant_type: u8,
    pub hidden: bool,
    pub metadata: Option<serde_json::Value>,
    pub pc_mode: PcMode,
}

/// 방 전원에게 갈 통지 하나. ★**`exclude` 는 받는 hub 가 적용한다**(정§15-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub room_id: String,
    pub exclude: Vec<String>,
    pub wire: Vec<u8>,
}

/// 한 프레임을 처리한 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// 그 요청의 응답 프레임. ★**언제나 하나 있다** — 조용한 성공이 없다.
    pub reply: Vec<u8>,
    pub notices: Vec<Notice>,
}

/// 이 유닛이 쥔 것 전부.
#[derive(Debug)]
pub struct Node {
    /// `epoch` = `sfu_id` = §15-1 키의 `{inst}` — ★**한 값이다**(정§14-1).
    pub epoch: String,
    pub dtls: Dtls,
    pub ip: String,
    pub port: u16,
    pub max_bitrate_bps: u64,
    pub rooms: Rooms,
    pub peers: Peers,
}

impl Node {
    pub fn new(epoch: String, dtls: Dtls, ip: String, port: u16, max_bitrate_bps: u64) -> Self {
        Self { epoch, dtls, ip, port, max_bitrate_bps, rooms: Rooms::new(), peers: Peers::new() }
    }

    /// 무전 audio 슬롯 하나 — ★**방과 수명이 같아 입장 응답에 이미 배관돼 있다**(정§6-2).
    ///
    /// ★`opus` 는 한 튜플이라 그 연결에서 PT 가 늘 `111` 이다(정§7-2-1 씨앗).
    fn audio_slot(&self, room_id: &str, ssrc: u32, mid: String) -> TrackEntry {
        TrackEntry {
            stream_type: StreamType::Slot,
            room_id: room_id.to_string(),
            track_id: format!("ptt-{room_id}-audio"),
            kind: Kind::Audio,
            ssrc,
            rtx_ssrc: None,
            codec: Some("opus".into()),
            fmtp: None,
            // ★슬롯에는 주인이 없다 — 화자가 바뀌어도 항목이 그대로다.
            user_id: None,
            source: None,
            active: None,
            muted: None,
            scalability: None,
            assign: Some(Assign { mid, pt: 111, rtx_pt: None }),
        }
    }

    fn server_config(&self, i: usize) -> ServerConfig {
        let p = self.peers.at(i);
        ServerConfig {
            sfu_id: self.epoch.clone(),
            // ★세션 확정값을 ★**에코**한다 — 조용히 다른 모드로 돌리는 경로가 없다.
            pc_mode: p.pc_mode,
            ice: p.ice.config(&self.ip, self.port),
            dtls: self.dtls.config(),
            codecs: identity::codecs(),
            codecs_sub: None,
            extmap: identity::extmap(),
            max_bitrate_bps: self.max_bitrate_bps,
        }
    }
}

fn ok(header: Header, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame::HEADER_LEN + body.len());
    frame::encode(&mut out, Header { kind: FrameKind::Ok, ..header }, body);
    out
}

fn fail(header: Header, code: Code) -> Vec<u8> {
    let body = serde_json::to_vec(&Failure::new(code)).unwrap_or_default();
    let mut out = Vec::with_capacity(frame::HEADER_LEN + body.len());
    frame::encode(&mut out, Header { kind: FrameKind::Fail, ..header }, &body);
    out
}

fn json(v: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_default()
}

/// 방 전원에게 갈 통지 프레임. ★**`pid` 는 hub 가 다시 매긴다**(흐름 창이 hub 것이다).
fn notice(room_id: &str, exclude: Vec<String>, op: Op, body: &impl serde::Serialize) -> Notice {
    let b = json(body);
    let mut wire = Vec::with_capacity(frame::HEADER_LEN + b.len());
    frame::encode(&mut wire, Header::new(FrameKind::Request, op, 0), &b);
    Notice { room_id: room_id.to_string(), exclude, wire }
}

/// ★**한 프레임 = 한 응답**(+ 통지 몇). 조용한 성공이 없다.
pub fn dispatch(node: &mut Node, ing: &Ingress, header: Header, body: &[u8]) -> Outcome {
    match header.op {
        Op::RoomJoin => room_join(node, ing, header, body),
        Op::RoomLeave => room_leave(node, ing, header, body),
        Op::Affiliation => affiliation(node, ing, header, body),
        // ★미디어 축은 다음 걸음이다 — 조용히 성공하지 않는다.
        _ => Outcome { reply: fail(header, Code::UnknownOp), notices: Vec::new() },
    }
}

fn room_join(node: &mut Node, ing: &Ingress, header: Header, body: &[u8]) -> Outcome {
    let Ok(req) = serde_json::from_slice::<RoomJoinReq>(body) else {
        return Outcome { reply: fail(header, Code::InvalidPayload), notices: Vec::new() };
    };
    if node.rooms.get(&req.room_id).is_none() {
        return Outcome { reply: fail(header, Code::RoomNotFound), notices: Vec::new() };
    }
    // ★**Peer 가 단위다** — 같은 신원의 옛 Peer 는 그 서버의 **모든 방**에서 함께 걷힌다(정§4-2 ②).
    let (i, orphaned) = node.peers.ensure(&ing.session_id, &ing.user_id, ing.pc_mode);
    let mut notices = Vec::new();
    for r in orphaned {
        if let Some(room) = node.rooms.get_mut(&r)
            && room.leave(&ing.user_id)
        {
            let v = room.version(&node.epoch);
            notices.push(left_notice(&r, &ing.user_id, v));
        }
    }

    let room = node.rooms.get_mut(&req.room_id).expect("위에서 봤다");
    let select = req.select.unwrap_or(true);
    let role = req.role.unwrap_or(255);
    if let Err(code) = room.join(Member {
        session_id: ing.session_id.clone(),
        user_id: ing.user_id.clone(),
        hidden: ing.hidden,
        participant_type: ing.participant_type,
        role,
        select,
        metadata: ing.metadata.clone(),
    }) {
        return Outcome { reply: fail(header, code), notices };
    }
    let version = room.version(&node.epoch);
    let participants = room.participants();
    let slot_ssrc = room.slot_audio_ssrc;

    let track_id = format!("ptt-{}-audio", req.room_id);
    let peer = node.peers.at_mut(i);
    if !peer.sub_rooms.contains(&req.room_id) {
        peer.sub_rooms.push(req.room_id.clone());
    }
    // ★`select` 는 *"이 방에서 말한다"* 이고 `pub_room` 은 하나뿐이라 ★**마지막 것이 이긴다.**
    if select {
        peer.pub_room = Some(req.room_id.clone());
    }
    let mid = match peer.assigns.get(&track_id) {
        Some(a) => a.mid.clone(),
        None => {
            let m = peer.mids.take(Kind::Audio);
            peer.assigns.insert(track_id.clone(), Assign { mid: m.clone(), pt: 111, rtx_pt: None });
            m
        }
    };
    let affiliation = peer.affiliation();

    let res = RoomJoinRes {
        room_id: req.room_id.clone(),
        participants,
        affiliation,
        server_config: node.server_config(i),
        tracks: vec![node.audio_slot(&req.room_id, slot_ssrc, mid)],
        version: version.clone(),
    };
    // ★**당사자에게는 응답이 그 스냅샷을 대신한다** — `exclude` 는 이 한 자리뿐이다(정§14-2).
    if !ing.hidden {
        notices.push(notice(
            &req.room_id,
            vec![ing.user_id.clone()],
            Op::ParticipantEvent,
            &ParticipantEvent {
                change: ParticipantChange::Joined,
                room_id: req.room_id.clone(),
                user_id: ing.user_id.clone(),
                role: Some(role),
                select: Some(select),
                participant_type: Some(ing.participant_type),
                metadata: ing.metadata.clone(),
                version,
            },
        ));
    }
    Outcome { reply: ok(header, &json(&res)), notices }
}

fn left_notice(room_id: &str, user_id: &str, version: oxsig::Version) -> Notice {
    notice(
        room_id,
        vec![user_id.to_string()],
        Op::ParticipantEvent,
        &ParticipantEvent {
            change: ParticipantChange::Left,
            room_id: room_id.to_string(),
            user_id: user_id.to_string(),
            // ★퇴장에는 프로필이 없다 — 누가 나갔나만 있으면 된다.
            role: None,
            select: None,
            participant_type: None,
            metadata: None,
            version,
        },
    )
}

fn room_leave(node: &mut Node, ing: &Ingress, header: Header, body: &[u8]) -> Outcome {
    let Ok(req) = serde_json::from_slice::<RoomLeaveReq>(body) else {
        return Outcome { reply: fail(header, Code::InvalidPayload), notices: Vec::new() };
    };
    let Some(room) = node.rooms.get_mut(&req.room_id) else {
        return Outcome { reply: fail(header, Code::RoomNotFound), notices: Vec::new() };
    };
    // ★**안 들어간 방에서 나가는 것은 `3002`** 다 — 없는 방(`3001`)과 다른 축이다.
    if !room.leave(&ing.user_id) {
        return Outcome { reply: fail(header, Code::NotInRoom), notices: Vec::new() };
    }
    let version = room.version(&node.epoch);
    let mut notices = Vec::new();
    if !ing.hidden {
        notices.push(left_notice(&req.room_id, &ing.user_id, version));
    }

    let Some(peer) = node.peers.get_mut(&ing.session_id) else {
        return Outcome { reply: fail(header, Code::SessionNotFound), notices };
    };
    peer.sub_rooms.retain(|r| r != &req.room_id);
    // ★**딸려 내려간다** — 발행하던 방에서 나가면 발행할 곳이 없다(연§6-2).
    if peer.pub_room.as_deref() == Some(req.room_id.as_str()) {
        peer.pub_room = None;
    }
    if let Some(a) = peer.assigns.remove(&format!("ptt-{}-audio", req.room_id)) {
        // ★자리를 되돌린다 — 안 돌리면 m-line 이 새 값으로만 자라 천장에 먼저 닿는다.
        peer.mids.give(Kind::Audio, &a.mid);
    }
    let res = RoomLeaveRes { room_id: req.room_id.clone(), affiliation: peer.affiliation() };
    Outcome { reply: ok(header, &json(&res)), notices }
}

fn affiliation(node: &mut Node, ing: &Ingress, header: Header, body: &[u8]) -> Outcome {
    let req: AffiliationReq = if body.is_empty() {
        AffiliationReq::default()
    } else {
        match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => return Outcome { reply: fail(header, Code::InvalidPayload), notices: Vec::new() },
        }
    };
    let Some(peer) = node.peers.get_mut(&ing.session_id) else {
        return Outcome { reply: fail(header, Code::SessionNotFound), notices: Vec::new() };
    };
    // ★**적용 순서는 `pub_deselect` → `pub_select`** 다 — 같은 요청에 둘 다 와도(연§6-4).
    if let Some(r) = &req.pub_deselect
        && peer.pub_room.as_deref() == Some(r.as_str())
    {
        peer.pub_room = None;
    }
    if let Some(r) = &req.pub_select {
        // ★**입장한 방에서만 말한다** — 안 들어간 방은 `3002`(없는 방 `3001` 과 다른 축).
        if !peer.sub_rooms.contains(r) {
            return Outcome { reply: fail(header, Code::NotInRoom), notices: Vec::new() };
        }
        peer.pub_room = Some(r.clone());
    }
    let res = AffiliationRes {
        affiliation: peer.affiliation(),
        // ★**응답의 `cause` 는 `user` 하나다** — 강제 변경은 `ROOM_EVENT` 가 나른다.
        cause: AffiliationCause::User,
        change_id: req.change_id.clone(),
    };
    // ★`version` 을 싣지 않는다 — 소속은 내 세션 것이라 방 공통 스냅샷에 없다(연§4-6-3).
    Outcome { reply: ok(header, &json(&res)), notices: Vec::new() }
}
