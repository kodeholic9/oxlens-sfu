// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-FWD-MEDIA-WORKERS · 정:RULE-CONC-PERF-AXES · 정:RULE-OPS-DROPS · 정:IMPL-RTCP-TERMINATE · 정:IMPL-FLR-MEDIA-BIND · model: claude-opus-5-5
//! 미디어 워커 — 수신 스레드 하나 · 미디어 워커 N(전송 객체마다 하나 고정 — 발행 · 구독을 따로 센다). 링은 워커마다 생산자 하나(워커마다 · 수신 스레드) + 제어 하나. 업링크 몫이 고른 줄은 목적지 워커마다 묶어 넘긴다. 키프레임 요청 · SR 번역 · 예열 프레임처럼 남의 전송 객체 상태를 쓰는 일도 그 워커로 넘긴다. 주기 RTCP · probe · 자동 층은 워커마다 제 전송 객체만 tick 으로 돈다. 링이 차면 기다리지 않고 버리고 센다. 일이 없으면 잠듦 표시를 세우고 잔다 — 넣는 쪽은 그 표시가 있을 때만 깨운다. 수신 스레드 · 워커가 끝나면 프로세스를 내린다.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread::Thread;
use std::time::Duration;

use bytes::Bytes;
use common::policy::MediaAutolayer;
use crossbeam_queue::ArrayQueue;

use super::hot::Downlink;
use crate::link::Transport;
use crate::link::io::Class;
use crate::ops::drops::{DROPS, bump};
use crate::r#pub::Uplink;

pub const READER_RING: usize = 4_096;
pub const WORKER_RING: usize = 1_024;
pub const CONTROL_RING: usize = 1_024;
pub const SLOT_BYTES: usize = 2_048;
const BATCH: usize = 64;
const SPIN: u32 = 256;
const PARK_MS: u64 = 50;
pub const EXIT_CODE: i32 = 70;

pub struct Out {
    pub dls: Vec<Arc<Downlink>>,
    pub up: Arc<Uplink>,
    pub buf: Bytes,
    pub keyframe: bool,
    pub level: Option<u8>,
    pub now_ms: u64,
}

pub enum Item {
    In { t: Arc<Transport>, class: Class, data: Vec<u8> },
    Out(Box<Out>),
    Key { up: Arc<Uplink>, now_ms: u64, infra: bool },
    Synth { dl: Arc<Downlink>, key: u64, seq: u16, ts: u32, payload: &'static [u8], now_ms: u64 },
    Sr { dls: Vec<Arc<Downlink>>, up: Arc<Uplink>, ntp: u64, rtp_ts: u32 },
}

#[derive(Clone, Copy)]
pub struct TickCfg {
    pub max_bitrate_bps: u64,
    pub auto_layer: MediaAutolayer,
}

struct Worker {
    rings: Vec<ArrayQueue<Item>>,
    sleeping: AtomicBool,
    thread: OnceLock<Thread>,
    pubs: AtomicUsize,
    subs: AtomicUsize,
    own: Mutex<Vec<Weak<Transport>>>,
}

pub struct Workers {
    w: Vec<Worker>,
    tick: TickCfg,
}

static WORKERS: OnceLock<Workers> = OnceLock::new();

thread_local! {
    static ME: Cell<usize> = const { Cell::new(usize::MAX) };
}

fn workers() -> Option<&'static Workers> {
    WORKERS.get()
}

pub fn count() -> usize {
    workers().map(|w| w.w.len()).unwrap_or(0)
}

fn producer(n: usize) -> usize {
    let me = ME.with(Cell::get);
    if me <= n { me } else { n + 1 }
}

pub fn current() -> Option<usize> {
    let me = ME.with(Cell::get);
    if me < count() { Some(me) } else { None }
}

fn here(idx: usize) -> bool {
    let n = count();
    n == 0 || idx >= n || current() == Some(idx)
}

fn counter(w: &Worker, subscribes: bool) -> &AtomicUsize {
    if subscribes { &w.subs } else { &w.pubs }
}

