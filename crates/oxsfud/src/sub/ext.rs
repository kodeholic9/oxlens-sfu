// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:REJ-SUB-EXT-SWAP · 정:RULE-CONC-SANS-IO · 연:RULE-LINK-EGRESS-REWRITE · model: claude-opus-5-5
//! 확장 다시 짓기(sans-IO) — 구독자 확장 표로 꼴을 다운링크 SSRC 마다 고정(one-byte · two-byte) · 값 식(mid · 음량 · abs-send-time · transport-cc).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ExtTable {
    pub mid: Option<u8>,
    pub audio_level: Option<u8>,
    pub abs_send_time: Option<u8>,
    pub twcc: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    One,
    Two,
}

impl ExtTable {
    pub fn ids(&self) -> impl Iterator<Item = u8> {
        [self.mid, self.audio_level, self.abs_send_time, self.twcc].into_iter().flatten()
    }

    pub fn form(&self) -> Form {
        if self.ids().all(|i| (1..=14).contains(&i)) {
            Form::One
        } else {
            Form::Two
        }
    }
}

pub fn abs_send_time(t_us: u64) -> u32 {
    let m = (t_us % 64_000_000) as u128;
    ((m << 18) / 1_000_000) as u32 & 0x00ff_ffff
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ExtValues<'a> {
    pub mid: Option<&'a str>,
    pub audio_level: Option<u8>,
    pub t_us: u64,
    pub twcc_seq: u16,
}

fn elem(out: &mut Vec<u8>, form: Form, id: u8, data: &[u8]) {
    match form {
        Form::One => out.push((id << 4) | ((data.len() as u8).saturating_sub(1) & 0x0f)),
        Form::Two => {
            out.push(id);
            out.push(data.len() as u8);
        }
    }
    out.extend_from_slice(data);
}

pub fn build(table: &ExtTable, form: Form, v: &ExtValues<'_>, out: &mut Vec<u8>) -> usize {
    let start = out.len();
    let any = (table.mid.is_some() && v.mid.is_some()) || (table.audio_level.is_some() && v.audio_level.is_some()) || table.abs_send_time.is_some() || table.twcc.is_some();
    if !any {
        return 0;
    }
    match form {
        Form::One => out.extend_from_slice(&[0xbe, 0xde, 0, 0]),
        Form::Two => out.extend_from_slice(&[0x10, 0x00, 0, 0]),
    }
    let body = out.len();
    if let (Some(id), Some(m)) = (table.mid, v.mid) {
        elem(out, form, id, m.as_bytes());
    }
    if let (Some(id), Some(l)) = (table.audio_level, v.audio_level) {
        elem(out, form, id, &[l]);
    }
    if let Some(id) = table.abs_send_time {
        let a = abs_send_time(v.t_us);
        elem(out, form, id, &[(a >> 16) as u8, (a >> 8) as u8, a as u8]);
    }
    if let Some(id) = table.twcc {
        elem(out, form, id, &v.twcc_seq.to_be_bytes());
    }
    while !(out.len() - body).is_multiple_of(4) {
        out.push(0);
    }
    let words = ((out.len() - body) / 4) as u16;
    out[start + 2..start + 4].copy_from_slice(&words.to_be_bytes());
    out.len() - start
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub mid: Option<Vec<u8>>,
    pub rid: Option<Vec<u8>>,
    pub rrid: Option<Vec<u8>>,
    pub audio_level: Option<u8>,
    pub twcc: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InIds {
    pub mid: Option<u8>,
    pub rid: Option<u8>,
    pub rrid: Option<u8>,
    pub audio_level: Option<u8>,
    pub twcc: Option<u8>,
}

pub fn parse(profile: u16, data: &[u8], ids: &InIds) -> Parsed {
    let mut p = Parsed::default();
    let mut put = |id: u8, v: &[u8]| {
        if Some(id) == ids.mid {
            p.mid = Some(v.to_vec());
        } else if Some(id) == ids.rid {
            p.rid = Some(v.to_vec());
        } else if Some(id) == ids.rrid {
            p.rrid = Some(v.to_vec());
        } else if Some(id) == ids.audio_level {
            p.audio_level = v.first().copied();
        } else if Some(id) == ids.twcc && v.len() >= 2 {
            p.twcc = Some(u16::from_be_bytes([v[0], v[1]]));
        }
    };
    let mut i = 0;
    if profile == 0xbede {
        while i < data.len() {
            let b = data[i];
            if b == 0 {
                i += 1;
                continue;
            }
            let id = b >> 4;
            let len = (b & 0x0f) as usize + 1;
            if id == 15 || i + 1 + len > data.len() {
                break;
            }
            put(id, &data[i + 1..i + 1 + len]);
            i += 1 + len;
        }
    } else if profile & 0xfff0 == 0x1000 {
        while i + 1 < data.len() {
            let id = data[i];
            if id == 0 {
                i += 1;
                continue;
            }
            let len = data[i + 1] as usize;
            if i + 2 + len > data.len() {
                break;
            }
            put(id, &data[i + 2..i + 2 + len]);
            i += 2 + len;
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_byte_round_trip() {
        let t = ExtTable { mid: Some(1), audio_level: Some(4), abs_send_time: Some(5), twcc: Some(6) };
        let mut out = Vec::new();
        build(&t, t.form(), &ExtValues { mid: Some("3"), audio_level: Some(159), t_us: 1_000_000, twcc_seq: 7 }, &mut out);
        let ids = InIds { mid: Some(1), audio_level: Some(4), twcc: Some(6), ..Default::default() };
        let p = parse(0xbede, &out[4..], &ids);
        assert_eq!(p.mid.as_deref(), Some(&b"3"[..]));
        assert_eq!(p.audio_level, Some(159));
        assert_eq!(p.twcc, Some(7));
    }
}
