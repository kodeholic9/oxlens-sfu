// ★생성 파일 — gen/axis_client.mts 가 원천(context/spec/protocol · areas/sub/behavior)에서 짓는다. 손으로 고치지 않는다.
// IR sha256 9f8284b8a4c4f8ec · 영역 SUB 클라 상태기 Rust 판 — 같은 생성기의 TS 판과 한 뿌리(같은 입구 · 같은 경우 · 같은 라벨).
// 집행은 부르는 쪽 몫이다 — out(보낼 것) · fx(이 영역의 효과) · seams(다른 영역 입구로 — 받는 곳은 위가 정했다) · start(새로 걸 타이머). armed() 밖의 타이머는 멈춘다.
// 모형 자리(트랙 · 배정표 사본의 열쇠)는 문자열이다 — 실제 값을 넣는 법은 결합이 정한다(SDK:RULE-MOD-BINDING-VALUES).
#![allow(dead_code, unused_variables, unused_mut, unused_parens, non_camel_case_types, clippy::all)]

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

/// 값 이름 SubReady
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SubReady {
    NONE,
    WANT,
    SENT,
    OK,
}
impl SubReady {
    pub const ALL: &'static [SubReady] = &[SubReady::NONE, SubReady::WANT, SubReady::SENT, SubReady::OK];
    pub fn as_str(self) -> &'static str {
        match self { SubReady::NONE => "NONE", SubReady::WANT => "WANT", SubReady::SENT => "SENT", SubReady::OK => "OK", }
    }
    pub fn parse(s: &str) -> Option<SubReady> {
        match s { "NONE" => Some(SubReady::NONE), "WANT" => Some(SubReady::WANT), "SENT" => Some(SubReady::SENT), "OK" => Some(SubReady::OK), _ => None }
    }
}

/// 이 영역의 효과(앱 알림 등)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Fx {
    #[cfg_attr(feature = "serde", serde(rename = "app.bug"))] app_bug,
}
impl Fx {
    pub const ALL: &'static [Fx] = &[Fx::app_bug];
    pub fn as_str(self) -> &'static str {
        match self { Fx::app_bug => "app.bug", }
    }
    pub fn parse(s: &str) -> Option<Fx> {
        match s { "app.bug" => Some(Fx::app_bug), _ => None }
    }
}

