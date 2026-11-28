// author: kodeholic (powered by Claude)
// spec: v1.14 · 정:IMPL-OPS-REC-UNIT · 정:IMPL-SES-TOKEN-ISSUE · 연:RULE-SES-BACKOFF-LADDER · 연:RULE-SES-EVICT-OLD · SDK:RULE-PLAT-REC-API · model: claude-opus-5-5
//! 방 작업 — 방마다 하나가 파이프라인 · 결과물 소비자 · 원시 기록기 · 세션(Client)의 주인(수명 = 그 방 버스 토큰의 한 차례 — 차례 번호가 바뀌면 새 작업) · 방 세션 신원 `rec-{node}-{room}`(BIND 마다 방 세션 토큰 · 토큰 받기는 레코드 소비를 막지 않는다) · 다시 붙기는 같은 세션 손잡이로 사다리 · 소비자 하나의 실패는 그 소비자만(같은 backoff · intensity 로 다시 짓고 넘으면 그 종류만 멈춤) · 방 작업의 패닉 · 되돌릴 수 없는 실패는 그 방만(창 안 상한을 넘으면 격리 — 방 토큰이 다시 설 때 풀림) · 작업이 끝날 때마다 그 실행 폴더를 회수 · 같은 방의 새 방 작업은 옛 작업이 끝난 뒤 · 쓰는 중 폴더 오류는 프로세스 전체의 실패.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use common::talk::Report;
use oxpeer::record::{Output, Pipeline, Policy, RecordConfig, Recorder};
use oxpeer::{Client, Config, Event};
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::consumer::{RecConsumer, RecoverCtx, Registry, RoomCtx, SPEC};
use crate::files::{self, Layout};
use crate::{Hub, Restart};

