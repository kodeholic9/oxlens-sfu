// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-FLR-LOCK · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-FLR-POLICY · 정:IMPL-FLR-FIRST-NOTICE · 정:RULE-FLR-SNAPSHOT · 연:FLR.wire · 연:RULE-FLR-TIMERS · 정:RULE-CONC-SIX · model: claude-opus-5-5
//! 발언권 집행 — 방마다 한 락이 판정 상태를 지키고 판정은 목록을 낸다 · 송신 · 결합은 락 밖 · 타이머는 가장 이른 만료 시각에 깨어난다 · 화자 RTP 는 사람마다 500ms 에 한 번 알린다.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use oxsig::flr::{self as w, tlv};
use serde_json::{json, Value};

use super::core::{armed_participant, armed_server, ms_of, Cfg, Fx, Msg, Out, Srv, Step, Timer, G, T};

pub const RTP_NOTE_MS: u64 = 500;

fn lock<U>(m: &Mutex<U>) -> std::sync::MutexGuard<'_, U> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn now_ms() -> u64 {
    common::clock::wall_ms()
}

#[derive(Default)]
struct St {
    srv: Srv,
    due: BTreeMap<(Timer, Option<String>), u64>,
    t1_base: u64,
    granted_at: u64,
    fseq: u64,
}

pub struct Floor {
    pub room_id: String,
    epoch: String,
    st: Mutex<St>,
    wake: tokio::sync::Notify,
    pub last_rtp_at: AtomicU64,
    pub first_rtp_at: AtomicU64,
    pub holder: ArcSwapOption<String>,
    noted: Mutex<BTreeMap<String, u64>>,
    q_max: usize,
    t2_secs: u32,
    started: AtomicBool,
    pub dead: AtomicBool,
}

pub enum Act {
    Send(String, Vec<u8>),
    Bind(String, bool),
    Release(Option<String>),
}

fn code(t: T) -> u8 {
    match t {
        T::Request => w::REQUEST,
        T::Granted => w::GRANTED,
        T::Taken => w::TAKEN,
        T::Deny => w::DENY,
        T::Release => w::RELEASE,
        T::Idle => w::IDLE,
        T::Revoke => w::REVOKE,
        T::QposReq => w::QPOS_REQ,
        T::Qinfo => w::QINFO,
        T::Ack => w::ACK,
        T::Rtp => 15,
    }
}

pub fn wire_out(room: &str, m: &Msg) -> Option<Vec<u8>> {
    let t = code(m.t);
    let mut x = w::Msg::new(t);
    x = match m.t {
        T::Granted => x.with_u8spare(tlv::PRIORITY, m.n.min(255) as u8).with_u16(tlv::DURATION, m.d.min(65535) as u16).with_u16(tlv::SEQ, m.q),
        T::Taken => x.with_str(tlv::SPEAKER, m.a.as_deref().unwrap_or("")).with_u16(tlv::SEQ, m.q),
        T::Idle => {
            let x = match &m.a {
                Some(p) => x.with_str(tlv::PREV, p),
                None => x,
            };
            x.with_u16(tlv::SEQ, m.q)
        }
        T::Deny | T::Revoke => x.with_u8(tlv::CAUSE, m.n.min(255) as u8),
        T::Qinfo => x.with_pair(tlv::QINFO, m.n.min(255) as u8, m.d.min(255) as u8),
        _ => x,
    };
    let x = x.with_str(tlv::ROOM, room);
    let payload = w::encode(&x)?;
    oxsig::trn::encode_dc(oxsig::trn::SVC_MBCP, &payload)
}

pub fn wire_in(x: &w::Msg) -> Option<Msg> {
    let t = match x.msg_type {
        w::REQUEST => T::Request,
        w::RELEASE => T::Release,
        w::QPOS_REQ => T::QposReq,
        w::ACK => T::Ack,
        _ => return None,
    };
    let mut m = Msg::new(t, None, 0, 0);
    match t {
        T::Request => m.n = x.priority() as u32,
        T::Ack => {
            m.n = x.ack_of().unwrap_or(0) as u32;
            m.q = x.seq().unwrap_or(0);
        }
        _ => {}
    }
    Some(m)
}

fn timer_name(t: Timer) -> &'static str {
    match t {
        Timer::T1 => "T1",
        Timer::T2 => "T2",
        Timer::T3 => "T3",
        Timer::T7 => "T7",
        Timer::T20 => "T20",
        Timer::T8 => "T8",
        Timer::T9 => "T9",
        Timer::T21 => "T21",
    }
}

fn g_name(g: G) -> &'static str {
    match g {
        G::Idle => "IDLE",
        G::Taken => "TAKEN",
        G::PRevoke => "PREVOKE",
    }
}

