// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:RULE-BUS-REQUEST · 정:RULE-BUS-PUBLIC · 정:CAND-BUS-REQUEST-TIMEOUT · 운영:RULE-READ-BUS · model: claude-opus-5-5
//! hub 의 버스 — zenoh router(임베드). 존재는 선언(liveliness) · `ev` put 은 `Drop` · node 사이 요청은 queryable · 답은 답하는 node 의 제 키로.

use std::time::Duration;

use common::bus::keys::KeySpace;
use common::system::Zenoh;
use zenoh::bytes::ZBytes;
use zenoh::handlers::FifoChannelHandler;
use zenoh::liveliness::LivelinessToken;
use zenoh::pubsub::Subscriber;
use zenoh::qos::{CongestionControl, Priority};
use zenoh::query::{ConsolidationMode, QueryTarget, Queryable};
use zenoh::sample::Sample;

pub struct Bus {
    pub ks: KeySpace,
    pub z: zenoh::Session,
    pub me: String,
    pub inst: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Peer {
    pub zid: String,
    pub whatami: String,
    pub links: Vec<String>,
}

#[derive(Debug)]
pub enum GetErr {
    Timeout,
    Failed,
}

impl Bus {
    pub async fn open(cfg: &Zenoh, me: &str, inst: &str) -> Result<Bus, String> {
        let zc = common::bus::zcfg::router(cfg)?;
        let z = zenoh::open(zc).await.map_err(|e| format!("zenoh open: {e}"))?;
        Ok(Bus { ks: KeySpace::new(&cfg.namespace), z, me: me.to_string(), inst: inst.to_string() })
    }

    pub async fn declare(&self, key: &str) -> Result<LivelinessToken, String> {
        self.z.liveliness().declare_token(key).await.map_err(|e| format!("token {key}: {e}"))
    }

    pub async fn declare_node(&self) -> Result<LivelinessToken, String> {
        self.declare(&self.ks.node(&self.me, &self.inst)).await
    }

    pub async fn declare_sess(&self, user: &str, sid: &str) -> Result<LivelinessToken, String> {
        self.declare(&self.ks.sess(user, &self.me, sid)).await
    }

    pub async fn watch(&self, key: &str) -> Result<Subscriber<FifoChannelHandler<Sample>>, String> {
        self.z.liveliness().declare_subscriber(key).history(true).await.map_err(|e| format!("watch {key}: {e}"))
    }

    pub async fn subscribe(&self, key: &str) -> Result<Subscriber<FifoChannelHandler<Sample>>, String> {
        self.z.declare_subscriber(key).await.map_err(|e| format!("subscribe {key}: {e}"))
    }

    pub async fn put(&self, key: &str, bytes: Vec<u8>) -> bool {
        self.z.put(key, ZBytes::from(bytes)).congestion_control(CongestionControl::Drop).priority(Priority::Data).await.is_ok()
    }

    pub async fn queryable(&self) -> Result<Queryable<FifoChannelHandler<zenoh::query::Query>>, String> {
        let key = self.ks.q_node_queryable(&self.me);
        self.z.declare_queryable(&key).await.map_err(|e| format!("queryable {key}: {e}"))
    }

    pub async fn tokens(&self, key: &str, timeout_ms: u64) -> Vec<String> {
        let Ok(rx) = self.z.liveliness().get(key).timeout(Duration::from_millis(timeout_ms)).await else { return Vec::new() };
        let mut out = Vec::new();
        while let Ok(r) = rx.recv_async().await {
            if let Ok(s) = r.result() {
                out.push(s.key_expr().as_str().to_string());
            }
        }
        out
    }

    pub async fn get_one(&self, key: &str, payload: Vec<u8>, timeout_ms: u64) -> Result<Vec<u8>, GetErr> {
        let rx = self
            .z
            .get(key)
            .payload(ZBytes::from(payload))
            .timeout(Duration::from_millis(timeout_ms))
            .await
            .map_err(|_| GetErr::Failed)?;
        match rx.recv_async().await {
            Ok(r) => match r.result() {
                Ok(s) => Ok(s.payload().to_bytes().to_vec()),
                Err(_) => Err(GetErr::Failed),
            },
            Err(_) => Err(GetErr::Timeout),
        }
    }

    pub async fn get_all(&self, key: &str, payload: Vec<u8>, timeout_ms: u64) -> Vec<(String, Result<Vec<u8>, String>)> {
        let Ok(rx) = self
            .z
            .get(key)
            .payload(ZBytes::from(payload))
            .target(QueryTarget::All)
            .consolidation(ConsolidationMode::None)
            .timeout(Duration::from_millis(timeout_ms))
            .await
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        while let Ok(r) = rx.recv_async().await {
            match r.result() {
                Ok(s) => {
                    let node = self.ks.parse_q_node(s.key_expr().as_str()).map(|x| x.0).unwrap_or_default();
                    out.push((node, Ok(s.payload().to_bytes().to_vec())));
                }
                Err(e) => out.push((String::new(), Err(String::from_utf8_lossy(&e.payload().to_bytes()).to_string()))),
            }
        }
        out
    }

    pub async fn peers(&self) -> Vec<Peer> {
        let transports: Vec<_> = self.z.info().transports().await.collect();
        let links: Vec<_> = self.z.info().links().await.collect();
        transports
            .into_iter()
            .map(|t| {
                let zid = t.zid().to_string();
                let l = links.iter().filter(|l| l.zid().to_string() == zid).map(|l| format!("{} -> {}", l.src(), l.dst())).collect();
                Peer { zid, whatami: t.whatami().to_string(), links: l }
            })
            .collect()
    }
}
