// author: kodeholic (powered by Claude)
//! SDK 루프 — 세션 · 방 · 연결 · 구독 · 트랙 · 발언권의 상태를 한 태스크가 쥔다(SDK:RULE-API-ASYNC 스레드 — 서버사이드 = SDK 루프 하나).
//! ★절차는 위(연동규격서)가 지은 영역 상태기(generated/*_client)가 정한다 — 이 루프는 입구 표(어느 사건이 어느 입구를) · 효과 표(위 효과를 어느 사건으로 —
//!   generated/*_binding) · 이음매(위가 받는 곳까지 정했다) 집행 · 위가 값을 두지 않은 손잡이(시계 · 사다리)만 한다.
//! 바깥 입력은 넷 — 앱 명령 · 소켓 사건 · 연결 사실 · 재동기 응답. 걸음 뒤 요청의 끝(응답 · 버림)과 미룬 일(협상)을 차례대로 비운다.

mod link;
mod pubs;
mod room;

pub(crate) use pubs::FRAME_QUEUE;
mod ses;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dtls::crypto::Certificate;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::api::{Config, Event, Layer, Snapshot, Stats, TrackSpec};
use crate::link::send::OutFrame;
use crate::link::transport::PubOut;
use crate::error::Error;
use crate::generated::{floor_client as flr, session_client as sc};
use crate::http;
use crate::link::recv::{MediaFrame, Unit};
use crate::link::transport::{LinkFact, MediaPacket, MediaSink};
use crate::sig::{Outcome, Purpose, Sig, SockIn};

pub(crate) use link::Link;
pub(crate) use room::Room;

/// 발행 응답 — (발행 번호, track_id, 코덱)
pub(crate) type PublishReply = oneshot::Sender<Result<(u64, String, String), Error>>;

pub(crate) enum Cmd {
    Connect(oneshot::Sender<Result<(), Error>>),
    SetToken(String),
    Join(String, oneshot::Sender<Result<(), Error>>),
    Leave(String, oneshot::Sender<Result<(), Error>>),
    Close(oneshot::Sender<()>),
    Stats(oneshot::Sender<Stats>),
    Diag(mpsc::UnboundedSender<crate::api::Transition>),
    SetUnit(String, Unit),
    SetLayer(String, Layer, oneshot::Sender<Result<(), Error>>),
    /// 받는 영상 트랙에 키프레임 요청(SDK:RULE-PLAT-SERVER 수신 정리층 — 녹화 조각 경계)
    RequestKey(String),
    /// 말하는 쪽(20261107c)
    JoinSpeaking(String, oneshot::Sender<Result<(), Error>>),
    SetSpeakingRoom(String, oneshot::Sender<Result<(), Error>>),
    Publish(String, TrackSpec, Vec<mpsc::Receiver<OutFrame>>, PublishReply),
    Unpublish(u64),
    SetDuplex(u64, bool, oneshot::Sender<Result<(), Error>>),
    /// 송신 시계 — 게이트가 열린 뒤 첫 패킷이 나갔다
    RtpTx(u64),
    PttPress(String, u8, oneshot::Sender<Result<(), Error>>),
    PttRelease(String, oneshot::Sender<Result<(), Error>>),
    Chat(String, String, Option<String>, oneshot::Sender<Result<u64, Error>>),
    ChatCatchup(Option<String>, Option<u64>, oneshot::Sender<Result<u64, Error>>),
    Snapshot(oneshot::Sender<Snapshot>),
    /// 앱 운영 로그 줄(SDK:RULE-PLAT-SERVER 운영 로그)
    Log(crate::log::AppLine),
    /// 라이브 레코드 통(지점 ①)
    #[cfg(feature = "record")]
    Records(mpsc::Sender<crate::record::Record>, std::sync::Arc<crate::record::Shared>),
}

/// 재동기 응답(HTTP — 루프 밖 태스크가 받아 넘긴다)
pub(crate) struct HttpDone {
    room: String,
    epoch: u64,
    result: Result<http::Reply, Error>,
}

/// 루프가 거는 시계 — 이름 하나에 하나
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Tk {
    Ses(sc::TimerName),
    /// 끊긴 뒤 경과가 재개 창을 넘었나
    Window,
    /// 요청 기한(T-req)을 센다
    ReqCheck,
    /// 재구축 칸 첫 입장 분산(massJitterMs)
    Jitter,
    /// 과부하 하한 · 재시도 사다리 — 방마다
    RoomRetry(String),
    RoomResend(String),
    /// 재동기 실패 뒤 다시
    Resync(String),
    Floor(String, flr::TimerName),
    LinkGrace(String),
    LinkApply(String),
    LinkReopenTry(String),
    LinkReopenJudge(String),
    LinkStable(String),
    LinkCheck(String),
    LinkProbe(String),
    /// 운영 로그 프로세스 계수(`stat`) 주기
    LogStat,
}

