// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-OPS-SURFACE · 정:RULE-OPS-TOKEN · 정:IMPL-OPS-CUT · 정:IMPL-OPS-DESTROY-REAP · 정:RULE-OPS-OBSERVE · model: claude-opus-5-5
//! `oxadmin` 명령표(운영 규격서 §6) — 도구는 이 표를 넘지 않는다 · 붙은 hub 를 매번 stderr 첫 줄에 · 확인값은 도구가 먼저 읽어 되싣는다 · 모르는 값은 `-`.

pub mod http;

use std::time::Duration;

use serde_json::Value;

use self::http::{Hub, Resp};

pub const DEFAULT_HUB: &str = "http://127.0.0.1:1974";

pub struct Opts {
    pub hub: Option<String>,
    pub key: Option<String>,
    pub secret: Option<String>,
    pub json: bool,
    pub ttl: Option<u64>,
    pub interval_ms: u64,
    pub all: bool,
    pub args: Vec<String>,
}

pub fn parse(argv: Vec<String>) -> Result<Opts, String> {
    let mut o = Opts { hub: None, key: None, secret: None, json: false, ttl: None, interval_ms: 1000, all: false, args: Vec::new() };
    let mut it = argv.into_iter();
    while let Some(a) = it.next() {
        let (k, inline) = match a.split_once('=') {
            Some((k, v)) if k.starts_with("--") => (k.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let mut val = |name: &str| inline.clone().or_else(|| it.next()).ok_or(format!("{name} 값 없음"));
        match k.as_str() {
            "--hub" => o.hub = Some(val("--hub")?),
            "--api-key" => o.key = Some(val("--api-key")?),
            "--api-secret" => o.secret = Some(val("--api-secret")?),
            "--ttl" => o.ttl = Some(val("--ttl")?.parse().map_err(|_| "--ttl 숫자")?),
            "--interval" => o.interval_ms = val("--interval")?.parse().map_err(|_| "--interval 숫자(ms)")?,
            "--json" => o.json = true,
            "--all" => o.all = true,
            x if x.starts_with("--") => return Err(format!("모르는 선택 {x}")),
            _ => o.args.push(a),
        }
    }
    Ok(o)
}

pub fn resolve(o: &Opts) -> (String, &'static str) {
    if let Some(h) = &o.hub {
        return (h.clone(), "--hub");
    }
    if let Ok(h) = std::env::var("OXADMIN_URL") {
        return (h, "OXADMIN_URL");
    }
    (DEFAULT_HUB.to_string(), "default")
}

fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        None | Some(Value::Null) => "-".to_string(),
        Some(Value::String(x)) => x.clone(),
        Some(x) => x.to_string(),
    }
}

