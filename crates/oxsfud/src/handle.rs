// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§4-2 · §6-2 · §7-2 · §14-1 · 연§6-2 · §6-4 · model: claude-opus-5

//! 클라 프레임 처리 — ★**여기가 방·명단·`seq` 의 권위다**(정§14-1·§15-4).
//!
//! ★**hub 는 재해석하지 않는다** — 프레임을 그대로 넘기고 응답 프레임을 그대로 돌려준다.
//! 그래서 이 모듈은 ★**소켓도 시계도 모른다**: 판정만 하고 나갈 것을 낸다(hub 의 `ws` 와 같은 규율).

use oxsig::body::data::{AffiliationCause, AffiliationReq, AffiliationRes};
use oxsig::body::media::{
    PublishAction, PublishTracksReq, PublishTracksRes, PublishedTrack, ReadyReq, ReadyType,
    TrackSetReq,
};
use oxsig::body::notify::{
    ParticipantChange, ParticipantEvent, RoomEvent, RoomEventType, TrackAction, TrackEvent,
};
use oxsig::body::room::{RoomJoinReq, RoomJoinRes, RoomLeaveReq, RoomLeaveRes, ServerConfig};
use oxsig::body::session::PcMode;
use oxsig::frame::{self, Header, Kind as FrameKind};
use oxsig::body::session::Duplex;
use oxsig::types::{Assign, Kind, Source, StreamType, TrackEntry, Version};
use std::sync::Arc;

use oxsig::{Code, Failure, Op};

use crate::floor::{self, Floor};
use crate::identity::{self, Dtls};
use crate::peer::Peers;
use crate::room::{Member, Rooms};
use crate::transport::{IceRole, IceTable};

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

/// 나갈 통지 하나. ★**`exclude` 는 받는 hub 가 적용한다**(정§15-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub room_id: String,
    pub exclude: Vec<String>,
    /// ★있으면 **그 사람에게만** — 없으면 그 방 전원(정§15-4 가름).
    pub target: Option<String>,
    pub wire: Vec<u8>,
}

/// 한 프레임을 처리한 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    /// 그 요청의 응답 프레임. ★**언제나 하나 있다** — 조용한 성공이 없다.
    pub reply: Vec<u8>,
    pub notices: Vec<Notice>,
    /// ★**전달표 갱신** — 제어 평면이 계산해 데이터 평면에 밀어 넣는다(핫패스 규율 H2).
    pub routes: Vec<(u32, Vec<crate::transport::udp::Target>)>,
    /// 시뮬캐스트 등록 `(발행 자격, vssrc)`.
    pub sims: Vec<(String, u32)>,
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
    /// ★**등록 층** — 스냅샷 밖이라 방이 아니라 node 가 쥔다(연§4-1-1).
    pub publications: Vec<Publication>,
    /// 방마다 발언권 하나 — ★**권위는 DC 단일이다**(연§11).
    pub floors: std::collections::BTreeMap<String, Floor>,
    /// `[floor] t2_stop_talking_secs` — 한 번에 말할 수 있는 상한.
    pub max_burst_ms: u64,
    /// ★**전송의 문패** — ufrag 로 찾는다. 읽는 쪽(UDP 루프)은 자물쇠를 안 잡는다.
    pub ice: Arc<IceTable>,
    /// 세션마다의 전송 생존 판정(정§2-2). ★**Peer 와 따로 둔다** — 한 값으로 합치면
    /// 미디어 흐름 단계와 섞여 비교 반전이 감지를 통째로 죽인다(20260816 실사고).
    health: std::collections::BTreeMap<String, crate::reaper::Health>,
}

