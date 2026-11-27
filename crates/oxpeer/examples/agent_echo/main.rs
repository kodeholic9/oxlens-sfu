// author: kodeholic (powered by Claude)
//! 예제 에이전트(context 202611/20261107c C9) — 서버사이드 SDK(oxpeer)로 방에 봇 참가자(`participant_type:2`)로 들어가 ★말한다.
//! `echo`(기본) — 무전방 되울림: 화자의 발언(무전 슬롯 오디오)을 받아 두었다가 그가 놓으면 발언권을 눌러 허가 사이에 그대로 되돌려 말하고 놓는다.
//!   받기 · 누르기 · 말하기(반이중 발행 · 송신 시계) · 놓기를 한 판에 다 지난다. 출처가 없는 패킷은 담지 않는다.
//! `beacon` — 전이중 발행 시험판: 표지 프레임(오디오 20ms · 영상 30fps · 2층 봇과 같은 본문 꼴 — 보낸 표지 · 번호)을 낸다. 키프레임 요청(PLI)이 오면 다음 영상 프레임을 키프레임으로.
//!   ★표지 프레임은 코덱 본문이 아니다(서버는 본문을 읽지 않는다) — 2층 시험이 「보낸 것 = 받은 것」을 견주는 재료다.
//!   `--simulcast` 면 영상 업링크 둘(층 표지 `l` · `h` — 층마다 따로 센다) · 목표 비트레이트 사건(`TargetBitrate`)을 요약에 적는다.
//! 아래 셋은 표지를 내며 정해진 때 한 걸음씩 밟는다(context 202611/20261108d — 짰지만 2층 형상이 없던 길) · 걸음마다 벽시계 ms 를 요약 `steps` 에 적는다.
//! `duplex` — 전이중으로 내다가 2s 반이중 · 3s 누름 · 5s 놓음 · 6s 전이중(`LocalTrack::set_duplex`).
//! `move` — `--room`(발언 방 · 반이중) + `--room2`(듣기)로 들어 2s 발언 방을 room2 로(서버를 건너면 옛 서버 내림 · 새 서버 발행) · 3s 누름 · 5s 놓음 · 6s room 으로 되돌림 · 7s 누름 · 9s 놓음.
//! `reconnect` — 전이중 표지 · `--reconnect-at N` 초에 `close()` 한 뒤 ★같은 객체로 `connect()` · 다시 들어 다시 발행(SDK:ses.md `close()`).
//! `file` — 진짜 코덱 표본을 되풀이해 전이중으로 낸다(`--opus a.ogg` · `--h264 v.h264` 또는 `--vp8 v.ivf` · 영상 30fps) · 키프레임 요청이 오면 다음 키프레임으로 건너뛴다(3층 — 브라우저가 실제로 디코드하나).
//!
//!   cargo run -p oxpeer --example agent_echo -- --base http://127.0.0.1:1974/media --room ch-1 [--secs 30] [--user agent-1] [--mode echo|beacon|duplex|move|reconnect] [--video [--simulcast]] [--room2 ch-2] [--reconnect-at N] [--tls-root ca.pem]
//!
//! 끝나면 표준 출력에 한 줄 JSON(되울림마다 · 발언권 사건 · 키프레임 요청 · 보내는 쪽 계수)을 쓴다 — 2층 시험이 읽는다.

mod media;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use oxpeer::{Client, Config, Duplex, Event, Kind, LocalTrack, OutFrame, TokenReq, TrackSpec};
use serde_json::{Value, json};

struct Args {
    base: String,
    room: String,
    secs: Option<u64>,
    user: String,
    mode: String,
    video: bool,
    simulcast: bool,
    room2: String,
    reconnect_at: u64,
    tls_root: Option<String>,
    opus: Option<String>,
    h264: Option<String>,
    vp8: Option<String>,
    api_key: String,
    api_secret: String,
}

