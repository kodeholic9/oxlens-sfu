// author: kodeholic (powered by Claude)
// spec: v1.7 · 정:IMPL-FLR-LOCK · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FLR-FIRST-NOTICE · 정:IMPL-FLR-MEMBER-JOIN · 정:RULE-FLR-SNAPSHOT · 연:RULE-FLR-TIMERS · 연:TRN.frame.DC · 정:IMPL-FLR-POLICY · 운영:RULE-LOG-TRANSITION · 정:IMPL-FWD-ROOM · 정:IMPL-ROOM-LIFE · 정:RULE-CONC-SIX · 정:IMPL-FLR-TALK-REPORT · 연:RULE-FLR-TLV · 정:RULE-HOOK-FACTS · 정:RULE-BUS-PUBLIC · model: claude-opus-5-5
//! 발언권 집행 — 판정은 방 락 안(보낼 것 · 걸 타이머 · 효과에 `grant_no`) · 집행은 락 밖(MBCP 를 DC 로 · 타이머는 가장 이른 만료 시각에 깨어난다 · 슬롯 결합 · PLI burst — 서버가 지은 프레임은 슬롯에 내지 않는다) · 발언 알림(허가에 발언 id · 끝에 상태 — 방마다 한 줄). 시계 · 집행 큐 · 집행자는 그 방의 것(`Room.exec`) — 지음 → 돎 을 이긴 쪽만 집행자를 띄우고, 돎일 때만 판정한다.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use oxsig::floor as mbcp;
use oxsig::msg::Kind;

use crate::flr::bind::Effect;
use crate::flr::core::{Floor, Fx, G, Msg, Mt, Step};
use crate::flr::talk::{self, End, Talk};
use common::envelope::{FloorIdle, FloorRevoke, FloorTaken, Material, RoomFloor as FloorView};
use common::talk::Report;
use crate::fwd::rt::mono_us;
use crate::fwd::SlotFan;
use crate::link::Transport;
use crate::room::Room;

pub const T1_MS: u64 = mbcp::timers::T1_MS;
pub const T3_MS: u64 = mbcp::timers::T3_MS;
pub const T7_MS: u64 = mbcp::timers::T7_MS;
pub const T8_MS: u64 = mbcp::timers::T8_MS;
pub const T9_MS: u64 = mbcp::timers::T9_MS;
pub const T20_MS: u64 = mbcp::timers::T20_MS;
pub const T21_MS: u64 = mbcp::timers::T21_MS;

type Key = (&'static str, Option<String>);

#[derive(Default)]
pub struct Clock {
    pub timers: BTreeMap<Key, u64>,
    pub events: u64,
    pub grant_no: u64,
    pub granted_at: u64,
    pub first_rtp_at: Option<u64>,
    pub last_rtp_at: Option<u64>,
    pub talk: Option<Talk>,
}

const BUILT: u8 = 0;
const RUNNING: u8 = 1;
const STOPPED: u8 = 2;

#[derive(Default)]
struct Drained {
    done: std::sync::atomic::AtomicBool,
    wake: tokio::sync::Notify,
}

pub struct RoomFloor {
    pub clock: Mutex<Clock>,
    pub wake: tokio::sync::Notify,
    end: Arc<Drained>,
    state: AtomicU8,
    jobs: tokio::sync::mpsc::UnboundedSender<Job>,
    rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Job>>>,
    pub talks: talk::Queue,
}

impl RoomFloor {
    pub fn new() -> RoomFloor {
        let (jobs, rx) = tokio::sync::mpsc::unbounded_channel();
        RoomFloor { clock: Mutex::new(Clock::default()), wake: tokio::sync::Notify::new(), end: Arc::new(Drained::default()), state: AtomicU8::new(BUILT), jobs, rx: Mutex::new(Some(rx)), talks: talk::Queue::new() }
    }

    pub fn running(&self) -> bool {
        self.state.load(Ordering::Acquire) == RUNNING
    }

    fn stopped(&self) -> bool {
        self.state.load(Ordering::Acquire) == STOPPED
    }
}

impl Default for RoomFloor {
    fn default() -> Self {
        RoomFloor::new()
    }
}

static LIVE: AtomicUsize = AtomicUsize::new(0);

struct Live(Arc<Drained>);

impl Drop for Live {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::AcqRel);
        self.0.done.store(true, Ordering::Release);
        self.0.wake.notify_waiters();
    }
}

