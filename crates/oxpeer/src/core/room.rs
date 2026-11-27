// author: kodeholic (powered by Claude)
//! 방 하나 — 방(ROOM) · 받는 트랙(TRK) · 구독(SUB) · 발언권 듣기(FLR) 상태기를 방마다 하나씩 쥔다(SDK:IMPL-ROOM-API · SDK:IMPL-TRK-RECEIVE · SDK:IMPL-FLR-EXPOSE).
//! 절차는 위 상태기 · 이벤트는 결합이 정한다 — 여기는 입구 표 · 효과 표 · 이음매 · 보낼 것 집행만.
//! ★발행 · 소속 영역은 첫 단계에 짓지 않았다(20261106c A3 — 녹음은 받기만 · 듣기로 들어간다) — 그 영역으로 가는 이음매는 받을 상태가 없어 할 일이 없다.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::{Core, Tk};
use crate::api::{Event, Layer};
use crate::error::{Category, Error};
use crate::generated::ws_wire::{MemberInfo, Op, ReadyReq, RoomJoinReq, RoomLeaveReq, SubscribeLayerReq, SubscribeLayerReqTargets, TrackEntry, TrackEntryType};
use crate::generated::{floor_client as fc, ptt_binding as pb, room_binding as rb, room_client as rc, session_client as sc, sub_binding as ub, sub_client as uc, trk_binding as tb, trk_client as tc};
use crate::sig::{Outcome, Purpose};
use crate::wire::dc;

/// 입장 응답 · 이어받기 · 재동기가 실어 온 보관본 — 이음매 trk.snapshot 이 쓴다
pub(crate) struct Carried {
    participants: Vec<MemberInfo>,
    me: Option<MemberInfo>,
    tracks: Option<Vec<TrackEntry>>,
    sub_rooms: Option<Vec<String>>,
}

pub(crate) struct Room {
    pub(crate) id: String,
    pub(crate) st: rc::St,
    pub(crate) trk: tc::St,
    pub(crate) sub: uc::St,
    pub(crate) flr: fc::Cl,
    pub(crate) sfu: Option<String>,
    pub(crate) entries: BTreeMap<String, TrackEntry>,
    pub(crate) participants: Vec<MemberInfo>,
    pub(crate) me: Option<MemberInfo>,
    pub(crate) req_epoch: u64,
    pub(crate) sync_epoch: u64,
    retries: usize,
    retry_after_ms: u64,
    resend_jid: i64,
    resync_tries: usize,
    join_waiters: Vec<oneshot::Sender<Result<(), Error>>>,
    leave_waiters: Vec<oneshot::Sender<Result<(), Error>>>,
    pub(crate) join_pending: bool,
    last_fault: Option<Error>,
    carried: Option<Carried>,
    notice: Option<(Op, Value)>,
    join_table: Option<Value>,
    /// 그 방의 미디어 코덱(입장 응답 — 발행은 이것으로 · 연:RULE-ROOM-MEDIA-CODECS) · 이 방에서 말하려고 들어간다(select:true) · 입장 응답의 발행 방
    pub(crate) media: Option<crate::generated::ws_wire::MediaCodecs>,
    pub(crate) select: bool,
    /// 마지막으로 보낸 ROOM_JOIN 의 select — 응답의 소속 몫(implicit)이 제 요청의 select 를 쓴다 · ★재구축의 다시 들어감은 늘 false(연:RULE-LINK-RECOVERY-REBUILD · context 20261113j ③-4)
    join_sel: bool,
    /// 들어간 뒤 소속 전환으로 발언 방을 옮긴다(이미 발행 방이 있었다)
    pub(crate) select_after: bool,
    aff_pub: Option<Option<String>>,
    pub(crate) rebuilding: bool,
    forced_cause: String,
    pub(crate) closed: bool,
    /// 앱이 고른 층(트랙마다 · 합친 것) — 다시 들어가면 다시 보낸다(서버는 새 입장에서 모른다)
    layers: BTreeMap<String, SubscribeLayerReqTargets>,
    /// 다음 SUBSCRIBE_LAYER 가 답할 앱 요청 · 보낸 요청마다 기다리는 앱
    layer_next: Option<oneshot::Sender<Result<(), Error>>>,
    layer_waiters: BTreeMap<u64, oneshot::Sender<Result<(), Error>>>,
    layer_seq: u64,
    /// 발언권 표면 거울(SDK:IMPL-FLR-EXPOSE) — 받은 거절 · 회수 사유는 효과가 날 때 `last_*` 로 옮긴다
    pub(crate) ptt_rx: Option<(bool, crate::api::FloorCause)>,
    pub(crate) last_deny: Option<crate::api::FloorCause>,
    pub(crate) last_revoke: Option<crate::api::FloorCause>,
    pub(crate) queue_position: Option<u8>,
}

impl Room {
    /// 이번 ROOM_JOIN 의 select — 앱 입장이면 그때 소속이 정한 값 · ★재구축의 다시 들어감은 false(연:RULE-LINK-RECOVERY-REBUILD — 발행 방은 들어간 뒤 되찾는다)
    fn join_select(&self) -> bool {
        self.select && !self.rebuilding
    }

    fn new(id: &str) -> Room {
        Room {
            id: id.to_string(),
            st: rc::initial(),
            trk: tc::St { lay: Default::default(), muted: Default::default() },
            sub: uc::initial(),
            flr: fc::initial(),
            sfu: None,
            entries: BTreeMap::new(),
            participants: Vec::new(),
            me: None,
            req_epoch: 0,
            sync_epoch: 0,
            retries: 0,
            retry_after_ms: 1_000,
            resend_jid: 0,
            resync_tries: 0,
            join_waiters: Vec::new(),
            leave_waiters: Vec::new(),
            join_pending: false,
            last_fault: None,
            carried: None,
            notice: None,
            join_table: None,
            media: None,
            select: false,
            join_sel: false,
            select_after: false,
            aff_pub: None,
            rebuilding: false,
            forced_cause: "kick".into(),
            closed: false,
            layers: BTreeMap::new(),
            ptt_rx: None,
            last_deny: None,
            last_revoke: None,
            queue_position: None,
            layer_next: None,
            layer_waiters: BTreeMap::new(),
            layer_seq: 0,
        }
    }

    /// 내 개인 트랙 — 받을 트랙이 아니다
    pub(super) fn own(&self, t: &TrackEntry, me: Option<&str>) -> bool {
        t.type_ == TrackEntryType::Individual && t.user_id.as_deref() == me && me.is_some()
    }
}

/// 재시도 사다리 둘째 ~ 넷째 칸(연:RULE-ERR-CLIENT-FAIL-DEFAULT 4 → 연:RULE-SES-BACKOFF-LADDER)
fn retry_wait(i: usize) -> u64 {
    let w = sc::LADDER.waits;
    w[(i + 1).min(w.len() - 1)] as u64
}

pub(crate) fn pc_mode_fault(res: crate::generated::ws_wire::PcMode, session: super::pcmode::Mode) -> Option<Error> {
    (super::pcmode::Mode::of_wire(res) != session).then(|| Error::sdk("PC_MODE", Category::Negotiation))
}

/// 재시도 없이 앱에(연:ROOM.C.JOINING — ROOM_NOT_FOUND · ROOM_FULL · LISTEN_LIMIT · MID_LIMIT)
const NO_RETRY: &[u16] = &[3001, 4001, 4004, 4005];

impl Core {
    // ── 앱 조작 ──────────────────────────────────────────────────────

    pub(super) fn join(&mut self, room: String, tx: oneshot::Sender<Result<(), Error>>) {
        if self.ses.cs != sc::Conn::ACTIVE {
            if self.ses.cs == sc::Conn::DISCONNECTED {
                let _ = tx.send(Err(Error::closed("CLOSED")));
            } else {
                self.parked.push((room, tx));
            }
            return;
        }
        if let Some(r) = self.rooms.get_mut(&room) {
            if r.st.rs == rc::RoomSt::JOINED && !r.join_pending {
                let _ = tx.send(Ok(()));
            } else {
                r.join_waiters.push(tx);
            }
            return;
        }
        let mut r = Room::new(&room);
        r.join_waiters.push(tx);
        if let Some((m, sel)) = self.join_mode.take().filter(|(m, _)| *m == room) {
            let _ = m;
            if sel {
                r.select = true;
            } else {
                r.select_after = true;
            }
        }
        self.rooms.insert(room.clone(), r);
        let s = rc::join(&self.rooms[&room].st);
        self.room_step(&room, "join", s, "app");
    }

    pub(super) fn leave(&mut self, room: String, tx: oneshot::Sender<Result<(), Error>>) {
        let Some(r) = self.rooms.get_mut(&room) else {
            let _ = tx.send(Ok(()));
            return;
        };
        if r.st.rs == rc::RoomSt::NONE && !r.st.rejoin {
            r.closed = true;
            let _ = tx.send(Ok(()));
            self.finish_rooms();
            return;
        }
        r.leave_waiters.push(tx);
        r.join_pending = false;
        let s = rc::leave(&r.st);
        self.room_step(&room, "leave", s, "app");
    }

