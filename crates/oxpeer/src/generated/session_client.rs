// ★생성 파일 — gen/axis_client.mts 가 원천(context/spec/protocol · areas/ses/behavior)에서 짓는다. 손으로 고치지 않는다.
// IR sha256 207c0df6ea211dc9 · 영역 SES 클라 상태기 Rust 판 — 같은 생성기의 TS 판과 한 뿌리(같은 입구 · 같은 경우 · 같은 라벨).
// 집행은 부르는 쪽 몫이다 — out(보낼 것) · fx(이 영역의 효과) · seams(다른 영역 입구로 — 받는 곳은 위가 정했다) · start(새로 걸 타이머). armed() 밖의 타이머는 멈춘다.
// 모형 자리(트랙 · 배정표 사본의 열쇠)는 문자열이다 — 실제 값을 넣는 법은 결합이 정한다(SDK:RULE-MOD-BINDING-VALUES).
#![allow(dead_code, unused_variables, unused_mut, unused_parens, non_camel_case_types, clippy::all)]

/// 값 이름 Conn
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Conn {
    DISCONNECTED,
    CONNECTING,
    ACTIVE,
    RESUMING,
}
impl Conn {
    pub const ALL: &'static [Conn] = &[Conn::DISCONNECTED, Conn::CONNECTING, Conn::ACTIVE, Conn::RESUMING];
    pub fn as_str(self) -> &'static str {
        match self { Conn::DISCONNECTED => "DISCONNECTED", Conn::CONNECTING => "CONNECTING", Conn::ACTIVE => "ACTIVE", Conn::RESUMING => "RESUMING", }
    }
    pub fn parse(s: &str) -> Option<Conn> {
        match s { "DISCONNECTED" => Some(Conn::DISCONNECTED), "CONNECTING" => Some(Conn::CONNECTING), "ACTIVE" => Some(Conn::ACTIVE), "RESUMING" => Some(Conn::RESUMING), _ => None }
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

/// 값 이름 PcMode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PcMode {
    #[cfg_attr(feature = "serde", serde(rename = "1pc"))] V1pc,
    #[cfg_attr(feature = "serde", serde(rename = "2pc"))] V2pc,
}
impl PcMode {
    pub const ALL: &'static [PcMode] = &[PcMode::V1pc, PcMode::V2pc];
    pub fn as_str(self) -> &'static str {
        match self { PcMode::V1pc => "1pc", PcMode::V2pc => "2pc", }
    }
    pub fn parse(s: &str) -> Option<PcMode> {
        match s { "1pc" => Some(PcMode::V1pc), "2pc" => Some(PcMode::V2pc), _ => None }
    }
}

/// 값 이름 Sock
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Sock {
    CLOSED,
    OPENING,
    OPEN,
}
impl Sock {
    pub const ALL: &'static [Sock] = &[Sock::CLOSED, Sock::OPENING, Sock::OPEN];
    pub fn as_str(self) -> &'static str {
        match self { Sock::CLOSED => "CLOSED", Sock::OPENING => "OPENING", Sock::OPEN => "OPEN", }
    }
    pub fn parse(s: &str) -> Option<Sock> {
        match s { "CLOSED" => Some(Sock::CLOSED), "OPENING" => Some(Sock::OPENING), "OPEN" => Some(Sock::OPEN), _ => None }
    }
}

/// 값 이름 Token
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Token {
    NONE,
    VALID,
    EXPIRED,
    FORGED,
}
impl Token {
    pub const ALL: &'static [Token] = &[Token::NONE, Token::VALID, Token::EXPIRED, Token::FORGED];
    pub fn as_str(self) -> &'static str {
        match self { Token::NONE => "NONE", Token::VALID => "VALID", Token::EXPIRED => "EXPIRED", Token::FORGED => "FORGED", }
    }
    pub fn parse(s: &str) -> Option<Token> {
        match s { "NONE" => Some(Token::NONE), "VALID" => Some(Token::VALID), "EXPIRED" => Some(Token::EXPIRED), "FORGED" => Some(Token::FORGED), _ => None }
    }
}

/// 값 이름 Wait
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Wait {
    NONE,
    BIND,
    RESUME,
}
impl Wait {
    pub const ALL: &'static [Wait] = &[Wait::NONE, Wait::BIND, Wait::RESUME];
    pub fn as_str(self) -> &'static str {
        match self { Wait::NONE => "NONE", Wait::BIND => "BIND", Wait::RESUME => "RESUME", }
    }
    pub fn parse(s: &str) -> Option<Wait> {
        match s { "NONE" => Some(Wait::NONE), "BIND" => Some(Wait::BIND), "RESUME" => Some(Wait::RESUME), _ => None }
    }
}