pub struct Env {
    pub layout: Layout,
    pub hub: Hub,
    pub hub_base: String,
    pub ca: Option<Vec<u8>>,
    pub token: watch::Receiver<String>,
    pub reports: mpsc::UnboundedSender<Report>,
    pub registry: Arc<Registry>,
    pub restart: Restart,
    pub fatal: mpsc::UnboundedSender<(&'static str, i32)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Presence {
    pub up: bool,
    pub turn: u64,
}

impl Presence {
    pub fn live(&self, turn: u64) -> bool {
        self.up && self.turn == turn
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum End {
    Gone,
    Empty,
    Failed(String),
}

pub fn rung(attempt: u32) -> Duration {
    let w = oxpeer::SESSION_LADDER.waits;
    let i = (attempt as usize).min(w.len().saturating_sub(1));
    Duration::from_millis(w.get(i).copied().unwrap_or(0).max(0) as u64)
}

pub fn restart_after(hist: &mut VecDeque<Instant>, now: Instant, r: &Restart, attempt: usize) -> Option<Duration> {
    hist.retain(|t| now.duration_since(*t) < r.intensity_window);
    hist.push_back(now);
    if hist.len() as u64 > u64::from(r.intensity_max) {
        return None;
    }
    let i = attempt.min(r.backoff_ms.len().saturating_sub(1));
    Some(Duration::from_millis(r.backoff_ms.get(i).copied().unwrap_or(0)))
}

pub fn kinds_of(detail: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in detail.get("record").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        if !k.is_empty() && !out.iter().any(|x| x == k) {
            out.push(k.to_string());
        }
    }
    out
}

pub fn claim_run(layout: &Layout, now: SystemTime) -> std::io::Result<(String, Option<std::fs::File>)> {
    std::fs::create_dir_all(layout.node_staging())?;
    let base: u128 = files::run_stamp(now).parse().unwrap_or(0);
    for i in 0..1_000u128 {
        let run = (base + i).to_string();
        match std::fs::create_dir(layout.run_dir(&run)) {
            Ok(()) => match files::lock_run(&layout.run_dir(&run)) {
                Ok(Some(lock)) => return Ok((run, Some(lock))),
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(kind = "diag", spec = SPEC, path = %layout.run_dir(&run).display(), "rec run folder lock not supported here: {e} — written without a lock");
                    return Ok((run, None));
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free run folder"))
}

async fn wait_or_gone(d: Duration, want: &mut watch::Receiver<Presence>, turn: u64) -> bool {
    if !want.borrow().live(turn) {
        return true;
    }
    let sleep = tokio::time::sleep(d);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            _ = &mut sleep => return !want.borrow().live(turn),
            r = want.changed() => if r.is_err() || !want.borrow().live(turn) { return true },
        }
    }
}

async fn park(want: &mut watch::Receiver<Presence>, turn: u64) -> bool {
    while want.borrow().live(turn) {
        if want.changed().await.is_err() {
            return false;
        }
    }
    true
}

async fn session_token(env: &Env, room: &str, want: &mut watch::Receiver<Presence>, turn: u64) -> Option<String> {
    let mut attempt = 0u32;
    loop {
        let node = env.token.borrow().clone();
        match crate::request_json(&env.hub, "POST", "/rec_unit/session_token", &node, Some(&serde_json::json!({ "room_id": room }))).await {
            Some((200, body)) => {
                if let Some(t) = body.get("token").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                    return Some(t.to_string());
                }
                tracing::warn!(kind = "diag", spec = SPEC, room = %room, "room session token answer without a token");
            }
            Some((status, _)) => tracing::warn!(kind = "diag", spec = SPEC, room = %room, status, "room session token refused by the local gateway"),
            None => tracing::warn!(kind = "diag", spec = SPEC, room = %room, "room session token not read from the local gateway"),
        }
        if wait_or_gone(rung(attempt.max(1)), want, turn).await {
            return None;
        }
        attempt += 1;
    }
}

async fn room_detail(env: &Env, room: &str, want: &mut watch::Receiver<Presence>, turn: u64) -> Option<Vec<String>> {
    let mut attempt = 0u32;
    loop {
        let node = env.token.borrow().clone();
        match crate::get_json(&env.hub, &format!("/rooms/{room}"), &node).await {
            Some((200, body)) if body.is_object() => return Some(kinds_of(&body)),
            Some((200, _)) => tracing::warn!(kind = "diag", spec = SPEC, room = %room, "room detail body not read — asked again"),
            Some((status, _)) => tracing::warn!(kind = "diag", spec = SPEC, room = %room, status, "room detail refused by the local gateway — asked again"),
            None => tracing::warn!(kind = "diag", spec = SPEC, room = %room, "room detail not read from the local gateway — asked again"),
        }
        if wait_or_gone(rung(attempt.max(1)), want, turn).await {
            return None;
        }
        attempt += 1;
    }
}

pub struct Slot {
    pub name: String,
    c: Option<Box<dyn RecConsumer>>,
    hist: VecDeque<Instant>,
    attempt: usize,
    retry_at: Option<Instant>,
}

impl Slot {
    pub fn new(name: impl Into<String>, c: Box<dyn RecConsumer>) -> Slot {
        Slot { name: name.into(), c: Some(c), hist: VecDeque::new(), attempt: 0, retry_at: None }
    }
}

pub struct Consumers {
    room: String,
    slots: Vec<Slot>,
    restart: Restart,
}

impl Consumers {
    pub fn new(room: impl Into<String>, restart: Restart, slots: Vec<Slot>) -> Consumers {
        Consumers { room: room.into(), slots, restart }
    }

    pub fn live(&self) -> Vec<String> {
        self.slots.iter().filter(|s| s.c.is_some()).map(|s| s.name.clone()).collect()
    }

    pub fn waiting(&self) -> Vec<String> {
        self.slots.iter().filter(|s| s.c.is_none() && s.retry_at.is_some()).map(|s| s.name.clone()).collect()
    }

    pub fn each(&mut self, mut f: impl FnMut(&mut dyn RecConsumer) -> Result<(), String>) {
        for s in &mut self.slots {
            let Some(c) = s.c.as_mut() else { continue };
            let r = match catch_unwind(AssertUnwindSafe(|| f(c.as_mut()))) {
                Ok(r) => r,
                Err(p) => Err(panic_text(&p)),
            };
            if let Err(e) = r {
                fail_slot(&self.room, s, &self.restart, &e, Instant::now());
            }
        }
    }

