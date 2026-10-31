// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-DOC-OWNERSHIP · 정:RULE-OPS-DROPS · 정:RULE-OPS-OBSERVE · 정:RULE-CONC-HOT-PATH · 정:IMPL-ERR-ISSUERS · 정:CAND-OPS-SERVER-TRANSITION-SPEC · 정:CAND-DOC-ORCHESTRATION-ASPECT · model: claude-opus-5-5
//! 가로지르는 것 — 프로세스 상태 하나(Peer · 방 · 발행 · 배정표) · 버림 계수(닫힌 목록) · 코드별 계수 · 영역 사이 조율(입장 · 발행 · 구독 · 발언권 집행 · 운영).

pub mod admin;
pub mod floor;
pub mod handle;
pub mod media;
pub mod rooms;
pub mod timers;

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use arc_swap::ArcSwap;
use common::envelope::Notice;
use common::policy::Policy;
use oxsig::Op;
use oxsig::msg::Kind;

use crate::fwd::{DownRow, SlotFan};
use crate::link::{Peer, PeerState, Transport};
use crate::r#pub::{PubStream, PubTrack, Pubs};
use crate::room::Rooms;
use crate::sub::{Table, Tables};

pub const DROP_NAMES: &[&str] = &[
    "demux_unknown",
    "stun_unverified",
    "tcp_unidentified",
    "send_fail",
    "ingress_queue_full",
    "egress_queue_full",
    "transport_gone",
    "oversize",
    "rtp_unknown_path",
    "srtp_not_ready",
    "srtp_fail",
    "srtcp_fail",
    "rtp_malformed",
    "rtp_unknown_ssrc",
    "rid_unknown",
    "no_pub_room",
    "prefan_gate",
    "ready_gate",
    "row_gate",
    "await_keyframe",
    "rtcp_unresolved",
    "pli_throttled",
    "nack_audio",
    "rtx_gate",
    "rtx_cache_miss",
    "rtx_budget",
    "dc_bad_frame",
    "dc_unknown_svc",
    "dc_no_room",
    "dc_backlog_full",
    "bus_ev_drop",
];

pub struct Sfu {
    pub node: String,
    pub epoch: String,
    pub build: String,
    pub policy: Policy,
    pub ip: IpAddr,
    pub udp_port: u16,
    pub tcp_port: Option<u16>,
    pub cert: dtls::crypto::Certificate,
    pub fingerprint: String,
    pub node_grace_ms: u64,
    pub peers: Mutex<HashMap<String, Arc<Peer>>>,
    pub rooms: Rooms,
    pub pubs: Pubs,
    pub tables: Tables,
    pub tracks: Mutex<HashMap<String, Arc<PubStream>>>,
    pub slots: ArcSwap<HashMap<String, [Arc<SlotFan>; 2]>>,
    pub next_id: AtomicU64,
    pub drops: Vec<AtomicU64>,
    pub codes: Mutex<BTreeMap<u16, u64>>,
    pub rt: tokio::runtime::Handle,
}

static SFU: OnceLock<Arc<Sfu>> = OnceLock::new();
static DEFAULT_POLICY: OnceLock<Policy> = OnceLock::new();
static FALLBACK_PUBS: OnceLock<Pubs> = OnceLock::new();

pub fn install(s: Sfu) -> Arc<Sfu> {
    let a = Arc::new(s);
    let _ = SFU.set(a.clone());
    a
}

pub fn sfu() -> Option<&'static Arc<Sfu>> {
    SFU.get()
}

pub fn policy() -> &'static Policy {
    match SFU.get() {
        Some(s) => &s.policy,
        None => DEFAULT_POLICY.get_or_init(Policy::default),
    }
}

pub fn pubs() -> &'static Pubs {
    match SFU.get() {
        Some(s) => &s.pubs,
        None => FALLBACK_PUBS.get_or_init(Pubs::default),
    }
}

pub fn auto_layer_on() -> bool {
    crate::sim::mode() != crate::sim::Mode::Off
}

pub fn drop_count(name: &str) {
    let Some(s) = SFU.get() else { return };
    if let Some(i) = DROP_NAMES.iter().position(|n| *n == name) {
        s.drops[i].fetch_add(1, Ordering::Relaxed);
    }
}

pub fn code_count(code: u16) {
    if let Some(s) = SFU.get()
        && let Ok(mut g) = s.codes.lock()
    {
        g.entry(code).and_modify(|x| *x += 1).or_insert(1);
    }
}

pub fn spawn<F>(f: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    if let Some(s) = SFU.get() {
        s.rt.spawn(f);
    }
}

impl Sfu {
    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn peer(&self, user: &str) -> Option<Arc<Peer>> {
        self.peers.lock().ok().and_then(|g| g.get(user).cloned())
    }

    pub fn peers(&self) -> Vec<Arc<Peer>> {
        self.peers.lock().map(|g| g.values().cloned().collect()).unwrap_or_default()
    }

    pub fn table(&self, p: &Peer) -> Option<Arc<Table>> {
        self.tables.get(p.subscribe.id)
    }

    pub fn stream(&self, track_id: &str) -> Option<Arc<PubStream>> {
        self.tracks.lock().ok().and_then(|g| g.get(track_id).cloned())
    }

