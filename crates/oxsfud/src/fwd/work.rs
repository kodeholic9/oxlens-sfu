// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-FWD-MEDIA-WORKERS · 정:RULE-CONC-PERF-AXES · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! 미디어 워커 — 수신 스레드 하나 · 미디어 워커 N(전송 객체마다 하나 고정) · 링은 생산자 하나(수신 스레드 · 워커마다) + 제어 하나. 업링크 몫이 고른 줄은 그 구독 전송 객체의 워커로 넘긴다. 링이 차면 기다리지 않고 버리고 센다. 일이 없으면 잠듦 표시를 세우고 잔다 — 넣는 쪽은 그 표시가 있을 때만 깨운다. 수신 스레드 · 워커가 끝나면 프로세스를 내린다.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::Thread;
use std::time::Duration;

use bytes::Bytes;
use crossbeam_queue::ArrayQueue;

use super::hot::Downlink;
use crate::link::Transport;
use crate::link::io::Class;
use crate::ops::drops::{DROPS, bump};
use crate::r#pub::Uplink;

pub const READER_RING: usize = 4_096;
pub const WORKER_RING: usize = 1_024;
pub const CONTROL_RING: usize = 1_024;
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
}

struct Worker {
    rings: Vec<ArrayQueue<Item>>,
    sleeping: AtomicBool,
    thread: OnceLock<Thread>,
    pubs: AtomicUsize,
    subs: AtomicUsize,
}

pub struct Workers {
    w: Vec<Worker>,
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

fn counter(w: &Worker, subscribes: bool) -> &AtomicUsize {
    if subscribes { &w.subs } else { &w.pubs }
}

pub fn assign(subscribes: bool) -> usize {
    let Some(ws) = workers() else { return 0 };
    let Some((i, w)) = ws.w.iter().enumerate().min_by_key(|(_, w)| counter(w, subscribes).load(Ordering::Relaxed)) else { return 0 };
    counter(w, subscribes).fetch_add(1, Ordering::Relaxed);
    i
}

pub fn release(i: usize, subscribes: bool) {
    if let Some(w) = workers().and_then(|ws| ws.w.get(i)) {
        let _ = counter(w, subscribes).fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1));
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

pub fn submit_in(t: &Arc<Transport>, class: Class, data: Vec<u8>) {
    if workers().is_none() {
        super::rx::dispatch(t, class, &data);
        return;
    }
    let idx = t.worker.load(Ordering::Relaxed);
    if !push(idx, Item::In { t: t.clone(), class, data }) {
        bump(&DROPS.ingress_queue_full);
    }
}

pub struct Fanout<'a> {
    r: &'a super::rtp::Rtp<'a>,
    buf: &'a Bytes,
    up: &'a Arc<Uplink>,
    keyframe: bool,
    level: Option<u8>,
    now_ms: u64,
    me: Option<usize>,
    per: Vec<Vec<Arc<Downlink>>>,
}

impl<'a> Fanout<'a> {
    pub fn new(r: &'a super::rtp::Rtp<'a>, buf: &'a Bytes, up: &'a Arc<Uplink>, keyframe: bool, level: Option<u8>, now_ms: u64) -> Fanout<'a> {
        Fanout { r, buf, up, keyframe, level, now_ms, me: current(), per: Vec::new() }
    }

    pub fn add(&mut self, dl: &Arc<Downlink>) {
        let Some(t) = dl.transport.upgrade() else { return };
        let idx = t.worker.load(Ordering::Relaxed);
        let n = count();
        if n == 0 || self.me == Some(idx) || idx >= n {
            super::egress::forward(dl, self.r, self.up, self.keyframe, self.level, self.now_ms);
            return;
        }
        if self.per.is_empty() {
            self.per = vec![Vec::new(); n];
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

fn handle(item: Item) {
    match item {
        Item::In { t, class, data } => super::rx::dispatch(&t, class, &data),
        Item::Out(o) => {
            if let Some(r) = super::rtp::parse(&o.buf) {
                for dl in &o.dls {
                    super::egress::forward(dl, &r, &o.up, o.keyframe, o.level, o.now_ms);
                }
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

fn run(i: usize) {
    let Some(ws) = workers() else { return };
    let w = &ws.w[i];
    loop {
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
            std::thread::park_timeout(Duration::from_millis(PARK_MS));
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

pub fn start(n: usize, rt: tokio::runtime::Handle) -> Result<(), String> {
    let n = n.max(1);
    let mut w = Vec::with_capacity(n);
    for _ in 0..n {
        let mut rings = Vec::with_capacity(n + 2);
        for _ in 0..n {
            rings.push(ArrayQueue::new(WORKER_RING));
        }
        rings.push(ArrayQueue::new(READER_RING));
        rings.push(ArrayQueue::new(CONTROL_RING));
        w.push(Worker { rings, sleeping: AtomicBool::new(false), thread: OnceLock::new(), pubs: AtomicUsize::new(0), subs: AtomicUsize::new(0) });
    }
    WORKERS.set(Workers { w }).map_err(|_| "미디어 워커를 두 번 띄웠다".to_string())?;
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

pub fn spawn_reader(io: Arc<crate::link::Io>, rt: tokio::runtime::Handle) -> Result<(), String> {
    let n = count();
    std::thread::Builder::new()
        .name("media-rx".into())
        .spawn(move || {
            ME.with(|m| m.set(n));
            let _g = rt.enter();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| super::rx::udp_loop(&io)));
            fatal("media-rx", r);
        })
        .map(|_| ())
        .map_err(|e| format!("수신 스레드: {e}"))
}
