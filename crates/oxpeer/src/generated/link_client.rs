// ★생성 파일 — gen/axis_client.mts 가 원천(context/spec/protocol · areas/link/behavior)에서 짓는다. 손으로 고치지 않는다.
// IR sha256 9f8284b8a4c4f8ec · 영역 LINK 클라 상태기 Rust 판 — 같은 생성기의 TS 판과 한 뿌리(같은 입구 · 같은 경우 · 같은 라벨).
// 집행은 부르는 쪽 몫이다 — out(보낼 것) · fx(이 영역의 효과) · seams(다른 영역 입구로 — 받는 곳은 위가 정했다) · start(새로 걸 타이머). armed() 밖의 타이머는 멈춘다.
// 모형 자리(트랙 · 배정표 사본의 열쇠)는 문자열이다 — 실제 값을 넣는 법은 결합이 정한다(SDK:RULE-MOD-BINDING-VALUES).
#![allow(dead_code, unused_variables, unused_mut, unused_parens, non_camel_case_types, clippy::all)]

/// 값 이름 Ice
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Ice {
    OK,
    DISC,
    FAILED,
}
impl Ice {
    pub const ALL: &'static [Ice] = &[Ice::OK, Ice::DISC, Ice::FAILED];
    pub fn as_str(self) -> &'static str {
        match self { Ice::OK => "OK", Ice::DISC => "DISC", Ice::FAILED => "FAILED", }
    }
    pub fn parse(s: &str) -> Option<Ice> {
        match s { "OK" => Some(Ice::OK), "DISC" => Some(Ice::DISC), "FAILED" => Some(Ice::FAILED), _ => None }
    }
}

/// 값 이름 LinkSt
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LinkSt {
    OK,
    WAIT,
    CHECK,
    RESTART,
    APPLY,
    REOPEN,
    REBUILD,
    NONE,
}
impl LinkSt {
    pub const ALL: &'static [LinkSt] = &[LinkSt::OK, LinkSt::WAIT, LinkSt::CHECK, LinkSt::RESTART, LinkSt::APPLY, LinkSt::REOPEN, LinkSt::REBUILD, LinkSt::NONE];
    pub fn as_str(self) -> &'static str {
        match self { LinkSt::OK => "OK", LinkSt::WAIT => "WAIT", LinkSt::CHECK => "CHECK", LinkSt::RESTART => "RESTART", LinkSt::APPLY => "APPLY", LinkSt::REOPEN => "REOPEN", LinkSt::REBUILD => "REBUILD", LinkSt::NONE => "NONE", }
    }
    pub fn parse(s: &str) -> Option<LinkSt> {
        match s { "OK" => Some(LinkSt::OK), "WAIT" => Some(LinkSt::WAIT), "CHECK" => Some(LinkSt::CHECK), "RESTART" => Some(LinkSt::RESTART), "APPLY" => Some(LinkSt::APPLY), "REOPEN" => Some(LinkSt::REOPEN), "REBUILD" => Some(LinkSt::REBUILD), "NONE" => Some(LinkSt::NONE), _ => None }
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

/// 값 이름 RebuildCause
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RebuildCause {
    NONE,
    RESTART_3XXX,
    ASSOC,
    DC_EXHAUST,
    EPOCH,
    PUB_LOST,
}
impl RebuildCause {
    pub const ALL: &'static [RebuildCause] = &[RebuildCause::NONE, RebuildCause::RESTART_3XXX, RebuildCause::ASSOC, RebuildCause::DC_EXHAUST, RebuildCause::EPOCH, RebuildCause::PUB_LOST];
    pub fn as_str(self) -> &'static str {
        match self { RebuildCause::NONE => "NONE", RebuildCause::RESTART_3XXX => "RESTART_3XXX", RebuildCause::ASSOC => "ASSOC", RebuildCause::DC_EXHAUST => "DC_EXHAUST", RebuildCause::EPOCH => "EPOCH", RebuildCause::PUB_LOST => "PUB_LOST", }
    }
    pub fn parse(s: &str) -> Option<RebuildCause> {
        match s { "NONE" => Some(RebuildCause::NONE), "RESTART_3XXX" => Some(RebuildCause::RESTART_3XXX), "ASSOC" => Some(RebuildCause::ASSOC), "DC_EXHAUST" => Some(RebuildCause::DC_EXHAUST), "EPOCH" => Some(RebuildCause::EPOCH), "PUB_LOST" => Some(RebuildCause::PUB_LOST), _ => None }
    }
}

