// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§12 · §17-2 · model: claude-opus-5

//! UDP 수신 루프 — ★**포트 하나에 모두 온다**(정§12 배치 전제).
//!
//! ★★**통로 장부는 이 태스크 혼자 쓴다 — 자물쇠가 없다**(핫패스 규율 H2). 데이터그램마다
//! 만지는 자료라 공유하면 그 자물쇠가 곧 병목이다. 바깥에서 건드릴 일(회수)은 ★**명령
//! 채널**로 들어온다 — 상태를 나눠 갖지 않고 한 곳에서만 바꾼다.
//!
//! ★**키는 ufrag 다**(정§12) — 주소는 latch 가 옮기므로 부차 색인일 뿐이다.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use super::conn::DemuxConn;
use super::demux::{classify, Packet};
use super::ice::{Binding, DropWhy, IceTable};
use super::dtls;

/// 한 스트림이 갈 곳 하나. ★**제어 평면이 미리 계산해 밀어 넣는다.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// 그 구독자의 **받기** 자격 — 주소는 latch 가, 열쇠는 그 ufrag 의 것이 답한다.
    pub ufrag: String,
    /// ★**구독자 표의 PT**(정§7-2-1) — 발행자 값이 아니다.
    pub pt: u8,
    /// ★**반이중이면 방 슬롯의 SSRC**(정§8-1). 있으면 그 값으로 갈아 끼우고 seq 를 이어 붙인다.
    ///
    /// ★**N:1 이라 화자가 바뀌어도 재협상이 없다** — 그 대가가 이 재기록이다.
    pub slot: Option<u32>,
    /// ★★**공간 단 상한 — 이 구독자 것이다**(연§6-3 `SUBSCRIBE_LAYER`).
    ///
    /// ★**"지정"이 아니라 "상한"이다** — 그 단이 없으면 실제 선택은 그 아래에서 난다.
    /// ★**스트림당 하나로 두면 안 된다**: 한 사람이 낮춰 달라고 한 것이 방 전원의 화질을 깎는다.
    pub spatial_cap: Option<u8>,
    /// ★**별개 축이다**(연§6-3) — *"안 받는다"* 와 *"낮은 화질로 받는다"* 는 다른 것이다.
    pub paused: bool,
    /// 재전송용 `(ssrc, pt)` — ★**발행자가 선언한 값을 쓴다**(정§11-1). 없으면 재전송을 안 한다.
    pub rtx: Option<(u32, u8)>,
}

/// 바깥에서 루프에 거는 것. ★**루프의 자료를 직접 만지지 않는다.**
///
/// ★★**전달표는 제어 평면이 밀고 데이터 평면은 제 것만 읽는다**(핫패스 규율 H2).
/// 패킷마다 방·명단·배정을 자물쇠 뒤에서 찾아보면 그 자물쇠가 곧 상한이 된다.
#[derive(Debug)]
pub enum Cmd {
    /// 그 세션을 거둔다 — ★**통로를 닫으면 DTLS 태스크가 끝난다**(정§12).
    DropSession(String),
    /// DTLS 가 섰다 — 그 자격의 SRTP 두 벌.
    SrtpReady { ufrag: String, keys: Box<super::dtls::SrtpKeys> },
    /// 그 발행 스트림이 갈 곳 전부. ★**빈 목록이면 아무 데도 안 간다**(지우는 것과 같다).
    ///
    /// ★★**키가 `ssrc` 하나면 안 된다** — SSRC 는 ★**발행자마다 제 공간**이라 다른 세션이
    /// 같은 값을 쓸 수 있다(클라가 재접속하면 흔히 같은 값을 다시 쓴다). `ssrc` 만으로
    /// 키를 잡으면 ★**죽은 세션의 전달표가 새 세션을 가로채** 산 스트림이 이미 사라진
    /// 구독자에게 간다(실측 20260912 — 오래 사는 서버에서만 드러난다).
    SetRoute { ufrag: String, ssrc: u32, targets: Vec<Target> },
    /// 그 자격의 DC 로 한 장. ★**채널이 아직이면 버린다** — 막지 않는다(정§13).
    DcSend { ufrag: String, wire: Vec<u8> },
    /// ★**그 발행 자격에서 `rid` 를 달고 오는 것은 이 `vssrc` 것**이다(정§10-1).
    ///
    /// 등록 항목이 `rid` 를 안 싣기 때문에(연§6-3) 어느 ssrc 가 어느 단인지는
    /// ★**RTP 로 배워야 한다** — 배울 대상을 알려 주는 것이 이 명령이다.
    ///
    /// ★**코덱도 같이 온다** — 단 전환이 서는 자리가 키프레임이고(정§10-2), 그 판정기는
    /// ★**코덱을 알아야 고른다**(모른 채 둘 다 돌리면 엉뚱한 쪽이 답한다, `keyframe` 모듈).
    SetSimulcast { ufrag: String, vssrc: u32, codec: Option<crate::keyframe::Codec> },
}

/// DC 로 들어온 것 — ★**판정은 제어 평면이 한다**(여기는 나르기만).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DcIn {
    pub ufrag: String,
    pub svc: u8,
    pub payload: Vec<u8>,
}

/// ★**데이터 평면이 1초마다 갈아 끼우는 사본** — 읽는 쪽은 자물쇠가 없다(H2 · RCU).
///
/// 키는 `(받는 자격, egress ssrc)`, 값은 ★**그 구독으로 내보낸 수**다.
/// ★★**정체 판정이 보는 것이 이 값이다**(정§14-3) — *"너에게 나가야 할 것이 안 나간다"* 는
/// 데이터 평면만 아는 사실이라, 제어 평면이 그것을 보려면 이렇게 건너와야 한다.
/// ★핫패스는 이 자료를 안 만진다 — 타이머가 제 사본을 지어 갈아 끼운다.
pub type EgressView = arc_swap::ArcSwap<HashMap<(String, u32), u64>>;

/// 데이터그램 상한 — ★**한 장이 이보다 크면 우리 것이 아니다.**
const MTU: usize = 2048;