    // ── 한 걸음 ──────────────────────────────────────────────────────

    pub(super) fn room_step(&mut self, id: &str, entry: &'static str, step: Option<rc::Step>, cause: &str) -> bool {
        let Some(step) = step else {
            if let Some(r) = self.rooms.get(id) {
                self.impossible("ROOM", id, r.st.rs.as_str(), entry);
            }
            return false;
        };
        let Some(r) = self.rooms.get_mut(id) else { return false };
        let prev = r.st.clone();
        let fx = step.fx.clone();
        let plan = rb::plan(step);
        r.st = plan.state.clone();
        let label = plan.label;
        self.record("ROOM", id, prev.rs.as_str(), plan.state.rs.as_str(), label, cause);
        for a in &plan.actions {
            match a {
                rb::RoomAction::ResyncAbandon => {
                    if let Some(r) = self.rooms.get_mut(id) {
                        r.sync_epoch += 1;
                    }
                }
                // 그 방에 걸린 요청의 응답을 버린다 — 같은 걸음에 보낼 것은 버림 뒤의 새 세대로(SDK:IMPL-ROOM-API ws.abandon)
                rb::RoomAction::RequestAbandon => {
                    if let Some(r) = self.rooms.get_mut(id) {
                        r.req_epoch += 1;
                    }
                    let rid = id.to_string();
                    self.sig.forget(|p| matches!(p, Purpose::RoomJoin { room, .. } | Purpose::RoomLeave { room, .. } | Purpose::Ready { room, .. } | Purpose::Layer { room, .. } if *room == rid));
                    if let Some(r) = self.rooms.get_mut(id) {
                        for (_, tx) in std::mem::take(&mut r.layer_waiters) {
                            let _ = tx.send(Err(Error::closed("ABANDONED")));
                        }
                    }
                }
            }
        }
        for tx in &plan.send {
            self.room_send(id, tx);
        }
        for sm in &plan.seams {
            self.room_seam(id, sm.fx, label);
        }
        for f in &fx {
            self.room_effect(id, *f);
        }
        if prev.rs != rc::RoomSt::JOINED && self.rooms.get(id).is_some_and(|r| r.st.rs == rc::RoomSt::JOINED) {
            self.pub_room_joined(id);
        }
        if let Some(r) = self.rooms.get_mut(id) {
            if r.st.retry && !prev.retry {
                let ms = r.retry_after_ms;
                self.timers.start(Tk::RoomRetry(id.to_string()), Duration::from_millis(ms));
            }
            if !self.rooms[id].st.retry {
                self.timers.stop(&Tk::RoomRetry(id.to_string()));
            }
        }
        self.finish_rooms();
        self.surface(false);
        true
    }

    /// 닫힌 방을 걷는다 — 기다리던 입장 · 퇴장을 끝내고 시계 · 경로를 치운다
    pub(super) fn finish_rooms(&mut self) {
        let gone: Vec<String> = self.rooms.iter().filter(|(_, r)| r.closed).map(|(k, _)| k.clone()).collect();
        for id in gone {
            if let Some(mut r) = self.rooms.remove(&id) {
                for tx in r.join_waiters.drain(..) {
                    let _ = tx.send(Err(r.last_fault.clone().unwrap_or_else(|| Error::closed("CLOSED"))));
                }
                for tx in r.leave_waiters.drain(..) {
                    let _ = tx.send(Ok(()));
                }
                let rid = id.clone();
                self.timers.stop_where(|t| matches!(t, Tk::RoomRetry(x) | Tk::RoomResend(x) | Tk::Resync(x) | Tk::Floor(x, _) if *x == rid));
            }
            self.jitter_rooms.retain(|x| *x != id);
            self.rebuild_routes();
        }
    }

    fn room_send(&mut self, id: &str, tx: &rc::Tx) {
        match tx {
            rc::Tx::RoomJoin { join_id } => self.send_join(id, *join_id),
            rc::Tx::RoomLeave { join_id } => {
                let req = RoomLeaveReq { room_id: id.to_string(), join_id: self.jids.str(*join_id).map(str::to_string) };
                let epoch = self.rooms[id].req_epoch;
                self.request(Op::RoomLeave, super::to_body(&req), Purpose::RoomLeave { room: id.to_string(), epoch });
            }
            rc::Tx::GetRoom { .. } => self.resync(id),
        }
    }

    /// 입장 · jid = 갈아 끼울 입장의 join_id(재구축 · 0 = 새 입장 — 연:RULE-ROOM-JOIN-REPLACE)
    /// select — 앱 입장은 그때 소속이 정한 값(연:RULE-ROOM-JOIN-START 2) · ★재구축의 다시 들어감은 false — 발행 방은 들어간 뒤 되찾는다
    ///   (처음 입장의 select 를 되실으면 다른 서버가 이미 발행 방을 가졌을 때 발행 방이 둘이 된다 — 소속 판 rebuild_sel 반례 AH2 · context 20261113j ③-4)
    fn send_join(&mut self, id: &str, jid: i64) {
        let sel = self.rooms.get(id).is_some_and(Room::join_select);
        if let Some(r) = self.rooms.get_mut(id) {
            r.join_sel = sel;
        }
        let (extmap, codecs) = self.pc_mode().join_tags();
        let req = RoomJoinReq { room_id: id.to_string(), role: None, select: Some(sel), extmap, codecs, join_id: self.jids.str(jid).map(str::to_string) };
        let epoch = self.rooms[id].req_epoch;
        self.request(Op::RoomJoin, super::to_body(&req), Purpose::RoomJoin { room: id.to_string(), epoch, jid });
    }

    pub(super) fn room_resend(&mut self, id: &str) {
        let Some(r) = self.rooms.get(id) else { return };
        if r.st.rs == rc::RoomSt::JOINING && !r.st.quit {
            let jid = r.resend_jid;
            self.send_join(id, jid);
        }
    }

    pub(super) fn room_retry_due(&mut self, id: &str) {
        let Some(r) = self.rooms.get(id) else { return };
        let s = rc::retry_due(&r.st);
        self.room_step(id, "retryDue", s, "clock");
    }

    pub(super) fn pc_mode(&self) -> super::pcmode::Mode {
        super::pcmode::Mode::of_session(self.ses.pcm)
    }

    fn pcm(&self) -> rc::PcMode {
        if self.pc_mode() == super::pcmode::Mode::One { rc::PcMode::V1pc } else { rc::PcMode::V2pc }
    }

    fn fail_msg(&self, code: i64) -> rc::JoinResMsg {
        rc::JoinResMsg {
            code,
            join_id: 0,
            part: rc::Part::fail,
            server_config: rc::JoinResMsgServerConfig { pc_mode: self.pcm(), sfu_id: 0 },
            version: rc::JoinResMsgVersion { epoch: 0, seq: 0 },
        }
    }

