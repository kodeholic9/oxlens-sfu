// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-BUS-REQUEST 정:RULE-BUS-SHAPE · model: claude-opus-5-5
//! B 평면 요청 — 로컬이면 gRPC Handle, 원격이면 `q/node/{node}/call`. 보내기 전에 `node` 토큰으로 생존을 선판정한다.

use std::time::Duration;

use oxsig::Code;
use oxsig::bplane::{NodeReq, NodeRes, UnitReq, UnitRes};

use crate::state::Hub;

pub const CALL_TIMEOUT: Duration = Duration::from_millis(10_000);

pub async fn unit(hub: &Hub, node: &str, req: UnitReq) -> Result<UnitRes, Code> {
    if node == hub.node {
        return hub.units.sfu_call(&req, CALL_TIMEOUT).await;
    }
    if !hub.node_live(node) {
        return Err(Code::SFU_UNAVAILABLE);
    }
    match node_call(hub, node, &NodeReq::Unit { req }).await? {
        NodeRes::Unit(r) => Ok(r),
        NodeRes::Fail { code, .. } => Err(Code(code)),
        _ => Err(Code::SFU_ERROR),
    }
}

pub async fn node_call(hub: &Hub, node: &str, req: &NodeReq) -> Result<NodeRes, Code> {
    let bus = hub.bus().ok_or(Code::SFU_UNAVAILABLE)?;
    let key = format!("{}/call", hub.keys.q_node(node));
    let replies = common::bus::query(bus, &key, oxsig::bplane::encode(req), CALL_TIMEOUT).await;
    for r in replies {
        match r {
            Ok(b) => return oxsig::bplane::decode::<NodeRes>(&b).map_err(|_| Code::SFU_ERROR),
            Err(e) => tracing::warn!(kind = "diag", node = node, "node call error: {e}"),
        }
    }
    Err(Code::SFU_ERROR)
}

pub async fn fanout(hub: &Hub, req: UnitReq) -> Vec<(String, Result<UnitRes, Code>)> {
    let mut nodes: Vec<String> = hub.nodes.read().map(|n| n.keys().cloned().collect()).unwrap_or_default();
    if !nodes.contains(&hub.node) {
        nodes.push(hub.node.clone());
    }
    let mut out = Vec::new();
    let mut tasks = Vec::new();
    for n in nodes {
        let req = req.clone();
        tasks.push(async move {
            let r = unit(hub, &n, req).await;
            (n, r)
        });
    }
    for t in tasks {
        out.push(t.await);
    }
    out
}

pub fn fail_frame(op: u16, pid: u32, f: &oxsig::wire::Failure) -> oxsig::Frame {
    oxsig::Frame::fail(op, pid, serde_json::to_vec(f).unwrap_or_default())
}

pub fn res_frame(op: u16, pid: u32, res: Result<UnitRes, Code>) -> oxsig::Frame {
    match res {
        Ok(UnitRes::Frame { ok: true, body }) => oxsig::Frame::ok(op, pid, body.into_bytes()),
        Ok(UnitRes::Frame { ok: false, body }) => oxsig::Frame::fail(op, pid, body.into_bytes()),
        Ok(UnitRes::Fail { code, message, details }) => {
            let f = match details {
                Some(d) => Code(code).failure_details(message, d),
                None => Code(code).failure_msg(message),
            };
            fail_frame(op, pid, &f)
        }
        Ok(_) => fail_frame(op, pid, &Code::SFU_ERROR.failure()),
        Err(c) => fail_frame(op, pid, &c.failure()),
    }
}
