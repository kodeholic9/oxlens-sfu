// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§15-0 · §15-1 · §15-7 · model: claude-opus-5

//! 이 node 의 버스 — ★**hub 가 router 를 임베드한다**(정§15-0).
//!
//! ★★**`node` 토큰이 노드 축의 유일한 권위다**(정§16-1) — 유닛 축(supervisor)과 다른 물음이다.
//! 그 토큰은 ★**선언자의 세션에 묶여 있어 급사·고립에서 저절로 꺼진다** — 켜고 끄는 코드를
//! 우리가 들지 않는다. 손으로 관리하면 *"죽었는데 살아 있다고 적힌"* 순간이 생긴다.

use std::collections::BTreeMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use common::bus::{self, Keys};
use common::system::Zenoh;

/// 지금 버스 위에 있는 node 들 — ★**읽기는 자물쇠를 안 잡는다**(RCU).
///
/// ★**`{inst}` 를 같이 든다** — 그 값이 바뀌면 그 node 는 ★**재기동한 것**이고,
/// 이름만 보면 *"계속 살아 있던 node"* 와 구별이 안 된다(운영 §3-8).
#[derive(Debug, Default)]
pub struct Live {
    nodes: ArcSwap<BTreeMap<String, String>>,
    /// 마지막 토큰 사건 — ★**조용한 것과 죽은 것을 가른다.**
    last: ArcSwap<Option<(u64, &'static str, String)>>,
    /// ★★**배워서 아는 방**(정§15-2 `room` 토큰) — `room` → 그 방을 쥔 `(node, inst)` 들.
    ///
    /// ★**내가 만든 방과 다른 축이다** — 대장(`hub.rooms`)은 *"내가 이런 방을 만들라고 했다"*
    /// 이고 이것은 *"그 방이 실제로 저기 서 있다"* 다. ★둘이 어긋나는 것이 사고의 형상이다.
    /// ★**값이 둘 이상이면 방 중복**이다(§15-3 — 1차는 탐지·계수·경보다).
    rooms: ArcSwap<BTreeMap<String, BTreeMap<String, String>>>,
    /// ★★**자기 고립**(정§15-7) — 타 node 토큰이 ★**일괄로** 사라진 시각.
    ///
    /// ★*"저들이 죽었다"* 가 아니라 *"내가 고립됐다"* 다 — zenoh 는 재연결 실패 시
    /// `**` 무효화를 보낸다. ★**이 둘을 가르지 않으면 고립된 hub 가 남의 방을
    /// 죽었다고 선언**하고, 그 방은 멀쩡히 살아 있다.
    isolated_at: ArcSwap<Option<u64>>,
    /// ★**남을 본 적이 있나** — 고립은 ★**일괄로 「사라진」 것**이지 「처음부터 혼자」가 아니다.
    /// 이 구분이 없으면 ★**한 대 형상이 영영 `ready` 503** 이다(정상 배치인데).
    saw_others: std::sync::atomic::AtomicBool,
    /// ★★**사라진 node 와 그 시각**(정§15-3 grace) — ★**배치 후보에서 곧바로 빼지 않는다.**
    ///
    /// ★빼면 망이 잠깐 흔들릴 때마다 ★**방이 통째로 옮겨 다닌다** — 미디어는 그 자리에
    /// 있는데 hub 만 딴 데를 가리킨다. `route::GRACE_MS` 가 그 창이다.
    gone: ArcSwap<BTreeMap<String, u64>>,
}

impl Live {
    pub fn snapshot(&self) -> Arc<BTreeMap<String, String>> {
        self.nodes.load_full()
    }