pub fn count() -> usize {
    LIVE.load(Ordering::Acquire)
}

pub fn now_ms() -> u64 {
    mono_us() / 1000
}

fn wall(at_mono: u64) -> u64 {
    common::now_ms().saturating_sub(now_ms()).saturating_add(at_mono)
}

pub fn start(room: &Arc<Room>) -> bool {
    let rf = &room.exec;
    if rf.state.compare_exchange(BUILT, RUNNING, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return false;
    }
    let Some(rx) = rf.rx.lock().ok().and_then(|mut g| g.take()) else { return false };
    LIVE.fetch_add(1, Ordering::AcqRel);
    let (r, live) = (room.clone(), Live(rf.end.clone()));
    super::spawn(async move { room_task(r, rx, live).await });
    if let Some(trx) = rf.talks.claim() {
        super::spawn(talk_reporter(room.id.clone(), trx));
    }
    true
}

async fn talk_reporter(room: String, mut rx: tokio::sync::mpsc::UnboundedReceiver<Report>) {
    let mut refused: std::collections::HashSet<String> = std::collections::HashSet::new();
    while let Some(r) = rx.recv().await {
        let id = r.talk_id().to_string();
        let last = matches!(r, Report::End { .. });
        if refused.contains(&id) {
            if last {
                refused.remove(&id);
            }
            continue;
        }
        let o = common::talk::deliver(&r, common::talk::RETRY_FOR, |p| async move {
            match crate::bus::get() {
                Some(b) => b.ask(common::talk::WHAT, p, common::talk::ASK_TIMEOUT).await,
                None => None,
            }
        })
        .await;
        match o {
            common::talk::Outcome::Done => {}
            common::talk::Outcome::Refused(c) => {
                if !last {
                    refused.insert(id.clone());
                }
                tracing::info!(kind = "diag", spec = "SV-IMPL-FLR-TALK-REPORT", room = %room, talk = %id, report = r.name(), code = c.num(), "talk not recorded — no space or chat off for this room");
            }
            common::talk::Outcome::Dropped => {
                super::drop_count("talk_drop");
                tracing::warn!(kind = "diag", spec = "SV-IMPL-FLR-TALK-REPORT", room = %room, talk = %id, report = r.name(), "talk report dropped after retry limit");
            }
        }
    }
}

pub async fn drained(room: &Room) {
    let end = &room.exec.end;
    loop {
        let n = end.wake.notified();
        tokio::pin!(n);
        n.as_mut().enable();
        if end.done.load(Ordering::Acquire) {
            return;
        }
        n.await;
    }
}

pub fn stop(room: &Room) -> bool {
    if room.exec.state.swap(STOPPED, Ordering::AcqRel) != RUNNING {
        return false;
    }
    if let Ok(mut ck) = room.exec.clock.lock()
        && let Some(t) = ck.talk.take()
    {
        let dur = ck.first_rtp_at.map(|f| now_ms().saturating_sub(f)).unwrap_or(0);
        room.exec.talks.push(Report::End { talk_id: t.id, state: oxsig::msg::TalkState::Lost, reason: None, duration_ms: dur });
    }
    room.exec.wake.notify_one();
    true
}

pub fn talk_deadline(at_ms: u64) -> u64 {
    talk::deadline(at_ms, T1_MS, t2_ms(), T3_MS)
}

fn talk_steps(room: &Room, e: &Entry, fl: &Floor, ck: &mut Clock, step: &Step, now: u64) {
    let binds = step.fx.iter().any(|x| matches!(x, Fx::Bind(_)));
    let release = step.fx.iter().any(|x| matches!(x, Fx::Release(_)));
    if (release || binds)
        && let Some(t) = ck.talk.take()
    {
        let dur = ck.first_rtp_at.map(|f| now.saturating_sub(f)).unwrap_or(0);
        let end = End { labels: &step.labels, entry: e.name, why: e.why, rcause: fl.rcause_g, heard: ck.first_rtp_at.is_some(), duration_ms: dur, min_ms: super::policy().floor.talk_min_ms as u64 };
        let (state, reason) = talk::end_state(&end);
        room.exec.talks.push(Report::End { talk_id: t.id, state, reason, duration_ms: dur });
    }
    if step.labels.contains(&"FLR.S.G.TAKEN.req_regrant")
        && let Some(t) = ck.talk.as_mut()
    {
        t.deadline = talk_deadline(common::now_ms());
        room.exec.talks.push(Report::Regrant { talk_id: t.id.clone(), deadline: t.deadline });
    }
}

