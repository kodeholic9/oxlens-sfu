// author: kodeholic (powered by Claude)
//! 진짜 코덱 표본 읽기(예제 agent `file` 형상 — context 202611/20261108d R9) — Ogg Opus 패킷 · IVF(VP8) 프레임 · H264 Annex-B 접근 단위.
//! 표본은 ffmpeg 가 짓는다(oxpeer `scripts/media_check.sh` 와 같은 꼴 — H264 는 `aud=1` 로 접근 단위 경계가 AUD).

use bytes::Bytes;

/// Ogg Opus — 머리 두 패킷(OpusHead · OpusTags)은 뺀다
pub fn read_ogg(b: &[u8]) -> Vec<Bytes> {
    let (mut at, mut cur, mut out) = (0, Vec::new(), Vec::new());
    while at + 27 <= b.len() && &b[at..at + 4] == b"OggS" {
        let n = usize::from(b[at + 26]);
        let mut body = at + 27 + n;
        for &x in &b[at + 27..at + 27 + n] {
            cur.extend_from_slice(&b[body..body + usize::from(x)]);
            body += usize::from(x);
            if x < 255 {
                out.push(Bytes::from(std::mem::take(&mut cur)));
            }
        }
        at = body;
    }
    out.split_off(2.min(out.len()))
}

/// IVF(VP8) — (프레임, 키프레임)
pub fn read_ivf(b: &[u8]) -> Vec<(Bytes, bool)> {
    if b.len() < 32 || &b[..4] != b"DKIF" {
        return Vec::new();
    }
    let mut i = usize::from(u16::from_le_bytes([b[6], b[7]]));
    let mut out = Vec::new();
    while i + 12 <= b.len() {
        let n = u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        let Some(f) = b.get(i + 12..i + 12 + n) else { break };
        out.push((Bytes::copy_from_slice(f), f.first().is_some_and(|x| x & 1 == 0)));
        i += 12 + n;
    }
    out
}

/// H264 Annex-B — AUD(9) 를 경계로 접근 단위마다 (AUD 뺀 Annex-B, IDR 있음)
pub fn read_h264(b: &[u8]) -> Vec<(Bytes, bool)> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut aus: Vec<(Vec<u8>, bool)> = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let mut e = starts.get(k + 1).map(|n| n - 3).unwrap_or(b.len());
        while e > s && b[e - 1] == 0 {
            e -= 1;
        }
        let nal = &b[s..e];
        let Some(&h) = nal.first() else { continue };
        if h & 0x1f == 9 {
            aus.push((Vec::new(), false));
            continue;
        }
        if let Some(a) = aus.last_mut() {
            a.0.extend_from_slice(&[0, 0, 0, 1]);
            a.0.extend_from_slice(nal);
            a.1 |= h & 0x1f == 5;
        }
    }
    aus.into_iter().filter(|a| !a.0.is_empty()).map(|(a, k)| (Bytes::from(a), k)).collect()
}
