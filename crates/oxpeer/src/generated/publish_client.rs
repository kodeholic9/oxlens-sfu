// ★생성 파일 — gen/axis_client.mts 가 원천(context/spec/protocol · areas/pub/behavior)에서 짓는다. 손으로 고치지 않는다.
// IR sha256 207c0df6ea211dc9 · 영역 PUB 클라 상태기 Rust 판 — 같은 생성기의 TS 판과 한 뿌리(같은 입구 · 같은 경우 · 같은 라벨).
// 집행은 부르는 쪽 몫이다 — out(보낼 것) · fx(이 영역의 효과) · seams(다른 영역 입구로 — 받는 곳은 위가 정했다) · start(새로 걸 타이머). armed() 밖의 타이머는 멈춘다.
// 모형 자리(트랙 · 배정표 사본의 열쇠)는 문자열이다 — 실제 값을 넣는 법은 결합이 정한다(SDK:RULE-MOD-BINDING-VALUES).
#![allow(dead_code, unused_variables, unused_mut, unused_parens, non_camel_case_types, clippy::all)]

/// 값 이름 Duplex
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Duplex {
    full,
    half,
}
impl Duplex {
    pub const ALL: &'static [Duplex] = &[Duplex::full, Duplex::half];
    pub fn as_str(self) -> &'static str {
        match self { Duplex::full => "full", Duplex::half => "half", }
    }
    pub fn parse(s: &str) -> Option<Duplex> {
        match s { "full" => Some(Duplex::full), "half" => Some(Duplex::half), _ => None }
    }
}

/// 값 이름 Part
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Part {
    req,
    res,
    fail,
    body,
}
impl Part {
    pub const ALL: &'static [Part] = &[Part::req, Part::res, Part::fail, Part::body];
    pub fn as_str(self) -> &'static str {
        match self { Part::req => "req", Part::res => "res", Part::fail => "fail", Part::body => "body", }
    }
    pub fn parse(s: &str) -> Option<Part> {
        match s { "req" => Some(Part::req), "res" => Some(Part::res), "fail" => Some(Part::fail), "body" => Some(Part::body), _ => None }
    }
}

/// 값 이름 PubAction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PubAction {
    add,
    remove,
}
impl PubAction {
    pub const ALL: &'static [PubAction] = &[PubAction::add, PubAction::remove];
    pub fn as_str(self) -> &'static str {
        match self { PubAction::add => "add", PubAction::remove => "remove", }
    }
    pub fn parse(s: &str) -> Option<PubAction> {
        match s { "add" => Some(PubAction::add), "remove" => Some(PubAction::remove), _ => None }
    }
}

/// 값 이름 PubSt
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PubSt {
    IDLE,
    STAGED,
    REGISTERED,
    SENDING,
}
impl PubSt {
    pub const ALL: &'static [PubSt] = &[PubSt::IDLE, PubSt::STAGED, PubSt::REGISTERED, PubSt::SENDING];
    pub fn as_str(self) -> &'static str {
        match self { PubSt::IDLE => "IDLE", PubSt::STAGED => "STAGED", PubSt::REGISTERED => "REGISTERED", PubSt::SENDING => "SENDING", }
    }
    pub fn parse(s: &str) -> Option<PubSt> {
        match s { "IDLE" => Some(PubSt::IDLE), "STAGED" => Some(PubSt::STAGED), "REGISTERED" => Some(PubSt::REGISTERED), "SENDING" => Some(PubSt::SENDING), _ => None }
    }
}

/// 값 묶음 — 원천에 이름이 없어 쓰인 자리 이름을 붙였다
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PublishTracksAction {
    add,
    remove,
}
impl PublishTracksAction {
    pub const ALL: &'static [PublishTracksAction] = &[PublishTracksAction::add, PublishTracksAction::remove];
    pub fn as_str(self) -> &'static str {
        match self { PublishTracksAction::add => "add", PublishTracksAction::remove => "remove", }
    }
    pub fn parse(s: &str) -> Option<PublishTracksAction> {
        match s { "add" => Some(PublishTracksAction::add), "remove" => Some(PublishTracksAction::remove), _ => None }
    }
}

/// 값 묶음 — 원천에 이름이 없어 쓰인 자리 이름을 붙였다
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PublishTracksDuplex {
    half,
    full,
}
impl PublishTracksDuplex {
    pub const ALL: &'static [PublishTracksDuplex] = &[PublishTracksDuplex::half, PublishTracksDuplex::full];
    pub fn as_str(self) -> &'static str {
        match self { PublishTracksDuplex::half => "half", PublishTracksDuplex::full => "full", }
    }
    pub fn parse(s: &str) -> Option<PublishTracksDuplex> {
        match s { "half" => Some(PublishTracksDuplex::half), "full" => Some(PublishTracksDuplex::full), _ => None }
    }
}