fn talk_start(room: &Room, ck: &mut Clock, user: &str) {
    let at = common::now_ms();
    let t = Talk { id: common::talk::talk_id(at), user: user.to_string(), start: at, deadline: talk_deadline(at) };
    room.exec.talks.push(Report::Start { talk_id: t.id.clone(), room_id: room.id.clone(), user_id: t.user.clone(), start: t.start, deadline: t.deadline, sfu_id: room.epoch.clone() });
    ck.talk = Some(t);
}

fn t2_ms() -> u64 {
    super::policy().floor.t2_stop_talking_secs as u64 * 1000
}

fn duration(name: &str) -> u64 {
    match name {
        "T1" => T1_MS,
        "T2" => t2_ms(),
        "T3" => T3_MS,
        "T7" => T7_MS,
        "T8" => T8_MS,
        "T9" => T9_MS,
        "T20" => T20_MS,
        "T21" => T21_MS,
        _ => 1_000,
    }
}

pub enum Exec {
    Bind { effect: Effect, user: String, g: u64, videos: Vec<Arc<crate::r#pub::PubStream>>, video_ok: bool },
    Release { effect: Effect },
}

pub struct Job {
    step: Step,
    sources: Vec<Vec<u32>>,
    effects: Vec<Exec>,
    talk: Option<String>,
    fact: Option<FactAt>,
}

pub struct Entry<'a> {
    pub name: &'static str,
    pub user: &'a str,
    pub ins: Vec<(&'static str, String)>,
    pub why: Option<&'static str>,
}

impl<'a> Entry<'a> {
    pub fn of(name: &'static str, user: &'a str) -> Self {
        Entry { name, user, ins: Vec::new(), why: None }
    }

    pub fn because(mut self, why: &'static str) -> Self {
        self.why = Some(why);
        self
    }

    pub fn with(mut self, k: &'static str, v: impl Into<String>) -> Self {
        self.ins.push((k, v.into()));
        self
    }
}

fn trace(room: &str, e: &Entry, from: G, to: G, step: &Step, n: u64) {
    let spec = step.labels.first().copied();
    let t = common::log::Transition { area: "FLR", key: room, spec, from: from.as_str(), to: spec.map(|_| to.as_str()), cause: e.name, inputs: &e.ins };
    let mut l = t.line().key("room", room).key("floor_sequence", n);
    if let Some(o) = step.out.iter().find(|o| matches!(o.m.t, crate::flr::core::Mt::Idle | crate::flr::core::Mt::Taken)) {
        l = l.key("mbcp_seq", o.m.q);
    }
    if !e.user.is_empty() {
        l = l.key("session", super::session_of(e.user)).key("user", e.user);
    }
    common::log::emit(l);
}

pub fn run<F>(room: &Arc<Room>, e: Entry, f: F) -> Step
where
    F: FnOnce(&mut Floor, u32) -> Option<Step>,
{
    let rf = &room.exec;
    if !rf.running() {
        return Step::default();
    }
    let plan = {
        let Ok(mut fl) = room.floor.lock() else { return Step::default() };
        let Ok(mut ck) = rf.clock.lock() else { return Step::default() };
        let now = now_ms();
        let t2_left = ck.timers.get(&("T2", None)).map(|at| (at.saturating_sub(now) / 1000) as u32).unwrap_or((t2_ms() / 1000) as u32);
        let from = fl.g;
        let held = fl.holder();
        let Some(step) = f(&mut fl, t2_left) else { return Step::default() };
        ck.events += 1;
        trace(&room.id, &e, from, fl.g, &step, ck.events);
        talk_steps(room, &e, &fl, &mut ck, &step, now);
        let mut effects = Vec::new();
        for fx in &step.fx {
            match fx {
                crate::flr::core::Fx::Bind(u) => {
                    talk_start(room, &mut ck, u);
                    ck.grant_no += 1;
                    ck.granted_at = now;
                    ck.first_rtp_at = None;
                    ck.last_rtp_at = None;
                    let (videos, video_ok) = slot_video(room, u);
                    effects.push(Exec::Bind { effect: Effect::Bind { user: u.clone(), g: ck.grant_no }, user: u.clone(), g: ck.grant_no, videos, video_ok });
                }
                crate::flr::core::Fx::Release(_) => {
                    effects.push(Exec::Release { effect: Effect::Release { g: ck.grant_no } });
                }
            }
        }
        rearm(&fl, &mut ck, now);
        let sources: Vec<Vec<u32>> = step.out.iter().map(|o| if o.m.t == Mt::Taken { o.m.a.as_deref().map(|u| slot_sources(room, u)).unwrap_or_default() } else { Vec::new() }).collect();
        let ret = step.clone();
        let talk = ck.talk.as_ref().map(|t| t.id.clone());
        let fact = fact_of((from, held.as_deref()), (fl.g, fl.holder().as_deref()))
            .map(|name| FactAt { name, speaker: fl.speaker.clone(), prev: fl.prev.clone(), seq: fl.seq, rcause: fl.rcause_g });
        if !(step.out.is_empty() && effects.is_empty() && fact.is_none()) && rf.jobs.send(Job { step, sources, effects, talk, fact }).is_err() {
            super::drop_count("exec_order");
        }
        ret
    };
    rf.wake.notify_one();
    plan
}

pub fn fact_of(before: (G, Option<&str>), after: (G, Option<&str>)) -> Option<&'static str> {
    let ((bg, bh), (ag, ah)) = (before, after);
    if ag == G::Taken && (bg != G::Taken || bh != ah) {
        return Some("floor_granted");
    }
    if ag == G::Idle && bg != G::Idle {
        return Some("floor_released");
    }
    if bg == G::Taken && ag == G::Prevoke {
        return Some("floor_revoked");
    }
    None
}

