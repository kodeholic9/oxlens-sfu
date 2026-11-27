// author: kodeholic (powered by Claude)
//! 다시 읽기(SDK:RULE-PLAT-REC-CLOCK 재생 차례 · REC-PART 정상 · 비정상) — 파트를 처음부터 선형으로 읽어 기록기가 적은 레코드를 같은 차례로 낸다.
//! Footer 가 없는 파트(비정상 종료 · 아직 쓰는 중)는 마지막 닫힌 청크까지 읽고 멈춘다 — 색인을 쓰지 않는다(색인 차례로 다시 늘어놓지 않는다).

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use mcap::records::Record as McapRecord;
use mcap::sans_io::linear_reader::{LinearReadEvent, LinearReader, LinearReaderOptions};

use super::{Channel, Record};

/// 파트 하나를 훑은 결과 — 사슬 · 회전 · 비정상 종료 판정 재료
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartScan {
    pub path: PathBuf,
    /// `oxlens.recording` · `oxlens.part` · `oxlens.part_end` 메타데이터
    pub recording: BTreeMap<String, String>,
    pub part: BTreeMap<String, String>,
    pub part_end: Option<BTreeMap<String, String>>,
    pub messages: u64,
    pub first_sequence: Option<u32>,
    pub last_sequence: Option<u32>,
    pub first_log_time: Option<u64>,
    pub last_log_time: Option<u64>,
    /// Footer 까지 읽혔나(정상으로 닫힌 파트)
    pub complete: bool,
    /// 끝까지 못 읽은 까닭(잘린 꼬리 등)
    pub error: Option<String>,
}

/// 파트 하나를 선형으로 읽는다 — `each` 는 메시지마다 · 끝까지 못 읽으면 `error` 에 적고 거기까지
fn read_part(path: &Path, mut each: impl FnMut(Record)) -> PartScan {
    let mut scan = PartScan { path: path.to_path_buf(), ..PartScan::default() };
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            scan.error = Some(e.to_string());
            return scan;
        }
    };
    let mut r = LinearReader::new_with_options(LinearReaderOptions::default().with_skip_end_magic(false));
    let mut topics: HashMap<u16, Channel> = HashMap::new();
    while let Some(ev) = r.next_event() {
        match ev {
            Ok(LinearReadEvent::ReadRequest(need)) => {
                let n = match f.read(r.insert(need)) {
                    Ok(n) => n,
                    Err(e) => {
                        scan.error = Some(e.to_string());
                        break;
                    }
                };
                r.notify_read(n);
            }
            Ok(LinearReadEvent::Record { opcode, data }) => match mcap::parse_record(opcode, data) {
                Ok(McapRecord::Channel(c)) => {
                    if let Some(ch) = Channel::parse(&c.topic) {
                        topics.insert(c.id, ch);
                    }
                }
                Ok(McapRecord::Message { header, data }) => {
                    let Some(ch) = topics.get(&header.channel_id).cloned() else { continue };
                    scan.messages += 1;
                    scan.first_sequence.get_or_insert(header.sequence);
                    scan.last_sequence = Some(header.sequence);
                    scan.first_log_time.get_or_insert(header.log_time);
                    scan.last_log_time = Some(header.log_time);
                    each(Record { sequence: header.sequence, log_time: header.log_time, channel: ch, data: data.into_owned() });
                }
                Ok(McapRecord::Metadata(m)) => match m.name.as_str() {
                    "oxlens.recording" => scan.recording = m.metadata,
                    "oxlens.part" => scan.part = m.metadata,
                    "oxlens.part_end" => scan.part_end = Some(m.metadata),
                    _ => {}
                },
                Ok(McapRecord::Footer(_)) => scan.complete = true,
                Ok(_) => {}
                Err(e) => {
                    scan.error = Some(e.to_string());
                    break;
                }
            },
            Err(e) => {
                scan.error = Some(e.to_string());
                break;
            }
        }
    }
    scan
}

/// 파트 하나를 훑는다(메시지는 세기만)
pub fn scan_part(path: &Path) -> PartScan {
    read_part(path, |_| {})
}

/// 원시 파트를 다시 흘리는 Source — 한 녹음의 파트를 파트 번호 차례로 이어 읽는다(파트 하나만 줘도 된다 — 머리 스냅숏이 있다)
pub struct FileSource {
    parts: Vec<PathBuf>,
    scans: Vec<PartScan>,
}

impl FileSource {
    /// 디렉터리의 `<recording_id>-NNNN.mcap` 을 파트 번호 차례로
    pub fn open(dir: &Path, recording_id: &str) -> std::io::Result<FileSource> {
        let pre = format!("{recording_id}-");
        let mut parts: Vec<(u32, PathBuf)> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter_map(|p| {
                let n = p.file_name()?.to_str()?.strip_prefix(&pre)?.strip_suffix(".mcap")?.parse().ok()?;
                Some((n, p))
            })
            .collect();
        parts.sort();
        Ok(FileSource { parts: parts.into_iter().map(|(_, p)| p).collect(), scans: Vec::new() })
    }

    /// 주어진 파트들만(차례 그대로)
    pub fn of(parts: Vec<PathBuf>) -> FileSource {
        FileSource { parts, scans: Vec::new() }
    }

    pub fn parts(&self) -> &[PathBuf] {
        &self.parts
    }

    /// 레코드 전부를 파일 차례로 `each` 에 준다 — 파트마다 훑은 결과를 남긴다(`scans`)
    pub fn run(&mut self, mut each: impl FnMut(Record)) {
        self.scans.clear();
        for p in self.parts.clone() {
            let s = read_part(&p, &mut each);
            self.scans.push(s);
        }
    }

    pub fn scans(&self) -> &[PartScan] {
        &self.scans
    }
}
