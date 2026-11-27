// author: kodeholic (powered by Claude)
//! 원시 파트 읽기 도구(context 202611/20261109d P8) — 녹음 원시 파트(MCAP)에서 프레임 목록을 JSON 한 줄로 낸다. ★판정은 하지 않는다(재는 쪽은 labs).
//!
//!   rawtool frames <dir> <recording_id>            전체 파트를 이어 파이프라인으로 조립한 프레임
//!   rawtool frames <dir> <recording_id> --part K   파트 K 하나만(파트 자족 — SDK:RULE-PLAT-REC-PART 머리 스냅숏)
//!   rawtool frames <dir> <recording_id> --ref      대조군 — webrtc-media SampleBuilder + 디패킷타이저(설계 D15 · 기능 record-ref)
//!   rawtool nals <dir> <recording_id>              H264 영상 프레임마다 NAL 종류(실측 M4 — SPS · PPS 가 키프레임 안에만 오나 · 202611/20261110b)
//!
//! 프레임 하나 = [줄 SSRC, RTP 시각, 키프레임, 길이, CRC32, 첫 레코드 순번, 끝 레코드 순번, 종류 a|v](대조군은 레코드 순번이 없어 0).

use std::path::PathBuf;

use oxpeer::record::{FileSource, Output, Pipeline, Policy, Record};
use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 || (args[0] != "frames" && args[0] != "nals") {
        eprintln!("사용: rawtool frames <dir> <recording_id> [--part K | --ref] · rawtool nals <dir> <recording_id>");
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[1]);
    let id = &args[2];
    let part = args.iter().position(|a| a == "--part").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u32>().ok());
    let reference = args.iter().any(|a| a == "--ref");
    let mut src = match part {
        Some(k) => FileSource::of(vec![dir.join(format!("{id}-{k:04}.mcap"))]),
        None => FileSource::open(&dir, id).unwrap_or_else(|e| {
            eprintln!("파트를 못 열었다: {e}");
            std::process::exit(1);
        }),
    };
    let mut recs: Vec<Record> = Vec::new();
    src.run(|r| recs.push(r));
    if args[0] == "nals" {
        println!("{}", json!({ "parts": src.parts().len(), "frames": nal_frames(&recs) }));
        return;
    }
    let frames = if reference { reference_frames(&recs) } else { our_frames(&recs) };
    println!("{}", json!({ "parts": src.parts().len(), "records": recs.len(), "reference": reference, "frames": frames }));
}

/// 우리 파이프라인 — 세그먼트와 상관없이 조립된 프레임 전부(출처 없는 프레임 포함) · ★메운 PLC 는 뺀다(조립이 아니라 정리층이 지은 것 — SDK:RULE-PLAT-REC-GAP · 202611/20261109f)
fn our_frames(recs: &[Record]) -> Vec<Value> {
    let mut p = Pipeline::new(Policy::Span { time_ns: u64::MAX, bytes: u64::MAX });
    let mut out = Vec::new();
    for r in recs {
        for o in p.push(r) {
            if let Output::Frame(f) | Output::Orphan(f) = o {
                if f.filled {
                    continue;
                }
                out.push(json!([f.ssrc, f.rtp_ts, f.keyframe, f.data.len(), crc32fast::hash(&f.data), f.rec_first, f.rec_last, if f.kind == "video" { "v" } else { "a" }]));
            }
        }
    }
    out
}

/// H264 영상 프레임마다 [줄 SSRC, RTP 시각, 키프레임, [NAL 종류 …]] — Annex-B 시작 부호로 가른다
fn nal_frames(recs: &[Record]) -> Vec<Value> {
    let mut p = Pipeline::new(Policy::Span { time_ns: u64::MAX, bytes: u64::MAX });
    let mut out = Vec::new();
    for r in recs {
        for o in p.push(r) {
            if let Output::Frame(f) = o
                && f.kind == "video"
                && f.codec.eq_ignore_ascii_case("h264")
            {
                let d = &f.data;
                let types: Vec<u8> = (0..d.len().saturating_sub(3)).filter(|&i| d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 1).map(|i| d[i + 3] & 0x1f).collect();
                out.push(json!([f.ssrc, f.rtp_ts, f.keyframe, types]));
            }
        }
    }
    out
}

