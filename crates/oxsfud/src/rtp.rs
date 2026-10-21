// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:IMPL-SUB-SUBSCRIBER-TABLE 정:IMPL-PUB-ACCEPT 연:RULE-LINK-EGRESS-REWRITE 연:RULE-ROOM-MEDIA-CODECS RFC 3550 RFC 8285 RFC 6184 RFC 7741 · model: claude-opus-5-5
//! RTP 바탕 — 헤더 읽기 · 확장(RFC 8285) 읽기와 다시 짓기 · 키프레임 판정기 · 코덱 튜플.

pub struct Rtp<'a> {
    pub buf: &'a [u8],
    pub marker: bool,
    pub pt: u8,
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
    pub csrc_end: usize,
    pub ext: Option<(u16, &'a [u8])>,
    pub payload_at: usize,
    pub payload_end: usize,
}

impl<'a> Rtp<'a> {
    pub fn parse(b: &'a [u8]) -> Option<Rtp<'a>> {
        if b.len() < 12 || b[0] >> 6 != 2 {
            return None;
        }
        let cc = (b[0] & 0x0f) as usize;
        let has_ext = b[0] & 0x10 != 0;
        let pad = b[0] & 0x20 != 0;
        let csrc_end = 12 + cc * 4;
        if b.len() < csrc_end {
            return None;
        }
        let mut at = csrc_end;
        let mut ext = None;
        if has_ext {
            if b.len() < at + 4 {
                return None;
            }
            let profile = u16::from_be_bytes([b[at], b[at + 1]]);
            let words = u16::from_be_bytes([b[at + 2], b[at + 3]]) as usize;
            let end = at + 4 + words * 4;
            if b.len() < end {
                return None;
            }
            ext = Some((profile, &b[at + 4..end]));
            at = end;
        }
        let mut end = b.len();
        if pad {
            let n = *b.last()? as usize;
            if n == 0 || end < at + n {
                return None;
            }
            end -= n;
        }
        Some(Rtp {
            buf: b,
            marker: b[1] & 0x80 != 0,
            pt: b[1] & 0x7f,
            seq: u16::from_be_bytes([b[2], b[3]]),
            ts: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            ssrc: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
            csrc_end,
            ext,
            payload_at: at,
            payload_end: end,
        })
    }

    pub fn payload(&self) -> &'a [u8] {
        &self.buf[self.payload_at..self.payload_end]
    }

    pub fn ext_value(&self, id: u8) -> Option<&'a [u8]> {
        let (profile, data) = self.ext?;
        ext_find(profile, data, id)
    }
}

pub fn ext_find(profile: u16, data: &[u8], want: u8) -> Option<&[u8]> {
    let mut i = 0;
    if profile == 0xBEDE {
        while i < data.len() {
            let h = data[i];
            if h == 0 {
                i += 1;
                continue;
            }
            let id = h >> 4;
            let len = (h & 0x0f) as usize + 1;
            if id == 15 || i + 1 + len > data.len() {
                return None;
            }
            if id == want {
                return Some(&data[i + 1..i + 1 + len]);
            }
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
                return None;
            }
            if id == want {
                return Some(&data[i + 2..i + 2 + len]);
            }
            i += 2 + len;
        }
    }
    None
}

pub struct Rewrite<'a> {
    pub pt: u8,
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
    pub marker: bool,
    pub exts: &'a [(u8, Vec<u8>)],
}

pub fn build(src: &Rtp<'_>, w: &Rewrite<'_>) -> Vec<u8> {
    let body = &src.buf[src.payload_at..];
    build_raw(&src.buf[12..src.csrc_end], src.buf[0] & 0x2f, w, body)
}

pub fn build_raw(csrc: &[u8], b0_keep: u8, w: &Rewrite<'_>, body: &[u8]) -> Vec<u8> {
    let mut ext = Vec::new();
    let mut two_byte = false;
    for (id, v) in w.exts {
        if v.is_empty() || v.len() > 16 || *id > 14 {
            two_byte = true;
        }
    }
    for (id, v) in w.exts {
        if two_byte {
            ext.push(*id);
            ext.push(v.len() as u8);
        } else {
            ext.push((id << 4) | ((v.len() as u8).saturating_sub(1) & 0x0f));
        }
        ext.extend_from_slice(v);
    }
    while ext.len() % 4 != 0 {
        ext.push(0);
    }
    let mut out = Vec::with_capacity(12 + csrc.len() + 4 + ext.len() + body.len());
    let has_ext = !w.exts.is_empty();
    out.push(0x80 | (b0_keep & 0x2f & !0x10) | if has_ext { 0x10 } else { 0 });
    out.push((w.pt & 0x7f) | if w.marker { 0x80 } else { 0 });
    out.extend_from_slice(&w.seq.to_be_bytes());
    out.extend_from_slice(&w.ts.to_be_bytes());
    out.extend_from_slice(&w.ssrc.to_be_bytes());
    out.extend_from_slice(csrc);
    if has_ext {
        out.extend_from_slice(&(if two_byte { 0x1000u16 } else { 0xBEDE }).to_be_bytes());
        out.extend_from_slice(&((ext.len() / 4) as u16).to_be_bytes());
        out.extend_from_slice(&ext);
    }
    out.extend_from_slice(body);
    out
}

