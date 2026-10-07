// author: kodeholic (powered by Claude)
// spec: context/spec/pilot/floor/source/Floor.tla · trace/gen.py
//! floor 원천 실행 기록 재생 — 원천이 걸은 서버 사건을 판정 로직에 그대로 넣고, 걸음마다 상태와 내보낸 것을 견준다.
//! 타이머는 시간 없는 사건이라 `expire` 로 직접 터뜨린다.

use super::*;
use serde_json::Value;

const TRACE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/floor_trace.json");

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn opt(v: &Value) -> Option<String> {
    match s(v) {
        "none" | "" => None,
        x => Some(x.to_string()),
    }
}

fn n(v: &Value) -> u64 {
    v.as_u64().unwrap_or_default()
}

fn general(g: &str) -> General {
    match g {
        "TAKEN" => General::Taken,
        "PREVOKE" => General::PendingRevoke,
        _ => General::Idle,
    }
}

fn participant(p: &str) -> Option<Participant> {
    match p {
        "NP_IDLE" => Some(Participant::NotPermittedIdle),
        "NP_TAKEN" => Some(Participant::NotPermittedTaken),
        "PERMITTED" => Some(Participant::Permitted),
        "PREVOKE" => Some(Participant::PendingRevoke),
        "NPSENDS" => Some(Participant::NotPermittedSends),
        _ => None,
    }
}

fn wire(m: &Value) -> Option<Wire> {
    let (a, nn, d, q) = (&m["a"], n(&m["n"]), n(&m["d"]), n(&m["q"]) as u16);
    Some(match s(&m["t"]) {
        "GRANTED" => Wire::Granted { priority: nn as u8, duration_s: d as u16 },
        "DENY" => Wire::Deny { cause: nn as u8, text: None },
        "REVOKE" => Wire::Revoke { cause: nn as u8 },
        "QINFO" => Wire::QueueInfo { position: nn as u8, priority: d as u8 },
        "TAKEN" => Wire::Taken { speaker: s(a).to_string(), seq: q },
        "IDLE" => Wire::Idle { prev: opt(a), seq: q },
        _ => return None,
    })
}

fn plain(w: &Wire) -> Wire {
    match w {
        Wire::Deny { cause, .. } => Wire::Deny { cause: *cause, text: None },
        x => x.clone(),
    }
}

fn sorted(mut v: Vec<(String, Wire)>) -> Vec<(String, Wire)> {
    v.sort_by_key(|x| format!("{x:?}"));
    v
}

fn outs(want: &Value) -> Vec<(String, Wire)> {
    sorted(
        want.as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| Some((s(&o["to"]).to_string(), wire(&o["m"])?)))
            .collect(),
    )
}

fn got(o: Vec<Out>) -> Vec<(String, Wire)> {
    sorted(o.into_iter().map(|x| (x.to, plain(&x.wire))).collect())
}

fn members(snap: &Value) -> Vec<String> {
    let mut m: Vec<String> = snap["members"].as_array().into_iter().flatten().map(|x| s(x).to_string()).collect();
    m.sort();
    m
}

fn flag(snap: &Value, key: &str, u: &str) -> bool {
    snap[key][u].as_bool().unwrap_or(false)
}

fn rx(m: &Value) -> Option<Rx> {
    Some(match s(&m["t"]) {
        "REQUEST" => Rx::Request { priority: n(&m["n"]) as u8 },
        "RELEASE" => Rx::Release,
        "QPOS_REQ" => Rx::QueuePos,
        "ACK" => Rx::Ack,
        _ => return None,
    })
}

fn timer(k: &str) -> Option<Timer> {
    Some(match k {
        "T1" => Timer::T1,
        "T2" => Timer::T2,
        "T3" => Timer::T3,
        "T7" => Timer::T7,
        "T8" => Timer::T8,
        "T9" => Timer::T9,
        "T20" => Timer::T20,
        _ => return None,
    })
}

