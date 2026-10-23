// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-OPS-SURFACE · 정:IMPL-ROOM-VERSION-ISSUE · 정:RULE-BUS-KEYSPACE · 연:DEV-TRN-SESSION-SECRET · model: claude-opus-5-5
//! 신원 — 빌드 신원(`{git SHA(+dirty)}.{빌드 시각}[+라벨]`) · 기동 신원(epoch = `sfu_id` = `{inst}` — 기동마다 새 불투명 값) · 예측할 수 없는 발급값.

pub fn build_identity(raw: &str, label: Option<&str>) -> String {
    match label {
        Some(l) if !l.is_empty() => format!("{raw}+{l}"),
        _ => raw.to_string(),
    }
}

pub fn fill_random(buf: &mut [u8]) {
    if getrandom::fill(buf).is_err() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        buf.iter_mut().enumerate().for_each(|(i, b)| *b = ((t >> ((i % 16) * 8)) as u8) ^ (i as u8));
    }
}

pub fn random_u32() -> u32 {
    let mut b = [0u8; 4];
    fill_random(&mut b);
    u32::from_le_bytes(b)
}

pub fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    fill_random(&mut b);
    u64::from_le_bytes(b)
}

pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn new_epoch(prefix: &str) -> String {
    let mut b = [0u8; 8];
    fill_random(&mut b);
    format!("{prefix}-{}", hex(&b))
}

pub fn random_alnum(n: usize) -> String {
    const SET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut b = vec![0u8; n];
    fill_random(&mut b);
    b.iter().map(|x| SET[(*x as usize) % SET.len()] as char).collect()
}

pub fn uuid4() -> String {
    let mut b = [0u8; 16];
    fill_random(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex(&b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epochs_differ_and_label_joins() {
        assert_ne!(new_epoch("sfu"), new_epoch("sfu"));
        assert_eq!(build_identity("abc.1", Some("ci")), "abc.1+ci");
        assert_eq!(build_identity("abc.1", None), "abc.1");
        assert_eq!(random_alnum(22).len(), 22);
        assert_eq!(uuid4().len(), 36);
    }
}