/// 이 영역의 효과(앱 알림 등)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Fx {
    #[cfg_attr(feature = "serde", serde(rename = "app.bug"))] app_bug,
    #[cfg_attr(feature = "serde", serde(rename = "app.connected"))] app_connected,
    #[cfg_attr(feature = "serde", serde(rename = "app.disconnected"))] app_disconnected,
    #[cfg_attr(feature = "serde", serde(rename = "app.need_token"))] app_need_token,
    #[cfg_attr(feature = "serde", serde(rename = "app.rebuilt"))] app_rebuilt,
    #[cfg_attr(feature = "serde", serde(rename = "app.resumed"))] app_resumed,
}
impl Fx {
    pub const ALL: &'static [Fx] = &[Fx::app_bug, Fx::app_connected, Fx::app_disconnected, Fx::app_need_token, Fx::app_rebuilt, Fx::app_resumed];
    pub fn as_str(self) -> &'static str {
        match self { Fx::app_bug => "app.bug", Fx::app_connected => "app.connected", Fx::app_disconnected => "app.disconnected", Fx::app_need_token => "app.need_token", Fx::app_rebuilt => "app.rebuilt", Fx::app_resumed => "app.resumed", }
    }
    pub fn parse(s: &str) -> Option<Fx> {
        match s { "app.bug" => Some(Fx::app_bug), "app.connected" => Some(Fx::app_connected), "app.disconnected" => Some(Fx::app_disconnected), "app.need_token" => Some(Fx::app_need_token), "app.rebuilt" => Some(Fx::app_rebuilt), "app.resumed" => Some(Fx::app_resumed), _ => None }
    }
}

/// 다른 영역으로 건너가는 효과(이음매)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SeamFx {
    #[cfg_attr(feature = "serde", serde(rename = "floor.down_all"))] floor_down_all,
    #[cfg_attr(feature = "serde", serde(rename = "media.close_all"))] media_close_all,
    #[cfg_attr(feature = "serde", serde(rename = "media.repair"))] media_repair,
    #[cfg_attr(feature = "serde", serde(rename = "pub.reregister_all"))] pub_reregister_all,
    #[cfg_attr(feature = "serde", serde(rename = "pub.resume_check"))] pub_resume_check,
    #[cfg_attr(feature = "serde", serde(rename = "rooms.close_all"))] rooms_close_all,
    #[cfg_attr(feature = "serde", serde(rename = "rooms.down_all"))] rooms_down_all,
    #[cfg_attr(feature = "serde", serde(rename = "rooms.rejoin"))] rooms_rejoin,
    #[cfg_attr(feature = "serde", serde(rename = "rooms.snapshot"))] rooms_snapshot,
}
impl SeamFx {
    pub const ALL: &'static [SeamFx] = &[SeamFx::floor_down_all, SeamFx::media_close_all, SeamFx::media_repair, SeamFx::pub_reregister_all, SeamFx::pub_resume_check, SeamFx::rooms_close_all, SeamFx::rooms_down_all, SeamFx::rooms_rejoin, SeamFx::rooms_snapshot];
    pub fn as_str(self) -> &'static str {
        match self { SeamFx::floor_down_all => "floor.down_all", SeamFx::media_close_all => "media.close_all", SeamFx::media_repair => "media.repair", SeamFx::pub_reregister_all => "pub.reregister_all", SeamFx::pub_resume_check => "pub.resume_check", SeamFx::rooms_close_all => "rooms.close_all", SeamFx::rooms_down_all => "rooms.down_all", SeamFx::rooms_rejoin => "rooms.rejoin", SeamFx::rooms_snapshot => "rooms.snapshot", }
    }
    pub fn parse(s: &str) -> Option<SeamFx> {
        match s { "floor.down_all" => Some(SeamFx::floor_down_all), "media.close_all" => Some(SeamFx::media_close_all), "media.repair" => Some(SeamFx::media_repair), "pub.reregister_all" => Some(SeamFx::pub_reregister_all), "pub.resume_check" => Some(SeamFx::pub_resume_check), "rooms.close_all" => Some(SeamFx::rooms_close_all), "rooms.down_all" => Some(SeamFx::rooms_down_all), "rooms.rejoin" => Some(SeamFx::rooms_rejoin), "rooms.snapshot" => Some(SeamFx::rooms_snapshot), _ => None }
    }
}