    pub(super) fn join_reply(&mut self, id: &str, epoch: u64, jid: i64, o: Outcome) {
        let Some(r) = self.rooms.get(id) else { return };
        if r.req_epoch != epoch {
            return;
        }
        let pcm = self.pcm();
        match o {
            Outcome::Ok(b) => {
                let parsed: Result<crate::generated::ws_wire::RoomJoinRes, _> = serde_json::from_value(b.clone());
                let Ok(res) = parsed else {
                    let e = Error { code: 1002, name: "INVALID_PAYLOAD".into(), category: Category::Bug, permanent: true, message: parsed.err().map(|e| e.to_string()) };
                    if let Some(r) = self.rooms.get_mut(id) {
                        r.last_fault = Some(e);
                    }
                    let m = self.fail_msg(1002);
                    let s = rc::join_res(&self.rooms[id].st, &m, pcm);
                    self.room_step(id, "joinRes", s, "ROOM_JOIN.bad");
                    return;
                };
                let sc_ = &res.server_config;
                let m = rc::JoinResMsg {
                    code: 0,
                    join_id: self.jids.id(&res.join_id),
                    part: rc::Part::res,
                    server_config: rc::JoinResMsgServerConfig {
                        pc_mode: if sc_.pc_mode == crate::generated::ws_wire::PcMode::V1pc { rc::PcMode::V1pc } else { rc::PcMode::V2pc },
                        sfu_id: self.epochs.id(&sc_.sfu_id),
                    },
                    version: rc::JoinResMsgVersion { epoch: self.epochs.id(&res.version.epoch), seq: res.version.seq as i64 },
                };
                if let Some(f) = pc_mode_fault(sc_.pc_mode, self.pc_mode())
                    && let Some(r) = self.rooms.get_mut(id) {
                        r.last_fault = Some(f);
                    }
                // 이 응답으로 들어가는가는 위 코드가 정한다(미리 보기 — 상태를 바꾸지 않는다) · 들어가면 그 서버 연결부터 세운다
                let joins = rc::join_res(&self.rooms[id].st, &m, pcm).is_some_and(|s| s.s.rs == rc::RoomSt::JOINED);
                if joins {
                    if let Some(r) = self.rooms.get_mut(id) {
                        r.sfu = Some(sc_.sfu_id.clone());
                        r.retries = 0;
                    }
                    self.link_adopt(sc_.clone());
                }
                if let Some(r) = self.rooms.get_mut(id) {
                    r.carried = Some(Carried { participants: res.participants.clone(), me: Some(res.me.clone()), tracks: Some(res.tracks.clone()), sub_rooms: Some(res.affiliation.sub_rooms.clone()) });
                    r.join_table = serde_json::to_value(&res.mline_table).ok();
                    r.media = Some(res.media_codecs.clone());
                    r.aff_pub = Some(res.affiliation.pub_room.clone());
                }
                let s = rc::join_res(&self.rooms[id].st, &m, pcm);
                self.room_step(id, "joinRes", s, "ROOM_JOIN.res");
                if let Some(r) = self.rooms.get_mut(id) {
                    r.carried = None;
                    r.join_table = None;
                }
            }
            Outcome::Fail(b) => {
                let f = Error::from_failure(&b);
                let retry = !f.permanent && !NO_RETRY.contains(&f.code) && f.code != 4009 && f.code >= 2000;
                let Some(r) = self.rooms.get_mut(id) else { return };
                r.last_fault = Some(f.clone());
                if retry && r.retries < 3 {
                    let wait = retry_wait(r.retries);
                    r.retries += 1;
                    r.resend_jid = jid;
                    self.timers.start(Tk::RoomResend(id.to_string()), Duration::from_millis(wait));
                    return;
                }
                if f.code == 4009 {
                    r.retry_after_ms = b.get("details").and_then(|d| d.get("retry_after_ms")).and_then(Value::as_u64).unwrap_or(1_000);
                }
                let m = self.fail_msg(i64::from(f.code));
                let s = rc::join_res(&self.rooms[id].st, &m, pcm);
                self.room_step(id, "joinRes", s, "ROOM_JOIN.fail");
            }
            Outcome::Expired => {
                if let Some(r) = self.rooms.get_mut(id) {
                    r.last_fault = Some(Error::closed("REQ_EXPIRED"));
                }
                let s = rc::req_expired(&self.rooms[id].st);
                self.room_step(id, "reqExpired", s, "ROOM_JOIN.expired");
            }
            Outcome::Transport | Outcome::Closed => {}
        }
    }

    pub(super) fn leave_reply(&mut self, id: &str, epoch: u64, o: Outcome) {
        let Some(r) = self.rooms.get(id) else { return };
        if r.req_epoch != epoch {
            return;
        }
        let ok = match o {
            Outcome::Ok(_) => true,
            Outcome::Fail(b) => {
                let f = Error::from_failure(&b);
                if f.code != 3001 && f.code != 3002 {
                    self.emit(Event::Error { room: Some(id.to_string()), error: f });
                }
                false
            }
            Outcome::Expired => {
                let s = rc::req_expired(&r.st);
                self.room_step(id, "reqExpired", s, "ROOM_LEAVE.expired");
                return;
            }
            Outcome::Transport | Outcome::Closed => false,
        };
        let s = rc::leave_res(&self.rooms[id].st, &rc::LeaveResMsg { part: if ok { rc::Part::res } else { rc::Part::fail } });
        self.room_step(id, "leaveRes", Some(s), if ok { "ROOM_LEAVE.res" } else { "ROOM_LEAVE.fail" });
    }

    // ── 통지 · 위 이음매가 부르는 입구 ───────────────────────────────

    /// 방 통지 — 견주기는 위 방 상태기 한 곳(입구 delta)
    pub(super) fn room_delta(&mut self, id: &str, op: Op, body: Value) {
        let v = body.get("version");
        let bare = op == Op::ParticipantState && v.is_none();
        let cepoch = self.rooms[id].st.cepoch;
        let epoch = v.and_then(|v| v.get("epoch")).and_then(Value::as_str).map(|e| self.epochs.id(e)).unwrap_or(cepoch);
        let seq = v.and_then(|v| v.get("seq")).and_then(Value::as_i64).unwrap_or(0);
        let Some(t) = rc::DeltaMsgT::parse(super::op_name(op)) else { return };
        let m = rc::DeltaMsg { t, version: rc::DeltaMsgVersion { epoch, seq } };
        if let Some(r) = self.rooms.get_mut(id) {
            r.notice = Some((op, body));
        }
        let s = rc::delta(&self.rooms[id].st, &m, bare);
        self.room_step(id, "delta", Some(s), super::op_name(op));
        if let Some(r) = self.rooms.get_mut(id) {
            r.notice = None;
        }
    }