    /// 배치 후보 — ★**산 것 + grace 안에 사라진 것**(정§15-3).
    ///
    /// ★**살아 있는 것만으로 좁히지 않는다** — 좁히면 node 하나가 죽는 창에 그 방이
    /// 다른 node 로 옮겨 붙어 *"한 방은 한 sfud"* 가 창마다 흔들린다.
    pub fn candidates(&self, now: u64) -> Vec<crate::route::Node> {
        let live = self.nodes.load();
        let mut out: Vec<crate::route::Node> = live
            .keys()
            .map(|n| crate::route::Node { node_id: n.clone(), live: true, gone_at: None })
            .collect();
        for (n, at) in self.gone.load().iter() {
            if !live.contains_key(n) {
                out.push(crate::route::Node {
                    node_id: n.clone(),
                    // ★**`live` 가 아니다** — 후보엔 남되 새 방은 여기 못 앉는다(§16-1 `5001`).
                    live: false,
                    gone_at: Some(*at),
                });
            }
        }
        let _ = now;
        out
    }

    /// 마지막 사건 `(시각, 종류, 키)`.
    pub fn last_event(&self) -> Option<(u64, &'static str, String)> {
        self.last.load_full().as_ref().clone()
    }

    /// ★**그 node 가 버스 위에 있나** — dial 이 아니다(정§16-1 *"붙는 것과 그 node 가
    /// 버스 위에 있는 것은 다른 물음"*).
    pub fn has(&self, node: &str) -> bool {
        self.nodes.load().contains_key(node)
    }

    fn put(&self, node: &str, inst: &str) {
        let mut next = BTreeMap::clone(&self.nodes.load());
        next.insert(node.to_string(), inst.to_string());
        self.nodes.store(Arc::new(next));
        // ★돌아왔으면 죽은 표에서 뺀다 — 안 빼면 같은 node 가 후보에 둘로 선다.
        let mut g = BTreeMap::clone(&self.gone.load());
        if g.remove(node).is_some() {
            self.gone.store(Arc::new(g));
        }
    }

    /// 그 방이 서 있는 자리들 — ★**빈 것과 「모른다」가 같다**(아직 못 배운 것이다).
    pub fn rooms(&self) -> Arc<BTreeMap<String, BTreeMap<String, String>>> {
        self.rooms.load_full()
    }

    /// 그 방을 쥔 node — ★**둘 이상이면 `None`** 이다(중복은 답이 아니라 사고다).
    pub fn room_node(&self, room: &str) -> Option<String> {
        let r = self.rooms.load();
        let m = r.get(room)?;
        (m.len() == 1).then(|| m.keys().next().cloned()).flatten()
    }

    /// 고립돼 있나 — ★**`ready` 가 이 값을 본다**(정§15-7 → LB 배수).
    pub fn isolated(&self) -> bool {
        self.isolated_at.load().is_some()
    }

    fn put_room(&self, room: &str, node: &str, inst: &str) {
        let mut next = BTreeMap::clone(&self.rooms.load());
        next.entry(room.to_string()).or_default().insert(node.to_string(), inst.to_string());
        self.rooms.store(Arc::new(next));
    }

    /// 그 자리를 거둔다. ★**`(room, node)` 짝으로 짚는다** — 방 이름만 보고 지우면
    /// 중복된 방의 산 자리까지 사라진다.
    ///
    /// ★**판정이 선 뒤에 부른다** — 토큰이 사라진 그 자리에서 지우면 배치 계산이 그 틈에
    /// 그 방을 옮겨 앉힌다.
    pub fn drop_room(&self, room: &str, node: &str) {
        let mut next = BTreeMap::clone(&self.rooms.load());
        let empty = match next.get_mut(room) {
            None => return,
            Some(m) => {
                m.remove(node);
                m.is_empty()
            }
        };
        if empty {
            next.remove(room);
        }
        self.rooms.store(Arc::new(next));
    }

    fn drop_one(&self, node: &str, now: u64) {
        let mut next = BTreeMap::clone(&self.nodes.load());
        next.remove(node);
        self.nodes.store(Arc::new(next));
        let mut g = BTreeMap::clone(&self.gone.load());
        g.insert(node.to_string(), now);
        // ★오래된 것은 거둔다 — 안 그러면 한 번 죽은 node 가 영원히 후보에 남는다.
        g.retain(|_, at| now.saturating_sub(*at) <= crate::route::GRACE_MS * 2);
        self.gone.store(Arc::new(g));
    }

