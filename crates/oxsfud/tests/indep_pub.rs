// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-PUB-ADD · 정:IMPL-AFF-PUB-MOVE · 연:RULE-ROOM-MEDIA-CODECS · model: claude-opus-5-5
//! 독립 1층 시험(B′ 맹검) — 발행 검사 차례(자격 → pub_room → 수 → 필수 칸 → 1pc SSRC → 코덱) · H264 profile 은 앞 네 자리 · 발행 방 전환 판정.

use oxsfud::aff::{decide, Step};
use oxsfud::r#pub::{allowed, check_add, perm_bit, AddCtx};
use oxsig::code::Code;
use oxsig::types::{Duplex, Kind, Permission, PublishTrack, Source};
use std::collections::HashSet;

fn perm(a: bool, v: bool, s: bool, f: bool) -> Permission {
    Permission { publish_audio: a, publish_video: v, publish_screen: s, floor_request: f }
}

fn audio(ssrc: u32, mid: &str) -> PublishTrack {
    PublishTrack { kind: Kind::Audio, ssrc, mid: mid.into(), codec: Some("opus".into()), fmtp: None, pt: 111, rtx_pt: None, rtx_ssrc: None, source: None, duplex: Some(Duplex::Full), simulcast: None }
}

fn video(ssrc: u32, mid: &str, codec: &str, fmtp: Option<&str>) -> PublishTrack {
    PublishTrack { kind: Kind::Video, ssrc, mid: mid.into(), codec: Some(codec.into()), fmtp: fmtp.map(|s| s.into()), pt: 102, rtx_pt: Some(103), rtx_ssrc: Some(ssrc + 1), source: Some(Source::Camera), duplex: Some(Duplex::Full), simulcast: None }
}

fn ctx<'a>(p: Permission, active: usize, dl: &'a HashSet<u32>, one_pc: bool) -> AddCtx<'a> {
    AddCtx { room: "r1", pub_room: Some("r1"), perm: p, active, downlink_ssrcs: dl, one_pc, room_audio: "opus", room_video: "H264" }
}

fn code(r: Result<Vec<String>, oxsig::code::Failure>) -> Option<Code> {
    r.err().map(|f| f.code())
}

const H264_OK: &str = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f";

#[test]
fn permission_checked_before_count_and_fields() {
    let dl = HashSet::new();
    let all = perm(true, true, true, true);
    let no_audio = perm(false, true, true, true);
    let nine: Vec<PublishTrack> = (0..9).map(|i| audio(1000 + i, &i.to_string())).collect();
    assert_eq!(code(check_add(&ctx(no_audio, 0, &dl, false), &nine)), Some(Code::NOT_AUTHORIZED), "S-sym-IMPL-PUB-ADD-3 막힌 비트 + 9 항목 → 2006(자격이 먼저)");
    let mut broken = audio(1, "");
    broken.codec = None;
    assert_eq!(code(check_add(&ctx(no_audio, 0, &dl, false), &[broken])), Some(Code::NOT_AUTHORIZED), "S-sym-IMPL-PUB-ADD-3 막힌 비트 + 칸 빠짐 → 2006");
    assert_eq!(code(check_add(&ctx(all, 0, &dl, false), &nine)), Some(Code::TRACK_LIMIT), "S-sym-IMPL-PUB-ADD-3 9 항목 → 4002");
    let bad_codec: Vec<PublishTrack> = (0..9).map(|i| video(2000 + 2 * i, &i.to_string(), "VP8", None)).collect();
    assert_eq!(code(check_add(&ctx(all, 0, &dl, false), &bad_codec)), Some(Code::TRACK_LIMIT), "S-sym-IMPL-PUB-ADD-3 수가 코덱보다 먼저");
    let no_video = perm(true, false, true, true);
    assert_eq!(code(check_add(&ctx(no_video, 0, &dl, false), &[video(9, "0", "VP8", None)])), Some(Code::NOT_AUTHORIZED), "S-sym-IMPL-PUB-ADD-3 자격은 코덱을 보기 전에");
}

#[test]
fn active_count_blocks_repeated_calls() {
    let dl = HashSet::new();
    let all = perm(true, true, true, true);
    let eight: Vec<PublishTrack> = (0..8).map(|i| audio(1000 + i, &i.to_string())).collect();
    assert!(check_add(&ctx(all, 0, &dl, false), &eight).is_ok(), "S-sym-IMPL-PUB-ADD-1 8 항목은 받는다");
    assert!(check_add(&ctx(all, 8, &dl, false), &eight).is_ok(), "S-sym-IMPL-PUB-ADD-1 활성 8 + 8 = 16 은 받는다");
    assert_eq!(code(check_add(&ctx(all, 16, &dl, false), &[audio(1, "x")])), Some(Code::TRACK_LIMIT), "S-sym-IMPL-PUB-ADD-1 8+8 뒤 1 추가 → 4002(반복 호출 우회 봉쇄)");
}

