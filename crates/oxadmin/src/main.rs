// author: kodeholic (powered by Claude)
// spec: v1.6 · 운영:RULE-TOOL-ONE · 운영:RULE-TOOL-CONTRACT · 운영:RULE-RISK-CONFIRM · 운영:RULE-CTL-SUPERVISOR · 운영:RULE-CTL-CUT · 운영:RULE-CTL-ROOM · 운영:RULE-CTL-LOG-LEVEL · 운영:RULE-READ-BUS-ALL · 운영:RULE-READ-USER-MEDIA · model: claude-opus-5-5
//! oxadmin — 운영 도구 하나 · 사람이 읽는 표가 기본(`--json` 은 raw) · 붙은 hub 를 매번 stderr 첫 줄에 · 되돌릴 수 없는 것은 도구가 먼저 읽은 판 값을 되싣는다 · `user` 는 두 번 읽어 Δ.

mod http;

use std::time::Duration;

use serde_json::{Value, json};

use http::Hub;

const USAGE: &str = "oxadmin [--hub <host:port|url>] [--json] [--api-key K --api-secret S] <명령>
  sfus | sfu <id> | rooms | room <id> | users | user <id> [--interval-ms <ms>] | snapshot | drops
  bus [--all] | unit list | unit show <id> | load <id> | stop <id> | kill <id> | shutdown
  probe <user> | user-cut <user> | reap <room> <user> | destroy <room> | log-level <level> [--ttl <secs>]";

struct Opts {
    hub: Option<String>,
    json: bool,
    api_key: Option<String>,
    api_secret: Option<String>,
    interval_ms: u64,
    all: bool,
    ttl: Option<u64>,
    rest: Vec<String>,
}

fn opts(argv: Vec<String>) -> Result<Opts, String> {
    let mut o = Opts { hub: None, json: false, api_key: None, api_secret: None, interval_ms: 1_000, all: false, ttl: None, rest: Vec::new() };
    let mut it = argv.into_iter();
    while let Some(a) = it.next() {
        let mut val = |n: &str| it.next().ok_or_else(|| format!("{n} 에 값이 없다"));
        match a.as_str() {
            "--hub" => o.hub = Some(val("--hub")?),
            "--json" => o.json = true,
            "--api-key" => o.api_key = Some(val("--api-key")?),
            "--api-secret" => o.api_secret = Some(val("--api-secret")?),
            "--interval-ms" => o.interval_ms = val("--interval-ms")?.parse().map_err(|_| "--interval-ms 는 수".to_string())?,
            "--all" => o.all = true,
            "--ttl" => o.ttl = Some(val("--ttl")?.parse().map_err(|_| "--ttl 은 수".to_string())?),
            "-h" | "--help" => return Err(USAGE.to_string()),
            x if x.starts_with("--") => return Err(format!("모르는 인자 {x}\n{USAGE}")),
            _ => o.rest.push(a),
        }
    }
    Ok(o)
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn cell(v: &Value) -> String {
    match v {
        Value::Null => "-".into(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "yes".into() } else { "no".into() },
        Value::Array(a) => a.iter().map(cell).collect::<Vec<_>>().join(","),
        x => x.to_string(),
    }
}

fn table(rows: &[Value], cols: &[&str]) {
    let cells: Vec<Vec<String>> = rows.iter().map(|r| cols.iter().map(|c| cell(r.get(*c).unwrap_or(&Value::Null))).collect()).collect();
    let mut w: Vec<usize> = cols.iter().map(|c| c.chars().count()).collect();
    for r in &cells {
        for (i, c) in r.iter().enumerate() {
            w[i] = w[i].max(c.chars().count());
        }
    }
    let line = |xs: Vec<String>| xs.iter().enumerate().map(|(i, x)| format!("{x:<width$}", width = w[i])).collect::<Vec<_>>().join("  ");
    println!("{}", line(cols.iter().map(|c| c.to_string()).collect()));
    for r in cells {
        println!("{}", line(r));
    }
    if rows.is_empty() {
        println!("(없음)");
    }
}

fn kv(v: &Value, keys: &[&str]) {
    for k in keys {
        println!("{k:<14} {}", cell(v.get(*k).unwrap_or(&Value::Null)));
    }
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}

struct Ctx {
    hub: Hub,
    json: bool,
}

impl Ctx {
    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        let r = self.hub.request(method, path, body.as_ref())?;
        if r.status >= 300 {
            let f = &r.body;
            return Err(format!("HTTP {} — {} {} {}", r.status, cell(f.get("code").unwrap_or(&Value::Null)), cell(f.get("name").unwrap_or(&Value::Null)), f.get("message").map(cell).unwrap_or_default()));
        }
        Ok(r.body)
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        self.call("GET", path, None)
    }

    fn post(&self, path: &str, body: Option<Value>) -> Result<Value, String> {
        self.call("POST", path, body)
    }

    fn raw(&self, v: &Value) -> bool {
        if self.json {
            println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
        }
        self.json
    }
}