#[derive(Default)]
pub(crate) struct Timers {
    at: BTreeMap<(Instant, u64), Tk>,
    by: HashMap<Tk, (Instant, u64)>,
    seq: u64,
}

impl Timers {
    pub(crate) fn start(&mut self, tk: Tk, after: Duration) {
        self.stop(&tk);
        self.seq += 1;
        let when = Instant::now() + after;
        self.at.insert((when, self.seq), tk.clone());
        self.by.insert(tk, (when, self.seq));
    }
    pub(crate) fn stop(&mut self, tk: &Tk) {
        if let Some(k) = self.by.remove(tk) {
            self.at.remove(&k);
        }
    }
    pub(crate) fn armed(&self, tk: &Tk) -> bool {
        self.by.contains_key(tk)
    }
    fn next(&self) -> Option<Instant> {
        self.at.keys().next().map(|(t, _)| *t)
    }
    fn due(&mut self, now: Instant) -> Vec<Tk> {
        let mut out = Vec::new();
        while let Some((&(t, s), _)) = self.at.iter().next() {
            if t > now {
                break;
            }
            if let Some(tk) = self.at.remove(&(t, s)) {
                self.by.remove(&tk);
                out.push(tk);
            }
        }
        out
    }
    pub(crate) fn stop_where(&mut self, pred: impl Fn(&Tk) -> bool) {
        let gone: Vec<Tk> = self.by.keys().filter(|k| pred(k)).cloned().collect();
        for k in gone {
            self.stop(&k);
        }
    }
}

/// 불투명 신원 → 작은 수(SDK:RULE-MOD-BINDING-VALUES) — 같은 문자열 ⇔ 같은 수 · 처음 본 차례로 1 부터 · 0 은 내지 않는다
#[derive(Default)]
pub(crate) struct Interner {
    to: HashMap<String, i64>,
    from: HashMap<i64, String>,
}

impl Interner {
    pub(crate) fn id(&mut self, s: &str) -> i64 {
        if let Some(n) = self.to.get(s) {
            return *n;
        }
        let n = self.to.len() as i64 + 1;
        self.to.insert(s.to_string(), n);
        self.from.insert(n, s.to_string());
        n
    }
    pub(crate) fn str(&self, n: i64) -> Option<&str> {
        self.from.get(&n).map(String::as_str)
    }
}

/// 루프 안에서 미룬 일 — 웹의 비동기 협상 · 다음 틱 자리
pub(crate) enum Job {
    Negotiate(String),
}

