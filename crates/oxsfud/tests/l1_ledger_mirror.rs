// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:ROOM.ledger.MaxRooms · 연:RULE-ROOM-MAX-ROOMS · 연:COND-ROOM-BUDGET · model: claude-opus-5-5
//! 1층 거울 — 서버가 손으로 둔 m-line 예산 상수(방 수 한도 · 구독 몫 · 전이중 줄 상한 · 슬롯 몫 기본)가 연동규격서 원장 counts 와 같다. 원장 값은 `sync:server` 가 옮긴 `testdata/spec/protocol/out/counts.json`(원천 `protocol/out/ir.json` 의 `vals.cnt`)에서 읽는다 — 값의 원천은 원장 하나(context 202611/20261124b MaxRooms 20).

use std::path::{Path, PathBuf};

use oxsfud::ops::rooms::MAX_ROOMS;
use oxsfud::sub::{FULL_AUDIO, FULL_VIDEO, SUB_1PC, SUB_2PC};

fn spec_dir() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/spec")
}

fn ledger(name: &str) -> usize {
    let dir = spec_dir();
    let copy = dir.join("protocol/out/counts.json");
    let source = dir.join("protocol/out/ir.json");
    let v: serde_json::Value = if copy.is_file() {
        serde_json::from_str(&std::fs::read_to_string(&copy).expect("counts.json")).expect("counts.json")
    } else {
        let text = std::fs::read_to_string(&source).unwrap_or_else(|e| panic!("원장 없음 {} · {}: {e} — context/spec 에서 npm run sync:server", copy.display(), source.display()));
        serde_json::from_str::<serde_json::Value>(&text).expect("ir.json")["vals"]["cnt"].clone()
    };
    v[name].as_str().and_then(|s| s.parse().ok()).unwrap_or_else(|| panic!("원장에 {name} 이 없다"))
}

#[test]
fn server_budget_constants_match_the_ledger() {
    assert_eq!(MAX_ROOMS, ledger("ROOM.MaxRooms"), "CP-CODE-LISTEN-LIMIT 연:ROOM.ledger.MaxRooms — the listen limit is the ledger's");
    assert_eq!(SUB_1PC, ledger("ROOM.Sub1pc"), "연:ROOM.ledger.Sub1pc — the 1pc subscribe budget is the ledger's");
    assert_eq!(SUB_2PC, ledger("ROOM.Sub2pc"), "연:ROOM.ledger.Sub2pc — the 2pc subscribe budget is the ledger's");
    assert_eq!((FULL_AUDIO, FULL_VIDEO), (ledger("ROOM.FullAudio"), ledger("ROOM.FullVideo")), "연:ROOM.ledger.FullAudio 연:ROOM.ledger.FullVideo — the full-duplex line caps are the ledger's");
    assert_eq!(common::policy::Policy::default().media.slot_mid_reserve as usize, ledger("ROOM.SlotBudget"), "연:ROOM.ledger.SlotBudget — the default slot budget is the ledger's");
}