/// 타이머
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimerName {
    TBind,
    TReq,
    THb,
    Backoff,
    TStable,
}
impl TimerName {
    pub const ALL: &'static [TimerName] = &[TimerName::TBind, TimerName::TReq, TimerName::THb, TimerName::Backoff, TimerName::TStable];
    pub fn as_str(self) -> &'static str {
        match self { TimerName::TBind => "TBind", TimerName::TReq => "TReq", TimerName::THb => "THb", TimerName::Backoff => "Backoff", TimerName::TStable => "TStable", }
    }
    pub fn parse(s: &str) -> Option<TimerName> {
        match s { "TBind" => Some(TimerName::TBind), "TReq" => Some(TimerName::TReq), "THb" => Some(TimerName::THb), "Backoff" => Some(TimerName::Backoff), "TStable" => Some(TimerName::TStable), _ => None }
    }
}

/// 레코드 — 모형 자리(열쇠)를 문자열로 넓힌 것 · 차례 고정
pub type Rec<V> = std::collections::BTreeMap<String, V>;
fn rec<V: Clone>(kv: &[(&str, V)]) -> Rec<V> {
    kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BindResMsg {
    pub code: i64,
    pub heartbeat_interval: i64,
    pub part: Part,
    pub pc_mode: PcMode,
    pub resume_window_ms: i64,
    pub session_id: i64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ResumeResMsg {
    pub part: Part,
}

/// 클라 상태 — 원천 SES K
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct St {
    /// 연결 상태(§[RULE-SES-CONN-DIAGRAM])
    pub cs: Conn,
    /// 소켓
    pub sock: Sock,
    /// 기다리는 응답
    pub wait: Wait,
    /// 보관한 session_id — 0 이면 없다(BIND 응답이 준다 · §[SES.C.CONNECTING.bind_ok])
    pub sid: i64,
    /// 이 소켓의 BIND 에 실은 session_id — 응답과 견준다(§[RULE-SES-RES-BIND-RES])
    pub sent: i64,
    /// 끊긴 순간부터의 경과가 resume_window_ms 를 넘었다(§[RULE-SES-RES-OPEN] 2 · §[RULE-SES-BACKOFF-LADDER])
    pub late: bool,
    /// 백오프 횟수(§[RULE-SES-BACKOFF-LADDER])
    pub tries: i64,
    /// 하트비트가 돈다(§[SES.C.CONNECTING.bind_ok] 3)
    pub hb: bool,
    /// 보관한 하트비트 주기(ms) — BIND 응답의 `heartbeat_interval`(§[SES.C.CONNECTING.bind_ok] 1) · 0 이면 아직 없다
    #[cfg_attr(feature = "serde", serde(rename = "hbMs"))]
    pub hb_ms: i64,
    /// 보관한 재개 창(ms) — BIND 응답의 `resume_window_ms` · 끊긴 뒤 경과를 이것과 견준다(§[RULE-SES-RES-OPEN] 2)
    #[cfg_attr(feature = "serde", serde(rename = "winMs"))]
    pub win_ms: i64,
    /// 앱이 준 토큰
    pub token: Token,
    /// BIND 에 싣는 연결 모드 — ★브라우저 능력이라 클라가 안다 · 고르는 법은 원문이 정하지 않는다(RULE-LINK-SDP-1PC-MODE · 모르면 2pc)
    #[cfg_attr(feature = "serde", serde(rename = "pcAsk"))]
    pub pc_ask: PcMode,
    /// ★세션 확정 연결 모드 — BIND 응답의 에코(RULE-SES-PC-MODE-SESSION) · 세션 안에서 바뀌지 않는다 · ROOM 이 사본 없이 읽는다(RULE-ROOM-JOIN-PC-MODE-CHECK)
    pub pcm: PcMode,
}
pub fn initial() -> St {
    St { cs: Conn::DISCONNECTED, sock: Sock::CLOSED, wait: Wait::NONE, sid: 0, sent: 0, late: false, tries: 0, hb: false, hb_ms: 0, win_ms: 0, token: Token::VALID, pc_ask: PcMode::V2pc, pcm: PcMode::V2pc }
}

/// 이 상태기가 내는 것
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Tx {
    Bind { token: Token, pc_mode: PcMode, #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))] session_id: Option<i64> },
    Resume,
    Leave,
    Heartbeat,
}
impl Tx {
    pub fn kind(&self) -> &'static str {
        match self {
            Tx::Bind { .. } => "ws",
            Tx::Resume => "ws",
            Tx::Leave => "ws",
            Tx::Heartbeat => "ws",
        }
    }
    pub fn op(&self) -> &'static str {
        match self {
            Tx::Bind { .. } => "BIND",
            Tx::Resume => "RESUME",
            Tx::Leave => "LEAVE",
            Tx::Heartbeat => "HEARTBEAT",
        }
    }
    pub fn part(&self) -> &'static str {
        match self {
            Tx::Bind { .. } => "req",
            Tx::Resume => "req",
            Tx::Leave => "req",
            Tx::Heartbeat => "req",
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
pub const SEAMS: &[(SeamFx, &str, &[&str])] = &[(SeamFx::floor_down_all, "FLR", &["CDown"]), (SeamFx::media_close_all, "LINK", &["CCloseAll"]), (SeamFx::media_repair, "LINK", &["CRepair"]), (SeamFx::pub_reregister_all, "PUB", &["CReregister"]), (SeamFx::pub_resume_check, "PUB", &["CResumeCheck"]), (SeamFx::rooms_close_all, "ROOM", &["CCloseAll"]), (SeamFx::rooms_down_all, "ROOM", &["CDownAll"]), (SeamFx::rooms_rejoin, "ROOM", &["CRejoin"]), (SeamFx::rooms_snapshot, "ROOM", &["CResumeSnap"])];
/// 이음매가 부르는 입구 — (입구, 부르는 절차) · 아래가 넣는 사건이 아니다
pub const REACTS: &[(&str, &[&str])] = &[];
/// 읽는 것 — (입구.인자, 읽는 다른 영역 칸) · 부르는 쪽이 넣는다
pub const READS: &[(&str, &[&str])] = &[("bind_res.marked", &["ROOM.rejoin"]), ("resume_res.dead", &["LINK.assoc", "LINK.ice"]), ("drop.carry", &["ROOM.rs", "PUB.ps"])];

/// 원장 — 타이머(ms) · 원장에 값이 없으면 None
pub fn timer_ms(t: TimerName) -> Option<i64> {
    match t { TimerName::TBind => Some(10000), TimerName::TReq => Some(30000), TimerName::THb => Some(10000), TimerName::Backoff => None, TimerName::TStable => Some(10000), }
}
/// 원장 — 횟수
#[allow(non_upper_case_globals)]
pub mod counts {
    pub const CBackoff: i64 = 10;
}
/// 원장 — 사다리
pub struct Ladder {
    pub name: &'static str,
    pub waits: &'static [i64],
    pub jitter_from: i64,
    pub jitter_ms: i64,
}
pub const LADDER: Ladder = Ladder { name: "Backoff", waits: &[0, 300, 1200, 2700, 4800, 7000, 7000, 7000, 7000, 7000], jitter_from: 3, jitter_ms: 1000 };

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
    if s.wait == Wait::BIND {
        on.push(TimerName::TBind);
    }
    if s.wait == Wait::RESUME {
        on.push(TimerName::TReq);
    }
    if s.hb && s.sock == Sock::OPEN {
        on.push(TimerName::THb);
    }
    if (s.cs == Conn::CONNECTING || s.cs == Conn::RESUMING) && s.sock == Sock::CLOSED {
        on.push(TimerName::Backoff);
    }
    if s.cs == Conn::ACTIVE && s.tries > 0 {
        on.push(TimerName::TStable);
    }
    on
}

/// 앱이 접속을 요청했다(§[RULE-SES-DISC-CONNECT]) (앱: 앱이 접속을 요청했다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn connect(s: &St) -> Option<Step> {
    if !(s.cs == Conn::DISCONNECTED) {
        return None;
    }
    if s.token == Token::NONE {
        return Some(done(s, s.clone(), "SES.C.DISCONNECTED.connect_no_token", vec![], vec![Fx::app_need_token], vec![], vec![]));
    }
    Some(done(s, St { cs: Conn::CONNECTING, sock: Sock::OPENING, tries: 0, late: false, ..s.clone() }, "SES.C.DISCONNECTED.connect", vec![], vec![], vec![], vec![]))
}

/// 앱이 새 토큰을 줬다(§[RULE-SES-CONN-BIND-FAIL] 2 · §[RULE-SES-RES-BIND-RES] 1 의 요구에 답한다) (앱: 앱이 새 토큰을 줬다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn new_token(s: &St) -> Option<Step> {
    if !(s.cs == Conn::DISCONNECTED && s.token != Token::VALID) {
        return None;
    }
    Some(done(s, St { token: Token::VALID, ..s.clone() }, "SES.C.DISCONNECTED.new_token", vec![], vec![], vec![], vec![]))
}

