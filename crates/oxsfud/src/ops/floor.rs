// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-FLR-LOCK · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FLR-FIRST-NOTICE · 정:IMPL-FLR-MEMBER-JOIN · 정:RULE-FLR-SNAPSHOT · 연:RULE-FLR-TIMERS · 연:TRN.frame.DC · 정:IMPL-FLR-POLICY · 정:CAND-FLR-RELEASE-SILENCE-SPEAKER · 정:CAND-FWD-PRIME-SOURCE · 정:CAND-FWD-PRIME-EPISODE · model: claude-opus-5-5
//! 발언권 집행 — 판정은 방 락 안(보낼 것 · 걸 타이머 · 효과에 `grant_no`) · 집행은 락 밖(MBCP 를 DC 로 · 타이머는 가장 이른 만료 시각에 깨어난다 · 슬롯 결합 · 예열 CN · 해제 무음 · PLI burst).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use oxsig::floor as mbcp;
use oxsig::msg::Kind;

use crate::flr::bind::Effect;
use crate::flr::core::{Floor, G, Msg, Mt, Step};
use crate::fwd::rt::{Item, media, mono_us};
use crate::fwd::{Ctl, DownRow, OPUS_SILENCE, PRIME_FRAMES, SILENCE_FRAMES, SlotFan};
use crate::link::Transport;
use crate::room::Room;

pub const T1_MS: u64 = mbcp::timers::T1_MS;
pub const T3_MS: u64 = mbcp::timers::T3_MS;
pub const T7_MS: u64 = mbcp::timers::T7_MS;
pub const T8_MS: u64 = mbcp::timers::T8_MS;
pub const T9_MS: u64 = mbcp::timers::T9_MS;
pub const T20_MS: u64 = mbcp::timers::T20_MS;
pub const T21_MS: u64 = mbcp::timers::T21_MS;
pub const FRAME_MS: u64 = 20;

