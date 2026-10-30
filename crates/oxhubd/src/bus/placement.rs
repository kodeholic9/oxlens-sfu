// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-PLACEMENT · model: claude-opus-5-5
//! 배치 — HRW 순수 함수 · 해시 = FNV-1a 64 → splitmix64 마무리 · 입력 `node_id ‖ 0x00 ‖ room_id` · 동점은 `node_id` 사전순 앞.

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub fn splitmix_finish(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z ^= z >> 30;
    z = z.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z ^= z >> 27;
    z = z.wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    z
}

pub fn score(node: &str, room: &str) -> u64 {
    let mut buf = Vec::with_capacity(node.len() + 1 + room.len());
    buf.extend_from_slice(node.as_bytes());
    buf.push(0);
    buf.extend_from_slice(room.as_bytes());
    splitmix_finish(fnv1a64(&buf))
}

pub fn pick<'a, I: IntoIterator<Item = &'a str>>(nodes: I, room: &str) -> Option<String> {
    let mut best: Option<(u64, &str)> = None;
    for n in nodes {
        let s = score(n, room);
        best = match best {
            None => Some((s, n)),
            Some((bs, bn)) if s > bs || (s == bs && n < bn) => Some((s, n)),
            keep => keep,
        };
    }
    best.map(|(_, n)| n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_known_values() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn pure_and_order_free() {
        let a = pick(["node-a", "node-b", "node-c"], "r1");
        let b = pick(["node-c", "node-a", "node-b"], "r1");
        assert_eq!(a, b);
        assert!(a.is_some());
        assert_eq!(pick(Vec::<&str>::new(), "r1"), None);
    }

    #[test]
    fn removal_moves_only_its_rooms() {
        let three = ["node-a", "node-b", "node-c"];
        let two = ["node-a", "node-b"];
        for i in 0..200 {
            let r = format!("room-{i}");
            let p3 = pick(three, &r).unwrap();
            if p3 != "node-c" {
                assert_eq!(pick(two, &r).unwrap(), p3);
            }
        }
    }
}
