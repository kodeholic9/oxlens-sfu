// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§7-2-1 · 연§4-2-1 · model: claude-opus-5

//! 구독자 PT 표 — ★**연결(Peer)마다 하나다.**
//!
//! ★★**egress 의 PT 는 구독자 표의 값이다** — 발행자 PT 를 그대로 내보내지 않는다(정§7-2-1).
//! 한 BUNDLE 안에서 PT 는 유일해야 하므로(RFC 8843), 발행자마다 다른 번호를 그대로 흘리면
//! 두 발행자가 같은 번호를 들고 와서 충돌한다.
//!
//! ★**키는 `fmtp` 원문이 아니다** — 이름·클럭·채널에 ★**구성/패킷화 파라미터**만 뽑아 정규화한
//! 것이고, ★**받는 쪽 선호는 키에서 버린다**(연§4-2-1 ① — RFC 8843 §9.1.1 · RFC 7587 §6.1).
//! 원문을 키로 쓰면 같은 코덱이 `usedtx` 하나 다르다고 새 PT 를 먹어 예산이 금세 마른다.

use std::collections::BTreeMap;

use oxsig::types::Kind;

/// 씨앗 — ★★**그 연결에서 처음 본 그 코덱의 구성 변종이 이 번호를 갖는다**(정§7-2-1).
///
/// ★**"fmtp 가 없는 튜플" 이 아니다** — 이름에 걸린 **예약**이고, 그 이름의 첫 튜플이 가져간다.
/// 그래서 `packetization-mode=1` 짜리 H264 하나만 오는 흔한 형상에서 그것이 `102/103` 을 쓴다.
const SEED: &[(Kind, &str, u8, Option<u8>)] = &[
    (Kind::Audio, "OPUS", 111, None),
    (Kind::Video, "VP8", 96, Some(97)),
    (Kind::Video, "H264", 102, Some(103)),
];

/// ★**배정 범위 — `96~127` 먼저, 모자라면 `35~63`**(정§7-2-1 · 17차 실측).
///
/// `64~95` 는 두 브라우저가 협상에서 거부하고, `0~34` 는 Chromium 이 그 m-line 을
/// ★**포트 0 으로 조용히 죽인다** — 그래서 둘 다 안 쓴다.
const RANGES: &[(u8, u8)] = &[(96, 127), (35, 63)];

/// ★**받는 쪽 선호** — 키에서 버린다(연§4-2-1 ①).
const RECEIVER_PREFS: &[&str] = &[
    "usedtx",
    "useinbandfec",
    "maxaveragebitrate",
    "minptime",
    "stereo",
    "maxplaybackrate",
    "cbr",
];

/// 코덱 튜플 하나 — ★**이것이 키다.**
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tuple {
    pub kind: Kind,
    /// 대문자로 맞춘 이름 — `vp8` 과 `VP8` 은 같은 코덱이다.
    pub name: String,
    /// ★**구성 파라미터만** 남긴 것. 정렬돼 있어 순서가 달라도 같은 키다.
    pub config: String,
}

impl Tuple {
    pub fn new(kind: Kind, name: &str, fmtp: Option<&str>) -> Self {
        Self { kind, name: name.to_ascii_uppercase(), config: normalize(fmtp) }
    }
}

/// ★**받는 쪽 선호를 버리고 나머지를 정렬해 붙인다.**
///
/// ★`opus` 는 그래서 한 튜플이다 — `fmtp` 원문이 달라도 구성(`sprop-stereo`)이 같으면
/// 같은 키이고, 그 연결에서 PT 하나(`111`)를 쓴다. 씨앗이 밀리는 일이 없다(정§7-2-1).
fn normalize(fmtp: Option<&str>) -> String {
    let Some(raw) = fmtp else { return String::new() };
    let mut kept: Vec<String> = raw
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .filter(|p| {
            let key = p.split('=').next().unwrap_or(p).trim().to_ascii_lowercase();
            !RECEIVER_PREFS.contains(&key.as_str())
        })
        .map(|p| p.to_ascii_lowercase())
        .collect();
    kept.sort();
    kept.join(";")
}