/// 소켓이 열렸다(§[SES.C.CONNECTING.open] · §[RULE-SES-RES-OPEN]) (바깥: 소켓이 열렸다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn sock_open(s: &St) -> Option<Step> {
    if !(s.cs == Conn::CONNECTING || s.cs == Conn::RESUMING) {
        return None;
    }
    if s.cs == Conn::CONNECTING {
        return Some(done(s, St { sock: Sock::OPEN, wait: Wait::BIND, sent: 0, ..s.clone() }, "SES.C.CONNECTING.open", vec![Tx::Bind { token: s.token, pc_mode: s.pc_ask, session_id: None }], vec![], vec![], vec![]));
    }
    if s.late {
        return Some(done(s, St { sock: Sock::OPEN, wait: Wait::BIND, sent: 0, ..s.clone() }, "SES.C.RESUMING.open_late", vec![Tx::Bind { token: s.token, pc_mode: s.pc_ask, session_id: None }], vec![], vec![], vec![]));
    }
    Some(done(s, St { sock: Sock::OPEN, wait: Wait::BIND, sent: s.sid, ..s.clone() }, "SES.C.RESUMING.open", vec![Tx::Bind { token: s.token, pc_mode: s.pc_ask, session_id: Some(s.sid) }], vec![], vec![], vec![]))
}