pub fn abs_send_time(now_us: u64) -> [u8; 3] {
    let v = ((now_us << 18) / 1_000_000) & 0x00ff_ffff;
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

pub fn is_keyframe(codec: &str, payload: &[u8]) -> bool {
    match codec.to_ascii_uppercase().as_str() {
        "VP8" => vp8_key(payload),
        "H264" => h264_key(payload),
        _ => false,
    }
}

pub fn has_keyframe_judge(codec: &str) -> bool {
    matches!(codec.to_ascii_uppercase().as_str(), "VP8" | "H264")
}

fn vp8_key(p: &[u8]) -> bool {
    if p.is_empty() {
        return false;
    }
    let x = p[0] & 0x80 != 0;
    let s = p[0] & 0x10 != 0;
    let pid = p[0] & 0x07;
    if !s || pid != 0 {
        return false;
    }
    let mut i = 1;
    if x {
        if p.len() <= i {
            return false;
        }
        let e = p[i];
        i += 1;
        if e & 0x80 != 0 {
            if p.len() <= i {
                return false;
            }
            i += if p[i] & 0x80 != 0 { 2 } else { 1 };
        }
        if e & 0x40 != 0 {
            i += 1;
        }
        if e & 0x30 != 0 {
            i += 1;
        }
    }
    p.len() > i && p[i] & 0x01 == 0
}

fn h264_nal_key(t: u8) -> bool {
    t == 5 || t == 7
}

fn h264_key(p: &[u8]) -> bool {
    if p.is_empty() {
        return false;
    }
    let t = p[0] & 0x1f;
    match t {
        1..=23 => h264_nal_key(t),
        24 => {
            let mut i = 1;
            while i + 2 < p.len() {
                let n = u16::from_be_bytes([p[i], p[i + 1]]) as usize;
                i += 2;
                if i < p.len() && h264_nal_key(p[i] & 0x1f) {
                    return true;
                }
                i += n;
            }
            false
        }
        28 => p.len() > 1 && p[1] & 0x80 != 0 && h264_nal_key(p[1] & 0x1f),
        _ => false,
    }
}

pub fn fmtp_params(f: &str) -> Vec<(String, String)> {
    f.split(';')
        .filter_map(|kv| {
            let kv = kv.trim();
            if kv.is_empty() {
                return None;
            }
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            Some((k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect()
}

fn param(f: Option<&str>, key: &str) -> Option<String> {
    fmtp_params(f.unwrap_or("")).into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

pub fn h264_profile(f: Option<&str>) -> (String, String) {
    let pm = param(f, "packetization-mode").unwrap_or_else(|| "0".into());
    let plid = param(f, "profile-level-id").unwrap_or_else(|| "42001f".into());
    let profile: String = plid.to_ascii_lowercase().chars().take(4).collect();
    (pm, profile)
}

pub fn codec_matches(room_codec: &str, room_fmtp: Option<&str>, codec: &str, fmtp: Option<&str>) -> bool {
    if !room_codec.eq_ignore_ascii_case(codec) {
        return false;
    }
    if room_codec.eq_ignore_ascii_case("H264") {
        return h264_profile(room_fmtp) == h264_profile(fmtp);
    }
    true
}

pub fn tuple_key(kind: &str, codec: &str, fmtp: Option<&str>) -> String {
    let c = codec.to_ascii_lowercase();
    match c.as_str() {
        "h264" => {
            let (pm, prof) = h264_profile(fmtp);
            format!("{kind}|h264|pm={pm}|{prof}")
        }
        "opus" => {
            let st = param(fmtp, "sprop-stereo").unwrap_or_else(|| "0".into());
            format!("{kind}|opus|st={st}")
        }
        other => format!("{kind}|{other}"),
    }
}

pub fn sub_fmtp(codec: &str) -> Option<&'static str> {
    match codec.to_ascii_uppercase().as_str() {
        "H264" => Some("level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"),
        "OPUS" => Some("minptime=10;useinbandfec=1"),
        _ => None,
    }
}

pub fn canon_codec(name: &str) -> String {
    match name.to_ascii_uppercase().as_str() {
        "OPUS" => "opus".into(),
        "VP8" => "VP8".into(),
        "VP9" => "VP9".into(),
        "H264" => "H264".into(),
        "AV1" => "AV1".into(),
        other => other.to_string(),
    }
}

pub fn clock(kind_video: bool) -> u32 {
    if kind_video { 90_000 } else { 48_000 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_round_trip() {
        let exts = vec![(1u8, b"32".to_vec()), (6u8, vec![0, 7])];
        let w = Rewrite { pt: 96, seq: 10, ts: 1000, ssrc: 0xdead_beef, marker: true, exts: &exts };
        let out = build_raw(&[], 0, &w, &[1, 2, 3]);
        let p = Rtp::parse(&out).expect("parse");
        assert_eq!(p.pt, 96);
        assert!(p.marker);
        assert_eq!(p.ext_value(1), Some(&b"32"[..]));
        assert_eq!(p.ext_value(6), Some(&[0u8, 7][..]));
        assert_eq!(p.payload(), &[1, 2, 3]);
    }

    #[test]
    fn h264_profile_compare() {
        let chrome = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f";
        let room = sub_fmtp("H264");
        assert!(codec_matches("H264", room, "H264", Some(chrome)));
        assert!(!codec_matches("H264", room, "H264", Some("packetization-mode=0;profile-level-id=42e01f")));
        assert!(codec_matches("H264", room, "H264", Some("packetization-mode=1;profile-level-id=42e034")));
    }

    #[test]
    fn keyframes() {
        assert!(vp8_key(&[0x10, 0x00]));
        assert!(!vp8_key(&[0x10, 0x01]));
        assert!(h264_key(&[0x65]));
        assert!(h264_key(&[0x7c, 0x85]));
        assert!(!h264_key(&[0x41]));
    }
}