/// 이 영역의 효과(앱 알림 등)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Fx {
    #[cfg_attr(feature = "serde", serde(rename = "app.duplex"))] app_duplex,
    #[cfg_attr(feature = "serde", serde(rename = "app.duplex_failed"))] app_duplex_failed,
    #[cfg_attr(feature = "serde", serde(rename = "app.publish_failed"))] app_publish_failed,
    #[cfg_attr(feature = "serde", serde(rename = "app.publishing"))] app_publishing,
    #[cfg_attr(feature = "serde", serde(rename = "app.revoked"))] app_revoked,
    #[cfg_attr(feature = "serde", serde(rename = "track.attach"))] track_attach,
    #[cfg_attr(feature = "serde", serde(rename = "track.detach"))] track_detach,
    #[cfg_attr(feature = "serde", serde(rename = "transceiver.inactive"))] transceiver_inactive,
}
impl Fx {
    pub const ALL: &'static [Fx] = &[Fx::app_duplex, Fx::app_duplex_failed, Fx::app_publish_failed, Fx::app_publishing, Fx::app_revoked, Fx::track_attach, Fx::track_detach, Fx::transceiver_inactive];
    pub fn as_str(self) -> &'static str {
        match self { Fx::app_duplex => "app.duplex", Fx::app_duplex_failed => "app.duplex_failed", Fx::app_publish_failed => "app.publish_failed", Fx::app_publishing => "app.publishing", Fx::app_revoked => "app.revoked", Fx::track_attach => "track.attach", Fx::track_detach => "track.detach", Fx::transceiver_inactive => "transceiver.inactive", }
    }
    pub fn parse(s: &str) -> Option<Fx> {
        match s { "app.duplex" => Some(Fx::app_duplex), "app.duplex_failed" => Some(Fx::app_duplex_failed), "app.publish_failed" => Some(Fx::app_publish_failed), "app.publishing" => Some(Fx::app_publishing), "app.revoked" => Some(Fx::app_revoked), "track.attach" => Some(Fx::track_attach), "track.detach" => Some(Fx::track_detach), "transceiver.inactive" => Some(Fx::transceiver_inactive), _ => None }
    }
}

/// 다른 영역으로 건너가는 효과(이음매)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SeamFx {
    #[cfg_attr(feature = "serde", serde(rename = "link.pub_lost"))] link_pub_lost,
    #[cfg_attr(feature = "serde", serde(rename = "sdp.rebuild"))] sdp_rebuild,
}
impl SeamFx {
    pub const ALL: &'static [SeamFx] = &[SeamFx::link_pub_lost, SeamFx::sdp_rebuild];
    pub fn as_str(self) -> &'static str {
        match self { SeamFx::link_pub_lost => "link.pub_lost", SeamFx::sdp_rebuild => "sdp.rebuild", }
    }
    pub fn parse(s: &str) -> Option<SeamFx> {
        match s { "link.pub_lost" => Some(SeamFx::link_pub_lost), "sdp.rebuild" => Some(SeamFx::sdp_rebuild), _ => None }
    }
}

/// 타이머
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimerName {}
impl TimerName {
    pub const ALL: &'static [TimerName] = &[];
    pub fn as_str(self) -> &'static str {
        match self { }
    }
    pub fn parse(s: &str) -> Option<TimerName> {
        let _ = s;
        None
    }
}