    pub(super) fn room_event(&mut self, id: &str, body: &Value) {
        let affiliation = body.get("type").and_then(Value::as_str) == Some("affiliation");
        if affiliation
            && let Some(r) = self.rooms.get_mut(id) {
                r.forced_cause = body.get("cause").and_then(Value::as_str).unwrap_or("kick").to_string();
            }
        let m = rc::RoomEventMsg { r#type: if affiliation { rc::RoomEv::affiliation } else { rc::RoomEv::sync_required } };
        let s = rc::room_event(&self.rooms[id].st, &m);
        self.room_step(id, "roomEvent", Some(s), "ROOM_EVENT");
    }

    pub(super) fn room_down_all(&mut self, id: &str) {
        let Some(r) = self.rooms.get(id) else { return };
        let s = rc::down_all(&r.st);
        self.room_step(id, "downAll", Some(s), "rebuild");
        if let Some(r) = self.rooms.get_mut(id) {
            r.rebuilding = r.st.rejoin;
        }
    }

    pub(super) fn room_close_all(&mut self, id: &str) {
        let Some(r) = self.rooms.get(id) else { return };
        let s = rc::close_all(&r.st);
        self.room_step(id, "closeAll", Some(s), "session");
        if let Some(r) = self.rooms.get_mut(id) {
            r.closed = true;
        }
        self.finish_rooms();
    }

    /// 재구축 칸의 첫 ROOM_JOIN 은 0 ~ massJitterMs 균등 무작위로 늦춘다(연:RULE-SES-REBUILD-JITTER)
    pub(super) fn rejoin_all(&mut self, rooms: Vec<String>) {
        let marked: Vec<String> = rooms.into_iter().filter(|r| self.rooms.get(r).is_some_and(|x| x.st.rejoin)).collect();
        if marked.is_empty() {
            return;
        }
        for r in marked {
            if !self.jitter_rooms.contains(&r) {
                self.jitter_rooms.push(r);
            }
        }
        if !self.timers.armed(&Tk::Jitter) {
            let ms = super::ses::rand_below(crate::generated::sdk_defaults::MASS_JITTER_MS as u64);
            self.timers.start(Tk::Jitter, Duration::from_millis(ms));
        }
    }

    pub(super) fn jitter_due(&mut self) {
        for id in std::mem::take(&mut self.jitter_rooms) {
            let Some(r) = self.rooms.get_mut(&id) else { continue };
            r.rebuilding = true;
            let s = rc::rejoin(&r.st);
            self.room_step(&id, "rejoin", s, "rebuild");
        }
    }

    /// 이어받기 응답의 그 방 몫 — 들어가 있지 않은 방이면 사건이 없던 것이다
    pub(super) fn room_resume_snap(&mut self, id: &str, snap: Option<Value>) {
        let parsed: Option<crate::generated::ws_wire::RoomSnapshot> = snap.and_then(|v| serde_json::from_value(v).ok());
        let Some(r) = self.rooms.get_mut(id) else { return };
        if parsed.is_none() {
            r.forced_cause = "missing".into();
        }
        r.carried = parsed.as_ref().map(|s| Carried { participants: s.participants.clone(), me: Some(s.me.clone()), tracks: Some(s.tracks.clone()), sub_rooms: None });
        let (seq, epoch) = match &parsed {
            Some(s) => (s.version.seq as i64, self.epochs.id(&s.version.epoch)),
            None => (0, 0),
        };
        let s = rc::resume_snap(&self.rooms[id].st, parsed.is_some(), seq, epoch);
        if s.is_some() {
            self.room_step(id, "resumeSnap", s, "RESUME.snapshot");
        }
        if let Some(r) = self.rooms.get_mut(id) {
            r.carried = None;
        }
    }

    // ── 재동기 — HTTP · 실패는 사다리 칸 간격으로 다시(SDK:RULE-ROOM-RESYNC) ──

    pub(super) fn resync(&mut self, id: &str) {
        let Some(r) = self.rooms.get(id) else { return };
        let epoch = r.sync_epoch;
        self.spawn_resync(id, epoch);
    }

    pub(super) fn resync_done(&mut self, id: String, epoch: u64, result: Result<crate::http::Reply, Error>) {
        let Some(r) = self.rooms.get_mut(&id) else { return };
        if r.sync_epoch != epoch {
            return;
        }
        let res = result.and_then(|rep| {
            if rep.status == 401 {
                return Err(Error { code: 2003, name: "TOKEN_EXPIRED".into(), category: Category::Auth, permanent: false, message: None });
            }
            if rep.status >= 400 {
                return Err(Error::from_failure(&rep.body));
            }
            serde_json::from_value::<crate::generated::ws_wire::GetRoomRes>(rep.body).map_err(|e| Error::transport(format!("재동기 응답: {e}")))
        });
        match res {
            Ok(g) => {
                r.resync_tries = 0;
                r.carried = Some(Carried { participants: g.participants.clone(), me: g.me.clone(), tracks: g.tracks.clone(), sub_rooms: None });
                let m = rc::SnapshotMsg { version: rc::SnapshotMsgVersion { epoch: self.epochs.id(&g.version.epoch), seq: g.version.seq as i64 } };
                let s = rc::snapshot(&self.rooms[&id].st, &m);
                let took = self.room_step(&id, "snapshot", Some(s), "GET_ROOM.res");
                if let Some(r) = self.rooms.get_mut(&id) {
                    r.carried = None;
                }
                if took {
                    self.emit(Event::Resync { room: id });
                }
            }
            Err(e) if e.category == Category::Auth => self.emit(Event::TokenRequired),
            Err(e) => {
                let n = r.resync_tries;
                r.resync_tries += 1;
                if n == 2 {
                    self.emit(Event::Error { room: Some(id.clone()), error: e });
                }
                self.timers.start(Tk::Resync(id), Duration::from_millis(retry_wait(n.min(2))));
            }
        }
    }

    // ── 이음매 — 받는 곳은 위가 정했다 ───────────────────────────────

    fn room_seam(&mut self, id: &str, fx: rc::SeamFx, label: &str) {
        let server = self.rooms.get(id).and_then(|r| r.sfu.clone());
        match fx {
            rc::SeamFx::trk_snapshot => {
                if let Some(r) = self.rooms.get_mut(id)
                    && let Some(c) = r.carried.take() {
                        r.participants = c.participants.clone();
                        r.me = c.me.clone();
                        if let Some(ts) = &c.tracks {
                            r.entries = ts.iter().map(|t| (t.track_id.clone(), t.clone())).collect();
                        }
                        r.carried = Some(c);
                        let p = r.participants.clone();
                        self.emit(Event::Participants { room: id.to_string(), participants: p });
                    }
                self.trk_snapshot(id, label);
            }
            rc::SeamFx::trk_delta => self.trk_delta(id),
            rc::SeamFx::store_drop => self.trk_store_drop(id, label),
            rc::SeamFx::ready_tracks => self.sub_enter(id, "ready", |s| Some(uc::ready(s)), label),
            rc::SeamFx::sub_drop => self.sub_enter(id, "dropRoom", |s| Some(uc::drop_room(s)), label),
            rc::SeamFx::floor_join => {
                self.floor_enter(id, "join", |s| Some(fc::join(s)), label);
            }
            rc::SeamFx::floor_leave => {
                self.floor_enter(id, "leave", |s| Some(fc::leave(s)), label);
            }
            rc::SeamFx::link_table => {
                let t = self.rooms.get(id).and_then(|r| r.join_table.clone());
                if let Some(s) = server {
                    match t {
                        Some(t) => self.link_table_carried(&s, t),
                        None => self.link_flush_pending(&s),
                    }
                }
            }
            rc::SeamFx::link_server_ice => {
                if let Some(s) = server {
                    self.link_server_ice(&s);
                }
            }
            rc::SeamFx::link_epoch_changed => {
                if let Some(s) = server {
                    self.link_epoch_changed(&s);
                }
            }
            rc::SeamFx::sdp_rebuild => {
                if let Some(s) = server {
                    self.link_renego(&s);
                }
            }
            rc::SeamFx::link_room_left => {
                if let Some(s) = server {
                    let last = self.rooms.values().filter(|r| r.sfu.as_deref() == Some(s.as_str())).all(|r| r.id == id || r.st.rs == rc::RoomSt::NONE);
                    self.link_room_left(&s, last);
                }
            }
            rc::SeamFx::room_sub_rooms => {
                let listed = self.rooms.get(id).and_then(|r| r.carried.as_ref()).and_then(|c| c.sub_rooms.clone());
                if let (Some(s), Some(listed)) = (server, listed) {
                    let others: Vec<String> = self.rooms.values().filter(|r| r.id != id && r.sfu.as_deref() == Some(s.as_str())).map(|r| r.id.clone()).collect();
                    for o in others {
                        let s = rc::sub_rooms(&self.rooms[&o].st, listed.contains(&o));
                        self.room_step(&o, "subRooms", Some(s), "ROOM_JOIN.sub_rooms");
                    }
                }
            }
            // 소속(발언 방 — 20261107c) — 입장이 곁들여 바꾼 발행 방 · 떠남 · 재구축 · 입장 실패
            rc::SeamFx::aff_implicit => {
                let (sel, p) = self.rooms.get_mut(id).map(|r| (r.join_sel, r.aff_pub.take().flatten())).unwrap_or((false, None));
                self.aff_implicit(id, sel, p);
            }
            rc::SeamFx::aff_leave => {
                let r = id.to_string();
                self.aff_enter("left", |s| Some(crate::generated::aff_client::left(s, &r)), label);
            }
            rc::SeamFx::aff_down => {
                let r = id.to_string();
                self.aff_enter("down", |s| Some(crate::generated::aff_client::down(s, &r)), label);
            }
            rc::SeamFx::aff_join_failed => {
                let r = id.to_string();
                self.aff_enter("joinFailed", |s| Some(crate::generated::aff_client::join_failed(s, &r)), label);
            }
            // 발행 — 방을 잃었다(서버가 먼저 정리) · 앱이 떠나기 시작했다
            rc::SeamFx::pub_room_gone => self.pub_room_gone(id, false),
            rc::SeamFx::pub_room_left => self.pub_room_gone(id, true),
        }
    }

    // ── 효과 표 ───────────────────────────────────────────────────────

    fn room_effect(&mut self, id: &str, f: rc::Fx) {
        let e = rb::effect(f).event;
        match e {
            Some(rb::RoomEvent::Joined) => {
                if let Some(r) = self.rooms.get_mut(id) {
                    r.rebuilding = false;
                    r.join_pending = true;
                }
                self.emit(Event::Joined { room: id.to_string() });
                if self.rooms.get_mut(id).is_some_and(|r| std::mem::take(&mut r.select_after)) {
                    let (tx, _rx) = oneshot::channel();
                    self.set_speaking_room(id.to_string(), tx);
                }
                if self.rooms.get(id).is_some_and(|r| !r.layers.is_empty()) {
                    self.sub_enter(id, "layer", |s| Some(uc::layer(s)), "joined");
                }

            }
            Some(rb::RoomEvent::JoinFailed) => {
                let Some(r) = self.rooms.get_mut(id) else { return };
                let err = r.last_fault.clone().unwrap_or_else(|| if f == rc::Fx::app_bug { Error::sdk("BUG", Category::Bug) } else { Error::sdk("JOIN_FAILED", Category::State) });
                for tx in r.join_waiters.drain(..) {
                    let _ = tx.send(Err(err.clone()));
                }
                if r.st.rs == rc::RoomSt::NONE && !r.st.rejoin {
                    r.closed = true;
                }
                self.emit(Event::JoinFailed { room: id.to_string(), error: err });
            }
            Some(rb::RoomEvent::Left) => {
                let Some(r) = self.rooms.get_mut(id) else { return };
                r.closed = true;
                self.emit(Event::Left { room: id.to_string() });
            }
            Some(rb::RoomEvent::Forced) => {
                let Some(r) = self.rooms.get_mut(id) else { return };
                r.closed = true;
                let cause = r.forced_cause.clone();
                self.emit(Event::Forced { room: id.to_string(), cause });
            }
            Some(rb::RoomEvent::Participants) => self.apply_members(id),
            None => {}
        }
    }

    /// 명단 통지를 보관본에 — 들고남 · 권한(투명 참가자는 오지 않는다)
    fn apply_members(&mut self, id: &str) {
        let me = self.user_id.clone();
        let mut mine: Option<Option<crate::generated::ws_wire::Permission>> = None;
        let Some(r) = self.rooms.get_mut(id) else { return };
        if let Some((op, b)) = r.notice.clone() {
            let uid = b.get("user_id").and_then(Value::as_str).unwrap_or("").to_string();
            match op {
                Op::ParticipantEvent => {
                    if b.get("type").and_then(Value::as_str) == Some("left") {
                        r.participants.retain(|p| p.user_id != uid);
                    } else if let Ok(m) = serde_json::from_value::<MemberInfo>(json!({
                        "user_id": uid, "role": b.get("role"), "select": b.get("select").and_then(Value::as_bool).unwrap_or(false),
                        "participant_type": b.get("participant_type").and_then(Value::as_u64).unwrap_or(0), "metadata": b.get("metadata"), "permission": b.get("permission"),
                    })) {
                        r.participants.retain(|p| p.user_id != uid);
                        r.participants.push(m);
                    }
                }
                Op::ParticipantState => {
                    let perm: Option<crate::generated::ws_wire::Permission> = b.get("permission").cloned().and_then(|p| serde_json::from_value(p).ok());
                    if Some(&uid) == me.as_ref()
                        && let Some(m) = r.me.as_mut() {
                            m.permission = perm.clone();
                            mine = Some(perm.clone());
                        }
                    for p in r.participants.iter_mut().filter(|p| p.user_id == uid) {
                        p.permission = perm.clone();
                    }
                }
                _ => {}
            }
        }
        let p = r.participants.clone();
        self.emit(Event::Participants { room: id.to_string(), participants: p });
        // 내 권한이 바뀌었다(SDK:IMPL-ROOM-API 권한 통지 — 투명 참가자도 본인에겐 온다)
        if let Some(permission) = mine {
            self.emit(Event::Permission { room: id.to_string(), permission });
        }
    }

    /// 첫 협상이 끝난 입장 — 기다리던 join() 을 끝낸다(SDK:IMPL-ROOM-API app.joined)
    pub(super) fn settle_joins(&mut self) {
        let ids: Vec<String> = self.rooms.iter().filter(|(_, r)| r.join_pending).map(|(k, _)| k.clone()).collect();
        for id in ids {
            let nego = self.rooms[&id].sfu.as_ref().and_then(|s| self.links.get(s)).is_some_and(|l| l.st.nego);
            if nego {
                continue;
            }
            if let Some(r) = self.rooms.get_mut(&id) {
                r.join_pending = false;
                for tx in r.join_waiters.drain(..) {
                    let _ = tx.send(Ok(()));
                }
            }
        }
    }

    // ── 받는 트랙(TRK) ───────────────────────────────────────────────

    fn trk_step(&mut self, id: &str, entry: &'static str, step: Option<tc::Step>, cause: &str, item: Option<(String, Option<TrackEntry>)>) {
        let Some(step) = step else {
            self.impossible("TRK", id, "-", entry);
            return;
        };
        let Some(r) = self.rooms.get_mut(id) else { return };
        let plan = tb::plan(step);
        r.trk = plan.state;
        self.record("TRK", id, "-", "-", plan.label, cause);
        // 이음매 pub.revoked — 내 발행이 트랙 목록에서 빠졌다(20261107c)
        if plan.seams.iter().any(|s| s.fx == tc::SeamFx::pub_revoked)
            && let Some((key, _)) = &item
        {
            self.pub_revoked(std::slice::from_ref(key));
        }
        for e in &plan.events {
            match e {
                tb::TrkEvent::Track => {
                    let Some(r) = self.rooms.get_mut(id) else { return };
                    if let Some((key, entry)) = &item {
                        let gone = r.trk.lay.get(key).is_none_or(|l| *l == tc::Layer::NONE);
                        if gone {
                            r.entries.remove(key);
                        } else if let Some(t) = entry {
                            r.entries.insert(key.clone(), t.clone());
                        }
                    }
                    self.tracks_changed(id);
                }
                tb::TrkEvent::Muted => {
                    let Some(r) = self.rooms.get_mut(id) else { return };
                    if let Some((key, _)) = &item {
                        let m = r.trk.muted.get(key).copied().unwrap_or(false);
                        if let Some(t) = r.entries.get_mut(key) {
                            t.muted = Some(m);
                        }
                        self.emit(Event::Muted { room: id.to_string(), track_id: key.clone(), muted: m });
                    }
                }
            }
        }
    }

    fn tracks_changed(&mut self, id: &str) {
        let me = self.user_id.clone();
        let Some(r) = self.rooms.get(id) else { return };
        let tracks: Vec<TrackEntry> = r.entries.values().filter(|t| !r.own(t, me.as_deref())).cloned().collect();
        self.emit(Event::Tracks { room: id.to_string(), tracks });
        self.rebuild_routes();
    }

    /// trk.delta — 방 상태기가 받아들인 트랙 통지만 닿는다
    fn trk_delta(&mut self, id: &str) {
        let me = self.user_id.clone();
        let Some((op, b)) = self.rooms.get(id).and_then(|r| r.notice.clone()) else { return };
        match op {
            Op::TrackEvent => {
                let action = if b.get("action").and_then(Value::as_str) == Some("remove") { tc::TrackAction::remove } else { tc::TrackAction::add };
                let list: Vec<TrackEntry> = b.get("tracks").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
                for t in list {
                    let mine = self.rooms[id].own(&t, me.as_deref());
                    let m = tc::TrackEventMsg { action, t: tc::TrackEventMsgT::TRACK_EVENT, tracks: tc::TrackEventMsgTracks { active: t.active.unwrap_or(true), muted: t.muted.unwrap_or(false), track_id: t.track_id.clone() } };
                    let s = tc::track_event(&self.rooms[id].trk, &m, mine);
                    self.trk_step(id, "trackEvent", s, "TRACK_EVENT", Some((t.track_id.clone(), Some(t))));
                }
            }
            Op::TrackState => {
                let Some(tid) = b.get("track_id").and_then(Value::as_str).map(str::to_string) else { return };
                let muted = b.get("muted").and_then(Value::as_bool).unwrap_or(false);
                let m = tc::TrackStateMsg { muted, t: tc::TrackEventMsgT::TRACK_STATE, track_id: tid.clone() };
                let s = tc::track_state(&self.rooms[id].trk, &m);
                self.trk_step(id, "trackState", s, "TRACK_STATE", Some((tid, None)));
            }
            _ => {}
        }
    }

    /// trk.snapshot — 받아들인 보관본의 그 방 트랙 전부
    fn trk_snapshot(&mut self, id: &str, cause: &str) {
        let me = self.user_id.clone();
        let Some(r) = self.rooms.get(id) else { return };
        let mut lay = tc::Rec::new();
        let mut muted = tc::Rec::new();
        for t in r.entries.values().filter(|t| !r.own(t, me.as_deref())) {
            lay.insert(t.track_id.clone(), if t.active == Some(false) { tc::Layer::RESTING } else { tc::Layer::ACTIVE });
            muted.insert(t.track_id.clone(), t.muted.unwrap_or(false));
        }
        let s = tc::snapshot(&r.trk, &lay, &muted);
        self.trk_step(id, "snapshot", Some(s), cause, None);
    }

    /// store.drop — 그 방을 떠났다 · 내렸다: 보관본을 버린다
    fn trk_store_drop(&mut self, id: &str, cause: &str) {
        let Some(r) = self.rooms.get_mut(id) else { return };
        r.entries.clear();
        let s = tc::store_drop(&r.trk);
        self.trk_step(id, "storeDrop", Some(s), cause, None);
        self.tracks_changed(id);
    }

    // ── 구독(SUB) ─────────────────────────────────────────────────────

    pub(super) fn sub_enter(&mut self, id: &str, entry: &'static str, f: impl Fn(&uc::St) -> Option<uc::Step>, cause: &str) {
        let Some(r) = self.rooms.get(id) else { return };
        let Some(step) = f(&r.sub) else {
            self.impossible("SUB", id, r.sub.ready.as_str(), entry);
            return;
        };
        let prev = r.sub.ready;
        let plan = ub::plan(step);
        let Some(r) = self.rooms.get_mut(id) else { return };
        r.sub = plan.state.clone();
        self.record("SUB", id, prev.as_str(), plan.state.ready.as_str(), plan.label, cause);
        for tx in &plan.send {
            match tx {
                uc::Tx::Ready => self.send_ready(id),
                uc::Tx::SubscribeLayer => self.send_layer(id),
            }
        }
        for sm in &plan.seams {
            match sm.fx {
                uc::SeamFx::sdp_rebuild => {
                    if let Some(s) = self.rooms.get(id).and_then(|r| r.sfu.clone()) {
                        self.link_renego(&s);
                    }
                }
            }
        }
        for e in &plan.events {
            match e {
                ub::SubEvent::Error => self.emit(Event::Error { room: Some(id.to_string()), error: Error::sdk("READY_FAILED", Category::Transport) }),
            }
        }
    }

    /// READY — 그 협상에 쓴 배정표의 차례를 싣는다(연:RULE-SUB-READY-PROC)
    pub(super) fn send_ready(&mut self, id: &str) {
        let Some(r) = self.rooms.get(id) else { return };
        let arev = r.sfu.as_ref().and_then(|s| self.links.get(s)).map(|l| l.sdp_arev).unwrap_or(0);
        let Some(join_id) = self.jids.str(r.st.jid).map(str::to_string) else { return };
        let req = ReadyReq { room_id: id.to_string(), join_id, arev };
        let epoch = r.req_epoch;
        self.request(Op::Ready, super::to_body(&req), Purpose::Ready { room: id.to_string(), epoch });
    }

    /// 앱의 층 고르기(SDK:IMPL-TRK-RECEIVE `RemoteTrack.setLayer`) — 값은 상한 · `scalability` 밖은 그 축의 끝으로 자른다(연:RULE-SUB-LAYER-PROC)
    pub(super) fn set_layer(&mut self, track: String, layer: Layer, tx: oneshot::Sender<Result<(), Error>>) {
        let id = self.rooms.values().find(|r| !r.closed && r.st.rs == rc::RoomSt::JOINED && r.entries.contains_key(&track)).map(|r| r.id.clone());
        let Some(id) = id else {
            let _ = tx.send(Err(Error::sdk("NO_TRACK", Category::State)));
            return;
        };
        let Some(r) = self.rooms.get_mut(&id) else { return };
        let (smax, tmax) = r.entries.get(&track).and_then(|t| t.scalability.as_deref()).and_then(scalability).unwrap_or((u8::MAX, u8::MAX));
        let t = r.layers.entry(track.clone()).or_insert_with(|| SubscribeLayerReqTargets { track_id: track.clone(), spatial: None, temporal: None, paused: None, priority: None });
        if let Some(v) = layer.spatial {
            t.spatial = Some(v.min(smax));
        }
        if let Some(v) = layer.temporal {
            t.temporal = Some(v.min(tmax));
        }
        if let Some(v) = layer.paused {
            t.paused = Some(v);
        }
        if let Some(v) = layer.priority {
            t.priority = Some(v);
        }
        r.layer_next = Some(tx);
        *self.layer_gen.entry(track).or_default() += 1;
        self.rebuild_routes();
        self.sub_enter(&id, "layer", |s| Some(uc::layer(s)), "setLayer");
    }

    /// SUBSCRIBE_LAYER — 이 방에서 고른 층 전부(생략한 칸은 안 바꾸는 부분 갱신이라 다시 보내도 같다)
    fn send_layer(&mut self, id: &str) {
        let Some(r) = self.rooms.get_mut(id) else { return };
        let tx = r.layer_next.take();
        if r.layers.is_empty() {
            if let Some(tx) = tx {
                let _ = tx.send(Ok(()));
            }
            return;
        }
        r.layer_seq += 1;
        let wid = r.layer_seq;
        if let Some(tx) = tx {
            r.layer_waiters.insert(wid, tx);
        }
        let targets: Vec<SubscribeLayerReqTargets> = r.layers.values().cloned().collect();
        let epoch = r.req_epoch;
        let Some(join_id) = self.jids.str(r.st.jid).map(str::to_string) else { return };
        let req = SubscribeLayerReq { room_id: id.to_string(), join_id, targets };
        self.request(Op::SubscribeLayer, super::to_body(&req), Purpose::Layer { room: id.to_string(), epoch, id: wid });
    }

    pub(super) fn layer_reply(&mut self, id: &str, epoch: u64, wid: u64, o: Outcome) {
        let Some(r) = self.rooms.get_mut(id) else { return };
        let tx = r.layer_waiters.remove(&wid);
        if r.req_epoch != epoch {
            if let Some(tx) = tx {
                let _ = tx.send(Err(Error::closed("ABANDONED")));
            }
            return;
        }
        let res = match o {
            Outcome::Ok(_) => Ok(()),
            Outcome::Fail(b) => Err(Error::from_failure(&b)),
            // 기한을 넘겼다 — 다시 보낸다(부분 갱신이라 멱등) · 기다리는 앱은 다음 응답이 답한다
            Outcome::Expired => {
                r.layer_next = tx;
                self.send_layer(id);
                return;
            }
            Outcome::Transport | Outcome::Closed => Err(Error::closed("CLOSED")),
        };
        let ok = res.is_ok();
        if let Some(tx) = tx {
            let _ = tx.send(res);
        }
        if ok {
            self.sub_enter(id, "layerRes", |s| Some(uc::layer_res(s)), "SUBSCRIBE_LAYER.res");
        }
    }

    pub(super) fn ready_reply(&mut self, id: &str, epoch: u64, o: Outcome) {
        let Some(r) = self.rooms.get(id) else { return };
        if r.req_epoch != epoch {
            return;
        }
        let ok = match o {
            Outcome::Ok(_) => true,
            Outcome::Fail(_) => false,
            // 기한을 넘겼다 — 다시 보낸다(멱등)
            Outcome::Expired => {
                self.send_ready(id);
                return;
            }
            Outcome::Transport | Outcome::Closed => return,
        };
        let m = uc::ReadyResMsg { part: if ok { uc::Part::res } else { uc::Part::fail } };
        self.sub_enter(id, "readyRes", |s| Some(uc::ready_res(s, &m)), if ok { "READY.res" } else { "READY.fail" });
    }

    // ── 발언권 듣기(FLR) ──────────────────────────────────────────────

    pub(super) fn floor_enter(&mut self, id: &str, entry: &'static str, f: impl Fn(&fc::Cl) -> Option<fc::Step>, cause: &str) -> Vec<fc::Fx> {
        let Some(r) = self.rooms.get(id) else { return vec![] };
        let Some(step) = f(&r.flr) else {
            self.impossible("FLR", id, r.flr.cs.as_str(), entry);
            return vec![];
        };
        let prev = r.flr.cs;
        let fx = step.fx.clone();
        let plan = pb::plan(step);
        let Some(r) = self.rooms.get_mut(id) else { return fx };
        r.flr = plan.state.clone();
        let shown = r.flr.shown.clone();
        let server = r.sfu.clone();
        self.record("FLR", id, prev.as_str(), plan.state.cs.as_str(), plan.label, cause);
        // 위 이음매 — mic.open · mic.close 는 그 방의 반이중 발행 게이트로(발행 floorOpen · floorClose).
        // ★DC 송신보다 먼저 — 놓기(RELEASE)는 송출을 멈춘 뒤 나간다(연:RULE-FLR-MSG-RELEASE · 202611/20261113h · 게이트 닫기는 동기라 곧바로 멈춘다)
        for f in &plan.routed {
            match f {
                fc::Fx::mic_open => self.floor_to_pub(id, true),
                fc::Fx::mic_close => self.floor_to_pub(id, false),
                _ => {}
            }
        }
        // 보낼 것 · ACK — 그 서버 DC 로(못 보내면 위 타이머가 다시 보낸다)
        let mut out: Vec<Vec<u8>> = plan.send.iter().filter_map(|t| dc::tx(t, id)).collect();
        out.extend(plan.ack.iter().filter_map(|a| dc::ack(a, id)));
        if let Some(l) = server.as_ref().and_then(|s| self.links.get(s)) {
            for b in out {
                l.send_dc(b);
            }
        }
        // 시계 — 새로 걸라고 한 것을 걸고 armed() 밖은 멈춘다
        let armed = fc::armed(&self.rooms[id].flr);
        for t in fc::TimerName::ALL {
            let tk = Tk::Floor(id.to_string(), *t);
            if !armed.contains(t) {
                self.timers.stop(&tk);
            } else if plan.timers.contains(t) || !self.timers.armed(&tk) {
                self.timers.start(tk, Duration::from_millis(fc::timer_ms(*t) as u64));
            }
        }
        // 누름 쪽 사건(20261107c) — 허가 · 거절 · 대기 · 회수 · 상태(시한 · 믿음) · 사유 · 허가 시간 · 순번(SDK:IMPL-FLR-EXPOSE)
        if let Some(r) = self.rooms.get_mut(id) {
            ptt_settle(r, &plan.events);
        }
        for e in &plan.events {
            let what = match e {
                pb::PttEvent::Granted => "granted",
                pb::PttEvent::Denied => "denied",
                pb::PttEvent::Queued => "queued",
                pb::PttEvent::Revoked => "revoked",
                pb::PttEvent::State => "state",
                _ => continue,
            };
            let Some(ptt) = self.rooms.get(id).map(ptt_info) else { continue };
            self.emit(Event::Ptt { room: id.to_string(), event: what.to_string(), ptt });
        }
        // 큐 승계 허가 — 누름은 늘 hold 다(SDK:IMPL-FLR-BUTTON `ptt.input` 기본 · 서버사이드에 toggle 표면이 없다) → 승계 허가가 온 순간 누른 채면 곧 의지 표시(연:RULE-FLR-PROC-QUEUED-ACCEPT)
        if entry != "intent" && self.rooms.get(id).is_some_and(hold_intent_due) {
            self.floor_enter(id, "intent", fc::intent, "hold");
        }
        // 효과 표 — 화자 · 믿음(듣기 몫) · 재생은 서버사이드 대상 아님(SDK:RULE-PLAT-SERVER)
        for f in &fx {
            match f {
                fc::Fx::app_speaker => self.emit(Event::Speaker { room: id.to_string(), user_id: shown.clone() }),
                fc::Fx::app_speaker_none => self.emit(Event::Speaker { room: id.to_string(), user_id: None }),
                fc::Fx::app_floor_unreliable => self.emit(Event::FloorTrusted { room: id.to_string(), trusted: false }),
                fc::Fx::app_floor_reliable => self.emit(Event::FloorTrusted { room: id.to_string(), trusted: true }),
                _ => {}
            }
        }
        fx
    }

    pub(super) fn floor_expire(&mut self, id: &str, t: fc::TimerName) {
        self.floor_enter(id, "expire", |s| fc::expire(s, t), "timer");
    }

    pub(super) fn floor_down(&mut self, id: &str) {
        self.floor_enter(id, "down", |s| Some(fc::down(s)), "rebuild");
    }

    /// 그 서버 DC 의 발언권 메시지 — 방 가르기 뒤 입구 receive
    pub(super) fn floor_receive(&mut self, server: &str, bytes: &[u8]) {
        let Some((room, rx)) = dc::parse(bytes) else { return };
        if self.rooms.get(&room).and_then(|r| r.sfu.as_deref()) != Some(server) {
            return;
        }
        // 화자의 출처 번호(연:RULE-FLR-SLOT-SOURCES) — 반이중 발행은 트랙 목록에 없어 슬롯 패킷의 CSRC 는 이것으로 주인을 안다
        if let fc::Rx::Taken(m) = &rx {
            let fresh: Vec<u32> = m.sources.iter().flatten().filter_map(|c| u32::try_from(*c).ok()).filter(|c| self.talk_sources.get(c) != Some(&m.speaker)).collect();
            if !fresh.is_empty() {
                for c in fresh {
                    self.talk_sources.insert(c, m.speaker.clone());
                }
                self.rebuild_routes();
            }
        }
        if let Some(r) = self.rooms.get_mut(&room) {
            ptt_rx(r, &rx);
        }
        let fx = self.floor_enter(&room, "receive", |s| Some(fc::receive(s, &rx)), "DC");
        // 발언권 사건(SDK:RULE-PLAT-REC-RAW `ctl/floor`) — 상태기 뒤에 적는다(받은 GRANTED 는 그 효과로 가린다)
        #[cfg(feature = "record")]
        if let Some(rec) = &self.sink.recorder
            && let Some(b) = floor_record(&room, self.user_id.as_deref(), &rx, &fx)
        {
            rec.push(crate::record::Channel::Floor, b.to_string().into_bytes());
        }
    }
}

/// `ctl/floor` 한 줄 — 받은 TAKEN · IDLE 과 출처 번호 · ★받은 GRANTED 는 상태기가 앞 화자를 지운 때(`app_speaker_none` — 요청 · 놓는 중 · 큐 승계 허가)만 `granted`(나) · 큐 승계 재전송 · 쥔 동안 온 허가는 상태기가 효과 없이 거른다 · 놓는 중 재전송은 다시 적히나 지울 발언이 없다
pub(crate) fn floor_record(room: &str, me: Option<&str>, rx: &fc::Rx, fx: &[fc::Fx]) -> Option<Value> {
    match rx {
        fc::Rx::Taken(m) => Some(json!({ "room": room, "event": "taken", "user_id": m.speaker, "sources": m.sources, "seq": m.seq, "talk": m.talk })),
        fc::Rx::Idle(m) => Some(json!({ "room": room, "event": "idle", "user_id": Value::Null, "prev": m.prev, "seq": m.seq })),
        fc::Rx::Granted(_) if fx.contains(&fc::Fx::app_speaker_none) => Some(json!({ "room": room, "event": "granted", "user_id": me, "seq": Value::Null })),
        _ => None,
    }
}

/// 승계 허가가 와 있고(`QUEUED` · `sub = GRANTED`) 앱이 아직 누른 채인가(`want`) — 그러면 의지 표시를 곧 넣는다(hold)
pub(crate) fn hold_intent_due(r: &Room) -> bool {
    r.flr.cs == fc::CState::QUEUED && r.flr.sub == fc::CSub::GRANTED && r.flr.want
}

/// 받은 발언권 메시지에서 표면이 쓸 것 — 거절 · 회수 사유는 효과가 날 때까지 들고 · 순번은 곧 적는다
pub(crate) fn ptt_rx(r: &mut Room, rx: &fc::Rx) {
    let cause = |c: i64, t: &Option<String>| crate::api::FloorCause { cause: u16::try_from(c).unwrap_or(u16::MAX), text: t.clone() };
    r.ptt_rx = match rx {
        fc::Rx::Revoke(m) => Some((true, cause(m.cause, &m.reason))),
        fc::Rx::Deny(m) => Some((false, cause(m.cause, &m.reason))),
        _ => None,
    };
    if let fc::Rx::Qinfo(m) = rx {
        r.queue_position = u8::try_from(m.info.position).ok();
    }
}

/// 위 걸음이 낸 효과로 사유를 옮긴다 — `denied` 면 그 거절 사유 · `revoked` 면 그 회수 사유(받지 않은 메시지의 사유는 버린다)
pub(crate) fn ptt_settle(r: &mut Room, events: &[pb::PttEvent]) {
    let rx = r.ptt_rx.take();
    for e in events {
        match (e, &rx) {
            (pb::PttEvent::Denied, Some((false, c))) => r.last_deny = Some(c.clone()),
            (pb::PttEvent::Revoked, Some((true, c))) => r.last_revoke = Some(c.clone()),
            _ => {}
        }
    }
    if r.flr.cs != fc::CState::QUEUED {
        r.queue_position = None;
    }
}

/// 발언권 표면 값(SDK:IMPL-FLR-EXPOSE 거울) — 사건 · 스냅숏이 같은 것을 낸다
pub(crate) fn ptt_info(r: &Room) -> crate::api::PttInfo {
    // 큐 승계 허가는 의지 표시(승계 시한)까지 `QUEUED` 에 남는다(연:RULE-FLR-PROC-GRANTED) — 그 창도 허가를 쥔 것으로 본다
    let held = r.flr.cs == fc::CState::PERM || (r.flr.cs == fc::CState::QUEUED && r.flr.sub == fc::CSub::GRANTED);
    let waiting = r.flr.cs == fc::CState::QUEUED || r.flr.cs == fc::CState::PREQ;
    crate::api::PttInfo {
        state: r.flr.cs.as_str().to_string(),
        trusted: r.flr.sub.as_str() != "WAIT",
        remaining_sec: (held && r.flr.grant > 0).then(|| u32::try_from(r.flr.grant).unwrap_or(u32::MAX)),
        priority: if held { Some(u8::try_from(r.flr.gprio).unwrap_or(u8::MAX)) } else if waiting { Some(u8::try_from(r.flr.prio).unwrap_or(u8::MAX)) } else { None },
        queue_position: if r.flr.cs == fc::CState::QUEUED { r.queue_position } else { None },
        last_deny: r.last_deny.clone(),
        last_revoke: r.last_revoke.clone(),
    }
}

/// `L{S}T{T}` → 축마다 가장 큰 인덱스(연:TRK.type.TrackEntry `scalability`)
fn scalability(v: &str) -> Option<(u8, u8)> {
    let rest = v.strip_prefix('L')?;
    let (s, t) = rest.split_once('T')?;
    let s: u8 = s.parse().ok()?;
    let t: u8 = t.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok()?;
    Some((s.checked_sub(1)?, t.checked_sub(1)?))
}

#[cfg(test)]
mod ptt_surface_test {
    use super::*;