fn table(rows: &[Vec<String>]) -> String {
    let n = rows.iter().map(Vec::len).max().unwrap_or(0);
    let w: Vec<usize> = (0..n).map(|i| rows.iter().map(|r| r.get(i).map(|c| c.chars().count()).unwrap_or(0)).max().unwrap_or(0)).collect();
    rows.iter()
        .map(|r| r.iter().enumerate().map(|(i, c)| format!("{c:<width$}", width = w[i])).collect::<Vec<_>>().join("  ").trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn list(v: &Value, k: &str) -> Vec<Value> {
    v.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn show(path: &str, v: &Value) -> String {
    let p: Vec<&str> = path.trim_matches('/').split('/').collect();
    match p.as_slice() {
        ["admin", "sfus"] => {
            let mut rows = vec![vec!["UNIT".into(), "SFU_ID".into(), "STATE".into(), "LIVE".into(), "NODE_LIVE".into(), "ADDR".into()]];
            for x in list(v, "sfus") {
                rows.push(vec![s(&x, "unit"), s(&x, "sfu_id"), s(&x, "unit_state"), s(&x, "live"), s(&x, "node_live"), s(&x, "addr")]);
            }
            table(&rows)
        }
        ["admin", "rooms"] => {
            let mut rows = vec![vec!["ROOM".into(), "NODE".into(), "SFU_ID".into(), "MEMBERS".into(), "MEMBER_IDS".into()]];
            for x in list(v, "rooms") {
                if x.get("error").is_some() {
                    rows.push(vec!["-".into(), s(&x, "node"), "-".into(), format!("error {}", s(&x, "error")), String::new()]);
                    continue;
                }
                rows.push(vec![s(&x, "room_id"), s(&x, "node"), s(&x, "sfu_id"), s(&x, "members"), s(&x, "member_ids")]);
            }
            format!("{}\ntotal {}", table(&rows), s(v, "total"))
        }
        ["admin", "users"] => {
            let mut rows = vec![vec!["USER".into(), "SESSION".into(), "TYPE".into(), "HIDDEN".into(), "LINKS".into()]];
            for x in list(v, "users") {
                let links: Vec<String> = list(&x, "links").iter().map(|l| format!("{}:{}:{}", s(l, "role"), s(l, "state"), s(l, "ufrag"))).collect();
                rows.push(vec![s(&x, "user_id"), s(&x, "session_id"), s(&x, "participant_type"), s(&x, "hidden"), links.join(" ")]);
            }
            format!("{}\ntotal {}", table(&rows), s(v, "total"))
        }
        ["admin", "supervisor", "status"] => {
            let mut rows = vec![vec!["ID".into(), "KIND".into(), "ORDER".into(), "STATE".into(), "RESTARTS".into(), "NEXT_RETRY_MS".into(), "EPOCH".into()]];
            for x in list(v, "units") {
                rows.push(vec![s(&x, "id"), s(&x, "kind"), s(&x, "order"), s(&x, "state"), s(&x, "restarts"), s(&x, "next_retry_ms"), s(&x, "epoch")]);
            }
            table(&rows)
        }
        ["admin", "rooms", _, "snapshot"] => {
            let r = v.get("room").cloned().unwrap_or(Value::Null);
            let f = r.get("floor").cloned().unwrap_or(Value::Null);
            let ver = r.get("version").cloned().unwrap_or(Value::Null);
            let mut out = format!(
                "room {}  name {}  users {}/{}  hidden {}  rec {}  version {}:{}\nfloor {}  speaker {}  queue {}  last_rtp_at {}\n",
                s(&r, "room_id"),
                s(&r, "name"),
                s(&r, "user_count"),
                s(&r, "capacity"),
                s(&r, "hidden_count"),
                s(&r, "rec"),
                s(&ver, "epoch"),
                s(&ver, "seq"),
                s(&f, "state"),
                s(&f, "speaker"),
                list(&f, "queue").len(),
                s(&f, "last_rtp_at")
            );
            let mut rows = vec![vec!["TRACK".into(), "TYPE".into(), "KIND".into(), "USER".into(), "ACTIVE".into(), "MUTED".into()]];
            for t in list(&r, "tracks") {
                rows.push(vec![s(&t, "track_id"), s(&t, "type"), s(&t, "kind"), s(&t, "user_id"), s(&t, "active"), s(&t, "muted")]);
            }
            out.push_str(&table(&rows));
            out
        }
        _ => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

fn ok(r: Resp) -> Result<Value, String> {
    if (200..300).contains(&r.status) {
        Ok(r.body)
    } else {
        Err(format!("HTTP {} {}", r.status, r.body))
    }
}

fn user_delta(a: &Value, b: &Value) -> String {
    let mut rows = vec![vec!["SFU".into(), "MID".into(), "KIND".into(), "TRACK".into(), "SRC".into(), "LAST_SEQ".into(), "ΔSEQ".into(), "ΔLAST_AT".into(), "FLOW".into()]];
    let la = list(a, "links");
    for (i, l2) in list(b, "links").iter().enumerate() {
        let l1 = la.get(i).cloned().unwrap_or(Value::Null);
        let m1 = list(&l1, "mlines");
        for m in list(l2, "mlines") {
            let e = m.get("entry").cloned().unwrap_or(Value::Null);
            let mid = s(&e, "mid");
            let prev = m1.iter().find(|x| x.get("entry").map(|y| s(y, "mid")) == Some(mid.clone())).and_then(|x| x.get("rewrite").cloned()).unwrap_or(Value::Null);
            let rw = m.get("rewrite").cloned().unwrap_or(Value::Null);
            let d = |k: &str| match (prev.get(k).and_then(Value::as_i64), rw.get(k).and_then(Value::as_i64)) {
                (Some(x), Some(y)) => (y - x).to_string(),
                _ => "-".to_string(),
            };
            let flow = if d("last_at") != "-" && d("last_at") != "0" { "흐름" } else if rw.is_null() { "-" } else { "멎음" };
            let src = rw.get("src").map(|x| s(x, "user_id")).unwrap_or_else(|| "-".into());
            rows.push(vec![s(l2, "sfu_id"), mid, s(&e, "kind"), s(&e, "track_id"), src, s(&rw, "last_seq"), d("last_seq"), d("last_at"), flow.into()]);
        }
    }
    table(&rows)
}

pub fn run(o: &Opts, hub: &Hub) -> Result<String, String> {
    let a: Vec<&str> = o.args.iter().map(String::as_str).collect();
    let get = |p: &str| -> Result<(String, Value), String> { Ok((p.to_string(), ok(hub.call("GET", p, None)?)?)) };
    let post = |p: &str, b: Option<&Value>| -> Result<(String, Value), String> { Ok((p.to_string(), ok(hub.call("POST", p, b)?)?)) };
    let (path, v) = match a.as_slice() {
        ["sfus"] => get("/admin/sfus")?,
        ["sfu", id] => get(&format!("/admin/sfus/{id}/rooms"))?,
        ["rooms"] => get("/admin/rooms")?,
        ["room", id] => get(&format!("/admin/rooms/{id}/snapshot"))?,
        ["users"] => get("/admin/users")?,
        ["user", id] => {
            let p = format!("/admin/users/{id}/media");
            let first = ok(hub.call("GET", &p, None)?)?;
            std::thread::sleep(Duration::from_millis(o.interval_ms));
            let second = ok(hub.call("GET", &p, None)?)?;
            if o.json {
                return Ok(serde_json::to_string_pretty(&serde_json::json!({"reads": [first, second], "interval_ms": o.interval_ms})).unwrap_or_default());
            }
            return Ok(user_delta(&first, &second));
        }
        ["snapshot"] => get("/admin/snapshot")?,
        ["drops"] => get("/admin/drops")?,
        ["bus"] => get(if o.all { "/admin/bus?all=1" } else { "/admin/bus" })?,
        ["unit", "list"] => get("/admin/supervisor/status")?,
        ["unit", "show", id] => {
            let (p, v) = get("/admin/supervisor/status")?;
            let u = list(&v, "units").into_iter().find(|x| s(x, "id") == *id).ok_or(format!("유닛 {id} 없음"))?;
            (p, u)
        }
        ["load", id] => post(&format!("/admin/supervisor/load/{id}"), None)?,
        ["stop", id] => post(&format!("/admin/supervisor/stop/{id}"), None)?,
        ["kill", id] => {
            let (_, st) = get("/admin/supervisor/status")?;
            let epoch = list(&st, "units").into_iter().find(|x| s(x, "id") == *id).map(|x| s(&x, "epoch")).filter(|e| e != "-").ok_or(format!("유닛 {id} 의 epoch 를 못 읽었다"))?;
            post(&format!("/admin/supervisor/kill/{id}?if_epoch={epoch}"), None)?
        }
        ["shutdown"] => {
            let (_, snap) = get("/admin/snapshot")?;
            let hid = s(&snap, "hub_id");
            post(&format!("/admin/supervisor/shutdown?confirm={hid}"), None)?
        }
        ["user-cut", user] => post(&format!("/admin/users/{user}/cut"), None)?,
        ["reap", room, user] => post(&format!("/admin/rooms/{room}/reap/{user}"), None)?,
        ["destroy", room] => {
            let (_, snap) = get(&format!("/admin/rooms/{room}/snapshot"))?;
            let ver = snap.pointer("/room/version").cloned().ok_or("방 version 을 못 읽었다")?;
            post(&format!("/admin/rooms/{room}/destroy?if_version={}:{}", s(&ver, "epoch"), s(&ver, "seq")), None)?
        }
        ["log-level", level] => {
            let mut b = serde_json::json!({"level": level});
            if let Some(t) = o.ttl {
                b["ttl_secs"] = serde_json::json!(t);
            }
            post("/admin/log-level", Some(&b))?
        }
        _ => return Err(USAGE.to_string()),
    };
    if o.json {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    Ok(show(path.split('?').next().unwrap_or(&path), &v))
}

pub const USAGE: &str = "oxadmin [--hub URL] [--api-key K --api-secret S] [--json] <명령>
  sfus | sfu <id> | rooms | room <id> | users | user <id> [--interval ms] | snapshot | drops | bus [--all]
  unit list | unit show <id> | load <id> | stop <id> | kill <id> | shutdown
  user-cut <user> | reap <room> <user> | destroy <room> | log-level <level> [--ttl secs]";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_prefers_flag() {
        let o = parse(vec!["--hub".into(), "http://h:1".into(), "rooms".into()]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resolve(&o), ("http://h:1".to_string(), "--hub"), "O-sym-6-2");
        assert_eq!(o.args, vec!["rooms".to_string()]);
    }

    #[test]
    fn unknown_values_are_dashes() {
        let v = serde_json::json!({"sfus": [{"unit": "node-a", "unit_state": "Down"}]});
        let t = show("/admin/sfus", &v);
        assert!(t.contains("node-a") && t.contains('-'), "O-sym-6-1 모르는 값은 - 로 둔다");
    }
}