/// 레코드 — 모형 자리(열쇠)를 문자열로 넓힌 것 · 차례 고정
pub type Rec<V> = std::collections::BTreeMap<String, V>;
fn rec<V: Clone>(kv: &[(&str, V)]) -> Rec<V> {
    kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// 실패 응답에 없는 칸 — 모형은 빈 글자로 싣는다 → None
#[cfg(feature = "serde")]
fn empty_none<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    let v = <serde_json::Value as serde::Deserialize>::deserialize(d)?;
    if v.is_null() || v.as_str() == Some("") { return Ok(None) }
    serde_json::from_value(v).map(Some).map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PubResMsg {
    #[cfg_attr(feature = "serde", serde(default, deserialize_with = "empty_none"))]
    pub action: Option<PubAction>,
    pub code: i64,
    pub part: Part,
    pub pid: i64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RevokedMsg {
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SetDuplexResMsg {
    pub code: i64,
    #[cfg_attr(feature = "serde", serde(default, deserialize_with = "empty_none"))]
    pub duplex: Option<Duplex>,
    pub noop: bool,
    pub part: Part,
}

/// 클라 상태 — 원천 PUB K
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct St {
    /// ★이 트랙이 붙은 미디어 서버 · 발행한 방 — 앱이 트랙을 만들 때 정하는 개체 신원(0 = 모형의 개체 하나 · 202610/20261013a 축 B)
    pub sfu: i64,
    /// undefined
    pub room: i64,
    /// 발행 상태(§[RULE-PUB-STATES])
    pub ps: PubSt,
    /// 앱이 켜 두려 한다(카메라 · 마이크를 켰다)
    pub want: bool,
    /// 반이중 트랙이다 — 발언권을 쥔 동안에만 SENDING(§[RULE-PUB-STATES]). TRACK_SET{duplex} 응답이 바꾼다(RULE-PUB-SET-PROC · 202611/20261102e)
    pub half: bool,
    /// 발행 협상이 진행 중이다(브라우저 — §[PUB.C.IDLE.on] · §[PUB.C.STAGED.nego_ok])
    pub nego: bool,
    /// ★서버를 건너는 발행 방 전환으로 옛 서버에서 내렸다 — 발행 방이 서면 그 서버에서 다시 켠다(RULE-AFF-SWITCH-TRACKS 서버 건너 행 · 202611/20261102e)
    pub mv: bool,
    /// TRACK_SET{duplex} 응답을 기다린다 — 응답 전엔 게이트 · 반이중 여부를 바꾸지 않는다(RULE-PUB-SET-PROC · 202611/20261102e)
    pub dset: bool,
    /// ★TRACK_SET{duplex} 응답을 기다리는 사이 발행한 방을 잃었다 — 남길지(반이중) 내릴지(전이중)를 그 응답이 정한다(PUB.C.any.room_gone_pending · 202611/20261104a G)
    pub lost: bool,
    /// 기다리는 PUBLISH_TRACKS{add} 요청의 pid(0 = 없다) — 그 요청의 응답만 적용한다(LINK req 와 같은 꼴 · 202611/20261104a)
    pub req: i64,
    /// 마지막으로 매긴 add 요청 pid
    pub rid: i64,
}
pub fn initial() -> St {
    St { sfu: 0, room: 0, ps: PubSt::IDLE, want: false, half: false, nego: false, mv: false, dset: false, lost: false, req: 0, rid: 0 }
}

/// 이 상태기가 내는 것
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Tx {
    PublishTracks { action: PublishTracksAction, #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))] duplex: Option<PublishTracksDuplex> },
    TrackSet { duplex: Duplex },
}
impl Tx {
    pub fn kind(&self) -> &'static str {
        match self {
            Tx::PublishTracks { .. } => "ws",
            Tx::TrackSet { .. } => "ws",
        }
    }
    pub fn op(&self) -> &'static str {
        match self {
            Tx::PublishTracks { .. } => "PUBLISH_TRACKS",
            Tx::TrackSet { .. } => "TRACK_SET",
        }
    }
    pub fn part(&self) -> &'static str {
        match self {
            Tx::PublishTracks { .. } => "req",
            Tx::TrackSet { .. } => "req",
        }
    }
}
/// 다른 영역으로 — 받는 영역(to) · 받는 개체(at: all = 그 영역 개체 전부 · self = 같은 개체 · sameServer · sameRoom)
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Seam {
    pub fx: SeamFx,
    pub to: &'static str,
    pub at: &'static str,
}
/// 한 걸음의 결과 — label 은 원천의 동작 라벨
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Step {
    pub s: St,
    pub label: &'static str,
    pub out: Vec<Tx>,
    pub fx: Vec<Fx>,
    pub seams: Vec<Seam>,
    pub start: Vec<TimerName>,
}

/// 이음매 — (효과, 받는 영역, 받는 절차) · 위가 정한다(IR seams)
pub const SEAMS: &[(SeamFx, &str, &[&str])] = &[(SeamFx::link_pub_lost, "LINK", &["CPubLost"]), (SeamFx::sdp_rebuild, "LINK", &["CRenego"])];
/// 이음매가 부르는 입구 — (입구, 부르는 절차) · 아래가 넣는 사건이 아니다
pub const REACTS: &[(&str, &[&str])] = &[("nego", &["LINK:pub.nego_ok", "LINK:pub.nego_fail"]), ("revoked", &["TRK:pub.revoked"]), ("reregister", &["LINK:pub.reregister_server", "SES:pub.reregister_all"]), ("floor_open", &["FLR:mic.open"]), ("floor_close", &["FLR:mic.close"]), ("resume_check", &["SES:pub.resume_check"]), ("drop_server", &["LINK:pub.drop_server"]), ("room_gone", &["ROOM:pub.room_gone", "ROOM:pub.room_left"]), ("move_out", &["AFF:pub.move_out"]), ("move_in", &["AFF:pub.move_in"])];
/// 읽는 것 — (입구.인자, 읽는 다른 영역 칸) · 부르는 쪽이 넣는다
pub const READS: &[(&str, &[&str])] = &[("move_out.here", &["AFF.pub", "AFF.want", "ROOM.sfu"])];

/// 원장 — 타이머(ms) · 원장에 값이 없으면 None
pub fn timer_ms(t: TimerName) -> Option<i64> {
    match t {}
}
/// 원장 — 횟수
#[allow(non_upper_case_globals)]
pub mod counts {
}

fn done(prev: &St, s: St, label: &'static str, out: Vec<Tx>, fx: Vec<Fx>, seams: Vec<Seam>, mut start: Vec<TimerName>) -> Step {
    let before = armed(prev);
    for t in armed(&s) {
        if !before.contains(&t) && !start.contains(&t) {
            start.push(t);
        }
    }
    Step { s, label, out, fx, seams, start }
}

/// 지금 돌아야 하는 타이머 — 원천 timer(…).armed
pub fn armed(s: &St) -> Vec<TimerName> {
    let mut on = Vec::new();
    on
}

/// 앱이 카메라 · 마이크를 켰다(§[PUB.C.IDLE.on]) (앱: 앱이 카메라 · 마이크를 켰다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn on(s: &St) -> Option<Step> {
    if !(s.ps == PubSt::IDLE) {
        return None;
    }
    if s.mv {
        return Some(done(s, St { want: true, ..s.clone() }, "PUB.C.IDLE.on_moving", vec![], vec![], vec![], vec![]));
    }
    Some(done(s, St { ps: PubSt::STAGED, want: true, nego: true, ..s.clone() }, "PUB.C.IDLE.on", vec![], vec![], vec![Seam { fx: SeamFx::sdp_rebuild, to: "LINK", at: "sameServer" }], vec![]))
}

