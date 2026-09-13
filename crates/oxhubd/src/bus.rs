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
}

impl Live {
    pub fn snapshot(&self) -> Arc<BTreeMap<String, String>> {
        self.nodes.load_full()
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
    }

    fn drop_one(&self, node: &str) {
        let mut next = BTreeMap::clone(&self.nodes.load());
        next.remove(node);
        self.nodes.store(Arc::new(next));
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
pub async fn open(z: &Zenoh, node_id: &str, inst: &str) -> Result<Bus, String> {
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
    let token = session
        .liveliness()
        .declare_token(keys.node(node_id, inst))
        .await
        .map_err(|e| format!("liveliness 선언: {e}"))?;

    let seen = live.clone();
    tokio::spawn(async move {
        while let Ok(s) = sub.recv_async().await {
            let key = s.key_expr().as_str().to_string();
            let Some((node, _inst)) = bus::node_of(&key) else { continue };
            match s.kind() {
                zenoh::sample::SampleKind::Put => {
                    eprintln!("[bus] node 등장 {node}");
                    seen.put(node, _inst);
                    seen.mark("put", &key);
                }
                // ★**Delete 는 라우팅 표를 비우는 데까지다** — 재배치를 촉발하지 않는다(정§15-7).
                zenoh::sample::SampleKind::Delete => {
                    eprintln!("[bus] node 소멸 {node}");
                    seen.drop_one(node);
                    seen.mark("delete", &key);
                }
            }
        }
        eprintln!("[bus] ★liveliness 구독이 끝났다 — 이 node 는 이제 남을 못 본다");
    });

    Ok(Bus {
        node_id: node_id.to_string(),
        inst: inst.to_string(),
        keys,
        live,
        _token: token,
        session,
    })
}
