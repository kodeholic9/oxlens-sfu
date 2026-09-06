// author: kodeholic (powered by Claude)
//! B 평면 — 정§15. tonic 생성물 + B 안에서만 쓰는 내부 op 번호 공간(클라 wire 에 절대 나가지 않는다).

pub mod pb {
    tonic::include_proto!("oxlens.b.v1");
}

pub use pb::sfu_service_client::SfuServiceClient;
pub use pb::sfu_service_server::{SfuService, SfuServiceServer};
pub use pb::{Envelope, SubscribeRequest};

/// hub↔sfud 내부 op — 연§6 의 16 과 겹치지 않는 `0x0Fxx`. HTTP 대행(연§5-3~5-5)과 배치 학습(정§15-1).
pub mod iop {
    /// hub→sfud `{}` → `{ rooms:[…] }` (연§5-3 형).
    pub const ROOM_LIST: u16 = 0x0F01;
    /// hub→sfud `{ room_id, name, capacity, unused_ttl_secs?, departure_ttl_secs? }` → `{ room_id, name, capacity, created_at }`.
    pub const ROOM_CREATE: u16 = 0x0F02;
    /// hub→sfud `{ room_id, tracks: bool }` (envelope `user_id` 가 있으면 그 사람이 입장한 방일 때만 `mid`) → 연§5-5 형.
    pub const ROOM_GET: u16 = 0x0F03;
    /// hub→sfud `{}` → 정§16-2 관측 스냅샷. ★계수를 읽는 자리 — 없으면 계수가 곧 조용한 drop 이다.
    pub const SFU_STATS: u16 = 0x0F04;
    /// sfud→hub `{ type: "created" | "destroyed", room_id }` — 배치 학습·해제.
    pub const ROOM_LIFECYCLE: u16 = 0x0F10;

    pub fn is_internal(op: u16) -> bool {
        (0x0F00..=0x0FFF).contains(&op)
    }
}
