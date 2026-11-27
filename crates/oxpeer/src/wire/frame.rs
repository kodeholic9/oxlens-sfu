// author: kodeholic (powered by Claude)
//! WS 프레임(연:TRN.frame.WS) — `ver(1)=0x01 · flags(1) · op(u16 BE) · pid(u32 BE) · body(JSON)`.
//! 빈 body 는 0바이트로 나가고 받을 때는 `{}` 로 돌아온다 · flags bit2-7 은 원본을 지킨다 · 끊는 조건이면 `Err`.

use serde_json::Value;

pub(crate) const VER: u8 = 0x01;
pub(crate) const HEADER_LEN: usize = 8;
/// 프레임 길이 상한 — 넘으면 끊는다(연:TRN.frame.WS)
pub(crate) const MAX_FRAME_LEN: usize = 1_048_576;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// 요청 · 통지(00)
    Msg,
    /// 성공 응답(01)
    Ok,
    /// 실패 응답(10)
    Fail,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Frame {
    pub kind: Kind,
    pub op: u16,
    pub pid: u32,
    pub reserved: u8,
    pub body: Value,
}

fn empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

pub(crate) fn encode(kind: Kind, op: u16, pid: u32, body: &Value) -> Vec<u8> {
    encode_raw(kind, 0, op, pid, body)
}

fn encode_raw(kind: Kind, reserved: u8, op: u16, pid: u32, body: &Value) -> Vec<u8> {
    let bits = match kind {
        Kind::Msg => 0b00,
        Kind::Ok => 0b01,
        Kind::Fail => 0b10,
    };
    let mut out = Vec::with_capacity(HEADER_LEN + 64);
    out.push(VER);
    out.push((reserved & !0b11) | bits);
    out.extend_from_slice(&op.to_be_bytes());
    out.extend_from_slice(&pid.to_be_bytes());
    if !empty(body) {
        out.extend_from_slice(&serde_json::to_vec(body).unwrap_or_default());
    }
    out
}

pub(crate) fn decode(data: &[u8]) -> Result<Frame, String> {
    if data.len() > MAX_FRAME_LEN {
        return Err(format!("프레임 {}B > {MAX_FRAME_LEN}B", data.len()));
    }
    if data.len() < HEADER_LEN {
        return Err(format!("머리보다 짧다({}B)", data.len()));
    }
    if data[0] != VER {
        return Err(format!("ver {:#04x} ≠ 0x01", data[0]));
    }
    let kind = match data[1] & 0b11 {
        0b00 => Kind::Msg,
        0b01 => Kind::Ok,
        0b10 => Kind::Fail,
        _ => return Err("flags 11 은 예약이다".into()),
    };
    let op = u16::from_be_bytes([data[2], data[3]]);
    let pid = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let raw = &data[HEADER_LEN..];
    let body = if raw.is_empty() { Value::Object(Default::default()) } else { serde_json::from_slice(raw).map_err(|_| "body 가 JSON 이 아니다".to_string())? };
    Ok(Frame { kind, op, pid, reserved: data[1] & !0b11, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// 판정 근거는 규격 벡터(oxlens-spec/vectors/frame.json — 웹 SDK 와 같은 것)다
    #[test]
    fn spec_vectors_frame() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../oxlens-spec/vectors/frame.json");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("규격 벡터")).unwrap();
        let cases = v["cases"].as_array().unwrap();
        assert!(!cases.is_empty());
        for c in cases {
            let name = c["name"].as_str().unwrap();
            let bytes = unhex(c["hex"].as_str().unwrap());
            let exp = &c["expect"];
            let got = decode(&bytes);
            if exp.get("error").and_then(Value::as_bool) == Some(true) {
                assert!(got.is_err(), "{name}: 끊어야 한다");
                continue;
            }
            let f = got.unwrap_or_else(|e| panic!("{name}: {e}"));
            let kind = match f.kind {
                Kind::Msg => "msg",
                Kind::Ok => "ok",
                Kind::Fail => "fail",
            };
            assert_eq!(kind, exp["kind"].as_str().unwrap(), "{name}");
            assert_eq!(u64::from(f.op), exp["op"].as_u64().unwrap(), "{name}");
            assert_eq!(u64::from(f.pid), exp["pid"].as_u64().unwrap(), "{name}");
            assert_eq!(u64::from(f.reserved), exp.get("reserved").and_then(Value::as_u64).unwrap_or(0), "{name}");
            assert_eq!(f.body, exp["body"], "{name}");
            assert_eq!(encode_raw(f.kind, f.reserved, f.op, f.pid, &f.body), bytes, "{name}: 다시 지으면 같은 바이트");
        }
    }

    #[test]
    fn empty_body_is_zero_bytes() {
        let ack = encode(Kind::Ok, 0x0701, 42, &Value::Null);
        assert_eq!(ack.len(), HEADER_LEN);
        assert_eq!(decode(&ack).unwrap().body, Value::Object(Default::default()));
        assert_eq!(encode(Kind::Msg, 0x0103, 1, &serde_json::json!({})).len(), HEADER_LEN);
    }
}