fn need<'a>(rest: &'a [String], i: usize, what: &str) -> Result<&'a str, String> {
    rest.get(i).map(|s| s.as_str()).ok_or_else(|| format!("{what} 가 필요하다\n{USAGE}"))
}

fn run(c: &Ctx, o: &Opts) -> Result<(), String> {
    let r = &o.rest;
    let cmd = r.first().map(|s| s.as_str()).ok_or_else(|| USAGE.to_string())?;
    match cmd {
        "sfus" => {
            let v = c.get("/admin/sfus")?;
            if !c.raw(&v) {
                table(arr(&v, "sfus"), &["unit", "sfu_id", "unit_state", "live", "node_live", "addr"]);
            }
        }
        "sfu" => {
            let id = need(r, 1, "유닛 id")?;
            let v = c.get(&format!("/admin/sfus/{}/rooms", enc(id)))?;
            if !c.raw(&v) {
                kv(&v, &["unit", "sfu_id", "total"]);
                table(arr(&v, "rooms"), &["room_id", "members", "member_ids"]);
            }
        }
        "rooms" => {
            let v = c.get("/admin/rooms")?;
            if !c.raw(&v) {
                table(arr(&v, "rooms"), &["room_id", "node", "sfu_id", "members", "member_ids"]);
                println!("total {}", cell(v.get("total").unwrap_or(&Value::Null)));
            }
        }
        "room" => {
            let id = need(r, 1, "방 id")?;
            let v = c.get(&format!("/admin/rooms/{}/snapshot", enc(id)))?;
            if !c.raw(&v) {
                let room = v.get("room").cloned().unwrap_or(Value::Null);
                kv(&room, &["room_id", "name", "capacity", "user_count", "hidden_count", "rec"]);
                let ver = room.get("version").cloned().unwrap_or(Value::Null);
                println!("{:<14} {}:{}", "version", cell(ver.get("epoch").unwrap_or(&Value::Null)), cell(ver.get("seq").unwrap_or(&Value::Null)));
                println!("\n참가자");
                table(arr(&room, "participants"), &["user_id", "participant_type", "select", "role"]);
                println!("\n트랙");
                table(arr(&room, "tracks"), &["type", "track_id", "kind", "user_id", "codec", "active", "muted", "simulcast"]);
                let f = room.get("floor").cloned().unwrap_or(Value::Null);
                println!("\n발언권");
                kv(&f, &["state", "speaker", "priority", "first_rtp_at", "last_rtp_at"]);
                table(arr(&f, "queue"), &["user_id", "priority"]);
                table(arr(&f, "timers"), &["name", "user_id", "at"]);
            }
        }
        "users" => {
            let v = c.get("/admin/users")?;
            if !c.raw(&v) {
                let rows: Vec<Value> = arr(&v, "users")
                    .iter()
                    .map(|u| {
                        let links: Vec<String> = arr(u, "links").iter().map(|l| format!("{}:{}:{}", cell(&l["sfu_id"]), cell(&l["ufrag"]), cell(&l["state"]))).collect();
                        json!({ "session": u["session"], "user_id": u["user_id"], "participant_type": u["participant_type"], "hidden": u["hidden"], "links": links })
                    })
                    .collect();
                table(&rows, &["session", "user_id", "participant_type", "hidden", "links"]);
            }
        }
        "user" => user(c, need(r, 1, "user id")?, o.interval_ms)?,
        "snapshot" => {
            let v = c.get("/admin/snapshot")?;
            if !c.raw(&v) {
                kv(&v, &["hub_id", "ready", "build", "supervising", "rooms", "users"]);
                table(arr(&v, "sfus"), &["unit", "sfu_id", "unit_state", "live", "node_live"]);
            }
        }
        "drops" => {
            let v = c.get("/admin/drops")?;
            if !c.raw(&v) {
                let mut rows = Vec::new();
                for u in arr(&v, "units") {
                    if let Some(o) = u.as_object() {
                        for (k, x) in o {
                            if matches!(k.as_str(), "unit" | "sfu_id") {
                                continue;
                            }
                            if k == "error" || x.as_u64().map(|n| n > 0).unwrap_or(true) {
                                rows.push(json!({ "unit": u["unit"], "sfu_id": u["sfu_id"], "name": k, "count": x }));
                            }
                        }
                    }
                }
                table(&rows, &["unit", "sfu_id", "name", "count"]);
            }
        }
        "bus" => bus(c, o.all)?,
        "unit" => match r.get(1).map(|s| s.as_str()) {
            Some("list") => {
                let v = c.get("/admin/supervisor/status")?;
                if !c.raw(&v) {
                    table(arr(&v, "units"), &["id", "kind", "order", "state", "restarts", "epoch", "next_retry_ms"]);
                }
            }
            Some("show") => {
                let id = need(r, 2, "유닛 id")?;
                let v = c.get("/admin/supervisor/status")?;
                let u = arr(&v, "units").iter().find(|u| u["id"].as_str() == Some(id)).cloned().ok_or_else(|| format!("유닛 {id} 없음"))?;
                if !c.raw(&u) {
                    kv(&u, &["id", "kind", "order", "state", "restarts", "epoch", "next_retry_ms"]);
                }
            }
            _ => return Err(format!("unit list | unit show <id>\n{USAGE}")),
        },
        "load" | "stop" => {
            let id = need(r, 1, "유닛 id")?;
            let v = c.post(&format!("/admin/supervisor/{cmd}/{}", enc(id)), None)?;
            if !c.raw(&v) {
                println!("{} {} → {}", cell(&v["id"]), cell(&v["from"]), cell(&v["to"]));
            }
        }
        "kill" => {
            let id = need(r, 1, "유닛 id")?;
            let st = c.get("/admin/supervisor/status")?;
            let epoch = arr(&st, "units").iter().find(|u| u["id"].as_str() == Some(id)).and_then(|u| u["epoch"].as_str().map(|s| s.to_string())).ok_or_else(|| format!("유닛 {id} 의 epoch 를 못 읽었다"))?;
            eprintln!("if_epoch={epoch}");
            let v = c.post(&format!("/admin/supervisor/kill/{}?if_epoch={}", enc(id), enc(&epoch)), None)?;
            if !c.raw(&v) {
                println!("{} {} → {}", cell(&v["id"]), cell(&v["from"]), cell(&v["to"]));
            }
        }
        "shutdown" => {
            let snap = c.get("/admin/snapshot")?;
            let hub_id = snap["hub_id"].as_str().map(|s| s.to_string()).ok_or("hub_id 를 못 읽었다")?;
            eprintln!("confirm={hub_id}");
            let v = c.post(&format!("/admin/supervisor/shutdown?confirm={}", enc(&hub_id)), None)?;
            if !c.raw(&v) {
                println!("accepted={} order={}", cell(&v["accepted"]), cell(&v["order"]));
            }
        }
        "probe" => {
            let u = need(r, 1, "user id")?;
            let v = c.get(&format!("/admin/users/{}/probe", enc(u)))?;
            if !c.raw(&v) {
                for s in arr(&v, "sessions") {
                    if s.get("timeout").and_then(|x| x.as_bool()).unwrap_or(false) {
                        println!("{}  무응답", cell(&s["session"]));
                    } else {
                        println!("{}  {}", cell(&s["session"]), s.get("result").map(|x| x.to_string()).unwrap_or_else(|| "-".into()));
                    }
                }
            }
        }
        "user-cut" => {
            let u = need(r, 1, "user id")?;
            let v = c.post(&format!("/admin/users/{}/cut", enc(u)), None)?;
            if !c.raw(&v) {
                println!("{v}");
            }
        }
        "reap" => {
            let room = need(r, 1, "방 id")?;
            let u = need(r, 2, "user id")?;
            let v = c.post(&format!("/admin/rooms/{}/reap/{}", enc(room), enc(u)), None)?;
            if !c.raw(&v) {
                let ver = &v["version"];
                println!("room={} user={} left={} version={}:{}", cell(&v["room_id"]), cell(&v["user_id"]), cell(&v["left"]), cell(&ver["epoch"]), cell(&ver["seq"]));
            }
        }
        "destroy" => {
            let room = need(r, 1, "방 id")?;
            let snap = c.get(&format!("/admin/rooms/{}/snapshot", enc(room)))?;
            let ver = &snap["room"]["version"];
            let (Some(e), Some(s)) = (ver["epoch"].as_str(), ver["seq"].as_u64()) else { return Err("방 version 을 못 읽었다".into()) };
            let iv = format!("{e}:{s}");
            eprintln!("if_version={iv}");
            let v = c.post(&format!("/admin/rooms/{}/destroy?if_version={}", enc(room), enc(&iv)), None)?;
            if !c.raw(&v) {
                println!("room={} destroyed={} notified={}", cell(&v["room_id"]), cell(&v["destroyed"]), cell(&v["notified"]));
            }
        }
        "log-level" => {
            let level = need(r, 1, "수준")?;
            let mut b = json!({ "level": level });
            if let Some(t) = o.ttl {
                b["ttl_secs"] = json!(t);
            }
            let v = c.post("/admin/log-level", Some(b))?;
            if !c.raw(&v) {
                println!("{} → {} until={} units={}", cell(&v["from"]), cell(&v["to"]), cell(&v["until"]), cell(&v["units"]));
            }
        }
        _ => return Err(format!("모르는 명령 {cmd}\n{USAGE}")),
    }
    Ok(())
}