pub(crate) struct Core {
    pub(crate) base: String,
    /// `https://` 면 TLS 설정(재동기 HTTP 가 같이 쓴다)
    pub(crate) tls: Option<Arc<tokio_rustls::rustls::ClientConfig>>,
    pub(crate) token: String,
    pub(crate) ses: sc::St,
    pub(crate) user_id: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) reason: Option<(u16, String)>,
    pub(crate) failure: Option<Error>,
    pub(crate) app_closed: bool,
    pub(crate) resume_on_token: bool,
    pub(crate) resume_body: Option<Value>,
    pub(crate) dropped_at: Option<Instant>,
    pub(crate) last_info: Option<crate::api::SessionInfo>,
    pub(crate) sig: Sig,
    pub(crate) rooms: BTreeMap<String, Room>,
    pub(crate) links: BTreeMap<String, Link>,
    pub(crate) sids: Interner,
    pub(crate) epochs: Interner,
    pub(crate) jids: Interner,
    /// 배정표 줄의 SDP 재료 지문(SDK:RULE-MOD-BINDING-VALUES `rows`)
    pub(crate) materials: Interner,
    pub(crate) timers: Timers,
    pub(crate) ev: mpsc::UnboundedSender<Event>,
    pub(crate) diag: std::sync::Mutex<Vec<mpsc::UnboundedSender<crate::api::Transition>>>,
    pub(crate) sock_tx: mpsc::UnboundedSender<SockIn>,
    pub(crate) fact_tx: mpsc::UnboundedSender<LinkFact>,
    pub(crate) http_tx: mpsc::UnboundedSender<HttpDone>,
    pub(crate) sink: MediaSink,
    /// 받는 단위 — 앱 기본 · 트랙마다 고친 것
    pub(crate) unit: Unit,
    pub(crate) units: HashMap<String, Unit>,
    /// 트랙마다 층을 바꾼 차례(정리층이 키프레임을 기다리는 표시)
    pub(crate) layer_gen: HashMap<String, u64>,
    /// 앱 키프레임 요청 차례(트랙마다 — `Client::request_keyframe`)
    pub(crate) key_gen: HashMap<String, u64>,
    /// 화자 출처 번호 → 사용자 — `TAKEN` 이 준다(연:RULE-FLR-SLOT-SOURCES · 번호는 트랙이 사는 동안 같다)
    pub(crate) talk_sources: BTreeMap<u32, String>,
    /// 운영 로그
    pub(crate) log: Arc<crate::log::Logger>,
    pub(crate) cert: Certificate,
    pub(crate) connect_waiters: Vec<oneshot::Sender<Result<(), Error>>>,
    /// ACTIVE 를 기다리는 입장(SDK:RULE-API-ASYNC 연결 전 요청)
    pub(crate) parked: Vec<(String, oneshot::Sender<Result<(), Error>>)>,
    pub(crate) jitter_rooms: Vec<String>,
    pub(crate) jobs: VecDeque<Job>,
    pub(crate) ended: Vec<(Purpose, Outcome)>,
    pub(crate) link_gen: u64,
    trace: bool,
    /// 말하는 쪽 — 발행 트랙 · 서버마다 발행 mid 차례 · 내보내는 손잡이 자리 · 루프 자신의 명령 통(송신 시계가 쓴다)
    pub(crate) pubs: BTreeMap<u64, pubs::PubTrack>,
    pub(crate) pub_seq: u64,
    pub(crate) pub_mids: HashMap<String, u32>,
    pub(crate) pub_outs: Arc<std::sync::RwLock<HashMap<String, Arc<std::sync::OnceLock<PubOut>>>>>,
    /// 루프 자신의 명령 통(송신 시계가 쓴다) — 앱 통과 따로라 앱이 `Client` 를 다 놓으면 앱 통이 닫혀 루프가 끝난다
    pub(crate) cmd_tx: mpsc::UnboundedSender<Cmd>,
    /// 세션의 소속(발언 방 — 위 AFF 상태기) · 기다리는 앱
    pub(crate) aff: crate::generated::aff_client::St,
    pub(crate) aff_waiters: Vec<oneshot::Sender<Result<(), Error>>>,
    pub(crate) msg_waiters: HashMap<u64, oneshot::Sender<Result<u64, Error>>>,
    pub(crate) msg_seq: u64,
    /// 다음 입장의 모양 — (방, select:true 로) · 아니면 (방, 들어간 뒤 소속 전환)
    pub(crate) join_mode: Option<(String, bool)>,
}

