// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:RULE-CHAT-REF-CHECK · 연:RULE-CHAT-EDIT · 연:RULE-CHAT-REACT · 연:RULE-CHAT-DELETE · 연:RULE-CHAT-REPLY · 정:IMPL-CHAT-STORE ·정:IMPL-CHAT-MEMBERSHIP · 정:IMPL-CHAT-READ · 정:IMPL-CHAT-UNDELIVERED · 정:IMPL-CHAT-CATCHUP · 정:IMPL-CHAT-STORE-FAULTS · 정:IMPL-CHAT-STORE-FORMS · 연:RULE-CHAT-MSG-ID · 연:RULE-CHAT-ORDER · 연:RULE-CHAT-VISIBLE · 연:RULE-CHAT-UNREAD · 연:RULE-CHAT-VIEW · 연:RULE-CHAT-HISTORY-PAGE · 연:RULE-CHAT-UNDELIVERED · 연:RULE-CHAT-CATCHUP · 연:RULE-CHAT-MEMBER-NOTICE · model: claude-opus-5-5
//! 채팅 저장소 — `[chat] db` 꼴 셋 중 하나를 직접 쥔다: `memory:` · `sqlite:<경로>` 는 SQLite 연결 하나(메모리 · 파일 — 저장소 전체 잠금 하나 · 쓰기 트랜잭션은 `BEGIN IMMEDIATE`) · `postgres://…` 는 연결 풀(`pg`) · 꼴마다 같은 계약 · `t` 는 저장 트랜잭션 안에서 찍은 ms(공간마다 줄 세우지 않음 · 같은 `t` 는 `msg_id` 차례) · 메시지와 같은 트랜잭션에 받을 사람 전원 몫 미전송 기록 · 멤버 입퇴장 구간과 멤버 변경 메시지 · 멤버 판 · 읽음 · 쪽은 같은 `t` 를 가르지 않음 · 따라잡기 쪽은 미전송 기록과 `t` 이상 메시지를 `msg_id` 로 합침(멤버 쪽에서 출발 · 하한 하나) · 관계는 `ref` 색인 · 저장소 오류는 원인을 진단 줄로 남김.

mod pg;

pub use pg::pg_config;

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::system::{ChatDb, DB_POOL_DEFAULT, Visibility};
use oxsig::Code;
use oxsig::msg::{ChatKind, ChatMessage, SpaceEntry};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params, params_from_iter};

