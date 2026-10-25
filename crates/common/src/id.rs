// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 연:DEV-TRN-SESSION-SECRET 정:IMPL-ROOM-VERSION-ISSUE 정:RULE-BUS-KEYSPACE · model: claude-opus-5-5
//! 예측할 수 없는 값 — `session_id` · 기동 신원(`epoch` = `{inst}`) · 자동 방 id.

pub fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    if getrandom::fill(&mut b).is_err() {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (i, x) in b.iter_mut().enumerate() {
            *x = (t >> ((i % 16) * 8)) as u8 ^ (i as u8).wrapping_mul(131);
        }
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn session_id() -> String {
    random_hex(24)
}

pub fn instance() -> String {
    random_hex(6)
}

pub fn uuid4() -> String {
    let h = random_hex(16);
    let mut c: Vec<char> = h.chars().collect();
    c[12] = '4';
    let v = c[16].to_digit(16).unwrap_or(0);
    c[16] = std::char::from_digit((v & 0x3) | 0x8, 16).unwrap_or('8');
    let s: String = c.into_iter().collect();
    format!(
        "{}-{}-{}-{}-{}",
        &s[0..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        assert_eq!(session_id().len(), 48);
        assert_ne!(session_id(), session_id());
        let u = uuid4();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
    }
}