fn args() -> Args {
    let mut a = Args { base: String::new(), room: String::new(), secs: None, user: "oxpeer-agent".into(), mode: "echo".into(), video: false, simulcast: false, room2: String::new(), reconnect_at: 3, tls_root: None, opus: None, h264: None, vp8: None, api_key: "ox_k_demo".into(), api_secret: "ox_s_demo".into() };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().unwrap_or_default();
        match k.as_str() {
            "--base" => a.base = v(),
            "--room" => a.room = v(),
            "--secs" => a.secs = v().parse().ok(),
            "--user" => a.user = v(),
            "--mode" => a.mode = v(),
            "--video" => a.video = true,
            "--simulcast" => a.simulcast = true,
            "--room2" => a.room2 = v(),
            "--reconnect-at" => a.reconnect_at = v().parse().unwrap_or(3),
            "--tls-root" => a.tls_root = Some(v()),
            "--opus" => a.opus = Some(v()),
            "--h264" => a.h264 = Some(v()),
            "--vp8" => a.vp8 = Some(v()),
            "--api-key" => a.api_key = v(),
            "--api-secret" => a.api_secret = v(),
            other => eprintln!("모르는 인자 {other}"),
        }
    }
    if a.base.is_empty() || a.room.is_empty() {
        eprintln!("사용: agent --base http://host:port/media --room <방> [--secs N] [--user id] [--mode echo|beacon|duplex|move|reconnect] [--video [--simulcast]] [--room2 방] [--reconnect-at N] [--tls-root ca.pem]");
        std::process::exit(2);
    }
    a
}

const OPUS_20MS: Duration = Duration::from_millis(20);
const SILENCE: [u8; 3] = [0xF8, 0xFF, 0xFE];
/// 표지 본문의 「보낸 이」 — 에이전트는 자기 ssrc 를 모른다(SDK 몫) · 표지 쪽 고정값
const BEACON_TAG_AUDIO: u32 = 0xA6E7_0001;
const BEACON_TAG_VIDEO: u32 = 0xA6E7_0002;

fn beacon_audio(n: u32) -> Bytes {
    let mut b = SILENCE.to_vec();
    b.extend(BEACON_TAG_AUDIO.to_be_bytes());
    b.extend((n as u16).to_be_bytes());
    b.extend([0u8; 14]);
    Bytes::from(b)
}

/// 영상 표지 — 층 표지 바이트(시뮬캐스트 `l` · `h` · 아니면 0) 뒤에 (표지, 번호) · 코덱 꼴로 싼다(2층 봇 `video_marked` 와 같은 꼴)
/// VP8 = 키 표시 바이트로 시작하는 프레임 · H264 = Annex-B NAL 하나(머리 IDR 0x65 · 비IDR 0x41 · 본문은 에뮬레이션 방지 · 채움 AA)
fn beacon_video(n: u32, key: bool, layer: u8, codec: &str) -> Bytes {
    let mut mark = vec![layer];
    mark.extend(BEACON_TAG_VIDEO.to_be_bytes());
    mark.extend(n.to_be_bytes());
    if codec.eq_ignore_ascii_case("H264") {
        let mut b = vec![0, 0, 0, 1, if key { 0x65 } else { 0x41 }];
        let mut zeros = 0;
        for x in mark {
            if zeros >= 2 && x <= 3 {
                b.push(3);
                zeros = 0;
            }
            b.push(x);
            zeros = if x == 0 { zeros + 1 } else { 0 };
        }
        b.extend([0xAAu8; 50]);
        return Bytes::from(b);
    }
    let mut b = vec![if key { 0x10 } else { 0x11 }];
    b.extend(mark);
    b.extend([0u8; 51]);
    Bytes::from(b)
}

