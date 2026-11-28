// author: kodeholic (powered by Claude)
//! 서버마다 연결 하나 — 복구 러너(SDK:IMPL-MOD-TABLE PeerLink · SDK:IMPL-LINK-PROJECTION · SDK:RULE-LINK-LAYERS · SDK:RULE-LINK-REBUILD).
//! 절차는 위 연결 클라 상태기(generated/link_client)가 정한다 — 여기는 층별 사실 → 입구 · 효과 · 이음매 · 위가 값을 두지 않은 손잡이(기다림 · 적용 시한 · 안정 관찰 · DC 다시 열기 · WS 점검 사다리).
//! 서버마다 전송 하나다(oxpeer 는 늘 1pc — SDK:RULE-PLAT-SERVER `pc_mode`) · 발행 자격으로 서고 · ICE 는 그 전송의 값 · DC 도 그 전송에.
//! ★협상 = 배정표를 받는 경로표(ssrc → 트랙)에 싣는 것이다(SDP 가 없다) — 늘 성공한다 · 결과는 입구 negoRes 로 넣는다.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Core, Job, Tk};
use crate::api::{Event, LinkInfo, LinkState};
use crate::generated::ws_wire::{IceRestartReq, IceRestartRes, MlineTable, Op, ServerConfig, ServerConfigIce, TrackEntryType};
use crate::generated::{link_binding as lb, link_client as lc, room_client as rc, sdk_defaults as dflt, session_client as sc, sub_client as uc};
use crate::link::transport::{Creds, Fact, IceState, LinkFact, Route, Routes, Source, Transport};
use crate::sig::{Outcome, Purpose};

/// 연결 한 걸음 안에서 다시 들어온 재협상 요청을 그 걸음 끝에 하나로 — 배정표 통지 하나가 그 서버의 방마다 낸 sdp_rebuild(표는 연결 단위 — 연:C-sym-R-ROOM-DETAIL-SYMPTOMS-3) · 떠 있는 협상이 있으면 위 상태기가 큐에 적는다
#[derive(Debug, Default)]
pub(crate) struct RenegoGate {
    depth: u32,
    asked: bool,
}

impl RenegoGate {
    pub(crate) fn enter(&mut self) {
        self.depth += 1;
    }

    pub(crate) fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// 지금 걸어도 되는가 — 걸음 안이면 적어 두고 거짓
    pub(crate) fn ask(&mut self) -> bool {
        if self.depth > 0 {
            self.asked = true;
            return false;
        }
        true
    }

    /// 바깥 걸음이 끝났고 적힌 요청이 있으면 참(한 번만)
    pub(crate) fn take(&mut self) -> bool {
        if self.depth == 0 && self.asked {
            self.asked = false;
            return true;
        }
        false
    }
}

pub(crate) struct Link {
    pub(crate) server: String,
    pub(crate) st: lc::St,
    pub(crate) sc: ServerConfig,
    sgen: u64,
    wire: Option<Transport>,
    ice: Option<IceState>,
    dc_open: bool,
    sctp_up: bool,
    pub(crate) table: Option<MlineTable>,
    /// 이 차례의 표가 실린 협상 — READY{arev} 가 싣는다(연:RULE-SUB-TABLE-PROC)
    pub(crate) sdp_arev: u64,
    pub(crate) pending_table: Option<Value>,
    incoming: Option<MlineTable>,
    restart_creds: Option<Creds>,
    reopen_tries: i64,
    dc_down_at: Instant,
    pub(crate) blocked: bool,
    blocked_seen: i64,
    rung: u32,
    attempt: u32,
    pending_check: bool,
    negotiating: bool,
    rebuild_owed: bool,
    renego_gate: RenegoGate,
    replaced: Option<i64>,
    probe_since: Option<Instant>,
    check_before: (u32, u32),
    last_info: Option<LinkInfo>,
}

pub(crate) fn publish_creds(ice: &ServerConfigIce) -> Creds {
    Creds { ufrag: ice.publish_ufrag.clone(), pwd: ice.publish_pwd.clone() }
}

impl Link {
    fn main(&self) -> Option<&Transport> {
        self.wire.as_ref()
    }

    pub(crate) fn out_slot(&self) -> Option<std::sync::Arc<std::sync::OnceLock<crate::link::transport::PubOut>>> {
        self.main().map(Transport::out_slot)
    }

    /// 앱 투영(SDK:IMPL-LINK-PROJECTION 위 상태 → 앱 값)
    pub(crate) fn state(&self) -> LinkState {
        match self.st.ls {
            lc::LinkSt::NONE => LinkState::Closed,
            lc::LinkSt::REBUILD => LinkState::Rebuilding,
            lc::LinkSt::WAIT | lc::LinkSt::REOPEN => LinkState::Wobbly,
            lc::LinkSt::CHECK | lc::LinkSt::RESTART | lc::LinkSt::APPLY => LinkState::Restarting,
            _ if self.st.ever_up => LinkState::Up,
            _ => LinkState::Connecting,
        }
    }

    pub(crate) fn send_dc(&self, b: Vec<u8>) -> bool {
        self.main().is_some_and(|t| t.send_dc(b))
    }

    fn info(&self) -> LinkInfo {
        let cause = if self.blocked { Some("path_unreachable".to_string()) } else if self.st.cause == lc::RebuildCause::NONE { None } else { Some(self.st.cause.as_str().to_lowercase()) };
        LinkInfo { server: self.server.clone(), state: self.state(), rung: self.rung, attempt: self.attempt, cause }
    }

    fn ice_up(&self) -> bool {
        self.ice == Some(IceState::Connected)
    }
}

/// 배정표 사본(SDK:RULE-MOD-BINDING-VALUES) — mid = track_id → 그 줄 mid · rows = SDP 재료 지문
fn mid_of(t: &MlineTable) -> lc::Rec<i64> {
    t.mlines.iter().filter_map(|m| Some((m.track_id.clone()?, m.mid.parse::<i64>().ok()?))).collect()
}

