// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§3-1 · §6-1 · 정§3-2 · §3-3 · §3-5 · model: claude-opus-5

//! WS 디스패치 — ★**프레임 하나에 답 하나.** 그것이 전부다.
//!
//! ★**판정만 한다** — 소켓도 시계도 주인이 쥔다. 그래서 갈래 전량을 1층에서 태운다.

use oxsig::body::session::{BindReq, BindRes, LeaveNotice};
use oxsig::frame::{DecodeError, Header, Kind};
use oxsig::{Code, Failure, Op};

use crate::session::{BindInput, BindOutcome, Sessions, VerifiedToken};

/// ★**`BIND` 가 안 오면 끊는 시간**(정§3-2) — 무인증 소켓이 남지 않게.
pub const BIND_DEADLINE_MS: u64 = 10_000;

/// 한 소켓의 상태. ★**인증 전 프레임 큐를 두지 않는다.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conn {
    /// 열렸고 아직 `BIND` 가 안 왔다.
    Unbound { opened_at: u64 },
    Bound { session_id: String },
}

/// 디스패치의 답. ★**끊을 때는 `LEAVE` 를 보내고 닫는다** — WS Close 에 사유를 싣지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// 성공 응답 — 같은 `op`·`pid` 에 `flags=01`.
    Ok { header: Header, body: Vec<u8> },
    /// 실패 응답 — 같은 `op`·`pid` 에 `flags=10`, body 는 `Failure`.
    Fail { header: Header, body: Vec<u8> },
    /// 답할 것이 없다(ACK 이거나 응답 없는 op).
    Silent,
    /// ★`LEAVE` 를 보내고 닫는다.
    Close(LeaveNotice),
}

fn ok(h: Header, body: Vec<u8>) -> Reply {
    Reply::Ok { header: Header { kind: Kind::Ok, ..h }, body }
}

fn fail(h: Header, code: Code) -> Reply {
    let body = serde_json::to_vec(&Failure::new(code)).unwrap_or_default();
    Reply::Fail { header: Header { kind: Kind::Fail, ..h }, body }
}

/// 토큰 검증을 바깥에서 받는다 — ★**여기서 비밀을 쥐지 않는다.**
pub type Verify<'a> = &'a dyn Fn(&str) -> Result<VerifiedToken, Code>;

