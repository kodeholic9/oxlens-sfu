// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:RULE-CONC-SANS-IO · 정:CAND-SUB-MID-EXT-ALWAYS · 정:REJ-SUB-EXT-SWAP · 연:RULE-LINK-EGRESS-REWRITE · 연:COND-LINK-REBUILD · model: claude-opus-5-5
//! 확장 다시 짓기(순수) — 구독자 확장 표로 꼴(one-byte · two-byte)을 정하고 mid · 음량 · abs-send-time · transport-cc 를 새로 쓴다. 시각 · 일련번호는 인자다.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExtTable {
    pub mid: Option<u8>,
    pub audio_level: Option<u8>,
    pub abs_send_time: Option<u8>,
    pub twcc: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    One,
    Two,
}

pub const URI_MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
pub const URI_AUDIO_LEVEL: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
pub const URI_ABS_SEND_TIME: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
pub const URI_TWCC: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
pub const URI_RID: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
pub const URI_REPAIRED_RID: &str = "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id";

impl ExtTable {
    pub fn from_extmap(list: &[oxsig::types::ExtMap]) -> ExtTable {
        let find = |uri: &str| list.iter().find(|e| e.uri == uri).map(|e| e.id);
        ExtTable { mid: find(URI_MID), audio_level: find(URI_AUDIO_LEVEL), abs_send_time: find(URI_ABS_SEND_TIME), twcc: find(URI_TWCC) }
    }

    pub fn for_kind(self, kind: oxsig::types::Kind) -> ExtTable {
        match kind {
            oxsig::types::Kind::Audio => self,
            oxsig::types::Kind::Video => ExtTable { audio_level: None, ..self },
        }
    }

    pub fn form(&self) -> Form {
        let ids = [self.mid, self.audio_level, self.abs_send_time, self.twcc];
        if ids.iter().flatten().any(|i| *i > 14) { Form::Two } else { Form::One }
    }
}

pub fn abs_send_time(t_us: u64) -> [u8; 3] {
    let v = (((t_us % 64_000_000) << 18) / 1_000_000) as u32 & 0x00ff_ffff;
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

pub fn elements(table: &ExtTable, mid: Option<&str>, audio_level: Option<u8>, t_us: u64, twcc_counter: u64) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::with_capacity(4);
    if let (Some(id), Some(m)) = (table.mid, mid) {
        out.push((id, m.as_bytes().to_vec()));
    }
    if let (Some(id), Some(l)) = (table.audio_level, audio_level) {
        out.push((id, vec![l]));
    }
    if let Some(id) = table.abs_send_time {
        out.push((id, abs_send_time(t_us).to_vec()));
    }
    if let Some(id) = table.twcc {
        out.push((id, (twcc_counter as u16).to_be_bytes().to_vec()));
    }
    out
}

pub fn build(table: &ExtTable, mid: Option<&str>, audio_level: Option<u8>, t_us: u64, twcc_counter: u64) -> Vec<u8> {
    let level = audio_level.map(|l| [l]);
    let abs = abs_send_time(t_us);
    let twcc = (twcc_counter as u16).to_be_bytes();
    let els: [(Option<u8>, Option<&[u8]>); 4] = [
        (table.mid, mid.map(str::as_bytes)),
        (table.audio_level, level.as_ref().map(|l| l.as_slice())),
        (table.abs_send_time, Some(abs.as_slice())),
        (table.twcc, Some(twcc.as_slice())),
    ];
    if !els.iter().any(|(i, v)| i.is_some() && v.is_some()) {
        return Vec::new();
    }
    let form = table.form();
    let mut body = Vec::with_capacity(16);
    for (id, v) in els.iter().filter_map(|(i, v)| Some(((*i)?, (*v)?))) {
        match form {
            Form::One => {
                body.push((id << 4) | ((v.len() as u8).saturating_sub(1) & 0x0f));
            }
            Form::Two => {
                body.push(id);
                body.push(v.len() as u8);
            }
        }
        body.extend_from_slice(v);
    }
    while !body.len().is_multiple_of(4) {
        body.push(0);
    }
    let words = (body.len() / 4) as u16;
    let mut out = Vec::with_capacity(4 + body.len());
    match form {
        Form::One => out.extend_from_slice(&[0xbe, 0xde]),
        Form::Two => out.extend_from_slice(&[0x10, 0x00]),
    }
    out.extend_from_slice(&words.to_be_bytes());
    out.extend_from_slice(&body);
    out
}

pub fn read(block: &[u8], id: u8) -> Option<&[u8]> {
    if block.len() < 4 {
        return None;
    }
    let profile = u16::from_be_bytes([block[0], block[1]]);
    let len = usize::from(u16::from_be_bytes([block[2], block[3]])) * 4;
    let body = block.get(4..4 + len)?;
    let mut i = 0;
    match profile {
        0xbede => {
            while i < body.len() {
                let b = body[i];
                if b == 0 {
                    i += 1;
                    continue;
                }
                let eid = b >> 4;
                let l = usize::from(b & 0x0f) + 1;
                if eid == 15 {
                    return None;
                }
                let v = body.get(i + 1..i + 1 + l)?;
                if eid == id {
                    return Some(v);
                }
                i += 1 + l;
            }
            None
        }
        p if p & 0xfff0 == 0x1000 => {
            while i + 1 < body.len() {
                let eid = body[i];
                if eid == 0 {
                    i += 1;
                    continue;
                }
                let l = usize::from(body[i + 1]);
                let v = body.get(i + 2..i + 2 + l)?;
                if eid == id {
                    return Some(v);
                }
                i += 2 + l;
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_back_what_build_wrote() {
        let t = ExtTable { mid: Some(1), audio_level: Some(4), abs_send_time: Some(5), twcc: Some(6) };
        let b = build(&t, Some("3"), Some(0x9f), 1_000_000, 7);
        assert_eq!(read(&b, 1), Some(&b"3"[..]));
        assert_eq!(read(&b, 6), Some(&[0, 7][..]));
        let t2 = ExtTable { mid: Some(15), audio_level: None, abs_send_time: Some(3), twcc: Some(4) };
        let b2 = build(&t2, Some("12"), None, 2_000_000, 1);
        assert_eq!(read(&b2, 15), Some(&b"12"[..]));
        assert_eq!(t2.form(), Form::Two);
    }
}