    pub fn revive(&mut self, registry: &Registry, ctx: &RoomCtx, now: Instant) {
        for s in &mut self.slots {
            if s.c.is_some() || s.retry_at.is_none_or(|t| t > now) {
                continue;
            }
            s.retry_at = None;
            let made = match catch_unwind(AssertUnwindSafe(|| registry.open(&s.name, ctx))) {
                Ok(Some(Ok(c))) => Ok(c),
                Ok(Some(Err(e))) => Err(e.to_string()),
                Ok(None) => continue,
                Err(p) => Err(panic_text(&p)),
            };
            match made {
                Ok(c) => {
                    tracing::info!(kind = "diag", spec = SPEC, room = %self.room, rec_kind = %s.name, "rec kind reopened");
                    s.c = Some(c);
                }
                Err(e) => fail_slot(&self.room, s, &self.restart, &e, now),
            }
        }
    }

    pub fn requests(&mut self) -> Vec<String> {
        let mut v = Vec::new();
        self.each(|c| {
            v.extend(c.keyframe_requests());
            Ok(())
        });
        v
    }

    pub fn finish(&mut self, reason: &str) {
        for s in &mut self.slots {
            let Some(mut c) = s.c.take() else { continue };
            s.retry_at = None;
            match catch_unwind(AssertUnwindSafe(|| c.finish(reason))) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::warn!(kind = "diag", spec = SPEC, room = %self.room, rec_kind = %s.name, reason = %reason, "rec kind not closed cleanly: {e} — left for recovery"),
                Err(p) => tracing::warn!(kind = "diag", spec = SPEC, room = %self.room, rec_kind = %s.name, reason = %reason, "rec kind panicked while closing: {} — left for recovery", panic_text(&p)),
            }
        }
    }
}

fn fail_slot(room: &str, s: &mut Slot, r: &Restart, e: &str, now: Instant) {
    if let Some(mut c) = s.c.take() {
        let _ = catch_unwind(AssertUnwindSafe(|| c.finish("error")));
    }
    match restart_after(&mut s.hist, now, r, s.attempt) {
        Some(wait) => {
            s.attempt += 1;
            s.retry_at = Some(now + wait);
            tracing::warn!(kind = "diag", spec = SPEC, room = %room, rec_kind = %s.name, reason = "error", "rec kind failed: {e} — reopened after backoff");
        }
        None => {
            s.retry_at = None;
            tracing::warn!(kind = "diag", spec = SPEC, room = %room, rec_kind = %s.name, reason = "error", "rec kind stopped: {e} — over the restart intensity");
        }
    }
}

pub fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into())
}

fn dispatch(cs: &mut Consumers, outs: Vec<Output>) {
    for o in outs {
        match &o {
            Output::Frame(f) => cs.each(|c| c.frame(f)),
            Output::Segment(s) => cs.each(|c| c.segment(s)),
            Output::Orphan(_) => {}
        }
    }
}