/// 프레임 하나를 판정한다.
pub fn dispatch(
    conn: &mut Conn,
    sessions: &mut Sessions,
    verify: Verify<'_>,
    window_ms: u64,
    now: u64,
    header: Header,
    body: &[u8],
) -> Reply {
    // ★응답·실패 프레임에는 답하지 않는다 — 답하면 둘이 서로를 낳는다.
    if header.kind != Kind::Request {
        if let Conn::Bound { .. } = conn {
            return Reply::Silent;
        }
        return Reply::Silent;
    }
    match (&conn.clone(), header.op) {
        (Conn::Unbound { .. }, Op::Bind) => {
            let req: BindReq = match serde_json::from_slice(body) {
                Ok(r) => r,
                Err(_) => return fail(header, Code::InvalidPayload),
            };
            let verified = verify(&req.token);
            let input = BindInput {
                session_id: req.session_id.clone(),
                verified: verified.as_ref().ok().cloned(),
                token_error: verified.err(),
                pc_mode: req.pc_mode,
                client_ver: req.client_ver,
            };
            match sessions.bind(&input, now, window_ms) {
                BindOutcome::Rejected(code) => fail(header, code),
                out => {
                    let (sid, _evicted) = match &out {
                        BindOutcome::Resumed { session_id } => (session_id.clone(), None),
                        BindOutcome::Fresh { session_id } => (session_id.clone(), None),
                        BindOutcome::FreshEvicting { session_id, evicted } => {
                            (session_id.clone(), Some(evicted.clone()))
                        }
                        BindOutcome::Rejected(_) => unreachable!(),
                    };
                    let s = sessions.get(&sid).expect("방금 만들었다");
                    let res = BindRes {
                        user_id: s.user_id.clone(),
                        server_ver: crate::session::SERVER_VER,
                        heartbeat_interval: 10_000,
                        // ★보낸 것과 같으면 이어받기가 섰다는 뜻이다 — 클라가 그것으로 안다.
                        session_id: sid.clone(),
                        resume_window_ms: window_ms,
                        pc_mode: s.pc_mode,
                    };
                    *conn = Conn::Bound { session_id: sid };
                    ok(header, serde_json::to_vec(&res).unwrap_or_default())
                }
            }
        }

        // ★서버는 빈 응답으로 답한다.
        (Conn::Bound { .. }, Op::Heartbeat) => ok(header, Vec::new()),

        (Conn::Bound { session_id }, Op::Resume) => match sessions.may_resume(session_id) {
            // 스냅샷 조립은 방 층이 한다(덩어리 4 와 한 쌍) — 여기서는 판정만 낸다.
            Ok(()) => ok(header, b"{\"snapshot\":{},\"publications\":[]}".to_vec()),
            Err(code) => fail(header, code),
        },

        // ★C→S `LEAVE` 는 빈 body 이고 응답이 없다. 세션은 ★**즉시 폐기**다.
        (Conn::Bound { session_id }, Op::Leave) => {
            sessions.discard(session_id);
            Reply::Silent
        }

        // 나머지는 방·미디어 층이 받는다(덩어리 4).
        (Conn::Bound { .. }, _) => fail(header, Code::UnknownOp),

        // ★`BIND` 전의 다른 op 은 받지 않는다 — 인증 전 프레임 큐를 두지 않는다.
        (Conn::Unbound { .. }, _) => fail(header, Code::NotBound),
    }
}

/// 이 `BIND` 가 누구를 축출했나 — ★**옛 연결에 `LEAVE` `2010` 을 보내야 한다**(정§3-2 #3).
///
/// ★**통보만 하고 유령으로 남기지 않는다** — 남기면 명단에 같은 사람이 둘이다.
pub fn evicted_by(before: &Conn, after: &Conn, sessions: &mut Sessions) -> Option<Evicted> {
    let (Conn::Unbound { .. }, Conn::Bound { session_id }) = (before, after) else {
        return None;
    };
    let me = sessions.get(session_id)?;
    let (user_id, resumed) = (me.user_id.clone(), me.resumed);
    // ★갈래 둘 — ①판정 3(새 세션이 같은 신원의 산 세션을 축출) ②판정 1(같은 세션을 새 소켓이 이어받음).
    //   ★**둘 다 옛 소켓에 `LEAVE 2010` 을 보내고 닫는다** — 한 세션에 소켓 하나다(정§3-2).
    if let Some(old) = sessions.take_evicted(&user_id, session_id) {
        return Some(Evicted::Session(old));
    }
    if resumed {
        // 같은 `session_id` 를 다른 소켓이 이어받았다 — 그 자리의 옛 소켓을 닫는다.
        return Some(Evicted::Socket(session_id.clone()));
    }
    None
}

/// 무엇을 닫아야 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evicted {
    /// 옛 **세션**이 통째로 축출됐다(판정 3).
    Session(String),
    /// 같은 세션의 옛 **소켓**이 남아 있다(판정 1).
    Socket(String),
}

/// ★**`BIND` 가 제때 안 왔나** — 무인증 소켓을 열어 두지 않는다.
pub fn bind_overdue(conn: &Conn, now: u64) -> Option<LeaveNotice> {
    match conn {
        Conn::Unbound { opened_at } if now.saturating_sub(*opened_at) >= BIND_DEADLINE_MS => {
            Some(LeaveNotice::new(Code::HeartbeatTimeout))
        }
        _ => None,
    }
}