impl Floor {
    pub fn new(room: &str, epoch: &str, q_max: usize, t2_secs: u32) -> Arc<Floor> {
        Arc::new(Floor {
            room_id: room.to_string(),
            epoch: epoch.to_string(),
            st: Mutex::new(St::default()),
            wake: tokio::sync::Notify::new(),
            last_rtp_at: AtomicU64::new(0),
            first_rtp_at: AtomicU64::new(0),
            holder: ArcSwapOption::empty(),
            noted: Mutex::new(BTreeMap::new()),
            q_max,
            t2_secs,
            started: AtomicBool::new(false),
            dead: AtomicBool::new(false),
        })
    }

    pub fn holder(&self) -> Option<String> {
        self.holder.load_full().map(|s| s.as_ref().clone())
    }

    fn cfg(&self, st: &St, now: u64) -> Cfg {
        let first = self.first_rtp_at.load(Ordering::Acquire);
        let used = if st.srv.first_rtp && first > 0 { now.saturating_sub(first) / 1000 } else { 0 };
        Cfg { q_max: self.q_max, t2_secs: self.t2_secs, t2_remain_secs: (self.t2_secs as u64).saturating_sub(used) as u32 }
    }

    fn t2_ms(&self) -> u64 {
        self.t2_secs as u64 * 1000
    }

    pub fn apply<F>(self: &Arc<Self>, cause: &str, f: F)
    where
        F: FnOnce(&mut Step<'_>),
    {
        let acts = self.decide(cause, f);
        self.exec(acts);
    }

    fn decide<F>(self: &Arc<Self>, cause: &str, f: F) -> Vec<Act>
    where
        F: FnOnce(&mut Step<'_>),
    {
        let now = now_ms();
        let mut g = lock(&self.st);
        let cfg = self.cfg(&g, now);
        let from = g.srv.g;
        let r = {
            let mut s = Step::new(&mut g.srv, &cfg);
            f(&mut s);
            s.r
        };
        g.fseq += 1;
        let fseq = g.fseq;
        let to = g.srv.g;
        for l in &r.l {
            tracing::info!(kind = "ev", spec = %l, room = %self.room_id, epoch = %self.epoch, floor_sequence = fseq, area = "FLR", key = %self.room_id, from = g_name(from), to = g_name(to), cause = %cause, "floor");
        }
        for (t, u) in &r.starts {
            if *t == Timer::T1 {
                g.t1_base = now;
                g.granted_at = now;
                self.first_rtp_at.store(0, Ordering::Release);
                self.last_rtp_at.store(0, Ordering::Release);
            }
            if *t == Timer::T2 {
                let _ = self.first_rtp_at.compare_exchange(0, now, Ordering::AcqRel, Ordering::Acquire);
            }
            g.due.insert((*t, u.clone()), now + ms_of(*t, self.t2_ms()));
        }
        self.rearm(&mut g, now);
        let holder = g.srv.holder();
        self.holder.store(holder.map(Arc::new));
        let succ = g.srv.succ && from != G::Idle;
        drop(g);
        self.wake.notify_one();
        let mut acts = Vec::new();
        for Out { to, m } in r.o {
            tracing::info!(kind = "ev", spec = %format!("CP-MSG-{}", w::name(code(m.t)).replace('_', "-")), room = %self.room_id, user = %to, epoch = %self.epoch, floor_sequence = fseq, mbcp_seq = m.q, cause = m.n, "mbcp out");
            if let Some(b) = wire_out(&self.room_id, &m) {
                acts.push(Act::Send(to, b));
            }
        }
        let prime = !succ;
        for fx in r.fx {
            match fx {
                Fx::Bind(u) => acts.push(Act::Bind(u, prime)),
                Fx::Release(p) => acts.push(Act::Release(p)),
            }
        }
        acts
    }

    fn rearm(&self, g: &mut St, now: u64) {
        let mut on: Vec<(Timer, Option<String>)> = armed_server(&g.srv).into_iter().map(|t| (t, None)).collect();
        let users: Vec<String> = g.srv.part.keys().chain(g.srv.retry.keys()).cloned().collect();
        for u in users {
            for t in armed_participant(&g.srv, &u) {
                on.push((t, Some(u.clone())));
            }
        }
        g.due.retain(|k, _| on.contains(k));
        for k in on {
            let d = now + ms_of(k.0, self.t2_ms());
            g.due.entry(k).or_insert(d);
        }
    }

    fn exec(self: &Arc<Self>, acts: Vec<Act>) {
        for a in acts {
            match a {
                Act::Send(u, b) => crate::aux::floor_send(&self.room_id, &u, b),
                Act::Bind(u, prime) => crate::fwd::shell::floor_bind(&self.room_id, &u, prime),
                Act::Release(p) => crate::fwd::shell::floor_release(&self.room_id, p.as_deref()),
            }
        }
    }

    pub fn on_rtp(self: &Arc<Self>, user: &str, passed: bool) {
        let now = now_ms();
        if passed {
            self.last_rtp_at.store(now, Ordering::Release);
        }
        let first = passed && self.first_rtp_at.load(Ordering::Acquire) == 0;
        {
            let mut n = lock(&self.noted);
            let last = n.get(user).copied().unwrap_or(0);
            if !first && now.saturating_sub(last) < RTP_NOTE_MS {
                return;
            }
            n.insert(user.to_string(), now);
        }
        let u = user.to_string();
        self.apply("RTP", move |s| s.on_rtp(&u));
    }

    pub fn spawn(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move { me.run().await });
    }

    async fn run(self: Arc<Self>) {
        loop {
            if self.dead.load(Ordering::Acquire) {
                return;
            }
            let next = lock(&self.st).due.values().min().copied();
            let now = now_ms();
            match next {
                Some(at) if at <= now => self.fire(now),
                Some(at) => {
                    let _ = tokio::time::timeout(Duration::from_millis(at - now), self.wake.notified()).await;
                }
                None => {
                    let _ = tokio::time::timeout(Duration::from_secs(5), self.wake.notified()).await;
                }
            }
        }
    }

    fn fire(self: &Arc<Self>, now: u64) {
        let expired: Vec<(Timer, Option<String>)> = {
            let mut g = lock(&self.st);
            let ks: Vec<(Timer, Option<String>)> = g.due.iter().filter(|(_, at)| **at <= now).map(|(k, _)| k.clone()).collect();
            let mut out = Vec::new();
            for k in ks {
                if k.0 == Timer::T1 {
                    let eff = g.t1_base.max(self.last_rtp_at.load(Ordering::Acquire)) + ms_of(Timer::T1, 0);
                    if eff > now {
                        g.due.insert(k, eff);
                        continue;
                    }
                }
                g.due.remove(&k);
                out.push(k);
            }
            out
        };
        for (t, u) in expired {
            let name = timer_name(t);
            let cause = format!("CP-TIMER-{name}");
            match (t, u) {
                (Timer::T1, _) => self.apply(&cause, |s| s.srv_t1()),
                (Timer::T2, _) => self.apply(&cause, |s| s.srv_t2()),
                (Timer::T3, _) => self.apply(&cause, |s| s.srv_t3()),
                (Timer::T7, _) => self.apply(&cause, |s| s.srv_t7()),
                (Timer::T20, _) => self.apply(&cause, |s| s.srv_t20()),
                (Timer::T8, Some(u)) => self.apply(&cause, move |s| s.srv_t8(&u)),
                (Timer::T9, Some(u)) => self.apply(&cause, move |s| s.srv_t9(&u)),
                (Timer::T21, Some(u)) => self.apply(&cause, move |s| s.srv_t21(&u)),
                _ => {}
            }
        }
    }

    pub fn state_name(&self) -> &'static str {
        match lock(&self.st).srv.g {
            G::Idle => "Idle",
            G::Taken => "Taken",
            G::PRevoke => "PendingRevoke",
        }
    }

