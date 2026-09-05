// author: kodeholic (powered by Claude)
//! RTP 헤더 최소 조작 — 정§7-2-1 egress 재기록의 도구. PT 는 byte1 의 7비트만, 확장 번호는 원소 헤더의 id 자리만 바꾼다.
//! ★어느 쪽도 길이를 바꾸지 않는다 — 길이가 변하면 SRTP 재암호 뒤 패딩·태그 계산이 어긋난다.

/// RFC 3550 고정 헤더.
pub const FIXED_LEN: usize = 12;
/// RFC 8285 one-byte 확장 프로파일.
const PROFILE_ONE_BYTE: u16 = 0xBEDE;
/// RFC 8285 two-byte 확장 프로파일(하위 4비트는 appbits).
const PROFILE_TWO_BYTE: u16 = 0x1000;
const TWO_BYTE_MASK: u16 = 0xFFF0;

pub fn is_rtp(pkt: &[u8]) -> bool {
    pkt.len() >= FIXED_LEN && pkt[0] >> 6 == 2
}

/// RTCP 는 PT 200~207 로 갈린다(RFC 5761 — 같은 포트에 섞여 온다).
pub fn is_rtcp(pkt: &[u8]) -> bool {
    matches!(pkt.get(1), Some(200..=207)) && pkt.len() >= 8
}

pub fn payload_type(pkt: &[u8]) -> Option<u8> {
    Some(pkt.get(1)? & 0x7F)
}

pub fn sequence(pkt: &[u8]) -> Option<u16> {
    Some(u16::from_be_bytes([*pkt.get(2)?, *pkt.get(3)?]))
}

pub fn timestamp(pkt: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(pkt.get(4..8)?.try_into().ok()?))
}

pub fn ssrc(pkt: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(pkt.get(8..12)?.try_into().ok()?))
}

/// marker 비트는 보존한다.
pub fn set_payload_type(pkt: &mut [u8], pt: u8) {
    if let Some(b) = pkt.get_mut(1) {
        *b = (*b & 0x80) | (pt & 0x7F);
    }
}

pub fn set_sequence(pkt: &mut [u8], seq: u16) {
    if pkt.len() >= 4 {
        pkt[2..4].copy_from_slice(&seq.to_be_bytes());
    }
}

pub fn set_timestamp(pkt: &mut [u8], ts: u32) {
    if pkt.len() >= 8 {
        pkt[4..8].copy_from_slice(&ts.to_be_bytes());
    }
}

pub fn set_ssrc(pkt: &mut [u8], ssrc: u32) {
    if pkt.len() >= 12 {
        pkt[8..12].copy_from_slice(&ssrc.to_be_bytes());
    }
}

/// 확장 블록의 (프로파일, 데이터 구간) — 확장이 없으면 `None`.
fn extension_span(pkt: &[u8]) -> Option<(u16, std::ops::Range<usize>)> {
    if !is_rtp(pkt) || pkt[0] & 0x10 == 0 {
        return None;
    }
    let at = FIXED_LEN + usize::from(pkt[0] & 0x0F) * 4;
    let profile = u16::from_be_bytes([*pkt.get(at)?, *pkt.get(at + 1)?]);
    let words = usize::from(u16::from_be_bytes([*pkt.get(at + 2)?, *pkt.get(at + 3)?]));
    let start = at + 4;
    let end = start + words * 4;
    (end <= pkt.len()).then_some((profile, start..end))
}

/// 그 패킷이 싣고 있는 확장 번호 전량 — 발행자가 어떤 번호를 썼는지 읽는 자리.
pub fn extension_ids(pkt: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    walk_extension(pkt, |id, _| out.push(id));
    out
}

fn walk_extension(pkt: &[u8], mut visit: impl FnMut(u8, usize)) {
    let Some((profile, span)) = extension_span(pkt) else { return };
    let two_byte = profile & TWO_BYTE_MASK == PROFILE_TWO_BYTE;
    if !two_byte && profile != PROFILE_ONE_BYTE {
        return;
    }
    let mut at = span.start;
    while at < span.end {
        let b = pkt[at];
        if b == 0 {
            at += 1;
            continue;
        }
        if two_byte {
            let Some(&len) = pkt.get(at + 1) else { return };
            visit(b, at);
            at += 2 + usize::from(len);
        } else {
            if b >> 4 == 15 {
                return;
            }
            visit(b >> 4, at);
            at += 1 + usize::from((b & 0x0F) + 1);
        }
    }
}

