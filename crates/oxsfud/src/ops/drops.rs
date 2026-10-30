// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-OPS-DROPS · 정:RULE-OPS-OBSERVE · 정:IMPL-ERR-ISSUERS · 운영:RULE-READ-DROPS · model: claude-opus-5-5
//! 버림 계수 — 이름은 닫힌 목록(`RULE-OPS-DROPS`) 그대로 · 실패 응답은 `code_<번호>` 로 따로 센다. 핫패스는 원자값 하나를 올린다.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

macro_rules! drops {
    ($($name:ident),* $(,)?) => {
        #[derive(Debug, Default)]
        pub struct Drops {
            $(pub $name: AtomicU64,)*
            codes: Mutex<BTreeMap<u16, u64>>,
        }

        impl Drops {
            pub fn rows(&self) -> Vec<(String, u64)> {
                let mut v: Vec<(String, u64)> = vec![$((stringify!($name).to_string(), self.$name.load(Ordering::Relaxed)),)*];
                if let Ok(c) = self.codes.lock() {
                    for (k, n) in c.iter() {
                        v.push((format!("code_{k}"), *n));
                    }
                }
                v
            }
        }
    };
}

drops!(
    demux_unknown,
    stun_unverified,
    tcp_unidentified,
    send_fail,
    ingress_queue_full,
    egress_queue_full,
    transport_gone,
    oversize,
    rtp_unknown_path,
    srtp_not_ready,
    srtp_fail,
    srtcp_fail,
    rtp_malformed,
    rtp_unknown_ssrc,
    rid_unknown,
    no_pub_room,
    prefan_gate,
    ready_gate,
    row_gate,
    await_keyframe,
    rtcp_unresolved,
    pli_throttled,
    nack_audio,
    rtx_gate,
    rtx_cache_miss,
    rtx_budget,
    dc_bad_frame,
    dc_unknown_svc,
    dc_no_room,
    bus_ev_drop,
);

pub static DROPS: std::sync::LazyLock<Drops> = std::sync::LazyLock::new(Drops::default);

pub fn bump(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

impl Drops {
    pub fn code(&self, code: u16) {
        if let Ok(mut c) = self.codes.lock() {
            let n = c.entry(code).or_insert(0);
            let next = n.saturating_add(1);
            c.insert(code, next);
        }
    }

    pub fn body(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        for (k, v) in self.rows() {
            m.insert(k, serde_json::json!(v));
        }
        for (k, v) in common::log::sender::drops() {
            m.insert(k.to_string(), serde_json::json!(v));
        }
        serde_json::Value::Object(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_the_closed_list() {
        let d = Drops::default();
        bump(&d.prefan_gate);
        d.code(3001);
        let rows = d.rows();
        assert!(rows.iter().any(|(k, v)| k == "prefan_gate" && *v == 1));
        assert!(rows.iter().any(|(k, v)| k == "code_3001" && *v == 1));
        assert_eq!(rows.len(), 31);
    }
}
