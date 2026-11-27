// author: kodeholic (powered by Claude)
//! RTP 꼴 → 코덱 원래 꼴(SDK:RULE-PLAT-SERVER 수신 단위 · 시각 — ★조립은 디코딩이 아니다).
//! VP8 은 페이로드 서술자를 뗀다(RFC 7741) · H264 는 단일 NAL · STAP-A · FU-A 를 Annex-B 로 잇는다(RFC 6184 · `packetization-mode=1`) · Opus 는 패킷 하나가 프레임 하나다.

/// 코덱 갈래 — 배정표 줄의 `codec` 이름에서 정한다(대소문자 무시)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Codec {
    Opus,
    Vp8,
    H264,
    /// 조립을 모르는 코덱 — 패킷 하나를 프레임 하나로 낸다
    Other,
}

impl Codec {
    pub(crate) fn of(name: &str) -> Codec {
        match name.to_ascii_lowercase().as_str() {
            "opus" => Codec::Opus,
            "vp8" => Codec::Vp8,
            "h264" => Codec::H264,
            _ => Codec::Other,
        }
    }

    /// RTP 시계(Hz) — 코덱 정의상 고정(연:RULE-LINK-SDP-SUB-MLINE 클럭 표)
    pub(crate) fn clock(self, kind_video: bool) -> u32 {
        match self {
            Codec::Opus => 48_000,
            Codec::Vp8 | Codec::H264 => 90_000,
            Codec::Other if kind_video => 90_000,
            Codec::Other => 48_000,
        }
    }
}

/// 패킷 하나를 푼 것 — 프레임 조각 · 프레임의 첫 조각인가 · 키프레임 표시
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    pub data: Vec<u8>,
    pub start: bool,
    pub key: bool,
}

const ANNEX_B: [u8; 4] = [0, 0, 0, 1];
const NAL_IDR: u8 = 5;
const NAL_STAP_A: u8 = 24;
const NAL_FU_A: u8 = 28;

/// 패킷 페이로드 하나를 푼다 — 꼴이 깨졌으면 None(그 프레임은 깨진 것으로 다룬다)
pub(crate) fn unpack(codec: Codec, p: &[u8]) -> Option<Piece> {
    match codec {
        Codec::Vp8 => vp8(p),
        Codec::H264 => h264(p),
        Codec::Opus | Codec::Other => Some(Piece { data: p.to_vec(), start: true, key: true }),
    }
}

/// RFC 7741 §4.2 서술자 — X · S · PID 와 확장(I · L · T · K)을 건너뛴다. 키프레임 = 파티션 0 의 첫 조각이고 VP8 머리 P 비트가 0
fn vp8(p: &[u8]) -> Option<Piece> {
    let b0 = *p.first()?;
    let mut i = 1;
    if b0 & 0x80 != 0 {
        let x = *p.get(i)?;
        i += 1;
        if x & 0x80 != 0 {
            let pid = *p.get(i)?;
            i += if pid & 0x80 != 0 { 2 } else { 1 };
        }
        if x & 0x40 != 0 {
            i += 1;
        }
        if x & 0x30 != 0 {
            i += 1;
        }
    }
    let body = p.get(i..)?;
    if body.is_empty() {
        return None;
    }
    let start = b0 & 0x10 != 0 && b0 & 0x07 == 0;
    let key = start && body[0] & 0x01 == 0;
    Some(Piece { data: body.to_vec(), start, key })
}

