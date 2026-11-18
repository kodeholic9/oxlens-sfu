// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-CHAT-STORE-FORMS · 정:IMPL-CHAT-STORE · 정:IMPL-CHAT-MEMBERSHIP · 정:IMPL-CHAT-READ · 정:IMPL-CHAT-UNDELIVERED · 정:IMPL-CHAT-CATCHUP · 정:IMPL-CHAT-STORE-FAULTS · 연:RULE-CHAT-MSG-ID · 연:RULE-CHAT-ORDER · 연:RULE-CHAT-REF-CHECK · 연:RULE-CHAT-UNDELIVERED · model: claude-opus-5-5
//! 채팅 저장소 PostgreSQL 꼴 — 연결 풀(`[chat] db_pool`)에서 요청마다 연결 하나 · 저장소가 쥔 런타임에서 돌고 부르는 쪽은 결과만 기다린다 · 원본 행은 `SELECT … FOR UPDATE` · `msg_id` 는 유일 제약 + `INSERT … ON CONFLICT`(먼저 찾아본 것은 이미 있는 행에만 쓴다) · 멤버 변경은 공간 행 잠금으로 줄 선다 · 문자열 키는 `COLLATE "C"`(바이트 차례 — 커서 비교가 SQLite 꼴과 같다) · 잠금 기다림 상한 · 진단 줄은 부른 쪽 스레드에서 남긴다 · 비밀번호는 `PGPASSWORD` · `~/.pgpass`(`PGPASSFILE`) · 기동 때 advisory 잠금 아래 표를 만든다.

use std::future::Future;
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use common::system::{ChatDb, Visibility};
use deadpool_postgres::{GenericClient, Manager, ManagerConfig, Object, Pool, RecyclingMethod};
use oxsig::Code;
use oxsig::msg::{ChatKind, ChatMessage, SpaceEntry};
use tokio_postgres::config::Host;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Config, NoTls, Row};

use common::talk::Report;

use super::{
    Ack, COLS, Cursor, Dir, Interval, Outgoing, Page, RELATION_KINDS, Roster, Sent, VISIBLE, again, fold_relations, fresh, member_msg, n, now_or, probe_of, ref_ok, since_floor, talk_merged, talk_missing, talk_msg,
    talk_started, talk_update_ok, u, within,
};

const LOCK_TIMEOUT_MS: u64 = 5_000;
const SCHEMA_LOCK: i64 = 0x6f78_6368_6174;
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS spaces (
  space_id TEXT COLLATE "C" PRIMARY KEY,
  last_t BIGINT,
  total BIGINT NOT NULL DEFAULT 0,
  ver BIGINT NOT NULL DEFAULT 0,
  visibility TEXT
);
CREATE TABLE IF NOT EXISTS members (
  space_id TEXT COLLATE "C" NOT NULL,
  user_id TEXT COLLATE "C" NOT NULL,
  join_t BIGINT NOT NULL,
  leave_t BIGINT,
  vis TEXT NOT NULL,
  PRIMARY KEY (space_id, user_id, join_t)
);
CREATE TABLE IF NOT EXISTS messages (
  msg_id TEXT COLLATE "C" PRIMARY KEY,
  space_id TEXT COLLATE "C" NOT NULL,
  user_id TEXT COLLATE "C" NOT NULL,
  t BIGINT NOT NULL,
  content TEXT NOT NULL,
  kind TEXT,
  target TEXT,
  ref_id TEXT COLLATE "C",
  keys TEXT,
  ext TEXT,
  deleted BOOLEAN NOT NULL DEFAULT FALSE,
  talk TEXT
);
ALTER TABLE messages ADD COLUMN IF NOT EXISTS talk TEXT;
CREATE INDEX IF NOT EXISTS messages_space_t_id ON messages (space_id, t, msg_id);
CREATE INDEX IF NOT EXISTS messages_ref ON messages (ref_id);
CREATE INDEX IF NOT EXISTS messages_t_id ON messages (t, msg_id);
CREATE TABLE IF NOT EXISTS reads (
  space_id TEXT COLLATE "C" NOT NULL,
  user_id TEXT COLLATE "C" NOT NULL,
  read BIGINT NOT NULL,
  PRIMARY KEY (space_id, user_id)
);
CREATE TABLE IF NOT EXISTS undelivered (
  user_id TEXT COLLATE "C" NOT NULL,
  msg_id TEXT COLLATE "C" NOT NULL,
  space_id TEXT COLLATE "C" NOT NULL,
  t BIGINT NOT NULL,
  PRIMARY KEY (user_id, msg_id)
);
CREATE INDEX IF NOT EXISTS undelivered_user_t ON undelivered (user_id, t, msg_id);
CREATE INDEX IF NOT EXISTS undelivered_t ON undelivered (t);
CREATE INDEX IF NOT EXISTS members_user ON members (user_id, leave_t);
"#;

