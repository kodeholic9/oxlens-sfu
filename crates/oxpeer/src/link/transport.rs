// author: kodeholic (powered by Claude)
//! 전송 하나(웹의 PeerConnection 한 개 자리) — UDP 소켓 하나 · 첫 바이트로 가른다(RFC 7983: STUN 0-3 · DTLS 20-63 · RTP/RTCP 128-191).
//! ICE(서버 ICE-lite · 클라 controlling · consent) → DTLS 능동(서버가 클라 인증서를 요구한다) → SRTP 키 → (발행 연결이면) SCTP + DCEP `"unreliable"`.
//! 층마다 사실을 따로 낸다(SDK:RULE-LINK-LAYERS — ICE · DTLS · SCTP · DC 를 섞지 않는다) · 판정은 SDK 루프의 연결 상태기가 한다.
//! 받은 RTP 는 SDK 루프를 거치지 않고 복호해 수신 정리층(`recv`)을 지나 미디어 통로로 바로 낸다 — 어느 방 · 트랙인지는 루프가 고쳐 두는 경로표(ssrc → 트랙)로 가른다.
//! 정리층이 짓는 RTCP(NACK · PLI · RR)는 이 수신 태스크가 SRTCP 로 싸서 보내고 · 받은 SRTCP 에서는 SR 만 읽는다.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use dtls::config::{Config, ExtendedMasterSecretType};
use dtls::conn::DTLSConn;
use dtls::crypto::Certificate;
use dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use sctp_proto::{AssociationHandle, ClientConfig, DatagramEvent, Endpoint, EndpointConfig, Event, Payload, PayloadProtocolIdentifier, ReliabilityType, StreamEvent, TransportConfig};
use tokio::net::UdpSocket;
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use tokio::task::AbortHandle;
use webrtc_srtp::config::Config as SrtpConfig;
use webrtc_srtp::context::Context;
use webrtc_srtp::protection_profile::ProtectionProfile;
use webrtc_util::conn::Conn;

use super::bwe::Bwe;
use super::recv::{self, Counters, MediaFrame, Out, Recv, Rtp, Unit};
use super::send::{self, SendTrack};
use super::stun;
use crate::wire::dc::CHANNEL_LABEL;

/// 검사 간격 — 서기 전(빠르게) · 선 뒤(consent · RFC 7675 의 5초보다 촘촘히)
const CHECK_EARLY: Duration = Duration::from_millis(500);
const CHECK_CONSENT: Duration = Duration::from_millis(2_500);
/// 응답이 이만큼 없으면 disconnected · failed(브라우저의 ICE 상태와 같은 뜻 — SDK:RULE-LINK-LAYERS)
const ICE_DISCONNECTED: Duration = Duration::from_millis(7_500);
const ICE_FAILED: Duration = Duration::from_secs(30);
const DTLS_TIMEOUT: Duration = Duration::from_secs(30);
const DCEP_OPEN: u8 = 0x03;
const DCEP_ACK: u8 = 0x02;
/// DCEP 채널 종류 — 순서 없음 · 재전송 횟수 한도(웹 `ordered:false · maxRetransmits:0` 과 같다)
const DCEP_PARTIAL_REXMIT_UNORDERED: u8 = 0x81;
/// 수신 정리층 시계 — NACK 을 이만큼 안에 낸다
const RECV_TICK: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// 발행용 연결 — DC 가 여기 붙는다(연:TRN.frame.DC 어디에 붙나)
    Publish,
    /// 구독용 연결 — 받는 미디어
    Subscribe,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Creds {
    pub ufrag: String,
    pub pwd: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IceState {
    Connected,
    Disconnected,
    Failed,
}

#[derive(Debug)]
pub(crate) enum Fact {
    Ice(IceState),
    /// DTLS 가 섰다(true) · 닫혔다 · 실패했다(false)
    Dtls(bool),
    /// SCTP 연관이 섰다(true) · 잃었다(false)
    Sctp(bool),
    /// DC 가 열렸다(true · DCEP ACK) · 닫혔다(false)
    Dc(bool),
    DcData(Vec<u8>),
    /// 이 ssrc 의 첫 복호 RTP 패킷(SDK:RULE-PLAT-SERVER `receiving`)
    FirstRtp(u32),
    /// 서버가 내 발행 ssrc 에 키프레임을 청했다(PLI · FIR) — 앱 몫(SDK 는 인코딩하지 않는다)
    Pli(u32),
    /// 대역 추정 목표가 바뀌었다(bps — `bwe`)
    Bwe(u64),
}

/// 사실 하나 — 어느 서버 · 몇째 전송 묶음 · 어느 연결
#[derive(Debug)]
pub(crate) struct LinkFact {
    pub server: String,
    pub sgen: u64,
    pub role: Role,
    pub fact: Fact,
}

/// 받는 트랙 하나의 경로 — 배정표 줄 + 트랙 보관본 + 서버 코덱 정책에서 루프가 짓는다
#[derive(Clone, Debug)]
pub(crate) struct Route {
    pub room: String,
    pub track_id: String,
    pub user_id: Option<String>,
    pub kind: String,
    pub codec: String,
    pub mid: String,
    /// 줄 ssrc(다운링크) · PT
    pub ssrc: u32,
    pub pt: u8,
    /// 그 줄의 `rtcp_fb` 에 `nack` · `nack pli` 가 있나(연:RULE-LINK-SDP-SUB-MLINE)
    pub nack: bool,
    pub pli: bool,
    pub unit: Unit,
    /// 앱이 이 트랙의 층을 바꾼 차례 — 바뀌면 정리층이 키프레임을 기다린다
    pub layer_gen: u64,
    /// 앱이 이 트랙에 키프레임을 요청한 차례 — 바뀌면 정리층이 다음 차례에 PLI(SDK:RULE-PLAT-SERVER 수신 정리층)
    pub key_gen: u64,
}

/// 경로표 — 줄 ssrc → 경로 · RTX ssrc → 줄 ssrc
#[derive(Clone, Debug, Default)]
pub(crate) struct Routes {
    pub media: HashMap<u32, Route>,
    pub rtx: HashMap<u32, u32>,
    /// 출처 번호(CSRC) → 주인 — 트랙 목록 `csrc`(연:RULE-TRK-CSRC) · `TAKEN` 화자 출처 번호(연:RULE-FLR-SLOT-SOURCES)
    pub sources: HashMap<u32, Source>,
}

/// 패킷 · 프레임의 출처 — 다운링크 패킷의 CSRC(연:RULE-LINK-EGRESS-REWRITE ⑤) · ★발행 신원의 정본(SDK:RULE-PLAT-SERVER 「출처 표지」)
/// 주인을 아직 모르면(목록 · `TAKEN` 이 아직 안 왔다) `user_id` · `track_id` 가 비고 번호만 든다 — 번호는 트랙이 사는 동안 같다
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub csrc: u32,
    pub user_id: Option<String>,
    /// 개인 트랙이면 그 `track_id` — 반이중 발행(무전 슬롯)은 목록에 없어 비어 있다
    pub track_id: Option<String>,
}

