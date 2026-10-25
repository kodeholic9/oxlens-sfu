// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-OPS-OBSERVE 정:IMPL-ERR-ISSUERS 운영:RULE-READ-DROPS · model: claude-opus-5-5
//! 사유별 버림 계수 — 버리는 전 경로가 이름 있는 계수기를 갖는다(조용한 drop 금지) · 실패 코드마다 계수.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

macro_rules! drops {
    ($($name:ident),* $(,)?) => {
        #[derive(Default)]
        pub struct Drops {
            $(pub $name: AtomicU64,)*
            codes: std::sync::Mutex<BTreeMap<u16, u64>>,
        }

        impl Drops {
            pub fn map(&self) -> BTreeMap<String, u64> {
                let mut m = BTreeMap::new();
                $(m.insert(stringify!($name).to_string(), self.$name.load(Ordering::Relaxed));)*
                if let Ok(c) = self.codes.lock() {
                    for (k, v) in c.iter() {
                        m.insert(format!("code_{k}"), *v);
                    }
                }
                m
            }
        }
    };
}

drops!(
    demux_unknown,
    stun_malformed,
    stun_unknown_ufrag,
    stun_integrity_fail,
    tcp_unidentified,
    tcp_bad_frame,
    dtls_unknown_path,
    dtls_queue_full,
    dtls_not_ready,
    rtp_unknown_path,
    srtp_not_ready,
    srtp_fail,
    srtcp_fail,
    rtp_malformed,
    rtp_unknown_ssrc,
    rid_unknown,
    pt_mismatch,
    no_pub_room,
    prefan_drop,
    ready_gate,
    ack_gate,
    await_keyframe,
    pli_throttled,
    rtcp_unresolved,
    rtx_no_rtx,
    rtx_gate,
    rtx_cache_miss,
    rtx_budget,
    rtx_sent,
    nack_up_sent,
    sim_switch,
    probe_sent,
    dc_no_room,
    dc_bad_frame,
    dc_unknown_svc,
    dc_overflow,
    num_tags_ignored,
    stall_sent,
    gate_timeout,
    bus_put_drop,
);

impl Drops {
    pub fn code(&self, c: u16) {
        if let Ok(mut m) = self.codes.lock() {
            let e = m.entry(c).or_insert(0);
            let v = e.saturating_add(1);
            let _ = std::mem::replace(e, v);
        }
    }
}
