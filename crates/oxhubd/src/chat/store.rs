// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-CHAT-STORE · 정:IMPL-CHAT-MEMBERSHIP · 정:IMPL-CHAT-READ · 정:IMPL-CHAT-UNDELIVERED · 정:IMPL-CHAT-CATCHUP · 정:CAND-CHAT-STORE-SCHEMA · 연:RULE-CHAT-MSG-ID · 연:RULE-CHAT-ORDER · 연:RULE-CHAT-VISIBLE · 연:RULE-CHAT-UNREAD · 연:RULE-CHAT-VIEW · 연:RULE-CHAT-HISTORY-PAGE · 연:RULE-CHAT-UNDELIVERED · 연:RULE-CHAT-CATCHUP · 연:RULE-CHAT-MEMBER-NOTICE · model: claude-opus-5-5
//! 채팅 저장소 — SQLite 하나 · 쓰기 트랜잭션은 SQLite 의 쓰기 꼴 `BEGIN IMMEDIATE` · `t` 는 저장 트랜잭션 안에서 찍은 ms(공간마다 줄 세우지 않음 · 같은 `t` 는 `msg_id` 차례) · 메시지와 같은 트랜잭션에 받을 사람 전원 몫 미전송 기록 · 멤버 입퇴장 구간과 멤버 변경 메시지 · 멤버 판 · 읽음 · 쪽은 같은 `t` 를 가르지 않음 · 따라잡기 쪽은 미전송 기록과 `t` 이상 메시지를 `msg_id` 로 합침.

use std::sync::Mutex;
use std::time::Duration;

use common::system::Visibility;
use oxsig::Code;
use oxsig::msg::{ChatMessage, MemberKind, SpaceEntry};
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
  target TEXT
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
";

