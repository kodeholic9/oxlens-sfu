// author: kodeholic (powered by Claude)
//! DCEP — RFC 8832. sctp-proto 는 순수 SCTP 라 채널 개통 규약은 여기서 다룬다.
//! 서버가 보는 것은 클라의 `DATA_CHANNEL_OPEN` 하나와 그에 대한 `DATA_CHANNEL_ACK` 회신뿐이다(연§9-7 클라가 연다).

const MSG_OPEN: u8 = 0x03;
const MSG_ACK: u8 = 0x02;
const OPEN_FIXED_LEN: usize = 12;

#[derive(Debug, PartialEq, Eq)]
pub enum Message {
    Open { label: String },
    Ack,
}

/// `None` = DCEP 이 아니거나 잘렸다.
pub fn parse(data: &[u8]) -> Option<Message> {
    match *data.first()? {
        MSG_ACK => Some(Message::Ack),
        MSG_OPEN => {
            if data.len() < OPEN_FIXED_LEN {
                return None;
            }
            let label_len = u16::from_be_bytes([data[8], data[9]]) as usize;
            let label = data.get(OPEN_FIXED_LEN..OPEN_FIXED_LEN + label_len)?;
            Some(Message::Open { label: String::from_utf8_lossy(label).into_owned() })
        }
        _ => None,
    }
}

pub fn ack() -> Vec<u8> {
    vec![MSG_ACK]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(label: &[u8], proto: &[u8]) -> Vec<u8> {
        let mut p = vec![MSG_OPEN, 0x81, 0, 0, 0, 0, 0, 0];
        p.extend_from_slice(&(label.len() as u16).to_be_bytes());
        p.extend_from_slice(&(proto.len() as u16).to_be_bytes());
        p.extend_from_slice(label);
        p.extend_from_slice(proto);
        p
    }

    #[test]
    fn open_and_ack() {
        assert_eq!(parse(&open(b"unreliable", b"")), Some(Message::Open { label: "unreliable".into() }));
        assert_eq!(parse(&open(b"x", b"proto")), Some(Message::Open { label: "x".into() }));
        assert_eq!(parse(&[MSG_ACK]), Some(Message::Ack));
        assert_eq!(ack(), vec![MSG_ACK]);
        assert_eq!(parse(&open(b"unreliable", b"")[..10]), None);
        assert_eq!(parse(&[0x09]), None);
        assert_eq!(parse(&[]), None);
    }
}