/// 발행 협상이 끝났다(§[PUB.C.STAGED.nego_ok]) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn nego(s: &St, ok: bool) -> Option<Step> {
    if !(s.ps == PubSt::STAGED && s.nego) {
        return None;
    }
    if ok && !s.want {
        return Some(done(s, St { ps: PubSt::IDLE, nego: false, ..s.clone() }, "PUB.C.STAGED.nego_ok_unwanted", vec![], vec![], vec![], vec![]));
    }
    if ok {
        return Some(done(s, St { nego: false, rid: s.rid + 1, req: s.rid + 1, ..s.clone() }, "PUB.C.STAGED.nego_ok", vec![Tx::PublishTracks { action: PublishTracksAction::add, duplex: Some(if s.half { PublishTracksDuplex::half } else { PublishTracksDuplex::full }) }], vec![], vec![], vec![]));
    }
    Some(done(s, St { ps: PubSt::IDLE, nego: false, ..s.clone() }, "PUB.C.STAGED.nego_fail", vec![], vec![Fx::app_publish_failed], vec![], vec![]))
}

/// PUBLISH_TRACKS 응답을 받았다(§[PUB.C.STAGED.pub_ok] · §[RULE-PUB-REMOVE-FORM]) (받음: PUBLISH_TRACKS)
pub fn pub_res(s: &St, m: &PubResMsg) -> Step {
    if s.ps != PubSt::STAGED || s.nego || m.part == Part::res && m.action == Some(PubAction::remove) || m.code == 3005 || m.pid != s.req {
        return done(s, s.clone(), "PUB.C.any.pub_res_other", vec![], vec![], vec![], vec![]);
    }
    if m.part == Part::res && !s.want {
        return done(s, St { ps: PubSt::IDLE, ..s.clone() }, "PUB.C.STAGED.pub_ok_unwanted", vec![Tx::PublishTracks { action: PublishTracksAction::remove, duplex: None }], vec![], vec![], vec![]);
    }
    if m.part == Part::res {
        return done(s, St { ps: PubSt::REGISTERED, ..s.clone() }, "PUB.C.STAGED.pub_ok", vec![], vec![], vec![], vec![]);
    }
    done(s, St { ps: PubSt::IDLE, ..s.clone() }, "PUB.C.STAGED.pub_fail", vec![], vec![Fx::app_publish_failed], vec![], vec![])
}

/// RTP 를 시작했다 — 실제 트랙을 트랜시버에 붙였다(§[PUB.C.REGISTERED]) (바깥: 실제 트랙을 트랜시버에 붙였다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn attach(s: &St, ok: bool) -> Option<Step> {
    if !(s.ps == PubSt::REGISTERED) {
        return None;
    }
    if ok && s.half {
        return Some(done(s, s.clone(), "PUB.C.REGISTERED.attach_ok_half", vec![], vec![Fx::app_publishing], vec![], vec![]));
    }
    if ok {
        return Some(done(s, St { ps: PubSt::SENDING, ..s.clone() }, "PUB.C.REGISTERED.attach_ok", vec![], vec![Fx::track_attach, Fx::app_publishing], vec![], vec![]));
    }
    Some(done(s, St { ps: PubSt::IDLE, ..s.clone() }, "PUB.C.REGISTERED.attach_fail", vec![Tx::PublishTracks { action: PublishTracksAction::remove, duplex: None }], vec![Fx::app_publish_failed], vec![], vec![]))
}