fn materials(t: &MlineTable) -> String {
    let mut rows: Vec<Value> = t.mlines.iter().map(|m| json!([m.mid, m.kind, m.pt, m.codec, m.fmtp, m.rtx_pt, m.ssrc, m.rtx_ssrc, m.stream_id])).collect();
    rows.sort_by_key(|r| r[0].as_str().unwrap_or("").to_string());
    Value::Array(rows).to_string()
}

impl Core {
    fn ufrag_gen(&mut self, u: &str) -> i64 {
        self.epochs.id(&format!("ufrag:{u}"))
    }

    fn rooms_on(&self, server: &str) -> Vec<String> {
        self.rooms.values().filter(|r| r.sfu.as_deref() == Some(server)).map(|r| r.id.clone()).collect()
    }

    /// 읽는 값 leaving(READS ROOM.rs) — 그 서버 방이 전부 나가는 중이다(하나 이상 LEAVING · 나머지는 NONE)
    fn leaving_all(&self, server: &str) -> bool {
        let rs: Vec<rc::RoomSt> = self.rooms.values().filter(|r| r.sfu.as_deref() == Some(server)).map(|r| r.st.rs).collect();
        rs.contains(&rc::RoomSt::LEAVING) && rs.iter().all(|x| matches!(x, rc::RoomSt::LEAVING | rc::RoomSt::NONE))
    }

    fn open_transport(&mut self, sc_: &ServerConfig) -> Option<(u64, Transport)> {
        let addr: SocketAddr = format!("{}:{}", sc_.ice.ip, sc_.ice.port).parse().ok()?;
        self.link_gen += 1;
        let g = self.link_gen;
        Some((g, Transport::open(addr, publish_creds(&sc_.ice), self.cert.clone(), (sc_.sfu_id.clone(), g), self.fact_tx.clone(), self.sink.clone())))
    }