pub struct FactAt {
    name: &'static str,
    speaker: Option<String>,
    prev: Option<String>,
    seq: u32,
    rcause: u32,
}

fn floor_material(fa: &FactAt, room: &str, step: &Step, sources: &[Vec<u32>], talk: Option<&str>) -> Option<Material> {
    let first = |t: Mt| step.out.iter().position(|o| o.m.t == t);
    let room = String::from(room);
    match fa.name {
        "floor_granted" => {
            let i = first(Mt::Taken);
            let speaker = i.and_then(|i| step.out[i].m.a.clone()).or_else(|| fa.speaker.clone())?;
            let seq = i.map(|i| step.out[i].m.q).unwrap_or(fa.seq) as u16;
            let src = i.and_then(|i| sources.get(i)).filter(|v| !v.is_empty()).cloned();
            Some(Material::FloorGranted(FloorTaken { speaker, sources: src, seq, room, talk: talk.map(str::to_string) }))
        }
        "floor_released" => {
            let i = first(Mt::Idle);
            let seq = i.map(|i| step.out[i].m.q).unwrap_or(fa.seq) as u16;
            let prev = i.and_then(|i| step.out[i].m.a.clone()).or_else(|| fa.prev.clone());
            Some(Material::FloorReleased(FloorIdle { prev, seq, room }))
        }
        "floor_revoked" => {
            let cause = first(Mt::Revoke).map(|i| step.out[i].m.n).unwrap_or(fa.rcause) as u8;
            Some(Material::FloorRevoked(FloorRevoke { cause, reason: None, room }))
        }
        _ => None,
    }
}

fn rearm(fl: &Floor, ck: &mut Clock, now: u64) {
    let mut want: BTreeSet<Key> = fl.armed_server().into_iter().map(|n| (n, None)).collect();
    for u in fl.parts.keys() {
        for n in fl.armed_participant(u) {
            want.insert((n, Some(u.clone())));
        }
    }
    ck.timers.retain(|k, _| want.contains(k));
    for k in want {
        let base = match k.0 {
            "T1" => ck.last_rtp_at.unwrap_or(ck.granted_at).max(ck.granted_at),
            "T2" => ck.first_rtp_at.unwrap_or(now),
            _ => now,
        };
        let d = duration(k.0);
        ck.timers.entry(k).or_insert(base + d);
    }
}

pub struct ExecPlan {
    pub applied: Vec<Exec>,
    pub burst: Vec<Arc<crate::r#pub::PubTrack>>,
}

