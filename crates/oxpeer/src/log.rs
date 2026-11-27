// author: kodeholic (powered by Claude)
//! 운영 로그(SDK:RULE-PLAT-SERVER 「운영 로그」 · 줄 꼴과 보내는 쪽은 서버와 같다 · context 202611/20261109e).
//! 줄 = logfmt 한 줄 — 머리(`ts` · `mono` · `node` · `proc=peer` · `kind` · `spec`) → 상관 키(`session` · `room` · `user` · `epoch` · `track` · `ssrc`) → 레코드 값 → `msg`.
//! ★보내는 쪽 — 유한 채널 · 전담 스레드 하나. 채널이 차면 그 자리에서 버리고 센다(기다리지 않는다 — 로그 한 줄이 녹음 · 발화를 멈추게 하지 않는다).
//! 로거 소켓(`SOCK_DGRAM`)이 있으면 거기로 · 보내기에 실패하면 spool 파일(디렉터리를 줬으면) 아니면 버림 + 계수 · 소켓이 없으면 표준 출력.
//! 회복 첫 줄로 `kind=stat spooled= dropped=` 를 낸다. 줄 상한을 넘으면 `msg` 를 자르고 `truncated=1`.

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// 운영 로그 설정
#[derive(Clone, Debug)]
pub struct LogConfig {
    /// 끄기(앱이 명시로 고른 것) — 기본은 켬
    pub enabled: bool,
    /// hub 로거 소켓(같은 장비 — 서버 줄과 한 파일) · 없으면 표준 출력
    pub socket: Option<PathBuf>,
    /// 소켓에 못 보낼 때 줄을 받을 디렉터리(`{node}.peer.spool.log`) · 없으면 버리고 센다
    pub spool_dir: Option<PathBuf>,
    /// 줄의 `node` — 이 프로그램이 도는 장비 이름
    pub node: String,
    /// `stat` 주기(정책 `logging.stat_interval_ms` 기본값과 같은 5초)
    pub stat_interval: Duration,
    pub queue_max_lines: usize,
    pub line_max_bytes: usize,
    pub spool_max_bytes: u64,
    /// 진단 줄(`diag`)만 거르는 수준 — `ev` · `stat` 은 늘 낸다(환경변수 `OX_LOG_LEVEL` 이 기동 때 덮는다)
    pub level: Level,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn parse(s: &str) -> Option<Level> {
        Some(match s.to_ascii_lowercase().as_str() {
            "trace" => Level::Trace,
            "debug" => Level::Debug,
            "info" => Level::Info,
            "warn" => Level::Warn,
            "error" => Level::Error,
            _ => return None,
        })
    }
}

/// 기본값 — 환경변수가 기동 때 덮는다(서버의 `OX_LOG_LEVEL` 과 같은 꼴): `OX_LOG_SOCKET`(hub 로거 소켓) · `OX_NODE`(장비 이름) · `OX_LOG_STAT_MS`(`stat` 주기) · `OX_LOG_SPOOL_DIR`
impl Default for LogConfig {
    fn default() -> LogConfig {
        let env = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
        LogConfig {
            enabled: true,
            socket: env("OX_LOG_SOCKET").map(PathBuf::from),
            spool_dir: env("OX_LOG_SPOOL_DIR").map(PathBuf::from),
            node: env("OX_NODE").unwrap_or_else(|| "peer".into()),
            stat_interval: Duration::from_millis(env("OX_LOG_STAT_MS").and_then(|v| v.parse().ok()).unwrap_or(5_000)),
            queue_max_lines: 8_192,
            line_max_bytes: 1_024,
            spool_max_bytes: 64 << 20,
            level: Level::Info,
        }
    }
}

/// 줄의 종류(운영 `kind`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Ev,
    Stat,
    Diag,
}

