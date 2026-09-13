// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§12 · §17-2 · model: claude-opus-5

//! ICE-lite 의 장부 — ★**ufrag 로 찾고, 주소는 latch 가 들고 있다.**
//!
//! ★★**세션 동일성은 주소가 아니라 ufrag 다**(정§12). 주소로 판정하면 NAT 이 산 peer 의
//! 포트를 남에게 재배정할 때 ★**옛 peer 의 세션이 죽는다**(20260826b 소스 감사가 찾은 구멍).
//! 그래서 여기 키는 늘 ufrag 이고 주소는 ufrag 아래에 매달린 값이다.
//!
//! ★**읽기는 락을 안 잡는다**(핫패스 규율 H2) — 표는 `ArcSwap` 으로 통째 바꿔 끼우고,
//! STUN 이 매번 하는 조회는 원자 적재 하나다. 바꿔 끼우는 쪽(입·퇴장)은 드물다.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use arc_swap::ArcSwap;

/// 한 Peer 는 자격을 두 벌 갖는다 — ★**연결이 둘이다**(`2pc`). `1pc` 은 보내기 한 벌만 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceRole {
    Publish,
    Subscribe,
}

/// 그 자격으로 붙은 상대의 현재 주소. ★**STUN 이 옮기고 전송이 읽는다.**
///
/// ★**주소 변경(모바일 망 전환)은 ICE 재시작 없이 여기서 흡수된다**(정§12) —
/// 새 주소에서 온 Binding 이 integrity 를 통과하면 그 자리에서 갈아 끼운다.
pub type Latch = Arc<RwLock<Option<SocketAddr>>>;

#[derive(Clone)]
pub struct TcpHandle(pub Arc<tokio::sync::mpsc::Sender<bytes::Bytes>>);

impl TcpHandle {
    pub fn send(&self, framed: bytes::Bytes) -> bool {
        self.0.try_send(framed).is_ok()
    }
}

impl std::fmt::Debug for TcpHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TcpHandle")
    }
}

#[derive(Debug, Default)]
pub struct RouteState {
    pub udp: Option<SocketAddr>,
    pub tcp: Option<TcpHandle>,
}

pub type Route = Arc<RwLock<RouteState>>;

pub const UDP_SILENCE_FLOOR_MS: u64 = 5_000;
pub const UDP_SILENCE_CEIL_MS: u64 = 30_000;

const GAP_EWMA_SHIFT: u32 = 3;

#[derive(Debug, Clone)]
pub enum Path {
    Udp(SocketAddr),
    Tcp(TcpHandle),
    Nowhere,
}

/// ufrag 한 벌이 가리키는 것.
#[derive(Debug)]
pub struct IceEntry {
    pub pwd: String,
    pub session_id: String,
    pub role: IceRole,
    pub route: Route,
    /// 마지막으로 그 자격에서 **UDP 가** 온 시각(ms). ★**좀비 판정의 유일한 근거다**(정§2-2).
    ///
    /// ★**`0` 은 "판정 불가"이지 "오래됨"이 아니다** — 아직 한 번도 못 본 것이라
    /// 접속 직후 즉사를 막는다. WS 하트비트는 이 값을 갱신하지 않는다.
    last_seen: AtomicU64,
    udp_gap_ewma_ms: AtomicU64,
}

impl IceEntry {
    pub fn touch(&self, now: u64) {
        // ★되감기를 막는다 — 늦게 도착한 옛 패킷이 시계를 되돌리면 죽은 것이 살아난다.
        let prev = self.last_seen.fetch_max(now, Ordering::Relaxed);
        if prev == 0 || now <= prev {
            return;
        }
        let gap = now - prev;
        let old = self.udp_gap_ewma_ms.load(Ordering::Relaxed);
        let next = if old == 0 {
            gap
        } else {
            old - (old >> GAP_EWMA_SHIFT) + (gap >> GAP_EWMA_SHIFT)
        };
        self.udp_gap_ewma_ms.store(next, Ordering::Relaxed);
    }

    pub fn udp_gap_ewma(&self) -> u64 {
        self.udp_gap_ewma_ms.load(Ordering::Relaxed)
    }

    pub fn udp_silence_threshold(&self) -> u64 {
        self.udp_gap_ewma()
            .saturating_mul(3)
            .clamp(UDP_SILENCE_FLOOR_MS, UDP_SILENCE_CEIL_MS)
    }

