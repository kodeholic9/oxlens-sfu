// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-BUS-PLACEMENT · 정:RULE-OPS-SURFACE · model: claude-opus-5-5
//! 배치 — HRW(rendezvous) 순수 함수. 키 = FNV-1a 64(`node_id ‖ 0x00 ‖ room_id`) → splitmix64 한 걸음 · 최대가 이긴다 · 동점은 `node_id` 사전순 앞.

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;
const MIX1: u64 = 0xbf58_476d_1ce4_e5b9;
const MIX2: u64 = 0x94d0_49bb_1331_11eb;

pub fn score(node_id: &str, room_id: &str) -> u64 {
    let mut z = FNV_OFFSET;
    for b in node_id.as_bytes().iter().chain(std::iter::once(&0u8)).chain(room_id.as_bytes()) {
        z ^= u64::from(b.to_owned());
        z = z.wrapping_mul(FNV_PRIME);
    }
    z = z.wrapping_add(GAMMA);
    z ^= z >> 30;
    z = z.wrapping_mul(MIX1);
    z ^= z >> 27;
    z = z.wrapping_mul(MIX2);
    z ^= z >> 31;
    z
}

pub fn place<'a>(room_id: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(&str, u64)> = None;
    for n in candidates {
        let s = score(n, room_id);
        best = match best {
            None => Some((n, s)),
            Some((bn, bs)) if s > bs || (s == bs && n < bn) => Some((n, s)),
            keep => keep,
        };
    }
    best.map(|(n, _)| n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_the_contract_formula() {
        assert_eq!(score("node-a", "r1"), 727_113_487_833_919_985);
        assert_eq!(score("node-b", "r1"), 11_024_237_899_649_879_443);
        assert_eq!(place("r1", ["node-a", "node-b", "node-c"]), Some("node-b"));
        assert_eq!(place("r1", ["node-c", "node-b", "node-a"]), Some("node-b"));
        assert_eq!(place("r1", std::iter::empty()), None);
    }
}