fn wall_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[tokio::main]
async fn main() {
    let a = args();
    let req = TokenReq { api_key: a.api_key.clone(), api_secret: a.api_secret.clone(), user_id: a.user.clone(), participant_type: Some(2), hidden: Some(false), metadata: None, permission: None };
    let root = a.tls_root.as_ref().map(|p| std::fs::read(p).expect("tls 뿌리 PEM"));
    let token = match oxpeer::issue_token_with_root(&a.base, &req, root.as_deref()).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("토큰 발급 실패: {e}");
            std::process::exit(1);
        }
    };
    let mut cfg = Config::new(&a.base, token);
    cfg.tls_root_pem = root;
    let (client, mut events, mut media, _frames) = match Client::create(cfg) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("만들기 실패: {e}");
            std::process::exit(1);
        }
    };
    // 전이 통(SDK:RULE-DIAG-EVENTS `transition`) — 걸음 수 · 받지 않는 사건 수 · 라벨 가짓수를 요약에 싣는다(20261109)
    let diag = std::sync::Arc::new(std::sync::Mutex::new((0u64, 0u64, std::collections::BTreeSet::<String>::new())));
    {
        let mut tr = client.transitions();
        let d = diag.clone();
        tokio::spawn(async move {
            while let Some(t) = tr.next().await {
                if let Ok(mut g) = d.lock() {
                    g.0 += 1;
                    if t.impossible {
                        g.1 += 1;
                    }
                    if let Some(l) = t.label {
                        g.2.insert(l);
                    }
                }
            }
        });
    }
    if let Err(e) = client.connect().await {
        eprintln!("접속 실패: {e}");
        std::process::exit(1);
    }
    if let Err(e) = client.join_speaking(&a.room).await {
        eprintln!("입장 실패: {e}");
        std::process::exit(1);
    }
    if a.mode == "move" && let Err(e) = client.join(&a.room2).await {
        eprintln!("둘째 방 입장 실패: {e}");
        std::process::exit(1);
    }
    let full = matches!(a.mode.as_str(), "beacon" | "duplex" | "reconnect" | "file");
    let (audio, video) = match publish_all(&client, &a, full).await {
        Ok(x) => x,
        Err(e) => {
            eprintln!("발행 실패: {e}");
            std::process::exit(1);
        }
    };
    let me = client.snapshot().await.ok().and_then(|s| s.session.and_then(|x| x.user_id)).unwrap_or_default();
    eprintln!("[agent_echo] {} 에 들어왔다({}) — 발행 {}", a.room, a.mode, audio.track_id);
    let secs = a.secs;
    let stop = async {
        match secs {
            Some(s) => tokio::time::sleep(Duration::from_secs(s)).await,
            None => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    };
    tokio::pin!(stop);
    let mut out = json!({ "room": a.room, "room2": a.room2, "mode": a.mode, "user": me, "audio_track": audio.track_id, "video_track": video.as_ref().map(|v| v.track_id.clone()), "simulcast": a.simulcast, "start_wall_ms": wall_ms() });
    if a.mode == "echo" {
        echo(&client, &mut events, &mut media, audio, &me, &a.room, &mut stop, &mut out).await;
    } else if a.mode == "file" {
        file(&a, &mut events, audio, video, &mut stop, &mut out).await;
    } else {
        beacon(&client, &a, &mut events, audio, video, &mut stop, &mut out).await;
    }
    let st = client.stats().await;
    if let Some(o) = out.as_object_mut() {
        if let Ok(g) = diag.lock() {
            o.insert("transitions".into(), json!({ "lines": g.0, "impossible": g.1, "labels": g.2.len() }));
        }
        o.insert("stats".into(), json!({ "frames_sent": st.frames_sent, "frames_gated": st.frames_gated, "rtx_sent": st.rtx_sent, "remb_bps": st.remb_bps, "qa_send_dropped": st.qa_send_dropped, "media_dropped": st.media_dropped, "target_bps": st.target_bps, "send_loss_permille": st.send_loss_permille }));
    }
    client.close().await;
    println!("{out}");
}

