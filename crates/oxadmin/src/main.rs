// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 운영:RULE-TOOL-ONE 운영:RULE-TOOL-CONTRACT 운영:RULE-RISK-CONFIRM 운영:RULE-AUTH-THREE 운영:RULE-FMT-NO-INVENT 운영:RULE-READ-BUS-ALL 운영:RULE-READ-USER-MEDIA 운영:RULE-CTL-SUPERVISOR 운영:RULE-CTL-ROOM 운영:RULE-CTL-CUT 운영:RULE-CTL-LOG-LEVEL 운영:RULE-BOOT-PORTS 정:RULE-OPS-TOKEN · model: claude-opus-5-5
//! oxadmin — 도구는 하나 · 사람이 읽는 표가 기본(`--json` raw) · 붙은 곳을 stderr 첫 줄에 · 확인값은 도구가 먼저 읽어 되싣는다 · 모르는 값은 `-`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::Value;

const DEFAULT_HUB: &str = "http://127.0.0.1:1974";
const OPS_TTL_SECS: u64 = 60;

struct Opts {
    hub: String,
    json: bool,
    api_key: Option<String>,
    api_secret: Option<String>,
    interval_ms: u64,
    ttl: Option<u32>,
    rest: Vec<String>,
}

fn usage() -> ! {
    eprintln!(
        "usage: oxadmin [--hub URL] [--json] [--api-key K --api-secret S] <cmd>\n  sfus | sfu <id> | rooms | room <id> | users | user <id> [--interval ms] | snapshot | drops | bus [--all]\n  unit list | unit show <id> | load <id> | stop <id> | kill <id> | shutdown\n  probe <user> | user-cut <user> | reap <room> <user> | destroy <room> | log-level <level> [--ttl secs]"
    );
    std::process::exit(2)
}

fn parse() -> Opts {
    let mut o = Opts { hub: String::new(), json: false, api_key: None, api_secret: None, interval_ms: 1000, ttl: None, rest: Vec::new() };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut hub_flag = None;
    let mut all = false;
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        let mut val = || -> String {
            i += 1;
            argv.get(i).cloned().unwrap_or_else(|| usage())
        };
        match a {
            "--hub" => hub_flag = Some(val()),
            "--json" => o.json = true,
            "--api-key" => o.api_key = Some(val()),
            "--api-secret" => o.api_secret = Some(val()),
            "--interval" => o.interval_ms = val().parse().unwrap_or(1000),
            "--ttl" => o.ttl = val().parse().ok(),
            "--all" => all = true,
            "-h" | "--help" => usage(),
            _ => o.rest.push(argv[i].clone()),
        }
        i += 1;
    }
    if all {
        o.rest.push("--all".into());
    }
    let (hub, src) = match (hub_flag, std::env::var("OXADMIN_URL").ok().filter(|s| !s.is_empty())) {
        (Some(h), _) => (h, "--hub"),
        (None, Some(e)) => (e, "OXADMIN_URL"),
        (None, None) => (DEFAULT_HUB.to_string(), "기본값"),
    };
    eprintln!("[oxadmin] hub={hub} ({src})");
    o.hub = hub.trim_end_matches('/').to_string();
    o
}

fn token(o: &Opts) -> Option<String> {
    let (k, s) = (o.api_key.as_ref()?, o.api_secret.as_ref()?);
    let now = common::jwt::now_secs();
    common::jwt::sign(&common::jwt::OpsClaims { iss: k.clone(), ops: true, iat: now, exp: now + OPS_TTL_SECS }, s).ok()
}

struct Resp {
    status: u16,
    body: Value,
}