pub async fn run_job(env: Arc<Env>, room: String, run: String, mut want: watch::Receiver<Presence>, turn: u64) -> End {
    let Some(kinds) = room_detail(&env, &room, &mut want, turn).await else { return End::Gone };
    if kinds.is_empty() {
        return End::Empty;
    }
    if !files::room_name_ok(&room, &run) {
        tracing::warn!(kind = "diag", spec = SPEC, room = %room, "room id cannot name recording files — the room is not recorded");
        return End::Empty;
    }
    let ctx = RoomCtx { room: room.clone(), run: run.clone(), layout: env.layout.clone(), reports: env.reports.clone() };
    let mut slots = Vec::new();
    for k in &kinds {
        match env.registry.open(k, &ctx) {
            Some(Ok(c)) => slots.push(Slot::new(k.clone(), c)),
            Some(Err(e)) => return End::Failed(format!("{k} open: {e}")),
            None => tracing::warn!(kind = "diag", spec = SPEC, room = %room, rec_kind = %k, "rec kind not registered in this unit — skipped"),
        }
    }
    if slots.is_empty() {
        return End::Empty;
    }
    let mut cs = Consumers::new(room.clone(), env.restart.clone(), slots);
    let raw: Option<PathBuf> = cs.slots.iter().filter_map(|s| s.c.as_ref()).find_map(|c| c.raw_dir());
    let mut rc = match &raw {
        Some(d) => RecordConfig::new(d, &room),
        None => RecordConfig::live(&room),
    };
    rc.run.clone_from(&run);
    let recorder = match Recorder::open(rc) {
        Ok(r) => r,
        Err(e) => return End::Failed(format!("recorder: {e}")),
    };
    let gone = |cs: &mut Consumers, recorder: &Recorder| {
        cs.finish("end");
        recorder.close();
        End::Gone
    };
    let Some(token) = session_token(&env, &room, &mut want, turn).await else { return gone(&mut cs, &recorder) };
    let mut cfg = Config::new(&env.hub_base, &token);
    cfg.tls_pin_pem.clone_from(&env.ca);
    cfg.record = Some(recorder.clone());
    let mut attempt = 0u32;
    let (client, mut events, mut media, _frames) = loop {
        match Client::open(cfg.clone()).await {
            Ok(x) => break x,
            Err(e) => {
                tracing::warn!(kind = "diag", spec = SPEC, room = %room, "rec room session not opened: {e}");
                if wait_or_gone(rung(attempt.max(1)), &mut want, turn).await {
                    return gone(&mut cs, &recorder);
                }
                attempt += 1;
                match session_token(&env, &room, &mut want, turn).await {
                    Some(t) => cfg.token = t,
                    None => return gone(&mut cs, &recorder),
                }
            }
        }
    };
    let mut records = recorder.records(1 << 16);
    let mut pipe = Pipeline::new(Policy::Utterance);
    let (fresh_tx, mut fresh_rx) = mpsc::unbounded_channel::<String>();
    let mut asking: Option<tokio::task::JoinHandle<()>> = None;
    let mut joined = join(&client, &room, &mut want, turn).await;
    let end = loop {
        if !joined {
            break End::Gone;
        }
        tokio::select! {
            r = want.changed() => if r.is_err() || !want.borrow().live(turn) { break End::Gone },
            r = records.next() => {
                let Some(r) = r else { break End::Failed("records closed".into()) };
                cs.each(|c| c.record(&r));
                dispatch(&mut cs, pipe.push(&r));
                cs.revive(&env.registry, &ctx, Instant::now());
                for t in cs.requests() {
                    let _ = client.request_keyframe(t);
                }
            }
            Some(t) = fresh_rx.recv() => {
                asking = None;
                if let Err(e) = client.set_token(t) {
                    tracing::warn!(kind = "diag", spec = SPEC, room = %room, "room session token not given to the session: {e}");
                }
            }
            e = events.next() => match e {
                None => break End::Failed("session events closed".into()),
                Some(Event::TokenRequired) => {
                    if asking.is_none() {
                        let (env2, room2, mut want2, tx) = (env.clone(), room.clone(), want.clone(), fresh_tx.clone());
                        asking = Some(tokio::spawn(async move {
                            if let Some(t) = session_token(&env2, &room2, &mut want2, turn).await {
                                let _ = tx.send(t);
                            }
                        }));
                    }
                }
                Some(Event::Closed { code, name, .. }) => {
                    tracing::warn!(kind = "diag", spec = SPEC, room = %room, code, name = %name, "rec room session closed — reattaching");
                    joined = reattach(&env, &client, &room, &mut want, turn).await;
                }
                Some(Event::Left { room: r }) | Some(Event::Forced { room: r, .. }) | Some(Event::JoinFailed { room: r, .. }) if r == room => {
                    joined = rejoin(&client, &room, &mut want, turn).await;
                }
                Some(Event::RecordStopped { reason }) => {
                    let dir = raw.as_ref().map(|d| crate::consumer::rel(&env.layout, d)).unwrap_or_default();
                    if reason == "name_taken" {
                        tracing::warn!(kind = "diag", spec = SPEC, room = %room, rec_kind = "raw", path = %dir, reason = %reason, "rec part refused");
                    } else {
                        tracing::warn!(kind = "diag", spec = SPEC, room = %room, rec_kind = "raw", path = %dir, reason = %reason, "raw recording stopped");
                    }
                }
                Some(Event::RecordConsumerStopped { reason }) => break End::Failed(format!("record stream stopped: {reason}")),
                Some(_) => {}
            },
            p = media.next() => if p.is_none() { break End::Failed("session media closed".into()) },
        }
    };
    if let Some(h) = asking {
        h.abort();
    }
    if end == End::Gone {
        dispatch(&mut cs, pipe.finish());
        cs.finish("end");
        recorder.close();
        client.close().await;
    }
    end
}