/// 한 자격의 DTLS 통로.
///
/// ★★**통로를 닫는 것만으로는 회수가 아니다.** 보내는 끝을 떨어뜨리면 `recv` 가 오류를
/// 내는데, dtls 크레이트의 읽기 고리는 그 오류를 물고 ★**한 코어를 100% 로 돈다**(실측
/// 20260912 — `DTLSConn::read_and_buffer` 에서 1,214 표본 중 전량). 그래서 태스크를
/// ★**직접 끊는다** — 정§12 가 *"회수 시 태스크 종료 필수"* 라 못박은 자리다.
struct Pipe {
    session_id: String,
    /// ★**자격 이름** — SRTP 열쇠도 latch 도 이 키로 찾는다(정§12 세션 동일성).
    ufrag: String,
    tx: mpsc::Sender<Bytes>,
    /// DC 로 내보낼 것. ★**채널이 서기 전에도 받아 둔다** — SCTP 루프가 열리면 흘린다.
    dc: mpsc::Sender<Vec<u8>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Pipe {
    /// ★**잊을 수 없게 여기 둔다** — 지우는 자리마다 끊으라고 적으면 한 군데는 빠진다.
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// 루프가 센 것 — ★**조용히 버린 것도 센다**(세지 않으면 없는 일이 된다).
///
/// ★**세기만 하고 아무도 안 보면 안 센 것과 같다** — 그래서 루프가 주기로 한 줄 남긴다.
/// 운영 표면(정§16-1)에 붙이는 것은 운영 덩어리의 일이다.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counters {
    pub stun_ok: u64,
    pub stun_dropped: u64,
    pub forged: u64,
    pub dtls_in: u64,
    pub srtp_in: u64,
    /// ★인증이 안 맞아 버린 것 — 위조이거나 열쇠가 어긋난 것이다.
    pub srtp_bad: u64,
    pub srtp_out: u64,
    pub rtcp_in: u64,
    pub rtcp_out: u64,
    pub rtcp_rr_in: u64,
    pub rtcp_fb_in: u64,
    pub rtcp_ignored: u64,
    /// rid 가 없어 단을 모르는 패킷 — ★**지어내지 않고 버린 것**이다.
    pub sim_unknown: u64,
    /// 지금 안 보내는 단이라 버린 것.
    pub sim_dropped: u64,
    /// 키프레임 경계에서 단을 갈아탄 수.
    pub sim_switch: u64,
    /// target 단 키프레임이 끝내 안 와 폐기한 전환 — ★**조용히 버리지 않는다**(정§10-2 증상).
    pub sim_pending_expired: u64,
    /// 자동 레이어가 내린 판정.
    pub auto_promote: u64,
    pub auto_demote: u64,
    /// 프로브로 내보낸 패딩 수.
    pub probe_out: u64,
    /// 발행자에게 돌려준 TWCC 피드백 수.
    pub twcc_fb_out: u64,
    /// 발행자에게 낸 REMB 수.
    pub remb_out: u64,
    /// 구독자 PLI 를 발행자에게 옮긴 수.
    pub pli_relay: u64,
    /// 되짚을 발행자를 못 찾아 버린 PLI — ★**조용히 버리지 않는다.**
    pub pli_orphan: u64,
    /// latch 를 안 지난 주소에서 온 것 — ★**우리가 버린 자리**다.
    pub no_latch: u64,
    /// 열쇠가 아직 없어 못 푼 것.
    pub no_key: u64,
    pub nack_in: u64,
    /// 관문별 사유 — ★**조용한 drop 금지**(정§11-1 *"관문마다 사유별 계수"*).
    pub nack_no_cache: u64,
    pub nack_no_rtx: u64,
    pub nack_budget: u64,
    pub nack_miss: u64,
    /// 그 구독자의 주소·열쇠를 못 찾은 것.
    pub nack_no_route: u64,
    /// RTX 를 못 짓거나 못 봉한 것 — ★**여기가 계수 없이 비면 "왜 0 인가" 를 못 짚는다.**
    pub nack_build: u64,
    pub rtx_out: u64,
    pub unknown: u64,
}

impl Counters {
    fn line(&self) -> String {
        format!(
            "stun {}/{}(위조 {}) · dtls {} · srtp in {}(버림 {}) out {} · rtcp in {} out {} · 단 모름 {} 안 보냄 {} 전환 {} 만료 {} · 자동 ↑{} ↓{} 프로브 {} twcc {} remb {} pli {}(고아 {}) · latch 전 {} 열쇠 전 {} · nack {}(캐시없음 {} rtx없음 {} 길없음 {} 예산 {} 못찾음 {} 못지음 {}) rtx {} · 모름 {}",
            self.stun_ok,
            self.stun_ok + self.stun_dropped,
            self.forged,
            self.dtls_in,
            self.srtp_in,
            self.srtp_bad,
            self.srtp_out,
            self.rtcp_in,
            self.rtcp_out,
            self.sim_unknown,
            self.sim_dropped,
            self.sim_switch,
            self.sim_pending_expired,
            self.auto_promote,
            self.auto_demote,
            self.probe_out,
            self.twcc_fb_out,
            self.remb_out,
            self.pli_relay,
            self.pli_orphan,
            self.no_latch,
            self.no_key,
            self.nack_in,
            self.nack_no_cache,
            self.nack_no_rtx,
            self.nack_no_route,
            self.nack_budget,
            self.nack_miss,
            self.nack_build,
            self.rtx_out,
            self.unknown
        )
    }
}

/// 계수 한 줄을 남기는 주기.
const REPORT_MS: u64 = 30_000;

/// ★발행자에게 RR 을 내는 주기(정§11-2) — ★**소비자는 타이머 하나다.**
const RR_MS: u64 = 1_000;

/// 서버가 선언하는 rid 확장 번호(`identity::extmap`) — 발행자 신고가 없으면 이 값이다.
const RID_EXT_ID: u8 = 10;
/// 서버가 선언하는 transport-cc 번호 — ★**구독자는 이 번호로 받는 m-line 을 짓는다.**
const TWCC_EXT_ID: u8 = 6;

/// ★발행자에게 돌려주는 TWCC 주기(정§11-2) — 브라우저 관례와 같은 값이다.
const TWCC_FB_MS: u64 = 100;

/// 서버가 RTCP 에 쓰는 제 SSRC — ★**미디어 SSRC 와 겹치지 않는 고정값**이다.
const SERVER_SSRC: u32 = 0x0000_0001;

/// 포트를 열고 루프를 돈다. ★**되돌아오지 않는다** — 태스크로 띄운다.
#[allow(clippy::too_many_arguments)]
pub async fn serve(
    socket: Arc<UdpSocket>,
    table: Arc<IceTable>,
    cert: dtls::Certificate,
    cmd_tx: mpsc::Sender<Cmd>,
    mut cmds: mpsc::Receiver<Cmd>,
    dc_in: mpsc::Sender<DcIn>,
    view: Arc<EgressView>,
    // ★**그 배포의 상한**(정§11-2 REMB) — 우리가 발행자에게 말하는 천장이다.
    max_bitrate_bps: u64,
) {
    // ★**그릇을 하나 잡아 재사용한다** — 데이터그램마다 새로 잡지 않는다(H3).
    let mut buf = vec![0u8; MTU];
    let mut by_ufrag: HashMap<String, Arc<Pipe>> = HashMap::new();
    let mut by_addr: HashMap<SocketAddr, Arc<Pipe>> = HashMap::new();
    // ★이 둘도 이 태스크 혼자 쓴다 — 자물쇠가 없다.
    let mut srtp: HashMap<String, super::srtp::SrtpPair> = HashMap::new();
    // ★키는 `(발행 자격, ssrc)` 다 — 위 `SetRoute` 의 이유와 같다.
    let mut routes: HashMap<(String, u32), Vec<Target>> = HashMap::new();
    // ★**전송로마다 스칼라 둘** — 구간 지도를 두면 화자 교대에서 egress seq 가 역행한다.
    let mut rewriters: HashMap<(String, u32), crate::rewriter::Rewriter> = HashMap::new();
    // ★**발행 스트림마다 수신 통계** — 핫패스가 갱신하고 1초 타이머가 소비한다(정§11-2).
    //   값은 `(그 자격, 통계)` 라 회수 때 같이 간다.
    let mut stats: HashMap<(String, u32), crate::rtcp::RecvStats> = HashMap::new();
    // ★시뮬캐스트 — `발행 자격 → vssrc` 는 제어 평면이 알려 주고,
    //   `들어온 ssrc → (vssrc, 단)` 은 rid 로 ★**배운다**(정§10-1).
    let mut sim_of: HashMap<String, u32> = HashMap::new();
    let mut layer_of: HashMap<(String, u32), (u32, u8)> = HashMap::new();
    // ★**구독자마다 지금 내보내는 단** — 키가 `(받는 자격, vssrc)` 다.
    //   상한은 사람마다 다르므로 스트림 하나에 값 하나를 두면 남의 상한이 내 화질을 깎는다.
    let mut sim_out: HashMap<(String, u32), Forward> = HashMap::new();
    // ★**발행 자격의 코덱** — 키프레임 판정기를 고르는 데만 쓴다(정§10-2).
    let mut codec_of: HashMap<String, crate::keyframe::Codec> = HashMap::new();
    // ★**PLI 스로틀 버킷** — 단마다 다르다(정§10-3). 키는 `(발행 자격, 그 단의 ssrc)`.
    let mut pli_at: HashMap<(String, u32), u64> = HashMap::new();
    // ★구독자에게 내보낸 수 — SR 번역이 이 값으로 카운터를 갈아 끼운다.
    let mut egress: HashMap<(String, u32), (u32, u32)> = HashMap::new();
    // ★**발행 전송로마다 도착 장부 하나**(정§11-2) — 발행자가 매긴 seq 를 그대로 돌려준다.
    //   ★없으면 발행자 송신 추정이 갱신되지 않아 화질이 안 올라간다.
    let mut up: HashMap<String, crate::twcc::RecvLedger> = HashMap::new();
    // ★★**구독 전송로마다 하나** — 스탬핑 장부·대역 추정·단 판정이 ★**한 자리**에 있다.
    //   흩어 두면 "무엇을 보고 내렸나" 를 되짚을 수 없다(정§10-3 판정은 순수 함수).
    let mut down: HashMap<String, Downlink> = HashMap::new();
    // ★**보낸 것을 잠깐 들고 있는다** — NACK 이 오면 그 자리에서 꺼내 되보낸다(정§11-1 관문 ②).
    //   키는 `(받는 자격, egress ssrc)`, 값은 링버퍼다.
    let mut cache: HashMap<(String, u32), SendCache> = HashMap::new();
    // ★**내보낼 것을 담는 그릇 하나** — 패킷마다 새로 잡지 않는다(H3).
    let mut scratch: Vec<u8> = Vec::with_capacity(MTU);
    let mut c = Counters::default();
    let mut last = Counters::default();
    let mut report = tokio::time::interval(std::time::Duration::from_millis(REPORT_MS));
    report.tick().await;
    let mut rr = tokio::time::interval(std::time::Duration::from_millis(RR_MS));
    rr.tick().await;
    // ★**프로브 청크 시계**(정§10-3) — 도는 프로브가 없으면 그 자리에서 돌아선다.
    //   burst 로 한 번에 쏟으면 그것이 곧 혼잡이라 ★**나눠 보내는 것이 계약**이다.
    let mut chunk =
        tokio::time::interval(std::time::Duration::from_millis(crate::autolayer::v::PROBE_CHUNK_MS));
    chunk.tick().await;
    // ★**발행자에게 돌려주는 TWCC 주기**(정§11-2) — 100ms 다.
    let mut fb = tokio::time::interval(std::time::Duration::from_millis(TWCC_FB_MS));
    fb.tick().await;

    loop {
        let (n, from) = tokio::select! {
            r = socket.recv_from(&mut buf) => match r {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[udp] recv: {e}");
                    continue;
                }
            },
            _ = rr.tick() => {
                // ★**자체 생성이다** — 발행자가 보는 *"우리 수신 품질"* 이고, 구독자 RR 을
                //   릴레이한 것이 아니다(릴레이하면 발행자가 남의 품질로 비트레이트를 깎는다).
                let now = now_ms();
                for (ufrag, blocks) in rr_blocks(&mut stats, now) {
                    let (Some(ctx), Some(dst)) =
                        (srtp.get_mut(&ufrag), table.get(&ufrag).and_then(|e| e.addr()))
                    else {
                        continue;
                    };
                    let plain = crate::rtcp::build_rr(SERVER_SSRC, &blocks);
                    if let Some(sealed) = ctx.seal_rtcp(&plain) {
                        let _ = socket.send_to(&sealed, dst).await;
                        c.rtcp_out += 1;
                    }
                }
                // ★**발행자에게 상한을 말한다**(정§11-2) — 값은 `min(수신측 추정, 상한)` 이고,
                //   ★**추정이 없으면 상한만** 말한다(없는 값을 `0` 으로 지어내지 않는다).
                for ufrag in stats.keys().map(|(f, _)| f.clone()).collect::<std::collections::BTreeSet<_>>() {
                    let mine: Vec<u32> = stats
                        .keys()
                        .filter(|(f, _)| f == &ufrag)
                        .map(|(_, s)| *s)
                        .collect();
                    let est = routes
                        .iter()
                        .filter(|((f, _), _)| f == &ufrag)
                        .flat_map(|(_, t)| t.iter())
                        .filter_map(|t| down.get(&t.ufrag))
                        .filter(|d| d.last_fb_ms.is_some())
                        .map(|d| d.gcc.estimate_bps())
                        .min();
                    let bps = est.map(|e| e.min(max_bitrate_bps)).unwrap_or(max_bitrate_bps);
                    let (Some(ctx), Some(dst)) =
                        (srtp.get_mut(&ufrag), table.get(&ufrag).and_then(|e| e.addr()))
                    else {
                        continue;
                    };
                    let plain = crate::rtcp::build_remb(SERVER_SSRC, bps, &mine);
                    if let Some(sealed) = ctx.seal_rtcp(&plain) {
                        let _ = socket.send_to(&sealed, dst).await;
                        c.rtcp_out += 1;
                        c.remb_out += 1;
                    }
                }
                // ★**내보낸 수를 제어 평면에 건넨다**(정§14-3) — 사본을 지어 갈아 끼운다.
                view.store(Arc::new(
                    egress.iter().map(|(k, (p, _))| (k.clone(), *p as u64)).collect(),
                ));
                // ★**판정도 1초 한 번이다**(정§10-3 tick 1,000ms) — 타이머를 또 두지 않는다.
                downlink_tick(
                    now, &mut down, &routes, &sim_of, &layer_of, &mut sim_out, &mut pli_at,
                    &mut srtp, &table, &socket, &mut c,
                )
                .await;
                continue;
            },
            _ = chunk.tick() => {
                probe_chunk(&mut down, &mut srtp, &table, &socket, &mut c).await;
                continue;
            },
            _ = fb.tick() => {
                // ★**발행자 축이다** — 우리가 잰 도착 시각을 그대로 돌려준다.
                for (ufrag, led) in up.iter_mut() {
                    if led.is_empty() {
                        continue;
                    }
                    let media = stats
                        .keys()
                        .find(|(f, _)| f == ufrag)
                        .map(|(_, s)| *s)
                        .unwrap_or(0);
                    let (Some(ctx), Some(dst)) =
                        (srtp.get_mut(ufrag), table.get(ufrag).and_then(|e| e.addr()))
                    else {
                        continue;
                    };
                    while let Some(plain) = led.build(SERVER_SSRC, media) {
                        let Some(sealed) = ctx.seal_rtcp(&plain) else { break };
                        let _ = socket.send_to(&sealed, dst).await;
                        c.rtcp_out += 1;
                        c.twcc_fb_out += 1;
                    }
                }
                continue;
            },
            _ = report.tick() => {
                // ★**바뀐 것이 없으면 안 적는다** — 조용한 로그가 조용한 서버를 말한다.
                if c != last {
                    eprintln!("[udp] {}", c.line());
                    last = c;
                }
                continue;
            },
            Some(cmd) = cmds.recv() => {
                match cmd {
                    Cmd::DropSession(sid) => {
                        // ★통로를 닫는 것이 회수다 — 보내는 끝이 사라지면 DTLS 가 끝난다.
                        let gone: Vec<String> = by_ufrag
                            .iter()
                            .filter(|(_, p)| p.session_id == sid)
                            .map(|(u, _)| u.clone())
                            .collect();
                        by_ufrag.retain(|_, p| p.session_id != sid);
                        by_addr.retain(|_, p| p.session_id != sid);
                        for u in &gone {
                            srtp.remove(u);
                            rewriters.retain(|(f, _), _| f != u);
                            stats.retain(|(f, _), _| f != u);
                            egress.retain(|(f, _), _| f != u);
                            routes.retain(|(f, _), _| f != u);
                            layer_of.retain(|(f, _), _| f != u);
                            sim_of.remove(u);
                            sim_out.retain(|(f, _), _| f != u);
                            codec_of.remove(u);
                            pli_at.retain(|(f, _), _| f != u);
                            down.remove(u);
                            up.remove(u);
                            cache.retain(|(f, _), _| f != u);
                            // ★**갈 곳 목록에서도 뺀다** — 안 빼면 죽은 자격으로 계속 잠근다.
                            for t in routes.values_mut() {
                                t.retain(|x| &x.ufrag != u);
                            }
                        }
                    }
                    Cmd::SrtpReady { ufrag, keys } => match super::srtp::SrtpPair::new(&keys) {
                        Ok(p) => {
                            srtp.insert(ufrag, p);
                        }
                        Err(e) => eprintln!("[srtp] {ufrag}: {e}"),
                    },
                    Cmd::DcSend { ufrag, wire } => {
                        if let Some(p) = by_ufrag.get(&ufrag) {
                            // ★**막지 않는다**(정§13) — 넘치면 그 장을 버리고 계수로 남는다.
                            let _ = p.dc.try_send(wire);
                        }
                    }
                    Cmd::SetSimulcast { ufrag, vssrc, codec } => {
                        if let Some(cd) = codec {
                            codec_of.insert(ufrag.clone(), cd);
                        }
                        // ★★**재발행이면 배운 것을 버린다.** 브라우저는 같은 단 SSRC 를 다시
                        //   쓸 수 있는데, 옛 `vssrc` 로 배워 둔 지도가 남아 있으면 새 패킷이
                        //   ★**이미 지워진 길로 가서 조용히 사라진다**(실측 20260912).
                        if let Some(old) = sim_of.insert(ufrag.clone(), vssrc)
                            && old != vssrc
                        {
                            layer_of.retain(|(f, _), (v, _)| f != &ufrag || *v != old);
                            sim_out.retain(|(_, v), _| *v != old);
                        }
                    }
                    Cmd::SetRoute { ufrag, ssrc, targets } => {
                        if targets.is_empty() {
                            routes.remove(&(ufrag, ssrc));
                        } else {
                            routes.insert((ufrag, ssrc), targets);
                        }
                    }
                }
                continue;
            }
        };
        let now = now_ms();
        match classify(&buf[..n]) {
            Packet::Stun => {
                match super::ice::on_binding(&table, &buf[..n], from, now) {
                    Binding::Respond { wire, ufrag, session_id, latched } => {
                        c.stun_ok += 1;
                        let _ = socket.send_to(&wire, from).await;
                        if latched {
                            let pipe = match by_ufrag.get(&ufrag) {
                                Some(p) => p.clone(),
                                None => {
                                    let Some(e) = table.get(&ufrag) else { continue };
                                    let (conn, tx) = DemuxConn::new(socket.clone(), e.addr.clone());
                                    let (dc_tx, dc_rx) = mpsc::channel(64);
                                    let task = spawn_dtls(
                                        conn,
                                        cert.clone(),
                                        ufrag.clone(),
                                        cmd_tx.clone(),
                                        dc_rx,
                                        dc_in.clone(),
                                    );
                                    let p = Arc::new(Pipe {
                                        session_id: session_id.clone(),
                                        ufrag: ufrag.clone(),
                                        tx,
                                        dc: dc_tx,
                                        task,
                                    });
                                    by_ufrag.insert(ufrag.clone(), p.clone());
                                    p
                                }
                            };
                            // ★**옛 주소를 지우고 새 주소를 건다** — 세션은 그대로다(ufrag 가 키다).
                            by_addr.retain(|_, p| !Arc::ptr_eq(p, &pipe));
                            by_addr.insert(from, pipe);
                        }
                    }
                    Binding::Drop(why) => {
                        c.stun_dropped += 1;
                        if why == DropWhy::BadIntegrity {
                            // ★위조는 따로 센다 — 섞으면 *"오래된 클라"* 와 *"공격"* 이 한 수다.
                            c.forged += 1;
                        }
                    }
                }
            }
            Packet::Dtls => {
                c.dtls_in += 1;
                // ★**latch 를 지난 주소만 DTLS 를 탄다** — 그 전 것은 아무것도 아니다.
                if let Some(p) = by_addr.get(&from) {
                    let _ = p.tx.try_send(Bytes::copy_from_slice(&buf[..n]));
                }
            }
            Packet::Srtp => {
                // ★**latch 를 지난 주소만 미디어를 탄다** — 그 전 것은 아무것도 아니다.
                //   ★**세고 버린다** — 여기가 계수 없는 drop 이면 *"어디서 없어졌나"* 를
                //   영영 못 짚는다(조용한 drop 금지).
                let Some(p) = by_addr.get(&from) else {
                    c.no_latch += 1;
                    continue;
                };
                let ufrag = p.ufrag.clone();
                // ★RTP 와 RTCP 는 같은 대역으로 온다(RFC 5761) — 둘째 바이트가 가른다.
                if crate::rtcp::is_rtcp(&buf[..n]) {
                    c.rtcp_in += 1;
                    let Some(ctx) = srtp.get_mut(&ufrag) else { continue };
                    let Some(plain) = ctx.open_rtcp(&buf[..n]) else {
                        c.srtp_bad += 1;
                        continue;
                    };
                    on_rtcp(&plain, &ufrag, &mut stats, &mut egress, &mut cache, &routes, &rewriters, &mut down, &sim_out, &layer_of, &mut pli_at, &table, &mut srtp, &socket, &mut c).await;
                    continue;
                }
                c.srtp_in += 1;
                let Some(ctx) = srtp.get_mut(&ufrag) else {
                    c.no_key += 1;
                    continue;
                };
                // ★인증이 안 맞으면 버린다 — 위조가 fan-out 을 타면 남의 화면에 남의 것이 뜬다.
                let Some(plain) = ctx.open(&buf[..n]) else {
                    c.srtp_bad += 1;
                    continue;
                };
                let Some(ssrc) = super::srtp::ssrc_of(&plain) else { continue };
                // ★그 자격이 살아 있다는 뜻이다 — 좀비 판정의 두 갱신원 중 하나(정§2-2).
                if let Some(e) = table.get(&ufrag) {
                    e.touch(now);
                }
                // ★**발행자가 매긴 번호를 그대로 적는다**(정§11-2) — 돌려줄 때도 그 번호다.
                //   ★egress 와 정반대다: 그쪽은 우리가 매기고, 여기는 받아 적는다.
                if let Some(v) = crate::rtpext::get(&plain, TWCC_EXT_ID)
                    && v.len() >= 2
                {
                    up.entry(ufrag.clone())
                        .or_default()
                        .on_rtp(u16::from_be_bytes([v[0], v[1]]), now);
                }
                {
                    // ★RR 의 재료 — 핫패스에서 세고 타이머가 소비한다.
                    let seq = u16::from_be_bytes([plain[2], plain[3]]);
                    let ts = u32::from_be_bytes([plain[4], plain[5], plain[6], plain[7]]);
                    stats
                        .entry((ufrag.clone(), ssrc))
                        .or_insert_with(|| crate::rtcp::RecvStats::new(ssrc, 48_000))
                        .on_rtp(seq, ts, now);
                }
                // ★**시뮬캐스트는 들어온 ssrc 가 곧 스트림이 아니다** — 단마다 다르다.
                //   rid 로 배워서 vssrc 로 합치고, 지금 고른 단만 내보낸다.
                let mut out_ssrc = ssrc;
                let mut sim_spatial: Option<u8> = None;
                // ★★**이미 아는 ssrc 는 그 스트림 것이다** — 같은 발행자가 audio 와 시뮬캐스트
                //   video 를 같이 올리므로, 자격만 보고 시뮬캐스트 갈래로 보내면 ★**audio 가
                //   rid 가 없다는 이유로 통째로 버려진다**(실측 20260912: 468 중 1 만 도착).
                //   갈래를 가르는 것은 자격이 아니라 ★**그 ssrc 를 아는가**다.
                if !routes.contains_key(&(ufrag.clone(), ssrc))
                    && let Some(&vssrc) = sim_of.get(&ufrag)
                {
                    let known = layer_of.get(&(ufrag.clone(), ssrc)).copied();
                    let (v, spatial) = match known {
                        Some(v) => v,
                        None => {
                            // ★rid 가 없으면 단을 모른다 — 지어내지 않고 버린다.
                            let Some(spatial) = crate::rtpext::rid(&plain, RID_EXT_ID)
                                .and_then(crate::rtpext::spatial_of)
                            else {
                                c.sim_unknown += 1;
                                continue;
                            };
                            layer_of.insert((ufrag.clone(), ssrc), (vssrc, spatial));
                            (vssrc, spatial)
                        }
                    };
                    sim_spatial = Some(spatial);
                    out_ssrc = v;
                }
                let Some(targets) = routes.get(&(ufrag.clone(), out_ssrc)) else { continue };
                // ★**키프레임 판정은 스트림마다 한 번** — 구독자 수만큼 다시 풀지 않는다(H4).
                let is_key = sim_spatial.is_some()
                    && codec_of.get(&ufrag).is_some_and(|cd| cd.is_keyframe(&plain));
                for t in targets {
                    // ★**받지 않겠다는 사람에게는 안 보낸다** — 레이어 축과 별개다.
                    if t.paused {
                        continue;
                    }
                    // ★**단 고르기는 사람마다** 한다(연§6-3 — 상한은 그 구독자 것이다).
                    if let Some(spatial) = sim_spatial {
                        // ★**수동 상한과 자동 cap 은 `min`**(정§10-1) — 자동이 올려도 못 넘는다.
                        let cap = t.spatial_cap.unwrap_or(u8::MAX);
                        let auto = down
                            .get(&t.ufrag)
                            .map(|d| d.policy.cap.spatial())
                            .unwrap_or(crate::autolayer::Layer::High.spatial());
                        let want = cap.min(auto);
                        let f = sim_out
                            .entry((t.ufrag.clone(), out_ssrc))
                            .or_insert(Forward { current: spatial.min(want), target: None });
                        // ★★**전환은 target 단의 키프레임에서만 선다**(정§10-2) — 아무 데서나
                        //   갈아타면 새 단의 첫 프레임이 ★**없는 앞 프레임을 참조**해 깨진다.
                        if let Some((tgt, since)) = f.target {
                            if tgt == spatial && is_key {
                                f.current = tgt;
                                f.target = None;
                                c.sim_switch += 1;
                            } else if now.saturating_sub(since) > crate::autolayer::v::PENDING_MS {
                                // ★**만료도 센다** — 조용히 버리면 *"왜 안 바뀌나"* 를 못 짚는다.
                                f.target = None;
                                c.sim_pending_expired += 1;
                            }
                        }
                        if spatial != f.current {
                            c.sim_dropped += 1;
                            continue;
                        }
                    }
                    if table.get(&t.ufrag).and_then(|e| e.addr()).is_none() {
                        continue;
                    }
                    // ★**제자리 재기록 · 길이 불변** — 본문은 건드리지 않는다(H1).
                    scratch.clear();
                    scratch.extend_from_slice(&plain);
                    match t.slot.or(if out_ssrc == ssrc { None } else { Some(out_ssrc) }) {
                        None => {
                            if !super::srtp::rewrite_pt(&mut scratch, t.pt) {
                                continue;
                            }
                        }
                        // ★반이중 — 방 슬롯 하나를 화자들이 돌려쓴다. SSRC 를 슬롯 것으로 갈고
                        //   seq·ts 는 ★**직전 egress 의 다음**으로 이어 붙인다(정§8-1).
                        Some(slot) => {
                            let rw = rewriters.entry((t.ufrag.clone(), slot)).or_default();
                            let in_seq = u16::from_be_bytes([scratch[2], scratch[3]]);
                            let in_ts = u32::from_be_bytes([
                                scratch[4], scratch[5], scratch[6], scratch[7],
                            ]);
                            let out = rw.map(ssrc, in_seq, in_ts);
                            if crate::rewriter::Rewriter::apply(&mut scratch, out, t.pt).is_err() {
                                continue;
                            }
                            scratch[8..12].copy_from_slice(&slot.to_be_bytes());
                        }
                    }
                    // ★**스탬핑·장부·봉인이 한 길이다**(정§11-2) — 이 길을 지나지 않은 것은
                    //   측정에 안 잡힌다(프로브도 같은 길로 보내는 까닭이다).
                    if send_to_sub(&mut scratch, &t.ufrag, &mut down, &mut srtp, &table, &socket, now)
                        .await
                    {
                        c.srtp_out += 1;
                        // ★**평문을 들고 있는다** — 되보낼 때 머리를 다시 써야 하므로
                        //   봉한 것을 그대로 쓸 수 없다(RTX 는 ssrc·pt·seq 가 다르다).
                        if let Some(rtx) = t.rtx {
                            let e = cache.entry((t.ufrag.clone(), out_ssrc)).or_default();
                            e.rtx = Some(rtx);
                            e.put(&scratch);
                        }
                        // ★SR 번역이 쓸 egress 카운터 — 발행자 수가 아니라 **내보낸 수**다.
                        let e = egress.entry((t.ufrag.clone(), out_ssrc)).or_insert((0, 0));
                        e.0 = e.0.wrapping_add(1);
                        e.1 = e.1.wrapping_add(scratch.len().saturating_sub(12) as u32);
                    }
                }
            }
            Packet::Unknown => c.unknown += 1,
        }
    }
}