/// 앱이 껐다(§[RULE-PUB-OFF]) (앱: 앱이 껐다)
pub fn off(s: &St) -> Step {
    if s.ps == PubSt::SENDING {
        return done(s, St { ps: PubSt::IDLE, want: false, ..s.clone() }, "PUB.C.SENDING.off", vec![Tx::PublishTracks { action: PublishTracksAction::remove, duplex: None }], vec![Fx::track_detach], vec![], vec![]);
    }
    if s.ps == PubSt::REGISTERED {
        return done(s, St { ps: PubSt::IDLE, want: false, ..s.clone() }, "PUB.C.REGISTERED.off", vec![Tx::PublishTracks { action: PublishTracksAction::remove, duplex: None }], vec![], vec![], vec![]);
    }
    if s.ps == PubSt::STAGED {
        return done(s, St { want: false, ..s.clone() }, "PUB.C.STAGED.off", vec![], vec![], vec![], vec![]);
    }
    done(s, St { want: false, ..s.clone() }, "PUB.C.IDLE.off", vec![], vec![], vec![], vec![])
}

/// 내 track_id 의 TRACK_EVENT{remove} — 서버가 내 발행을 지웠다(§[RULE-PUB-REVOKED])
pub fn revoked(s: &St, m: &RevokedMsg) -> Step {
    if s.ps == PubSt::SENDING {
        return done(s, St { ps: PubSt::IDLE, ..s.clone() }, "PUB.C.SENDING.revoked", vec![], vec![Fx::track_detach, Fx::transceiver_inactive, Fx::app_revoked], vec![], vec![]);
    }
    if s.ps == PubSt::REGISTERED {
        return done(s, St { ps: PubSt::IDLE, ..s.clone() }, "PUB.C.REGISTERED.revoked", vec![], vec![Fx::track_detach, Fx::transceiver_inactive, Fx::app_revoked], vec![], vec![]);
    }
    done(s, s.clone(), "PUB.C.any.revoked_other", vec![], vec![], vec![], vec![])
}

/// 그 서버 재구축 — 다시 발행한다(RULE-LINK-RECOVERY-REBUILD · 절차는 §[PUB.C.IDLE.on])
pub fn reregister(s: &St) -> Step {
    if s.mv {
        return done(s, s.clone(), "PUB.C.any.reregister_moving", vec![], vec![], vec![], vec![]);
    }
    if s.want {
        return done(s, St { ps: PubSt::STAGED, nego: true, ..s.clone() }, "PUB.C.any.reregister", vec![], vec![], vec![Seam { fx: SeamFx::sdp_rebuild, to: "LINK", at: "sameServer" }], vec![]);
    }
    done(s, St { ps: PubSt::IDLE, nego: false, ..s.clone() }, "PUB.C.any.reregister_off", vec![], vec![], vec![], vec![])
}

/// 발언권을 얻었다 — 반이중 트랙이 보내기 시작한다(§[RULE-PUB-STATES])
pub fn floor_open(s: &St) -> Step {
    if s.half && s.ps == PubSt::REGISTERED {
        return done(s, St { ps: PubSt::SENDING, ..s.clone() }, "PUB.C.REGISTERED.floor_open", vec![], vec![Fx::track_attach], vec![], vec![]);
    }
    done(s, s.clone(), "PUB.C.any.floor_open_other", vec![], vec![], vec![], vec![])
}

/// 발언권을 놓았다 · 잃었다 — 반이중 트랙이 보내기를 멈춘다(§[RULE-PUB-STATES])
pub fn floor_close(s: &St) -> Step {
    if s.half && s.ps == PubSt::SENDING {
        return done(s, St { ps: PubSt::REGISTERED, ..s.clone() }, "PUB.C.SENDING.floor_close", vec![], vec![Fx::track_detach], vec![], vec![]);
    }
    done(s, s.clone(), "PUB.C.any.floor_close_other", vec![], vec![], vec![], vec![])
}