pub fn plan_exec(room: &Room, effects: Vec<Exec>) -> ExecPlan {
    let mut applied = Vec::new();
    for e in effects {
        let effect = match &e {
            Exec::Bind { effect, .. } | Exec::Release { effect, .. } => effect.clone(),
        };
        let a = room.slot(Kind::Audio).bind.lock().map(|mut s| s.apply(&effect)).unwrap_or(false);
        let v = room.slot(Kind::Video).bind.lock().map(|mut s| s.apply(&effect)).unwrap_or(false);
        if !(a || v) {
            if matches!(effect, Effect::Bind { .. }) {
                super::drop_count("exec_order");
            }
            continue;
        }
        match &e {
            Exec::Bind { user, g, video_ok, .. } => {
                if !video_ok {
                    tracing::warn!(kind = "diag", spec = "SV-IMPL-FLR-MEDIA-BIND", session = %super::session_of(user), room = %room.id, user = %user, "slot video codec differs from room — video not bound");
                }
                let speaker = Arc::new(user.clone());
                room.slot(Kind::Audio).bind(Some(speaker.clone()), *g);
                room.slot(Kind::Video).bind(video_ok.then_some(speaker), *g);
            }
            Exec::Release { .. } => {
                room.slot(Kind::Audio).release();
                room.slot(Kind::Video).release();
            }
        }
        applied.push(e);
    }
    let burst = applied
        .iter()
        .filter_map(|e| match e {
            Exec::Bind { videos, video_ok: true, .. } => Some(videos),
            _ => None,
        })
        .flat_map(|vs| vs.iter().flat_map(|st| st.physical()))
        .collect();
    ExecPlan { applied, burst }
}

fn execute(room: &Arc<Room>, job: Job) {
    let plan = plan_exec(room, job.effects);
    let reason = if job.step.labels.contains(&"FLR.S.gate.not_in_room") { Some("not_in_room") } else { None };
    for (o, sources) in job.step.out.iter().zip(job.sources.iter()) {
        if let Some(m) = to_mbcp(&o.m, &room.id, reason, sources, job.talk.as_deref()) {
            send(&o.to, &m);
        }
    }
    if let Some(fa) = &job.fact
        && let Some(m) = floor_material(fa, &room.id, &job.step, &job.sources, job.talk.as_deref())
    {
        crate::bus::room_material(&room.id, &m);
    }
    let fan = room.slot(Kind::Video);
    for t in &plan.burst {
        pli_while_bound(fan, t);
    }
}

pub fn to_mbcp(m: &Msg, room: &str, reason: Option<&str>, sources: &[u32], talk: Option<&str>) -> Option<mbcp::Msg> {
    Some(match m.t {
        Mt::Granted => mbcp::granted(m.n as u8, m.d as u16, room, talk),
        Mt::Taken => mbcp::taken(m.a.as_deref().unwrap_or(""), m.q as u16, room, sources, talk),
        Mt::Idle => mbcp::idle(m.q as u16, room, m.a.as_deref()),
        Mt::Deny => mbcp::deny_msg(m.n as u8, room, reason),
        Mt::Revoke => mbcp::revoke_msg(m.n as u8, room, None),
        Mt::Qinfo => mbcp::queue_info(m.n.min(255) as u8, m.d as u8, room),
        Mt::Ack => mbcp::ack(m.n as u8, room),
        _ => return None,
    })
}

pub fn from_mbcp(m: &mbcp::Msg) -> Option<Msg> {
    Some(match m.msg_type {
        mbcp::REQUEST => Msg::new(Mt::Request, None, m.priority().unwrap_or(0) as u32, 0),
        mbcp::RELEASE => Msg::new(Mt::Release, None, 0, 0),
        mbcp::QPOS_REQ => Msg::new(Mt::QposReq, None, 0, 0),
        mbcp::ACK => Msg::new(Mt::Ack, None, m.ack_of().unwrap_or(0) as u32, 0),
        _ => return None,
    })
}

pub fn send(user: &str, m: &mbcp::Msg) {
    let Some(b) = mbcp::encode(m) else { return };
    let Some(frame) = oxsig::dc::encode(oxsig::dc::SVC_FLOOR, &b) else { return };
    let Some(s) = super::sfu() else { return };
    if let Some(p) = s.peer(user) {
        crate::aux::dc_send(&p, frame);
    }
}

