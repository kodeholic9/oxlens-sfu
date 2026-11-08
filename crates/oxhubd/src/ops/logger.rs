// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-OPS-LOGGER · 운영:RULE-LOG-LOGGER · 운영:RULE-LOG-SENDER · 운영:RULE-READ-HISTORY · model: claude-opus-5-5
//! 로거 유닛(hub 안) — 남은 소켓 파일을 지우고 bind · 수신 버퍼 · 파일 하나(날짜 ∨ 크기 회전) · 보존 · hub 줄은 채널로 바로 · bind 와 파일 연 뒤 준비 · 제 파일을 읽어 이력에 답한다(쓰기 스레드 밖).

use std::io::Write;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use common::log::Sender;
use common::policy::Logging;

pub struct LoggerCfg {
    pub socket: PathBuf,
    pub dir: PathBuf,
    pub policy: Logging,
}

pub fn start(cfg: LoggerCfg, sender: &Arc<Sender>) -> Result<(), String> {
    std::fs::create_dir_all(&cfg.dir).map_err(|e| format!("log_dir {}: {e}", cfg.dir.display()))?;
    if cfg.socket.exists() {
        let _ = std::fs::remove_file(&cfg.socket);
    }
    if let Some(p) = cfg.socket.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let sock = UnixDatagram::bind(&cfg.socket).map_err(|e| format!("log socket {}: {e}", cfg.socket.display()))?;
    let s2 = socket2::SockRef::from(&sock);
    let _ = s2.set_recv_buffer_size(cfg.policy.socket_rcvbuf_bytes as usize);
    let mut w = FileWriter::open(&cfg.dir, cfg.policy.rotate_max_bytes, cfg.policy.retention_days)?;
    let (tx, rx): (SyncSender<String>, Receiver<String>) = sync_channel(cfg.policy.queue_max_lines.max(256) as usize);
    let tx2 = tx.clone();
    std::thread::Builder::new()
        .name("log-recv".into())
        .spawn(move || {
            let mut buf = vec![0u8; 4096];
            while let Ok(n) = sock.recv(&mut buf) {
                let line = String::from_utf8_lossy(&buf[..n]).trim_end_matches('\n').to_string();
                if tx2.try_send(line).is_err() {
                    continue;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("log-write".into())
        .spawn(move || {
            while let Ok(line) = rx.recv() {
                w.write(&line);
            }
        })
        .map_err(|e| e.to_string())?;
    sender.attach_inproc(tx);
    Ok(())
}

struct FileWriter {
    dir: PathBuf,
    date: String,
    n: u32,
    file: std::io::BufWriter<std::fs::File>,
    bytes: u64,
    max: u64,
    retention_days: u16,
    lines_since_flush: u32,
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn name(date: &str, n: u32) -> String {
    if n == 0 { format!("ox.log.{date}") } else { format!("ox.log.{date}.{n}") }
}

impl FileWriter {
    fn open(dir: &Path, max: u64, retention_days: u16) -> Result<FileWriter, String> {
        let date = today();
        let mut n = 0;
        while dir.join(name(&date, n + 1)).exists() {
            n += 1;
        }
        let path = dir.join(name(&date, n));
        let f = std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let bytes = f.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(FileWriter {
            dir: dir.to_path_buf(),
            date,
            n,
            file: std::io::BufWriter::new(f),
            bytes,
            max,
            retention_days,
            lines_since_flush: 0,
        })
    }

    fn rotate(&mut self) {
        let _ = self.file.flush();
        let d = today();
        if d != self.date {
            self.date = d;
            self.n = 0;
        } else {
            self.n += 1;
        }
        let path = self.dir.join(name(&self.date, self.n));
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            self.bytes = f.metadata().map(|m| m.len()).unwrap_or(0);
            self.file = std::io::BufWriter::new(f);
        }
        self.retain();
    }

    fn retain(&self) {
        if self.retention_days == 0 {
            return;
        }
        let horizon = std::time::Duration::from_secs(self.retention_days as u64 * 86_400);
        let current = name(&self.date, self.n);
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let fname = e.file_name().to_string_lossy().to_string();
                let ours = fname.starts_with("ox.log.") || fname.ends_with(".spool.log");
                if !ours || fname == current {
                    continue;
                }
                let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map(|a| a > horizon);
                if old == Some(true) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }

    fn write(&mut self, line: &str) {
        if today() != self.date || self.bytes + line.len() as u64 + 1 > self.max {
            self.rotate();
        }
        if writeln!(self.file, "{line}").is_ok() {
            self.bytes += line.len() as u64 + 1;
        }
        self.lines_since_flush += 1;
        if self.lines_since_flush >= 1 {
            let _ = self.file.flush();
            self.lines_since_flush = 0;
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Ask {
    pub user: Option<String>,
    pub room: Option<String>,
    pub session: Option<String>,
    pub since: u64,
    pub until: u64,
    pub diag: bool,
    pub stat: bool,
    pub max: usize,
}

#[derive(Debug, Default, PartialEq)]
pub struct Found {
    pub lines: Vec<String>,
    pub boots: Vec<String>,
    pub truncated: bool,
}

type Proc = (String, Option<String>);

fn get<'a>(f: &'a [(String, String)], k: &str) -> Option<&'a str> {
    f.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str())
}

fn ts_ms(f: &[(String, String)]) -> Option<u64> {
    let ts = get(f, "ts")?;
    chrono::DateTime::parse_from_rfc3339(ts).ok().map(|t| t.timestamp_millis().max(0) as u64)
}

fn proc_of(f: &[(String, String)]) -> Option<Proc> {
    Some((get(f, "proc")?.to_string(), get(f, "epoch").map(|e| e.to_string())))
}

fn is_boot(f: &[(String, String)]) -> bool {
    get(f, "spec") == Some("SV-STATE-UNIT-STATE") && get(f, "build").is_some()
}

impl Ask {
    fn picks(&self, f: &[(String, String)]) -> bool {
        let kind_ok = match get(f, "kind") {
            Some("ev") => true,
            Some("diag") => self.diag,
            Some("stat") => self.stat,
            _ => false,
        };
        let eq = |k: &str, want: &Option<String>| want.as_deref().is_none_or(|w| get(f, k) == Some(w));
        kind_ok
            && eq("user", &self.user)
            && eq("room", &self.room)
            && eq("session", &self.session)
            && ts_ms(f).is_some_and(|t| t >= self.since && t <= self.until)
    }
}

fn files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut logs: Vec<((String, u32), PathBuf, u64)> = Vec::new();
    let mut spools: Vec<(String, PathBuf, u64)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    for e in rd.flatten() {
        let fname = e.file_name().to_string_lossy().to_string();
        let mtime = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if let Some(rest) = fname.strip_prefix("ox.log.") {
            let mut parts = rest.splitn(2, '.');
            let date = parts.next().unwrap_or("").to_string();
            let n = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
            logs.push(((date, n), e.path(), mtime));
        } else if fname.ends_with(".spool.log") {
            spools.push((fname, e.path(), mtime));
        }
    }
    logs.sort_by(|a, b| a.0.cmp(&b.0));
    spools.sort_by(|a, b| a.0.cmp(&b.0));
    logs.into_iter().map(|(_, p, m)| (p, m)).chain(spools.into_iter().map(|(_, p, m)| (p, m))).collect()
}

fn each_line(path: &Path, mut f: impl FnMut(&str)) {
    use std::io::BufRead;
    let Ok(file) = std::fs::File::open(path) else { return };
    for line in std::io::BufReader::new(file).split(b'\n').map_while(Result::ok) {
        f(String::from_utf8_lossy(&line).trim_end_matches('\r'));
    }
}

type Boots = std::collections::HashMap<Proc, Vec<(u64, String)>>;

fn boot_of(boots: &Boots, p: &Proc, t: u64) -> Option<String> {
    boots.get(p)?.iter().filter(|(bt, _)| *bt <= t).max_by_key(|(bt, _)| *bt).map(|(_, l)| l.clone())
}

fn unmatched(kept: &std::collections::VecDeque<(String, Proc, u64)>, boots: &Boots) -> Vec<Proc> {
    let mut out: Vec<Proc> = Vec::new();
    for (_, p, t) in kept {
        if boot_of(boots, p, *t).is_none() && !out.contains(p) {
            out.push(p.clone());
        }
    }
    out
}

pub fn read(dir: &Path, ask: &Ask) -> Found {
    let all = files(dir);
    let cut = all.iter().position(|(_, m)| *m >= ask.since).unwrap_or(all.len());
    let needle = match (&ask.user, &ask.room) {
        (Some(u), _) => format!("user={}", common::log::line::quote(u)),
        (None, Some(r)) => format!("room={}", common::log::line::quote(r)),
        (None, None) => String::new(),
    };
    let mut boots: Boots = Boots::new();
    let mut kept: std::collections::VecDeque<(String, Proc, u64)> = std::collections::VecDeque::new();
    let mut truncated = false;
    for (path, _) in &all[cut..] {
        each_line(path, |line| {
            let boot_like = line.contains(" build=");
            if !boot_like && !line.contains(&needle) {
                return;
            }
            let f = common::log::line::fields(line);
            let Some(p) = proc_of(&f) else { return };
            let t = ts_ms(&f).unwrap_or(0);
            if boot_like && is_boot(&f) {
                boots.entry(p.clone()).or_default().push((t, line.to_string()));
            }
            if !ask.picks(&f) {
                return;
            }
            kept.push_back((line.to_string(), p, t));
            if kept.len() > ask.max {
                kept.pop_front();
                truncated = true;
            }
        });
    }
    let mut missing = unmatched(&kept, &boots);
    for (path, _) in all[..cut].iter().rev() {
        if missing.is_empty() {
            break;
        }
        each_line(path, |line| {
            if !line.contains(" build=") {
                return;
            }
            let f = common::log::line::fields(line);
            if let Some(p) = proc_of(&f)
                && is_boot(&f)
                && missing.contains(&p)
            {
                boots.entry(p).or_default().push((ts_ms(&f).unwrap_or(0), line.to_string()));
            }
        });
        missing = unmatched(&kept, &boots);
    }
    let mut out: Vec<String> = Vec::new();
    for (_, p, t) in &kept {
        if let Some(line) = boot_of(&boots, p, *t)
            && !out.contains(&line)
        {
            out.push(line);
        }
    }
    Found { lines: kept.into_iter().map(|(l, _, _)| l).collect(), boots: out, truncated }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: &str = "2026-11-08T10:00:00.000000+09:00";

    fn at(sec: u32) -> String {
        format!("2026-11-08T10:00:{sec:02}.000000+09:00")
    }

    fn ms(ts: &str) -> u64 {
        chrono::DateTime::parse_from_rfc3339(ts).unwrap().timestamp_millis() as u64
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oxhub-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(d: &Path, file: &str, lines: &[String]) {
        std::fs::write(d.join(file), lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    }

    fn ask(user: &str) -> Ask {
        Ask { user: Some(user.into()), since: ms(T0), until: ms(&at(59)), max: 100, ..Ask::default() }
    }

    #[test]
    fn picks_by_token_kind_and_window() {
        let d = dir("pick");
        let hub_boot = format!("ts={T0} mono=1 node=a proc=hub kind=ev spec=SV-STATE-UNIT-STATE build=b1 part=1 parts=2 msg=\"hub boot\"");
        let sfu_boot = format!("ts={} mono=2 node=a proc=sfu kind=ev spec=SV-STATE-UNIT-STATE epoch=e1 unit=a build=b1 msg=\"sfud boot\"", at(1));
        let ev = format!(
            "ts={} mono=3 node=a proc=sfu kind=ev spec=SV-STATE-PEER-STATE session=s1 user=u1 epoch=e1 area=LINK key=publish from=- to=Alive cause=peer_created",
            at(2)
        );
        let diag = format!("ts={} mono=4 node=a proc=sfu kind=diag session=s1 user=u1 epoch=e1 msg=\"transport ready\"", at(3));
        let quoted = format!("ts={} mono=5 node=a proc=sfu kind=ev spec=CP-OP-X epoch=e1 msg=\"user=u1 inside\"", at(4));
        let hub_ev = format!("ts={} mono=6 node=a proc=hub kind=ev spec=SES.S.bind_new session=s1 user=u1 area=SES key=s1 from=- to=live cause=BIND", at(5));
        let other = format!("ts={} mono=7 node=a proc=sfu kind=ev spec=CP-OP-X user=u2 epoch=e1", at(6));
        put(&d, "ox.log.2026-11-08", &[hub_boot.clone(), sfu_boot.clone(), ev.clone(), diag.clone(), quoted, hub_ev.clone(), other]);
        let f = read(&d, &ask("u1"));
        assert_eq!(f.lines, vec![ev.clone(), hub_ev.clone()], "O-sym-RULE-READ-HISTORY-1 파일 줄 그대로 · 토큰 일치(msg 안 user= 는 아니다)");
        assert_eq!(f.boots, vec![sfu_boot.clone(), hub_boot.clone()]);
        assert!(!f.truncated);
        let f = read(&d, &Ask { diag: true, ..ask("u1") });
        assert_eq!(f.lines, vec![ev.clone(), diag, hub_ev.clone()]);
        let f = read(&d, &Ask { since: ms(&at(3)), ..ask("u1") });
        assert_eq!(f.lines, vec![hub_ev]);
        assert_eq!(f.boots, vec![hub_boot]);
        let f = read(&d, &Ask { session: Some("s2".into()), ..ask("u1") });
        assert!(f.lines.is_empty() && f.boots.is_empty());
    }

    #[test]
    fn boot_comes_from_older_file_and_truncation_keeps_newest() {
        let d = dir("older");
        let sfu_boot = format!("ts={T0} mono=1 node=a proc=sfu kind=ev spec=SV-STATE-UNIT-STATE epoch=e9 unit=a build=b9 msg=\"sfud boot\"");
        put(&d, "ox.log.2026-11-07", std::slice::from_ref(&sfu_boot));
        let old = std::fs::File::options().write(true).open(d.join("ox.log.2026-11-07")).unwrap();
        old.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1)).unwrap();
        let evs: Vec<String> = (10..15).map(|s| format!("ts={} mono={s} node=a proc=sfu kind=ev spec=CP-OP-X user=u1 epoch=e9", at(s))).collect();
        put(&d, "ox.log.2026-11-08", &evs);
        let f = read(&d, &Ask { since: ms(&at(10)), max: 3, ..ask("u1") });
        assert_eq!(f.lines, evs[2..].to_vec(), "PL-KEY-LOGGING-HISTORY-MAX-LINES 줄 수 = 값 · 앞(오래된 쪽)을 자른다");
        assert!(f.truncated, "PL-KEY-LOGGING-HISTORY-MAX-LINES truncated");
        assert_eq!(f.boots, vec![sfu_boot]);
    }

    #[test]
    fn boot_in_spool_and_restart_pick_latest_before_line() {
        let d = dir("spool");
        let boot1 = format!("ts={} mono=1 node=a proc=hub kind=ev spec=SV-STATE-UNIT-STATE build=b1 msg=\"hub boot\"", at(1));
        let boot2 = format!("ts={} mono=1 node=a proc=hub kind=ev spec=SV-STATE-UNIT-STATE build=b2 msg=\"hub boot\"", at(20));
        let e1 = format!("ts={} mono=2 node=a proc=hub kind=ev spec=CP-OP-BIND session=s1 user=u1", at(5));
        let e2 = format!("ts={} mono=2 node=a proc=hub kind=ev spec=CP-OP-BIND session=s2 user=u1", at(25));
        put(&d, "ox.log.2026-11-08", &[e1.clone(), e2.clone()]);
        put(&d, "a.hub.spool.log", &[boot1.clone(), boot2.clone()]);
        let f = read(&d, &ask("u1"));
        assert_eq!(f.lines, vec![e1, e2]);
        assert_eq!(f.boots, vec![boot1, boot2], "O-sym-RULE-READ-HISTORY-4 줄마다 그 시각 앞 가장 늦은 기동 줄");
    }

    #[test]
    fn files_order_dates_then_rotations_then_spools() {
        let d = dir("order");
        for f in
            ["ox.log.2026-11-08.1", "a.hub.spool.log", "ox.log.2026-11-08", "ox.log.2026-11-07", "ox.log.2026-11-08.10", "ox.log.2026-11-08.2", "other.txt"]
        {
            put(&d, f, &[]);
        }
        let names: Vec<String> = files(&d).into_iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["ox.log.2026-11-07", "ox.log.2026-11-08", "ox.log.2026-11-08.1", "ox.log.2026-11-08.2", "ox.log.2026-11-08.10", "a.hub.spool.log"]);
    }
}
