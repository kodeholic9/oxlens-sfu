// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-REQUEST · 정:RULE-BUS-EVENTS · 정:IMPL-BUS-HOME-LOST · 정:CAND-BUS-HANDLE-SHAPE · 정:CAND-BUS-NOTICE-ENVELOPE · 정:RULE-HOOK-FACTS · 정:RULE-HOOK-PERMISSION · 정:RULE-HOOK-STATUS · 연:RULE-TRN-OP-NUMBERS · 연:FLR.wire · model: claude-opus-5-5
//! node 안 Handle 과 node 사이 `q/node/{node}/call` 이 함께 나르는 봉투 · 답 · 통지 봉투(JSON) · hub 만 읽는 사실 재료(표지 0xF001 · 몸 `{type, body}` — 발언권은 MBCP 필드 이름 그대로 JSON) · 대외 권한 제어 Call · 방 상세의 발언권 요청 칸 · 대외 방 상세(`GET_ROOM` 몸 + 발언권 칸 `floor{speaker, queue, start}` — 칸 셋은 늘 있다 · 정:RULE-FLR-SNAPSHOT 꼴).

use oxsig::Failure;
use oxsig::msg::{GetRoomRes, PcMode, Permission, RoomSummary};
use serde::{Deserialize, Serialize};

pub mod handle {
    tonic::include_proto!("handle");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    Op,
    SessionGone,
    Resume,
    CreateRoom,
    GetRoom,
    ListRooms,
    Admin,
    LogLevel,
    Permission,
}

pub const OP_FACT_MATERIAL: u16 = 0xF001;

pub const FACT_CATALOG: [&str; 13] = [
    "room_created",
    "room_closed",
    "participant_joined",
    "participant_left",
    "track_published",
    "track_unpublished",
    "track_state",
    "permission_changed",
    "floor_granted",
    "floor_released",
    "floor_revoked",
    "recording_started",
    "recording_ended",
];