const SEEN_JOINED: &str =
    " AND EXISTS(SELECT 1 FROM members m WHERE m.space_id = x.space_id AND m.user_id = $2 AND m.join_t <= x.t AND (m.leave_t IS NULL OR x.t < m.leave_t))";
const SEEN_ALL: &str = " AND $2::text = $2::text";

type Args<'a> = [&'a (dyn ToSql + Sync)];

fn cause(e: &tokio_postgres::Error) -> String {
    match e.code() {
        Some(c) => c.code().to_string(),
        None if e.is_closed() => "Closed".into(),
        None => "Other".into(),
    }
}

fn said(e: &tokio_postgres::Error) -> String {
    e.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| e.to_string())
}

type Line = (String, String, String);

tokio::task_local! {
    static LINES: Arc<Mutex<Vec<Line>>>;
}

fn diag((cause, at, detail): Line) {
    tracing::warn!(kind = "diag", spec = "SV-IMPL-CHAT-STORE-FAULTS", cause = %cause, at = %at, "chat store: {detail}");
}

fn note(line: Line) {
    let mut line = Some(line);
    let _ = LINES.try_with(|l| {
        if let (Ok(mut g), Some(x)) = (l.lock(), line.take()) {
            g.push(x);
        }
    });
    if let Some(x) = line {
        diag(x);
    }
}

fn pg_at<T>(r: Result<T, tokio_postgres::Error>, at: &'static Location<'static>) -> Result<T, Code> {
    r.map_err(|e| {
        note((cause(&e), format!("{}:{}", at.file(), at.line()), said(&e)));
        Code::InternalError
    })
}

#[track_caller]
fn pg<T>(r: Result<T, tokio_postgres::Error>) -> Result<T, Code> {
    pg_at(r, Location::caller())
}

#[track_caller]
fn all<'a, C: GenericClient>(c: &'a C, sql: &'a str, p: &'a Args<'a>) -> impl Future<Output = Result<Vec<Row>, Code>> + Send + 'a {
    let at = Location::caller();
    async move {
        let st = pg_at(c.prepare_cached(sql).await, at)?;
        pg_at(c.query(&st, p).await, at)
    }
}

#[track_caller]
fn one<'a, C: GenericClient>(c: &'a C, sql: &'a str, p: &'a Args<'a>) -> impl Future<Output = Result<Option<Row>, Code>> + Send + 'a {
    let at = Location::caller();
    async move {
        let st = pg_at(c.prepare_cached(sql).await, at)?;
        Ok(pg_at(c.query(&st, p).await, at)?.into_iter().next())
    }
}

#[track_caller]
fn exec<'a, C: GenericClient>(c: &'a C, sql: &'a str, p: &'a Args<'a>) -> impl Future<Output = Result<u64, Code>> + Send + 'a {
    let at = Location::caller();
    async move {
        let st = pg_at(c.prepare_cached(sql).await, at)?;
        pg_at(c.execute(&st, p).await, at)
    }
}

#[track_caller]
fn get<'r, T: tokio_postgres::types::FromSql<'r>>(r: &'r Row, i: usize) -> Result<T, Code> {
    pg_at(r.try_get(i), Location::caller())
}

fn msg_of(r: &Row) -> Result<ChatMessage, tokio_postgres::Error> {
    let kind: Option<String> = r.try_get(5)?;
    let keys: Option<String> = r.try_get(8)?;
    let ext: Option<String> = r.try_get(9)?;
    let talk: Option<String> = r.try_get(10)?;
    Ok(ChatMessage {
        msg_id: r.try_get(0)?,
        space_id: r.try_get(1)?,
        user_id: r.try_get(2)?,
        t: u(r.try_get(3)?),
        content: r.try_get(4)?,
        kind: kind.as_deref().and_then(ChatKind::parse),
        r#ref: r.try_get(7)?,
        keys: keys.and_then(|k| serde_json::from_str(&k).ok()),
        ext: ext.and_then(|e| serde_json::from_str(&e).ok()),
        target: r.try_get(6)?,
        talk: talk.and_then(|x| serde_json::from_str(&x).ok()),
    })
}

#[track_caller]
fn msgs(rows: Vec<Row>) -> Result<Vec<ChatMessage>, Code> {
    let at = Location::caller();
    rows.iter().map(|r| pg_at(msg_of(r), at)).collect()
}

async fn open_member(c: &impl GenericClient, space: &str, user: &str) -> Result<bool, Code> {
    let r = one(c, "SELECT EXISTS(SELECT 1 FROM members WHERE space_id = $1 AND user_id = $2 AND leave_t IS NULL)", &[&space, &user]).await?;
    r.map(|r| get(&r, 0)).transpose().map(|v| v.unwrap_or(false))
}

async fn space_exists(c: &impl GenericClient, space: &str) -> Result<bool, Code> {
    let r = one(c, "SELECT EXISTS(SELECT 1 FROM spaces WHERE space_id = $1)", &[&space]).await?;
    r.map(|r| get(&r, 0)).transpose().map(|v| v.unwrap_or(false))
}

