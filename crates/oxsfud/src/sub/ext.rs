// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-SUB-SUBSCRIBER-TABLE 정:REJ-SUB-EXT-SWAP 정:CAND-SUB-MID-EXT-ALWAYS 정:RULE-CONC-SANS-IO 연:RULE-LINK-EGRESS-REWRITE 연:RULE-LINK-SDES-MID-BOTH · model: claude-opus-5-5
//! egress 헤더 확장 다시 짓기 — 들어온 영역을 버리고 구독자 표의 ID 로 mid · 음량 · abs-send-time · transport-cc 를 새로 쓴다(RFC 8285 · 꼴은 표로 정한다).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExtTable {
    pub mid: Option<u8>,
    pub audio_level: Option<u8>,
    pub abs_send_time: Option<u8>,
    pub twcc: Option<u8>,
}

impl ExtTable {
    pub fn one_byte(&self) -> bool {
        [self.mid, self.audio_level, self.abs_send_time, self.twcc].iter().flatten().all(|id| (1..=14).contains(id))
    }

    pub fn is_empty(&self) -> bool {
        self.mid.is_none() && self.audio_level.is_none() && self.abs_send_time.is_none() && self.twcc.is_none()
    }
}

pub fn abs_send_time(t_us: u64) -> u32 {
    let v = ((t_us % 64_000_000) as u128 * (1u128 << 18)) / 1_000_000;
    (v as u32) & 0x00ff_ffff
}

pub fn build(table: &ExtTable, mid: Option<&str>, audio_level: Option<u8>, t_us: u64, twcc_counter: u64, out: &mut Vec<u8>) {
    let one = table.one_byte();
    let start = out.len();
    out.extend_from_slice(if one { &[0xbe, 0xde, 0, 0] } else { &[0x10, 0x00, 0, 0] });
    let put =|out: &mut Vec<u8>, id: u8, data: &[u8]| {
        if one {
            out.push((id << 4) | ((data.len() as u8).saturating_sub(1) & 0x0f));
        } else {
            out.push(id);
            out.push(data.len() as u8);
        }
        out.extend_from_slice(data);
    };
    if let (Some(id), Some(m)) = (table.mid, mid) {
        let b = m.as_bytes();
        put(out, id, &b[..b.len().min(16)]);
    }
    if let (Some(id), Some(l)) = (table.audio_level, audio_level) {
        put(out, id, &[l]);
    }
    if let Some(id) = table.abs_send_time {
        let v = abs_send_time(t_us);
        put(out, id, &[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
    }
    if let Some(id) = table.twcc {
        let s = (twcc_counter % 65_536) as u16;
        put(out, id, &s.to_be_bytes());
    }
    while !(out.len() - start).is_multiple_of(4) {
        out.push(0);
    }
    let words = ((out.len() - start - 4) / 4) as u16;
    out[start + 2..start + 4].copy_from_slice(&words.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_byte_shape() {
        let t = ExtTable { mid: Some(1), audio_level: Some(4), abs_send_time: Some(5), twcc: Some(6) };
        let mut v = Vec::new();
        build(&t, Some("3"), Some(0x9f), 1_000_000, 7, &mut v);
        assert_eq!(v, vec![0xbe, 0xde, 0, 3, 0x10, 0x33, 0x40, 0x9f, 0x52, 0x04, 0, 0, 0x61, 0, 7, 0]);
    }
}