pub fn dc_in(tr: &Arc<Transport>, buf: &[u8]) {
    let Some(fr) = oxsig::dc::decode(buf) else {
        super::drop_count("dc_bad_frame");
        return;
    };
    match fr.svc {
        oxsig::dc::SVC_FLOOR => {}
        oxsig::dc::SVC_VOICE_ACTIVITY => return,
        _ => {
            super::drop_count("dc_unknown_svc");
            return;
        }
    }
    let Some(m) = mbcp::decode(fr.payload) else {
        super::drop_count("dc_bad_frame");
        return;
    };
    let Some(s) = super::sfu() else { return };
    let Some(room) = m.room().and_then(|r| s.rooms.get(&r)) else {
        super::drop_count("dc_no_room");
        return;
    };
    let joined = s.peer(&tr.user).map(|p| p.in_room(&room.id)).unwrap_or(false);
    if !joined && m.msg_type != mbcp::REQUEST {
        super::drop_count("dc_no_room");
        return;
    }
    if m.msg_type == mbcp::RELEASE {
        tr.release_rx_us.store(tr.dtls_rx_us.load(Ordering::Acquire), Ordering::Release);
    }
    let Some(cm) = from_mbcp(&m) else { return };
    let user = tr.user.clone();
    let half = super::media::has_half(&user);
    run(&room, Entry::of("server_rx", &user), |f, t2| Some(f.server_rx(&user, &cm, half, t2)));
}

pub fn obs_recorded(pkt_grant: u64, clock_grant: u64) -> bool {
    pkt_grant != 0 && pkt_grant == clock_grant
}

pub fn on_rtp(room_id: &str, user: &str, first: bool, grant: u64) {
    let Some(s) = super::sfu() else { return };
    let Some(room) = s.rooms.get(room_id) else { return };
    if room.exec.running()
        && let Ok(mut ck) = room.exec.clock.lock()
        && obs_recorded(grant, ck.grant_no)
    {
        let now = now_ms();
        if first || ck.first_rtp_at.is_none() {
            ck.first_rtp_at.get_or_insert(now);
        }
        ck.last_rtp_at = Some(now);
    }
    run(&room, Entry::of("rtp", user), |f, _| Some(f.rtp(user)));
}

fn last_rtp_ms(room: &Room, grant: u64) -> Option<u64> {
    let at = |k: Kind| {
        let ep = room.slot(k).ep.load();
        if obs_recorded(ep.grant, grant) { ep.last_rtp_us.load(Ordering::Acquire) } else { 0 }
    };
    let m = at(Kind::Audio).max(at(Kind::Video));
    if m == 0 { None } else { Some(m / 1000) }
}

async fn room_task(room: Arc<Room>, mut jobs: tokio::sync::mpsc::UnboundedReceiver<Job>, _live: Live) {
    let rf = &room.exec;
    loop {
        if rf.stopped() {
            while let Ok(job) = jobs.try_recv() {
                execute(&room, job);
            }
            return;
        }
        let next = rf.clock.lock().ok().and_then(|c| c.timers.values().min().copied());
        let wait = next.map(|at| at.saturating_sub(now_ms())).unwrap_or(60_000);
        tokio::select! {
            biased;
            job = jobs.recv() => match job {
                Some(job) => execute(&room, job),
                None => return,
            },
            _ = rf.wake.notified() => {}
            _ = tokio::time::sleep(Duration::from_millis(wait)) => fire_due(&room),
        }
    }
}

fn fire_due(room: &Arc<Room>) {
    let now = now_ms();
    let due: Vec<Key> = match room.exec.clock.lock() {
        Ok(mut c) => {
            let lr = last_rtp_ms(room, c.grant_no).filter(|t| *t >= c.granted_at);
            if lr.is_some() {
                c.last_rtp_at = lr;
            }
            if let Some(at) = c.timers.get(&("T1", None)).copied()
                && at <= now
            {
                let base = c.last_rtp_at.unwrap_or(c.granted_at).max(c.granted_at);
                if base + T1_MS > now {
                    c.timers.insert(("T1", None), base + T1_MS);
                }
            }
            let ks: Vec<Key> = c.timers.iter().filter(|(_, at)| **at <= now).map(|(k, _)| k.clone()).collect();
            for k in &ks {
                c.timers.remove(k);
            }
            ks
        }
        Err(_) => return,
    };
    for (name, user) in due {
        let u = user.clone().unwrap_or_default();
        run(room, Entry::of(timer_entry(name), &u), |f, _| {
            let armed = if user.is_some() { f.armed_participant(&u) } else { f.armed_server() };
            if !armed.contains(&name) {
                return None;
            }
            Some(fire(f, name, &u))
        });
    }
}