pub fn adopt(t: &Arc<Transport>) {
    let Some(ws) = workers() else { return };
    let subscribes = t.role.subscribes();
    let Some((i, w)) = ws.w.iter().enumerate().min_by_key(|(_, w)| counter(w, subscribes).load(Ordering::Relaxed)) else { return };
    counter(w, subscribes).fetch_add(1, Ordering::Relaxed);
    t.worker.store(i, Ordering::Relaxed);
    if let Ok(mut own) = w.own.lock() {
        own.push(Arc::downgrade(t));
    }
}

pub fn forsake(t: &Transport) {
    let Some(w) = workers().and_then(|ws| ws.w.get(t.worker.load(Ordering::Relaxed))) else { return };
    let _ = counter(w, t.role.subscribes()).fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1));
    if let Ok(mut own) = w.own.lock() {
        own.retain(|x| x.upgrade().is_some_and(|o| o.id != t.id));
    }
}

fn wake(w: &Worker) {
    if w.sleeping.load(Ordering::SeqCst)
        && let Some(t) = w.thread.get()
    {
        t.unpark();
    }
}

fn push(idx: usize, item: Item) -> bool {
    let Some(ws) = workers() else { return false };
    let Some(w) = ws.w.get(idx) else { return false };
    let p = producer(ws.w.len());
    let ok = w.rings[p].push(item).is_ok();
    wake(w);
    ok
}

fn owner(t: &Transport) -> usize {
    t.worker.load(Ordering::Relaxed)
}

pub fn submit_in(t: &Arc<Transport>, class: Class, data: Vec<u8>) {
    if data.len() > SLOT_BYTES {
        bump(&DROPS.oversize);
        return;
    }
    let idx = owner(t);
    if workers().is_none() {
        super::rx::dispatch(t, class, &data);
        return;
    }
    if !push(idx, Item::In { t: t.clone(), class, data }) {
        bump(&DROPS.ingress_queue_full);
    }
}

pub fn keyframe(up: &Arc<Uplink>, now_ms: u64, infra: bool) {
    let Some(t) = up.stream.upgrade().and_then(|s| s.transport.upgrade()) else { return };
    let idx = owner(&t);
    if here(idx) {
        crate::rtcp::request_keyframe(up, now_ms, infra);
        return;
    }
    if !push(idx, Item::Key { up: up.clone(), now_ms, infra }) {
        bump(&DROPS.egress_queue_full);
    }
}

pub fn synthetic(dl: &Arc<Downlink>, key: u64, seq: u16, ts: u32, payload: &'static [u8], now_ms: u64) {
    let Some(t) = dl.transport.upgrade() else {
        bump(&DROPS.transport_gone);
        return;
    };
    let idx = owner(&t);
    if here(idx) {
        super::egress::synthetic(dl, key, seq, ts, payload, now_ms);
        return;
    }
    if !push(idx, Item::Synth { dl: dl.clone(), key, seq, ts, payload, now_ms }) {
        bump(&DROPS.egress_queue_full);
    }
}

pub fn relay_sr(up: &Arc<Uplink>, targets: Vec<Arc<Downlink>>, ntp: u64, rtp_ts: u32) {
    let n = count();
    let mut per: Vec<Vec<Arc<Downlink>>> = Vec::new();
    for dl in targets {
        let Some(t) = dl.transport.upgrade() else { continue };
        let idx = owner(&t);
        if here(idx) {
            super::egress::sr_to(&dl, up, ntp, rtp_ts);
            continue;
        }
        if per.is_empty() {
            per = vec![Vec::new(); n];
        }
        per[idx].push(dl);
    }
    for (idx, dls) in per.into_iter().enumerate() {
        if !dls.is_empty() && !push(idx, Item::Sr { dls, up: up.clone(), ntp, rtp_ts }) {
            bump(&DROPS.egress_queue_full);
        }
    }
}

pub struct Fanout<'a> {
    r: &'a super::rtp::Rtp<'a>,
    buf: &'a Bytes,
    up: &'a Arc<Uplink>,
    keyframe: bool,
    level: Option<u8>,
    now_ms: u64,
    per: Vec<Vec<Arc<Downlink>>>,
}