/// 줄 하나 — 칸 차례는 넣는 쪽이 지킨다(머리는 `Logger` 가 · 상관 키 → 레코드 값 → `msg`)
pub(crate) struct Line {
    kind: Kind,
    level: Level,
    spec: Option<String>,
    corr: Vec<(&'static str, String)>,
    rest: Vec<(String, String)>,
    msg: Option<String>,
}

/// 앱이 내는 운영 로그 줄(SDK:RULE-PLAT-SERVER 운영 로그 — 앱 몫의 판단 · 계수도 같은 로거 · 같은 꼴로) — `Client::log` 로 낸다.
/// 머리 · `session` · `user` · `epoch` 는 SDK 가 붙인다 · `diag` 만 수준으로 걸러진다 · 채널이 차면 버리고 센다
#[derive(Clone, Debug)]
pub struct AppLine {
    pub(crate) kind: AppKind,
    pub(crate) level: Level,
    pub(crate) spec: Option<String>,
    pub(crate) room: Option<String>,
    pub(crate) keys: Vec<(String, String)>,
    pub(crate) msg: Option<String>,
}

/// 줄 종류 — `ev` 사건 · `stat` 계수 · `diag` 진단
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppKind {
    Ev,
    Stat,
    Diag,
}

impl AppLine {
    pub fn new(kind: AppKind) -> AppLine {
        AppLine { kind, level: Level::Info, spec: None, room: None, keys: Vec::new(), msg: None }
    }
    pub fn level(mut self, l: Level) -> AppLine {
        self.level = l;
        self
    }
    /// 판단 근거 — 규격 문장 라벨
    pub fn spec(mut self, s: impl Into<String>) -> AppLine {
        self.spec = Some(s.into());
        self
    }
    pub fn room(mut self, r: impl Into<String>) -> AppLine {
        self.room = Some(r.into());
        self
    }
    pub fn key(mut self, k: impl Into<String>, v: impl ToString) -> AppLine {
        self.keys.push((k.into(), v.to_string()));
        self
    }
    pub fn msg(mut self, m: impl Into<String>) -> AppLine {
        self.msg = Some(m.into());
        self
    }

    pub(crate) fn into_line(self) -> Line {
        let kind = match self.kind {
            AppKind::Ev => Kind::Ev,
            AppKind::Stat => Kind::Stat,
            AppKind::Diag => Kind::Diag,
        };
        let mut l = Line::new(kind);
        l.level = self.level;
        l.spec = self.spec;
        if let Some(r) = self.room {
            l = l.corr("room", r);
        }
        for (k, v) in self.keys {
            l = l.key(k, v);
        }
        l.msg = self.msg;
        l
    }
}

/// 상관 키 차례(운영 줄 모형 그대로)
const CORR: [&str; 6] = ["session", "room", "user", "epoch", "track", "ssrc"];

impl Line {
    pub(crate) fn new(kind: Kind) -> Line {
        Line { kind, level: Level::Info, spec: None, corr: Vec::new(), rest: Vec::new(), msg: None }
    }
    #[cfg_attr(not(feature = "record"), allow(dead_code))]
    pub(crate) fn level(mut self, l: Level) -> Line {
        self.level = l;
        self
    }
    pub(crate) fn spec(mut self, s: impl Into<String>) -> Line {
        self.spec = Some(s.into());
        self
    }
    pub(crate) fn corr(mut self, k: &'static str, v: impl ToString) -> Line {
        let v = v.to_string();
        if !v.is_empty() {
            self.corr.retain(|(x, _)| *x != k);
            self.corr.push((k, v));
        }
        self
    }
    pub(crate) fn key(mut self, k: impl Into<String>, v: impl ToString) -> Line {
        let v = v.to_string();
        if !v.is_empty() {
            self.rest.push((k.into(), v));
        }
        self
    }
    pub(crate) fn msg(mut self, m: impl Into<String>) -> Line {
        self.msg = Some(m.into());
        self
    }
}

/// 운영 로거 — 프로세스 하나에 하나(Client 마다 하나여도 줄은 같은 꼴 · 같은 `epoch` 가 아니다)
pub(crate) struct Logger {
    cfg: LogConfig,
    epoch: String,
    mono0: Instant,
    tx: Option<mpsc::SyncSender<String>>,
    dropped: Arc<AtomicU64>,
    /// 세션 귀속 — BIND 뒤 채운다
    who: Mutex<(Option<String>, Option<String>)>,
}

impl Logger {
    pub(crate) fn start(mut cfg: LogConfig) -> Arc<Logger> {
        if let Some(l) = std::env::var("OX_LOG_LEVEL").ok().and_then(|v| Level::parse(&v)) {
            cfg.level = l;
        }
        let mut e = [0u8; 8];
        let _ = getrandom::fill(&mut e);
        let epoch: String = e.iter().map(|b| format!("{b:02x}")).collect();
        let dropped = Arc::new(AtomicU64::new(0));
        let mono0 = Instant::now();
        let tx = cfg.enabled.then(|| {
            let (tx, rx) = mpsc::sync_channel::<String>(cfg.queue_max_lines.max(16));
            let me = Me { node: cfg.node.clone(), epoch: epoch.clone(), mono0 };
            let (sock, spool, cap, d) = (cfg.socket.clone(), cfg.spool_dir.clone(), cfg.spool_max_bytes, dropped.clone());
            let _ = std::thread::Builder::new().name("oxpeer-log".into()).spawn(move || sender(rx, sock, spool, me, cap, d));
            tx
        });
        Arc::new(Logger { cfg, epoch, mono0, tx, dropped, who: Mutex::new((None, None)) })
    }

