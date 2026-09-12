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
    /// 그 발행 `ssrc` 가 갈 곳 전부. ★**빈 목록이면 아무 데도 안 간다**(지우는 것과 같다).
    SetRoute { ssrc: u32, targets: Vec<Target> },
    /// 그 자격의 DC 로 한 장. ★**채널이 아직이면 버린다** — 막지 않는다(정§13).
    DcSend { ufrag: String, wire: Vec<u8> },
}

/// DC 로 들어온 것 — ★**판정은 제어 평면이 한다**(여기는 나르기만).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DcIn {
    pub ufrag: String,
    pub svc: u8,
    pub payload: Vec<u8>,
}

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
    pub unknown: u64,
}

impl Counters {
    fn line(&self) -> String {
        format!(
            "stun {}/{}(위조 {}) · dtls {} · srtp in {}(버림 {}) out {} · rtcp in {} out {} · 모름 {}",
            self.stun_ok,
            self.stun_ok + self.stun_dropped,
            self.forged,
            self.dtls_in,
            self.srtp_in,
            self.srtp_bad,
            self.srtp_out,
            self.rtcp_in,
            self.rtcp_out,
            self.unknown
        )
    }
}

/// 계수 한 줄을 남기는 주기.
const REPORT_MS: u64 = 30_000;

/// ★발행자에게 RR 을 내는 주기(정§11-2) — ★**소비자는 타이머 하나다.**
const RR_MS: u64 = 1_000;

/// 서버가 RTCP 에 쓰는 제 SSRC — ★**미디어 SSRC 와 겹치지 않는 고정값**이다.
const SERVER_SSRC: u32 = 0x0000_0001;

