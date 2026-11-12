// author: kodeholic (powered by Claude)
// spec: v1.7 · 정:IMPL-FLR-LOCK · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FLR-FIRST-NOTICE · 정:IMPL-FLR-MEMBER-JOIN · 정:RULE-FLR-SNAPSHOT · 연:RULE-FLR-TIMERS · 연:TRN.frame.DC · 정:IMPL-FLR-POLICY · 정:CAND-FLR-RELEASE-SILENCE-SPEAKER · 정:CAND-FWD-PRIME-SOURCE · 정:CAND-FWD-PRIME-EPISODE · 운영:RULE-LOG-TRANSITION · model: claude-opus-5-5
//! 발언권 집행 — 판정은 방 락 안(보낼 것 · 걸 타이머 · 효과에 `grant_no`) · 집행은 락 밖(MBCP 를 DC 로 · 타이머는 가장 이른 만료 시각에 깨어난다 · 슬롯 결합 · 예열 CN · 해제 무음 · PLI burst).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use oxsig::floor as mbcp;
use oxsig::msg::Kind;

use crate::flr::bind::Effect;
use crate::flr::core::{Floor, G, Msg, Mt, Step};
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
}

pub struct RoomFloor {
    pub clock: Mutex<Clock>,
    pub wake: tokio::sync::Notify,
    pub stop: std::sync::atomic::AtomicBool,
    pub jobs: tokio::sync::mpsc::UnboundedSender<Job>,
}

static CLOCKS: OnceLock<Mutex<HashMap<String, Arc<RoomFloor>>>> = OnceLock::new();

fn clocks() -> &'static Mutex<HashMap<String, Arc<RoomFloor>>> {
    CLOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn now_ms() -> u64 {
    mono_us() / 1000
}

fn wall(at_mono: u64) -> u64 {
    common::now_ms().saturating_sub(now_ms()).saturating_add(at_mono)
}

pub fn of(room: &str) -> Option<Arc<RoomFloor>> {
    clocks().lock().ok().and_then(|g| g.get(room).cloned())
}

pub fn open(room: &Arc<Room>) {
    let (jobs, rx) = tokio::sync::mpsc::unbounded_channel();
    let rf = Arc::new(RoomFloor { clock: Mutex::new(Clock::default()), wake: tokio::sync::Notify::new(), stop: Default::default(), jobs });
    if let Ok(mut g) = clocks().lock() {
        g.insert(room.id.clone(), rf.clone());
    }
    let r = room.clone();
    super::spawn(async move { room_task(r, rf, rx).await });
}

pub fn close(room: &str) {
    if let Some(rf) = clocks().lock().ok().and_then(|mut g| g.remove(room)) {
        rf.stop.store(true, Ordering::Release);
        rf.wake.notify_one();
    }
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
}

pub struct Entry<'a> {
    pub name: &'static str,
    pub user: &'a str,
    pub ins: Vec<(&'static str, String)>,
}

impl<'a> Entry<'a> {
    pub fn of(name: &'static str, user: &'a str) -> Self {
        Entry { name, user, ins: Vec::new() }
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
    let Some(rf) = of(&room.id) else { return Step::default() };
    let plan = {
        let Ok(mut fl) = room.floor.lock() else { return Step::default() };
        let Ok(mut ck) = rf.clock.lock() else { return Step::default() };
        let now = now_ms();
        let t2_left = ck.timers.get(&("T2", None)).map(|at| (at.saturating_sub(now) / 1000) as u32).unwrap_or((t2_ms() / 1000) as u32);
        let from = fl.g;
        let Some(step) = f(&mut fl, t2_left) else { return Step::default() };
        ck.events += 1;
        trace(&room.id, &e, from, fl.g, &step, ck.events);
        let mut effects = Vec::new();
        for fx in &step.fx {
            match fx {
                crate::flr::core::Fx::Bind(u) => {
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
        let sources = step.out.iter().map(|o| if o.m.t == Mt::Taken { o.m.a.as_deref().map(|u| slot_sources(room, u)).unwrap_or_default() } else { Vec::new() }).collect();
        let ret = step.clone();
        if !(step.out.is_empty() && effects.is_empty()) && rf.jobs.send(Job { step, sources, effects }).is_err() {
            super::drop_count("exec_order");
        }
        ret
    };
    rf.wake.notify_one();
    plan
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

fn execute(room: &Arc<Room>, job: Job) {
    let mut applied = Vec::new();
    for e in job.effects {
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
    let reason = if job.step.labels.contains(&"FLR.S.gate.not_in_room") { Some("not_in_room") } else { None };
    for (o, sources) in job.step.out.iter().zip(job.sources.iter()) {
        if let Some(m) = to_mbcp(&o.m, &room.id, reason, sources) {
            send(&o.to, &m);
        }
    }
    for e in &applied {
        if let Exec::Bind { videos, video_ok: true, .. } = e {
            let fan = room.slot(Kind::Video);
            for st in videos {
                for t in st.physical() {
                    pli_while_bound(fan, &t);
                }
            }
        }
    }
}

pub fn to_mbcp(m: &Msg, room: &str, reason: Option<&str>, sources: &[u32]) -> Option<mbcp::Msg> {
    Some(match m.t {
        Mt::Granted => mbcp::granted(m.n as u8, m.d as u16, room),
        Mt::Taken => mbcp::taken(m.a.as_deref().unwrap_or(""), m.q as u16, room, sources),
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
    if let Some(rf) = of(room_id)
        && let Ok(mut ck) = rf.clock.lock()
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

async fn room_task(room: Arc<Room>, rf: Arc<RoomFloor>, mut jobs: tokio::sync::mpsc::UnboundedReceiver<Job>) {
    loop {
        if rf.stop.load(Ordering::Acquire) {
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
            _ = tokio::time::sleep(Duration::from_millis(wait)) => fire_due(&room, &rf),
        }
    }
}

fn fire_due(room: &Arc<Room>, rf: &Arc<RoomFloor>) {
    let now = now_ms();
    let due: Vec<Key> = match rf.clock.lock() {
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
    let Some(ctx) = super::sfu().and_then(|s| s.pubs.get(user)) else { return Vec::new() };
    ctx.streams().into_iter().filter(|st| st.is_half() && st.room.load().as_str() == room.id).map(|st| st.csrc).collect()
}

fn slot_video(room: &Room, user: &str) -> (Vec<Arc<crate::r#pub::PubStream>>, bool) {
    let Some(ctx) = super::sfu().and_then(|s| s.pubs.get(user)) else { return (Vec::new(), true) };
    let vs: Vec<_> = ctx.streams().into_iter().filter(|st| st.kind == Kind::Video && st.is_half() && st.room.load().as_str() == room.id).collect();
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
                crate::rtcp::request_pli(&t, true);
            }
        });
    }
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
    let (first, last, mut timers, granted, grant_no) = match of(&room.id).and_then(|rf| rf.clock.lock().ok().map(|c| (c.first_rtp_at, c.last_rtp_at, c.timers.clone(), c.granted_at, c.grant_no))) {
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