async fn stored(c: &impl GenericClient, msg_id: &str) -> Result<Option<ChatMessage>, Code> {
    let sql = format!("SELECT {COLS} FROM messages x WHERE x.msg_id = $1");
    let r = one(c, &sql, &[&msg_id]).await?;
    r.map(|r| pg(msg_of(&r))).transpose()
}

async fn judged(c: &impl GenericClient, m: ChatMessage, space: &str, user: &str, out: &Outgoing) -> Result<Sent, Code> {
    let probe = probe_of(&m);
    let r = one(c, "SELECT deleted FROM messages WHERE msg_id = $1", &[&probe]).await?;
    let erased = r.map(|r| get::<bool>(&r, 0)).transpose()?.unwrap_or(false);
    again(m, erased, space, user, out)
}

async fn insert_msg(c: &impl GenericClient, m: &ChatMessage) -> Result<bool, Code> {
    let keys = m.keys.as_ref().map(|k| serde_json::to_string(k).unwrap_or_default());
    let ext = m.ext.as_ref().map(|e| e.to_string());
    let kind = m.kind.map(|k| k.as_str());
    let talk = m.talk.as_ref().map(|x| serde_json::to_string(x).unwrap_or_default());
    let t = n(m.t);
    let made = exec(
        c,
        "INSERT INTO messages (msg_id, space_id, user_id, t, content, kind, target, ref_id, keys, ext, talk) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) ON CONFLICT (msg_id) DO NOTHING",
        &[&m.msg_id, &m.space_id, &m.user_id, &t, &m.content, &kind, &m.target, &m.r#ref, &keys, &ext, &talk],
    )
    .await?;
    Ok(made == 1)
}

async fn record(c: &impl GenericClient, m: &ChatMessage, exclude: &str) -> Result<(), Code> {
    let t = n(m.t);
    exec(
        c,
        "INSERT INTO undelivered (user_id, msg_id, space_id, t)
         SELECT DISTINCT user_id, $2::text, $1::text, $3::bigint FROM members
         WHERE space_id = $1 AND join_t <= $3 AND (leave_t IS NULL OR $3 < leave_t) AND user_id <> $4
         ON CONFLICT DO NOTHING",
        &[&m.space_id, &m.msg_id, &t, &exclude],
    )
    .await?;
    Ok(())
}

async fn erase(c: &impl GenericClient, orig: &str) -> Result<(), Code> {
    exec(c, "UPDATE messages SET content = '', ext = NULL, deleted = TRUE WHERE msg_id = $1", &[&orig]).await?;
    exec(c, "UPDATE messages SET content = '', ext = NULL WHERE ref_id = $1 AND kind = 'edit'", &[&orig]).await?;
    exec(c, "UPDATE messages SET keys = '[]' WHERE ref_id = $1 AND kind = 'react'", &[&orig]).await?;
    Ok(())
}

async fn pick(c: &impl GenericClient, (seen, order): (&str, &str), cond: &str, (space, user): (&str, &str), edge: i64, lim: i64) -> Result<Vec<ChatMessage>, Code> {
    let sql = format!("SELECT {COLS} FROM messages x WHERE x.space_id = $1{seen}{VISIBLE}{cond} ORDER BY x.t {order}, x.msg_id {order} LIMIT $4");
    msgs(all(c, &sql, &[&space, &user, &edge, &lim]).await?)
}

fn fields(line: &str) -> Vec<String> {
    let mut v = vec![String::new()];
    let mut esc = false;
    for ch in line.chars() {
        let last = v.len() - 1;
        if esc {
            v[last].push(ch);
            esc = false;
        } else if ch == '\\' {
            esc = true;
        } else if ch == ':' && v.len() < 5 {
            v.push(String::new());
        } else {
            v[last].push(ch);
        }
    }
    v
}

fn password(cfg: &Config) -> Option<String> {
    if let Ok(p) = std::env::var("PGPASSWORD") {
        return Some(p);
    }
    let file = std::env::var_os("PGPASSFILE").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".pgpass")))?;
    let meta = std::fs::metadata(&file).ok()?;
    use std::os::unix::fs::PermissionsExt;
    if meta.permissions().mode() & 0o077 != 0 {
        tracing::warn!(kind = "diag", spec = "SV-IMPL-CHAT-STORE-FAULTS", file = %file.display(), "chat store: password file is open to group or others — not read");
        return None;
    }
    let text = std::fs::read_to_string(&file).ok()?;
    let host = match cfg.get_hosts().first() {
        Some(Host::Tcp(h)) => h.clone(),
        _ => "localhost".to_string(),
    };
    let port = cfg.get_ports().first().copied().unwrap_or(5432).to_string();
    let user = cfg.get_user().unwrap_or_default();
    let db = cfg.get_dbname().unwrap_or(user);
    let hit = |pat: &str, v: &str| pat == "*" || pat == v;
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(fields)
        .find(|f| f.len() == 5 && hit(&f[0], &host) && hit(&f[1], &port) && hit(&f[2], db) && hit(&f[3], user))
        .map(|f| f[4].clone())
}