fn http(o: &Opts, method: &str, path: &str, body: Option<&Value>) -> Result<Resp, String> {
    let rest = match o.hub.split_once("://") {
        Some(("http", r)) => r,
        Some(_) => return Err(format!("http:// 만 지원한다: {}", o.hub)),
        None => o.hub.as_str(),
    };
    let (host, base) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let mut s = TcpStream::connect(host).map_err(|e| format!("connect {host}: {e}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let auth = token(o).map(|t| format!("authorization: Bearer {t}\r\n")).unwrap_or_default();
    let req = format!(
        "{method} {base}{path} HTTP/1.1\r\nhost: {host}\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{auth}\r\n{payload}",
        payload.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, rest) = text.split_once("\r\n\r\n").ok_or("응답 형식")?;
    let status: u16 = head.split_whitespace().nth(1).and_then(|x| x.parse().ok()).ok_or("상태 줄")?;
    let chunked = head.lines().any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked"));
    let body_text = if chunked { dechunk(rest) } else { rest.to_string() };
    let body = serde_json::from_str(&body_text).unwrap_or(Value::Null);
    Ok(Resp { status, body })
}

fn dechunk(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some((len, after)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(len.trim(), 16).unwrap_or(0);
        if n == 0 || after.len() < n {
            break;
        }
        out.push_str(&after[..n]);
        rest = after[n..].trim_start_matches("\r\n");
    }
    out
}

fn call(o: &Opts, method: &str, path: &str, body: Option<&Value>) -> Value {
    match http(o, method, path, body) {
        Ok(r) if (200..300).contains(&r.status) => r.body,
        Ok(r) => {
            let code = r.body.get("code").map(|c| c.to_string()).unwrap_or_else(|| "-".into());
            let name = r.body.get("name").and_then(Value::as_str).unwrap_or("-");
            eprintln!("[oxadmin] {method} {path} → {} {code} {name}", r.status);
            if o.json {
                println!("{}", r.body);
            }
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("[oxadmin] {method} {path} 실패: {e}");
            std::process::exit(1);
        }
    }
}

fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        None | Some(Value::Null) => "-".into(),
        Some(Value::String(x)) => x.clone(),
        Some(x) => x.to_string(),
    }
}

fn table(head: &[&str], rows: &[Vec<String>]) {
    let mut w: Vec<usize> = head.iter().map(|h| h.chars().count()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            if i < w.len() {
                w[i] = w[i].max(c.chars().count());
            }
        }
    }
    let line = |cells: Vec<String>| {
        let parts: Vec<String> = cells.iter().enumerate().map(|(i, c)| format!("{c:<width$}", width = w.get(i).copied().unwrap_or(0))).collect();
        println!("{}", parts.join("  ").trim_end());
    };
    line(head.iter().map(|h| h.to_string()).collect());
    for r in rows {
        line(r.clone());
    }
}

fn arr<'a>(v: &'a Value, k: &str) -> Vec<&'a Value> {
    v.get(k).and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default()
}

fn out(o: &Opts, v: &Value, human: impl FnOnce(&Value)) {
    if o.json {
        println!("{v}");
    } else {
        human(v);
    }
}

fn kv(v: &Value) {
    if let Some(m) = v.as_object() {
        for (k, x) in m {
            println!("{k}: {}", if let Value::String(s) = x { s.clone() } else { x.to_string() });
        }
    }
}

fn media_rows(v: &Value) -> Vec<(String, String, String, u64, u64)> {
    let mut rows = Vec::new();
    for l in arr(v, "links") {
        let sfu = s(l, "sfu_id");
        for m in arr(l, "mlines") {
            let e = m.get("entry").cloned().unwrap_or(Value::Null);
            let rw = m.get("rewrite").cloned().unwrap_or(Value::Null);
            let last_seq = rw.get("last_seq").and_then(Value::as_u64).unwrap_or(0);
            let last_at = rw.get("last_at").and_then(Value::as_u64).unwrap_or(0);
            rows.push((sfu.clone(), s(&e, "mid"), format!("{} {} {}", s(&e, "kind"), s(&e, "track_id"), s(&e, "room_id")), last_seq, last_at));
        }
    }
    rows
}