/// 프레임을 못 읽었다 — ★**응답을 지을 수 없으니 `LEAVE` 를 보내고 닫는다.**
pub fn on_decode_error(e: DecodeError) -> Reply {
    Reply::Close(LeaveNotice::new(e.leave_reason()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxsig::Permission;

    fn good(_t: &str) -> Result<VerifiedToken, Code> {
        Ok(VerifiedToken {
            user_id: "u1".into(),
            participant_type: 0,
            hidden: false,
            permission: Permission::default(),
        })
    }

    fn expired(_t: &str) -> Result<VerifiedToken, Code> {
        Err(Code::TokenExpired)
    }

    fn h(op: Op, pid: u32) -> Header {
        Header::new(Kind::Request, op, pid)
    }

    fn bind_body(session_id: Option<&str>) -> Vec<u8> {
        let r = BindReq {
            token: "t".into(),
            session_id: session_id.map(|s| s.to_string()),
            client_ver: None,
            pc_mode: None,
        };
        serde_json::to_vec(&r).expect("ser")
    }

    #[test]
    fn bind_전의_다른_op_은_2001_이다() {
        let mut c = Conn::Unbound { opened_at: 0 };
        let mut s = Sessions::new();
        let r = dispatch(&mut c, &mut s, &good, 60_000, 0, h(Op::RoomJoin, 1), b"{}");
        let Reply::Fail { body, .. } = r else { panic!("실패여야 한다") };
        let f: Failure = serde_json::from_slice(&body).expect("failure");
        assert_eq!(f.code, Code::NotBound.as_u16());
    }

    #[test]
    fn bind_가_안_오면_끊는다() {
        let c = Conn::Unbound { opened_at: 0 };
        assert!(bind_overdue(&c, BIND_DEADLINE_MS - 1).is_none());
        let n = bind_overdue(&c, BIND_DEADLINE_MS).expect("끊는다");
        assert_eq!(n.code, Code::HeartbeatTimeout.as_u16());
    }

    #[test]
    fn 응답에는_답하지_않는다() {
        // ★답하면 둘이 서로를 낳는다(정§9-6 이 발언권 축에서 실측한 그 증폭).
        let mut c = Conn::Bound { session_id: "s-1".into() };
        let mut s = Sessions::new();
        let ack = Header { kind: Kind::Ok, reserved: 0, op: Op::TrackEvent, pid: 1 };
        assert_eq!(dispatch(&mut c, &mut s, &good, 60_000, 0, ack, b""), Reply::Silent);
    }

    #[test]
    fn 세션이_이어지면_같은_번호가_돌아온다() {
        let mut c = Conn::Unbound { opened_at: 0 };
        let mut s = Sessions::new();
        let r = dispatch(&mut c, &mut s, &good, 60_000, 0, h(Op::Bind, 1), &bind_body(None));
        let Reply::Ok { body, header } = r else { panic!("성공이어야 한다") };
        assert_eq!(header.kind, Kind::Ok);
        assert_eq!(header.pid, 1, "★받은 번호를 그대로 되돌린다");
        let res: BindRes = serde_json::from_slice(&body).expect("res");
        let first = res.session_id.clone();

        // 끊겼다 창 안에 다시 온다 — ★토큰이 만료돼도 판정 1 이 이긴다.
        s.on_socket_dead(&first, 1_000);
        let mut c2 = Conn::Unbound { opened_at: 2_000 };
        let r2 = dispatch(&mut c2, &mut s, &expired, 60_000, 2_000, h(Op::Bind, 9), &bind_body(Some(&first)));
        let Reply::Ok { body, .. } = r2 else { panic!("이어받아야 한다") };
        let res2: BindRes = serde_json::from_slice(&body).expect("res");
        assert_eq!(res2.session_id, first, "★같으면 세션이 살아 있다는 뜻이다");
    }

    /// ★**옛 축출 기록이 다음 판정에 새면 안 된다**(실측 20260912).
    #[test]
    fn 축출_기록은_한_번만_쓰인다() {
        let mut s = Sessions::new();
        let mut c1 = Conn::Unbound { opened_at: 0 };
        // ①같은 사람이 이미 붙어 있다 — 이 BIND 가 그를 축출한다.
        dispatch(&mut c1, &mut s, &good, 60_000, 1, h(Op::Bind, 1), br#"{"token":"t"}"#);
        let mut c2 = Conn::Unbound { opened_at: 0 };
        let before = c2.clone();
        dispatch(&mut c2, &mut s, &good, 60_000, 2, h(Op::Bind, 1), br#"{"token":"t"}"#);
        let first = evicted_by(&before, &c2, &mut s);
        assert!(matches!(first, Some(Evicted::Session(_))), "{first:?}");

        // ②그 다음 이어받기(판정 1) — ★**옛 기록이 아니라 이 소켓의 자리**가 답이어야 한다.
        let Conn::Bound { session_id } = c2.clone() else { panic!("붙었다") };
        let mut c3 = Conn::Unbound { opened_at: 0 };
        let before = c3.clone();
        // ★토큰은 형식상 필수다 — 판정 1 은 그 값을 **보지 않는다**(정§3-2).
        let body = format!(r#"{{"token":"expired","session_id":"{session_id}"}}"#);
        dispatch(&mut c3, &mut s, &good, 60_000, 3, h(Op::Bind, 1), body.as_bytes());
        assert_eq!(
            evicted_by(&before, &c3, &mut s),
            Some(Evicted::Socket(session_id)),
            "★기록을 안 비우면 여기서 이미 죽은 옛 세션을 닫으라고 답한다"
        );
    }

    #[test]
    fn 새_세션에_resume_은_2008_이다() {
        let mut c = Conn::Unbound { opened_at: 0 };
        let mut s = Sessions::new();
        dispatch(&mut c, &mut s, &good, 60_000, 0, h(Op::Bind, 1), &bind_body(None));
        let r = dispatch(&mut c, &mut s, &good, 60_000, 1, h(Op::Resume, 2), b"");
        let Reply::Fail { body, .. } = r else { panic!("2008 이어야 한다") };
        let f: Failure = serde_json::from_slice(&body).expect("f");
        assert_eq!(f.code, Code::SessionNotFound.as_u16());
    }

    #[test]
    fn 하트비트는_빈_응답이다() {
        let mut c = Conn::Unbound { opened_at: 0 };
        let mut s = Sessions::new();
        dispatch(&mut c, &mut s, &good, 60_000, 0, h(Op::Bind, 1), &bind_body(None));
        let r = dispatch(&mut c, &mut s, &good, 60_000, 1, h(Op::Heartbeat, 2), b"");
        assert_eq!(r, Reply::Ok { header: Header { kind: Kind::Ok, reserved: 0, op: Op::Heartbeat, pid: 2 }, body: Vec::new() });
    }

    #[test]
    fn 클라_leave_는_즉시_폐기다() {
        let mut c = Conn::Unbound { opened_at: 0 };
        let mut s = Sessions::new();
        let Reply::Ok { body, .. } = dispatch(&mut c, &mut s, &good, 60_000, 0, h(Op::Bind, 1), &bind_body(None))
        else {
            panic!("bind")
        };
        let res: BindRes = serde_json::from_slice(&body).expect("res");
        let r = dispatch(&mut c, &mut s, &good, 60_000, 1, h(Op::Leave, 2), b"");
        assert_eq!(r, Reply::Silent, "★응답 없는 op 이다");
        // ★창을 안 잡는다 — 뜻을 갖고 끝낸 것이라 "죽었나 끊었나"를 가릴 필요가 없다.
        assert!(s.get(&res.session_id).is_none());
    }

    #[test]
    fn 못_읽은_프레임은_leave_로_닫는다() {
        // ★`op`·`pid` 를 못 믿으면 응답 프레임을 지을 수가 없다.
        let Reply::Close(n) = on_decode_error(DecodeError::ReservedKind) else { panic!("닫는다") };
        assert_eq!(n.code, Code::ProtocolError.as_u16());
    }
}
