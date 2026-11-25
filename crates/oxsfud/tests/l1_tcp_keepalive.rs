// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-FWD-MEDIA-WORKERS · 정:RULE-LINK-TRANSPORT · model: claude-opus-5-5
//! 1층 상수 대조 · 소스 가드 — TCP 전송로의 keepalive 는 20,000ms 마다 RFC 4571 길이 0 프레임이다: 주기 상수 · 빈 본문의 프레임 꼴 · 워커 tick 이 그 주기로 빈 본문을 TCP 연결에 보낸다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use support::*;

#[test]
fn tcp_keepalive_is_a_zero_length_frame_every_twenty_seconds() {
    assert_eq!(oxsfud::link::tcp::KEEPALIVE_MS, 20_000, "S-sym-RULE-FWD-MEDIA-WORKERS-11③ the TCP keepalive period is 20,000ms");
    assert_eq!(oxsfud::link::tcp::frame(&[]), Some(vec![0, 0]), "S-sym-RULE-FWD-MEDIA-WORKERS-11③ the keepalive is an RFC 4571 frame of length 0");
    let lines = product_lines();
    assert!(lines.len() > 1_000, "stand check — the product sources were read ({})", lines.len());
    let tick: Vec<String> = lines.iter().filter(|l| l.file == "rtcp/mod.rs" && l.text.contains("due(&tr.rtc.next_keepalive, now, crate::link::tcp::KEEPALIVE_MS)")).map(at).collect();
    assert_eq!(tick.len(), 1, "S-sym-RULE-FWD-MEDIA-WORKERS-11③ the worker tick fires the keepalive on the 20s period: {tick:?}");
    let sends: Vec<String> = lines.iter().filter(|l| l.file == "rtcp/mod.rs" && l.func == "keepalive" && l.text.contains(".send(&[])")).map(at).collect();
    assert_eq!(sends.len(), 1, "S-sym-RULE-FWD-MEDIA-WORKERS-11③ the keepalive sends an empty body on each TCP path: {sends:?}");
    let framed: Vec<String> = lines.iter().filter(|l| l.file == "fwd/rt.rs" && l.text.contains("tcp::frame(payload)")).map(at).collect();
    assert_eq!(framed.len(), 1, "S-sym-RULE-FWD-MEDIA-WORKERS-11③ a TCP send always frames its body: {framed:?}");
}