/// 그 번호의 확장 값 — 첫 것만(같은 id 가 여럿이면 순서는 계약이 아니다).
pub fn extension_value(pkt: &[u8], want: u8) -> Option<&[u8]> {
    let (profile, span) = extension_span(pkt)?;
    let two_byte = profile & TWO_BYTE_MASK == PROFILE_TWO_BYTE;
    if !two_byte && profile != PROFILE_ONE_BYTE {
        return None;
    }
    let mut at = span.start;
    while at < span.end {
        let b = pkt[at];
        if b == 0 {
            at += 1;
            continue;
        }
        let (id, len, from) = if two_byte {
            (b, usize::from(*pkt.get(at + 1)?), at + 2)
        } else {
            if b >> 4 == 15 {
                return None;
            }
            (b >> 4, usize::from((b & 0x0F) + 1), at + 1)
        };
        if id == want {
            return pkt.get(from..from + len);
        }
        at = from + len;
    }
    None
}

/// 헤더(+CSRC·확장)의 끝 — payload 가 시작하는 자리.
pub fn payload_offset(pkt: &[u8]) -> Option<usize> {
    if !is_rtp(pkt) {
        return None;
    }
    let at = match extension_span(pkt) {
        Some((_, span)) => span.end,
        None => FIXED_LEN + usize::from(pkt[0] & 0x0F) * 4,
    };
    (at <= pkt.len()).then_some(at)
}

/// 헤더(+CSRC·확장) 뒤 — 코덱이 읽는 자리. 패딩은 걷지 않는다(키프레임 판정은 앞쪽만 본다).
pub fn payload(pkt: &[u8]) -> Option<&[u8]> {
    pkt.get(payload_offset(pkt)?..)
}

/// 정§10-3 v2 — 프로브 패딩 한 장. ★RTX PT·SSRC 로 나간다 — 구독자는 모르는 원본이라 버리지만,
/// ★TWCC 번호는 찍혀 돌아온다. 그 되돌아옴이 「이만큼은 실제로 지나간다」의 유일한 증거다.
pub fn probe_padding(pt: u8, ssrc: u32, seq: u16, bytes: usize) -> Vec<u8> {
    let mut p = Vec::with_capacity(bytes.max(FIXED_LEN));
    p.extend_from_slice(&[0x80, pt & 0x7F]);
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&0u32.to_be_bytes());
    p.extend_from_slice(&ssrc.to_be_bytes());
    p.resize(bytes.max(FIXED_LEN + 2), 0);
    p
}

/// 정§10-3 v2 — 구독 전송로 단위 TWCC 시퀀스를 egress 에 ★**교체하거나 삽입**한다(정§7-2-1 TWCC 행).
///
/// ★발행자 seq 를 그대로 통과시키면 여러 발행자의 번호가 한 구독 전송로에 섞여 구독자 피드백이
/// 뜻을 잃는다. 자리는 ★**구독자 표의 번호**이고, 값은 그 전송로가 발급한 것이다.
/// one-byte form 만 쓴다 — 서버가 내는 확장은 전부 그 꼴이고, two-byte 가 오면 손대지 않는다(계수는 부르는 쪽).
pub fn upsert_twcc(pkt: &mut Vec<u8>, id: u8, seq: u16) -> bool {
    if !(1..=14).contains(&id) || !is_rtp(pkt) {
        return false;
    }
    let val = seq.to_be_bytes();
    match extension_span(pkt) {
        Some((PROFILE_ONE_BYTE, span)) => {
            // ① 이미 그 번호가 있으면 값만 바꾼다 — 길이 불변이 이 경로의 성질이다.
            let mut at = span.start;
            while at < span.end {
                let b = pkt[at];
                if b == 0 {
                    at += 1;
                    continue;
                }
                let len = usize::from((b & 0x0F) + 1);
                if b >> 4 == id {
                    if len != val.len() || at + 1 + len > pkt.len() {
                        return false;
                    }
                    pkt[at + 1..at + 1 + len].copy_from_slice(&val);
                    return true;
                }
                at += 1 + len;
            }
            // ② 없으면 꼬리 패딩에 얹거나, 모자라면 워드 하나를 늘린다.
            let pad = span.clone().rev().take_while(|&i| pkt[i] == 0).count();
            let elem = [id << 4 | 1, val[0], val[1]];
            if pad >= elem.len() {
                let at = span.end - pad;
                pkt[at..at + elem.len()].copy_from_slice(&elem);
                return true;
            }
            let mut word = [0u8; 4];
            word[..elem.len()].copy_from_slice(&elem);
            let insert_at = span.end - pad;
            pkt.splice(insert_at..insert_at, word.iter().copied());
            let words_at = span.start - 2;
            let words = u16::from_be_bytes([pkt[words_at], pkt[words_at + 1]]) + 1;
            pkt[words_at..words_at + 2].copy_from_slice(&words.to_be_bytes());
            true
        }
        // ③ 확장 블록 자체가 없다 — 새로 만든다(X 비트를 세운다).
        None => {
            let at = FIXED_LEN + usize::from(pkt[0] & 0x0F) * 4;
            if at > pkt.len() {
                return false;
            }
            let block = [0xBE, 0xDE, 0x00, 0x01, id << 4 | 1, val[0], val[1], 0x00];
            pkt.splice(at..at, block.iter().copied());
            pkt[0] |= 0x10;
            true
        }
        // two-byte form — 서버가 만들지 않는 꼴이라 손대지 않는다.
        Some(_) => false,
    }
}