    /// 입장 응답 — 처음 붙는 서버면 연결을 세우고 · 이미 붙은 서버면 그대로 · 자격이 바뀌었거나 연결이 닫혔으면 새 전송으로 갈아 끼운다
    pub(super) fn link_adopt(&mut self, sc_: ServerConfig) {
        let server = sc_.sfu_id.clone();
        let live = |l: &Link| l.wire.is_some();
        let stale = self.links.get(&server).is_some_and(|l| live(l) && l.sc.ice.publish_ufrag != sc_.ice.publish_ufrag);
        if self.links.get(&server).is_some_and(live) && !stale {
            return;
        }
        let Some((g, wire)) = self.open_transport(&sc_) else {
            self.emit(Event::Error { room: None, error: crate::error::Error::transport(format!("ICE 주소를 못 읽었다: {}:{}", sc_.ice.ip, sc_.ice.port)) });
            return;
        };
        let ug = self.ufrag_gen(&sc_.ice.publish_ufrag);
        if let Some(l) = self.links.get_mut(&server) {
            l.wire = Some(wire);
            l.sgen = g;
            l.sc = sc_;
            l.ice = None;
            l.dc_open = false;
            l.sctp_up = false;
            l.negotiating = false;
            if stale {
                l.replaced = Some(ug);
            }
            self.sync_outs();
            self.link_surface(&server);
            return;
        }
        // 위 상태기가 서기 전의 첫 수립 — everUp 거짓 · 표 사본은 비어 있다(SDK:IMPL-LINK-PROJECTION connecting)
        let st = lc::St { ever_up: false, mid: lc::Rec::new(), rows: 0, r#gen: ug, ..lc::initial() };
        let link = Link {
            server: server.clone(),
            st,
            sc: sc_,
            sgen: g,
            wire: Some(wire),
            ice: None,
            dc_open: false,
            sctp_up: false,
            table: None,
            sdp_arev: 0,
            pending_table: None,
            incoming: None,
            restart_creds: None,
            reopen_tries: 0,
            dc_down_at: Instant::now(),
            blocked: false,
            blocked_seen: 0,
            rung: 0,
            attempt: 0,
            pending_check: false,
            negotiating: false,
            rebuild_owed: false,
            renego_gate: RenegoGate::default(),
            replaced: None,
            probe_since: None,
            check_before: (0, 0),
            last_info: None,
        };
        self.links.insert(server.clone(), link);
        self.record("LINK", &server, "-", "OK", "SDK.link_new", "join");
        self.sync_outs();
        self.link_surface(&server);
    }

    // ── 층별 사실 → 위 입구(SDK:IMPL-LINK-PROJECTION 입구 표) ──────────

    pub(super) fn link_fact(&mut self, f: LinkFact) {
        let Some(l) = self.links.get_mut(&f.server) else { return };
        if l.sgen != f.sgen {
            return;
        }
        let server = f.server.clone();
        match f.fact {
            Fact::Pli(ssrc) => self.pli_in(ssrc),
            Fact::Bwe(bps) => self.emit(Event::TargetBitrate { server: server.clone(), bps }),
            Fact::Ice(s) => {
                if l.ice == Some(s) {
                    return;
                }
                l.ice = Some(s);
                self.ice_fact(&server, s);
            }
            Fact::Dtls(true) => {}
            Fact::Dtls(false) => {
                let leaving = self.leaving_all(&server);
                let s = lc::assoc_lost(&self.links[&server].st, leaving);
                self.link_step(&server, "assocLost", Some(s), "dtls");
            }
            Fact::Sctp(up) => {
                l.sctp_up = up;
                if !up {
                    let leaving = self.leaving_all(&server);
                    let s = lc::assoc_lost(&self.links[&server].st, leaving);
                    self.link_step(&server, "assocLost", Some(s), "sctp");
                }
            }
            Fact::Dc(true) => {
                l.dc_open = true;
                if l.st.ls == lc::LinkSt::REOPEN {
                    self.timers.stop(&Tk::LinkReopenTry(server.clone()));
                    self.timers.stop(&Tk::LinkReopenJudge(server.clone()));
                    let s = lc::reopened(&self.links[&server].st, lc::Reopen::ok);
                    self.link_step(&server, "reopened", s, "dc");
                }
                self.first_up(&server);
                self.try_rebuilt(&server);
            }
            Fact::Dc(false) => {
                l.dc_open = false;
                let leaving = self.leaving_all(&server);
                let l = &self.links[&server];
                if !l.sctp_up {
                    let s = lc::assoc_lost(&l.st, leaving);
                    self.link_step(&server, "assocLost", Some(s), "dc");
                    return;
                }
                // 다시 열기 중에 새 DC 가 닫혔다 — 그 시도의 판정은 시한에 한다
                if l.st.ls == lc::LinkSt::REOPEN {
                    return;
                }
                if let Some(l) = self.links.get_mut(&server) {
                    l.dc_down_at = Instant::now();
                }
                let s = lc::dc_closed(&self.links[&server].st, leaving);
                self.link_step(&server, "dcClosed", Some(s), "dc");
            }
            Fact::DcData(b) => self.floor_receive(&server, &b),
            Fact::FirstRtp(ssrc) => self.first_rtp(&server, ssrc),
        }
    }

    fn ice_fact(&mut self, server: &str, s: IceState) {
        let ls = self.links[server].st.ls;
        match s {
            IceState::Connected => {
                if ls == lc::LinkSt::WAIT {
                    let s = lc::ice_back(&self.links[server].st);
                    self.link_step(server, "iceBack", s, "ice");
                } else if ls == lc::LinkSt::APPLY && !self.links[server].st.nego {
                    let s = lc::applied(&self.links[server].st);
                    self.link_step(server, "applied", s, "ice");
                } else if ls == lc::LinkSt::REBUILD {
                    self.try_rebuilt(server);
                }
                self.first_up(server);
            }
            IceState::Disconnected => {
                let leaving = self.leaving_all(server);
                let s = lc::ice_disc(&self.links[server].st, leaving);
                self.link_step(server, "iceDisc", Some(s), "ice");
            }
            IceState::Failed => {
                if ls == lc::LinkSt::REBUILD {
                    let g = self.peer_gen(server);
                    let s = lc::rebuilt(&self.links[server].st, g, false);
                    self.link_step(server, "rebuilt", s, "ice");
                } else {
                    let leaving = self.leaving_all(server);
                    let s = lc::wait_over(&self.links[server].st, true, leaving);
                    self.link_step(server, "waitOver", Some(s), "ice");
                }
            }
        }
    }

    fn peer_gen(&mut self, server: &str) -> i64 {
        let u = self.links[server].sc.ice.publish_ufrag.clone();
        self.ufrag_gen(&u)
    }

    /// 첫 수립 — 전 층이 처음 섰다(위 상태기가 서기 전의 connecting 이 끝난다 · 위 상태 everUp)
    fn first_up(&mut self, server: &str) {
        let Some(l) = self.links.get_mut(server) else { return };
        if l.st.ever_up || l.st.ls != lc::LinkSt::OK || !l.dc_open || !l.ice_up() {
            return;
        }
        l.st = lc::St { ever_up: true, ..l.st.clone() };
        l.blocked_seen = 0;
        l.blocked = false;
        let ls = l.st.ls.as_str();
        self.record("LINK", server, ls, ls, "SDK.first_up", "ice+dc");
        self.link_surface(server);
    }

    fn try_rebuilt(&mut self, server: &str) {
        let Some(l) = self.links.get(server) else { return };
        if l.st.ls != lc::LinkSt::REBUILD || !l.ice_up() || !l.dc_open {
            return;
        }
        let g = self.peer_gen(server);
        let s = lc::rebuilt(&self.links[server].st, g, true);
        self.link_step(server, "rebuilt", s, "rebuild");
    }

    /// 그 트랙의 첫 복호 패킷 — receiving · 무전 슬롯 오디오면 발언권 입구 rtpRx(재생은 서버사이드 대상 아님)
    fn first_rtp(&mut self, server: &str, ssrc: u32) {
        let route = self.sink.routes.read().ok().and_then(|g| g.media.get(&ssrc).cloned());
        let Some(r) = route else { return };
        self.emit(Event::Receiving { room: r.room.clone(), track_id: r.track_id.clone() });
        let slot_audio = r.kind == "audio" && self.rooms.get(&r.room).and_then(|x| x.entries.get(&r.track_id)).is_some_and(|t| t.type_ == TrackEntryType::Slot);
        if slot_audio && self.rooms.get(&r.room).and_then(|x| x.sfu.as_deref()) == Some(server) {
            self.floor_enter(&r.room, "rtpRx", |s| Some(crate::generated::floor_client::rtp_rx(s)), "rtp");
        }
    }

    // ── 위 이음매가 부르는 입구(위 코드 REACTS) ─────────────────────

    pub(super) fn link_renego(&mut self, server: &str) {
        let Some(l) = self.links.get_mut(server) else { return };
        if !l.renego_gate.ask() {
            return;
        }
        let s = lc::renego(&l.st);
        self.link_step(server, "renego", Some(s), "seam");
    }

    pub(super) fn link_close_all(&mut self, server: &str) {
        let Some(l) = self.links.get(server) else { return };
        let s = lc::close_all(&l.st);
        self.link_step(server, "closeAll", Some(s), "session");
    }

    pub(super) fn link_repair(&mut self, server: &str) {
        let Some(l) = self.links.get(server) else { return };
        let s = lc::repair(&l.st);
        self.link_step(server, "repair", Some(s), "resume");
    }

    pub(super) fn link_epoch_changed(&mut self, server: &str) {
        let Some(l) = self.links.get(server) else { return };
        let s = lc::epoch_changed(&l.st);
        self.link_step(server, "epochChanged", Some(s), "epoch");
    }

    pub(super) fn link_room_left(&mut self, server: &str, last: bool) {
        let Some(l) = self.links.get(server) else { return };
        let s = lc::room_left(&l.st, last);
        self.link_step(server, "roomLeft", Some(s), "leave");
    }

    /// 입장 응답의 자격이 보관값과 달랐나(갈아 끼웠나) — 위가 그 서버를 다시 세운다(연:RULE-ROOM-JOIN-PROC)
    pub(super) fn link_server_ice(&mut self, server: &str) {
        let Some(l) = self.links.get_mut(server) else { return };
        let g = l.replaced.take();
        let s = lc::server_ice(&l.st, g.is_some(), g.unwrap_or(l.st.r#gen));
        self.link_step(server, "serverIce", Some(s), "ROOM_JOIN.ice");
    }

    /// 입장 응답 · 이어받기 응답이 실어 온 표 — MLINE_EVENT 와 같게 받는다
    pub(super) fn link_table_carried(&mut self, server: &str, t: Value) {
        let t = match serde_json::from_value::<MlineTable>(t) {
            Ok(t) => t,
            Err(e) => {
                self.log.emit(crate::log::Line::new(crate::log::Kind::Diag).level(crate::log::Level::Warn).key("op", "mline_table").key("err", e.to_string()).msg("mline table dropped - undecodable"));
                return;
            }
        };
        if t.sfu_id != server {
            return;
        }
        let (mid, rows) = (mid_of(&t), self.materials.id(&materials(&t)));
        let arev = t.arev as i64;
        self.accept_table(server, t, |st| lc::table_carried(st, arev, &mid, rows), "tableCarried");
    }

    pub(super) fn link_flush_pending(&mut self, server: &str) {
        let Some(t) = self.links.get_mut(server).and_then(|l| l.pending_table.take()) else { return };
        self.link_table_carried(server, t);
    }

    /// MLINE_EVENT — 그 서버 연결의 배정표 전체
    pub(super) fn mline_event(&mut self, t: Value) {
        let t = match serde_json::from_value::<MlineTable>(t) {
            Ok(t) => t,
            Err(e) => {
                self.log.emit(crate::log::Line::new(crate::log::Kind::Diag).level(crate::log::Level::Warn).key("op", "MLINE_EVENT").key("err", e.to_string()).msg("mline table dropped - undecodable"));
                return;
            }
        };
        let server = t.sfu_id.clone();
        if !self.links.contains_key(&server) {
            return;
        }
        let m = lc::MlineEventMsg { table: lc::MlineEventMsgTable { arev: t.arev as i64, mid: mid_of(&t), rows: self.materials.id(&materials(&t)) } };
        self.accept_table(&server, t, |st| lc::mline_event(st, &m), "mlineEvent");
    }

    /// 배정표 — 받아들였는가(차례 번호가 올랐다)는 위가 돌려준 상태로 읽는다
    fn accept_table(&mut self, server: &str, t: MlineTable, f: impl Fn(&lc::St) -> lc::Step, entry: &'static str) {
        let Some(l) = self.links.get_mut(server) else { return };
        l.incoming = Some(t);
        let s = f(&l.st);
        self.link_step(server, entry, Some(s), "table");
        if let Some(l) = self.links.get_mut(server) {
            l.incoming = None;
        }
    }

    pub(super) fn link_event(&mut self, server: &str, ufrag: &str) {
        let Some(l) = self.links.get(server) else { return };
        let current = l.sc.ice.publish_ufrag == ufrag;
        let m = lc::LinkEventMsg { ufrag: if current { l.st.r#gen } else { -1 } };
        let s = lc::link_event(&l.st, &m);
        self.link_step(server, "linkEvent", Some(s), "LINK_EVENT");
    }

    /// 세션 층 복구가 끝났다 — 보류했던 WS 점검을 이어 간다
    pub(super) fn link_session_back(&mut self, server: &str) {
        let Some(l) = self.links.get_mut(server) else { return };
        if l.pending_check {
            l.pending_check = false;
            self.ws_check(server);
        }
    }

    // ── 한 걸음 ──────────────────────────────────────────────────────

    pub(super) fn link_step(&mut self, server: &str, entry: &'static str, step: Option<lc::Step>, cause: &str) {
        let Some(l) = self.links.get_mut(server) else { return };
        let Some(step) = step else {
            let ls = l.st.ls.as_str();
            self.impossible("LINK", server, ls, entry);
            return;
        };
        let prev = l.st.clone();
        let fx = step.fx.clone();
        let plan = lb::plan(step);
        l.st = plan.state.clone();
        self.record("LINK", server, prev.ls.as_str(), plan.state.ls.as_str(), plan.label, cause);
        if let Some(l) = self.links.get_mut(server) {
            l.renego_gate.enter();
        }
        self.take_table(server, &prev);
        for f in &fx {
            // 드러내기 손잡이 — unreachableAfter 회째에 드러낸다 · 포기하지 않는다
            if *f == lc::Fx::app_link_blocked
                && let Some(l) = self.links.get_mut(server) {
                    l.blocked_seen += 1;
                    if l.blocked_seen >= dflt::UNREACHABLE_AFTER {
                        l.blocked = true;
                    }
                }
        }
        for a in &plan.actions {
            match a {
                lb::LinkAction::PeerClose => {
                    if let Some(l) = self.links.get_mut(server) {
                        l.wire = None;
                        l.negotiating = false;
                        l.dc_open = false;
                        l.sctp_up = false;
                        l.ice = None;
                    }
                    self.sync_outs();
                }
                lb::LinkAction::DcReopen => self.schedule_reopen(server, prev.ls != lc::LinkSt::REOPEN),
                lb::LinkAction::SdpRestart => self.load_creds(server),
            }
        }
        for tx in &plan.send {
            match tx {
                lc::Tx::Ready => {
                    for r in self.rooms_on(server) {
                        self.send_ready(&r);
                    }
                }
                lc::Tx::Heartbeat => self.ws_check(server),
                lc::Tx::IceRestart => {
                    let req = self.links[server].st.req;
                    self.request(Op::IceRestart, super::to_body(&IceRestartReq { sfu_id: server.to_string() }), Purpose::IceRestart { server: server.to_string(), req });
                }
            }
        }
        for sm in &plan.seams {
            self.link_seam(server, sm.fx);
        }
        if let Some(l) = self.links.get_mut(server) {
            l.renego_gate.leave();
        }
        self.link_follow(server, &prev, &plan);
        if self.links.get_mut(server).is_some_and(|l| l.renego_gate.take()) {
            self.link_renego(server);
        }
    }

    /// 위가 받아들인 표를 이음매보다 먼저 보관한다 · 재료가 그대로면 이 표가 적용된 협상의 가장 새 표다
    fn take_table(&mut self, server: &str, prev: &lc::St) {
        let Some(l) = self.links.get_mut(server) else { return };
        let Some(t) = l.incoming.clone() else { return };
        if l.st.arev == prev.arev {
            return;
        }
        let arev = t.arev;
        l.table = Some(t);
        if !l.negotiating && l.st.rows == prev.rows {
            l.sdp_arev = arev;
        }
        self.rebuild_routes();
    }

    fn link_seam(&mut self, server: &str, fx: lc::SeamFx) {
        let rooms = self.rooms_on(server);
        match fx {
            lc::SeamFx::floor_dc_down => rooms.iter().for_each(|r| {
                self.floor_enter(r, "dcDown", |s| Some(crate::generated::floor_client::dc_down(s)), "dc");
            }),
            lc::SeamFx::floor_dc_up => rooms.iter().for_each(|r| {
                self.floor_enter(r, "dcUp", |s| Some(crate::generated::floor_client::dc_up(s)), "dc");
            }),
            lc::SeamFx::floor_down_server => rooms.iter().for_each(|r| self.floor_down(r)),
            lc::SeamFx::rooms_rejoin_server => self.rejoin_all(rooms),
            lc::SeamFx::store_drop_server => rooms.iter().for_each(|r| self.room_down_all(r)),
            lc::SeamFx::sub_nego_ok => rooms.iter().for_each(|r| self.sub_enter(r, "negoOk", |s| Some(uc::nego_ok(s)), "nego")),
            lc::SeamFx::sub_nego_fail => rooms.iter().for_each(|r| self.sub_enter(r, "negoFail", |s| Some(uc::nego_fail(s)), "nego")),
            lc::SeamFx::sub_table_changed => rooms.iter().for_each(|r| self.sub_enter(r, "tableChanged", |s| Some(uc::table_changed(s)), "table")),
            lc::SeamFx::sub_table_rebound => rooms.iter().for_each(|r| self.sub_enter(r, "tableRebound", |s| Some(uc::table_rebound(s)), "table")),
            // 발행은 첫 단계에 없다
            // 발행(20261107c) — 그 서버 연결이 닫혔다 · 다시 섰다(재구축의 다시 발행). 협상 결과는 발행이 SDP 없이 곧바로 받는다
            lc::SeamFx::pub_drop_server => self.pub_server(server, false),
            lc::SeamFx::pub_reregister_server => self.pub_server(server, true),
            lc::SeamFx::pub_nego_ok | lc::SeamFx::pub_nego_fail => {}
        }
    }

    /// 한 걸음 뒤 — 위가 값을 두지 않은 손잡이 · 협상 큐 · 표면
    fn link_follow(&mut self, server: &str, prev: &lc::St, plan: &lb::Plan) {
        let s = server.to_string();
        let Some(l) = self.links.get(server) else { return };
        let ls = l.st.ls;
        if ls == lc::LinkSt::WAIT && prev.ls != lc::LinkSt::WAIT {
            self.timers.start(Tk::LinkGrace(s.clone()), Duration::from_millis(dflt::WOBBLY_GRACE_MS as u64));
        }
        if ls != lc::LinkSt::WAIT {
            self.timers.stop(&Tk::LinkGrace(s.clone()));
        }
        if ls == lc::LinkSt::APPLY && prev.ls != lc::LinkSt::APPLY {
            self.timers.start(Tk::LinkApply(s.clone()), Duration::from_millis(dflt::ICE_RESTART_TIMEOUT_MS as u64));
        }
        if ls != lc::LinkSt::APPLY {
            self.timers.stop(&Tk::LinkApply(s.clone()));
        }
        // 협상 중에 받은 새 자격 — 위는 큐에 적기만 한다 · 다음 협상이 싣도록 연결에 넘긴다
        if ls == lc::LinkSt::APPLY && prev.ls == lc::LinkSt::RESTART && !plan.actions.contains(&lb::LinkAction::SdpRestart) {
            self.load_creds(server);
        }
        if ls != lc::LinkSt::REOPEN {
            self.timers.stop(&Tk::LinkReopenTry(s.clone()));
            self.timers.stop(&Tk::LinkReopenJudge(s.clone()));
            if let Some(l) = self.links.get_mut(server) {
                l.reopen_tries = 0;
            }
        }
        if ls == lc::LinkSt::OK && prev.ls != lc::LinkSt::OK && prev.ls != lc::LinkSt::NONE {
            let ms = sc::timer_ms(sc::TimerName::TStable).unwrap_or(10_000) as u64;
            self.timers.start(Tk::LinkStable(s.clone()), Duration::from_millis(ms));
        } else if ls != lc::LinkSt::OK {
            self.timers.stop(&Tk::LinkStable(s.clone()));
        }
        if ls != lc::LinkSt::CHECK {
            self.timers.stop(&Tk::LinkCheck(s.clone()));
            self.timers.stop(&Tk::LinkProbe(s.clone()));
        }
        let Some(l) = self.links.get_mut(server) else { return };
        if ls == lc::LinkSt::OK && l.st.ever_up && l.blocked {
            l.blocked = false;
            l.blocked_seen = 0;
        }
        if prev.ls == lc::LinkSt::REBUILD && ls == lc::LinkSt::OK && prev.dirty {
            l.rebuild_owed = true;
        }
        let renego = l.rebuild_owed && !l.negotiating && !l.st.nego;
        if renego {
            l.rebuild_owed = false;
        }
        if renego {
            self.link_renego(server);
        }
        let Some(l) = self.links.get_mut(server) else { return };
        if l.st.nego && !l.negotiating {
            l.negotiating = true;
            self.jobs.push_back(Job::Negotiate(s.clone()));
        }
        let l = &self.links[server];
        if ls == lc::LinkSt::APPLY && !l.st.nego && prev.nego && l.ice_up() {
            let st = lc::applied(&l.st);
            self.link_step(server, "applied", st, "nego");
        }
        if ls == lc::LinkSt::NONE && prev.ls != lc::LinkSt::NONE {
            // 사슬의 끝 줄(운영:RULE-LOG-TRANSITION 「마지막 줄은 소멸 to=-」) — 같은 서버 연결을 다시 만들면 새 사슬이 `from=-` 로 선다
            self.record("LINK", server, ls.as_str(), "-", "SDK.link_drop", "close");
            self.links.remove(server);
            self.timers.stop_where(|t| matches!(t, Tk::LinkGrace(x) | Tk::LinkApply(x) | Tk::LinkReopenTry(x) | Tk::LinkReopenJudge(x) | Tk::LinkStable(x) | Tk::LinkCheck(x) | Tk::LinkProbe(x) if *x == s));
            self.rebuild_routes();
            self.emit(Event::Link(LinkInfo { server: s, state: LinkState::Closed, rung: 0, attempt: 0, cause: None }));
            self.surface(false);
            return;
        }
        self.link_surface(server);
    }

    /// 협상 — 배정표를 받는 경로표에 싣는다(SDP 가 없다 · 늘 성공) · 결과는 입구 negoRes
    pub(super) fn negotiate(&mut self, server: &str) {
        let Some(l) = self.links.get_mut(server) else { return };
        l.negotiating = false;
        if let Some(t) = &l.table {
            l.sdp_arev = t.arev;
        }
        self.rebuild_routes();
        let s = lc::nego_res(&self.links[server].st, true);
        self.link_step(server, "negoRes", Some(s), "nego.ok");
    }

    /// 새 ICE 자격을 전송에 넘긴다(지문 · setup 은 그대로)
    fn load_creds(&mut self, server: &str) {
        let Some(l) = self.links.get(server) else { return };
        if let (Some(p), Some(t)) = (l.restart_creds.clone(), &l.wire) {
            t.set_creds(p);
        }
    }

    pub(super) fn restart_reply(&mut self, server: &str, req: i64, o: Outcome) {
        if !self.links.contains_key(server) {
            return;
        }
        let m = match o {
            Outcome::Ok(b) => {
                let Ok(res) = serde_json::from_value::<IceRestartRes>(b) else { return };
                let ug = self.ufrag_gen(&res.publish.ufrag);
                let Some(l) = self.links.get_mut(server) else { return };
                l.restart_creds = Some(Creds { ufrag: res.publish.ufrag.clone(), pwd: res.publish.pwd.clone() });
                l.sc.ice.publish_ufrag = res.publish.ufrag.clone();
                l.sc.ice.publish_pwd = res.publish.pwd.clone();
                lc::RestartResMsg { code: 0, part: lc::Part::res, pid: req, publish: lc::RestartResMsgPublish { ufrag: ug } }
            }
            Outcome::Fail(b) => {
                let code = b.get("code").and_then(Value::as_i64).unwrap_or(5003);
                lc::RestartResMsg { code, part: lc::Part::fail, pid: req, publish: lc::RestartResMsgPublish { ufrag: 0 } }
            }
            _ => return,
        };
        let s = lc::restart_res(&self.links[server].st, &m);
        self.link_step(server, "restartRes", Some(s), "ICE_RESTART");
    }

    // ── 손잡이 ──────────────────────────────────────────────────────

    /// DC 다시 열기 — 시도 k(1 부터)는 DC 가 닫힌 뒤 k × intervalMs 에 연다 · 판정 시한은 다음 시도 시각
    fn schedule_reopen(&mut self, server: &str, first: bool) {
        let Some(l) = self.links.get_mut(server) else { return };
        if first {
            l.reopen_tries = 0;
        }
        let k = l.reopen_tries + 1;
        let at = l.dc_down_at + Duration::from_millis((k * dflt::dc_reopen::INTERVAL_MS) as u64);
        self.timers.start(Tk::LinkReopenTry(server.to_string()), at.saturating_duration_since(Instant::now()));
    }

    pub(super) fn link_reopen_try(&mut self, server: &str) {
        let Some(l) = self.links.get_mut(server) else { return };
        l.reopen_tries += 1;
        if let Some(t) = l.main() {
            t.reopen_dc();
        }
        self.timers.start(Tk::LinkReopenJudge(server.to_string()), Duration::from_millis(dflt::dc_reopen::INTERVAL_MS as u64));
    }

    pub(super) fn link_reopen_judge(&mut self, server: &str) {
        let Some(l) = self.links.get(server) else { return };
        if l.st.ls != lc::LinkSt::REOPEN {
            return;
        }
        let r = if l.reopen_tries >= dflt::dc_reopen::TRIES { lc::Reopen::exhausted } else { lc::Reopen::fail };
        let s = lc::reopened(&l.st, r);
        self.link_step(server, "reopened", s, "dc");
    }

    pub(super) fn link_grace(&mut self, server: &str) {
        let Some(l) = self.links.get(server) else { return };
        let leaving = self.leaving_all(server);
        let s = lc::wait_over(&l.st, false, leaving);
        self.link_step(server, "waitOver", Some(s), "grace");
    }

    pub(super) fn link_apply_timeout(&mut self, server: &str) {
        let Some(l) = self.links.get(server) else { return };
        let s = lc::apply_timeout(&l.st);
        self.link_step(server, "applyTimeout", s, "timer");
    }

    pub(super) fn link_stable(&mut self, server: &str) {
        if let Some(l) = self.links.get_mut(server) {
            l.rung = 0;
            l.attempt = 0;
        }
    }

    /// WS 점검 — 같은 치료를 다시 할 때의 간격은 사다리 칸값(정책:sdk.iceRestartBackoffMs = 연 사다리) · 시도 번호 = 칸 번호(`check_rung`) · 끝 칸 되풀이 · 상한 없음
    fn ws_check(&mut self, server: &str) {
        let active = self.ses.cs == sc::Conn::ACTIVE;
        let Some(l) = self.links.get_mut(server) else { return };
        if !active {
            l.pending_check = true;
            return;
        }
        l.check_before = (l.attempt, l.rung);
        let (rung, wait) = check_rung(l.attempt, sc::LADDER.waits);
        l.rung = rung;
        l.attempt += 1;
        self.timers.start(Tk::LinkCheck(server.to_string()), Duration::from_millis(wait));
    }

    pub(super) fn link_check_send(&mut self, server: &str) {
        let active = self.ses.cs == sc::Conn::ACTIVE;
        let Some(l) = self.links.get_mut(server) else { return };
        if l.st.ls != lc::LinkSt::CHECK {
            return;
        }
        if !active {
            // 보내지 않은 점검은 치료로 세지 않는다 — 세션이 돌아오면 같은 칸에서 다시
            (l.attempt, l.rung) = l.check_before;
            l.pending_check = true;
            return;
        }
        l.probe_since = Some(Instant::now());
        self.request(Op::Heartbeat, Value::Null, Purpose::Heartbeat);
        self.timers.start(Tk::LinkProbe(server.to_string()), Duration::from_millis(dflt::WS_PROBE_TIMEOUT_MS as u64));
    }

    /// 점검 시한 — 그 사이 무엇이든 받았으면 산 것이다 · 아니면 소켓을 LEAVE 없이 닫는다
    pub(super) fn link_probe_due(&mut self, server: &str) {
        let Some(l) = self.links.get(server) else { return };
        if l.st.ls != lc::LinkSt::CHECK {
            return;
        }
        let alive = matches!((self.sig.last_rx(), l.probe_since), (Some(rx), Some(since)) if rx >= since);
        if alive {
            let s = lc::hb_res(&l.st, &lc::HbResMsg {});
            self.link_step(server, "hbRes", Some(s), "HEARTBEAT");
        } else {
            let mut ended = std::mem::take(&mut self.ended);
            self.sig.close(&mut ended);
            self.ended = ended;
            let g = self.sig.generation();
            self.sock_closed_now(g);
        }
    }

    // ── 표면 · 경로표 ───────────────────────────────────────────────

    fn link_surface(&mut self, server: &str) {
        let Some(l) = self.links.get_mut(server) else { return };
        let i = l.info();
        if l.last_info.as_ref() == Some(&i) {
            return;
        }
        l.last_info = Some(i.clone());
        self.emit(Event::Link(i));
        self.surface(false);
    }

    /// 받는 경로표 — 배정표 줄(ssrc · mid · 트랙 · RTX 짝) + 방 보관본(주인) + 서버 코덱 정책(`rtcp_fb`) + 앱이 고른 단위 · 층 차례 · 미디어 경로가 이것으로 가른다
    pub(super) fn rebuild_routes(&mut self) {
        let mut routes = Routes::default();
        for l in self.links.values() {
            let Some(t) = &l.table else { continue };
            // 구독 피드백은 `codecs_sub` 가 오면 그것 · 없으면 `codecs`(연:RULE-LINK-SDP-SUB-MLINE)
            let policy = l.sc.codecs_sub.as_ref().unwrap_or(&l.sc.codecs);
            for m in &t.mlines {
                let Some(tid) = &m.track_id else { continue };
                let room = m.room_id.clone().or_else(|| self.rooms.values().find(|r| r.sfu.as_deref() == Some(l.server.as_str()) && r.entries.contains_key(tid)).map(|r| r.id.clone()));
                let Some(room) = room else { continue };
                let entry = self.rooms.get(&room).and_then(|r| r.entries.get(tid));
                let kind = serde_json::to_value(m.kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
                let fb: Vec<&str> = policy.iter().filter(|c| c.kind == m.kind && c.name.eq_ignore_ascii_case(&m.codec)).flat_map(|c| c.rtcp_fb.iter().map(String::as_str)).collect();
                let route = Route {
                    room,
                    track_id: tid.clone(),
                    user_id: entry.and_then(|e| e.user_id.clone()),
                    kind,
                    codec: m.codec.clone(),
                    mid: m.mid.clone(),
                    ssrc: m.ssrc,
                    pt: m.pt,
                    nack: fb.contains(&"nack"),
                    pli: fb.contains(&"nack pli"),
                    unit: self.units.get(tid).copied().unwrap_or(self.unit),
                    layer_gen: self.layer_gen.get(tid).copied().unwrap_or(0),
                    key_gen: self.key_gen.get(tid).copied().unwrap_or(0),
                };
                if let Some(x) = m.rtx_ssrc {
                    routes.rtx.insert(x, m.ssrc);
                }
                routes.media.insert(m.ssrc, route);
            }
        }
        for r in self.rooms.values() {
            for (tid, e) in &r.entries {
                if let Some(c) = e.csrc {
                    routes.sources.insert(c, Source { csrc: c, user_id: e.user_id.clone(), track_id: Some(tid.clone()) });
                }
            }
        }
        for (c, u) in &self.talk_sources {
            routes.sources.entry(*c).or_insert_with(|| Source { csrc: *c, user_id: Some(u.clone()), track_id: None });
        }
        // 트랙 목록 전체 + 줄 SSRC → track_id(SDK:RULE-PLAT-REC-RAW `ctl/track`) — 방마다 앞과 같으면 기록기가 거른다
        #[cfg(feature = "record")]
        if let Some(rec) = &self.sink.recorder {
            let me = self.user_id.clone();
            for r in self.rooms.values() {
                let tracks: Vec<&crate::generated::ws_wire::TrackEntry> = r.entries.values().filter(|t| !r.own(t, me.as_deref())).collect();
                let rows: std::collections::BTreeMap<String, serde_json::Value> = routes.media.iter().filter(|(_, x)| x.room == r.id).map(|(k, x)| (k.to_string(), serde_json::json!({ "track_id": x.track_id, "kind": x.kind, "codec": x.codec, "pt": x.pt }))).collect();
                use crate::generated::room_client::RoomSt;
                let phase = match r.st.rs {
                    _ if r.closed => crate::record::RoomPhase::Gone,
                    RoomSt::JOINED => crate::record::RoomPhase::Joined,
                    RoomSt::JOINING => crate::record::RoomPhase::Rejoining,
                    RoomSt::NONE if r.st.rejoin => crate::record::RoomPhase::Rejoining,
                    _ => crate::record::RoomPhase::Gone,
                };
                if crate::record::track_fact(phase, tracks.len(), rows.len()) {
                    rec.track(&r.id, serde_json::json!({ "room": r.id, "tracks": tracks, "rows": rows }));
                }
            }
        }
        if let Ok(mut g) = self.sink.routes.write() {
            *g = routes;
        }
    }
}

/// 시도 번호 = 칸 번호 — 첫 점검(`attempt` 0)은 칸[0] · n 번째 다시 하기는 칸[n] · 칸을 다 쓰면 끝 칸(연:RULE-LINK-RECOVERY-UNIT) · 반환 = (쓴 칸, 기다릴 ms)
pub(crate) fn check_rung(attempt: u32, waits: &[i64]) -> (u32, u64) {
    let i = (attempt as usize).min(waits.len().saturating_sub(1));
    (i as u32, waits.get(i).copied().unwrap_or(0).max(0) as u64)
}

#[cfg(test)]
mod check_rung_test {
    use super::check_rung;

    #[test]
    fn the_attempt_number_is_the_rung_number() {
        let waits = crate::generated::session_client::LADDER.waits;
        let seq: Vec<u64> = (0..12).map(|a| check_rung(a, waits).1).collect();
        assert_eq!(&seq[..4], &[0, 300, 1200, 2700], "PL-SDK-ICE-RESTART-BACKOFF-MS 첫 점검 칸[0] · n 번째 다시 하기 칸[n]: {seq:?}");
        assert_eq!(seq.iter().filter(|w| **w == 0).count(), 1, "PL-SDK-ICE-RESTART-BACKOFF-MS 0ms 는 첫 점검 하나뿐: {seq:?}");
        assert!(seq[10..].iter().all(|w| *w == *waits.last().unwrap_or(&0) as u64), "PL-SDK-ICE-RESTART-BACKOFF-MS 칸을 다 쓰면 끝 칸 되풀이: {seq:?}");
        assert_eq!(check_rung(1, waits).0, 1, "stand check - 쓴 칸이 앱에 나가는 rung");
    }
}

#[cfg(test)]
mod renego_gate_test {
    use super::RenegoGate;

    #[test]
    fn requests_inside_one_step_become_one_at_its_end() {
        let mut g = RenegoGate::default();
        assert!(g.ask(), "걸음 밖 요청은 곧바로");
        g.enter();
        let now = [g.ask(), g.ask()];
        g.leave();
        assert_eq!(now, [false, false], "걸음 안 요청 둘은 그 자리에서 걷지 않는다");
        assert!(g.take(), "C-sym-R-ROOM-DETAIL-SYMPTOMS-3 걸음 끝에 재협상 하나(방 둘의 요청 둘 → 하나)");
        assert!(!g.take(), "C-sym-R-ROOM-DETAIL-SYMPTOMS-3 한 번만");
        g.enter();
        g.enter();
        assert!(!g.ask());
        g.leave();
        assert!(!g.take(), "안쪽 걸음 끝은 아직 — 바깥 걸음 끝에");
        g.leave();
        assert!(g.take(), "바깥 걸음 끝에 하나");
    }
}

#[cfg(test)]
mod wire_test {
    use super::publish_creds;
    use crate::generated::ws_wire::ServerConfigIce;

    #[test]
    fn the_one_transport_takes_the_publish_creds() {
        let i: ServerConfigIce = serde_json::from_value(serde_json::json!({ "ip": "127.0.0.1", "port": 19750, "publish_ufrag": "pubA", "publish_pwd": "pwA", "subscribe_ufrag": "subB", "subscribe_pwd": "pwB", "tcp_port": 0 })).expect("ice");
        let c = publish_creds(&i);
        assert_eq!((c.ufrag.as_str(), c.pwd.as_str()), ("pubA", "pwA"), "SDK:RULE-PLAT-SERVER 1pc 서버마다 전송 하나 · 발행 자격(구독 자격은 쓰지 않는다 — ICE_RESTART 응답에도 없다)");
    }
}
