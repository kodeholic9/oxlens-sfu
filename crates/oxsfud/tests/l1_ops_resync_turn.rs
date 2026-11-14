// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SUB-PIPELINE · 정:IMPL-SUB-TABLE · 정:RULE-CONC-SIX · 정:IMPL-GC-PEER-REAP · 정:RULE-GC-LEAVE-SEVEN · 정:IMPL-SUB-READY-GATE · model: claude-opus-5-5
//! 1층 제어 층 — 재배정은 배정표마다 한 차례다: 재료 셈 · 판정 · 줄 목록 · 줄 밖 표지 · 통지가 끼어듦 없이 돌고, 회수는 그 차례 안에서 표를 닫는다. 공개 입구(`resync_peer` · `gc::leave` · `gc::evict`)를 스레드 둘로 겹쳐 부르고, 시험이 발행 스트림 · 줄의 잠금 하나를 쥐어 한쪽을 걸음 한가운데 세운다. 세움은 잠이 아니라 관측(스트림 `Arc` 를 쥠 · 표가 판정을 마침)으로 확인하고, 다른 쪽은 기한 안에 끝나는지만 본다(context 202611/20261114j A-7 · A′-8).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use common::envelope::{Claims, Home};
use oxsfud::flr::core::Floor;
use oxsfud::fwd::DownRow;
use oxsfud::gc::Why;
use oxsfud::link::{Peer, Role, Transport};
use oxsfud::ops::{self, Sfu};
use oxsfud::r#pub::{PubStream, PubTrack, StreamInit};
use oxsfud::room::{Member, PERMANENT, Room, RoomInit};
use oxsfud::sub::{SyncOut, Table};
use oxsig::msg::{Duplex, Kind, MediaCodecs, PcMode, Permission};

const WAIT: Duration = Duration::from_millis(1_500);

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static SERIAL: Mutex<()> = Mutex::new(());
static NEXT: AtomicU64 = AtomicU64::new(1);

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

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

fn enter(p: &Arc<Peer>, room: &Room) -> SyncOut {
    p.rooms.lock().expect("rooms").insert(room.id.clone(), format!("j-{}", room.id));
    let out = ops::media::resync_peer(p, false);
    room.join(
        Member { user: p.user.clone(), role: Some(255), select: true, participant_type: 0, hidden: false, metadata: None, permission: Permission::default() },
        false,
        None,
    );
    out
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
    s.tracks.lock().expect("tracks").insert(st.track_id.clone(), st.clone());
    st
}

