// author: kodeholic (powered by Claude)
//! 미디어 서버 노드 표 — 정§18-1 유닛 목록에서(자기 등록·discovery 없음, hub→sfud dial). 연결은 게으르게 맺고 실패하면 다음 요청이 다시 시도한다.

use std::sync::Arc;

use common::bplane::SfuServiceClient;
use dashmap::DashMap;
use tokio::sync::Mutex;
use tonic::transport::Channel;

pub type Client = SfuServiceClient<Channel>;

pub struct Node {
    pub id: String,
    pub addr: String,
    client: Mutex<Option<Client>>,
}

pub struct NodeTable {
    nodes: Vec<Arc<Node>>,
    by_id: DashMap<String, Arc<Node>>,
}

impl NodeTable {
    /// `(node_id, addr)` 목록 — role=sfu 유닛. 순서는 배치와 무관하다(HRW).
    pub fn new(units: impl IntoIterator<Item = (String, String)>) -> Self {
        let nodes: Vec<Arc<Node>> = units.into_iter().map(|(id, addr)| Arc::new(Node { id, addr, client: Mutex::new(None) })).collect();
        let by_id = DashMap::new();
        for n in &nodes {
            by_id.insert(n.id.clone(), n.clone());
        }
        Self { nodes, by_id }
    }

    pub fn ids(&self) -> Vec<&str> {
        self.nodes.iter().map(|n| n.id.as_str()).collect()
    }

    pub fn all(&self) -> &[Arc<Node>] {
        &self.nodes
    }

    pub fn get(&self, id: &str) -> Option<Arc<Node>> {
        self.by_id.get(id).map(|n| n.clone())
    }
}

impl Node {
    /// 살아 있는 클라이언트를 돌려준다. 못 붙으면 `None` — 호출자가 `5001` 로 답한다.
    pub async fn client(&self) -> Option<Client> {
        let mut slot = self.client.lock().await;
        if slot.is_none() {
            let endpoint = format!("http://{}", self.addr);
            *slot = Client::connect(endpoint).await.ok();
        }
        slot.clone()
    }

    /// 요청이 실패했다 — 다음 요청이 다시 붙게 한다.
    pub async fn drop_client(&self) {
        *self.client.lock().await = None;
    }
}