    pub fn queued(&self, user: &str) -> bool {
        lock(&self.st).srv.queue.iter().any(|w| w.u == user)
    }

    pub fn speaker_kinds_ok(&self) -> bool {
        lock(&self.st).srv.g != G::Idle
    }

    pub fn snapshot(&self) -> Value {
        let g = lock(&self.st);
        let speaker = g.srv.holder();
        let prio = if g.srv.g == G::Taken { g.srv.sprio } else { g.srv.rprio };
        let queue: Vec<Value> = g.srv.queue.iter().map(|w| json!({"user_id": w.u, "priority": w.p})).collect();
        let lr = self.last_rtp_at.load(Ordering::Acquire);
        let fr = self.first_rtp_at.load(Ordering::Acquire);
        let timers: Vec<Value> = g
            .due
            .iter()
            .map(|((t, u), at)| {
                let at = if *t == Timer::T1 { g.t1_base.max(lr) + ms_of(Timer::T1, 0) } else { *at };
                match u {
                    Some(u) => json!({"name": timer_name(*t), "user_id": u, "at": at}),
                    None => json!({"name": timer_name(*t), "at": at}),
                }
            })
            .collect();
        json!({
            "state": match g.srv.g { G::Idle => "Idle", G::Taken => "Taken", G::PRevoke => "PendingRevoke" },
            "speaker": speaker,
            "priority": speaker.as_ref().map(|_| prio),
            "queue": queue,
            "since": if speaker.is_some() { json!(g.granted_at) } else { Value::Null },
            "first_rtp_at": if fr > 0 { json!(fr) } else { Value::Null },
            "last_rtp_at": if lr > 0 { json!(lr) } else { Value::Null },
            "timers": timers,
        })
    }
}
