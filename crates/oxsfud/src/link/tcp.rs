// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-LINK-TRANSPORT 정:RULE-CONC-SANS-IO 정:RULE-DOC-FIXED-PARTS · model: claude-opus-5-5
//! RFC 4571 — 2바이트 BE 길이 접두 · 최대 65,535 · 길이 0 허용(keepalive) · 부분 수신은 보류.

#[derive(Debug, Default)]
pub struct Deframer {
    buf: Vec<u8>,
}

impl Deframer {
    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        if self.buf.len() < 2 {
            return None;
        }
        let len = u16::from_be_bytes([self.buf[0], self.buf[1]]) as usize;
        if self.buf.len() < 2 + len {
            return None;
        }
        let f = self.buf[2..2 + len].to_vec();
        self.buf.drain(..2 + len);
        Some(f)
    }
}

pub fn frame(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > u16::MAX as usize {
        return None;
    }
    let mut out = Vec::with_capacity(payload.len() + 2);
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

pub fn demux_ok(first: u8) -> bool {
    first < 4 || (20..=63).contains(&first) || (128..=191).contains(&first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_frames_wait() {
        let mut d = Deframer::default();
        let f = frame(&[1, 2, 3]).expect("frame");
        d.push(&f[..3]);
        assert!(d.next_frame().is_none());
        d.push(&f[3..]);
        d.push(&[0, 0]);
        assert_eq!(d.next_frame(), Some(vec![1, 2, 3]));
        assert_eq!(d.next_frame(), Some(vec![]));
        assert!(d.next_frame().is_none());
    }
}
