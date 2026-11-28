// ★생성 파일 — gen/axis_client.mts 가 원천(context/spec/protocol · areas/trk/behavior)에서 짓는다. 손으로 고치지 않는다.
// IR sha256 9f8284b8a4c4f8ec · 영역 TRK 클라 상태기 Rust 판 — 같은 생성기의 TS 판과 한 뿌리(같은 입구 · 같은 경우 · 같은 라벨).
// 집행은 부르는 쪽 몫이다 — out(보낼 것) · fx(이 영역의 효과) · seams(다른 영역 입구로 — 받는 곳은 위가 정했다) · start(새로 걸 타이머). armed() 밖의 타이머는 멈춘다.
// 모형 자리(트랙 · 배정표 사본의 열쇠)는 문자열이다 — 실제 값을 넣는 법은 결합이 정한다(SDK:RULE-MOD-BINDING-VALUES).
#![allow(dead_code, unused_variables, unused_mut, unused_parens, non_camel_case_types, clippy::all)]

/// 값 이름 Layer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Layer {
    NONE,
    ACTIVE,
    RESTING,
}
impl Layer {
    pub const ALL: &'static [Layer] = &[Layer::NONE, Layer::ACTIVE, Layer::RESTING];
    pub fn as_str(self) -> &'static str {
        match self { Layer::NONE => "NONE", Layer::ACTIVE => "ACTIVE", Layer::RESTING => "RESTING", }
    }
    pub fn parse(s: &str) -> Option<Layer> {
        match s { "NONE" => Some(Layer::NONE), "ACTIVE" => Some(Layer::ACTIVE), "RESTING" => Some(Layer::RESTING), _ => None }
    }
}

/// 값 이름 TrackAction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TrackAction {
    add,
    remove,
}
impl TrackAction {
    pub const ALL: &'static [TrackAction] = &[TrackAction::add, TrackAction::remove];
    pub fn as_str(self) -> &'static str {
        match self { TrackAction::add => "add", TrackAction::remove => "remove", }
    }
    pub fn parse(s: &str) -> Option<TrackAction> {
        match s { "add" => Some(TrackAction::add), "remove" => Some(TrackAction::remove), _ => None }
    }
}

/// 값 묶음 — 원천에 이름이 없어 쓰인 자리 이름을 붙였다
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TrackEventMsgT {
    TRACK_EVENT,
    TRACK_STATE,
}
impl TrackEventMsgT {
    pub const ALL: &'static [TrackEventMsgT] = &[TrackEventMsgT::TRACK_EVENT, TrackEventMsgT::TRACK_STATE];
    pub fn as_str(self) -> &'static str {
        match self { TrackEventMsgT::TRACK_EVENT => "TRACK_EVENT", TrackEventMsgT::TRACK_STATE => "TRACK_STATE", }
    }
    pub fn parse(s: &str) -> Option<TrackEventMsgT> {
        match s { "TRACK_EVENT" => Some(TrackEventMsgT::TRACK_EVENT), "TRACK_STATE" => Some(TrackEventMsgT::TRACK_STATE), _ => None }
    }
}

/// 이 영역의 효과(앱 알림 등)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Fx {
    #[cfg_attr(feature = "serde", serde(rename = "app.track_state"))] app_track_state,
    #[cfg_attr(feature = "serde", serde(rename = "app.tracks"))] app_tracks,
}
impl Fx {
    pub const ALL: &'static [Fx] = &[Fx::app_track_state, Fx::app_tracks];
    pub fn as_str(self) -> &'static str {
        match self { Fx::app_track_state => "app.track_state", Fx::app_tracks => "app.tracks", }
    }
    pub fn parse(s: &str) -> Option<Fx> {
        match s { "app.track_state" => Some(Fx::app_track_state), "app.tracks" => Some(Fx::app_tracks), _ => None }
    }
}