/// 그 통로 위에 DTLS 를 세우고, 선 뒤에는 ★**같은 태스크에서 SCTP 를 돈다.**
///
/// ★**둘을 한 태스크에 두는 이유** — DTLS 연결은 SCTP 의 전송로라 수명이 같다.
/// 태스크를 갈라 두면 회수 신호를 두 번 보내야 하고, 한쪽만 지나가는 창이 생긴다.
fn spawn_dtls(
    conn: DemuxConn,
    cert: dtls::Certificate,
    ufrag: String,
    cmds: mpsc::Sender<Cmd>,
    dc_out: mpsc::Receiver<Vec<u8>>,
    dc_in: mpsc::Sender<DcIn>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let cfg = super::dtls::server_config(&cert);
        let c = match super::dtls::accept(Arc::new(conn), cfg).await {
            Ok(c) => c,
            // ★**조용히 실패하지 않는다** — 클라는 제 쪽 타임아웃만 보고 이유를 모른다.
            Err(e) => {
                eprintln!("[dtls] {ufrag} 핸드셰이크: {e}");
                return;
            }
        };
        match super::dtls::export_srtp(&c).await {
            Ok(keys) => {
                eprintln!("[dtls] {ufrag} 섰다 — SRTP 열쇠 넷");
                // ★열쇠는 ★**루프에게 넘긴다** — 잠그고 푸는 것은 데이터 평면의 일이다.
                let _ = cmds.send(Cmd::SrtpReady { ufrag: ufrag.clone(), keys: Box::new(keys) }).await;
            }
            Err(e) => {
                eprintln!("[dtls] {ufrag} 열쇠: {e}");
                return;
            }
        }
        // ★DC 는 ★**보내기용 연결에만** 붙는다(연§3-3) — 클라가 안 열면 아무 일도 없다.
        let (ev_tx, mut ev_rx) = tokio::sync::mpsc::channel(64);
        let who = ufrag.clone();
        tokio::spawn(async move {
            while let Some(e) = ev_rx.recv().await {
                match e {
                    super::sctp::DcEvent::Open => eprintln!("[dc] {who} \"unreliable\" 열렸다"),
                    super::sctp::DcEvent::Frame { svc, payload } => {
                        let _ = dc_in.send(DcIn { ufrag: who.clone(), svc, payload }).await;
                    }
                    super::sctp::DcEvent::Closed => eprintln!("[dc] {who} 닫혔다"),
                }
            }
        });
        super::sctp::run(&c, dc_out, ev_tx).await;
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 자격마다 RR 한 장에 담을 칸들. ★**구간은 여기서 한 번만 닫힌다**(정§11-2).
fn rr_blocks(
    stats: &mut HashMap<(String, u32), crate::rtcp::RecvStats>,
    now: u64,
) -> Vec<(String, Vec<crate::rtcp::ReportBlock>)> {
    let mut by: HashMap<String, Vec<crate::rtcp::ReportBlock>> = HashMap::new();
    for ((ufrag, _), s) in stats.iter_mut() {
        // ★아직 한 장도 못 받은 스트림은 낼 것이 없다 — 빈 칸을 지어내지 않는다.
        if s.received() == 0 {
            continue;
        }
        by.entry(ufrag.clone()).or_default().push(s.report(now));
    }
    by.into_iter().collect()
}

/// 들어온 RTCP 한 덩어리. ★**복호 뒤 평문에서 패킷 단위로 분해한다**(정§11-2 `1pc` 분해).
#[allow(clippy::too_many_arguments)]
async fn on_rtcp(
    plain: &[u8],
    ufrag: &str,
    stats: &mut HashMap<(String, u32), crate::rtcp::RecvStats>,
    egress: &mut HashMap<(String, u32), (u32, u32)>,
    cache: &mut HashMap<(String, u32), SendCache>,
    routes: &HashMap<(String, u32), Vec<Target>>,
    rewriters: &HashMap<(String, u32), crate::rewriter::Rewriter>,
    down: &mut HashMap<String, Downlink>,
    sim_out: &HashMap<(String, u32), Forward>,
    layer_of: &HashMap<(String, u32), (u32, u8)>,
    pli_at: &mut HashMap<(String, u32), u64>,
    table: &Arc<IceTable>,
    srtp: &mut HashMap<String, super::srtp::SrtpPair>,
    socket: &Arc<UdpSocket>,
    c: &mut Counters,
) {
    use crate::rtcp;
    let now = now_ms();
    for pkt in rtcp::split(plain) {
        match pkt[1] {
            rtcp::PT_SR => {
                let Some((ssrc, hi, lo)) = rtcp::sr_ntp(pkt) else { continue };
                if let Some(s) = stats.get_mut(&(ufrag.to_string(), ssrc)) {
                    s.on_sr(hi, lo, now);
                }
                // ★**자체 생성 금지 — 번역 릴레이다**(정§11-2). 구독자마다 값이 다르다.
                let Some(targets) = routes.get(&(ufrag.to_string(), ssrc)) else { continue };
                for t in targets {
                    let (Some(dst), Some(out)) =
                        (table.get(&t.ufrag).and_then(|e| e.addr()), srtp.get_mut(&t.ufrag))
                    else {
                        continue;
                    };
                    let (packets, octets) =
                        egress.get(&(t.ufrag.clone(), ssrc)).copied().unwrap_or((0, 0));
                    let patch = rtcp::SrPatch {
                        ssrc: t.slot,
                        // ★**RTP 전달과 같은 offset** — 다르면 NTP↔RTP 사상이 어긋나 AV sync 가 무너진다.
                        rtp_ts: t.slot.and_then(|slot| {
                            let rw = rewriters.get(&(t.ufrag.clone(), slot))?;
                            let raw = u32::from_be_bytes([pkt[16], pkt[17], pkt[18], pkt[19]]);
                            Some(rw.map_ts(raw))
                        }),
                        packet_count: packets,
                        octet_count: octets,
                    };
                    if let Some(translated) = rtcp::translate_sr(pkt, &patch)
                        && let Some(sealed) = out.seal_rtcp(&translated)
                    {
                        let _ = socket.send_to(&sealed, dst).await;
                        c.rtcp_out += 1;
                    }
                }
            }
            // ★**구독자 RR 은 서버가 소비한다** — 발행자에게 릴레이하면 발행자가
            //   남의 수신 품질로 비트레이트를 깎는다(정§11-2).
            rtcp::PT_RR => c.rtcp_rr_in += 1,
            // ★**하향 복구** — 구독자가 빠졌다고 한 것을 캐시에서 꺼내 되보낸다(정§11-1).
            rtcp::PT_RTPFB if rtcp::nack_seqs(pkt).is_some() => {
                let Some((media, seqs)) = rtcp::nack_seqs(pkt) else { continue };
                c.nack_in += 1;
                // ★**그 구독자의 NACK 만 센다** — 판정은 전송로마다 따로다(정§10-3).
                down.entry(ufrag.to_string()).or_default().nack_seen += 1;
                let Some(entry) = cache.get_mut(&(ufrag.to_string(), media)) else {
                    c.nack_no_cache += 1;
                    continue;
                };
                let Some((rtx_ssrc, rtx_pt)) = entry.rtx else {
                    c.nack_no_rtx += 1;
                    continue;
                };
                let (Some(dst), Some(out)) =
                    (table.get(ufrag).and_then(|e| e.addr()), srtp.get_mut(ufrag))
                else {
                    c.nack_no_route += 1;
                    continue;
                };
                for want in seqs {
                    // ★**예산**(정§11-1 관문 ③) — 그 구독자만 막고 남은 참가자를 보호한다.
                    if !entry.spend(now) {
                        c.nack_budget += 1;
                        break;
                    }
                    let Some(orig) = entry.get(want) else {
                        c.nack_miss += 1;
                        continue;
                    };
                    // ★**재전송과 프로브 패딩이 같은 ssrc 를 탄다** — 번호도 한 곳에서 뗀다.
                    let seq = down.entry(ufrag.to_string()).or_default().next_rtx_seq(rtx_ssrc);
                    // ★**못 지었으면 센다** — 여기가 조용하면 "넷 다 0인데 rtx 0" 이 된다.
                    let sealed =
                        rtcp::build_rtx(&orig, rtx_ssrc, rtx_pt, seq).and_then(|r| out.seal(&r));
                    let Some(sealed) = sealed else {
                        c.nack_build += 1;
                        continue;
                    };
                    let _ = socket.send_to(&sealed, dst).await;
                    c.rtx_out += 1;
                }
            }
            // ★★**구독자 피드백을 서버가 소비한다**(정§10-3 v2) — 우리가 매긴 번호로
            //   돌아오므로 장부와 대조해 표본이 되고, 그것이 대역 추정의 유일한 재료다.
            rtcp::PT_RTPFB if crate::twcc::parse(pkt).is_some() => {
                c.rtcp_fb_in += 1;
                let Some(fb) = crate::twcc::parse(pkt) else { continue };
                let d = down.entry(ufrag.to_string()).or_default();
                let (samples, lost) = d.ledger.samples(&fb);
                // ★**장부에 없던 것도 받은 것은 받은 것이다** — 표본에서만 빠진다.
                d.fb_recv += fb.packets.iter().filter(|p| p.is_some()).count() as u64;
                d.fb_lost += lost as u64;
                d.gcc.on_feedback(&samples, lost, now as f64);
                d.last_fb_ms = Some(now);
            }
            // ★★**구독자가 청한 키프레임을 발행자에게 옮긴다**(정§11-2) — 이것이 없으면
            //   늦게 들어온 사람이 ★**다음 키프레임까지 검은 화면**을 본다(발행자는 아무도
            //   안 청했다고 알고 주기대로만 낸다).
            rtcp::PT_PSFB if pkt[0] & 0x1F == 1 && pkt.len() >= 12 => {
                c.rtcp_fb_in += 1;
                let media = u32::from_be_bytes([pkt[8], pkt[9], pkt[10], pkt[11]]);
                // ★**되짚는 길은 전달표다** — 그 egress 값을 그 구독자에게 보내는 발행이 주인이다.
                //   ★슬롯이면 `slot` 이, 시뮬캐스트면 키의 vssrc 가 그 값이다.
                let owner = routes.iter().find(|((_, key), targets)| {
                    targets.iter().any(|t| t.ufrag == ufrag && t.slot.unwrap_or(*key) == media)
                });
                let Some(((pub_ufrag, key), _)) = owner else {
                    // ★**세고 버린다** — 조용히 버리면 *"왜 화면이 안 뜨나"* 를 못 짚는다.
                    c.pli_orphan += 1;
                    continue;
                };
                // 시뮬캐스트면 ★**지금 보내는 단**에 청한다 — 안 그러면 안 쓰는 단의 키프레임이 온다.
                let spatial = sim_out.get(&(ufrag.to_string(), *key)).map(|f| f.current);
                let target = match spatial {
                    Some(sp) => layer_of
                        .iter()
                        .find(|((f, _), (v, s))| f == pub_ufrag && v == key && *s == sp)
                        .map(|((_, s), _)| *s),
                    None => Some(*key),
                };
                let Some(ssrc) = target else {
                    c.pli_orphan += 1;
                    continue;
                };
                // ★**스로틀 버킷은 단마다 다르다**(정§10-3) — 여기도 같은 표를 쓴다.
                let throttle = match spatial {
                    Some(0) => crate::autolayer::v::PLI_THROTTLE_L_MS,
                    _ => crate::autolayer::v::PLI_THROTTLE_H_MS,
                };
                let key2 = (pub_ufrag.clone(), ssrc);
                if let Some(&at) = pli_at.get(&key2)
                    && now.saturating_sub(at) < throttle
                {
                    continue;
                }
                let (Some(dst), Some(ctx)) =
                    (table.get(pub_ufrag).and_then(|e| e.addr()), srtp.get_mut(pub_ufrag))
                else {
                    c.pli_orphan += 1;
                    continue;
                };
                let plain = rtcp::build_pli(SERVER_SSRC, ssrc);
                if let Some(sealed) = ctx.seal_rtcp(&plain) {
                    let _ = socket.send_to(&sealed, dst).await;
                    pli_at.insert(key2, now);
                    c.rtcp_out += 1;
                    c.pli_relay += 1;
                }
            }
            // ★**무시한다**(정§11-2) — 조용히가 아니라 세고 무시한다.
            rtcp::PT_SDES | rtcp::PT_BYE | rtcp::PT_APP => c.rtcp_ignored += 1,
            // PLI·REMB 는 다음 걸음이다 — 세고 버린다(조용한 drop 금지).
            rtcp::PT_RTPFB | rtcp::PT_PSFB => c.rtcp_fb_in += 1,
            _ => c.rtcp_ignored += 1,
        }
    }
}


/// 1초 판정 — ★**구독자마다 한 번**(정§10-3 tick 1,000ms).
///
/// ★**판정 자체는 순수 함수**(`autolayer::policy_tick`)이고 여기는 ★**신호를 모아 주고
/// 결과를 집행**할 뿐이다. 그래야 *"무엇을 보고 내렸나"* 를 시험이 시계 없이 되짚는다.
#[allow(clippy::too_many_arguments)]
async fn downlink_tick(
    now: u64,
    down: &mut HashMap<String, Downlink>,
    routes: &HashMap<(String, u32), Vec<Target>>,
    sim_of: &HashMap<String, u32>,
    layer_of: &HashMap<(String, u32), (u32, u8)>,
    sim_out: &mut HashMap<(String, u32), Forward>,
    pli_at: &mut HashMap<(String, u32), u64>,
    srtp: &mut HashMap<String, super::srtp::SrtpPair>,
    table: &Arc<IceTable>,
    socket: &Arc<UdpSocket>,
    c: &mut Counters,
) {
    use crate::autolayer::{self as al, Decision};

    // 그 구독자가 받는 ★시뮬캐스트 스트림만 모은다 — 아닌 사람은 비용이 이 훑기뿐이다.
    let mut per_sub: HashMap<String, Vec<SimSub>> = HashMap::new();
    for ((pub_u, ssrc), targets) in routes {
        if sim_of.get(pub_u) != Some(ssrc) {
            continue;
        }
        for t in targets {
            if t.paused {
                continue;
            }
            per_sub.entry(t.ufrag.clone()).or_default().push(SimSub {
                pub_ufrag: pub_u.clone(),
                vssrc: *ssrc,
                cap: t
                    .spatial_cap
                    .unwrap_or(al::Layer::High.spatial())
                    .min(al::Layer::High.spatial()),
                rtx: t.rtx,
            });
        }
    }

    for (sub, streams) in per_sub {
        // 아직 한 장도 안 보낸 구독자는 장부가 없다 — 잴 것이 없으니 판정도 없다.
        let Some(d) = down.get_mut(&sub) else { continue };

        // ★**신호가 낡으면 못 잰 것으로 둔다**(정§10-3 불신선 1s) — 낡은 값으로 내리면
        //   피드백이 잠깐 끊긴 것을 혼잡으로 오판한다.
        let fresh = matches!(d.last_fb_ms, Some(t) if now.saturating_sub(t) <= al::v::DISTRUST_MS);
        let remb_bps = fresh.then(|| d.gcc.estimate_bps());
        let total = d.fb_recv + d.fb_lost;
        // ★못 잰 것은 `0` 이 아니라 「없음」이다.
        let loss_pct = (total > 0).then(|| 100.0 * d.fb_lost as f32 / total as f32);
        d.fb_recv = 0;
        d.fb_lost = 0;
        if loss_pct.is_some_and(|p| p > al::v::DEMOTE_LOSS_PCT) {
            d.loss_bad_streak = d.loss_bad_streak.saturating_add(1);
        } else {
            d.loss_bad_streak = 0;
        }
        let nack_per_s = (d.nack_seen.saturating_sub(d.prev_nack)) as f64 * 1_000.0
            / al::v::TICK_MS as f64;
        d.prev_nack = d.nack_seen;

        let demand_bps: u64 = streams
            .iter()
            .map(|s| {
                match sim_out.get(&(sub.clone(), s.vssrc)).map(|f| f.current) {
                    Some(0) => al::v::L_BPS,
                    _ => al::v::H_BPS,
                }
            })
            .sum();
        let sig = al::Signals {
            now_ms: now,
            remb_bps,
            loss_pct,
            loss_bad_streak: d.loss_bad_streak,
            nack_per_s,
            // ★**우리에겐 egress 큐가 없다** — 단일 태스크가 그 자리에서 보내므로 *"밀려서
            //   버린"* 자리가 아예 없다. 이 사유는 그래서 상시 0 이고, 큐를 두는 날 채운다.
            drop_delta: 0,
            demand_bps,
            demand_high_bps: streams.len() as u64 * al::v::H_BPS,
            want_high: streams.iter().any(|s| s.cap >= al::Layer::High.spatial()),
            // ★프로브는 v2 신호가 살아 있는 전송로만 쏜다(v1 폴백은 "올림은 시도").
            probe_capable: fresh,
        };
        let decision = al::policy_tick(&mut d.policy, &sig);
        match decision {
            Decision::Promote => c.auto_promote += 1,
            Decision::Demote(_) => c.auto_demote += 1,
            Decision::Probe => {
                // ★**구독자가 아는 RTX 로 쏜다**(정§11-1) — 모르는 ssrc 면 복호도 못 한다.
                if let Some(rtx) = streams.iter().find_map(|s| s.rtx) {
                    let rate = al::probe_rate_bps(sig.demand_high_bps);
                    let chunk_bytes =
                        (rate / 8.0 * (al::v::PROBE_CHUNK_MS as f64 / 1_000.0)) as usize;
                    d.probe = Some(Probe {
                        until_ms: now + al::v::PROBE_MS,
                        per_chunk: chunk_bytes.div_ceil(al::v::PROBE_PAD_BYTES).max(1),
                        rtx,
                        sent: 0,
                        settle_at: None,
                    });
                }
            }
            Decision::Hold => {}
        }
        let auto = d.policy.cap.spatial();

        // ★**집행** — 전환 중인 것에는 손대지 않는다(진행 중 전환은 늘 완주시킨다).
        for st in &streams {
            let want = st.cap.min(auto);
            let Some(f) = sim_out.get_mut(&(sub.clone(), st.vssrc)) else { continue };
            if f.current == want || f.target.is_some() {
                continue;
            }
            f.target = Some((want, now));
            let ask = Ask { pub_ufrag: &st.pub_ufrag, vssrc: st.vssrc, spatial: want, now };
            ask_keyframe(ask, layer_of, pli_at, srtp, table, socket, c).await;
        }
    }
}

/// 프로브 청크 하나 — ★**도는 것이 없으면 아무 일도 안 한다.**
async fn probe_chunk(
    down: &mut HashMap<String, Downlink>,
    srtp: &mut HashMap<String, super::srtp::SrtpPair>,
    table: &Arc<IceTable>,
    socket: &Arc<UdpSocket>,
    c: &mut Counters,
) {
    let subs: Vec<String> =
        down.iter().filter(|(_, d)| d.probe.is_some()).map(|(u, _)| u.clone()).collect();
    if subs.is_empty() {
        return;
    }
    let now = now_ms();
    for sub in subs {
        let Some(d) = down.get_mut(&sub) else { continue };
        let Some(mut p) = d.probe else { continue };
        // ★**혼잡이면 그 자리에서 멈춘다** — 약한 길을 남은 램프 내내 두들기지 않는다.
        //   ★멈춘 것 자체가 *"수요를 못 받는다"* 는 측정이다.
        if d.gcc.state() == crate::gcc::Usage::Overusing {
            d.probe = None;
            continue;
        }
        if now >= p.until_ms {
            match p.settle_at {
                // ★꼬리 피드백을 기다렸다 잰다 — 곧장 재면 프로브 구간이 덜 찼다.
                None => {
                    p.settle_at = Some(now + PROBE_SETTLE_MS);
                    d.probe = Some(p);
                }
                Some(at) if now >= at => {
                    d.gcc.apply_probe();
                    d.probe = None;
                }
                Some(_) => {}
            }
            continue;
        }
        let (rtx_ssrc, rtx_pt) = p.rtx;
        let per = p.per_chunk;
        p.sent += per as u64;
        d.probe = Some(p);
        for _ in 0..per {
            let Some(d) = down.get_mut(&sub) else { break };
            let seq = d.next_rtx_seq(rtx_ssrc);
            let mut pkt = crate::twcc::probe_padding(
                rtx_ssrc,
                seq,
                rtx_pt,
                crate::autolayer::v::PROBE_PAD_BYTES,
            );
            if send_to_sub(&mut pkt, &sub, down, srtp, table, socket, now).await {
                c.probe_out += 1;
            }
        }
    }
}

/// 판정 한 판에서 보는 "그 구독자가 받는 시뮬캐스트 스트림" 하나.
struct SimSub {
    pub_ufrag: String,
    vssrc: u32,
    /// 그 구독자의 수동 상한(연§6-3).
    cap: u8,
    rtx: Option<(u32, u8)>,
}

/// 어느 발행 스트림의 어느 단에 키프레임을 청하는가.
#[derive(Debug, Clone, Copy)]
struct Ask<'a> {
    pub_ufrag: &'a str,
    vssrc: u32,
    spatial: u8,
    now: u64,
}

