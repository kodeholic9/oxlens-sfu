// ★생성 파일 — gen/axis_client.mts 가 원천(context/spec/protocol · areas/aff/behavior)에서 짓는다. 손으로 고치지 않는다.
// IR sha256 207c0df6ea211dc9 · 영역 AFF 클라 상태기 Rust 판 — 같은 생성기의 TS 판과 한 뿌리(같은 입구 · 같은 경우 · 같은 라벨).
// 집행은 부르는 쪽 몫이다 — out(보낼 것) · fx(이 영역의 효과) · seams(다른 영역 입구로 — 받는 곳은 위가 정했다) · start(새로 걸 타이머). armed() 밖의 타이머는 멈춘다.
// 모형 자리(트랙 · 배정표 사본의 열쇠)는 문자열이다 — 실제 값을 넣는 법은 결합이 정한다(SDK:RULE-MOD-BINDING-VALUES).
#![allow(dead_code, unused_variables, unused_mut, unused_parens, non_camel_case_types, clippy::all)]

/// 값 이름 AffWait
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AffWait {
    NONE,
    SELECT,
    DESELECT,
    JOIN,
}
impl AffWait {
    pub const ALL: &'static [AffWait] = &[AffWait::NONE, AffWait::SELECT, AffWait::DESELECT, AffWait::JOIN];
    pub fn as_str(self) -> &'static str {
        match self { AffWait::NONE => "NONE", AffWait::SELECT => "SELECT", AffWait::DESELECT => "DESELECT", AffWait::JOIN => "JOIN", }
    }
    pub fn parse(s: &str) -> Option<AffWait> {
        match s { "NONE" => Some(AffWait::NONE), "SELECT" => Some(AffWait::SELECT), "DESELECT" => Some(AffWait::DESELECT), "JOIN" => Some(AffWait::JOIN), _ => None }
    }
}

/// 값 이름 RoomSt
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RoomSt {
    NONE,
    JOINING,
    JOINED,
    LEAVING,
}
impl RoomSt {
    pub const ALL: &'static [RoomSt] = &[RoomSt::NONE, RoomSt::JOINING, RoomSt::JOINED, RoomSt::LEAVING];
    pub fn as_str(self) -> &'static str {
        match self { RoomSt::NONE => "NONE", RoomSt::JOINING => "JOINING", RoomSt::JOINED => "JOINED", RoomSt::LEAVING => "LEAVING", }
    }
    pub fn parse(s: &str) -> Option<RoomSt> {
        match s { "NONE" => Some(RoomSt::NONE), "JOINING" => Some(RoomSt::JOINING), "JOINED" => Some(RoomSt::JOINED), "LEAVING" => Some(RoomSt::LEAVING), _ => None }
    }
}

/// 이 영역의 효과(앱 알림 등)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Fx {
    #[cfg_attr(feature = "serde", serde(rename = "app.aff_failed"))] app_aff_failed,
    #[cfg_attr(feature = "serde", serde(rename = "app.affiliation"))] app_affiliation,
}
impl Fx {
    pub const ALL: &'static [Fx] = &[Fx::app_aff_failed, Fx::app_affiliation];
    pub fn as_str(self) -> &'static str {
        match self { Fx::app_aff_failed => "app.aff_failed", Fx::app_affiliation => "app.affiliation", }
    }
    pub fn parse(s: &str) -> Option<Fx> {
        match s { "app.aff_failed" => Some(Fx::app_aff_failed), "app.affiliation" => Some(Fx::app_affiliation), _ => None }
    }
}

