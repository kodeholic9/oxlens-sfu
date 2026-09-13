// author: kodeholic (powered by Claude)
// spec: v1.1 · 운영 §6 · §5 · model: claude-opus-5

//! `oxadmin` — ★**도구는 하나다**(운영 §6). 별칭·wrapper 를 만들지 않는다.
//!
//! ★★**도구가 표를 넘지 않는다** — 서버가 안 내는 값을 계산해 채우지 않고(`-` 로 둔다),
//! 확인값을 자기 프롬프트로 대신하지 않는다(★스크립트로 부르면 그 물음이 사라진다, §5).

mod cli;
mod http;
mod table;

use table::{cell, render, row};

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

const USAGE: &str = "\
oxadmin — OxLens 운영 도구(운영 규격서 §6)

  읽기   sfus · sfu <id> · rooms · room <id> · users · snapshot · drops
         bus [--all]
         unit list | unit show <id>
  제어   load <id> · stop <id> · kill <id> · shutdown
         user-cut <user> · reap <room> <user> · destroy <room>

  --hub <host:port>     붙을 곳(env OXADMIN_URL · 기본 127.0.0.1:19745)
  --api-key/--api-secret  원격일 때 그 자리에서 운영 토큰을 서명한다(§1)
  --json                자동화용 raw(기본은 사람이 읽는 표)
";

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let c = match cli::parse(argv) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return std::process::ExitCode::from(2);
        }
    };
    if c.help || c.args.is_empty() {
        eprintln!("{USAGE}");
        return std::process::ExitCode::from(if c.help { 0 } else { 2 });
    }
    let ep = cli::resolve(c.hub.as_deref(), std::env::var(cli::ENV_URL).ok());
    // ★★**매 실행 첫 줄에 해석 결과를 적는다**(운영 §6) — stderr 라 `--json` 파이프를 안 더럽힌다.
    eprintln!("[oxadmin] {} ({})", ep.url, ep.from);

    // ★원격이면 그 자리에서 서명한다 — loopback 은 토큰 없이 통과한다(§1).
    let bearer = match (&c.api_key, &c.api_secret) {
        (Some(k), Some(s)) => match http::sign_ops(k, s, now(), http::TOKEN_TTL_S) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("{e}");
                return std::process::ExitCode::from(2);
            }
        },
        (None, None) => None,
        // ★한 쪽만 주면 실패다 — 조용히 무자격으로 떨어지면 원격에서 401 만 보고 헤맨다.
        _ => {
            eprintln!("--api-key 와 --api-secret 은 둘 다 준다(§1)");
            return std::process::ExitCode::from(2);
        }
    };
    let conn = http::Conn::new(ep.url.clone(), bearer);
    match run(&conn, &c).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::from(1)
        }
    }
}

/// 부른 결과를 그대로 낼지(`--json`) 표로 낼지. ★**실패는 어느 쪽이든 사유를 낸다.**
fn show(c: &cli::Cli, r: &http::Reply, f: impl FnOnce(&serde_json::Value)) -> Result<(), String> {
    if !r.ok() {
        // ★**본문이 사유다**(운영 §2) — 상태 코드만 내면 `1003` 과 `3009` 가 같아 보인다.
        return Err(format!(
            "HTTP {} · code={} {}",
            r.status,
            r.code().map(|c| c.to_string()).unwrap_or_else(|| table::UNKNOWN.into()),
            r.body
        ));
    }
    if c.json {
        println!("{}", r.body);
    } else {
        f(&r.body);
    }
    Ok(())
}

fn list<'a>(v: &'a serde_json::Value, key: &str) -> &'a [serde_json::Value] {
    v.get(key).and_then(|x| x.as_array()).map(|x| &x[..]).unwrap_or(&[])
}

fn need(c: &cli::Cli, i: usize, what: &str) -> Result<String, String> {
    c.args.get(i).cloned().ok_or_else(|| format!("{what} 를 안 줬다\n\n{USAGE}"))
}

