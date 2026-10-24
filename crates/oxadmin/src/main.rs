// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-SURFACE · 정:RULE-OPS-TOKEN · 정:CAND-OPS-ADMIN-OUTPUT · 운영 규격서 §1 · 운영 규격서 §3 · 운영 규격서 §4 · 운영 규격서 §5 · 운영 규격서 §6 · model: claude-opus-5-5
//! oxadmin — 운영 도구 하나. C 평면 경로표를 넘지 않는다 · 붙은 곳을 매번 stderr 첫 줄에 · 확인값은 먼저 읽어 되싣는다 · 서버가 안 낸 값은 `-`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::Value;

pub const DEFAULT_HUB: &str = "http://127.0.0.1:1974";
pub const ENV_URL: &str = "OXADMIN_URL";

struct Opts {
    hub: String,
    source: &'static str,
    json: bool,
    key: Option<String>,
    secret: Option<String>,
    ttl: Option<u64>,
    all: bool,
    rest: Vec<String>,
}

fn parse(argv: &[String]) -> Result<Opts, String> {
    let mut o = Opts { hub: String::new(), source: "기본값", json: false, key: None, secret: None, ttl: None, all: false, rest: Vec::new() };
    let mut hub_arg = None;
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        let mut val = || {
            i += 1;
            argv.get(i).cloned().ok_or_else(|| format!("{a} 에 값이 없다"))
        };
        match a {
            "--json" => o.json = true,
            "--all" => o.all = true,
            "--hub" => hub_arg = Some(val()?),
            "--api-key" => o.key = Some(val()?),
            "--api-secret" => o.secret = Some(val()?),
            "--ttl" => o.ttl = Some(val()?.parse().map_err(|_| "--ttl 은 초")?),
            _ if a.starts_with("--") => return Err(format!("모르는 인자 {a}")),
            _ => o.rest.push(a.to_string()),
        }
        i += 1;
    }
    match (hub_arg, std::env::var(ENV_URL).ok().filter(|v| !v.is_empty())) {
        (Some(h), _) => {
            o.hub = h;
            o.source = "--hub";
        }
        (None, Some(e)) => {
            o.hub = e;
            o.source = ENV_URL;
        }
        (None, None) => o.hub = DEFAULT_HUB.to_string(),
    }
    o.hub = o.hub.trim_end_matches('/').to_string();
    Ok(o)
}

fn token(o: &Opts) -> Option<String> {
    let (k, s) = (o.key.as_ref()?, o.secret.as_ref()?);
    let now = common::token::now_s();
    common::token::sign(&common::token::OpsClaims { iss: k.clone(), ops: true, sub: None, iat: now, exp: now + 300 }, s)
}

