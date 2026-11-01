// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-MESH-SESSION · 정:RULE-MESH-FENCE · 정:RULE-MESH-LANE · model: claude-opus-5-5
//! 세션 하나 = TCP 연결 하나 — 레인 넷의 송신 큐(제어 · 선언은 무한 · 요청은 차면 거절 · 데이터는 차면 버리고 센다) · 쓰기 태스크(높은 레인부터 · 놀면 keepalive) · 읽기 태스크(틱 공백 울타리 뒤에만 lease 갱신).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Notify, mpsc};

use crate::wire::{Assembled, CloseReason, FRAME_MAX, LANES, Lane, Msg, Reassembler};

pub const DATA_CAP: usize = 8192;
pub const REQ_CAP: usize = 4096;
const WRITE_BATCH: usize = 64 * 1024;

pub struct Clock {
    start: Instant,
    last_tick: AtomicU64,
}

impl Clock {
    pub fn new() -> Clock {
        Clock { start: Instant::now(), last_tick: AtomicU64::new(0) }
    }

    pub fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn tick(&self) -> u64 {
        let now = self.now_ms();
        let prev = self.last_tick.swap(now, Ordering::AcqRel);
        now.saturating_sub(prev)
    }

    pub fn gap_ms(&self) -> u64 {
        self.now_ms().saturating_sub(self.last_tick.load(Ordering::Acquire))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Push {
    Queued,
    Dropped,
    Full,
    Closed,
}

struct Lanes {
    q: [VecDeque<Vec<u8>>; LANES],
    closing: Option<Option<Vec<u8>>>,
}

pub struct Outbox {
    lanes: Mutex<Lanes>,
    notify: Notify,
    pub dropped: AtomicU64,
    pub full: AtomicU64,
    total_dropped: Arc<AtomicU64>,
}

enum Pop {
    Frames(Vec<u8>),
    Empty,
    Close(Option<Vec<u8>>),
}

impl Outbox {
    pub fn new(total_dropped: Arc<AtomicU64>) -> Outbox {
        Outbox {
            lanes: Mutex::new(Lanes { q: Default::default(), closing: None }),
            notify: Notify::new(),
            dropped: AtomicU64::new(0),
            full: AtomicU64::new(0),
            total_dropped,
        }
    }

    pub fn push(&self, m: &Msg) -> Push {
        let (lane, frames) = m.frames();
        let Ok(mut g) = self.lanes.lock() else { return Push::Closed };
        if g.closing.is_some() {
            return Push::Closed;
        }
        let q = &mut g.q[lane as usize];
        let cap = match lane {
            Lane::Data => Some(DATA_CAP),
            Lane::Request => Some(REQ_CAP),
            _ => None,
        };
        if let Some(c) = cap
            && q.len() + frames.len() > c
        {
            drop(g);
            if lane == Lane::Data {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                self.total_dropped.fetch_add(1, Ordering::Relaxed);
                return Push::Dropped;
            }
            self.full.fetch_add(1, Ordering::Relaxed);
            return Push::Full;
        }
        q.extend(frames);
        drop(g);
        self.notify.notify_one();
        Push::Queued
    }

    pub fn close(&self, reason: Option<CloseReason>) {
        if let Ok(mut g) = self.lanes.lock()
            && g.closing.is_none()
        {
            g.closing = Some(reason.map(|r| Msg::Close(r).frames().1.concat()));
        }
        self.notify.notify_one();
    }

    pub fn depths(&self) -> [usize; LANES] {
        self.lanes.lock().map(|g| [g.q[0].len(), g.q[1].len(), g.q[2].len(), g.q[3].len()]).unwrap_or_default()
    }

    fn pop(&self) -> Pop {
        let Ok(mut g) = self.lanes.lock() else { return Pop::Close(None) };
        if let Some(c) = g.closing.take() {
            g.closing = Some(None);
            for q in g.q.iter_mut() {
                q.clear();
            }
            return Pop::Close(c);
        }
        let mut out = Vec::new();
        'fill: while out.len() < WRITE_BATCH {
            for q in g.q.iter_mut() {
                if let Some(f) = q.pop_front() {
                    out.extend_from_slice(&f);
                    continue 'fill;
                }
            }
            break;
        }
        if out.is_empty() { Pop::Empty } else { Pop::Frames(out) }
    }
}

pub enum SessEv {
    Msg(u64, Msg),
    Closed(u64, CloseReason),
    Paused,
}

pub async fn write_loop(mut w: OwnedWriteHalf, ob: Arc<Outbox>, ka: Duration) {
    let keepalive = Msg::KeepAlive.frames().1.concat();
    loop {
        match ob.pop() {
            Pop::Frames(f) => {
                if w.write_all(&f).await.is_err() {
                    return;
                }
            }
            Pop::Close(f) => {
                if let Some(f) = f {
                    let _ = w.write_all(&f).await;
                }
                let _ = w.shutdown().await;
                return;
            }
            Pop::Empty => {
                tokio::select! {
                    _ = ob.notify.notified() => {}
                    _ = tokio::time::sleep(ka) => {
                        if w.write_all(&keepalive).await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }
}

pub struct ReadCtx {
    pub uid: u64,
    pub tx: mpsc::UnboundedSender<SessEv>,
    pub clock: Arc<Clock>,
    pub last_rx: Arc<AtomicU64>,
    pub fence_ms: u64,
    pub stop: Arc<AtomicBool>,
}

pub async fn read_frame(r: &mut (impl AsyncReadExt + Unpin)) -> Result<(Lane, u8, Vec<u8>), CloseReason> {
    let mut head = [0u8; 4];
    r.read_exact(&mut head).await.map_err(|_| CloseReason::Io)?;
    let n = u32::from_be_bytes(head) as usize;
    if !(2..=FRAME_MAX - 4).contains(&n) {
        return Err(CloseReason::Protocol);
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body).await.map_err(|_| CloseReason::Io)?;
    let lane = Lane::from_u8(body[0]).ok_or(CloseReason::Protocol)?;
    let flags = body[1];
    body.drain(..2);
    Ok((lane, flags, body))
}

pub async fn read_loop(r: OwnedReadHalf, cx: ReadCtx) {
    let mut rd = BufReader::new(r);
    let mut asm = Reassembler::default();
    let why = loop {
        let (lane, flags, chunk) = match read_frame(&mut rd).await {
            Ok(f) => f,
            Err(e) => break e,
        };
        if cx.stop.load(Ordering::Acquire) {
            return;
        }
        if cx.clock.gap_ms() > cx.fence_ms {
            let _ = cx.tx.send(SessEv::Paused);
            return;
        }
        cx.last_rx.store(cx.clock.now_ms(), Ordering::Release);
        match asm.push(lane, flags, &chunk) {
            Assembled::Partial => {}
            Assembled::TooBig => break CloseReason::Protocol,
            Assembled::Done(b) => match Msg::decode(&b) {
                Some(Msg::KeepAlive) => {}
                Some(m) => {
                    if cx.tx.send(SessEv::Msg(cx.uid, m)).is_err() {
                        return;
                    }
                }
                None => break CloseReason::Protocol,
            },
        }
    };
    if !cx.stop.load(Ordering::Acquire) {
        let _ = cx.tx.send(SessEv::Closed(cx.uid, why));
    }
}

pub async fn send_direct(s: &mut TcpStream, m: &Msg) -> std::io::Result<()> {
    s.write_all(&m.frames().1.concat()).await
}

pub async fn recv_direct(s: &mut TcpStream, timeout: Duration) -> Option<Msg> {
    let (lane, flags, chunk) = tokio::time::timeout(timeout, read_frame(s)).await.ok()?.ok()?;
    if lane != Lane::Control || flags != 0 {
        return None;
    }
    Msg::decode(&chunk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_lane_full_drops_and_counts_into_the_process_total() {
        let total = Arc::new(AtomicU64::new(0));
        let a = Outbox::new(total.clone());
        let b = Outbox::new(total.clone());
        let put = Msg::Put { key: "ox/t/int/ev/room/r1".into(), payload: vec![0; 8] };
        for _ in 0..DATA_CAP {
            assert_eq!(a.push(&put), Push::Queued);
        }
        assert_eq!(a.push(&put), Push::Dropped);
        assert_eq!(b.push(&put), Push::Queued);
        let req = Msg::Request { qid: 1, expr: "ox/t/int/q/node/n1/call".into(), payload: Vec::new(), timeout_ms: 10 };
        for _ in 0..REQ_CAP {
            assert_eq!(a.push(&req), Push::Queued);
        }
        assert_eq!(a.push(&req), Push::Full);
        assert_eq!(a.dropped.load(Ordering::Relaxed), 1);
        assert_eq!(a.full.load(Ordering::Relaxed), 1);
        assert_eq!(total.load(Ordering::Relaxed), 1);
    }
}