async fn run(conn: &http::Conn, c: &cli::Cli) -> Result<(), String> {
    match c.args[0].as_str() {
        "sfus" => {
            let r = conn.get("/admin/sfus").await?;
            show(c, &r, |b| {
                let rows: Vec<_> = list(b, "sfus")
                    .iter()
                    // ★두 축을 나란히 놓는다 — 합치면 sfud 가 죽어도 안 보인다(§3-1).
                    .map(|x| row(x, &["sfu_id", "unit_state", "live", "node_live", "epoch"]))
                    .collect();
                print!("{}", render(&["sfu_id", "unit_state", "live", "node_live", "epoch"], &rows));
                println!("supervising: {}", cell(b.get("supervising")));
            })
        }
        "sfu" => {
            let id = need(c, 1, "sfu_id")?;
            let r = conn.get(&format!("/admin/sfus/{id}/rooms")).await?;
            show(c, &r, |b| {
                let rows: Vec<_> =
                    list(b, "rooms").iter().map(|x| row(x, &["room_id", "members"])).collect();
                print!("{}", render(&["room_id", "members"], &rows));
                println!("sfu_id: {} · total: {}", cell(b.get("sfu_id")), cell(b.get("total")));
            })
        }
        "rooms" => {
            let r = conn.get("/admin/rooms").await?;
            show(c, &r, |b| {
                let rows: Vec<_> = list(b, "rooms")
                    .iter()
                    .map(|x| row(x, &["room_id", "sfu_id", "members", "member_ids"]))
                    .collect();
                print!("{}", render(&["room_id", "sfu_id", "members", "member_ids"], &rows));
                println!("total: {}", cell(b.get("total")));
            })
        }
        "room" => {
            let id = need(c, 1, "room_id")?;
            let r = conn.get(&format!("/admin/rooms/{id}/snapshot")).await?;
            show(c, &r, print_room)
        }
        "users" => {
            let r = conn.get("/admin/users").await?;
            show(c, &r, |b| {
                let k = ["session_id", "user_id", "participant_type", "hidden"];
                let rows: Vec<_> = list(b, "users").iter().map(|x| row(x, &k)).collect();
                print!("{}", render(&k, &rows));
                println!("total: {}", cell(b.get("total")));
            })
        }
        "snapshot" => {
            let r = conn.get("/admin/snapshot").await?;
            show(c, &r, |b| {
                for k in ["hub_id", "ready", "build", "supervising", "rooms", "users"] {
                    println!("{k:<12} {}", cell(b.get(k)));
                }
            })
        }
        "drops" => {
            let r = conn.get("/admin/drops").await?;
            show(c, &r, print_drops)
        }
        "bus" => {
            // ★★**모으는 것은 hub 다**(§3-8) — 도구가 엔드포인트 목록을 들지 않는다.
            let r = conn.get(if c.all { "/admin/bus?all=1" } else { "/admin/bus" }).await?;
            show(c, &r, |b| if c.all { print_bus_all(b) } else { print_bus(b) })
        }
        "unit" => {
            let what = need(c, 1, "list|show")?;
            let r = conn.get("/admin/supervisor/status").await?;
            match what.as_str() {
                "list" => show(c, &r, |b| {
                    let k = ["id", "kind", "order", "state", "restarts", "next_retry_ms", "epoch"];
                    let rows: Vec<_> = list(b, "units").iter().map(|x| row(x, &k)).collect();
                    print!("{}", render(&k, &rows));
                    println!("supervising: {}", cell(b.get("supervising")));
                }),
                "show" => {
                    let id = need(c, 2, "unit id")?;
                    show(c, &r, |b| match list(b, "units").iter().find(|x| cell(x.get("id")) == id)
                    {
                        // ★없는 것을 빈 표로 내지 않는다 — 「멈춰 있다」로 읽힌다.
                        None => println!("{id} 는 이 hub 의 유닛이 아니다(§3-1 범위)"),
                        Some(u) => {
                            for k in
                                ["id", "kind", "order", "state", "restarts", "next_retry_ms", "epoch"]
                            {
                                println!("{k:<14} {}", cell(u.get(k)));
                            }
                        }
                    })
                }
                _ => Err(format!("unit 은 list|show 다 — 받은 값 {what}\n\n{USAGE}")),
            }
        }
        "load" | "stop" => {
            let id = need(c, 1, "unit id")?;
            let r = conn.post(&format!("/admin/supervisor/{}/{id}", c.args[0])).await?;
            show(c, &r, |b| {
                // ★`from`·`to` 를 둘 다 낸다(§4-1) — 무엇이 바뀌었는지 모르면 두 번 친다.
                println!("{} {} → {}", cell(b.get("id")), cell(b.get("from")), cell(b.get("to")));
            })
        }
        "kill" => {
            let id = need(c, 1, "unit id")?;
            // ★★**도구가 먼저 읽어 되싣는다**(§5) — 확인값은 「당신이 본 그것이 아직 그것인가」다.
            //   ★도구의 프롬프트로 대신하지 않는다 — 스크립트로 부르면 그 물음이 사라진다.
            let st = conn.get("/admin/supervisor/status").await?;
            if !st.ok() {
                return Err(format!("확인값을 못 읽었다 — HTTP {} {}", st.status, st.body));
            }
            let epoch = list(&st.body, "units")
                .iter()
                .find(|x| cell(x.get("id")) == id)
                .map(|x| cell(x.get("epoch")))
                .filter(|e| e != table::UNKNOWN)
                .ok_or_else(|| {
                    format!("{id} 의 기동 신원(epoch)이 없다 — 안 떠 있는 유닛은 못 죽인다(§5)")
                })?;
            eprintln!("[oxadmin] if_epoch={epoch}");
            let r = conn
                .post(&format!("/admin/supervisor/kill/{id}?if_epoch={epoch}"))
                .await?;
            show(c, &r, |b| {
                println!("{} {} → {}", cell(b.get("id")), cell(b.get("from")), cell(b.get("to")));
            })
        }
        "shutdown" => {
            // ★hub 는 쥔 판 값이 없다 — 이름이 유일한 수단이다(§5).
            let snap = conn.get("/admin/snapshot").await?;
            if !snap.ok() {
                return Err(format!("확인값을 못 읽었다 — HTTP {} {}", snap.status, snap.body));
            }
            let hub_id = cell(snap.body.get("hub_id"));
            if hub_id == table::UNKNOWN {
                return Err("snapshot 이 hub_id 를 안 냈다 — 확인값의 출처가 없다(§3-4)".into());
            }
            eprintln!("[oxadmin] confirm={hub_id}");
            let r = conn.post(&format!("/admin/supervisor/shutdown?confirm={hub_id}")).await?;
            show(c, &r, |b| {
                println!("accepted: {} · order: {}", cell(b.get("accepted")), cell(b.get("order")));
            })
        }
        "user-cut" => {
            let u = need(c, 1, "user_id")?;
            let r = conn.post(&format!("/admin/users/{u}/cut")).await?;
            show(c, &r, |b| {
                let rows: Vec<_> =
                    list(b, "cut").iter().map(|x| row(x, &["session_id", "code"])).collect();
                print!("{}", render(&["session_id", "code"], &rows));
                println!("user_id: {} · total: {}", cell(b.get("user_id")), cell(b.get("total")));
            })
        }
        "reap" => {
            let room = need(c, 1, "room_id")?;
            let user = need(c, 2, "user_id")?;
            let r = conn.post(&format!("/admin/rooms/{room}/reap/{user}")).await?;
            show(c, &r, |b| {
                // ★**바뀐 것을 낸다**(§5 되돌릴 수 있음) — `left` 가 그 값이다.
                println!(
                    "{} / {} · left={} · version={}",
                    cell(b.get("room_id")),
                    cell(b.get("user_id")),
                    cell(b.get("left")),
                    version_of(b.get("version"))
                );
            })
        }
        "destroy" => {
            let room = need(c, 1, "room_id")?;
            let snap = conn.get(&format!("/admin/rooms/{room}/snapshot")).await?;
            if !snap.ok() {
                return Err(format!("확인값을 못 읽었다 — HTTP {} {}", snap.status, snap.body));
            }
            // ★★**이름은 방이 재생성되면 통과한다** — 그래서 판 값을 되싣는다(§5).
            let v = snap.body.get("room").and_then(|r| r.get("version"));
            let want = match (
                v.and_then(|x| x.get("epoch")).and_then(|x| x.as_str()),
                v.and_then(|x| x.get("seq")).and_then(|x| x.as_u64()),
            ) {
                (Some(e), Some(s)) => format!("{e}:{s}"),
                _ => return Err(format!("{room} 의 판 값이 없다 — 확인값의 출처가 없다(§3-6)")),
            };
            eprintln!("[oxadmin] if_version={want}");
            let r = conn.post(&format!("/admin/rooms/{room}/destroy?if_version={want}")).await?;
            show(c, &r, |b| {
                println!(
                    "{} · destroyed={} · notified={}",
                    cell(b.get("room_id")),
                    cell(b.get("destroyed")),
                    cell(b.get("notified"))
                );
            })
        }
        other => Err(format!("모르는 명령: {other}\n\n{USAGE}")),
    }
}

