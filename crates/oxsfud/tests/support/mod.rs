// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SUB-PIPELINE · 정:IMPL-SUB-TABLE · 정:IMPL-GC-PEER-REAP · model: claude-opus-5-5
//! 1층 제어 층 받침 — `ops::install` 로 미디어 없이 서버 상태 하나를 세우고, 공개 입구로 방 · Peer · 배정표 · 발행 스트림을 짓는다. 줄 목록과 표가 같은지 보는 불변식(`breaks`) · 제품 소스를 줄로 읽는 감시 도우미를 함께 둔다. 시험 파일마다 따로 짓는 바이너리라 설치는 파일마다 한 번이다.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use common::envelope::{CallKind, Claims, Envelope, Home};
use oxsfud::flr::core::Floor;
use oxsfud::fwd::DownRow;
use oxsfud::link::{Peer, Role, Transport};
use oxsfud::ops::handle::Ctx;
use oxsfud::ops::{self, Sfu};
use oxsfud::r#pub::{PubStream, PubTrack, StreamInit};
use oxsfud::room::{Member, PERMANENT, Room, RoomInit};
use oxsfud::sub::{SyncOut, Table};
use oxsig::msg::{Duplex, Kind, MediaCodecs, PcMode, Permission};

pub const WAIT: Duration = Duration::from_millis(1_500);

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static SERIAL: Mutex<()> = Mutex::new(());
static NEXT: AtomicU64 = AtomicU64::new(1);

pub fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn sfu() -> &'static Arc<Sfu> {
    sfu_with(common::policy::Policy::default())
}

pub fn sfu_with(policy: common::policy::Policy) -> &'static Arc<Sfu> {
    if let Some(s) = ops::sfu() {
        return s;
    }
    let rt = RT.get_or_init(|| tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime"));
    let (cert, fingerprint) = oxsfud::link::dtls::certificate().expect("certificate");
    ops::install(Sfu {
        node: "l1".into(),
        epoch: "l1-epoch".into(),
        build: "l1".into(),
        policy,
        ip: "127.0.0.1".parse().expect("ip"),
        udp_port: 0,
        tcp_port: None,
        cert,
        fingerprint,
        node_grace_ms: 0,
        peers: Default::default(),
        rooms: Default::default(),
        pubs: Default::default(),
        tables: Default::default(),
        tracks: Default::default(),
        next_id: AtomicU64::new(1),
        drops: ops::DROP_NAMES.iter().map(|_| AtomicU64::new(0)).collect(),
        codes: Default::default(),
        rt: rt.handle().clone(),
    });
    ops::sfu().expect("installed")
}

pub fn drive<F: std::future::Future>(f: F) -> F::Output {
    sfu();
    RT.get().expect("runtime").block_on(f)
}