    pub fn udp_fresh(&self, now: u64) -> bool {
        let seen = self.last_seen();
        seen > 0 && now.saturating_sub(seen) < self.udp_silence_threshold()
    }

    pub fn pick(&self, now: u64) -> Path {
        let (udp, tcp) = {
            let r = self.route.read().expect("route 자물쇠는 패닉을 건너지 않는다");
            (r.udp, r.tcp.clone())
        };
        match (udp, tcp) {
            (None, None) => Path::Nowhere,
            (Some(a), None) => Path::Udp(a),
            (None, Some(h)) => Path::Tcp(h),
            (Some(a), Some(h)) => {
                if self.udp_fresh(now) {
                    Path::Udp(a)
                } else {
                    Path::Tcp(h)
                }
            }
        }
    }

    pub fn last_seen(&self) -> u64 {
        self.last_seen.load(Ordering::Relaxed)
    }

    /// 그 자격이 지금 가리키는 주소. ★**latch 전에는 `None`** 이다 — 지어내지 않는다.
    pub fn addr(&self) -> Option<SocketAddr> {
        self.route.read().expect("route 자물쇠는 패닉을 건너지 않는다").udp
    }

    pub fn tcp(&self) -> Option<TcpHandle> {
        self.route.read().expect("route 자물쇠는 패닉을 건너지 않는다").tcp.clone()
    }

    pub fn attach_tcp(&self, handle: TcpHandle) {
        self.route.write().expect("route 자물쇠").tcp = Some(handle);
    }

    pub fn detach_tcp_if(&self, mine: &TcpHandle) {
        let mut r = self.route.write().expect("route 자물쇠");
        if r.tcp.as_ref().is_some_and(|h| Arc::ptr_eq(&h.0, &mine.0)) {
            r.tcp = None;
        }
    }

    pub fn reachable(&self) -> bool {
        let r = self.route.read().expect("route 자물쇠는 패닉을 건너지 않는다");
        r.udp.is_some() || r.tcp.is_some()
    }
}

/// ufrag → 자격. ★**읽기가 락을 안 잡는다.**
#[derive(Debug, Default)]
pub struct IceTable(ArcSwap<HashMap<String, Arc<IceEntry>>>);

impl IceTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// ★**핫패스** — STUN 하나마다 한 번. 원자 적재 하나다.
    pub fn get(&self, ufrag: &str) -> Option<Arc<IceEntry>> {
        self.0.load().get(ufrag).cloned()
    }

    pub fn routable(&self, ufrag: &str) -> Option<Arc<IceEntry>> {
        self.get(ufrag).filter(|e| e.reachable())
    }

    pub fn len(&self) -> usize {
        self.0.load().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 자격 한 벌을 올린다(입장). ★**표를 새로 지어 바꿔 끼운다** — 읽는 쪽은 멈추지 않는다.
    pub fn insert(&self, ufrag: &str, pwd: &str, session_id: &str, role: IceRole) -> Arc<IceEntry> {
        let e = Arc::new(IceEntry {
            pwd: pwd.to_string(),
            session_id: session_id.to_string(),
            role,
            route: Route::default(),
            // ★**관찰 전이다** — 0 으로 시작해야 reaper 가 판정을 건너뛴다(정§2-2).
            last_seen: AtomicU64::new(0),
            udp_gap_ewma_ms: AtomicU64::new(0),
        });
        let mut next = HashMap::clone(&self.0.load());
        next.insert(ufrag.to_string(), e.clone());
        self.0.store(Arc::new(next));
        e
    }

    /// 그 세션의 자격 전부를 내린다(회수·퇴장). ★**한 번에 바꾼다** — 반만 지운 창을 두지 않는다.
    pub fn drop_session(&self, session_id: &str) -> usize {
        let cur = self.0.load();
        let mut next = HashMap::clone(&cur);
        next.retain(|_, e| e.session_id != session_id);
        let gone = cur.len() - next.len();
        if gone > 0 {
            self.0.store(Arc::new(next));
        }
        gone
    }

    /// 세션마다 ★**가장 최근의 UDP 관찰**. 자격 두 벌 중 하나만 흘러도 그 Peer 는 살아 있다
    /// (`2pc` 는 받기만 흐르는 구간이 정상 형상이다).
    ///
    /// ★**전부 `0`(관찰 전)이면 `0`** 을 낸다 — 부르는 쪽이 판정을 건너뛴다.
    pub fn last_seen_by_session(&self) -> Vec<(String, u64)> {
        let mut out: Vec<(String, u64)> = Vec::new();
        for e in self.0.load().values() {
            let t = e.last_seen();
            match out.iter_mut().find(|(s, _)| s == &e.session_id) {
                Some((_, cur)) => *cur = (*cur).max(t),
                None => out.push((e.session_id.clone(), t)),
            }
        }
        out
    }
}