impl Node {
    pub fn new(epoch: String, dtls: Dtls, ip: String, port: u16, max_bitrate_bps: u64) -> Self {
        Self {
            epoch,
            dtls,
            ip,
            port,
            max_bitrate_bps,
            rooms: Rooms::new(),
            peers: Peers::new(),
            publications: Vec::new(),
            floors: std::collections::BTreeMap::new(),
            max_burst_ms: floor::timers::T2_MS,
            ice: Arc::new(IceTable::new()),
            health: std::collections::BTreeMap::new(),
        }
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
    Notice { room_id: room_id.to_string(), exclude, target: None, wire }
}

/// 그 사람에게만 가는 통지. ★**생존을 판정하지 않는다**(정§17-2 ⑧) — 그 사람의 키에
/// 놓으면 그만이고, 붙어 있으면 닿는다. 안 닿으면 재접속의 `RESUME` 스냅샷이 답한다.
fn unicast(room_id: &str, user_id: &str, op: Op, body: &impl serde::Serialize) -> Notice {
    let mut n = notice(room_id, Vec::new(), op, body);
    n.target = Some(user_id.to_string());
    n
}

/// 그 방의 발행 전부에 대해 ★**지금 갈 곳**을 다시 센다.
///
/// ★**다시 세는 것이 갱신이다** — 델타로 고치면 입·퇴장과 발행·해제가 겹칠 때
/// 한 걸음이 빠지고, 그 빠짐은 *"한 사람만 영상이 안 나온다"* 로 나타난다.
pub fn routes_for_room(node: &Node, room_id: &str) -> Vec<(u32, Vec<crate::transport::udp::Target>)> {
    let Some(room) = node.rooms.get(room_id) else { return Vec::new() };
    let members = room.session_ids();
    node.publications
        .iter()
        .filter(|p| p.room_id == room_id && p.duplex == Duplex::Full)
        .map(|p| {
            let targets = members
                .iter()
                .filter(|sid| *sid != &p.session_id)
                .filter_map(|sid| {
                    let peer = node.peers.get(sid)?;
                    // ★**배정이 있는 사람에게만 간다** — 배정이 없으면 받을 자리가 없다.
                    let a = peer.assigns.get(&p.track_id)?;
                    Some(crate::transport::udp::Target {
                        ufrag: peer.recv_ufrag().to_string(),
                        pt: a.pt,
                        slot: None,
                    })
                })
                .collect();
            // ★**시뮬캐스트는 vssrc 로 건다** — 들어오는 ssrc 는 단마다 다르고 미리 알 수 없다.
            (p.vssrc.unwrap_or(p.ssrc), targets)
        })
        .collect()
}

/// 시뮬캐스트 등록 — ★**그 발행 자격에서 rid 를 달고 오는 것은 이 vssrc 것**이다.
///
/// ★**등록 항목이 `rid` 를 안 싣기 때문에**(연§6-3) 데이터 평면이 RTP 로 배워야 하고,
/// 배울 대상을 알려 주는 것이 이 한 줄이다.
pub fn simulcast_regs(node: &Node, session_id: &str) -> Vec<(String, u32)> {
    let Some(peer) = node.peers.get(session_id) else { return Vec::new() };
    let ufrag = peer.ice.publish_ufrag.clone();
    node.publications
        .iter()
        .filter(|p| p.session_id == session_id)
        .filter_map(|p| p.vssrc.map(|v| (ufrag.clone(), v)))
        .collect()
}

/// ★**한 프레임 = 한 응답**(+ 통지 몇). 조용한 성공이 없다.
pub fn dispatch(node: &mut Node, ing: &Ingress, header: Header, body: &[u8]) -> Outcome {
    match header.op {
        Op::RoomJoin => room_join(node, ing, header, body),
        Op::RoomLeave => room_leave(node, ing, header, body),
        Op::Affiliation => affiliation(node, ing, header, body),
        Op::PublishTracks => publish_tracks(node, ing, header, body),
        Op::Ready => ready(node, ing, header, body),
        Op::TrackSet => track_set(node, ing, header, body),
        // ★미디어 축은 다음 걸음이다 — 조용히 성공하지 않는다.
        _ => Outcome { reply: fail(header, Code::UnknownOp), ..Default::default() },
    }
}

fn room_join(node: &mut Node, ing: &Ingress, header: Header, body: &[u8]) -> Outcome {
    let Ok(req) = serde_json::from_slice::<RoomJoinReq>(body) else {
        return Outcome { reply: fail(header, Code::InvalidPayload), ..Default::default() };
    };
    if node.rooms.get(&req.room_id).is_none() {
        return Outcome { reply: fail(header, Code::RoomNotFound), ..Default::default() };
    }
    // ★**Peer 가 단위다** — 같은 신원의 옛 Peer 는 그 서버의 **모든 방**에서 함께 걷힌다(정§4-2 ②).
    let e = node.peers.ensure(&ing.session_id, &ing.user_id, ing.pc_mode);
    let i = e.idx;
    if let Some(old) = &e.evicted {
        // ★걷힌 Peer 의 전송 자격도 같이 내린다 — 안 내리면 ★**옛 패킷이 새 Peer 를 건드린다.**
        node.ice.drop_session(old);
    }
    if e.created {
        // ★**자격은 Peer 마다 새로 발급한다**(연§9-3) — 재입장이 새 자격인 근거가 여기다.
        let p = node.peers.at(i);
        let (pu, su) = (p.ice.publish_ufrag.clone(), p.ice.subscribe_ufrag.clone());
        let (pp, sp) = (p.ice.publish_pwd.clone(), p.ice.subscribe_pwd.clone());
        node.ice.insert(&pu, &pp, &ing.session_id, IceRole::Publish);
        node.ice.insert(&su, &sp, &ing.session_id, IceRole::Subscribe);
    }
    let mut notices = Vec::new();
    for r in e.orphaned {
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
        return Outcome { reply: fail(header, code), notices, ..Default::default() };
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

    // ★★**완결 스냅샷이다 — 기존 트랙까지 담는다**(연§6-2 · 정§15-4 안전망).
    //
    // ★**늦게 들어온 사람에게는 이것이 유일한 기회다** — `TRACK_EVENT{add}` 는 발행하는
    // 순간에만 나가므로, 여기서 안 담으면 그 사람은 배정을 ★**영영 못 받고** 이미 흐르는
    // 스트림이 화면에 안 뜬다(관심 선언과 JOIN 사이의 창을 메우는 것도 이 자리다).
    let mut tracks = vec![node.audio_slot(&req.room_id, slot_ssrc, mid)];
    let existing: Vec<Publication> = node
        .publications
        .iter()
        .filter(|p| p.room_id == req.room_id && p.duplex == Duplex::Full)
        .filter(|p| p.session_id != ing.session_id)
        .cloned()
        .collect();
    for p in &existing {
        let mut e = p.entry();
        e.assign = assign_for(node.peers.at_mut(i), p);
        tracks.push(e);
    }

    let res = RoomJoinRes {
        room_id: req.room_id.clone(),
        participants,
        affiliation,
        server_config: node.server_config(i),
        tracks,
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
    let routes = routes_for_room(node, &req.room_id);
    Outcome { reply: ok(header, &json(&res)), notices, routes, ..Default::default() }
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
        return Outcome { reply: fail(header, Code::InvalidPayload), ..Default::default() };
    };
    let Some(room) = node.rooms.get_mut(&req.room_id) else {
        return Outcome { reply: fail(header, Code::RoomNotFound), ..Default::default() };
    };
    // ★**안 들어간 방에서 나가는 것은 `3002`** 다 — 없는 방(`3001`)과 다른 축이다.
    if !room.leave(&ing.user_id) {
        return Outcome { reply: fail(header, Code::NotInRoom), ..Default::default() };
    }
    let version = room.version(&node.epoch);
    let mut notices = Vec::new();
    if !ing.hidden {
        notices.push(left_notice(&req.room_id, &ing.user_id, version));
    }

    let Some(peer) = node.peers.get_mut(&ing.session_id) else {
        return Outcome { reply: fail(header, Code::SessionNotFound), notices, ..Default::default() };
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
    let routes = routes_for_room(node, &req.room_id);
    Outcome { reply: ok(header, &json(&res)), notices, routes, ..Default::default() }
}

fn affiliation(node: &mut Node, ing: &Ingress, header: Header, body: &[u8]) -> Outcome {
    let req: AffiliationReq = if body.is_empty() {
        AffiliationReq::default()
    } else {
        match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => return Outcome { reply: fail(header, Code::InvalidPayload), ..Default::default() },
        }
    };
    let Some(peer) = node.peers.get_mut(&ing.session_id) else {
        return Outcome { reply: fail(header, Code::SessionNotFound), ..Default::default() };
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
            return Outcome { reply: fail(header, Code::NotInRoom), ..Default::default() };
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
    Outcome { reply: ok(header, &json(&res)), ..Default::default() }
}

/// ★**퇴장 정리 — 순서가 계약이다**(정§17-2). `ROOM_LEAVE`·축출·좀비 회수 셋이 이리로 모인다.
///
/// ```text
/// ① 발언권 정리 먼저      — 발언권 덩어리에서 붙인다(이행)
/// ② 명단 제거             — 실패하면 멈춘다(거짓 성공 금지)
/// ③ 구독 역색인 detach    — 구독 덩어리(이행)
/// ④ 마지막 방이면 전송 해제 — ufrag·주소 내리고 Peer 제거, ★**태스크 종료까지가 회수다**
/// ⑤ 화자 판정기에서 제거   — 발언권 덩어리(이행)
/// ⑥ TRACK_EVENT{remove}   — 트랙 덩어리(이행)
/// ⑦ PARTICIPANT_EVENT{left} broadcast
/// ⑧ 좀비 경로만 — 방마다 `ROOM_EVENT{media_lost}` unicast
/// ```
///
/// ★**④가 태스크 종료를 빠뜨리면 누수다**(실사고 20260814) — 그래서 부르는 쪽이
/// `Reaped.session_id` 로 전송 루프의 통로까지 닫는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaped {
    pub session_id: String,
    pub user_id: String,
    pub notices: Vec<Notice>,
    pub routes: Vec<(u32, Vec<crate::transport::udp::Target>)>,
}

pub fn reap(node: &mut Node, session_id: &str, zombie: bool) -> Option<Reaped> {
    let (user_id, hidden) = {
        let p = node.peers.get_mut(session_id)?;
        // 투명 여부는 방 명단이 안다 — Peer 는 신원만 쥔다.
        (p.user_id.clone(), false)
    };
    let rooms = node.peers.drop_session(session_id);
    let mut notices = Vec::new();
    for r in &rooms {
        let Some(room) = node.rooms.get_mut(r) else { continue };
        let was_hidden = room.is_hidden(&user_id);
        // ② 명단 제거 — 없으면 그 방은 건너뛴다(거짓 성공 금지).
        if !room.leave(&user_id) {
            continue;
        }
        let version = room.version(&node.epoch);
        // ⑦ ★`hidden` 은 **발신 제외**다 — 그 사람의 퇴장을 남에게 보내지 않는다.
        if !was_hidden && !hidden {
            notices.push(left_notice(r, &user_id, version));
        }
        // ⑧ ★**좀비 경로만** — 방마다 하나씩, 당사자에게만.
        if zombie {
            notices.push(unicast(
                r,
                &user_id,
                Op::RoomEvent,
                &RoomEvent {
                    event_type: RoomEventType::Affiliation,
                    room_id: r.clone(),
                    // ★회수 뒤의 스냅샷이다 — 그 서버에서 아무 데도 안 듣고 안 말한다.
                    affiliation: Some(oxsig::Affiliation { sub_rooms: Vec::new(), pub_room: None }),
                    cause: Some(AffiliationCause::MediaLost),
                    reason: None,
                },
            ));
        }
    }
    // ④ 전송 등록 해제 — ★자격을 내려야 옛 패킷이 새 Peer 를 못 건드린다.
    node.ice.drop_session(session_id);
    // ★**그 사람이 올리던 것도 끊는다** — 발행자가 갔는데 목록만 남으면 죽은 ssrc 가 표에 남는다.
    let mut routes: Vec<(u32, Vec<crate::transport::udp::Target>)> = node
        .publications
        .iter()
        .filter(|p| p.session_id == session_id)
        .map(|p| (p.ssrc, Vec::new()))
        .collect();
    node.publications.retain(|p| p.session_id != session_id);
    for r in &rooms {
        routes.extend(routes_for_room(node, r));
    }
    Some(Reaped { session_id: session_id.to_string(), user_id, notices, routes })
}

/// reaper 한 걸음 — ★**주체는 하나다**(정§2-2). 회수까지 같은 tick 안에서 끝난다.
///
/// ★`Zombie` 는 종착이 아니라 삭제다 — 상태로 남겨 두면 다음 tick 이 또 회수한다.
pub fn reaper_tick(node: &mut Node, now: u64) -> Vec<Reaped> {
    let seen = node.ice.last_seen_by_session();
    let mut dead = Vec::new();
    for (sid, last) in seen {
        let h = node.health.entry(sid.clone()).or_default();
        *h = h.tick(last, now);
        if h.state == crate::reaper::PeerState::Zombie {
            dead.push(sid);
        }
    }
    let mut out = Vec::new();
    for sid in dead {
        node.health.remove(&sid);
        if let Some(r) = reap(node, &sid, true) {
            eprintln!("[reap] {} 회수 — 방 {}", r.session_id, r.notices.len());
            out.push(r);
        }
    }
    // 사라진 세션의 판정표도 함께 거둔다 — 안 거두면 표가 단조 증가한다.
    let live: Vec<String> = node.ice.last_seen_by_session().into_iter().map(|(s, _)| s).collect();
    node.health.retain(|s, _| live.contains(s));
    out
}

// ─── 트랙 등록(연§6-3) ──────────────────────────────────────────────────────

/// 등록 층 하나 — ★**스냅샷 밖이다**(연§4-1-1). 서버와 발행자 본인만 안다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub track_id: String,
    pub room_id: String,
    /// 발행자 쪽 m-line 번호 — ★**등록 신고값 그대로다**(서버는 SDP 를 안 본다).
    pub mid: String,
    pub session_id: String,
    pub user_id: String,
    pub kind: Kind,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub codec: Option<String>,
    pub fmtp: Option<String>,
    /// 발행자가 쓰는 PT — ★**egress 에 그대로 나가지 않는다**(구독자 표가 정한다, 정§7-2-1).
    pub pt: u8,
    pub duplex: Duplex,
    pub simulcast: bool,
    pub source: Option<Source>,
    /// ★**가상 SSRC** — 시뮬캐스트의 egress 값이다(정§4-1 식별 3평면).
    ///
    /// ★**물리가 바뀌어도 논리가 산다** — 단이 분화·교체돼도 이 값이 보존되므로
    /// 구독자는 단 전환에 재협상을 하지 않는다. 시뮬캐스트가 아니면 `None`(원본을 쓴다).
    pub vssrc: Option<u32>,
}

/// ★`0` 을 피한다 — RTP 에서 `0` 은 값이 아니라 *"안 정해졌다"* 로 읽히는 자리가 많다.
fn fresh_ssrc() -> u32 {
    let mut raw = [0u8; 4];
    getrandom::fill(&mut raw).expect("OS 난수");
    u32::from_be_bytes(raw) | 1
}

/// ★**지원하는 video 코덱 전량**(정§6-2 1차) — 키프레임 판정기가 없으면 지원이 아니다.
pub const SUPPORTED_VIDEO: &[&str] = &["VP8", "H264"];
/// 요청당 상한 · 한 사람 활성 상한(연§6-3).
const PER_REQUEST_MAX: usize = 8;
const PER_USER_MAX: usize = 16;

impl Publication {
    /// 이 스트림이 구독자에게 보이는 모습(스트림 층). ★**배정은 부르는 쪽이 얹는다.**
    fn entry(&self) -> TrackEntry {
        TrackEntry {
            stream_type: StreamType::Individual,
            room_id: self.room_id.clone(),
            track_id: self.track_id.clone(),
            kind: self.kind,
            // ★전이중 non-sim 의 egress SSRC 는 ★**원본**이고, 시뮬캐스트는 ★**vssrc** 다
            //   (정§8-1). 항목의 값이 곧 도착할 값이라야 클라가 지은 받기 m-line 의
            //   `a=ssrc` 와 맞는다 — 여기가 어긋나면 패킷은 오는데 화면이 검다.
            ssrc: self.vssrc.unwrap_or(self.ssrc),
            rtx_ssrc: self.rtx_ssrc,
            codec: self.codec.clone(),
            fmtp: self.fmtp.clone(),
            user_id: Some(self.user_id.clone()),
            source: self.source,
            active: None,
            muted: None,
            scalability: if self.simulcast { Some("L2T1".into()) } else { None },
            assign: None,
        }
    }
}

fn publish_tracks(node: &mut Node, ing: &Ingress, header: Header, body: &[u8]) -> Outcome {
    let Ok(req) = serde_json::from_slice::<PublishTracksReq>(body) else {
        return Outcome { reply: fail(header, Code::InvalidPayload), ..Default::default() };
    };
    // ★**입장한 방만**(연§6-3 `3002`).
    let in_room = node
        .peers
        .get_mut(&ing.session_id)
        .is_some_and(|p| p.sub_rooms.iter().any(|r| r == &req.room_id));
    if !in_room {
        return Outcome { reply: fail(header, Code::NotInRoom), ..Default::default() };
    }
    match req.action.unwrap_or_default() {
        PublishAction::Add => publish_add(node, ing, header, &req),
        PublishAction::Remove => publish_remove(node, ing, header, &req),
    }
}

/// ★★**전량 수용 또는 전량 거절** — 부분 수용이 없다(연§6-3).
///
/// ★**검사를 먼저 다 하고 그다음에 담는다** — 담으면서 검사하면 뒤엣것이 틀렸을 때
/// 앞엣것이 이미 들어가 있다(되돌리는 코드가 또 필요해지고, 그 코드가 빠진다).
fn publish_add(node: &mut Node, ing: &Ingress, header: Header, req: &PublishTracksReq) -> Outcome {
    let none = Vec::new();
    if req.tracks.is_empty() {
        return Outcome { reply: fail(header, Code::MissingField), notices: none, ..Default::default() };
    }
    let active = node.publications.iter().filter(|p| p.user_id == ing.user_id).count();
    if req.tracks.len() > PER_REQUEST_MAX || active + req.tracks.len() > PER_USER_MAX {
        return Outcome { reply: fail(header, Code::TrackLimit), notices: none, ..Default::default() };
    }
    for t in &req.tracks {
        // ★`pt`·`ssrc`·`mid` 미신고는 `1003` — audio 도 예외가 아니다(폴백은 무음을 조용히 만든다).
        if t.mid.is_empty() || t.pt == 0 || (t.ssrc == 0 && !t.simulcast.unwrap_or(false)) {
            return Outcome { reply: fail(header, Code::MissingField), notices: none, ..Default::default() };
        }
        // ★`source` 는 video 만 — audio 항목에 실리면 `1002`(닫힌 집합은 oxsig 가 이미 걸렀다).
        if t.kind == Kind::Audio && t.source.is_some() {
            return Outcome { reply: fail(header, Code::InvalidPayload), notices: none, ..Default::default() };
        }
        if t.kind == Kind::Video {
            let ok = t
                .codec
                .as_deref()
                .is_some_and(|c| SUPPORTED_VIDEO.iter().any(|s| s.eq_ignore_ascii_case(c)));
            if !ok {
                // ★**무엇이 되는지 같이 준다** — 거절만 하면 클라가 찍어 보며 배운다.
                let f = Failure::new(Code::CodecRequired)
                    .with_details(serde_json::json!({ "supported": SUPPORTED_VIDEO }));
                let b = serde_json::to_vec(&f).unwrap_or_default();
                let mut out = Vec::with_capacity(frame::HEADER_LEN + b.len());
                frame::encode(&mut out, Header { kind: FrameKind::Fail, ..header }, &b);
                return Outcome { reply: out, notices: none, ..Default::default() };
            }
        }
    }

    // 검사를 다 지났다 — 이제 담는다.
    let mut made = Vec::new();
    for t in &req.tracks {
        let track_id = format!("tr-{}", uuid::Uuid::new_v4().simple());
        let simulcast = match t.kind {
            Kind::Audio => false,
            Kind::Video => t.simulcast.unwrap_or(t.duplex.unwrap_or(Duplex::Full) == Duplex::Full),
        };
        node.publications.push(Publication {
            track_id: track_id.clone(),
            room_id: req.room_id.clone(),
            mid: t.mid.clone(),
            session_id: ing.session_id.clone(),
            user_id: ing.user_id.clone(),
            kind: t.kind,
            ssrc: t.ssrc,
            rtx_ssrc: t.rtx_ssrc,
            codec: t.codec.clone(),
            fmtp: t.fmtp.clone(),
            pt: t.pt,
            duplex: t.duplex.unwrap_or(Duplex::Full),
            // ★추론은 video 만 — audio 는 언제나 `false`(오디오에 시뮬캐스트는 없다).
            simulcast,
            source: t.source,
            // ★시뮬캐스트면 그 자리에서 발급한다 — 단이 오기 **전에** 있어야
            //   구독자에게 알릴 `TrackEntry.ssrc` 가 선다.
            vssrc: simulcast.then(fresh_ssrc),
        });
        made.push(PublishedTrack { mid: t.mid.clone(), track_id });
    }

    let notices = announce(node, &made, &req.room_id, &ing.user_id, TrackAction::Add);
    let routes = routes_for_room(node, &req.room_id);
    let sims = simulcast_regs(node, &ing.session_id);
    let res = PublishTracksRes { action: PublishAction::Add, tracks: made };
    Outcome { reply: ok(header, &json(&res)), notices, routes, sims }
}

fn publish_remove(node: &mut Node, ing: &Ingress, header: Header, req: &PublishTracksReq) -> Outcome {
    let mut gone = Vec::new();
    for id in &req.track_ids {
        if let Some(i) = node
            .publications
            .iter()
            .position(|p| &p.track_id == id && p.session_id == ing.session_id)
        {
            gone.push(node.publications.remove(i));
        }
    }
    let mut notices = Vec::new();
    if !gone.is_empty() {
        let room = gone[0].room_id.clone();
        let version = match node.rooms.get_mut(&room) {
            Some(r) => {
                r.bump_stream();
                r.version(&node.epoch)
            }
            None => Version { epoch: node.epoch.clone(), seq: 0 },
        };
        let members = node.rooms.get(&room).map(|r| r.session_ids()).unwrap_or_default();
        for sid in members {
            let Some(p) = node.peers.get_mut(&sid) else { continue };
            let mut entries = Vec::new();
            for g in &gone {
                // ★**세 자료를 같이 지운다**(정§17-2 ⑥) — mid 만 지우고 자리를 안 돌리면
                //   재입장 영상이 안 나온다(실사고).
                if let Some(a) = p.assigns.remove(&g.track_id) {
                    p.mids.give(g.kind, &a.mid);
                    let mut e = g.entry();
                    e.assign = Some(a);
                    entries.push(e);
                }
            }
            if entries.is_empty() {
                continue;
            }
            let user = p.user_id.clone();
            notices.push(unicast(
                &room,
                &user,
                Op::TrackEvent,
                &TrackEvent {
                    action: TrackAction::Remove,
                    room_id: room.clone(),
                    tracks: entries,
                    version: version.clone(),
                },
            ));
        }
    }
    // ★지워진 `ssrc` 는 빈 목록으로 밀어 끊는다 — 안 끊으면 죽은 트랙이 계속 흐른다.
    let mut routes: Vec<(u32, Vec<crate::transport::udp::Target>)> =
        gone.iter().map(|g| (g.ssrc, Vec::new())).collect();
    if let Some(g) = gone.first() {
        routes.extend(routes_for_room(node, &g.room_id));
    }
    // ★응답에 `tracks` 필드 자체가 없다(연§6-3).
    let res = PublishTracksRes { action: PublishAction::Remove, tracks: Vec::new() };
    Outcome { reply: ok(header, &json(&res)), notices, routes, ..Default::default() }
}

/// 그 구독자에게 이 스트림의 자리를 잡아 준다. ★**이미 있으면 그것을 그대로 쓴다** —
/// 다시 발급하면 클라가 지은 m-line 과 어긋난다.
///
/// ★**고갈이면 `None`** — 배정 없이 항목만 간다(연§4-1 *"고갈 시 없다"*).
fn assign_for(peer: &mut crate::peer::Peer, p: &Publication) -> Option<Assign> {
    if let Some(a) = peer.assigns.get(&p.track_id) {
        return Some(a.clone());
    }
    let key = crate::pt::Tuple::new(p.kind, p.codec.as_deref().unwrap_or("opus"), p.fmtp.as_deref());
    let (pt, rtx_pt) = peer.pt.get_or_assign(&key, p.rtx_ssrc.is_some())?;
    let a = Assign { mid: peer.mids.take(p.kind), pt, rtx_pt };
    peer.assigns.insert(p.track_id.clone(), a.clone());
    Some(a)
}

/// ★★**수신자마다 프레임이 다르다** — `assign` 이 수신자 것이기 때문이다(연§4-1-1).
///
/// 그래서 broadcast 가 아니라 ★**사람마다 한 장**이고, ★**발행자 본인에게도 자기 항목**이 간다
/// (`assign` 없이 — 자기 트랙이라 배정할 자리가 없다, 정§14-2).
fn announce(
    node: &mut Node,
    made: &[PublishedTrack],
    room_id: &str,
    publisher: &str,
    action: TrackAction,
) -> Vec<Notice> {
    let pubs: Vec<Publication> = made
        .iter()
        .filter_map(|m| node.publications.iter().find(|p| p.track_id == m.track_id).cloned())
        // ★**반이중 개인 트랙은 구독자 항목을 만들지 않는다**(연§4-1-1) — 슬롯으로 흐른다.
        .filter(|p| p.duplex == Duplex::Full)
        .collect();
    if pubs.is_empty() {
        return Vec::new();
    }
    let Some(room) = node.rooms.get_mut(room_id) else { return Vec::new() };
    // ★스트림 층이 바뀌었다 — `seq` 가 오른다(연§4-6-1).
    room.bump_stream();
    let version = room.version(&node.epoch);
    let members = room.session_ids();

    let mut out = Vec::new();
    for sid in members {
        let Some(peer) = node.peers.get_mut(&sid) else { continue };
        let me = peer.user_id.clone();
        let mut entries = Vec::new();
        for p in &pubs {
            let mut e = p.entry();
            // ★**발행자 본인에게는 `assign` 이 없다** — 자기 트랙이라 배정할 자리가 없다.
            if me != publisher {
                e.assign = assign_for(peer, p);
            }
            entries.push(e);
        }
        out.push(unicast(
            room_id,
            &me,
            Op::TrackEvent,
            &TrackEvent {
                action,
                room_id: room_id.to_string(),
                tracks: entries,
                version: version.clone(),
            },
        ));
    }
    out
}

/// `0x0302 READY` — ★**클라의 준비 신호**다(연§6-3). 응답은 빈 body.
///
/// ★**`tracks` 가 푸는 것은 그 서버의 내 것 전부**이고 키프레임 요청만 그 방 것이다.
/// 그래서 게이트는 Peer 에 한 벌만 둔다 — 방마다 두면 같은 것을 여러 번 풀게 된다.
fn ready(node: &mut Node, ing: &Ingress, header: Header, body: &[u8]) -> Outcome {
    let Ok(req) = serde_json::from_slice::<ReadyReq>(body) else {
        return Outcome { reply: fail(header, Code::InvalidPayload), ..Default::default() };
    };
    let Some(peer) = node.peers.get_mut(&ing.session_id) else {
        return Outcome { reply: fail(header, Code::SessionNotFound), ..Default::default() };
    };
    if !peer.sub_rooms.iter().any(|r| r == &req.room_id) {
        return Outcome { reply: fail(header, Code::NotInRoom), ..Default::default() };
    }
    match req.ready_type {
        ReadyType::Tracks => peer.ready = true,
        // ★`camera` 는 ★**정체 판정의 시작점**일 뿐 — 키프레임을 요청하지 않고
        //   남에게 통지도 내지 않는다(정§7-4 · 연§6-3 16차 결재). 정체 감지 덩어리에서 쓴다.
        ReadyType::Camera => {}
    }
    Outcome { reply: ok(header, b"{}"), ..Default::default() }
}

// ─── 발언권(연§11) ─────────────────────────────────────────────────────────

/// DC 로 나갈 한 장 — ★**받는 자격과 바이트**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DcOut {
    pub ufrag: String,
    pub wire: Vec<u8>,
}