fn http(o: &Opts, method: &str, path: &str, body: Option<&Value>) -> Result<(u16, Value), String> {
    let base = o.hub.strip_prefix("http://").ok_or("hub 주소는 http:// 로 시작해야 한다")?;
    let (host, prefix) = match base.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (base, String::new()),
    };
    let mut s = TcpStream::connect(host).map_err(|e| format!("hub {host} 에 못 붙는다: {e}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let auth = token(o).map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    let req = format!("{method} {prefix}{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{auth}\r\n{payload}", payload.len());
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let code: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { dechunk(rest) } else { rest.to_string() };
    let v = serde_json::from_str(&body).unwrap_or(Value::String(body));
    Ok((code, v))
}

fn dechunk(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some((len, tail)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(len.trim(), 16).unwrap_or(0);
        if n == 0 || tail.len() < n {
            break;
        }
        out.push_str(&tail[..n]);
        rest = tail[n..].trim_start_matches("\r\n");
    }
    out
}

fn s(v: &Value) -> String {
    match v {
        Value::Null => "-".into(),
        Value::String(x) => x.clone(),
        x => x.to_string(),
    }
}

fn kv(v: &Value, keys: &[&str]) -> String {
    keys.iter().map(|k| format!("{k}={}", s(&v[*k]))).collect::<Vec<_>>().join(" ")
}

fn show(o: &Opts, code: u16, v: &Value, rows: Option<(&str, &[&str])>) -> i32 {
    if o.json {
        println!("{v}");
    } else if let Some((field, keys)) = rows
        && let Some(list) = v[field].as_array()
    {
        for r in list {
            println!("{}", kv(r, keys));
        }
        if let Some(t) = v.get("total") {
            println!("total={}", s(t));
        }
    } else if let Some(m) = v.as_object() {
        for (k, x) in m {
            println!("{k}={}", s(x));
        }
    } else {
        println!("{}", s(v));
    }
    if (200..300).contains(&code) { 0 } else { 1 }
}

fn arg(o: &Opts, i: usize, what: &str) -> Result<String, String> {
    o.rest.get(i).cloned().ok_or_else(|| format!("{what} 가 없다"))
}

fn run(o: &Opts) -> Result<i32, String> {
    let cmd = o.rest.first().map(String::as_str).unwrap_or("snapshot");
    let r = match cmd {
        "sfus" => {
            let (c, v) = http(o, "GET", "/admin/sfus", None)?;
            show(o, c, &v, Some(("sfus", &["unit", "sfu_id", "unit_state", "live", "node_live", "addr"])))
        }
        "sfu" => {
            let (c, v) = http(o, "GET", &format!("/admin/sfus/{}/rooms", arg(o, 1, "sfu_id")?), None)?;
            show(o, c, &v, Some(("rooms", &["room_id", "members"])))
        }
        "rooms" => {
            let (c, v) = http(o, "GET", "/admin/rooms", None)?;
            show(o, c, &v, Some(("rooms", &["room_id", "node", "sfu_id", "members"])))
        }
        "room" => {
            let (c, v) = http(o, "GET", &format!("/admin/rooms/{}/snapshot", arg(o, 1, "room_id")?), None)?;
            if o.json || !(200..300).contains(&c) {
                show(o, c, &v, None)
            } else {
                let r = &v["room"];
                println!("{}", kv(r, &["room_id", "name", "capacity", "user_count", "hidden_count", "rec"]));
                println!("version={}:{}", s(&r["version"]["epoch"]), s(&r["version"]["seq"]));
                for p in r["participants"].as_array().cloned().unwrap_or_default() {
                    println!("participant {}", kv(&p, &["user_id", "participant_type", "select"]));
                }
                for t in r["tracks"].as_array().cloned().unwrap_or_default() {
                    println!("track {}", kv(&t, &["type", "track_id", "kind", "user_id", "active", "muted"]));
                }
                println!("floor {}", kv(&r["floor"], &["state", "speaker", "last_rtp_at"]));
                0
            }
        }
        "users" => {
            let (c, v) = http(o, "GET", "/admin/users", None)?;
            show(o, c, &v, Some(("users", &["session", "user_id", "participant_type", "hidden"])))
        }
        "user" => {
            let u = arg(o, 1, "user_id")?;
            let (c1, a) = http(o, "GET", &format!("/admin/users/{u}/media"), None)?;
            std::thread::sleep(Duration::from_secs(1));
            let (c2, b) = http(o, "GET", &format!("/admin/users/{u}/media"), None)?;
            if o.json {
                println!("{}", serde_json::json!({"first": a, "second": b}));
            } else {
                for (la, lb) in a["links"].as_array().cloned().unwrap_or_default().iter().zip(b["links"].as_array().cloned().unwrap_or_default().iter()) {
                    println!("link {}", kv(lb, &["sfu_id", "ufrag", "arev", "acked", "error"]));
                    let ma = la["mlines"].as_array().cloned().unwrap_or_default();
                    for (i, m) in lb["mlines"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                        let before = ma.get(i).map(|x| x["rewrite"]["last_seq"].clone()).unwrap_or(Value::Null);
                        let after = m["rewrite"]["last_seq"].clone();
                        let moved = if before.is_null() || after.is_null() { "-".to_string() } else { (before != after).to_string() };
                        println!("  mid={} track={} ssrc={} last_seq={} moved={}", s(&m["entry"]["mid"]), s(&m["entry"]["track_id"]), s(&m["entry"]["ssrc"]), s(&after), moved);
                    }
                }
            }
            if (200..300).contains(&c1) && (200..300).contains(&c2) { 0 } else { 1 }
        }
        "snapshot" => {
            let (c, v) = http(o, "GET", "/admin/snapshot", None)?;
            show(o, c, &v, None)
        }
        "drops" => {
            let (c, v) = http(o, "GET", "/admin/drops", None)?;
            if o.json {
                show(o, c, &v, None)
            } else {
                for u in v["units"].as_array().cloned().unwrap_or_default() {
                    let line: Vec<String> = u.as_object().map(|m| m.iter().map(|(k, x)| format!("{k}={}", s(x))).collect()).unwrap_or_default();
                    println!("{}", line.join(" "));
                }
                0
            }
        }
        "bus" => {
            let path = if o.all { "/admin/bus?all=1" } else { "/admin/bus" };
            let (c, v) = http(o, "GET", path, None)?;
            if o.json || !o.all {
                show(o, c, &v, None)
            } else {
                let mut views = Vec::new();
                for n in v["nodes"].as_array().cloned().unwrap_or_default() {
                    if n.get("error").is_some() {
                        println!("node={} error={}", s(&n["node"]), s(&n["error"]));
                        continue;
                    }
                    let mut seen: Vec<String> = n["bus"]["nodes"].as_array().cloned().unwrap_or_default().iter().map(|x| format!("{}/{}", s(&x["node"]), s(&x["inst"]))).collect();
                    seen.sort();
                    println!("node={} sees={}", s(&n["node"]), seen.join(","));
                    views.push(seen);
                }
                if views.windows(2).all(|w| w[0] == w[1]) {
                    println!("합의: 전 node 가 같은 것을 본다");
                } else {
                    println!("불일치: node 마다 보는 것이 다르다");
                }
                0
            }
        }
        "unit" => match o.rest.get(1).map(String::as_str) {
            Some("show") => {
                let id = arg(o, 2, "unit id")?;
                let (c, v) = http(o, "GET", "/admin/supervisor/status", None)?;
                let u = v["units"].as_array().and_then(|l| l.iter().find(|x| x["id"] == Value::String(id.clone())).cloned()).unwrap_or(Value::Null);
                show(o, c, &u, None)
            }
            _ => {
                let (c, v) = http(o, "GET", "/admin/supervisor/status", None)?;
                show(o, c, &v, Some(("units", &["id", "kind", "order", "state", "restarts", "next_retry_ms", "epoch"])))
            }
        },
        "load" | "stop" => {
            let (c, v) = http(o, "POST", &format!("/admin/supervisor/{cmd}/{}", arg(o, 1, "unit id")?), None)?;
            show(o, c, &v, None)
        }
        "kill" => {
            let id = arg(o, 1, "unit id")?;
            let (_, st) = http(o, "GET", "/admin/supervisor/status", None)?;
            let epoch = st["units"].as_array().and_then(|l| l.iter().find(|x| x["id"] == Value::String(id.clone())).map(|x| s(&x["epoch"]))).unwrap_or_else(|| "-".into());
            let _ = writeln!(std::io::stderr(), "if_epoch={epoch}");
            let (c, v) = http(o, "POST", &format!("/admin/supervisor/kill/{id}?if_epoch={epoch}"), None)?;
            show(o, c, &v, None)
        }
        "shutdown" => {
            let (_, snap) = http(o, "GET", "/admin/snapshot", None)?;
            let hub = s(&snap["hub_id"]);
            let _ = writeln!(std::io::stderr(), "confirm={hub}");
            let (c, v) = http(o, "POST", &format!("/admin/supervisor/shutdown?confirm={hub}"), None)?;
            show(o, c, &v, None)
        }
        "user-cut" => {
            let (c, v) = http(o, "POST", &format!("/admin/users/{}/cut", arg(o, 1, "user_id")?), None)?;
            show(o, c, &v, Some(("cut", &["session", "code"])))
        }
        "reap" => {
            let (c, v) = http(o, "POST", &format!("/admin/rooms/{}/reap/{}", arg(o, 1, "room_id")?, arg(o, 2, "user_id")?), None)?;
            show(o, c, &v, None)
        }
        "destroy" => {
            let room = arg(o, 1, "room_id")?;
            let (c0, snap) = http(o, "GET", &format!("/admin/rooms/{room}/snapshot"), None)?;
            if !(200..300).contains(&c0) {
                return Ok(show(o, c0, &snap, None));
            }
            let ver = format!("{}:{}", s(&snap["room"]["version"]["epoch"]), s(&snap["room"]["version"]["seq"]));
            let _ = writeln!(std::io::stderr(), "if_version={ver}");
            let (c, v) = http(o, "POST", &format!("/admin/rooms/{room}/destroy?if_version={ver}"), None)?;
            if o.json || !(200..300).contains(&c) {
                show(o, c, &v, None)
            } else {
                println!("room_id={} destroyed={} notified={}", s(&v["room_id"]), if v["destroyed"] == Value::Bool(true) { "yes" } else { "no" }, s(&v["notified"]));
                0
            }
        }
        "log-level" => {
            let level = arg(o, 1, "level")?;
            let mut body = serde_json::json!({"level": level});
            if let Some(t) = o.ttl {
                body["ttl_secs"] = Value::from(t);
            }
            let (c, v) = http(o, "POST", "/admin/log-level", Some(&body))?;
            show(o, c, &v, None)
        }
        other => return Err(format!("모르는 명령 {other}")),
    };
    Ok(r)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let o = match parse(&argv) {
        Ok(o) => o,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "oxadmin: {e}");
            std::process::exit(2);
        }
    };
    let _ = writeln!(std::io::stderr(), "[oxadmin] hub={} ({})", o.hub, o.source);
    match run(&o) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "oxadmin: {e}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_ladder_and_dechunk() {
        let o = parse(&["--hub".into(), "http://h:1/".into(), "rooms".into()]).expect("ok");
        assert_eq!((o.hub.as_str(), o.source), ("http://h:1", "--hub"));
        assert!(parse(&["--nope".into()]).is_err());
        assert_eq!(dechunk("3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"), "abcde");
        assert_eq!(s(&Value::Null), "-", "서버가 안 낸 값은 지어 채우지 않는다");
    }
}
