// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-WEBHOOK · 정:RULE-BUS-PUBLIC · 정:RULE-HOOK-FACTS-SUB · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! 1층 — webhook 운반을 가짜 수신기(평문 HTTP/1.1 · 시험이 응답 상태를 정한다 · https 는 자가 서명 수신기)에 붙여 본다: 서명(Standard Webhooks 참조 라이브러리 python `libraries/python/tests/test_webhooks.py` 의 시험 값 — 표준 본문의 예가 아니다) · 머리 셋 · `POST` · 몸 = 묶음 몸 · 다시 보내기에 같은 `webhook-id` · 시도마다 새 `webhook-timestamp` · 2xx 만 성공(3xx · 4xx · 5xx · 410 · 429 는 실패 · 따라가지 않음) · 시한 · 연결 거절 · 재시도 상한에서 버리고 `fact_given_up` 과 되돌려 세기 · 큐에 사실이 들면 깨어 보낸다.

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
        "S-rule-RULE-HOOK-WEBHOOK-3①② — base64(HMAC-SHA256(whsec_ 뒤를 푼 키, id.timestamp.몸)) · Standard Webhooks 참조 라이브러리(python tests/test_webhooks.py) 시험 값"
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
    assert_eq!(got, vec![Some(1000), Some(2000), Some(4000), Some(8000), Some(16000), None], "S-rule-RULE-HOOK-WEBHOOK-4① · PL-KEY-HOOK-RETRY-BACKOFF-MS① — 간격이 배열대로(지수로) 는다(retry_max 를 넘으면 더 기다리지 않는다)");
    assert_eq!(retry_after(1, &b, 0), None, "형상 — retry_max 0 이면 첫 실패에 버린다");
}

#[test]
fn urls() {
    assert_eq!(target("https://h.example/in?x=1"), Ok(Target { https: true, host: "h.example".into(), port: 443, path: "/in?x=1".into() }));
    assert_eq!(target("http://127.0.0.1:8080"), Ok(Target { https: false, host: "127.0.0.1".into(), port: 8080, path: "/".into() }));
    assert!(target("ftp://h/in").is_err());
    let v6 = target("http://[::1]:8080/in").expect("v6");
    assert_eq!(v6.host, "::1");
    assert_eq!(oxhubd::hook::webhook::host_header(&v6), "[::1]:8080", "정:RULE-HOOK-WEBHOOK 꼴 — IPv6 Host 머리는 대괄호(RFC 9110 §7.2 · RFC 3986 IP-literal)");
    assert_eq!(oxhubd::hook::webhook::host_header(&target("https://[::1]/in").expect("v6")), "[::1]");
    assert_eq!(oxhubd::hook::webhook::host_header(&target("http://h:8080/").expect("h")), "h:8080");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn https_checks_the_system_trust_store() {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let k = rcgen::KeyPair::generate().expect("key");
    let cert = rcgen::CertificateParams::new(vec!["localhost".to_string()]).expect("params").self_signed(&k).expect("self signed");
    let cfg = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], tokio_rustls::rustls::pki_types::PrivateKeyDer::try_from(k.serialize_der()).expect("key der"))
        .expect("server cfg");
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    let l = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = l.local_addr().expect("addr").port();
    let tries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let t2 = tries.clone();
    tokio::spawn(async move {
        while let Ok((c, _)) = l.accept().await {
            t2.fetch_add(1, Ordering::Relaxed);
            let a = acceptor.clone();
            tokio::spawn(async move {
                let _ = a.accept(c).await;
            });
        }
    });
    let h = hook();
    let b = bundle(&h, 1);
    let w = Webhook::of(&common::system::Webhook { url: format!("https://localhost:{port}/in"), secret: SECRET.into(), key: "acme".into() }).expect("webhook");
    let ok = deliver(&h, &w, &knobs(1, 2_000), &tls(), b).await;
    assert!(!ok, "S-rule-RULE-HOOK-WEBHOOK-11② — 시스템 신뢰 저장소에 없는 CA(자가 서명)는 TLS 검증 실패 = 실패");
    assert_eq!(tries.load(Ordering::Relaxed), 2, "S-rule-RULE-HOOK-WEBHOOK-11② — 같은 재시도를 탄다(처음 하나 + retry_max 1)");
    assert_eq!(h.given_up.load(Ordering::Relaxed), 1, "형상 — 상한에서 버리고 센다");
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
    assert_eq!(h.given_up.load(Ordering::Relaxed), 3, "S-rule-RULE-HOOK-WEBHOOK-4② · PL-KEY-HOOK-RETRY-MAX② — retry_max 를 넘으면 버리고 fact_given_up 에 사실 수");
    let next = h.take("acme", 65_536).expect("next bundle");
    assert_eq!(next.dropped, Some(8), "S-rule-RULE-HOOK-WEBHOOK-4④ · PL-KEY-HOOK-RETRY-MAX② — 버린 묶음의 사실 수와 실었던 dropped 를 다음 묶음이 알린다(버림 표지에 더한다)");
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

