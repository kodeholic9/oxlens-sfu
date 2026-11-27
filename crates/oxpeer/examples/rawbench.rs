// author: kodeholic (powered by Claude)
//! 녹음 원시 기록 실측(context 202611/20261109c M6 · M8 · M9 · M10) — 서버 없이 합성 RTP 를 기록기와 같은 쓰기 경로(MCAP · 청크 · 시간으로 닫기)로 쓴다.
//! 시각은 합성(미디어 시간)이다 — 60초 분량을 될 수 있는 대로 빨리 써서 「미디어 1초당 비용」을 잰다.
//!
//!   cargo run -p oxpeer --release --example rawbench --features record-lz4,record-zstd -- --out /tmp/rawbench
//!
//! 표준 출력에 한 줄 JSON — 판마다 (형상 · F · 압축 · fsync) 파일 바이트 · 페이로드 바이트 · 오버헤드 · 미디어 1초당 CPU µs · 시간당 크기.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use mcap::records::MessageHeader;
use mcap::{Compression, WriteOptions, Writer};
use serde_json::{Value, json};

struct Shape {
    name: &'static str,
    /// (트랙 수, 초당 패킷, 페이로드 바이트)
    tracks: Vec<(u32, u32, usize)>,
}

fn shapes() -> Vec<Shape> {
    vec![
        // Opus 20ms · 32kbps 근처 — 페이로드 80B
        Shape { name: "audio_1", tracks: vec![(1, 50, 80)] },
        // 영상 1 트랙 1.5Mbps — 1,100B 패킷 170개/s + 음성 1
        Shape { name: "video_1", tracks: vec![(1, 170, 1_100), (1, 50, 80)] },
        // 줄 상한 26 영상(낮은 층 300kbps 근처 — 35개/s) + 음성 슬롯 4
        Shape { name: "video_26", tracks: vec![(26, 35, 1_100), (4, 50, 80)] },
    ]
}

/// 결정론 의사 난수(코덱 출력처럼 압축이 안 되는 바이트)
struct Rng(u64);
impl Rng {
    fn fill(&mut self, b: &mut [u8]) {
        for x in b.iter_mut() {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            *x = self.0 as u8;
        }
    }
}

fn run(out: &Path, sh: &Shape, flush_s: u64, comp: Option<Compression>, cname: &str, fsync: bool, secs: u64) -> Value {
    let path = out.join(format!("{}-f{}-{}-{}.mcap", sh.name, flush_s, cname, if fsync { "fsync" } else { "nosync" }));
    let f = File::create(&path).unwrap();
    let sync = f.try_clone().unwrap();
    let mut w: Writer<BufWriter<File>> = WriteOptions::new().use_chunks(true).chunk_size(None).compression(comp).profile("oxlens").create(BufWriter::new(f)).unwrap();
    let mut chans = Vec::new();
    let mut ssrc = 0x8000_0000u32;
    for (n, pps, size) in &sh.tracks {
        for _ in 0..*n {
            ssrc += 1;
            let id = w.add_channel(0, &format!("rtp/{ssrc}"), "oxlens.rtp", &BTreeMap::new()).unwrap();
            chans.push((id, *pps, *size, ssrc));
        }
    }
    let tick = w.add_channel(0, "tick", "json", &BTreeMap::new()).unwrap();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut seq = 0u32;
    let (mut payload, mut flushes) = (0u64, 0u64);
    let mut buf = vec![0u8; 1_500];
    let t0 = Instant::now();
    // 1ms 칸마다 그 칸에 떨어지는 패킷을 쓴다 · 250ms 마다 tick · F 마다 청크를 닫는다
    for ms in 0..secs * 1_000 {
        let t = 1_800_000_000_000_000_000u64 + ms * 1_000_000;
        for (id, pps, size, s) in &chans {
            let due = (ms * *pps as u64) / 1_000 != ((ms + 1) * *pps as u64) / 1_000;
            if !due {
                continue;
            }
            // RTP 머리 12 + CSRC 4 + 확장 8 = 24B
            let n = 24 + size;
            buf[0] = 0x91;
            buf[1] = 0x6f;
            buf[8..12].copy_from_slice(&s.to_be_bytes());
            rng.fill(&mut buf[24..n]);
            seq = seq.wrapping_add(1);
            w.write_to_known_channel(&MessageHeader { channel_id: *id, sequence: seq, log_time: t, publish_time: t }, &buf[..n]).unwrap();
            payload += *size as u64;
        }
        if ms % 250 == 0 {
            seq = seq.wrapping_add(1);
            w.write_to_known_channel(&MessageHeader { channel_id: tick, sequence: seq, log_time: t, publish_time: t }, b"{}").unwrap();
            if ms % (flush_s * 1_000) == 0 && ms > 0 {
                w.flush().unwrap();
                flushes += 1;
                if fsync {
                    sync.sync_data().unwrap();
                }
            }
        }
    }
    w.finish().unwrap();
    let mut bw = w.into_inner();
    bw.flush().unwrap();
    bw.get_ref().sync_all().unwrap();
    let cpu = t0.elapsed();
    let bytes = std::fs::metadata(&path).unwrap().len();
    let _ = std::fs::remove_file(&path);
    json!({
        "shape": sh.name, "flush_s": flush_s, "compression": cname, "fsync": fsync,
        "file_bytes": bytes, "payload_bytes": payload,
        "overhead_pct": ((bytes as f64 / payload as f64) - 1.0) * 100.0,
        "mb_per_hour": bytes as f64 / secs as f64 * 3_600.0 / 1e6,
        "us_per_media_s": cpu.as_micros() as f64 / secs as f64,
        "flushes": flushes,
    })
}

fn main() {
    let mut out = std::env::temp_dir().join("rawbench");
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        if k == "--out" {
            out = PathBuf::from(it.next().unwrap_or_default());
        }
    }
    std::fs::create_dir_all(&out).unwrap();
    let secs = 60;
    let mut rows = Vec::new();
    let comps: Vec<(Option<Compression>, &str)> = vec![
        (None, "none"),
        #[cfg(feature = "record-lz4")]
        (Some(Compression::Lz4), "lz4"),
        #[cfg(feature = "record-zstd")]
        (Some(Compression::Zstd), "zstd"),
    ];
    for sh in shapes() {
        for f in [1, 2, 5, 10] {
            for fsync in [false, true] {
                rows.push(run(&out, &sh, f, None, "none", fsync, secs));
            }
        }
        for (c, n) in &comps {
            rows.push(run(&out, &sh, 2, *c, n, false, secs));
        }
    }
    println!("{}", json!({ "media_secs": secs, "rows": rows }));
}
