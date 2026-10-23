// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-AUX-TASK · 연:AUX.op.TASK · 연:RULE-AUX-TASK-PROC · 연:RULE-AUX-TASK-TYPES · model: claude-opus-5-5
//! AUX 영역(hub 쪽) — `TASK` 의 갈래 판정. 클라 `report` 는 받고 진단은 미디어 서버를 거치지 않는다.

use oxsig::aux::TASK_TYPES;
use oxsig::err::{ErrCode, Failure};
use serde_json::Value;

pub fn task(body: &Value) -> Result<Option<Value>, Failure> {
    let ty = body.get("type").and_then(Value::as_str).unwrap_or("");
    if !TASK_TYPES.contains(&ty) {
        return Err(Failure::new(ErrCode::InvalidPayload).msg(format!("type {ty}")));
    }
    match body.get("phase").and_then(Value::as_str) {
        Some("report") => {
            let req_id = body.get("req_id").and_then(|v| v.as_u64()).unwrap_or(0);
            tracing::info!(kind = "ev", spec = "CP-OP-TASK", req_id = req_id, "task report");
            Ok(None)
        }
        Some("request") => Ok(None),
        _ => Err(Failure::new(ErrCode::InvalidPayload).msg("phase")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_type_is_answered_not_dropped() {
        let r = task(&json!({"phase":"report","req_id":1,"type":"bogus","result":{}}));
        assert_eq!(r.map_err(|f| f.code), Err(1002));
        assert_eq!(task(&json!({"phase":"report","req_id":1,"type":"probe","result":{}})), Ok(None));
    }
}
