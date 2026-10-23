// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-SUB-PIPELINE · 정:IMPL-SUB-TABLE · 정:RULE-FWD-STEPS · 연:RULE-SUB-TABLE-SERVER · 연:RULE-SUB-FILL · 연:LINK.S.track_on · 연:LINK.S.track_off · 연:LINK.S.reissue · 연:LINK.S.reassign · model: claude-opus-5-5
//! 구독 배관(cold path) — 받을 트랙 산출(슬롯 둘 · 남의 전이중 활성 트랙 · `paused` 는 줄을 원하지 않는다) · 배정표에 싣기 · 발행측 목록에 `Weak` 붙이기 · 표가 바뀌면 `MLINE_EVENT`(표 전체).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use common::bus::Notice;
use oxsig::link::MlineEvent;
use oxsig::trk::{MediaKind, TrackEntry, TrackType};

use crate::cfg::Ctx;
use crate::link::table::{Change, Class, Fill, Want};
use crate::link::{lock, Load, Peer};
use crate::r#pub::PubStream;
use crate::room::Room;

fn now() -> u64 {
    common::clock::wall_ms()
}

pub fn stream_of(ctx: &Ctx, track_id: &str) -> Option<Arc<PubStream>> {
    ctx.links.all().iter().find_map(|p| p.pub_by_id(track_id))
}

fn rank(ctx: &Ctx, viewer: &Peer, track_id: &str, kind: MediaKind) -> u64 {
    match kind {
        MediaKind::Video => crate::sub::pref(viewer, track_id).priority as u64,
        MediaKind::Audio => stream_of(ctx, track_id).map(|s| s.last_talk_ms.load(Ordering::Acquire)).unwrap_or(0),
    }
}

pub fn want_of(ctx: &Ctx, viewer: &Peer, room: &Room, t: &TrackEntry) -> Option<Want> {
    let class = match t.kind_of {
        TrackType::Slot => Class::Slot,
        TrackType::Individual => Class::Full,
    };
    if class == Class::Full {
        if t.user_id.as_deref() == Some(viewer.user_id.as_str()) || t.active == Some(false) {
            return None;
        }
        if t.kind == MediaKind::Video && crate::sub::pref(viewer, &t.track_id).paused {
            return None;
        }
    }
    let codec = if t.kind == MediaKind::Audio { room.media.audio() } else { room.media.video() };
    Some(Want { track_id: t.track_id.clone(), room_id: room.id.clone(), kind: t.kind, class, codec: codec.to_string(), rank: rank(ctx, viewer, &t.track_id, t.kind), since_ms: now() })
}

pub fn emit_table(p: &Peer) {
    let Some(ctx) = crate::cfg::get() else { return };
    let table = p.table_wire();
    tracing::info!(kind = "ev", spec = "CP-OP-MLINE-EVENT", user = %p.user_id, session = %p.session(), epoch = %ctx.boot.epoch, arev = table.arev, rows = table.mlines.len(), "배정표");
    let body = serde_json::to_value(MlineEvent { table }).unwrap_or_default();
    ctx.bus.put_user(&p.user_id, Notice { op: oxsig::op::MLINE_EVENT, body, exclude: Vec::new(), room_id: None, sfu_id: Some(p.sfu_id.clone()), control: None });
}

pub fn apply(ctx: &Ctx, p: &Arc<Peer>, changes: &[Change]) {
    for c in changes {
        match c {
            Change::Load { mid, track_id, room_id } => {
                let Some(d) = p.ensure_down(*mid) else { continue };
                let (since, pt, rtx) = {
                    let t = lock(&p.table);
                    t.rows.iter().find(|r| r.mid == *mid).map(|r| (r.since, r.pt, r.rtx_pt.unwrap_or(0))).unwrap_or((0, 0, 0))
                };
                d.since.store(since, Ordering::Release);
                d.pt.store(pt, Ordering::Release);
                d.rtx_pt.store(rtx, Ordering::Release);
                d.track.store(Some(Arc::new(Load { track_id: track_id.clone(), room_id: room_id.clone() })));
                if d.class == Class::Slot {
                    if let Some(r) = ctx.rooms.get(room_id) {
                        r.slot(d.kind).attach(&d);
                    }
                    continue;
                }
                if let Some(s) = stream_of(ctx, track_id) {
                    {
                        let mut st = lock(&d.st);
                        st.sel = crate::sim::Sel::default();
                        st.sel.cap = crate::sub::pref(p, track_id).spatial;
                        if s.simulcast {
                            let auto = if ctx.auto() { Some(lock(&p.sub_transport().sim).policy.layer) } else { None };
                            st.sel.aim(auto, now());
                        }
                    }
                    s.attach(&d);
                    if s.kind == MediaKind::Video {
                        crate::fwd::shell::key_for(&d);
                    }
                }
            }
            Change::Unload { mid, track_id, room_id } => {
                let Some(d) = p.down(*mid) else { continue };
                let since = lock(&p.table).rows.iter().find(|r| r.mid == *mid).map(|r| r.since).unwrap_or(0);
                d.since.store(since, Ordering::Release);
                d.track.store(None);
                if d.class == Class::Slot {
                    if let Some(r) = ctx.rooms.get(room_id) {
                        r.slot(d.kind).detach(&d);
                    }
                } else if let Some(s) = stream_of(ctx, track_id) {
                    s.detach(&d);
                }
            }
        }
    }
}