fn timer_entry(name: &str) -> &'static str {
    match name {
        "T1" => "t1",
        "T2" => "t2",
        "T3" => "t3",
        "T7" => "t7",
        "T20" => "t20",
        "T8" => "t8",
        "T21" => "t21",
        "T9" => "t9",
        _ => "timer",
    }
}

fn fire(f: &mut Floor, name: &str, u: &str) -> Step {
    match name {
        "T1" => f.t1(),
        "T2" => f.t2(),
        "T3" => f.t3(),
        "T7" => f.t7(),
        "T20" => f.t20(),
        "T8" => f.t8(u),
        "T21" => f.t21(u),
        "T9" => f.t9(u),
        _ => Step::default(),
    }
}

fn slot_sources(room: &Room, user: &str) -> Vec<u32> {
    super::media::half_in(&room.id, user).into_iter().map(|st| st.csrc).collect()
}

fn slot_video(room: &Room, user: &str) -> (Vec<Arc<crate::r#pub::PubStream>>, bool) {
    let vs: Vec<_> = super::media::half_in(&room.id, user).into_iter().filter(|st| st.kind == Kind::Video).collect();
    let ok = vs.iter().all(|st| crate::trk::matches_room(Kind::Video, st.codec.as_deref(), st.fmtp.as_deref(), &room.media_codecs));
    (vs, ok)
}

fn pli_while_bound(fan: &Arc<SlotFan>, t: &Arc<crate::r#pub::PubTrack>) {
    let ep = fan.episode();
    for d in crate::rtcp::PLI_BURST_MS {
        let (fan, t) = (fan.clone(), t.clone());
        super::spawn(async move {
            tokio::time::sleep(Duration::from_millis(d)).await;
            if fan.episode() == ep {
                crate::rtcp::request_pli(&t, crate::rtcp::PLI_BURST);
            }
        });
    }
}

pub fn detail(room: &Room) -> FloorView {
    let (speaker, queue) = match room.floor.lock() {
        Ok(f) => (f.holder(), f.queue.iter().map(|w| w.u.clone()).collect()),
        Err(_) => (None, Vec::new()),
    };
    let start = speaker.as_ref().and_then(|_| room.exec.clock.lock().ok().map(|c| wall(c.granted_at)));
    FloorView { speaker, queue, start }
}

pub fn snapshot(room: &Room) -> serde_json::Value {
    let (state, speaker, prio, queue) = match room.floor.lock() {
        Ok(f) => {
            let st = match f.g {
                G::Idle => "IDLE",
                G::Taken => "TAKEN",
                G::Prevoke => "PREVOKE",
            };
            let q: Vec<serde_json::Value> = f.queue.iter().map(|w| serde_json::json!({ "user_id": w.u, "priority": w.p })).collect();
            (st, f.holder(), if f.g == G::Taken { f.sprio } else { f.rprio }, q)
        }
        Err(_) => ("IDLE", None, 0, Vec::new()),
    };
    let rf = &room.exec;
    let (first, last, mut timers, granted, grant_no) = match rf.running().then(|| rf.clock.lock().ok().map(|c| (c.first_rtp_at, c.last_rtp_at, c.timers.clone(), c.granted_at, c.grant_no))).flatten() {
        Some((a, b, t, g, n)) => (a, b, t, g, n),
        None => (None, None, BTreeMap::new(), 0, 0),
    };
    let last = last_rtp_ms(room, grant_no).filter(|t| *t >= granted).max(last);
    if let (Some(at), Some(lr)) = (timers.get_mut(&("T1", None)), last) {
        let pushed = lr.max(granted) + T1_MS;
        if pushed > *at {
            let _old = std::mem::replace(at, pushed);
        }
    }
    let timers: Vec<serde_json::Value> = timers
        .into_iter()
        .map(|((n, u), at)| {
            let mut o = serde_json::json!({ "name": n, "at": wall(at) });
            if let Some(u) = u {
                o["user_id"] = serde_json::json!(u);
            }
            o
        })
        .collect();
    serde_json::json!({
        "state": state,
        "speaker": speaker,
        "priority": prio,
        "queue": queue,
        "first_rtp_at": first.map(wall),
        "last_rtp_at": last.map(wall),
        "timers": timers,
    })
}