/// 한 구독 연결의 표. ★**같은 튜플은 그 연결에서 늘 같은 PT** 다.
#[derive(Debug, Clone)]
pub struct PtTable {
    map: BTreeMap<Tuple, (u8, Option<u8>)>,
    /// 아직 임자 없는 씨앗 — `(kind, 이름, pt, rtx)`.
    seeds: Vec<(Kind, &'static str, u8, Option<u8>)>,
}

impl Default for PtTable {
    fn default() -> Self {
        Self::new()
    }
}

impl PtTable {
    /// 씨앗을 ★**예약으로** 들고 시작한다(아직 아무 튜플의 것도 아니다).
    pub fn new() -> Self {
        Self { map: BTreeMap::new(), seeds: SEED.to_vec() }
    }

    /// ★`1pc` — 그 Peer 의 발행 PT 가 같은 BUNDLE 이므로 ★**번호표를 씨앗으로 먼저 넣는다**
    /// (연§6-2 `ROOM_JOIN` 요청의 `codecs`). 신고된 PT 는 다른 튜플에 주지 않는다.
    pub fn seed_from_client(&mut self, kind: Kind, name: &str, fmtp: Option<&str>, pt: u8) {
        let key = Tuple::new(kind, name, fmtp);
        // ★그 번호를 쥔 다른 튜플·씨앗이 있으면 비켜 준다 — 클라 번호가 이긴다.
        self.map.retain(|k, v| k == &key || v.0 != pt);
        self.seeds.retain(|(_, _, p, r)| *p != pt && *r != Some(pt));
        self.map.insert(key, (pt, None));
    }

    /// 그 튜플의 `(pt, rtx_pt)`. ★**없으면 그 자리에서 발급**한다.
    pub fn get_or_assign(&mut self, key: &Tuple, want_rtx: bool) -> Option<(u8, Option<u8>)> {
        if let Some(v) = self.map.get(key) {
            return Some(*v);
        }
        // ★**그 이름의 첫 튜플이 씨앗을 가져간다** — 둘째 변종부터 새로 발급받는다.
        if let Some(i) = self.seeds.iter().position(|(k, n, ..)| *k == key.kind && *n == key.name) {
            let (_, _, pt, rtx) = self.seeds.remove(i);
            let v = (pt, if want_rtx { rtx } else { None });
            self.map.insert(key.clone(), v);
            return Some(v);
        }
        let pt = self.next_free()?;
        let rtx = if want_rtx {
            // ★짝을 같이 잡는다 — 하나만 잡으면 재전송이 조용히 안 간다.
            self.map.insert(key.clone(), (pt, None));
            let r = self.next_free();
            self.map.remove(key);
            r
        } else {
            None
        };
        if want_rtx && rtx.is_none() {
            return None;
        }
        self.map.insert(key.clone(), (pt, rtx));
        Some((pt, rtx))
    }

    /// ★**예약된 씨앗도 "쓰는 중"이다** — 안 그러면 둘째 변종이 아직 안 온 코덱의
    /// 번호를 먼저 집어 가고, 그 코덱이 왔을 때 씨앗이 이미 없다.
    fn taken(&self, pt: u8) -> bool {
        self.map.values().any(|(p, r)| *p == pt || *r == Some(pt))
            || self.seeds.iter().any(|(_, _, p, r)| *p == pt || *r == Some(pt))
    }