/// 오디오(전이중 · 반이중) · 영상(`--video` · 전이중 · `--simulcast`) 발행
async fn publish_all(client: &Client, a: &Args, full: bool) -> Result<(LocalTrack, Option<LocalTrack>), oxpeer::Error> {
    let duplex = if full { Duplex::Full } else { Duplex::Half };
    let audio = client.publish(&a.room, TrackSpec { kind: Kind::Audio, duplex, simulcast: false }).await?;
    let video = if full && (a.video || a.h264.is_some() || a.vp8.is_some()) { Some(client.publish(&a.room, TrackSpec { kind: Kind::Video, duplex: Duplex::Full, simulcast: a.simulcast }).await?) } else { None };
    Ok((audio, video))
}

/// 되울림 — 남의 발언을 받아 두고 · 그가 놓으면(화자 없음) 눌러서 허가 뒤 그대로 말하고 놓는다
#[allow(clippy::too_many_arguments)]
async fn echo(client: &Client, events: &mut oxpeer::Events, media: &mut oxpeer::Media, track: LocalTrack, me: &str, room: &str, stop: &mut std::pin::Pin<&mut impl std::future::Future<Output = ()>>, out: &mut Value) {
    let mut speaker: Option<String> = None;
    let mut heard: Vec<Bytes> = Vec::new();
    // 들은 패킷의 출처 주인(SDK:RULE-PLAT-SERVER 「출처 표지」) — 「누가 말했나」를 화자 사건이 아니라 패킷이 알린다
    let mut heard_from: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut echoes: Vec<Value> = Vec::new();
    let mut ptt: Vec<Value> = Vec::new();
    let t0 = Instant::now();
    loop {
        tokio::select! {
            _ = &mut *stop => break,
            e = events.next() => {
                let Some(e) = e else { break };
                match e {
                    Event::Speaker { room: r, user_id } if r == room => {
                        let was = speaker.take();
                        speaker = user_id.filter(|u| u != me);
                        // 남의 발언이 끝났다 — 되울린다
                        if speaker.is_none() && let Some(from) = was && !heard.is_empty() {
                            let packets = std::mem::take(&mut heard);
                            let sources: Vec<String> = std::mem::take(&mut heard_from).into_iter().collect();
                            let at = t0.elapsed().as_millis() as u64;
                            let granted = press_and_wait(client, events, room, &mut ptt, t0).await;
                            let mut sent = 0u32;
                            if granted {
                                for p in &packets {
                                    if track.write(OutFrame { data: p.clone(), duration: OPUS_20MS, keyframe: true, audio_level: None }).await.is_err() {
                                        break;
                                    }
                                    sent += 1;
                                }
                                // 송신 시계가 다 내보낼 때까지(통에 쌓인 만큼) 기다렸다 놓는다
                                track.drain().await;
                            }
                            let _ = client.ptt_release(room).await;
                            echoes.push(json!({ "from": from, "sources": sources, "heard": packets.len(), "sent": sent, "granted": granted, "at_ms": at, "released_ms": t0.elapsed().as_millis() as u64 }));
                        }
                    }
                    Event::Ptt { room: r, event, ptt: info } if r == room => ptt.push(ptt_json(&event, &info, json!({ "ms": t0.elapsed().as_millis() as u64 }))),
                    Event::Closed { .. } => break,
                    _ => {}
                }
            }
            p = media.next() => {
                let Some(p) = p else { break };
                // 무전 슬롯 오디오(주인 없음)만 · 남이 말하는 동안만 · ★출처가 남인 패킷만(출처 없음은 담지 않는다)
                let from = p.source.as_ref().and_then(|s| s.user_id.clone());
                if p.room == room && p.kind == "audio" && p.user_id.is_none() && speaker.is_some() && p.source.is_some() && from.as_deref() != Some(me) {
                    if let Some(u) = from {
                        heard_from.insert(u);
                    }
                    heard.push(p.payload.clone());
                }
            }
        }
    }
    if let Some(o) = out.as_object_mut() {
        o.insert("echoes".into(), json!(echoes));
        o.insert("ptt".into(), json!(ptt));
    }
}