/// RESUME 성공 — 응답의 publications[] 에 내 발행이 있나(listed) · 있으면 서버가 아는 반이중 여부(shalf — 그 항목의 duplex)를 본다(RULE-SES-RES-RESUME-OK 2 ③ · 4 · RULE-TRN-REQ-ABANDON 의 TRACK_SET 행) · 끊긴 동안 사라진 요청 · 응답도 이것으로 잡는다
pub fn resume_check(s: &St, listed: bool, shalf: bool) -> Step {
    if s.lost && (s.ps == PubSt::REGISTERED || s.ps == PubSt::SENDING) && !listed {
        return done(s, St { lost: false, ps: PubSt::IDLE, nego: false, ..s.clone() }, "PUB.C.any.resume_lost_drop", vec![], vec![Fx::track_detach], vec![], vec![]);
    }
    if s.lost && !s.dset && (s.ps == PubSt::REGISTERED || s.ps == PubSt::SENDING) {
        return done(s, St { lost: false, half: shalf, ps: if shalf { PubSt::REGISTERED } else { s.ps }, ..s.clone() }, "PUB.C.any.resume_lost_keep", vec![], vec![Fx::track_detach, Fx::app_duplex], vec![], vec![]);
    }
    if (s.ps == PubSt::REGISTERED || s.ps == PubSt::SENDING) && !listed {
        return done(s, s.clone(), "PUB.C.any.resume_unlisted", vec![], vec![], vec![Seam { fx: SeamFx::link_pub_lost, to: "LINK", at: "sameServer" }], vec![]);
    }
    if (s.ps == PubSt::REGISTERED || s.ps == PubSt::SENDING) && shalf && !s.half {
        return done(s, St { half: true, ps: PubSt::REGISTERED, ..s.clone() }, "PUB.C.any.resume_half", vec![], vec![Fx::track_detach, Fx::app_duplex], vec![], vec![]);
    }
    if (s.ps == PubSt::REGISTERED || s.ps == PubSt::SENDING) && !shalf && s.half {
        return done(s, St { half: false, ps: PubSt::SENDING, ..s.clone() }, "PUB.C.any.resume_full", vec![], vec![Fx::track_attach, Fx::app_duplex], vec![], vec![]);
    }
    if s.ps == PubSt::STAGED && !s.nego {
        return done(s, s.clone(), "PUB.C.STAGED.resume_pending", vec![], vec![], vec![], vec![]);
    }
    if (s.ps == PubSt::IDLE || s.ps == PubSt::STAGED && s.nego) && listed {
        return done(s, s.clone(), "PUB.C.any.resume_orphan", vec![Tx::PublishTracks { action: PublishTracksAction::remove, duplex: None }], vec![], vec![], vec![]);
    }
    done(s, s.clone(), "PUB.C.any.resume_listed", vec![], vec![], vec![], vec![])
}

/// 그 서버 연결이 닫혔다 — 서버가 Peer 와 함께 발행 트랙을 풀었다(RULE-ROOM-LEAVE-RES 2)
pub fn drop_server(s: &St) -> Step {
    if s.ps == PubSt::STAGED {
        return done(s, St { ps: PubSt::IDLE, nego: false, ..s.clone() }, "PUB.C.STAGED.drop_server", vec![], vec![Fx::app_publish_failed], vec![], vec![]);
    }
    done(s, St { ps: PubSt::IDLE, nego: false, ..s.clone() }, "PUB.C.any.drop_server", vec![], vec![], vec![], vec![])
}

/// 발행한 방을 잃었다(퇴장 · 강퇴 · 방 닫힘 · 스냅샷에 없는 방) — 서버가 그 방의 전이중 발행을 정리했다(원칙 ③) · self = 클라가 떠나기 시작했다(pub.room_left — 서버는 먼저 보낸 요청을 처리한 뒤 떠남을 처리한다) · 아니면 서버가 먼저 정리했고 클라가 나중에 안다
pub fn room_gone(s: &St, self_: bool) -> Step {
    if self_ && s.dset && (s.ps == PubSt::REGISTERED || s.ps == PubSt::SENDING) {
        return done(s, St { lost: true, ..s.clone() }, "PUB.C.any.room_gone_pending", vec![], vec![], vec![], vec![]);
    }
    if !self_ && s.dset && (s.ps == PubSt::REGISTERED || s.ps == PubSt::SENDING) {
        return done(s, St { lost: true, ..s.clone() }, "PUB.C.any.room_gone_kicked_pending", vec![], vec![], vec![], vec![]);
    }
    if !s.half && (s.ps == PubSt::REGISTERED || s.ps == PubSt::SENDING) {
        return done(s, St { ps: PubSt::IDLE, nego: false, lost: false, ..s.clone() }, "PUB.C.any.room_gone", vec![], vec![Fx::track_detach], vec![], vec![]);
    }
    if !s.half && s.ps == PubSt::STAGED {
        return done(s, St { ps: PubSt::IDLE, nego: false, ..s.clone() }, "PUB.C.STAGED.room_gone", vec![], vec![Fx::app_publish_failed], vec![], vec![]);
    }
    done(s, St { lost: false, ..s.clone() }, "PUB.C.any.room_gone_other", vec![], vec![], vec![], vec![])
}

