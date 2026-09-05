// author: kodeholic (powered by Claude)
//! 전송 세션 등록부 — 정§12. ★세션 동일성은 주소가 아니라 ufrag 다.
//! 그래서 정본 색인은 `by_ufrag` 이고 `by_addr` 은 STUN latch 가 만드는 파생 색인일 뿐이다
//! (NAT 이 산 peer 의 포트를 남에게 재배정해도 옛 세션이 죽지 않는다 — 주소만 잃는다).

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use dashmap::DashMap;
use oxsig::schema::PcMode;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use super::IceCredentials;
use super::srtp::SrtpContext;
use crate::media::twcc::{Arrivals, Departures};
use crate::peer::Peer;

/// 정§13 — DC 준비 전 버퍼. 넘치면 오래된 것부터 버리고 센다.
pub const DC_PENDING_CAP: usize = 64;

/// 연결의 몫. `2pc` 는 둘, `1pc` 는 `Publish` 하나가 겸한다(정§12 1pc/2pc 행).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnRole {
    Publish,
    Subscribe,
}

impl ConnRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Publish => "publish",
            Self::Subscribe => "subscribe",
        }
    }
}

/// latch 된 주소를 공유하는 셀 — `DemuxConn` 과 등록부가 같은 것을 본다(정§12 주소 latch).
#[derive(Clone, Default)]
pub struct AddrCell(Arc<RwLock<Option<SocketAddr>>>);

impl AddrCell {
    pub fn get(&self) -> Option<SocketAddr> {
        *self.0.read().unwrap_or_else(|e| e.into_inner())
    }
    fn set(&self, addr: SocketAddr) -> Option<SocketAddr> {
        self.0.write().unwrap_or_else(|e| e.into_inner()).replace(addr)
    }
}

#[derive(Default)]
struct DcState {
    tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    ready: AtomicBool,
    pending: Mutex<VecDeque<Vec<u8>>>,
    dropped: AtomicU64,
}

pub struct TransportSession {
    pub user_id: String,
    pub role: ConnRole,
    pub creds: IceCredentials,
    /// ★핫패스 — 주인을 직접 든다(매 패킷 등록부 조회를 없앤다). Peer 가 이 세션을 도로 들므로 `Weak` 다.
    owner: std::sync::Weak<Peer>,
    pub addr: AddrCell,
    /// 정§11-2 — Ingress TWCC 의 도착 장부. ★SSRC 가 아니라 전송로 단위다(transport-wide).
    pub arrivals: Arrivals,
    /// 정§10-3 v2 — Egress 의 출발 장부. 번호를 발급하고 무엇을 언제 얼마나 보냈는지 담는다.
    pub departures: Departures,
    /// 정§17-2 ④ — 이것을 켜야 DTLS·SCTP 태스크가 끝난다. 종료 신호까지가 회수다.
    pub cancel: CancellationToken,
    handshake: AtomicBool,
    dtls_tx: Mutex<Option<super::conn::PacketTx>>,
    srtp_in: Mutex<Option<SrtpContext>>,
    srtp_out: Mutex<Option<SrtpContext>>,
    dc: DcState,
}

impl TransportSession {
    fn new(owner: &Arc<Peer>, role: ConnRole, creds: IceCredentials) -> Self {
        Self {
            user_id: owner.user_id.clone(),
            role,
            creds,
            owner: Arc::downgrade(owner),
            addr: AddrCell::default(),
            arrivals: Arrivals::default(),
            departures: Departures::default(),
            cancel: CancellationToken::new(),
            handshake: AtomicBool::new(false),
            dtls_tx: Mutex::new(None),
            srtp_in: Mutex::new(None),
            srtp_out: Mutex::new(None),
            dc: DcState::default(),
        }
    }

    pub fn peer(&self) -> Option<Arc<Peer>> {
        self.owner.upgrade()
    }

    /// 정§2-3 계약 5 — 핸드셰이크 착수는 원샷 latch. USE-CANDIDATE 재전송이 겹쳐도 한 번만 시작한다.
    pub fn claim_handshake(&self) -> bool {
        !self.handshake.swap(true, Ordering::AcqRel)
    }

    pub fn set_dtls_tx(&self, tx: super::conn::PacketTx) {
        *self.dtls_tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    }