/// 발언권 한 걸음이 낸 것 — ★**말과 배관이 한 묶음**이다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FloorOut {
    pub dc: Vec<DcOut>,
    pub routes: Vec<(u32, Vec<crate::transport::udp::Target>)>,
}

/// 발언권 한 통을 처리한다. ★**판정은 `floor::Floor` 가 하고 여기는 어휘를 옮긴다.**
///
/// ★**방은 요청이 들고 온다**(`0x1D` 단수, 연§11-5) — 서버가 추측하지 않는다.
/// 실을 방이 없으면 ★**돌려줄 곳이 없어** 조용히 버린다(정§9-6 입구 관문).
pub fn on_floor(node: &mut Node, ufrag: &str, payload: &[u8], now: u64) -> FloorOut {
    let Some(entry) = node.ice.get(ufrag) else { return FloorOut::default() };
    let session_id = entry.session_id.clone();
    let Some(msg) = oxsig::mbcp::decode(payload) else { return FloorOut::default() };
    let Some(room_id) = msg.room().map(str::to_string) else { return FloorOut::default() };
    let Some(peer) = node.peers.get(&session_id) else { return FloorOut::default() };
    let user_id = peer.user_id.clone();
    let in_pub_room = peer.pub_room.as_deref() == Some(room_id.as_str());
    if !peer.sub_rooms.iter().any(|r| r == &room_id) {
        return FloorOut::default();
    }
    let has_half_track = node
        .publications
        .iter()
        .any(|p| p.session_id == session_id && p.duplex == Duplex::Half);

    let max_burst = node.max_burst_ms;
    let floor = node.floors.entry(room_id.clone()).or_insert_with(|| Floor::new(max_burst));
    let outs = match msg.msg_type {
        oxsig::mbcp::REQUEST => floor.on_request(
            &floor::Request {
                user_id: user_id.clone(),
                // ★**요청이 실은 값이 전부다** — 서버가 깎지 않는다(연§11-3).
                priority: msg.get_u8(oxsig::mbcp::F_PRIORITY).unwrap_or(0),
                // ★**안 실었으면 `None`** — *"0초"* 가 아니다(0 이면 허가 즉시 만료다).
                want_ms: msg.get_u16(oxsig::mbcp::F_DURATION).map(|s| u64::from(s) * 1_000),
                in_pub_room,
                has_half_track,
                // 권한 비트는 방이 기억한다(연§4-4-1) — 아직 기본이 전부다.
                allowed: true,
                others_present: node
                    .rooms
                    .get(&room_id)
                    .is_some_and(|r| r.participants().len() > 1),
            },
            now,
        ),
        oxsig::mbcp::RELEASE => floor.on_release(&user_id, now),
        oxsig::mbcp::QUEUE_POS_REQUEST => floor.on_queue_pos(&user_id),
        // ★**ACK 에 답하지 않는다**(정§9-6 · 22차) — 답하면 ACK 의 ACK 가 생긴다.
        oxsig::mbcp::ACK => Vec::new(),
        // 그 밖은 서버가 내는 것이라 받을 일이 없다 — 조용히 버린다.
        _ => Vec::new(),
    };
    // ★**말과 배관을 같이 낸다** — 가르면 허가는 갔는데 소리가 안 나는 창이 생긴다.
    FloorOut { dc: emit_floor(node, &room_id, outs), routes: floor_routes(node, &room_id) }
}