pub fn tag(name: &str) -> String {
    format!("{name}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

pub fn room(id: &str) -> Arc<Room> {
    room_with(id, 1_000)
}

pub fn room_with(id: &str, capacity: u32) -> Arc<Room> {
    let s = sfu();
    let (r, fresh) = s.rooms.insert(Room::new(RoomInit {
        id: id.into(),
        name: id.into(),
        capacity,
        unused_ttl: PERMANENT,
        departure_ttl: PERMANENT,
        media_codecs: MediaCodecs::default(),
        epoch: s.epoch.clone(),
        floor: Floor::new(10, 30),
        record: Vec::new(),
    }));
    assert!(fresh, "stand check — room {id} is new in the room set");
    r
}

pub struct Sub {
    pub p: Arc<Peer>,
    pub t: Arc<Table>,
}

pub fn subscriber(user: &str) -> Sub {
    subscriber_as(user, &format!("sid-{user}"))
}

pub fn subscriber_as(user: &str, sid: &str) -> Sub {
    subscriber_slots(user, sid, sfu().policy.media.slot_mid_reserve as usize)
}

pub fn subscriber_slots(user: &str, sid: &str, slot_budget: usize) -> Sub {
    let s = sfu();
    let session = format!("ses-{user}");
    let publish = Transport::new(s.next_id(), user, &session, Role::Publish, 0, false);
    let subscribe = Transport::new(s.next_id(), user, &session, Role::Subscribe, 0, false);
    let t = Table::new(subscribe.clone(), PcMode::Two, slot_budget);
    t.mark_seeded();
    s.tables.insert(t.clone());
    let p = Arc::new(Peer {
        user: user.into(),
        sid: sid.into(),
        session,
        home: Mutex::new(Home { node: s.node.clone(), inst: String::new() }),
        pc_mode: PcMode::Two,
        claims: Claims::default(),
        publish,
        subscribe,
        rooms: Mutex::new(BTreeMap::new()),
        created_at: common::now_ms(),
        ever_sent_join: AtomicBool::new(false),
    });
    s.peers.lock().expect("peers").insert(user.into(), p.clone());
    Sub { p, t }
}

pub fn member(user: &str) -> Member {
    Member { user: user.into(), role: Some(255), select: true, participant_type: 0, hidden: false, metadata: None, permission: Permission::default() }
}

pub fn enter(p: &Arc<Peer>, room: &Room) -> SyncOut {
    p.rooms.lock().expect("rooms").insert(room.id.clone(), format!("j-{}", room.id));
    let out = ops::media::resync_peer(p, false);
    room.join(member(&p.user), None).expect("registered");
    out
}

pub fn ctx_of(p: &Peer) -> Ctx {
    Ctx { user: p.user.clone(), sid: p.sid.clone(), session: p.session.clone(), pc_mode: p.pc_mode, source_tag: false, home: None, claims: None, over: None }
}

pub fn envelope(kind: CallKind, user: Option<&str>, sid: Option<&str>) -> Envelope {
    let mut e = Envelope::new(kind);
    e.user_id = user.map(|x| x.to_string());
    e.sid = sid.map(|x| x.to_string());
    e
}

pub fn stream(user: &str, room: &str, kind: Kind) -> Arc<PubStream> {
    stream_as(user, room, kind, Duplex::Full)
}

pub fn stream_as(user: &str, room: &str, kind: Kind, duplex: Duplex) -> Arc<PubStream> {
    let s = sfu();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let audio = kind == Kind::Audio;
    let st = PubStream::new(StreamInit {
        track_id: format!("t-{user}-{n}"),
        user: user.into(),
        session: format!("ses-{user}"),
        kind,
        mid: n.to_string(),
        pt: if audio { 111 } else { 102 },
        rtx_pt: if audio { None } else { Some(103) },
        codec: Some(if audio { "opus" } else { "H264" }.into()),
        fmtp: if audio { None } else { Some("level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".into()) },
        source: None,
        simulcast: false,
        duplex,
        room: room.into(),
        ssrc: 0x1000_0000 + n as u32 * 2,
        rtx_ssrc: if audio { None } else { Some(0x1000_0001 + n as u32 * 2) },
    });
    s.tracks.lock().expect("tracks").insert(st.track_id.clone(), st.clone());
    st
}

pub fn published(p: &Peer, room: &str, kind: Kind, duplex: Duplex) -> (Arc<PubStream>, Arc<PubTrack>) {
    let s = sfu();
    let st = stream_as(&p.user, room, kind, duplex);
    if let Ok(mut g) = s.pubs.ctx(&p.user).streams.lock() {
        g.insert(st.track_id.clone(), st.clone());
    }
    let t = PubTrack::new(st.clone(), st.ssrc, st.rtx_ssrc, None, &p.publish);
    st.tracks.lock().expect("tracks").push(t.clone());
    (st, t)
}

pub fn voiced(st: &Arc<PubStream>, at: u64) {
    let s = sfu();
    let have = st.physical();
    let t = match have.first() {
        Some(t) => t.clone(),
        None => {
            let tr = Transport::new(s.next_id(), &st.user, &st.session, Role::Publish, 0, false);
            let t = PubTrack::new(st.clone(), st.ssrc, None, None, &tr);
            st.tracks.lock().expect("tracks").push(t.clone());
            t
        }
    };
    t.last_voice_ms.store(at, Ordering::Release);
}

pub fn list(st: &PubStream, on: bool) {
    st.listed.store(on, Ordering::Release);
}

pub fn row_of(t: &Table, kind: Kind, track: &str) -> Arc<DownRow> {
    t.loaded().into_iter().find(|(tid, _, d)| tid == track && d.kind == kind).map(|(_, _, d)| d).expect("row loaded with the track")
}

pub fn loads(o: &SyncOut, track: &str, room: &str) -> bool {
    o.changes.iter().any(|c| c.new.as_ref().is_some_and(|n| n.0 == track && n.1 == room))
}

pub fn unloads(o: &SyncOut, track: &str, room: &str) -> bool {
    o.changes.iter().any(|c| c.old.as_ref().is_some_and(|n| n.0 == track && n.1 == room) && c.new.is_none())
}

pub fn has(t: &Table, track: &str, room: &str, row: &Arc<DownRow>) -> bool {
    t.loaded().iter().any(|(tid, rid, d)| tid == track && rid == room && Arc::ptr_eq(d, row))
}

pub fn holds(t: &Table, track: &str, room: &str) -> bool {
    t.loaded().iter().any(|(tid, rid, _)| tid == track && rid == room)
}

pub fn remembers_unplaced(t: &Table, track: &str) -> bool {
    t.st.lock().map(|s| s.unplaced.contains(track)).unwrap_or(false)
}

pub fn wait_until(mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if f() {
            return true;
        }
        if start.elapsed() > WAIT {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub fn finished_within<T>(h: &JoinHandle<T>) -> bool {
    wait_until(|| h.is_finished())
}

pub fn resync_in_thread(p: &Arc<Peer>) -> JoinHandle<SyncOut> {
    let p = p.clone();
    std::thread::spawn(move || ops::media::resync_peer(&p, true))
}

pub fn breaks(sub: &Sub) -> Vec<String> {
    let s = sfu();
    let conn = sub.p.subscribe.id;
    let registered = s.tables.get(conn).is_some_and(|x| Arc::ptr_eq(&x, &sub.t));
    let loaded = if registered { sub.t.loaded() } else { Vec::new() };
    let streams: Vec<Arc<PubStream>> = s.tracks.lock().map(|g| g.values().cloned().collect()).unwrap_or_default();
    let mut lists: Vec<(String, String, Arc<DownRow>)> = Vec::new();
    for st in &streams {
        let rid = st.room.load().as_str().to_string();
        for r in st.rows.load().iter().filter(|r| r.conn.id == conn) {
            lists.push((st.track_id.clone(), rid.clone(), r.clone()));
        }
    }
    for rm in s.rooms.all() {
        for k in [Kind::Audio, Kind::Video] {
            for r in rm.slot(k).rows.load().iter().filter(|r| r.conn.id == conn) {
                lists.push((oxsfud::trk::slot_track_id(&rm.id, k), rm.id.clone(), r.clone()));
            }
        }
    }
    let mut out = Vec::new();
    for (tid, rid, r) in &lists {
        if !registered {
            out.push(format!("I3 row mid {} of a retired table is in the list of {tid}@{rid}", r.mid));
        } else if !loaded.iter().any(|(t, m, d)| t == tid && m == rid && Arc::ptr_eq(d, r)) {
            out.push(format!("I1 row mid {} is in the list of {tid}@{rid} but the table does not load it there", r.mid));
        }
    }
    for (tid, rid, r) in &loaded {
        let n = lists.iter().filter(|(t, m, d)| t == tid && m == rid && Arc::ptr_eq(d, r)).count();
        if n != 1 {
            out.push(format!("I2 table loads {tid}@{rid} on row mid {} but the list holds it {n} times", r.mid));
        }
        if !sub.p.in_room(rid) {
            out.push(format!("I4 table loads {tid}@{rid} on row mid {} but the peer is not in {rid}", r.mid));
        }
    }
    for st in &streams {
        let marked = st.unplaced_by.lock().map(|g| g.contains(&sub.p.user)).unwrap_or(false);
        let remembered = registered && remembers_unplaced(&sub.t, &st.track_id);
        if marked != remembered {
            out.push(format!("I5 unplaced mark of {} is {marked} but the table remembers {remembered}", st.track_id));
        }
    }
    out
}

pub struct Src {
    pub file: String,
    pub func: String,
    pub line: usize,
    pub text: String,
}

pub fn product_lines() -> Vec<Src> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|x| x.to_str()) != Some("rs") {
                continue;
            }
            let rel = p.strip_prefix(&root).map(|x| x.display().to_string()).unwrap_or_default();
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let mut func = String::new();
            for (n, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("#[cfg(test)]") {
                    break;
                }
                let t = line.trim_start();
                let indent = line.len() - t.len();
                if indent <= 4 {
                    for head in ["pub fn ", "pub(crate) fn ", "fn ", "pub async fn ", "pub(crate) async fn ", "async fn "] {
                        if let Some(rest) = t.strip_prefix(head) {
                            func = rest.split(['(', '<']).next().unwrap_or("").to_string();
                            break;
                        }
                    }
                }
                out.push(Src { file: rel.clone(), func: func.clone(), line: n + 1, text: line.to_string() });
            }
        }
    }
    out
}

pub fn at(l: &Src) -> String {
    format!("{}:{} ({})", l.file, l.line, l.func)
}
