// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-BUS-FATE · model: claude-opus-5-5
//! 1층 소스 가드 — hub 의 기동은 버스 → sfu(sfud 유닛은 열린 버스를 받아 뜬다) · 정지는 sfu → 버스 → hub(유닛을 다 멈춘 뒤 버스를 닫는다). 차례를 뒤집으면 생기는 증상(첫 선언 유실 · 거짓 사망 구간)은 갈래B 로 확인하고 셈에 넣지 않는다(context 202611/20261124b 서버 규칙 갈래 · 메인 결정 ⓐ).

fn run_lines() -> Vec<String> {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs")).expect("hub main.rs");
    let start = text.find("async fn run(").expect("stand check — the hub run function");
    let body = &text[start..];
    let end = body[1..].find("\nfn ").or_else(|| body[1..].find("\nasync fn ")).map(|i| i + 1).unwrap_or(body.len());
    body[..end].lines().map(|l| l.trim().to_string()).filter(|l| !l.starts_with("//")).collect()
}

fn at(lines: &[String], needle: &str) -> usize {
    let hits: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.contains(needle)).map(|(i, _)| i).collect();
    assert_eq!(hits.len(), 1, "stand check — {needle} appears once in run ({hits:?})");
    hits[0]
}

#[test]
fn the_bus_opens_before_the_media_server_starts() {
    let lines = run_lines();
    assert!(lines.len() > 100, "stand check — the run function was read ({})", lines.len());
    let (open, sfu) = (at(&lines, "Bus::open("), at(&lines, "sup.start_sfu("));
    assert!(open < sfu, "S-sym-RULE-BUS-FATE-2 boot order is bus → sfu — the media server starts after the bus is open (open {open} · start_sfu {sfu})");
    assert!(lines[sfu].contains("start_sfu(bus.clone())"), "S-sym-RULE-BUS-FATE-2 the media server unit is started with the opened bus: {}", lines[sfu]);
}

#[test]
fn the_media_server_stops_before_the_bus_closes() {
    let lines = run_lines();
    let (stop, close) = (at(&lines, "sup.stop_all().await"), at(&lines, "bus.close().await"));
    assert!(stop < close, "S-sym-RULE-BUS-FATE-1 shutdown order is sfu → bus → hub — the units stop before the bus closes (stop_all {stop} · bus.close {close})");
}