/// BIND 응답을 받았다(§[SES.C.CONNECTING.bind_ok] · §[RULE-SES-CONN-BIND-FAIL] · §[RULE-SES-RES-BIND-RES]) (받음: BIND) — 받는 상태 밖이면 None(사건이 없던 것) · 읽는 것: marked = ROOM.rejoin
pub fn bind_res(s: &St, m: &BindResMsg, marked: bool) -> Option<Step> {
    if !(s.cs == Conn::CONNECTING || s.cs == Conn::RESUMING) {
        return None;
    }
    if s.cs == Conn::CONNECTING && m.part == Part::res && marked {
        return Some(done(s, St { sid: m.session_id, hb_ms: m.heartbeat_interval, win_ms: m.resume_window_ms, pcm: m.pc_mode, wait: Wait::NONE, hb: true, cs: Conn::ACTIVE, ..s.clone() }, "SES.C.CONNECTING.bind_ok_rebuild", vec![], vec![Fx::app_rebuilt], vec![Seam { fx: SeamFx::rooms_rejoin, to: "ROOM", at: "all" }, Seam { fx: SeamFx::pub_reregister_all, to: "PUB", at: "all" }], vec![]));
    }
    if s.cs == Conn::CONNECTING && m.part == Part::res {
        return Some(done(s, St { sid: m.session_id, hb_ms: m.heartbeat_interval, win_ms: m.resume_window_ms, pcm: m.pc_mode, wait: Wait::NONE, hb: true, cs: Conn::ACTIVE, ..s.clone() }, "SES.C.CONNECTING.bind_ok", vec![], vec![Fx::app_connected], vec![], vec![]));
    }
    if s.cs == Conn::CONNECTING && m.code == 2002 {
        return Some(done(s, St { cs: Conn::DISCONNECTED, sock: Sock::CLOSED, wait: Wait::NONE, ..s.clone() }, "SES.C.CONNECTING.bind_2002", vec![], vec![Fx::app_disconnected], vec![Seam { fx: SeamFx::rooms_close_all, to: "ROOM", at: "all" }], vec![]));
    }
    if s.cs == Conn::CONNECTING && m.code == 2003 {
        return Some(done(s, St { cs: Conn::DISCONNECTED, sock: Sock::CLOSED, wait: Wait::NONE, token: Token::NONE, ..s.clone() }, "SES.C.CONNECTING.bind_2003", vec![], vec![Fx::app_need_token], vec![], vec![]));
    }
    if s.cs == Conn::CONNECTING && m.code < 2000 {
        return Some(done(s, St { cs: Conn::DISCONNECTED, sock: Sock::CLOSED, wait: Wait::NONE, ..s.clone() }, "SES.C.CONNECTING.bind_1xxx", vec![], vec![Fx::app_bug], vec![Seam { fx: SeamFx::rooms_close_all, to: "ROOM", at: "all" }], vec![]));
    }
    if s.cs == Conn::CONNECTING {
        return Some(done(s, St { sock: Sock::CLOSED, wait: Wait::NONE, ..s.clone() }, "SES.C.CONNECTING.bind_fail", vec![], vec![], vec![], vec![]));
    }
    if m.code == 2003 {
        return Some(done(s, St { cs: Conn::DISCONNECTED, sock: Sock::CLOSED, wait: Wait::NONE, token: Token::NONE, ..s.clone() }, "SES.C.RESUMING.bind_2003", vec![], vec![Fx::app_need_token], vec![Seam { fx: SeamFx::floor_down_all, to: "FLR", at: "all" }, Seam { fx: SeamFx::rooms_down_all, to: "ROOM", at: "all" }, Seam { fx: SeamFx::media_close_all, to: "LINK", at: "all" }], vec![]));
    }
    if m.code == 2002 {
        return Some(done(s, St { cs: Conn::DISCONNECTED, sock: Sock::CLOSED, wait: Wait::NONE, ..s.clone() }, "SES.C.RESUMING.bind_2002", vec![], vec![Fx::app_disconnected], vec![Seam { fx: SeamFx::floor_down_all, to: "FLR", at: "all" }, Seam { fx: SeamFx::rooms_close_all, to: "ROOM", at: "all" }, Seam { fx: SeamFx::media_close_all, to: "LINK", at: "all" }], vec![]));
    }
    if m.part == Part::fail {
        return Some(done(s, St { sock: Sock::CLOSED, wait: Wait::NONE, ..s.clone() }, "SES.C.RESUMING.bind_fail", vec![], vec![], vec![], vec![]));
    }
    if s.sent != 0 && m.session_id == s.sent {
        return Some(done(s, St { sid: m.session_id, hb_ms: m.heartbeat_interval, win_ms: m.resume_window_ms, pcm: m.pc_mode, hb: true, wait: Wait::RESUME, ..s.clone() }, "SES.C.RESUMING.bind_same", vec![Tx::Resume], vec![], vec![], vec![]));
    }
    Some(done(s, St { sid: m.session_id, hb_ms: m.heartbeat_interval, win_ms: m.resume_window_ms, pcm: m.pc_mode, hb: true, wait: Wait::NONE, cs: Conn::ACTIVE, late: false, ..s.clone() }, "SES.C.RESUMING.bind_new", vec![], vec![Fx::app_rebuilt], vec![Seam { fx: SeamFx::floor_down_all, to: "FLR", at: "all" }, Seam { fx: SeamFx::rooms_down_all, to: "ROOM", at: "all" }, Seam { fx: SeamFx::media_close_all, to: "LINK", at: "all" }, Seam { fx: SeamFx::rooms_rejoin, to: "ROOM", at: "all" }, Seam { fx: SeamFx::pub_reregister_all, to: "PUB", at: "all" }], vec![]))
}

