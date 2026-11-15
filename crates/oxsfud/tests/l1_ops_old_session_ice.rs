// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-LINK-ICE-RESTART · model: claude-fable-5-2
//! 1층 제어 층 — 옛 세션의 늦은 `ICE_RESTART`(context 202611/20261115e). 「그 세션의 Peer 가 없으면 `3010`」 — 같은 사람이 새 세션으로 다시 붙은 뒤에 닿은 옛 세션의 요청은 새 세션 Peer 의 자격을 바꾸지 않는다. 이 파일만 미디어 층을 세운다(자격을 실제로 거는 길을 타야 해서).

mod support;

use std::sync::Once;

use oxsfud::fwd::rt::{self, StartCfg};
use oxsfud::link::Peer;
use oxsfud::ops;
use oxsfud::ops::handle::Ctx;
use oxsig::Failure;
use serde_json::{Value, json};

use support::*;

static MEDIA: Once = Once::new();

fn media() {
    MEDIA.call_once(|| {
        let at = "127.0.0.1:0".parse().expect("addr");
        rt::start(StartCfg { bind: at, advertised: at, tcp_port: None, workers: 1, queue_limit: rt::DEFAULT_QUEUE_PACKETS }).expect("media");
    });
    assert!(rt::media().is_some(), "stand check — the media layer is up");
}

fn old_of(p: &Peer) -> Ctx {
    let mut c = ctx_of(p);
    c.sid = format!("old-{}", p.sid);
    c
}

fn code(r: &Result<Value, Failure>) -> Option<u16> {
    r.as_ref().err().map(|f| f.code)
}

fn ufrags(p: &Peer) -> Vec<Vec<String>> {
    p.transports().iter().map(|t| t.creds.lock().expect("creds").ufrags()).collect()
}

#[test]
fn old_session_ice_restart_does_not_touch_the_new_session_credentials() {
    let _g = serial();
    media();
    let u = subscriber(&tag("u"));
    let before = ufrags(&u.p);
    let got = ops::media::ice_restart(&old_of(&u.p), &json!({ "sfu_id": sfu().epoch }));
    assert_eq!(code(&got), Some(3010), "an old session's restart finds no peer of that session");
    assert_eq!(ufrags(&u.p), before, "the new session's credentials are untouched");
}

#[test]
fn the_same_session_still_restarts() {
    let _g = serial();
    media();
    let u = subscriber(&tag("u"));
    let before = ufrags(&u.p);
    let got = ops::media::ice_restart(&ctx_of(&u.p), &json!({ "sfu_id": sfu().epoch }));
    assert!(got.is_ok(), "the session of the peer restarts: {:?}", code(&got));
    let after = ufrags(&u.p);
    assert_eq!(after.len(), before.len(), "one credential set per transport");
    assert!(after.iter().zip(&before).all(|(a, b)| a.len() == b.len() + 1), "each transport holds the old and the new credential");
}
