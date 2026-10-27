// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-TRANSPORT · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! ICE-TCP 프레이밍(순수) — RFC 4571: 2바이트 BE 길이 접두 · 최대 65,535 · 길이 0 허용(keepalive) · 부분 수신은 보류.

pub const IDENTIFY_MS: u64 = 10_000;
pub const KEEPALIVE_MS: u64 = 20_000;

#[derive(Debug, Clone, Default)]
pub struct Deframer {
    buf: Vec<u8>,
}

impl Deframer {
    pub fn new() -> Deframer {
        Deframer::default()
    }

    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        let mut i = 0;
        while self.buf.len() >= i + 2 {
            let len = usize::from(u16::from_be_bytes([self.buf[i], self.buf[i + 1]]));
            if self.buf.len() < i + 2 + len {
                break;
            }
            out.push(self.buf[i + 2..i + 2 + len].to_vec());
            i += 2 + len;
        }
        self.buf.drain(..i);
        out
    }
}

pub fn frame(payload: &[u8]) -> Option<Vec<u8>> {
    let len = u16::try_from(payload.len()).ok()?;
    let mut out = Vec::with_capacity(2 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_and_zero_length() {
        let mut d = Deframer::new();
        let mut wire = frame(b"abc").expect("f");
        wire.extend_from_slice(&frame(b"").expect("ka"));
        wire.extend_from_slice(&[0, 4, b'x']);
        let got = d.push(&wire);
        assert_eq!(got, vec![b"abc".to_vec(), Vec::new()]);
        assert_eq!(d.push(b"yzw"), vec![b"xyzw".to_vec()]);
        assert!(frame(&vec![0u8; 70_000]).is_none());
    }
}
