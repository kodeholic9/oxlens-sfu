// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 운영:READ.http.SFUS 운영:READ.http.ROOMS 운영:READ.http.USERS 운영:READ.http.SNAPSHOT 운영:READ.http.DROPS 운영:READ.http.BUS 운영:READ.http.BUS_ALL 운영:READ.http.USER_MEDIA 운영:READ.http.USER_PROBE 운영:CTL.http.SUP_STATUS 운영:CTL.http.SUP_LOAD 운영:CTL.http.SUP_STOP 운영:CTL.http.SUP_KILL 운영:CTL.http.SUP_SHUTDOWN 운영:CTL.http.USER_CUT 운영:CTL.http.LOG_LEVEL 운영:BOOT.http.HEALTH_READY 운영:AUTH.http.HEALTH_LIVE 운영:RULE-FMT-NO-INVENT · model: claude-opus-5-5
//! hub 가 짓는 운영 응답의 꼴 — 칸 · 차례 · 있음은 운영 꼴 원천 그대로(모르는 칸은 키째 뺀다).

use std::collections::BTreeMap;

use oxsig::msg::ProbeResult;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SfuRow {
    pub unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sfu_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_state: Option<String>,
    pub live: bool,
    pub node_live: bool,
}

#[derive(Debug, Serialize)]
pub struct SfusRes {
    pub sfus: Vec<SfuRow>,
    pub supervising: bool,
}

#[derive(Debug, Serialize)]
pub struct RoomRow {
    pub room_id: String,
    pub node: String,
    pub sfu_id: String,
    pub members: u32,
    pub member_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RoomsRes {
    pub rooms: Vec<RoomRow>,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct LinkRow {
    pub sfu_id: String,
    pub ufrag: String,
    pub state: String,
    pub since: u64,
}

#[derive(Debug, Serialize)]
pub struct UserRow {
    pub session: String,
    pub user_id: String,
    pub participant_type: u8,
    pub hidden: bool,
    pub links: Vec<LinkRow>,
}

#[derive(Debug, Serialize)]
pub struct UsersRes {
    pub users: Vec<UserRow>,
    pub total: u32,
}

#[derive(Debug, Serialize)]
pub struct SnapshotRes {
    pub hub_id: String,
    pub ready: bool,
    pub build: String,
    pub supervising: bool,
    pub sfus: Vec<SfuRow>,
    pub rooms: u32,
    pub users: u32,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct DropsUnit {
    pub unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sfu_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<u32>,
    #[serde(flatten)]
    pub counts: BTreeMap<String, u64>,
}

#[derive(Debug, Serialize)]
pub struct DropsRes {
    pub units: Vec<DropsUnit>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Peer {
    pub zid: String,
    pub whatami: String,
    pub links: Vec<String>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct SeenRow {
    pub room_id: String,
    pub node: String,
    pub inst: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Pending {
    pub rooms_unplaced: Vec<String>,
    pub rooms_no_view: Vec<String>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct LastEv {
    pub at_ms: u64,
    pub kind: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct BusBody {
    pub node: String,
    pub inst: String,
    pub mode: String,
    pub namespace: String,
    pub listen: Vec<String>,
    pub connect: Vec<String>,
    pub peers: Vec<Peer>,
    pub nodes: Vec<serde_json::Map<String, serde_json::Value>>,
    pub rooms_seen: Vec<SeenRow>,
    pub pending: Pending,
    pub last_event: Option<LastEv>,
}

#[derive(Debug, Serialize)]
pub struct BusAllRow {
    pub node: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bus: Option<BusBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct BusAllRes {
    pub asked: String,
    pub nodes: Vec<BusAllRow>,
}

#[derive(Debug, Serialize)]
pub struct UserMediaRes {
    pub user_id: String,
    pub now: u64,
    pub links: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ProbeRow {
    pub session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ProbeResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ProbeRes {
    pub user_id: String,
    pub sessions: Vec<ProbeRow>,
}

#[derive(Debug, Serialize)]
pub struct SupRow {
    pub id: String,
    pub kind: String,
    pub order: u32,
    pub state: String,
    pub restarts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_retry_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epoch: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SupStatusRes {
    pub units: Vec<SupRow>,
    pub supervising: bool,
}

#[derive(Debug, Serialize)]
pub struct SupMoveRes {
    pub id: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Serialize)]
pub struct ShutdownRes {
    pub accepted: bool,
    pub order: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct CutRow {
    pub session: String,
    pub code: u16,
}

#[derive(Debug, Serialize)]
pub struct CutRes {
    pub user_id: String,
    pub cut: Vec<CutRow>,
    pub total: u32,
}

#[derive(Debug, Serialize)]
pub struct LogLevelRes {
    pub from: String,
    pub to: String,
    pub until: Option<u64>,
    pub units: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LiveRes {
    pub live: bool,
}

#[derive(Debug, Serialize)]
pub struct ReadyRes {
    pub ready: bool,
    pub build: String,
    pub down: Vec<String>,
}