/// RESUME 응답을 받았다(§[RULE-SES-RES-RESUME-OK] · §[RULE-SES-REBUILD]) (받음: RESUME) — 받는 상태 밖이면 None(사건이 없던 것) · 읽는 것: dead = LINK.assoc · LINK.ice
pub fn resume_res(s: &St, m: &ResumeResMsg, dead: bool) -> Option<Step> {
    if !(s.cs == Conn::RESUMING) {
        return None;
    }
    if m.part == Part::res && dead {
        return Some(done(s, St { cs: Conn::ACTIVE, wait: Wait::NONE, late: false, ..s.clone() }, "SES.C.RESUMING.resume_ok_repair", vec![], vec![Fx::app_resumed], vec![Seam { fx: SeamFx::rooms_snapshot, to: "ROOM", at: "all" }, Seam { fx: SeamFx::pub_resume_check, to: "PUB", at: "all" }, Seam { fx: SeamFx::media_repair, to: "LINK", at: "all" }], vec![]));
    }
    if m.part == Part::res {
        return Some(done(s, St { cs: Conn::ACTIVE, wait: Wait::NONE, late: false, ..s.clone() }, "SES.C.RESUMING.resume_ok", vec![], vec![Fx::app_resumed], vec![Seam { fx: SeamFx::rooms_snapshot, to: "ROOM", at: "all" }, Seam { fx: SeamFx::pub_resume_check, to: "PUB", at: "all" }], vec![]));
    }
    Some(done(s, St { wait: Wait::NONE, cs: Conn::ACTIVE, late: false, ..s.clone() }, "SES.C.RESUMING.resume_2008", vec![], vec![Fx::app_rebuilt], vec![Seam { fx: SeamFx::floor_down_all, to: "FLR", at: "all" }, Seam { fx: SeamFx::rooms_down_all, to: "ROOM", at: "all" }, Seam { fx: SeamFx::media_close_all, to: "LINK", at: "all" }, Seam { fx: SeamFx::rooms_rejoin, to: "ROOM", at: "all" }, Seam { fx: SeamFx::pub_reregister_all, to: "PUB", at: "all" }], vec![]))
}