    pub fn dtls_tx(&self) -> Option<super::conn::PacketTx> {
        self.dtls_tx.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn install_srtp(&self, keys: &super::dtls::SrtpKeys) -> Result<(), String> {
        *self.srtp_in.lock().unwrap_or_else(|e| e.into_inner()) = Some(SrtpContext::install(&keys.client_key, &keys.client_salt)?);
        *self.srtp_out.lock().unwrap_or_else(|e| e.into_inner()) = Some(SrtpContext::install(&keys.server_key, &keys.server_salt)?);
        Ok(())
    }

    pub fn srtp_ready(&self) -> bool {
        self.srtp_in.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    pub fn decrypt_rtp(&self, packet: &[u8]) -> Result<bytes::Bytes, String> {
        self.srtp_in.lock().unwrap_or_else(|e| e.into_inner()).as_mut().ok_or_else(|| "no key".to_owned())?.decrypt_rtp(packet)
    }

    pub fn decrypt_rtcp(&self, packet: &[u8]) -> Result<bytes::Bytes, String> {
        self.srtp_in.lock().unwrap_or_else(|e| e.into_inner()).as_mut().ok_or_else(|| "no key".to_owned())?.decrypt_rtcp(packet)
    }

    pub fn encrypt_rtp(&self, packet: &[u8]) -> Result<bytes::Bytes, String> {
        self.srtp_out.lock().unwrap_or_else(|e| e.into_inner()).as_mut().ok_or_else(|| "no key".to_owned())?.encrypt_rtp(packet)
    }

    pub fn encrypt_rtcp(&self, packet: &[u8]) -> Result<bytes::Bytes, String> {
        self.srtp_out.lock().unwrap_or_else(|e| e.into_inner()).as_mut().ok_or_else(|| "no key".to_owned())?.encrypt_rtcp(packet)
    }

    /// DCEP `unreliable` 개통 — 밀려 있던 프레임을 먼저 흘려보내고 준비를 알린다(정§13).
    pub fn dc_open(&self, tx: mpsc::Sender<Vec<u8>>) {
        let mut pending = self.dc.pending.lock().unwrap_or_else(|e| e.into_inner());
        while let Some(f) = pending.pop_front() {
            if tx.try_send(f).is_err() {
                break;
            }
        }
        *self.dc.tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        self.dc.ready.store(true, Ordering::Release);
    }

    pub fn dc_close(&self) {
        self.dc.ready.store(false, Ordering::Release);
        *self.dc.tx.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn dc_ready(&self) -> bool {
        self.dc.ready.load(Ordering::Acquire)
    }

    pub fn dc_dropped(&self) -> u64 {
        self.dc.dropped.load(Ordering::Relaxed)
    }

    /// 정§13 — `try_send` 실패는 계수이지 연결 종료 사유가 아니다. 미개통이면 64 버퍼에 쌓는다.
    pub fn dc_send(&self, frame: Vec<u8>) {
        if self.dc_ready()
            && let Some(tx) = self.dc.tx.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        {
            if tx.try_send(frame).is_err() {
                self.dc.dropped.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
        let mut pending = self.dc.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending.len() >= DC_PENDING_CAP {
            pending.pop_front();
            self.dc.dropped.fetch_add(1, Ordering::Relaxed);
        }
        pending.push_back(frame);
    }
}

/// ufrag → 세션(정본) · 주소 → ufrag(latch 파생).
#[derive(Default)]
pub struct TransportRegistry {
    by_ufrag: DashMap<String, Arc<TransportSession>>,
    by_addr: DashMap<SocketAddr, String>,
}

impl TransportRegistry {
    /// Peer 생성 때 한 번. `1pc` 도 자격 넷을 다 싣지만(정§4-2) 쓰는 연결은 `Publish` 하나다.
    pub fn register(&self, peer: &Arc<Peer>) {
        self.insert(peer, ConnRole::Publish, &peer.publish_ice);
        if peer.pc_mode == PcMode::TwoPc {
            self.insert(peer, ConnRole::Subscribe, &peer.subscribe_ice);
        }
    }

    fn insert(&self, peer: &Arc<Peer>, role: ConnRole, creds: &IceCredentials) {
        self.by_ufrag.insert(creds.ufrag.clone(), Arc::new(TransportSession::new(peer, role, creds.clone())));
    }

    pub fn by_ufrag(&self, ufrag: &str) -> Option<Arc<TransportSession>> {
        self.by_ufrag.get(ufrag).map(|s| s.clone())
    }

    pub fn by_addr(&self, addr: &SocketAddr) -> Option<Arc<TransportSession>> {
        let ufrag = self.by_addr.get(addr)?.clone();
        self.by_ufrag(&ufrag)
    }

    /// 정§12 — 검증을 통과한 Binding 만 여기 온다. 옛 주소 색인을 걷고 새 주소를 건다. 반환: 주소가 바뀌었나.
    pub fn latch(&self, session: &TransportSession, addr: SocketAddr) -> bool {
        let previous = session.addr.set(addr);
        if previous == Some(addr) {
            return false;
        }
        if let Some(old) = previous {
            self.by_addr.remove_if(&old, |_, u| u == &session.creds.ufrag);
        }
        self.by_addr.insert(addr, session.creds.ufrag.clone());
        true
    }

    /// 정§17-2 ④ — 취소가 먼저, 색인 해제가 본체보다 먼저. 반환: 회수한 세션 수.
    pub fn unregister(&self, user_id: &str) -> usize {
        let mine: Vec<String> = self.by_ufrag.iter().filter(|e| e.user_id == user_id).map(|e| e.key().clone()).collect();
        for ufrag in &mine {
            let Some((_, session)) = self.by_ufrag.remove(ufrag) else { continue };
            session.cancel.cancel();
            session.dc_close();
            if let Some(addr) = session.addr.get() {
                self.by_addr.remove_if(&addr, |_, u| u == ufrag);
            }
            debug!(user = user_id, role = session.role.as_str(), ufrag = %ufrag, "transport released");
        }
        mine.len()
    }

    /// 그 사용자의 그 연결 — 발언권 DC·구독 전달의 주소다.
    pub fn session_for(&self, user_id: &str, role: ConnRole) -> Option<Arc<TransportSession>> {
        self.by_ufrag.iter().find(|e| e.user_id == user_id && e.role == role).map(|e| e.clone())
    }

    pub fn len(&self) -> usize {
        self.by_ufrag.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_ufrag.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(u: &str) -> IceCredentials {
        IceCredentials { ufrag: u.to_owned(), pwd: format!("pwd-{u}") }
    }

    /// 자격을 지정해 만든 Peer — 등록부 시험이 ufrag 를 이름으로 집을 수 있게.
    fn peer(user: &str, pc_mode: PcMode, publish: &str, subscribe: &str) -> Arc<Peer> {
        Arc::new(Peer::with_credentials(user, 0, pc_mode, 0, creds(publish), creds(subscribe)))
    }

    #[test]
    fn register_by_mode_and_release() {
        let r = TransportRegistry::default();
        // Arc 는 `PeerMap` 이 든다 — 시험도 같은 수명을 만들어 준다(세션은 Weak 로 잡는다).
        let (p1, p2) = (peer("u1", PcMode::TwoPc, "pub1", "sub1"), peer("u2", PcMode::OnePc, "pub2", "sub2"));
        r.register(&p1);
        r.register(&p2);
        assert_eq!(r.len(), 3, "2pc 는 둘, 1pc 는 publish 하나");
        assert!(r.by_ufrag("sub2").is_none());
        assert_eq!(r.session_for("u1", ConnRole::Subscribe).unwrap().creds.pwd, "pwd-sub1");
        let s = r.by_ufrag("pub1").unwrap();
        assert_eq!(s.peer().map(|p| p.user_id.clone()), Some("u1".to_owned()), "세션은 주인을 직접 든다");
        assert_eq!(r.unregister("u1"), 2);
        assert!(s.cancel.is_cancelled(), "회수는 종료 신호까지");
        assert_eq!((r.len(), r.unregister("nobody")), (1, 0));
    }

    #[test]
    fn latch_moves_address_index_and_port_reuse_keeps_identity() {
        let r = TransportRegistry::default();
        let (a, b) = (peer("u1", PcMode::OnePc, "pubA", "subA"), peer("u2", PcMode::OnePc, "pubB", "subB"));
        r.register(&a);
        r.register(&b);
        let a: SocketAddr = "203.0.113.7:5000".parse().unwrap();
        let b: SocketAddr = "203.0.113.7:5001".parse().unwrap();
        let sa = r.by_ufrag("pubA").unwrap();
        assert!(r.latch(&sa, a));
        assert!(!r.latch(&sa, a), "같은 주소 재-binding 은 변화 없음");
        assert_eq!(r.by_addr(&a).unwrap().creds.ufrag, "pubA");
        assert!(r.latch(&sa, b), "모바일 망 전환 — 재시작 없이 latch 가 흡수");
        assert!(r.by_addr(&a).is_none() && r.by_addr(&b).unwrap().user_id == "u1");
        let sb = r.by_ufrag("pubB").unwrap();
        assert!(r.latch(&sb, b), "OS 가 포트를 재배정 — 주소는 새 세션에 간다");
        assert_eq!(r.by_addr(&b).unwrap().user_id, "u2");
        assert!(r.by_ufrag("pubA").is_some() && !sa.cancel.is_cancelled(), "옛 세션은 주소만 잃지 죽지 않는다");
    }

    #[test]
    fn dc_buffers_until_open_then_drains() {
        let r = TransportRegistry::default();
        let owner = peer("u", PcMode::OnePc, "p", "s");
        r.register(&owner);
        let s = r.by_ufrag("p").unwrap();
        assert!(!s.dc_ready());
        for i in 0..(DC_PENDING_CAP + 3) {
            s.dc_send(vec![i as u8]);
        }
        assert_eq!(s.dc_dropped(), 3, "넘친 만큼 오래된 것부터");
        let (tx, mut rx) = mpsc::channel(DC_PENDING_CAP * 2);
        s.dc_open(tx);
        assert!(s.dc_ready());
        assert_eq!(rx.try_recv().unwrap(), vec![3u8], "가장 오래된 셋이 버려졌다");
        s.dc_send(vec![200]);
        let mut last = Vec::new();
        while let Ok(f) = rx.try_recv() {
            last = f;
        }
        assert_eq!(last, vec![200u8]);
        s.dc_close();
        assert!(!s.dc_ready());
    }

    #[test]
    fn handshake_claim_is_one_shot() {
        let owner = peer("u", PcMode::OnePc, "x", "y");
        let s = TransportSession::new(&owner, ConnRole::Publish, creds("x"));
        assert!(s.claim_handshake());
        assert!(!s.claim_handshake(), "USE-CANDIDATE 재전송이 겹쳐도 한 번만");
        assert!(!s.srtp_ready() && s.dtls_tx().is_none());
    }
}