/// 한 구독자에게 지금 무슨 단을 보내고 있는가. ★**바뀌는 순간이 키프레임**이다(정§10-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Forward {
    /// 지금 내보내는 공간 단.
    current: u8,
    /// 갈아타려는 단과 그것을 정한 시각 — ★`None` 이면 전환 중이 아니다.
    target: Option<(u8, u64)>,
}

/// 도는 프로브 하나. ★**나눠 보낸다** — 한 번에 쏟으면 그것이 곧 혼잡이다.
#[derive(Debug, Clone, Copy)]
struct Probe {
    /// 램프가 끝나는 시각.
    until_ms: u64,
    /// 한 청크에 보낼 장 수.
    per_chunk: usize,
    /// 패딩이 탈 `(ssrc, pt)` — ★**구독자가 아는 RTX 값**이다(정§11-1).
    rtx: (u32, u8),
    sent: u64,
    /// 꼬리 피드백을 기다리는 시각 — ★`None` 이면 아직 램프 중이다.
    settle_at: Option<u64>,
}

/// ★**꼬리 여유**(정§10-3 판정 창 안) — 마지막 청크의 피드백이 도착할 짬이다.
/// 이 여유 없이 곧장 재면 ★**프로브 구간이 덜 찬 채로** 실측이 매겨진다.
const PROBE_SETTLE_MS: u64 = 300;

