// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 운영 규격서 §1 §3 §4 §5 §6 · 정:RULE-OPS-TOKEN · model: claude-opus-5-5
//! oxadmin — 운영 도구 하나. §6 표의 경로만 부른다 · 붙은 곳을 매번 stderr 첫 줄에 적는다 · 확인값은 먼저 읽어 되싣는다.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};

const DEFAULT_HUB: &str = "http://127.0.0.1:1974";

struct Opts {
    hub: String,
    from: &'static str,
    api_key: Option<String>,
    api_secret: Option<String>,
    json: bool,
    all: bool,
    ttl: Option<u64>,
    rest: Vec<String>,
}

fn usage() -> ! {
    eprintln!(
        "oxadmin [--hub URL] [--api-key K --api-secret S] [--json] <명령>\n  sfus | sfu <id> | rooms | room <id> | users | snapshot | drops | bus [--all]\n  unit list | unit show <id> | load <id> | stop <id> | kill <id> | shutdown\n  user-cut <user> | reap <room> <user> | destroy <room> | log-level <level> [--ttl <secs>]"
    );
    std::process::exit(2);
}

fn parse() -> Opts {
    let mut o = Opts { hub: String::new(), from: "default", api_key: None, api_secret: None, json: false, all: false, ttl: None, rest: vec![] };
    let mut hub_arg: Option<String> = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--hub" => hub_arg = it.next(),
            "--api-key" => o.api_key = it.next(),
            "--api-secret" => o.api_secret = it.next(),
            "--json" => o.json = true,
            "--all" => o.all = true,
            "--ttl" => o.ttl = it.next().and_then(|v| v.parse().ok()),
            "-h" | "--help" => usage(),
            _ => o.rest.push(a),
        }
    }
    match (hub_arg, std::env::var("OXADMIN_URL").ok().filter(|s| !s.is_empty())) {
        (Some(h), _) => {
            o.hub = h;
            o.from = "--hub";
        }
        (None, Some(e)) => {
            o.hub = e;
            o.from = "env OXADMIN_URL";
        }
        _ => o.hub = DEFAULT_HUB.to_string(),
    }
    o
}

fn token(o: &Opts) -> Option<String> {
    let (k, s) = (o.api_key.as_ref()?, o.api_secret.as_ref()?);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let claims = json!({ "iss": k, "ops": true, "iat": now, "exp": now + 300 });
    jsonwebtoken::encode(&jsonwebtoken::Header::default(), &claims, &jsonwebtoken::EncodingKey::from_secret(s.as_bytes())).ok()
}

fn split_url(u: &str) -> Result<(String, String), String> {
    let rest = u.strip_prefix("http://").ok_or_else(|| format!("http:// 주소만 받는다: {u}"))?;
    let (host, base) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].trim_end_matches('/')),
        None => (rest, ""),
    };
    Ok((host.to_string(), base.to_string()))
}