/// 받은 RTP 한 패킷(SRTP 를 푼 것 · 코덱 페이로드 그대로 — 디코딩은 앱 몫, SDK:RULE-PLAT-SERVER `mediaStreamTrack` 의 뜻)
#[derive(Clone, Debug)]
pub struct MediaPacket {
    pub room: String,
    pub track_id: String,
    /// 트랙 주인(개인 트랙) — 무전 슬롯은 없다(화자는 발언권 `speaker` 가 알린다)
    pub user_id: Option<String>,
    /// `audio` · `video`
    pub kind: String,
    pub codec: String,
    pub mid: String,
    pub ssrc: u32,
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub marker: bool,
    pub payload: Bytes,
    pub received_at: Instant,
    /// 이 패킷의 발행 출처 — 없으면 서버가 지은 패킷(대역 probe 패딩) · 표지를 끈 연결
    pub source: Option<Source>,
}

/// 경로표와 미디어 통로 — 모든 구독 연결이 같이 쓴다
#[derive(Clone)]
pub(crate) struct MediaSink {
    pub routes: Arc<RwLock<Routes>>,
    pub tx: mpsc::Sender<MediaPacket>,
    pub frames: mpsc::Sender<MediaFrame>,
    /// 통로가 가득 차 버린 패킷 수(앱이 늦게 꺼낸다)
    pub dropped: Arc<AtomicU64>,
    pub frames_dropped: Arc<AtomicU64>,
    pub counters: Arc<Counters>,
    /// ★QA 스위치(`OXPEER_QA_DROP_VIDEO_EVERY`) — 받은 영상 원본 패킷 N 개마다 하나를 정리층 앞에서 버린다(망 손실 흉내 · RTX 는 안 버린다). 0 = 끔 · 2층 손실 시나리오의 시험 수단
    pub qa_drop_every: u64,
    /// ★QA 스위치(`OXPEER_QA_DROP_VIDEO_UNTIL_MS`) — 영상 버림을 이 시각까지만(손실이 멎는 판 — 서버가 층을 되올리나 · context 202611/20261110e 2-①). 없으면 내내
    pub qa_drop_until: Option<Instant>,
    /// ★QA 스위치(`OXPEER_QA_DROP_AUDIO_EVERY`) — 받은 음성 원본 N 개마다 하나를 정리층 앞에서 버린다(녹음 구멍 메우기 시험 — context 202611/20261109f) · `qa_dropped` 에 함께 센다
    pub qa_drop_audio_every: u64,
    pub qa_dropped: Arc<AtomicU64>,
    /// 보내는 트랙(발행 ssrc → RTP 상태) — 송신 시계가 쥐고 발행 연결의 NACK 이 빌린다
    pub senders: Arc<RwLock<HashMap<u32, Arc<Mutex<SendTrack>>>>>,
    /// 서버가 준 대역 상한(REMB · bps) · RTX 로 다시 보낸 패킷 수
    pub remb: Arc<AtomicU64>,
    pub rtx_sent: Arc<AtomicU64>,
    /// 녹음 원시 기록(SDK:RULE-PLAT-REC-TAP) — 정리층 출구 · SR 을 여기 직접 적는다
    #[cfg(feature = "record")]
    pub recorder: Option<Arc<crate::record::Recorder>>,
    /// 운영 로그(SDK:RULE-PLAT-SERVER 「운영 로그」) — 전송로 수명 진단 · 받는 줄마다 수신 계수
    pub log: Arc<crate::log::Logger>,
    /// 경로표에 없는 ssrc 로 온 패킷 수
    pub unrouted: Arc<AtomicU64>,
    pub decrypt_fail: Arc<AtomicU64>,
}

enum Ctl {
    Creds(Creds),
    SendDc(Vec<u8>),
    ReopenDc,
}

enum SctpCtl {
    Send(Vec<u8>),
    Reopen,
}

pub(crate) struct Transport {
    ctl: mpsc::UnboundedSender<Ctl>,
    tasks: Arc<Mutex<Vec<AbortHandle>>>,
    out: Arc<OnceLock<PubOut>>,
}

/// 내보내는 손잡이 — 소켓 · 서버 주소 · 송신 SRTP 문맥(DTLS 가 서면 찬다). 송신 시계가 복제해 쥔다
#[derive(Clone)]
pub(crate) struct PubOut {
    sock: Arc<UdpSocket>,
    server: SocketAddr,
    tx: Arc<OnceLock<Mutex<Context>>>,
    /// 이 연결의 대역 추정(transport-cc 번호 공간은 연결의 것)
    bwe: Arc<Mutex<Bwe>>,
}