fn user(c: &Ctx, u: &str, interval_ms: u64) -> Result<(), String> {
    let path = format!("/admin/users/{}/media", enc(u));
    let a = c.get(&path)?;
    std::thread::sleep(Duration::from_millis(interval_ms));
    let b = c.get(&path)?;
    if c.json {
        println!("{}", serde_json::to_string_pretty(&json!({ "first": a, "second": b, "interval_ms": interval_ms })).unwrap_or_default());
        return Ok(());
    }
    println!("user {u} · 두 번 읽음 간격 {interval_ms}ms");
    for (la, lb) in arr(&a, "links").iter().zip(arr(&b, "links")) {
        if let Some(e) = lb.get("error") {
            println!("sfu {} error {}", cell(&lb["sfu_id"]), cell(e));
            continue;
        }
        println!("sfu {} ufrag {} arev {} acked {}", cell(&lb["sfu_id"]), cell(&lb["ufrag"]), cell(&lb["arev"]), cell(&lb["acked"]));
        let ma = arr(la, "mlines");
        let rows: Vec<Value> = arr(lb, "mlines")
            .iter()
            .map(|m| {
                let mid = &m["entry"]["mid"];
                let prev = ma.iter().find(|x| &x["entry"]["mid"] == mid);
                let rw = &m["rewrite"];
                let pw = prev.map(|p| &p["rewrite"]);
                let seq_now = rw["last_seq"].as_u64();
                let seq_before = pw.and_then(|p| p["last_seq"].as_u64());
                let flow = match (seq_before, seq_now, pw.and_then(|p| p["last_at"].as_u64()), rw["last_at"].as_u64()) {
                    (Some(x), Some(y), Some(t0), Some(t1)) if x != y || t0 != t1 => "흐름",
                    (Some(_), Some(_), _, _) => "멎음",
                    _ => "-",
                };
                let seq = match (seq_before, seq_now) {
                    (Some(x), Some(y)) => format!("{x}→{y}"),
                    (None, Some(y)) => format!("-→{y}"),
                    _ => "-".into(),
                };
                let src = if rw["src"].is_null() { "-".to_string() } else { format!("{}/{}", cell(&rw["src"]["user_id"]), cell(&rw["src"]["track_id"])) };
                json!({ "mid": mid, "kind": m["entry"]["kind"], "track_id": m["entry"]["track_id"], "room_id": m["entry"]["room_id"], "ssrc": m["entry"]["ssrc"], "src": src, "last_seq": seq, "switches": rw["switches"], "flow": flow })
            })
            .collect();
        table(&rows, &["mid", "kind", "track_id", "room_id", "ssrc", "src", "last_seq", "switches", "flow"]);
    }
    Ok(())
}