/// 한 구독 전송로의 대역 축 ★전부 — 장부·추정·판정이 한 자리에 있다.
#[derive(Debug, Default)]
struct Downlink {
    ledger: crate::twcc::SendLedger,
    gcc: crate::gcc::Gcc,
    policy: crate::autolayer::Policy,
    /// 마지막 피드백 시각 — ★`None` 이면 아직 한 번도 못 받았다(`0` 을 부재 표식으로 안 쓴다).
    last_fb_ms: Option<u64>,
    /// 이번 tick 창에서 구독자가 말한 받음/못 받음.
    fb_recv: u64,
    fb_lost: u64,
    /// 손실이 이어서 나쁜 횟수.
    loss_bad_streak: u8,
    /// 그 구독자가 보낸 NACK 누적 — tick 이 Δ 로 읽는다.
    nack_seen: u64,
    prev_nack: u64,
    probe: Option<Probe>,
    /// ★**RTX 는 제 seq 공간을 쓴다**(RFC 4588) — 재전송과 프로브 패딩이 ★**같은 ssrc** 를
    /// 타므로 번호도 한 곳에서 떼야 한다. 따로 세면 그 스트림의 seq 가 두 갈래로 갈린다.
    rtx_seq: HashMap<u32, u16>,
}

impl Downlink {
    fn next_rtx_seq(&mut self, ssrc: u32) -> u16 {
        let n = self.rtx_seq.entry(ssrc).or_default();
        *n = n.wrapping_add(1);
        *n
    }
}