/// 서버를 건너는 전환 ① — 옛 서버의 내 발행 트랙을 내린다(PUBLISH_TRACKS{remove}) · 읽는 것: here = AFF.pub · AFF.want · ROOM.sfu
pub fn move_out(s: &St, here: bool) -> Step {
    if !here {
        return done(s, s.clone(), "PUB.C.any.move_out_other", vec![], vec![], vec![], vec![]);
    }
    if s.ps == PubSt::SENDING {
        return done(s, St { ps: PubSt::IDLE, mv: true, ..s.clone() }, "PUB.C.SENDING.move_out", vec![Tx::PublishTracks { action: PublishTracksAction::remove, duplex: None }], vec![Fx::track_detach], vec![], vec![]);
    }
    if s.ps == PubSt::REGISTERED {
        return done(s, St { ps: PubSt::IDLE, mv: true, ..s.clone() }, "PUB.C.REGISTERED.move_out", vec![Tx::PublishTracks { action: PublishTracksAction::remove, duplex: None }], vec![], vec![], vec![]);
    }
    if s.ps == PubSt::STAGED && !s.nego {
        return done(s, St { ps: PubSt::IDLE, mv: true, ..s.clone() }, "PUB.C.STAGED.move_out_sent", vec![Tx::PublishTracks { action: PublishTracksAction::remove, duplex: None }], vec![], vec![], vec![]);
    }
    if s.ps == PubSt::STAGED {
        return done(s, St { ps: PubSt::IDLE, nego: false, mv: true, ..s.clone() }, "PUB.C.STAGED.move_out_nego", vec![], vec![], vec![], vec![]);
    }
    done(s, St { mv: s.want, ..s.clone() }, "PUB.C.IDLE.move_out", vec![], vec![], vec![], vec![])
}

/// 서버를 건너는 전환 ② · ③ — 발행 방이 섰다 · 내린 트랙을 그 서버에서 다시 켠다(처음 켤 때와 같은 절차 — §[PUB.C.IDLE.on] · 그 서버엔 내 업링크가 없어 재협상한다 §[RULE-PUB-2PC-1PC])
pub fn move_in(s: &St) -> Step {
    if s.mv && s.want {
        return done(s, St { mv: false, ps: PubSt::STAGED, nego: true, ..s.clone() }, "PUB.C.any.move_in", vec![], vec![], vec![Seam { fx: SeamFx::sdp_rebuild, to: "LINK", at: "sameServer" }], vec![]);
    }
    if s.mv {
        return done(s, St { mv: false, ..s.clone() }, "PUB.C.any.move_in_off", vec![], vec![], vec![], vec![]);
    }
    done(s, s.clone(), "PUB.C.any.move_in_other", vec![], vec![], vec![], vec![])
}

/// 응답 없이 버려진 발행 · 전환 요청 — 상태를 맞추고 앱에 알린다(§[RULE-TRN-REQ-ABANDON]) · remove 는 응답을 기다리지 않으니 버릴 것이 없다 (바깥: 이 트랙의 발행 · 전환 요청이 기한(T-req)을 넘겼다 — 응답 없이 버린다(운반 · RULE-TRN-REQ-LIFE 5)) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn req_expired(s: &St) -> Option<Step> {
    if !(s.ps == PubSt::STAGED && !s.nego || s.dset) {
        return None;
    }
    if s.ps == PubSt::STAGED && !s.nego {
        return Some(done(s, St { ps: PubSt::IDLE, req: 0, ..s.clone() }, "PUB.C.STAGED.expired", vec![], vec![Fx::app_publish_failed], vec![], vec![]));
    }
    if s.lost {
        return Some(done(s, St { dset: false, ..s.clone() }, "PUB.C.any.expired_lost", vec![], vec![Fx::app_duplex_failed], vec![], vec![]));
    }
    Some(done(s, St { dset: false, lost: false, ..s.clone() }, "PUB.C.any.expired_set", vec![], vec![Fx::app_duplex_failed], vec![], vec![]))
}

/// 무전 ↔ 회의 전환을 보낸다(TRACK_SET{duplex} — 이 경로로만 · RULE-PUB-SET-PROC) · 응답 전엔 아무것도 바꾸지 않는다 (앱: 앱이 트랙을 반이중 · 전이중으로 바꾸려 한다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn set_duplex(s: &St, d: Duplex) -> Option<Step> {
    if !((s.ps == PubSt::REGISTERED || s.ps == PubSt::SENDING) && !s.dset && !s.lost) {
        return None;
    }
    Some(done(s, St { dset: true, ..s.clone() }, "PUB.C.any.set_duplex", vec![Tx::TrackSet { duplex: d }], vec![], vec![], vec![]))
}