pub(crate) fn spawn(cfg: Config, ev: mpsc::UnboundedSender<Event>, media: mpsc::Sender<MediaPacket>, frames: mpsc::Sender<MediaFrame>) -> Result<mpsc::UnboundedSender<Cmd>, Error> {
    let b = http::split_base(&cfg.base)?;
    let tls = match (b.secure, cfg.tls_pin_pem.as_deref()) {
        (false, _) => None,
        (true, Some(pin)) => Some(crate::tls::pinned_config(pin)?),
        (true, None) => Some(crate::tls::client_config(cfg.tls_root_pem.as_deref())?),
    };
    let cert = Certificate::generate_self_signed(vec!["oxpeer".to_string()]).map_err(|e| Error::transport(format!("인증서: {e}")))?;
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (int_tx, int_rx) = mpsc::unbounded_channel();
    let (sock_tx, sock_rx) = mpsc::unbounded_channel();
    let (fact_tx, fact_rx) = mpsc::unbounded_channel();
    let (http_tx, http_rx) = mpsc::unbounded_channel();
    // 운영 로그(SDK:RULE-PLAT-SERVER 「운영 로그」) — 기동 줄 하나(hub · sfud 와 같은 `spec` · `build`)
    let log = crate::log::Logger::start(cfg.log.clone());
    log.emit(crate::log::Line::new(crate::log::Kind::Ev).spec("SV-STATE-UNIT-STATE").key("build", concat!("oxpeer-", env!("CARGO_PKG_VERSION"))).msg("oxpeer boot"));
    let sink = MediaSink {
        log: log.clone(),
        routes: Arc::default(),
        tx: media,
        frames,
        dropped: Arc::new(AtomicU64::new(0)),
        frames_dropped: Arc::new(AtomicU64::new(0)),
        counters: Arc::default(),
        qa_drop_every: std::env::var("OXPEER_QA_DROP_VIDEO_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(0),
        qa_drop_until: std::env::var("OXPEER_QA_DROP_VIDEO_UNTIL_MS").ok().and_then(|v| v.parse().ok()).map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms)),
        qa_drop_audio_every: std::env::var("OXPEER_QA_DROP_AUDIO_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(0),
        qa_dropped: Arc::new(AtomicU64::new(0)),
        senders: Arc::default(),
        remb: Arc::new(AtomicU64::new(0)),
        rtx_sent: Arc::new(AtomicU64::new(0)),
        #[cfg(feature = "record")]
        recorder: match cfg.record.clone() {
            Some(rc) => Some(crate::record::Recorder::start(rc, ev.clone(), log.clone()).map_err(|e| Error::transport(format!("녹음 원시 기록: {e}")))?),
            None => None,
        },
        unrouted: Arc::new(AtomicU64::new(0)),
        decrypt_fail: Arc::new(AtomicU64::new(0)),
    };
    let req = Duration::from_millis(sc::timer_ms(sc::TimerName::TReq).unwrap_or(30_000) as u64);
    let core = Core {
        base: cfg.base.clone(),
        token: cfg.token,
        ses: sc::initial(),
        user_id: None,
        session_id: None,
        reason: None,
        failure: None,
        app_closed: false,
        resume_on_token: false,
        resume_body: None,
        dropped_at: None,
        last_info: None,
        sig: Sig::new(b.ws_url(), b.auth.clone(), tls.clone(), req),
        tls,
        rooms: BTreeMap::new(),
        links: BTreeMap::new(),
        sids: Interner::default(),
        epochs: Interner::default(),
        jids: Interner::default(),
        materials: Interner::default(),
        timers: Timers::default(),
        ev,
        sock_tx,
        fact_tx,
        http_tx,
        sink,
        unit: cfg.unit,
        units: HashMap::new(),
        layer_gen: HashMap::new(),
        key_gen: HashMap::new(),
        talk_sources: BTreeMap::new(),
        cert,
        connect_waiters: Vec::new(),
        parked: Vec::new(),
        jitter_rooms: Vec::new(),
        jobs: VecDeque::new(),
        ended: Vec::new(),
        link_gen: 0,
        trace: std::env::var_os("OXPEER_TRACE").is_some(),
        log: log.clone(),
        diag: std::sync::Mutex::new(Vec::new()),
        pubs: BTreeMap::new(),
        pub_seq: 0,
        pub_mids: HashMap::new(),
        pub_outs: Arc::default(),
        cmd_tx: int_tx,
        aff: crate::generated::aff_client::initial(),
        aff_waiters: Vec::new(),
        msg_waiters: HashMap::new(),
        msg_seq: 0,
        join_mode: None,
    };
    let mut core = core;
    let every = core.log.config().stat_interval;
    core.timers.start(Tk::LogStat, every);
    tokio::spawn(run(core, cmd_rx, int_rx, sock_rx, fact_rx, http_rx));
    Ok(cmd_tx)
}