fn version_of(v: Option<&serde_json::Value>) -> String {
    match (
        v.and_then(|x| x.get("epoch")).and_then(|x| x.as_str()),
        v.and_then(|x| x.get("seq")).and_then(|x| x.as_u64()),
    ) {
        (Some(e), Some(s)) => format!("{e}:{s}"),
        _ => table::UNKNOWN.to_string(),
    }
}

fn print_room(b: &serde_json::Value) {
    let Some(r) = b.get("room") else {
        println!("room 이 없다 — 서버가 정본을 못 가져왔다(§3-6)");
        return;
    };
    for k in ["room_id", "name", "capacity", "user_count", "hidden_count", "rec"] {
        println!("{k:<14} {}", cell(r.get(k)));
    }
    println!("{:<14} {}", "version", version_of(r.get("version")));
    let f = r.get("floor");
    println!(
        "{:<14} {} · speaker={} · queue={}",
        "floor",
        cell(f.and_then(|x| x.get("state"))),
        cell(f.and_then(|x| x.get("speaker"))),
        cell(f.and_then(|x| x.get("queue"))),
    );
    let s = r.get("slots");
    println!(
        "{:<14} audio={} video={}",
        "slots",
        cell(s.and_then(|x| x.get("audio"))),
        cell(s.and_then(|x| x.get("video"))),
    );
    let ps: Vec<_> = r
        .get("participants")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().map(|x| row(x, &["user_id", "participant_type"])).collect())
        .unwrap_or_default();
    print!("{}", render(&["user_id", "participant_type"], &ps));
}

