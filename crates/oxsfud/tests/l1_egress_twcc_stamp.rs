// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-RTCP-TERMINATE · 연:RULE-LINK-EGRESS-REWRITE · model: claude-opus-5-5
//! 1층 소스 가드 · 순수 함수 — 다운링크 twcc 일련번호는 서버가 찍는다: 구독 전송로 계수기에서 한 칸씩 떼어 헤더 확장을 다시 짓고(확장 짓기는 그 값 · mid · 음량 · 보낸 시각만 받는다), 발행자 패킷의 헤더(확장 영역 포함)는 옮겨 싣지 않는다 — 본문만 싣는다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use oxsfud::sub::ext::{ExtInput, build};
use oxsfud::sub::{ext_table, server_extmap};

use support::*;

fn ext_value(out: &[u8], id: u8) -> Option<Vec<u8>> {
    if out.len() < 4 || out[0] != 0xbe || out[1] != 0xde {
        return None;
    }
    let words = u16::from_be_bytes([out[2], out[3]]) as usize;
    let mut i = 4;
    let end = 4 + words * 4;
    while i < end && i < out.len() {
        let b = out[i];
        if b == 0 {
            i += 1;
            continue;
        }
        let (eid, len) = (b >> 4, (b & 0x0f) as usize + 1);
        if eid == id {
            return Some(out[i + 1..i + 1 + len].to_vec());
        }
        i += 1 + len;
    }
    None
}

#[test]
fn the_downlink_twcc_number_is_the_servers_own() {
    let table = ext_table(&server_extmap());
    let id = table.twcc.expect("stand check — the server extmap carries transport-cc");
    let mut out = Vec::new();
    let n = build(&table, &ExtInput { mid: b"3", audio_level: None, t_us: 0, twcc_seq: Some(0x1234) }, &mut out);
    assert!(n > 0, "stand check — an extension block was written");
    assert_eq!(ext_value(&out, id), Some(vec![0x12, 0x34]), "S-sym-IMPL-RTCP-TERMINATE-5② the twcc extension carries the server's number handed to the builder");
    let lines = product_lines();
    assert!(lines.len() > 1_000, "stand check — the product sources were read ({})", lines.len());
    let down: Vec<&Src> = lines.iter().filter(|l| l.file == "fwd/mod.rs" && l.func == "send_down").collect();
    let has = |s: &str| down.iter().any(|l| l.text.contains(s));
    assert!(has("let twcc = conn.twcc_seq.fetch_add(1, Ordering::AcqRel);"), "S-sym-IMPL-RTCP-TERMINATE-5② the number is taken from the subscriber transport counter");
    assert!(has("twcc_seq: Some(twcc)"), "S-sym-IMPL-RTCP-TERMINATE-5② that counter value is what the builder stamps");
    let copies: Vec<String> = down.iter().filter(|l| l.text.contains("extend_from_slice(&pkt.data")).map(|l| l.text.trim().to_string()).collect();
    assert_eq!(copies, vec!["out.extend_from_slice(&pkt.data[pkt.hlen..]);".to_string()], "S-sym-IMPL-RTCP-TERMINATE-5② only the publisher's payload is carried — its header and extension area (with its twcc number) are not");
}