async fn run(mut core: Core, mut cmd: mpsc::UnboundedReceiver<Cmd>, mut int: mpsc::UnboundedReceiver<Cmd>, mut sock: mpsc::UnboundedReceiver<SockIn>, mut facts: mpsc::UnboundedReceiver<LinkFact>, mut http: mpsc::UnboundedReceiver<HttpDone>) {
    loop {
        let deadline = core.timers.next();
        let sleep = async {
            match deadline {
                Some(d) => tokio::time::sleep_until(tokio::time::Instant::from_std(d)).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            c = cmd.recv() => match c {
                Some(c) => core.command(c),
                None => {
                    core.close_for_app();
                    core.drain();
                    #[cfg(feature = "record")]
                    if let Some(r) = &core.sink.recorder {
                        r.close();
                    }
                    break;
                }
            },
            Some(c) = int.recv() => core.command(c),
            Some(s) = sock.recv() => core.socket(s),
            Some(f) = facts.recv() => core.link_fact(f),
            Some(h) = http.recv() => core.resync_done(h.room, h.epoch, h.result),
            _ = sleep => {
                for tk in core.timers.due(Instant::now()) {
                    core.fire(tk);
                    core.drain();
                }
            }
        }
        core.drain();
    }
}

impl Core {
    /// 전이 기록 — OXPEER_TRACE 가 있으면 표준 오류로(층 · 열쇠 · 앞 → 뒤 · 라벨 · 까닭)
    pub(crate) fn record(&self, layer: &str, key: &str, from: &str, to: &str, label: &str, cause: &str) {
        if self.trace {
            eprintln!("[oxpeer] {layer} {key} {from}→{to} {label} ({cause})");
        }
        self.log_transition(layer, key, from, Some(to), Some(label), cause);
        self.diag_send(layer, key, from, Some(to), Some(label), cause);
    }

    /// 전이 줄(운영:RULE-LOG-TRANSITION 꼴 — SDK:RULE-PLAT-SERVER 「운영 로그」) — 걸음마다 한 줄 · 받지 않는 사건은 `spec` · `to` 없이 `impossible=1`.
    /// 방 단위 상태기(방 · 구독 · 트랙 · 발언권 — 열쇠가 방)는 `room` 을 싣는다(방으로 물을 때 사슬이 이어진다)
    fn log_transition(&self, area: &str, key: &str, from: &str, to: Option<&str>, label: Option<&str>, cause: &str) {
        let mut l = crate::log::Line::new(crate::log::Kind::Ev);
        if let Some(sp) = label {
            l = l.spec(sp);
        }
        if matches!(area, "ROOM" | "SUB" | "TRK" | "FLR") {
            l = l.corr("room", key);
        }
        l = l.key("area", area).key("key", key).key("from", from);
        if let Some(t) = to {
            l = l.key("to", t);
        }
        l = l.key("cause", cause);
        if to.is_none() {
            l = l.key("impossible", 1);
        }
        self.log.emit(l);
    }

    /// 프로세스 계수(운영 `stat` — `proc=peer` 프로세스 줄) — 「닫았는데 아직 도는 것」을 줄 하나로 가른다(links · dtls_open · cpu_time_ms)
    pub(crate) fn log_stat(&mut self) {
        use std::sync::atomic::Ordering::Relaxed;
        let (cpu, rss) = crate::log::rusage().unwrap_or((0, 0));
        let s = &self.sink;
        #[cfg_attr(not(feature = "record"), allow(unused_mut))]
        let mut l = crate::log::Line::new(crate::log::Kind::Stat)
            .key("cpu_time_ms", cpu)
            .key("max_rss_bytes", rss)
            .key("links", crate::link::transport::LINKS_ALIVE.load(Relaxed))
            .key("dtls_open", crate::link::transport::DTLS_OPEN.load(Relaxed))
            .key("media_dropped", s.dropped.load(Relaxed))
            .key("frames_dropped", s.frames_dropped.load(Relaxed))
            .key("decrypt_fail", s.decrypt_fail.load(Relaxed))
            .key("media_unrouted", s.unrouted.load(Relaxed));
        #[cfg(feature = "record")]
        if let Some(r) = &s.recorder {
            let st = r.stat();
            l = l.key("record_parts", st.parts).key("record_subscribers", st.subscribers).key("record_degraded", st.degraded).key("record_stopped", u8::from(st.stopped));
            if let Some(f) = st.disk_free {
                l = l.key("disk_free_bytes", f);
            }
        }
        self.log.emit(l);
        let every = self.log.config().stat_interval;
        self.timers.start(Tk::LogStat, every);
    }

    /// 받지 않는 사건 — 조용히 무시하지 않는다(SDK:IMPL-LINK-PROJECTION 받지 않는 사건) · 전이 통에도 `impossible` 로 낸다
    pub(crate) fn impossible(&self, layer: &str, key: &str, from: &str, entry: &str) {
        if self.trace {
            eprintln!("[oxpeer] {layer} {key} {from} — 입구 {entry} 를 받지 않는다");
        }
        self.log_transition(layer, key, from, None, None, entry);
        self.diag_send(layer, key, from, None, None, entry);
    }

    fn diag_send(&self, area: &str, key: &str, from: &str, to: Option<&str>, label: Option<&str>, cause: &str) {
        let Ok(mut subs) = self.diag.lock() else { return };
        if subs.is_empty() {
            return;
        }
        let at_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let t = crate::api::Transition {
            at_ms,
            area: area.to_string(),
            key: key.to_string(),
            from: from.to_string(),
            to: to.map(str::to_string),
            label: label.map(str::to_string),
            cause: cause.to_string(),
            impossible: to.is_none(),
        };
        subs.retain(|s| s.send(t.clone()).is_ok());
    }

    pub(crate) fn emit(&self, e: Event) {
        #[cfg(feature = "record")]
        if let Some(r) = &self.sink.recorder
            && let Some(b) = crate::record::session_body(&e)
        {
            r.push(crate::record::Channel::Session, b.to_string().into_bytes());
        }
        let _ = self.ev.send(e);
    }

    /// 요청 — 끝은 drain 이 쓰임새로 가른다
    pub(crate) fn request(&mut self, op: crate::generated::ws_wire::Op, body: Value, purpose: Purpose) {
        let mut ended = std::mem::take(&mut self.ended);
        self.sig.request(op, body, purpose, &mut ended);
        self.ended = ended;
        self.watch_requests();
    }

    pub(crate) fn watch_requests(&mut self) {
        if self.sig.counting() && !self.timers.armed(&Tk::ReqCheck) {
            self.timers.start(Tk::ReqCheck, Duration::from_secs(1));
        }
    }

    /// 걸음 뒤 — 요청의 끝과 미룬 일을 바닥까지
    pub(crate) fn drain(&mut self) {
        for _ in 0..10_000 {
            if !self.ended.is_empty() {
                let batch = std::mem::take(&mut self.ended);
                for (p, o) in batch {
                    self.outcome(p, o);
                }
                continue;
            }
            if let Some(j) = self.jobs.pop_front() {
                match j {
                    Job::Negotiate(server) => self.negotiate(&server),
                }
                continue;
            }
            break;
        }
        self.settle_joins();
    }

    fn command(&mut self, c: Cmd) {
        match c {
            Cmd::Connect(tx) => self.connect(tx),
            Cmd::SetToken(t) => self.set_token(t),
            Cmd::Join(room, tx) => self.join(room, tx),
            Cmd::Leave(room, tx) => self.leave(room, tx),
            Cmd::Close(tx) => {
                self.close_for_app();
                let _ = tx.send(());
            }
            #[cfg(feature = "record")]
            Cmd::Records(tx, shared) => {
                if let Some(r) = &self.sink.recorder {
                    r.subscribe(tx, shared);
                }
            }
            Cmd::Diag(tx) => {
                if let Ok(mut g) = self.diag.lock() {
                    g.push(tx);
                }
            }
            Cmd::Stats(tx) => {
                let c = &self.sink.counters;
                let n = |a: &AtomicU64| a.load(Ordering::Relaxed);
                let _ = tx.send(Stats {
                    media_dropped: n(&self.sink.dropped),
                    media_unrouted: n(&self.sink.unrouted),
                    decrypt_fail: n(&self.sink.decrypt_fail),
                    frames_dropped: n(&self.sink.frames_dropped),
                    nack_sent: n(&c.nack_sent),
                    rtx_recovered: n(&c.rtx_recovered),
                    lost: n(&c.lost),
                    pli_sent: n(&c.pli_sent),
                    rr_sent: n(&c.rr_sent),
                    frames_broken: n(&c.frames_broken),
                    frames_skipped: n(&c.frames_skipped),
                    qa_dropped: n(&self.sink.qa_dropped),
                    rtx_sent: n(&self.sink.rtx_sent),
                    remb_bps: n(&self.sink.remb),
                    target_bps: self.bwe_view().0,
                    send_loss_permille: (self.bwe_view().1 * 1000.0).round() as u64,
                    frames_sent: self.pubs.values().map(|t| t.io.sent.load(Ordering::Relaxed)).sum(),
                    frames_gated: self.pubs.values().map(|t| t.io.gated.load(Ordering::Relaxed)).sum(),
                    qa_send_dropped: self.pubs.values().map(|t| t.io.qa_dropped.load(Ordering::Relaxed)).sum(),
                });
            }
            Cmd::SetUnit(track, unit) => {
                self.units.insert(track, unit);
                self.rebuild_routes();
            }
            Cmd::SetLayer(track, layer, tx) => self.set_layer(track, layer, tx),
            Cmd::RequestKey(track) => {
                *self.key_gen.entry(track).or_default() += 1;
                self.rebuild_routes();
            }
            Cmd::JoinSpeaking(room, tx) => self.join_speaking(room, tx),
            Cmd::SetSpeakingRoom(room, tx) => self.set_speaking_room(room, tx),
            Cmd::Publish(room, spec, frames, tx) => self.publish(room, spec, frames, tx),
            Cmd::Unpublish(id) => self.unpublish(id),
            Cmd::SetDuplex(id, half, tx) => self.set_duplex(id, half, tx),
            Cmd::RtpTx(id) => self.rtp_tx(id),
            Cmd::PttPress(room, p, tx) => {
                let _ = tx.send(self.ptt_press(&room, p));
            }
            Cmd::PttRelease(room, tx) => {
                let _ = tx.send(self.ptt_release(&room));
            }
            Cmd::Chat(space, content, msg_id, tx) => self.send_chat(space, content, msg_id, tx),
            Cmd::ChatCatchup(space, t, tx) => self.chat_catchup(space, t, tx),
            Cmd::Snapshot(tx) => {
                let _ = tx.send(self.snapshot());
            }
            Cmd::Log(line) => self.log.emit(line.into_line()),
        }
    }

    fn socket(&mut self, s: SockIn) {
        match s {
            SockIn::Open(g) => self.sock_open(g),
            SockIn::Closed(g) => self.sock_closed(g),
            SockIn::Frame(g, f) => {
                let mut ended = std::mem::take(&mut self.ended);
                let n = self.sig.receive(g, f, &mut ended);
                self.ended = ended;
                match n {
                    crate::sig::Got::Notice(op, body) => self.notify(op, body),
                    crate::sig::Got::Unknown(op) => self.log.emit(
                        crate::log::Line::new(crate::log::Kind::Diag)
                            .level(crate::log::Level::Warn)
                            .spec("RULE-TRN-NOTIFY-ACK")
                            .key("op", format!("{op:#06x}"))
                            .msg("unknown notify op - refused with 1001 and dropped"),
                    ),
                    crate::sig::Got::Nothing => {}
                }
            }
        }
    }

    /// 통지 — 받는 영역으로 가른다
    fn notify(&mut self, op: crate::generated::ws_wire::Op, body: Value) {
        use crate::generated::ws_wire::Op;
        let room_of = |b: &Value| b.get("room_id").and_then(Value::as_str).unwrap_or("").to_string();
        match op {
            Op::ParticipantEvent | Op::ParticipantState | Op::TrackEvent | Op::TrackState => {
                let r = room_of(&body);
                if self.rooms.contains_key(&r) {
                    self.room_delta(&r, op, body);
                }
            }
            Op::RoomEvent => {
                let r = room_of(&body);
                if self.rooms.contains_key(&r) {
                    self.room_event(&r, &body);
                }
            }
            Op::MlineEvent => {
                if let Some(t) = body.get("table").cloned() {
                    self.mline_event(t);
                }
            }
            Op::LinkEvent => {
                if let Some((server, ufrag)) = suspect_of(&body) {
                    self.link_event(&server, &ufrag);
                }
            }
            Op::Message => self.message_in(body),
            Op::Messages => self.messages_in(body),
            _ => self.record("SES", "notify", "-", "-", "SDK.notify_ack", op_name(op)),
        }
    }

    fn fire(&mut self, tk: Tk) {
        match tk {
            Tk::Ses(t) => self.ses_expired(t),
            Tk::Window => self.ses_window(),
            Tk::ReqCheck => {
                let mut ended = std::mem::take(&mut self.ended);
                let cut = self.sig.check_timeouts(&mut ended);
                self.ended = ended;
                if cut {
                    let mut ended = std::mem::take(&mut self.ended);
                    self.sig.close(&mut ended);
                    self.ended = ended;
                    let g = self.sig.generation();
                    self.sock_closed_now(g);
                }
                self.watch_requests();
            }
            Tk::Jitter => self.jitter_due(),
            Tk::RoomRetry(r) => self.room_retry_due(&r),
            Tk::RoomResend(r) => self.room_resend(&r),
            Tk::Resync(r) => self.resync(&r),
            Tk::Floor(r, t) => self.floor_expire(&r, t),
            Tk::LinkGrace(s) => self.link_grace(&s),
            Tk::LinkApply(s) => self.link_apply_timeout(&s),
            Tk::LinkReopenTry(s) => self.link_reopen_try(&s),
            Tk::LinkReopenJudge(s) => self.link_reopen_judge(&s),
            Tk::LinkStable(s) => self.link_stable(&s),
            Tk::LinkCheck(s) => self.link_check_send(&s),
            Tk::LinkProbe(s) => self.link_probe_due(&s),
            Tk::LogStat => self.log_stat(),
        }
    }

    /// 요청의 끝 — 쓰임새대로 받는 곳에
    fn outcome(&mut self, p: Purpose, o: Outcome) {
        match p {
            Purpose::Bind => self.bind_reply(o),
            Purpose::Resume => self.resume_reply(o),
            Purpose::Heartbeat => {}
            Purpose::RoomJoin { room, epoch, jid } => self.join_reply(&room, epoch, jid, o),
            Purpose::RoomLeave { room, epoch } => self.leave_reply(&room, epoch, o),
            Purpose::Ready { room, epoch } => self.ready_reply(&room, epoch, o),
            Purpose::Layer { room, epoch, id } => self.layer_reply(&room, epoch, id, o),
            Purpose::IceRestart { server, req } => self.restart_reply(&server, req, o),
            Purpose::Publish { track, pid } => self.publish_reply(track, pid, o),
            Purpose::Unpublish => {}
            Purpose::TrackSet { track } => self.track_set_reply(track, o),
            Purpose::Affiliation { server } => self.affiliation_reply(server, o),
            Purpose::Message { id } => self.message_reply(id, o),
        }
    }

    /// 발행 연결들의 대역 추정 — (가장 큰 목표, 그 연결의 손실률) · 발행 연결이 없으면 (0, 0)
    fn bwe_view(&self) -> (u64, f64) {
        let outs: Vec<PubOut> = self.pub_outs.read().map(|g| g.values().filter_map(|o| o.get().cloned()).collect()).unwrap_or_default();
        outs.iter().map(|o| o.bwe_now()).max_by_key(|x| x.0).unwrap_or((0, 0.0))
    }

    /// 재동기 — HTTP(세션 헤더 · 연:RULE-TRN-HTTP-AUTH-TWO) · 응답은 루프로 돌아온다
    pub(crate) fn spawn_resync(&self, room: &str, epoch: u64) {
        let (base, tls, sid, token, tx, r) = (self.base.clone(), self.tls.clone(), self.session_id.clone(), self.token.clone(), self.http_tx.clone(), room.to_string());
        tokio::spawn(async move {
            let path = format!("/rooms/{}?tracks=1", url_escape(&r));
            let header = match &sid {
                Some(s) => ("X-OxLens-Session", s.clone()),
                None => ("Authorization", format!("Bearer {token}")),
            };
            let result = http::call(&base, tls.as_ref(), "GET", &path, &[(header.0, header.1.as_str())], None).await;
            let _ = tx.send(HttpDone { room: r, epoch, result });
        });
    }
}

/// 요청 본문 — ★생성 wire 의 요청 형(연동규격서가 정한 칸)에서만 짓는다(손으로 json 을 짓지 않는다 — 빠진 칸을 형이 막는다 · 20261108d)
pub(crate) fn to_body<T: serde::Serialize>(req: &T) -> Value {
    serde_json::to_value(req).unwrap_or(Value::Null)
}

pub(crate) fn op_name(op: crate::generated::ws_wire::Op) -> &'static str {
    use crate::generated::ws_wire::Op;
    match op {
        Op::ParticipantEvent => "PARTICIPANT_EVENT",
        Op::ParticipantState => "PARTICIPANT_STATE",
        Op::TrackEvent => "TRACK_EVENT",
        Op::TrackState => "TRACK_STATE",
        Op::RoomEvent => "ROOM_EVENT",
        Op::MlineEvent => "MLINE_EVENT",
        Op::LinkEvent => "LINK_EVENT",
        Op::Message => "MESSAGE",
        Op::Task => "TASK",
        _ => "OTHER",
    }
}