    fn step(r: &mut Room, rx: fc::Rx) -> crate::api::PttInfo {
        ptt_rx(r, &rx);
        let plan = pb::plan(fc::receive(&r.flr, &rx));
        r.flr = plan.state.clone();
        ptt_settle(r, &plan.events);
        ptt_info(r)
    }

    fn held(prio: i64) -> Room {
        let mut r = Room::new("r1");
        r.flr = fc::Cl { cs: fc::CState::PERM, prio, gprio: prio, grant: 25, is_pub: true, ..fc::initial() };
        r
    }

    #[test]
    fn own_grant_is_written_where_the_machine_clears_the_speaker() {
        let g = || fc::Rx::Granted(fc::RxGranted { ack: true, priority: 0, duration: 30, talk: None });
        let rec = |cl: fc::Cl, rx: &fc::Rx| floor_record("r1", Some("me"), rx, &fc::receive(&cl, rx).fx);
        let want = Some(json!({ "room": "r1", "event": "granted", "user_id": "me", "seq": Value::Null }));
        let base = fc::Cl { want: true, is_pub: true, ..fc::initial() };
        assert_eq!(rec(fc::Cl { cs: fc::CState::PREQ, ..base.clone() }, &g()), want, "요청 뒤 허가");
        let prel = fc::Cl { cs: fc::CState::PREL, ..base.clone() };
        assert_eq!(rec(prel.clone(), &g()), want, "놓는 중 승계 허가");
        assert_eq!(rec(fc::receive(&prel, &g()).s, &g()), want, "놓는 중 허가의 재전송은 PREL 에 머물러 다시 적힌다 — 파이프라인에서 해가 없다(지울 발언이 없다)");
        let queued = fc::Cl { cs: fc::CState::QUEUED, sub: fc::CSub::WAIT, ..base.clone() };
        assert_eq!(rec(queued.clone(), &g()), want, "큐 승계 허가");
        let again = fc::receive(&queued, &g()).s;
        assert_eq!(rec(again, &g()), None, "같은 허가 재전송(A 비트)은 두 번째 줄을 내지 않는다");
        assert_eq!(rec(fc::Cl { cs: fc::CState::PERM, ..base.clone() }, &g()), None, "쥔 동안 온 허가");
        let taken = fc::Rx::Taken(fc::RxTaken { ack: false, speaker: "A".into(), sources: Some(vec![7]), seq: 3, talk: None });
        assert_eq!(rec(base.clone(), &taken).map(|v| v["event"].clone()), Some(json!("taken")));
    }