fn request(o: &Opts, method: &str, path: &str, body: Option<&Value>) -> Result<(u16, Value), String> {
    let (host, base) = split_url(&o.hub)?;
    let mut s = TcpStream::connect(&host).map_err(|e| format!("{host}: {e}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(15)));
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = format!("{method} {base}{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: application/json\r\n");
    if let Some(t) = token(o) {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    if body.is_some() {
        req.push_str("Content-Type: application/json\r\n");
    }
    req.push_str(&format!("Content-Length: {}\r\n\r\n{payload}", payload.len()));
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, rest) = text.split_once("\r\n\r\n").ok_or("응답 머리가 없다")?;
    let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("상태 줄이 없다")?;
    let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
    let body = if chunked { dechunk(rest) } else { rest.to_string() };
    let v = if body.trim().is_empty() { Value::Null } else { serde_json::from_str(&body).unwrap_or(Value::String(body)) };
    Ok((status, v))
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

fn cell(v: &Value) -> String {
    match v {
        Value::Null => "-".into(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(cell).collect::<Vec<_>>().join(","),
        other => other.to_string(),
    }
}

fn table(rows: &[Value], cols: &[&str]) {
    let cells: Vec<Vec<String>> = rows.iter().map(|r| cols.iter().map(|c| r.get(*c).map(cell).unwrap_or_else(|| "-".into())).collect()).collect();
    let widths: Vec<usize> = cols.iter().enumerate().map(|(i, c)| cells.iter().map(|r| r[i].chars().count()).max().unwrap_or(0).max(c.len())).collect();
    let line = |vals: Vec<String>| vals.iter().enumerate().map(|(i, v)| format!("{v:<w$}", w = widths[i])).collect::<Vec<_>>().join("  ");
    println!("{}", line(cols.iter().map(|c| c.to_string()).collect()));
    for r in cells {
        println!("{}", line(r));
    }
}

fn show(o: &Opts, v: &Value, list: Option<(&str, &[&str])>) {
    if o.json {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
        return;
    }
    match list.and_then(|(k, cols)| v.get(k).and_then(Value::as_array).map(|a| (a, cols))) {
        Some((rows, cols)) => {
            table(rows, cols);
            for (k, x) in v.as_object().into_iter().flatten() {
                if !x.is_array() {
                    println!("{k}: {}", cell(x));
                }
            }
        }
        None => println!("{}", serde_json::to_string_pretty(v).unwrap_or_default()),
    }
}

fn arg(o: &Opts, i: usize) -> String {
    o.rest.get(i).cloned().unwrap_or_else(|| usage())
}

fn call(o: &Opts, method: &str, path: &str, body: Option<&Value>) -> Value {
    match request(o, method, path, body) {
        Ok((s, v)) if (200..300).contains(&s) => v,
        Ok((s, v)) => {
            eprintln!("oxadmin: HTTP {s} {}", cell(&v));
            if o.json {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("oxadmin: {e}");
            std::process::exit(1);
        }
    }
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn main() {
    let o = parse();
    eprintln!("oxadmin: hub={} ({})", o.hub, o.from);
    let cmd = o.rest.first().cloned().unwrap_or_else(|| usage());
    match cmd.as_str() {
        "sfus" => {
            let v = call(&o, "GET", "/admin/sfus", None);
            show(&o, &v, Some(("sfus", &["unit", "sfu_id", "addr", "unit_state", "live", "node_live"])));
        }
        "sfu" => {
            let v = call(&o, "GET", &format!("/admin/sfus/{}/rooms", enc(&arg(&o, 1))), None);
            show(&o, &v, Some(("rooms", &["room_id", "members"])));
        }
        "rooms" => {
            let v = call(&o, "GET", "/admin/rooms", None);
            show(&o, &v, Some(("rooms", &["room_id", "node", "sfu_id", "members", "member_ids"])));
        }
        "room" => {
            let v = call(&o, "GET", &format!("/admin/rooms/{}/snapshot", enc(&arg(&o, 1))), None);
            show(&o, &v, None);
        }
        "users" => {
            let v = call(&o, "GET", "/admin/users", None);
            if o.json {
                show(&o, &v, None);
            } else {
                let rows: Vec<Value> = v
                    .get("users")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|u| {
                        let links: Vec<String> = u
                            .get("links")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .map(|l| format!("{}/{}", cell(l.get("ufrag").unwrap_or(&Value::Null)), cell(l.get("state").unwrap_or(&Value::Null))))
                            .collect();
                        json!({ "session_id": u.get("session_id"), "user_id": u.get("user_id"), "participant_type": u.get("participant_type"), "hidden": u.get("hidden"), "links": links })
                    })
                    .collect();
                table(&rows, &["user_id", "session_id", "participant_type", "hidden", "links"]);
                println!("total: {}", cell(v.get("total").unwrap_or(&Value::Null)));
            }
        }
        "snapshot" => show(&o, &call(&o, "GET", "/admin/snapshot", None), None),
        "drops" => show(&o, &call(&o, "GET", "/admin/drops", None), None),
        "bus" => {
            let path = if o.all { "/admin/bus?all=1" } else { "/admin/bus" };
            show(&o, &call(&o, "GET", path, None), None);
        }
        "unit" => {
            let v = call(&o, "GET", "/admin/supervisor/status", None);
            match o.rest.get(1).map(String::as_str) {
                Some("show") => {
                    let id = arg(&o, 2);
                    let u = v.get("units").and_then(Value::as_array).and_then(|a| a.iter().find(|u| u.get("id").and_then(Value::as_str) == Some(id.as_str())).cloned());
                    match u {
                        Some(u) => show(&o, &u, None),
                        None => {
                            eprintln!("oxadmin: 유닛 없음: {id}");
                            std::process::exit(1);
                        }
                    }
                }
                _ => show(&o, &v, Some(("units", &["id", "kind", "order", "state", "restarts", "next_retry_ms", "epoch"]))),
            }
        }
        "load" | "stop" => {
            let v = call(&o, "POST", &format!("/admin/supervisor/{cmd}/{}", enc(&arg(&o, 1))), None);
            show(&o, &v, None);
        }
        "kill" => {
            let id = arg(&o, 1);
            let st = call(&o, "GET", "/admin/supervisor/status", None);
            let epoch = st
                .get("units")
                .and_then(Value::as_array)
                .and_then(|a| a.iter().find(|u| u.get("id").and_then(Value::as_str) == Some(id.as_str())))
                .and_then(|u| u.get("epoch").and_then(Value::as_str).map(str::to_string));
            let Some(epoch) = epoch else {
                eprintln!("oxadmin: {id} 의 epoch 를 읽지 못했다(Running 이 아니다)");
                std::process::exit(1);
            };
            let v = call(&o, "POST", &format!("/admin/supervisor/kill/{}?if_epoch={}", enc(&id), enc(&epoch)), None);
            show(&o, &v, None);
        }
        "shutdown" => {
            let snap = call(&o, "GET", "/admin/snapshot", None);
            let Some(hub_id) = snap.get("hub_id").and_then(Value::as_str) else {
                eprintln!("oxadmin: hub_id 를 읽지 못했다");
                std::process::exit(1);
            };
            let v = call(&o, "POST", &format!("/admin/supervisor/shutdown?confirm={}", enc(hub_id)), None);
            show(&o, &v, None);
        }
        "user-cut" => {
            let v = call(&o, "POST", &format!("/admin/users/{}/cut", enc(&arg(&o, 1))), None);
            show(&o, &v, Some(("cut", &["session_id", "code"])));
        }
        "reap" => {
            let v = call(&o, "POST", &format!("/admin/rooms/{}/reap/{}", enc(&arg(&o, 1)), enc(&arg(&o, 2))), None);
            show(&o, &v, None);
        }
        "destroy" => {
            let room = arg(&o, 1);
            let snap = call(&o, "GET", &format!("/admin/rooms/{}/snapshot", enc(&room)), None);
            let ver = snap.pointer("/room/version").and_then(|v| Some(format!("{}:{}", v.get("epoch")?.as_str()?, v.get("seq")?.as_u64()?)));
            let Some(ver) = ver else {
                eprintln!("oxadmin: {room} 의 version 을 읽지 못했다");
                std::process::exit(1);
            };
            let v = call(&o, "POST", &format!("/admin/rooms/{}/destroy?if_version={}", enc(&room), enc(&ver)), None);
            show(&o, &v, None);
        }
        "log-level" => {
            let level = arg(&o, 1);
            let body = json!({ "level": level, "ttl_secs": o.ttl });
            show(&o, &call(&o, "POST", "/admin/log-level", Some(&body)), None);
        }
        _ => usage(),
    }
}
