// author: kodeholic (powered by Claude)
//! 세션 — 입구 표 · 효과 표 · 이음매 · 시계(SDK:IMPL-SES-API · SDK:IMPL-SES-EVENTS · SDK:IMPL-SES-CLOCK).
//! 절차는 위 세션 클라 상태기(generated/session_client) · 이벤트는 결합(generated/session_binding)이 정한다.

use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::oneshot;

use super::{Core, Tk};
use crate::api::{Event, Health, LinkState, SessionInfo, SessionState};
use crate::error::{Category, Error};
use crate::generated::ws_wire::{BindReq, Op, PcMode, ResumeReq};
use crate::generated::{room_client as rc, session_binding as sb, session_client as sc};
use crate::sig::{Outcome, Purpose};

impl Core {
    // ── 앱 조작 셋 ──────────────────────────────────────────────────────

    pub(super) fn connect(&mut self, tx: oneshot::Sender<Result<(), Error>>) {
        if self.ses.cs == sc::Conn::ACTIVE {
            let _ = tx.send(Ok(()));
            return;
        }
        self.app_closed = false;
        self.connect_waiters.push(tx);
        if self.ses.cs == sc::Conn::DISCONNECTED {
            self.failure = None;
            self.resume_on_token = false;
            // BIND 에 싣는 연결 모드 — 서버사이드는 능력표에 행이 없어 2pc(SDK:RULE-PLAT-SERVER pc_mode) · 앱이 준 토큰의 유무
            let token = if self.token.is_empty() || self.ses.token == sc::Token::NONE { sc::Token::NONE } else { self.ses.token };
            self.ses = sc::St { pc_ask: sc::PcMode::V2pc, token, ..self.ses.clone() };
            let s = sc::connect(&self.ses);
            self.ses_step("connect", s, "app");
        }
    }

    pub(super) fn set_token(&mut self, token: String) {
        self.token = token;
        let s = sc::new_token(&self.ses);
        if s.is_none() {
            return;
        }
        self.ses_step("newToken", s, "app");
        if self.resume_on_token && self.ses.cs == sc::Conn::DISCONNECTED && !self.app_closed {
            self.resume_on_token = false;
            let s = sc::connect(&self.ses);
            self.ses_step("connect", s, "token");
        }
    }

    /// 끊기 — 위 입구 disconnect 하나 · 응답을 기다리지 않는다 · 클라 주도 종료라 사유는 없다
    pub(super) fn close_for_app(&mut self) {
        self.app_closed = true;
        self.resume_on_token = false;
        self.reason = None;
        self.failure = None;
        let s = sc::disconnect(&self.ses);
        if s.is_some() {
            self.ses_step("disconnect", s, "app");
        }
        for (_, tx) in self.parked.drain(..) {
            let _ = tx.send(Err(Error::closed("CLOSED")));
        }
        for tx in self.connect_waiters.drain(..) {
            let _ = tx.send(Err(Error::closed("CLOSED")));
        }
        let gone: Vec<(String, String)> = self.links.iter().map(|(k, l)| (k.clone(), l.st.ls.as_str().to_string())).collect();
        for (server, ls) in gone {
            self.record("LINK", &server, &ls, "-", "SDK.link_drop", "close");
        }
        self.links.clear();
        self.timers = super::Timers::default();
        // 프로세스 계수(운영 로그 `stat`)는 Client 가 사는 동안 계속 낸다 — 닫은 뒤에도 「아직 도는 것」을 줄로 가르게(context 202611/20261109e)
        self.timers.start(super::Tk::LogStat, self.log.config().stat_interval);
        // 같은 객체로 다시 connect 하면 새 세션이다(SDK:ses.md `close()`) — 방 · 발행 · 소속 · 출처 대응은 이 세션의 것이라 비운다(방은 앱이 다시 join)
        self.rooms.clear();
        self.pubs.clear();
        self.pub_mids.clear();
        self.talk_sources.clear();
        self.aff = crate::generated::aff_client::initial();
        for tx in self.aff_waiters.drain(..) {
            let _ = tx.send(Err(Error::closed("CLOSED")));
        }
        self.join_mode = None;
        self.sync_outs();
        self.rebuild_routes();
    }

    // ── 한 걸음 ────────────────────────────────────────────────────────