/// 정§7-2-1 — 원소마다 (발행자 번호 → 구독자 번호). `map` 이 `None` 을 주면 그 번호는 그대로 둔다.
/// ★id 자리만 바꾸므로 길이 불변이고, ★중간 자료 없이 제자리에서 한 번에 훑는다(매 패킷·구독자마다 도는 자리).
pub fn rewrite_extension_ids(pkt: &mut [u8], map: impl Fn(u8) -> Option<u8>) -> usize {
    let Some((profile, span)) = extension_span(pkt) else { return 0 };
    let two_byte = profile & TWO_BYTE_MASK == PROFILE_TWO_BYTE;
    if !two_byte && profile != PROFILE_ONE_BYTE {
        return 0;
    }
    let (mut at, mut edits) = (span.start, 0);
    while at < span.end {
        let b = pkt[at];
        if b == 0 {
            at += 1;
            continue;
        }
        if two_byte {
            let Some(&len) = pkt.get(at + 1) else { return edits };
            if let Some(next) = map(b) {
                pkt[at] = next;
                edits += 1;
            }
            at += 2 + usize::from(len);
        } else {
            if b >> 4 == 15 {
                return edits;
            }
            if let Some(next) = map(b >> 4) {
                pkt[at] = (next << 4) | (b & 0x0F);
                edits += 1;
            }
            at += 1 + usize::from((b & 0x0F) + 1);
        }
    }
    edits
}

#[cfg(test)]
mod rtx_tests {
    use super::*;

    #[test]
    fn a_retransmit_keeps_the_header_and_hides_the_original_seq_in_the_payload() {
        let mut pkt = vec![0x80, 96, 0x12, 0x34, 0, 0, 0, 9, 0, 0, 0, 7];
        pkt.extend_from_slice(&[0xAA, 0xBB]);
        let rtx = to_rtx(&pkt, 97, 0xDEAD, 5).expect("rtp 여야 한다");

        assert_eq!(payload_type(&rtx), Some(97));
        assert_eq!(sequence(&rtx), Some(5), "재전송은 자기 seq 공간이다");
        assert_eq!(ssrc(&rtx), Some(0xDEAD));
        assert_eq!(timestamp(&rtx), timestamp(&pkt), "시각은 원본이다");
        assert_eq!(
            payload(&rtx).unwrap(),
            &[0x12, 0x34, 0xAA, 0xBB],
            "★앞 두 바이트가 원본 seq(OSN) 다 — 없으면 수신측이 어느 것의 재전송인지 모른다"
        );
    }

