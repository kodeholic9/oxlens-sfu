// ★생성 파일 — gen/axis_client.mts 가 원천(context/spec/protocol · areas/room/behavior)에서 짓는다. 손으로 고치지 않는다.
// IR sha256 9f8284b8a4c4f8ec · 영역 ROOM 클라 상태기 Rust 판 — 같은 생성기의 TS 판과 한 뿌리(같은 입구 · 같은 경우 · 같은 라벨).
// 집행은 부르는 쪽 몫이다 — out(보낼 것) · fx(이 영역의 효과) · seams(다른 영역 입구로 — 받는 곳은 위가 정했다) · start(새로 걸 타이머). armed() 밖의 타이머는 멈춘다.
// 모형 자리(트랙 · 배정표 사본의 열쇠)는 문자열이다 — 실제 값을 넣는 법은 결합이 정한다(SDK:RULE-MOD-BINDING-VALUES).
#![allow(dead_code, unused_variables, unused_mut, unused_parens, non_camel_case_types, clippy::all)]

/// 값 이름 Part
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Part {
    req,
    res,
    fail,
    body,
}
impl Part {
    pub const ALL: &'static [Part] = &[Part::req, Part::res, Part::fail, Part::body];
    pub fn as_str(self) -> &'static str {
        match self { Part::req => "req", Part::res => "res", Part::fail => "fail", Part::body => "body", }
    }
    pub fn parse(s: &str) -> Option<Part> {
        match s { "req" => Some(Part::req), "res" => Some(Part::res), "fail" => Some(Part::fail), "body" => Some(Part::body), _ => None }
    }
}

/// 값 이름 PcMode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PcMode {
    #[cfg_attr(feature = "serde", serde(rename = "1pc"))] V1pc,
    #[cfg_attr(feature = "serde", serde(rename = "2pc"))] V2pc,
}
impl PcMode {
    pub const ALL: &'static [PcMode] = &[PcMode::V1pc, PcMode::V2pc];
    pub fn as_str(self) -> &'static str {
        match self { PcMode::V1pc => "1pc", PcMode::V2pc => "2pc", }
    }
    pub fn parse(s: &str) -> Option<PcMode> {
        match s { "1pc" => Some(PcMode::V1pc), "2pc" => Some(PcMode::V2pc), _ => None }
    }
}

/// 값 이름 RoomEv
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RoomEv {
    affiliation,
    sync_required,
}
impl RoomEv {
    pub const ALL: &'static [RoomEv] = &[RoomEv::affiliation, RoomEv::sync_required];
    pub fn as_str(self) -> &'static str {
        match self { RoomEv::affiliation => "affiliation", RoomEv::sync_required => "sync_required", }
    }
    pub fn parse(s: &str) -> Option<RoomEv> {
        match s { "affiliation" => Some(RoomEv::affiliation), "sync_required" => Some(RoomEv::sync_required), _ => None }
    }
}

/// 값 이름 RoomSt
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RoomSt {
    NONE,
    JOINING,
    JOINED,
    LEAVING,
}
impl RoomSt {
    pub const ALL: &'static [RoomSt] = &[RoomSt::NONE, RoomSt::JOINING, RoomSt::JOINED, RoomSt::LEAVING];
    pub fn as_str(self) -> &'static str {
        match self { RoomSt::NONE => "NONE", RoomSt::JOINING => "JOINING", RoomSt::JOINED => "JOINED", RoomSt::LEAVING => "LEAVING", }
    }
    pub fn parse(s: &str) -> Option<RoomSt> {
        match s { "NONE" => Some(RoomSt::NONE), "JOINING" => Some(RoomSt::JOINING), "JOINED" => Some(RoomSt::JOINED), "LEAVING" => Some(RoomSt::LEAVING), _ => None }
    }
}

/// 값 묶음 — 원천에 이름이 없어 쓰인 자리 이름을 붙였다
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DeltaMsgT {
    PARTICIPANT_EVENT,
    PARTICIPANT_STATE,
    TRACK_EVENT,
    TRACK_STATE,
}
impl DeltaMsgT {
    pub const ALL: &'static [DeltaMsgT] = &[DeltaMsgT::PARTICIPANT_EVENT, DeltaMsgT::PARTICIPANT_STATE, DeltaMsgT::TRACK_EVENT, DeltaMsgT::TRACK_STATE];
    pub fn as_str(self) -> &'static str {
        match self { DeltaMsgT::PARTICIPANT_EVENT => "PARTICIPANT_EVENT", DeltaMsgT::PARTICIPANT_STATE => "PARTICIPANT_STATE", DeltaMsgT::TRACK_EVENT => "TRACK_EVENT", DeltaMsgT::TRACK_STATE => "TRACK_STATE", }
    }
    pub fn parse(s: &str) -> Option<DeltaMsgT> {
        match s { "PARTICIPANT_EVENT" => Some(DeltaMsgT::PARTICIPANT_EVENT), "PARTICIPANT_STATE" => Some(DeltaMsgT::PARTICIPANT_STATE), "TRACK_EVENT" => Some(DeltaMsgT::TRACK_EVENT), "TRACK_STATE" => Some(DeltaMsgT::TRACK_STATE), _ => None }
    }
}

