// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-DOC-OWNERSHIP 정:RULE-BUS-PRESENCE 정:RULE-BUS-PLACEMENT 정:IMPL-BUS-FAULTS · model: claude-opus-5-5
//! 게이트웨이 상태 — 세션과 배치만 든다. 방 상태는 복제하지 않는다.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::Instant;

use common::keyspace::Keys;
use common::policy::Policy;
use common::system::System;

use crate::session::Registry;
use crate::supervisor::Supervisor;

#[derive(Debug, Clone)]
pub struct NodeEntry {
    pub inst: String,
    pub gone_at: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub node: String,
    pub inst: String,
}

pub struct Interest {
    pub count: usize,
    pub task: tokio::task::JoinHandle<()>,
}

pub struct Hub {
    pub node: String,
    pub build: String,
    pub system: System,
    pub policy: Policy,
    pub keys: Keys,
    pub bus: tokio::sync::OnceCell<zenoh::Session>,
    pub nodes: RwLock<BTreeMap<String, NodeEntry>>,
    pub rooms: RwLock<HashMap<String, Vec<Place>>>,
    pub sessions: Mutex<Registry>,
    pub interest: Mutex<HashMap<String, Interest>>,
    pub units: std::sync::Arc<Supervisor>,
    pub isolated: AtomicBool,
    pub shutting_down: AtomicBool,
    pub node_token: tokio::sync::Mutex<Option<(String, zenoh::liveliness::LivelinessToken)>>,
    pub last_event: Mutex<Option<serde_json::Value>>,
    pub code_counts: Mutex<BTreeMap<u16, u64>>,
    pub dup_rooms: AtomicU64,
    pub join_window: Mutex<(u64, u32)>,
}

impl Hub {
    pub fn new(node: String, build: String, system: System, policy: Policy) -> Hub {
        let keys = Keys::new(&system.zenoh.namespace);
        let units = std::sync::Arc::new(Supervisor::new(&system));
        Hub {
            node,
            build,
            system,
            policy,
            keys,
            bus: tokio::sync::OnceCell::new(),
            nodes: RwLock::new(BTreeMap::new()),
            rooms: RwLock::new(HashMap::new()),
            sessions: Mutex::new(Registry::default()),
            interest: Mutex::new(HashMap::new()),
            units,
            isolated: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            node_token: tokio::sync::Mutex::new(None),
            last_event: Mutex::new(None),
            code_counts: Mutex::new(BTreeMap::new()),
            dup_rooms: AtomicU64::new(0),
            join_window: Mutex::new((0, 0)),
        }
    }

    pub fn bus(&self) -> Option<&zenoh::Session> {
        self.bus.get()
    }

    pub fn my_inst(&self) -> String {
        self.nodes.read().ok().and_then(|n| n.get(&self.node).map(|e| e.inst.clone())).unwrap_or_default()
    }

    pub fn node_live(&self, node: &str) -> bool {
        self.nodes.read().map(|n| n.get(node).is_some_and(|e| e.gone_at.is_none())).unwrap_or(false)
    }

    pub fn node_by_inst(&self, inst: &str) -> Option<String> {
        self.nodes
            .read()
            .ok()?
            .iter()
            .find(|(_, e)| e.inst == inst && e.gone_at.is_none())
            .map(|(n, _)| n.clone())
    }

    pub fn candidates(&self) -> Vec<String> {
        let grace = std::time::Duration::from_secs(self.system.supervisor.node_grace_secs);
        self.nodes
            .read()
            .map(|n| {
                n.iter()
                    .filter(|(_, e)| e.gone_at.is_none_or(|t| t.elapsed() < grace))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn room_place(&self, room: &str) -> Option<Place> {
        self.rooms.read().ok()?.get(room).and_then(|v| v.first().cloned())
    }

    pub fn placement(&self, room: &str) -> Option<String> {
        if let Some(p) = self.room_place(room) {
            return Some(p.node);
        }
        let c = self.candidates();
        common::placement::place(c.iter().map(String::as_str), room).map(str::to_string)
    }

    pub fn count_code(&self, code: u16) {
        if let Ok(mut m) = self.code_counts.lock() {
            let e = m.entry(code).or_insert(0);
            let next = e.saturating_add(1);
            m.insert(code, next);
        }
    }

    pub fn note_event(&self, kind: &str, key: &str) {
        if let Ok(mut g) = self.last_event.lock() {
            g.replace(serde_json::json!({ "at_ms": common::ident::now_ms(), "kind": kind, "key": key }));
        }
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::Relaxed)
    }
}