pub(super) struct Pg {
    pool: Pool,
    queued: Arc<AtomicUsize>,
    rt: Option<tokio::runtime::Runtime>,
}

impl Drop for Pg {
    fn drop(&mut self) {
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

pub fn pg_config(url: &str) -> Result<Config, String> {
    let shown = ChatDb::shown_of(url);
    let mut cfg: Config = url.parse().map_err(|e| format!("chat db {shown}: {e}"))?;
    if cfg.get_password().is_none()
        && let Some(pw) = password(&cfg)
    {
        cfg.password(pw);
    }
    Ok(cfg)
}

impl Pg {
    pub(super) fn open(url: &str, size: usize, queued: Arc<AtomicUsize>) -> Result<Pg, String> {
        let shown = ChatDb::shown_of(url);
        let mut cfg = pg_config(url)?;
        let opts = match cfg.get_options() {
            Some(o) if !o.trim().is_empty() => format!("{o} -c lock_timeout={LOCK_TIMEOUT_MS}"),
            _ => format!("-c lock_timeout={LOCK_TIMEOUT_MS}"),
        };
        cfg.options(opts);
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("chat-pg")
            .enable_all()
            .build()
            .map_err(|e| format!("chat db runtime: {e}"))?;
        let mgr = Manager::from_config(cfg, NoTls, ManagerConfig { recycling_method: RecyclingMethod::Fast });
        let pool = Pool::builder(mgr).max_size(size.max(1)).build().map_err(|e| format!("chat db {shown}: {e}"))?;
        let me = Pg { pool, queued, rt: Some(rt) };
        let pool = me.pool.clone();
        let made = me.run(async move {
            let mut c = pool.get().await.map_err(|e| e.to_string())?;
            let tx = c.transaction().await.map_err(|e| said(&e))?;
            tx.execute("SELECT pg_advisory_xact_lock($1)", &[&SCHEMA_LOCK]).await.map_err(|e| said(&e))?;
            tx.batch_execute(SCHEMA).await.map_err(|e| said(&e))?;
            tx.commit().await.map_err(|e| said(&e))
        });
        match made {
            Some(Ok(())) => Ok(me),
            Some(Err(e)) => Err(format!("chat db {shown}: {e}")),
            None => Err(format!("chat db {shown}: runtime stopped")),
        }
    }

    fn run<T: Send + 'static>(&self, f: impl Future<Output = T> + Send + 'static) -> Option<T> {
        let rt = self.rt.as_ref()?;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let lines: Arc<Mutex<Vec<Line>>> = Arc::new(Mutex::new(Vec::new()));
        rt.spawn(LINES.scope(lines.clone(), async move {
            let _ = tx.send(f.await);
        }));
        let out = rx.recv().ok();
        let got = lines.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
        got.into_iter().for_each(diag);
        out
    }

    fn call<T: Send + 'static>(&self, f: impl Future<Output = Result<T, Code>> + Send + 'static) -> Result<T, Code> {
        self.run(f).unwrap_or(Err(Code::InternalError))
    }