const COLS: &str = "x.msg_id, x.space_id, x.user_id, x.t, x.content, x.kind, x.target";

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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cursor {
    pub t: u64,
    pub msg_id: String,
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
    Ok(ChatMessage {
        msg_id: r.get(0)?,
        space_id: r.get(1)?,
        user_id: r.get(2)?,
        t: u(r.get(3)?),
        content: r.get(4)?,
        kind: kind.as_deref().and_then(MemberKind::parse),
        target: r.get(6)?,
    })
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

fn now_or(at: Option<u64>) -> u64 {
    at.unwrap_or_else(common::now_ms)
}

pub fn sys_msg_id(t: u64) -> String {
    let r = uuid::Uuid::new_v4();
    let b = r.as_bytes();
    let rand: String = b[..8].iter().map(|x| format!("{x:02x}")).collect();
    format!("{SYS_HASH}{:012x}{rand}", t & 0xffff_ffff_ffff)
}

fn insert_msg(tx: &Transaction, m: &ChatMessage) -> Result<(), Code> {
    db(tx.execute(
        "INSERT INTO messages (msg_id, space_id, user_id, t, content, kind, target) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![m.msg_id, m.space_id, m.user_id, n(m.t), m.content, m.kind.map(|k| k.as_str()), m.target],
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
    pub fn open(path: &str, default_vis: Visibility) -> Result<Store, String> {
        let c = if path == ":memory:" { Connection::open_in_memory() } else { Connection::open(path) }.map_err(|e| format!("chat db {path}: {e}"))?;
        c.busy_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
        if path != ":memory:" {
            let _ = c.pragma_update(None, "journal_mode", "WAL");
        }
        c.execute_batch(SCHEMA).map_err(|e| format!("chat db schema: {e}"))?;
        for col in ["kind", "target"] {
            let _ = c.execute(&format!("ALTER TABLE messages ADD COLUMN {col} TEXT"), []);
        }
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

    pub fn join(&self, space: &str, user: &str, by: Option<&str>, at: Option<u64>) -> Result<Option<(ChatMessage, u64)>, Code> {
        let default_vis = self.default_vis;
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
            let actor = by.unwrap_or(user);
            let (kind, target) = if actor == user { (MemberKind::Join, None) } else { (MemberKind::Add, Some(user.to_string())) };
            let msg = ChatMessage {
                space_id: space.into(),
                msg_id: sys_msg_id(t),
                user_id: actor.into(),
                t,
                content: String::new(),
                kind: Some(kind),
                target,
            };
            insert_msg(&tx, &msg)?;
            record(&tx, &msg, "")?;
            db(tx.execute("UPDATE spaces SET ver = ver + 1 WHERE space_id = ?1", params![space]))?;
            db(tx.commit())?;
            Ok(Some((msg, ver + 1)))
        })
    }

    pub fn leave(&self, space: &str, user: &str, by: Option<&str>, at: Option<u64>) -> Result<(ChatMessage, u64), Code> {
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
            let actor = by.unwrap_or(user);
            let (kind, target) = if actor == user { (MemberKind::Leave, None) } else { (MemberKind::Remove, Some(user.to_string())) };
            let msg = ChatMessage {
                space_id: space.into(),
                msg_id: sys_msg_id(t),
                user_id: actor.into(),
                t,
                content: String::new(),
                kind: Some(kind),
                target,
            };
            insert_msg(&tx, &msg)?;
            record(&tx, &msg, "")?;
            db(tx.execute("UPDATE spaces SET ver = ver + 1 WHERE space_id = ?1", params![space]))?;
            db(tx.commit())?;
            Ok((msg, ver + 1))
        })
    }

    pub fn send(&self, user: &str, space: &str, msg_id: &str, content: &str, at: Option<u64>) -> Result<Sent, Code> {
        self.with(|c| {
            let tx = db(c.transaction_with_behavior(TransactionBehavior::Immediate))?;
            let ver = space_ver(&tx, space)?.ok_or(Code::SpaceNotFound)?;
            let old = db(tx
                .query_row(&format!("SELECT {COLS} FROM messages x WHERE x.msg_id = ?1"), params![msg_id], msg_of)
                .optional())?;
            if let Some(m) = old {
                if m.space_id == space && m.user_id == user && m.content == content && m.kind.is_none() {
                    return Ok(Sent::Again { msg: m });
                }
                return Err(Code::MsgIdConflict);
            }
            if !open_member(&tx, space, user)? {
                return Err(Code::NotSpaceMember);
            }
            let t = now_or(at);
            let msg = ChatMessage {
                space_id: space.into(),
                msg_id: msg_id.into(),
                user_id: user.into(),
                t,
                content: content.into(),
                kind: None,
                target: None,
            };
            insert_msg(&tx, &msg)?;
            db(tx.execute(
                "UPDATE spaces SET last_t = MAX(COALESCE(last_t, ?2), ?2), total = total + 1 WHERE space_id = ?1",
                params![space, n(t)],
            ))?;
            db(tx.execute(
                "INSERT INTO reads (space_id, user_id, read) SELECT ?1, ?2, total FROM spaces WHERE space_id = ?1 ON CONFLICT(space_id, user_id) DO UPDATE SET read = excluded.read",
                params![space, user],
            ))?;
            record(&tx, &msg, user)?;
            db(tx.commit())?;
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
            let seen = if vis == Visibility::Joined.as_str() {
                " AND EXISTS(SELECT 1 FROM members m WHERE m.space_id = x.space_id AND m.user_id = ?2 AND m.join_t <= x.t AND (m.leave_t IS NULL OR x.t < m.leave_t))"
            } else {
                " AND ?2 = ?2"
            };
            let (cond, order, edge) = match dir {
                Dir::After(t) => (" AND x.t > ?3", "ASC", n(t)),
                Dir::Before(t) => (" AND x.t < ?3", "DESC", n(t)),
                Dir::Latest => (" AND ?3 = ?3", "DESC", 0),
            };
            let pick = |cond: &str, edge: i64, lim: i64| -> Result<Vec<ChatMessage>, Code> {
                let sql = format!("SELECT {COLS} FROM messages x WHERE x.space_id = ?1{seen}{cond} ORDER BY x.t {order}, x.msg_id {order} LIMIT ?4");
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
        self.with(|c| {
            let sp = space.unwrap_or("");
            let mut all = rows(
                c,
                &format!(
                    "SELECT {COLS} FROM undelivered d JOIN messages x ON x.msg_id = d.msg_id
                     WHERE d.user_id = ?1 AND (?2 = '' OR d.space_id = ?2) AND (d.t, d.msg_id) > (?3, ?4)
                     ORDER BY d.t, d.msg_id LIMIT ?5"
                ),
                &[&user, &sp, &n(after.t), &after.msg_id, &lim],
            )?;
            if let Some(since) = since {
                all.extend(rows(
                    c,
                    &format!(
                        "SELECT {COLS} FROM messages x
                         WHERE (?2 = '' OR x.space_id = ?2) AND x.t >= ?6 AND (x.t, x.msg_id) > (?3, ?4)
                           AND EXISTS(SELECT 1 FROM members mm WHERE mm.space_id = x.space_id AND mm.user_id = ?1 AND mm.leave_t IS NULL
                             AND (mm.vis = 'shared' OR EXISTS(SELECT 1 FROM members m2 WHERE m2.space_id = x.space_id AND m2.user_id = ?1
                               AND m2.join_t <= x.t AND (m2.leave_t IS NULL OR x.t < m2.leave_t))))
                         ORDER BY x.t, x.msg_id LIMIT ?5"
                    ),
                    &[&user, &sp, &n(after.t), &after.msg_id, &lim, &n(since)],
                )?);
            }
            all.sort_by(|a, b| (a.t, &a.msg_id).cmp(&(b.t, &b.msg_id)));
            all.dedup_by(|a, b| a.msg_id == b.msg_id);
            let mut out = Vec::new();
            let mut bytes = 0usize;
            for m in all {
                let size = serde_json::to_vec(&m).map(|b| b.len()).unwrap_or(0);
                if !out.is_empty() && (out.len() >= max_msgs || bytes + size > max_bytes) {
                    break;
                }
                bytes += size;
                out.push(m);
            }
            Ok(out)
        })
    }

    pub fn ack(&self, user: &str, msg_ids: &[String]) -> Result<(), Code> {
        if msg_ids.is_empty() {
            return Ok(());
        }
        self.with(|c| {
            let marks = vec!["?"; msg_ids.len()].join(", ");
            let sql = format!("DELETE FROM undelivered WHERE user_id = ? AND msg_id IN ({marks})");
            let mut p: Vec<&dyn rusqlite::ToSql> = vec![&user];
            p.extend(msg_ids.iter().map(|m| m as &dyn rusqlite::ToSql));
            db(c.execute(&sql, params_from_iter(p)))?;
            Ok(())
        })
    }

    pub fn purge(&self, before_t: u64) -> Result<usize, Code> {
        self.with(|c| db(c.execute("DELETE FROM undelivered WHERE t < ?1", params![n(before_t)])))
    }

    pub fn pending(&self, user: &str) -> Result<Vec<String>, Code> {
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