    #[test]
    fn granted_carries_remaining_and_priority() {
        let p = ptt_info(&held(3));
        assert_eq!((p.state.as_str(), p.remaining_sec, p.priority, p.queue_position), ("PERM", Some(25), Some(3), None));
    }

    #[test]
    fn regrant_updates_remaining_and_priority_to_the_received_values() {
        let mut r = Room::new("r1");
        r.flr = fc::Cl { cs: fc::CState::PREQ, want: true, is_pub: true, prio: 0, ..fc::initial() };
        let p = step(&mut r, fc::Rx::Granted(fc::RxGranted { ack: true, priority: 0, duration: 30, talk: None }));
        assert_eq!((p.state.as_str(), p.remaining_sec, p.priority), ("PERM", Some(30), Some(0)), "C-proc-RULE-FLR-PROC-GRANTED③ 처음 허가 — 받은 시간 · 우선순위");
        let p = step(&mut r, fc::Rx::Granted(fc::RxGranted { ack: true, priority: 7, duration: 60, talk: None }));
        assert_eq!((p.state.as_str(), p.remaining_sec, p.priority), ("PERM", Some(60), Some(7)), "C-proc-RULE-FLR-PROC-PERM-GRANTED② 재허가의 시간 · 우선순위로 갱신(요청에 실은 값이 아니다)");
    }