    fn conn(&self) -> impl Future<Output = Result<Object, Code>> + Send + 'static {
        let (pool, queued) = (self.pool.clone(), self.queued.clone());
        async move {
            queued.fetch_add(1, Ordering::AcqRel);
            let o = pool.get().await;
            queued.fetch_sub(1, Ordering::AcqRel);
            o.map_err(|e| {
                note(("Pool".into(), format!("{}:{}", file!(), line!()), e.to_string()));
                Code::InternalError
            })
        }
    }

    pub(super) fn take(&self) -> Result<Object, Code> {
        self.call(self.conn())
    }

    pub(super) fn create_space(&self, space: &str, vis: Option<Visibility>) -> Result<(), Code> {
        let (conn, space, vis) = (self.conn(), space.to_string(), vis.map(|v| v.as_str().to_string()));
        self.call(async move {
            let c = conn.await?;
            exec(&c,"INSERT INTO spaces (space_id, visibility) VALUES ($1, $2) ON CONFLICT DO NOTHING", &[&space, &vis]).await?;
            Ok(())
        })
    }

    pub(super) fn join(&self, space: &str, user: &str, by: Option<&str>, at: Option<u64>, default_vis: Visibility) -> Result<Option<(ChatMessage, u64)>, Code> {
        let (conn, space, user, actor) = (self.conn(), space.to_string(), user.to_string(), by.unwrap_or(user).to_string());
        self.call(async move {
            let mut c = conn.await?;
            let tx = pg(c.transaction().await)?;
            let Some(row) = one(&tx, "SELECT ver, visibility FROM spaces WHERE space_id = $1 FOR UPDATE", &[&space]).await? else {
                return Err(Code::SpaceNotFound);
            };
            let ver: i64 = get(&row, 0)?;
            let own: Option<String> = get(&row, 1)?;
            if open_member(&tx, &space, &user).await? {
                return Ok(None);
            }
            let vis = own.unwrap_or_else(|| default_vis.as_str().to_string());
            let t = now_or(at);
            let tn = n(t);
            exec(&tx, "INSERT INTO members (space_id, user_id, join_t, vis) VALUES ($1, $2, $3, $4)", &[&space, &user, &tn, &vis]).await?;
            let msg = member_msg(&space, &actor, &user, t, true);
            insert_msg(&tx, &msg).await?;
            record(&tx, &msg, "").await?;
            exec(&tx, "UPDATE spaces SET ver = ver + 1 WHERE space_id = $1", &[&space]).await?;
            pg(tx.commit().await)?;
            Ok(Some((msg, u(ver) + 1)))
        })
    }

    pub(super) fn leave(&self, space: &str, user: &str, by: Option<&str>, at: Option<u64>) -> Result<(ChatMessage, u64), Code> {
        let (conn, space, user, actor) = (self.conn(), space.to_string(), user.to_string(), by.unwrap_or(user).to_string());
        self.call(async move {
            let mut c = conn.await?;
            let tx = pg(c.transaction().await)?;
            let Some(row) = one(&tx, "SELECT ver FROM spaces WHERE space_id = $1 FOR UPDATE", &[&space]).await? else {
                return Err(Code::SpaceNotFound);
            };
            let ver: i64 = get(&row, 0)?;
            if !open_member(&tx, &space, &user).await? {
                return Err(Code::NotSpaceMember);
            }
            let t = now_or(at);
            let tn = n(t);
            exec(&tx, "UPDATE members SET leave_t = $3 WHERE space_id = $1 AND user_id = $2 AND leave_t IS NULL", &[&space, &user, &tn]).await?;
            let msg = member_msg(&space, &actor, &user, t, false);
            insert_msg(&tx, &msg).await?;
            record(&tx, &msg, "").await?;
            exec(&tx, "UPDATE spaces SET ver = ver + 1 WHERE space_id = $1", &[&space]).await?;
            pg(tx.commit().await)?;
            Ok((msg, u(ver) + 1))
        })
    }

    pub(super) fn post(&self, user: &str, space: &str, msg_id: &str, out: &Outgoing, at: Option<u64>) -> Result<Sent, Code> {
        let (conn, user, space, msg_id, out) = (self.conn(), user.to_string(), space.to_string(), msg_id.to_string(), out.clone());
        self.call(async move {
            let mut c = conn.await?;
            let tx = pg(c.transaction().await)?;
            let Some(row) = one(&tx, "SELECT ver FROM spaces WHERE space_id = $1", &[&space]).await? else {
                return Err(Code::SpaceNotFound);
            };
            let ver: i64 = get(&row, 0)?;
            if let Some(m) = stored(&tx, &msg_id).await? {
                return judged(&tx, m, &space, &user, &out).await;
            }
            if !open_member(&tx, &space, &user).await? {
                return Err(Code::NotSpaceMember);
            }
            if let Some(r) = out.r#ref.as_deref() {
                let orig = one(&tx, "SELECT space_id, kind, user_id, deleted FROM messages WHERE msg_id = $1 FOR UPDATE", &[&r]).await?;
                let orig = match orig {
                    Some(x) => Some((get(&x, 0)?, get(&x, 1)?, get(&x, 2)?, get(&x, 3)?)),
                    None => None,
                };
                ref_ok(orig, &space, &user, &out)?;
            }
            let t = now_or(at);
            let tn = n(t);
            let msg = fresh(&space, &msg_id, &user, t, &out);
            if !insert_msg(&tx, &msg).await? {
                let Some(m) = stored(&tx, &msg_id).await? else { return Err(Code::InternalError) };
                return judged(&tx, m, &space, &user, &out).await;
            }
            if out.kind.is_none() {
                exec(&tx, "UPDATE spaces SET last_t = GREATEST(COALESCE(last_t, $2), $2), total = total + 1 WHERE space_id = $1", &[&space, &tn]).await?;
                exec(
                    &tx,
                    "INSERT INTO reads (space_id, user_id, read) SELECT $1::text, $2::text, total FROM spaces WHERE space_id = $1 ON CONFLICT (space_id, user_id) DO UPDATE SET read = excluded.read",
                    &[&space, &user],
                )
                .await?;
            } else {
                exec(&tx, "UPDATE spaces SET last_t = GREATEST(COALESCE(last_t, $2), $2) WHERE space_id = $1", &[&space, &tn]).await?;
            }
            if out.kind == Some(ChatKind::Delete)
                && let Some(r) = out.r#ref.as_deref()
            {
                erase(&tx, r).await?;
            }
            record(&tx, &msg, &user).await?;
            pg(tx.commit().await)?;
            Ok(Sent::Fresh { msg, ver: u(ver) })
        })
    }

    pub(super) fn talk(&self, r: &Report, at: Option<u64>) -> Result<Sent, Code> {
        let (conn, r) = (self.conn(), r.clone());
        self.call(async move {
            let mut c = conn.await?;
            let tx = pg(c.transaction().await)?;
            if let Some((talk_id, space, user, info)) = talk_started(&r) {
                let Some(row) = one(&tx, "SELECT ver FROM spaces WHERE space_id = $1", &[&space]).await? else {
                    return Err(Code::SpaceNotFound);
                };
                let ver: i64 = get(&row, 0)?;
                if let Some(m) = stored(&tx, talk_id).await? {
                    return Ok(Sent::Again { msg: m });
                }
                let t = now_or(at);
                let tn = n(t);
                let msg = talk_msg(space, talk_id, user, t, ChatKind::Talk, None, info);
                if !insert_msg(&tx, &msg).await? {
                    let Some(m) = stored(&tx, talk_id).await? else { return Err(Code::InternalError) };
                    return Ok(Sent::Again { msg: m });
                }
                exec(&tx, "UPDATE spaces SET last_t = GREATEST(COALESCE(last_t, $2), $2), total = total + 1 WHERE space_id = $1", &[&space, &tn]).await?;
                exec(
                    &tx,
                    "INSERT INTO reads (space_id, user_id, read) SELECT $1::text, $2::text, total FROM spaces WHERE space_id = $1 ON CONFLICT (space_id, user_id) DO UPDATE SET read = excluded.read",
                    &[&space, &user],
                )
                .await?;
                record(&tx, &msg, "").await?;
                pg(tx.commit().await)?;
                return Ok(Sent::Fresh { msg, ver: u(ver) });
            }
            let id = common::talk::update_msg_id(r.talk_id(), &r.tail());
            if let Some(m) = stored(&tx, &id).await? {
                return Ok(Sent::Again { msg: m });
            }
            let lock = format!("SELECT {COLS} FROM messages x WHERE x.msg_id = $1 FOR UPDATE");
            let talk_id = r.talk_id().to_string();
            let Some(row) = one(&tx, &lock, &[&talk_id]).await? else { return Err(talk_missing(&r)) };
            let orig = pg(msg_of(&row))?;
            talk_update_ok(&orig, &r)?;
            if let Some(m) = stored(&tx, &id).await? {
                return Ok(Sent::Again { msg: m });
            }
            let Some(row) = one(&tx, "SELECT ver FROM spaces WHERE space_id = $1", &[&orig.space_id]).await? else {
                return Err(Code::SpaceNotFound);
            };
            let ver: i64 = get(&row, 0)?;
            let sql = format!("SELECT {COLS} FROM messages x WHERE x.ref_id = $1 AND x.kind = 'talk_update' ORDER BY x.t DESC, x.msg_id DESC LIMIT 1");
            let last = match one(&tx, &sql, &[&orig.msg_id]).await? {
                Some(row) => Some(pg(msg_of(&row))?),
                None => None,
            };
            let Some(prev) = last.and_then(|m| m.talk).or(orig.talk.clone()) else { return Err(Code::InternalError) };
            let t = now_or(at);
            let tn = n(t);
            let msg = talk_msg(&orig.space_id, &id, &orig.user_id, t, ChatKind::TalkUpdate, Some(&orig.msg_id), talk_merged(prev, &r));
            if !insert_msg(&tx, &msg).await? {
                let Some(m) = stored(&tx, &id).await? else { return Err(Code::InternalError) };
                return Ok(Sent::Again { msg: m });
            }
            exec(&tx, "UPDATE spaces SET last_t = GREATEST(COALESCE(last_t, $2), $2) WHERE space_id = $1", &[&orig.space_id, &tn]).await?;
            record(&tx, &msg, "").await?;
            pg(tx.commit().await)?;
            Ok(Sent::Fresh { msg, ver: u(ver) })
        })
    }

    pub(super) fn rec_seen(&self, user: &str, space: &str, talk_id: &str) -> Result<(), Code> {
        let (conn, user, space, talk_id) = (self.conn(), user.to_string(), space.to_string(), talk_id.to_string());
        self.call(async move {
            let c = conn.await?;
            let Some(v) = one(&c, "SELECT vis FROM members WHERE space_id = $1 AND user_id = $2 AND leave_t IS NULL LIMIT 1", &[&space, &user]).await? else {
                return Err(Code::NotSpaceMember);
            };
            let vis: String = get(&v, 0)?;
            let seen = if vis == Visibility::Joined.as_str() { SEEN_JOINED } else { SEEN_ALL };
            let sql = format!(
                "SELECT EXISTS(SELECT 1 FROM messages x WHERE x.msg_id = $3 AND x.space_id = $1 AND x.kind = 'talk'{seen}), EXISTS(SELECT 1 FROM messages WHERE msg_id = $3 AND space_id = $1 AND kind = 'talk')"
            );
            let Some(row) = one(&c, &sql, &[&space, &user, &talk_id]).await? else { return Err(Code::InternalError) };
            match (get::<bool>(&row, 1)?, get::<bool>(&row, 0)?) {
                (false, _) => Err(Code::RecNotFound),
                (true, false) => Err(Code::NotSpaceMember),
                (true, true) => Ok(()),
            }
        })
    }

    pub(super) fn roster(&self, space: &str) -> Result<Roster, Code> {
        let (conn, space) = (self.conn(), space.to_string());
        self.call(async move {
            let c = conn.await?;
            let rows = all(&c,"SELECT s.ver, m.user_id, m.join_t, m.leave_t FROM spaces s LEFT JOIN members m ON m.space_id = s.space_id WHERE s.space_id = $1", &[&space]).await?;
            let Some(first) = rows.first() else { return Err(Code::SpaceNotFound) };
            let ver: i64 = get(first, 0)?;
            let mut intervals = Vec::new();
            for r in &rows {
                let Some(user) = get::<Option<String>>(r, 1)? else { continue };
                intervals.push(Interval { user, join_t: u(get(r, 2)?), leave_t: get::<Option<i64>>(r, 3)?.map(u) });
            }
            Ok(Roster { ver: u(ver), intervals })
        })
    }

    pub(super) fn history_within(&self, user: &str, space: &str, dir: Dir, limit: u32, max_bytes: usize) -> Result<Page, Code> {
        let (conn, user, space) = (self.conn(), user.to_string(), space.to_string());
        self.call(async move {
            let c = conn.await?;
            let c = &c;
            if !space_exists(c, &space).await? {
                return Err(Code::SpaceNotFound);
            }
            let Some(v) = one(c, "SELECT vis FROM members WHERE space_id = $1 AND user_id = $2 AND leave_t IS NULL LIMIT 1", &[&space, &user]).await? else {
                return Err(Code::NotSpaceMember);
            };
            let vis: String = get(&v, 0)?;
            let joined = vis == Visibility::Joined.as_str();
            let seen = if joined { SEEN_JOINED } else { SEEN_ALL };
            let (cond, order, edge) = match dir {
                Dir::After(t) => (" AND x.t > $3", "ASC", n(t)),
                Dir::Before(t) => (" AND x.t < $3", "DESC", n(t)),
                Dir::Latest => (" AND $3::bigint = $3::bigint", "DESC", 0),
            };
            let mut messages = pick(c, (seen, order), cond, (&space, &user), edge, i64::from(limit) + 1).await?;
            let lim = limit as usize;
            let mut more = messages.len() > lim;
            if more && messages[lim].t == messages[lim - 1].t {
                let cut = messages[lim - 1].t;
                messages.retain(|m| m.t != cut);
                messages.extend(pick(c, (seen, order), " AND x.t = $3", (&space, &user), n(cut), i64::MAX).await?);
                let beyond = if order == "ASC" { " AND x.t > $3" } else { " AND x.t < $3" };
                more = !pick(c, (seen, order), beyond, (&space, &user), n(cut), 1).await?.is_empty();
            } else {
                messages.truncate(lim);
            }
            if order == "DESC" {
                messages.reverse();
            }
            let ids: Vec<String> = messages.iter().filter(|m| m.kind.is_none() || m.kind == Some(ChatKind::Talk)).map(|m| m.msg_id.clone()).collect();
            let relations = if ids.is_empty() {
                Vec::new()
            } else {
                let sql = format!("SELECT {COLS} FROM messages x WHERE x.space_id = $1{seen} AND x.kind IN {RELATION_KINDS} AND x.ref_id = ANY($3)");
                fold_relations(msgs(all(c, &sql, &[&space, &user, &ids]).await?)?)
            };
            let (messages, relations, cut) = within(messages, relations, order == "ASC", max_bytes);
            Ok(Page { messages, relations, more: more || cut })
        })
    }

    pub(super) fn list(&self, user: &str) -> Result<Vec<SpaceEntry>, Code> {
        let (conn, user) = (self.conn(), user.to_string());
        self.call(async move {
            let c = conn.await?;
            let rows = all(
                &c,
                "SELECT s.space_id, s.total, s.last_t, COALESCE(r.read, 0) FROM spaces s
                 LEFT JOIN reads r ON r.space_id = s.space_id AND r.user_id = $1
                 WHERE EXISTS(SELECT 1 FROM members m WHERE m.space_id = s.space_id AND m.user_id = $1 AND m.leave_t IS NULL)
                 ORDER BY s.space_id",
                &[&user],
            )
            .await?;
            rows.iter()
                .map(|r| Ok(SpaceEntry { space_id: get(r, 0)?, total: u(get(r, 1)?), last_t: get::<Option<i64>>(r, 2)?.map(u), read: u(get(r, 3)?) }))
                .collect()
        })
    }

    pub(super) fn view(&self, user: &str, space: &str) -> Result<u64, Code> {
        let (conn, user, space) = (self.conn(), user.to_string(), space.to_string());
        self.call(async move {
            let mut c = conn.await?;
            let tx = pg(c.transaction().await)?;
            let Some(row) = one(&tx, "SELECT total FROM spaces WHERE space_id = $1", &[&space]).await? else {
                return Err(Code::SpaceNotFound);
            };
            let total: i64 = get(&row, 0)?;
            if !open_member(&tx, &space, &user).await? {
                return Err(Code::NotSpaceMember);
            }
            exec(
                &tx,
                "INSERT INTO reads (space_id, user_id, read) VALUES ($1, $2, $3) ON CONFLICT (space_id, user_id) DO UPDATE SET read = excluded.read",
                &[&space, &user, &total],
            )
            .await?;
            pg(tx.commit().await)?;
            Ok(u(total))
        })
    }

    pub(super) fn catchup_check(&self, user: &str, space: &str) -> Result<(), Code> {
        let (conn, user, space) = (self.conn(), user.to_string(), space.to_string());
        self.call(async move {
            let c = conn.await?;
            if !space_exists(&c,&space).await? {
                return Err(Code::SpaceNotFound);
            }
            if !open_member(&c,&space, &user).await? {
                return Err(Code::NotSpaceMember);
            }
            Ok(())
        })
    }

    pub(super) fn catchup(&self, user: &str, space: Option<&str>, since: Option<u64>, after: &Cursor, lim: i64) -> Result<Vec<ChatMessage>, Code> {
        let (conn, user, space, after) = (self.conn(), user.to_string(), space.map(str::to_string), after.clone());
        self.call(async move {
            let c = conn.await?;
            let c = &c;
            let sp = space.clone().unwrap_or_default();
            let d_space = if space.is_some() { "d.space_id = $2" } else { "$2::text = $2::text" };
            let sql = format!(
                "SELECT {COLS} FROM undelivered d JOIN messages x ON x.msg_id = d.msg_id
                 WHERE d.user_id = $1 AND {d_space} AND (d.t, d.msg_id) > ($3, $4)
                 ORDER BY d.t, d.msg_id LIMIT $5"
            );
            let at = n(after.t);
            let mut got = msgs(all(c, &sql, &[&user, &sp, &at, &after.msg_id, &lim]).await?)?;
            if let Some(since) = since {
                let (ft, fid) = since_floor(since, &after);
                let ft = n(ft);
                let in_space = if space.is_some() { "mm.space_id = $2" } else { "$2::text = $2::text" };
                let sql = format!(
                    "SELECT {COLS} FROM members mm JOIN messages x ON x.space_id = mm.space_id
                     WHERE mm.user_id = $1 AND mm.leave_t IS NULL AND {in_space} AND (x.t, x.msg_id) > ($3, $4)
                       AND (mm.vis = 'shared' OR EXISTS(SELECT 1 FROM members m2 WHERE m2.space_id = x.space_id AND m2.user_id = $1
                         AND m2.join_t <= x.t AND (m2.leave_t IS NULL OR x.t < m2.leave_t)))
                     ORDER BY x.t, x.msg_id LIMIT $5"
                );
                got.extend(msgs(all(c, &sql, &[&user, &sp, &ft, &fid, &lim]).await?)?);
            }
            Ok(got)
        })
    }

    pub(super) fn ack_batch(&self, conn: Option<Object>, batch: Vec<Ack>) -> Result<(), Code> {
        let fresh_conn = self.conn();
        self.call(async move {
            let mut c = match conn {
                Some(c) => c,
                None => fresh_conn.await?,
            };
            let tx = pg(c.transaction().await)?;
            for (user, ids) in &batch {
                exec(&tx, "DELETE FROM undelivered WHERE user_id = $1 AND msg_id = ANY($2)", &[user, ids]).await?;
            }
            pg(tx.commit().await)
        })
    }

    pub(super) fn purge(&self, before_t: u64) -> Result<usize, Code> {
        let conn = self.conn();
        self.call(async move {
            let c = conn.await?;
            let b = n(before_t);
            let k = exec(&c,"DELETE FROM undelivered WHERE t < $1", &[&b]).await?;
            Ok(usize::try_from(k).unwrap_or(usize::MAX))
        })
    }

    pub(super) fn pending(&self, user: &str) -> Result<Vec<String>, Code> {
        let (conn, user) = (self.conn(), user.to_string());
        self.call(async move {
            let c = conn.await?;
            let rows = all(&c,"SELECT msg_id FROM undelivered WHERE user_id = $1 ORDER BY t, msg_id", &[&user]).await?;
            rows.iter().map(|r| get(r, 0)).collect()
        })
    }

    pub(super) fn plan(&self, sql: &str) -> Result<String, Code> {
        let (conn, sql) = (self.conn(), format!("EXPLAIN (GENERIC_PLAN) {sql}"));
        self.call(async move {
            let c = conn.await?;
            let rows = pg(c.query(&sql, &[]).await)?;
            let lines: Result<Vec<String>, Code> = rows.iter().map(|r| get(r, 0)).collect();
            Ok(lines?.join("\n"))
        })
    }
}