/// 한 구독자에게 한 장 보낸다 — ★**스탬핑·장부·봉인이 여기 하나로 모인다.**
///
/// ★★**egress twcc seq 는 서버가 교체 스탬핑한다**(정§11-2) — 발행자 값을 그대로 흘리면
/// 발행자 시계가 우리 측정에 섞이고, 아예 없으면 ★**구독자가 피드백을 지을 재료가 없다**
/// (실측 20260912: 봇의 합성 축이 통째로 비었다).
///
/// ★**크기는 스탬핑 뒤에 적는다** — 확장을 더한 길이가 실제로 나간 양이다.
async fn send_to_sub(
    pkt: &mut Vec<u8>,
    sub: &str,
    down: &mut HashMap<String, Downlink>,
    srtp: &mut HashMap<String, super::srtp::SrtpPair>,
    table: &Arc<IceTable>,
    socket: &Arc<UdpSocket>,
    now: u64,
) -> bool {
    let Some(dst) = table.get(sub).and_then(|e| e.addr()) else { return false };
    let d = down.entry(sub.to_string()).or_default();
    let seq = d.ledger.next_seq();
    if let Some(v) = crate::rtpext::stamp(pkt, TWCC_EXT_ID, &seq.to_be_bytes()) {
        pkt.clear();
        pkt.extend_from_slice(&v);
    }
    let size = pkt.len() as u16;
    d.ledger.record(seq, now, size);
    d.gcc.on_sent(now as f64, size);
    let Some(out) = srtp.get_mut(sub) else { return false };
    let Some(sealed) = out.seal(pkt) else { return false };
    socket.send_to(&sealed, dst).await.is_ok()
}

