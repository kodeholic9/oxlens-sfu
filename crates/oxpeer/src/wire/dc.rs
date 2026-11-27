// author: kodeholic (powered by Claude)
//! DC 프레임(연:TRN.frame.DC) — `ver(1)=0x01 · svc(1) · len(u16 BE) · payload` · 발언권은 svc 0x01(MBCP).
//! MBCP 머리 · TLV 는 생성 코덱(generated/floor_codec)이, TLV ↔ 받은 · 보낼 메시지는 생성 상태기(generated/floor_client)가 맡는다 — 여기는 DC 머리만.
//! 잘린 프레임 · 다른 svc · 못 읽는 메시지는 조용히 버린다(연:TRN.frame.DC 잘린 프레임).

use crate::generated::floor_client::{Ack, Rx, Tx, ack_to_tlv, rx_from_tlv, tx_to_tlv};
use crate::generated::floor_codec::{Msg, decode, encode};

pub(crate) const VER: u8 = 0x01;
pub(crate) const SVC_FLOOR: u8 = 0x01;
/// payload 상한 — 서버 `a=max-message-size` 65536 − 머리 4(연:TRN.frame.DC)
pub(crate) const MAX_PAYLOAD: usize = 65_532;
/// 채널 이름 — 하나뿐이다(다른 이름은 서버가 거부한다)
pub(crate) const CHANNEL_LABEL: &str = "unreliable";

pub(crate) fn wrap(payload: &[u8], svc: u8) -> Option<Vec<u8>> {
    if payload.len() > MAX_PAYLOAD {
        return None;
    }
    let mut out = Vec::with_capacity(4 + payload.len());
    out.push(VER);
    out.push(svc);
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

pub(crate) fn unwrap(buf: &[u8]) -> Option<&[u8]> {
    if buf.len() < 4 || buf[0] != VER || buf[1] != SVC_FLOOR {
        return None;
    }
    let len = usize::from(u16::from_be_bytes([buf[2], buf[3]]));
    buf.get(4..4 + len)
}

/// 받은 DC 바이트 → (방, 받은 메시지)
pub(crate) fn parse(buf: &[u8]) -> Option<(String, Rx)> {
    let m = decode(unwrap(buf)?)?;
    rx_from_tlv(m.msg_type, m.ack, &m.tlvs)
}

fn frame(t: (u8, bool, Vec<(u8, Vec<u8>)>)) -> Option<Vec<u8>> {
    let (msg_type, ack, tlvs) = t;
    wrap(&encode(&Msg { msg_type, ack, tlvs })?, SVC_FLOOR)
}

/// 상태기가 낸 보낼 메시지 → DC 바이트
pub(crate) fn tx(t: &Tx, room: &str) -> Option<Vec<u8>> {
    frame(tx_to_tlv(t, room))
}

/// 상태기가 낸 ACK → DC 바이트
pub(crate) fn ack(a: &Ack, room: &str) -> Option<Vec<u8>> {
    frame(ack_to_tlv(a, room))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::floor_client::{AckOf, RxIdle, RxTaken};
    use serde_json::Value;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// MBCP 머리 · TLV — 규격 벡터(oxlens-spec/vectors/mbcp.json)
    #[test]
    fn spec_vectors_mbcp() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../oxlens-spec/vectors/mbcp.json");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("규격 벡터")).unwrap();
        let cases = v["cases"].as_array().unwrap();
        assert!(!cases.is_empty());
        for c in cases {
            let name = c["name"].as_str().unwrap();
            let got = decode(&unhex(c["hex"].as_str().unwrap()));
            let exp = &c["expect"];
            if exp.get("drop").and_then(Value::as_bool) == Some(true) || exp.get("error").and_then(Value::as_bool) == Some(true) {
                assert!(got.is_none(), "{name}: 버려야 한다");
                continue;
            }
            let m = got.unwrap_or_else(|| panic!("{name}: 읽지 못했다"));
            assert_eq!(u64::from(m.msg_type), exp["type"].as_u64().unwrap(), "{name}");
            assert_eq!(m.ack, exp["ack"].as_bool().unwrap(), "{name}");
            let tlvs: Vec<(u64, String)> = m.tlvs.iter().map(|(i, v)| (u64::from(*i), v.iter().map(|b| format!("{b:02x}")).collect())).collect();
            let want: Vec<(u64, String)> = exp["tlvs"].as_array().unwrap().iter().map(|t| (t["id"].as_u64().unwrap(), t["hex"].as_str().unwrap().to_string())).collect();
            assert_eq!(tlvs, want, "{name}");
        }
    }

    #[test]
    fn taken_idle_round() {
        let taken = wrap(&unhex("020308020007040275321d027231"), SVC_FLOOR).unwrap();
        assert_eq!(parse(&taken), Some(("r1".to_string(), Rx::Taken(RxTaken { ack: false, speaker: "u2".into(), sources: None, seq: 7, talk: None }))));
        let with_src = wrap(&encode(&Msg { msg_type: 2, ack: false, tlvs: vec![(4, b"u2".to_vec()), (8, vec![0, 7]), (29, b"r1".to_vec()), (31, vec![0, 0, 0, 3, 0, 0, 0, 4])] }).unwrap(), SVC_FLOOR).unwrap();
        assert_eq!(parse(&with_src), Some(("r1".to_string(), Rx::Taken(RxTaken { ack: false, speaker: "u2".into(), sources: Some(vec![3, 4]), seq: 7, talk: None }))));
        let idle = wrap(&encode(&Msg { msg_type: 5, ack: true, tlvs: vec![(8, vec![0, 9]), (29, b"r1".to_vec())] }).unwrap(), SVC_FLOOR).unwrap();
        assert_eq!(parse(&idle), Some(("r1".to_string(), Rx::Idle(RxIdle { ack: true, prev: None, seq: 9 }))));
        let ackb = ack(&Ack { of: AckOf::TAKEN }, "r1").unwrap();
        let m = decode(unwrap(&ackb).unwrap()).unwrap();
        assert_eq!((m.msg_type, m.tlvs.clone()), (10, vec![(12, vec![2, 0]), (29, b"r1".to_vec())]));
    }

    #[test]
    fn truncated_and_other_svc_dropped() {
        assert!(parse(&[1, 1, 0, 9, 1, 2]).is_none());
        assert!(parse(&[1, 2, 0, 0]).is_none());
        assert!(wrap(&vec![0u8; MAX_PAYLOAD + 1], SVC_FLOOR).is_none());
    }
}