/// 소켓이 끊겼다 — c 는 LEAVE 사유(0 = LEAVE 없이 끊겼다 · §[SES.C.any]) (바깥: 소켓이 끊겼다) · 읽는 것: carry = ROOM.rs · PUB.ps
pub fn drop(s: &St, c: i64, carry: bool) -> Step {
    if c == 1008 || c == 2001 || c == 4006 || c == 4007 || c == 2010 {
        return done(s, St { cs: Conn::DISCONNECTED, sock: Sock::CLOSED, wait: Wait::NONE, hb: false, ..s.clone() }, "SES.C.any.drop_final", vec![], vec![Fx::app_disconnected], vec![Seam { fx: SeamFx::floor_down_all, to: "FLR", at: "all" }, Seam { fx: SeamFx::rooms_close_all, to: "ROOM", at: "all" }, Seam { fx: SeamFx::media_close_all, to: "LINK", at: "all" }], vec![]);
    }
    if c == 2009 || c == 5004 {
        return done(s, St { cs: Conn::CONNECTING, sock: Sock::CLOSED, wait: Wait::NONE, hb: false, sid: 0, late: false, ..s.clone() }, "SES.C.any.drop_reclaimed", vec![], vec![], vec![Seam { fx: SeamFx::floor_down_all, to: "FLR", at: "all" }, Seam { fx: SeamFx::rooms_down_all, to: "ROOM", at: "all" }, Seam { fx: SeamFx::media_close_all, to: "LINK", at: "all" }], vec![]);
    }
    if carry {
        return done(s, St { cs: Conn::RESUMING, sock: Sock::CLOSED, wait: Wait::NONE, hb: false, late: if s.cs == Conn::ACTIVE || s.wait == Wait::RESUME { false } else { s.late }, ..s.clone() }, "SES.C.any.drop_carry", vec![], vec![], vec![], vec![]);
    }
    done(s, St { cs: Conn::CONNECTING, sock: Sock::CLOSED, wait: Wait::NONE, hb: false, ..s.clone() }, "SES.C.any.drop", vec![], vec![], vec![], vec![])
}

/// 끊긴 순간부터의 경과가 resume_window_ms 를 넘었다(클라 시계 · §[RULE-SES-BACKOFF-LADDER]) (바깥: 끊긴 뒤 경과가 재개 창을 넘었다(클라 시계)) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn late(s: &St) -> Option<Step> {
    if !(s.cs == Conn::RESUMING && !s.late) {
        return None;
    }
    Some(done(s, St { late: true, ..s.clone() }, "SES.C.RESUMING.window_passed", vec![], vec![], vec![], vec![]))
}

/// 앱이 연결을 끊는다(RULE-SES-LEAVE-PROC) (앱: 앱이 연결을 끊는다) — 받는 상태 밖이면 None(사건이 없던 것)
pub fn disconnect(s: &St) -> Option<Step> {
    if !(s.cs != Conn::DISCONNECTED) {
        return None;
    }
    if s.cs == Conn::ACTIVE {
        return Some(done(s, St { cs: Conn::DISCONNECTED, sock: Sock::CLOSED, wait: Wait::NONE, hb: false, ..s.clone() }, "SES.C.ACTIVE.disconnect", vec![Tx::Leave], vec![Fx::app_disconnected], vec![Seam { fx: SeamFx::floor_down_all, to: "FLR", at: "all" }, Seam { fx: SeamFx::rooms_close_all, to: "ROOM", at: "all" }, Seam { fx: SeamFx::media_close_all, to: "LINK", at: "all" }], vec![]));
    }
    Some(done(s, St { cs: Conn::DISCONNECTED, sock: Sock::CLOSED, wait: Wait::NONE, hb: false, ..s.clone() }, "SES.C.any.disconnect", vec![], vec![Fx::app_disconnected], vec![Seam { fx: SeamFx::floor_down_all, to: "FLR", at: "all" }, Seam { fx: SeamFx::rooms_close_all, to: "ROOM", at: "all" }, Seam { fx: SeamFx::media_close_all, to: "LINK", at: "all" }], vec![]))
}

