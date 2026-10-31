// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-KEYSPACE · model: claude-opus-5-5
//! 키 식 — `*`(한 조각) · `**`(여러 조각) 둘만 · 겹침 · 포함은 zenoh 1.10.1 `commons/zenoh-keyexpr/src/key_expr/intersect/classical.rs` · `include.rs` 에서 가져와 `$*` · `@` 갈래를 걷었다(Apache-2.0 · crate NOTICE).

const FORBIDDEN: &[char] = &['$', '?', '#'];

pub fn valid(e: &str) -> bool {
    if e.is_empty() || e.contains(FORBIDDEN) {
        return false;
    }
    let mut prev_double = false;
    for c in e.split('/') {
        if c.is_empty() || (c.contains('*') && c != "*" && c != "**") {
            return false;
        }
        let double = c == "**";
        if double && prev_double {
            return false;
        }
        prev_double = double;
    }
    true
}

pub fn concrete(e: &str) -> bool {
    !e.contains('*')
}

fn next(s: &str) -> (&str, &str) {
    match s.find('/') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    }
}

pub fn intersects(mut a: &str, mut b: &str) -> bool {
    if a == b {
        return true;
    }
    while !a.is_empty() && !b.is_empty() {
        let (c1, r1) = next(a);
        let (c2, r2) = next(b);
        if c1 == "**" {
            if r1.is_empty() {
                return true;
            }
            return intersects(a, r2) || intersects(r1, b);
        }
        if c2 == "**" {
            if r2.is_empty() {
                return true;
            }
            return intersects(r1, b) || intersects(a, r2);
        }
        if c1 == c2 || c1 == "*" || c2 == "*" {
            a = r1;
            b = r2;
        } else {
            return false;
        }
    }
    (a.is_empty() || a == "**") && (b.is_empty() || b == "**")
}

pub fn includes(mut l: &str, mut r: &str) -> bool {
    if l == r {
        return true;
    }
    loop {
        let (lc, lr) = next(l);
        if lc == "**" {
            if lr.is_empty() || includes(lr, r) {
                return true;
            }
            r = next(r).1;
            if r.is_empty() {
                return false;
            }
        } else {
            let (rc, rr) = next(r);
            if rc.is_empty() || rc == "**" || !(lc == rc || lc == "*") {
                return false;
            }
            if lr.is_empty() {
                return rr.is_empty();
            }
            l = lr;
            r = rr;
        }
    }
}