/// 이 영역의 효과(앱 알림 등)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Fx {
    #[cfg_attr(feature = "serde", serde(rename = "app.bug"))] app_bug,
    #[cfg_attr(feature = "serde", serde(rename = "app.join_failed"))] app_join_failed,
    #[cfg_attr(feature = "serde", serde(rename = "app.joined"))] app_joined,
    #[cfg_attr(feature = "serde", serde(rename = "app.left"))] app_left,
    #[cfg_attr(feature = "serde", serde(rename = "app.members"))] app_members,
    #[cfg_attr(feature = "serde", serde(rename = "app.room_gone"))] app_room_gone,
    #[cfg_attr(feature = "serde", serde(rename = "http.abandon"))] http_abandon,
    #[cfg_attr(feature = "serde", serde(rename = "ws.abandon"))] ws_abandon,
}
impl Fx {
    pub const ALL: &'static [Fx] = &[Fx::app_bug, Fx::app_join_failed, Fx::app_joined, Fx::app_left, Fx::app_members, Fx::app_room_gone, Fx::http_abandon, Fx::ws_abandon];
    pub fn as_str(self) -> &'static str {
        match self { Fx::app_bug => "app.bug", Fx::app_join_failed => "app.join_failed", Fx::app_joined => "app.joined", Fx::app_left => "app.left", Fx::app_members => "app.members", Fx::app_room_gone => "app.room_gone", Fx::http_abandon => "http.abandon", Fx::ws_abandon => "ws.abandon", }
    }
    pub fn parse(s: &str) -> Option<Fx> {
        match s { "app.bug" => Some(Fx::app_bug), "app.join_failed" => Some(Fx::app_join_failed), "app.joined" => Some(Fx::app_joined), "app.left" => Some(Fx::app_left), "app.members" => Some(Fx::app_members), "app.room_gone" => Some(Fx::app_room_gone), "http.abandon" => Some(Fx::http_abandon), "ws.abandon" => Some(Fx::ws_abandon), _ => None }
    }
}

/// 다른 영역으로 건너가는 효과(이음매)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SeamFx {
    #[cfg_attr(feature = "serde", serde(rename = "aff.down"))] aff_down,
    #[cfg_attr(feature = "serde", serde(rename = "aff.implicit"))] aff_implicit,
    #[cfg_attr(feature = "serde", serde(rename = "aff.join_failed"))] aff_join_failed,
    #[cfg_attr(feature = "serde", serde(rename = "aff.leave"))] aff_leave,
    #[cfg_attr(feature = "serde", serde(rename = "floor.join"))] floor_join,
    #[cfg_attr(feature = "serde", serde(rename = "floor.leave"))] floor_leave,
    #[cfg_attr(feature = "serde", serde(rename = "link.epoch_changed"))] link_epoch_changed,
    #[cfg_attr(feature = "serde", serde(rename = "link.room_left"))] link_room_left,
    #[cfg_attr(feature = "serde", serde(rename = "link.server_ice"))] link_server_ice,
    #[cfg_attr(feature = "serde", serde(rename = "link.table"))] link_table,
    #[cfg_attr(feature = "serde", serde(rename = "pub.room_gone"))] pub_room_gone,
    #[cfg_attr(feature = "serde", serde(rename = "pub.room_left"))] pub_room_left,
    #[cfg_attr(feature = "serde", serde(rename = "ready.tracks"))] ready_tracks,
    #[cfg_attr(feature = "serde", serde(rename = "room.sub_rooms"))] room_sub_rooms,
    #[cfg_attr(feature = "serde", serde(rename = "sdp.rebuild"))] sdp_rebuild,
    #[cfg_attr(feature = "serde", serde(rename = "store.drop"))] store_drop,
    #[cfg_attr(feature = "serde", serde(rename = "sub.drop"))] sub_drop,
    #[cfg_attr(feature = "serde", serde(rename = "trk.delta"))] trk_delta,
    #[cfg_attr(feature = "serde", serde(rename = "trk.snapshot"))] trk_snapshot,
}
impl SeamFx {
    pub const ALL: &'static [SeamFx] = &[SeamFx::aff_down, SeamFx::aff_implicit, SeamFx::aff_join_failed, SeamFx::aff_leave, SeamFx::floor_join, SeamFx::floor_leave, SeamFx::link_epoch_changed, SeamFx::link_room_left, SeamFx::link_server_ice, SeamFx::link_table, SeamFx::pub_room_gone, SeamFx::pub_room_left, SeamFx::ready_tracks, SeamFx::room_sub_rooms, SeamFx::sdp_rebuild, SeamFx::store_drop, SeamFx::sub_drop, SeamFx::trk_delta, SeamFx::trk_snapshot];
    pub fn as_str(self) -> &'static str {
        match self { SeamFx::aff_down => "aff.down", SeamFx::aff_implicit => "aff.implicit", SeamFx::aff_join_failed => "aff.join_failed", SeamFx::aff_leave => "aff.leave", SeamFx::floor_join => "floor.join", SeamFx::floor_leave => "floor.leave", SeamFx::link_epoch_changed => "link.epoch_changed", SeamFx::link_room_left => "link.room_left", SeamFx::link_server_ice => "link.server_ice", SeamFx::link_table => "link.table", SeamFx::pub_room_gone => "pub.room_gone", SeamFx::pub_room_left => "pub.room_left", SeamFx::ready_tracks => "ready.tracks", SeamFx::room_sub_rooms => "room.sub_rooms", SeamFx::sdp_rebuild => "sdp.rebuild", SeamFx::store_drop => "store.drop", SeamFx::sub_drop => "sub.drop", SeamFx::trk_delta => "trk.delta", SeamFx::trk_snapshot => "trk.snapshot", }
    }
    pub fn parse(s: &str) -> Option<SeamFx> {
        match s { "aff.down" => Some(SeamFx::aff_down), "aff.implicit" => Some(SeamFx::aff_implicit), "aff.join_failed" => Some(SeamFx::aff_join_failed), "aff.leave" => Some(SeamFx::aff_leave), "floor.join" => Some(SeamFx::floor_join), "floor.leave" => Some(SeamFx::floor_leave), "link.epoch_changed" => Some(SeamFx::link_epoch_changed), "link.room_left" => Some(SeamFx::link_room_left), "link.server_ice" => Some(SeamFx::link_server_ice), "link.table" => Some(SeamFx::link_table), "pub.room_gone" => Some(SeamFx::pub_room_gone), "pub.room_left" => Some(SeamFx::pub_room_left), "ready.tracks" => Some(SeamFx::ready_tracks), "room.sub_rooms" => Some(SeamFx::room_sub_rooms), "sdp.rebuild" => Some(SeamFx::sdp_rebuild), "store.drop" => Some(SeamFx::store_drop), "sub.drop" => Some(SeamFx::sub_drop), "trk.delta" => Some(SeamFx::trk_delta), "trk.snapshot" => Some(SeamFx::trk_snapshot), _ => None }
    }
}