/// 토큰이 만료됐다(시간이 흘렀다) (바깥: 토큰이 만료됐다(시간))
pub fn token_aged(s: &St) -> Step {
    done(s, St { token: Token::EXPIRED, ..s.clone() }, "SES.C.any.token_expired", vec![], vec![], vec![], vec![])
}

/// 타이머 만료 — 돌고 있지 않은 타이머면 None(부르는 쪽이 멈췄어야 한다)
pub fn expire(s: &St, t: TimerName) -> Option<Step> {
    if !armed(s).contains(&t) {
        return None;
    }
    match t {
        TimerName::TBind => {
            Some(done(s, St { sock: Sock::CLOSED, wait: Wait::NONE, ..s.clone() }, "SES.C.any.TBind", vec![], vec![], vec![], vec![]))
        }
        TimerName::TReq => {
            Some(done(s, St { sock: Sock::CLOSED, wait: Wait::NONE, hb: false, ..s.clone() }, "SES.C.any.TReq", vec![], vec![], vec![], vec![]))
        }
        TimerName::THb => {
            Some(done(s, s.clone(), "SES.C.any.THb", vec![Tx::Heartbeat], vec![], vec![], vec![]))
        }
        TimerName::Backoff => {
            if s.tries < counts::CBackoff {
                return Some(done(s, St { sock: Sock::OPENING, tries: s.tries + 1, ..s.clone() }, "SES.C.any.Backoff", vec![], vec![], vec![], vec![]));
            }
            if s.cs == Conn::RESUMING {
                return Some(done(s, St { cs: Conn::DISCONNECTED, ..s.clone() }, "SES.C.RESUMING.Backoff_exhaust", vec![], vec![Fx::app_disconnected], vec![Seam { fx: SeamFx::floor_down_all, to: "FLR", at: "all" }, Seam { fx: SeamFx::rooms_close_all, to: "ROOM", at: "all" }, Seam { fx: SeamFx::media_close_all, to: "LINK", at: "all" }], vec![]));
            }
            Some(done(s, St { cs: Conn::DISCONNECTED, ..s.clone() }, "SES.C.CONNECTING.Backoff_exhaust", vec![], vec![Fx::app_disconnected], vec![Seam { fx: SeamFx::rooms_close_all, to: "ROOM", at: "all" }], vec![]))
        }
        TimerName::TStable => {
            Some(done(s, St { tries: 0, ..s.clone() }, "SES.C.any.TStable", vec![], vec![], vec![], vec![]))
        }
    }
}

/// 벡터 시험 — TS 판 입구 이름과 JSON 인자로 부른다(oxpeer 1층 · TS 판 재생이 찾은 호출을 Rust 판에 넣어 맞댄다)
#[cfg(feature = "serde")]
pub fn call_json(entry: &str, s: &St, a: &[serde_json::Value]) -> Result<Option<Step>, String> {
    fn arg<T: serde::de::DeserializeOwned>(a: &[serde_json::Value], i: usize) -> Result<T, String> {
        serde_json::from_value(a.get(i).cloned().unwrap_or(serde_json::Value::Null)).map_err(|e| format!("인자 {i}: {e}"))
    }
    Ok(match entry {
        "connect" => connect(s),
        "newToken" => new_token(s),
        "sockOpen" => sock_open(s),
        "bindRes" => bind_res(s, &arg::<BindResMsg>(a, 0)?, arg::<bool>(a, 1)?),
        "resumeRes" => resume_res(s, &arg::<ResumeResMsg>(a, 0)?, arg::<bool>(a, 1)?),
        "drop" => Some(drop(s, arg::<i64>(a, 0)?, arg::<bool>(a, 1)?)),
        "late" => late(s),
        "disconnect" => disconnect(s),
        "tokenAged" => Some(token_aged(s)),
        "expire" => expire(s, arg::<TimerName>(a, 0)?),
        _ => return Err(format!("모르는 입구 {entry}")),
    })
}