/// 누르고 허가(또는 거절 · 시한)를 기다린다
async fn press_and_wait(client: &Client, events: &mut oxpeer::Events, room: &str, ptt: &mut Vec<Value>, t0: Instant) -> bool {
    if client.ptt_press(room, 0).await.is_err() {
        return false;
    }
    let deadline = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return false,
            e = events.next() => match e {
                Some(Event::Ptt { room: r, event, ptt: info }) if r == room => {
                    ptt.push(ptt_json(&event, &info, json!({ "ms": t0.elapsed().as_millis() as u64 })));
                    match event.as_str() {
                        "granted" => return true,
                        "denied" | "revoked" => return false,
                        _ => {}
                    }
                }
                Some(_) => {}
                None => return false,
            },
        }
    }
}

/// 걸음 — 형상마다 정해진 때(시작 기준 ms)
#[derive(Clone, Debug)]
enum Step {
    Half,
    Full,
    Press(String),
    Release(String),
    Speak(String),
    Reconnect,
}

fn plan(a: &Args) -> Vec<(u64, Step)> {
    let (r, r2) = (a.room.clone(), a.room2.clone());
    match a.mode.as_str() {
        "duplex" => vec![(2_000, Step::Half), (3_000, Step::Press(r.clone())), (5_000, Step::Release(r)), (6_000, Step::Full)],
        "move" => vec![(2_000, Step::Speak(r2.clone())), (3_000, Step::Press(r2.clone())), (5_000, Step::Release(r2)), (6_000, Step::Speak(r.clone())), (7_000, Step::Press(r.clone())), (9_000, Step::Release(r))],
        "reconnect" => vec![(a.reconnect_at * 1_000, Step::Reconnect)],
        _ => Vec::new(),
    }
}