/// 타이머
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimerName {}
impl TimerName {
    pub const ALL: &'static [TimerName] = &[];
    pub fn as_str(self) -> &'static str {
        match self { }
    }
    pub fn parse(s: &str) -> Option<TimerName> {
        let _ = s;
        None
    }
}

/// 레코드 — 모형 자리(열쇠)를 문자열로 넓힌 것 · 차례 고정
pub type Rec<V> = std::collections::BTreeMap<String, V>;
fn rec<V: Clone>(kv: &[(&str, V)]) -> Rec<V> {
    kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct JoinResMsgServerConfig {
    pub pc_mode: PcMode,
    pub sfu_id: i64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct JoinResMsgVersion {
    pub epoch: i64,
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct JoinResMsg {
    pub code: i64,
    pub join_id: i64,
    pub part: Part,
    pub server_config: JoinResMsgServerConfig,
    pub version: JoinResMsgVersion,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LeaveResMsg {
    pub part: Part,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DeltaMsgVersion {
    pub epoch: i64,
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DeltaMsg {
    pub t: DeltaMsgT,
    pub version: DeltaMsgVersion,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RoomEventMsg {
    pub r#type: RoomEv,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SnapshotMsgVersion {
    pub epoch: i64,
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SnapshotMsg {
    pub version: SnapshotMsgVersion,
}

/// 클라 상태 — 원천 ROOM K
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct St {
    /// ★그 방이 붙은 미디어 서버 — 입장 응답 server_config.sfu_id(0 = 아직) · 같은 방의 SUB · TRK · FLR 은 사본 없이 이것으로 서버를 안다(202610/20261013a 축 B)
    pub sfu: i64,
    /// 방 상태(§[RULE-ROOM-STATES])
    pub rs: RoomSt,
    /// 보관본의 version.seq — 이 번호의 내용을 들고 있다(§[RULE-ROOM-VERSION-CLIENT-RULES])
    pub cseq: i64,
    /// 보관본의 version.epoch — 이 기동의 내용이다(0 = 보관본 없음 · 규칙 1)
    pub cepoch: i64,
    /// 그 방 재동기(GET /rooms)가 진행 중이다(§[RULE-ROOM-TIMERS-MEDIA] 재동기 in-flight)
    pub rsync: bool,
    /// 재동기 중에 버린 델타의 가장 큰 seq — 응답이 이보다 낮으면 따라잡지 못한 것이다(DEV-ROOM-INFLIGHT-AHEAD)
    pub ahead: i64,
    /// 4009 를 받아 retry_after_ms 를 기다린다(§[ROOM.C.JOINING] 6)
    pub retry: bool,
    /// 입장 요청이 나가 있는 동안 앱이 나가려 했다 — 응답을 기다렸다가 성공이면 ROOM_LEAVE 를 보낸다(DEV-ROOM-LEAVE-WHILE-JOINING)
    pub quit: bool,
    /// 재구축이 다시 들어갈 방이다 — 내릴 때 들어가 있었거나 들어가려던 방(DEV-ROOM-REJOIN-LIST · 원칙 ①)
    pub rejoin: bool,
    /// 마지막으로 안 그 방 입장의 join_id(§[RULE-ROOM-JOIN-ID] — 모형은 세대 수 · 0 = 모름) — 재구축의 다시 입장이 갈아 끼울 값으로 싣는다(§[RULE-ROOM-JOIN-REPLACE])
    pub jid: i64,
    /// jid 가 서버의 지금 입장이다 — 입장 응답을 받았다 · 입장 요청을 보내면 거짓(그 요청이 값을 바꿨을 수 있다) · 퇴장이 그 값을 싣는다(거짓이면 비운다 — §[RULE-ROOM-LEAVE-FORM])
    pub jsure: bool,
}
pub fn initial() -> St {
    St { sfu: 0, rs: RoomSt::NONE, cseq: 0, cepoch: 0, rsync: false, ahead: 0, retry: false, quit: false, rejoin: false, jid: 0, jsure: false }
}

/// 이 상태기가 내는 것
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Tx {
    RoomJoin { join_id: i64 },
    RoomLeave { join_id: i64 },
    GetRoom { tracks: i64 },
}
impl Tx {
    pub fn kind(&self) -> &'static str {
        match self {
            Tx::RoomJoin { .. } => "ws",
            Tx::RoomLeave { .. } => "ws",
            Tx::GetRoom { .. } => "ws",
        }
    }
    pub fn op(&self) -> &'static str {
        match self {
            Tx::RoomJoin { .. } => "ROOM_JOIN",
            Tx::RoomLeave { .. } => "ROOM_LEAVE",
            Tx::GetRoom { .. } => "GET_ROOM",
        }
    }
    pub fn part(&self) -> &'static str {
        match self {
            Tx::RoomJoin { .. } => "req",
            Tx::RoomLeave { .. } => "req",
            Tx::GetRoom { .. } => "req",
        }
    }
}
/// 다른 영역으로 — 받는 영역(to) · 받는 개체(at: all = 그 영역 개체 전부 · self = 같은 개체 · sameServer · sameRoom)
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Seam {
    pub fx: SeamFx,
    pub to: &'static str,
    pub at: &'static str,
}
/// 한 걸음의 결과 — label 은 원천의 동작 라벨
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Step {
    pub s: St,
    pub label: &'static str,
    pub out: Vec<Tx>,
    pub fx: Vec<Fx>,
    pub seams: Vec<Seam>,
    pub start: Vec<TimerName>,
}

/// 이음매 — (효과, 받는 영역, 받는 절차) · 위가 정한다(IR seams)
pub const SEAMS: &[(SeamFx, &str, &[&str])] = &[(SeamFx::aff_down, "AFF", &["CDown"]), (SeamFx::aff_implicit, "AFF", &["CImplicit"]), (SeamFx::aff_join_failed, "AFF", &["CJoinFailed"]), (SeamFx::aff_leave, "AFF", &["CLeft"]), (SeamFx::floor_join, "FLR", &["CJoin"]), (SeamFx::floor_leave, "FLR", &["CLeave"]), (SeamFx::link_epoch_changed, "LINK", &["CEpochChanged"]), (SeamFx::link_room_left, "LINK", &["CRoomLeft"]), (SeamFx::link_server_ice, "LINK", &["CServerIce"]), (SeamFx::link_table, "LINK", &["CTableCarried"]), (SeamFx::pub_room_gone, "PUB", &["CRoomGone"]), (SeamFx::pub_room_left, "PUB", &["CRoomGone"]), (SeamFx::ready_tracks, "SUB", &["CReady"]), (SeamFx::room_sub_rooms, "ROOM", &["CSubRooms"]), (SeamFx::sdp_rebuild, "LINK", &["CRenego"]), (SeamFx::store_drop, "TRK", &["CStoreDrop"]), (SeamFx::sub_drop, "SUB", &["CDropRoom"]), (SeamFx::trk_delta, "TRK", &["CTrackEvent", "CTrackState"]), (SeamFx::trk_snapshot, "TRK", &["CSnapshot"])];
/// 이음매가 부르는 입구 — (입구, 부르는 절차) · 아래가 넣는 사건이 아니다
pub const REACTS: &[(&str, &[&str])] = &[("sub_rooms", &["ROOM:room.sub_rooms"]), ("down_all", &["SES:rooms.down_all", "LINK:store.drop_server"]), ("close_all", &["SES:rooms.close_all"]), ("rejoin", &["SES:rooms.rejoin", "LINK:rooms.rejoin_server"]), ("resume_snap", &["SES:rooms.snapshot"])];
/// 읽는 것 — (입구.인자, 읽는 다른 영역 칸) · 부르는 쪽이 넣는다
pub const READS: &[(&str, &[&str])] = &[("join_res.pcm", &["SES.pcm"])];

/// 원장 — 타이머(ms) · 원장에 값이 없으면 None
pub fn timer_ms(t: TimerName) -> Option<i64> {
    match t {}
}
/// 원장 — 횟수
#[allow(non_upper_case_globals)]
pub mod counts {
    pub const BundleCap: i64 = 128;
    pub const ClientMlines: i64 = 32;
    pub const Sub1pc: i64 = 96;
    pub const Sub2pc: i64 = 128;
    pub const DispatchCh: i64 = 20;
    pub const SlotBudget: i64 = 40;
    pub const FullAudio: i64 = 25;
    pub const FullVideo: i64 = 25;
    pub const MaxRooms: i64 = 20;
}

fn done(prev: &St, s: St, label: &'static str, out: Vec<Tx>, fx: Vec<Fx>, seams: Vec<Seam>, mut start: Vec<TimerName>) -> Step {
    let before = armed(prev);
    for t in armed(&s) {
        if !before.contains(&t) && !start.contains(&t) {
            start.push(t);
        }
    }
    Step { s, label, out, fx, seams, start }
}

/// 지금 돌아야 하는 타이머 — 원천 timer(…).armed
pub fn armed(s: &St) -> Vec<TimerName> {
    let mut on = Vec::new();
    on
}

/// 앱이 방에 들어가려 한다(§[ROOM.C.NONE.join]) (앱: 앱이 방에 들어가려 한다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn join(s: &St) -> Option<Step> {
    if !(s.rs == RoomSt::NONE) {
        return None;
    }
    Some(done(s, St { rs: RoomSt::JOINING, cseq: 0, cepoch: 0, rsync: false, ahead: 0, retry: false, quit: false, jid: if s.rejoin { s.jid } else { 0 }, jsure: false, ..s.clone() }, "ROOM.C.NONE.join", vec![Tx::RoomJoin { join_id: if s.rejoin { s.jid } else { 0 } }], vec![], vec![], vec![]))
}

/// ROOM_JOIN 응답을 받았다(§[ROOM.C.JOINING.join_ok] · §[ROOM.C.JOINING]) (받음: ROOM_JOIN) — 받는 상태 밖이면 None(사건이 없던 것) · 읽는 것: pcm = SES.pcm
pub fn join_res(s: &St, m: &JoinResMsg, pcm: PcMode) -> Option<Step> {
    if !(s.rs == RoomSt::JOINING) {
        return None;
    }
    if m.part == Part::res && s.quit {
        return Some(done(s, St { rs: RoomSt::LEAVING, quit: false, sfu: m.server_config.sfu_id, jid: m.join_id, jsure: true, ..s.clone() }, "ROOM.C.JOINING.join_ok_quit", vec![Tx::RoomLeave { join_id: m.join_id }], vec![], vec![Seam { fx: SeamFx::aff_leave, to: "AFF", at: "self" }, Seam { fx: SeamFx::room_sub_rooms, to: "ROOM", at: "sameServer" }], vec![]));
    }
    if m.part != Part::res && s.quit && m.code >= 2000 {
        return Some(done(s, St { rs: RoomSt::NONE, quit: false, retry: false, jid: 0, ..s.clone() }, "ROOM.C.JOINING.join_fail_quit", vec![], vec![Fx::app_left], vec![Seam { fx: SeamFx::aff_join_failed, to: "AFF", at: "self" }, Seam { fx: SeamFx::link_room_left, to: "LINK", at: "sameServer" }], vec![]));
    }
    if m.part == Part::res && m.server_config.pc_mode != pcm {
        return Some(done(s, St { rs: RoomSt::LEAVING, sfu: m.server_config.sfu_id, jid: m.join_id, jsure: true, ..s.clone() }, "ROOM.C.JOINING.join_pc_mismatch", vec![Tx::RoomLeave { join_id: m.join_id }], vec![Fx::app_join_failed], vec![Seam { fx: SeamFx::aff_leave, to: "AFF", at: "self" }, Seam { fx: SeamFx::room_sub_rooms, to: "ROOM", at: "sameServer" }], vec![]));
    }
    if m.part == Part::res {
        return Some(done(s, St { rs: RoomSt::JOINED, cseq: m.version.seq, cepoch: m.version.epoch, sfu: m.server_config.sfu_id, jid: m.join_id, jsure: true, ..s.clone() }, "ROOM.C.JOINING.join_ok", vec![], vec![Fx::app_joined], vec![Seam { fx: SeamFx::trk_snapshot, to: "TRK", at: "self" }, Seam { fx: SeamFx::link_server_ice, to: "LINK", at: "sameServer" }, Seam { fx: SeamFx::link_table, to: "LINK", at: "sameServer" }, Seam { fx: SeamFx::sdp_rebuild, to: "LINK", at: "sameServer" }, Seam { fx: SeamFx::ready_tracks, to: "SUB", at: "self" }, Seam { fx: SeamFx::room_sub_rooms, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::floor_join, to: "FLR", at: "self" }, Seam { fx: SeamFx::aff_implicit, to: "AFF", at: "self" }], vec![]));
    }
    if m.code == 4009 {
        return Some(done(s, St { retry: true, ..s.clone() }, "ROOM.C.JOINING.join_4009", vec![], vec![], vec![], vec![]));
    }
    if m.code < 2000 {
        return Some(done(s, St { rs: RoomSt::NONE, jid: 0, ..s.clone() }, "ROOM.C.JOINING.join_1xxx", vec![], vec![Fx::app_bug], vec![Seam { fx: SeamFx::aff_join_failed, to: "AFF", at: "self" }, Seam { fx: SeamFx::link_room_left, to: "LINK", at: "sameServer" }], vec![]));
    }
    Some(done(s, St { rs: RoomSt::NONE, jid: 0, ..s.clone() }, "ROOM.C.JOINING.join_fail", vec![], vec![Fx::app_join_failed], vec![Seam { fx: SeamFx::aff_join_failed, to: "AFF", at: "self" }, Seam { fx: SeamFx::link_room_left, to: "LINK", at: "sameServer" }], vec![]))
}

/// 4009 의 retry_after_ms 하한이 지났다(서버가 준 값 — 원장 시한이 아니다) (바깥: 4009 의 retry_after_ms 하한이 지났다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn retry_due(s: &St) -> Option<Step> {
    if !(s.rs == RoomSt::JOINING && s.retry) {
        return None;
    }
    Some(done(s, St { retry: false, ..s.clone() }, "ROOM.C.JOINING.retry_join", vec![Tx::RoomJoin { join_id: s.jid }], vec![], vec![], vec![]))
}

/// 앱이 나가려 한다(§[ROOM.C.JOINED.leave]) · JOINING 이면 4009 대기를 멈춘다 (앱: 앱이 방을 나가려 한다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn leave(s: &St) -> Option<Step> {
    if !(s.rs == RoomSt::JOINED || s.rs == RoomSt::JOINING && !s.quit || s.rs == RoomSt::NONE && s.rejoin) {
        return None;
    }
    if s.rs == RoomSt::NONE {
        return Some(done(s, St { rs: RoomSt::LEAVING, rejoin: false, ..s.clone() }, "ROOM.C.NONE.app_leave", vec![Tx::RoomLeave { join_id: if s.jsure { s.jid } else { 0 } }], vec![], vec![Seam { fx: SeamFx::aff_leave, to: "AFF", at: "self" }], vec![]));
    }
    if s.rs == RoomSt::JOINING && s.retry {
        return Some(done(s, St { rs: RoomSt::NONE, retry: false, ..s.clone() }, "ROOM.C.JOINING.app_leave", vec![], vec![Fx::app_left], vec![Seam { fx: SeamFx::aff_join_failed, to: "AFF", at: "self" }, Seam { fx: SeamFx::link_room_left, to: "LINK", at: "sameServer" }], vec![]));
    }
    if s.rs == RoomSt::JOINING {
        return Some(done(s, St { quit: true, ..s.clone() }, "ROOM.C.JOINING.app_leave_wait", vec![], vec![], vec![], vec![]));
    }
    Some(done(s, St { rs: RoomSt::LEAVING, rsync: false, ahead: 0, ..s.clone() }, "ROOM.C.JOINED.leave", vec![Tx::RoomLeave { join_id: s.jid }], vec![Fx::http_abandon], vec![Seam { fx: SeamFx::floor_leave, to: "FLR", at: "self" }, Seam { fx: SeamFx::aff_leave, to: "AFF", at: "self" }, Seam { fx: SeamFx::sub_drop, to: "SUB", at: "self" }, Seam { fx: SeamFx::pub_room_left, to: "PUB", at: "sameRoom" }], vec![]))
}

/// ROOM_LEAVE 응답을 받았다(§[RULE-ROOM-LEAVE-RES]) (받음: ROOM_LEAVE)
pub fn leave_res(s: &St, m: &LeaveResMsg) -> Step {
    if s.rs != RoomSt::LEAVING {
        return done(s, s.clone(), "ROOM.C.any.leave_res_stale", vec![], vec![], vec![], vec![]);
    }
    if m.part == Part::res {
        return done(s, St { rs: RoomSt::NONE, cseq: 0, cepoch: 0, rsync: false, jid: 0, jsure: false, ..s.clone() }, "ROOM.C.LEAVING.leave_ok", vec![], vec![Fx::app_left], vec![Seam { fx: SeamFx::store_drop, to: "TRK", at: "self" }, Seam { fx: SeamFx::link_room_left, to: "LINK", at: "sameServer" }], vec![]);
    }
    done(s, St { rs: RoomSt::NONE, cseq: 0, cepoch: 0, rsync: false, jid: 0, jsure: false, ..s.clone() }, "ROOM.C.LEAVING.leave_fail", vec![], vec![Fx::app_left], vec![Seam { fx: SeamFx::store_drop, to: "TRK", at: "self" }, Seam { fx: SeamFx::link_room_left, to: "LINK", at: "sameServer" }], vec![])
}

/// version 을 실은 통지를 받았다 — ACK · 판정(§[RULE-ROOM-VERSION-CLIENT-RULES] 규칙 3~5) · 반영(§[RULE-ROOM-NOTE-ORDER]) (받음: PARTICIPANT_EVENT · PARTICIPANT_STATE · TRACK_EVENT · TRACK_STATE)
pub fn delta(s: &St, m: &DeltaMsg, bare: bool) -> Step {
    if s.rs != RoomSt::JOINED {
        return done(s, s.clone(), "ROOM.C.any.delta_not_joined", vec![], vec![], vec![], vec![]);
    }
    if bare {
        return done(s, s.clone(), "ROOM.C.JOINED.delta_bare", vec![], vec![Fx::app_members], vec![], vec![]);
    }
    if m.version.epoch != s.cepoch {
        return done(s, s.clone(), "ROOM.C.JOINED.delta_epoch", vec![], vec![], vec![Seam { fx: SeamFx::link_epoch_changed, to: "LINK", at: "sameServer" }], vec![]);
    }
    if m.version.seq <= s.cseq {
        return done(s, s.clone(), "ROOM.C.JOINED.delta_old", vec![], vec![], vec![], vec![]);
    }
    if m.version.seq == s.cseq + 1 && (m.t == DeltaMsgT::TRACK_EVENT || m.t == DeltaMsgT::TRACK_STATE) {
        return done(s, St { cseq: m.version.seq, ..s.clone() }, "ROOM.C.JOINED.delta_apply_track", vec![], vec![], vec![Seam { fx: SeamFx::trk_delta, to: "TRK", at: "self" }], vec![]);
    }
    if m.version.seq == s.cseq + 1 {
        return done(s, St { cseq: m.version.seq, ..s.clone() }, "ROOM.C.JOINED.delta_apply", vec![], vec![Fx::app_members], vec![Seam { fx: SeamFx::trk_delta, to: "TRK", at: "self" }], vec![]);
    }
    if s.rsync {
        return done(s, St { ahead: if m.version.seq > s.ahead { m.version.seq } else { s.ahead }, ..s.clone() }, "ROOM.C.JOINED.delta_gap_inflight", vec![], vec![], vec![], vec![]);
    }
    done(s, St { rsync: true, ..s.clone() }, "ROOM.C.JOINED.delta_gap", vec![Tx::GetRoom { tracks: 1 }], vec![], vec![], vec![])
}

/// ROOM_EVENT 를 받았다 — version 없음 · 견주지 않는다(§[RULE-ROOM-NOTE-ORDER] · §[RULE-ROOM-VERSION-CLIENT-RULES] 종결) (받음: ROOM_EVENT)
pub fn room_event(s: &St, m: &RoomEventMsg) -> Step {
    if s.rs != RoomSt::JOINED {
        return done(s, s.clone(), "ROOM.C.any.room_event_not_joined", vec![], vec![], vec![], vec![]);
    }
    if m.r#type == RoomEv::affiliation {
        return done(s, St { rs: RoomSt::NONE, cseq: 0, cepoch: 0, rsync: false, ahead: 0, jid: 0, jsure: false, ..s.clone() }, "ROOM.C.JOINED.kicked", vec![], vec![Fx::http_abandon, Fx::app_room_gone], vec![Seam { fx: SeamFx::store_drop, to: "TRK", at: "self" }, Seam { fx: SeamFx::floor_leave, to: "FLR", at: "self" }, Seam { fx: SeamFx::aff_leave, to: "AFF", at: "self" }, Seam { fx: SeamFx::sub_drop, to: "SUB", at: "self" }, Seam { fx: SeamFx::link_room_left, to: "LINK", at: "sameServer" }, Seam { fx: SeamFx::pub_room_gone, to: "PUB", at: "sameRoom" }], vec![]);
    }
    if s.rsync {
        return done(s, s.clone(), "ROOM.C.JOINED.sync_required_inflight", vec![], vec![], vec![], vec![]);
    }
    done(s, St { rsync: true, ..s.clone() }, "ROOM.C.JOINED.sync_required", vec![Tx::GetRoom { tracks: 1 }], vec![], vec![], vec![])
}

/// GET /rooms/{room_id}?tracks=1 응답을 받았다(§[ROOM.C.JOINED] 3 · §[ROOM.http.GET_ROOM] · §[RULE-ROOM-VERSION-CLIENT-RULES] 규칙 2) (받음: GET_ROOM)
pub fn snapshot(s: &St, m: &SnapshotMsg) -> Step {
    if s.rs != RoomSt::JOINED || !s.rsync {
        return done(s, s.clone(), "ROOM.C.any.snapshot_stale", vec![], vec![], vec![], vec![]);
    }
    if m.version.epoch != s.cepoch {
        return done(s, St { rsync: false, ahead: 0, ..s.clone() }, "ROOM.C.JOINED.snapshot_epoch", vec![], vec![], vec![Seam { fx: SeamFx::link_epoch_changed, to: "LINK", at: "sameServer" }], vec![]);
    }
    if m.version.seq < s.ahead {
        return done(s, St { cseq: if m.version.seq > s.cseq { m.version.seq } else { s.cseq }, ahead: 0, ..s.clone() }, "ROOM.C.JOINED.snapshot_short", vec![Tx::GetRoom { tracks: 1 }], vec![], vec![], vec![]);
    }
    if m.version.seq < s.cseq {
        return done(s, St { rsync: false, ahead: 0, ..s.clone() }, "ROOM.C.JOINED.snapshot_behind", vec![], vec![], vec![], vec![]);
    }
    done(s, St { cseq: m.version.seq, rsync: false, ahead: 0, ..s.clone() }, "ROOM.C.JOINED.snapshot", vec![], vec![Fx::app_members], vec![Seam { fx: SeamFx::trk_snapshot, to: "TRK", at: "self" }, Seam { fx: SeamFx::sdp_rebuild, to: "LINK", at: "sameServer" }, Seam { fx: SeamFx::ready_tracks, to: "SUB", at: "self" }], vec![])
}

/// 같은 서버 방의 입장 응답이 왔다 — 응답 affiliation.sub_rooms 가 그 서버의 진실이다(RULE-ROOM-JOIN-PROC) · listed = 이 방이 그 목록에 있다
pub fn sub_rooms(s: &St, listed: bool) -> Step {
    if s.rs == RoomSt::JOINED && !listed {
        return done(s, St { rs: RoomSt::JOINING, cseq: 0, cepoch: 0, rsync: false, ahead: 0, retry: false, jid: 0, jsure: false, ..s.clone() }, "ROOM.C.JOINED.not_in_sub_rooms", vec![Tx::RoomJoin { join_id: 0 }], vec![Fx::http_abandon], vec![Seam { fx: SeamFx::store_drop, to: "TRK", at: "self" }, Seam { fx: SeamFx::floor_leave, to: "FLR", at: "self" }, Seam { fx: SeamFx::aff_down, to: "AFF", at: "self" }, Seam { fx: SeamFx::sub_drop, to: "SUB", at: "self" }, Seam { fx: SeamFx::pub_room_gone, to: "PUB", at: "sameRoom" }], vec![]);
    }
    done(s, s.clone(), "ROOM.C.any.sub_rooms_ok", vec![], vec![], vec![], vec![])
}

/// 재구축으로 방을 내린다 — 세션 전체면 전부 · 그 서버면 그 서버 방(§[RULE-SES-REBUILD] 2 · RULE-LINK-RECOVERY-REBUILD)
pub fn down_all(s: &St) -> Step {
    if s.rs == RoomSt::LEAVING {
        return done(s, St { rs: RoomSt::NONE, cseq: 0, cepoch: 0, rsync: false, ahead: 0, retry: false, rejoin: false, jid: 0, jsure: false, ..s.clone() }, "ROOM.C.LEAVING.down", vec![], vec![Fx::http_abandon, Fx::ws_abandon, Fx::app_left], vec![Seam { fx: SeamFx::store_drop, to: "TRK", at: "self" }, Seam { fx: SeamFx::aff_leave, to: "AFF", at: "self" }, Seam { fx: SeamFx::sub_drop, to: "SUB", at: "self" }], vec![]);
    }
    if s.rs == RoomSt::JOINING && s.quit {
        return done(s, St { rs: RoomSt::LEAVING, quit: false, cseq: 0, cepoch: 0, rsync: false, ahead: 0, retry: false, rejoin: false, jid: 0, jsure: false, ..s.clone() }, "ROOM.C.JOINING.down_quit", vec![Tx::RoomLeave { join_id: 0 }], vec![Fx::http_abandon, Fx::ws_abandon], vec![Seam { fx: SeamFx::store_drop, to: "TRK", at: "self" }, Seam { fx: SeamFx::aff_leave, to: "AFF", at: "self" }, Seam { fx: SeamFx::sub_drop, to: "SUB", at: "self" }], vec![]);
    }
    done(s, St { rs: RoomSt::NONE, cseq: 0, cepoch: 0, rsync: false, ahead: 0, retry: false, rejoin: s.rs == RoomSt::JOINED || s.rs == RoomSt::JOINING, ..s.clone() }, "ROOM.C.any.down", vec![], vec![Fx::http_abandon, Fx::ws_abandon], vec![Seam { fx: SeamFx::store_drop, to: "TRK", at: "self" }, Seam { fx: SeamFx::aff_down, to: "AFF", at: "self" }, Seam { fx: SeamFx::sub_drop, to: "SUB", at: "self" }], vec![])
}

/// 응답 없이 버려진 입장 · 퇴장 요청 — 상태를 맞추고 앱에 알린다(§[RULE-TRN-REQ-ABANDON]) (바깥: 그 방의 입장 · 퇴장 요청이 기한(T-req)을 넘겼다 — 응답 없이 버린다(운반 · RULE-TRN-REQ-LIFE 5)) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn req_expired(s: &St) -> Option<Step> {
    if !(s.rs == RoomSt::JOINING && !s.retry || s.rs == RoomSt::LEAVING) {
        return None;
    }
    if s.rs == RoomSt::JOINING && s.quit {
        return Some(done(s, St { rs: RoomSt::NONE, quit: false, jid: 0, jsure: false, ..s.clone() }, "ROOM.C.JOINING.expired_quit", vec![Tx::RoomLeave { join_id: 0 }], vec![Fx::ws_abandon, Fx::app_left], vec![Seam { fx: SeamFx::aff_join_failed, to: "AFF", at: "self" }, Seam { fx: SeamFx::link_room_left, to: "LINK", at: "sameServer" }], vec![]));
    }
    if s.rs == RoomSt::JOINING {
        return Some(done(s, St { rs: RoomSt::NONE, jid: 0, jsure: false, ..s.clone() }, "ROOM.C.JOINING.expired", vec![Tx::RoomLeave { join_id: 0 }], vec![Fx::ws_abandon, Fx::app_join_failed], vec![Seam { fx: SeamFx::aff_join_failed, to: "AFF", at: "self" }, Seam { fx: SeamFx::link_room_left, to: "LINK", at: "sameServer" }], vec![]));
    }
    Some(done(s, St { rs: RoomSt::NONE, cseq: 0, cepoch: 0, rsync: false, jid: 0, jsure: false, ..s.clone() }, "ROOM.C.LEAVING.expired", vec![Tx::RoomLeave { join_id: if s.jsure { s.jid } else { 0 } }], vec![Fx::ws_abandon, Fx::app_left], vec![Seam { fx: SeamFx::store_drop, to: "TRK", at: "self" }, Seam { fx: SeamFx::link_room_left, to: "LINK", at: "sameServer" }], vec![]))
}

