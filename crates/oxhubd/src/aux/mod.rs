// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-AUX-TASK · 정:RULE-OPS-OBSERVE · 연:AUX.op.TASK · 연:RULE-AUX-TASK-PROC · 연:RULE-AUX-TASK-TYPES · 연:RULE-AUX-PROBE-RESULT · 연:RULE-AUX-PROBE-NO-INVENT · 정:CAND-AUX-TASK-CLIENT-REQUEST · model: claude-opus-5-5
//! 보조 — `TASK` 는 게이트웨이에서 끝난다(진단 수집은 미디어 서버를 거치지 않는다). 보고는 형(ProbeResult)으로 읽고, 모르는 `type` 은 실패로 답한다.

use oxsig::{Frame, Kind, TaskReq};

pub fn task(f: &Frame, session_fp: &str) -> Vec<u8> {
    match oxsig::parse::body::<TaskReq>(&f.body) {
        Err(e) => f.reply(Kind::Fail, oxsig::parse::encode(&e.code().failure_with(e.message(), None))).encode(),
        Ok(TaskReq::Report { req_id, result, .. }) => {
            let fields = [result.pub_tracks.is_some(), result.sub_tracks.is_some(), result.env.is_some(), result.network.is_some()].iter().filter(|x| **x).count();
            tracing::info!(kind = "ev", spec = "CP-OP-TASK", session = %session_fp, req_id, error = result.error.as_deref().unwrap_or(""), sections = fields, "probe report");
            f.reply(Kind::Ok, Vec::new()).encode()
        }
        Ok(TaskReq::Request { req_id, .. }) => {
            tracing::info!(kind = "diag", session = %session_fp, req_id, "client-issued TASK request — nothing to collect on the server");
            f.reply(Kind::Ok, Vec::new()).encode()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxsig::Code;

    fn frame(body: &str) -> Frame {
        Frame::new(Kind::Request, oxsig::Op::Task.as_u16(), 3, body.as_bytes().to_vec())
    }

    #[test]
    fn task_report_and_failures() {
        let ok = Frame::decode(&task(&frame(r#"{"phase":"report","req_id":1,"type":"probe","result":{}}"#), "fp")).expect("dec");
        assert_eq!(ok.kind(), Kind::Ok);
        let unk = Frame::decode(&task(&frame(r#"{"phase":"report","req_id":1,"type":"nope","result":{}}"#), "fp")).expect("dec");
        assert_eq!(unk.kind(), Kind::Fail);
        let v: serde_json::Value = serde_json::from_slice(&unk.body).expect("json");
        assert_eq!(v["code"], u64::from(Code::InvalidPayload.number()));
        let miss = Frame::decode(&task(&frame(r#"{"phase":"report","type":"probe","result":{}}"#), "fp")).expect("dec");
        let v: serde_json::Value = serde_json::from_slice(&miss.body).expect("json");
        assert_eq!(v["code"], 1003);
    }
}