    /// 입구 하나의 결과 — 위가 안 받으면(None) 받지 않는 사건 · plan() 목록대로 직접 한다
    pub(super) fn ses_step(&mut self, entry: &'static str, step: Option<sc::Step>, cause: &str) -> bool {
        let Some(step) = step else {
            self.impossible("SES", "session", self.ses.cs.as_str(), entry);
            return false;
        };
        let prev = self.ses.clone();
        let fx = step.fx.clone();
        let plan = sb::plan(step);
        self.ses = plan.state.clone();
        self.record("SES", "session", prev.cs.as_str(), self.ses.cs.as_str(), plan.label, cause);
        for tx in &plan.send {
            self.ses_send(tx);
        }
        self.sock_follow(&prev);
        let mut ended = std::mem::take(&mut self.ended);
        if self.ses.cs == sc::Conn::ACTIVE && prev.cs != sc::Conn::ACTIVE {
            self.sig.session_up(fx.contains(&sc::Fx::app_resumed), &mut ended);
        }
        if self.ses.cs == sc::Conn::DISCONNECTED && prev.cs != sc::Conn::DISCONNECTED {
            self.sig.session_end(&mut ended);
        }
        self.ended = ended;
        self.watch_requests();
        self.ses_clock(entry, &prev, &plan.timers);
        for sm in &plan.seams {
            self.ses_seam(sm.fx);
        }
        for f in &fx {
            self.ses_effect(*f, entry);
        }
        if self.ses.cs == sc::Conn::ACTIVE && prev.cs != sc::Conn::ACTIVE {
            for (room, tx) in std::mem::take(&mut self.parked) {
                self.join(room, tx);
            }
            let servers: Vec<String> = self.links.keys().cloned().collect();
            for s in servers {
                self.link_session_back(&s);
            }
        }
        if self.ses.cs == sc::Conn::DISCONNECTED && prev.cs != sc::Conn::DISCONNECTED {
            let fault = self.failure.clone().unwrap_or_else(|| Error::closed(if entry == "expire" { "BACKOFF_EXHAUSTED" } else { "CLOSED" }));
            for tx in self.connect_waiters.drain(..) {
                let _ = tx.send(Err(fault.clone()));
            }
            for (_, tx) in self.parked.drain(..) {
                let _ = tx.send(Err(Error::closed("CLOSED")));
            }
        }
        self.surface(false);
        true
    }

    fn sock_follow(&mut self, prev: &sc::St) {
        let mut ended = std::mem::take(&mut self.ended);
        if self.ses.sock == sc::Sock::OPENING && prev.sock != sc::Sock::OPENING {
            let tx = self.sock_tx.clone();
            self.sig.connect(&tx, &mut ended);
        }
        // 닫힘 = 열린 소켓도 여는 중인 소켓도 닫는다 · 내가 닫은 소켓의 닫힘 사건은 사건이 아니다(Sig 가 뗀 소켓)
        if self.ses.sock == sc::Sock::CLOSED && prev.sock != sc::Sock::CLOSED {
            self.sig.close(&mut ended);
        }
        self.ended = ended;
    }

    fn ses_send(&mut self, tx: &sc::Tx) {
        match tx {
            sc::Tx::Bind { pc_mode, session_id, .. } => {
                // 출처 표지는 늘 켠다 — 패킷마다 발행 신원(SDK:RULE-PLAT-SERVER 「출처 표지」 · 연:RULE-SES-SOURCE-TAG-SESSION)
                let req = BindReq {
                    token: Some(self.token.clone()),
                    session_id: session_id.and_then(|id| self.sids.str(id).map(str::to_string)),
                    client_ver: None,
                    pc_mode: Some(if *pc_mode == sc::PcMode::V1pc { PcMode::V1pc } else { PcMode::V2pc }),
                    source_tag: Some(true),
                };
                self.request(Op::Bind, super::to_body(&req), Purpose::Bind);
            }
            sc::Tx::Resume => self.request(Op::Resume, super::to_body(&ResumeReq {}), Purpose::Resume),
            sc::Tx::Heartbeat => self.request(Op::Heartbeat, Value::Null, Purpose::Heartbeat),
            sc::Tx::Leave => {
                let mut ended = std::mem::take(&mut self.ended);
                self.sig.send_leave(&mut ended);
                self.ended = ended;
            }
        }
    }

    // ── 소켓 · 응답 → 입구 ─────────────────────────────────────────────

    pub(super) fn sock_open(&mut self, sgen: u64) {
        if !self.sig.opened(sgen) {
            return;
        }
        let s = sc::sock_open(&self.ses);
        self.ses_step("sockOpen", s, "ws.open");
    }