fn step(f: &mut Floor, act: &Value, before: &Value) -> Option<Vec<Out>> {
    let k = s(&act["k"]);
    let c = s(&act["c"]);
    let ms = members(before);
    if let Some(t) = timer(k) {
        let u = (c != "none").then_some(c);
        return Some(f.expire(t, u, &ms, 0));
    }
    Some(match k {
        "dC" => {
            let ctx = Ctx {
                in_room: ms.iter().any(|m| m == c),
                has_half: flag(before, "half", c),
                in_pub_room: flag(before, "pub", c),
                permitted: flag(before, "perm", c),
            };
            f.rx(c, rx(&act["m"])?, ctx, &ms, 0)
        }
        "rtp" => f.rtp(c, 0, 0),
        "ready" => f.ready(c),
        "perm_down" => f.permission_lost(c, 0),
        "pub_away" => f.away(c, &ms, 0),
        "leave" => f.leave(c, &ms, 0),
        _ => return None,
    })
}

fn check(f: &Floor, snap: &Value, at: &str) -> Vec<String> {
    let mut bad = Vec::new();
    let mut eq = |what: &str, want: String, have: String| {
        if want != have {
            bad.push(format!("{at} {what}: 원천 {want} · 코드 {have}"));
        }
    };
    eq("g", format!("{:?}", general(s(&snap["g"]))), format!("{:?}", f.g));
    eq("speaker", format!("{:?}", opt(&snap["speaker"])), format!("{:?}", f.speaker));
    eq("revoking", format!("{:?}", opt(&snap["revoking"])), format!("{:?}", f.revoking));
    let q: Vec<String> = snap["queue"].as_array().into_iter().flatten().map(|x| s(x).to_string()).collect();
    let fq: Vec<String> = f.queue_view().into_iter().map(|(u, _)| u).collect();
    eq("queue", format!("{q:?}"), format!("{fq:?}"));
    for u in members(snap) {
        let want = participant(s(&snap["part"][&u]));
        eq(&format!("part[{u}]"), format!("{want:?}"), format!("{:?}", Some(f.participant(&u))));
        eq(&format!("retry[{u}]"), flag(snap, "retry", &u).to_string(), f.retry.contains_key(&u).to_string());
        let rel = f.parts.get(&u).is_some_and(|p| p.released);
        eq(&format!("released[{u}]"), flag(snap, "released", &u).to_string(), rel.to_string());
    }
    bad
}

#[test]
fn source_traces_replay_step_for_step() {
    let text = std::fs::read_to_string(TRACE)
        .unwrap_or_else(|_| panic!("기록 없음: {TRACE} — context/spec/pilot/floor/source/trace/gen.py 로 짓는다"));
    let doc: Value = serde_json::from_str(&text).expect("기록 해독");
    let traces = doc["traces"].as_array().expect("traces");
    assert!(!traces.is_empty());
    let mut steps = 0usize;
    let mut fails = Vec::new();
    for (ti, t) in traces.iter().enumerate() {
        let run = s(&t["run"]);
        let st = t["steps"].as_array().expect("steps");
        let mut f = Floor::new(timers::T2_MS);
        f.queue_max = t["cfg"]["QMax"].as_u64().expect("cfg.QMax") as usize;
        for i in 1..st.len() {
            let (before, now) = (&st[i - 1]["snap"], &st[i]);
            let act = &now["act"];
            let at = format!("{run}#{ti} 걸음 {i} {}({})", s(&act["k"]), s(&act["c"]));
            let Some(out) = step(&mut f, act, before) else { continue };
            steps += 1;
            let (want, have) = (outs(&act["o"]), got(out));
            let mut bad = Vec::new();
            if want != have {
                bad.push(format!("{at} 내보냄: 원천 {want:?} · 코드 {have:?}"));
            }
            bad.extend(check(&f, &now["snap"], &at));
            if !bad.is_empty() {
                fails.extend(bad);
                break;
            }
        }
    }
    assert!(fails.is_empty(), "원천 {} · 재생 {steps}걸음 · 어긋난 기록 {}\n{}", s(&doc["source_sha256"]), fails.len(), fails.join("\n"));
    assert!(steps > 0);
}
