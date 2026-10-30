// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CFG-THREE-PLACES · 정:RULE-BUS-KEYSPACE · 운영:RULE-BOOT-ARGS · 운영:RULE-LOG-SENDER · model: claude-opus-5-5
//! hub · sfud 가 함께 쓰는 것 — 값의 자리 셋(정책 · 시스템 파일 · 인자) · 빌드 신원 · 로그 줄 · 버스 키.

pub mod args;
pub mod envelope;
pub mod ident;
pub mod keyspace;
pub mod log;
pub mod policy;
pub mod system;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    if getrandom::fill(&mut buf).is_ok() {
        return buf;
    }
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    (0..n).map(|i| ((t >> ((i % 16) * 8)) as u8) ^ (i as u8).wrapping_mul(131)).collect()
}

pub fn random_hex(bytes: usize) -> String {
    random_bytes(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