/// 값 이름 Reopen
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Reopen {
    ok,
    fail,
    exhausted,
}
impl Reopen {
    pub const ALL: &'static [Reopen] = &[Reopen::ok, Reopen::fail, Reopen::exhausted];
    pub fn as_str(self) -> &'static str {
        match self { Reopen::ok => "ok", Reopen::fail => "fail", Reopen::exhausted => "exhausted", }
    }
    pub fn parse(s: &str) -> Option<Reopen> {
        match s { "ok" => Some(Reopen::ok), "fail" => Some(Reopen::fail), "exhausted" => Some(Reopen::exhausted), _ => None }
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
    #[cfg_attr(feature = "serde", serde(rename = "app.link_blocked"))] app_link_blocked,
    #[cfg_attr(feature = "serde", serde(rename = "app.link_ok"))] app_link_ok,
    #[cfg_attr(feature = "serde", serde(rename = "app.link_rebuilding"))] app_link_rebuilding,
    #[cfg_attr(feature = "serde", serde(rename = "app.link_reviving"))] app_link_reviving,
    #[cfg_attr(feature = "serde", serde(rename = "app.link_shaky"))] app_link_shaky,
    #[cfg_attr(feature = "serde", serde(rename = "conn.close"))] conn_close,
    #[cfg_attr(feature = "serde", serde(rename = "dc.reopen"))] dc_reopen,
    #[cfg_attr(feature = "serde", serde(rename = "sdp.restart"))] sdp_restart,
}
impl Fx {
    pub const ALL: &'static [Fx] = &[Fx::app_link_blocked, Fx::app_link_ok, Fx::app_link_rebuilding, Fx::app_link_reviving, Fx::app_link_shaky, Fx::conn_close, Fx::dc_reopen, Fx::sdp_restart];
    pub fn as_str(self) -> &'static str {
        match self { Fx::app_link_blocked => "app.link_blocked", Fx::app_link_ok => "app.link_ok", Fx::app_link_rebuilding => "app.link_rebuilding", Fx::app_link_reviving => "app.link_reviving", Fx::app_link_shaky => "app.link_shaky", Fx::conn_close => "conn.close", Fx::dc_reopen => "dc.reopen", Fx::sdp_restart => "sdp.restart", }
    }
    pub fn parse(s: &str) -> Option<Fx> {
        match s { "app.link_blocked" => Some(Fx::app_link_blocked), "app.link_ok" => Some(Fx::app_link_ok), "app.link_rebuilding" => Some(Fx::app_link_rebuilding), "app.link_reviving" => Some(Fx::app_link_reviving), "app.link_shaky" => Some(Fx::app_link_shaky), "conn.close" => Some(Fx::conn_close), "dc.reopen" => Some(Fx::dc_reopen), "sdp.restart" => Some(Fx::sdp_restart), _ => None }
    }
}