    pub fn publish_slots(&self) {
        let m: HashMap<String, [Arc<SlotFan>; 2]> = self.rooms.all().into_iter().map(|r| (r.id.clone(), r.slots.clone())).collect();
        self.slots.store(Arc::new(m));
    }
}

pub fn slot_of(room: &str, kind: Kind) -> Option<Arc<SlotFan>> {
    let s = SFU.get()?;
    let g = s.slots.load();
    let pair = g.get(room)?;
    Some(if kind == Kind::Audio { pair[0].clone() } else { pair[1].clone() })
}

pub fn table_of(conn: u64) -> Option<Arc<Table>> {
    SFU.get()?.tables.get(conn)
}

pub fn rows_of_source(t: &PubTrack) -> Vec<Arc<DownRow>> {
    if t.stream.is_half() {
        let room = t.stream.room.load_full();
        return slot_of(&room, t.stream.kind).map(|f| f.rows.load().iter().cloned().collect()).unwrap_or_default();
    }
    t.stream.rows.load().iter().cloned().collect()
}

pub fn user_notice<T: serde::Serialize>(user: &str, op: Op, body: &T, room: Option<&str>) {
    let n = Notice {
        op: op.as_u16(),
        body: serde_json::to_value(body).unwrap_or_default(),
        exclude: Vec::new(),
        room_id: room.map(|r| r.to_string()),
        sfu_id: SFU.get().map(|s| s.epoch.clone()),
    };
    crate::bus::user_notice(user, n);
}

pub fn transition(area: &str, key: &str, spec: &str, from: &str, to: &str, cause: &str, keys: &[(&str, &str)]) {
    let t = common::log::Transition { area, key, spec: Some(spec), from, to: Some(to), cause, inputs: &[] };
    let mut l = t.line();
    for (k, v) in keys {
        l = l.key(k, v);
    }
    common::log::emit(l);
}

pub fn transport_ready(tr: &Arc<Transport>) {
    tracing::info!(kind = "diag", user = %tr.user, "transport ready (dtls+srtp) conn {}", tr.id);
}

pub fn dc_opened(tr: &Arc<Transport>) {
    tr.dc_open.store(true, Ordering::Release);
    crate::aux::flush(tr);
}

pub fn dc_in(tr: &Arc<Transport>, buf: &[u8]) {
    floor::dc_in(tr, buf);
}

pub fn ufrag_promoted(tr: &Arc<Transport>, ufrag: &str) {
    transition("LINK", "PeerState", "SV-STATE-PEER-STATE", tr.peer_state().as_str(), "Alive", "ice_restart_checked", &[("user", &tr.user)]);
    tracing::info!(kind = "diag", user = %tr.user, "ufrag promoted {ufrag}");
}

pub fn peer_state_changed(tr: &Arc<Transport>, to: PeerState, path_changed: bool) {
    let dwell = common::now_ms().saturating_sub(tr.state_since.load(Ordering::Acquire));
    let cause = if path_changed { "nominated_new_pair" } else { "nominated" };
    transition("LINK", "PeerState", "SV-STATE-PEER-STATE", "Suspect", to.as_str(), cause, &[("user", &tr.user)]);
    tracing::info!(kind = "diag", user = %tr.user, "peer state {} after {dwell}ms (conn {})", to.as_str(), tr.id);
}

pub fn floor_blocked_rtp(room: &Arc<String>, tr: &Arc<Transport>) {
    let (room, tr) = (room.clone(), tr.clone());
    spawn(async move { floor::on_rtp(&room, &tr.user, false) });
}

pub fn floor_rtp(room: &Arc<String>, tr: &Arc<Transport>, first: bool) {
    let (room, tr) = (room.clone(), tr.clone());
    spawn(async move { floor::on_rtp(&room, &tr.user, first) });
}

pub fn simulcast_first_rtp(stream: &Arc<PubStream>) {
    if stream.listed.swap(true, Ordering::AcqRel) || stream.is_half() {
        return;
    }
    let s = stream.clone();
    spawn(async move { media::announce_add(&s) });
}

pub fn drops_json() -> serde_json::Value {
    let mut o = serde_json::Map::new();
    if let Some(s) = SFU.get() {
        for (i, n) in DROP_NAMES.iter().enumerate() {
            o.insert(n.to_string(), serde_json::json!(s.drops[i].load(Ordering::Relaxed)));
        }
        if let Ok(g) = s.codes.lock() {
            for (k, v) in g.iter() {
                o.insert(format!("code_{k}"), serde_json::json!(v));
            }
        }
        if let Some(b) = crate::bus::get() {
            o.insert("bus_ev_drop".into(), serde_json::json!(b.ev_drop.load(Ordering::Relaxed) + s.drops[DROP_NAMES.len() - 1].load(Ordering::Relaxed)));
        }
        if let Some(l) = common::log::global() {
            o.insert("log_queue_full".into(), serde_json::json!(l.counters.queue_full.load(Ordering::Relaxed)));
            o.insert("log_spool_full".into(), serde_json::json!(l.counters.spool_full.load(Ordering::Relaxed)));
        }
    }
    serde_json::Value::Object(o)
}

pub fn ufrag_of(tr: &Transport) -> String {
    tr.ufrag()
}
