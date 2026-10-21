// author: kodeholic (powered by Claude)
// spec: v1.2 @5f1b3b9 · 정:RULE-BUS-REQUEST 정:RULE-BUS-EVENTS 정:IMPL-SES-DATA 정:IMPL-SES-HANDOVER 정:RULE-OPS-SURFACE · model: claude-opus-5-5
//! B 평면 봉투 — hub ↔ sfud(로컬 gRPC Handle) · hub ↔ hub(zenoh `q/node`) · sfud → hub 이벤트(zenoh `ev`).

use serde::{Deserialize, Serialize};

use crate::wire::{
    CreateRoomReq, CreateRoomRes, GetRoomRes, MlineTable, PcMode, Permission, Publication, RoomSnapshot, RoomSummary,
    Version,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Home {
    pub node: String,
    pub inst: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Caller {
    pub user_id: String,
    pub session_id: String,
    pub pc_mode: PcMode,
    pub participant_type: u8,
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub permission: Permission,
    pub home: Home,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndPath {
    Window,
    Cut,
    Leave,
    Evict,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UnitReq {
    Hello,
    RoomOp { caller: Caller, op: u16, body: String },
    SessionEnd { user_id: String, session_id: String, path: EndPath },
    Resume { user_id: String, session_id: String },
    CreateRoom { req: CreateRoomReq, capacity: u32, unused_ttl_secs: u32, departure_ttl_secs: u32 },
    GetRoom { room_id: String, viewer: Option<String>, tracks: bool },
    ListRooms,
    Admin(AdminReq),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AdminReq {
    Rooms,
    RoomSnapshot { room_id: String },
    Users,
    Drops,
    Reap { room_id: String, user_id: String },
    Destroy { room_id: String, if_version: Option<String> },
    LogLevel { level: String, ttl_secs: Option<u64> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub epoch: String,
    pub build: String,
    pub node: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumePart {
    pub snapshot: std::collections::BTreeMap<String, RoomSnapshot>,
    pub publications: Vec<Publication>,
    pub mline_tables: Vec<MlineTable>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminRoom {
    pub room_id: String,
    pub members: u32,
    pub member_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminLink {
    pub sfu_id: String,
    pub ufrag: String,
    pub state: String,
    pub since: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminUser {
    pub session_id: String,
    pub user_id: String,
    pub links: Vec<AdminLink>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReapRes {
    pub room_id: String,
    pub user_id: String,
    pub left: bool,
    pub version: Version,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DestroyRes {
    pub room_id: String,
    pub destroyed: bool,
    pub notified: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UnitRes {
    Hello(Hello),
    Frame { ok: bool, body: String },
    Done,
    Resume(ResumePart),
    Created(CreateRoomRes),
    Room(Box<GetRoomRes>),
    Rooms(Vec<RoomSummary>),
    AdminRooms(Vec<AdminRoom>),
    AdminJson(serde_json::Value),
    AdminUsers(Vec<AdminUser>),
    Reaped(ReapRes),
    Destroyed(DestroyRes),
    Fail { code: u16, message: String, details: Option<serde_json::Value> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitEvent {
    pub op: u16,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionData {
    pub session_id: String,
    pub user_id: String,
    pub pc_mode: PcMode,
    pub participant_type: u8,
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub permission: Permission,
    pub units: Vec<String>,
    pub rooms: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeReq {
    Unit { req: UnitReq },
    TakeSession { session_id: String },
    DropUser { user_id: String, code: u16, except_session: Option<String> },
    Bus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeRes {
    Unit(UnitRes),
    Session(Option<SessionData>),
    Cut(Vec<String>),
    Bus(serde_json::Value),
    Fail { code: u16, message: String },
}

pub fn encode<T: Serialize>(v: &T) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_default()
}

pub fn decode<T: serde::de::DeserializeOwned>(b: &[u8]) -> Result<T, serde_json::Error> {
    serde_json::from_slice(b)
}