/// STUN Binding 하나를 처리한 결과. ★**판정만 낸다** — 소켓은 부르는 쪽이 쥔다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// 답할 것. `latched` 면 이 요청이 주소를 옮겼다.
    ///
    /// ★**`ufrag` 를 같이 낸다** — 세션 동일성이 주소가 아니라 그것이므로, 부르는 쪽의
    /// 통로 장부도 같은 키를 써야 한다(정§12).
    Respond { wire: Vec<u8>, ufrag: String, session_id: String, latched: bool },
    /// ★**조용히 버린다** — 위조에 답하면 그 자체가 신호가 된다(어떤 ufrag 이 사는지 알려 준다).
    Drop(DropWhy),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    Udp,
    Tcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropWhy {
    NotStun,
    NotBinding,
    NoUsername,
    UnknownUfrag,
    /// ★위조 — ufrag 는 맞는데 서명이 틀렸다.
    BadIntegrity,
}

/// ★★**순서가 곧 방어다**(정§12): ufrag 조회 → integrity → latch → 응답 → `last_seen`.
pub fn on_binding(
    table: &IceTable,
    buf: &[u8],
    from: SocketAddr,
    now: u64,
    via: Arrival,
) -> Binding {
    let Some(msg) = stun_parse(buf) else {
        return Binding::Drop(DropWhy::NotStun);
    };
    if !msg.is_binding_request() {
        return Binding::Drop(DropWhy::NotBinding);
    }
    let Some(ufrag) = msg.local_ufrag() else {
        return Binding::Drop(DropWhy::NoUsername);
    };
    let Some(e) = table.get(ufrag) else {
        return Binding::Drop(DropWhy::UnknownUfrag);
    };
    // ★**여기를 지나기 전에는 아무것도 바꾸지 않는다.**
    if !msg.integrity_ok(&e.pwd) {
        return Binding::Drop(DropWhy::BadIntegrity);
    }
    let latched = match via {
        Arrival::Udp => {
            let mut cur = e.route.write().expect("route 자물쇠");
            let moved = cur.udp != Some(from);
            cur.udp = Some(from);
            e.touch(now);
            moved
        }
        Arrival::Tcp => false,
    };
    Binding::Respond {
        wire: crate::transport::stun::binding_response(&msg.transaction_id, from, &e.pwd),
        ufrag: ufrag.to_string(),
        session_id: e.session_id.clone(),
        latched,
    }
}

