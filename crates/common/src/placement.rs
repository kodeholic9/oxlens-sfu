// author: kodeholic (powered by Claude)
// spec: v1.2 @250e836 · 정:RULE-BUS-PLACEMENT · model: claude-opus-5-5
//! 방 배치 — HRW 순수 함수. 해시는 계약이다: FNV-1a 64(`node_id ‖ 0x00 ‖ room_id`) → splitmix64 한 걸음(감마 덧셈부터).

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

pub fn score(node_id: &str, room_id: &str) -> u64 {
    let mut h = FNV_OFFSET;
    for b in node_id.as_bytes().iter().chain([0u8].iter()).chain(room_id.as_bytes().iter()) {
        h ^= u64::from(*b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    mix(h)
}

fn mix(h: u64) -> u64 {
    let mut z = h.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub fn place<'a, I>(nodes: I, room_id: &str) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut best: Option<(u64, &str)> = None;
    for n in nodes {
        let s = score(n, room_id);
        best = match best {
            None => Some((s, n)),
            Some((bs, bn)) if s > bs || (s == bs && n < bn) => Some((s, n)),
            keep => keep,
        };
    }
    best.map(|(_, n)| n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_empty_input_is_offset_mixed() {
        let mut h = FNV_OFFSET;
        h ^= 0;
        h = h.wrapping_mul(FNV_PRIME);
        assert_eq!(score("", ""), mix(h));
    }

    #[test]
    fn same_answer_as_layer2_harness() {
        assert_eq!(score("node-a", "r1"), 0x0a17_39dd_61bc_71f1);
        assert_eq!(score("node-b", "nd1_3"), 0xbd17_cd18_93fe_bf09);
        assert_eq!(score("node-c", "smoke-1"), 0xae3e_1535_7ed9_458a);
    }

    #[test]
    fn deterministic_and_order_free() {
        let a = place(["node-a", "node-b", "node-c"], "r1");
        let b = place(["node-c", "node-a", "node-b"], "r1");
        assert_eq!(a, b);
        assert!(a.is_some());
        assert_eq!(place(std::iter::empty::<&str>(), "r1"), None);
    }
}