/// TRACK_SET{duplex} 응답을 받았다 — 적용됐으면 게이트 · 반이중 여부를 바꾼다(RULE-PUB-SET-PROC) (받음: TRACK_SET)
pub fn set_duplex_res(s: &St, m: &SetDuplexResMsg) -> Step {
    if !s.dset {
        return done(s, s.clone(), "PUB.C.any.set_res_other", vec![], vec![], vec![], vec![]);
    }
    if s.ps != PubSt::REGISTERED && s.ps != PubSt::SENDING {
        return done(s, St { dset: false, lost: false, ..s.clone() }, "PUB.C.any.set_res_gone", vec![], vec![Fx::app_duplex_failed], vec![], vec![]);
    }
    if s.lost && (m.part == Part::res && m.duplex == Some(Duplex::full) || m.part != Part::res && (!s.half || m.code == 3005)) {
        return done(s, St { dset: false, lost: false, ps: PubSt::IDLE, nego: false, ..s.clone() }, "PUB.C.any.set_lost_drop", vec![], vec![Fx::track_detach, Fx::app_duplex_failed], vec![], vec![]);
    }
    if m.part != Part::res {
        return done(s, St { dset: false, lost: false, ..s.clone() }, "PUB.C.any.set_fail", vec![], vec![Fx::app_duplex_failed], vec![], vec![]);
    }
    if m.noop && (m.duplex == Some(Duplex::half)) == s.half {
        return done(s, St { dset: false, lost: false, ..s.clone() }, "PUB.C.any.set_noop", vec![], vec![Fx::app_duplex], vec![], vec![]);
    }
    if m.duplex == Some(Duplex::half) && s.ps == PubSt::SENDING {
        return done(s, St { dset: false, lost: false, half: true, ps: PubSt::REGISTERED, ..s.clone() }, "PUB.C.SENDING.set_half", vec![], vec![Fx::track_detach, Fx::app_duplex], vec![], vec![]);
    }
    if m.duplex == Some(Duplex::half) && s.ps == PubSt::REGISTERED {
        return done(s, St { dset: false, lost: false, half: true, ..s.clone() }, "PUB.C.REGISTERED.set_half", vec![], vec![Fx::app_duplex], vec![], vec![]);
    }
    if s.half && s.ps == PubSt::REGISTERED {
        return done(s, St { dset: false, lost: false, half: false, ps: PubSt::SENDING, ..s.clone() }, "PUB.C.REGISTERED.set_full", vec![], vec![Fx::track_attach, Fx::app_duplex], vec![], vec![]);
    }
    done(s, St { dset: false, lost: false, half: false, ..s.clone() }, "PUB.C.any.set_full", vec![], vec![Fx::app_duplex], vec![], vec![])
}

/// 벡터 시험 — TS 판 입구 이름과 JSON 인자로 부른다(oxpeer 1층 · TS 판 재생이 찾은 호출을 Rust 판에 넣어 맞댄다)
#[cfg(feature = "serde")]
pub fn call_json(entry: &str, s: &St, a: &[serde_json::Value]) -> Result<Option<Step>, String> {
    fn arg<T: serde::de::DeserializeOwned>(a: &[serde_json::Value], i: usize) -> Result<T, String> {
        serde_json::from_value(a.get(i).cloned().unwrap_or(serde_json::Value::Null)).map_err(|e| format!("인자 {i}: {e}"))
    }
    Ok(match entry {
        "on" => on(s),
        "nego" => nego(s, arg::<bool>(a, 0)?),
        "pubRes" => Some(pub_res(s, &arg::<PubResMsg>(a, 0)?)),
        "attach" => attach(s, arg::<bool>(a, 0)?),
        "off" => Some(off(s)),
        "revoked" => Some(revoked(s, &arg::<RevokedMsg>(a, 0)?)),
        "reregister" => Some(reregister(s)),
        "floorOpen" => Some(floor_open(s)),
        "floorClose" => Some(floor_close(s)),
        "resumeCheck" => Some(resume_check(s, arg::<bool>(a, 0)?, arg::<bool>(a, 1)?)),
        "dropServer" => Some(drop_server(s)),
        "roomGone" => Some(room_gone(s, arg::<bool>(a, 0)?)),
        "moveOut" => Some(move_out(s, arg::<bool>(a, 0)?)),
        "moveIn" => Some(move_in(s)),
        "reqExpired" => req_expired(s),
        "setDuplex" => set_duplex(s, arg::<Duplex>(a, 0)?),
        "setDuplexRes" => Some(set_duplex_res(s, &arg::<SetDuplexResMsg>(a, 0)?)),
        _ => return Err(format!("모르는 입구 {entry}")),
    })
}
