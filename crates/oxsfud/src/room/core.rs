// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-JOIN · 정:IMPL-ROOM-PERMISSION · 정:RULE-GC-FOUR · 연:RULE-ROOM-JOIN-PROC · 연:RULE-ROOM-JOIN-FAIL · 연:RULE-ROOM-MAX-100 · 연:RULE-ROOM-PERMISSION-MAP · 연:ROOM.S.join · 연:ROOM.S.join_again · 연:ROOM.S.join_hidden · 연:ROOM.S.join_3001 · 연:ROOM.S.join_4001 · 연:ROOM.S.perm_absent · 정:CAND-ROOM-TTL-PERMANENT-MARK · model: claude-opus-5-5
//! 방 판정(순수) — 입장 차례(재입장 축출 뒤 값으로 정원 · 서버당 방 수) · 빈 방 TTL(생성 시각 기준 · 최초 관측 tick 은 판정 안 함 · 이상) · 권한 오버라이드(멱등 · 기본값이면 항목을 지운다).

use std::collections::HashMap;

use oxsig::{Code, Failure, Permission};
use serde_json::json;

use super::MAX_ROOMS_PER_SERVER;

pub const TTL_PERMANENT: u32 = u32::MAX;

pub fn check_capacity(user_count: u32, capacity: u32, hidden: bool) -> Result<(), Failure> {
    if !hidden && user_count >= capacity {
        return Err(Code::RoomFull.failure_with("정원", Some(json!({"capacity": capacity}))));
    }
    Ok(())
}

pub fn check_rooms(current: usize) -> Result<(), Failure> {
    if current >= MAX_ROOMS_PER_SERVER {
        return Err(Code::ListenLimit.failure_with("서버당 방 수", Some(json!({"limit": MAX_ROOMS_PER_SERVER, "current": current}))));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    Keep,
    MarkEmpty,
    Expire,
}

pub fn sweep(registered: usize, ever_joined: bool, empty_since: u64, unused_ttl_secs: u32, departure_ttl_secs: u32, now_ms: u64) -> Sweep {
    if registered > 0 {
        return Sweep::Keep;
    }
    if empty_since == 0 {
        return Sweep::MarkEmpty;
    }
    let ttl = if ever_joined { departure_ttl_secs } else { unused_ttl_secs };
    if ttl == TTL_PERMANENT {
        return Sweep::Keep;
    }
    if now_ms.saturating_sub(empty_since) >= ttl as u64 * 1000 {
        return Sweep::Expire;
    }
    Sweep::Keep
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermChange {
    pub before: Permission,
    pub after: Permission,
}

impl PermChange {
    pub fn lowered(&self) -> Vec<crate::r#pub::core::Bit> {
        use crate::r#pub::core::Bit;
        [Bit::Audio, Bit::Video, Bit::Screen, Bit::Floor].into_iter().filter(|b| Bit::of(&self.before, *b) && !Bit::of(&self.after, *b)).collect()
    }

    pub fn raised(&self) -> Vec<crate::r#pub::core::Bit> {
        use crate::r#pub::core::Bit;
        [Bit::Audio, Bit::Video, Bit::Screen, Bit::Floor].into_iter().filter(|b| !Bit::of(&self.before, *b) && Bit::of(&self.after, *b)).collect()
    }
}

pub fn set_override(overrides: &mut HashMap<String, Permission>, user: &str, current: Permission, next: Permission) -> Option<PermChange> {
    if current == next {
        return None;
    }
    if next.is_default() {
        overrides.remove(user);
    } else {
        overrides.insert(user.to_string(), next);
    }
    Some(PermChange { before: current, after: next })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_rules() {
        assert_eq!(sweep(1, true, 5, 0, 0, 10), Sweep::Keep, "등록 기준 — hidden 도 사람");
        assert_eq!(sweep(0, true, 0, 30, 5, 100), Sweep::MarkEmpty, "S-sym-IMPL-ROOM-LIFE-1 최초 관측 tick 은 만료 판정을 안 한다");
        assert_eq!(sweep(0, true, 100, 30, 5, 5099), Sweep::Keep);
        assert_eq!(sweep(0, true, 100, 30, 5, 5100), Sweep::Expire, "이상");
        assert_eq!(sweep(0, false, 100, 30, 5, 5100), Sweep::Keep, "unused 는 생성 시각 기준 30s");
        assert_eq!(sweep(0, false, 100, TTL_PERMANENT, TTL_PERMANENT, u64::MAX), Sweep::Keep, "정책값 0 = 영구(봉투에 영구 표지)");
        assert_eq!(sweep(0, true, 100, 30, 0, 100), Sweep::Expire, "요청의 0초 TTL 은 다음 sweep 에서 즉시");
    }

    #[test]
    fn override_idempotent_and_default_erases() {
        let mut m = HashMap::new();
        let low = Permission { floor_request: false, ..Permission::ALL };
        assert!(set_override(&mut m, "u", Permission::ALL, Permission::ALL).is_none(), "S-sym-IMPL-ROOM-PERMISSION-1 지금 값과 같으면 아무것도 안 한다");
        let c = set_override(&mut m, "u", Permission::ALL, low).expect("change");
        assert_eq!(c.lowered().len(), 1);
        assert!(m.contains_key("u"));
        set_override(&mut m, "u", low, Permission::ALL);
        assert!(!m.contains_key("u"), "S-sym-IMPL-ROOM-PERMISSION-2 기본과 같은 값은 보관하지 않는다");
    }

    #[test]
    fn join_limits() {
        assert_eq!(check_capacity(2, 2, false).map_err(|f| f.code), Err(Code::RoomFull.number()));
        assert!(check_capacity(2, 2, true).is_ok(), "투명에는 상한 축이 없다");
        assert_eq!(check_rooms(100).map_err(|f| f.code), Err(Code::ListenLimit.number()));
    }
}
