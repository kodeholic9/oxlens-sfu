// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-PUB-ACCEPT · 정:IMPL-SUB-SUBSCRIBER-TABLE · model: claude-opus-5-5
//! 1층 제어 층 — 선언 PT 와 다른 PT 로 RTP 가 들어오면 서버는 그 어긋남을 진단 줄 `[PT:MISMATCH]` 로 트랙당 한 번만 드러내고 고치지 않는다: 발행 선언 PT 와 구독 줄의 PT 는 그대로이고, 같은 트랙에 패킷이 더 와도 줄은 하나뿐이며, 선언대로 오는 트랙은 줄 0(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use common::envelope::CallKind;
use oxsfud::fwd::Rtp;
use oxsfud::link::ExtIds;
use oxsfud::ops;
use oxsfud::r#pub::PubTrack;
use oxsfud::room::Room;
use oxsig::msg::{Duplex, Kind};
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata};

use support::*;

#[derive(Default)]
struct Lines {
    next: AtomicU64,
    got: Mutex<Vec<(String, String)>>,
}

#[derive(Default)]
struct Seen {
    kind: String,
    spec: String,
    all: String,
}

impl Visit for Seen {
    fn record_str(&mut self, f: &Field, v: &str) {
        self.put(f.name(), v.to_string());
    }
    fn record_debug(&mut self, f: &Field, v: &dyn Debug) {
        self.put(f.name(), format!("{v:?}"));
    }
    fn record_u64(&mut self, f: &Field, v: u64) {
        self.put(f.name(), v.to_string());
    }
}

impl Seen {
    fn put(&mut self, name: &str, v: String) {
        match name {
            "kind" => self.kind = v,
            "spec" => self.spec = v,
            _ => self.all.push_str(&format!("{name}={v} ")),
        }
    }
}

impl tracing::Subscriber for Lines {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(self.next.fetch_add(1, Ordering::Relaxed) + 1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, e: &Event<'_>) {
        let mut seen = Seen::default();
        e.record(&mut seen);
        if seen.kind == "diag"
            && let Ok(mut g) = self.got.lock()
        {
            g.push((seen.spec, seen.all));
        }
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

fn mismatch_lines(f: impl FnOnce()) -> Vec<String> {
    let lines = Arc::new(Lines::default());
    tracing::subscriber::with_default(lines.clone(), f);
    lines.got.lock().map(|g| g.iter().filter(|(s, a)| s == "SV-IMPL-PUB-ACCEPT" && a.contains("[PT:MISMATCH]")).map(|(_, a)| a.clone()).collect()).unwrap_or_default()
}

fn feed(t: &PubTrack, pt: u8, n: u16) {
    for i in 0..n {
        let seq = 100 + i;
        let ts = 3_000 * i as u32;
        let mut raw = vec![0x80, pt & 0x7f];
        raw.extend_from_slice(&seq.to_be_bytes());
        raw.extend_from_slice(&ts.to_be_bytes());
        raw.extend_from_slice(&t.ssrc.to_be_bytes());
        raw.extend_from_slice(&[0u8; 20]);
        let rtp = Rtp { raw: &raw, pt, marker: false, seq, ts, ssrc: t.ssrc, hlen: 12, ext: None };
        t.on_ingress(&rtp, 1_000 + i as u64 * 33, 0, &ExtIds::default());
    }
}

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    assert!(drive(ops::rooms::create(&e)).is_ok(), "stand check — create {id}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

#[test]
fn a_pt_mismatch_is_surfaced_once_per_track_and_not_healed() {
    let _g = serial();
    let r = create(&tag("ptm"));
    let (a, b) = (subscriber(&tag("ptm-a")), subscriber(&tag("ptm-b")));
    enter(&a.p, &r);
    enter(&b.p, &r);
    let (v, vt) = published(&a.p, &r.id, Kind::Video, Duplex::Full);
    list(&v, true);
    ops::media::announce_add(&v);
    let row = row_of(&b.t, Kind::Video, &v.track_id);
    let (declared, sub_pt) = (v.pt, row.pt.load(Ordering::Acquire));
    let other = if declared == 120 { 121 } else { 120 };
    let got = mismatch_lines(|| feed(&vt, other, 30));
    assert_eq!(got.len(), 1, "S-sym-IMPL-PUB-ACCEPT-1 a track whose RTP arrives with another PT leaves exactly one [PT:MISMATCH] line across 30 packets: {got:?}");
    assert!(
        got[0].contains(&format!("in.declared={declared} ")) && got[0].contains(&format!("in.first={other} ")),
        "S-sym-IMPL-PUB-ACCEPT-1 the line names the declared PT and the first RTP PT: {got:?}"
    );
    assert_eq!(
        (v.pt, row.pt.load(Ordering::Acquire), row_of(&b.t, Kind::Video, &v.track_id).pt.load(Ordering::Acquire)),
        (declared, sub_pt, sub_pt),
        "S-sym-IMPL-PUB-ACCEPT-1 nothing is healed — the declared PT and the subscriber row's PT stay as they were"
    );
    let again = mismatch_lines(|| feed(&vt, other, 5));
    assert!(again.is_empty(), "S-sym-IMPL-PUB-ACCEPT-1 the latch holds for the track's life — no second line: {again:?}");
    let (w, wt) = published(&a.p, &r.id, Kind::Audio, Duplex::Full);
    let fine = mismatch_lines(|| feed(&wt, w.pt, 10));
    assert!(fine.is_empty(), "S-sym-IMPL-PUB-ACCEPT-1 a track that sends its declared PT leaves no line (control): {fine:?}");
}
