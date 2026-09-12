// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§13 · 연§3-3 · model: claude-opus-5

//! DataChannel 위의 두 겹 — ★**DCEP**(채널을 여는 말)와 ★**DC 프레임**(그 위에 흐르는 것).
//!
//! ```text
//!  0        1        2        3        4 …
//! +--------+--------+-----------------+-------------------------+
//! | ver=1  | svc(1) |    len(2 BE)    |   payload (len bytes)   |
//! +--------+--------+-----------------+-------------------------+
//! ```
//!
//! ★**머리가 4바이트다**(연§3-3) — 구판의 3바이트(`ver` 없음)는 이행 항목이었다.
//! WS 프레임과 ★**첫 바이트의 뜻이 같아** 어느 전송로든 판을 같은 자리에서 읽는다.

/// DC 프레임의 판. ★**WS 와 같은 값이다.**
pub const VER: u8 = 0x01;
pub const DC_HEADER_LEN: usize = 4;

/// ★**발언권(MBCP)** — 규격이 정하는 유일한 `svc`(연§3-3).
pub const SVC_FLOOR: u8 = 0x01;
/// 발성 감지 — ★**확장이다**(연동규격서 밖). 무전 방은 발신 금지·수신 무시(정§13).
pub const SVC_SPEAKING: u8 = 0x02;

/// ★**채널 이름은 이것 하나다.** 다른 이름은 거부한다(연§3-3) — DCEP 에 거부 메시지가
/// 없어서 클라에겐 `open` 이 영영 안 오는 것으로 보인다. 그것이 규격이 고른 형상이다.
pub const LABEL: &str = "unreliable";

/// 프레임 하나를 짓는다. ★**`len` 이 상한을 넘으면 보내는 쪽이 거부한다**(연§3-3).
pub fn build(svc: u8, payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > u16::MAX as usize {
        return None;
    }
    let mut buf = Vec::with_capacity(DC_HEADER_LEN + payload.len());
    buf.push(VER);
    buf.push(svc);
    buf.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    buf.extend_from_slice(payload);
    Some(buf)
}

/// 프레임 하나를 읽는다. ★**잘린 것은 조용히 버린다** — 예외를 던지지 않는다(연§3-3).
///
/// ★**복사하지 않는다** — payload 는 준 버퍼를 가리킨다(핫패스 규율 H1).
pub fn parse(data: &[u8]) -> Option<(u8, &[u8])> {
    if data.len() < DC_HEADER_LEN || data[0] != VER {
        return None;
    }
    let len = u16::from_be_bytes([data[2], data[3]]) as usize;
    if data.len() < DC_HEADER_LEN + len {
        return None;
    }
    Some((data[1], &data[DC_HEADER_LEN..DC_HEADER_LEN + len]))
}

// ─── DCEP(RFC 8832) — 채널을 여는 말 ────────────────────────────────────────

/// `PPID` — SCTP 가 이 덩어리를 무엇으로 읽어야 하나(RFC 8832 §8).
pub const PPID_DCEP: u32 = 50;

const MSG_OPEN: u8 = 0x03;
const MSG_ACK: u8 = 0x02;
/// `OPEN` 의 고정 머리 — 형식(1)·종류(1)·우선순위(2)·신뢰(4)·label 길이(2)·protocol 길이(2).
const OPEN_FIXED: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dcep {
    Open { label: String },
    /// 클라가 보낸 ACK — ★**우리는 여는 쪽이 아니라 받는 쪽**이라 올 일이 없다.
    Ack,
}

/// ★**모양이 아니면 `None`** — 모르는 메시지에 답하지 않는다.
pub fn parse_dcep(data: &[u8]) -> Option<Dcep> {
    match *data.first()? {
        MSG_ACK => Some(Dcep::Ack),
        MSG_OPEN => {
            if data.len() < OPEN_FIXED {
                return None;
            }
            let label_len = u16::from_be_bytes([data[8], data[9]]) as usize;
            let proto_len = u16::from_be_bytes([data[10], data[11]]) as usize;
            if data.len() < OPEN_FIXED + label_len + proto_len {
                return None;
            }
            // ★이름은 사람이 고른 값이라 UTF-8 이 깨질 수 있다 — 던지지 않고 대체한다.
            let label = String::from_utf8_lossy(&data[OPEN_FIXED..OPEN_FIXED + label_len]).into_owned();
            Some(Dcep::Open { label })
        }
        _ => None,
    }
}

/// ★**한 바이트다** — 여는 쪽이 기다리는 것은 이것 하나다.
pub fn ack() -> Vec<u8> {
    vec![MSG_ACK]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 머리는_넷이고_판을_싣는다() {
        let w = build(SVC_FLOOR, b"hi").expect("짓는다");
        assert_eq!(&w[..4], &[VER, SVC_FLOOR, 0x00, 0x02]);
        assert_eq!(parse(&w), Some((SVC_FLOOR, &b"hi"[..])));
    }

    #[test]
    fn 잘린_것은_조용히_버린다() {
        let w = build(SVC_FLOOR, b"hello").expect("짓는다");
        // ★길이가 말하는 것보다 짧다 — 던지지 않는다.
        assert_eq!(parse(&w[..6]), None);
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&[VER, SVC_FLOOR, 0, 0][..3]), None);
    }

    #[test]
    fn 다른_판은_우리_것이_아니다() {
        let mut w = build(SVC_FLOOR, b"x").expect("짓는다");
        w[0] = 0x02;
        assert_eq!(parse(&w), None, "★첫 바이트의 뜻이 WS 와 같다");
    }

    #[test]
    fn 상한을_넘으면_보내는_쪽이_거부한다() {
        assert!(build(SVC_FLOOR, &vec![0u8; u16::MAX as usize]).is_some());
        assert!(build(SVC_FLOOR, &vec![0u8; u16::MAX as usize + 1]).is_none());
    }

    #[test]
    fn dcep_open_에서_이름을_읽는다() {
        let label = LABEL.as_bytes();
        let mut p = vec![MSG_OPEN, 0x81, 0, 0, 0, 0, 0, 1];
        p.extend_from_slice(&(label.len() as u16).to_be_bytes());
        p.extend_from_slice(&0u16.to_be_bytes());
        p.extend_from_slice(label);
        assert_eq!(parse_dcep(&p), Some(Dcep::Open { label: LABEL.into() }));
        // ★잘린 것은 안 읽는다 — 이름이 반만 온 것을 이름으로 쓰면 거부 판정이 뒤집힌다.
        assert_eq!(parse_dcep(&p[..p.len() - 2]), None);
    }

    #[test]
    fn dcep_ack_는_한_바이트다() {
        assert_eq!(ack(), vec![MSG_ACK]);
        assert_eq!(parse_dcep(&ack()), Some(Dcep::Ack));
        assert_eq!(parse_dcep(&[0x07]), None, "★모르는 것에 답하지 않는다");
    }
}
