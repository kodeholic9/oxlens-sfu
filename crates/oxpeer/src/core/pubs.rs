// author: kodeholic (powered by Claude)
//! 말하는 쪽(20261107c 에이전트 단계) — 발행 · 소속(발언 방) · 발언 누름 · 메시지. 절차는 위 발행(PUB) · 소속(AFF) · 발언권(FLR) 상태기(생성본)가 정하고
//! 이 파일은 입구 표 · 효과 표 · 이음매(SDK:IMPL-PUB-API · SDK:IMPL-FLR-BUTTON)만 집행한다.
//! ★서버사이드엔 SDP 가 없다 — 발행의 협상 이음매(`sdp.rebuild`)는 곧바로 성공으로 돌아오고(발행 m-line = PUBLISH_TRACKS 의 줄 하나),
//!   트랙을 송신 시계에 얹는 것(`attach`)도 곧바로다(SDK:RULE-PLAT-SERVER — 앱이 인코딩한 프레임을 넣는다).
//! 송신 시계(SDK:RULE-PLAT-SERVER 송신 시계)는 트랙마다 태스크 하나 — 프레임 길이대로 내보내고 · 게이트(허가 · 전이중 송신)가 닫혀 있으면 버리고 센다.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;

use super::{Cmd, Core, PublishReply};
use crate::api::{ChatSource, Event, RoomView, Snapshot, TrackSpec};
use crate::error::{Category, Error};
use crate::generated::ws_wire::{AffiliationReq, AffiliationRes, ChatMessage, Duplex, Kind, MessageReq, Op, PublishTrack, PublishTracksReq, PublishTracksReqAddAction, PublishTracksReqRemoveAction, Source, TrackSetReq};
use crate::generated::{aff_binding as ab, aff_client as ac, floor_client as fc, pub_binding as pbd, publish_client as pc};
use crate::link::depack::Codec;
use crate::link::send::{ExtIds, OutFrame, SendTrack};
use crate::link::transport::PubOut;
use crate::sig::{Outcome, Purpose};

use super::pcmode::{URI_AUDIO_LEVEL, URI_MID, URI_REPAIR_RID, URI_RID, URI_TWCC, ext_id, pt_row};

