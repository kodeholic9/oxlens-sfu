// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-WEBHOOK · 정:RULE-BUS-PUBLIC · 정:RULE-HOOK-FACTS-SUB · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! 1층 — webhook 운반을 가짜 수신기(평문 HTTP/1.1 · 시험이 응답 상태를 정한다)에 붙여 본다: 서명(Standard Webhooks 1.0.0 공개 시험 벡터) · 머리 셋 · `POST` · 몸 = 묶음 몸 · 다시 보내기에 같은 `webhook-id` · 시도마다 새 `webhook-timestamp` · 2xx 만 성공(3xx · 4xx · 5xx · 410 · 429 는 실패 · 따라가지 않음) · 시한 · 연결 거절 · 재시도 상한에서 버리고 `fact_given_up` 과 되돌려 세기 · 큐에 사실이 들면 깨어 보낸다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use bytes::Bytes;
use oxhubd::hook::Hook;
use oxhubd::hook::webhook::{Knobs, Sent, Target, Webhook, deliver, post, retry_after, run, sign, success, target, tls};
use oxhubd::hook::zgate::send::FactsBundle;
use serde_json::json;
use std::sync::atomic::Ordering;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
struct Seen {
    line: String,
    head: HashMap<String, String>,
    body: Vec<u8>,
}

async fn receiver(statuses: Vec<u16>, hang: bool) -> (u16, Arc<Mutex<Vec<Seen>>>) {
    let l = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = l.local_addr().expect("addr").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    tokio::spawn(async move {
        let mut i = 0usize;
        loop {
            let Ok((mut c, _)) = l.accept().await else { break };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            let (line, head, body) = loop {
                let n = c.read(&mut tmp).await.unwrap_or(0);
                if n == 0 {
                    break (String::new(), HashMap::new(), Vec::new());
                }
                buf.extend_from_slice(&tmp[..n]);
                let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                let text = String::from_utf8_lossy(&buf[..end]).to_string();
                let mut lines = text.split("\r\n");
                let line = lines.next().unwrap_or("").to_string();
                let head: HashMap<String, String> =
                    lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string())).collect();
                let len: usize = head.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
                while buf.len() < end + 4 + len {
                    let n = c.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                break (line, head, buf[end + 4..(end + 4 + len).min(buf.len())].to_vec());
            };
            s2.lock().expect("seen").push(Seen { line, head, body });
            if hang {
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }
            let st = statuses.get(i).copied().unwrap_or(200);
            i += 1;
            let _ = c.write_all(format!("HTTP/1.1 {st} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").as_bytes()).await;
            let _ = c.shutdown().await;
        }
    });
    (port, seen)
}

const SECRET: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";

fn hook() -> Arc<Hook> {
    Arc::new(Hook::new("hubA", vec![("acme".into(), vec!["*".into()])], 100))
}

fn webhook(port: u16) -> Webhook {
    Webhook::of(&common::system::Webhook { url: format!("http://127.0.0.1:{port}/in"), secret: SECRET.into(), key: "acme".into() }).expect("webhook")
}

fn knobs(retry_max: u8, timeout_ms: u64) -> Knobs {
    Knobs { backoff_ms: vec![10, 20, 40, 80, 160], retry_max, timeout: Duration::from_millis(timeout_ms), bundle_max_bytes: 65_536 }
}

fn bundle(h: &Hook, n: u64) -> FactsBundle {
    for t in 0..n {
        h.offer(h.build("r1", "participant_joined", json!({ "type": "joined", "user_id": format!("u{t}") }), 1_764_000_000_000 + t));
    }
    h.take("acme", 65_536).expect("bundle")
}