pub const HISTORY_DEFAULT: u32 = 60;
pub const HISTORY_MAX: u32 = 200;
pub const MSG_ID_MAX: usize = 64;
pub const CONTENT_MAX: usize = 65_535;
pub const SYS: &str = "$sys";
pub const SYS_HASH: &str = "81330e4c";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS spaces (
  space_id TEXT PRIMARY KEY,
  last_t INTEGER,
  total INTEGER NOT NULL DEFAULT 0,
  ver INTEGER NOT NULL DEFAULT 0,
  visibility TEXT
);
CREATE TABLE IF NOT EXISTS members (
  space_id TEXT NOT NULL,
  user_id TEXT NOT NULL,
  join_t INTEGER NOT NULL,
  leave_t INTEGER,
  vis TEXT NOT NULL,
  PRIMARY KEY (space_id, user_id, join_t)
);
CREATE TABLE IF NOT EXISTS messages (
  msg_id TEXT PRIMARY KEY,
  space_id TEXT NOT NULL,
  user_id TEXT NOT NULL,
  t INTEGER NOT NULL,
  content TEXT NOT NULL,
  kind TEXT,
  target TEXT,
  ref_id TEXT,
  keys TEXT,
  ext TEXT,
  deleted INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS messages_space_t_id ON messages (space_id, t, msg_id);
CREATE TABLE IF NOT EXISTS reads (
  space_id TEXT NOT NULL,
  user_id TEXT NOT NULL,
  read INTEGER NOT NULL,
  PRIMARY KEY (space_id, user_id)
);
CREATE TABLE IF NOT EXISTS undelivered (
  user_id TEXT NOT NULL,
  msg_id TEXT NOT NULL,
  space_id TEXT NOT NULL,
  t INTEGER NOT NULL,
  PRIMARY KEY (user_id, msg_id)
);
CREATE INDEX IF NOT EXISTS undelivered_user_t ON undelivered (user_id, t, msg_id);
CREATE INDEX IF NOT EXISTS undelivered_t ON undelivered (t);
CREATE INDEX IF NOT EXISTS members_user ON members (user_id, leave_t);
";

pub(crate) const COLS: &str ="x.msg_id, x.space_id, x.user_id, x.t, x.content, x.kind, x.target, x.ref_id, x.keys, x.ext";
pub(crate) const VISIBLE: &str =" AND (x.kind IS NULL OR x.kind IN ('join', 'leave', 'add', 'remove'))";
const SEEN_JOINED: &str =
    " AND EXISTS(SELECT 1 FROM members m WHERE m.space_id = x.space_id AND m.user_id = ?2 AND m.join_t <= x.t AND (m.leave_t IS NULL OR x.t < m.leave_t))";
const SEEN_ALL: &str = " AND ?2 = ?2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interval {
    pub user: String,
    pub join_t: u64,
    pub leave_t: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Roster {
    pub ver: u64,
    pub intervals: Vec<Interval>,
}

impl Roster {
    pub fn at(&self, t: u64) -> Vec<String> {
        let mut v: Vec<String> = self
            .intervals
            .iter()
            .filter(|i| i.join_t <= t && i.leave_t.is_none_or(|l| t < l))
            .map(|i| i.user.clone())
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    Fresh { msg: ChatMessage, ver: u64 },
    Again { msg: ChatMessage },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub messages: Vec<ChatMessage>,
    pub relations: Vec<ChatMessage>,
    pub more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    After(u64),
    Before(u64),
    Latest,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cursor {
    pub t: u64,
    pub msg_id: String,
}

pub struct Store {
    form: Form,
    default_vis: Visibility,
    ack_tx: AtomicU64,
    queued: Arc<AtomicUsize>,
}

enum Form {
    Lite(Mutex<Connection>),
    Pg(pg::Pg),
}

pub type Ack = (String, Vec<String>);

pub(crate) const YIELD: Duration =Duration::from_micros(200);

fn cause(e: &rusqlite::Error) -> String {
    match e.sqlite_error_code() {
        Some(c) => format!("{c:?}"),
        None => "Other".to_string(),
    }
}

#[track_caller]
fn db<T>(r: rusqlite::Result<T>) -> Result<T, Code> {
    let at = std::panic::Location::caller();
    r.map_err(|e| {
        tracing::warn!(kind = "diag", spec = "SV-IMPL-CHAT-STORE-FAULTS", cause = %cause(&e), at = %format!("{}:{}", at.file(), at.line()), "chat store: {e}");
        Code::InternalError
    })
}

fn delete_acks(tx: &Transaction, batch: &[Ack]) -> Result<(), Code> {
    let mut st = db(tx.prepare_cached("DELETE FROM undelivered WHERE user_id = ?1 AND msg_id = ?2"))?;
    for (user, ids) in batch {
        for id in ids {
            db(st.execute(params![user, id]))?;
        }
    }
    Ok(())
}

fn duplicate_column(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(_, Some(m)) if m.starts_with("duplicate column name"))
}

pub fn relations_sql(joined: bool, refs: usize) -> String {
    let seen = if joined { SEEN_JOINED } else { SEEN_ALL };
    let marks = (3..refs + 3).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
    format!("SELECT {COLS} FROM messages x WHERE +x.space_id = ?1{seen} AND x.kind IN ('edit', 'react', 'delete') AND x.ref_id IN ({marks})")
}

pub fn catchup_since_sql(space: bool) -> String {
    let in_space = if space { "mm.space_id = ?2" } else { "?2 = ?2" };
    format!(
        "SELECT {COLS} FROM members mm JOIN messages x ON x.space_id = mm.space_id
         WHERE mm.user_id = ?1 AND mm.leave_t IS NULL AND {in_space} AND (x.t, x.msg_id) > (?3, ?4)
           AND (mm.vis = 'shared' OR EXISTS(SELECT 1 FROM members m2 WHERE m2.space_id = x.space_id AND m2.user_id = ?1
             AND m2.join_t <= x.t AND (m2.leave_t IS NULL OR x.t < m2.leave_t)))
         ORDER BY x.t, x.msg_id LIMIT ?5"
    )
}

pub(crate) fn since_floor(since: u64, after: &Cursor) -> (u64, String) {
    let floor = (since, String::new());
    let cur = (after.t, after.msg_id.clone());
    if cur > floor { cur } else { floor }
}

pub(crate) fn n(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

pub(crate) fn u(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn space_ver(tx: &Transaction, space: &str) -> Result<Option<u64>, Code> {
    db(tx.query_row("SELECT ver FROM spaces WHERE space_id = ?1", params![space], |r| Ok(u(r.get(0)?))).optional())
}

fn open_member(c: &Connection, space: &str, user: &str) -> Result<bool, Code> {
    db(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM members WHERE space_id = ?1 AND user_id = ?2 AND leave_t IS NULL)",
        params![space, user],
        |r| r.get::<_, bool>(0),
    ))
}

fn msg_of(r: &rusqlite::Row) -> rusqlite::Result<ChatMessage> {
    let kind: Option<String> = r.get(5)?;
    let keys: Option<String> = r.get(8)?;
    let ext: Option<String> = r.get(9)?;
    Ok(ChatMessage {
        msg_id: r.get(0)?,
        space_id: r.get(1)?,
        user_id: r.get(2)?,
        t: u(r.get(3)?),
        content: r.get(4)?,
        kind: kind.as_deref().and_then(ChatKind::parse),
        r#ref: r.get(7)?,
        keys: keys.and_then(|k| serde_json::from_str(&k).ok()),
        ext: ext.and_then(|e| serde_json::from_str(&e).ok()),
        target: r.get(6)?,
    })
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outgoing {
    pub kind: Option<ChatKind>,
    pub r#ref: Option<String>,
    pub content: String,
    pub keys: Option<Vec<String>>,
    pub ext: Option<serde_json::Value>,
}

impl Outgoing {
    pub fn text(content: &str) -> Outgoing {
        Outgoing { content: content.into(), ..Outgoing::default() }
    }
}

pub(crate) fn ref_ok(orig: Option<(String, Option<String>, String, bool)>, space: &str, user: &str, out: &Outgoing) -> Result<(), Code> {
    let Some((sp, kind, owner, deleted)) = orig else { return Err(Code::RefNotFound) };
    if sp != space || kind.is_some() {
        return Err(Code::RefNotFound);
    }
    let Some(k) = out.kind else { return Ok(()) };
    if matches!(k, ChatKind::Edit | ChatKind::Delete) && owner != user {
        return Err(Code::NotRefSender);
    }
    if deleted {
        return Err(Code::RefDeleted);
    }
    Ok(())
}

pub(crate) fn probe_of(m: &ChatMessage) -> String {
    match m.kind {
        Some(k) if k.is_relation() => m.r#ref.clone().unwrap_or_default(),
        _ => m.msg_id.clone(),
    }
}

pub(crate) fn again(m: ChatMessage, erased: bool, space: &str, user: &str, out: &Outgoing) -> Result<Sent, Code> {
    let envelope = m.space_id == space && m.user_id == user && m.kind == out.kind && m.r#ref == out.r#ref;
    let body = m.content == out.content && m.keys == out.keys && m.ext == out.ext;
    if envelope && (erased || body) {
        return Ok(Sent::Again { msg: m });
    }
    Err(Code::MsgIdConflict)
}

pub(crate) fn fresh(space: &str, msg_id: &str, user: &str, t: u64, out: &Outgoing) -> ChatMessage {
    ChatMessage {
        space_id: space.into(),
        msg_id: msg_id.into(),
        user_id: user.into(),
        t,
        content: out.content.clone(),
        kind: out.kind,
        r#ref: out.r#ref.clone(),
        keys: out.keys.clone(),
        ext: out.ext.clone(),
        target: None,
    }
}

pub(crate) fn member_msg(space: &str, actor: &str, user: &str, t: u64, joining: bool) -> ChatMessage {
    let (kind, target) = match (actor == user, joining) {
        (true, true) => (ChatKind::Join, None),
        (false, true) => (ChatKind::Add, Some(user.to_string())),
        (true, false) => (ChatKind::Leave, None),
        (false, false) => (ChatKind::Remove, Some(user.to_string())),
    };
    ChatMessage {
        space_id: space.into(),
        msg_id: sys_msg_id(t),
        user_id: actor.into(),
        t,
        content: String::new(),
        kind: Some(kind),
        r#ref: None,
        keys: None,
        ext: None,
        target,
    }
}

pub(crate) fn bundle(all: Vec<ChatMessage>, max_msgs: usize, max_bytes: usize) -> Vec<ChatMessage> {
    let mut all = all;
    all.sort_by(|a, b| (a.t, &a.msg_id).cmp(&(b.t, &b.msg_id)));
    all.dedup_by(|a, b| a.msg_id == b.msg_id);
    let mut out = Vec::new();
    let mut bytes = r#"{"messages":[]}"#.len();
    for m in all {
        let size = serde_json::to_vec(&m).map(|b| b.len()).unwrap_or(0) + usize::from(!out.is_empty());
        if !out.is_empty() && (out.len() >= max_msgs || bytes + size > max_bytes) {
            break;
        }
        bytes += size;
        out.push(m);
    }
    out
}

fn check_ref(tx: &Transaction, space: &str, user: &str, out: &Outgoing) -> Result<(), Code> {
    let Some(r) = out.r#ref.as_deref() else { return Ok(()) };
    let orig = db(tx
        .query_row("SELECT space_id, kind, user_id, deleted FROM messages WHERE msg_id = ?1", params![r], |x| {
            Ok((x.get::<_, String>(0)?, x.get::<_, Option<String>>(1)?, x.get::<_, String>(2)?, x.get::<_, bool>(3)?))
        })
        .optional())?;
    ref_ok(orig, space, user, out)
}

fn erase(tx: &Transaction, orig: &str) -> Result<(), Code> {
    db(tx.execute("UPDATE messages SET content = '', ext = NULL, deleted = 1 WHERE msg_id = ?1", params![orig]))?;
    db(tx.execute("UPDATE messages SET content = '', ext = NULL WHERE ref_id = ?1 AND kind = 'edit'", params![orig]))?;
    db(tx.execute("UPDATE messages SET keys = '[]' WHERE ref_id = ?1 AND kind = 'react'", params![orig]))?;
    Ok(())
}

pub(crate) fn within(messages: Vec<ChatMessage>, relations: Vec<ChatMessage>, from_start: bool, max_bytes: usize) -> (Vec<ChatMessage>, Vec<ChatMessage>, bool) {
    let size = |m: &ChatMessage| serde_json::to_vec(m).map(|v| v.len()).unwrap_or(0);
    let mut rel_bytes: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for r in &relations {
        let s = size(r);
        rel_bytes.entry(r.r#ref.as_deref().unwrap_or_default()).and_modify(|v| *v += s).or_insert(s);
    }
    let cost = |m: &ChatMessage| size(m) + rel_bytes.get(m.msg_id.as_str()).copied().unwrap_or(0);
    let order: Vec<usize> = if from_start { (0..messages.len()).collect() } else { (0..messages.len()).rev().collect() };
    let mut keep = vec![false; messages.len()];
    let (mut used, mut i) = (0usize, 0usize);
    while i < order.len() {
        let t = messages[order[i]].t;
        let mut j = i;
        let mut group = 0usize;
        while j < order.len() && messages[order[j]].t == t {
            group += cost(&messages[order[j]]);
            j += 1;
        }
        if used > 0 && used + group > max_bytes {
            break;
        }
        used += group;
        for k in &order[i..j] {
            keep[*k] = true;
        }
        i = j;
    }
    let cut = keep.iter().any(|k| !k);
    let kept: Vec<ChatMessage> = messages.into_iter().zip(&keep).filter(|(_, k)| **k).map(|(m, _)| m).collect();
    let ids: std::collections::HashSet<&str> = kept.iter().map(|m| m.msg_id.as_str()).collect();
    let rel: Vec<ChatMessage> = relations.into_iter().filter(|r| r.r#ref.as_deref().is_some_and(|x| ids.contains(x))).collect();
    (kept, rel, cut)
}

pub(crate) fn fold_relations(rel: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut best: std::collections::BTreeMap<(String, u8, String), ChatMessage> = std::collections::BTreeMap::new();
    for m in rel {
        let Some(k) = m.kind else { continue };
        let Some(r) = m.r#ref.clone() else { continue };
        let key = match k {
            ChatKind::Delete => (r, 0, String::new()),
            ChatKind::Edit => (r, 1, String::new()),
            ChatKind::React => (r, 2, m.user_id.clone()),
            _ => continue,
        };
        let newer = best.get(&key).is_none_or(|b| (m.t, &m.msg_id) > (b.t, &b.msg_id));
        if newer {
            best.insert(key, m);
        }
    }
    let mut v: Vec<ChatMessage> = best.into_values().collect();
    v.sort_by(|a, b| (a.t, &a.msg_id).cmp(&(b.t, &b.msg_id)));
    v
}

fn rows(c: &Connection, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<ChatMessage>, Code> {
    let mut st = db(c.prepare(sql))?;
    let it = db(st.query_map(p, msg_of))?;
    let mut v = Vec::new();
    for m in it {
        v.push(db(m)?);
    }
    Ok(v)
}

pub(crate) fn now_or(at: Option<u64>) -> u64 {
    at.unwrap_or_else(common::now_ms)
}

pub fn sys_msg_id(t: u64) -> String {
    let r = uuid::Uuid::new_v4();
    let b = r.as_bytes();
    let rand: String = b[..8].iter().map(|x| format!("{x:02x}")).collect();
    format!("{SYS_HASH}{:012x}{rand}", t & 0xffff_ffff_ffff)
}

fn insert_msg(tx: &Transaction, m: &ChatMessage) -> Result<(), Code> {
    let keys = m.keys.as_ref().map(|k| serde_json::to_string(k).unwrap_or_default());
    let ext = m.ext.as_ref().map(|e| e.to_string());
    db(tx.execute(
        "INSERT INTO messages (msg_id, space_id, user_id, t, content, kind, target, ref_id, keys, ext) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![m.msg_id, m.space_id, m.user_id, n(m.t), m.content, m.kind.map(|k| k.as_str()), m.target, m.r#ref, keys, ext],
    ))?;
    Ok(())
}

fn record(tx: &Transaction, m: &ChatMessage, exclude: &str) -> Result<(), Code> {
    db(tx.execute(
        "INSERT OR IGNORE INTO undelivered (user_id, msg_id, space_id, t)
         SELECT DISTINCT user_id, ?2, ?1, ?3 FROM members
         WHERE space_id = ?1 AND join_t <= ?3 AND (leave_t IS NULL OR ?3 < leave_t) AND user_id <> ?4",
        params![m.space_id, m.msg_id, n(m.t), exclude],
    ))?;
    Ok(())
}

impl Store {
    pub fn open(db: &str, default_vis: Visibility) -> Result<Store, String> {
        Store::open_pool(db, DB_POOL_DEFAULT, default_vis)
    }

    pub fn open_pool(db: &str, pool: usize, default_vis: Visibility) -> Result<Store, String> {
        let queued = Arc::new(AtomicUsize::new(0));
        let form = match ChatDb::parse(db)? {
            ChatDb::Memory => Form::Lite(Mutex::new(Store::lite(None)?)),
            ChatDb::Sqlite(path) => Form::Lite(Mutex::new(Store::lite(Some(&path))?)),
            ChatDb::Postgres(url) => Form::Pg(pg::Pg::open(&url, pool, queued.clone())?),
        };
        Ok(Store { form, default_vis, ack_tx: AtomicU64::new(0), queued })
    }

    pub fn form(&self) -> &'static str {
        match &self.form {
            Form::Lite(_) => "sqlite",
            Form::Pg(_) => "postgres",
        }
    }

    fn lite(file: Option<&str>) -> Result<Connection, String> {
        let path = file.unwrap_or(":memory:");
        let c = match file {
            Some(p) => Connection::open(p),
            None => Connection::open_in_memory(),
        }
        .map_err(|e| format!("chat db {path}: {e}"))?;
        c.busy_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
        if file.is_some() {
            match c.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0)) {
                Ok(m) if m.eq_ignore_ascii_case("wal") => {}
                Ok(m) => tracing::warn!(kind = "diag", spec = "SV-IMPL-CHAT-STORE-FAULTS", path = %path, "chat store: journal mode stays {m} — WAL not set"),
                Err(e) => tracing::warn!(kind = "diag", spec = "SV-IMPL-CHAT-STORE-FAULTS", path = %path, cause = %cause(&e), "chat store: WAL not set: {e}"),
            }
        }
        c.execute_batch(SCHEMA).map_err(|e| format!("chat db schema: {e}"))?;
        for col in ["kind TEXT", "target TEXT", "ref_id TEXT", "keys TEXT", "ext TEXT", "deleted INTEGER NOT NULL DEFAULT 0"] {
            if let Err(e) = c.execute(&format!("ALTER TABLE messages ADD COLUMN {col}"), [])
                && !duplicate_column(&e)
            {
                tracing::warn!(kind = "diag", spec = "SV-IMPL-CHAT-STORE-FAULTS", path = %path, column = %col, cause = %cause(&e), "chat store: add column failed: {e}");
            }
        }
        c.execute_batch("CREATE INDEX IF NOT EXISTS messages_ref ON messages (ref_id); CREATE INDEX IF NOT EXISTS messages_t_id ON messages (t, msg_id);")
            .map_err(|e| format!("chat db schema: {e}"))?;
        Ok(c)
    }

    fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T, Code>) -> Result<T, Code> {
        let Form::Lite(m) = &self.form else { return Err(Code::InternalError) };
        self.queued.fetch_add(1, Ordering::AcqRel);
        let g = m.lock();
        self.queued.fetch_sub(1, Ordering::AcqRel);
        let mut g = g.map_err(|_| Code::InternalError)?;
        f(&mut g)
    }

    pub fn create_space(&self, space: &str, vis: Option<Visibility>) -> Result<(), Code> {
        if let Form::Pg(p) = &self.form {
            return p.create_space(space, vis);
        }
        self.with(|c| {
            db(c.execute(
                "INSERT OR IGNORE INTO spaces (space_id, visibility) VALUES (?1, ?2)",
                params![space, vis.map(|v| v.as_str())],
            ))?;
            Ok(())
        })
    }

    pub fn join(&self, space: &str, user: &str, by: Option<&str>, at: Option<u64>) -> Result<Option<(ChatMessage, u64)>, Code> {
        let default_vis = self.default_vis;
        if let Form::Pg(p) = &self.form {
            return p.join(space, user, by, at, default_vis);
        }
        self.with(|c| {
            let tx = db(c.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let ver = space_ver(&tx, space)?.ok_or(Code::SpaceNotFound)?;
            if open_member(&tx, space, user)? {
                return Ok(None);
            }
            let own = db(tx.query_row("SELECT visibility FROM spaces WHERE space_id = ?1", params![space], |r| r.get::<_, Option<String>>(0)))?;
            let vis = own.unwrap_or_else(|| default_vis.as_str().to_string());
            let t = now_or(at);
            db(tx.execute(
                "INSERT INTO members (space_id, user_id, join_t, vis) VALUES (?1, ?2, ?3, ?4)",
                params![space, user, n(t), vis],
            ))?;
            let msg = member_msg(space, by.unwrap_or(user), user, t, true);
            insert_msg(&tx, &msg)?;
            record(&tx, &msg, "")?;
            db(tx.execute("UPDATE spaces SET ver = ver + 1 WHERE space_id = ?1", params![space]))?;
            db(tx.commit())?;
            Ok(Some((msg, ver + 1)))
        })
    }

    pub fn leave(&self, space: &str, user: &str, by: Option<&str>, at: Option<u64>) -> Result<(ChatMessage, u64), Code> {
        if let Form::Pg(p) = &self.form {
            return p.leave(space, user, by, at);
        }
        self.with(|c| {
            let tx = db(c.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let ver = space_ver(&tx, space)?.ok_or(Code::SpaceNotFound)?;
            if !open_member(&tx, space, user)? {
                return Err(Code::NotSpaceMember);
            }
            let t = now_or(at);
            db(tx.execute(
                "UPDATE members SET leave_t = ?3 WHERE space_id = ?1 AND user_id = ?2 AND leave_t IS NULL",
                params![space, user, n(t)],
            ))?;
            let msg = member_msg(space, by.unwrap_or(user), user, t, false);
            insert_msg(&tx, &msg)?;
            record(&tx, &msg, "")?;
            db(tx.execute("UPDATE spaces SET ver = ver + 1 WHERE space_id = ?1", params![space]))?;
            db(tx.commit())?;
            Ok((msg, ver + 1))
        })
    }

    pub fn send(&self, user: &str, space: &str, msg_id: &str, content: &str, at: Option<u64>) -> Result<Sent, Code> {
        self.post(user, space, msg_id, &Outgoing::text(content), at)
    }

    pub fn post(&self, user: &str, space: &str, msg_id: &str, out: &Outgoing, at: Option<u64>) -> Result<Sent, Code> {
        if let Form::Pg(p) = &self.form {
            return p.post(user, space, msg_id, out, at);
        }
        self.with(|c| {
            let tx = db(c.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let ver = space_ver(&tx, space)?.ok_or(Code::SpaceNotFound)?;
            let old = db(tx
                .query_row(&format!("SELECT {COLS} FROM messages x WHERE x.msg_id = ?1"), params![msg_id], msg_of)
                .optional())?;
            if let Some(m) = old {
                let erased = db(tx
                    .query_row("SELECT deleted FROM messages WHERE msg_id = ?1", params![probe_of(&m)], |r| r.get::<_, bool>(0))
                    .optional())?
                .unwrap_or(false);
                return again(m, erased, space, user, out);
            }
            if !open_member(&tx, space, user)? {
                return Err(Code::NotSpaceMember);
            }
            check_ref(&tx, space, user, out)?;
            let t = now_or(at);
            let msg = fresh(space, msg_id, user, t, out);
            insert_msg(&tx, &msg)?;
            if out.kind.is_none() {
                db(tx.execute(
                    "UPDATE spaces SET last_t = MAX(COALESCE(last_t, ?2), ?2), total = total + 1 WHERE space_id = ?1",
                    params![space, n(t)],
                ))?;
                db(tx.execute(
                    "INSERT INTO reads (space_id, user_id, read) SELECT ?1, ?2, total FROM spaces WHERE space_id = ?1 ON CONFLICT(space_id, user_id) DO UPDATE SET read = excluded.read",
                    params![space, user],
                ))?;
            } else {
                db(tx.execute("UPDATE spaces SET last_t = MAX(COALESCE(last_t, ?2), ?2) WHERE space_id = ?1", params![space, n(t)]))?;
            }
            if out.kind == Some(ChatKind::Delete)
                && let Some(r) = out.r#ref.as_deref()
            {
                erase(&tx, r)?;
            }
            record(&tx, &msg, user)?;
            db(tx.commit())?;
            Ok(Sent::Fresh { msg, ver })
        })
    }

    pub fn roster(&self, space: &str) -> Result<Roster, Code> {
        if let Form::Pg(p) = &self.form {
            return p.roster(space);
        }
        self.with(|c| {
            let ver = db(c
                .query_row("SELECT ver FROM spaces WHERE space_id = ?1", params![space], |r| r.get::<_, i64>(0))
                .optional())?
            .ok_or(Code::SpaceNotFound)?;
            let mut st = db(c.prepare("SELECT user_id, join_t, leave_t FROM members WHERE space_id = ?1"))?;
            let it = db(st.query_map(params![space], |r| {
                Ok(Interval { user: r.get(0)?, join_t: u(r.get(1)?), leave_t: r.get::<_, Option<i64>>(2)?.map(u) })
            }))?;
            let mut intervals = Vec::new();
            for i in it {
                intervals.push(db(i)?);
            }
            Ok(Roster { ver: u(ver), intervals })
        })
    }

    pub fn history(&self, user: &str, space: &str, dir: Dir, limit: u32) -> Result<Page, Code> {
        self.history_within(user, space, dir, limit, usize::MAX)
    }

    pub fn history_within(&self, user: &str, space: &str, dir: Dir, limit: u32, max_bytes: usize) -> Result<Page, Code> {
        if let Form::Pg(p) = &self.form {
            return p.history_within(user, space, dir, limit, max_bytes);
        }
        self.with(|c| {
            if !db(c.query_row("SELECT EXISTS(SELECT 1 FROM spaces WHERE space_id = ?1)", params![space], |r| r.get::<_, bool>(0)))? {
                return Err(Code::SpaceNotFound);
            }
            let vis = db(c
                .query_row(
                    "SELECT vis FROM members WHERE space_id = ?1 AND user_id = ?2 AND leave_t IS NULL",
                    params![space, user],
                    |r| r.get::<_, String>(0),
                )
                .optional())?
            .ok_or(Code::NotSpaceMember)?;
            let joined = vis == Visibility::Joined.as_str();
            let seen = if joined { SEEN_JOINED } else { SEEN_ALL };
            let (cond, order, edge) = match dir {
                Dir::After(t) => (" AND x.t > ?3", "ASC", n(t)),
                Dir::Before(t) => (" AND x.t < ?3", "DESC", n(t)),
                Dir::Latest => (" AND ?3 = ?3", "DESC", 0),
            };
            let pick = |cond: &str, edge: i64, lim: i64| -> Result<Vec<ChatMessage>, Code> {
                let sql = format!("SELECT {COLS} FROM messages x WHERE x.space_id = ?1{seen}{VISIBLE}{cond} ORDER BY x.t {order}, x.msg_id {order} LIMIT ?4");
                rows(c, &sql, &[&space, &user, &edge, &lim])
            };
            let mut messages = pick(cond, edge, i64::from(limit) + 1)?;
            let lim = limit as usize;
            let mut more = messages.len() > lim;
            if more && messages[lim].t == messages[lim - 1].t {
                let cut = messages[lim - 1].t;
                messages.retain(|m| m.t != cut);
                messages.extend(pick(" AND x.t = ?3", n(cut), i64::MAX)?);
                let beyond = if order == "ASC" { " AND x.t > ?3" } else { " AND x.t < ?3" };
                more = !pick(beyond, n(cut), 1)?.is_empty();
            } else {
                messages.truncate(lim);
            }
            if order == "DESC" {
                messages.reverse();
            }
            let ids: Vec<&String> = messages.iter().filter(|m| m.kind.is_none()).map(|m| &m.msg_id).collect();
            let relations = if ids.is_empty() {
                Vec::new()
            } else {
                let sql = relations_sql(joined, ids.len());
                let mut p: Vec<&dyn rusqlite::ToSql> = vec![&space, &user];
                p.extend(ids.iter().map(|i| *i as &dyn rusqlite::ToSql));
                fold_relations(rows(c, &sql, &p)?)
            };
            let (messages, relations, cut) = within(messages, relations, order == "ASC", max_bytes);
            Ok(Page { messages, relations, more: more || cut })
        })
    }

    pub fn list(&self, user: &str) -> Result<Vec<SpaceEntry>, Code> {
        if let Form::Pg(p) = &self.form {
            return p.list(user);
        }
        self.with(|c| {
            let mut st = db(c.prepare(
                "SELECT s.space_id, s.total, s.last_t, COALESCE(r.read, 0) FROM spaces s
                 LEFT JOIN reads r ON r.space_id = s.space_id AND r.user_id = ?1
                 WHERE EXISTS(SELECT 1 FROM members m WHERE m.space_id = s.space_id AND m.user_id = ?1 AND m.leave_t IS NULL)
                 ORDER BY s.space_id",
            ))?;
            let it = db(st.query_map(params![user], |r| {
                Ok(SpaceEntry {
                    space_id: r.get(0)?,
                    total: u(r.get(1)?),
                    last_t: r.get::<_, Option<i64>>(2)?.map(u),
                    read: u(r.get(3)?),
                })
            }))?;
            let mut v = Vec::new();
            for e in it {
                v.push(db(e)?);
            }
            Ok(v)
        })
    }

    pub fn view(&self, user: &str, space: &str) -> Result<u64, Code> {
        if let Form::Pg(p) = &self.form {
            return p.view(user, space);
        }
        self.with(|c| {
            let tx = db(c.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let total = db(tx
                .query_row("SELECT total FROM spaces WHERE space_id = ?1", params![space], |r| r.get::<_, i64>(0))
                .optional())?
            .ok_or(Code::SpaceNotFound)?;
            if !open_member(&tx, space, user)? {
                return Err(Code::NotSpaceMember);
            }
            db(tx.execute(
                "INSERT INTO reads (space_id, user_id, read) VALUES (?1, ?2, ?3) ON CONFLICT(space_id, user_id) DO UPDATE SET read = excluded.read",
                params![space, user, total],
            ))?;
            db(tx.commit())?;
            Ok(u(total))
        })
    }

    pub fn catchup_check(&self, user: &str, space: &str) -> Result<(), Code> {
        if let Form::Pg(p) = &self.form {
            return p.catchup_check(user, space);
        }
        self.with(|c| {
            if !db(c.query_row("SELECT EXISTS(SELECT 1 FROM spaces WHERE space_id = ?1)", params![space], |r| r.get::<_, bool>(0)))? {
                return Err(Code::SpaceNotFound);
            }
            if !open_member(c, space, user)? {
                return Err(Code::NotSpaceMember);
            }
            Ok(())
        })
    }

    pub fn catchup(&self, user: &str, space: Option<&str>, since: Option<u64>, after: &Cursor, max_msgs: usize, max_bytes: usize) -> Result<Vec<ChatMessage>, Code> {
        let lim = n(max_msgs.max(1) as u64);
        if let Form::Pg(p) = &self.form {
            return p.catchup(user, space, since, after, lim).map(|all| bundle(all, max_msgs, max_bytes));
        }
        self.with(|c| {
            let sp = space.unwrap_or("");
            let d_space = if space.is_some() { "d.space_id = ?2" } else { "?2 = ?2" };
            let mut all = rows(
                c,
                &format!(
                    "SELECT {COLS} FROM undelivered d JOIN messages x ON x.msg_id = d.msg_id
                     WHERE d.user_id = ?1 AND {d_space} AND (d.t, d.msg_id) > (?3, ?4)
                     ORDER BY d.t, d.msg_id LIMIT ?5"
                ),
                &[&user, &sp, &n(after.t), &after.msg_id, &lim],
            )?;
            if let Some(since) = since {
                let (ft, fid) = since_floor(since, after);
                all.extend(rows(c, &catchup_since_sql(space.is_some()), &[&user, &sp, &n(ft), &fid, &lim])?);
            }
            Ok(bundle(all, max_msgs, max_bytes))
        })
    }

    pub fn ack(&self, user: &str, msg_ids: &[String]) -> Result<(), Code> {
        if msg_ids.is_empty() {
            return Ok(());
        }
        if let Form::Pg(p) = &self.form {
            p.ack_batch(None, vec![(user.to_string(), msg_ids.to_vec())])?;
            self.ack_tx.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        self.with(|c| {
            let tx = db(c.transaction_with_behavior(TransactionBehavior::Immediate))?;
            delete_acks(&tx, &[(user.to_string(), msg_ids.to_vec())])?;
            db(tx.commit())?;
            self.ack_tx.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
    }

    pub fn ack_drain(&self, first: Ack, more: &Receiver<Ack>, cap: usize) -> Result<usize, Code> {
        if self.queued.load(Ordering::Acquire) > 0 {
            std::thread::sleep(YIELD);
        }
        if let Form::Pg(p) = &self.form {
            let conn = p.take()?;
            let mut rows = first.1.len();
            let mut batch = vec![first];
            while rows < cap {
                let Ok(a) = more.try_recv() else { break };
                rows += a.1.len();
                batch.push(a);
            }
            p.ack_batch(Some(conn), batch)?;
            self.ack_tx.fetch_add(1, Ordering::Relaxed);
            return Ok(rows);
        }
        self.with(|c| {
            let tx = db(c.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let mut rows = first.1.len();
            let mut batch = vec![first];
            while rows < cap {
                let Ok(a) = more.try_recv() else { break };
                rows += a.1.len();
                batch.push(a);
            }
            delete_acks(&tx, &batch)?;
            db(tx.commit())?;
            self.ack_tx.fetch_add(1, Ordering::Relaxed);
            Ok(rows)
        })
    }

    pub fn ack_transactions(&self) -> u64 {
        self.ack_tx.load(Ordering::Relaxed)
    }

    pub fn plan(&self, sql: &str) -> Result<String, Code> {
        if let Form::Pg(p) = &self.form {
            return p.plan(sql);
        }
        self.with(|c| {
            let mut st = db(c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")))?;
            let unbound = st.parameter_count();
            let it = db(st.query_map(params_from_iter(std::iter::repeat_n(rusqlite::types::Null, unbound)), |r| r.get::<_, String>(3)))?;
            let mut v = Vec::new();
            for d in it {
                v.push(db(d)?);
            }
            Ok(v.join("\n"))
        })
    }

    pub fn purge(&self, before_t: u64) -> Result<usize, Code> {
        if let Form::Pg(p) = &self.form {
            return p.purge(before_t);
        }
        self.with(|c| db(c.execute("DELETE FROM undelivered WHERE t < ?1", params![n(before_t)])))
    }

    pub fn pending(&self, user: &str) -> Result<Vec<String>, Code> {
        if let Form::Pg(p) = &self.form {
            return p.pending(user);
        }
        self.with(|c| {
            let mut st = db(c.prepare("SELECT msg_id FROM undelivered WHERE user_id = ?1 ORDER BY t, msg_id"))?;
            let it = db(st.query_map(params![user], |r| r.get::<_, String>(0)))?;
            let mut v = Vec::new();
            for m in it {
                v.push(db(m)?);
            }
            Ok(v)
        })
    }
}
