// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:REJ-SUB-EXT-SWAP · 정:RULE-CONC-SANS-IO · 정:CAND-SUB-MID-EXT-ALWAYS · model: claude-opus-5-5
//! egress 헤더 확장 다시 짓기 — 들어온 영역은 버리고 구독자 표의 번호로 mid · 음량 · abs-send-time · transport-cc 만. 꼴은 표로 정한다(ID 전부 1~14 면 one-byte).

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
}

pub fn abs_send_time(t_us: u64) -> u32 {
    (((t_us % 64_000_000) as u128 * (1u128 << 18) / 1_000_000) as u32) & 0x00ff_ffff
}

pub struct ExtInput<'a> {
    pub mid: &'a [u8],
    pub audio_level: Option<u8>,
    pub t_us: u64,
    pub twcc_seq: Option<u16>,
}

pub fn build(table: &ExtTable, inp: &ExtInput<'_>, out: &mut Vec<u8>) -> usize {
    let start = out.len();
    let one = table.one_byte();
    out.extend_from_slice(if one { &[0xbe, 0xde, 0, 0] } else { &[0x10, 0x00, 0, 0] });
    let body = out.len();
    let put = |out: &mut Vec<u8>, id: u8, v: &[u8]| {
        if v.is_empty() {
            return;
        }
        if one {
            out.push((id << 4) | ((v.len() as u8 - 1) & 0x0f));
        } else {
            out.push(id);
            out.push(v.len() as u8);
        }
        out.extend_from_slice(v);
    };
    if let Some(id) = table.mid
        && !inp.mid.is_empty()
    {
        put(out, id, inp.mid);
    }
    if let (Some(id), Some(l)) = (table.audio_level, inp.audio_level) {
        put(out, id, &[l]);
    }
    if let Some(id) = table.abs_send_time {
        let v = abs_send_time(inp.t_us).to_be_bytes();
        put(out, id, &v[1..4]);
    }
    if let (Some(id), Some(s)) = (table.twcc, inp.twcc_seq) {
        put(out, id, &s.to_be_bytes());
    }
    while !(out.len() - body).is_multiple_of(4) {
        out.push(0);
    }
    let words = ((out.len() - body) / 4) as u16;
    if words == 0 {
        out.truncate(start);
        return 0;
    }
    out[body - 2..body].copy_from_slice(&words.to_be_bytes());
    out.len() - start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abs_time_one_second() {
        assert_eq!(abs_send_time(1_000_000), 0x040000);
    }
}
