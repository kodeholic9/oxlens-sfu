// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · 정:RULE-MESH-SHAPE · 정:RULE-MESH-SESSION · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-REQUEST · model: claude-opus-5-5
//! 유닛(sfud · 녹음 유닛)이 함께 쓰는 버스 client — 로컬 hub 의 oxmesh router 에만 붙는다(이름 `{node}.{role}`) · 묻기(첫 답 하나 · 시한) · put · 토큰 선언 · 토큰 구독(history) · 토큰 목록 · 닫기 · 버린 수 · oxmesh 형은 밖에 내지 않는다(버스를 바꿀 때 손댈 곳은 이 파일과 hub 의 `bus/` 둘).

use std::time::Duration;

#[derive(Clone)]
pub struct Client {
    mesh: oxmesh::Mesh,
}

pub struct Token(oxmesh::Token);

impl Token {
    pub fn key(&self) -> &str {
        self.0.key()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenEvent {
    Put(String),
    Delete(String),
    BatchEnd,
}

pub struct TokenSub(oxmesh::TokenSub);

impl TokenSub {
    pub async fn recv(&mut self) -> Option<TokenEvent> {
        self.0.recv().await.map(|e| match e {
            oxmesh::TokenEvent::Put(k) => TokenEvent::Put(k),
            oxmesh::TokenEvent::Delete(k) => TokenEvent::Delete(k),
            oxmesh::TokenEvent::BatchEnd => TokenEvent::BatchEnd,
        })
    }
}

impl Client {
    pub async fn open(node: &str, role: &str, connect: &str) -> Result<Client, String> {
        let mesh = oxmesh::Mesh::open(oxmesh::Config::client(&format!("{node}.{role}"), oxmesh::endpoint(connect))).await.map_err(|e| e.to_string())?;
        Ok(Client { mesh })
    }

    pub fn name(&self) -> &str {
        self.mesh.node()
    }

    pub async fn ask(&self, key: &str, payload: Vec<u8>, timeout: Duration) -> Option<Vec<u8>> {
        let mut replies = self.mesh.get(key, payload, timeout).ok()?;
        replies.recv().await.and_then(|r| r.result.ok())
    }

    pub fn put(&self, key: &str, payload: Vec<u8>) -> Result<(), String> {
        self.mesh.put(key, payload).map_err(|e| e.to_string())
    }

    pub async fn declare_token(&self, key: &str) -> Result<Token, String> {
        self.mesh.declare_token(key).await.map(Token).map_err(|e| e.to_string())
    }

    pub async fn token_sub(&self, expr: &str, history: bool) -> Result<TokenSub, String> {
        self.mesh.token_sub(expr, history).await.map(TokenSub).map_err(|e| e.to_string())
    }

    pub async fn tokens(&self, expr: &str, timeout: Duration) -> Result<Vec<String>, String> {
        self.mesh.tokens(expr, timeout).await.map_err(|e| e.to_string())
    }

    pub fn dropped(&self) -> u64 {
        self.mesh.dropped()
    }

    pub async fn close(&self) {
        self.mesh.close().await;
    }
}