fn print_drops(b: &serde_json::Value) {
    // ★사유가 늘어도 도구를 안 고친다 — 서버가 낸 키를 그대로 열로 삼는다.
    let units = list(b, "units");
    let mut keys: Vec<String> = Vec::new();
    for u in units {
        if let Some(o) = u.as_object() {
            for k in o.keys() {
                if k != "sfu_id" && !keys.iter().any(|x| x == k) {
                    keys.push(k.clone());
                }
            }
        }
    }
    keys.sort();
    let mut head = vec!["sfu_id".to_string()];
    head.extend(keys);
    let refs: Vec<&str> = head.iter().map(String::as_str).collect();
    let rows: Vec<_> = units.iter().map(|u| row(u, &refs)).collect();
    print!("{}", render(&refs, &rows));
}

fn print_bus(b: &serde_json::Value) {
    for k in ["node", "inst", "open", "mode", "namespace", "listen", "connect"] {
        println!("{k:<12} {}", cell(b.get(k)));
    }
    // ★**붙은 것**과 설정의 `connect` 는 다른 물음이다(§3-8) — 따로 찍는다.
    let peers: Vec<String> = list(b, "peers")
        .iter()
        .map(|p| format!("{}({})", cell(p.get("zid")), cell(p.get("whatami"))))
        .collect();
    println!(
        "{:<12} {}",
        "peers",
        if peers.is_empty() { table::UNKNOWN.into() } else { peers.join(" ") }
    );
    // ★★**고립은 「남이 다 죽었다」가 아니라 「내가 끊겼다」다**(§15-7) — 그 한 줄이
    //   `ready` 503 의 사유라 표에 둔다. 없으면 운영자가 `down` 이 비었는데 왜 503 인지 모른다.
    println!("{:<12} {}", "isolated", cell(b.get("isolated")));
    let rows: Vec<_> =
        list(b, "nodes").iter().map(|x| row(x, &["node", "inst", "self"])).collect();
    print!("{}", render(&["node", "inst", "self"], &rows));
    let p = b.get("pending");
    println!(
        "pending      unplaced={} · no_view={}",
        cell(p.and_then(|x| x.get("rooms_unplaced"))),
        cell(p.and_then(|x| x.get("rooms_no_view"))),
    );
    let e = b.get("last_event");
    println!(
        "last_event   {} {} @{}",
        cell(e.and_then(|x| x.get("kind"))),
        cell(e.and_then(|x| x.get("key"))),
        cell(e.and_then(|x| x.get("at_ms"))),
    );
    // ★**배워서 아는 방**(§3-8) — 내가 만든 방(`rooms`)과 다른 축이다.
    //   ★둘이 어긋나는 것이 사고의 형상이라 섞어 내지 않는다.
    let rows: Vec<_> =
        list(b, "rooms_seen").iter().map(|x| row(x, &["room_id", "node", "inst"])).collect();
    println!("\nrooms_seen — 토큰으로 배운 자리(정§15-2)");
    print!("{}", render(&["room_id", "node", "inst"], &rows));
}