    pub(super) fn sock_closed(&mut self, sgen: u64) {
        let mut ended = std::mem::take(&mut self.ended);
        let r = self.sig.closed(sgen, &mut ended);
        self.ended = ended;
        let Some(leave) = r else { return };
        self.dropped(leave);
    }

    /// 루프가 스스로 소켓을 닫았다(T-req 초과 · WS 점검 실패) — 사유 없는 끊김으로 넣는다
    pub(super) fn sock_closed_now(&mut self, _gen: u64) {
        self.dropped(None);
    }

    fn dropped(&mut self, leave: Option<Value>) {
        let code = leave.as_ref().and_then(|b| b.get("code")).and_then(Value::as_u64).unwrap_or(0) as u16;
        let name = leave.as_ref().and_then(|b| b.get("name")).and_then(Value::as_str).unwrap_or("TRANSPORT").to_string();
        self.reason = Some((code, name.clone()));
        self.failure = Some(if leave.is_some() { Error { code, name, category: Category::Closed, permanent: false, message: None } } else { Error::closed("TRANSPORT") });
        // 읽는 값 carry = 들어가 있는 방이나 발행이 있다(발행은 첫 단계에 없다)
        let carry = self.rooms.values().any(|r| r.st.rs != rc::RoomSt::NONE);
        let s = sc::drop(&self.ses, i64::from(code), carry);
        self.ses_step("drop", Some(s), "ws.close");
    }

    pub(super) fn bind_reply(&mut self, o: Outcome) {
        let (ok, body) = match o {
            Outcome::Ok(b) => (true, b),
            Outcome::Fail(b) => (false, b),
            _ => return,
        };
        let code = if ok { 0 } else { body.get("code").and_then(Value::as_i64).unwrap_or(5003) };
        let sid = if ok { body.get("session_id").and_then(Value::as_str).unwrap_or("").to_string() } else { String::new() };
        let m = sc::BindResMsg {
            code,
            part: if ok { sc::Part::res } else { sc::Part::fail },
            heartbeat_interval: body.get("heartbeat_interval").and_then(Value::as_i64).unwrap_or(0),
            pc_mode: if body.get("pc_mode").and_then(Value::as_str) == Some("1pc") { sc::PcMode::V1pc } else { sc::PcMode::V2pc },
            resume_window_ms: body.get("resume_window_ms").and_then(Value::as_i64).unwrap_or(0),
            session_id: if ok { self.sids.id(&sid) } else { 0 },
        };
        if ok {
            self.user_id = body.get("user_id").and_then(Value::as_str).map(str::to_string);
            self.session_id = Some(sid);
            self.log.bind(self.session_id.as_deref(), self.user_id.as_deref());
        } else {
            self.failure = Some(Error::from_failure(&body));
            self.reason = Some((0, "BIND_FAILED".into()));
        }
        let marked = self.rooms.values().any(|r| r.st.rejoin);
        let s = sc::bind_res(&self.ses, &m, marked);
        self.ses_step("bindRes", s, if ok { "BIND.res" } else { "BIND.fail" });
    }

    pub(super) fn resume_reply(&mut self, o: Outcome) {
        let ok = match o {
            Outcome::Ok(b) => {
                self.resume_body = Some(b);
                true
            }
            Outcome::Fail(_) => false,
            _ => return,
        };
        let dead = self.links.values().any(|l| !l.st.assoc || l.st.ice != crate::generated::link_client::Ice::OK);
        let s = sc::resume_res(&self.ses, &sc::ResumeResMsg { part: if ok { sc::Part::res } else { sc::Part::fail } }, dead);
        self.ses_step("resumeRes", s, if ok { "RESUME.res" } else { "RESUME.fail" });
        self.resume_body = None;
    }

    // ── 시계 ──────────────────────────────────────────────────────────

    /// 한 걸음 뒤 — 새로 걸라고 한 것을 걸고 armed() 밖은 멈춘다 · 끊긴 시각을 적고 재개 창을 잰다
    fn ses_clock(&mut self, entry: &str, prev: &sc::St, start: &[sc::TimerName]) {
        let armed = sc::armed(&self.ses);
        for t in sc::TimerName::ALL {
            let tk = Tk::Ses(*t);
            if !armed.contains(t) {
                self.timers.stop(&tk);
                continue;
            }
            if start.contains(t) || !self.timers.armed(&tk) {
                let ms = self.ses_ms(*t);
                self.timers.start(tk, Duration::from_millis(ms));
            }
        }
        // 끊긴 시각 = 서버에 살아 있던(하트비트가 돌던) 세션의 소켓이 끊긴 때
        if entry == "drop" && prev.hb {
            self.dropped_at = Some(Instant::now());
        }
        self.ses_window_arm();
    }