    #[test]
    fn what_is_not_rtp_is_not_retransmittable() {
        assert!(to_rtx(&[0x00, 0x01], 97, 1, 1).is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_byte_packet() -> Vec<u8> {
        let mut p = vec![0x90, 0xE0, 0x12, 0x34, 0, 0, 0x30, 0x39, 0xAA, 0xBB, 0xCC, 0xDD];
        p.extend_from_slice(&PROFILE_ONE_BYTE.to_be_bytes());
        p.extend_from_slice(&1u16.to_be_bytes());
        p.extend_from_slice(&[0x20, 0x77, 0x40, 0x11]);
        p.extend_from_slice(&[9, 9, 9]);
        p
    }

    #[test]
    fn header_fields_are_read_and_written_in_place() {
        let mut p = one_byte_packet();
        let before = p.len();
        assert_eq!((is_rtp(&p), is_rtcp(&p)), (true, false));
        assert_eq!((payload_type(&p), sequence(&p), timestamp(&p), ssrc(&p)), (Some(0x60), Some(0x1234), Some(12345), Some(0xAABB_CCDD)));
        set_payload_type(&mut p, 111);
        assert_eq!((payload_type(&p), p[1] & 0x80), (Some(111), 0x80), "marker 보존");
        set_sequence(&mut p, 7);
        set_timestamp(&mut p, 8);
        set_ssrc(&mut p, 9);
        assert_eq!((sequence(&p), timestamp(&p), ssrc(&p), p.len()), (Some(7), Some(8), Some(9), before));
    }

    #[test]
    fn one_byte_extension_ids_are_renumbered_without_resizing() {
        let mut p = one_byte_packet();
        let before = p.len();
        assert_eq!(extension_ids(&p), vec![2, 4]);
        assert_eq!(rewrite_extension_ids(&mut p, |id| match id {
            2 => Some(5),
            4 => Some(6),
            _ => None,
        }), 2);
        assert_eq!(extension_ids(&p), vec![5, 6]);
        assert_eq!((p.len(), &p[16..20]), (before, &[0x50, 0x77, 0x60, 0x11][..]), "길이·값 그대로, id 자리만");
    }

    #[test]
    fn extension_values_and_payload_start() {
        let p = one_byte_packet();
        assert_eq!(extension_value(&p, 2), Some(&[0x77u8][..]));
        assert_eq!(extension_value(&p, 4), Some(&[0x11u8][..]));
        assert_eq!(extension_value(&p, 9), None);
        assert_eq!(payload(&p), Some(&[9u8, 9, 9][..]), "확장 뒤가 코덱의 자리다");
        let plain = vec![0x80, 0x60, 0, 1, 0, 0, 0, 0, 1, 2, 3, 4, 0xAB];
        assert_eq!((extension_value(&plain, 1), payload(&plain)), (None, Some(&[0xABu8][..])));
    }

    /// 정§10-3 v2 — 스탬핑 세 갈래: 확장 없음(신설) · 있는데 그 번호 없음(추가) · 이미 있음(교체).
    #[test]
    fn twcc_is_stamped_whether_the_packet_had_an_extension_or_not() {
        // ① 확장 블록이 없다 — 새로 만들고 X 비트를 세운다.
        let mut plain = vec![0x80, 0x60, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0xAA, 0xBB];
        assert!(upsert_twcc(&mut plain, 6, 0x1234));
        assert_eq!(plain[0] & 0x10, 0x10, "X 비트가 선다");
        assert_eq!(extension_value(&plain, 6), Some(&[0x12u8, 0x34][..]));
        assert_eq!(payload(&plain), Some(&[0xAAu8, 0xBB][..]), "payload 는 그대로다");

        // ② 확장은 있는데 그 번호가 없다 — 얹는다(꼬리 패딩이 없어 워드가 하나 는다).
        //    그리고 ③ 다시 부르면 교체다 — 길이 불변.
        let mut p = one_byte_packet();
        let words_before = p.len();
        assert!(upsert_twcc(&mut p, 6, 0x0001));
        assert_eq!(extension_value(&p, 6), Some(&[0x00u8, 0x01][..]));
        assert_eq!(extension_value(&p, 2), Some(&[0x77u8][..]), "남의 원소는 안 건드린다");
        assert_eq!(payload(&p), Some(&[9u8, 9, 9][..]), "payload 는 뒤로 밀릴 뿐 그대로다");
        assert_eq!(p.len(), words_before + 4, "워드 하나만 는다");
        let len_before = p.len();
        assert!(upsert_twcc(&mut p, 6, 0xFFFE));
        assert_eq!(p.len(), len_before, "★교체는 길이 불변");
        assert_eq!(extension_value(&p, 6), Some(&[0xFFu8, 0xFE][..]));
        assert_eq!(extension_ids(&p), vec![2, 4, 6]);
    }

    #[test]
    fn two_byte_extension_and_absent_extension() {
        let mut p = vec![0x90, 0x60, 0, 1, 0, 0, 0, 0, 1, 2, 3, 4];
        p.extend_from_slice(&PROFILE_TWO_BYTE.to_be_bytes());
        p.extend_from_slice(&1u16.to_be_bytes());
        p.extend_from_slice(&[0x0A, 0x01, 0x77, 0x00]);
        assert_eq!(extension_ids(&p), vec![10]);
        assert_eq!(rewrite_extension_ids(&mut p, |id| (id == 10).then_some(3)), 1);
        assert_eq!((extension_ids(&p), p[18]), (vec![3], 0x77));

        let mut plain = vec![0x80, 0x60, 0, 1, 0, 0, 0, 0, 1, 2, 3, 4, 9];
        assert!(extension_ids(&plain).is_empty());
        assert_eq!(rewrite_extension_ids(&mut plain, |_| Some(1)), 0);
        assert!(!is_rtp(&plain[..8]));
    }
}

/// RFC 4588 §4 — 재전송 패킷. 원본 헤더를 그대로 두고 PT·SSRC·seq 만 갈고
/// ★payload 앞에 원본 seq(OSN) 두 바이트를 끼운다. 확장·CSRC 는 원본을 따라간다.
pub fn to_rtx(packet: &[u8], pt: u8, ssrc: u32, seq: u16) -> Option<Vec<u8>> {
    let head = payload_offset(packet)?;
    let original = sequence(packet)?;
    let mut out = Vec::with_capacity(packet.len() + 2);
    out.extend_from_slice(&packet[..head]);
    out.extend_from_slice(&original.to_be_bytes());
    out.extend_from_slice(&packet[head..]);
    set_payload_type(&mut out, pt);
    out[2..4].copy_from_slice(&seq.to_be_bytes());
    out[8..12].copy_from_slice(&ssrc.to_be_bytes());
    Some(out)
}