fn stun_parse(buf: &[u8]) -> Option<crate::transport::stun::Message<'_>> {
    crate::transport::stun::parse(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PWD: &str = "zxcvbnmasdfghjklqwerty";

    fn table() -> IceTable {
        let t = IceTable::new();
        t.insert("srvufrag", PWD, "s-1", IceRole::Publish);
        t
    }

    fn request(ufrag: &str, pwd: &str) -> Vec<u8> {
        crate::transport::stun::binding_request(&[9u8; 12], &format!("{ufrag}:bot"), pwd, true)
    }

    fn addr(s: &str) -> SocketAddr {
        s.parse().expect("주소")
    }

    #[test]
    fn 맞는_자격은_latch_를_옮기고_답한다() {
        let t = table();
        let r = on_binding(&t, &request("srvufrag", PWD), addr("1.2.3.4:5"), 100, Arrival::Udp);
        let Binding::Respond { session_id, ufrag, latched, wire } = r else { panic!("답해야 한다") };
        assert_eq!((session_id.as_str(), ufrag.as_str(), latched), ("s-1", "srvufrag", true));
        assert!(!wire.is_empty());
        let e = t.get("srvufrag").expect("있다");
        assert_eq!((e.addr(), e.last_seen()), (Some(addr("1.2.3.4:5")), 100));

        // ★같은 주소에서 또 오면 옮긴 것이 아니다 — 그래야 "옮겼다"가 신호로 쓰인다.
        let r = on_binding(&t, &request("srvufrag", PWD), addr("1.2.3.4:5"), 200, Arrival::Udp);
        assert!(matches!(r, Binding::Respond { latched: false, .. }));

        // ★망이 바뀌면 새 주소가 그 자리에서 들어온다 — ICE 재시작이 없다(정§12).
        let r = on_binding(&t, &request("srvufrag", PWD), addr("9.9.9.9:7"), 300, Arrival::Udp);
        assert!(matches!(r, Binding::Respond { latched: true, .. }));
        assert_eq!(t.get("srvufrag").expect("있다").addr(), Some(addr("9.9.9.9:7")));
    }

    fn handle() -> (TcpHandle, tokio::sync::mpsc::Receiver<bytes::Bytes>) {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        (TcpHandle(Arc::new(tx)), rx)
    }

    #[test]
    fn 갈_데가_없으면_아무_데도_안_간다() {
        let t = table();
        assert!(matches!(t.get("srvufrag").expect("있다").pick(1000), Path::Nowhere));
    }

    #[test]
    fn tcp_가_없으면_묵은_udp_라도_udp_로_간다() {
        let t = table();
        on_binding(&t, &request("srvufrag", PWD), addr("1.2.3.4:5"), 100, Arrival::Udp);
        let e = t.get("srvufrag").expect("있다");
        assert!(matches!(e.pick(999_999), Path::Udp(_)), "★선택지가 없다");
    }

    #[test]
    fn udp_가_최근이면_tcp_가_있어도_udp_다() {
        let t = table();
        on_binding(&t, &request("srvufrag", PWD), addr("1.2.3.4:5"), 10_000, Arrival::Udp);
        let e = t.get("srvufrag").expect("있다");
        e.attach_tcp(handle().0);
        assert!(matches!(e.pick(11_000), Path::Udp(_)));
    }

    #[test]
    fn udp_가_묵으면_tcp_로_넘어간다() {
        let t = table();
        on_binding(&t, &request("srvufrag", PWD), addr("1.2.3.4:5"), 10_000, Arrival::Udp);
        let e = t.get("srvufrag").expect("있다");
        e.attach_tcp(handle().0);
        assert!(matches!(e.pick(10_000 + UDP_SILENCE_FLOOR_MS + 1), Path::Tcp(_)));
    }

    #[test]
    fn udp_가_다시_오면_그_자리에서_되돌아온다() {
        let t = table();
        on_binding(&t, &request("srvufrag", PWD), addr("1.2.3.4:5"), 10_000, Arrival::Udp);
        let e = t.get("srvufrag").expect("있다");
        e.attach_tcp(handle().0);
        let late = 10_000 + UDP_SILENCE_FLOOR_MS + 1;
        assert!(matches!(e.pick(late), Path::Tcp(_)));
        e.touch(late);
        assert!(matches!(e.pick(late), Path::Udp(_)), "★별도 복귀 조건을 두지 않는다");
    }

    #[test]
    fn 임계는_관측한_간격에서_자란다() {
        let t = table();
        let e = t.get("srvufrag").expect("있다");
        assert_eq!(e.udp_silence_threshold(), UDP_SILENCE_FLOOR_MS, "★표본 전에는 하한이다");
        let mut at = 1_000u64;
        e.touch(at);
        for _ in 0..40 {
            at += 20_000;
            e.touch(at);
        }
        assert!(e.udp_gap_ewma() > 15_000, "★절감 설정 클라의 간격을 배운다: {}", e.udp_gap_ewma());
        assert_eq!(e.udp_silence_threshold(), UDP_SILENCE_CEIL_MS, "★그래도 상한에서 멈춘다");
    }

    #[test]
    fn 촘촘한_클라는_하한_아래로_안_내려간다() {
        let t = table();
        let e = t.get("srvufrag").expect("있다");
        let mut at = 1_000u64;
        e.touch(at);
        for _ in 0..40 {
            at += 200;
            e.touch(at);
        }
        assert!(e.udp_gap_ewma() < 1_000, "간격은 작게 배웠는데");
        assert_eq!(e.udp_silence_threshold(), UDP_SILENCE_FLOOR_MS, "★하한이 받친다");
    }

    #[test]
    fn tcp_로_온_것은_udp_latch_를_안_옮긴다() {
        let t = table();
        on_binding(&t, &request("srvufrag", PWD), addr("1.2.3.4:5"), 100, Arrival::Udp);
        let r = on_binding(&t, &request("srvufrag", PWD), addr("9.9.9.9:7"), 200, Arrival::Tcp);
        assert!(matches!(r, Binding::Respond { latched: false, .. }), "★답은 한다");
        let e = t.get("srvufrag").expect("있다");
        assert_eq!(e.addr(), Some(addr("1.2.3.4:5")), "★udp 하향 주소가 그대로다");
    }

    #[test]
    fn tcp_로_온_것은_last_seen_을_안_건드린다() {
        let t = table();
        on_binding(&t, &request("srvufrag", PWD), addr("1.2.3.4:5"), 100, Arrival::Udp);
        on_binding(&t, &request("srvufrag", PWD), addr("9.9.9.9:7"), 900, Arrival::Tcp);
        let e = t.get("srvufrag").expect("있다");
        assert_eq!(e.last_seen(), 100, "★좀비 판정의 근거는 udp 관찰뿐이다");
    }

    #[test]
    fn tcp_는_latch_전에도_답한다() {
        let t = table();
        let r = on_binding(&t, &request("srvufrag", PWD), addr("9.9.9.9:7"), 100, Arrival::Tcp);
        assert!(matches!(r, Binding::Respond { latched: false, .. }));
        let e = t.get("srvufrag").expect("있다");
        assert_eq!(e.addr(), None, "★지어내지 않는다");
        assert_eq!(e.last_seen(), 0, "★관찰 전이다");
    }

    #[test]
    fn 위조는_latch_를_못_옮긴다() {
        let t = table();
        on_binding(&t, &request("srvufrag", PWD), addr("1.2.3.4:5"), 100, Arrival::Udp);
        // ★**ufrag 는 맞는데 서명이 틀리다** — `server_config` 를 본 사람이면 ufrag 는 다 안다.
        let r = on_binding(&t, &request("srvufrag", "wrong-password-000000"), addr("6.6.6.6:6"), 200, Arrival::Udp);
        assert_eq!(r, Binding::Drop(DropWhy::BadIntegrity));
        let e = t.get("srvufrag").expect("있다");
        assert_eq!(e.addr(), Some(addr("1.2.3.4:5")), "★주소가 그대로다");
        assert_eq!(e.last_seen(), 100, "★죽어 가는 세션을 위조가 살려 주지도 않는다");
    }

    #[test]
    fn 모르는_ufrag_는_조용히_버린다() {
        let t = table();
        let r = on_binding(&t, &request("nope", PWD), addr("1.2.3.4:5"), 0, Arrival::Udp);
        // ★답하면 그 자체가 신호다 — 어떤 ufrag 이 사는지 알려 준다.
        assert_eq!(r, Binding::Drop(DropWhy::UnknownUfrag));
    }

    #[test]
    fn 세션을_내리면_자격_두_벌이_같이_간다() {
        let t = IceTable::new();
        t.insert("pub-u", PWD, "s-1", IceRole::Publish);
        t.insert("sub-u", PWD, "s-1", IceRole::Subscribe);
        t.insert("other", PWD, "s-2", IceRole::Publish);
        assert_eq!(t.drop_session("s-1"), 2);
        assert_eq!(t.len(), 1, "남의 자격은 그대로다");
    }

    #[test]
    fn 자격_한_벌만_흘러도_그_peer_는_살아_있다() {
        let t = IceTable::new();
        let a = t.insert("pub-u", PWD, "s-1", IceRole::Publish);
        t.insert("sub-u", PWD, "s-1", IceRole::Subscribe);
        // ★받기만 흐르는 구간은 `2pc` 의 정상 형상이다 — 보내기가 조용하다고 회수하면 안 된다.
        a.touch(1_000);
        assert_eq!(t.last_seen_by_session(), vec![("s-1".to_string(), 1_000)]);
    }

    #[test]
    fn 관찰_전에는_0_이다() {
        let t = table();
        assert_eq!(t.last_seen_by_session(), vec![("s-1".to_string(), 0)], "★지어내지 않는다");
    }

    #[test]
    fn 시각은_되감기지_않는다() {
        let t = table();
        let e = t.get("srvufrag").expect("있다");
        e.touch(1_000);
        // ★늦게 온 옛 패킷이 시계를 되돌리면 죽은 세션이 살아난다.
        e.touch(10);
        assert_eq!(e.last_seen(), 1_000);
    }

    #[test]
    fn latch_는_처음에_비어_있다() {
        let t = table();
        assert_eq!(t.get("srvufrag").expect("있다").addr(), None, "★지어내지 않는다");
    }
}