    /// 타이머 값 — 위 원장 · 위 상태가 보관한 하트비트 주기 · 사다리 칸값(+ 지터)
    fn ses_ms(&self, t: sc::TimerName) -> u64 {
        let s = &self.ses;
        match t {
            sc::TimerName::THb if s.hb_ms > 0 => s.hb_ms as u64,
            sc::TimerName::Backoff => {
                let w = sc::LADDER.waits;
                let i = (s.tries.max(0) as usize).min(w.len() - 1);
                let jitter = if s.tries >= sc::LADDER.jitter_from - 1 { rand_below(sc::LADDER.jitter_ms as u64) } else { 0 };
                w[i] as u64 + jitter
            }
            _ => sc::timer_ms(t).unwrap_or(10_000) as u64,
        }
    }

    pub(super) fn ses_expired(&mut self, t: sc::TimerName) {
        if t == sc::TimerName::TBind {
            self.reason = Some((0, "BIND_TIMEOUT".into()));
            self.failure = Some(Error::closed("BIND_TIMEOUT"));
        }
        let s = sc::expire(&self.ses, t);
        self.ses_step("expire", s, "timer");
    }

    fn ses_window_arm(&mut self) {
        if self.ses.cs != sc::Conn::RESUMING || self.ses.late {
            self.timers.stop(&Tk::Window);
            return;
        }
        if self.timers.armed(&Tk::Window) {
            return;
        }
        let win = Duration::from_millis(if self.ses.win_ms > 0 { self.ses.win_ms as u64 } else { 60_000 });
        let since = self.dropped_at.map(|t| t.elapsed()).unwrap_or_default();
        self.timers.start(Tk::Window, win.saturating_sub(since));
    }

    pub(super) fn ses_window(&mut self) {
        let s = sc::late(&self.ses);
        self.ses_step("late", s, "clock");
    }

    // ── 이음매 — 받는 곳은 위가 정했다 ───────────────────────────────

    fn ses_seam(&mut self, fx: sc::SeamFx) {
        let rooms: Vec<String> = self.rooms.keys().cloned().collect();
        let servers: Vec<String> = self.links.keys().cloned().collect();
        match fx {
            sc::SeamFx::floor_down_all => rooms.iter().for_each(|r| self.floor_down(r)),
            sc::SeamFx::rooms_down_all => rooms.iter().for_each(|r| self.room_down_all(r)),
            sc::SeamFx::rooms_close_all => rooms.iter().for_each(|r| self.room_close_all(r)),
            sc::SeamFx::media_close_all => servers.iter().for_each(|s| self.link_close_all(s)),
            sc::SeamFx::media_repair => servers.iter().for_each(|s| self.link_repair(s)),
            sc::SeamFx::rooms_rejoin => self.rejoin_all(rooms),
            sc::SeamFx::rooms_snapshot => self.resume_snapshots(),
            // 발행(20261107c) — 새 세션이면 다시 발행 · 이어받기면 응답의 발행 목록과 맞댄다
            sc::SeamFx::pub_reregister_all => self.pub_reregister_all(),
            sc::SeamFx::pub_resume_check => {
                let body = self.resume_body.clone().unwrap_or(Value::Null);
                self.pub_resume_check(&body);
            }
        }
    }

    /// 이어받기 응답의 방마다 몫 · 배정표
    fn resume_snapshots(&mut self) {
        let body = self.resume_body.clone().unwrap_or(Value::Null);
        if let Some(ts) = body.get("mline_tables").and_then(Value::as_array) {
            for t in ts {
                let server = t.get("sfu_id").and_then(Value::as_str).unwrap_or("").to_string();
                if let Some(l) = self.links.get_mut(&server) {
                    l.pending_table = Some(t.clone());
                }
            }
        }
        let rooms: Vec<String> = self.rooms.keys().cloned().collect();
        for r in rooms {
            let snap = body.get("snapshot").and_then(|s| s.get(&r)).cloned();
            self.room_resume_snap(&r, snap);
        }
    }

    // ── 효과 표 ───────────────────────────────────────────────────────

