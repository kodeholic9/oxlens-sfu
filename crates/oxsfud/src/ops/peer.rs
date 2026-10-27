// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-LINK-PEER-STATE · 정:RULE-LINK-TRANSPORT · 정:IMPL-SUB-TABLE · 정:IMPL-SUB-PIPELINE · 정:IMPL-SUB-READY-GATE · 정:IMPL-BUS-HOME-LOST · 정:RULE-AFF-ONE-SET · 정:CAND-RTCP-NACK-BUDGET-UNIT · 정:CAND-DOC-ORCHESTRATION-ASPECT · 연:RULE-LINK-SERVER-REAP · model: claude-opus-5-5
//! Peer — 세션 × 이 미디어 서버. 수명 = 세션 수명(침묵으로 거두지 않는다). 전송 객체(`2pc` 둘 · `1pc` 하나) · 배정표 하나(구독 연결) · 입장 방 집합 · `pub_room` · 발행 · 방마다 READY 게이트. 조율은 측면(ops)에 둔다.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use common::bus::env::SessionCtx;
use oxsig::types::{ExtMap, PcMode};

use crate::aff::Hot;
use crate::link::{Creds, Io, Role, Transport};
use crate::r#pub::Stream;
use crate::rtcp::Budget;
use crate::sub::ext::ExtTable;
use crate::sub::table::{PtTable, Table};

pub struct Prefs {
    pub spatial: u8,
    pub paused: bool,
    pub priority: u8,
}

pub struct Peer {
    pub user: String,
    pub ctx: SessionCtx,
    pub fp: String,
    pub home: (String, String),
    pub pc: PcMode,
    pub publish: Arc<Transport>,
    pub subscribe: Option<Arc<Transport>>,
    pub table: Table,
    pub sub_ext: ExtTable,
    pub rooms: BTreeSet<String>,
    pub pub_room: Option<String>,
    pub aff: Arc<Hot>,
    pub streams: BTreeMap<String, Arc<Stream>>,
    pub gates: BTreeMap<String, Arc<AtomicBool>>,
    pub first_ready: BTreeMap<String, u64>,
    pub prefs: BTreeMap<String, Prefs>,
    pub created_ms: u64,
    pub sub_active: BTreeMap<u8, bool>,
    pub kick_lock: Mutex<()>,
}

pub fn server_extmap() -> Vec<ExtMap> {
    use crate::sub::ext::*;
    vec![
        ExtMap { id: 1, uri: URI_MID.into() },
        ExtMap { id: 4, uri: URI_AUDIO_LEVEL.into() },
        ExtMap { id: 5, uri: URI_ABS_SEND_TIME.into() },
        ExtMap { id: 6, uri: URI_TWCC.into() },
        ExtMap { id: 10, uri: URI_RID.into() },
        ExtMap { id: 11, uri: URI_REPAIRED_RID.into() },
    ]
}

pub fn sub_ext_1pc(client: &[ExtMap]) -> ExtTable {
    let server = ExtTable::from_extmap(&server_extmap());
    let c = ExtTable::from_extmap(client);
    ExtTable { mid: c.mid.or(server.mid), audio_level: c.audio_level.or(server.audio_level), abs_send_time: c.abs_send_time.or(server.abs_send_time), twcc: c.twcc.or(server.twcc) }
}

impl Peer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(ctx: &SessionCtx, io: &Arc<Io>, sfu_id: &str, slot_budget: usize, codecs_1pc: Option<&[oxsig::msg::RoomJoinReqCodecs]>, ext_1pc: Option<&[ExtMap]>, arev_from: u64, now_ms: u64) -> Peer {
        let budget = Arc::new(Budget::default());
        let pc = ctx.pc_mode;
        let (publish, subscribe) = match pc {
            PcMode::V1pc => (Arc::new(Transport::new(io.next_id(), Role::Both, &ctx.user_id, &ctx.session, Creds::generate(), io, budget.clone(), now_ms)), None),
            PcMode::V2pc => (
                Arc::new(Transport::new(io.next_id(), Role::Publish, &ctx.user_id, &ctx.session, Creds::generate(), io, budget.clone(), now_ms)),
                Some(Arc::new(Transport::new(io.next_id(), Role::Subscribe, &ctx.user_id, &ctx.session, Creds::generate(), io, budget.clone(), now_ms))),
            ),
        };
        io.register(&publish);
        if let Some(s) = &subscribe {
            io.register(s);
        }
        let one = pc == PcMode::V1pc;
        let pts = match (one, codecs_1pc) {
            (true, Some(c)) => PtTable::seed_1pc(c),
            _ => PtTable::seed_2pc(),
        };
        let sub_ext = match (one, ext_1pc) {
            (true, Some(e)) => sub_ext_1pc(e),
            _ => ExtTable::from_extmap(&server_extmap()),
        };
        Peer {
            user: ctx.user_id.clone(),
            ctx: ctx.clone(),
            fp: ctx.session.clone(),
            home: (ctx.home_node.clone(), ctx.home_inst.clone()),
            pc,
            publish,
            subscribe,
            table: Table::new(sfu_id, one, &ctx.user_id, pts, slot_budget, arev_from),
            sub_ext,
            rooms: BTreeSet::new(),
            pub_room: None,
            aff: Arc::new(Hot::new(&ctx.user_id)),
            streams: BTreeMap::new(),
            gates: BTreeMap::new(),
            first_ready: BTreeMap::new(),
            prefs: BTreeMap::new(),
            created_ms: now_ms,
            sub_active: BTreeMap::new(),
            kick_lock: Mutex::new(()),
        }
    }

    pub fn sub_transport(&self) -> &Arc<Transport> {
        self.subscribe.as_ref().unwrap_or(&self.publish)
    }

    pub fn transports(&self) -> Vec<Arc<Transport>> {
        let mut v = vec![self.publish.clone()];
        if let Some(s) = &self.subscribe {
            v.push(s.clone());
        }
        v
    }

    pub fn gate(&mut self, room: &str) -> Arc<AtomicBool> {
        self.gates.entry(room.to_string()).or_insert_with(|| Arc::new(AtomicBool::new(false))).clone()
    }

    pub fn refresh_downlinks(&self) {
        let mut m = HashMap::new();
        for r in self.table.rows() {
            m.insert(r.ssrc, r.dl.clone());
            if let Some(x) = r.rtx_ssrc {
                m.insert(x, r.dl.clone());
            }
        }
        self.sub_transport().downlinks.store(Arc::new(m));
    }

    pub fn uplink_ssrcs(&self) -> Vec<u32> {
        self.publish.uplinks.load().keys().copied().collect()
    }

    pub fn active_tracks(&self) -> usize {
        self.streams.len()
    }
}