/// 다른 영역으로 건너가는 효과(이음매)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SeamFx {
    #[cfg_attr(feature = "serde", serde(rename = "floor.dc_down"))] floor_dc_down,
    #[cfg_attr(feature = "serde", serde(rename = "floor.dc_up"))] floor_dc_up,
    #[cfg_attr(feature = "serde", serde(rename = "floor.down_server"))] floor_down_server,
    #[cfg_attr(feature = "serde", serde(rename = "pub.drop_server"))] pub_drop_server,
    #[cfg_attr(feature = "serde", serde(rename = "pub.nego_fail"))] pub_nego_fail,
    #[cfg_attr(feature = "serde", serde(rename = "pub.nego_ok"))] pub_nego_ok,
    #[cfg_attr(feature = "serde", serde(rename = "pub.reregister_server"))] pub_reregister_server,
    #[cfg_attr(feature = "serde", serde(rename = "rooms.rejoin_server"))] rooms_rejoin_server,
    #[cfg_attr(feature = "serde", serde(rename = "store.drop_server"))] store_drop_server,
    #[cfg_attr(feature = "serde", serde(rename = "sub.nego_fail"))] sub_nego_fail,
    #[cfg_attr(feature = "serde", serde(rename = "sub.nego_ok"))] sub_nego_ok,
    #[cfg_attr(feature = "serde", serde(rename = "sub.table_changed"))] sub_table_changed,
    #[cfg_attr(feature = "serde", serde(rename = "sub.table_rebound"))] sub_table_rebound,
}
impl SeamFx {
    pub const ALL: &'static [SeamFx] = &[SeamFx::floor_dc_down, SeamFx::floor_dc_up, SeamFx::floor_down_server, SeamFx::pub_drop_server, SeamFx::pub_nego_fail, SeamFx::pub_nego_ok, SeamFx::pub_reregister_server, SeamFx::rooms_rejoin_server, SeamFx::store_drop_server, SeamFx::sub_nego_fail, SeamFx::sub_nego_ok, SeamFx::sub_table_changed, SeamFx::sub_table_rebound];
    pub fn as_str(self) -> &'static str {
        match self { SeamFx::floor_dc_down => "floor.dc_down", SeamFx::floor_dc_up => "floor.dc_up", SeamFx::floor_down_server => "floor.down_server", SeamFx::pub_drop_server => "pub.drop_server", SeamFx::pub_nego_fail => "pub.nego_fail", SeamFx::pub_nego_ok => "pub.nego_ok", SeamFx::pub_reregister_server => "pub.reregister_server", SeamFx::rooms_rejoin_server => "rooms.rejoin_server", SeamFx::store_drop_server => "store.drop_server", SeamFx::sub_nego_fail => "sub.nego_fail", SeamFx::sub_nego_ok => "sub.nego_ok", SeamFx::sub_table_changed => "sub.table_changed", SeamFx::sub_table_rebound => "sub.table_rebound", }
    }
    pub fn parse(s: &str) -> Option<SeamFx> {
        match s { "floor.dc_down" => Some(SeamFx::floor_dc_down), "floor.dc_up" => Some(SeamFx::floor_dc_up), "floor.down_server" => Some(SeamFx::floor_down_server), "pub.drop_server" => Some(SeamFx::pub_drop_server), "pub.nego_fail" => Some(SeamFx::pub_nego_fail), "pub.nego_ok" => Some(SeamFx::pub_nego_ok), "pub.reregister_server" => Some(SeamFx::pub_reregister_server), "rooms.rejoin_server" => Some(SeamFx::rooms_rejoin_server), "store.drop_server" => Some(SeamFx::store_drop_server), "sub.nego_fail" => Some(SeamFx::sub_nego_fail), "sub.nego_ok" => Some(SeamFx::sub_nego_ok), "sub.table_changed" => Some(SeamFx::sub_table_changed), "sub.table_rebound" => Some(SeamFx::sub_table_rebound), _ => None }
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
pub struct LinkEventMsg {
    pub ufrag: i64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HbResMsg {
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RestartResMsgPublish {
    pub ufrag: i64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RestartResMsg {
    pub code: i64,
    pub part: Part,
    pub pid: i64,
    pub publish: RestartResMsgPublish,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MlineEventMsgTable {
    pub arev: i64,
    pub mid: Rec<i64>,
    pub rows: i64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MlineEventMsg {
    pub table: MlineEventMsgTable,
}

/// 클라 상태 — 원천 LINK K
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct St {
    /// 복구 단계
    pub ls: LinkSt,
    /// ICE 층의 브라우저 사실
    pub ice: Ice,
    /// SCTP 연관 · DTLS 가 살아 있다
    pub assoc: bool,
    /// DC 채널이 열려 있다
    pub dc: bool,
    /// 적용한 ICE 자격의 세대(ufrag)
    pub r#gen: i64,
    /// ICE_RESTART 응답이 준 새 세대 — 재협상으로 적용 중
    pub pgen: i64,
    /// 지금 기다리는 ICE_RESTART 요청의 pid(0 = 없다) — 그 요청의 응답만 적용한다(§[RULE-PUB-REMOVE-FORM])
    pub req: i64,
    /// 마지막으로 매긴 요청 pid
    pub rid: i64,
    /// 끊긴 뒤 발언권을 다시 맞췄다(READY — §[RULE-LINK-RECOVERY-UNIT] 5)
    pub sync: bool,
    /// 재협상(구독 · 발행 SDP 다시 조립)이 진행 중이다(RULE-LINK-SDP-RENEGOTIATE)
    pub nego: bool,
    /// 협상 중에 방아쇠가 또 왔다 — 끝나면 한 번 더 조립한다(협상 큐 · 연달아 온 것은 한 번에 — RULE-LINK-SDP-RENEGOTIATE)
    pub dirty: bool,
    /// 이 연결이 한 번이라도 섰다(ICE 가 열렸다) — 막힌 망이면 서지 못한 채 되살리기가 되풀이된다(RULE-LINK-RECOVERY-NEVER-UP)
    #[cfg_attr(feature = "serde", serde(rename = "everUp"))]
    pub ever_up: bool,
    /// 재구축을 확정한 사건
    pub cause: RebuildCause,
    /// 배정표의 사본 — 그 트랙이 실린 줄의 mid(0 = 줄이 없다) · ★연결마다 하나(방 둘의 트랙이 한 표에) · MLINE_EVENT 와 입장 · RESUME 응답만 고친다(202610/20261017c · 20261018b)
    pub mid: Rec<i64>,
    /// 배정표의 SDP 재료 — 줄이 늘거나 줄 하나의 튜플이 바뀌면 바뀐다(같은지만 본다 · 모형은 줄이 늘 때만 바뀌는 수로 줄인다 — 튜플 바뀜도 SDP 를 바꾼다 §[SUB.type.MlineEntry])
    pub rows: i64,
    /// 받은 배정표의 차례 번호(arev) — 이보다 큰 표만 받는다
    pub arev: i64,
}
pub fn initial() -> St {
    St { ls: LinkSt::OK, ice: Ice::OK, assoc: true, dc: true, r#gen: 1, pgen: 0, req: 0, rid: 0, sync: true, nego: false, dirty: false, ever_up: true, cause: RebuildCause::NONE, mid: rec(&[("A", 0), ("B", 0), ("Sa", 1), ("Sb", 2)]), rows: 2, arev: 0 }
}

/// 이 상태기가 내는 것
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Tx {
    Ready,
    Heartbeat,
    IceRestart,
}
impl Tx {
    pub fn kind(&self) -> &'static str {
        match self {
            Tx::Ready => "ws",
            Tx::Heartbeat => "ws",
            Tx::IceRestart => "ws",
        }
    }
    pub fn op(&self) -> &'static str {
        match self {
            Tx::Ready => "READY",
            Tx::Heartbeat => "HEARTBEAT",
            Tx::IceRestart => "ICE_RESTART",
        }
    }
    pub fn part(&self) -> &'static str {
        match self {
            Tx::Ready => "req",
            Tx::Heartbeat => "req",
            Tx::IceRestart => "req",
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
pub const SEAMS: &[(SeamFx, &str, &[&str])] = &[(SeamFx::floor_dc_down, "FLR", &["CDcDown"]), (SeamFx::floor_dc_up, "FLR", &["CDcUp"]), (SeamFx::floor_down_server, "FLR", &["CDown"]), (SeamFx::pub_drop_server, "PUB", &["CDropServer"]), (SeamFx::pub_nego_fail, "PUB", &["CNego"]), (SeamFx::pub_nego_ok, "PUB", &["CNego"]), (SeamFx::pub_reregister_server, "PUB", &["CReregister"]), (SeamFx::rooms_rejoin_server, "ROOM", &["CRejoin"]), (SeamFx::store_drop_server, "ROOM", &["CDownAll"]), (SeamFx::sub_nego_fail, "SUB", &["CNegoFail"]), (SeamFx::sub_nego_ok, "SUB", &["CNegoOk"]), (SeamFx::sub_table_changed, "SUB", &["CTableChanged"]), (SeamFx::sub_table_rebound, "SUB", &["CTableRebound"])];
/// 이음매가 부르는 입구 — (입구, 부르는 절차) · 아래가 넣는 사건이 아니다
pub const REACTS: &[(&str, &[&str])] = &[("server_ice", &["ROOM:link.server_ice"]), ("close_all", &["SES:media.close_all"]), ("room_left", &["ROOM:link.room_left"]), ("repair", &["SES:media.repair"]), ("epoch_changed", &["ROOM:link.epoch_changed"]), ("pub_lost", &["PUB:link.pub_lost"]), ("renego", &["ROOM:sdp.rebuild", "PUB:sdp.rebuild", "SUB:sdp.rebuild"]), ("table_carried", &["ROOM:link.table"])];
/// 읽는 것 — (입구.인자, 읽는 다른 영역 칸) · 부르는 쪽이 넣는다
pub const READS: &[(&str, &[&str])] = &[("ice_disc.leaving", &["ROOM.rs"]), ("wait_over.leaving", &["ROOM.rs"]), ("assoc_lost.leaving", &["ROOM.rs"]), ("dc_closed.leaving", &["ROOM.rs"])];

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

/// ICE 가 disconnected 가 됐다(브라우저) (바깥: 브라우저 ICE 가 disconnected) · 읽는 것: leaving = ROOM.rs
pub fn ice_disc(s: &St, leaving: bool) -> Step {
    if leaving && s.ls != LinkSt::REBUILD {
        return done(s, s.clone(), "LINK.C.any.ice_disc_leaving", vec![], vec![], vec![], vec![]);
    }
    if s.ls == LinkSt::OK {
        return done(s, St { ls: LinkSt::WAIT, sync: false, ice: Ice::DISC, ..s.clone() }, "LINK.C.OK.ice_disc", vec![], vec![Fx::app_link_shaky], vec![], vec![]);
    }
    done(s, St { ice: Ice::DISC, ..s.clone() }, "LINK.C.any.ice_disc_absorbed", vec![], vec![], vec![], vec![])
}

/// ICE 가 스스로 회복했다(기다리는 동안) (바깥: 브라우저 ICE 가 스스로 회복했다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn ice_back(s: &St) -> Option<Step> {
    if !(s.ls == LinkSt::WAIT && s.ice == Ice::DISC) {
        return None;
    }
    if !s.dc {
        return Some(done(s, St { ls: LinkSt::REOPEN, ice: Ice::OK, ..s.clone() }, "LINK.C.WAIT.ice_back_dc", vec![], vec![Fx::dc_reopen], vec![], vec![]));
    }
    Some(done(s, St { ls: LinkSt::OK, sync: true, ice: Ice::OK, ..s.clone() }, "LINK.C.WAIT.ice_back", vec![Tx::Ready], vec![Fx::app_link_ok], vec![], vec![]))
}

/// disconnected 기다림이 끝나도 안 풀렸다 · 또는 failed 가 됐다 (바깥: disconnected 기다림이 끝났다 · failed) · 읽는 것: leaving = ROOM.rs
pub fn wait_over(s: &St, failed: bool, leaving: bool) -> Step {
    if leaving && s.ls != LinkSt::REBUILD {
        return done(s, s.clone(), "LINK.C.any.wait_over_leaving", vec![], vec![], vec![], vec![]);
    }
    if s.ls == LinkSt::WAIT {
        return done(s, St { ls: LinkSt::CHECK, sync: false, ice: if failed { Ice::FAILED } else { s.ice }, ..s.clone() }, "LINK.C.WAIT.wait_over", vec![Tx::Heartbeat], vec![], vec![], vec![]);
    }
    if s.ls == LinkSt::OK {
        return done(s, St { ls: LinkSt::CHECK, sync: false, ice: if failed { Ice::FAILED } else { s.ice }, ..s.clone() }, "LINK.C.OK.wait_over", vec![Tx::Heartbeat], vec![], vec![], vec![]);
    }
    if s.ls == LinkSt::REOPEN {
        return done(s, St { ls: LinkSt::CHECK, sync: false, ice: if failed { Ice::FAILED } else { s.ice }, ..s.clone() }, "LINK.C.REOPEN.wait_over", vec![Tx::Heartbeat], vec![], vec![], vec![]);
    }
    done(s, St { ice: if failed { Ice::FAILED } else { s.ice }, ..s.clone() }, "LINK.C.any.wait_over_absorbed", vec![], vec![], vec![], vec![])
}

/// LINK_EVENT{suspect} 를 받았다(§[RULE-ROOM-NOTIFY-VERSION-FOUR]) — 세대를 견준다 (받음: LINK_EVENT)
pub fn link_event(s: &St, m: &LinkEventMsg) -> Step {
    if m.ufrag != s.r#gen {
        return done(s, s.clone(), "LINK.C.any.link_event_stale", vec![], vec![], vec![], vec![]);
    }
    if s.ls == LinkSt::OK {
        return done(s, St { ls: LinkSt::CHECK, sync: false, ..s.clone() }, "LINK.C.OK.link_event", vec![Tx::Heartbeat], vec![], vec![], vec![]);
    }
    done(s, s.clone(), "LINK.C.any.link_event_absorbed", vec![], vec![], vec![], vec![])
}

/// 브라우저 `online` 사건 — 망이 바뀌었다(옛 TCP 가 반쯤 열린 채 남는다 · WS 도 죽었을 수 있다) (바깥: 브라우저 online 사건 — 망이 바뀌었다)
pub fn online(s: &St) -> Step {
    if s.ls == LinkSt::OK {
        return done(s, St { ls: LinkSt::CHECK, sync: false, ..s.clone() }, "LINK.C.OK.online", vec![Tx::Heartbeat], vec![], vec![], vec![]);
    }
    done(s, s.clone(), "LINK.C.any.online_absorbed", vec![], vec![], vec![], vec![])
}

/// WS 점검의 HEARTBEAT 응답 — 무엇이든 받았다(WS 가 산다) (받음: HEARTBEAT)
pub fn hb_res(s: &St, m: &HbResMsg) -> Step {
    if s.ls == LinkSt::CHECK {
        return done(s, St { ls: LinkSt::RESTART, rid: s.rid + 1, req: s.rid + 1, ..s.clone() }, "LINK.C.CHECK.ws_alive", vec![Tx::IceRestart], vec![Fx::app_link_reviving], vec![], vec![]);
    }
    done(s, s.clone(), "LINK.C.any.hb_res", vec![], vec![], vec![], vec![])
}

/// ICE_RESTART 응답을 받았다(§[RULE-PUB-REMOVE-FORM]) — 앞자리 뜻으로 받는다(§[RULE-ERR-UNKNOWN-BY-LEAD]) (받음: ICE_RESTART)
pub fn restart_res(s: &St, m: &RestartResMsg) -> Step {
    if s.ls != LinkSt::RESTART || m.pid != s.req {
        return done(s, s.clone(), "LINK.C.any.restart_res_other", vec![], vec![], vec![], vec![]);
    }
    if m.part == Part::res && s.nego {
        return done(s, St { ls: LinkSt::APPLY, pgen: m.publish.ufrag, req: 0, dirty: true, ..s.clone() }, "LINK.C.RESTART.restart_ok_queued", vec![], vec![], vec![], vec![]);
    }
    if m.part == Part::res {
        return done(s, St { ls: LinkSt::APPLY, pgen: m.publish.ufrag, req: 0, nego: true, ..s.clone() }, "LINK.C.RESTART.restart_ok", vec![], vec![Fx::sdp_restart], vec![], vec![]);
    }
    if m.code >= 3000 && m.code < 4000 {
        return done(s, St { ls: LinkSt::REBUILD, cause: RebuildCause::RESTART_3XXX, req: 0, ..s.clone() }, "LINK.C.RESTART.restart_3xxx", vec![], vec![Fx::conn_close, Fx::app_link_rebuilding], vec![Seam { fx: SeamFx::floor_dc_down, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::floor_down_server, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::store_drop_server, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::rooms_rejoin_server, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::pub_reregister_server, to: "PUB", at: "sameServer" }], vec![]);
    }
    done(s, St { ls: LinkSt::CHECK, req: 0, ..s.clone() }, "LINK.C.RESTART.restart_5xxx", vec![Tx::Heartbeat], vec![], vec![], vec![])
}

/// 새 자격을 적용했는데 시한 안에 안 열렸다(적용 뒤 시한 만료 — 방아쇠) (바깥: 새 자격 적용 뒤 시한이 지났다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn apply_timeout(s: &St) -> Option<Step> {
    if !(s.ls == LinkSt::APPLY) {
        return None;
    }
    if !s.ever_up {
        return Some(done(s, St { ls: LinkSt::CHECK, pgen: 0, ..s.clone() }, "LINK.C.APPLY.apply_timeout_blocked", vec![Tx::Heartbeat], vec![Fx::app_link_blocked], vec![], vec![]));
    }
    Some(done(s, St { ls: LinkSt::CHECK, pgen: 0, ..s.clone() }, "LINK.C.APPLY.apply_timeout", vec![Tx::Heartbeat], vec![], vec![], vec![]))
}

/// 새 자격의 재협상이 끝나 ICE 가 섰다 — 새 자격을 실은 협상이 끝난 뒤다(협상 큐 — RULE-LINK-SDP-RENEGOTIATE) (바깥: 새 자격의 재협상이 끝나 ICE 가 섰다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn applied(s: &St) -> Option<Step> {
    if !(s.ls == LinkSt::APPLY && !s.nego) {
        return None;
    }
    if !s.dc {
        return Some(done(s, St { ls: LinkSt::REOPEN, ice: Ice::OK, r#gen: s.pgen, pgen: 0, ever_up: true, ..s.clone() }, "LINK.C.APPLY.applied_dc", vec![], vec![Fx::dc_reopen], vec![], vec![]));
    }
    Some(done(s, St { ls: LinkSt::OK, ice: Ice::OK, r#gen: s.pgen, pgen: 0, sync: true, ever_up: true, ..s.clone() }, "LINK.C.APPLY.applied", vec![Tx::Ready], vec![Fx::app_link_ok], vec![], vec![]))
}

/// SCTP 연관이 사라졌다 · DTLS 가 닫혔다 (바깥: SCTP 연관 소멸 · DTLS 닫힘) · 읽는 것: leaving = ROOM.rs
pub fn assoc_lost(s: &St, leaving: bool) -> Step {
    if leaving && s.ls != LinkSt::REBUILD {
        return done(s, s.clone(), "LINK.C.any.assoc_lost_leaving", vec![], vec![], vec![], vec![]);
    }
    if s.ls == LinkSt::REBUILD {
        return done(s, St { assoc: false, dc: false, ..s.clone() }, "LINK.C.REBUILD.assoc_lost_absorbed", vec![], vec![], vec![], vec![]);
    }
    done(s, St { ls: LinkSt::REBUILD, cause: RebuildCause::ASSOC, assoc: false, dc: false, sync: false, pgen: 0, req: 0, ..s.clone() }, "LINK.C.any.assoc_lost", vec![], vec![Fx::conn_close, Fx::app_link_rebuilding], vec![Seam { fx: SeamFx::floor_dc_down, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::floor_down_server, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::store_drop_server, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::rooms_rejoin_server, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::pub_reregister_server, to: "PUB", at: "sameServer" }], vec![])
}

/// DC 채널만 닫혔다(연관은 산다) (바깥: DC 채널만 닫혔다) · 읽는 것: leaving = ROOM.rs
pub fn dc_closed(s: &St, leaving: bool) -> Step {
    if leaving && s.ls != LinkSt::REBUILD {
        return done(s, s.clone(), "LINK.C.any.dc_closed_leaving", vec![], vec![], vec![], vec![]);
    }
    if s.ls == LinkSt::OK {
        return done(s, St { ls: LinkSt::REOPEN, dc: false, sync: false, ..s.clone() }, "LINK.C.OK.dc_closed", vec![], vec![Fx::dc_reopen], vec![Seam { fx: SeamFx::floor_dc_down, to: "FLR", at: "sameServer" }], vec![]);
    }
    done(s, St { dc: false, ..s.clone() }, "LINK.C.any.dc_closed_absorbed", vec![], vec![], vec![Seam { fx: SeamFx::floor_dc_down, to: "FLR", at: "sameServer" }], vec![])
}

/// DC 다시 열기의 결과 (바깥: DC 다시 열기의 결과) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn reopened(s: &St, r: Reopen) -> Option<Step> {
    if !(s.ls == LinkSt::REOPEN) {
        return None;
    }
    if r == Reopen::ok {
        return Some(done(s, St { ls: LinkSt::OK, dc: true, sync: true, ..s.clone() }, "LINK.C.REOPEN.reopen_ok", vec![Tx::Ready], vec![Fx::app_link_ok], vec![Seam { fx: SeamFx::floor_dc_up, to: "FLR", at: "sameServer" }], vec![]));
    }
    if r == Reopen::fail {
        return Some(done(s, s.clone(), "LINK.C.REOPEN.reopen_retry", vec![], vec![Fx::dc_reopen], vec![], vec![]));
    }
    Some(done(s, St { ls: LinkSt::REBUILD, cause: RebuildCause::DC_EXHAUST, ..s.clone() }, "LINK.C.REOPEN.reopen_exhaust", vec![], vec![Fx::conn_close, Fx::app_link_rebuilding], vec![Seam { fx: SeamFx::floor_dc_down, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::floor_down_server, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::store_drop_server, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::rooms_rejoin_server, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::pub_reregister_server, to: "PUB", at: "sameServer" }], vec![]))
}

/// 그 서버 재구축이 끝났다 — 새 연결 · 다시 입장 · 다시 발행(g = 새 연결의 세대 · stood = 새 연결의 ICE 가 열렸다) (받음: ROOM_JOIN) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn rebuilt(s: &St, g: i64, stood: bool) -> Option<Step> {
    if !(s.ls == LinkSt::REBUILD) {
        return None;
    }
    if !stood {
        return Some(done(s, St { ls: LinkSt::CHECK, ice: Ice::FAILED, ever_up: false, assoc: true, r#gen: g, pgen: 0, sync: false, cause: RebuildCause::NONE, nego: false, dirty: false, ..s.clone() }, "LINK.C.REBUILD.rebuilt_unstood", vec![Tx::Heartbeat], vec![], vec![], vec![]));
    }
    Some(done(s, St { ls: LinkSt::OK, ice: Ice::OK, assoc: true, dc: true, r#gen: g, pgen: 0, sync: true, cause: RebuildCause::NONE, nego: false, dirty: false, ever_up: true, ..s.clone() }, "LINK.C.REBUILD.rebuilt", vec![], vec![Fx::app_link_ok], vec![Seam { fx: SeamFx::floor_dc_up, to: "FLR", at: "sameServer" }], vec![]))
}

/// 입장 응답의 server_config.ice(§[RULE-ROOM-JOIN-PROC]) — fresh = 보관한 자격과 다르다 · g = 그 자격의 세대
pub fn server_ice(s: &St, fresh: bool, g: i64) -> Step {
    if !fresh {
        return done(s, s.clone(), "LINK.C.any.server_ice_same", vec![], vec![], vec![], vec![]);
    }
    if s.ls == LinkSt::REBUILD {
        return done(s, s.clone(), "LINK.C.REBUILD.server_ice_rebuild", vec![], vec![], vec![], vec![]);
    }
    if s.ls == LinkSt::NONE {
        return done(s, s.clone(), "LINK.C.NONE.server_ice_closed", vec![], vec![], vec![], vec![]);
    }
    done(s, St { ls: LinkSt::OK, ice: Ice::OK, assoc: true, dc: true, r#gen: g, pgen: 0, req: 0, sync: true, cause: RebuildCause::NONE, nego: false, dirty: false, ever_up: true, ..s.clone() }, "LINK.C.any.peer_replaced", vec![], vec![Fx::app_link_ok], vec![Seam { fx: SeamFx::floor_dc_down, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::floor_down_server, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::pub_reregister_server, to: "PUB", at: "sameServer" }, Seam { fx: SeamFx::floor_dc_up, to: "FLR", at: "sameServer" }], vec![])
}

/// 세션이 미디어 연결을 전부 닫는다(RULE-SES-REBUILD 2 · RULE-SES-ON-DROP 2·3)
pub fn close_all(s: &St) -> Step {
    done(s, St { ls: LinkSt::NONE, dc: false, sync: false, nego: false, dirty: false, pgen: 0, req: 0, ..s.clone() }, "LINK.C.any.close_all", vec![], vec![Fx::conn_close], vec![Seam { fx: SeamFx::pub_drop_server, to: "PUB", at: "sameServer" }], vec![])
}

/// 그 서버의 방 하나를 떠났다 — last = 그 서버에 남은 방이 없다(RULE-ROOM-LEAVE-RES 2)
pub fn room_left(s: &St, last: bool) -> Step {
    if last {
        return done(s, St { ls: LinkSt::NONE, dc: false, sync: false, nego: false, dirty: false, pgen: 0, req: 0, ..s.clone() }, "LINK.C.any.last_room_left", vec![], vec![Fx::conn_close], vec![Seam { fx: SeamFx::pub_drop_server, to: "PUB", at: "sameServer" }], vec![]);
    }
    done(s, s.clone(), "LINK.C.any.room_left", vec![], vec![], vec![], vec![])
}

/// RESUME 뒤 죽은 연결을 층별 판정대로 되살린다(RULE-SES-RES-RESUME-OK 4 · RULE-LINK-RECOVERY-UNIT — 처음부터 다시 판정)
pub fn repair(s: &St) -> Step {
    if !s.assoc {
        return done(s, St { ls: LinkSt::REBUILD, cause: RebuildCause::ASSOC, dc: false, sync: false, pgen: 0, req: 0, ..s.clone() }, "LINK.C.any.repair_rebuild", vec![], vec![Fx::conn_close, Fx::app_link_rebuilding], vec![Seam { fx: SeamFx::floor_dc_down, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::floor_down_server, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::store_drop_server, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::rooms_rejoin_server, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::pub_reregister_server, to: "PUB", at: "sameServer" }], vec![]);
    }
    if s.ice != Ice::OK {
        return done(s, St { ls: LinkSt::RESTART, rid: s.rid + 1, req: s.rid + 1, ..s.clone() }, "LINK.C.any.repair_restart", vec![Tx::IceRestart], vec![Fx::app_link_reviving], vec![], vec![]);
    }
    if s.ls == LinkSt::CHECK || s.ls == LinkSt::RESTART {
        return done(s, St { ls: LinkSt::RESTART, rid: s.rid + 1, req: s.rid + 1, ..s.clone() }, "LINK.C.any.repair_recheck", vec![Tx::IceRestart], vec![Fx::app_link_reviving], vec![], vec![]);
    }
    done(s, s.clone(), "LINK.C.any.repair_ok", vec![], vec![], vec![], vec![])
}

/// 그 서버의 방 version.epoch 가 바뀌었다 — 서버가 재기동돼 그 연결을 모른다 · 그 서버만 재구축한다(RULE-SES-RES-RESUME-OK 4 · RULE-LINK-RECOVERY-UNIT)
pub fn epoch_changed(s: &St) -> Step {
    if s.ls == LinkSt::REBUILD {
        return done(s, s.clone(), "LINK.C.REBUILD.epoch_absorbed", vec![], vec![], vec![], vec![]);
    }
    done(s, St { ls: LinkSt::REBUILD, cause: RebuildCause::EPOCH, dc: false, sync: false, pgen: 0, req: 0, ..s.clone() }, "LINK.C.any.epoch_rebuild", vec![], vec![Fx::conn_close, Fx::app_link_rebuilding], vec![Seam { fx: SeamFx::floor_dc_down, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::floor_down_server, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::store_drop_server, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::rooms_rejoin_server, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::pub_reregister_server, to: "PUB", at: "sameServer" }], vec![])
}

/// RESUME 응답에 내 발행이 없다 — 서버가 그 연결을 모른다 · 그 서버만 재구축한다(RULE-SES-RES-RESUME-OK 2 ③ · 4)
pub fn pub_lost(s: &St) -> Step {
    if s.ls == LinkSt::REBUILD {
        return done(s, s.clone(), "LINK.C.REBUILD.publost_absorbed", vec![], vec![], vec![], vec![]);
    }
    done(s, St { ls: LinkSt::REBUILD, cause: RebuildCause::PUB_LOST, dc: false, sync: false, pgen: 0, req: 0, ..s.clone() }, "LINK.C.any.publost_rebuild", vec![], vec![Fx::conn_close, Fx::app_link_rebuilding], vec![Seam { fx: SeamFx::floor_dc_down, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::floor_down_server, to: "FLR", at: "sameServer" }, Seam { fx: SeamFx::store_drop_server, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::rooms_rejoin_server, to: "ROOM", at: "sameServer" }, Seam { fx: SeamFx::pub_reregister_server, to: "PUB", at: "sameServer" }], vec![])
}

/// 구독 · 발행 SDP 를 다시 조립해 협상한다(RULE-LINK-SDP-RENEGOTIATE · 그 서버 전체 — RULE-TRK-TRACK-EVENT-PROC)
pub fn renego(s: &St) -> Step {
    if s.ls == LinkSt::NONE {
        return done(s, s.clone(), "LINK.C.NONE.renego_closed", vec![], vec![], vec![], vec![]);
    }
    if s.nego {
        return done(s, St { dirty: true, ..s.clone() }, "LINK.C.any.renego_queued", vec![], vec![], vec![], vec![]);
    }
    done(s, St { nego: true, ..s.clone() }, "LINK.C.any.renego", vec![], vec![], vec![], vec![])
}

/// 재협상이 끝났다 — ok = 성공 (바깥: 브라우저 협상(offer/answer)의 결과)
pub fn nego_res(s: &St, ok: bool) -> Step {
    if ok && s.dirty {
        return done(s, St { dirty: false, ..s.clone() }, "LINK.C.any.nego_ok_next", vec![], vec![], vec![], vec![]);
    }
    if ok {
        return done(s, St { nego: false, ..s.clone() }, "LINK.C.any.nego_ok", vec![], vec![], vec![Seam { fx: SeamFx::sub_nego_ok, to: "SUB", at: "sameServer" }, Seam { fx: SeamFx::pub_nego_ok, to: "PUB", at: "sameServer" }], vec![]);
    }
    if s.dirty {
        return done(s, St { dirty: false, ..s.clone() }, "LINK.C.any.nego_fail_next", vec![], vec![], vec![], vec![]);
    }
    done(s, St { nego: false, ..s.clone() }, "LINK.C.any.nego_fail", vec![], vec![], vec![Seam { fx: SeamFx::sub_nego_fail, to: "SUB", at: "sameServer" }, Seam { fx: SeamFx::pub_nego_fail, to: "PUB", at: "sameServer" }], vec![])
}

/// MLINE_EVENT 를 받았다 — 그 서버 연결의 배정표 전체다(SUB.type.MlineTable) · 보관한 차례(arev)보다 크면 통째로 갈아 끼운다(RULE-SUB-TABLE-PROC) · ★표는 연결의 것이라 그 서버의 방 전부가 한 표를 쓴다 (받음: MLINE_EVENT)
pub fn mline_event(s: &St, m: &MlineEventMsg) -> Step {
    if m.table.arev > s.arev && m.table.mid == s.mid && m.table.rows == s.rows {
        return done(s, St { arev: m.table.arev, ..s.clone() }, "LINK.C.any.table_same", vec![], vec![], vec![], vec![]);
    }
    if m.table.arev > s.arev && m.table.rows != s.rows {
        return done(s, St { mid: m.table.mid.clone(), rows: m.table.rows, arev: m.table.arev, ..s.clone() }, "LINK.C.any.table", vec![], vec![], vec![Seam { fx: SeamFx::sub_table_changed, to: "SUB", at: "sameServer" }], vec![]);
    }
    if m.table.arev > s.arev {
        return done(s, St { mid: m.table.mid.clone(), arev: m.table.arev, ..s.clone() }, "LINK.C.any.table_rebound", vec![], vec![], vec![Seam { fx: SeamFx::sub_table_rebound, to: "SUB", at: "sameServer" }], vec![]);
    }
    done(s, s.clone(), "LINK.C.any.table_old", vec![], vec![], vec![], vec![])
}

/// 입장 응답 · RESUME 응답이 실어 온 그 서버의 배정표 — MLINE_EVENT 와 같게 받는다(RULE-SUB-TABLE-PROC · 어느 길로 와도 새 arev 가 이긴다)
pub fn table_carried(s: &St, t_arev: i64, t_mid: &Rec<i64>, t_rows: i64) -> Step {
    if t_arev > s.arev && *t_mid == s.mid && t_rows == s.rows {
        return done(s, St { arev: t_arev, ..s.clone() }, "LINK.C.any.table_carried_same", vec![], vec![], vec![], vec![]);
    }
    if t_arev > s.arev && t_rows != s.rows {
        return done(s, St { mid: t_mid.clone(), rows: t_rows, arev: t_arev, ..s.clone() }, "LINK.C.any.table_carried", vec![], vec![], vec![Seam { fx: SeamFx::sub_table_changed, to: "SUB", at: "sameServer" }], vec![]);
    }
    if t_arev > s.arev {
        return done(s, St { mid: t_mid.clone(), arev: t_arev, ..s.clone() }, "LINK.C.any.table_carried_rebound", vec![], vec![], vec![Seam { fx: SeamFx::sub_table_rebound, to: "SUB", at: "sameServer" }], vec![]);
    }
    done(s, s.clone(), "LINK.C.any.table_carried_old", vec![], vec![], vec![], vec![])
}

/// 벡터 시험 — TS 판 입구 이름과 JSON 인자로 부른다(oxpeer 1층 · TS 판 재생이 찾은 호출을 Rust 판에 넣어 맞댄다)
#[cfg(feature = "serde")]
pub fn call_json(entry: &str, s: &St, a: &[serde_json::Value]) -> Result<Option<Step>, String> {
    fn arg<T: serde::de::DeserializeOwned>(a: &[serde_json::Value], i: usize) -> Result<T, String> {
        serde_json::from_value(a.get(i).cloned().unwrap_or(serde_json::Value::Null)).map_err(|e| format!("인자 {i}: {e}"))
    }
    Ok(match entry {
        "iceDisc" => Some(ice_disc(s, arg::<bool>(a, 0)?)),
        "iceBack" => ice_back(s),
        "waitOver" => Some(wait_over(s, arg::<bool>(a, 0)?, arg::<bool>(a, 1)?)),
        "linkEvent" => Some(link_event(s, &arg::<LinkEventMsg>(a, 0)?)),
        "online" => Some(online(s)),
        "hbRes" => Some(hb_res(s, &arg::<HbResMsg>(a, 0)?)),
        "restartRes" => Some(restart_res(s, &arg::<RestartResMsg>(a, 0)?)),
        "applyTimeout" => apply_timeout(s),
        "applied" => applied(s),
        "assocLost" => Some(assoc_lost(s, arg::<bool>(a, 0)?)),
        "dcClosed" => Some(dc_closed(s, arg::<bool>(a, 0)?)),
        "reopened" => reopened(s, arg::<Reopen>(a, 0)?),
        "rebuilt" => rebuilt(s, arg::<i64>(a, 0)?, arg::<bool>(a, 1)?),
        "serverIce" => Some(server_ice(s, arg::<bool>(a, 0)?, arg::<i64>(a, 1)?)),
        "closeAll" => Some(close_all(s)),
        "roomLeft" => Some(room_left(s, arg::<bool>(a, 0)?)),
        "repair" => Some(repair(s)),
        "epochChanged" => Some(epoch_changed(s)),
        "pubLost" => Some(pub_lost(s)),
        "renego" => Some(renego(s)),
        "negoRes" => Some(nego_res(s, arg::<bool>(a, 0)?)),
        "mlineEvent" => Some(mline_event(s, &arg::<MlineEventMsg>(a, 0)?)),
        "tableCarried" => Some(table_carried(s, arg::<i64>(a, 0)?, &arg::<Rec<i64>>(a, 1)?, arg::<i64>(a, 2)?)),
        _ => return Err(format!("모르는 입구 {entry}")),
    })
}
