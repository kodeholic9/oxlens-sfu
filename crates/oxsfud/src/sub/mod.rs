// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-SUB-READY-GATE · 정:IMPL-SUB-PIPELINE · 정:RULE-SUB-STATE · 정:IMPL-PUB-RID-SPATIAL · 연:RULE-SUB-READY-PROC · 연:RULE-SUB-TGATE · 연:RULE-SUB-LAYER-PROC · 연:LINK.S.ack · 정:RULE-SUB-SERVER-SCOPE · model: claude-opus-5-5
//! SUB — `READY`(게이트 해제 · 확인된 차례 · 처음 알리기 · `ready_at` 은 첫 번째만) · 게이트 시한(관측 latch) · `SUBSCRIBE_LAYER`(부분 갱신 · 상한 · 범위 밖은 그 축 최대로 자른다).

pub mod ext;

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use oxsig::sub::LayerTarget;

use crate::flr::shell::Floor;
use crate::link::{lock, Gate, LayerPref, Peer};

pub const SPATIAL_MAX: u8 = 1;
pub const TEMPORAL_MAX: u8 = 0;

pub fn gate_close(p: &Peer, room: &str, now: u64) {
    lock(&p.gates).insert(room.to_string(), Gate { open: false, gated_at: now, first_ready_at: 0, warned: false });
    let r = room.to_string();
    p.gate_closed.rcu(|m| {
        let mut m: BTreeMap<String, u64> = (**m).clone();
        m.insert(r.clone(), now);
        m
    });
}

fn gate_open(p: &Peer, room: &str) {
    let r = room.to_string();
    p.gate_closed.rcu(|m| {
        let mut m: BTreeMap<String, u64> = (**m).clone();
        m.remove(&r);
        m
    });
}

pub fn gate_timeout(p: &Peer, room: &str) {
    let warn = {
        let mut g = lock(&p.gates);
        match g.get_mut(room) {
            Some(x) if !x.warned => {
                x.warned = true;
                x.open = true;
                true
            }
            _ => false,
        }
    };
    if warn {
        tracing::warn!(kind = "diag", user = %p.user_id, room = %room, "READY 미수신 의심 — 게이트 시한 해제(키프레임 요청 없음)");
    }
    gate_open(p, room);
}

pub fn gate_drop(p: &Peer, room: &str) {
    lock(&p.gates).remove(room);
    gate_open(p, room);
}

pub struct Ready {
    pub first: bool,
    pub acked: bool,
}

pub fn ready(p: &Arc<Peer>, room: &str, arev: u64, floor: &Arc<Floor>, now: u64) -> Ready {
    let first = {
        let mut g = lock(&p.gates);
        let e = g.entry(room.to_string()).or_default();
        e.open = true;
        let first = e.first_ready_at == 0;
        if first {
            e.first_ready_at = now;
        }
        first
    };
    gate_open(p, room);
    let acked = {
        let mut t = lock(&p.table);
        let a = t.acked(arev);
        p.ack.store(t.ack, Ordering::Release);
        a
    };
    tracing::info!(kind = "ev", spec = if acked { "LINK.S.ack" } else { "LINK.S.ack_old" }, user = %p.user_id, room = %room, arev, area = "LINK", key = %p.user_id, cause = "READY", "확인된 차례");
    let u = p.user_id.clone();
    floor.apply("READY", move |s| s.ready(&u));
    Ready { first, acked }
}

pub fn ready_at(p: &Peer, room: &str) -> u64 {
    lock(&p.gates).get(room).map(|g| g.first_ready_at).unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerChange {
    pub track_id: String,
    pub pref: LayerPref,
    pub unpaused: bool,
    pub paused: bool,
}

pub fn layer(p: &Peer, targets: &[LayerTarget]) -> Vec<LayerChange> {
    let mut out = Vec::new();
    let mut g = lock(&p.layers);
    for t in targets {
        let cur = g.get(&t.track_id).copied().unwrap_or_default();
        let mut n = cur;
        if let Some(s) = t.spatial {
            n.spatial = Some(s.min(SPATIAL_MAX));
        }
        if t.temporal.is_some() {
            n.temporal = Some(TEMPORAL_MAX);
        }
        if let Some(ps) = t.paused {
            n.paused = ps;
        }
        if let Some(pr) = t.priority {
            n.priority = pr;
        }
        g.insert(t.track_id.clone(), n);
        out.push(LayerChange { track_id: t.track_id.clone(), pref: n, unpaused: cur.paused && !n.paused, paused: !cur.paused && n.paused });
    }
    out
}

pub fn pref(p: &Peer, track_id: &str) -> LayerPref {
    lock(&p.layers).get(track_id).copied().unwrap_or_default()
}