#[cfg(feature = "record-ref")]
fn reference_frames(recs: &[Record]) -> Vec<Value> {
    use std::collections::BTreeMap;

    use bytes::Bytes;
    use rtp::codecs::{h264::H264Packet, opus::OpusPacket, vp8::Vp8Packet};
    use rtp::packet::Packet;
    use webrtc_media::io::sample_builder::SampleBuilder;
    use webrtc_util::marshal::Unmarshal;

    enum Sb {
        Opus(SampleBuilder<OpusPacket>),
        Vp8(SampleBuilder<Vp8Packet>),
        H264(SampleBuilder<H264Packet>),
    }
    // 줄 SSRC → (코덱 · 영상인가) — ctl/track rows(SDK:RULE-PLAT-REC-RAW)
    let mut rows: BTreeMap<u32, (String, bool)> = BTreeMap::new();
    let mut sbs: BTreeMap<u32, Sb> = BTreeMap::new();
    let mut out: BTreeMap<u32, Vec<Value>> = BTreeMap::new();
    let mut order: Vec<(u32, usize)> = Vec::new();
    let key_of = |codec: &str, d: &[u8]| -> bool {
        match codec {
            "vp8" => d.first().is_some_and(|b| b & 0x01 == 0),
            "h264" => d.windows(5).any(|w| w[..4] == [0, 0, 0, 1] && w[4] & 0x1f == 5),
            _ => true,
        }
    };
    let pop = |ssrc: u32, sb: &mut Sb, codec: &str, out: &mut BTreeMap<u32, Vec<Value>>, order: &mut Vec<(u32, usize)>| loop {
        let s = match sb {
            Sb::Opus(b) => b.pop_with_timestamp(),
            Sb::Vp8(b) => b.pop_with_timestamp(),
            Sb::H264(b) => b.pop_with_timestamp(),
        };
        let Some((s, ts)) = s else { break };
        let v = out.entry(ssrc).or_default();
        order.push((ssrc, v.len()));
        v.push(json!([ssrc, ts, key_of(codec, &s.data), s.data.len(), crc32fast::hash(&s.data), 0, 0, if codec == "opus" { "a" } else { "v" }]));
    };
    for r in recs {
        match &r.channel {
            oxpeer::record::Channel::Track => {
                let Ok(v) = serde_json::from_slice::<Value>(&r.data) else { continue };
                for (k, x) in v["rows"].as_object().into_iter().flatten() {
                    if let (Ok(s), Some(c)) = (k.parse::<u32>(), x["codec"].as_str()) {
                        rows.insert(s, (c.to_ascii_lowercase(), x["kind"].as_str() == Some("video")));
                    }
                }
            }
            oxpeer::record::Channel::Rtp(ssrc) => {
                let Some((codec, _)) = rows.get(ssrc).cloned() else { continue };
                let mut b = Bytes::from(r.data.clone());
                let Ok(pkt) = Packet::unmarshal(&mut b) else { continue };
                let sb = sbs.entry(*ssrc).or_insert_with(|| match codec.as_str() {
                    "vp8" => Sb::Vp8(SampleBuilder::new(64, Vp8Packet::default(), 90_000)),
                    "h264" => Sb::H264(SampleBuilder::new(64, H264Packet::default(), 90_000)),
                    _ => Sb::Opus(SampleBuilder::new(64, OpusPacket, 48_000)),
                });
                match sb {
                    Sb::Opus(x) => x.push(pkt),
                    Sb::Vp8(x) => x.push(pkt),
                    Sb::H264(x) => x.push(pkt),
                }
                pop(*ssrc, sb, &codec, &mut out, &mut order);
            }
            _ => {}
        }
    }
    order.into_iter().filter_map(|(s, i)| out.get(&s).and_then(|v| v.get(i)).cloned()).collect()
}

#[cfg(not(feature = "record-ref"))]
fn reference_frames(_: &[Record]) -> Vec<Value> {
    eprintln!("--ref 는 기능 record-ref 가 있어야 한다");
    std::process::exit(2);
}