    pub(crate) fn config(&self) -> &LogConfig {
        &self.cfg
    }

    /// 세션 귀속 — 세션 지문(sha256 앞 4바이트 — 연:RULE-SES-FINGERPRINT)과 사용자
    pub(crate) fn bind(&self, session_id: Option<&str>, user: Option<&str>) {
        if let Ok(mut g) = self.who.lock() {
            *g = (session_id.map(fingerprint), user.map(str::to_string));
        }
    }

    /// 줄 하나를 낸다 — `diag` 만 수준으로 거른다
    pub(crate) fn emit(&self, l: Line) {
        let Some(tx) = &self.tx else { return };
        if l.kind == Kind::Diag && l.level < self.cfg.level {
            return;
        }
        let text = self.render(self.prepare(l));
        if tx.try_send(text).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 세션 · 사용자 · 기동 신원을 붙인다(줄이 따로 준 값이 먼저)
    fn prepare(&self, mut l: Line) -> Line {
        if let Ok(g) = self.who.lock() {
            if let Some(s) = &g.0
                && !l.corr.iter().any(|(k, _)| *k == "session")
            {
                l.corr.push(("session", s.clone()));
            }
            if let Some(u) = &g.1
                && !l.corr.iter().any(|(k, _)| *k == "user")
            {
                l.corr.push(("user", u.clone()));
            }
        }
        l.corr.push(("epoch", self.epoch.clone()));
        l
    }

    #[cfg(test)]
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn render(&self, l: Line) -> String {
        let mut s = String::with_capacity(256);
        let kind = match l.kind {
            Kind::Ev => "ev",
            Kind::Stat => "stat",
            Kind::Diag => "diag",
        };
        let _ = write!(s, "ts={} mono={} node={} proc=peer kind={kind}", iso_now(), self.mono0.elapsed().as_micros(), val(&self.cfg.node));
        if let Some(sp) = &l.spec {
            let _ = write!(s, " spec={}", val(sp));
        }
        for k in CORR {
            if let Some((_, v)) = l.corr.iter().find(|(x, _)| *x == k) {
                let _ = write!(s, " {k}={}", val(v));
            }
        }
        for (k, v) in &l.rest {
            let _ = write!(s, " {k}={}", val(v));
        }
        let max = self.cfg.line_max_bytes;
        if let Some(m) = &l.msg {
            let room = max.saturating_sub(s.len() + 7);
            if m.len() + 2 > room {
                let mut cut = room.saturating_sub(16);
                while cut > 0 && !m.is_char_boundary(cut) {
                    cut -= 1;
                }
                let _ = write!(s, " truncated=1 msg={}", val(&m[..cut]));
            } else {
                let _ = write!(s, " msg={}", val(m));
            }
        }
        s
    }
}

/// 값 — 공백 · 따옴표 · `=` 가 있으면 큰따옴표(안의 큰따옴표는 작은따옴표로)
fn val(v: &str) -> String {
    if v.chars().any(|c| c.is_whitespace() || c == '"' || c == '=') {
        format!("\"{}\"", v.replace('"', "'").replace(['\n', '\r'], " "))
    } else {
        v.to_string()
    }
}

/// 세션 지문(연:RULE-SES-FINGERPRINT — 서버 `common::log::fingerprint` 와 같은 셈)
pub(crate) fn fingerprint(session_id: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(session_id.as_bytes()).iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// 로컬 시각 · µs · UTC 오프셋(운영 `ts` — 「로컬 시각 · 오프셋 필수」 · 서버 `common::log` 와 같은 꼴)
fn iso_now() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f%:z").to_string()
}

/// 전담 스레드가 회복 첫 줄을 짓는 데 쓰는 신원
struct Me {
    node: String,
    epoch: String,
    mono0: Instant,
}

/// 전담 스레드 — 소켓 → (실패) spool → (없음 · 상한) 버림. 소켓이 없으면 표준 출력
fn sender(rx: mpsc::Receiver<String>, sock: Option<PathBuf>, spool: Option<PathBuf>, me: Me, cap: u64, dropped: Arc<AtomicU64>) {
    let node = me.node.clone();
    let dg = sock.as_ref().and_then(|_| UnixDatagram::unbound().ok());
    if let Some(d) = &dg {
        let _ = d.set_nonblocking(true);
    }
    let mut spooled = 0u64;
    let mut spool_bytes = 0u64;
    let mut failing = false;
    let mut spool_file = None;
    let out = std::io::stdout();
    while let Ok(line) = rx.recv() {
        let (Some(path), Some(d)) = (&sock, &dg) else {
            let mut o = out.lock();
            let _ = writeln!(o, "{line}");
            continue;
        };
        if d.send_to(line.as_bytes(), path).is_ok() {
            if failing {
                failing = false;
                // 회복 첫 줄(운영:RULE-LOG-SENDER ⑤ 와 같은 꼴)
                let back = format!("ts={} mono={} node={} proc=peer kind=stat epoch={} spooled={spooled} dropped={}", iso_now(), me.mono0.elapsed().as_micros(), val(&me.node), me.epoch, dropped.load(Ordering::Relaxed));
                let _ = d.send_to(back.as_bytes(), path);
            }
            continue;
        }
        failing = true;
        let Some(dir) = &spool else {
            dropped.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        if spool_bytes + line.len() as u64 + 1 > cap {
            dropped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if spool_file.is_none() {
            spool_file = OpenOptions::new().create(true).append(true).open(dir.join(format!("{node}.peer.spool.log"))).ok();
        }
        match spool_file.as_mut().map(|f| writeln!(f, "{line}")) {
            Some(Ok(())) => {
                spooled += 1;
                spool_bytes += line.len() as u64 + 1;
            }
            _ => {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// 프로세스 자원 — CPU 시간 누계(ms) · 최대 상주 메모리(바이트)
pub(crate) fn rusage() -> Option<(u64, u64)> {
    use nix::sys::resource::{UsageWho, getrusage};
    let u = getrusage(UsageWho::RUSAGE_SELF).ok()?;
    let tv = |t: nix::sys::time::TimeVal| t.tv_sec() as u64 * 1_000 + t.tv_usec() as u64 / 1_000;
    let rss = u.max_rss() as u64;
    // macOS 는 바이트 · 리눅스는 KiB
    let rss = if cfg!(target_os = "macos") { rss } else { rss * 1_024 };
    Some((tv(u.user_time()) + tv(u.system_time()), rss))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logger() -> (Arc<Logger>, Line) {
        let l = Logger::start(LogConfig { enabled: false, node: "n1".into(), ..LogConfig::default() });
        (l, Line::new(Kind::Ev))
    }

    #[test]
    fn head_then_corr_in_model_order_then_rest_then_msg() {
        let (lg, _) = logger();
        lg.bind(Some("sess-1"), Some("u1"));
        let s = lg.render(Line::new(Kind::Ev).spec("SES.C.x").corr("ssrc", 7).corr("room", "r1").key("area", "SES").key("from", "A").msg("hello world"));
        let keys: Vec<&str> = s.split(' ').filter_map(|kv| kv.split_once('=').map(|x| x.0)).filter(|k| !k.is_empty() && !k.contains('"')).collect();
        assert_eq!(&keys[..6], &["ts", "mono", "node", "proc", "kind", "spec"]);
        assert!(s.contains(" room=r1 ssrc=7 area=SES from=A msg=\"hello world\""), "{s}");
        assert!(s.contains(" mono="));
    }

    #[test]
    fn app_line_renders_in_the_same_shape() {
        let (lg, _) = logger();
        lg.bind(Some("sess-1"), Some("agent-1"));
        let s = lg.render(lg.prepare(AppLine::new(AppKind::Diag).level(Level::Warn).spec("SDK:RULE-PLAT-AGENT-SILENCE").room("ch-1").key("reply", "r7").msg("reply truncated").into_line()));
        assert!(s.contains(" proc=peer kind=diag spec=SDK:RULE-PLAT-AGENT-SILENCE"), "{s}");
        assert!(s.contains(&format!(" session={} room=ch-1 user=agent-1 epoch={} reply=r7 msg=\"reply truncated\"", fingerprint("sess-1"), lg.epoch)), "{s}");
    }

    #[test]
    fn emit_attaches_session_user_epoch_after_bind() {
        let (lg, _) = logger();
        lg.bind(Some("sess-1"), Some("u1"));
        let s = lg.render(lg.prepare(Line::new(Kind::Ev).spec("x")));
        assert!(s.contains(&format!(" session={} user=u1 epoch={}", fingerprint("sess-1"), lg.epoch)), "{s}");
    }

    #[test]
    fn long_msg_is_truncated_under_line_cap() {
        let (lg, _) = logger();
        let s = lg.render(Line::new(Kind::Diag).msg("가".repeat(800)));
        assert!(s.len() <= 1_024, "{}", s.len());
        assert!(s.contains(" truncated=1 msg="));
    }

    #[test]
    fn full_channel_drops_and_counts_never_blocks() {
        let lg = Logger::start(LogConfig { socket: Some(PathBuf::from("/nonexistent/oxpeer-test.sock")), queue_max_lines: 16, ..LogConfig::default() });
        let t = Instant::now();
        for i in 0..100_000 {
            lg.emit(Line::new(Kind::Diag).msg(format!("line {i}")));
        }
        assert!(t.elapsed() < Duration::from_secs(2), "기다리지 않는다");
        assert!(lg.dropped() > 0, "소켓도 spool 도 없으면 버리고 센다");
    }

    #[test]
    fn ts_is_local_time_with_offset() {
        let t = iso_now();
        assert_eq!(t.len(), "2026-11-09T21:00:00.123456+09:00".len(), "{t}");
        assert!(t.as_bytes()[26] == b'+' || t.as_bytes()[26] == b'-', "{t}");
    }

    #[test]
    fn rusage_reads() {
        let (cpu, rss) = rusage().unwrap();
        assert!(rss > 0);
        let _ = cpu;
    }
}
