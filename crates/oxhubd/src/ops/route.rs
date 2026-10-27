// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-REQUEST · 정:RULE-BUS-SHAPE · 정:CAND-BUS-HANDLE-SHAPE · 정:CAND-BUS-REQUEST-TIMEOUT · 정:IMPL-ERR-ISSUERS · model: claude-opus-5-5
//! 요청 넘기기 — 로컬이면 gRPC Handle · 원격이면 `q/node/{node}/call` get(보내기 전에 `node` 토큰으로 생존 선판정). sfud 실패 wire 는 그대로 통과한다.

use common::bus::REQUEST_TIMEOUT_MS;
use common::bus::env::{Call, Reply};
use oxsig::code::Code;

use super::Hub;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub node: String,
    pub inst: Option<String>,
}

pub async fn call(hub: &Hub, t: &Target, c: &Call) -> Reply {
    if t.node == hub.me() {
        return local(hub, t.inst.as_deref(), c).await;
    }
    if !hub.node_live(&t.node) {
        return Reply::fail(Code::SFU_UNAVAILABLE);
    }
    let Some(bus) = hub.bus() else { return Reply::fail(Code::SFU_UNAVAILABLE) };
    let key = hub.ks.q_node(&t.node, "call");
    match bus.get_one(&key, c.encode(), REQUEST_TIMEOUT_MS).await {
        Ok(b) => Reply::decode(&b).unwrap_or_else(|_| Reply::fail(Code::SFU_ERROR)),
        Err(_) => Reply::fail(Code::SFU_ERROR),
    }
}

pub async fn local(hub: &Hub, inst: Option<&str>, c: &Call) -> Reply {
    let Some(mut client) = hub.sup.client_for(inst).await else { return Reply::fail(Code::SFU_UNAVAILABLE) };
    let req = common::grpc::CallReq { envelope: c.encode() };
    let fut = client.call(req);
    match tokio::time::timeout(std::time::Duration::from_millis(REQUEST_TIMEOUT_MS), fut).await {
        Ok(Ok(res)) => Reply::decode(&res.into_inner().reply).unwrap_or_else(|_| Reply::fail(Code::SFU_ERROR)),
        Ok(Err(_)) | Err(_) => Reply::fail(Code::SFU_ERROR),
    }
}

pub fn room_target(hub: &Hub, room: &str) -> Option<Target> {
    hub.rooms.lock().ok().and_then(|m| m.locate(room)).map(|l| Target { node: l.node, inst: Some(l.inst) })
}

pub fn sfu_target(hub: &Hub, sfu_id: &str) -> Option<Target> {
    hub.node_of_sfu(sfu_id).map(|node| Target { node, inst: Some(sfu_id.to_string()) })
}