type Key = (&'static str, Option<String>);

#[derive(Default)]
pub struct Clock {
    pub timers: BTreeMap<Key, u64>,
    pub grant_no: u64,
    pub granted_at: u64,
    pub first_rtp_at: Option<u64>,
    pub last_rtp_at: Option<u64>,
}

pub struct RoomFloor {
    pub clock: Mutex<Clock>,
    pub wake: tokio::sync::Notify,
    pub stop: std::sync::atomic::AtomicBool,
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
    let rf = Arc::new(RoomFloor { clock: Mutex::new(Clock::default()), wake: tokio::sync::Notify::new(), stop: Default::default() });
    if let Ok(mut g) = clocks().lock() {
        g.insert(room.id.clone(), rf.clone());
    }
    let r = room.clone();
    super::spawn(async move { timer_task(r, rf).await });
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

struct Plan {
    step: Step,
    effects: Vec<Effect>,
    from_idle: bool,
}

pub fn run<F>(room: &Arc<Room>, f: F) -> Step
where
    F: FnOnce(&mut Floor, u32) -> Step,
{
    let Some(rf) = of(&room.id) else { return Step::default() };
    let plan = {
        let Ok(mut fl) = room.floor.lock() else { return Step::default() };
        let Ok(mut ck) = rf.clock.lock() else { return Step::default() };
        let now = now_ms();
        let t2_left = ck.timers.get(&("T2", None)).map(|at| (at.saturating_sub(now) / 1000) as u32).unwrap_or((t2_ms() / 1000) as u32);
        let was_idle = fl.g == G::Idle;
        let step = f(&mut fl, t2_left);
        let mut effects = Vec::new();
        for fx in &step.fx {
            match fx {
                crate::flr::core::Fx::Bind(u) => {
                    ck.grant_no += 1;
                    ck.granted_at = now;
                    ck.first_rtp_at = None;
                    ck.last_rtp_at = None;
                    effects.push(Effect::Bind { user: u.clone(), g: ck.grant_no });
                }
                crate::flr::core::Fx::Release(_) => effects.push(Effect::Release { g: ck.grant_no }),
            }
        }
        rearm(&fl, &mut ck, now);
        Plan { step, effects, from_idle: was_idle }
    };
    rf.wake.notify_one();
    execute(room, &plan);
    plan.step
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

fn execute(room: &Arc<Room>, p: &Plan) {
    let reason = if p.step.labels.contains(&"FLR.S.gate.not_in_room") { Some("not_in_room") } else { None };
    for o in &p.step.out {
        if let Some(m) = to_mbcp(&o.m, &room.id, reason) {
            send(&o.to, &m);
        }
    }
    let prev = room.floor.lock().ok().and_then(|f| f.prev.clone());
    for e in &p.effects {
        let a = room.slot(Kind::Audio).bind.lock().map(|mut s| s.apply(e)).unwrap_or(false);
        let v = room.slot(Kind::Video).bind.lock().map(|mut s| s.apply(e)).unwrap_or(false);
        if !(a || v) {
            continue;
        }
        match e {
            Effect::Bind { user, .. } => bind_media(room, user, p.from_idle),
            Effect::Release { .. } => release_media(room, prev.as_deref()),
        }
    }
}

pub fn to_mbcp(m: &Msg, room: &str, reason: Option<&str>) -> Option<mbcp::Msg> {
    Some(match m.t {
        Mt::Granted => mbcp::granted(m.n as u8, m.d as u16, m.q as u16, room),
        Mt::Taken => mbcp::taken(m.a.as_deref().unwrap_or(""), m.q as u16, room),
        Mt::Idle => mbcp::idle(m.q as u16, room, m.a.as_deref()),
        Mt::Deny => mbcp::deny_msg(m.n as u8, room, reason),
        Mt::Revoke => mbcp::revoke_msg(m.n as u8, room, None),
        Mt::Qinfo => mbcp::queue_info(m.n.min(255) as u8, m.d as u8, room),
        Mt::Ack => mbcp::ack(m.n as u8, room, Some(m.q as u16)),
        _ => return None,
    })
}

pub fn from_mbcp(m: &mbcp::Msg) -> Option<Msg> {
    Some(match m.msg_type {
        mbcp::REQUEST => Msg::new(Mt::Request, None, m.priority().unwrap_or(0) as u32, 0),
        mbcp::RELEASE => Msg::new(Mt::Release, None, 0, 0),
        mbcp::QPOS_REQ => Msg::new(Mt::QposReq, None, 0, 0),
        mbcp::ACK => {
            let mut x = Msg::new(Mt::Ack, None, m.ack_of().unwrap_or(0) as u32, 0);
            x.q = m.seq().unwrap_or(0) as u32;
            x
        }
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
    run(&room, |f, t2| f.server_rx(&user, &cm, t2));
}

pub fn on_rtp(room_id: &str, user: &str, first: bool) {
    let Some(s) = super::sfu() else { return };
    let Some(room) = s.rooms.get(room_id) else { return };
    if let Some(rf) = of(room_id)
        && let Ok(mut ck) = rf.clock.lock()
    {
        let speaker = room.slot(Kind::Audio).speaker_is(user) || room.slot(Kind::Video).speaker_is(user);
        if speaker {
            let now = now_ms();
            if first || ck.first_rtp_at.is_none() {
                ck.first_rtp_at.get_or_insert(now);
            }
            ck.last_rtp_at = Some(now);
        }
    }
    run(&room, |f, _| f.rtp(user));
}

fn last_rtp_ms(room: &Room) -> Option<u64> {
    let a = room.slot(Kind::Audio).last_rtp_us.load(Ordering::Acquire);
    let v = room.slot(Kind::Video).last_rtp_us.load(Ordering::Acquire);
    let m = a.max(v);
    if m == 0 { None } else { Some(m / 1000) }
}

async fn timer_task(room: Arc<Room>, rf: Arc<RoomFloor>) {
    loop {
        if rf.stop.load(Ordering::Acquire) {
            return;
        }
        let next = rf.clock.lock().ok().and_then(|c| c.timers.values().min().copied());
        let wait = next.map(|at| at.saturating_sub(now_ms())).unwrap_or(60_000);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
            _ = rf.wake.notified() => continue,
        }
        fire_due(&room, &rf);
    }
}

fn fire_due(room: &Arc<Room>, rf: &Arc<RoomFloor>) {
    let now = now_ms();
    let due: Vec<Key> = match rf.clock.lock() {
        Ok(mut c) => {
            let lr = last_rtp_ms(room).filter(|t| *t >= c.granted_at);
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
        run(room, |f, _| {
            let armed = if user.is_some() { f.armed_participant(&u) } else { f.armed_server() };
            if !armed.contains(&name) {
                return Step::default();
            }
            fire(f, name, &u)
        });
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

fn group_rows(rows: &[Arc<DownRow>], skip: Option<&str>) -> Vec<Vec<Arc<DownRow>>> {
    let n = media().map(|m| m.workers.len()).unwrap_or(1).max(1);
    let mut by: Vec<Vec<Arc<DownRow>>> = vec![Vec::new(); n];
    for r in rows {
        if Some(r.user.as_str()) == skip {
            continue;
        }
        by[r.conn.worker.min(n - 1)].push(r.clone());
    }
    by
}

fn push_frames(fan: &SlotFan, skip: Option<&str>, key: u64) {
    let Some(m) = media() else { return };
    let rows: Vec<Arc<DownRow>> = fan.rows.load().iter().cloned().collect();
    let seq = fan.prime_seq.fetch_add(1, Ordering::AcqRel);
    let ts = fan.prime_ts.fetch_add(960, Ordering::AcqRel);
    let from = m.workers.len() + 1;
    for (i, g) in group_rows(&rows, skip).into_iter().enumerate() {
        if g.is_empty() {
            continue;
        }
        let ctl = Ctl::Frames { rows: g, payload: OPUS_SILENCE.to_vec(), src_key: key, seq, ts };
        if !m.workers[i].push(from, Item::Ctl(ctl)) {
            super::drop_count("egress_queue_full");
        }
    }
}

fn room_audio_opus(room: &Room) -> bool {
    room.media_codecs.audio.as_deref().map(|a| a.eq_ignore_ascii_case("opus")).unwrap_or(true)
}

fn slot_video(room: &Room, user: &str) -> (Vec<Arc<crate::r#pub::PubStream>>, bool) {
    let Some(ctx) = super::sfu().and_then(|s| s.pubs.get(user)) else { return (Vec::new(), true) };
    let vs: Vec<_> = ctx.streams().into_iter().filter(|st| st.kind == Kind::Video && st.is_half() && st.room.load().as_str() == room.id).collect();
    let ok = vs.iter().all(|st| crate::trk::matches_room(Kind::Video, st.codec.as_deref(), st.fmtp.as_deref(), &room.media_codecs));
    (vs, ok)
}

fn pli_while_bound(fan: &Arc<SlotFan>, t: &Arc<crate::r#pub::PubTrack>) {
    let ep = fan.episode.load(Ordering::Acquire);
    for d in crate::rtcp::PLI_BURST_MS {
        let (fan, t) = (fan.clone(), t.clone());
        super::spawn(async move {
            tokio::time::sleep(Duration::from_millis(d)).await;
            if fan.episode.load(Ordering::Acquire) == ep {
                crate::rtcp::request_pli(&t, true);
            }
        });
    }
}

fn bind_media(room: &Arc<Room>, user: &str, from_idle: bool) {
    let (videos, video_ok) = slot_video(room, user);
    if !video_ok {
        tracing::warn!(kind = "diag", room = %room.id, user = %user, "slot video codec differs from room — video not bound");
    }
    for k in [Kind::Audio, Kind::Video] {
        let fan = room.slot(k);
        let bind = k == Kind::Audio || video_ok;
        fan.speaker.store(bind.then(|| Arc::new(user.to_string())));
        fan.episode.fetch_add(1, Ordering::AcqRel);
        fan.first_rtp_us.store(0, Ordering::Release);
        fan.last_notify_us.store(0, Ordering::Release);
        if k == Kind::Video {
            for r in fan.rows.load().iter() {
                r.rtx_gate_until_new.store(true, Ordering::Release);
            }
        }
    }
    let audio = room.slot(Kind::Audio).clone();
    if from_idle && room_audio_opus(room) {
        audio.priming.store(PRIME_FRAMES, Ordering::Release);
        let key = audio.episode.load(Ordering::Acquire);
        let speaker = user.to_string();
        super::spawn(async move {
            for _ in 0..PRIME_FRAMES {
                if audio.priming.load(Ordering::Acquire) == 0 || audio.episode.load(Ordering::Acquire) != key {
                    break;
                }
                push_frames(&audio, Some(&speaker), key);
                tokio::time::sleep(Duration::from_millis(FRAME_MS)).await;
            }
            audio.priming.store(0, Ordering::Release);
        });
    }
    if video_ok {
        let fan = room.slot(Kind::Video);
        for st in &videos {
            for t in st.physical() {
                pli_while_bound(fan, &t);
            }
        }
    }
}

fn release_media(room: &Arc<Room>, prev: Option<&str>) {
    if let Some(p) = prev {
        for st in slot_video(room, p).0 {
            for t in st.physical() {
                if let Ok(mut g) = t.pli.lock() {
                    g.pending = false;
                }
            }
        }
    }
    for k in [Kind::Audio, Kind::Video] {
        let fan = room.slot(k);
        fan.speaker.store(None);
        fan.priming.store(0, Ordering::Release);
        fan.episode.fetch_add(1, Ordering::AcqRel);
        if k == Kind::Video {
            for r in fan.rows.load().iter() {
                r.rtx_gate_until_new.store(true, Ordering::Release);
            }
        }
    }
    if !room_audio_opus(room) {
        return;
    }
    let audio = room.slot(Kind::Audio).clone();
    let key = audio.episode.load(Ordering::Acquire);
    let skip = prev.map(|p| p.to_string());
    super::spawn(async move {
        for _ in 0..SILENCE_FRAMES {
            if audio.episode.load(Ordering::Acquire) != key {
                break;
            }
            push_frames(&audio, skip.as_deref(), key);
            tokio::time::sleep(Duration::from_millis(FRAME_MS)).await;
        }
    });
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
    let (first, last, mut timers, granted) = match of(&room.id).and_then(|rf| rf.clock.lock().ok().map(|c| (c.first_rtp_at, c.last_rtp_at, c.timers.clone(), c.granted_at))) {
        Some((a, b, t, g)) => (a, b, t, g),
        None => (None, None, BTreeMap::new(), 0),
    };
    let last = last_rtp_ms(room).filter(|t| *t >= granted).max(last);
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