fn rank_fn<'a>(ctx: &'a Ctx, p: &'a Peer) -> impl Fn(&str) -> u64 + 'a {
    move |tid: &str| {
        let kind = stream_of(ctx, tid).map(|s| s.kind).unwrap_or(MediaKind::Audio);
        rank(ctx, p, tid, kind)
    }
}

pub fn want(ctx: &Ctx, p: &Arc<Peer>, w: Want) -> (Fill, bool) {
    let (fill, mut ch) = lock(&p.table).want(w, now(), common::ident::random_u32);
    if fill == Fill::Waiting {
        let rf = rank_fn(ctx, p);
        ch.extend(lock(&p.table).reassign(&rf, now()));
    }
    apply(ctx, p, &ch);
    (fill, !ch.is_empty())
}

pub fn unwant(ctx: &Ctx, p: &Arc<Peer>, track_id: &str) -> bool {
    let ch = lock(&p.table).unwant(track_id, now());
    apply(ctx, p, &ch);
    !ch.is_empty()
}

pub fn room_rows(ctx: &Ctx, p: &Arc<Peer>, room: &Room) -> bool {
    let mut changed = false;
    for t in ctx.tracks.list(&room.id) {
        if let Some(w) = want_of(ctx, p, room, &t) {
            changed |= want(ctx, p, w).1;
        }
    }
    changed
}

pub fn drop_room_rows(ctx: &Ctx, p: &Arc<Peer>, room_id: &str) -> bool {
    let ch = lock(&p.table).unwant_room(room_id, now());
    apply(ctx, p, &ch);
    !ch.is_empty()
}

pub fn track_on(ctx: &Ctx, room: &Room, t: &TrackEntry) {
    for u in room.member_ids() {
        let Some(p) = ctx.links.peer(&u) else { continue };
        if !p.in_room(&room.id) {
            continue;
        }
        let Some(w) = want_of(ctx, &p, room, t) else { continue };
        if want(ctx, &p, w).1 {
            emit_table(&p);
        }
    }
}

pub fn track_off(ctx: &Ctx, track_id: &str) {
    for p in ctx.links.all() {
        if lock(&p.table).row_of(track_id).is_none() && !lock(&p.table).waiting.iter().any(|w| w.track_id == track_id) {
            continue;
        }
        if unwant(ctx, &p, track_id) {
            emit_table(&p);
        }
    }
}

pub fn move_track(ctx: &Ctx, t: &TrackEntry, to: &Room) {
    for p in ctx.links.all() {
        let has = {
            let tb = lock(&p.table);
            tb.row_of(&t.track_id).is_some() || tb.waiting.iter().any(|w| w.track_id == t.track_id)
        };
        let inside = p.in_room(&to.id);
        if has && inside {
            if lock(&p.table).rehome(&t.track_id, &to.id) {
                if let Some(mid) = lock(&p.table).row_of(&t.track_id).map(|r| r.mid)
                    && let Some(d) = p.down(mid) {
                        d.track.store(Some(Arc::new(Load { track_id: t.track_id.clone(), room_id: to.id.clone() })));
                    }
                emit_table(&p);
            }
        } else if has {
            if unwant(ctx, &p, &t.track_id) {
                emit_table(&p);
            }
        } else if inside
            && let Some(w) = want_of(ctx, &p, to, t)
                && want(ctx, &p, w).1 {
                    emit_table(&p);
                }
    }
}

pub fn reassign_tick(ctx: &Ctx) {
    for p in ctx.links.all() {
        if lock(&p.table).waiting.is_empty() {
            continue;
        }
        let rf = rank_fn(ctx, &p);
        let ch = lock(&p.table).reassign(&rf, now());
        if !ch.is_empty() {
            apply(ctx, &p, &ch);
            emit_table(&p);
        }
    }
}
