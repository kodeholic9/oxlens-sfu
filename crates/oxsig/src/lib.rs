// author: kodeholic (powered by Claude)
//! OxLens 연동규격서 wire 계층(`client_ver=1`) — 프레임·op·실패/Close 코드·공통 스키마·op body·DC·MBCP·규격 상수.
//! 서버(hub·sfud)와 적합성 봇이 같은 계약을 여기서 가져간다. 공유 벡터의 정본은 `oxlens-spec/vectors/`,
//! 이 크레이트의 `vectors/` 는 그 사본이다(`OXLENS_SPEC_DIR` 가 있으면 시험이 어긋남을 잡는다).

pub mod body;
pub mod close;
pub mod code;
pub mod dc;
pub mod frame;
pub mod mbcp;
pub mod op;
pub mod schema;
pub mod timers;

pub use close::CloseCode;
pub use code::{Class, FailCode, Failure};
pub use frame::{Header, Kind};
pub use op::Op;
pub use schema::{Affiliation, MemberInfo, ServerConfig, TrackEntry, Version};

#[cfg(test)]
mod vector_tests {
    //! 공유 벡터 — 적합성 봇 시험과 같은 파일을 읽는다.
    use super::*;
    use serde_json::Value;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn drift_guard(name: &str, vendored: &str) {
        if let Ok(dir) = std::env::var("OXLENS_SPEC_DIR") {
            let canonical = std::fs::read_to_string(format!("{dir}/vectors/{name}")).unwrap();
            assert_eq!(canonical, vendored, "vectors/{name} drifted from oxlens-spec");
        }
    }

    #[test]
    fn frame_vectors() {
        let raw = include_str!("../vectors/frame.json");
        drift_guard("frame.json", raw);
        let v: Value = serde_json::from_str(raw).unwrap();
        for case in v["cases"].as_array().unwrap() {
            let bytes = hex(case["hex"].as_str().unwrap());
            let name = case["name"].as_str().unwrap();
            match frame::decode(&bytes) {
                Ok((h, body)) => {
                    let exp = &case["expect"];
                    assert_eq!(h.kind, match exp["kind"].as_str().unwrap() { "msg" => Kind::Msg, "ok" => Kind::Ok, _ => Kind::Fail }, "{name}");
                    assert_eq!(u64::from(h.op), exp["op"].as_u64().unwrap(), "{name}");
                    assert_eq!(u64::from(h.pid), exp["pid"].as_u64().unwrap(), "{name}");
                    assert_eq!(u64::from(h.reserved), exp["reserved"].as_u64().unwrap_or(0), "{name}");
                    assert_eq!(frame::body_json(body).unwrap(), exp["body"], "{name}");
                    assert_eq!(frame::encode_json(&h, &exp["body"]), bytes, "{name} re-encode");
                }
                Err(_) => assert_eq!(case["expect"]["error"].as_bool(), Some(true), "{name}"),
            }
        }
    }

    #[test]
    fn mbcp_vectors() {
        let raw = include_str!("../vectors/mbcp.json");
        drift_guard("mbcp.json", raw);
        let v: Value = serde_json::from_str(raw).unwrap();
        for case in v["cases"].as_array().unwrap() {
            let bytes = hex(case["hex"].as_str().unwrap());
            let name = case["name"].as_str().unwrap();
            let exp = &case["expect"];
            match mbcp::Msg::decode(&bytes) {
                None => assert_eq!(exp["drop"].as_bool(), Some(true), "{name}"),
                Some(m) => {
                    assert_eq!(u64::from(m.msg_type.code()), exp["type"].as_u64().unwrap(), "{name}");
                    assert_eq!(m.ack_req, exp["ack"].as_bool().unwrap(), "{name}");
                    let tlvs = exp["tlvs"].as_array().unwrap();
                    assert_eq!(m.tlvs.len(), tlvs.len(), "{name} tlv count");
                    for (t, e) in m.tlvs.iter().zip(tlvs) {
                        assert_eq!(u64::from(t.id), e["id"].as_u64().unwrap(), "{name}");
                        assert_eq!(t.value, hex(e["hex"].as_str().unwrap()), "{name}");
                    }
                    if exp["reencode"].as_bool().unwrap_or(true) {
                        assert_eq!(m.encode().unwrap(), bytes, "{name} re-encode");
                    }
                }
            }
        }
    }
}