impl<'a> Fanout<'a> {
    pub fn new(r: &'a super::rtp::Rtp<'a>, buf: &'a Bytes, up: &'a Arc<Uplink>, keyframe: bool, level: Option<u8>, now_ms: u64) -> Fanout<'a> {
        Fanout { r, buf, up, keyframe, level, now_ms, per: Vec::new() }
    }

    pub fn add(&mut self, dl: &Arc<Downlink>) {
        let Some(t) = dl.transport.upgrade() else {
            bump(&DROPS.transport_gone);
            return;
        };
        let idx = owner(&t);
        if here(idx) {
            super::egress::forward(dl, self.r, self.up, self.keyframe, self.level, self.now_ms);
            return;
        }
        if self.per.is_empty() {
            self.per = vec![Vec::new(); count()];
        }
        self.per[idx].push(dl.clone());
    }

    pub fn flush(self) {
        for (idx, dls) in self.per.into_iter().enumerate() {
            if dls.is_empty() {
                continue;
            }
            let lost = dls.len() as u64;
            let out = Out { dls, up: self.up.clone(), buf: self.buf.clone(), keyframe: self.keyframe, level: self.level, now_ms: self.now_ms };
            if !push(idx, Item::Out(Box::new(out))) {
                DROPS.egress_queue_full.fetch_add(lost, Ordering::Relaxed);
            }
        }
    }
}

fn alive(dl: &Downlink) -> bool {
    if dl.transport.strong_count() == 0 {
        bump(&DROPS.transport_gone);
        return false;
    }
    true
}

fn handle(item: Item) {
    match item {
        Item::In { t, class, data } => {
            if t.closed.load(Ordering::Acquire) {
                bump(&DROPS.transport_gone);
                return;
            }
            super::rx::dispatch(&t, class, &data);
        }
        Item::Out(o) => {
            if let Some(r) = super::rtp::parse(&o.buf) {
                for dl in o.dls.iter().filter(|d| alive(d)) {
                    super::egress::forward(dl, &r, &o.up, o.keyframe, o.level, o.now_ms);
                }
            }
        }
        Item::Key { up, now_ms, infra } => {
            crate::rtcp::request_keyframe(&up, now_ms, infra);
        }
        Item::Synth { dl, key, seq, ts, payload, now_ms } => {
            if alive(&dl) {
                super::egress::synthetic(&dl, key, seq, ts, payload, now_ms);
            }
        }
        Item::Sr { dls, up, ntp, rtp_ts } => {
            for dl in dls.iter().filter(|d| alive(d)) {
                super::egress::sr_to(dl, &up, ntp, rtp_ts);
            }
        }
    }
}

fn drain(w: &Worker) -> usize {
    let mut did = 0;
    for ring in &w.rings {
        for _ in 0..BATCH {
            match ring.pop() {
                Some(it) => {
                    handle(it);
                    did += 1;
                }
                None => break,
            }
        }
    }
    did
}

fn empty(w: &Worker) -> bool {
    w.rings.iter().all(ArrayQueue::is_empty)
}

fn own(w: &Worker) -> Vec<Arc<Transport>> {
    w.own.lock().map(|o| o.iter().filter_map(Weak::upgrade).collect()).unwrap_or_default()
}

fn tick(w: &Worker, cfg: TickCfg, n: u64, now: u64, sim: bool) {
    let twcc = n.is_multiple_of(crate::rtcp::TWCC_MS / crate::rtcp::NACK_POLL_MS);
    let rr = n.is_multiple_of(crate::rtcp::RR_MS / crate::rtcp::NACK_POLL_MS);
    let keepalive = n.is_multiple_of(crate::link::tcp::KEEPALIVE_MS / crate::rtcp::NACK_POLL_MS);
    for t in own(w) {
        if keepalive
            && let Some(io) = t.io.upgrade()
        {
            for c in t.pairs.lock().map(|p| p.tcp_conns()).unwrap_or_default() {
                io.keepalive(c);
            }
        }
        crate::rtcp::send_nacks(&t, now);
        if twcc {
            crate::rtcp::send_twcc(&t);
        }
        if rr {
            crate::rtcp::send_rr_remb(&t, common::time::mono_us(), cfg.max_bitrate_bps);
        }
        crate::sim::probe_chunk(&t, now);
        if sim {
            crate::sim::tick(&t, now, cfg.auto_layer);
        }
    }
}