    #[test]
    fn revoke_causes_are_kept_apart() {
        for (cause, text) in [(i64::from(crate::api::floor::revoke::PREEMPTED), "preempted"), (i64::from(crate::api::floor::revoke::TOO_LONG), "too long"), (i64::from(crate::api::floor::revoke::NO_PERMISSION), "no permission")] {
            let mut r = held(0);
            let p = step(&mut r, fc::Rx::Revoke(fc::RxRevoke { ack: false, cause, reason: Some(text.into()) }));
            assert_eq!(p.last_revoke, Some(crate::api::FloorCause { cause: cause as u16, text: Some(text.into()) }), "회수 사유 {cause}");
            assert_eq!(p.state, "PREL");
            assert_eq!(p.remaining_sec, None, "허가를 놓으면 허가 시간은 없다");
        }
    }

    #[test]
    fn deny_cause_and_queue_position() {
        let mut r = Room::new("r1");
        r.flr = fc::Cl { cs: fc::CState::PREQ, want: true, is_pub: true, ..fc::initial() };
        let p = step(&mut r, fc::Rx::Deny(fc::RxDeny { ack: false, cause: i64::from(crate::api::floor::deny::RETRY_AFTER), reason: None }));
        assert_eq!(p.last_deny.map(|c| c.cause), Some(u16::from(crate::api::floor::deny::RETRY_AFTER)));
        assert_eq!(p.last_revoke, None, "거절은 회수 사유를 건드리지 않는다");
        let mut r = Room::new("r1");
        r.flr = fc::Cl { cs: fc::CState::PREQ, want: true, is_pub: true, prio: 1, ..fc::initial() };
        let p = step(&mut r, fc::Rx::Qinfo(fc::RxQinfo { ack: false, info: fc::RxQinfoInfo { position: 2, priority: 1 } }));
        assert_eq!((p.state.as_str(), p.queue_position, p.priority), ("QUEUED", Some(2), Some(1)));
        let p = step(&mut r, fc::Rx::Granted(fc::RxGranted { ack: false, priority: 1, duration: 20, talk: None }));
        assert_eq!((p.state.as_str(), p.remaining_sec), ("QUEUED", Some(20)), "큐 승계 허가 — 의지 표시 전에도 허가 시간이 보인다");
        assert!(hold_intent_due(&r), "누른 채 승계 허가 — hold 면 의지 표시가 곧 들어가야 한다");
        let plan = pb::plan(fc::intent(&r.flr).expect("승계 허가 창의 의지 표시"));
        assert_eq!(plan.state.cs, fc::CState::PERM);
        assert!(plan.routed.contains(&fc::Fx::mic_open), "의지 표시가 마이크 게이트를 연다");
        let mut gone = Room::new("r1");
        gone.flr = fc::Cl { want: false, ..r.flr.clone() };
        assert!(!hold_intent_due(&gone), "놓은 뒤 온 승계 허가는 의지 표시하지 않는다(승계 시한이 처리)");
    }
}

#[cfg(test)]
mod own_track_test {
    use super::*;

