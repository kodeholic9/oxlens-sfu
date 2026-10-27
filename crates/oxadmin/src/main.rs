// author: kodeholic (powered by Claude)
// spec: v1.4 · 운영:RULE-TOOL-ONE · 운영:RULE-TOOL-CONTRACT · 운영:RULE-AUTH-THREE · 운영:RULE-RISK-CONFIRM · 운영:RULE-READ-USER-MEDIA · 운영:RULE-READ-BUS-ALL · 운영:RULE-FMT-NO-INVENT · 정:CAND-OPS-TOOL-HUB-FORM · 정:CAND-OPS-ADMIN-OUTPUT · 정:RULE-OPS-TOKEN · model: claude-opus-5-5
//! 운영 도구 `oxadmin` — 하나뿐인 도구 · 명령은 표를 넘지 않는다. 붙은 곳을 매번 stderr 첫 줄에 적는다(`--hub` > `OXADMIN_URL` > 기본값 · `host:port` 도 받는다) · 원격이면 그 자리에서 운영 토큰을 서명한다 · 확인값은 먼저 읽어 되싣는다 · 서버가 안 내는 값은 `-`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const DEFAULT_HUB: &str = "http://127.0.0.1:1974";
const TOKEN_TTL_SECS: u64 = 60;

struct Opts {
    hub: String,
    source: &'static str,
    json: bool,
    api_key: Option<String>,
    api_secret: Option<String>,
    ttl: Option<u64>,
    all: bool,
    rest: Vec<String>,
}

fn parse_args(argv: Vec<String>) -> Result<Opts, String> {
    let mut hub: Option<String> = None;
    let mut o = Opts { hub: String::new(), source: "기본값", json: false, api_key: None, api_secret: None, ttl: None, all: false, rest: Vec::new() };
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        let mut val = || -> Result<String, String> {
            i += 1;
            argv.get(i).cloned().ok_or(format!("{a} 뒤에 값이 없다"))
        };
        match a {
            "--hub" => hub = Some(val()?),
            "--json" => o.json = true,
            "--all" => o.all = true,
            "--api-key" => o.api_key = Some(val()?),
            "--api-secret" => o.api_secret = Some(val()?),
            "--ttl" => o.ttl = Some(val()?.parse().map_err(|_| "--ttl 은 초(정수)".to_string())?),
            _ => o.rest.push(argv[i].clone()),
        }
        i += 1;
    }
    let (h, src) = match (hub, std::env::var("OXADMIN_URL").ok().filter(|v| !v.is_empty())) {
        (Some(h), _) => (h, "--hub"),
        (None, Some(e)) => (e, "OXADMIN_URL"),
        (None, None) => (DEFAULT_HUB.to_string(), "기본값"),
    };
    o.hub = normalize(&h);
    o.source = src;
    Ok(o)
}

fn normalize(h: &str) -> String {
    let h = h.trim_end_matches('/');
    if h.contains("://") { h.to_string() } else { format!("http://{h}") }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn token(o: &Opts) -> Option<String> {
    let (k, s) = (o.api_key.as_ref()?, o.api_secret.as_ref()?);
    let now = now_secs();
    let claims = json!({"iss": k, "ops": true, "iat": now, "exp": now + TOKEN_TTL_SECS});
    jsonwebtoken::encode(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256), &claims, &jsonwebtoken::EncodingKey::from_secret(s.as_bytes())).ok()
}

struct Resp {
    status: u16,
    body: Value,
}