fn run(i: usize) {
    let Some(ws) = workers() else { return };
    let w = &ws.w[i];
    let start = common::time::mono_ms();
    let mut next = start + crate::rtcp::NACK_POLL_MS;
    let mut next_sim = start + crate::sim::core::TICK_MS;
    let mut n: u64 = 0;
    loop {
        let now = common::time::mono_ms();
        if now >= next {
            n += 1;
            let sim = now >= next_sim;
            if sim {
                next_sim = now + crate::sim::core::TICK_MS;
            }
            tick(w, ws.tick, n, now, sim);
            next = now + crate::rtcp::NACK_POLL_MS;
        }
        if drain(w) > 0 {
            continue;
        }
        let mut spun = 0;
        while spun < SPIN && empty(w) {
            std::hint::spin_loop();
            spun += 1;
        }
        if !empty(w) {
            continue;
        }
        w.sleeping.store(true, Ordering::SeqCst);
        if empty(w) {
            let wait = next.saturating_sub(common::time::mono_ms()).clamp(1, PARK_MS);
            std::thread::park_timeout(Duration::from_millis(wait));
        }
        w.sleeping.store(false, Ordering::SeqCst);
    }
}

fn fatal(name: &str, r: std::thread::Result<()>) -> ! {
    match r {
        Ok(()) => tracing::error!(thread = name, "미디어 스레드가 끝났다 — 프로세스를 내린다"),
        Err(_) => tracing::error!(thread = name, "미디어 스레드 panic — 프로세스를 내린다"),
    }
    std::thread::sleep(Duration::from_millis(100));
    std::process::exit(EXIT_CODE);
}

pub fn start(n: usize, tick: TickCfg, rt: tokio::runtime::Handle) -> Result<(), String> {
    let n = n.max(1);
    let mut w = Vec::with_capacity(n);
    for _ in 0..n {
        let mut rings = Vec::with_capacity(n + 2);
        for _ in 0..n {
            rings.push(ArrayQueue::new(WORKER_RING));
        }
        rings.push(ArrayQueue::new(READER_RING));
        rings.push(ArrayQueue::new(CONTROL_RING));
        w.push(Worker { rings, sleeping: AtomicBool::new(false), thread: OnceLock::new(), pubs: AtomicUsize::new(0), subs: AtomicUsize::new(0), own: Mutex::new(Vec::new()) });
    }
    WORKERS.set(Workers { w, tick }).map_err(|_| "미디어 워커를 두 번 띄웠다".to_string())?;
    for i in 0..n {
        let rt = rt.clone();
        let name = format!("media-{i}");
        let h = std::thread::Builder::new()
            .name(name.clone())
            .spawn(move || {
                ME.with(|m| m.set(i));
                let _g = rt.enter();
                let r = std::panic::catch_unwind(|| run(i));
                fatal(&name, r);
            })
            .map_err(|e| format!("미디어 워커 {i}: {e}"))?;
        if let Some(ws) = workers() {
            let _ = ws.w[i].thread.set(h.thread().clone());
        }
    }
    Ok(())
}

pub fn spawn_reader(reader: crate::link::io::Reader, rt: tokio::runtime::Handle) -> Result<(), String> {
    let n = count();
    std::thread::Builder::new()
        .name("media-rx".into())
        .spawn(move || {
            ME.with(|m| m.set(n));
            let _g = rt.enter();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || reader.run()));
            fatal("media-rx", r);
        })
        .map(|_| ())
        .map_err(|e| format!("수신 스레드: {e}"))
}