/// 포트를 열고 루프를 돈다. ★**되돌아오지 않는다** — 태스크로 띄운다.
pub async fn serve(
    socket: Arc<UdpSocket>,
    table: Arc<IceTable>,
    cert: dtls::Certificate,
    cmd_tx: mpsc::Sender<Cmd>,
    mut cmds: mpsc::Receiver<Cmd>,
    dc_in: mpsc::Sender<DcIn>,
) {
    // ★**그릇을 하나 잡아 재사용한다** — 데이터그램마다 새로 잡지 않는다(H3).
    let mut buf = vec![0u8; MTU];
    let mut by_ufrag: HashMap<String, Arc<Pipe>> = HashMap::new();
    let mut by_addr: HashMap<SocketAddr, Arc<Pipe>> = HashMap::new();
    // ★이 둘도 이 태스크 혼자 쓴다 — 자물쇠가 없다.
    let mut srtp: HashMap<String, super::srtp::SrtpPair> = HashMap::new();
    let mut routes: HashMap<u32, Vec<Target>> = HashMap::new();
    // ★**전송로마다 스칼라 둘** — 구간 지도를 두면 화자 교대에서 egress seq 가 역행한다.
    let mut rewriters: HashMap<(String, u32), crate::rewriter::Rewriter> = HashMap::new();
    // ★**발행 스트림마다 수신 통계** — 핫패스가 갱신하고 1초 타이머가 소비한다(정§11-2).
    //   값은 `(그 자격, 통계)` 라 회수 때 같이 간다.
    let mut stats: HashMap<u32, (String, crate::rtcp::RecvStats)> = HashMap::new();
    // ★구독자에게 내보낸 수 — SR 번역이 이 값으로 카운터를 갈아 끼운다.
    let mut egress: HashMap<(String, u32), (u32, u32)> = HashMap::new();
    // ★**내보낼 것을 담는 그릇 하나** — 패킷마다 새로 잡지 않는다(H3).
    let mut scratch: Vec<u8> = Vec::with_capacity(MTU);
    let mut c = Counters::default();
    let mut last = Counters::default();
    let mut report = tokio::time::interval(std::time::Duration::from_millis(REPORT_MS));
    report.tick().await;
    let mut rr = tokio::time::interval(std::time::Duration::from_millis(RR_MS));
    rr.tick().await;

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
                            stats.retain(|_, (f, _)| f != u);
                            egress.retain(|(f, _), _| f != u);
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
                    Cmd::SetRoute { ssrc, targets } => {
                        if targets.is_empty() {
                            routes.remove(&ssrc);
                        } else {
                            routes.insert(ssrc, targets);
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
                let Some(p) = by_addr.get(&from) else { continue };
                let ufrag = p.ufrag.clone();
                // ★RTP 와 RTCP 는 같은 대역으로 온다(RFC 5761) — 둘째 바이트가 가른다.
                if crate::rtcp::is_rtcp(&buf[..n]) {
                    c.rtcp_in += 1;
                    let Some(ctx) = srtp.get_mut(&ufrag) else { continue };
                    let Some(plain) = ctx.open_rtcp(&buf[..n]) else {
                        c.srtp_bad += 1;
                        continue;
                    };
                    on_rtcp(&plain, &ufrag, &mut stats, &mut egress, &routes, &rewriters, &table, &mut srtp, &socket, &mut c).await;
                    continue;
                }
                c.srtp_in += 1;
                let Some(ctx) = srtp.get_mut(&ufrag) else { continue };
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
                {
                    // ★RR 의 재료 — 핫패스에서 세고 타이머가 소비한다.
                    let seq = u16::from_be_bytes([plain[2], plain[3]]);
                    let ts = u32::from_be_bytes([plain[4], plain[5], plain[6], plain[7]]);
                    stats
                        .entry(ssrc)
                        .or_insert_with(|| (ufrag.clone(), crate::rtcp::RecvStats::new(ssrc, 48_000)))
                        .1
                        .on_rtp(seq, ts, now);
                }
                let Some(targets) = routes.get(&ssrc) else { continue };
                for t in targets {
                    let Some(dst) = table.get(&t.ufrag).and_then(|e| e.addr()) else { continue };
                    let Some(out) = srtp.get_mut(&t.ufrag) else { continue };
                    // ★**제자리 재기록 · 길이 불변** — 본문은 건드리지 않는다(H1).
                    scratch.clear();
                    scratch.extend_from_slice(&plain);
                    match t.slot {
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
                    if let Some(sealed) = out.seal(&scratch) {
                        let _ = socket.send_to(&sealed, dst).await;
                        c.srtp_out += 1;
                        // ★SR 번역이 쓸 egress 카운터 — 발행자 수가 아니라 **내보낸 수**다.
                        let e = egress.entry((t.ufrag.clone(), ssrc)).or_insert((0, 0));
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
    stats: &mut HashMap<u32, (String, crate::rtcp::RecvStats)>,
    now: u64,
) -> Vec<(String, Vec<crate::rtcp::ReportBlock>)> {
    let mut by: HashMap<String, Vec<crate::rtcp::ReportBlock>> = HashMap::new();
    for (ufrag, s) in stats.values_mut() {
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
    stats: &mut HashMap<u32, (String, crate::rtcp::RecvStats)>,
    egress: &mut HashMap<(String, u32), (u32, u32)>,
    routes: &HashMap<u32, Vec<Target>>,
    rewriters: &HashMap<(String, u32), crate::rewriter::Rewriter>,
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
                if let Some((_, s)) = stats.get_mut(&ssrc) {
                    s.on_sr(hi, lo, now);
                }
                // ★**자체 생성 금지 — 번역 릴레이다**(정§11-2). 구독자마다 값이 다르다.
                let Some(targets) = routes.get(&ssrc) else { continue };
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
            // ★**무시한다**(정§11-2) — 조용히가 아니라 세고 무시한다.
            rtcp::PT_SDES | rtcp::PT_BYE | rtcp::PT_APP => c.rtcp_ignored += 1,
            // NACK·PLI·REMB·TWCC 는 다음 걸음이다 — 세고 버린다(조용한 drop 금지).
            rtcp::PT_RTPFB | rtcp::PT_PSFB => c.rtcp_fb_in += 1,
            _ => c.rtcp_ignored += 1,
        }
    }
    let _ = ufrag;
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
