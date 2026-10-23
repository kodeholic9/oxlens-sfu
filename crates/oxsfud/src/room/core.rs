// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-ROOM-JOIN · 정:IMPL-ROOM-PERMISSION · 연:RULE-ROOM-MAX-100 · 연:RULE-ROOM-PERMISSION-MAP · 연:RULE-ROOM-BITS-OUTLIVE · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 방 판정(순수) — 정원(이상) · 서버별 입장 방 수 · 권한 비트(오버라이드가 토큰값보다 앞선다 · 기본과 같으면 지운다) · 회수 대상.

use oxsig::err::{ErrCode, Failure};
use oxsig::room::Permission;
use serde_json::json;

pub const MAX_ROOMS: usize = 100;

pub fn capacity(hidden: bool, user_count: u32, capacity: u32) -> Result<(), Failure> {
    if !hidden && user_count >= capacity {
        return Err(Failure::new(ErrCode::RoomFull));
    }
    Ok(())
}

pub fn listen_limit(current: usize) -> Result<(), Failure> {
    if current >= MAX_ROOMS {
        return Err(Failure::new(ErrCode::ListenLimit).details(json!({"limit": MAX_ROOMS, "current": current})));
    }
    Ok(())
}

pub fn initial(token: Permission, over: Option<Permission>) -> Permission {
    over.unwrap_or(token)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Lowered {
    pub audio: bool,
    pub video: bool,
    pub screen: bool,
    pub floor: bool,
}

impl Lowered {
    pub fn any(&self) -> bool {
        self.audio || self.video || self.screen || self.floor
    }
}

pub fn lowered(old: &Permission, new: &Permission) -> Lowered {
    Lowered {
        audio: old.publish_audio && !new.publish_audio,
        video: old.publish_video && !new.publish_video,
        screen: old.publish_screen && !new.publish_screen,
        floor: old.floor_request && !new.floor_request,
    }
}

pub fn store(over: Option<Permission>) -> Option<Permission> {
    over.filter(|p| !p.is_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_is_at_least_and_hidden_free() {
        assert!(capacity(false, 2, 2).is_err());
        assert!(capacity(true, 2, 2).is_ok(), "정:IMPL-ROOM-JOIN ③");
        assert!(capacity(false, 1, 2).is_ok());
    }

    #[test]
    fn override_wins_and_default_is_dropped() {
        let low = Permission { floor_request: false, ..Permission::default() };
        assert_eq!(initial(Permission::default(), Some(low)), low);
        assert_eq!(store(Some(Permission::default())), None, "정:IMPL-ROOM-PERMISSION 기본값 되돌림");
        assert!(lowered(&Permission::default(), &low).floor);
    }
}
