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
    /// ★★**보내는 큐가 하나다** — put 마다 태스크를 띄우면 두 통지가 서로를 앞질러
    /// ★**`version.seq` 가 역전된다**(클라는 뒤엣것을 버린다, 정§15-4 우선순위 줄).
    tx: tokio::sync::mpsc::Sender<(String, Vec<u8>)>,
}

/// 버스를 연다. ★**못 열면 `Err`** — 조용히 없는 채로 돌면 통지가 아무 데도 안 간다.
pub async fn open(connect: &str, namespace: &str) -> Result<Bus, String> {
    if !common::bus::is_key_segment(namespace) {
        return Err(format!("namespace 가 키 조각이 못 된다: {namespace:?}"));
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
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(String, Vec<u8>)>(QUEUE);
    tokio::spawn(async move {
        while let Some((key, body)) = rx.recv().await {
            // ★★**`Drop` 이다**(정§15-4 혼잡 정책). `Block` 이면 큐가 밀릴 때 메시지가
            //   아니라 ★**전송 세션이 닫히고, 그 세션이 선언한 토큰이 전부 사라진다.**
            //   ★**이벤트 하나를 지키려고 노드 전체를 잃지 않는다.**
            let _ = session.put(key, body).congestion_control(CongestionControl::Drop).await;
        }
    });
    Ok(Bus { keys: Keys::new(namespace), tx })
}

impl Bus {
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