/// 다른 영역으로 건너가는 효과(이음매)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SeamFx {
    #[cfg_attr(feature = "serde", serde(rename = "sdp.rebuild"))] sdp_rebuild,
}
impl SeamFx {
    pub const ALL: &'static [SeamFx] = &[SeamFx::sdp_rebuild];
    pub fn as_str(self) -> &'static str {
        match self { SeamFx::sdp_rebuild => "sdp.rebuild", }
    }
    pub fn parse(s: &str) -> Option<SeamFx> {
        match s { "sdp.rebuild" => Some(SeamFx::sdp_rebuild), _ => None }
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
pub struct ReadyResMsg {
    pub part: Part,
}

/// 클라 상태 — 원천 SUB K
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct St {
    /// 그 방의 READY(RULE-SUB-READY-PROC — 방 단위)
    pub ready: SubReady,
}
pub fn initial() -> St {
    St { ready: SubReady::NONE }
}

/// 이 상태기가 내는 것
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Tx {
    Ready,
    SubscribeLayer,
}
impl Tx {
    pub fn kind(&self) -> &'static str {
        match self {
            Tx::Ready => "ws",
            Tx::SubscribeLayer => "ws",
        }
    }
    pub fn op(&self) -> &'static str {
        match self {
            Tx::Ready => "READY",
            Tx::SubscribeLayer => "SUBSCRIBE_LAYER",
        }
    }
    pub fn part(&self) -> &'static str {
        match self {
            Tx::Ready => "req",
            Tx::SubscribeLayer => "req",
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
pub const SEAMS: &[(SeamFx, &str, &[&str])] = &[(SeamFx::sdp_rebuild, "LINK", &["CRenego"])];
/// 이음매가 부르는 입구 — (입구, 부르는 절차) · 아래가 넣는 사건이 아니다
pub const REACTS: &[(&str, &[&str])] = &[("ready", &["ROOM:ready.tracks"]), ("nego_ok", &["LINK:sub.nego_ok"]), ("nego_fail", &["LINK:sub.nego_fail"]), ("table_changed", &["LINK:sub.table_changed"]), ("table_rebound", &["LINK:sub.table_rebound"]), ("drop_room", &["ROOM:sub.drop"])];
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

/// 그 방에 READY 를 보낼 때가 됐다 — 입장 · 스냅샷 반영 뒤(RULE-SUB-READY-PROC)
pub fn ready(s: &St) -> Step {
    done(s, St { ready: SubReady::WANT, ..s.clone() }, "SUB.C.any.ready_want", vec![], vec![], vec![], vec![])
}

/// 그 서버 재협상이 끝났다 — 이번에 바뀐 방마다 READY(RULE-TRK-TRACK-EVENT-PROC)
pub fn nego_ok(s: &St) -> Step {
    if s.ready == SubReady::WANT {
        return done(s, St { ready: SubReady::SENT, ..s.clone() }, "SUB.C.WANT.nego_ok", vec![Tx::Ready], vec![], vec![], vec![]);
    }
    done(s, s.clone(), "SUB.C.any.nego_ok_other", vec![], vec![], vec![], vec![])
}

/// 협상이 실패했다 — READY 를 보내지 않는다(RULE-SUB-READY-PROC)
pub fn nego_fail(s: &St) -> Step {
    if s.ready == SubReady::WANT {
        return done(s, s.clone(), "SUB.C.WANT.nego_fail", vec![], vec![], vec![], vec![]);
    }
    done(s, s.clone(), "SUB.C.any.nego_fail_other", vec![], vec![], vec![], vec![])
}

/// READY 응답을 받았다(RULE-SUB-READY-FAIL) (받음: READY)
pub fn ready_res(s: &St, m: &ReadyResMsg) -> Step {
    if s.ready != SubReady::SENT {
        return done(s, s.clone(), "SUB.C.any.ready_res_old", vec![], vec![], vec![], vec![]);
    }
    if m.part == Part::res {
        return done(s, St { ready: SubReady::OK, ..s.clone() }, "SUB.C.SENT.ready_ok", vec![], vec![], vec![], vec![]);
    }
    done(s, St { ready: SubReady::NONE, ..s.clone() }, "SUB.C.SENT.ready_fail", vec![], vec![Fx::app_bug], vec![], vec![])
}

/// 그 서버의 배정표가 바뀌었다(MLINE_EVENT · 입장 응답 · RESUME) — 구독 SDP 를 그 표만으로 다시 짓고 협상이 끝나면 READY{arev}(RULE-SUB-TABLE-PROC)
pub fn table_changed(s: &St) -> Step {
    done(s, St { ready: SubReady::WANT, ..s.clone() }, "SUB.C.any.table_changed", vec![], vec![], vec![Seam { fx: SeamFx::sdp_rebuild, to: "LINK", at: "sameServer" }], vec![])
}

/// 그 서버의 배정표에서 줄은 그대로이고 실린 트랙만 바뀌었다(재배정 · 빈 줄 · 다시 채움 — RULE-SUB-FILL) — SDP 를 다시 짓지 않고 READY{arev}(RULE-SUB-TABLE-PROC)
pub fn table_rebound(s: &St) -> Step {
    if s.ready == SubReady::WANT {
        return done(s, s.clone(), "SUB.C.WANT.table_rebound", vec![], vec![], vec![], vec![]);
    }
    if s.ready == SubReady::NONE {
        return done(s, s.clone(), "SUB.C.NONE.table_rebound", vec![], vec![], vec![], vec![]);
    }
    done(s, St { ready: SubReady::SENT, ..s.clone() }, "SUB.C.any.table_rebound", vec![Tx::Ready], vec![], vec![], vec![])
}

/// SUBSCRIBE_LAYER 를 보낸다 — 지정이 아니라 상한 · 생략한 필드는 안 바꾼다(RULE-SUB-LAYER-PROC) (앱: 앱이 받을 레이어 상한을 고른다)
pub fn layer(s: &St) -> Step {
    done(s, s.clone(), "SUB.C.any.layer", vec![Tx::SubscribeLayer], vec![], vec![], vec![])
}

/// SUBSCRIBE_LAYER 응답 — 대상별 실패는 조용히 건너뛴다(RULE-SUB-LAYER-FAIL) (받음: SUBSCRIBE_LAYER)
pub fn layer_res(s: &St) -> Step {
    done(s, s.clone(), "SUB.C.any.layer_res", vec![], vec![], vec![], vec![])
}

/// 그 방을 떠났다 · 잃었다 — 그 방의 구독을 버린다
pub fn drop_room(s: &St) -> Step {
    done(s, St { ready: SubReady::NONE, ..s.clone() }, "SUB.C.any.drop", vec![], vec![], vec![], vec![])
}

/// 벡터 시험 — TS 판 입구 이름과 JSON 인자로 부른다(oxpeer 1층 · TS 판 재생이 찾은 호출을 Rust 판에 넣어 맞댄다)
#[cfg(feature = "serde")]
pub fn call_json(entry: &str, s: &St, a: &[serde_json::Value]) -> Result<Option<Step>, String> {
    fn arg<T: serde::de::DeserializeOwned>(a: &[serde_json::Value], i: usize) -> Result<T, String> {
        serde_json::from_value(a.get(i).cloned().unwrap_or(serde_json::Value::Null)).map_err(|e| format!("인자 {i}: {e}"))
    }
    Ok(match entry {
        "ready" => Some(ready(s)),
        "negoOk" => Some(nego_ok(s)),
        "negoFail" => Some(nego_fail(s)),
        "readyRes" => Some(ready_res(s, &arg::<ReadyResMsg>(a, 0)?)),
        "tableChanged" => Some(table_changed(s)),
        "tableRebound" => Some(table_rebound(s)),
        "layer" => Some(layer(s)),
        "layerRes" => Some(layer_res(s)),
        "dropRoom" => Some(drop_room(s)),
        _ => return Err(format!("모르는 입구 {entry}")),
    })
}
