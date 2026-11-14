// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SUB-PIPELINE · 정:IMPL-SUB-TABLE · 정:RULE-CONC-SIX · model: claude-opus-5-5
//! 1층 제어 층 · 통계 — 스레드 넷이 한 배정표에 재배정을 겹쳐 부르며 사이사이 재료(나열 · 선호 · 말소리 순위 · 든 방)를 바꾼다. 가라앉힌 뒤 재배정 한 번을 더 돌려도 줄 목록 · 줄 밖 표지가 표와 같아야 한다 — 어긋남은 다음 재배정으로 낫지 않는다(바뀐 것만 집행). 결정적 재현은 l1_ops_resync_turn.rs 가 들고 이 파일은 따로 프로세스로 넓게 흔든다 — 재발 시험이 아니라 흔들기다(차례 없는 빌드에서도 초록일 수 있다). 판마다 기한을 두어 차례끼리 서로 기다리는 고리(교착)는 끝나지 않는 대신 빨강으로 드러난다.

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use oxsfud::ops;
use oxsfud::r#pub::PubStream;
use oxsfud::room::Room;
use oxsig::msg::Kind;

use support::*;

const ROUNDS: usize = 40;
const THREADS: u64 = 4;
const STEPS: usize = 300;
const ROUND_LIMIT: Duration = Duration::from_secs(30);

fn stream(user: &str, room: &str, kind: Kind) -> Arc<PubStream> {
    let st = support::stream(user, room, kind);
    if kind == Kind::Audio {
        voiced(&st, 0);
    }
    list(&st, true);
    st
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

struct Stage {
    sub: Sub,
    y: Arc<Room>,
    audio: Vec<Arc<PubStream>>,
    video: Vec<Arc<PubStream>>,
}

fn stage(round: usize) -> Stage {
    let x = room(&tag(&format!("r4-{round}-x")));
    let y = room(&tag(&format!("r4-{round}-y")));
    let sub = subscriber(&tag(&format!("r4-{round}-p")));
    enter(&sub.p, &x);
    let mut audio: Vec<Arc<PubStream>> = (0..26).map(|_| stream(&tag("r4-ua"), &x.id, Kind::Audio)).collect();
    audio.push(stream(&tag("r4-uy"), &y.id, Kind::Audio));
    let video: Vec<Arc<PubStream>> = (0..3).map(|_| stream(&tag("r4-uv"), &x.id, Kind::Video)).collect();
    ops::media::resync_peer(&sub.p, true);
    Stage { sub, y, audio, video }
}

fn churn(st: &Stage, seed: u64) {
    let mut rng = Rng(seed);
    let now = common::now_ms();
    for _ in 0..STEPS {
        match rng.next() % 6 {
            0 => {
                let all: Vec<&Arc<PubStream>> = st.audio.iter().chain(st.video.iter()).collect();
                let s = all[(rng.next() % all.len() as u64) as usize];
                s.listed.fetch_xor(true, Ordering::AcqRel);
            }
            1 => {
                let v = &st.video[(rng.next() % st.video.len() as u64) as usize];
                st.sub.t.set_pref(&v.track_id, Some((rng.next() % 3) as u8), Some(rng.next().is_multiple_of(2)));
            }
            2 | 3 => {
                let a = &st.audio[(rng.next() % st.audio.len() as u64) as usize];
                if let Some(t) = a.physical().first() {
                    t.last_voice_ms.store(now.saturating_sub(rng.next() % 2_000), Ordering::Release);
                }
            }
            4 => {
                let mut g = st.sub.p.rooms.lock().expect("rooms");
                if g.remove(&st.y.id).is_none() {
                    g.insert(st.y.id.clone(), "j-y".into());
                }
            }
            _ => {}
        }
        ops::media::resync_peer(&st.sub.p, true);
    }
}

#[test]
fn r4_overlapping_resyncs_keep_lists_equal_to_the_table() {
    let start = Instant::now();
    let mut red = Vec::new();
    for round in 0..ROUNDS {
        let st = Arc::new(stage(round));
        let hs: Vec<_> = (0..THREADS)
            .map(|i| {
                let st = st.clone();
                std::thread::spawn(move || churn(&st, 0x9e37_79b9_7f4a_7c15 ^ (((round as u64) << 8) | (i + 1))))
            })
            .collect();
        let until = Instant::now() + ROUND_LIMIT;
        while hs.iter().any(|h| !h.is_finished()) {
            assert!(
                Instant::now() < until,
                "정:RULE-CONC-SIX round {round} did not finish within {ROUND_LIMIT:?} — overlapping resyncs of one table are stuck (a turn is waiting on a turn or on a lock taken inside one)"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        for h in hs {
            h.join().expect("churn");
        }
        ops::media::resync_peer(&st.sub.p, true);
        let b = breaks(&st.sub);
        if !b.is_empty() {
            red.push((round, b));
        }
        if let Ok(mut g) = sfu().tracks.lock() {
            for s in st.audio.iter().chain(st.video.iter()) {
                g.remove(&s.track_id);
            }
        }
    }
    let first = red.first().map(|(r, b)| format!("round {r}: {b:?}")).unwrap_or_default();
    assert!(
        red.is_empty(),
        "정:RULE-CONC-SIX 정:IMPL-SUB-PIPELINE after overlapping resyncs settle, one more resync leaves the row lists and unplaced marks equal to the table — {} of {ROUNDS} rounds broke ({:?} elapsed); first {first}",
        red.len(),
        start.elapsed()
    );
}