fn request(o: &Opts, method: &str, path: &str, body: Option<&Value>) -> Result<Resp, String> {
    let rest = o.hub.strip_prefix("http://").ok_or_else(|| format!("스킴을 지원하지 않는다: {} (http 만)", o.hub))?;
    let (authority, base) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let base = base.trim_end_matches("/media");
    let mut s = TcpStream::connect(authority).map_err(|e| format!("{authority} 에 못 붙었다: {e}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(15)));
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = format!("{method} {base}{path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nContent-Length: {}\r\n", payload.len());
    if body.is_some() {
        req.push_str("Content-Type: application/json\r\n");
    }
    if let Some(t) = token(o) {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(&payload);
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { dechunk(body) } else { body.to_string() };
    Ok(Resp { status, body: serde_json::from_str(&body).unwrap_or(Value::String(body)) })
}

fn dechunk(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some((size, tail)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
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
        Value::Null => "-".to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn table(rows: &[Value], cols: &[&str]) -> String {
    let mut grid: Vec<Vec<String>> = vec![cols.iter().map(|c| c.to_string()).collect()];
    for r in rows {
        grid.push(cols.iter().map(|c| cell(r.get(*c).unwrap_or(&Value::Null))).collect());
    }
    let widths: Vec<usize> = (0..cols.len()).map(|i| grid.iter().map(|r| r[i].chars().count()).max().unwrap_or(0)).collect();
    grid.iter().map(|r| r.iter().enumerate().map(|(i, c)| format!("{c:<w$}", w = widths[i])).collect::<Vec<_>>().join("  ").trim_end().to_string()).collect::<Vec<_>>().join("\n")
}

fn kv(v: &Value) -> String {
    match v {
        Value::Object(m) => m.iter().map(|(k, x)| format!("{k}={}", cell(x))).collect::<Vec<_>>().join("\n"),
        other => cell(other),
    }
}

fn show(o: &Opts, r: &Resp, render: impl Fn(&Value) -> String) -> i32 {
    if o.json {
        println!("{}", serde_json::to_string_pretty(&r.body).unwrap_or_default());
    } else if r.status >= 400 {
        println!("실패 status={} {}", r.status, kv(&r.body));
    } else {
        println!("{}", render(&r.body));
    }
    if r.status >= 400 { 1 } else { 0 }
}

fn arr(v: &Value, k: &str) -> Vec<Value> {
    v.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn user_media(o: &Opts, user: &str) -> Result<i32, String> {
    let path = format!("/admin/users/{user}/media");
    let a = request(o, "GET", &path, None)?;
    std::thread::sleep(Duration::from_secs(1));
    let b = request(o, "GET", &path, None)?;
    if o.json {
        println!("{}", serde_json::to_string_pretty(&json!({"first": a.body, "second": b.body})).unwrap_or_default());
        return Ok(if b.status >= 400 { 1 } else { 0 });
    }
    if b.status >= 400 {
        println!("실패 status={} {}", b.status, kv(&b.body));
        return Ok(1);
    }
    let mut rows = Vec::new();
    for (li, link) in arr(&b.body, "links").iter().enumerate() {
        let before = arr(&a.body, "links").get(li).cloned().unwrap_or(Value::Null);
        if let Some(e) = link.get("error") {
            rows.push(json!({"sfu_id": link.get("sfu_id"), "mid": "-", "track_id": "-", "last_seq": "-", "Δseq": "-", "Δat_ms": "-", "error": e}));
            continue;
        }
        for (mi, m) in arr(link, "mlines").iter().enumerate() {
            let p = before.get("mlines").and_then(|x| x.get(mi)).cloned().unwrap_or(Value::Null);
            let rw = m.get("rewrite").cloned().unwrap_or(Value::Null);
            let prw = p.get("rewrite").cloned().unwrap_or(Value::Null);
            let d = |k: &str| match (rw.get(k).and_then(Value::as_i64), prw.get(k).and_then(Value::as_i64)) {
                (Some(x), Some(y)) => json!(x - y),
                _ => Value::Null,
            };
            rows.push(json!({
                "sfu_id": link.get("sfu_id"),
                "mid": m.pointer("/entry/mid"),
                "track_id": m.pointer("/entry/track_id"),
                "last_seq": rw.get("last_seq"),
                "Δseq": d("last_seq"),
                "Δat_ms": d("last_at"),
                "error": Value::Null,
            }));
        }
    }
    println!("{}", table(&rows, &["sfu_id", "mid", "track_id", "last_seq", "Δseq", "Δat_ms", "error"]));
    Ok(0)
}

fn bus_all(o: &Opts) -> Result<i32, String> {
    let r = request(o, "GET", "/admin/bus?all=1", None)?;
    if o.json || r.status >= 400 {
        return Ok(show(o, &r, kv));
    }
    let nodes = arr(&r.body, "nodes");
    let mut views = Vec::new();
    let mut rows = Vec::new();
    for n in &nodes {
        let bus = n.get("bus").cloned().unwrap_or(Value::Null);
        let seen: Vec<String> = arr(&bus, "nodes").iter().map(|x| x.get("node").map(cell).unwrap_or_else(|| cell(x))).collect();
        let rooms = bus.get("rooms_seen").cloned().unwrap_or(Value::Null);
        views.push((seen.clone(), rooms.to_string(), n.get("error").is_some()));
        rows.push(json!({"node": n.get("node"), "nodes_seen": seen.join(","), "rooms_seen": rooms, "isolated": bus.get("isolated"), "error": n.get("error")}));
    }
    println!("{}", table(&rows, &["node", "nodes_seen", "rooms_seen", "isolated", "error"]));
    let agree = !views.is_empty() && views.iter().all(|v| !v.2 && v.0 == views[0].0 && v.1 == views[0].1);
    if agree {
        println!("합의: 전 node 가 같은 것을 본다");
    } else {
        println!("불일치: node 마다 본 것이 다르다");
    }
    Ok(if agree { 0 } else { 1 })
}

fn destroy(o: &Opts, room: &str) -> Result<i32, String> {
    let snap = request(o, "GET", &format!("/admin/rooms/{room}/snapshot"), None)?;
    if snap.status >= 400 {
        return Ok(show(o, &snap, kv));
    }
    let v = snap.body.pointer("/room/version").cloned().unwrap_or(Value::Null);
    let (Some(e), Some(s)) = (v.get("epoch").and_then(Value::as_str), v.get("seq").and_then(Value::as_u64)) else {
        return Err("방 상세에 version 이 없다".into());
    };
    let iv = format!("{e}:{s}");
    eprintln!("if_version={iv}");
    let r = request(o, "POST", &format!("/admin/rooms/{room}/destroy?if_version={iv}"), None)?;
    Ok(show(o, &r, |b| {
        let yes = b.get("destroyed").and_then(Value::as_bool).unwrap_or(false);
        format!("destroyed={}\nnotified={}", if yes { "yes" } else { "no" }, cell(b.get("notified").unwrap_or(&Value::Null)))
    }))
}

fn kill(o: &Opts, id: &str) -> Result<i32, String> {
    let st = request(o, "GET", "/admin/supervisor/status", None)?;
    let epoch = arr(&st.body, "units").iter().find(|u| u.get("id").and_then(Value::as_str) == Some(id)).and_then(|u| u.get("epoch").and_then(Value::as_str).map(str::to_string));
    let Some(epoch) = epoch else { return Err(format!("유닛 {id} 의 epoch 를 읽지 못했다(돌고 있지 않다)")) };
    eprintln!("if_epoch={epoch}");
    let r = request(o, "POST", &format!("/admin/supervisor/kill/{id}?if_epoch={epoch}"), None)?;
    Ok(show(o, &r, kv))
}

fn shutdown(o: &Opts) -> Result<i32, String> {
    let s = request(o, "GET", "/admin/snapshot", None)?;
    let Some(hub_id) = s.body.get("hub_id").and_then(Value::as_str).map(str::to_string) else { return Err("hub_id 를 읽지 못했다".into()) };
    eprintln!("confirm={hub_id}");
    let r = request(o, "POST", &format!("/admin/supervisor/shutdown?confirm={hub_id}"), None)?;
    Ok(show(o, &r, kv))
}

fn usage() -> &'static str {
    "oxadmin [--hub <host:port|URL>] [--json] [--api-key K --api-secret S] <명령>\n  sfus · sfu <id> · rooms · room <id> · users · user <id> · snapshot · drops · bus [--all]\n  unit list · unit show <id> · load <id> · stop <id> · kill <id> · shutdown\n  probe <user> · user-cut <user> · reap <room> <user> · destroy <room> · log-level <level> [--ttl <secs>]"
}

fn run(o: &Opts) -> Result<i32, String> {
    let r: Vec<&str> = o.rest.iter().map(String::as_str).collect();
    let get = |p: &str| request(o, "GET", p, None);
    let post = |p: &str, b: Option<&Value>| request(o, "POST", p, b);
    Ok(match r.as_slice() {
        ["sfus"] => show(o, &get("/admin/sfus")?, |b| table(&arr(b, "sfus"), &["unit", "sfu_id", "unit_state", "live", "node_live", "addr"])),
        ["sfu", id] => show(o, &get(&format!("/admin/sfus/{id}/rooms"))?, |b| format!("unit={} sfu_id={} total={}\n{}", cell(&b["unit"]), cell(&b["sfu_id"]), cell(&b["total"]), table(&arr(b, "rooms"), &["room_id", "members"]))),
        ["rooms"] => show(o, &get("/admin/rooms")?, |b| format!("{}\ntotal={}", table(&arr(b, "rooms"), &["room_id", "node", "sfu_id", "members"]), cell(&b["total"]))),
        ["room", id] => show(o, &get(&format!("/admin/rooms/{id}/snapshot"))?, |b| {
            let r = &b["room"];
            format!(
                "room_id={} name={} user_count={} hidden_count={} rec={} version={}:{}\nfloor state={} speaker={} queue={}\nslots audio={} video={}\n{}",
                cell(&r["room_id"]),
                cell(&r["name"]),
                cell(&r["user_count"]),
                cell(&r["hidden_count"]),
                cell(&r["rec"]),
                cell(&r["version"]["epoch"]),
                cell(&r["version"]["seq"]),
                cell(&r["floor"]["state"]),
                cell(&r["floor"]["speaker"]),
                cell(&r["floor"]["queue"]),
                cell(&r["slots"]["audio"]),
                cell(&r["slots"]["video"]),
                table(&arr(r, "tracks"), &["type", "track_id", "kind", "user_id", "active", "muted"])
            )
        }),
        ["users"] => show(o, &get("/admin/users")?, |b| {
            let rows: Vec<Value> = arr(b, "users").iter().map(|u| json!({"session": u["session"], "user_id": u["user_id"], "participant_type": u["participant_type"], "hidden": u["hidden"], "links": arr(u, "links").iter().map(|l| format!("{}:{}:{}", cell(&l["sfu_id"]), cell(&l["ufrag"]), cell(&l["state"]))).collect::<Vec<_>>().join(" ")})).collect();
            format!("{}\ntotal={}", table(&rows, &["session", "user_id", "participant_type", "hidden", "links"]), cell(&b["total"]))
        }),
        ["user", id] => user_media(o, id)?,
        ["snapshot"] => show(o, &get("/admin/snapshot")?, |b| format!("hub_id={} ready={} build={}\n{}", cell(&b["hub_id"]), cell(&b["ready"]), cell(&b["build"]), table(&arr(b, "sfus"), &["unit", "sfu_id", "unit_state", "live"]))),
        ["drops"] => show(o, &get("/admin/drops")?, |b| {
            arr(b, "units")
                .iter()
                .map(|u| {
                    let mut kvs: Vec<String> = u.as_object().map(|m| m.iter().filter(|(k, _)| *k != "unit" && *k != "sfu_id").map(|(k, v)| format!("{k}={}", cell(v))).collect()).unwrap_or_default();
                    kvs.sort();
                    format!("[{} {}]\n{}", cell(&u["unit"]), cell(u.get("sfu_id").unwrap_or(&Value::Null)), kvs.join("\n"))
                })
                .collect::<Vec<_>>()
                .join("\n")
        }),
        ["bus"] if o.all => bus_all(o)?,
        ["bus"] => show(o, &get("/admin/bus")?, kv),
        ["unit", "list"] => show(o, &get("/admin/supervisor/status")?, |b| table(&arr(b, "units"), &["id", "kind", "order", "state", "restarts", "epoch", "next_retry_ms"])),
        ["unit", "show", id] => show(o, &get("/admin/supervisor/status")?, |b| arr(b, "units").into_iter().find(|u| u.get("id").and_then(Value::as_str) == Some(id)).map(|u| kv(&u)).unwrap_or_else(|| format!("유닛 없음 {id}"))),
        ["load", id] => show(o, &post(&format!("/admin/supervisor/load/{id}"), None)?, kv),
        ["stop", id] => show(o, &post(&format!("/admin/supervisor/stop/{id}"), None)?, kv),
        ["kill", id] => kill(o, id)?,
        ["shutdown"] => shutdown(o)?,
        ["probe", u] => show(o, &get(&format!("/admin/users/{u}/probe"))?, |b| serde_json::to_string_pretty(b).unwrap_or_default()),
        ["user-cut", u] => show(o, &post(&format!("/admin/users/{u}/cut"), None)?, |b| format!("{}\ntotal={}", table(&arr(b, "cut"), &["session", "code"]), cell(&b["total"]))),
        ["reap", room, u] => show(o, &post(&format!("/admin/rooms/{room}/reap/{u}"), None)?, kv),
        ["destroy", room] => destroy(o, room)?,
        ["log-level", level] => {
            let body = match o.ttl {
                Some(t) => json!({"level": level, "ttl_secs": t}),
                None => json!({"level": level}),
            };
            show(o, &post("/admin/log-level", Some(&body))?, kv)
        }
        _ => {
            eprintln!("{}", usage());
            2
        }
    })
}

fn main() {
    let o = match parse_args(std::env::args().skip(1).collect()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("oxadmin: {e}");
            std::process::exit(2);
        }
    };
    eprintln!("[oxadmin] hub={} ({})", o.hub, o.source);
    let code = match run(&o) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("oxadmin: {e}");
            1
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_forms() {
        assert_eq!(normalize("127.0.0.1:1974"), "http://127.0.0.1:1974");
        assert_eq!(normalize("http://h:1/"), "http://h:1");
        assert_eq!(dechunk("5\r\nhello\r\n0\r\n\r\n"), "hello");
        let t = table(&[json!({"a": 1, "b": null})], &["a", "b"]);
        assert!(t.contains('-'));
    }
}