fn main() {
    let o = parse();
    let cmd: Vec<&str> = o.rest.iter().map(String::as_str).collect();
    match cmd.as_slice() {
        ["sfus"] => {
            let v = call(&o, "GET", "/admin/sfus", None);
            out(&o, &v, |v| {
                let rows: Vec<Vec<String>> =
                    arr(v, "sfus").iter().map(|x| vec![s(x, "unit"), s(x, "sfu_id"), s(x, "unit_state"), s(x, "live"), s(x, "node_live")]).collect();
                table(&["unit", "sfu_id", "state", "live", "node_live"], &rows);
            });
        }
        ["sfu", id] => {
            let v = call(&o, "GET", &format!("/admin/sfus/{id}/rooms"), None);
            out(&o, &v, |v| {
                println!("unit={} sfu_id={} total={}", s(v, "unit"), s(v, "sfu_id"), s(v, "total"));
                let rows: Vec<Vec<String>> = arr(v, "rooms").iter().map(|x| vec![s(x, "room_id"), s(x, "members")]).collect();
                table(&["room_id", "members"], &rows);
            });
        }
        ["rooms"] => {
            let v = call(&o, "GET", "/admin/rooms", None);
            out(&o, &v, |v| {
                let rows: Vec<Vec<String>> = arr(v, "rooms").iter().map(|x| vec![s(x, "room_id"), s(x, "node"), s(x, "sfu_id"), s(x, "members")]).collect();
                table(&["room_id", "node", "sfu_id", "members"], &rows);
            });
        }
        ["room", id] => {
            let v = call(&o, "GET", &format!("/admin/rooms/{id}/snapshot"), None);
            out(&o, &v, |v| {
                let r = v.get("room").cloned().unwrap_or(Value::Null);
                let ver = r.get("version").cloned().unwrap_or(Value::Null);
                println!(
                    "room={} name={} capacity={} user_count={} hidden_count={} rec={} version={}:{}",
                    s(&r, "room_id"),
                    s(&r, "name"),
                    s(&r, "capacity"),
                    s(&r, "user_count"),
                    s(&r, "hidden_count"),
                    s(&r, "rec"),
                    s(&ver, "epoch"),
                    s(&ver, "seq")
                );
                let rows: Vec<Vec<String>> =
                    arr(&r, "participants").iter().map(|x| vec![s(x, "user_id"), s(x, "role"), s(x, "select"), s(x, "participant_type")]).collect();
                table(&["user_id", "role", "select", "ptype"], &rows);
                let rows: Vec<Vec<String>> = arr(&r, "tracks")
                    .iter()
                    .map(|x| vec![s(x, "track_id"), s(x, "type"), s(x, "kind"), s(x, "user_id"), s(x, "active"), s(x, "muted")])
                    .collect();
                table(&["track_id", "type", "kind", "user_id", "active", "muted"], &rows);
                let f = r.get("floor").cloned().unwrap_or(Value::Null);
                println!(
                    "floor state={} speaker={} priority={} first_rtp_at={} last_rtp_at={}",
                    s(&f, "state"),
                    s(&f, "speaker"),
                    s(&f, "priority"),
                    s(&f, "first_rtp_at"),
                    s(&f, "last_rtp_at")
                );
                for q in arr(&f, "queue") {
                    println!("  queue {} p={}", s(q, "user_id"), s(q, "priority"));
                }
                for t in arr(&f, "timers") {
                    println!("  timer {} user={} at={}", s(t, "name"), s(t, "user_id"), s(t, "at"));
                }
                let sl = r.get("slots").cloned().unwrap_or(Value::Null);
                println!("slots audio={} video={}", s(&sl, "audio"), s(&sl, "video"));
            });
        }
        ["users"] => {
            let v = call(&o, "GET", "/admin/users", None);
            out(&o, &v, |v| {
                let mut rows = Vec::new();
                for u in arr(v, "users") {
                    let links: Vec<String> = arr(u, "links").iter().map(|l| format!("{}:{}:{}", s(l, "sfu_id"), s(l, "ufrag"), s(l, "state"))).collect();
                    rows.push(vec![
                        s(u, "session"),
                        s(u, "user_id"),
                        s(u, "participant_type"),
                        s(u, "hidden"),
                        if links.is_empty() { "-".into() } else { links.join(",") },
                    ]);
                }
                table(&["session", "user_id", "ptype", "hidden", "links"], &rows);
            });
        }
        ["user", id] => {
            let a = call(&o, "GET", &format!("/admin/users/{id}/media"), None);
            std::thread::sleep(Duration::from_millis(o.interval_ms));
            let b = call(&o, "GET", &format!("/admin/users/{id}/media"), None);
            if o.json {
                println!("{}", serde_json::json!([a, b]));
                return;
            }
            let (ra, rb) = (media_rows(&a), media_rows(&b));
            let mut rows = Vec::new();
            for (sfu, mid, what, seq, at) in &rb {
                let prev = ra.iter().find(|x| x.0 == *sfu && x.1 == *mid);
                let (ds, da) = match prev {
                    Some(p) => ((*seq as i64 - p.3 as i64).to_string(), (*at as i64 - p.4 as i64).to_string()),
                    None => ("-".into(), "-".into()),
                };
                rows.push(vec![sfu.clone(), mid.clone(), what.clone(), seq.to_string(), ds, at.to_string(), da]);
            }
            println!("user={id} interval_ms={}", o.interval_ms);
            table(&["sfu_id", "mid", "kind track room", "last_seq", "Δseq", "last_at", "Δat"], &rows);
        }
        ["snapshot"] => {
            let v = call(&o, "GET", "/admin/snapshot", None);
            out(&o, &v, kv);
        }
        ["drops"] => {
            let v = call(&o, "GET", "/admin/drops", None);
            out(&o, &v, |v| {
                for u in arr(v, "units") {
                    println!("[{}] sfu_id={} error={}", s(u, "unit"), s(u, "sfu_id"), s(u, "error"));
                    if let Some(m) = u.as_object() {
                        for (k, x) in m {
                            if !["unit", "sfu_id", "error"].contains(&k.as_str()) && x.as_u64().is_some_and(|n| n > 0) {
                                println!("  {k}={x}");
                            }
                        }
                    }
                }
            });
        }
        ["bus"] => {
            let v = call(&o, "GET", "/admin/bus", None);
            out(&o, &v, kv);
        }
        ["bus", "--all"] => {
            let v = call(&o, "GET", "/admin/bus?all=1", None);
            out(&o, &v, |v| {
                let mut views: Vec<(String, String)> = Vec::new();
                let mut rows = Vec::new();
                for n in arr(v, "nodes") {
                    let b = n.get("bus").cloned().unwrap_or(Value::Null);
                    let mut nodes: Vec<String> = arr(&b, "nodes").iter().map(|x| format!("{}/{}", s(x, "node"), s(x, "inst"))).collect();
                    nodes.sort();
                    rows.push(vec![s(n, "node"), s(&b, "inst"), nodes.join(","), arr(&b, "rooms_seen").len().to_string(), s(n, "error")]);
                    if n.get("error").is_none() {
                        views.push((s(n, "node"), nodes.join(",")));
                    }
                }
                table(&["node", "inst", "nodes_seen", "rooms_seen", "error"], &rows);
                let same = views.windows(2).all(|w| w[0].1 == w[1].1) && views.len() == arr(v, "nodes").len();
                if same {
                    println!("합의: 전 node 가 같은 것을 본다");
                } else {
                    println!("불일치: node 마다 본 것이 다르다");
                }
            });
        }
        ["unit", "list"] => {
            let v = call(&o, "GET", "/admin/supervisor/status", None);
            out(&o, &v, |v| {
                let rows: Vec<Vec<String>> = arr(v, "units")
                    .iter()
                    .map(|x| vec![s(x, "id"), s(x, "kind"), s(x, "order"), s(x, "state"), s(x, "restarts"), s(x, "epoch"), s(x, "next_retry_ms")])
                    .collect();
                table(&["id", "kind", "order", "state", "restarts", "epoch", "next_retry_ms"], &rows);
            });
        }
        ["unit", "show", id] => {
            let v = call(&o, "GET", "/admin/supervisor/status", None);
            let u = arr(&v, "units").into_iter().find(|x| s(x, "id") == *id).cloned().unwrap_or(Value::Null);
            out(&o, &u, kv);
        }
        [op @ ("load" | "stop"), id] => {
            let v = call(&o, "POST", &format!("/admin/supervisor/{op}/{id}"), None);
            out(&o, &v, |v| println!("{} {} → {}", s(v, "id"), s(v, "from"), s(v, "to")));
        }
        ["kill", id] => {
            let st = call(&o, "GET", "/admin/supervisor/status", None);
            let epoch = arr(&st, "units").into_iter().find(|x| s(x, "id") == *id).map(|x| s(x, "epoch")).unwrap_or_else(|| "-".into());
            eprintln!("if_epoch={epoch}");
            let v = call(&o, "POST", &format!("/admin/supervisor/kill/{id}?if_epoch={epoch}"), None);
            out(&o, &v, |v| println!("{} {} → {}", s(v, "id"), s(v, "from"), s(v, "to")));
        }
        ["shutdown"] => {
            let snap = call(&o, "GET", "/admin/snapshot", None);
            let hub = s(&snap, "hub_id");
            eprintln!("confirm={hub}");
            let v = call(&o, "POST", &format!("/admin/supervisor/shutdown?confirm={hub}"), None);
            out(&o, &v, kv);
        }
        ["probe", user] => {
            let v = call(&o, "GET", &format!("/admin/users/{user}/probe"), None);
            out(&o, &v, |v| {
                for x in arr(v, "sessions") {
                    let r = if x.get("timeout").and_then(Value::as_bool) == Some(true) {
                        "timeout".to_string()
                    } else {
                        x.get("result").map(Value::to_string).unwrap_or_else(|| "-".into())
                    };
                    println!("{} {}", s(x, "session"), r);
                }
            });
        }
        ["user-cut", user] => {
            let v = call(&o, "POST", &format!("/admin/users/{user}/cut"), None);
            out(&o, &v, |v| {
                for c in arr(v, "cut") {
                    println!("cut session={} code={}", s(c, "session"), s(c, "code"));
                }
                println!("total={}", s(v, "total"));
            });
        }
        ["reap", room, user] => {
            let v = call(&o, "POST", &format!("/admin/rooms/{room}/reap/{user}"), None);
            out(&o, &v, |v| {
                let ver = v.get("version").cloned().unwrap_or(Value::Null);
                println!("reaped room={} user={} left={} version={}:{}", s(v, "room_id"), s(v, "user_id"), s(v, "left"), s(&ver, "epoch"), s(&ver, "seq"));
            });
        }
        ["destroy", room] => {
            let snap = call(&o, "GET", &format!("/admin/rooms/{room}/snapshot"), None);
            let ver = snap.get("room").and_then(|r| r.get("version")).cloned().unwrap_or(Value::Null);
            let iv = format!("{}:{}", s(&ver, "epoch"), s(&ver, "seq"));
            eprintln!("if_version={iv}");
            let v = call(&o, "POST", &format!("/admin/rooms/{room}/destroy?if_version={iv}"), None);
            out(&o, &v, |v| {
                let d = if v.get("destroyed").and_then(Value::as_bool) == Some(true) { "yes" } else { "no" };
                println!("destroyed={d} room={} notified={}", s(v, "room_id"), s(v, "notified"));
            });
        }
        ["log-level", level] => {
            let mut b = serde_json::json!({ "level": level });
            if let Some(t) = o.ttl {
                b["ttl_secs"] = t.into();
            }
            let v = call(&o, "POST", "/admin/log-level", Some(&b));
            out(&o, &v, |v| println!("log-level {} → {} until={} units={}", s(v, "from"), s(v, "to"), s(v, "until"), s(v, "units")));
        }
        _ => usage(),
    }
}