/// 세션이 끝났다 — 방을 전부 내리고 다시 들어갈 방 표시를 지운다(DEV-SES-FRESH-REJOIN · 앱이 다시 들어간다)
pub fn close_all(s: &St) -> Step {
    done(s, St { rs: RoomSt::NONE, cseq: 0, cepoch: 0, rsync: false, ahead: 0, retry: false, rejoin: false, jid: 0, jsure: false, ..s.clone() }, "ROOM.C.any.close_all", vec![], vec![Fx::http_abandon, Fx::ws_abandon], vec![Seam { fx: SeamFx::store_drop, to: "TRK", at: "self" }, Seam { fx: SeamFx::aff_leave, to: "AFF", at: "self" }, Seam { fx: SeamFx::sub_drop, to: "SUB", at: "self" }], vec![])
}

/// 재구축 — 내렸던 방에 다시 들어간다(§[RULE-SES-REBUILD] 3 · RULE-LINK-RECOVERY-REBUILD) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn rejoin(s: &St) -> Option<Step> {
    if !(s.rs == RoomSt::NONE && s.rejoin) {
        return None;
    }
    Some(done(s, St { rs: RoomSt::JOINING, cseq: 0, cepoch: 0, rsync: false, ahead: 0, retry: false, rejoin: false, jsure: false, ..s.clone() }, "ROOM.C.NONE.rejoin", vec![Tx::RoomJoin { join_id: s.jid }], vec![], vec![], vec![]))
}