async fn join(client: &Client, room: &str, want: &mut watch::Receiver<Presence>, turn: u64) -> bool {
    let mut attempt = 0u32;
    loop {
        match client.join(room).await {
            Ok(()) => return true,
            Err(e) => tracing::warn!(kind = "diag", spec = SPEC, room = %room, "rec join failed: {e}"),
        }
        if wait_or_gone(rung(attempt.max(1)), want, turn).await {
            return false;
        }
        attempt += 1;
    }
}

async fn rejoin(client: &Client, room: &str, want: &mut watch::Receiver<Presence>, turn: u64) -> bool {
    if wait_or_gone(rung(1), want, turn).await {
        return false;
    }
    join(client, room, want, turn).await
}

async fn reattach(env: &Env, client: &Client, room: &str, want: &mut watch::Receiver<Presence>, turn: u64) -> bool {
    let mut attempt = 0u32;
    loop {
        if wait_or_gone(rung(attempt), want, turn).await {
            return false;
        }
        attempt += 1;
        let Some(t) = session_token(env, room, want, turn).await else { return false };
        if let Err(e) = client.set_token(t) {
            tracing::warn!(kind = "diag", spec = SPEC, room = %room, "room session token not given to the session: {e}");
        }
        match client.connect().await {
            Ok(()) => return join(client, room, want, turn).await,
            Err(e) => tracing::warn!(kind = "diag", spec = SPEC, room = %room, "rec room session reconnect failed: {e}"),
        }
    }
}

pub fn recover_run(env: &Env, run: &str, reason: &'static str) {
    let ctx = RecoverCtx { layout: env.layout.clone(), runs: vec![env.layout.run_dir(run)], reports: env.reports.clone(), reason };
    env.registry.recover(&ctx);
}

pub async fn room_actor(env: Arc<Env>, room: String, mut want: watch::Receiver<Presence>) {
    let mut hist: VecDeque<Instant> = VecDeque::new();
    let mut attempt = 0usize;
    let mut last_turn = 0u64;
    loop {
        let p = *want.borrow_and_update();
        if !p.up {
            if want.changed().await.is_err() {
                return;
            }
            continue;
        }
        if p.turn != last_turn {
            hist.clear();
            attempt = 0;
            last_turn = p.turn;
        }
        let turn = p.turn;
        let (run, lock) = match claim_run(&env.layout, SystemTime::now()) {
            Ok(x) => x,
            Err(e) => {
                tracing::error!(kind = "diag", spec = SPEC, room = %room, "rec run folder: {e} — the unit stops");
                let _ = env.fatal.send(("write folder error", 1));
                return;
            }
        };
        tracing::info!(kind = "diag", spec = SPEC, room = %room, run = %run, "rec room job opened");
        let end = tokio::spawn(run_job(env.clone(), room.clone(), run.clone(), want.clone(), turn)).await;
        drop(lock);
        let why = match end {
            Ok(End::Gone) => {
                tracing::info!(kind = "diag", spec = SPEC, room = %room, run = %run, "rec room job ended");
                recover_run(&env, &run, "crash");
                continue;
            }
            Ok(End::Empty) => {
                recover_run(&env, &run, "crash");
                if !park(&mut want, turn).await {
                    return;
                }
                continue;
            }
            Ok(End::Failed(e)) => e,
            Err(j) => match j.try_into_panic() {
                Ok(p) => format!("panic: {}", panic_text(&p)),
                Err(e) => e.to_string(),
            },
        };
        match restart_after(&mut hist, Instant::now(), &env.restart, attempt) {
            Some(wait) => {
                attempt += 1;
                tracing::warn!(kind = "diag", spec = SPEC, room = %room, run = %run, reason = "crash", "rec room job failed: {why} — reopened after backoff");
                recover_run(&env, &run, "crash");
                wait_or_gone(wait, &mut want, turn).await;
            }
            None => {
                recover_run(&env, &run, "quarantine");
                tracing::warn!(kind = "diag", spec = SPEC, room = %room, run = %run, reason = "quarantine", "rec quarantined");
                if !park(&mut want, turn).await {
                    return;
                }
            }
        }
    }
}
