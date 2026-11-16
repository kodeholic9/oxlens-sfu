// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-CHAT-STORE · 정:IMPL-CHAT-MEMBERSHIP · 정:IMPL-CHAT-READ · 정:CAND-CHAT-STORE-SCHEMA · 연:RULE-CHAT-MSG-ID · 연:RULE-CHAT-ORDER · 연:RULE-CHAT-VISIBLE · 연:RULE-CHAT-UNREAD · 연:RULE-CHAT-VIEW · 연:RULE-CHAT-HISTORY-PAGE · 연:RULE-CHAT-CATCHUP-BASIS · model: claude-opus-5-5
//! 채팅 저장소 — SQLite 하나 · 쓰기는 `BEGIN IMMEDIATE`(공간 행 잠금을 덮는 잠금) · 공간 시계 `t = max(지금, 앞 t + 1)` · 멤버 입퇴장 구간(보이기 시작점 값은 입장 때 구간에 저장) · 멤버 판 · 읽음(보낸 사람은 보낸 순간까지 읽은 것으로).

use std::sync::Mutex;
use std::time::Duration;

use common::system::Visibility;
use oxsig::Code;
use oxsig::msg::{ChatMessage, SpaceEntry};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

pub const HISTORY_DEFAULT: u32 = 60;
pub const HISTORY_MAX: u32 = 200;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS spaces (
  space_id TEXT PRIMARY KEY,
  clock INTEGER NOT NULL DEFAULT 0,
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
  content TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_space_t ON messages (space_id, t);
CREATE TABLE IF NOT EXISTS reads (
  space_id TEXT NOT NULL,
  user_id TEXT NOT NULL,
  read INTEGER NOT NULL,
  PRIMARY KEY (space_id, user_id)
);
";

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
    pub more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    After(u64),
    Before(u64),
    Latest,
}

pub struct Store {
    conn: Mutex<Connection>,
    default_vis: Visibility,
}

fn db<T>(r: rusqlite::Result<T>) -> Result<T, Code> {
    r.map_err(|_| Code::InternalError)
}

fn n(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn u(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn space_row(tx: &Transaction, space: &str) -> Result<Option<(u64, u64)>, Code> {
    db(tx
        .query_row("SELECT clock, ver FROM spaces WHERE space_id = ?1", params![space], |r| Ok((u(r.get(0)?), u(r.get(1)?))))
        .optional())
}

fn open_member(c: &Connection, space: &str, user: &str) -> Result<bool, Code> {
    db(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM members WHERE space_id = ?1 AND user_id = ?2 AND leave_t IS NULL)",
        params![space, user],
        |r| r.get::<_, bool>(0),
    ))
}

fn stamp(tx: &Transaction, space: &str, clock: u64, now: u64) -> Result<u64, Code> {
    let t = now.max(clock.saturating_add(1));
    db(tx.execute("UPDATE spaces SET clock = ?2 WHERE space_id = ?1", params![space, n(t)]))?;
    Ok(t)
}

fn msg_of(r: &rusqlite::Row) -> rusqlite::Result<ChatMessage> {
    Ok(ChatMessage { msg_id: r.get(0)?, space_id: r.get(1)?, user_id: r.get(2)?, t: u(r.get(3)?), content: r.get(4)? })
}

impl Store {
    pub fn open(path: &str, default_vis: Visibility) -> Result<Store, String> {
        let c = if path == ":memory:" { Connection::open_in_memory() } else { Connection::open(path) }.map_err(|e| format!("chat db {path}: {e}"))?;
        c.busy_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
        if path != ":memory:" {
            let _ = c.pragma_update(None, "journal_mode", "WAL");
        }
        c.execute_batch(SCHEMA).map_err(|e| format!("chat db schema: {e}"))?;
        Ok(Store { conn: Mutex::new(c), default_vis })
    }

    fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T, Code>) -> Result<T, Code> {
        let mut g = self.conn.lock().map_err(|_| Code::InternalError)?;
        f(&mut g)
    }

    pub fn create_space(&self, space: &str, vis: Option<Visibility>) -> Result<(), Code> {
        self.with(|c| {
            db(c.execute(
                "INSERT OR IGNORE INTO spaces (space_id, visibility) VALUES (?1, ?2)",
                params![space, vis.map(|v| v.as_str())],
            ))?;
            Ok(())
        })
    }

    pub fn join(&self, space: &str, user: &str, now: u64) -> Result<u64, Code> {
        let default_vis = self.default_vis;
        self.with(|c| {
            let tx = db(c.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let (clock, _) = space_row(&tx, space)?.ok_or(Code::SpaceNotFound)?;
            if open_member(&tx, space, user)? {
                return Ok(clock);
            }
            let own = db(tx.query_row("SELECT visibility FROM spaces WHERE space_id = ?1", params![space], |r| r.get::<_, Option<String>>(0)))?;
            let vis = own.unwrap_or_else(|| default_vis.as_str().to_string());
            let t = stamp(&tx, space, clock, now)?;
            db(tx.execute(
                "INSERT INTO members (space_id, user_id, join_t, vis) VALUES (?1, ?2, ?3, ?4)",
                params![space, user, n(t), vis],
            ))?;
            db(tx.execute("UPDATE spaces SET ver = ver + 1 WHERE space_id = ?1", params![space]))?;
            db(tx.commit())?;
            Ok(t)
        })
    }

    pub fn leave(&self, space: &str, user: &str, now: u64) -> Result<u64, Code> {
        self.with(|c| {
            let tx = db(c.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let (clock, _) = space_row(&tx, space)?.ok_or(Code::SpaceNotFound)?;
            if !open_member(&tx, space, user)? {
                return Err(Code::NotSpaceMember);
            }
            let t = stamp(&tx, space, clock, now)?;
            db(tx.execute(
                "UPDATE members SET leave_t = ?3 WHERE space_id = ?1 AND user_id = ?2 AND leave_t IS NULL",
                params![space, user, n(t)],
            ))?;
            db(tx.execute("UPDATE spaces SET ver = ver + 1 WHERE space_id = ?1", params![space]))?;
            db(tx.commit())?;
            Ok(t)
        })
    }

    pub fn send(&self, user: &str, space: &str, msg_id: &str, content: &str, now: u64) -> Result<Sent, Code> {
        self.with(|c| {
            let tx = db(c.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let (clock, ver) = space_row(&tx, space)?.ok_or(Code::SpaceNotFound)?;
            let old = db(tx
                .query_row(
                    "SELECT msg_id, space_id, user_id, t, content FROM messages WHERE msg_id = ?1",
                    params![msg_id],
                    msg_of,
                )
                .optional())?;
            if let Some(m) = old {
                if m.space_id == space && m.user_id == user && m.content == content {
                    return Ok(Sent::Again { msg: m });
                }
                return Err(Code::MsgIdConflict);
            }
            if !open_member(&tx, space, user)? {
                return Err(Code::NotSpaceMember);
            }
            let t = stamp(&tx, space, clock, now)?;
            db(tx.execute(
                "INSERT INTO messages (msg_id, space_id, user_id, t, content) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![msg_id, space, user, n(t), content],
            ))?;
            db(tx.execute("UPDATE spaces SET last_t = ?2, total = total + 1 WHERE space_id = ?1", params![space, n(t)]))?;
            db(tx.execute(
                "INSERT INTO reads (space_id, user_id, read) SELECT ?1, ?2, total FROM spaces WHERE space_id = ?1 ON CONFLICT(space_id, user_id) DO UPDATE SET read = excluded.read",
                params![space, user],
            ))?;
            db(tx.commit())?;
            let msg = ChatMessage { space_id: space.into(), msg_id: msg_id.into(), user_id: user.into(), t, content: content.into() };
            Ok(Sent::Fresh { msg, ver })
        })
    }

    pub fn roster(&self, space: &str) -> Result<Roster, Code> {
        self.with(|c| {
            let ver = db(c
                .query_row("SELECT ver FROM spaces WHERE space_id = ?1", params![space], |r| r.get::<_, i64>(0))
                .optional())?
            .ok_or(Code::SpaceNotFound)?;
            let mut st = db(c.prepare("SELECT user_id, join_t, leave_t FROM members WHERE space_id = ?1"))?;
            let rows = db(st.query_map(params![space], |r| {
                Ok(Interval { user: r.get(0)?, join_t: u(r.get(1)?), leave_t: r.get::<_, Option<i64>>(2)?.map(u) })
            }))?;
            let mut intervals = Vec::new();
            for i in rows {
                intervals.push(db(i)?);
            }
            Ok(Roster { ver: u(ver), intervals })
        })
    }

    pub fn history(&self, user: &str, space: &str, dir: Dir, limit: u32) -> Result<Page, Code> {
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
            let seen = if joined {
                " AND EXISTS(SELECT 1 FROM members m WHERE m.space_id = x.space_id AND m.user_id = ?2 AND m.join_t <= x.t AND (m.leave_t IS NULL OR x.t < m.leave_t))"
            } else {
                " AND ?2 = ?2"
            };
            let (cond, order, edge) = match dir {
                Dir::After(t) => (" AND x.t > ?3", "ASC", n(t)),
                Dir::Before(t) => (" AND x.t < ?3", "DESC", n(t)),
                Dir::Latest => (" AND ?3 = ?3", "DESC", 0),
            };
            let sql = format!(
                "SELECT x.msg_id, x.space_id, x.user_id, x.t, x.content FROM messages x WHERE x.space_id = ?1{seen}{cond} ORDER BY x.t {order} LIMIT ?4"
            );
            let mut st = db(c.prepare(&sql))?;
            let rows = db(st.query_map(params![space, user, edge, i64::from(limit) + 1], msg_of))?;
            let mut messages = Vec::new();
            for m in rows {
                messages.push(db(m)?);
            }
            let more = messages.len() > limit as usize;
            messages.truncate(limit as usize);
            if order == "DESC" {
                messages.reverse();
            }
            Ok(Page { messages, more })
        })
    }

    pub fn list(&self, user: &str) -> Result<Vec<SpaceEntry>, Code> {
        self.with(|c| {
            let mut st = db(c.prepare(
                "SELECT s.space_id, s.total, s.last_t, COALESCE(r.read, 0) FROM spaces s
                 LEFT JOIN reads r ON r.space_id = s.space_id AND r.user_id = ?1
                 WHERE EXISTS(SELECT 1 FROM members m WHERE m.space_id = s.space_id AND m.user_id = ?1 AND m.leave_t IS NULL)
                 ORDER BY s.space_id",
            ))?;
            let rows = db(st.query_map(params![user], |r| {
                Ok(SpaceEntry {
                    space_id: r.get(0)?,
                    total: u(r.get(1)?),
                    last_t: r.get::<_, Option<i64>>(2)?.map(u),
                    read: u(r.get(3)?),
                })
            }))?;
            let mut v = Vec::new();
            for e in rows {
                v.push(db(e)?);
            }
            Ok(v)
        })
    }

    pub fn view(&self, user: &str, space: &str) -> Result<u64, Code> {
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
}