/// ★**견주는 일은 도구가 한다**(§3-8) — 서버는 나란히 놓기까지다.
fn print_bus_all(b: &serde_json::Value) {
    println!("asked: {}", cell(b.get("asked")));
    let rows: Vec<Vec<String>> = list(b, "nodes")
        .iter()
        .map(|r| {
            let bus = r.get("bus");
            let seen = bus
                .map(|x| list(x, "nodes").iter().map(|n| cell(n.get("node"))).collect::<Vec<_>>())
                .unwrap_or_default();
            vec![
                cell(r.get("node")),
                cell(r.get("error")),
                cell(bus.and_then(|x| x.get("isolated"))),
                if seen.is_empty() { table::UNKNOWN.into() } else { seen.join(",") },
                cell(bus.and_then(|x| x.get("rooms_seen")).and_then(|x| x.as_array()).map(|a| serde_json::json!(a.len())).as_ref()),
                cell(bus.and_then(|x| x.get("pending")).and_then(|x| x.get("rooms_unplaced"))),
                cell(bus.and_then(|x| x.get("pending")).and_then(|x| x.get("rooms_no_view"))),
            ]
        })
        .collect();
    let head = ["node", "error", "isolated", "본 node", "배운 방", "unplaced", "no_view"];
    print!("{}", render(&head, &rows));
    // ★**어긋난 곳을 짚어 준다** — 나란히만 놓으면 사람이 매번 눈으로 맞춰야 한다(§3-8 까닭).
    let views: Vec<(String, Vec<String>)> = list(b, "nodes")
        .iter()
        .filter(|r| r.get("error").is_none())
        .map(|r| {
            let mut v: Vec<String> = r
                .get("bus")
                .map(|x| list(x, "nodes").iter().map(|n| cell(n.get("node"))).collect())
                .unwrap_or_default();
            v.sort();
            (cell(r.get("node")), v)
        })
        .collect();
    let same = views.windows(2).all(|w| w[0].1 == w[1].1);
    let broken: Vec<String> = list(b, "nodes")
        .iter()
        .filter(|r| r.get("error").is_some())
        .map(|r| cell(r.get("node")))
        .collect();
    match (same, broken.is_empty()) {
        (true, true) => println!("합의: 전 node 가 같은 것을 본다"),
        // ★**「못 닿았다」를 「어긋났다」와 가른다** — 처방이 다르다.
        (_, false) => println!("★못 닿은 node: {} — 그 node 의 눈은 모른다", broken.join(",")),
        (false, true) => println!("★어긋났다 — node 마다 보는 집합이 다르다(위 표 「본 node」)"),
    }
}