#[test]
fn full_duplex_outside_pub_room_after_permission_before_count() {
    let dl = HashSet::new();
    let all = perm(true, true, true, true);
    let nine: Vec<PublishTrack> = (0..9).map(|i| audio(1000 + i, &i.to_string())).collect();
    let c = AddCtx { pub_room: Some("r2"), ..ctx(all, 0, &dl, false) };
    assert_eq!(code(check_add(&c, &nine)), Some(Code::NOT_PUB_ROOM), "S-sym-IMPL-PUB-ADD-3 전이중이 pub_room 밖이면 3011 — 수 앞");
    let no_audio = perm(false, true, true, true);
    let c = AddCtx { pub_room: Some("r2"), ..ctx(no_audio, 0, &dl, false) };
    assert_eq!(code(check_add(&c, &nine)), Some(Code::NOT_AUTHORIZED), "S-sym-IMPL-PUB-ADD-3 3011 은 자격 뒤");
}

#[test]
fn one_pc_ssrc_collision_only_in_1pc() {
    let dl: HashSet<u32> = [5555].into_iter().collect();
    let all = perm(true, true, true, true);
    assert_eq!(code(check_add(&ctx(all, 0, &dl, true), &[audio(5555, "0")])), Some(Code::SSRC_COLLISION), "SV-CODE-3008 1pc 발행 SSRC = 다운링크 SSRC → 3008");
    assert_eq!(code(check_add(&ctx(all, 0, &dl, true), &[audio(1, "0"), audio(5555, "1")])), Some(Code::SSRC_COLLISION), "SV-CODE-3008 전체 거절(부분 수용 0)");
    assert!(check_add(&ctx(all, 0, &dl, false), &[audio(5555, "0")]).is_ok(), "SV-CODE-3008 2pc 에선 검사 0");
}

#[test]
fn h264_profile_compares_first_four_hex_only() {
    let dl = HashSet::new();
    let all = perm(true, true, true, true);
    assert!(check_add(&ctx(all, 0, &dl, false), &[video(10, "0", "H264", Some(H264_OK))]).is_ok(), "SV-CODE-1005-1006 방 프로파일 그대로");
    let lvl = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e034";
    assert!(check_add(&ctx(all, 0, &dl, false), &[video(10, "0", "H264", Some(lvl))]).is_ok(), "SV-CODE-1005-1006 level 은 안 본다");
    let order = "profile-level-id=42e01f;packetization-mode=1";
    assert!(check_add(&ctx(all, 0, &dl, false), &[video(10, "0", "H264", Some(order))]).is_ok(), "SV-CODE-1005-1006 문자열이 아니라 파라미터로 견준다");
    let high = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640c1f";
    assert_eq!(code(check_add(&ctx(all, 0, &dl, false), &[video(10, "0", "H264", Some(high))])), Some(Code::CODEC_MISMATCH), "SV-CODE-1005-1006 다른 profile → 1006");
    assert_eq!(code(check_add(&ctx(all, 0, &dl, false), &[video(10, "0", "VP8", None)])), Some(Code::CODEC_MISMATCH), "SV-CODE-1005-1006 방 코덱과 다르면 1006");
    let r = check_add(&ctx(all, 0, &dl, false), &[video(10, "0", "VP8", None)]).unwrap_err();
    assert!(format!("{r:?}").contains("codec"), "SV-CODE-1005-1006 details.codec 동봉 — {r:?}");
}

#[test]
fn perm_bit_mapping() {
    assert_eq!(perm_bit(Kind::Audio, None), "publish_audio", "S-sym-IMPL-ROOM-PERMISSION-1 audio → publish_audio");
    assert_eq!(perm_bit(Kind::Video, Some(Source::Camera)), "publish_video", "S-sym-IMPL-ROOM-PERMISSION-1 camera → publish_video");
    assert_eq!(perm_bit(Kind::Video, Some(Source::Screen)), "publish_screen", "S-sym-IMPL-ROOM-PERMISSION-1 screen → publish_screen");
    assert!(!allowed(&perm(true, false, true, true), "publish_video"), "S-sym-IMPL-ROOM-PERMISSION-1 내린 비트는 막힌다");
    assert!(allowed(&perm(true, false, true, true), "publish_audio"), "S-sym-IMPL-ROOM-PERMISSION-1 산 비트는 통과");
}

#[test]
fn pub_select_recheck_rejects_whole_request() {
    assert_eq!(decide(Some("r1"), Some("r3"), None, true, false), Step::Reject, "S-sym-IMPL-AFF-PUB-ROOM-1 코덱이 다른 방으로 pub_select → 거절(1006)");
    assert_eq!(decide(Some("r1"), Some("r3"), None, false, true), Step::RejectPerm, "S-sym-IMPL-AFF-PUB-MOVE-1 막힌 비트 → 거절(2006)");
    assert_eq!(decide(Some("r1"), Some("r3"), None, false, false), Step::SelectMoved, "S-sym-IMPL-AFF-PUB-ROOM-2 다른 방으로 옮김");
    assert_eq!(decide(None, Some("r3"), None, false, false), Step::Select, "S-sym-IMPL-AFF-PUB-ROOM-2 처음 고름");
    assert_eq!(decide(Some("r1"), None, Some("r1"), false, false), Step::Away, "S-sym-IMPL-AFF-PUB-ROOM-2 지금 방 해제");
}
