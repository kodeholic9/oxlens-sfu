// author: kodeholic (powered by Claude)
//! 구독자 PT 표 — 정§7-2-1. ★egress PT 는 발행자 값이 아니라 **구독 연결마다 하나인 이 표**의 값이다.
//! 같은 `(codec, fmtp)` 는 그 연결에서 늘 같은 PT 라 화자 교대에 재협상이 없고,
//! 한 BUNDLE 안 PT 유일(RFC 8843)이 구조적으로 성립한다.

use std::collections::BTreeMap;

use oxsig::body::media::CodecLine;

/// 씨앗 — 처음 본 그 코덱 변종이 가져간다.
const SEEDS: [(&str, u8, Option<u8>); 3] = [("opus", 111, None), ("VP8", 96, Some(97)), ("H264", 102, Some(103))];
/// 동적 배정 구간 — 96 위부터, 모자라면 35~63.
const DYNAMIC: [(u8, u8); 2] = [(96, 127), (35, 63)];

fn key(codec: &str, fmtp: Option<&str>) -> String {
    format!("{codec}|{}", fmtp.unwrap_or_default())
}

#[derive(Debug, Default)]
pub struct PtTable {
    assigned: BTreeMap<String, (u8, Option<u8>)>,
    used: std::collections::BTreeSet<u8>,
}

impl PtTable {
    /// 정§7-2-1 — 1pc 는 `READY{transport}` 신고 `codecs` 를 씨앗으로 **먼저** 넣는다.
    /// 신고된 PT 는 다른 튜플에 주지 않는다. 반환: 앞서 배정한 값과 어긋나 재배정된 키들.
    pub fn seed_from_ready(&mut self, codecs: &[CodecLine]) -> Vec<String> {
        let mut moved = Vec::new();
        for line in codecs {
            let k = key(&line.name, line.fmtp.as_deref());
            let want = (line.pt, line.rtx_pt);
            match self.assigned.get(&k) {
                Some(have) if *have == want => continue,
                Some(have) => {
                    self.release(*have);
                    moved.push(k.clone());
                }
                None => {}
            }
            self.take(k, want);
        }
        moved
    }

    /// 그 튜플의 PT — 없으면 배정한다. 고갈이면 `None`.
    pub fn assign(&mut self, codec: &str, fmtp: Option<&str>, want_rtx: bool) -> Option<(u8, Option<u8>)> {
        let k = key(codec, fmtp);
        if let Some(v) = self.assigned.get(&k) {
            return Some(*v);
        }
        let seed = SEEDS.iter().find(|(name, ..)| name.eq_ignore_ascii_case(codec));
        let pair = seed
            .filter(|(_, pt, rtx)| self.free(*pt) && rtx.is_none_or(|r| self.free(r)))
            .map(|(_, pt, rtx)| (*pt, rtx.filter(|_| want_rtx)))
            .or_else(|| self.next_free(want_rtx))?;
        self.take(k, pair);
        Some(pair)
    }

    pub fn get(&self, codec: &str, fmtp: Option<&str>) -> Option<(u8, Option<u8>)> {
        self.assigned.get(&key(codec, fmtp)).copied()
    }

    fn free(&self, pt: u8) -> bool {
        !self.used.contains(&pt)
    }

    fn take(&mut self, k: String, pair: (u8, Option<u8>)) {
        self.used.insert(pair.0);
        if let Some(r) = pair.1 {
            self.used.insert(r);
        }
        self.assigned.insert(k, pair);
    }

    fn release(&mut self, pair: (u8, Option<u8>)) {
        self.used.remove(&pair.0);
        if let Some(r) = pair.1 {
            self.used.remove(&r);
        }
    }

    fn next_free(&self, want_rtx: bool) -> Option<(u8, Option<u8>)> {
        for (lo, hi) in DYNAMIC {
            for pt in lo..=hi {
                if !self.free(pt) {
                    continue;
                }
                if !want_rtx {
                    return Some((pt, None));
                }
                if pt < hi && self.free(pt + 1) {
                    return Some((pt, Some(pt + 1)));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_go_to_the_first_variant_then_next_free_pair() {
        let mut t = PtTable::default();
        assert_eq!(t.assign("opus", None, false), Some((111, None)));
        assert_eq!(t.assign("opus", None, false), Some((111, None)), "같은 튜플은 늘 같은 PT");
        assert_eq!(t.assign("VP8", None, true), Some((96, Some(97))));
        assert_eq!(t.assign("H264", Some("profile-level-id=42e01f"), true), Some((102, Some(103))));
        let other = t.assign("H264", Some("profile-level-id=640c1f"), true).unwrap();
        assert_eq!(other, (98, Some(99)), "그 밖의 변종은 96 위로 다음 빈 짝");
        assert_eq!(t.get("H264", Some("profile-level-id=42e01f")), Some((102, Some(103))));
        assert_eq!(t.get("H264", None), None, "fmtp 가 다르면 다른 튜플");
    }

    #[test]
    fn ready_report_seeds_first_and_reports_moves() {
        let mut t = PtTable::default();
        assert_eq!(t.assign("VP8", None, true), Some((96, Some(97))));
        let moved = t.seed_from_ready(&[
            CodecLine { pt: 100, name: "VP8".into(), fmtp: None, rtx_pt: Some(101) },
            CodecLine { pt: 111, name: "opus".into(), fmtp: None, rtx_pt: None },
        ]);
        assert_eq!(moved, vec!["VP8|".to_owned()], "어긋난 것만 재배정 대상");
        assert_eq!(t.assign("VP8", None, true), Some((100, Some(101))));
        assert_eq!(t.assign("opus", None, false), Some((111, None)));
        let fresh = t.assign("H264", None, true).unwrap();
        assert_eq!(fresh, (102, Some(103)), "신고에 없는 코덱은 제 씨앗을 쓴다");
        assert!(t.seed_from_ready(&[CodecLine { pt: 100, name: "VP8".into(), fmtp: None, rtx_pt: Some(101) }]).is_empty());
    }

    #[test]
    fn exhaustion_is_reported_not_hidden() {
        let mut t = PtTable::default();
        for i in 0..(96..=127u8).len() + (35..=63u8).len() {
            assert!(t.assign(&format!("X{i}"), None, false).is_some(), "{i}");
        }
        assert_eq!(t.assign("Xover", None, false), None);
    }
}