async fn read_one(c: &mut tokio::net::TcpStream) -> Option<Seen> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = c.read(&mut tmp).await.unwrap_or(0);
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
        let text = String::from_utf8_lossy(&buf[..end]).to_string();
        let mut lines = text.split("\r\n");
        let line = lines.next().unwrap_or("").to_string();
        let head: HashMap<String, String> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string())).collect();
        let len: usize = head.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
        while buf.len() < end + 4 + len {
            let n = c.read(&mut tmp).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        return Some(Seen { line, head, body: buf[end + 4..(end + 4 + len).min(buf.len())].to_vec() });
    }
}

type Arrivals = Arc<Mutex<Vec<(std::time::Instant, Seen)>>>;

async fn responder(steps: Vec<(u64, u16, &'static str)>) -> (u16, Arrivals) {
    let l = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = l.local_addr().expect("addr").port();
    let seen: Arrivals = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    tokio::spawn(async move {
        let mut i = 0usize;
        loop {
            let Ok((mut c, _)) = l.accept().await else { break };
            let Some(req) = read_one(&mut c).await else { continue };
            s2.lock().expect("seen").push((std::time::Instant::now(), req));
            let (delay, st, extra) = steps.get(i).copied().unwrap_or((0, 200, ""));
            i += 1;
            tokio::time::sleep(Duration::from_millis(delay)).await;
            let _ = c.write_all(format!("HTTP/1.1 {st} X\r\n{extra}content-length: 0\r\nconnection: close\r\n\r\n").as_bytes()).await;
            let _ = c.shutdown().await;
        }
    });
    (port, seen)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_response_inside_the_timeout_decides() {
    let (port, _seen) = responder(vec![(150, 503, ""), (150, 204, ""), (150, 200, "")]).await;
    let t = target(&format!("http://127.0.0.1:{port}/in")).expect("target");
    let late_fail = post(&t, &tls(), &[], Bytes::from_static(b"{}"), Duration::from_millis(1_000)).await;
    assert_eq!(late_fail, Sent::Status(503), "PL-KEY-HOOK-WEBHOOK-TIMEOUT-MS② — 값(1000 ms) 안에 온 응답은 150 ms 늦어도 그 응답(503 = 실패)으로 판정");
    let late_ok = post(&t, &tls(), &[], Bytes::from_static(b"{}"), Duration::from_millis(1_000)).await;
    assert!(late_ok == Sent::Status(204) && late_ok.ok(), "PL-KEY-HOOK-WEBHOOK-TIMEOUT-MS② — 값 안에 온 2xx 는 150 ms 늦어도 성공으로 판정 {late_ok:?}");
    let over = post(&t, &tls(), &[], Bytes::from_static(b"{}"), Duration::from_millis(50)).await;
    assert_eq!(over, Sent::Timeout, "positive control — 같은 늦음(150 ms)도 값(50 ms)을 넘기면 시한 넘김");
}

fn verify(key: &[u8], s: &Seen, now: u64, tolerance: u64) -> bool {
    use hmac::Mac;
    let (Some(id), Some(ts), Some(sigs)) = (s.head.get("webhook-id"), s.head.get("webhook-timestamp").and_then(|v| v.parse::<u64>().ok()), s.head.get("webhook-signature")) else {
        return false;
    };
    if now.abs_diff(ts) > tolerance {
        return false;
    }
    let Ok(mut m) = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(key) else { return false };
    m.update(format!("{id}.{ts}.").as_bytes());
    m.update(&s.body);
    let want = format!("v1,{}", base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes()));
    sigs.split(' ').any(|one| one == want)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_receiver_can_drop_unsigned_and_replayed_requests() {
    let (port, seen) = receiver(vec![200], false).await;
    let h = hook();
    let b = bundle(&h, 1);
    assert!(deliver(&h, &webhook(port), &knobs(1, 2_000), &tls(), b).await, "형상 — 보냈다");
    let s = seen.lock().expect("seen")[0].clone();
    let key = common::system::webhook_secret_key(SECRET).expect("secret");
    let now = common::now_ms() / 1000;
    let ts: u64 = s.head.get("webhook-timestamp").and_then(|v| v.parse().ok()).expect("webhook-timestamp");
    assert!(now.abs_diff(ts) <= 2, "S-rule-RULE-HOOK-WEBHOOK-3④ — webhook-timestamp 는 보낸 시각이다({ts} · 지금 {now})");
    assert!(verify(&key, &s, now, 300), "positive control — 우리가 보낸 요청은 수신기 검증(Standard Webhooks 대칭 v1 · 허용 오차 5 분)을 지난다");
    let mut unsigned = s.clone();
    unsigned.head.remove("webhook-signature");
    assert!(!verify(&key, &unsigned, now, 300), "형상 — 시험 수신기는 서명 없는 요청을 거른다");
    let mut forged = s.clone();
    forged.body = br#"{"facts":[]}"#.to_vec();
    assert!(!verify(&key, &forged, now, 300), "형상 — 시험 수신기는 몸을 바꾼 요청을 거른다");
    assert!(!verify(&key, &s, ts + 600, 300), "S-rule-RULE-HOOK-WEBHOOK-3④ — 10 분 뒤 그대로 되쏘면 발신 시각이 허용 오차를 넘어 걸린다");
    let mut refreshed = s.clone();
    refreshed.head.insert("webhook-timestamp".into(), (ts + 600).to_string());
    assert!(!verify(&key, &refreshed, ts + 600, 300), "S-rule-RULE-HOOK-WEBHOOK-3④ — 시각만 새로 바꿔 되쏘면 서명이 시각을 덮어 맞지 않는다");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gone_too_many_and_retry_after_are_plain_failures() {
    let (port, seen) = responder(vec![(0, 410, ""), (0, 429, "retry-after: 30\r\n"), (0, 503, "retry-after: 30\r\n"), (0, 200, "")]).await;
    let h = hook();
    let b = bundle(&h, 1);
    let ok = deliver(&h, &webhook(port), &knobs(5, 2_000), &tls(), b).await;
    let at: Vec<std::time::Instant> = seen.lock().expect("seen").iter().map(|(t, _)| *t).collect();
    assert!(ok && at.len() == 4, "S-rule-RULE-HOOK-WEBHOOK-10① — 410 도 「그 밖의 응답 = 실패」로 같은 재시도를 탄다(넷째에 2xx · 요청 {})", at.len());
    let gaps: Vec<Duration> = at.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps[0] >= Duration::from_millis(10), "S-rule-RULE-HOOK-WEBHOOK-10① — 410 뒤 첫 백오프(10 ms) 뒤 다시 보낸다 {gaps:?}");
    assert!(
        gaps[1] >= Duration::from_millis(20) && gaps[1] < Duration::from_secs(5),
        "S-rule-RULE-HOOK-WEBHOOK-10② — 429 + retry-after 30 은 그 머리를 따르지 않고 백오프 간격(20 ms)으로 다시 보낸다 {gaps:?}"
    );
    assert!(
        gaps[2] >= Duration::from_millis(40) && gaps[2] < Duration::from_secs(5),
        "S-rule-RULE-HOOK-WEBHOOK-10③ — 5xx + retry-after 30 도 그 머리를 따르지 않고 백오프 간격(40 ms)으로 다시 보낸다 {gaps:?}"
    );
    let (port2, seen2) = responder(vec![(0, 410, ""); 400]).await;
    let h2 = hook();
    tokio::spawn(run(h2.clone(), webhook(port2), knobs(1, 2_000), tls()));
    h2.offer(h2.build("r1", "room_created", json!({ "room_id": "r1" }), 1));
    assert!(wait_given_up(&h2, 1).await, "형상 — 410 만 받은 첫 묶음은 재시도 상한(1)에서 버렸다");
    let kinds = |s: &Arrivals| -> Vec<String> {
        s.lock()
            .map(|g| g.iter().filter_map(|(_, x)| serde_json::from_slice::<FactsBundle>(&x.body).ok()).flat_map(|b| b.facts.into_iter().map(|f| f.kind)).collect())
            .unwrap_or_default()
    };
    h2.offer(h2.build("r1", "room_closed", json!({ "room_id": "r1" }), 2));
    let until = tokio::time::Instant::now() + Duration::from_secs(2);
    while !kinds(&seen2).iter().any(|k| k == "room_closed") && tokio::time::Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(kinds(&seen2).iter().any(|k| k == "room_closed"), "S-rule-RULE-HOOK-WEBHOOK-10① — 410 을 받은 수신처를 끄지 않는다(다음 묶음도 그 수신처로 간다)");
}

async fn wait_given_up(h: &Hook, n: u64) -> bool {
    let until = tokio::time::Instant::now() + Duration::from_secs(2);
    while h.given_up.load(Ordering::Relaxed) < n && tokio::time::Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    h.given_up.load(Ordering::Relaxed) >= n
}

#[test]
fn there_is_no_slot_for_a_webhook_ca() {
    let base = format!(
        "[hub]\nlisten = \"0.0.0.0:1974\"\n[bus]\nnamespace = \"ox\"\n[[hub.auth.api_keys]]\nkey = \"acme\"\nsecret = \"s\"\nhook_allowed = true\nfacts = [\"*\"]\n[[hook.webhooks]]\nurl = \"https://h.example/in\"\nsecret = \"{SECRET}\"\nkey = \"acme\"\n"
    );
    assert!(common::system::System::parse(&base).is_ok(), "positive control — https 수신처 하나가 선다");
    for extra in ["ca = \"/etc/ca.pem\"\n", "root_ca = \"/etc/ca.pem\"\n", "ca_file = \"/etc/ca.pem\"\n"] {
        assert!(
            common::system::System::parse(&format!("{base}{extra}")).is_err(),
            "S-rule-RULE-HOOK-WEBHOOK-11③ — 수신처마다 CA 를 두는 칸이 없다(모르는 키 = 기동 실패) {extra:?}"
        );
    }
    assert!(common::system::System::parse(&format!("[hook]\nfacts_bundle_max_bytes = 1024\n{base}")).is_ok(), "positive control — [hook] 표의 아는 키는 선다");
    assert!(
        common::system::System::parse(&format!("[hook]\nwebhook_ca = \"/etc/ca.pem\"\n{base}")).is_err(),
        "S-rule-RULE-HOOK-WEBHOOK-11③ — [hook] 에도 webhook CA 칸이 없다(모르는 키 = 기동 실패)"
    );
}
