// author: kodeholic (powered by Claude)
// spec: v1.6 · 운영:RULE-READ-HISTORY · model: claude-opus-5-5
//! 1층 — 이력 조회의 천장: 로그 파일 하나가 회전 상한(정책:logging.rotate_max_bytes)까지 찬 node 에서 묻는다. 읽는 쪽 비용은 창이 아니라 읽는 파일 크기를 따르고 그 크기는 회전 상한이 막는다 — 천장에서 계약(줄 상한 · 잘림 · 기동 줄)이 서고 시간이 예산 안이어야 한다(context 202611/20261119c ops_history).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use oxhubd::ops::logger::{Ask, read};

const CEILING: u64 = 268_435_456;
const MAX_LINES: usize = 5_000;
const BUDGET_RELEASE_MS: u128 = 5_000;
const DEBUG_SLOWDOWN: u128 = 4;
const BUDGET_MS: u128 = if cfg!(debug_assertions) { BUDGET_RELEASE_MS * DEBUG_SLOWDOWN } else { BUDGET_RELEASE_MS };

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("oxhubd-history-ceiling-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("dir");
    d
}

fn stamp(base: chrono::DateTime<chrono::FixedOffset>, i: u64) -> String {
    (base + chrono::Duration::microseconds((i * 3_000) as i64)).format("%Y-%m-%dT%H:%M:%S%.6f%:z").to_string()
}

fn fill(d: &Path, every: u64) -> u64 {
    let base = chrono::DateTime::parse_from_rfc3339("2026-11-22T00:00:00.000000+09:00").expect("base");
    let mut w = std::io::BufWriter::new(std::fs::File::create(d.join("ox.log.2026-11-22")).expect("file"));
    let mut size = 0u64;
    let boot = format!("ts={} mono=1 node=node-a proc=sfu kind=ev spec=SV-STATE-UNIT-STATE epoch=e1 unit=node-a build=b1 msg=\"sfud boot\"\n", stamp(base, 0));
    size += boot.len() as u64;
    w.write_all(boot.as_bytes()).expect("write");
    let mut i = 1u64;
    while size < CEILING {
        let room = if i % every == 0 { "busy".to_string() } else { format!("r-{}", i % 997) };
        let kind = ["ev", "diag", "stat"][(i % 3) as usize];
        let line = format!(
            "ts={} mono={i} node=node-a proc=sfu kind={kind} spec=SV-RULE-SIM-TICK session=s{} room={room} user=bot{}~scenario epoch=e1 in.target=2890491 in.cluster={i} msg=\"probe judged\"\n",
            stamp(base, i),
            i % 4096,
            i % 64
        );
        size += line.len() as u64;
        w.write_all(line.as_bytes()).expect("write");
        i += 1;
    }
    w.flush().expect("flush");
    i
}

fn ask() -> Ask {
    Ask { room: Some("busy".into()), since: 0, until: u64::MAX, diag: true, stat: true, max: MAX_LINES, ..Ask::default() }
}

fn measure(name: &str, every: u64) -> (u128, usize, bool, usize) {
    let d = dir(name);
    let n = fill(&d, every);
    let t = Instant::now();
    let f = read(&d, &ask());
    let ms = t.elapsed().as_millis();
    let _ = std::fs::remove_dir_all(&d);
    eprintln!("history ceiling {name}: lines {n} · read {ms} ms · kept {} · truncated {}", f.lines.len(), f.truncated);
    (ms, f.lines.len(), f.truncated, f.boots.len())
}

#[test]
fn a_full_log_file_is_read_within_budget_when_the_room_is_rare() {
    let (ms, kept, truncated, boots) = measure("rare", 100_000);
    assert!(kept > 0 && kept <= MAX_LINES && !truncated && boots == 1, "운영:RULE-READ-HISTORY the rare room's lines come back with their boot line");
    assert!(ms <= BUDGET_MS, "운영:RULE-READ-HISTORY O-sym-RULE-READ-HISTORY-5 a node whose log file is at the rotation ceiling answers within {BUDGET_MS} ms (took {ms} ms)");
}

#[test]
fn a_full_log_file_is_read_within_budget_when_every_line_is_the_room() {
    let (ms, kept, truncated, boots) = measure("busy", 1);
    assert!(kept == MAX_LINES && truncated && boots == 1, "운영:RULE-READ-HISTORY the newest {MAX_LINES} lines come back cut at the front with truncated");
    assert!(ms <= BUDGET_MS, "운영:RULE-READ-HISTORY O-sym-RULE-READ-HISTORY-5 a node whose log file is at the rotation ceiling answers within {BUDGET_MS} ms even when every line is picked (took {ms} ms)");
}