    fn ses_effect(&mut self, f: sc::Fx, entry: &str) {
        let event = sb::effect(f).event;
        match f {
            sc::Fx::app_connected => {
                self.reason = None;
                self.failure = None;
                for tx in self.connect_waiters.drain(..) {
                    let _ = tx.send(Ok(()));
                }
            }
            // 최초 접속 — 이벤트 대신 기다리던 connect() 를 reject(auth)
            sc::Fx::app_need_token if !self.connect_waiters.is_empty() => {
                let fault = self.failure.clone().unwrap_or(Error { code: 2003, name: "TOKEN_EXPIRED".into(), category: Category::Auth, permanent: false, message: None });
                for tx in self.connect_waiters.drain(..) {
                    let _ = tx.send(Err(fault.clone()));
                }
                return;
            }
            sc::Fx::app_need_token => self.resume_on_token = true,
            // 최초 접속의 BIND 거절(위조 · 손상 토큰) — 이벤트 대신 reject
            sc::Fx::app_disconnected if entry == "bindRes" && !self.connect_waiters.is_empty() => {
                let fault = self.failure.clone().unwrap_or(Error { code: 2002, name: "TOKEN_INVALID".into(), category: Category::Auth, permanent: true, message: None });
                for tx in self.connect_waiters.drain(..) {
                    let _ = tx.send(Err(fault.clone()));
                }
                return;
            }
            sc::Fx::app_bug => {
                let fault = self.failure.clone().unwrap_or(Error { code: 1002, name: "INVALID_PAYLOAD".into(), category: Category::Bug, permanent: true, message: None });
                for tx in self.connect_waiters.drain(..) {
                    let _ = tx.send(Err(fault.clone()));
                }
            }
            _ => {}
        }
        if let Some(e) = event {
            self.ses_emit(e, entry);
        }
        // 덧붙임 — 앱이 끊은 것이 아니면 closed 도 낸다
        if f == sc::Fx::app_disconnected && !self.app_closed {
            self.ses_emit(sb::SessionEvent::Closed, entry);
        }
    }

    fn ses_emit(&mut self, e: sb::SessionEvent, entry: &str) {
        match e {
            sb::SessionEvent::Session => self.surface(true),
            sb::SessionEvent::TokenRequired => self.emit(Event::TokenRequired),
            sb::SessionEvent::Rebuilt => self.emit(Event::Rebuilt),
            sb::SessionEvent::Closed => {
                let (code, name) = if entry == "bindRes" {
                    self.failure.as_ref().map(|f| (f.code, f.name.clone())).unwrap_or((1002, "INVALID_PAYLOAD".into()))
                } else {
                    self.reason.clone().unwrap_or((0, "TRANSPORT".into()))
                };
                self.emit(Event::Closed { code, name, retryable: entry == "expire" });
            }
        }
    }

    // ── 표면 — 파생값은 여기 한 곳에서만 ────────────────────────────

    pub(crate) fn session_info(&self) -> SessionInfo {
        let state = match self.ses.cs {
            sc::Conn::DISCONNECTED => SessionState::Disconnected,
            sc::Conn::CONNECTING => SessionState::Connecting,
            sc::Conn::ACTIVE => SessionState::Active,
            sc::Conn::RESUMING => SessionState::Resuming,
        };
        let recovering = self.ses.cs == sc::Conn::RESUMING || self.rooms.values().any(|r| r.rebuilding) || self.links.values().any(|l| l.st.ever_up && l.state() != LinkState::Up);
        let health = if self.ses.cs != sc::Conn::ACTIVE || self.links.values().any(|l| l.blocked) {
            Health::Offline
        } else if self.rooms.values().any(|r| r.rebuilding) || self.links.values().any(|l| l.state() == LinkState::Rebuilding) {
            Health::Recovering
        } else if self.links.values().any(|l| matches!(l.state(), LinkState::Wobbly | LinkState::Restarting)) {
            Health::Degraded
        } else {
            Health::Ok
        };
        let pc_mode = (self.ses.cs == sc::Conn::ACTIVE || self.session_id.is_some()).then(|| self.ses.pcm.as_str().to_string());
        SessionInfo { state, recovering, reason: self.reason.clone(), user_id: self.user_id.clone(), pc_mode, health }
    }

    /// 상태 · recovering · reason · health 가 바뀔 때마다 Session 사건
    pub(crate) fn surface(&mut self, force: bool) {
        let i = self.session_info();
        if !force && self.last_info.as_ref() == Some(&i) {
            return;
        }
        self.last_info = Some(i.clone());
        self.emit(Event::Session(i));
    }
}

/// [0, n) 균등 — 지터
pub(crate) fn rand_below(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    u64::from_le_bytes(b) % n
}