/// 다른 영역으로 건너가는 효과(이음매)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SeamFx {
    #[cfg_attr(feature = "serde", serde(rename = "pub.revoked"))] pub_revoked,
}
impl SeamFx {
    pub const ALL: &'static [SeamFx] = &[SeamFx::pub_revoked];
    pub fn as_str(self) -> &'static str {
        match self { SeamFx::pub_revoked => "pub.revoked", }
    }
    pub fn parse(s: &str) -> Option<SeamFx> {
        match s { "pub.revoked" => Some(SeamFx::pub_revoked), _ => None }
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

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TrackEventMsgTracks {
    pub active: bool,
    pub muted: bool,
    pub track_id: String,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TrackEventMsg {
    pub action: TrackAction,
    pub t: TrackEventMsgT,
    pub tracks: TrackEventMsgTracks,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TrackStateMsg {
    pub muted: bool,
    pub t: TrackEventMsgT,
    pub track_id: String,
}

/// 클라 상태 — 원천 TRK K
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct St {
    /// 트랙 층 — 받은 항목으로 통째 교체한다 · ★받을 트랙만 — 내 개인 트랙은 넣지 않는다(델타 길 mine_other · 스냅샷은 내 것을 빼고 넘긴다 — 202611/20261113j ③-2)
    pub lay: Rec<Layer>,
    /// 트랙 층의 muted — TRACK_STATE 와 항목 교체가 고친다
    pub muted: Rec<bool>,
}
pub fn initial() -> St {
    St { lay: rec(&[("A", Layer::NONE), ("B", Layer::NONE), ("S", Layer::ACTIVE)]), muted: rec(&[("A", false), ("B", false), ("S", false)]) }
}

/// 이 상태기가 내는 것
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Tx {}
impl Tx {
    pub fn kind(&self) -> &'static str {
        match *self {}
    }
    pub fn op(&self) -> &'static str {
        match *self {}
    }
    pub fn part(&self) -> &'static str {
        match *self {}
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
pub const SEAMS: &[(SeamFx, &str, &[&str])] = &[(SeamFx::pub_revoked, "PUB", &["CRevoked"])];
/// 이음매가 부르는 입구 — (입구, 부르는 절차) · 아래가 넣는 사건이 아니다
pub const REACTS: &[(&str, &[&str])] = &[("track_event", &["ROOM:trk.delta"]), ("track_state", &["ROOM:trk.delta"]), ("snapshot", &["ROOM:trk.snapshot"]), ("store_drop", &["ROOM:store.drop"])];
/// 읽는 것 — (입구.인자, 읽는 다른 영역 칸) · 부르는 쪽이 넣는다
pub const READS: &[(&str, &[&str])] = &[];

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

/// TRACK_EVENT 를 받았다 — 트랙 목록(보기용)을 고친다(RULE-TRK-STORE-UPDATE · 키 = track_id) · mine = 내 track_id(★ROOM 이 받은 델타만 온다 — version 판정은 ROOM 한 곳 · trk.delta · RULE-TRK-TRACK-EVENT-PROC) · ★배정표는 건드리지 않는다 — 받는 상태 밖이면 None(사건이 없던 것)
pub fn track_event(s: &St, m: &TrackEventMsg, mine: bool) -> Option<Step> {
    if !(m.t == TrackEventMsgT::TRACK_EVENT) {
        return None;
    }
    if mine && m.action == TrackAction::remove {
        return Some(done(s, s.clone(), "TRK.C.any.mine_removed", vec![], vec![], vec![Seam { fx: SeamFx::pub_revoked, to: "PUB", at: "sameRoom" }], vec![]));
    }
    if mine {
        return Some(done(s, s.clone(), "TRK.C.any.mine_other", vec![], vec![], vec![], vec![]));
    }
    if m.action == TrackAction::remove {
        return Some(done(s, St { lay: { let mut r = s.lay.clone(); r.insert(m.tracks.track_id.clone(), Layer::NONE); r }, muted: { let mut r = s.muted.clone(); r.insert(m.tracks.track_id.clone(), false); r }, ..s.clone() }, "TRK.C.any.remove", vec![], vec![Fx::app_tracks], vec![], vec![]));
    }
    Some(done(s, St { lay: { let mut r = s.lay.clone(); r.insert(m.tracks.track_id.clone(), if m.tracks.active { Layer::ACTIVE } else { Layer::RESTING }); r }, muted: { let mut r = s.muted.clone(); r.insert(m.tracks.track_id.clone(), m.tracks.muted); r }, ..s.clone() }, "TRK.C.any.track_event", vec![], vec![Fx::app_tracks], vec![], vec![]))
}

/// TRACK_STATE 를 받았다(★ROOM 이 받은 델타만 온다 — trk.delta) — track_id 가 식별 키(RULE-TRK-TRACK-STATE-PROC) · muted 는 트랙 층이라 보관본의 그 값이 바뀐다(아직 구독하지 않은 트랙도 · RULE-TRK-TRACK-STATE-TO-ALL) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn track_state(s: &St, m: &TrackStateMsg) -> Option<Step> {
    if !(m.t == TrackEventMsgT::TRACK_STATE) {
        return None;
    }
    Some(done(s, St { muted: { let mut r = s.muted.clone(); r.insert(m.track_id.clone(), m.muted); r }, ..s.clone() }, "TRK.C.any.track_state", vec![], vec![Fx::app_track_state], vec![], vec![]))
}

/// 스냅샷(입장 응답 · RESUME · GET)의 tracks 를 받았다 — 그 방 트랙 목록 전부다(보기용 · RULE-TRK-FOUR-PATHS · RULE-TRK-PER-PATH) · ★sLay · sMuted 는 내 개인 트랙을 뺀 것이다 — 받을 트랙이 아니다(델타 길 mine_other 와 같은 보관본 · 202611/20261113j ③-2) · ★배정표는 실려 오지 않는다(입장 응답 · RESUME 의 표는 MLINE_EVENT 와 같은 길로 받는다)
pub fn snapshot(s: &St, s_lay: &Rec<Layer>, s_muted: &Rec<bool>) -> Step {
    if *s_lay != s.lay || *s_muted != s.muted {
        return done(s, St { lay: s_lay.clone(), muted: s_muted.clone(), ..s.clone() }, "TRK.C.any.snapshot", vec![], vec![Fx::app_tracks], vec![], vec![]);
    }
    done(s, s.clone(), "TRK.C.any.snapshot_same", vec![], vec![], vec![], vec![])
}

/// 그 방을 떠났다 · 내렸다 — 그 방 트랙 목록 보관본을 버린다(RULE-ROOM-LEAVE-RES) · 배정표는 연결의 것이라 서버가 새 표(MLINE_EVENT)로 고친다
pub fn store_drop(s: &St) -> Step {
    done(s, St { lay: rec(&[("A", Layer::NONE), ("B", Layer::NONE), ("S", Layer::NONE)]), muted: rec(&[("A", false), ("B", false), ("S", false)]), ..s.clone() }, "TRK.C.any.store_drop", vec![], vec![Fx::app_tracks], vec![], vec![])
}

/// 벡터 시험 — TS 판 입구 이름과 JSON 인자로 부른다(oxpeer 1층 · TS 판 재생이 찾은 호출을 Rust 판에 넣어 맞댄다)
#[cfg(feature = "serde")]
pub fn call_json(entry: &str, s: &St, a: &[serde_json::Value]) -> Result<Option<Step>, String> {
    fn arg<T: serde::de::DeserializeOwned>(a: &[serde_json::Value], i: usize) -> Result<T, String> {
        serde_json::from_value(a.get(i).cloned().unwrap_or(serde_json::Value::Null)).map_err(|e| format!("인자 {i}: {e}"))
    }
    Ok(match entry {
        "trackEvent" => track_event(s, &arg::<TrackEventMsg>(a, 0)?, arg::<bool>(a, 1)?),
        "trackState" => track_state(s, &arg::<TrackStateMsg>(a, 0)?),
        "snapshot" => Some(snapshot(s, &arg::<Rec<Layer>>(a, 0)?, &arg::<Rec<bool>>(a, 1)?)),
        "storeDrop" => Some(store_drop(s)),
        _ => return Err(format!("모르는 입구 {entry}")),
    })
}
