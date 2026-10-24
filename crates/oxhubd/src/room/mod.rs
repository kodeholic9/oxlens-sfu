// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-JOIN · 정:IMPL-ROOM-RESYNC · 정:IMPL-ROOM-DERIVED · 정:IMPL-SES-ADMISSION · 정:RULE-BUS-PLACEMENT · 정:RULE-OPS-SURFACE · 정:CAND-ROOM-CREATE-WAITS-TOKEN · 정:CAND-ROOM-CORS-MAX-AGE · 연:ROOM.http.CREATE_ROOM · 연:RULE-ROOM-CREATE-PROC · 연:RULE-TRN-CORS · 연:RULE-ERR-FAILURE-FORM · model: claude-opus-5-5
//! 영역 ROOM(게이트웨이 쪽) — 방 생성 검사 · 배치(HRW, 후보 = node 토큰 전량) · 입장 수용 한도 · 클라 경로의 CORS. 방 상태는 미디어 서버가 든다(그림자 금지).

use std::sync::Mutex;

use common::policy::Policy;
use oxsig::{Code, CreateRoomReq, Failure, MediaCodecs};

pub const CAPACITY_MAX: u32 = 1000;
pub const DEFAULT_AUDIO: &str = "opus";
pub const DEFAULT_VIDEO: &str = "H264";
pub const CORS_MAX_AGE_SECS: u32 = 600;
pub const CORS_HEADERS: &str = "authorization, content-type, x-oxlens-session";
pub const CREATE_WAIT_MS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub room_id: String,
    pub capacity: u32,
    pub unused_ttl_secs: u32,
    pub departure_ttl_secs: u32,
    pub req: CreateRoomReq,
}

pub fn prepare_create(mut req: CreateRoomReq, policy: &Policy, fresh_id: impl FnOnce() -> String) -> Result<Prepared, Failure> {
    if req.name.is_empty() {
        return Err(Code::MissingField.failure_with("name", None));
    }
    let capacity = req.capacity.unwrap_or(policy.hub.room_default_capacity.min(CAPACITY_MAX));
    if capacity == 0 || capacity > CAPACITY_MAX {
        return Err(Code::InvalidPayload.failure_with(format!("capacity {capacity} — 1~{CAPACITY_MAX}"), None));
    }
    let room_id = match req.room_id.as_deref() {
        None | Some("") => fresh_id(),
        Some(r) if oxsig::room_id_ok(r) => r.to_string(),
        Some(r) => return Err(Code::InvalidPayload.failure_with(format!("room_id {r:?} — 255바이트 이하 · `/` `*` `$` `?` `#` 금지"), None)),
    };
    let codecs = req.media_codecs.clone().unwrap_or(MediaCodecs { audio: None, video: None });
    req.media_codecs = Some(MediaCodecs {
        audio: Some(codecs.audio.filter(|a| !a.is_empty()).unwrap_or_else(|| DEFAULT_AUDIO.into())),
        video: Some(codecs.video.filter(|v| !v.is_empty()).unwrap_or_else(|| DEFAULT_VIDEO.into())),
    });
    req.room_id = Some(room_id.clone());
    Ok(Prepared {
        room_id,
        capacity,
        unused_ttl_secs: req.unused_ttl_secs.unwrap_or(policy.hub.room_unused_ttl_secs),
        departure_ttl_secs: req.departure_ttl_secs.unwrap_or(policy.hub.room_departure_ttl_secs),
        req,
    })
}

pub fn place(room_id: &str, candidates: &[String]) -> Option<String> {
    common::bus::placement::place(room_id, candidates.iter().map(String::as_str)).map(str::to_string)
}

pub struct Admission {
    per_sec: u32,
    retry_ms: u32,
    window: Mutex<(u64, u32)>,
}

impl Admission {
    pub fn new(per_sec: u32, retry_ms: u32) -> Admission {
        Admission { per_sec, retry_ms, window: Mutex::new((0, 0)) }
    }