fn voiced(st: &Arc<PubStream>, at: u64) {
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

fn list(st: &PubStream, on: bool) {
    st.listed.store(on, Ordering::Release);
}

fn row_of(t: &Table, kind: Kind, track: &str) -> Arc<DownRow> {
    t.loaded().into_iter().find(|(tid, _, d)| tid == track && d.kind == kind).map(|(_, _, d)| d).expect("row loaded with the track")
}

fn loads(o: &SyncOut, track: &str, room: &str) -> bool {
    o.changes.iter().any(|c| c.new.as_ref().is_some_and(|n| n.0 == track && n.1 == room))
}

fn unloads(o: &SyncOut, track: &str, room: &str) -> bool {
    o.changes.iter().any(|c| c.old.as_ref().is_some_and(|n| n.0 == track && n.1 == room) && c.new.is_none())
}

fn has(t: &Table, track: &str, room: &str, row: &Arc<DownRow>) -> bool {
    t.loaded().iter().any(|(tid, rid, d)| tid == track && rid == room && Arc::ptr_eq(d, row))
}

fn remembers_unplaced(t: &Table, track: &str) -> bool {
    t.st.lock().map(|s| s.unplaced.contains(track)).unwrap_or(false)
}

fn wait_until(mut f: impl FnMut() -> bool) -> bool {
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

fn finished_within<T>(h: &JoinHandle<T>) -> bool {
    wait_until(|| h.is_finished())
}

fn resync_in_thread(p: &Arc<Peer>) -> JoinHandle<SyncOut> {
    let p = p.clone();
    std::thread::spawn(move || ops::media::resync_peer(&p, true))
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
        let remembered = registered && remembers_unplaced(&sub.t, &st.track_id);
        if marked != remembered {
            out.push(format!("I5 unplaced mark of {} is {marked} but the table remembers {remembered}", st.track_id));
        }
    }
    out
}

#[test]
fn r0_harness_runs_without_media_and_applies_row_commands_on_the_calling_thread() {
    let _g = serial();
    assert!(oxsfud::fwd::rt::media().is_none(), "the harness runs without media workers");
    let x = room(&tag("r0-x"));
    let sub = subscriber(&tag("r0-p"));
    let o = enter(&sub.p, &x);
    let slots = [oxsfud::trk::slot_track_id(&x.id, Kind::Audio), oxsfud::trk::slot_track_id(&x.id, Kind::Video)];
    assert!(slots.iter().all(|t| loads(&o, t, &x.id)), "정:IMPL-SUB-PIPELINE joining loads both slots of the room");
    let a = stream(&tag("r0-ua"), &x.id, Kind::Audio);
    let v = stream(&tag("r0-uv"), &x.id, Kind::Video);
    list(&a, true);
    list(&v, true);
    let o = ops::media::resync_peer(&sub.p, true);
    assert!(loads(&o, &a.track_id, &x.id) && loads(&o, &v.track_id, &x.id), "정:IMPL-SUB-PIPELINE a listed full-duplex track is loaded");
    for (_, _, d) in sub.t.loaded() {
        assert_eq!(
            d.applied.load(Ordering::Acquire),
            d.since.load(Ordering::Acquire),
            "정:RULE-FWD-MEDIA-WORKERS without media the row command is applied on the calling thread before resync returns"
        );
    }
    assert!(breaks(&sub).is_empty(), "정:IMPL-SUB-TABLE lists equal the table after a resync: {:?}", breaks(&sub));
    list(&a, false);
    let o = ops::media::resync_peer(&sub.p, true);
    assert!(unloads(&o, &a.track_id, &x.id), "정:IMPL-SUB-PIPELINE an unlisted track is unloaded");
    assert!(breaks(&sub).is_empty(), "정:IMPL-SUB-TABLE lists equal the table after an unload: {:?}", breaks(&sub));
    assert!(oxsfud::gc::leave(&sub.p, &x.id, Why::Leave, false), "leave finds the peer in the roster");
    assert!(sfu().table(&sub.p).is_none(), "정:IMPL-GC-PEER-REAP the last leave reaps the table");
    assert!(breaks(&sub).is_empty(), "정:IMPL-GC-PEER-REAP nothing of a reaped table stays in a list: {:?}", breaks(&sub));
}

#[test]
fn r0_the_same_steps_one_after_another_keep_lists_equal_to_the_table() {
    let _g = serial();
    let x = room(&tag("r0s-x"));
    let y = room(&tag("r0s-y"));
    let sub = subscriber(&tag("r0s-p"));
    enter(&sub.p, &x);
    enter(&sub.p, &y);
    let a = stream(&tag("r0s-ua"), &x.id, Kind::Audio);
    list(&a, true);
    ops::media::resync_peer(&sub.p, true);
    assert!(oxsfud::gc::leave(&sub.p, &x.id, Why::Kick, false), "kick finds the peer");
    ops::media::resync_peer(&sub.p, true);
    assert!(breaks(&sub).is_empty(), "정:IMPL-SUB-PIPELINE a resync after the kick loads nothing of the left room: {:?}", breaks(&sub));
    let b = stream(&tag("r0s-ub"), &y.id, Kind::Audio);
    list(&b, true);
    ops::media::resync_peer(&sub.p, true);
    list(&b, false);
    ops::media::resync_peer(&sub.p, true);
    assert!(breaks(&sub).is_empty(), "정:IMPL-SUB-TABLE load then unload in turn: {:?}", breaks(&sub));
    assert!(oxsfud::gc::leave(&sub.p, &y.id, Why::Leave, false), "leave finds the peer");
    ops::media::resync_peer(&sub.p, true);
    assert!(breaks(&sub).is_empty(), "정:IMPL-GC-PEER-REAP a resync after the reap loads nothing: {:?}", breaks(&sub));
}

#[test]
fn r1_late_resync_after_last_leave_loads_nothing() {
    let _g = serial();
    let x = room(&tag("r1-x"));
    let sub = subscriber(&tag("r1-p"));
    enter(&sub.p, &x);
    let a = stream(&tag("r1-u"), &x.id, Kind::Audio);
    assert!(breaks(&sub).is_empty(), "setup: {:?}", breaks(&sub));
    let held = a.tracks.lock().expect("tracks");
    list(&a, true);
    let base = Arc::strong_count(&a);
    let t2 = resync_in_thread(&sub.p);
    let stood = wait_until(|| Arc::strong_count(&a) > base);
    let (p1, xid) = (sub.p.clone(), x.id.clone());
    let t1 = std::thread::spawn(move || oxsfud::gc::leave(&p1, &xid, Why::Leave, false));
    let t1_alone = finished_within(&t1);
    drop(held);
    let o2 = t2.join().expect("t2");
    let left = t1.join().expect("t1");
    assert!(stood, "stand failed — T2 never held the listed stream while reading its material");
    assert!(loads(&o2, &a.track_id, &x.id), "stand check — T2 read room X and the listed track A before the leave");
    assert!(left, "leave finds the peer");
    let b = breaks(&sub);
    assert!(
        b.is_empty(),
        "정:IMPL-GC-PEER-REAP 정:IMPL-SUB-PIPELINE S-sym-IMPL-GC-PEER-REAP-1 a resync that read its material before the last leave loads nothing into the reaped table (leave finished while T2 stood: {t1_alone}): {b:?}"
    );
}

#[test]
fn r2_stale_resync_does_not_reload_left_room() {
    let _g = serial();
    let x = room(&tag("r2-x"));
    let y = room(&tag("r2-y"));
    let sub = subscriber(&tag("r2-p"));
    enter(&sub.p, &x);
    enter(&sub.p, &y);
    let a = stream(&tag("r2-u"), &x.id, Kind::Audio);
    assert!(breaks(&sub).is_empty(), "setup: {:?}", breaks(&sub));
    let held = a.tracks.lock().expect("tracks");
    list(&a, true);
    let base = Arc::strong_count(&a);
    let t2 = resync_in_thread(&sub.p);
    let stood = wait_until(|| Arc::strong_count(&a) > base);
    let (p1, xid) = (sub.p.clone(), x.id.clone());
    let t1 = std::thread::spawn(move || oxsfud::gc::leave(&p1, &xid, Why::Kick, false));
    let t1_alone = finished_within(&t1);
    drop(held);
    let o2 = t2.join().expect("t2");
    let left = t1.join().expect("t1");
    assert!(stood, "stand failed — T2 never held the listed stream while reading its material");
    assert!(loads(&o2, &a.track_id, &x.id), "stand check — T2 read room X and the listed track A before the kick");
    assert!(left, "kick finds the peer");
    assert!(sfu().table(&sub.p).is_some(), "the peer stays in room Y");
    let b = breaks(&sub);
    assert!(
        b.is_empty(),
        "정:IMPL-SUB-PIPELINE S-sym-IMPL-SUB-PIPELINE-1 a resync that read room X before the kick does not load X again — a left room is never heard (kick finished while T2 stood: {t1_alone}): {b:?}"
    );
}

#[test]
fn r3_judgment_order_is_execution_order() {
    let _g = serial();
    let x = room(&tag("r3-x"));
    let sub = subscriber(&tag("r3-p"));
    enter(&sub.p, &x);
    let a = stream(&tag("r3-u"), &x.id, Kind::Audio);
    list(&a, true);
    ops::media::resync_peer(&sub.p, true);
    let r = row_of(&sub.t, Kind::Audio, &a.track_id);
    list(&a, false);
    ops::media::resync_peer(&sub.p, true);
    assert!(breaks(&sub).is_empty(), "setup: {:?}", breaks(&sub));
    let held = r.rw.lock().expect("rw");
    list(&a, true);
    let t1 = resync_in_thread(&sub.p);
    let stood = wait_until(|| has(&sub.t, &a.track_id, &x.id, &r));
    list(&a, false);
    let t2 = resync_in_thread(&sub.p);
    let t2_alone = finished_within(&t2);
    drop(held);
    let o1 = t1.join().expect("t1");
    let o2 = t2.join().expect("t2");
    assert!(stood, "stand failed — T1 never judged A onto the empty row");
    assert!(loads(&o1, &a.track_id, &x.id), "stand check — T1 loads A");
    assert!(unloads(&o2, &a.track_id, &x.id), "stand check — T2 unloads A after T1 judged");
    let b = breaks(&sub);
    assert!(
        b.is_empty(),
        "정:RULE-CONC-SIX 정:IMPL-SUB-PIPELINE S-sym-IMPL-SUB-PIPELINE-2 judge 1 · execute 1 · judge 2 · execute 2 — a later unload is not undone by the earlier load's late list insert (T2 finished while T1 stood: {t2_alone}): {b:?}"
    );
}

#[test]
fn r7_a_later_reload_keeps_its_row_in_the_list() {
    let _g = serial();
    let x = room(&tag("r7-x"));
    let sub = subscriber(&tag("r7-p"));
    enter(&sub.p, &x);
    let b_audio = stream(&tag("r7-ub"), &x.id, Kind::Audio);
    let v = stream(&tag("r7-uv"), &x.id, Kind::Video);
    list(&b_audio, true);
    ops::media::resync_peer(&sub.p, true);
    let r2 = row_of(&sub.t, Kind::Audio, &b_audio.track_id);
    list(&b_audio, false);
    ops::media::resync_peer(&sub.p, true);
    list(&v, true);
    ops::media::resync_peer(&sub.p, true);
    let r = row_of(&sub.t, Kind::Video, &v.track_id);
    assert!(breaks(&sub).is_empty(), "setup: {:?}", breaks(&sub));
    let held = r2.rw.lock().expect("rw");
    list(&b_audio, true);
    sub.t.set_pref(&v.track_id, None, Some(true));
    let t1 = resync_in_thread(&sub.p);
    let stood = wait_until(|| has(&sub.t, &b_audio.track_id, &x.id, &r2) && !has(&sub.t, &v.track_id, &x.id, &r));
    sub.t.set_pref(&v.track_id, None, Some(false));
    let t2 = resync_in_thread(&sub.p);
    let t2_alone = finished_within(&t2);
    drop(held);
    let o1 = t1.join().expect("t1");
    let o2 = t2.join().expect("t2");
    assert!(stood, "stand failed — T1 never judged B onto R2 and V off R");
    assert!(loads(&o1, &b_audio.track_id, &x.id) && unloads(&o1, &v.track_id, &x.id), "stand check — T1 loads B and unloads V");
    assert!(loads(&o2, &v.track_id, &x.id), "stand check — T2 loads V again");
    let b = breaks(&sub);
    assert!(
        b.is_empty(),
        "정:RULE-CONC-SIX 정:IMPL-SUB-PIPELINE S-sym-IMPL-SUB-PIPELINE-2 an earlier unload executed after a later reload does not take the reloaded row out of the list (T2 finished while T1 stood: {t2_alone}): {b:?}"
    );
}

#[test]
fn r8_unplaced_mark_follows_the_judgment_order() {
    let _g = serial();
    let x = room(&tag("r8-x"));
    let sub = subscriber(&tag("r8-p"));
    enter(&sub.p, &x);
    let v = stream(&tag("r8-uv"), &x.id, Kind::Video);
    list(&v, true);
    ops::media::resync_peer(&sub.p, true);
    let r2 = row_of(&sub.t, Kind::Video, &v.track_id);
    list(&v, false);
    ops::media::resync_peer(&sub.p, true);
    let d = stream(&tag("r8-ud"), &x.id, Kind::Audio);
    voiced(&d, 2);
    list(&d, true);
    let others: Vec<Arc<PubStream>> = (0..24)
        .map(|i| {
            let o = stream(&tag("r8-uo"), &x.id, Kind::Audio);
            voiced(&o, 3 + i);
            list(&o, true);
            o
        })
        .collect();
    let a = stream(&tag("r8-ua"), &x.id, Kind::Audio);
    voiced(&a, 1);
    ops::media::resync_peer(&sub.p, true);
    assert_eq!(sub.t.loaded().iter().filter(|(_, _, r)| r.kind == Kind::Audio && !r.slot).count(), 25, "setup: 25 full audio rows are loaded");
    assert!(breaks(&sub).is_empty(), "setup: {:?}", breaks(&sub));
    let held = r2.rw.lock().expect("rw");
    list(&a, true);
    list(&v, true);
    let t1 = resync_in_thread(&sub.p);
    let stood = wait_until(|| remembers_unplaced(&sub.t, &a.track_id) && has(&sub.t, &v.track_id, &x.id, &r2));
    voiced(&a, 1_000);
    let t2 = resync_in_thread(&sub.p);
    let t2_alone = finished_within(&t2);
    drop(held);
    let o1 = t1.join().expect("t1");
    let o2 = t2.join().expect("t2");
    assert!(stood, "stand failed — T1 never judged A unplaced with V on R2");
    assert!(o1.unplaced_add.contains(&a.track_id) && loads(&o1, &v.track_id, &x.id), "stand check — T1 marks A unplaced and loads V");
    assert!(o2.unplaced_del.contains(&a.track_id) && loads(&o2, &a.track_id, &x.id), "stand check — T2 places A and clears its mark");
    assert!(o2.unplaced_add.contains(&d.track_id), "stand check — T2 leaves D out instead");
    assert_eq!(others.len(), 24);
    let b = breaks(&sub);
    assert!(
        b.is_empty(),
        "정:RULE-CONC-SIX 정:IMPL-SUB-TABLE S-sym-IMPL-SUB-PIPELINE-2 the unplaced mark is applied in the order it was judged — an earlier add does not land after a later delete (T2 finished while T1 stood: {t2_alone}): {b:?}"
    );
}

#[test]
fn r9_reap_with_rooms_left_takes_every_row_out_of_the_lists() {
    let _g = serial();
    let x = room(&tag("r9-x"));
    let sub = subscriber(&tag("r9-p"));
    enter(&sub.p, &x);
    let a = stream(&tag("r9-u"), &x.id, Kind::Audio);
    list(&a, true);
    ops::media::resync_peer(&sub.p, true);
    assert!(breaks(&sub).is_empty(), "setup: {:?}", breaks(&sub));
    assert!(x.remove(&sub.p.user).is_some(), "the roster drops the peer first");
    oxsfud::gc::evict(&sub.p, None, Why::Evict);
    assert!(sfu().table(&sub.p).is_none(), "정:IMPL-GC-PEER-REAP evict reaps the table");
    assert!(sub.p.in_room(&x.id), "setup: leave returned early so the peer still names room X");
    let b = breaks(&sub);
    assert!(
        b.is_empty(),
        "정:IMPL-GC-PEER-REAP S-sym-IMPL-GC-PEER-REAP-2 closing a table takes its rows out of every list even when the peer still names a room: {b:?}"
    );
}

#[test]
fn r5_retired_table_takes_no_rows() {
    let _g = serial();
    let x = room(&tag("r5-x"));
    let sub = subscriber(&tag("r5-p"));
    sub.p.rooms.lock().expect("rooms").insert(x.id.clone(), "j".into());
    let base = Arc::strong_count(&sub.t);
    let held = sub.t.turn.lock().expect("turn");
    let t2 = resync_in_thread(&sub.p);
    let stood = wait_until(|| Arc::strong_count(&sub.t) > base);
    let gone = sfu().tables.remove(sub.p.subscribe.id).is_some();
    drop(held);
    let o2 = t2.join().expect("t2");
    assert!(stood && gone, "stand failed — T2 never held the table while waiting for its turn");
    assert!(o2.changes.is_empty(), "정:IMPL-GC-PEER-REAP S-sym-IMPL-GC-PEER-REAP-1 a resync that waited for the turn of a retired table judges nothing");
    let b = breaks(&sub);
    assert!(b.is_empty(), "정:IMPL-GC-PEER-REAP S-sym-IMPL-GC-PEER-REAP-1 a retired table takes no rows into any list: {b:?}");
}

struct Src {
    file: String,
    func: String,
    line: usize,
    text: String,
}

fn product_lines() -> Vec<Src> {
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

fn at(l: &Src) -> String {
    format!("{}:{} ({})", l.file, l.line, l.func)
}

#[test]
fn r6_reassignment_writers_live_in_one_turn() {
    let src = product_lines();
    let mut bad = Vec::new();
    let defines = |l: &Src, name: &str| l.text.contains(&format!("fn {name}("));
    for l in &src {
        if l.text.contains("rows.rcu(") && l.func != "apply_changes" {
            bad.push(format!("row list rcu outside apply_changes: {}", at(l)));
        }
        if l.text.contains("rows.store(") && l.func != "drop_stream" {
            bad.push(format!("row list store outside drop_stream: {}", at(l)));
        }
        if l.text.contains("unplaced_by") && l.file != "pub/mod.rs" && l.func != "mark_unplaced" {
            bad.push(format!("unplaced mark touched outside mark_unplaced: {}", at(l)));
        }
        if l.text.contains("turn.lock()") && l.file != "ops/media.rs" {
            bad.push(format!("the resync turn is taken outside ops/media.rs: {}", at(l)));
        }
    }
    let calls = |pat: &str, name: &str| -> Vec<&Src> { src.iter().filter(|l| l.text.contains(pat) && !defines(l, name)).collect() };
    let apply = calls("apply_changes(", "apply_changes");
    if apply.len() != 1 || apply.iter().any(|l| l.func != "settle") {
        bad.push(format!("apply_changes must be called once, from settle: {:?}", apply.iter().map(|l| at(l)).collect::<Vec<_>>()));
    }
    let sync = calls(".sync(", "sync");
    if sync.len() != 1 || sync.iter().any(|l| l.func != "settle") {
        bad.push(format!("Table::sync must be called once, from settle: {:?}", sync.iter().map(|l| at(l)).collect::<Vec<_>>()));
    }
    let mark = calls("mark_unplaced(", "mark_unplaced");
    if mark.len() != 1 || mark.iter().any(|l| l.func != "settle") {
        bad.push(format!("mark_unplaced must be called once, from settle: {:?}", mark.iter().map(|l| at(l)).collect::<Vec<_>>()));
    }
    let remove = calls("tables.remove(", "remove");
    if remove.len() != 1 || remove.iter().any(|l| l.func != "retire_table") {
        bad.push(format!("a table leaves the table set only in retire_table: {:?}", remove.iter().map(|l| at(l)).collect::<Vec<_>>()));
    }
    let turns: Vec<&Src> = src.iter().filter(|l| l.text.contains("turn.lock()")).collect();
    let mut owners: Vec<&str> = turns.iter().map(|l| l.func.as_str()).collect();
    owners.sort();
    if owners != ["resync_peer", "retire_table"] {
        bad.push(format!("the resync turn is taken exactly in resync_peer and retire_table: {:?}", turns.iter().map(|l| at(l)).collect::<Vec<_>>()));
    }
    let settle: Vec<&Src> = src.iter().filter(|l| l.file == "ops/media.rs" && l.func == "settle").collect();
    if settle.is_empty() {
        bad.push("ops/media.rs has no settle — the body of one resync turn".into());
    }
    let inside = ["settle", "cands_for", "apply_changes", "mark_unplaced", "room_streams", "voice_rank"];
    for name in inside {
        if !src.iter().any(|l| l.file == "ops/media.rs" && l.func == name) {
            bad.push(format!("ops/media.rs has no {name} — a function that runs inside the resync turn"));
        }
    }
    for l in src.iter().filter(|l| l.file == "ops/media.rs" && inside.contains(&l.func.as_str())) {
        for pat in ["floor::run", "resync_", "gc::", "retire_table", ".turn"] {
            if l.text.contains(pat) {
                bad.push(format!("{} reaches out of the turn ({pat}): {}", l.func, at(l)));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "정:RULE-CONC-SIX 정:IMPL-SUB-PIPELINE S-sym-IMPL-SUB-PIPELINE-2 the row lists · unplaced marks · table set are written only inside one resync turn, and the functions of ops/media.rs that run inside the turn reach no other turn · no floor judgment · no gc: {bad:#?}"
    );
}

#[test]
fn r6_ready_gate_is_forgotten_by_the_judgment_not_by_leave() {
    let src = product_lines();
    let calls: Vec<&Src> = src.iter().filter(|l| l.text.contains("forget_room(") && !l.text.contains("fn forget_room(")).collect();
    let ok = calls.len() == 1 && calls[0].file == "ops/rooms.rs" && calls[0].func == "destroy_room";
    assert!(
        ok,
        "정:IMPL-SUB-READY-GATE the resync that loses a room forgets its gate — forget_room is called only by room destroy: {:?}",
        calls.iter().map(|l| at(l)).collect::<Vec<_>>()
    );
}