fn bus(c: &Ctx, all: bool) -> Result<(), String> {
    if !all {
        let v = c.get("/admin/bus")?;
        if !c.raw(&v) {
            kv(&v, &["node", "inst", "mode", "namespace", "listen", "connect"]);
            println!("\npeers");
            table(arr(&v, "peers"), &["zid", "whatami", "links"]);
            println!("\nnodes");
            table(arr(&v, "nodes"), &["node", "inst", "self"]);
            println!("\nrooms_seen");
            table(arr(&v, "rooms_seen"), &["room_id", "node", "inst"]);
            println!("\nisolated {}", cell(v.get("isolated").unwrap_or(&Value::Null)));
        }
        return Ok(());
    }
    let v = c.get("/admin/bus?all=1")?;
    if c.raw(&v) {
        return Ok(());
    }
    let mut views: Vec<(String, Option<Vec<String>>)> = Vec::new();
    for n in arr(&v, "nodes") {
        let name = cell(&n["node"]);
        match n.get("bus") {
            Some(b) => {
                let mut seen: Vec<String> = arr(b, "nodes").iter().map(|x| format!("{}/{}", cell(&x["node"]), cell(&x["inst"]))).collect();
                seen.sort();
                views.push((name, Some(seen)));
            }
            None => views.push((name, None)),
        }
    }
    let rows: Vec<Value> = views
        .iter()
        .map(|(n, s)| match s {
            Some(s) => json!({ "node": n, "sees": s }),
            None => json!({ "node": n, "sees": "error" }),
        })
        .collect();
    table(&rows, &["node", "sees"]);
    let first = views.first().and_then(|x| x.1.clone());
    let agree = !views.is_empty() && views.iter().all(|(_, s)| s.is_some() && s.clone() == first);
    if agree {
        println!("합의: 전 node 가 같은 것을 본다");
    } else {
        println!("불일치: node 마다 보는 것이 다르다(또는 못 닿은 node 가 있다)");
    }
    Ok(())
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let o = match opts(argv) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let hub = match Hub::resolve(o.hub.as_deref(), std::env::var("OXADMIN_URL").ok()) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[oxadmin] {e}");
            std::process::exit(2);
        }
    };
    eprintln!("[oxadmin] hub={} ({})", hub.url, hub.source);
    let token = match (&o.api_key, &o.api_secret) {
        (Some(k), Some(s)) => match http::sign(k, s) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("[oxadmin] 서명 실패: {e}");
                std::process::exit(2);
            }
        },
        _ => None,
    };
    if token.is_none() && !hub.loopback() {
        eprintln!("[oxadmin] 원격 hub 는 --api-key · --api-secret 로 서명해야 한다");
        std::process::exit(2);
    }
    let c = Ctx { hub: hub.with_token(token), json: o.json };
    if let Err(e) = run(&c, &o) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