    pub fn admit(&self, now_ms: u64) -> Result<(), u64> {
        if self.per_sec == 0 {
            return Ok(());
        }
        let sec = now_ms / 1000;
        let mut w = self.window.lock().unwrap_or_else(|p| p.into_inner());
        if w.0 != sec {
            w.0 = sec;
            w.1 = 0;
        }
        if w.1 >= self.per_sec {
            let base = u64::from(self.retry_ms);
            let jitter = u64::from_str_radix(&common::identity::fresh_hex(4), 16).unwrap_or(0) % base.max(1);
            return Err(base + jitter);
        }
        w.1 += 1;
        Ok(())
    }
}

pub fn cors(origin: Option<&str>, allowed: &[String]) -> Vec<(&'static str, String)> {
    if allowed.is_empty() {
        return Vec::new();
    }
    let allow = if allowed.iter().any(|a| a == "*") {
        "*".to_string()
    } else {
        match origin {
            Some(o) if allowed.iter().any(|a| a == o) => o.to_string(),
            _ => return Vec::new(),
        }
    };
    vec![
        ("access-control-allow-origin", allow),
        ("access-control-allow-headers", CORS_HEADERS.to_string()),
        ("access-control-allow-methods", "GET, POST, OPTIONS".to_string()),
        ("access-control-max-age", CORS_MAX_AGE_SECS.to_string()),
        ("vary", "origin".to_string()),
    ]
}

pub fn ws_origin_ok(origin: Option<&str>, host: Option<&str>, allowed: &[String]) -> bool {
    let Some(o) = origin else { return true };
    if allowed.iter().any(|a| a == "*" || a == o) {
        return true;
    }
    match host {
        Some(h) => o.trim_start_matches("https://").trim_start_matches("http://") == h,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(name: &str) -> CreateRoomReq {
        CreateRoomReq { room_id: None, name: name.into(), capacity: None, unused_ttl_secs: None, departure_ttl_secs: None, media_codecs: None }
    }

    #[test]
    fn create_defaults_and_rejections() {
        let p = Policy::default();
        let ok = prepare_create(req("n"), &p, || "u-1".into()).expect("ok");
        assert_eq!(ok.room_id, "u-1");
        assert_eq!((ok.unused_ttl_secs, ok.departure_ttl_secs), (0, 0), "정책값 0 = 영구(상용 기본)");
        assert_eq!(ok.req.media_codecs.as_ref().and_then(|m| m.video.clone()).as_deref(), Some("H264"));
        assert_eq!(prepare_create(req(""), &p, String::new).map_err(|f| f.code), Err(1003));
        let big = CreateRoomReq { capacity: Some(1001), ..req("n") };
        assert_eq!(prepare_create(big, &p, String::new).map_err(|f| f.code), Err(1002));
        let bad = CreateRoomReq { room_id: Some("a/b".into()), ..req("n") };
        assert_eq!(prepare_create(bad, &p, String::new).map_err(|f| f.code), Err(1002), "S-sym-RULE-BUS-KEYSPACE-4 금지 문자 방 id 는 1002");
    }

    #[test]
    fn admission_window_and_jitter() {
        let a = Admission::new(2, 100);
        assert!(a.admit(1000).is_ok());
        assert!(a.admit(1500).is_ok());
        let r = a.admit(1999).expect_err("over");
        assert!((100..200).contains(&r), "SV-CODE-4009 retry_after_ms 는 [기준, 2×기준)");
        assert!(a.admit(2000).is_ok());
        assert!(Admission::new(0, 100).admit(0).is_ok());
    }

    #[test]
    fn cors_rules() {
        assert!(cors(Some("https://a"), &[]).is_empty());
        assert_eq!(cors(Some("https://a"), &["*".to_string()])[0].1, "*");
        assert!(cors(Some("https://b"), &["https://a".to_string()]).is_empty());
        assert!(ws_origin_ok(None, None, &[]));
        assert!(!ws_origin_ok(Some("https://evil"), Some("srv"), &["https://a".to_string()]));
        assert!(ws_origin_ok(Some("https://srv"), Some("srv"), &[]));
    }
}