/// 표지 — 오디오 20ms · 영상 33ms(30 장마다 키프레임 · 키프레임 요청이 오면 그 층 다음 장) · 걸음표대로 밟는다
async fn beacon(client: &Client, a: &Args, events: &mut oxpeer::Events, audio: LocalTrack, video: Option<LocalTrack>, stop: &mut std::pin::Pin<&mut impl std::future::Future<Output = ()>>, out: &mut Value) {
    let (mut audio, mut video) = (audio, video);
    let layers = video.as_ref().map(|v| v.layers()).unwrap_or(0);
    let (mut an, mut vn) = (0u32, vec![0u32; layers]);
    let mut want_key = vec![false; layers];
    let mut kreq: Vec<Value> = Vec::new();
    let mut targets: Vec<Value> = Vec::new();
    let mut steps: Vec<Value> = Vec::new();
    let mut published: Vec<Value> = Vec::new();
    let mut ptt: Vec<Value> = Vec::new();
    let mut sessions: Vec<Value> = Vec::new();
    let mut joins: Vec<Value> = Vec::new();
    let t0 = Instant::now();
    let mut vt = tokio::time::interval(Duration::from_millis(33));
    let mut todo = plan(a);
    todo.reverse();
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    loop {
        tokio::select! {
            _ = &mut *stop => break,
            r = audio.write(OutFrame { data: beacon_audio(an), duration: OPUS_20MS, keyframe: true, audio_level: Some(20) }) => {
                if r.is_err() { break }
                an += 1;
            }
            _ = vt.tick(), if video.is_some() => {
                if let Some(v) = &video {
                    for k in 0..layers {
                        let key = vn[k] % 30 == 0 || std::mem::take(&mut want_key[k]);
                        let tag = if layers > 1 { [b'l', b'h'][k] } else { 0 };
                        let f = OutFrame { data: beacon_video(vn[k], key, tag, &v.codec), duration: Duration::from_millis(33), keyframe: key, audio_level: None };
                        let r = if layers > 1 { v.write_layer(k as u8, f).await } else { v.write(f).await };
                        if r.is_err() {
                            break;
                        }
                        vn[k] += 1;
                    }
                }
            }
            _ = tick.tick() => {
                while let Some((at, _)) = todo.last() && t0.elapsed().as_millis() as u64 >= *at {
                    let Some((_, step)) = todo.pop() else { break };
                    let (ok, why) = match &step {
                        Step::Half => res(audio.set_duplex(Duplex::Half).await),
                        Step::Full => res(audio.set_duplex(Duplex::Full).await),
                        Step::Press(r) => res(client.ptt_press(r, 0).await),
                        Step::Release(r) => {
                            audio.drain().await;
                            res(client.ptt_release(r).await)
                        }
                        Step::Speak(r) => res(client.set_speaking_room(r).await),
                        Step::Reconnect => {
                            client.close().await;
                            let w = wall_ms();
                            let r = async {
                                client.connect().await?;
                                client.join_speaking(&a.room).await?;
                                publish_all(client, a, true).await
                            }.await;
                            match r {
                                Ok((au, vi)) => {
                                    steps.push(json!({ "step": "close", "ok": true, "wall_ms": w }));
                                    audio = au;
                                    video = vi;
                                    (true, None)
                                }
                                Err(e) => (false, Some(e.to_string())),
                            }
                        }
                    };
                    steps.push(json!({ "step": format!("{step:?}"), "ok": ok, "why": why, "wall_ms": wall_ms() }));
                }
            }
            e = events.next() => match e {
                Some(Event::KeyframeRequested { layer, .. }) => {
                    let k = usize::from(layer.unwrap_or(0));
                    if let Some(w) = want_key.get_mut(k) {
                        *w = true;
                    }
                    kreq.push(json!({ "ms": t0.elapsed().as_millis() as u64, "layer": layer }));
                }
                Some(Event::TargetBitrate { bps, .. }) => targets.push(json!([wall_ms(), bps])),
                Some(Event::Published { room, track_id }) => published.push(json!({ "room": room, "track_id": track_id, "wall_ms": wall_ms() })),
                Some(Event::Ptt { room, event, ptt: info }) => ptt.push(ptt_json(&event, &info, json!({ "room": room, "wall_ms": wall_ms() }))),
                Some(Event::Session(s)) => sessions.push(json!({ "state": format!("{:?}", s.state), "wall_ms": wall_ms() })),
                Some(Event::Rebuilt) => sessions.push(json!({ "state": "Rebuilt", "wall_ms": wall_ms() })),
                // 입장(처음 · 재구축 · 다시 connect) — 2층이 명단 통지를 셀 때 에이전트의 입장 사실로 쓴다
                Some(Event::Joined { room }) => joins.push(json!({ "room": room, "wall_ms": wall_ms() })),
                Some(Event::Closed { .. }) if a.mode != "reconnect" => break,
                None => break,
                _ => {}
            },
        }
    }
    if let Some(o) = out.as_object_mut() {
        o.insert("audio_frames".into(), json!(an));
        o.insert("video_frames".into(), json!(vn));
        o.insert("keyframe_requests".into(), json!(kreq));
        o.insert("beacon_tags".into(), json!({ "audio": BEACON_TAG_AUDIO, "video": BEACON_TAG_VIDEO }));
        o.insert("targets".into(), json!(targets));
        o.insert("steps".into(), json!(steps));
        o.insert("published".into(), json!(published));
        o.insert("ptt".into(), json!(ptt));
        o.insert("sessions".into(), json!(sessions));
        o.insert("joins".into(), json!(joins));
        o.insert("end_wall_ms".into(), json!(wall_ms()));
    }
}