    /// ★**`96` 위로 다음 빈 짝, 모자라면 `35~63`** — 범위 순서가 계약이다.
    fn next_free(&self) -> Option<u8> {
        RANGES.iter().flat_map(|(a, b)| *a..=*b).find(|pt| !self.taken(*pt))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 씨앗은_그대로_나온다() {
        let mut t = PtTable::new();
        assert_eq!(t.get_or_assign(&Tuple::new(Kind::Audio, "opus", None), false), Some((111, None)));
        assert_eq!(t.get_or_assign(&Tuple::new(Kind::Video, "vp8", None), true), Some((96, Some(97))));
        assert_eq!(t.get_or_assign(&Tuple::new(Kind::Video, "H264", None), true), Some((102, Some(103))));
    }

    #[test]
    fn 받는_쪽_선호는_키를_안_가른다() {
        let mut t = PtTable::new();
        let a = Tuple::new(Kind::Audio, "opus", Some("minptime=10;useinbandfec=1"));
        let b = Tuple::new(Kind::Audio, "opus", Some("usedtx=1"));
        // ★opus 는 한 튜플이다 — 셋 다 `111` 이다(씨앗이 안 밀린다).
        assert_eq!(a, b);
        assert_eq!(t.get_or_assign(&a, false), Some((111, None)));
        assert_eq!(t.get_or_assign(&b, false), Some((111, None)));
        assert_eq!(t.len(), 1, "★둘이 한 자리를 쓴다");
    }

    #[test]
    fn 구성_파라미터가_다르면_다른_튜플이다() {
        let mut t = PtTable::new();
        let base = Tuple::new(Kind::Video, "H264", Some("packetization-mode=1;profile-level-id=42e01f"));
        let other = Tuple::new(Kind::Video, "H264", Some("packetization-mode=0;profile-level-id=42e01f"));
        assert_ne!(base, other);
        // 첫 변종이 씨앗 자리를 갖고, 다음 변종은 96 위로 다음 빈 짝이다.
        assert_eq!(t.get_or_assign(&base, true), Some((102, Some(103))));
        // ★아직 임자 없는 씨앗(VP8 96/97)도 "쓰는 중"이라 건너뛴다 — 뒤에 올 VP8 의 몫이다.
        assert_eq!(t.get_or_assign(&other, true), Some((98, Some(99))));
        assert_eq!(t.get_or_assign(&Tuple::new(Kind::Video, "VP8", None), true), Some((96, Some(97))));
        // ★같은 튜플은 늘 같은 PT 다 — 여기가 깨지면 egress 가 프레임마다 번호를 바꾼다.
        assert_eq!(t.get_or_assign(&base, true), Some((102, Some(103))));
    }

    #[test]
    fn 순서가_달라도_같은_키다() {
        let a = Tuple::new(Kind::Video, "H264", Some("profile-level-id=42e01f;packetization-mode=1"));
        let b = Tuple::new(Kind::Video, "H264", Some("packetization-mode=1; profile-level-id=42e01f"));
        assert_eq!(a, b, "★정렬해 붙이므로 순서는 뜻이 아니다");
    }

    #[test]
    fn 클라_번호표가_이긴다() {
        let mut t = PtTable::new();
        // ★`1pc` — 클라가 `111` 을 제 video 에 쓰고 있다면 opus 씨앗이 비켜야 한다.
        t.seed_from_client(Kind::Video, "VP8", None, 111);
        let (pt, _) = t.get_or_assign(&Tuple::new(Kind::Video, "vp8", None), false).expect("있다");
        assert_eq!(pt, 111);
        let (opus, _) = t.get_or_assign(&Tuple::new(Kind::Audio, "opus", None), false).expect("발급");
        assert_ne!(opus, 111, "★신고된 PT 는 다른 튜플에 안 준다");
    }

    #[test]
    fn 금지_대역은_안_쓴다() {
        let t = PtTable::new();
        let pt = t.next_free().expect("있다");
        // ★`64~95` 는 브라우저가 거부하고 `0~34` 는 조용히 죽는다.
        assert!((96..=127).contains(&pt) || (35..=63).contains(&pt), "{pt}");
    }
}