    /// ★**일괄 소멸 = 자기 고립**(정§15-7). 남이 하나도 안 남았는데 아까는 있었다면
    /// 그것은 ★**저들이 다 죽은 것이 아니라 내가 끊긴 것**이다.
    ///
    /// ★**「하나가 죽었다」와 「전부 사라졌다」를 가른다** — 앞은 그 node 의 방이 죽은
    /// 것이고 뒤는 ★**내 방들이 멀쩡한데 내가 못 보는 것**이다. 처방이 반대다.
    fn judge_isolation(&self, me: &str, now: u64) {
        use std::sync::atomic::Ordering;
        let alone = self.nodes.load().keys().all(|n| n == me);
        if !alone {
            self.saw_others.store(true, Ordering::Relaxed);
        }
        // ★**본 적이 없으면 잃은 것도 없다** — 한 대 형상은 고립이 아니다.
        let alone = alone && self.saw_others.load(Ordering::Relaxed);
        match (alone, self.isolated_at.load().is_some()) {
            (true, false) => {
                eprintln!("[bus] ★남이 하나도 안 보인다 — 자기 고립으로 읽는다(정§15-7)");
                self.isolated_at.store(Arc::new(Some(now)));
            }
            (false, true) => {
                eprintln!("[bus] 고립이 풀렸다 — 남이 다시 보인다");
                self.isolated_at.store(Arc::new(None));
            }
            _ => {}
        }
    }

    fn mark(&self, kind: &'static str, key: &str) {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.last.store(Arc::new(Some((at, kind, key.to_string()))));
    }
}

/// 선 버스 하나. ★**세션을 들고 있는 것이 곧 토큰을 살려 두는 것**이다.
pub struct Bus {
    pub node_id: String,
    pub inst: String,
    pub keys: Keys,
    pub live: Arc<Live>,
    /// ★**떨어뜨리면 토큰이 죽는다** — 그래서 들고 있는다(우리가 끄는 코드를 안 든다).
    _token: zenoh::liveliness::LivelinessToken,
    pub session: zenoh::Session,
}

/// node 간 요청 한 갈래 — ★**그 node 의 hub 에게 묻는다**(정§15-1 `q/node`).
///
/// ★★**관문 원칙이다** — 남의 sfud 에 직결하지 않는다. 그 node 의 관문(hub)이 제 유닛으로
/// 넘기고 답을 되돌린다. 직결하면 peer 형상이 되고 node N 대에 세션이 N(N−1)/2 로 는다.
pub const Q_HANDLE: &str = "handle";
/// 방 대장 밀기·현황 받기 — 같은 관문의 다른 갈래다.
pub const Q_ROOMS: &str = "rooms";
/// 그 node 가 제 눈으로 본 버스 — ★**전 node fan-out 의 대상**이다(운영 §3-8 `?all=1`).
pub const Q_BUS: &str = "bus";
/// 방 상세 정본(운영 §3-6) — ★**hub 사본이 아니라 그 방을 쥔 sfud 가 답한다.**
pub const Q_SNAPSHOT: &str = "snapshot";
/// C 평면이 시키는 것(정§16-1-2·§16-1-3) — ★**확인값 판정은 부르는 쪽이 이미 했다.**
pub const Q_OPS: &str = "ops";

impl Bus {
    /// 남의 node 에 프레임 하나를 넘기고 답을 받는다. ★**없으면 `None`** — 지어내지 않는다.
    pub async fn ask(&self, node: &str, rest: &str, body: Vec<u8>) -> Option<Vec<u8>> {
        let replies = self
            .keys
            .q_node(node, rest)
            .parse::<zenoh::key_expr::KeyExpr>()
            .ok()
            .map(|k| self.session.get(k).payload(body))?
            .await
            .ok()?;
        // ★**첫 답 하나다** — 한 node 가 답한다(전 node fan-out 은 아래가 따로 본다).
        let r = replies.recv_async().await.ok()?;
        let sample = r.result().ok()?;
        Some(sample.payload().to_bytes().to_vec())
    }