pub fn facts_item_ok(s: &str) -> bool {
    s == "*" || FACT_CATALOG.contains(&s)
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RoomFloor {
    pub speaker: Option<String>,
    pub queue: Vec<String>,
    pub start: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomDetail {
    #[serde(flatten)]
    pub room: GetRoomRes,
    pub floor: RoomFloor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionCall {
    pub room_id: String,
    pub permission: Permission,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloorTaken {
    pub speaker: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<u32>>,
    pub seq: u16,
    pub room: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub talk: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloorIdle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<String>,
    pub seq: u16,
    pub room: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloorRevoke {
    pub cause: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub room: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum Material {
    RoomCreated(RoomSummary),
    RoomClosed(RoomSummary),
    RecordingStarted(RoomSummary),
    RecordingEnded(RoomSummary),
    FloorGranted(FloorTaken),
    FloorReleased(FloorIdle),
    FloorRevoked(FloorRevoke),
}

impl Material {
    pub fn name(&self) -> &'static str {
        match self {
            Material::RoomCreated(_) => "room_created",
            Material::RoomClosed(_) => "room_closed",
            Material::RecordingStarted(_) => "recording_started",
            Material::RecordingEnded(_) => "recording_ended",
            Material::FloorGranted(_) => "floor_granted",
            Material::FloorReleased(_) => "floor_released",
            Material::FloorRevoked(_) => "floor_revoked",
        }
    }

    pub fn notice(&self, room_id: &str, sfu_id: &str) -> Notice {
        Notice {
            op: OP_FACT_MATERIAL,
            body: serde_json::to_value(self).unwrap_or_default(),
            exclude: Vec::new(),
            room_id: Some(room_id.to_string()),
            sfu_id: Some(sfu_id.to_string()),
        }
    }

    pub fn of_notice(n: &Notice) -> Option<Material> {
        if n.op != OP_FACT_MATERIAL {
            return None;
        }
        serde_json::from_value(n.body.clone()).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Home {
    pub node: String,
    pub inst: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Claims {
    pub participant_type: u8,
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub kind: CallKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<u16>,
    #[serde(default)]
    pub body: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pc_mode: Option<PcMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tag: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<Home>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claims: Option<Claims>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admit_retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub floor: bool,
}

impl Envelope {
    pub fn new(kind: CallKind) -> Envelope {
        Envelope {
            kind,
            op: None,
            body: serde_json::Value::Null,
            user_id: None,
            session: None,
            sid: None,
            pc_mode: None,
            source_tag: None,
            home: None,
            claims: None,
            method: None,
            path: None,
            target_user: None,
            admit_retry_after_ms: None,
            floor: false,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> Result<Envelope, serde_json::Error> {
        serde_json::from_slice(b)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "r", rename_all = "snake_case")]
pub enum Reply {
    Ok { body: serde_json::Value },
    Fail { failure: Failure },
    Http { status: u16, body: serde_json::Value },
}

impl Reply {
    pub fn ok(body: serde_json::Value) -> Reply {
        Reply::Ok { body }
    }

    pub fn fail(f: impl Into<Failure>) -> Reply {
        Reply::Fail { failure: f.into() }
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> Result<Reply, serde_json::Error> {
        serde_json::from_slice(b)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub op: u16,
    pub body: serde_json::Value,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfu_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_roundtrip() {
        let r = Reply::fail(oxsig::Code::RoomNotFound);
        assert_eq!(Reply::decode(&r.encode()).unwrap(), r);
        let e = Envelope { op: Some(0x0201), ..Envelope::new(CallKind::Op) };
        assert_eq!(Envelope::decode(&e.encode()).unwrap(), e);
    }

    #[test]
    fn floor_flag_absent_unless_set() {
        let plain = Envelope::new(CallKind::GetRoom);
        assert!(!String::from_utf8(plain.encode()).unwrap().contains("floor"), "연:RULE-ROOM-DETAIL-NO-FLOOR — 클라 길 GetRoom 봉투에 발언권 칸 없음");
        let hook = Envelope { floor: true, ..Envelope::new(CallKind::GetRoom) };
        assert_eq!(Envelope::decode(&hook.encode()).unwrap(), hook, "정:RULE-HOOK-STATUS — 방 상세의 발언권 요청 칸");
        let perm = Envelope::new(CallKind::Permission);
        assert!(String::from_utf8(perm.encode()).unwrap().contains("\"permission\""));
    }

    #[test]
    fn room_floor_fields_always_present() {
        let v = serde_json::to_value(RoomFloor::default()).unwrap();
        assert_eq!(v, serde_json::json!({ "speaker": null, "queue": [], "start": null }), "정:RULE-FLR-SNAPSHOT 꼴 — 발언권 칸 셋은 늘 있다(비었으면 null · [] · null)");
        let f = RoomFloor { speaker: Some("u1".into()), queue: vec!["u2".into(), "u3".into()], start: Some(1) };
        assert_eq!(serde_json::from_value::<RoomFloor>(serde_json::to_value(&f).unwrap()).unwrap(), f);
    }

    #[test]
    fn material_marker_and_shape() {
        let room = RoomSummary { room_id: "r1".into(), name: "r1".into(), capacity: 10, user_count: 0, created_at: 1, rec: false, record: Vec::new() };
        let m = Material::RoomCreated(room);
        let n = m.notice("r1", "e1");
        assert_eq!(n.op, 0xF001, "정:RULE-BUS-EVENTS hub 만 읽는 갈래 — 재료 표지 0xF001");
        assert_eq!(n.body["type"], "room_created", "정:RULE-BUS-EVENTS hub 만 읽는 갈래 — 재료 몸 type 은 카탈로그 이름");
        assert_eq!(Material::of_notice(&n), Some(m));
        let g = Material::FloorGranted(FloorTaken { speaker: "u1".into(), sources: None, seq: 7, room: "r1".into(), talk: Some("t1".into()) });
        let v = serde_json::to_value(&g).unwrap();
        assert_eq!(v["body"]["talk"], "t1", "정:RULE-HOOK-FACTS — 획득 재료에 발언 id 칸");
        let i = serde_json::to_value(Material::FloorReleased(FloorIdle { prev: None, seq: 8, room: "r1".into() })).unwrap();
        assert!(i["body"].get("talk").is_none(), "정:RULE-HOOK-FACTS — 비움 재료에 발언 id 칸 없음");
        let other = Notice { op: 0xF002, body: v, exclude: Vec::new(), room_id: None, sfu_id: None };
        assert_eq!(Material::of_notice(&other), None, "정:RULE-BUS-EVENTS — 구간의 다른 번호는 재료가 아님");
        for (m, name) in [
            (Material::RoomClosed(RoomSummary { room_id: "r".into(), name: "r".into(), capacity: 1, user_count: 0, created_at: 0, rec: false, record: Vec::new() }), "room_closed"),
            (Material::FloorRevoked(FloorRevoke { cause: 1, reason: None, room: "r".into() }), "floor_revoked"),
        ] {
            assert_eq!(m.name(), name);
            assert_eq!(serde_json::to_value(&m).unwrap()["type"], name);
        }
    }
}