/// 그 단의 실제 ssrc 에 키프레임을 청한다.
///
/// ★★**target 단의 rid 로 청해야 한다**(정§10-2) — `h` 에 PLI 를 보내면 `l` 키프레임이
/// 오지 않아 전환이 pending 만료로 폐기된다. 어느 ssrc 가 어느 단인지는 `rid` 로 배운 것이다.
async fn ask_keyframe(
    ask: Ask<'_>,
    layer_of: &HashMap<(String, u32), (u32, u8)>,
    pli_at: &mut HashMap<(String, u32), u64>,
    srtp: &mut HashMap<String, super::srtp::SrtpPair>,
    table: &Arc<IceTable>,
    socket: &Arc<UdpSocket>,
    c: &mut Counters,
) {
    // ★**배운 지도를 되짚는다** — 등록이 rid 를 안 싣기 때문에 이 사상은 RTP 로만 온다.
    let Ask { pub_ufrag, vssrc, spatial, now } = ask;
    let Some(ssrc) = layer_of
        .iter()
        .find(|((f, _), (v, sp))| f == pub_ufrag && *v == vssrc && *sp == spatial)
        .map(|((_, s), _)| *s)
    else {
        return;
    };
    // ★**단마다 버킷이 다르다**(정§10-3) — `l` 이 촘촘한 까닭은 강등 전환이 그 단의
    //   키프레임을 기다리기 때문이고, 그 단은 비트가 작아 자주 청해도 값이 싸다.
    let throttle = if spatial == 0 {
        crate::autolayer::v::PLI_THROTTLE_L_MS
    } else {
        crate::autolayer::v::PLI_THROTTLE_H_MS
    };
    let key = (pub_ufrag.to_string(), ssrc);
    if let Some(&at) = pli_at.get(&key)
        && now.saturating_sub(at) < throttle
    {
        return;
    }
    let (Some(dst), Some(ctx)) =
        (table.get(pub_ufrag).and_then(|e| e.addr()), srtp.get_mut(pub_ufrag))
    else {
        return;
    };
    let plain = crate::rtcp::build_pli(SERVER_SSRC, ssrc);
    if let Some(sealed) = ctx.seal_rtcp(&plain) {
        let _ = socket.send_to(&sealed, dst).await;
        pli_at.insert(key, now);
        c.rtcp_out += 1;
    }
}

