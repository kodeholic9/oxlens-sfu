// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§15-0 · §15-4 · model: claude-opus-5

//! sfud 쪽 버스 — ★**client 하나다**(정§15-0). router 는 제 node 의 hub 가 든다.
//!
//! ★★**이벤트는 sfud 가 직접 낸다**(정§15-4) — hub 가 재발행하지 않는다.
//! 재발행을 두면 ★**재발행 계층과 사망 시 명시 철회 로직**이 통째로 따라붙는다.
//!
//! ★★**로컬 멤버에게도 구독으로만 간다.** 같은 node 라고 hub 에 직접 건네고 남에게는
//! put 하면, ★**같은 통지가 두 번 가는 창**이 생긴다(정§15-4 증상 첫 줄).

use common::bus::Keys;
use zenoh::qos::CongestionControl;

/// ★**밀린 통지의 상한** — 넘으면 버린다(정§15-4 `Drop`). 갭은 `seq` 가 드러내고
/// §14-3·§14-4 가 메운다.
const QUEUE: usize = 1024;

/// 이 sfud 가 쥔 버스 한 자리.
pub struct Bus {
    pub keys: Keys,
    /// ★**배치 키**(정§15-3) — 안정값이라 재기동해도 같다.
    pub node_id: String,
    /// ★**기동 신원**(`epoch`=`sfu_id`=`{inst}`) — 기동마다 새 값이라 재기동을 가른다.
    pub inst: String,
    /// ★★**보내는 큐가 하나다** — put 마다 태스크를 띄우면 두 통지가 서로를 앞질러
    /// ★**`version.seq` 가 역전된다**(클라는 뒤엣것을 버린다, 정§15-4 우선순위 줄).
    tx: tokio::sync::mpsc::Sender<(String, Vec<u8>)>,
    session: zenoh::Session,
}

/// ★**떨어뜨리면 그 방의 자리가 꺼진다** — 그래서 방이 사는 동안 들고 있는다.
pub struct RoomToken(#[allow(dead_code)] zenoh::liveliness::LivelinessToken);

impl std::fmt::Debug for RoomToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RoomToken")
    }
}

/// 버스를 연다. ★**못 열면 `Err`** — 조용히 없는 채로 돌면 통지가 아무 데도 안 간다.
pub async fn open(
    connect: &str,
    namespace: &str,
    node_id: &str,
    inst: &str,
) -> Result<Bus, String> {
    // ★**키를 깨는 값은 여기서 막는다** — 붙고 나서 아무에게도 안 보이는 것이 제일 나쁘다.
    for (what, v) in [("namespace", namespace), ("node_id", node_id), ("inst", inst)] {
        if !common::bus::is_key_segment(v) {
            return Err(format!("{what} 가 키 조각이 못 된다: {v:?}"));
        }
    }
    let mut cfg = zenoh::Config::default();
    // ★**client 다** — peer 로 열면 관문이 무너진다(정§15-0).
    for (path, v) in [
        ("mode", "\"client\"".to_string()),
        ("connect/endpoints", format!("[\"{connect}\"]")),
        ("scouting/multicast/enabled", "false".into()),
        ("transport/unicast/qos/enabled", "true".into()),
        ("transport/unicast/lowlatency", "false".into()),
    ] {
        cfg.insert_json5(path, &v).map_err(|e| format!("zenoh 설정 {path}: {e}"))?;
    }
    let session = zenoh::open(cfg).await.map_err(|e| format!("zenoh open: {e}"))?;
    let session2 = session.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(String, Vec<u8>)>(QUEUE);
    tokio::spawn(async move {
        while let Some((key, body)) = rx.recv().await {
            // ★★**`Drop` 이다**(정§15-4 혼잡 정책). `Block` 이면 큐가 밀릴 때 메시지가
            //   아니라 ★**전송 세션이 닫히고, 그 세션이 선언한 토큰이 전부 사라진다.**
            //   ★**이벤트 하나를 지키려고 노드 전체를 잃지 않는다.**
            let _ = session.put(key, body).congestion_control(CongestionControl::Drop).await;
        }
    });
    Ok(Bus {
        keys: Keys::new(namespace),
        node_id: node_id.to_string(),
        inst: inst.to_string(),
        tx,
        session: session2,
    })
}

impl Bus {
    /// 이 sfud 가 쥔 방을 버스에 세운다 — ★★**토큰이 곧 「그 방은 여기 있다」**(정§15-2).
    ///
    /// ★**선언 주체 = 그 상태의 소유자**다. 통보가 아니라 선언이라 ★**주체가 죽으면
    /// 아무도 지우지 않아도 키가 사라진다** — 종전 브로드캐스트 방식의 약점
    /// (*"소멸 통보를 빠뜨리면 죽은 방이 노드를 영원히 문다"*)이 여기엔 없다.
    pub async fn declare_room(&self, room: &str) -> Option<RoomToken> {
        // ★키를 깨는 방 이름은 선언하지 않는다 — 선언해도 아무에게도 안 보인다.
        if !common::bus::is_key_segment(room) {
            eprintln!("[bus] ★방 이름이 키 조각이 못 된다 — 위치를 못 세운다: {room:?}");
            return None;
        }
        self.session
            .liveliness()
            .declare_token(self.keys.room(room, &self.node_id, &self.inst))
            .await
            .ok()
            .map(RoomToken)
    }

    /// ★**낸 순서 그대로 줄에 세운다** — 순서가 `version.seq` 의 뜻이다.
    ///
    /// ★**줄이 차면 버린다**(막지 않는다) — 여기서 기다리면 통지 하나가 dispatch 를 세운다.
    pub fn put(&self, key: String, body: Vec<u8>) {
        if self.tx.try_send((key, body)).is_err() {
            eprintln!("[bus] ★통지 줄이 찼다 — 한 건 버렸다(seq 갭으로 드러난다)");
        }
    }

}

impl std::fmt::Debug for Bus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Bus")
    }
}