const SSRC_COLLISION: i64 = 3008;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SsrcTry {
    First,
    Reissued,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OnPubFail {
    Reissue,
    Fail,
}

pub(crate) fn on_pub_fail(code: i64, tried: SsrcTry, simulcast: bool) -> OnPubFail {
    match (code, tried, simulcast) {
        (SSRC_COLLISION, SsrcTry::First, false) => OnPubFail::Reissue,
        _ => OnPubFail::Fail,
    }
}

pub(crate) fn publish_fault(wire: Error, tried: SsrcTry) -> Error {
    match (i64::from(wire.code), tried) {
        (SSRC_COLLISION, SsrcTry::Reissued) => Error::negotiation("SSRC_COLLISION", wire),
        _ => wire,
    }
}

pub(crate) fn ext_ids(video: bool, simulcast: bool) -> ExtIds {
    let id = |u: &str| ext_id(u).unwrap_or(0);
    ExtIds {
        mid: id(URI_MID),
        audio_level: if video { 0 } else { id(URI_AUDIO_LEVEL) },
        rid: if simulcast { id(URI_RID) } else { 0 },
        rrid: if simulcast { id(URI_REPAIR_RID) } else { 0 },
        twcc: id(URI_TWCC),
    }
}

/// 시뮬캐스트 업링크 층 이름 — 낮은 화질부터(연:RULE-PUB-RID-TO-SPATIAL · l=0 · h=1)
const SIM_RIDS: [&str; 2] = ["l", "h"];
/// 송신 통 — 앱이 실시간보다 빨리 넣으면 이만큼 앞서 쌓이고 그 뒤 `write` 가 기다린다(역압)
pub(crate) const FRAME_QUEUE: usize = 25;
const SR_EVERY: Duration = Duration::from_secs(1);

/// 송신 시계와 SDK 루프가 같이 보는 것 — 게이트 · 게이트가 열린 뒤 첫 패킷 표시 · 계수 · 지금 붙은 서버
pub(crate) struct TrackIo {
    pub gate: AtomicBool,
    pub fresh: AtomicBool,
    pub gated: AtomicU64,
    pub sent: AtomicU64,
    pub qa_dropped: AtomicU64,
    pub server: Mutex<String>,
}

pub(crate) struct PubTrack {
    pub(crate) id: u64,
    pub(crate) room: String,
    pub(crate) st: pc::St,
    pub(crate) kind: Kind,
    pub(crate) half: bool,
    pub(crate) codec: String,
    pub(crate) fmtp: Option<String>,
    pub(crate) pt: u8,
    pub(crate) rtx: Option<(u32, u8)>,
    /// 시뮬캐스트면 0(wire 꼴 — 서버가 rid 확장으로 배운다) · 층마다 진짜 ssrc 는 `ssrcs`
    pub(crate) ssrc: u32,
    pub(crate) ssrcs: Vec<u32>,
    pub(crate) simulcast: bool,
    pub(crate) mid: String,
    pub(crate) track_id: Option<String>,
    pub(crate) io: Arc<TrackIo>,
    waiters: Vec<PublishReply>,
    /// 마지막 거절 응답(앱에 실패 사유로 준다)
    fault: Option<Value>,
    dup_waiters: Vec<oneshot::Sender<Result<(), Error>>>,
    /// 층마다 송신 시계
    pacers: Vec<AbortHandle>,
    /// 협상 이음매를 그 방이 다시 설 때까지 미뤘다(재구축 — 새 세션은 방보다 먼저 선다 · 20261108d)
    nego_wait: bool,
    tried: SsrcTry,
}

impl Drop for PubTrack {
    fn drop(&mut self) {
        for p in &self.pacers {
            p.abort();
        }
    }
}

/// 송신 시계 — 프레임을 그 길이대로 낸다. 늦게 온 프레임(앱이 쉬었다)은 지금부터 다시 센다 · RTP 시각은 보낸 때에서 나온다
#[allow(clippy::too_many_arguments)]
async fn pacer(id: u64, mut rx: mpsc::Receiver<OutFrame>, st: Arc<Mutex<SendTrack>>, io: Arc<TrackIo>, outs: Arc<RwLock<HashMap<String, Arc<OnceLock<PubOut>>>>>, cmd: mpsc::UnboundedSender<Cmd>) {
    let out = |io: &TrackIo| -> Option<PubOut> {
        let server = io.server.lock().ok()?.clone();
        outs.read().ok()?.get(&server)?.get().cloned()
    };
    let mut t0: Option<Instant> = None;
    let mut next: Option<Instant> = None;
    let qa_every: u64 = std::env::var("OXPEER_QA_DROP_SEND_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut qa_n = 0u64;
    let video = st.lock().map(|t| t.clock == 90_000).unwrap_or(false);
    let mut sr = tokio::time::interval(SR_EVERY);
    sr.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            f = rx.recv() => {
                let Some(f) = f else { break };
                let now = Instant::now();
                let spurt = next.is_none_or(|n| now > n + f.duration);
                let due = match next { Some(n) if n > now => n, _ => now };
                if due > now {
                    tokio::time::sleep_until(tokio::time::Instant::from_std(due)).await;
                }
                let base = *t0.get_or_insert(due);
                next = Some(due + f.duration);
                if !io.gate.load(Ordering::Acquire) {
                    io.gated.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let Some(o) = out(&io) else {
                    io.gated.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                let pkts = match st.lock() {
                    Ok(mut t) => {
                        let ts = t.ts_at(due.duration_since(base));
                        t.frame(&f, ts, due, spurt)
                    }
                    Err(_) => continue,
                };
                for p in &pkts {
                    // ★QA 스위치(OXPEER_QA_DROP_SEND_EVERY) — 영상 패킷 N 개마다 하나를 보내지 않는다(보낸 기록엔 남는다 → 서버 NACK 에 RTX 로 답한다) · 시험 수단.
                    //   transport-cc 번호는 매긴다 — 망에서 잃은 것과 같아야 대역 추정이 손실로 본다(번호를 안 매기면 서버 TWCC 에 빈자리가 없다 · 20261108d)
                    //   ★키프레임은 버리지 않는다 — 출발 키프레임이 RTX 로 늦게 오면 그 사이 먼저 온 뒤 번호를 서버 출발 게이트가 버린다(정:IMPL-SUB-READY-GATE 가 허용한 구멍) ·
                    //   그 구멍은 받는 쪽 판정이 망 손실과 못 가른다 — 이 스위치의 몫(업링크 손실 → NACK · RTX · 대역 추정)을 흐린다(20261108d 실측)
                    if qa_every > 0 && video && !f.keyframe {
                        qa_n += 1;
                        if qa_n.is_multiple_of(qa_every) {
                            io.qa_dropped.fetch_add(1, Ordering::Relaxed);
                            o.rtp_lost(p);
                            continue;
                        }
                    }
                    o.rtp(p);
                }
                io.sent.fetch_add(1, Ordering::Relaxed);
                if io.fresh.swap(false, Ordering::AcqRel) {
                    let _ = cmd.send(Cmd::RtpTx(id));
                }
            }
            _ = sr.tick() => {
                if let Some(o) = out(&io)
                    && let Some(b) = st.lock().ok().and_then(|t| t.sender_report(Instant::now(), SystemTime::now()))
                {
                    o.rtcp(&b);
                }
            }
        }
    }
}

fn rand_u32() -> u32 {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    u32::from_be_bytes(b) | 1
}

fn str_or_none(v: Option<&str>) -> String {
    v.map(str::to_string).unwrap_or_else(|| "NONE".to_string())
}

impl Core {
    /// 서버마다 내보내는 손잡이 자리 — 연결을 세우거나 닫을 때 고친다(송신 시계가 서버 이름으로 찾는다)
    pub(super) fn sync_outs(&mut self) {
        let m: HashMap<String, Arc<OnceLock<PubOut>>> = self.links.iter().filter_map(|(k, l)| l.out_slot().map(|o| (k.clone(), o))).collect();
        if let Ok(mut g) = self.pub_outs.write() {
            *g = m;
        }
    }

    // ── 발행(PUB) ──────────────────────────────────────────────────────

    /// 발행 — 그 방의 미디어 코덱으로(연:RULE-PUB-PUBLISH-PROC — 보내기 전에 맞춘다) · 응답의 `track_id` 와 함께 돌아온다
    pub(super) fn publish(&mut self, room: String, spec: TrackSpec, frames: Vec<mpsc::Receiver<OutFrame>>, tx: PublishReply) {
        let Some(r) = self.rooms.get(&room).filter(|r| r.st.rs == crate::generated::room_client::RoomSt::JOINED) else {
            let _ = tx.send(Err(Error::sdk("NOT_JOINED", Category::State)));
            return;
        };
        let Some(server) = r.sfu.clone() else {
            let _ = tx.send(Err(Error::sdk("NOT_JOINED", Category::State)));
            return;
        };
        let video = spec.kind == Kind::Video;
        if spec.simulcast && !video {
            // 오디오에 시뮬캐스트는 없다(PublishTrack.simulcast — 1002)
            let _ = tx.send(Err(Error::sdk("SIMULCAST_AUDIO", Category::State)));
            return;
        }
        let codec = if video { r.media.as_ref().and_then(|m| m.video.clone()).unwrap_or_else(|| "H264".into()) } else { r.media.as_ref().and_then(|m| m.audio.clone()).unwrap_or_else(|| "opus".into()) };
        let c = Codec::of(&codec);
        let Some(row) = pt_row(c) else {
            let _ = tx.send(Err(Error::sdk("CODEC_UNSUPPORTED", Category::Negotiation)));
            return;
        };
        let (pt, rtx_pt, fmtp) = (row.pt, row.rtx_pt, row.fmtp.map(str::to_string));
        let ext = ext_ids(video, spec.simulcast);
        if spec.simulcast && (ext.rid == 0 || ext.rrid == 0) {
            let _ = tx.send(Err(Error::sdk("SIMULCAST_UNSUPPORTED", Category::Negotiation)));
            return;
        }
        // 대역 추정기에 transport-cc 번호를 알린다(서버마다 연결 하나 — 번호 공간은 연결의 것)
        if ext.twcc != 0
            && let Some(o) = self.pub_outs.read().ok().and_then(|g| g.get(&server).and_then(|s| s.get().cloned()))
        {
            o.set_twcc(ext.twcc);
        }
        let n = self.pub_mids.entry(server.clone()).or_insert(0);
        let mid = n.to_string();
        *n += 1;
        self.pub_seq += 1;
        let id = self.pub_seq;
        let picks: Vec<u32> = (0..frames.len()).map(|_| self.pub_ssrc(&server, spec.simulcast)).collect();
        let io = Arc::new(TrackIo { gate: AtomicBool::new(false), fresh: AtomicBool::new(false), gated: AtomicU64::new(0), sent: AtomicU64::new(0), qa_dropped: AtomicU64::new(0), server: Mutex::new(server) });
        let mut ssrcs = Vec::new();
        let mut pacers = Vec::new();
        let mut rtx = None;
        for (k, rx) in frames.into_iter().enumerate() {
            let ssrc = picks.get(k).copied().unwrap_or_else(rand_u32);
            let r = rtx_pt.map(|p| (rand_u32(), p));
            rtx = rtx.or(r);
            let mut t = SendTrack::new(ssrc, pt, r, c, video, mid.clone(), ext);
            if spec.simulcast {
                t.rid = SIM_RIDS.get(k).map(|x| x.to_string());
            }
            let st = Arc::new(Mutex::new(t));
            if let Ok(mut g) = self.sink.senders.write() {
                g.insert(ssrc, st.clone());
            }
            ssrcs.push(ssrc);
            pacers.push(tokio::spawn(pacer(id, rx, st, io.clone(), self.pub_outs.clone(), self.cmd_tx.clone())).abort_handle());
        }
        let ssrc = if spec.simulcast { 0 } else { ssrcs.first().copied().unwrap_or(0) };
        let half = spec.duplex == Duplex::Half;
        let t = PubTrack {
            id,
            room: room.clone(),
            st: pc::St { half, ..pc::initial() },
            kind: spec.kind,
            half,
            codec,
            fmtp,
            pt,
            rtx,
            ssrc,
            ssrcs,
            simulcast: spec.simulcast,
            mid,
            track_id: None,
            io,
            waiters: vec![tx],
            fault: None,
            dup_waiters: Vec::new(),
            pacers,
            nego_wait: false,
            tried: SsrcTry::First,
        };
        self.pubs.insert(id, t);
        self.pub_enter(id, "on", pc::on, "publish");
    }

    pub(super) fn unpublish(&mut self, id: u64) {
        self.pub_enter(id, "off", |s| Some(pc::off(s)), "unpublish");
        self.pub_forget(id);
    }

    fn pub_ssrc(&mut self, server: &str, simulcast: bool) -> u32 {
        if self.qa_collide == 0 || simulcast {
            return rand_u32();
        }
        let down = self.links.get(server).and_then(|l| l.table.as_ref()).and_then(|t| t.mlines.first()).map(|m| m.ssrc);
        let line = crate::log::Line::new(crate::log::Kind::Diag).spec("SD-RULE-PLAT-PC-MODE").key("in.server", server).key("in.left", self.qa_collide);
        match down {
            Some(ssrc) => {
                self.qa_collide -= 1;
                self.log.emit(line.key("in.ssrc", ssrc).msg("qa publish ssrc set to a downlink ssrc"));
                ssrc
            }
            None => {
                self.log.emit(line.msg("qa publish ssrc collide skipped - no downlink row"));
                rand_u32()
            }
        }
    }

    fn pub_reissue(&mut self, id: u64) -> bool {
        let Some(t) = self.pubs.get(&id) else { return false };
        let (n, simulcast, server) = (t.ssrcs.len(), t.simulcast, t.io.server.lock().map(|s| s.clone()).unwrap_or_default());
        let fresh: Vec<u32> = (0..n).map(|_| self.pub_ssrc(&server, simulcast)).collect();
        let Some(t) = self.pubs.get_mut(&id) else { return false };
        let Ok(mut senders) = self.sink.senders.write() else { return false };
        t.tried = SsrcTry::Reissued;
        let mut ssrcs = Vec::with_capacity(t.ssrcs.len());
        let mut rtx = None;
        for (old, ssrc) in t.ssrcs.iter().zip(fresh) {
            if let Some(st) = senders.remove(old) {
                if let Ok(mut s) = st.lock() {
                    s.reissue(ssrc, rand_u32());
                    rtx = rtx.or(s.rtx);
                }
                senders.insert(ssrc, st);
            }
            ssrcs.push(ssrc);
        }
        drop(senders);
        t.ssrc = if t.simulcast { 0 } else { ssrcs.first().copied().unwrap_or(0) };
        t.ssrcs = ssrcs;
        t.rtx = rtx;
        self.log.emit(crate::log::Line::new(crate::log::Kind::Diag).spec("SD-RULE-PLAT-SERVER").key("in.publish", id).key("in.code", SSRC_COLLISION).msg("publish ssrc collision - reissued with new ssrc"));
        self.send_publish(id);
        true
    }

    fn pub_forget(&mut self, id: u64) {
        // 기다리던 publish 는 통이 버려져 닫힘으로 끝난다
        if let Some(t) = self.pubs.remove(&id)
            && let Ok(mut g) = self.sink.senders.write()
        {
            for s in &t.ssrcs {
                g.remove(s);
            }
        }
    }

    pub(super) fn pub_enter(&mut self, id: u64, entry: &'static str, f: impl Fn(&pc::St) -> Option<pc::Step>, cause: &str) {
        let Some(t) = self.pubs.get(&id) else { return };
        let Some(step) = f(&t.st) else {
            self.impossible("PUB", &id.to_string(), t.st.ps.as_str(), entry);
            return;
        };
        self.pub_apply(id, step, cause);
    }

    /// 한 걸음 적용(제네릭 아님 — 이 안에서 다른 입구를 다시 부른다)
    fn pub_apply(&mut self, id: u64, step: pc::Step, cause: &str) {
        let Some(t) = self.pubs.get(&id) else { return };
        let prev = t.st.ps;
        let plan = pbd::plan(step);
        let label = plan.label;
        let Some(t) = self.pubs.get_mut(&id) else { return };
        t.st = plan.state.clone();
        t.half = plan.state.half;
        self.record("PUB", &id.to_string(), prev.as_str(), plan.state.ps.as_str(), label, cause);
        for tx in &plan.send {
            match tx {
                pc::Tx::PublishTracks { action: pc::PublishTracksAction::add, .. } => {
                    if let Some(t) = self.pubs.get_mut(&id) {
                        t.tried = SsrcTry::First;
                    }
                    self.send_publish(id);
                }
                pc::Tx::PublishTracks { action: pc::PublishTracksAction::remove, .. } => self.send_unpublish(id),
                pc::Tx::TrackSet { duplex } => self.send_track_set(id, *duplex),
            }
        }
        for a in &plan.actions {
            let Some(t) = self.pubs.get(&id) else { break };
            match a {
                pbd::PubAction::GateOpen => {
                    t.io.fresh.store(true, Ordering::Release);
                    t.io.gate.store(true, Ordering::Release);
                }
                pbd::PubAction::GateClose => t.io.gate.store(false, Ordering::Release),
                pbd::PubAction::TransceiverInactive => {}
            }
        }
        for e in &plan.events {
            let Some(t) = self.pubs.get_mut(&id) else { break };
            let (room, track_id) = (t.room.clone(), t.track_id.clone());
            match e {
                pbd::PubEvent::Published => {
                    let tid = track_id.clone().unwrap_or_default();
                    let codec = t.codec.clone();
                    for w in t.waiters.drain(..) {
                        let _ = w.send(Ok((id, tid.clone(), codec.clone())));
                    }
                    self.emit(Event::Published { room, track_id: tid });
                }
                pbd::PubEvent::PublishFailed => {
                    let err = t.fault.as_ref().map(|b| publish_fault(Error::from_failure(b), t.tried)).unwrap_or_else(|| Error::sdk("PUBLISH_FAILED", Category::State));
                    for w in t.waiters.drain(..) {
                        let _ = w.send(Err(err.clone()));
                    }
                    self.emit(Event::PublishFailed { room });
                }
                pbd::PubEvent::Ended => self.emit(Event::TrackEnded { room, track_id: track_id.unwrap_or_default() }),
                pbd::PubEvent::DuplexApplied => {
                    for w in t.dup_waiters.drain(..) {
                        let _ = w.send(Ok(()));
                    }
                }
                pbd::PubEvent::DuplexFailed => {
                    for w in t.dup_waiters.drain(..) {
                        let _ = w.send(Err(Error::sdk("DUPLEX_FAILED", Category::State)));
                    }
                }
            }
        }
        // 협상 이음매 — SDP 가 없으니 성공으로 돌아온다(발행 m-line 은 PUBLISH_TRACKS 의 줄). ★단 그 방이 JOINED 일 때만 —
        //   웹은 협상이 연결 · 방이 선 뒤에 끝나지만 서버사이드는 곧바로라 재구축(새 세션이 방보다 먼저 선다)에서 방 밖 PUBLISH_TRACKS 가 나가 거절됐다(20261108d 실측).
        //   방이 아직이면 미뤘다가 그 방이 다시 서면(`pub_room_joined`) 그 방이 지금 붙은 서버를 다시 읽고(SDK:pub.md `LocalTrack.server`) 이음매를 끝낸다
        if plan.seams.iter().any(|s| s.fx == pc::SeamFx::sdp_rebuild) {
            let joined = self.pubs.get(&id).and_then(|t| self.rooms.get(&t.room)).is_some_and(|r| !r.closed && r.st.rs == crate::generated::room_client::RoomSt::JOINED);
            if joined {
                self.pub_rebind(id);
                self.pub_enter(id, "nego", |s| pc::nego(s, true), "no-sdp");
            } else if let Some(t) = self.pubs.get_mut(&id) {
                t.nego_wait = true;
            }
        }
        // 등록됐다 — 송신 시계는 이미 섰다(트랙을 트랜시버에 붙인 것과 같다)
        if prev != pc::PubSt::REGISTERED && plan.state.ps == pc::PubSt::REGISTERED {
            self.pub_enter(id, "attach", |s| pc::attach(s, true), "pacer");
        }
        if plan.state.ps == pc::PubSt::IDLE && !plan.state.want {
            self.pub_forget_if_done(id);
        }
    }

    fn pub_forget_if_done(&mut self, id: u64) {
        if self.pubs.get(&id).is_some_and(|t| t.waiters.is_empty()) {
            self.pub_forget(id);
        }
    }

    fn send_publish(&mut self, id: u64) {
        let Some(t) = self.pubs.get(&id) else { return };
        let Some(join_id) = self.rooms.get(&t.room).and_then(|r| self.jids.str(r.st.jid)).map(str::to_string) else { return };
        let video = t.kind == Kind::Video;
        let x = ext_ids(video, t.simulcast);
        let some = |id: u8| (id != 0).then_some(id);
        let (mid_ext, level_ext, rid_ext, rrid_ext, twcc_ext) = (some(x.mid), some(x.audio_level), some(x.rid), some(x.rrid), some(x.twcc));
        let track = PublishTrack {
            kind: t.kind,
            ssrc: t.ssrc,
            mid: t.mid.clone(),
            codec: Some(t.codec.clone()),
            fmtp: t.fmtp.clone(),
            pt: t.pt,
            rtx_pt: t.rtx.map(|r| r.1),
            // 시뮬캐스트의 RTX 는 서버가 repaired-rid 로 배운다(rtx_ssrc 는 시뮬캐스트가 아닐 때만)
            rtx_ssrc: if t.simulcast { None } else { t.rtx.map(|r| r.0) },
            // 오디오 항목에 source 를 실으면 거절이다(연:RULE-PUB-PUBLISH-FAIL 1002)
            source: if video { Some(Source::Camera) } else { None },
            duplex: Some(if t.half { Duplex::Half } else { Duplex::Full }),
            simulcast: if video { Some(t.simulcast) } else { None },
        };
        let body = PublishTracksReq::Add {
            action: Some(PublishTracksReqAddAction::Add),
            join_id,
            room_id: t.room.clone(),
            tracks: vec![track],
            twcc_extmap_id: twcc_ext,
            rid_extmap_id: rid_ext,
            repair_rid_extmap_id: rrid_ext,
            mid_extmap_id: mid_ext,
            audio_level_extmap_id: level_ext,
            dd_extmap_id: None,
        };
        let pid = t.st.req;
        if let Ok(v) = serde_json::to_value(&body) {
            self.request(Op::PublishTracks, v, Purpose::Publish { track: id, pid });
        }
    }

    fn send_unpublish(&mut self, id: u64) {
        let Some(t) = self.pubs.get(&id) else { return };
        let (Some(tid), Some(join_id)) = (t.track_id.clone(), self.rooms.get(&t.room).and_then(|r| self.jids.str(r.st.jid)).map(str::to_string)) else { return };
        let body = PublishTracksReq::Remove { action: PublishTracksReqRemoveAction::Remove, room_id: t.room.clone(), join_id, track_ids: vec![tid] };
        if let Ok(v) = serde_json::to_value(&body) {
            self.request(Op::PublishTracks, v, Purpose::Unpublish);
        }
    }

    fn send_track_set(&mut self, id: u64, d: pc::Duplex) {
        let Some(t) = self.pubs.get(&id) else { return };
        let Some(tid) = t.track_id.clone() else { return };
        let Some(join_id) = self.rooms.get(&t.room).and_then(|r| self.jids.str(r.st.jid)).map(str::to_string) else { return };
        let req = TrackSetReq { room_id: t.room.clone(), join_id, track_id: Some(tid), ssrc: None, muted: None, duplex: Some(if d == pc::Duplex::half { Duplex::Half } else { Duplex::Full }) };
        self.request(Op::TrackSet, super::to_body(&req), Purpose::TrackSet { track: id });
    }

    pub(super) fn publish_reply(&mut self, id: u64, pid: i64, o: Outcome) {
        let m = match o {
            Outcome::Ok(b) => {
                if let Some(t) = self.pubs.get_mut(&id) {
                    let mid = t.mid.clone();
                    t.track_id = b.get("tracks").and_then(Value::as_array).and_then(|a| a.iter().find(|x| x.get("mid").and_then(Value::as_str) == Some(mid.as_str()))).and_then(|x| x.get("track_id")).and_then(Value::as_str).map(str::to_string);
                }
                pc::PubResMsg { action: Some(pc::PubAction::add), code: 0, part: pc::Part::res, pid }
            }
            Outcome::Fail(b) => {
                let code = b.get("code").and_then(Value::as_i64).unwrap_or(5003);
                let Some(t) = self.pubs.get_mut(&id) else { return };
                let current = t.st.ps == pc::PubSt::STAGED && !t.st.nego && t.st.req == pid;
                if current && on_pub_fail(code, t.tried, t.simulcast) == OnPubFail::Reissue && self.pub_reissue(id) {
                    return;
                }
                if let Some(t) = self.pubs.get_mut(&id) {
                    t.fault = Some(b);
                }
                pc::PubResMsg { action: None, code, part: pc::Part::fail, pid }
            }
            Outcome::Expired => {
                self.pub_enter(id, "reqExpired", pc::req_expired, "T-req");
                return;
            }
            Outcome::Transport | Outcome::Closed => return,
        };
        self.pub_enter(id, "pubRes", |s| Some(pc::pub_res(s, &m)), if m.part == pc::Part::res { "PUBLISH_TRACKS.res" } else { "PUBLISH_TRACKS.fail" });
    }

    pub(super) fn track_set_reply(&mut self, id: u64, o: Outcome) {
        let m = match o {
            Outcome::Ok(b) => pc::SetDuplexResMsg {
                code: 0,
                duplex: b.get("duplex").and_then(Value::as_str).and_then(pc::Duplex::parse),
                noop: b.get("noop").and_then(Value::as_bool).unwrap_or(false),
                part: pc::Part::res,
            },
            Outcome::Fail(b) => pc::SetDuplexResMsg { code: b.get("code").and_then(Value::as_i64).unwrap_or(5003), duplex: None, noop: false, part: pc::Part::fail },
            Outcome::Expired => {
                self.pub_enter(id, "reqExpired", pc::req_expired, "T-req");
                return;
            }
            Outcome::Transport | Outcome::Closed => return,
        };
        self.pub_enter(id, "setDuplexRes", |s| Some(pc::set_duplex_res(s, &m)), "TRACK_SET");
    }

    pub(super) fn set_duplex(&mut self, id: u64, half: bool, tx: oneshot::Sender<Result<(), Error>>) {
        let Some(t) = self.pubs.get_mut(&id) else {
            let _ = tx.send(Err(Error::sdk("NO_TRACK", Category::State)));
            return;
        };
        t.dup_waiters.push(tx);
        let d = if half { pc::Duplex::half } else { pc::Duplex::full };
        self.pub_enter(id, "setDuplex", |s| pc::set_duplex(s, d), "app");
    }

    /// 발언권 효과 mic.open · mic.close 가 그 방의 반이중 발행에 닿는다(위 이음매 — 발행 floorOpen · floorClose)
    pub(super) fn floor_to_pub(&mut self, room: &str, open: bool) {
        let ids: Vec<u64> = self.pubs.values().filter(|t| t.room == room && t.half).map(|t| t.id).collect();
        for id in ids {
            if open {
                self.pub_enter(id, "floorOpen", |s| Some(pc::floor_open(s)), "mic.open");
            } else {
                self.pub_enter(id, "floorClose", |s| Some(pc::floor_close(s)), "mic.close");
            }
        }
    }

    /// 게이트가 열린 뒤 첫 반이중 RTP 가 나갔다 — 발언권 입구 rtpTx
    pub(super) fn rtp_tx(&mut self, id: u64) {
        let Some(t) = self.pubs.get(&id) else { return };
        if t.half {
            let room = t.room.clone();
            self.floor_enter(&room, "rtpTx", |s| Some(fc::rtp_tx(s)), "pacer");
        }
    }

    /// 방을 잃었다 · 떠난다 — 그 방의 발행(위 이음매 roomGone)
    pub(super) fn pub_room_gone(&mut self, room: &str, by_app: bool) {
        let ids: Vec<u64> = self.pubs.values().filter(|t| t.room == room).map(|t| t.id).collect();
        for id in ids {
            self.pub_enter(id, "roomGone", |s| Some(pc::room_gone(s, by_app)), if by_app { "leave" } else { "room_gone" });
        }
    }

    /// 새 세션 — 발행을 전부 다시 한다(위 이음매 pub.reregister_all) · 서버를 다시 읽는 것은 협상 이음매가 끝날 때(방이 다시 선 뒤)
    pub(super) fn pub_reregister_all(&mut self) {
        let ids: Vec<u64> = self.pubs.keys().copied().collect();
        for id in ids {
            self.pub_enter(id, "reregister", |s| Some(pc::reregister(s)), "rebuild");
        }
    }

    /// 그 방이 (다시) 섰다 — 미뤄 둔 협상 이음매를 끝낸다
    pub(super) fn pub_room_joined(&mut self, room: &str) {
        let ids: Vec<u64> = self.pubs.values().filter(|t| t.room == room && t.nego_wait).map(|t| t.id).collect();
        for id in ids {
            if let Some(t) = self.pubs.get_mut(&id) {
                t.nego_wait = false;
            }
            self.pub_rebind(id);
            self.pub_enter(id, "nego", |s| pc::nego(s, true), "room_joined");
        }
    }

    /// 트랙의 서버를 그 발행 방이 지금 붙은 서버로 — 바뀌었으면 그 연결의 mid 를 새로 받고 송신 상태에 알린다(SDK:pub.md `LocalTrack.server` — `moveIn` 전 · 재구축의 다시 발행 전)
    fn pub_rebind(&mut self, id: u64) {
        let Some(t) = self.pubs.get(&id) else { return };
        let Some(server) = self.rooms.get(&t.room).and_then(|r| r.sfu.clone()) else { return };
        let old = t.io.server.lock().map(|s| s.clone()).unwrap_or_default();
        if old == server {
            return;
        }
        let n = self.pub_mids.entry(server.clone()).or_insert(0);
        let mid = n.to_string();
        *n += 1;
        let twcc = ext_id(URI_TWCC).unwrap_or(0);
        if twcc != 0
            && let Some(o) = self.pub_outs.read().ok().and_then(|g| g.get(&server).and_then(|s| s.get().cloned()))
        {
            o.set_twcc(twcc);
        }
        let Some(t) = self.pubs.get_mut(&id) else { return };
        if let Ok(mut g) = t.io.server.lock() {
            *g = server.clone();
        }
        t.mid = mid.clone();
        let senders = self.sink.senders.read().map(|g| t.ssrcs.iter().filter_map(|x| g.get(x).cloned()).collect::<Vec<_>>()).unwrap_or_default();
        for st in senders {
            if let Ok(mut st) = st.lock() {
                st.rebind(mid.clone());
            }
        }
        // 발행을 다른 서버로 옮김 — 발행 상태가 바뀐 걸음이 아니라 판단이다(전이 줄에 서버 ID 를 상태로 적지 않는다 · context 202611/20261109e 판정기가 짚음)
        self.log.emit(crate::log::Line::new(crate::log::Kind::Diag).spec("SD-IMPL-PUB-API").key("in.publish", id).key("in.from_server", &old).key("in.to_server", &server).msg("publish rebound to another server"));
    }

    /// 그 서버 연결이 닫혔다(drop) · 재구축으로 다시 섰다(reregister) — 위 이음매 pub.drop_server · pub.reregister_server
    pub(super) fn pub_server(&mut self, server: &str, up: bool) {
        let ids: Vec<u64> = self.pubs.values().filter(|t| t.io.server.lock().is_ok_and(|s| *s == server)).map(|t| t.id).collect();
        for id in ids {
            if up {
                self.pub_enter(id, "reregister", |s| Some(pc::reregister(s)), "rebuild_server");
            } else {
                self.pub_enter(id, "dropServer", |s| Some(pc::drop_server(s)), "drop_server");
            }
        }
    }

    /// 이어받기 응답의 발행 목록과 맞댄다(위 이음매 pub.resume_check) — listed = 내 트랙이 목록에 있다 · shalf = 서버가 아는 반이중
    pub(super) fn pub_resume_check(&mut self, body: &Value) {
        let list: Vec<crate::generated::ws_wire::Publication> = body.get("publications").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        let ids: Vec<(u64, bool, bool)> = self
            .pubs
            .values()
            .map(|t| {
                let p = t.track_id.as_ref().and_then(|x| list.iter().find(|p| &p.track_id == x));
                (t.id, p.is_some(), p.is_some_and(|p| p.duplex == Duplex::Half))
            })
            .collect();
        for (id, listed, shalf) in ids {
            self.pub_enter(id, "resumeCheck", |s| Some(pc::resume_check(s, listed, shalf)), "RESUME");
        }
    }

    /// 내 발행이 트랙 목록에서 빠졌다(TRACK_EVENT remove — 위 이음매 revoked)
    pub(super) fn pub_revoked(&mut self, track_ids: &[String]) {
        let ids: Vec<u64> = self.pubs.values().filter(|t| t.track_id.as_ref().is_some_and(|x| track_ids.contains(x))).map(|t| t.id).collect();
        for id in ids {
            self.pub_enter(id, "revoked", |s| Some(pc::revoked(s, &pc::RevokedMsg {})), "TRACK_EVENT.remove");
        }
    }

    /// 서버가 내 발행에 키프레임을 청했다 — 앱 몫(SDK 는 인코딩하지 않는다)
    pub(super) fn pli_in(&mut self, ssrc: u32) {
        if let Some(t) = self.pubs.values().find(|t| t.ssrcs.contains(&ssrc)) {
            let layer = if t.simulcast { t.ssrcs.iter().position(|x| *x == ssrc).map(|k| k as u8) } else { None };
            self.emit(Event::KeyframeRequested { room: t.room.clone(), track_id: t.track_id.clone().unwrap_or_default(), layer });
        }
    }

    // ── 발언 누름(FLR) ───────────────────────────────────────────────

    pub(super) fn ptt_press(&mut self, room: &str, priority: u8) -> Result<(), Error> {
        if !self.rooms.contains_key(room) {
            return Err(Error::sdk("NOT_JOINED", Category::State));
        }
        self.floor_enter(room, "press", |s| Some(fc::press(s, i64::from(priority))), "app");
        Ok(())
    }

    pub(super) fn ptt_release(&mut self, room: &str) -> Result<(), Error> {
        if !self.rooms.contains_key(room) {
            return Err(Error::sdk("NOT_JOINED", Category::State));
        }
        self.floor_enter(room, "release", fc::release, "app");
        Ok(())
    }

    // ── 소속(AFF — 발언 방) ──────────────────────────────────────────

    pub(super) fn aff_enter(&mut self, entry: &'static str, f: impl Fn(&ac::St) -> Option<ac::Step>, cause: &str) {
        let Some(step) = f(&self.aff) else {
            self.impossible("AFF", "session", &self.aff.r#pub, entry);
            return;
        };
        let prev = self.aff.r#pub.clone();
        let plan = ab::plan(step);
        self.aff = plan.state.clone();
        self.record("AFF", "session", &prev, &self.aff.r#pub.clone(), plan.label, cause);
        for tx in &plan.send {
            match tx {
                ac::Tx::Affiliation { pub_select, pub_deselect } => self.send_affiliation(pub_select.clone(), pub_deselect.clone()),
            }
        }
        for sm in &plan.seams {
            match sm.fx {
                ac::SeamFx::floor_pub_select => {
                    let r = self.aff.r#pub.clone();
                    if self.rooms.contains_key(&r) {
                        self.floor_enter(&r, "pubSelect", |s| Some(fc::pub_select(s)), "affiliation");
                    }
                }
                ac::SeamFx::floor_pub_away => {
                    if self.rooms.contains_key(&prev) {
                        self.floor_enter(&prev, "pubAway", |s| Some(fc::pub_away(s)), "affiliation");
                    }
                }
                ac::SeamFx::pub_move_out => {
                    // 읽는 값 here = 트랙의 서버가 고른 방(옮겨 갈 방 want — 없으면 새 발행 방 pub)의 서버가 아니다(연:RULE-AFF-SWITCH-TRACKS · 반이중은 사용자당 하나 — 20261124)
                    let tgt = if self.aff.want != "NONE" { self.aff.want.clone() } else { self.aff.r#pub.clone() };
                    let tsfu = self.rooms.get(&tgt).and_then(|r| r.sfu.clone());
                    let ids: Vec<u64> = self
                        .pubs
                        .values()
                        .filter(|t| move_out_here(self.rooms.get(&t.room).and_then(|r| r.sfu.as_deref()), tsfu.as_deref()))
                        .map(|t| t.id)
                        .collect();
                    for id in ids {
                        self.pub_enter(id, "moveOut", |s| Some(pc::move_out(s, true)), "affiliation");
                    }
                }
                ac::SeamFx::pub_move_in => {
                    // ★넣기 전에 트랙의 방 · 서버를 그 발행 방의 것으로(SDK:pub.md `moveIn` · `LocalTrack.server`)
                    let to = self.aff.r#pub.clone();
                    let ids: Vec<u64> = self.pubs.values().filter(|t| t.st.mv).map(|t| t.id).collect();
                    for id in ids {
                        if self.rooms.contains_key(&to)
                            && let Some(t) = self.pubs.get_mut(&id)
                        {
                            t.room = to.clone();
                        }
                        self.pub_rebind(id);
                        self.pub_enter(id, "moveIn", |s| Some(pc::move_in(s)), "affiliation");
                    }
                }
            }
        }
        for e in &plan.events {
            match e {
                ab::AffEvent::SpeakingRoom => {
                    let room = (self.aff.r#pub != "NONE").then(|| self.aff.r#pub.clone());
                    for w in self.aff_waiters.drain(..) {
                        let _ = w.send(Ok(()));
                    }
                    self.emit(Event::SpeakingRoom { room });
                }
                ab::AffEvent::SpeakingRoomFailed => {
                    for w in self.aff_waiters.drain(..) {
                        let _ = w.send(Err(Error::sdk("AFFILIATION_FAILED", Category::State)));
                    }
                }
            }
        }
    }

    fn send_affiliation(&mut self, sel: Option<String>, desel: Option<String>) {
        let jid = |c: &Core, r: &Option<String>| r.as_ref().and_then(|r| c.rooms.get(r)).and_then(|x| c.jids.str(x.st.jid)).map(str::to_string);
        let body = AffiliationReq { pub_select_join_id: jid(self, &sel), pub_deselect_join_id: jid(self, &desel), pub_select: sel.clone(), pub_deselect: desel.clone(), change_id: None };
        let server = sel.as_ref().or(desel.as_ref()).and_then(|r| self.rooms.get(r)).and_then(|r| r.sfu.clone());
        if let Ok(v) = serde_json::to_value(&body) {
            self.request(Op::Affiliation, v, Purpose::Affiliation { server });
        }
    }

    pub(super) fn affiliation_reply(&mut self, server: Option<String>, o: Outcome) {
        let (ok, p) = match &o {
            Outcome::Ok(b) => (true, serde_json::from_value::<AffiliationRes>(b.clone()).ok().and_then(|x| x.pub_room)),
            Outcome::Fail(_) => (false, None),
            Outcome::Expired | Outcome::Transport | Outcome::Closed => (false, None),
        };
        let p = str_or_none(p.as_deref());
        let sfu_of = |c: &Core, r: &str| c.rooms.get(r).and_then(|x| x.sfu.clone());
        let on_pub = server.is_some() && sfu_of(self, &self.aff.r#pub) == server;
        let on_want = server.is_some() && sfu_of(self, &self.aff.want) == server;
        let p_in = self.rooms.contains_key(&p);
        self.aff_enter("res", |s| Some(ac::res(s, ok, &p, on_pub, on_want, p_in)), if ok { "AFFILIATION.res" } else { "AFFILIATION.fail" });
    }

    /// 이 방에서 말하려고 들어간다 — 발행 방이 없으면 select:true 입장(위 소속 입구 joinSelect) · 있으면 select:false 로 들어간 뒤 소속 전환(연:ROOM 입장 절차 2)
    pub(super) fn join_speaking(&mut self, room: String, tx: oneshot::Sender<Result<(), Error>>) {
        if self.rooms.get(&room).is_some_and(|r| !r.closed) {
            return self.set_speaking_room(room, tx);
        }
        let r = room.clone();
        // 입장 요청이 나가기 전에 표시가 서야 한다(ROOM_JOIN 은 방을 만드는 걸음에서 곧 나간다)
        if self.aff.r#pub == "NONE" && ac::join_select(&self.aff, &r).is_some() {
            self.aff_enter("joinSelect", |s| ac::join_select(s, &r), "app");
            self.join_mode = Some((room.clone(), true));
        } else {
            self.join_mode = Some((room.clone(), false));
        }
        self.join(room, tx);
    }

    /// 발언 방을 이 방으로(SDK:IMPL-AFF-SPEAKING-ROOM `setSpeakingRoom`)
    pub(super) fn set_speaking_room(&mut self, room: String, tx: oneshot::Sender<Result<(), Error>>) {
        if !self.rooms.contains_key(&room) {
            let _ = tx.send(Err(Error::sdk("NOT_JOINED", Category::State)));
            return;
        }
        if self.aff.r#pub == room {
            let _ = tx.send(Ok(()));
            return;
        }
        let same = self.rooms.get(&room).and_then(|r| r.sfu.clone()) == self.rooms.get(&self.aff.r#pub).and_then(|r| r.sfu.clone());
        self.aff_waiters.push(tx);
        self.aff_enter("select", |s| ac::select(s, &room, same), "app");
    }

    /// 입장 응답의 affiliation — 입장이 곁들여 바꾼 발행 방(위 입구 implicit)
    pub(super) fn aff_implicit(&mut self, room: &str, sel: bool, pub_room: Option<String>) {
        let p = str_or_none(pub_room.as_deref());
        let server = self.rooms.get(room).and_then(|r| r.sfu.clone());
        let on_pub = server.is_some() && self.rooms.get(&self.aff.r#pub).and_then(|r| r.sfu.clone()) == server;
        let p_in = self.rooms.contains_key(&p);
        let r = room.to_string();
        self.aff_enter("implicit", |s| Some(ac::implicit(s, &r, sel, &p, on_pub, p_in)), "ROOM_JOIN.res");
    }

    // ── 채팅(CHAT) · 지금 값 ─────────────────────────────────────────

    pub(super) fn send_chat(&mut self, space: String, content: String, msg_id: Option<String>, tx: oneshot::Sender<Result<u64, Error>>) {
        let Some(user) = self.user_id.clone() else {
            let _ = tx.send(Err(Error::sdk("NOT_BOUND", Category::State)));
            return;
        };
        let msg_id = msg_id.unwrap_or_else(|| crate::api::new_msg_id(&user));
        self.msg_seq += 1;
        let id = self.msg_seq;
        self.msg_waiters.insert(id, tx);
        if let Ok(v) = serde_json::to_value(MessageReq { space_id: space, msg_id, kind: None, ref_: None, content: Some(content), keys: None, ext: None }) {
            self.request(Op::Message, v, Purpose::Message { id });
        }
    }

    pub(super) fn message_reply(&mut self, id: u64, o: Outcome) {
        let Some(tx) = self.msg_waiters.remove(&id) else { return };
        let r = match o {
            Outcome::Ok(b) => Ok(b.get("t").and_then(Value::as_u64).unwrap_or_default()),
            Outcome::Fail(b) => Err(Error::from_failure(&b)),
            _ => Err(Error::closed("CLOSED")),
        };
        let _ = tx.send(r);
    }

    pub(super) fn chat_catchup(&mut self, space: Option<String>, t: Option<u64>, tx: oneshot::Sender<Result<u64, Error>>) {
        self.msg_seq += 1;
        let id = self.msg_seq;
        self.msg_waiters.insert(id, tx);
        let mut v = serde_json::Map::new();
        if let Some(s) = space {
            v.insert("space_id".into(), Value::from(s));
        }
        if let Some(t) = t {
            v.insert("t".into(), Value::from(t));
        }
        self.request(Op::Catchup, Value::Object(v), Purpose::Message { id });
    }

    pub(super) fn message_in(&mut self, body: Value) {
        if let Some(e) = chat_event(body, ChatSource::Live) {
            self.emit(e);
        }
    }

    /// 묶음(연:CHAT.op.MESSAGES) — ACK 는 받는 길이 먼저 보냈다(연:RULE-TRN-NOTIFY-ACK-STEPS) · 메시지마다 같은 사건으로 낸다
    pub(super) fn messages_in(&mut self, body: Value) {
        for e in bundle_events(body) {
            self.emit(e);
        }
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        let rooms = self
            .rooms
            .values()
            .map(|r| RoomView {
                room: r.id.clone(),
                joined: r.st.rs == crate::generated::room_client::RoomSt::JOINED,
                server: r.sfu.clone(),
                speaker: r.flr.shown.clone(),
                ptt: super::room::ptt_info(r),
                permission: r.me.as_ref().and_then(|m| m.permission.clone()),
                published: self.pubs.values().filter(|t| t.room == r.id).filter_map(|t| t.track_id.clone()).collect(),
            })
            .collect();
        Snapshot {
            session: self.last_info.clone(),
            speaking_room: (self.aff.r#pub != "NONE").then(|| self.aff.r#pub.clone()),
            rooms,
            gated: self.pubs.values().map(|t| t.io.gated.load(Ordering::Relaxed)).sum(),
            sent: self.pubs.values().map(|t| t.io.sent.load(Ordering::Relaxed)).sum(),
        }
    }
}

/// 메시지 하나를 사건으로 — ★`kind` 는 문자열로 받는다(모르는 종류도 그대로 · 연:RULE-CHAT-UNKNOWN-KIND) · 관계 메시지(`edit` · `react` · `delete`)도 접지 않고 낸다
fn chat_event(mut v: Value, source: ChatSource) -> Option<Event> {
    let kind = v.get("kind").and_then(Value::as_str).map(str::to_string);
    if let Some(o) = v.as_object_mut() {
        o.remove("kind");
    }
    let m = serde_json::from_value::<ChatMessage>(v).ok()?;
    Some(Event::Chat {
        space: m.space_id,
        msg_id: m.msg_id,
        user_id: m.user_id,
        t: m.t,
        content: m.content,
        kind,
        target: m.target,
        reply_to: m.ref_,
        keys: m.keys,
        ext: m.ext,
        source,
    })
}

/// 묶음의 사건들 — ★`live:true` 면 혼잡할 때 묶인 실시간(`Live`) · 칸이 없으면 따라잡기(`Catchup` — SDK:IMPL-CHAT-CATCHUP · 연:RULE-CHAT-CATCHUP-BUNDLE).
/// 몸을 `MessagesBody` 로 통째 풀지 않는 까닭 — 원소의 `kind` 가 모르는 종류면 생성 형이 풀지 못해 묶음 전체를 잃는다(원소마다 `chat_event` 가 문자열로 받는다)
fn bundle_events(body: Value) -> Vec<Event> {
    let source = if body.get("live").and_then(Value::as_bool) == Some(true) { ChatSource::Live } else { ChatSource::Catchup };
    let list = body.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();
    list.into_iter().filter_map(|m| chat_event(m, source)).collect()
}

/// 위 이음매 pub.move_out 의 읽는 값 here — 트랙의 서버가 고른 방의 서버가 아니다(둘 다 알 때만 · 연:RULE-AFF-SWITCH-TRACKS · 반이중은 사용자당 하나 — 20261124)
pub(crate) fn move_out_here(track_sfu: Option<&str>, target_sfu: Option<&str>) -> bool {
    matches!((track_sfu, target_sfu), (Some(a), Some(b)) if a != b)
}

#[cfg(test)]
mod move_out_test {
    #[test]
    fn move_out_takes_tracks_off_other_servers_only() {
        // 고른 방의 서버가 아닌 트랙만 내린다 — 서버를 건너는 전환(옛 발행 방 서버) · 발언 방이 없을 때의 고르기(다른 서버에 남은 것)
        assert!(super::move_out_here(Some("sfu-a"), Some("sfu-b")));
        assert!(!super::move_out_here(Some("sfu-b"), Some("sfu-b")), "고른 방의 서버 트랙은 그대로 — 반이중이 둘이 되지 않는다");
        assert!(!super::move_out_here(None, Some("sfu-b")), "서버를 모르는 트랙은 내리지 않는다");
        assert!(!super::move_out_here(Some("sfu-a"), None), "고른 방의 서버를 모르면 내리지 않는다");
    }
}

#[cfg(test)]
mod chat_source_test {
    use super::*;
    use serde_json::json;

    fn msg(id: &str, t: u64) -> Value {
        json!({ "space_id": "S", "msg_id": id, "user_id": "u2", "t": t, "content": "x" })
    }

    fn sources(es: &[Event]) -> Vec<(String, ChatSource)> {
        es.iter()
            .filter_map(|e| match e {
                Event::Chat { msg_id, source, .. } => Some((msg_id.clone(), *source)),
                _ => None,
            })
            .collect()
    }

    /// K-sym-IMPL-CHAT-CATCHUP-3 — `MESSAGES{live:true}` 는 실시간 · `live` 칸 없는 `MESSAGES` 는 따라잡기 · 실시간 `MESSAGE` 는 실시간(context 202611/20261118w)
    #[test]
    fn bundle_source_follows_live_field() {
        let live = bundle_events(json!({ "live": true, "messages": [msg("a", 1), msg("b", 2)] }));
        assert_eq!(sources(&live), vec![("a".into(), ChatSource::Live), ("b".into(), ChatSource::Live)], "혼잡할 때 묶인 실시간은 Live");
        let catchup = bundle_events(json!({ "messages": [msg("c", 3)] }));
        assert_eq!(sources(&catchup), vec![("c".into(), ChatSource::Catchup)], "live 칸 없는 묶음은 Catchup");
        let off = bundle_events(json!({ "live": false, "messages": [msg("d", 4)] }));
        assert_eq!(sources(&off), vec![("d".into(), ChatSource::Catchup)], "live:false 도 따라잡기(칸은 true 만 뜻이 있다)");
        let one = chat_event(msg("e", 5), ChatSource::Live).into_iter().collect::<Vec<_>>();
        assert_eq!(sources(&one), vec![("e".into(), ChatSource::Live)], "실시간 MESSAGE 는 Live");
    }

    /// 무전 발언 기록 · 그 갱신(연:RULE-CHAT-TALK · 연:RULE-CHAT-TALK-UPDATE) — oxpeer 는 접지 않으므로 `talk` · `talk_update` 도 다른 관계 메시지처럼 사건 하나씩 그대로 낸다
    /// (본문 `talk` 칸이 든 몸이 생성 형으로 풀려 묶음을 잃지 않는다 · 갱신은 `reply_to` = 발언 기록 · 20261118v §18)
    #[test]
    fn talk_records_pass_through_unfolded() {
        let talk = json!({ "state": "talking", "room_id": "S", "start": 1, "deadline": 2 });
        let mut t = msg("T1", 8);
        t["kind"] = json!("talk");
        t["content"] = json!("");
        t["talk"] = talk.clone();
        let mut u = msg("T1-end", 9);
        u["kind"] = json!("talk_update");
        u["content"] = json!("");
        u["ref"] = json!("T1");
        u["talk"] = json!({ "state": "ok", "room_id": "S", "start": 1, "deadline": 2, "duration_ms": 900, "rec": { "path": "rec/S/S-T1-u2.ogg", "size": 9, "mime": "audio/ogg", "duration_ms": 900 } });
        let es = bundle_events(json!({ "messages": [t, u, msg("h", 10)] }));
        let got: Vec<(String, Option<String>, Option<String>)> = es
            .iter()
            .filter_map(|e| match e {
                Event::Chat { msg_id, kind, reply_to, .. } => Some((msg_id.clone(), kind.clone(), reply_to.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            got,
            vec![("T1".into(), Some("talk".into()), None), ("T1-end".into(), Some("talk_update".into()), Some("T1".into())), ("h".into(), None, None)],
            "발언 기록 · 갱신이 접히지 않고 그대로 · 묶음의 다른 글도 남는다"
        );
    }

    /// 모르는 `kind` 하나가 묶음 전체를 잃게 하지 않는다(연:RULE-CHAT-UNKNOWN-KIND)
    #[test]
    fn unknown_kind_keeps_the_bundle() {
        let mut odd = msg("f", 6);
        odd["kind"] = json!("poll");
        let es = bundle_events(json!({ "live": true, "messages": [odd, msg("g", 7)] }));
        assert_eq!(sources(&es).len(), 2, "모르는 종류도 사건으로 · 묶음은 그대로");
    }
}

#[cfg(test)]
mod ssrc_collision_test {
    use super::*;
    use serde_json::json;

    #[test]
    fn first_3008_reissues_and_the_second_fails() {
        assert_eq!(on_pub_fail(3008, SsrcTry::First, false), OnPubFail::Reissue, "SDK:RULE-PLAT-SERVER 3008 이면 새 SSRC 로 한 번 다시 낸다");
        assert_eq!(on_pub_fail(3008, SsrcTry::Reissued, false), OnPubFail::Fail, "다시 낸 것도 3008 이면 앱에");
        assert_eq!(on_pub_fail(3008, SsrcTry::First, true), OnPubFail::Fail, "시뮬캐스트 발행은 ssrc:0 이라 이 갈래가 없다");
        for code in [1003, 1005, 2006, 3002, 4002] {
            assert_eq!(on_pub_fail(code, SsrcTry::First, false), OnPubFail::Fail, "{code} 는 다시 내지 않는다");
        }
    }

    #[test]
    fn the_second_3008_reaches_the_app_as_negotiation_with_the_wire_failure_kept() {
        let wire = Error::from_failure(&json!({ "code": 3008, "name": "SSRC_COLLISION", "permanent": true, "details": { "ssrc": 7 } }));
        let e = publish_fault(wire.clone(), SsrcTry::Reissued);
        assert_eq!((e.code, e.name.as_str(), e.category, e.permanent), (0, "NEGOTIATION_SSRC_COLLISION", Category::Negotiation, true), "SDK:RULE-PLAT-SERVER 두 번째 3008 은 negotiation 으로 앱에(웹 local.ts 와 같은 모양)");
        let c = e.cause.as_deref().expect("원 3008 실패를 cause 로 보존");
        assert_eq!(c, &wire);
        assert_eq!((c.code, c.name.as_str(), c.category), (3008, "SSRC_COLLISION", Category::State));
        assert_eq!(c.details.as_ref().and_then(|d| d.get("ssrc")).and_then(Value::as_u64), Some(7), "details.ssrc 보존");
        assert!(e.details.is_none());
    }

    #[test]
    fn a_3008_without_reissue_and_other_failures_reach_the_app_unchanged() {
        let first = Error::from_failure(&json!({ "code": 3008, "name": "SSRC_COLLISION", "permanent": true, "details": { "ssrc": 7 } }));
        assert_eq!(publish_fault(first.clone(), SsrcTry::First), first, "SDK:IMPL-ERR-TYPE 다시 내지 않고 진 3008 은 wire 코드 그대로");
        for tried in [SsrcTry::First, SsrcTry::Reissued] {
            for (code, name) in [(1003, "MISSING_FIELD"), (3002, "NOT_IN_ROOM"), (4002, "LIMIT_EXCEEDED"), (5003, "INTERNAL_ERROR")] {
                let wire = Error::from_failure(&json!({ "code": code, "name": name }));
                assert_eq!(publish_fault(wire.clone(), tried), wire, "{code} 는 wire 그대로");
            }
            let sdk = Error::sdk("PUBLISH_FAILED", Category::State);
            assert_eq!(publish_fault(sdk.clone(), tried), sdk);
        }
    }

    #[test]
    fn publish_extension_ids_are_the_table_numbers() {
        let a = ext_ids(false, false);
        assert_eq!((a.mid, a.audio_level, a.rid, a.rrid, a.twcc), (1, 4, 0, 0, 6), "오디오 — 음량을 싣는다");
        let v = ext_ids(true, true);
        assert_eq!((v.mid, v.audio_level, v.rid, v.rrid, v.twcc), (1, 0, 10, 11, 6), "시뮬캐스트 영상 — 층 이름 둘");
    }
}