/// 다른 영역으로 건너가는 효과(이음매)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SeamFx {
    #[cfg_attr(feature = "serde", serde(rename = "floor.pub_away"))] floor_pub_away,
    #[cfg_attr(feature = "serde", serde(rename = "floor.pub_select"))] floor_pub_select,
    #[cfg_attr(feature = "serde", serde(rename = "pub.move_in"))] pub_move_in,
    #[cfg_attr(feature = "serde", serde(rename = "pub.move_out"))] pub_move_out,
}
impl SeamFx {
    pub const ALL: &'static [SeamFx] = &[SeamFx::floor_pub_away, SeamFx::floor_pub_select, SeamFx::pub_move_in, SeamFx::pub_move_out];
    pub fn as_str(self) -> &'static str {
        match self { SeamFx::floor_pub_away => "floor.pub_away", SeamFx::floor_pub_select => "floor.pub_select", SeamFx::pub_move_in => "pub.move_in", SeamFx::pub_move_out => "pub.move_out", }
    }
    pub fn parse(s: &str) -> Option<SeamFx> {
        match s { "floor.pub_away" => Some(SeamFx::floor_pub_away), "floor.pub_select" => Some(SeamFx::floor_pub_select), "pub.move_in" => Some(SeamFx::pub_move_in), "pub.move_out" => Some(SeamFx::pub_move_out), _ => None }
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

/// 클라 상태 — 원천 AFF K
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct St {
    /// 세션의 발행 방 — 방 하나 또는 없음(응답이 그렇게 말했다 — 직접 고치지 않는다 · RULE-AFF-PROC)
    pub r#pub: String,
    /// 기다리는 응답 — 요청은 한 번에 하나(서버를 건너면 옛 서버 해제 → 응답 → 새 서버 선택 · RULE-AFF-SWITCH-TRACKS)
    pub wait: AffWait,
    /// 옮겨 갈 방 — 서버를 건너는 전환에서 옛 서버 해제 응답을 기다리는 동안
    pub want: String,
    /// 서버를 건너는 전환의 옛 발행 방 — 새 서버 선택이 실패하면 되돌아갈 곳(RULE-AFF-SWITCH-TRACKS ③ · 202611/20261102e)
    pub from: String,
    /// 재구축으로 내린 발행 방 — 다시 들어간 뒤 pub_select 로 되찾는다(RULE-LINK-RECOVERY-REBUILD · 202611/20261102e)
    pub back: String,
}
pub fn initial() -> St {
    St { r#pub: "NONE".to_string(), wait: AffWait::NONE, want: "NONE".to_string(), from: "NONE".to_string(), back: "NONE".to_string() }
}

/// 이 상태기가 내는 것
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Tx {
    Affiliation { #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))] pub_select: Option<String>, #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))] pub_deselect: Option<String> },
}
impl Tx {
    pub fn kind(&self) -> &'static str {
        match self {
            Tx::Affiliation { .. } => "ws",
        }
    }
    pub fn op(&self) -> &'static str {
        match self {
            Tx::Affiliation { .. } => "AFFILIATION",
        }
    }
    pub fn part(&self) -> &'static str {
        match self {
            Tx::Affiliation { .. } => "req",
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
pub const SEAMS: &[(SeamFx, &str, &[&str])] = &[(SeamFx::floor_pub_away, "FLR", &["CAway"]), (SeamFx::floor_pub_select, "FLR", &["CPubSel"]), (SeamFx::pub_move_in, "PUB", &["CMoveIn"]), (SeamFx::pub_move_out, "PUB", &["CMoveOut"])];
/// 이음매가 부르는 입구 — (입구, 부르는 절차) · 아래가 넣는 사건이 아니다
pub const REACTS: &[(&str, &[&str])] = &[("implicit", &["ROOM:aff.implicit"]), ("left", &["ROOM:aff.leave"]), ("down", &["ROOM:aff.down"]), ("join_failed", &["ROOM:aff.join_failed"])];
/// 읽는 것 — (입구.인자, 읽는 다른 영역 칸) · 부르는 쪽이 넣는다
pub const READS: &[(&str, &[&str])] = &[("res.p_in", &["ROOM.rs"]), ("implicit.p_in", &["ROOM.rs"])];

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

/// 방 r 을 발행 방으로(§[RULE-AFF-CHANGE-PROC] — 같은 서버면 요청 하나 · 서버를 건너면 옛 서버 해제 먼저 · RULE-AFF-SWITCH-TRACKS) (앱: 앱이 이 방에서 말하려 한다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn select(s: &St, r: &String, same: bool) -> Option<Step> {
    if !(s.wait == AffWait::NONE && s.r#pub != *r) {
        return None;
    }
    if s.r#pub == "NONE" {
        return Some(done(s, St { wait: AffWait::SELECT, want: r.clone(), back: "NONE".to_string(), ..s.clone() }, "AFF.C.NOPUB.select", vec![Tx::Affiliation { pub_select: Some(r.clone()), pub_deselect: None }], vec![], vec![], vec![]));
    }
    if same {
        return Some(done(s, St { wait: AffWait::SELECT, want: r.clone(), back: "NONE".to_string(), ..s.clone() }, "AFF.C.PUB.switch_same", vec![Tx::Affiliation { pub_select: Some(r.clone()), pub_deselect: Some(s.r#pub.clone()) }], vec![], vec![], vec![]));
    }
    Some(done(s, St { wait: AffWait::DESELECT, want: r.clone(), from: s.r#pub.clone(), ..s.clone() }, "AFF.C.PUB.switch_cross", vec![Tx::Affiliation { pub_select: None, pub_deselect: Some(s.r#pub.clone()) }], vec![], vec![Seam { fx: SeamFx::pub_move_out, to: "PUB", at: "all" }], vec![]))
}

/// 발행 방을 뺀다(§[RULE-AFF-CHANGE-PROC]) (앱: 앱이 발행을 그만둔다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn deselect(s: &St) -> Option<Step> {
    if !(s.wait == AffWait::NONE && s.r#pub != "NONE") {
        return None;
    }
    Some(done(s, St { wait: AffWait::DESELECT, want: "NONE".to_string(), back: "NONE".to_string(), ..s.clone() }, "AFF.C.PUB.deselect", vec![Tx::Affiliation { pub_select: None, pub_deselect: Some(s.r#pub.clone()) }], vec![], vec![], vec![]))
}

/// AFFILIATION 응답 — ok = 성공 응답 · p = 응답한 서버가 아는 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub · onWant = 응답한 서버가 지금 발행 방 · 옮겨 갈 방의 서버다(§[RULE-AFF-CHANGE-PROC] 3 · 4) (받음: AFFILIATION) · 읽는 것: pIn = ROOM.rs
pub fn res(s: &St, ok: bool, p: &String, on_pub: bool, on_want: bool, p_in: bool) -> Step {
    if !ok && s.wait == AffWait::SELECT && s.from != "NONE" {
        return done(s, St { want: s.from.clone(), from: "NONE".to_string(), ..s.clone() }, "AFF.C.PUB.switch_back", vec![Tx::Affiliation { pub_select: Some(s.from.clone()), pub_deselect: None }], vec![Fx::app_aff_failed], vec![], vec![]);
    }
    if !ok && s.wait == AffWait::DESELECT && s.want != "NONE" {
        return done(s, St { wait: AffWait::NONE, want: "NONE".to_string(), from: "NONE".to_string(), ..s.clone() }, "AFF.C.PUB.switch_out_fail", vec![], vec![Fx::app_aff_failed], vec![Seam { fx: SeamFx::pub_move_in, to: "PUB", at: "all" }], vec![]);
    }
    if !ok {
        return done(s, St { wait: AffWait::NONE, want: "NONE".to_string(), from: "NONE".to_string(), ..s.clone() }, "AFF.C.any.fail", vec![], vec![Fx::app_aff_failed], vec![], vec![]);
    }
    if *p != "NONE" && !p_in && s.from != "NONE" {
        return done(s, St { wait: AffWait::SELECT, want: s.from.clone(), from: "NONE".to_string(), ..s.clone() }, "AFF.C.PUB.res_gone_back", vec![Tx::Affiliation { pub_select: Some(s.from.clone()), pub_deselect: None }], vec![Fx::app_aff_failed], vec![], vec![]);
    }
    if *p != "NONE" && !p_in {
        return done(s, St { r#pub: "NONE".to_string(), wait: AffWait::NONE, want: "NONE".to_string(), ..s.clone() }, "AFF.C.any.res_gone", vec![], vec![Fx::app_affiliation, Fx::app_aff_failed], vec![Seam { fx: SeamFx::floor_pub_away, to: "FLR", at: "room" }], vec![]);
    }
    if s.wait == AffWait::DESELECT && s.want != "NONE" && !on_want && *p == "NONE" {
        return done(s, St { r#pub: "NONE".to_string(), wait: AffWait::SELECT, ..s.clone() }, "AFF.C.PUB.switch_in", vec![Tx::Affiliation { pub_select: Some(s.want.clone()), pub_deselect: None }], vec![Fx::app_affiliation], vec![Seam { fx: SeamFx::floor_pub_away, to: "FLR", at: "room" }], vec![]);
    }
    if s.wait == AffWait::DESELECT && s.want != "NONE" && !on_want {
        return done(s, St { wait: AffWait::NONE, want: "NONE".to_string(), from: "NONE".to_string(), ..s.clone() }, "AFF.C.PUB.switch_stuck", vec![], vec![Fx::app_aff_failed], vec![Seam { fx: SeamFx::pub_move_in, to: "PUB", at: "all" }], vec![]);
    }
    if s.wait == AffWait::SELECT && s.from != "NONE" && *p != s.want {
        return done(s, St { want: s.from.clone(), from: "NONE".to_string(), ..s.clone() }, "AFF.C.PUB.switch_in_skipped", vec![Tx::Affiliation { pub_select: Some(s.from.clone()), pub_deselect: None }], vec![Fx::app_aff_failed], vec![], vec![]);
    }
    if *p != "NONE" && s.r#pub == "NONE" {
        return done(s, St { r#pub: p.clone(), wait: AffWait::NONE, want: "NONE".to_string(), from: "NONE".to_string(), ..s.clone() }, "AFF.C.NOPUB.res_pub", vec![], vec![Fx::app_affiliation], vec![Seam { fx: SeamFx::floor_pub_select, to: "FLR", at: "room" }, Seam { fx: SeamFx::pub_move_out, to: "PUB", at: "all" }, Seam { fx: SeamFx::pub_move_in, to: "PUB", at: "all" }], vec![]);
    }
    if *p != "NONE" && s.r#pub != *p {
        return done(s, St { r#pub: p.clone(), wait: AffWait::NONE, want: "NONE".to_string(), from: "NONE".to_string(), ..s.clone() }, "AFF.C.PUB.res_moved", vec![], vec![Fx::app_affiliation], vec![Seam { fx: SeamFx::floor_pub_away, to: "FLR", at: "room" }, Seam { fx: SeamFx::floor_pub_select, to: "FLR", at: "room" }], vec![]);
    }
    if *p == "NONE" && on_pub && s.r#pub != "NONE" {
        return done(s, St { r#pub: "NONE".to_string(), wait: AffWait::NONE, want: "NONE".to_string(), from: "NONE".to_string(), ..s.clone() }, "AFF.C.PUB.res_away", vec![], vec![Fx::app_affiliation], vec![Seam { fx: SeamFx::floor_pub_away, to: "FLR", at: "room" }], vec![]);
    }
    done(s, St { wait: AffWait::NONE, want: "NONE".to_string(), from: "NONE".to_string(), ..s.clone() }, "AFF.C.any.res_same", vec![], vec![], vec![], vec![])
}

/// select:true 입장을 보낸다 — 입장이 곁들여 발행 방을 바꾸므로 ★소속 전환이 떠 있는 것과 같다(RULE-ROOM-JOIN-START 2 · 그 응답이 올 때까지 다른 전환을 막는다) (앱: 앱이 이 방에서 말하려고 들어간다(select:true 입장)) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn join_select(s: &St, r: &String) -> Option<Step> {
    if !(s.wait == AffWait::NONE && s.r#pub == "NONE") {
        return None;
    }
    Some(done(s, St { wait: AffWait::JOIN, want: r.clone(), back: "NONE".to_string(), ..s.clone() }, "AFF.C.NOPUB.join_select", vec![], vec![], vec![], vec![]))
}

/// 입장 응답의 affiliation — 입장이 곁들여 바꾼 발행 방(RULE-AFF-CAUSE-USER · cause 없음) · r = 그 입장의 방 · sel = 그 입장 요청이 select:true 였다(응답은 pid 로 제 요청과 짝지어진다 — RULE-TRN-ONE-REPLY) · p = 그 응답이 말한 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub = 지금 발행 방이 그 응답을 낸 서버의 방 · 읽는 것: pIn = ROOM.rs
pub fn implicit(s: &St, r: &String, sel: bool, p: &String, on_pub: bool, p_in: bool) -> Step {
    if s.back != "NONE" && s.back == *r && *p == *r {
        return done(s, St { r#pub: p.clone(), back: "NONE".to_string(), ..s.clone() }, "AFF.C.any.implicit_back_held", vec![], vec![Fx::app_affiliation], vec![Seam { fx: SeamFx::floor_pub_select, to: "FLR", at: "room" }], vec![]);
    }
    if s.back != "NONE" && s.back == *r && s.wait == AffWait::NONE {
        return done(s, St { back: "NONE".to_string(), wait: AffWait::SELECT, want: r.clone(), ..s.clone() }, "AFF.C.any.implicit_back", vec![Tx::Affiliation { pub_select: Some(r.clone()), pub_deselect: None }], vec![], vec![], vec![]);
    }
    if s.back != "NONE" && s.back == *r {
        return done(s, St { back: "NONE".to_string(), ..s.clone() }, "AFF.C.any.implicit_back_busy", vec![], vec![], vec![], vec![]);
    }
    if sel && s.wait == AffWait::JOIN && s.want == *r && *p == *r {
        return done(s, St { r#pub: p.clone(), wait: AffWait::NONE, want: "NONE".to_string(), ..s.clone() }, "AFF.C.any.implicit_joined", vec![], vec![Fx::app_affiliation], vec![Seam { fx: SeamFx::floor_pub_select, to: "FLR", at: "room" }, Seam { fx: SeamFx::pub_move_out, to: "PUB", at: "all" }, Seam { fx: SeamFx::pub_move_in, to: "PUB", at: "all" }], vec![]);
    }
    if sel && s.wait == AffWait::JOIN && s.want == *r {
        return done(s, St { wait: AffWait::NONE, want: "NONE".to_string(), ..s.clone() }, "AFF.C.any.implicit_join_unselected", vec![], vec![Fx::app_aff_failed], vec![], vec![]);
    }
    if s.wait != AffWait::NONE {
        return done(s, s.clone(), "AFF.C.any.implicit_busy", vec![], vec![], vec![], vec![]);
    }
    if *p != "NONE" && !p_in {
        return done(s, s.clone(), "AFF.C.any.implicit_gone", vec![], vec![], vec![], vec![]);
    }
    if *p != "NONE" && s.r#pub != *p {
        return done(s, St { r#pub: p.clone(), ..s.clone() }, "AFF.C.any.implicit_pub", vec![], vec![Fx::app_affiliation], vec![Seam { fx: SeamFx::floor_pub_away, to: "FLR", at: "room" }, Seam { fx: SeamFx::floor_pub_select, to: "FLR", at: "room" }], vec![]);
    }
    if *p == "NONE" && on_pub && s.r#pub != "NONE" {
        return done(s, St { r#pub: "NONE".to_string(), ..s.clone() }, "AFF.C.any.implicit_away", vec![], vec![Fx::app_affiliation], vec![Seam { fx: SeamFx::floor_pub_away, to: "FLR", at: "room" }], vec![]);
    }
    done(s, s.clone(), "AFF.C.any.implicit_same", vec![], vec![], vec![], vec![])
}

/// 듣던 방 r 을 떠났다 — 그 방이 발행 방이면 발행도 함께 풀린다(RULE-AFF-PROC 끝 항)
pub fn left(s: &St, r: &String) -> Step {
    if s.r#pub != "NONE" && s.r#pub == *r {
        return done(s, St { r#pub: "NONE".to_string(), from: if s.from == *r { "NONE".to_string() } else { s.from.clone() }, ..s.clone() }, "AFF.C.any.left", vec![], vec![Fx::app_affiliation], vec![], vec![]);
    }
    if s.back != "NONE" && s.back == *r {
        return done(s, St { back: "NONE".to_string(), ..s.clone() }, "AFF.C.any.left_back", vec![], vec![], vec![], vec![]);
    }
    if s.from != "NONE" && s.from == *r {
        return done(s, St { from: "NONE".to_string(), ..s.clone() }, "AFF.C.any.left_from", vec![], vec![], vec![], vec![]);
    }
    if s.wait == AffWait::JOIN && s.want == *r {
        return done(s, St { wait: AffWait::NONE, want: "NONE".to_string(), ..s.clone() }, "AFF.C.any.left_join", vec![], vec![Fx::app_aff_failed], vec![], vec![]);
    }
    if s.want != "NONE" && s.want == *r {
        return done(s, St { want: "NONE".to_string(), ..s.clone() }, "AFF.C.any.left_want", vec![], vec![], vec![], vec![]);
    }
    done(s, s.clone(), "AFF.C.any.left_other", vec![], vec![], vec![], vec![])
}

/// 재구축이 방 r 을 내렸다 — 다시 들어간다(RULE-LINK-RECOVERY-REBUILD · RULE-SES-REBUILD) · 떠남과 달리 발행 방은 다시 들어간 뒤 되찾는다
pub fn down(s: &St, r: &String) -> Step {
    if s.r#pub != "NONE" && s.r#pub == *r {
        return done(s, St { r#pub: "NONE".to_string(), back: r.clone(), ..s.clone() }, "AFF.C.any.down", vec![], vec![Fx::app_affiliation], vec![], vec![]);
    }
    if s.wait == AffWait::JOIN && s.want == *r {
        return done(s, St { wait: AffWait::NONE, want: "NONE".to_string(), back: r.clone(), ..s.clone() }, "AFF.C.any.down_join", vec![], vec![], vec![], vec![]);
    }
    if s.want != "NONE" && s.want == *r {
        return done(s, St { want: "NONE".to_string(), ..s.clone() }, "AFF.C.any.down_want", vec![], vec![], vec![], vec![]);
    }
    done(s, s.clone(), "AFF.C.any.down_other", vec![], vec![], vec![], vec![])
}

/// 방 r 입장이 실패했다(응답이 실패 · 응답의 pc_mode 불일치로 되돌림) — select:true 입장이 건 기다림이면 푼다(소속 요청의 실패 응답이 AFF.C.any.fail 로 푸는 것과 같은 꼴)
pub fn join_failed(s: &St, r: &String) -> Step {
    if s.back != "NONE" && s.back == *r {
        return done(s, St { back: "NONE".to_string(), ..s.clone() }, "AFF.C.any.join_failed_back", vec![], vec![], vec![], vec![]);
    }
    if s.wait == AffWait::JOIN && s.want == *r {
        return done(s, St { wait: AffWait::NONE, want: "NONE".to_string(), ..s.clone() }, "AFF.C.any.join_failed", vec![], vec![Fx::app_aff_failed], vec![], vec![]);
    }
    done(s, s.clone(), "AFF.C.any.join_failed_other", vec![], vec![], vec![], vec![])
}

/// 벡터 시험 — TS 판 입구 이름과 JSON 인자로 부른다(oxpeer 1층 · TS 판 재생이 찾은 호출을 Rust 판에 넣어 맞댄다)
#[cfg(feature = "serde")]
pub fn call_json(entry: &str, s: &St, a: &[serde_json::Value]) -> Result<Option<Step>, String> {
    fn arg<T: serde::de::DeserializeOwned>(a: &[serde_json::Value], i: usize) -> Result<T, String> {
        serde_json::from_value(a.get(i).cloned().unwrap_or(serde_json::Value::Null)).map_err(|e| format!("인자 {i}: {e}"))
    }
    Ok(match entry {
        "select" => select(s, &arg::<String>(a, 0)?, arg::<bool>(a, 1)?),
        "deselect" => deselect(s),
        "res" => Some(res(s, arg::<bool>(a, 0)?, &arg::<String>(a, 1)?, arg::<bool>(a, 2)?, arg::<bool>(a, 3)?, arg::<bool>(a, 4)?)),
        "joinSelect" => join_select(s, &arg::<String>(a, 0)?),
        "implicit" => Some(implicit(s, &arg::<String>(a, 0)?, arg::<bool>(a, 1)?, &arg::<String>(a, 2)?, arg::<bool>(a, 3)?, arg::<bool>(a, 4)?)),
        "left" => Some(left(s, &arg::<String>(a, 0)?)),
        "down" => Some(down(s, &arg::<String>(a, 0)?)),
        "joinFailed" => Some(join_failed(s, &arg::<String>(a, 0)?)),
        _ => return Err(format!("모르는 입구 {entry}")),
    })
}