/// 지금 말하는 사람의 반이중 발행이 갈 곳. ★**게이트가 곧 배관이다**(정§7-3 prefan).
///
/// ★★**허가가 없으면 목록이 빈다** — 그것이 *"허가 전 발화가 안 나간다"* 의 실체다.
/// 검사로 막는 것이 아니라 ★**보낼 곳이 없는 것**이라, 검사를 빠뜨릴 자리가 없다.
pub fn floor_routes(node: &Node, room_id: &str) -> Vec<(u32, Vec<crate::transport::udp::Target>)> {
    let speaker = node.floors.get(room_id).and_then(|f| f.speaker()).map(str::to_string);
    let Some(room) = node.rooms.get(room_id) else { return Vec::new() };
    let slot_ssrc = room.slot_audio_ssrc;
    let slot_track = format!("ptt-{room_id}-audio");
    let members = room.session_ids();
    node.publications
        .iter()
        .filter(|p| p.room_id == room_id && p.duplex == Duplex::Half && p.kind == Kind::Audio)
        .map(|p| {
            let talking = speaker.as_deref() == Some(p.user_id.as_str());
            let targets = if talking {
                members
                    .iter()
                    .filter(|sid| *sid != &p.session_id)
                    .filter_map(|sid| {
                        let peer = node.peers.get(sid)?;
                        // ★**슬롯의 배정**을 쓴다 — 입장 응답으로 이미 알린 그 값이라야
                        //   클라가 지은 m-line 과 맞는다.
                        let a = peer.assigns.get(&slot_track)?;
                        Some(crate::transport::udp::Target {
                            ufrag: peer.recv_ufrag().to_string(),
                            pt: a.pt,
                            slot: Some(slot_ssrc),
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            (p.ssrc, targets)
        })
        .collect()
}

/// 발언권 tick — ★**주기는 하나다**(정§9 `2초`). `T1`·`T2` 가 여기서 돈다.
pub fn floor_tick(node: &mut Node, now: u64) -> FloorOut {
    let rooms: Vec<String> = node.floors.keys().cloned().collect();
    let mut out = FloorOut::default();
    for r in rooms {
        let Some(f) = node.floors.get_mut(&r) else { continue };
        let outs = f.tick(now);
        if !outs.is_empty() {
            out.dc.extend(emit_floor(node, &r, outs));
            out.routes.extend(floor_routes(node, &r));
        }
    }
    out
}

/// `Out` 을 wire 로 옮긴다. ★**unicast 와 broadcast 를 여기서 가른다.**
fn emit_floor(node: &Node, room_id: &str, outs: Vec<floor::Out>) -> Vec<DcOut> {
    use oxsig::mbcp::{self, Msg};
    let mut wire = Vec::new();
    for o in outs {
        let (to, m) = match o {
            floor::Out::Granted { user_id, priority, remaining_ms } => (
                Some(user_id),
                // ★서버가 내는 `GRANTED`·`DENY` 는 `A` 비트를 세운다(연§11-2).
                Msg::new(mbcp::GRANTED)
                    .with_str(mbcp::F_ROOM, room_id)
                    .with_u8(mbcp::F_PRIORITY, priority)
                    .with_u16(mbcp::F_DURATION, (remaining_ms / 1_000) as u16)
                    .ack(),
            ),
            floor::Out::Deny { user_id, cause } => (
                Some(user_id),
                Msg::new(mbcp::DENY)
                    .with_str(mbcp::F_ROOM, room_id)
                    .with_u8(mbcp::F_CAUSE, cause)
                    .ack(),
            ),
            floor::Out::Revoke { user_id, cause } => (
                Some(user_id),
                Msg::new(mbcp::REVOKE).with_str(mbcp::F_ROOM, room_id).with_u8(mbcp::F_CAUSE, cause),
            ),
            floor::Out::QueueInfo { user_id, position, size, priority } => (
                Some(user_id),
                Msg::new(mbcp::QUEUE_INFO)
                    .with_str(mbcp::F_ROOM, room_id)
                    // ★**두 바이트인데 u16 이 아니다**(연§11-3) — byte0 순번 · byte1 허가된 우선순위.
                    .with(mbcp::F_QUEUE_INFO, vec![position, priority])
                    // ★대기 인원은 u8 이다.
                    .with_u8(mbcp::F_QUEUE_SIZE, size),
            ),
            floor::Out::Taken { speaker, seq } => (
                None,
                Msg::new(mbcp::TAKEN)
                    .with_str(mbcp::F_ROOM, room_id)
                    .with_str(mbcp::F_GRANTED_PARTY, &speaker)
                    .with_u16(mbcp::F_SEQ, seq),
            ),
            floor::Out::Idle { seq, prev } => {
                let mut m =
                    Msg::new(mbcp::IDLE).with_str(mbcp::F_ROOM, room_id).with_u16(mbcp::F_SEQ, seq);
                // ★**있을 때만 싣는다** — 처음부터 조용한 방에는 직전 화자가 없다.
                if let Some(p) = prev {
                    m = m.with_str(mbcp::F_PREV_SPEAKER, &p);
                }
                (None, m)
            }
        };
        let Some(body) = m.encode() else { continue };
        let Some(frame) = crate::transport::dc::build(crate::transport::dc::SVC_FLOOR, &body) else {
            continue;
        };
        // ★`TAKEN`·`IDLE` 은 그 방 전원인데 ★**화자는 제외한다**(자기 것은 `GRANTED` 로 안다).
        let skip = match &m.msg_type {
            &mbcp::TAKEN => m.get_str(mbcp::F_GRANTED_PARTY).map(str::to_string),
            _ => None,
        };
        for sid in node.rooms.get(room_id).map(|r| r.session_ids()).unwrap_or_default() {
            let Some(peer) = node.peers.get(&sid) else { continue };
            if let Some(t) = &to
                && &peer.user_id != t
            {
                continue;
            }
            if skip.as_deref() == Some(peer.user_id.as_str()) {
                continue;
            }
            // ★**DC 는 보내기용 연결에 붙는다**(연§3-3) — 받기 자격이 아니다.
            wire.push(DcOut { ufrag: peer.ice.publish_ufrag.clone(), wire: frame.clone() });
        }
    }
    wire
}

/// `0x0304 TRACK_SET` — ★**무전↔회의 전환은 이 경로로만** 한다(연§6-3).
///
/// ★**축 둘은 배타다** — 둘 다 없거나 둘 다 있으면 `1007`. 한 요청이 두 축을 바꾸면
/// 되돌리기가 두 갈래가 되고, 그 둘이 갈리는 순간을 아무도 못 짚는다.
fn track_set(node: &mut Node, ing: &Ingress, header: Header, body: &[u8]) -> Outcome {
    let Ok(req) = serde_json::from_slice::<TrackSetReq>(body) else {
        return Outcome { reply: fail(header, Code::InvalidPayload), ..Default::default() };
    };
    if req.muted.is_some() == req.duplex.is_some() {
        return Outcome { reply: fail(header, Code::FieldConflict), ..Default::default() };
    }
    // ★**`track_id` 를 먼저 쓴다** — `ssrc` 만 보면 다시 발행한 뒤 엉뚱한 트랙이 바뀐다.
    let found = node.publications.iter().position(|p| {
        p.session_id == ing.session_id
            && p.room_id == req.room_id
            && match (&req.track_id, req.ssrc) {
                (Some(id), _) => &p.track_id == id,
                (None, Some(s)) => p.ssrc == s,
                (None, None) => false,
            }
    });
    let Some(i) = found else {
        return Outcome { reply: fail(header, Code::TrackNotFound), ..Default::default() };
    };
    let p = &node.publications[i];
    if let Some(m) = req.muted {
        // ★**반이중에는 mute 가 없다**(연§6-3) — 송출 여부는 발언권 게이트 하나가 정하고,
        //   상대 화면에 개인 타일이 없어 표시할 자리도 없다.
        if p.duplex == Duplex::Half {
            return Outcome { reply: fail(header, Code::TrackOpUnsupported), ..Default::default() };
        }
        // 음소거 축은 다음 걸음이다(`TRACK_STATE` 배달) — 지금은 값만 돌려준다.
        let res = serde_json::json!({ "ssrc": p.ssrc, "muted": m });
        return Outcome { reply: ok(header, &json(&res)), ..Default::default() };
    }
    let want = req.duplex.expect("축 둘 중 하나는 있다");
    // ★★**시뮬캐스트 트랙은 무전 전환 불가**(연§6-3 · 정§8-1) — 두 재기록기가 같은
    //   vssrc 를 다툰다. ★**영구다** — 시뮬캐스트를 끄기 전엔 같은 답이라 클라는
    //   `3006` 을 받으면 바뀌지 않은 것으로 확정하고 다시 시도하지 않는다.
    if want == Duplex::Half && p.simulcast {
        return Outcome { reply: fail(header, Code::TrackOpUnsupported), ..Default::default() };
    }
    if p.duplex == want {
        let res = serde_json::json!({ "ssrc": p.ssrc, "duplex": want, "noop": true });
        return Outcome { reply: ok(header, &json(&res)), ..Default::default() };
    }
    // 전환 자체(잔존 항목 교체·mid 반납·`TRACK_EVENT{add}`)는 트랙 전환 덩어리다 —
    // ★**조용히 성공하지 않는다**: 아직 못 하는 것은 `3006` 으로 답한다.
    Outcome { reply: fail(header, Code::TrackOpUnsupported), ..Default::default() }
}