fn res(r: Result<(), oxpeer::Error>) -> (bool, Option<String>) {
    match r {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    }
}

/// 진짜 코덱 표본 — 오디오 20ms · 영상 33ms 로 되풀이(끝나면 처음부터 — 첫 프레임이 키프레임) · 키프레임 요청이 오면 다음 키프레임으로
async fn file(a: &Args, events: &mut oxpeer::Events, audio: LocalTrack, video: Option<LocalTrack>, stop: &mut std::pin::Pin<&mut impl std::future::Future<Output = ()>>, out: &mut Value) {
    let read = |p: &Option<String>| p.as_ref().map(|x| std::fs::read(x).unwrap_or_else(|e| panic!("표본 {x}: {e}")));
    let opus = read(&a.opus).map(|b| media::read_ogg(&b)).unwrap_or_default();
    let frames: Vec<(Bytes, bool)> = match (read(&a.h264), read(&a.vp8)) {
        (Some(b), _) => media::read_h264(&b),
        (None, Some(b)) => media::read_ivf(&b),
        _ => Vec::new(),
    };
    let keys: Vec<usize> = frames.iter().enumerate().filter(|(_, f)| f.1).map(|(i, _)| i).collect();
    let (mut ai, mut vi, mut an, mut vn) = (0usize, 0usize, 0u64, 0u64);
    let mut kreq: Vec<u64> = Vec::new();
    let t0 = Instant::now();
    let mut vt = tokio::time::interval(Duration::from_millis(33));
    loop {
        tokio::select! {
            _ = &mut *stop => break,
            r = audio.write(OutFrame { data: opus.get(ai).cloned().unwrap_or_else(|| Bytes::from_static(&SILENCE)), duration: OPUS_20MS, keyframe: true, audio_level: None }), if !opus.is_empty() => {
                if r.is_err() { break }
                ai = (ai + 1) % opus.len();
                an += 1;
            }
            _ = vt.tick(), if video.is_some() && !frames.is_empty() => {
                let (data, key) = frames[vi].clone();
                if let Some(v) = &video && v.write(OutFrame { data, duration: Duration::from_millis(33), keyframe: key, audio_level: None }).await.is_err() {
                    break;
                }
                vi = (vi + 1) % frames.len();
                vn += 1;
            }
            e = events.next() => match e {
                Some(Event::KeyframeRequested { .. }) => {
                    // 다음 키프레임으로 — 없으면 처음(첫 프레임은 키프레임)
                    vi = keys.iter().copied().find(|k| *k >= vi).unwrap_or(0);
                    kreq.push(t0.elapsed().as_millis() as u64);
                }
                Some(Event::Closed { .. }) | None => break,
                _ => {}
            },
        }
    }
    if let Some(o) = out.as_object_mut() {
        o.insert("audio_frames".into(), json!(an));
        o.insert("video_frames".into(), json!(vn));
        o.insert("opus_packets".into(), json!(opus.len()));
        o.insert("video_samples".into(), json!(frames.len()));
        o.insert("keyframe_requests".into(), json!(kreq));
    }
}

/// 발언권 사건 한 줄 — 상태 · 사유(거절 · 회수) · 허가 시간 · 순번(SDK:IMPL-FLR-EXPOSE 거울)
fn ptt_json(event: &str, p: &oxpeer::PttInfo, mut base: Value) -> Value {
    let cause = match event {
        "denied" => p.last_deny.as_ref(),
        "revoked" => p.last_revoke.as_ref(),
        _ => None,
    };
    if let Some(o) = base.as_object_mut() {
        o.insert("event".into(), json!(event));
        o.insert("state".into(), json!(p.state));
        o.insert("cause".into(), json!(cause.map(|c| c.cause)));
        o.insert("remaining_sec".into(), json!(p.remaining_sec));
        o.insert("queue_position".into(), json!(p.queue_position));
    }
    base
}