    fn entry(v: Value) -> TrackEntry {
        serde_json::from_value(v).expect("트랙 항목")
    }

    /// 스냅샷 · 델타가 같은 보관본을 짓는 거르기(연:TRK.C.any.snapshot · 연:TRK.C.any.mine_other · context 202611/20261113j ③-2) — 내 개인 트랙만 뺀다
    #[test]
    fn own_is_only_my_individual_track() {
        let r = Room::new("r1");
        let mine = entry(json!({ "type": "individual", "room_id": "r1", "track_id": "t1", "kind": "video", "user_id": "me" }));
        let other = entry(json!({ "type": "individual", "room_id": "r1", "track_id": "t2", "kind": "video", "user_id": "you" }));
        let slot = entry(json!({ "type": "slot", "room_id": "r1", "track_id": "ptt-r1-video", "kind": "video" }));
        assert!(r.own(&mine, Some("me")), "내 개인 트랙은 받을 트랙이 아니다");
        assert!(!r.own(&other, Some("me")), "남의 트랙은 받는다");
        assert!(!r.own(&slot, Some("me")), "방 슬롯은 내가 말해도 받을 트랙이다");
        assert!(!r.own(&mine, None), "내 신원을 모르면 거르지 않는다");
    }
}

#[cfg(test)]
mod rejoin_select_test {
    use super::*;

    /// 재구축의 다시 들어감은 select:false — 처음 입장의 select 를 되실으면 다른 서버의 발행 방과 둘이 된다(소속 판 rebuild_sel 반례 AH2 · context 202611/20261113j ③-4)
    #[test]
    fn rebuild_rejoin_never_selects() {
        let mut r = Room::new("r1");
        assert!(!r.join_select(), "듣기 입장은 select:false");
        r.select = true;
        assert!(r.join_select(), "발행 방 없이 말하려는 앱 입장은 select:true(연:RULE-ROOM-JOIN-START 2)");
        r.rebuilding = true;
        assert!(!r.join_select(), "재구축의 다시 들어감은 처음 입장이 select:true 였어도 select:false");
        r.rebuilding = false;
        assert!(r.join_select(), "재구축이 끝난 뒤 그 방의 앱 입장 값은 그대로");
    }
}

#[cfg(test)]
mod pc_mode_guard_test {
    use super::pc_mode_fault;
    use crate::core::pcmode::Mode;
    use crate::error::Category;
    use crate::generated::ws_wire::PcMode;

    #[test]
    fn join_response_mode_must_equal_the_session_value() {
        assert!(pc_mode_fault(PcMode::V1pc, Mode::One).is_none(), "1pc 세션의 1pc 응답은 정상");
        assert!(pc_mode_fault(PcMode::V2pc, Mode::Two).is_none(), "2pc 세션의 2pc 응답은 정상");
        let f = pc_mode_fault(PcMode::V2pc, Mode::One).expect("1pc 세션의 2pc 응답은 불일치");
        assert_eq!((f.name.as_str(), f.category), ("PC_MODE", Category::Negotiation), "ROOM.C.JOINING.join_pc_mismatch 는 negotiation");
        assert!(pc_mode_fault(PcMode::V1pc, Mode::Two).is_some(), "2pc 세션의 1pc 응답도 불일치");
    }
}
