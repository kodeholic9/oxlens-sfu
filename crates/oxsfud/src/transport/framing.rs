use bytes::{Buf, Bytes, BytesMut};

pub const LENGTH_PREFIX: usize = 2;
pub const MAX_FRAME: usize = u16::MAX as usize;

pub fn frame(payload: &[u8]) -> Option<Bytes> {
    if payload.len() > MAX_FRAME {
        return None;
    }
    let mut out = BytesMut::with_capacity(LENGTH_PREFIX + payload.len());
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    Some(out.freeze())
}

#[derive(Debug, Default)]
pub struct Unframer {
    buf: BytesMut,
}

impl Unframer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn next_frame(&mut self) -> Option<Bytes> {
        if self.buf.len() < LENGTH_PREFIX {
            return None;
        }
        let len = u16::from_be_bytes([self.buf[0], self.buf[1]]) as usize;
        if self.buf.len() < LENGTH_PREFIX + len {
            return None;
        }
        self.buf.advance(LENGTH_PREFIX);
        Some(self.buf.split_to(len).freeze())
    }

    pub fn pending(&self) -> usize {
        self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 씌운_것을_그대로_벗긴다() {
        let payload = b"\x00\x01\x02stun-ish";
        let wire = frame(payload).expect("65535 이하");
        assert_eq!(wire.len(), LENGTH_PREFIX + payload.len());
        let mut u = Unframer::new();
        u.push(&wire);
        assert_eq!(u.next_frame().as_deref(), Some(&payload[..]));
        assert_eq!(u.next_frame(), None);
    }

    #[test]
    fn 한_바이트씩_와도_다_모이면_나온다() {
        let payload = b"partial arrival";
        let wire = frame(payload).expect("65535 이하");
        let mut u = Unframer::new();
        for (i, b) in wire.iter().enumerate() {
            u.push(&[*b]);
            if i + 1 < wire.len() {
                assert_eq!(u.next_frame(), None, "덜 온 프레임을 내주면 안 된다");
            }
        }
        assert_eq!(u.next_frame().as_deref(), Some(&payload[..]));
    }

    #[test]
    fn 한_덩어리에_여러_프레임이_와도_다_뺀다() {
        let mut wire = Vec::new();
        for p in [&b"one"[..], &b"two"[..], &b"three"[..]] {
            wire.extend_from_slice(&frame(p).expect("65535 이하"));
        }
        let mut u = Unframer::new();
        u.push(&wire);
        let got: Vec<Vec<u8>> = std::iter::from_fn(|| u.next_frame()).map(|b| b.to_vec()).collect();
        assert_eq!(got, vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]);
        assert_eq!(u.pending(), 0);
    }

    #[test]
    fn 길이_0_은_빈_프레임이고_이것이_keepalive_다() {
        let wire = frame(b"").expect("빈 것도 프레임이다");
        assert_eq!(&wire[..], &[0u8, 0]);
        let mut u = Unframer::new();
        u.push(&wire);
        assert_eq!(u.next_frame().as_deref(), Some(&b""[..]));
        assert_eq!(u.pending(), 0);
    }

    #[test]
    fn 최대_길이는_65535_다() {
        let payload = vec![0xABu8; MAX_FRAME];
        let wire = frame(&payload).expect("경계는 들어간다");
        assert_eq!(&wire[..LENGTH_PREFIX], &[0xFF, 0xFF]);
        let mut u = Unframer::new();
        u.push(&wire);
        assert_eq!(u.next_frame().expect("나온다").len(), MAX_FRAME);
    }

    #[test]
    fn 넘치면_아예_안_만든다() {
        assert!(frame(&vec![0u8; MAX_FRAME + 1]).is_none());
    }

    #[test]
    fn 헤더만_와도_붙들고_기다린다() {
        let mut u = Unframer::new();
        u.push(&[0x00, 0x05]);
        assert_eq!(u.next_frame(), None);
        assert_eq!(u.pending(), LENGTH_PREFIX);
        u.push(b"hello");
        assert_eq!(u.next_frame().as_deref(), Some(&b"hello"[..]));
    }

    #[test]
    fn 프레임_뒤에_남은_조각은_다음_것이다() {
        let mut wire = frame(b"first").expect("65535 이하").to_vec();
        wire.extend_from_slice(&[0x00, 0x03, b'a']);
        let mut u = Unframer::new();
        u.push(&wire);
        assert_eq!(u.next_frame().as_deref(), Some(&b"first"[..]));
        assert_eq!(u.next_frame(), None);
        u.push(b"bc");
        assert_eq!(u.next_frame().as_deref(), Some(&b"abc"[..]));
    }
}
