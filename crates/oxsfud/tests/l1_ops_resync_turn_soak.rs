// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SUB-PIPELINE · 정:IMPL-SUB-TABLE · 정:RULE-CONC-SIX · model: claude-opus-5-5
//! 1층 제어 층 · 통계 — 스레드 넷이 한 배정표에 재배정을 겹쳐 부르며 사이사이 재료(나열 · 선호 · 말소리 순위 · 든 방)를 바꾼다. 가라앉힌 뒤 재배정 한 번을 더 돌려도 줄 목록 · 줄 밖 표지가 표와 같아야 한다 — 어긋남은 다음 재배정으로 낫지 않는다(바뀐 것만 집행). 결정적 재현은 l1_ops_resync_turn.rs 가 들고 이 파일은 따로 프로세스로 넓게 흔든다 — 재발 시험이 아니라 흔들기다(차례 없는 빌드에서도 초록일 수 있다). 판마다 기한을 두어 차례끼리 서로 기다리는 고리(교착)는 끝나지 않는 대신 빨강으로 드러난다.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use common::envelope::{Claims, Home};
use oxsfud::flr::core::Floor;
use oxsfud::fwd::DownRow;
use oxsfud::link::{Peer, Role, Transport};
use oxsfud::ops::{self, Sfu};
use oxsfud::r#pub::{PubStream, PubTrack, StreamInit};
use oxsfud::room::{Member, PERMANENT, Room, RoomInit};
use oxsfud::sub::Table;
use oxsig::msg::{Duplex, Kind, MediaCodecs, PcMode, Permission};

const ROUNDS: usize = 40;
const THREADS: u64 = 4;
const STEPS: usize = 300;
const ROUND_LIMIT: Duration = Duration::from_secs(30);

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);

fn sfu() -> &'static Arc<Sfu> {
    if let Some(s) = ops::sfu() {
        return s;
    }
    let rt = RT.get_or_init(|| tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime"));
    let (cert, fingerprint) = oxsfud::link::dtls::certificate().expect("certificate");
    ops::install(Sfu {
        node: "l1".into(),
        epoch: "l1-epoch".into(),
        build: "l1".into(),
        policy: common::policy::Policy::default(),
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
        slots: arc_swap::ArcSwap::from_pointee(Default::default()),
        next_id: AtomicU64::new(1),
        drops: ops::DROP_NAMES.iter().map(|_| AtomicU64::new(0)).collect(),
        codes: Default::default(),
        rt: rt.handle().clone(),
    });
    ops::sfu().expect("installed")
}