#[test]
fn signature_matches_the_standard_vector() {
    let key = common::system::webhook_secret_key(SECRET).expect("secret");
    assert_eq!(
        sign(&key, "msg_p5jXN8AQM9LWM0D4loKWxJek", 1_614_265_330, br#"{"test": 2432232314}"#),
        "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE=",
        "S-rule-RULE-HOOK-WEBHOOK-3①② — base64(HMAC-SHA256(whsec_ 뒤를 푼 키, id.timestamp.몸)) · Standard Webhooks 1.0.0 공개 시험 벡터"
    );
}

#[test]
fn only_2xx_succeeds() {
    for s in [200, 201, 204, 299] {
        assert!(success(s), "정:RULE-HOOK-WEBHOOK 판정 — {s} 성공");
    }
    for s in [199, 301, 302, 307, 308] {
        assert!(!success(s), "정:RULE-HOOK-WEBHOOK 판정 — 3xx 는 실패 · {s}");
    }
    for s in [400, 404, 500, 503] {
        assert!(!success(s), "정:RULE-HOOK-WEBHOOK 판정 — {s} 실패");
    }
    for s in [410, 429] {
        assert!(!success(s), "정:RULE-HOOK-WEBHOOK 들이지 않는 것 — {s} 도 「그 밖의 응답 = 실패」");
    }
}

#[test]
fn backoff_then_give_up() {
    let b = [1000, 2000, 4000, 8000, 16000];
    let got: Vec<Option<u64>> = (1..=6).map(|f| retry_after(f, &b, 5).map(|d| d.as_millis() as u64)).collect();
    assert_eq!(got, vec![Some(1000), Some(2000), Some(4000), Some(8000), Some(16000), None], "S-rule-RULE-HOOK-WEBHOOK-4① — 간격이 지수로 는다(retry_max 를 넘으면 더 기다리지 않는다)");
    assert_eq!(retry_after(1, &b, 0), None, "형상 — retry_max 0 이면 첫 실패에 버린다");
}

#[test]
fn urls() {
    assert_eq!(target("https://h.example/in?x=1"), Ok(Target { https: true, host: "h.example".into(), port: 443, path: "/in?x=1".into() }));
    assert_eq!(target("http://127.0.0.1:8080"), Ok(Target { https: false, host: "127.0.0.1".into(), port: 8080, path: "/".into() }));
    assert!(target("ftp://h/in").is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retries_carry_the_same_bundle() {
    let (port, seen) = receiver(vec![500, 302, 200], false).await;
    let h = hook();
    let b = bundle(&h, 2);
    let first_fact = b.facts[0].fact_id.clone();
    let ok = deliver(&h, &webhook(port), &knobs(5, 2_000), &tls(), b.clone()).await;
    assert!(ok, "S-rule-RULE-HOOK-WEBHOOK-9① — 2xx 면 끝(그 묶음은 다시 안 간다)");
    let seen = seen.lock().expect("seen").clone();
    assert_eq!(seen.len(), 3, "S-rule-RULE-HOOK-WEBHOOK-9②③ — 5xx · 3xx 는 실패 · 따라가지 않고 같은 재시도");
    let key = common::system::webhook_secret_key(SECRET).expect("secret");
    let id = seen[0].head.get("webhook-id").cloned().expect("webhook-id");
    for s in &seen {
        assert_eq!(s.line, "POST /in HTTP/1.1", "S-rule-RULE-HOOK-WEBHOOK-7① — POST");
        assert_eq!(s.head.get("content-type").map(String::as_str), Some("application/json"), "S-rule-RULE-HOOK-WEBHOOK-7② — content-type");
        assert_eq!(serde_json::from_slice::<FactsBundle>(&s.body).expect("bundle body"), b, "S-rule-RULE-HOOK-WEBHOOK-7③ · S-rule-RULE-HOOK-WEBHOOK-2① — 몸은 묶음 몸 그대로");
        assert_eq!(s.head.get("webhook-id"), Some(&id), "S-rule-RULE-HOOK-WEBHOOK-7④ · S-rule-RULE-HOOK-WEBHOOK-8① · S-rule-RULE-HOOK-WEBHOOK-4③ — 머리 webhook-id · 다시 보내기에도 같은 값");
        let ts: u64 = s.head.get("webhook-timestamp").and_then(|v| v.parse().ok()).expect("webhook-timestamp");
        assert!(ts > 1_700_000_000 && ts < 4_000_000_000, "정:RULE-HOOK-WEBHOOK 머리 — webhook-timestamp 는 UNIX 초");
        let sig = s.head.get("webhook-signature").cloned().expect("webhook-signature");
        assert!(sig.starts_with("v1,"), "S-rule-RULE-HOOK-WEBHOOK-8③ — v1,<서명>");
        assert_eq!(sig, sign(&key, &id, ts, &s.body), "S-rule-RULE-HOOK-WEBHOOK-3① — 수신기가 다시 계산한 서명과 같다");
        assert!(base64::engine::general_purpose::STANDARD.decode(&sig[3..]).is_ok());
    }
    assert_ne!(id, first_fact, "S-rule-RULE-HOOK-WEBHOOK-8① — 묶음 id 는 첫 사실의 fact_id 와 다르다");
    assert_eq!(h.given_up.load(Ordering::Relaxed), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn give_up_counts_and_hands_back_dropped() {
    let (port, seen) = receiver(vec![500, 500, 500, 500], false).await;
    let h = hook();
    let mut b = bundle(&h, 3);
    b.dropped = Some(5);
    let ok = deliver(&h, &webhook(port), &knobs(2, 2_000), &tls(), b).await;
    assert!(!ok, "S-rule-RULE-HOOK-WEBHOOK-4② — 상한을 넘으면 버린다");
    assert_eq!(seen.lock().expect("seen").len(), 3, "정:RULE-HOOK-WEBHOOK 재시도 — 처음 하나 + 재시도 retry_max");
    assert_eq!(h.given_up.load(Ordering::Relaxed), 3, "S-rule-RULE-HOOK-WEBHOOK-4② — fact_given_up 에 사실 수");
    let next = h.take("acme", 65_536).expect("next bundle");
    assert_eq!(next.dropped, Some(8), "S-rule-RULE-HOOK-WEBHOOK-4④ — 버린 묶음의 사실 수와 실었던 dropped 를 다음 묶음이 알린다");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_and_refused_are_failures() {
    let (port, _seen) = receiver(vec![], true).await;
    let t = target(&format!("http://127.0.0.1:{port}/in")).expect("target");
    let s = post(&t, &tls(), &[], Bytes::from_static(b"{}"), Duration::from_millis(200)).await;
    assert_eq!(s, Sent::Timeout, "S-rule-RULE-HOOK-WEBHOOK-9④ · PL-KEY-HOOK-WEBHOOK-TIMEOUT-MS① — 시한을 넘기면 실패");
    let dead = TcpListener::bind("127.0.0.1:0").await.expect("bind").local_addr().expect("addr").port();
    let t = target(&format!("http://127.0.0.1:{dead}/in")).expect("target");
    let s = post(&t, &tls(), &[], Bytes::from_static(b"{}"), Duration::from_millis(500)).await;
    assert!(matches!(s, Sent::Failed(_)), "S-rule-RULE-HOOK-WEBHOOK-9⑤ — 연결 거절은 실패: {s:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_wakes_on_new_facts() {
    let (port, seen) = receiver(vec![], false).await;
    let h = hook();
    tokio::spawn(run(h.clone(), webhook(port), knobs(1, 2_000), tls()));
    tokio::time::sleep(Duration::from_millis(50)).await;
    h.offer(h.build("r1", "room_created", json!({ "room_id": "r1" }), 1));
    let until = tokio::time::Instant::now() + Duration::from_millis(800);
    while seen.lock().expect("seen").is_empty() && tokio::time::Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let s = seen.lock().expect("seen").clone();
    assert_eq!(s.len(), 1, "S-rule-RULE-HOOK-WEBHOOK-2② — 요청 하나 = 묶음 하나 · 큐에 들면 깨어 보낸다(1 초 훑기 전에)");
    let b: FactsBundle = serde_json::from_slice(&s[0].body).expect("body");
    assert_eq!((b.facts.len(), b.dropped), (1, None), "S-rule-RULE-HOOK-FACTS-SUB-5③ — 버린 것이 없으면 dropped 칸 없음");
}