/// RFC 6184 §5.6 · §5.7.1 · §5.8 — 단일 NAL · STAP-A · FU-A 를 Annex-B(시작 부호 + NAL)로. 키프레임 = IDR(5)을 품었다
fn h264(p: &[u8]) -> Option<Piece> {
    let h = *p.first()?;
    let t = h & 0x1f;
    match t {
        1..=23 => {
            let mut data = ANNEX_B.to_vec();
            data.extend_from_slice(p);
            Some(Piece { data, start: true, key: t == NAL_IDR })
        }
        NAL_STAP_A => {
            let mut data = Vec::with_capacity(p.len() + 8);
            let mut key = false;
            let mut i = 1;
            while i + 2 <= p.len() {
                let n = usize::from(u16::from_be_bytes([p[i], p[i + 1]]));
                i += 2;
                let nal = p.get(i..i + n)?;
                if nal.is_empty() {
                    return None;
                }
                key |= nal[0] & 0x1f == NAL_IDR;
                data.extend_from_slice(&ANNEX_B);
                data.extend_from_slice(nal);
                i += n;
            }
            if i != p.len() || data.is_empty() {
                return None;
            }
            Some(Piece { data, start: true, key })
        }
        NAL_FU_A => {
            let fu = *p.get(1)?;
            let body = p.get(2..)?;
            let start = fu & 0x80 != 0;
            let nt = fu & 0x1f;
            let mut data = Vec::with_capacity(body.len() + 5);
            if start {
                data.extend_from_slice(&ANNEX_B);
                data.push((h & 0xe0) | nt);
            }
            data.extend_from_slice(body);
            // FU-A 는 이어지는 조각이 프레임의 첫 조각이 아니다 — 프레임 경계는 시각 · marker 가 정한다
            Some(Piece { data, start, key: start && nt == NAL_IDR })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vp8_strips_descriptor_and_marks_key() {
        // X=1 · S=1 · PID=0 / I=1(15비트 picture id) · L=1 · T=1 / 본문 첫 바이트 P=0 → 키프레임
        let p = [0x90, 0xe0, 0x80, 0x05, 0x07, 0x20, 0x10, 0xaa, 0xbb];
        let got = unpack(Codec::Vp8, &p).unwrap();
        assert_eq!(got, Piece { data: vec![0x10, 0xaa, 0xbb], start: true, key: true });
        // 같은 프레임의 다음 조각(S=0) — 첫 조각도 키프레임도 아니다
        let p = [0x80, 0x80, 0x05, 0x10, 0xcc];
        assert_eq!(unpack(Codec::Vp8, &p).unwrap(), Piece { data: vec![0x10, 0xcc], start: false, key: false });
        // 짧은 서술자 · P=1 → 델타
        assert_eq!(unpack(Codec::Vp8, &[0x10, 0x01, 0x02]).unwrap(), Piece { data: vec![0x01, 0x02], start: true, key: false });
        assert!(unpack(Codec::Vp8, &[0x90, 0x80]).is_none());
    }

    #[test]
    fn h264_single_stap_fu() {
        let single = [0x65, 0x88, 0x84];
        assert_eq!(unpack(Codec::H264, &single).unwrap(), Piece { data: vec![0, 0, 0, 1, 0x65, 0x88, 0x84], start: true, key: true });
        // STAP-A: SPS(2바이트) + PPS(1바이트)
        let stap = [0x78, 0x00, 0x02, 0x67, 0x42, 0x00, 0x01, 0x68];
        assert_eq!(unpack(Codec::H264, &stap).unwrap(), Piece { data: vec![0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x68], start: true, key: false });
        assert!(unpack(Codec::H264, &[0x78, 0x00, 0x05, 0x67]).is_none());
        // FU-A: 시작 조각은 NAL 머리를 되살린다(NRI 는 지시자에서 · 형은 FU 머리에서)
        let fu_s = [0x7c, 0x85, 0xaa];
        assert_eq!(unpack(Codec::H264, &fu_s).unwrap(), Piece { data: vec![0, 0, 0, 1, 0x65, 0xaa], start: true, key: true });
        let fu_m = [0x7c, 0x45, 0xbb];
        assert_eq!(unpack(Codec::H264, &fu_m).unwrap(), Piece { data: vec![0xbb], start: false, key: false });
    }

    #[test]
    fn codec_names_and_clock() {
        assert_eq!(Codec::of("H264"), Codec::H264);
        assert_eq!(Codec::of("VP8"), Codec::Vp8);
        assert_eq!(Codec::of("opus").clock(false), 48_000);
        assert_eq!(Codec::of("AV1").clock(true), 90_000);
    }
}