fn tag(name: &str) -> String {
    format!("{name}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn room(id: &str) -> Arc<Room> {
    let s = sfu();
    s.rooms.insert(Room::new(RoomInit {
        id: id.into(),
        name: id.into(),
        capacity: 1_000,
        unused_ttl: PERMANENT,
        departure_ttl: PERMANENT,
        media_codecs: MediaCodecs::default(),
        epoch: s.epoch.clone(),
        floor: Floor::new(10, 30),
    }))
}

struct Sub {
    p: Arc<Peer>,
    t: Arc<Table>,
}

fn subscriber(user: &str) -> Sub {
    let s = sfu();
    let session = format!("ses-{user}");
    let publish = Transport::new(s.next_id(), user, &session, Role::Publish, 0, false);
    let subscribe = Transport::new(s.next_id(), user, &session, Role::Subscribe, 0, false);
    let t = Table::new(subscribe.clone(), PcMode::Two, s.policy.media.slot_mid_reserve as usize);
    t.mark_seeded();
    s.tables.insert(t.clone());
    let p = Arc::new(Peer {
        user: user.into(),
        sid: format!("sid-{user}"),
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

fn enter(p: &Arc<Peer>, room: &Room) {
    p.rooms.lock().expect("rooms").insert(room.id.clone(), format!("j-{}", room.id));
    ops::media::resync_peer(p, false);
    room.join(
        Member { user: p.user.clone(), role: Some(255), select: true, participant_type: 0, hidden: false, metadata: None, permission: Permission::default() },
        false,
        None,
    );
}

fn stream(user: &str, room: &str, kind: Kind) -> Arc<PubStream> {
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
        fmtp: None,
        source: None,
        simulcast: false,
        duplex: Duplex::Full,
        room: room.into(),
        ssrc: 0x1000_0000 + n as u32 * 2,
        rtx_ssrc: if audio { None } else { Some(0x1000_0001 + n as u32 * 2) },
    });
    if audio {
        let tr = Transport::new(s.next_id(), user, &st.session, Role::Publish, 0, false);
        st.tracks.lock().expect("tracks").push(PubTrack::new(st.clone(), st.ssrc, None, None, &tr));
    }
    st.listed.store(true, Ordering::Release);
    s.tracks.lock().expect("tracks").insert(st.track_id.clone(), st.clone());
    st
}

fn breaks(sub: &Sub) -> Vec<String> {
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
        let remembered = registered && sub.t.st.lock().map(|g| g.unplaced.contains(&st.track_id)).unwrap_or(false);
        if marked != remembered {
            out.push(format!("I5 unplaced mark of {} is {marked} but the table remembers {remembered}", st.track_id));
        }
    }
    out
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

struct Stage {
    sub: Sub,
    y: Arc<Room>,
    audio: Vec<Arc<PubStream>>,
    video: Vec<Arc<PubStream>>,
}

fn stage(round: usize) -> Stage {
    let x = room(&tag(&format!("r4-{round}-x")));
    let y = room(&tag(&format!("r4-{round}-y")));
    let sub = subscriber(&tag(&format!("r4-{round}-p")));
    enter(&sub.p, &x);
    let mut audio: Vec<Arc<PubStream>> = (0..26).map(|_| stream(&tag("r4-ua"), &x.id, Kind::Audio)).collect();
    audio.push(stream(&tag("r4-uy"), &y.id, Kind::Audio));
    let video: Vec<Arc<PubStream>> = (0..3).map(|_| stream(&tag("r4-uv"), &x.id, Kind::Video)).collect();
    ops::media::resync_peer(&sub.p, true);
    Stage { sub, y, audio, video }
}

fn churn(st: &Stage, seed: u64) {
    let mut rng = Rng(seed);
    let now = common::now_ms();
    for _ in 0..STEPS {
        match rng.next() % 6 {
            0 => {
                let all: Vec<&Arc<PubStream>> = st.audio.iter().chain(st.video.iter()).collect();
                let s = all[(rng.next() % all.len() as u64) as usize];
                s.listed.fetch_xor(true, Ordering::AcqRel);
            }
            1 => {
                let v = &st.video[(rng.next() % st.video.len() as u64) as usize];
                st.sub.t.set_pref(&v.track_id, Some((rng.next() % 3) as u8), Some(rng.next().is_multiple_of(2)));
            }
            2 | 3 => {
                let a = &st.audio[(rng.next() % st.audio.len() as u64) as usize];
                if let Some(t) = a.physical().first() {
                    t.last_voice_ms.store(now.saturating_sub(rng.next() % 2_000), Ordering::Release);
                }
            }
            4 => {
                let mut g = st.sub.p.rooms.lock().expect("rooms");
                if g.remove(&st.y.id).is_none() {
                    g.insert(st.y.id.clone(), "j-y".into());
                }
            }
            _ => {}
        }
        ops::media::resync_peer(&st.sub.p, true);
    }
}

#[test]
fn r4_overlapping_resyncs_keep_lists_equal_to_the_table() {
    let start = Instant::now();
    let mut red = Vec::new();
    for round in 0..ROUNDS {
        let st = Arc::new(stage(round));
        let hs: Vec<_> = (0..THREADS)
            .map(|i| {
                let st = st.clone();
                std::thread::spawn(move || churn(&st, 0x9e37_79b9_7f4a_7c15 ^ (((round as u64) << 8) | (i + 1))))
            })
            .collect();
        let until = Instant::now() + ROUND_LIMIT;
        while hs.iter().any(|h| !h.is_finished()) {
            assert!(
                Instant::now() < until,
                "정:RULE-CONC-SIX round {round} did not finish within {ROUND_LIMIT:?} — overlapping resyncs of one table are stuck (a turn is waiting on a turn or on a lock taken inside one)"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        for h in hs {
            h.join().expect("churn");
        }
        ops::media::resync_peer(&st.sub.p, true);
        let b = breaks(&st.sub);
        if !b.is_empty() {
            red.push((round, b));
        }
        if let Ok(mut g) = sfu().tracks.lock() {
            for s in st.audio.iter().chain(st.video.iter()) {
                g.remove(&s.track_id);
            }
        }
    }
    let first = red.first().map(|(r, b)| format!("round {r}: {b:?}")).unwrap_or_default();
    assert!(
        red.is_empty(),
        "정:RULE-CONC-SIX 정:IMPL-SUB-PIPELINE after overlapping resyncs settle, one more resync leaves the row lists and unplaced marks equal to the table — {} of {ROUNDS} rounds broke ({:?} elapsed); first {first}",
        red.len(),
        start.elapsed()
    );
}
