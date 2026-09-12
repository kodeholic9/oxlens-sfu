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

/// 바깥에서 루프에 거는 것. ★**루프의 자료를 직접 만지지 않는다.**
#[derive(Debug)]
pub enum Cmd {
    /// 그 세션을 거둔다 — ★**통로를 닫으면 DTLS 태스크가 끝난다**(정§12).
    DropSession(String),
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
    tx: mpsc::Sender<Bytes>,
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
    pub unknown: u64,
}

impl Counters {
    fn line(&self) -> String {
        format!(
            "stun {}/{}(위조 {}) · dtls {} · srtp {} · 모름 {}",
            self.stun_ok, self.stun_ok + self.stun_dropped, self.forged, self.dtls_in, self.srtp_in, self.unknown
        )
    }
}

/// 계수 한 줄을 남기는 주기.
const REPORT_MS: u64 = 30_000;

/// 포트를 열고 루프를 돈다. ★**되돌아오지 않는다** — 태스크로 띄운다.
pub async fn serve(
    socket: Arc<UdpSocket>,
    table: Arc<IceTable>,
    cert: dtls::Certificate,
    mut cmds: mpsc::Receiver<Cmd>,
) {
    // ★**그릇을 하나 잡아 재사용한다** — 데이터그램마다 새로 잡지 않는다(H3).
    let mut buf = vec![0u8; MTU];
    let mut by_ufrag: HashMap<String, Arc<Pipe>> = HashMap::new();
    let mut by_addr: HashMap<SocketAddr, Arc<Pipe>> = HashMap::new();
    let mut c = Counters::default();
    let mut last = Counters::default();
    let mut report = tokio::time::interval(std::time::Duration::from_millis(REPORT_MS));
    report.tick().await;

    loop {
        let (n, from) = tokio::select! {
            r = socket.recv_from(&mut buf) => match r {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[udp] recv: {e}");
                    continue;
                }
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
                        by_ufrag.retain(|_, p| p.session_id != sid);
                        by_addr.retain(|_, p| p.session_id != sid);
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
                                    let task = spawn_dtls(conn, cert.clone(), ufrag.clone());
                                    let p = Arc::new(Pipe { session_id: session_id.clone(), tx, task });
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
                c.srtp_in += 1;
                // 미디어 루프는 다음 걸음이다 — ★**조용히 성공하지 않는다**(세고 버린다).
            }
            Packet::Unknown => c.unknown += 1,
        }
    }
}

/// 그 통로 위에 DTLS 를 세우고, 선 뒤에는 ★**같은 태스크에서 SCTP 를 돈다.**
///
/// ★**둘을 한 태스크에 두는 이유** — DTLS 연결은 SCTP 의 전송로라 수명이 같다.
/// 태스크를 갈라 두면 회수 신호를 두 번 보내야 하고, 한쪽만 지나가는 창이 생긴다.
fn spawn_dtls(conn: DemuxConn, cert: dtls::Certificate, ufrag: String) -> tokio::task::JoinHandle<()> {
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
            Ok(_keys) => eprintln!("[dtls] {ufrag} 섰다 — SRTP 열쇠 넷"),
            Err(e) => {
                eprintln!("[dtls] {ufrag} 열쇠: {e}");
                return;
            }
        }
        // ★DC 는 ★**보내기용 연결에만** 붙는다(연§3-3) — 클라가 안 열면 아무 일도 없다.
        let (_out_tx, out_rx) = tokio::sync::mpsc::channel(64);
        let (ev_tx, mut ev_rx) = tokio::sync::mpsc::channel(64);
        let who = ufrag.clone();
        tokio::spawn(async move {
            while let Some(e) = ev_rx.recv().await {
                match e {
                    super::sctp::DcEvent::Open => eprintln!("[dc] {who} \"unreliable\" 열렸다"),
                    // 발언권은 다음 덩어리다 — ★**조용히 성공하지 않는다**(받은 것을 말한다).
                    super::sctp::DcEvent::Frame { svc, payload } => {
                        eprintln!("[dc] {who} svc=0x{svc:02X} {}바이트", payload.len())
                    }
                    super::sctp::DcEvent::Closed => eprintln!("[dc] {who} 닫혔다"),
                }
            }
        });
        super::sctp::run(&c, out_rx, ev_tx).await;
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// STUN 응답 한 장의 크기 감각 — 상한을 넘지 않는지 본다.
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
