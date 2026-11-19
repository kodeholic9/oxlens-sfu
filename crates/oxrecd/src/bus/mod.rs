// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:IMPL-FLR-TALK-REPORT · 정:IMPL-OPS-REC-UNIT · model: claude-opus-5-5
//! 녹음 유닛의 버스 — `common::bus` client(`{node}.rec` · 로컬 hub 에만) · 그 node 의 방 토큰 history 구독 · 로컬 hub 에 묻기(`q/node/{node}/talk` · 시한 `common::talk::ASK_TIMEOUT`).

use common::bus::{Client, TokenSub};
use common::keyspace::Keys;

#[derive(Clone)]
pub struct Bus {
    pub keys: Keys,
    pub node: String,
    session: Client,
}

impl Bus {
    pub async fn open(node: &str, ns: &str, connect: &str) -> Result<Bus, String> {
        let session = Client::open(node, "rec", connect).await?;
        Ok(Bus { keys: Keys::new(ns), node: node.to_string(), session })
    }

    pub async fn ask(&self, what: &str, payload: Vec<u8>) -> Option<Vec<u8>> {
        let key = self.keys.q_node(&self.node, what);
        self.session.ask(&key, payload, common::talk::ASK_TIMEOUT).await
    }

    pub async fn rooms(&self) -> Result<TokenSub, String> {
        self.session.token_sub(&self.keys.rooms_of_node(&self.node), true).await
    }
}