/// RESUME 성공 — 응답 스냅샷으로 방 보관본을 다시 맞춘다(§[RULE-SES-RES-RESUME-OK] 1 · 2 ②) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn resume_snap(s: &St, present: bool, seq: i64, epoch: i64) -> Option<Step> {
    if !(s.rs == RoomSt::JOINED) {
        return None;
    }
    if !present {
        return Some(done(s, St { rs: RoomSt::NONE, cseq: 0, cepoch: 0, rsync: false, ahead: 0, retry: false, jid: 0, jsure: false, ..s.clone() }, "ROOM.C.JOINED.resume_gone", vec![], vec![Fx::http_abandon, Fx::app_room_gone], vec![Seam { fx: SeamFx::store_drop, to: "TRK", at: "self" }, Seam { fx: SeamFx::floor_leave, to: "FLR", at: "self" }, Seam { fx: SeamFx::aff_leave, to: "AFF", at: "self" }, Seam { fx: SeamFx::sub_drop, to: "SUB", at: "self" }, Seam { fx: SeamFx::link_room_left, to: "LINK", at: "sameServer" }, Seam { fx: SeamFx::pub_room_gone, to: "PUB", at: "sameRoom" }], vec![]));
    }
    if epoch != s.cepoch {
        return Some(done(s, s.clone(), "ROOM.C.JOINED.resume_epoch", vec![], vec![], vec![Seam { fx: SeamFx::link_epoch_changed, to: "LINK", at: "sameServer" }], vec![]));
    }
    Some(done(s, St { cseq: seq, ..s.clone() }, "ROOM.C.JOINED.resume_snapshot", vec![], vec![Fx::app_members], vec![Seam { fx: SeamFx::trk_snapshot, to: "TRK", at: "self" }, Seam { fx: SeamFx::link_table, to: "LINK", at: "sameServer" }], vec![]))
}

