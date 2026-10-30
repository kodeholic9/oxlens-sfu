// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-LINK-TRANSPORT · 정:RULE-FWD-MEDIA-WORKERS · model: claude-opus-5-5
//! RFC 4571 프레이밍 — 2바이트 BE 길이 접두 · 최대 65,535 · 길이 0 허용(keepalive) · 부분 수신은 보류.

pub const KEEPALIVE_MS: u64 = 20_000;
pub const IDENTIFY_MS: u64 = 10_000;
pub const SEND_BUF: usize = 64 * 1024;

#[derive(Debug, Default)]
pub struct Deframer {
    buf: Vec<u8>,
}

impl Deframer {
    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            if self.buf.len() < 2 {
                return None;
            }
            let n = u16::from_be_bytes([self.buf[0], self.buf[1]]) as usize;
            if self.buf.len() < 2 + n {
                return None;
            }
            let f = self.buf[2..2 + n].to_vec();
            self.buf.drain(..2 + n);
            if n == 0 {
                continue;
            }
            return Some(f);
        }
    }
}

pub fn frame(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > 65_535 {
        return None;
    }
    let mut out = Vec::with_capacity(2 + payload.len());
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_and_zero() {
        let mut d = Deframer::default();
        d.push(&[0, 0, 0, 3, 1]);
        assert_eq!(d.next_frame(), None);
        d.push(&[2, 3]);
        assert_eq!(d.next_frame(), Some(vec![1, 2, 3]));
        assert_eq!(frame(&[9]), Some(vec![0, 1, 9]));
    }
}