impl PubOut {
    /// 평문 RTP 하나 — transport-cc 번호를 채워 싸서 보낸다(아직 DTLS 전이면 false)
    pub(crate) fn rtp(&self, plain: &[u8]) -> bool {
        let Some(ctx) = self.tx.get() else { return false };
        let mut p = plain.to_vec();
        if let Ok(mut b) = self.bwe.lock() {
            b.stamp(&mut p, Instant::now());
        }
        let Some(enc) = ctx.lock().ok().and_then(|mut c| c.encrypt_rtp(&p).ok()) else { return false };
        self.sock.try_send_to(&enc, self.server).is_ok()
    }

    /// ★QA 손실 — 번호는 채워 보낸 것으로 적되 내보내지 않는다(망에서 잃은 것과 같다 — 서버 TWCC 가 「못 받음」으로 알리고 NACK 이 RTX 를 부른다)
    pub(crate) fn rtp_lost(&self, plain: &[u8]) {
        let mut p = plain.to_vec();
        if let Ok(mut b) = self.bwe.lock() {
            b.stamp(&mut p, Instant::now());
        }
    }

    /// transport-cc 확장 번호(서버 선언) — 발행이 처음 쓸 때 알린다
    pub(crate) fn set_twcc(&self, ext: u8) {
        if let Ok(mut b) = self.bwe.lock() {
            b.ext = ext;
        }
    }

    /// (목표 bps, 손실률)
    pub(crate) fn bwe_now(&self) -> (u64, f64) {
        self.bwe.lock().map(|b| (b.target(), b.loss)).unwrap_or((0, 0.0))
    }

    pub(crate) fn rtcp(&self, plain: &[u8]) -> bool {
        let Some(enc) = self.tx.get().and_then(|c| c.lock().ok().and_then(|mut c| c.encrypt_rtcp(plain).ok())) else { return false };
        self.sock.try_send_to(&enc, self.server).is_ok()
    }
}

impl Transport {
    /// 연다 — 곧바로 돌아오고 층마다의 사실은 `facts` 로 온다
    pub(crate) fn open(server: SocketAddr, creds: Creds, cert: Certificate, role: Role, tag: (String, u64), facts: mpsc::UnboundedSender<LinkFact>, sink: MediaSink) -> Transport {
        let (name, sgen) = tag;
        let (ctl, ctl_rx) = mpsc::unbounded_channel();
        let tasks: Arc<Mutex<Vec<AbortHandle>>> = Arc::default();
        let out: Arc<OnceLock<PubOut>> = Arc::default();
        let (t2, o2) = (tasks.clone(), out.clone());
        let main = tokio::spawn(async move {
            let say = Say { facts, server: name, sgen, role };
            if let Err(why) = run(server, creds, cert, role, say.clone(), sink, ctl_rx, t2, o2).await {
                say.send(Fact::Dtls(false));
                let _ = why;
            }
        });
        if let Ok(mut g) = tasks.lock() {
            g.push(main.abort_handle());
        }
        Transport { ctl, tasks, out }
    }

    /// 내보내는 손잡이 자리(소켓이 서면 찬다)
    pub(crate) fn out_slot(&self) -> Arc<OnceLock<PubOut>> {
        self.out.clone()
    }

    /// ICE 재시작 — 새 자격으로 검사한다(DTLS · SRTP 는 그대로 — 지문 · setup 이 같다)
    pub(crate) fn set_creds(&self, c: Creds) {
        let _ = self.ctl.send(Ctl::Creds(c));
    }

    pub(crate) fn send_dc(&self, b: Vec<u8>) -> bool {
        self.ctl.send(Ctl::SendDc(b)).is_ok()
    }

    /// 같은 연관 위에 DC 를 새로 연다(SDK:RULE-LINK-LAYERS DC 다시 열기)
    pub(crate) fn reopen_dc(&self) {
        let _ = self.ctl.send(Ctl::ReopenDc);
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        if let Ok(g) = self.tasks.lock() {
            for h in g.iter() {
                h.abort();
            }
        }
    }
}

#[derive(Clone)]
struct Say {
    facts: mpsc::UnboundedSender<LinkFact>,
    server: String,
    sgen: u64,
    role: Role,
}

impl Say {
    fn send(&self, fact: Fact) {
        let _ = self.facts.send(LinkFact { server: self.server.clone(), sgen: self.sgen, role: self.role, fact });
    }
}

fn keep(tasks: &Arc<Mutex<Vec<AbortHandle>>>, h: AbortHandle) {
    if let Ok(mut g) = tasks.lock() {
        g.push(h);
    }
}

/// 선 DTLS 와 SRTP 문맥 둘(로컬 · 원격)
type Secured = (Arc<DTLSConn>, Context, Context);

enum Inner {
    StunOk([u8; 12]),
    Dtls(Box<Result<Secured, String>>),
}