/// 한 구독자·한 스트림의 송신 캐시. ★**되보낼 것을 들고 있는 자리**다(정§11-1 관문 ②).
///
/// ★**링버퍼다** — 무한히 들면 오래 사는 서버의 메모리가 그만큼 자란다. 30fps 기준
/// 1,024장이면 ~34초이고, 그보다 늦은 NACK 은 재전송으로 못 메우는 영역이다.
struct SendCache {
    /// 재전송용 `(ssrc, pt)` — ★**보낼 때 같이 적어 둔다.** NACK 은 구독자 자격으로 오는데
    /// 전달표는 발행자 자격으로 걸려 있어, 되짚는 길을 두면 그 길이 또 갈린다.
    rtx: Option<(u32, u8)>,
    ring: std::collections::VecDeque<(u16, Vec<u8>)>,
    /// 예산 창의 시작과 그 창에서 쓴 수.
    window_at: u64,
    spent: u32,
}

/// 캐시 깊이 — 30fps 기준 ~34초.
const CACHE_MAX: usize = 1024;
/// ★**구독자당 예산**(정§11-1) — 정상 10~30 이 통과하고 폭풍은 막힌다.
const RTX_BUDGET: u32 = 200;
const RTX_WINDOW_MS: u64 = 3_000;

impl Default for SendCache {
    fn default() -> Self {
        Self {
            rtx: None,
            ring: std::collections::VecDeque::with_capacity(CACHE_MAX),
            window_at: 0,
            spent: 0,
        }
    }
}

impl SendCache {
    fn put(&mut self, pkt: &[u8]) {
        if pkt.len() < 12 {
            return;
        }
        if self.ring.len() == CACHE_MAX {
            self.ring.pop_front();
        }
        let seq = u16::from_be_bytes([pkt[2], pkt[3]]);
        self.ring.push_back((seq, pkt.to_vec()));
    }

    fn get(&self, seq: u16) -> Option<Vec<u8>> {
        self.ring.iter().rev().find(|(s, _)| *s == seq).map(|(_, p)| p.clone())
    }

    /// ★**예산 하나를 쓴다** — 창이 지나면 새로 찬다. `false` 면 그 구독자는 이 창에서 끝이다.
    fn spend(&mut self, now: u64) -> bool {
        if now.saturating_sub(self.window_at) > RTX_WINDOW_MS {
            self.window_at = now;
            self.spent = 0;
        }
        if self.spent >= RTX_BUDGET {
            return false;
        }
        self.spent += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 응답은_한_데이터그램에_들어간다() {
        let w = super::super::stun::binding_response(
            &[1u8; 12],
            "203.0.113.1:5000".parse().expect("주소"),
            "pwd",
        );
        assert!(w.len() < MTU, "{}", w.len());
    }

    #[test]
    fn 계수는_바뀐_것만_말한다() {
        let a = Counters::default();
        let mut b = a;
        b.forged += 1;
        assert_ne!(a, b);
        assert!(b.line().contains("위조 1"), "{}", b.line());
    }
}