/// 벡터 시험 — TS 판 입구 이름과 JSON 인자로 부른다(oxpeer 1층 · TS 판 재생이 찾은 호출을 Rust 판에 넣어 맞댄다)
#[cfg(feature = "serde")]
pub fn call_json(entry: &str, s: &St, a: &[serde_json::Value]) -> Result<Option<Step>, String> {
    fn arg<T: serde::de::DeserializeOwned>(a: &[serde_json::Value], i: usize) -> Result<T, String> {
        serde_json::from_value(a.get(i).cloned().unwrap_or(serde_json::Value::Null)).map_err(|e| format!("인자 {i}: {e}"))
    }
    Ok(match entry {
        "join" => join(s),
        "joinRes" => join_res(s, &arg::<JoinResMsg>(a, 0)?, arg::<PcMode>(a, 1)?),
        "retryDue" => retry_due(s),
        "leave" => leave(s),
        "leaveRes" => Some(leave_res(s, &arg::<LeaveResMsg>(a, 0)?)),
        "delta" => Some(delta(s, &arg::<DeltaMsg>(a, 0)?, arg::<bool>(a, 1)?)),
        "roomEvent" => Some(room_event(s, &arg::<RoomEventMsg>(a, 0)?)),
        "snapshot" => Some(snapshot(s, &arg::<SnapshotMsg>(a, 0)?)),
        "subRooms" => Some(sub_rooms(s, arg::<bool>(a, 0)?)),
        "downAll" => Some(down_all(s)),
        "reqExpired" => req_expired(s),
        "closeAll" => Some(close_all(s)),
        "rejoin" => rejoin(s),
        "resumeSnap" => resume_snap(s, arg::<bool>(a, 0)?, arg::<i64>(a, 1)?, arg::<i64>(a, 2)?),
        _ => return Err(format!("모르는 입구 {entry}")),
    })
}