#[allow(clippy::too_many_arguments)]
async fn run(server: SocketAddr, mut creds: Creds, cert: Certificate, role: Role, say: Say, sink: MediaSink, mut ctl: mpsc::UnboundedReceiver<Ctl>, tasks: Arc<Mutex<Vec<AbortHandle>>>, out: Arc<OnceLock<PubOut>>) -> Result<(), String> {
    let sock = Arc::new(UdpSocket::bind("0.0.0.0:0").await.map_err(|e| format!("UDP bind: {e}"))?);
    LINKS_ALIVE.fetch_add(1, Ordering::Relaxed);
    let _alive = LinkAlive { log: sink.log.clone(), server, role };
    let (inner_tx, mut inner_rx) = mpsc::unbounded_channel::<Inner>();
    let (dtls_tx, dtls_rx) = mpsc::channel::<Vec<u8>>(256);
    let rx_ctx: Arc<OnceLock<Mutex<Context>>> = Arc::default();
    let tx_ctx: Arc<OnceLock<Mutex<Context>>> = Arc::default();
    let _ = out.set(PubOut { sock: sock.clone(), server, tx: tx_ctx.clone(), bwe: Arc::default() });
    let po = out.get().cloned();

    let (s2, i2, rx2, tx2, say2, sink2) = (sock.clone(), inner_tx.clone(), rx_ctx.clone(), tx_ctx.clone(), say.clone(), sink.clone());
    let demux = tokio::spawn(async move {
        let mut buf = vec![0u8; 2048];
        let mut seen: HashSet<u32> = HashSet::new();
        let mut qa_n = 0u64;
        let mut me = [0u8; 4];
        let _ = getrandom::fill(&mut me);
        let mut rv = Recv::new(u32::from_be_bytes(me), sink2.counters.clone());
        rv.record = sink2.recording();
        let mut tick = tokio::time::interval(RECV_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut stat_at = Instant::now();
        loop {
            tokio::select! {
                r = s2.recv_from(&mut buf) => {
                    let Ok((n, from)) = r else { break };
                    if n == 0 || from != server {
                        continue;
                    }
                    let b0 = buf[0];
                    if b0 <= 3 {
                        if let Some(tid) = stun::binding_success(&buf[..n]) {
                            let _ = i2.send(Inner::StunOk(tid));
                        }
                    } else if (20..=63).contains(&b0) {
                        let _ = dtls_tx.try_send(buf[..n].to_vec());
                    } else if (128..=191).contains(&b0) && n >= 2 {
                        if (192..=223).contains(&buf[1]) {
                            on_rtcp(&buf[..n], &rx2, &mut rv, &sink2, po.as_ref(), &say2);
                        } else {
                            on_rtp(&buf[..n], &rx2, &sink2, &mut seen, &say2, &mut rv, &mut qa_n);
                        }
                    }
                }
                _ = tick.tick() => {
                    // 받는 줄마다 수신 계수(운영 `stat` — W3C 이름) · 주기는 운영 로그 `stat_interval`
                    if stat_at.elapsed() >= sink2.log.config().stat_interval {
                        stat_at = Instant::now();
                        for x in rv.stats() {
                            sink2.log.emit(crate::log::Line::new(crate::log::Kind::Stat).corr("room", &x.room).corr("track", &x.track_id).corr("ssrc", x.ssrc).key("packets_received", x.received).key("packets_lost", x.lost).key("jitter", format!("{:.6}", x.jitter_s)));
                        }
                    }
                    let live = sink2.routes.read().map(|g| g.media.iter().map(|(k, r)| (*k, r.track_id.clone())).collect::<HashMap<u32, String>>()).unwrap_or_default();
                    rv.retain(|k, t| live.get(&k).is_some_and(|x| x == t));
                    let (outs, rtcp) = rv.tick(Instant::now());
                    deliver(outs, &sink2);
                    if let Some(p) = rtcp
                        && let Some(enc) = tx2.get().and_then(|c| c.lock().ok().and_then(|mut c| c.encrypt_rtcp(&p).ok()))
                    {
                        let _ = s2.send_to(&enc, server).await;
                    }
                }
            }
        }
    });
    keep(&tasks, demux.abort_handle());

    let local = stun::random_ice_string(8);
    let mut tie_b = [0u8; 8];
    let _ = getrandom::fill(&mut tie_b);
    let tie = u64::from_be_bytes(tie_b);
    let mut pending: HashMap<[u8; 12], Instant> = HashMap::new();
    let started = Instant::now();
    let mut last_ok: Option<Instant> = None;
    let mut ice: Option<IceState> = None;
    let mut last_sent = Instant::now() - CHECK_CONSENT;
    let mut dtls_rx = Some(dtls_rx);
    let mut cert = Some(cert);
    let mut sctp: Option<mpsc::UnboundedSender<SctpCtl>> = None;
    // 전송로가 끝나면(돌아가든 abort 되든) DTLS 를 닫는다 — dtls 크레이트가 띄운 읽기 고리를 정식 닫힘 통로로 멈춘다
    let mut _dtls_guard: Option<DtlsGuard> = None;
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let now = Instant::now();
                let every = if ice == Some(IceState::Connected) { CHECK_CONSENT } else { CHECK_EARLY };
                if now.duration_since(last_sent) >= every {
                    let (req, tid) = stun::binding_request(&creds.ufrag, &local, &creds.pwd, tie);
                    pending.retain(|_, t| now.duration_since(*t) < ICE_FAILED);
                    pending.insert(tid, now);
                    let _ = sock.send_to(&req, server).await;
                    last_sent = now;
                }
                let quiet = now.duration_since(last_ok.unwrap_or(started));
                let next = if quiet >= ICE_FAILED { Some(IceState::Failed) } else if quiet >= ICE_DISCONNECTED && last_ok.is_some() { Some(IceState::Disconnected) } else { ice };
                if next != ice && next.is_some() && next != Some(IceState::Connected) {
                    ice = next;
                    say.send(Fact::Ice(ice.unwrap_or(IceState::Failed)));
                }
            }
            Some(m) = inner_rx.recv() => match m {
                Inner::StunOk(tid) => {
                    if pending.remove(&tid).is_none() {
                        continue;
                    }
                    last_ok = Some(Instant::now());
                    if ice != Some(IceState::Connected) {
                        ice = Some(IceState::Connected);
                        say.send(Fact::Ice(IceState::Connected));
                    }
                    if let (Some(rx), Some(c)) = (dtls_rx.take(), cert.take()) {
                        let (s3, i3) = (sock.clone(), inner_tx.clone());
                        let hs = tokio::spawn(async move {
                            let _ = i3.send(Inner::Dtls(Box::new(handshake(s3, server, rx, c).await)));
                        });
                        keep(&tasks, hs.abort_handle());
                    }
                }
                Inner::Dtls(r) => {
                    let (conn, tx, rx) = match *r {
                        Ok(x) => x,
                        Err(why) => {
                            say.send(Fact::Dtls(false));
                            return Err(why);
                        }
                    };
                    let _ = tx_ctx.set(Mutex::new(tx));
                    let _ = rx_ctx.set(Mutex::new(rx));
                    DTLS_OPEN.fetch_add(1, Ordering::Relaxed);
                    _dtls_guard = Some(DtlsGuard(conn.clone(), sink.log.clone()));
                    sink.log.emit(crate::log::Line::new(crate::log::Kind::Diag).key("server", server).key("role", role_name(role)).msg("transport up"));
                    say.send(Fact::Dtls(true));
                    if role == Role::Publish {
                        let (stx, srx) = mpsc::unbounded_channel();
                        sctp = Some(stx);
                        let h = tokio::spawn(sctp_client(conn, say.clone(), srx));
                        keep(&tasks, h.abort_handle());
                    } else {
                        let say3 = say.clone();
                        let h = tokio::spawn(async move {
                            let mut buf = vec![0u8; 2048];
                            while conn.read(&mut buf, None).await.is_ok() {}
                            say3.send(Fact::Dtls(false));
                        });
                        keep(&tasks, h.abort_handle());
                    }
                }
            },
            c = ctl.recv() => match c {
                None => return Ok(()),
                Some(Ctl::Creds(c)) => {
                    creds = c;
                    pending.clear();
                    last_sent = Instant::now() - CHECK_CONSENT;
                }
                Some(Ctl::SendDc(b)) => {
                    if let Some(s) = &sctp {
                        let _ = s.send(SctpCtl::Send(b));
                    }
                }
                Some(Ctl::ReopenDc) => {
                    if let Some(s) = &sctp {
                        let _ = s.send(SctpCtl::Reopen);
                    }
                }
            },
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn on_rtp(pkt: &[u8], rx: &Arc<OnceLock<Mutex<Context>>>, sink: &MediaSink, seen: &mut HashSet<u32>, say: &Say, rv: &mut Recv, qa_n: &mut u64) {
    let Some(ctx) = rx.get() else { return };
    let plain = ctx.lock().ok().and_then(|mut c| c.decrypt_rtp(pkt).ok());
    let Some(p) = plain else {
        sink.decrypt_fail.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(h) = rtp_header(&p) else { return };
    let (route, rtx, source) = match sink.routes.read() {
        Ok(g) => {
            let source = h.csrc.map(|c| g.sources.get(&c).cloned().unwrap_or(Source { csrc: c, user_id: None, track_id: None }));
            match g.media.get(&h.ssrc) {
                Some(r) => (Some(r.clone()), false, source),
                None => (g.rtx.get(&h.ssrc).and_then(|m| g.media.get(m)).cloned(), true, source),
            }
        }
        Err(_) => (None, false, None),
    };
    let Some(r) = route else {
        sink.unrouted.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let body = p.slice(h.start..h.end);
    let rtp = if rtx {
        // RFC 4588 §4 — 앞 두 바이트가 원래 seq(OSN) · 나머지가 원래 페이로드. 그보다 짧으면 채움(대역 탐침)이라 버린다
        if body.len() < 2 {
            return;
        }
        let osn = u16::from_be_bytes([body[0], body[1]]);
        let raw = sink.recording().then(|| rtx_original(&p, &h, osn, r.pt, r.ssrc));
        Rtp { seq: osn, ts: h.ts, marker: h.marker, pt: r.pt, payload: body.slice(2..), recovered: true, source, raw }
    } else {
        if seen.insert(h.ssrc) {
            say.send(Fact::FirstRtp(h.ssrc));
        }
        if sink.qa_drop_audio_every > 0 && r.kind == "audio" {
            *qa_n += 1;
            if (*qa_n).is_multiple_of(sink.qa_drop_audio_every) {
                sink.qa_dropped.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        if sink.qa_drop_every > 0 && r.kind == "video" && sink.qa_drop_until.is_none_or(|u| Instant::now() < u) {
            *qa_n += 1;
            if (*qa_n).is_multiple_of(sink.qa_drop_every) {
                sink.qa_dropped.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        let raw = sink.recording().then(|| p.clone());
        Rtp { seq: h.seq, ts: h.ts, marker: h.marker, pt: h.pt, payload: body, recovered: false, source, raw }
    };
    let outs = rv.on_rtp(&r, rtp, Instant::now(), std::time::SystemTime::now());
    deliver(outs, sink);
}

fn on_rtcp(pkt: &[u8], rx: &Arc<OnceLock<Mutex<Context>>>, rv: &mut Recv, sink: &MediaSink, out: Option<&PubOut>, say: &Say) {
    let Some(ctx) = rx.get() else { return };
    let Some(p) = ctx.lock().ok().and_then(|mut c| c.decrypt_rtcp(pkt).ok()) else { return };
    let now = Instant::now();
    for (ssrc, ntp, rtp) in recv::sender_reports(&p) {
        rv.on_sr(ssrc, ntp, rtp, now);
        #[cfg(feature = "record")]
        if let Some(r) = &sink.recorder {
            r.push(crate::record::Channel::Sr, serde_json::json!({ "ssrc": ssrc, "ntp": ntp, "rtp": rtp }).to_string().into_bytes());
        }
    }
    // 발행자 몫 — 서버가 내 발행에 묻는 것(정:IMPL-RTCP-TERMINATE): NACK → RTX · PLI → 앱 · REMB → 계수
    let fb = send::feedback(&p);
    for (ssrc, seqs) in fb.nacks {
        let st = sink.senders.read().ok().and_then(|g| g.get(&ssrc).cloned());
        let (Some(st), Some(o)) = (st, out) else { continue };
        let pkts = st.lock().map(|mut t| t.retransmit(&seqs)).unwrap_or_default();
        for x in pkts {
            if o.rtp(&x) {
                sink.rtx_sent.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    for ssrc in fb.plis {
        if sink.senders.read().is_ok_and(|g| g.contains_key(&ssrc)) {
            say.send(Fact::Pli(ssrc));
        }
    }
    if let Some(b) = fb.remb {
        sink.remb.store(b, Ordering::Relaxed);
        if let Some(t) = out.and_then(|o| o.bwe.lock().ok().and_then(|mut x| x.on_remb(b))) {
            say.send(Fact::Bwe(t));
        }
    }
    if let Some(o) = out {
        for r in fb.twcc {
            if let Some(t) = o.bwe.lock().ok().and_then(|mut x| x.on_feedback(&r, now)) {
                say.send(Fact::Bwe(t));
            }
        }
    }
}

fn deliver(outs: Vec<Out>, sink: &MediaSink) {
    for o in outs {
        match o {
            Out::Packet(m) => {
                if sink.tx.try_send(m).is_err() {
                    sink.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            Out::Frame(f) => {
                if sink.frames.try_send(f).is_err() {
                    sink.frames_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            #[cfg(feature = "record")]
            Out::Raw { ssrc, data } => {
                if let Some(r) = &sink.recorder {
                    r.push(crate::record::Channel::Rtp(ssrc), data.to_vec());
                }
            }
            #[cfg(feature = "record")]
            Out::Gap(g) => {
                if let Some(r) = &sink.recorder {
                    r.push(crate::record::Channel::Gap, serde_json::json!({ "ssrc": g.ssrc, "from": g.from, "to": g.to, "count": g.count, "reason": g.reason }).to_string().into_bytes());
                }
            }
            #[cfg(not(feature = "record"))]
            Out::Raw { .. } | Out::Gap(_) => {}
        }
    }
}

impl MediaSink {
    /// 녹음 원시 기록을 켰나
    pub(crate) fn recording(&self) -> bool {
        #[cfg(feature = "record")]
        {
            self.recorder.is_some()
        }
        #[cfg(not(feature = "record"))]
        {
            false
        }
    }
}

/// RTX 패킷 → 원래 패킷(SDK:RULE-PLAT-REC-TAP) — seq = OSN · PT = 원래 PT · SSRC = 원래 줄 · 앞 두 바이트(OSN)를 뗀다 · 채움을 뗀다(P 비트 내림) · CSRC · 확장은 그대로
fn rtx_original(p: &Bytes, h: &RtpHeader, osn: u16, pt: u8, ssrc: u32) -> Bytes {
    let mut v = Vec::with_capacity(h.end.saturating_sub(2));
    v.extend_from_slice(&p[..h.start]);
    v[0] &= !0x20;
    v[1] = (v[1] & 0x80) | (pt & 0x7f);
    v[2..4].copy_from_slice(&osn.to_be_bytes());
    v[8..12].copy_from_slice(&ssrc.to_be_bytes());
    v.extend_from_slice(&p[h.start + 2..h.end]);
    Bytes::from(v)
}

pub(crate) struct RtpHeader {
    pub marker: bool,
    pub pt: u8,
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
    pub csrc: Option<u32>,
    pub start: usize,
    pub end: usize,
}

/// RTP 머리(RFC 3550) — CSRC · 확장 머리를 건너뛰고 채움을 뗀다
pub(crate) fn rtp_header(p: &[u8]) -> Option<RtpHeader> {
    if p.len() < 12 || p[0] >> 6 != 2 {
        return None;
    }
    let cc = usize::from(p[0] & 0x0f);
    let mut start = 12 + 4 * cc;
    if p[0] & 0x10 != 0 {
        if p.len() < start + 4 {
            return None;
        }
        let words = usize::from(u16::from_be_bytes([p[start + 2], p[start + 3]]));
        start += 4 + 4 * words;
    }
    let mut end = p.len();
    if p[0] & 0x20 != 0 {
        let pad = usize::from(*p.last()?);
        end = end.checked_sub(pad)?;
    }
    if start > end {
        return None;
    }
    Some(RtpHeader {
        marker: p[1] & 0x80 != 0,
        pt: p[1] & 0x7f,
        seq: u16::from_be_bytes([p[2], p[3]]),
        ts: u32::from_be_bytes([p[4], p[5], p[6], p[7]]),
        ssrc: u32::from_be_bytes([p[8], p[9], p[10], p[11]]),
        csrc: (cc > 0).then(|| u32::from_be_bytes([p[12], p[13], p[14], p[15]])),
        start,
        end,
    })
}

/// DTLS 능동 → SRTP 키(로컬 · 원격)
async fn handshake(sock: Arc<UdpSocket>, server: SocketAddr, rx: mpsc::Receiver<Vec<u8>>, cert: Certificate) -> Result<Secured, String> {
    let conn: Arc<dyn Conn + Send + Sync> = Arc::new(ChanConn { rx: AsyncMutex::new(rx), sock, peer: server });
    let cfg = Config {
        certificates: vec![cert],
        srtp_protection_profiles: vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80],
        extended_master_secret: ExtendedMasterSecretType::Require,
        insecure_skip_verify: true,
        mtu: 1_200,
        ..Config::default()
    };
    let dtls = match tokio::time::timeout(DTLS_TIMEOUT, DTLSConn::new(conn, cfg, true, None)).await {
        Ok(Ok(c)) => Arc::new(c),
        Ok(Err(e)) => return Err(format!("DTLS: {e}")),
        Err(_) => return Err("DTLS 시한".into()),
    };
    let mut sc = SrtpConfig { profile: ProtectionProfile::Aes128CmHmacSha1_80, ..Default::default() };
    let state = dtls.connection_state().await;
    sc.extract_session_keys_from_dtls(state, true).await.map_err(|e| format!("SRTP 키: {e}"))?;
    let tx = Context::new(&sc.keys.local_master_key, &sc.keys.local_master_salt, sc.profile, None, None).map_err(|e| e.to_string())?;
    let rx = Context::new(&sc.keys.remote_master_key, &sc.keys.remote_master_salt, sc.profile, None, None).map_err(|e| e.to_string())?;
    Ok((dtls, tx, rx))
}

/// 프로세스의 살아 있는 전송로 · 닫히지 않은 DTLS 수 — 운영 로그 `stat` 의 `links` · `dtls_open`(같은 프로세스의 Client 를 다 합친다)
pub(crate) static LINKS_ALIVE: AtomicU64 = AtomicU64::new(0);
pub(crate) static DTLS_OPEN: AtomicU64 = AtomicU64::new(0);

/// 전송로 하나가 산 동안 — 놓이면(돌아가든 abort 되든) 줄이고 진단 한 줄
struct LinkAlive {
    log: Arc<crate::log::Logger>,
    server: SocketAddr,
    role: Role,
}

impl Drop for LinkAlive {
    fn drop(&mut self) {
        LINKS_ALIVE.fetch_sub(1, Ordering::Relaxed);
        self.log.emit(crate::log::Line::new(crate::log::Kind::Diag).key("server", self.server).key("role", role_name(self.role)).msg("transport down"));
    }
}

fn role_name(r: Role) -> &'static str {
    if r == Role::Publish { "publish" } else { "subscribe" }
}

/// 전송로 수명에 묶인 DTLS 닫기 — 놓일 때(전송로 끝 · abort) `close()` 를 띄운다(Drop 은 기다릴 수 없다) · 닫히면 `dtls_open` 을 줄인다
struct DtlsGuard(Arc<DTLSConn>, Arc<crate::log::Logger>);

impl Drop for DtlsGuard {
    fn drop(&mut self) {
        let (c, log) = (self.0.clone(), self.1.clone());
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            h.spawn(async move {
                let r = c.close().await;
                DTLS_OPEN.fetch_sub(1, Ordering::Relaxed);
                log.emit(crate::log::Line::new(crate::log::Kind::Diag).key("result", if r.is_ok() { "ok" } else { "error" }).msg("dtls closed"));
            });
        }
    }
}

/// DTLS 를 UDP 소켓 위에 — 받는 것은 갈라 받은 DTLS 레코드만
struct ChanConn {
    rx: AsyncMutex<mpsc::Receiver<Vec<u8>>>,
    sock: Arc<UdpSocket>,
    peer: SocketAddr,
}

#[async_trait]
impl Conn for ChanConn {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        Ok(())
    }
    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let mut r = self.rx.lock().await;
        match r.recv().await {
            Some(p) => {
                let n = p.len().min(buf.len());
                buf[..n].copy_from_slice(&p[..n]);
                Ok(n)
            }
            // ★통로가 닫혔으면(전송로 태스크가 끝났다) 기다린다 — 오류를 내면 dtls 읽기 고리가 ErrAlertFatalOrClose 가 아닌 오류에서 곧바로 다시 돌아
            //   코어 하나를 태운다(20261109d 2층 — close 뒤 살아 있는 녹음기 CPU 100%). 고리는 DtlsGuard 가 닫아 멈춘다
            None => std::future::pending().await,
        }
    }
    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let n = self.recv(buf).await?;
        Ok((n, self.peer))
    }
    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        self.sock.send_to(buf, self.peer).await.map_err(|e| webrtc_util::Error::Other(e.to_string()))
    }
    async fn send_to(&self, buf: &[u8], _target: SocketAddr) -> webrtc_util::Result<usize> {
        self.send(buf).await
    }
    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        self.sock.local_addr().map_err(|e| webrtc_util::Error::Other(e.to_string()))
    }
    fn remote_addr(&self) -> Option<SocketAddr> {
        Some(self.peer)
    }
    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

fn dcep_open() -> Vec<u8> {
    let label = CHANNEL_LABEL.as_bytes();
    let mut m = vec![DCEP_OPEN, DCEP_PARTIAL_REXMIT_UNORDERED, 0, 0, 0, 0, 0, 0];
    m.extend_from_slice(&(label.len() as u16).to_be_bytes());
    m.extend_from_slice(&0u16.to_be_bytes());
    m.extend_from_slice(label);
    m
}

async fn write_payload(dtls: &DTLSConn, p: Payload) -> bool {
    if let Payload::RawEncode(chunks) = p {
        for c in chunks {
            if dtls.write(&c, None).await.is_err() {
                return false;
            }
        }
    }
    true
}

/// SCTP 연관(클라) — DTLS 클라는 짝수 스트림(RFC 8832 §6). 서면 DCEP OPEN · ACK 를 받으면 DC 열림 · 그 뒤 받은 이진 메시지를 낸다.
async fn sctp_client(dtls: Arc<DTLSConn>, say: Say, mut ctl: mpsc::UnboundedReceiver<SctpCtl>) {
    let (pkt_tx, mut pkt_rx) = mpsc::channel::<Vec<u8>>(256);
    let reader = dtls.clone();
    let rh = tokio::spawn(async move {
        let mut buf = vec![0u8; 65_536];
        while let Ok(n) = reader.read(&mut buf, None).await {
            if pkt_tx.send(buf[..n].to_vec()).await.is_err() {
                break;
            }
        }
    });
    let remote = SocketAddr::from(([127, 0, 0, 1], 5_000));
    let mut endpoint = Endpoint::new(Arc::new(EndpointConfig::new()), None);
    let mut cc = ClientConfig::new();
    cc.transport = Arc::new(TransportConfig::default().with_max_data_retransmits(None).with_max_init_retransmits(None));
    let Ok((h, mut a)) = endpoint.connect(cc, remote) else {
        rh.abort();
        say.send(Fact::Sctp(false));
        return;
    };
    let h: AssociationHandle = h;
    let mut sid: u16 = 0;
    let mut open = false;
    let open_stream = |a: &mut sctp_proto::Association, sid: u16| {
        if let Ok(mut s) = a.open_stream(sid, PayloadProtocolIdentifier::Dcep) {
            let _ = s.set_reliability_params(true, ReliabilityType::Rexmit, 0);
            let _ = s.write_sctp(&Bytes::from(dcep_open()), PayloadProtocolIdentifier::Dcep);
        }
    };
    loop {
        let now = Instant::now();
        let mut alive = true;
        while let Some(t) = endpoint.poll_transmit() {
            alive &= write_payload(&dtls, t.payload).await;
        }
        while let Some(t) = a.poll_transmit(now) {
            alive &= write_payload(&dtls, t.payload).await;
        }
        while let Some(ev) = a.poll_endpoint_event() {
            if let Some(back) = endpoint.handle_event(h, ev) {
                a.handle_event(back);
            }
        }
        while let Some(ev) = a.poll() {
            match ev {
                Event::Connected => {
                    say.send(Fact::Sctp(true));
                    open_stream(&mut a, sid);
                }
                Event::HandshakeFailed { .. } | Event::AssociationLost { .. } => {
                    rh.abort();
                    say.send(Fact::Sctp(false));
                    return;
                }
                Event::Stream(StreamEvent::Readable { id }) => {
                    if let Ok(mut s) = a.stream(id) {
                        while let Ok(Some(chunks)) = s.read_sctp() {
                            let mut buf = vec![0u8; chunks.len()];
                            let Ok(n) = chunks.read(&mut buf) else { continue };
                            buf.truncate(n);
                            if chunks.ppi == PayloadProtocolIdentifier::Dcep {
                                if id == sid && buf.first() == Some(&DCEP_ACK) && !open {
                                    open = true;
                                    say.send(Fact::Dc(true));
                                }
                            } else if matches!(chunks.ppi, PayloadProtocolIdentifier::Binary | PayloadProtocolIdentifier::String) {
                                say.send(Fact::DcData(buf));
                            }
                        }
                    }
                }
                Event::Stream(StreamEvent::Stopped { id, .. }) | Event::Stream(StreamEvent::Finished { id }) => {
                    if id == sid && open {
                        open = false;
                        say.send(Fact::Dc(false));
                    }
                }
                _ => {}
            }
        }
        let now = Instant::now();
        while let Some(t) = a.poll_transmit(now) {
            alive &= write_payload(&dtls, t.payload).await;
        }
        if !alive {
            rh.abort();
            say.send(Fact::Dtls(false));
            return;
        }
        let deadline = a.poll_timeout();
        let sleep = async {
            match deadline {
                Some(d) => tokio::time::sleep_until(tokio::time::Instant::from_std(d)).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            p = pkt_rx.recv() => {
                let Some(p) = p else {
                    say.send(Fact::Dtls(false));
                    return;
                };
                if let Some((_, DatagramEvent::AssociationEvent(e))) = endpoint.handle(Instant::now(), remote, None, None, Bytes::from(p)) {
                    a.handle_event(e);
                }
            }
            c = ctl.recv() => match c {
                None => {
                    rh.abort();
                    return;
                }
                Some(SctpCtl::Send(b)) => {
                    if open
                        && let Ok(mut s) = a.stream(sid) {
                            let _ = s.write_sctp(&Bytes::from(b), PayloadProtocolIdentifier::Binary);
                        }
                }
                Some(SctpCtl::Reopen) => {
                    if let Ok(mut s) = a.stream(sid) {
                        let _ = s.stop();
                    }
                    open = false;
                    sid = sid.wrapping_add(2);
                    open_stream(&mut a, sid);
                }
            },
            _ = sleep => a.handle_timeout(Instant::now()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtx_original_restores_seq_pt_ssrc_and_keeps_csrc_ext() {
        // SDK:RULE-PLAT-REC-TAP — RTX(PT 97 · ssrc 0xBB · seq 900 · OSN 57) → 원래 패킷(PT 96 · ssrc 0xAA · seq 57) · CSRC · 확장 그대로 · 채움 뗌
        let mut p = vec![0xB1, 0x80 | 97, 0x03, 0x84, 0, 0, 0, 9, 0, 0, 0, 0xBB];
        p.extend_from_slice(&[0, 0, 0, 0x12]);
        p.extend_from_slice(&[0xBE, 0xDE, 0, 1, 1, 2, 3, 4]);
        p.extend_from_slice(&[0, 57, 0xAA, 0xCC]);
        p.extend_from_slice(&[0, 0, 0, 4]);
        let h = rtp_header(&p).unwrap();
        let o = rtx_original(&Bytes::from(p.clone()), &h, 57, 96, 0xAA);
        let g = rtp_header(&o).unwrap();
        assert_eq!((g.seq, g.pt, g.ssrc, g.csrc, g.marker), (57, 96, 0xAA, Some(0x12), true));
        assert_eq!(&o[g.start..g.end], &[0xAA, 0xCC]);
        assert_eq!(&o[12..24], &p[12..24], "CSRC · 확장 머리 그대로");
        assert_eq!(o[0] & 0x20, 0, "채움 비트 내림");
        assert_eq!(o.len(), 24 + 2);
    }

    #[test]
    fn rtp_header_skips_csrc_ext_padding() {
        let mut p = vec![0xB1, 0xE0, 0x00, 0x05, 0, 0, 0, 9, 0, 0, 0, 7];
        p.extend_from_slice(&[0, 0, 0, 1]);
        p.extend_from_slice(&[0xBE, 0xDE, 0, 1, 1, 2, 3, 4]);
        p.extend_from_slice(&[0xAA, 0xBB, 0, 2]);
        let h = rtp_header(&p).unwrap();
        assert!(h.marker);
        assert_eq!((h.pt, h.seq, h.ts, h.ssrc), (0x60, 5, 9, 7));
        assert_eq!(h.csrc, Some(1));
        assert_eq!(&p[h.start..h.end], &[0xAA, 0xBB]);
        let plain = [0x80, 0x60, 0, 5, 0, 0, 0, 9, 0, 0, 0, 7, 0xAA];
        assert_eq!(rtp_header(&plain).unwrap().csrc, None);
    }

    #[test]
    fn dcep_open_is_unordered_label() {
        let m = dcep_open();
        assert_eq!(&m[..2], &[DCEP_OPEN, DCEP_PARTIAL_REXMIT_UNORDERED]);
        assert_eq!(&m[12..], CHANNEL_LABEL.as_bytes());
    }
}