    /// ★**전 node 에 묻고 오는 대로 모은다**(정§15-1 `q/node/*`).
    ///
    /// ★**도구가 아니라 여기가 모은다** — 도구가 하려면 엔드포인트 목록을 들어야 하고,
    /// 그 목록은 ★**node 를 늘릴 때마다 낡는다**(운영 §3-8).
    /// ★**못 닿은 node 는 목록에 안 온다** — 부르는 쪽이 「기대한 node」와 견줘 빈자리를 찾는다.
    pub async fn ask_all(&self, rest: &str) -> Vec<(String, Vec<u8>)> {
        let Ok(key) = self.keys.q_all(rest).parse::<zenoh::key_expr::KeyExpr>() else {
            return Vec::new();
        };
        let Ok(replies) = self.session.get(key).await else { return Vec::new() };
        let mut out = Vec::new();
        while let Ok(r) = replies.recv_async().await {
            let Ok(sample) = r.result() else { continue };
            // 답한 키가 곧 누구인지다 — `…/q/node/{node}/{rest}`.
            let k = sample.key_expr().as_str();
            let node = k
                .split("/q/node/")
                .nth(1)
                .and_then(|t| t.split('/').next())
                .unwrap_or("")
                .to_string();
            out.push((node, sample.payload().to_bytes().to_vec()));
        }
        out
    }
}

/// 버스를 연다. ★**못 열면 `Err`** — 조용히 없는 채로 돌면 *"혼자 있는 줄 모르는 node"* 가 된다.
/// ★**방이 죽은 것을 hub 가 알아야 하는 자리**(정§15-4 예외) — 낼 주체가 사라졌으므로
/// ★**살아남은 각 hub 가 자기 로컬 멤버에게 합성**한다. 이것이 그 사실을 건네는 관이다.
/// 값은 `(room, 그 방을 쥐고 있던 node)` 다 — ★**누가 사라졌나까지 알아야 판정이 선다.**
pub type Closed = tokio::sync::mpsc::Receiver<(String, String)>;

pub async fn open(z: &Zenoh, node_id: &str, inst: &str) -> Result<(Bus, Closed), String> {
    // ★**키를 깨는 값은 여기서 막는다** — 붙고 나서 아무에게도 안 보이는 것이 제일 나쁘다.
    for (what, v) in [("node_id", node_id), ("inst", inst), ("namespace", &z.namespace[..])] {
        if !bus::is_key_segment(v) {
            return Err(format!("{what} 가 키 조각이 못 된다: {v:?}"));
        }
    }
    let mut cfg = zenoh::Config::default();
    for (path, v) in bus::config_json(z) {
        cfg.insert_json5(path, &v).map_err(|e| format!("zenoh 설정 {path}: {e}"))?;
    }
    let session = zenoh::open(cfg).await.map_err(|e| format!("zenoh open: {e}"))?;

    let keys = Keys::new(&z.namespace);
    let live = Arc::new(Live::default());
    // ★**나부터 목록에 넣는다** — 내 토큰의 메아리를 기다리지 않는다(자기 선언은 안 올 수 있다).
    live.put(node_id, inst);

    // ★**듣기를 먼저 걸고 선언한다** — 순서가 뒤집히면 내 선언과 남의 선언 사이가 빈다.
    let sub = session
        .liveliness()
        .declare_subscriber(keys.node_all())
        .history(true)
        .await
        .map_err(|e| format!("liveliness 구독: {e}"))?;
    // ★★**`history` 가 계약이다**(정§15-2 증상 첫 줄) — 없으면 ★**늦게 뜬 hub 가 이미 있는
    //   방을 영영 모르고**, 그 hub 에 붙은 클라는 멀쩡한 방에 `3001` 을 받는다.
    let rsub = session
        .liveliness()
        .declare_subscriber(keys.room_all())
        .history(true)
        .await
        .map_err(|e| format!("room 구독: {e}"))?;
    let token = session
        .liveliness()
        .declare_token(keys.node(node_id, inst))
        .await
        .map_err(|e| format!("liveliness 선언: {e}"))?;

    let (closed_tx, closed_rx) = tokio::sync::mpsc::channel::<(String, String)>(1024);
    let rseen = live.clone();
    let me = node_id.to_string();
    tokio::spawn(async move {
        while let Ok(s) = rsub.recv_async().await {
            let key = s.key_expr().as_str().to_string();
            let Some((room, node, inst)) = bus::room_of(&key) else { continue };
            match s.kind() {
                zenoh::sample::SampleKind::Put => {
                    rseen.put_room(room, node, inst);
                    if rseen.rooms().get(room).is_some_and(|m| m.len() > 1) {
                        // ★**1차는 탐지·계수·경보다**(정§15-3) — 자동 수선은 실측 뒤다.
                        eprintln!("[bus] ★방 중복 {room} — node 둘이 같은 방을 쥐었다");
                    }
                }
                zenoh::sample::SampleKind::Delete => {
                    // ★★**여기서 지우지도 않는다.** 지우면 그 방의 자리가 「모른다」가 되고,
                    //   ★배치 계산이 그 틈에 ★**그 방을 내 node 로 옮겨 앉힌다** — 고립된
                    //   hub 가 남의 방을 제 것으로 만드는 형상이다(실측 20260913).
                    //   ★**판정이 선 뒤에 지운다**(아래 `Closed` 소비자).
                    // ★★**여기서 판정하지 않는다** — 「그 방이 죽었다」인지 「평범한 소멸」인지는
                    //   ★**그 node 가 살아 있나**로 갈리는데, node 토큰 Delete 와 room 토큰
                    //   Delete 는 ★**한 세션이 끊기며 함께 나가 순서가 정해져 있지 않다**
                    //   (실측 20260913: room 이 먼저 와서 판정이 통째로 빗나갔다).
                    //   ★그래서 사실만 넘기고 판정은 가라앉은 뒤에 한다.
                    if node != me {
                        let _ = closed_tx.try_send((room.to_string(), node.to_string()));
                    }
                    rseen.mark("delete", &key);
                }
            }
        }
        eprintln!("[bus] ★room 구독이 끝났다 — 이 node 는 이제 남의 방을 못 본다");
    });

    let seen = live.clone();
    let mine = node_id.to_string();
    tokio::spawn(async move {
        while let Ok(s) = sub.recv_async().await {
            let key = s.key_expr().as_str().to_string();
            let Some((node, _inst)) = bus::node_of(&key) else { continue };
            match s.kind() {
                zenoh::sample::SampleKind::Put => {
                    eprintln!("[bus] node 등장 {node}");
                    seen.put(node, _inst);
                    seen.mark("put", &key);
                    seen.judge_isolation(&mine, now_ms());
                }
                // ★**Delete 는 라우팅 표를 비우는 데까지다** — 재배치를 촉발하지 않는다(정§15-7).
                zenoh::sample::SampleKind::Delete => {
                    eprintln!("[bus] node 소멸 {node}");
                    let at = now_ms();
                    seen.drop_one(node, at);
                    seen.mark("delete", &key);
                    seen.judge_isolation(&mine, at);
                }
            }
        }
        eprintln!("[bus] ★liveliness 구독이 끝났다 — 이 node 는 이제 남을 못 본다");
    });

    Ok((
        Bus { node_id: node_id.to_string(), inst: inst.to_string(), keys, live, _token: token, session },
        closed_rx,
    ))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 죽은_node_는_grace_동안_후보에_남는다() {
        // ★★**빼면 망이 흔들릴 때마다 방이 옮겨 다닌다** — 미디어는 그 자리에 있는데
        //   hub 만 딴 데를 가리킨다(실측 20260913: node 하나를 죽이자 그 방이 곧바로 이사했다).
        let l = Live::default();
        l.put("node-a", "i1");
        l.put("node-b", "i2");
        l.drop_one("node-b", 1_000);
        let c = l.candidates(1_000);
        assert_eq!(c.len(), 2, "죽자마자 후보에서 빠졌다");
        let b = c.iter().find(|n| n.node_id == "node-b").expect("node-b");
        // ★후보엔 남되 ★**`live` 가 아니다** — 새 방은 여기 못 앉는다(정§16-1 `5001`).
        assert!(!b.live && b.gone_at == Some(1_000));
        // ★배치 함수가 grace 를 본다 — 그래서 창 안에서는 답이 안 바뀐다.
        assert!(crate::route::place(&c, "r1", 1_000).is_some());
        // 돌아오면 죽은 표에서 빠진다 — 같은 node 가 후보에 둘로 서면 안 된다.
        l.put("node-b", "i3");
        assert_eq!(l.candidates(2_000).len(), 2);
    }

    #[test]
    fn 배운_자리는_짝으로_짚는다() {
        // ★방 이름만 보고 지우면 ★**중복된 방의 산 자리까지** 사라진다.
        let l = Live::default();
        l.put_room("r1", "node-a", "i1");
        l.put_room("r1", "node-b", "i2");
        assert_eq!(l.rooms().get("r1").map(|m| m.len()), Some(2));
        // ★중복은 답이 아니라 사고다 — 둘이면 「어디냐」에 답하지 않는다.
        assert_eq!(l.room_node("r1"), None);
        l.drop_room("r1", "node-b");
        assert_eq!(l.room_node("r1"), Some("node-a".into()));
        l.drop_room("r1", "node-a");
        // ★빈 자리를 남기지 않는다 — 남기면 「있는데 아무도 안 쥔 방」이 장부에 는다.
        assert!(l.rooms().get("r1").is_none());
    }

    #[test]
    fn 모르는_방은_없는_것이_아니라_모르는_것이다() {
        // ★그래서 라우팅이 계산(HRW)으로 넘어간다 — 여기서 `None` 은 「아직 못 배웠다」다.
        let l = Live::default();
        assert_eq!(l.room_node("r9"), None);
    }

    #[test]
    fn 하나_죽은_것과_내가_끊긴_것을_가른다() {
        // ★★처방이 반대다 — 앞은 그 node 의 방이 죽은 것이고, 뒤는 ★**내 방들이 멀쩡한데
        //   내가 못 보는 것**이다. 합치면 고립된 hub 가 남의 방을 죽었다고 선언한다.
        let l = Live::default();
        l.put("node-a", "i1");
        l.put("node-b", "i2");
        l.put("node-c", "i3");
        l.judge_isolation("node-a", 1000);
        assert!(!l.isolated());
        // 하나만 사라진다 — 고립이 아니다.
        l.drop_one("node-b", 1100);
        l.judge_isolation("node-a", 1100);
        assert!(!l.isolated());
        // 나머지가 일괄로 사라진다 — 고립이다.
        l.drop_one("node-c", 1200);
        l.judge_isolation("node-a", 1200);
        assert!(l.isolated());
        // 돌아오면 풀린다 — ★한 번 고립되면 영영 503 인 것이 아니다.
        l.put("node-b", "i2");
        l.judge_isolation("node-a", 1300);
        assert!(!l.isolated());
    }

    #[test]
    fn 혼자_뜬_node_는_고립이_아니다() {
        // ★★**한 대 형상은 정상이다** — 그것을 고립으로 읽으면 ★개발기가 영영 `ready` 503 이다.
        //   「일괄로 **사라졌다**」가 고립이지 「처음부터 혼자」가 아니다.
        let l = Live::default();
        l.put("node-a", "i1");
        l.judge_isolation("node-a", 1000);
        assert!(!l.isolated(), "★한 대 형상을 고립으로 읽으면 개발기가 영영 503 이다");
    }
}
