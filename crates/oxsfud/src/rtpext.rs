// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§10-1 · 연§6-3 · model: claude-opus-5

//! RTP 헤더 확장 읽기(RFC 8285) — ★**시뮬캐스트는 이것으로만 갈린다.**
//!
//! ★★**등록 항목은 `rid` 를 안 싣는다**(연§6-3) — 논리 스트림 하나이고, 서버는 ★**RTP 의
//! rid 확장으로 단을 배운다.** 그래서 이 읽개가 없으면 시뮬캐스트가 통째로 성립하지 않는다.
//!
//! ★**실측 전제**: 브라우저는 rid 를 **매 패킷** 싣고 mid 는 첫 몇 장에만 싣는다 — 그래서
//! 분류는 ★**rid 단독 경로**가 정본이다(mid 로 가르는 길에 기대면 그 뒤 패킷을 못 가른다).

/// 한 바이트 형 확장의 표식(RFC 8285 §4.2).
const ONE_BYTE: u16 = 0xBEDE;
/// 두 바이트 형의 상위 12비트(RFC 8285 §4.3).
const TWO_BYTE_TAG: u16 = 0x1000;
const FIXED: usize = 12;

/// `X` 비트가 서 있으면 확장 구역의 `(시작, 끝)`. ★**CSRC 를 건너뛰고 잰다.**
fn region(pkt: &[u8]) -> Option<(usize, usize, u16)> {
    if pkt.len() < FIXED || pkt[0] >> 6 != 2 || pkt[0] & 0x10 == 0 {
        return None;
    }
    let csrc = (pkt[0] & 0x0F) as usize * 4;
    let at = FIXED + csrc;
    if pkt.len() < at + 4 {
        return None;
    }
    let profile = u16::from_be_bytes([pkt[at], pkt[at + 1]]);
    let words = u16::from_be_bytes([pkt[at + 2], pkt[at + 3]]) as usize;
    let start = at + 4;
    let end = start + words * 4;
    if pkt.len() < end {
        return None;
    }
    Some((start, end, profile))
}

/// 그 `id` 의 값. ★**복사하지 않는다** — 준 버퍼를 가리킨다(핫패스 규율 H1).
///
/// ★**모르는 형은 `None`** — 억지로 읽으면 남의 바이트를 값으로 쓴다.
pub fn get(pkt: &[u8], id: u8) -> Option<&[u8]> {
    let (start, end, profile) = region(pkt)?;
    if profile == ONE_BYTE {
        let mut at = start;
        while at < end {
            let b = pkt[at];
            // ★`0` 은 채움이다 — 값이 아니다.
            if b == 0 {
                at += 1;
                continue;
            }
            let (eid, len) = (b >> 4, (b & 0x0F) as usize + 1);
            // ★`15` 는 예약(그 뒤는 못 읽는다).
            if eid == 15 || at + 1 + len > end {
                return None;
            }
            if eid == id {
                return Some(&pkt[at + 1..at + 1 + len]);
            }
            at += 1 + len;
        }
        return None;
    }
    if profile & 0xFFF0 == TWO_BYTE_TAG {
        let mut at = start;
        while at + 1 < end {
            let eid = pkt[at];
            if eid == 0 {
                at += 1;
                continue;
            }
            let len = pkt[at + 1] as usize;
            if at + 2 + len > end {
                return None;
            }
            if eid == id {
                return Some(&pkt[at + 2..at + 2 + len]);
            }
            at += 2 + len;
        }
    }
    None
}

/// `rid` 값(RFC 8852) — 문자열이다.
pub fn rid(pkt: &[u8], id: u8) -> Option<&str> {
    std::str::from_utf8(get(pkt, id)?).ok()
}

/// ★**낮은 화질부터 `0`**(정§10-1 · 17차 `l;h`) — 인덱스가 곧 구독 제어의 어휘다.
///
/// ★**모르는 이름은 `None`** — 지어낸 인덱스로 단을 고르면 엉뚱한 화질이 간다.
pub fn spatial_of(rid: &str) -> Option<u8> {
    match rid {
        "l" => Some(0),
        "h" => Some(1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 한 바이트 형 확장 하나를 단 RTP 한 장.
    fn with_one_byte(id: u8, val: &[u8]) -> Vec<u8> {
        let mut p = vec![0x90, 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 7];
        p.extend_from_slice(&ONE_BYTE.to_be_bytes());
        let body_len = 1 + val.len();
        let words = body_len.div_ceil(4);
        p.extend_from_slice(&(words as u16).to_be_bytes());
        p.push((id << 4) | (val.len() as u8 - 1));
        p.extend_from_slice(val);
        p.resize(p.len() + (words * 4 - body_len), 0);
        p.extend_from_slice(b"payload");
        p
    }

    #[test]
    fn 한_바이트_형에서_rid_를_읽는다() {
        let p = with_one_byte(10, b"h");
        assert_eq!(rid(&p, 10), Some("h"));
        assert_eq!(spatial_of("h"), Some(1));
        assert_eq!(spatial_of("l"), Some(0));
        // ★**없는 id 는 없는 것이다** — 남의 값을 가져오지 않는다.
        assert_eq!(get(&p, 1), None);
    }

    #[test]
    fn 두_바이트_형도_읽는다() {
        let mut p = vec![0x90, 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 7];
        p.extend_from_slice(&TWO_BYTE_TAG.to_be_bytes());
        p.extend_from_slice(&1u16.to_be_bytes());
        p.extend_from_slice(&[10, 1, b'l', 0]);
        assert_eq!(rid(&p, 10), Some("l"));
    }

    #[test]
    fn 확장이_없으면_없는_것이다() {
        // `X` 비트가 안 선 평범한 RTP.
        let p = vec![0x80, 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 7];
        assert_eq!(rid(&p, 10), None);
        assert_eq!(get(&[], 10), None);
    }

    #[test]
    fn csrc_를_건너뛴다() {
        // ★CSRC 를 안 건너뛰면 확장 구역의 시작을 4바이트씩 잘못 짚는다.
        let mut p = with_one_byte(10, b"h");
        p[0] |= 1; // CC = 1
        // 고정 머리 뒤에 CSRC 한 칸을 끼운다.
        let mut q = p[..FIXED].to_vec();
        q.extend_from_slice(&[0, 0, 0, 9]);
        q.extend_from_slice(&p[FIXED..]);
        assert_eq!(rid(&q, 10), Some("h"));
    }

    #[test]
    fn 길이가_거짓이면_안_읽는다() {
        let mut p = with_one_byte(10, b"h");
        // 확장 워드 수를 부풀린다 — 버퍼 밖을 가리킨다.
        let at = FIXED + 2;
        p[at..at + 2].copy_from_slice(&99u16.to_be_bytes());
        assert_eq!(rid(&p, 10), None, "★남의 바이트를 값으로 쓰지 않는다");
    }

    #[test]
    fn 모르는_rid_이름은_단이_아니다() {
        assert_eq!(spatial_of("m"), None, "★지어낸 인덱스로 단을 고르면 엉뚱한 화질이 간다");
    }
}