fn url_escape(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn suspect_of(body: &Value) -> Option<(String, String)> {
    if body.get("type").and_then(Value::as_str) != Some("suspect") {
        return None;
    }
    let server = body.get("sfu_id").and_then(Value::as_str).unwrap_or("").to_string();
    let ufrag = body.get("ufrag").and_then(Value::as_str).unwrap_or("").to_string();
    Some((server, ufrag))
}

#[cfg(test)]
mod link_event_test {
    use super::suspect_of;
    use serde_json::json;

    #[test]
    fn only_suspect_reaches_the_link() {
        let suspect = json!({ "type": "suspect", "sfu_id": "sfu-a", "ufrag": "pubUf1" });
        assert_eq!(suspect_of(&suspect), Some(("sfu-a".to_string(), "pubUf1".to_string())), "CP-OP-LINK-EVENT 정의된 type suspect 는 연결의 세대 견줌으로 간다");
        let probe = json!({ "type": "probe", "sfu_id": "sfu-a", "ufrag": "pubUf1" });
        assert_eq!(suspect_of(&probe), None, "CP-OP-LINK-EVENT④ 모르는 type 은 ACK 만 - 지금 세대 ufrag 를 실어 와도 복구 방아쇠 0");
        let bare = json!({ "sfu_id": "sfu-a", "ufrag": "pubUf1" });
        assert_eq!(suspect_of(&bare), None, "CP-OP-LINK-EVENT④ type 없음도 ACK 만");
    }
}
