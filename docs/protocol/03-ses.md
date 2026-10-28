[← 차례](README.md)

## 3. SES — 세션

클라 접속 하나. 인증 · 하트비트 · 재접속(이어받기)의 주인

| 정체성 | 누가 내나 | 무엇마다 | 수명 | 상태 |
|---|---|---|---|---|
| session_id | server | 클라 접속 하나 | BIND 성공 → LEAVE · 이어받기 창(resume_window_ms) 초과 | DISCONNECTED · CONNECTING · ACTIVE · RESUMING |

| 의존 | 기댐 | 다중도 | 뜻 |
|---|---|---|---|
| LINK | SES | N:1 | 연결은 세션 안에서만 산다 — Peer 수명 = 세션 수명 |
| ROOM | SES | N:1 | 참여는 세션의 것 — RESUMING 동안 그대로 두고 snapshot 으로 정리한다(§4-4-1) |
| AFF | SES | 1:1 | 소속은 세션 하나에 하나 |

### 3-1. 구조

<a id="SES.op.BIND"></a>
#### 3-1-1. `0x0101 BIND` — 접속한다   **C→S** · 응답 있음
단 1 · 출처 §3-1

**요청**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `token` | str | `session_id` 갈래가 아닐 때 — ★클라는 늘 싣는다(`session_id` 가 무효면 토큰 갈래로 떨어진다 · §3-1-1) | JWT. ★**body 로 보낸다** — 접속 주소 질의값에 안 싣는다. ★서버는 유효한 `session_id` 면 토큰을 보지 않는다 — 없어도 실패가 아니다 |
| ★`session_id` | str | — | ★**재접속 이어받기용**(§3-2-3). 끊기기 전 세션의 것 — 아래 "인증 두 갈래" |
| `client_ver` | int | — (기본 1) | 클라 프로토콜 세대. ★**이 규격서의 세대는 `1` 이다.** 안 보내면 `1` 로 본다 |
| ★`pc_mode` | "1pc" / "2pc" | — (기본 "2pc") | `"1pc"` / `"2pc"`. ★**세션 단위**다 — 아래. 안 보내면 `"2pc"` |

**응답**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `user_id` | str | ✔ |  |
| `server_ver` | u32 | ✔ |  |
| `heartbeat_interval` | u64 | ✔ | ms — 이 주기로 0x0103 을 보낸다 |
| `session_id` | str | ✔ | 재접속 BIND 에 실을 값 (§3-1 인증 두 갈래 · §3-2-3) |
| `resume_window_ms` | u64 | ✔ | 이 시간 안에 돌아오면 이어받을 수 있다 |
| `pc_mode` | "1pc" / "2pc" | ✔ | ★확정값 에코 — 아래 |

**실패** — `1002` ★**모르는 `pc_mode` 포함** · `1004` `VERSION_MISMATCH`(`client_ver` 가 서버가 받을 수 없는 세대다) · `2002` · `2003` · `4003` ★**이 배포**의 동시 세션 수 상한(§11) · `1008` ★**이미 묶인 소켓**의 두 번째 `BIND` — 응답 뒤 `LEAVE` 1008 로 끊는다(§3-1-1). ★`2005` 는 ★**여기 없다** — 발급(HTTP §3-1-5) 때만 난다. ★**전부 연결이 안 선다.**

<a id="EX-SES-BIND-RES"></a>**EX-SES-BIND-RES** · 예시(비규범)

→ 구조 — [`SES.example.EX-SES-BIND-RES`](#EX-SES-BIND-RES) · [`SES.op.BIND`](#SES.op.BIND)

예시 — `BIND` 응답(형과 대조된 값 · 주석은 칸 설명)

```jsonc
{ "user_id": "u1",
  "server_ver": 1,
  "heartbeat_interval": 10000,   // ms — 이 주기로 0x0103 을 보낸다
  "session_id": "…",             // 재접속 BIND 에 실을 값 (§3-1 인증 두 갈래 · §3-2-3)
  "resume_window_ms": 60000,     // 이 시간 안에 돌아오면 이어받을 수 있다
  "pc_mode": "1pc" }             // ★확정값 에코 — 아래
```

<a id="RULE-SES-BIND-PROC"></a>**RULE-SES-BIND-PROC** · 규칙

**절차**
- 클라 ★**연결되면 이것부터 보낸다.** 서버가 먼저 말하지 않는다.
- 클라 응답을 받기 전에는 **다른 요청을 보내지 않는다.** ★**예외가 없다** — 재접속의 `RESUME` 도 `BIND` 응답을 받은 뒤 보낸다(§3-2-3, 직렬). ★`HEARTBEAT` 도 같다 — 윈도우 밖(§2-2-1)은 「앞 요청을 기다리지 않는다」는 뜻이지 `BIND` 응답 전 소켓에 보내도 된다는 뜻이 아니다. 그 소켓에 나가면 아래 규칙대로 끊긴다 — WS 점검(§9-2-1 4)이 늦게 나가는 길(간격을 벌린 다시 점검 등)은 ★**보내는 순간에 세션이 `ACTIVE` 인지 다시 본다**(아니면 세션 층이 먼저다 — §9-2-1). 파이프라인을 두면 응답 순서 조합과 서버측 인증 전 큐가 생기고, `BIND` 가 새 세션을 내면 뒤따른 `RESUME` 의 `2008` 을 버려야 하는 자리가 생긴다 — 1 RTT 를 사려고 예외 셋을 파는 것이라 하지 않는다.
- 서버 ★**BIND 전의 다른 op 은 받지 않고 끊는다** — `LEAVE` `2001 NOT_BOUND` 를 보내고 닫는다(클라 버그다 — 바로 위 규칙대로 짓는 클라는 이 사유를 받을 수 없다). 응답만 하고 붙여 두지 않는다(선례가 모두 끊는다 — Discord Gateway 4003 · graphql-ws 4401 · RFC 6120 §4.3.5). ★**10초** 안에 BIND 가 안 오면 끊는다(`LEAVE` `4008 HEARTBEAT_TIMEOUT`, §3-1).
- 서버 ★**이미 묶인 소켓의 두 번째 `BIND` 는 프로토콜 위반이다** — 실패 `1008` 를 답하고 `LEAVE` `1008` 로 끊는다(§2-1-2 「`op` · `pid` 는 읽히나 대화가 이미 어긋났다 — 응답과 `LEAVE` 둘 다」와 같은 꼴 · 위 「BIND 전 다른 op」의 짝 · 선례 MQTT 3.1.1 §3.1 — 두 번째 CONNECT 는 프로토콜 위반으로 끊는다). 한 소켓 안에서 세션을 바꾸지 않는다 — 바꾸면 그 소켓의 방 · 흐름 상태가 어느 세션 것인지 흐려진다. 이 규칙대로 짓는 클라는 이 사유를 받을 수 없다.
- 서버 `user_id` 는 토큰에서 온다.

→ 구조 — [`SES.C.CONNECTING.open`](#SES.C.CONNECTING.open) · [`SES.S.pre_bind_other`](#SES.S.pre_bind_other) · [`SES.S.bind_bound`](#SES.S.bind_bound) · [`SES.C.any.TBind`](#SES.C.any.TBind)

<a id="RULE-SES-TOKEN-NO-EXPIRY"></a>**RULE-SES-TOKEN-NO-EXPIRY** · 규칙

★**토큰은 접속 중에 만료되지 않는다.** 접속 시점에만 검사한다 — 무전은 종일 켜두는 물건이라
백그라운드에서 갱신 왕복을 지킬 수 없다. ★**들여보낸 사람을 내보내려면 운영자가 연결을 끊는다**(`LEAVE` `2009 SESSION_REVOKED`, 아래) — ★토큰의 `permission`(§3-1-5)은 입장 초기값이라 접속 중 다시 읽지 않고, 비트 회수는 방 단위 갱신(§4-2-6 · §4-2-6 `PARTICIPANT_STATE`)으로 따로 간다.
★**이것은 WS 세션 얘기다** — HTTP 는 매 요청 검사하므로 §2-2-3 의 세션 헤더가 따로 있다.

→ 구조 — [`SES.C.any.token_expired`](#SES.C.any.token_expired)

<a id="RULE-SES-AUTH-TWO-WAYS"></a>**RULE-SES-AUTH-TWO-WAYS** · 규칙

★★**인증은 두 갈래다 — `session_id` 가 유효하면 그것이 이긴다.**

| 갈래 | 규칙 |
|---|---|
| ★**`session_id` 가 실려 있고 유효하다**(`resume_window_ms` 안·서버 생존) | ★**그 세션의 신원으로 인증한다. 토큰은 보지 않는다** — 종일 켜둔 클라의 재접속 시점 토큰은 거의 확실히 만료 상태이고, 그때마다 앱 백엔드 왕복을 요구하면 ★**60초 윈도우 안에 미디어 복구가 안 된다.** 근거는 §2-2-3 세션 헤더와 같다 — ★**새 비밀을 만들지 않는다.** `session_id` 는 이미 재접속의 자격증명이다 |
| `session_id` 가 없거나 무효다 | `token` 으로 인증한다(만료면 `2003`). ★**무효는 실패가 아니다** — 새 세션으로 붙는다 |

→ 구조 — [`SES.S.bind_resume`](#SES.S.bind_resume) · [`SES.S.bind_new`](#SES.S.bind_new) · [`SES.C.RESUMING.bind_same`](#SES.C.RESUMING.bind_same) · [`SES.C.RESUMING.bind_new`](#SES.C.RESUMING.bind_new)

<a id="RULE-SES-WINDOW-NEEDS-BACKEND"></a>**RULE-SES-WINDOW-NEEDS-BACKEND** · 규칙

★★**그래서 창을 넘기면 앱 백엔드 왕복이 필요하다 — 이것은 제약이고, 알고 두는 것이다.**
창 안에서는 `session_id` 가 토큰을 대신하지만, 넘기면 갈래 2 로 떨어져 ★**만료된 토큰이 `2003` 을 받는다.**
그때 SDK 가 할 수 있는 것은 ★**앱에 새 토큰을 달라고 하는 것뿐**이다 —
★**SDK 는 토큰을 스스로 못 얻는다**(발급은 앱 백엔드 몫, §3-1-5).

<a id="RULE-SES-RESUME-BY-SESSION-ID"></a>**RULE-SES-RESUME-BY-SESSION-ID** · 규칙

★★**응답의 `session_id` 로 이어받기 성부를 안다.** ★**내가 보낸 것과 같으면** 세션이
살아 있다 — 그 응답을 받은 뒤 `RESUME` 을 보낸다(§3-2-3). ★**다르면** 새 세션이다 — 이어받을 것이
없으므로 ★**`RESUME` 을 보내지 않고 전면 재구축으로 간다**(§3-2-3). 세션을
이어받았으면 `session_id` 는 ★**바뀌지 않는다** — BIND 가 새로 발급하는 것은 새 세션일 때뿐이라,
*"어느 것이 내 세션인가"* 가 갈릴 일이 없다.

→ 구조 — [`SES.C.RESUMING.bind_same`](#SES.C.RESUMING.bind_same) · [`SES.C.RESUMING.bind_new`](#SES.C.RESUMING.bind_new)

<a id="RULE-SES-PC-MODE-SESSION"></a>**RULE-SES-PC-MODE-SESSION** · 규칙

★★**`pc_mode` 는 세션 단위다.**

| | |
|---|---|
| 무엇의 속성인가 | ★**브라우저 능력**이다. 연결을 몇 개 세울 수 있느냐이므로 ★**방도 서버도 아니고 클라 자신**의 성질이다 |
| ★**왜 여기인가** | ★**클라가 붙기 전에 아는 유일한 값**이기 때문이다. `ROOM_JOIN` 에 실으면 순환이다 — 그 방이 어느 미디어 서버인지는 ★**`ROOM_JOIN` 응답이 와야** 알므로(§9-1-1), 요청 시점에 *"서버마다 다른 값"* 을 실을 방법이 **원리적으로 없다** |
| 그 뒤 | ★**세션 안에서 바뀌지 않는다.** 바꾸려면 다시 붙는다 |
| ★**확인** | ★**응답이 확정값을 에코한다.** 그리고 각 미디어 서버가 적용한 값이 `server_config.pc_mode`(§9-1-1) 로 다시 온다 — ★**둘은 반드시 같다** |
| 안 보내면 | `"2pc"` 로 본다 |
| 모르는 값 | ★**거절한다**(`1002`). ★**조용히 기본값으로 넘어가지 않는다** — 오타가 `2pc` 로 등록되면 클라는 `1pc` 로 도는 줄 알고 ★**폴백이 일어난 것도 모른다** |

<a id="RULE-SES-PC-MODE-DECIDE"></a>**RULE-SES-PC-MODE-DECIDE** · 규칙

★**어느 값으로 붙을지 정하는 법은 §9-4-20** 다. ★**붙기 전에 정한다.**

<a id="RULE-SES-BIND-FAIL"></a>**RULE-SES-BIND-FAIL** · 규칙

→ 구조 — [`SES.op.BIND`](#SES.op.BIND)

<a id="RULE-SES-EVICT-OLD"></a>**RULE-SES-EVICT-OLD** · 규칙

살아 있던 옛 세션은 ★**`LEAVE` `2010 DUPLICATE_SESSION`**(아래)으로 닫힌다
★`2003`(만료)만 **새 토큰을 받아 다시 붙으면 된다** — 나머지는 다시 붙어도 같다(§3-2-2).

→ 구조 — [`SES.S.bind_new`](#SES.S.bind_new) · [`SES.S.bind_resume`](#SES.S.bind_resume) · [`ROOM.S.session_gone`](04-room.md#ROOM.S.session_gone)

<a id="SES.op.RESUME"></a>
#### 3-1-2. `0x0102 RESUME` — 시그널만 끊겼다 다시 붙었다   **C→S** · 응답 있음
단 1 · 출처 §3-1

**요청**
— 빈 body

**응답**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `snapshot` | {방: RoomSnapshot} | ✔ | ★그 세션이 아는 방 전부 · 방마다 version (§4-1-6) |
| `publications` | Publication[] | ✔ | ★발행 층 — 서버가 아는 내 발행 전량 (§6-1-1) |
| ★`mline_tables` | MlineTable[] | ✔ | ★**서버마다 구독 m-line 배정표 전체**(§8-1-2) — 받는 법은 `MLINE_EVENT` 와 같다(§8-1-2) |

**실패** — `2008` ★세션이 이미 지워졌다 · `5002` ★그 세션의 방을 든 미디어 서버가 답하지 않았다 — 방을 뺀 스냅샷을 성공으로 내지 않는다 `2008` 은 세션이 이미 지워졌다(`resume_window_ms` 초과·서버 재시작·새 세션에 보냄) · `5002` 는 세션은 있으나 그 세션의 방을 든 미디어 서버 하나라도 답하지 않았다. ★**둘 다 전면 재구축으로 간다**(§3-2-3) — `ROOM_JOIN` 부터 전부 다시. ★**부분 실패는 없다** — 방 하나가 실패하는 응답 형이 없고, 방을 뺀 스냅샷은 클라가 그 방을 내리게 한다(§3-1-2).

<a id="RULE-SES-RESUME-SIGNAL-ONLY"></a>**RULE-SES-RESUME-SIGNAL-ONLY** · 규칙

★★**이것은 "재접속" 이 아니라 "시그널 복구" 다.** 복구는 세 단이고 `RESUME` 은 **1단**이다.

| 단 | 무엇이 죽었나 | 무엇을 하나 | 우리 규격 |
|---|---|---|---|
| **1. 시그널 복구** | ★**WS 만.** 미디어(ICE·DTLS·RTP)는 살아 있다 | ★`RESUME` — 방·트랙을 그대로 두고 새 소켓에 다시 물린다 | ★**이 op** |
| **2. ICE 재시작** | 미디어 경로만 — ★**서버 하나의 연결** | 서버가 새 ICE 자격을 주고(`ICE_RESTART`, §7-1-2) 클라가 그 자격으로 다시 협상한다. ★**DTLS·SCTP·구독·발행·발언권은 그대로다** | ★**§9-2-1** |
| **3. 재구축** | 서버가 그 연결을 모른다, 또는 세션 자체 | `ROOM_JOIN` 부터 다시 — ★**두 갈래다**: ①★**그 서버만**(Peer 없음 `3010` · SCTP 연관 소멸 · 스냅샷에 그 서버 방이 없음 — §9-2-1) ②★**세션 전체**(세션이 사라졌다 — `2008` · 새 세션 · 창 초과, §3-2-3) | 두 갈래 |

<a id="RULE-SES-RESUME-NO-SESSION-ID"></a>**RULE-SES-RESUME-NO-SESSION-ID** · 규칙

★**`session_id` 필드는 없다.** 세션은 ★**`BIND` 가 이미 특정했다**(§3-1 인증 두 갈래 —
재접속 `BIND` 에 실려 온다). 여기 또 실으면 ★**둘이 다를 때의 갈래**가 생긴다 — 같은 것을
두 번 나르지 않는다. 이어받은 세션이 아닌데(새 세션) `RESUME` 이 오면 `2008` 이다.

→ 구조 — [`SES.op.RESUME`](#SES.op.RESUME)

<a id="EX-SES-RESUME-RES"></a>**EX-SES-RESUME-RES** · 예시(비규범)

→ 구조 — [`SES.example.EX-SES-RESUME-RES`](#EX-SES-RESUME-RES) · [`SES.op.RESUME`](#SES.op.RESUME)

예시 — `RESUME` 응답(형과 대조된 값 · 주석은 칸 설명)

```jsonc
{ "snapshot": { "r1": { "participants": [ {"user_id":"u1","role":255,"select":false,"participant_type":0,"metadata":{"name":"…"}} ],   // MemberInfo[] §4-1-3
                        "me": {"user_id":"u1","role":255,"select":false,"participant_type":0,"metadata":{"name":"…"}},   // ★**나** — 명단과 따로 싣는다(투명이든 아니든 늘 · 권한 포함 — ★내 권한의 출처는 이것이다 · DEV-ROOM-ME-APART)
                        "tracks": [ /* TrackEntry… §6-1-1 */ ],
                        "version": {"epoch":"sfu-7f3a…","seq":10245} } },   // ★그 세션이 아는 방 전부 · 방마다 version (§4-1-6)
  "publications": [ {"track_id":"t-u1-mic","room_id":"r1","kind":"audio","duplex":"half","codec":null,"fmtp":"…","simulcast":false} ],   // ★발행 층 — 서버가 아는 내 발행 전량 (§6-1-1)
  "mline_tables": [ { "sfu_id": "sfu-7f3a…",            // ★**어느 미디어 서버의 연결 표인가**(§9-1-1 — 재기동하면 바뀐다)
                      "arev": 3,                        // ★**표의 차례 번호** — 그 서버가 이 세션의 표를 바꿀 때마다 하나 오른다(재구축해도 이어진다 · 재기동하면 `sfu_id` 가 바뀐다). ★**같은 `sfu_id` 에서 보관값보다 큰 것만 받는다** — 표 전체라 낡은 것은 버리면 된다(사이를 메울 것이 없다)
                      "mlines": [ /* MlineEntry… §8-1-1 */ ] } ] }   // ★**표 전체** — 그 연결의 구독 m-line 전부(빈 줄 포함 · mid 순서는 받는 쪽이 수치로 세운다) · ★**서버마다 구독 m-line 배정표 전체**(§8-1-2) — 받는 법은 `MLINE_EVENT` 와 같다(§8-1-2)
```

<a id="RULE-SES-RESUME-RES-TWO"></a>**RULE-SES-RESUME-RES-TWO** · 규칙

★**응답은 `snapshot` · `publications` · `mline_tables` 셋이다**(구조 SES.op.RESUME — 서버마다 배정표 전체 · §8-1-2). `resumed[]`·`failed[]`·`reason{}`·`publish_failed[]` 는 ★**없다** — 방 단위 판정이 없어졌다.
★**`publications[]` 는 발행 층**(§6-1-1) — 서버가 아는 내 발행(전이중·반이중 다, `PUBLISH_TRACKS` 응답의 `track_id` 로 식별)이다. 트랙 층에는
반이중 발행이 없어(§6-2-2 대응 표) 스냅샷만으로는 내 무전 마이크가 서버에 살아 있는지 대조할 수 없다 — 규칙 ⑤(내 발행 상태는 나에게만 온다)의 자리다.

→ 구조 — [`SES.op.RESUME`](#SES.op.RESUME)

<a id="RULE-SES-RESUME-PROC"></a>**RULE-SES-RESUME-PROC** · 규칙

**절차**

| 주체 | |
|---|---|
| 클라 | `epoch` 가 다르면 그 방 보관본을 통째로 버린다(규칙 1 — 미디어 서버가 재기동됐다) |
| 클라 | ★★**자기 미디어와 대조한다**(§3-2-3) — 미디어 연결이 전부 살아 있고, 들어가 있던 방이 스냅샷에 다 있고, 내 발행이 `publications[]` 에 다 있으면 ★**이어 쓴다**(남의 트랙이 달라졌으면 구독 SDP 만 다시 조립한다 · 그대로면 재협상도 `READY` 도 없다). ★**어긋난 것은 서버마다 가른다**(§3-2-3) — 스냅샷에 빠진 방은 ★**내리고 다시 들어가지 않는다**, 죽은 연결은 2단, 서버가 모르는 연결은 그 서버만 3단(남은 방만 다시 들어간다) |
| 클라 | ★**보내다 만 프레임을 다시 보내지 않는다.** 큐를 비운다 |

→ 구조 — [`SES.C.RESUMING.bind_same`](#SES.C.RESUMING.bind_same) · [`PROP-SES-RESUMING-BIND-THEN-RESUME`](#PROP-SES-RESUMING-BIND-THEN-RESUME) · [`PROP-SES-RESUME-NEEDS-CARRY`](#PROP-SES-RESUME-NEEDS-CARRY) · [`SES.C.any.drop`](#SES.C.any.drop) · [`SES.C.RESUMING.open_late`](#SES.C.RESUMING.open_late) · [`SES.op.RESUME`](#SES.op.RESUME) · [`SES.S.resume_ok`](#SES.S.resume_ok) · [`ROOM.type.RoomSnapshot`](04-room.md#ROOM.type.RoomSnapshot) · [`ROOM.C.JOINED.resume_snapshot`](04-room.md#ROOM.C.JOINED.resume_snapshot) · [`SES.C.RESUMING.resume_ok`](#SES.C.RESUMING.resume_ok) · [`SES.C.RESUMING.resume_ok_repair`](#SES.C.RESUMING.resume_ok_repair)

<a id="RULE-SES-RESUME-FAIL"></a>**RULE-SES-RESUME-FAIL** · 규칙

→ 구조 — [`SES.op.RESUME`](#SES.op.RESUME)

<a id="SES.op.HEARTBEAT"></a>
#### 3-1-3. `0x0103 HEARTBEAT` — 하트비트   **양방향** · 응답 있음
단 1 · 윈도우 밖 · 출처 §3-1

**요청**
— 빈 body

**응답**
— 빈 body

<a id="RULE-SES-HEARTBEAT"></a>**RULE-SES-HEARTBEAT** · 규칙

**절차**
- ★**서버가 먼저 보낼 수도 있다.** 그때 클라는 ★**다른 통지와 똑같이 ACK 만** 한다(§2-2-5) — 자기 주기를 앞당기거나 미루지 않는다.
- ★**WS 점검에도 쓴다**(§9-2-1 4) — 주기와 상관없이 곧바로 보낸다. ★단 `BIND` 응답을 받은 소켓에만 — 그 전에는 다른 op 과 같이 끊긴다(§3-1-1).
★서버는 ★**읽는 즉시 답한다** — 방 요청의 처리를 기다리지 않는다.

→ 구조 — [`SES.op.HEARTBEAT`](#SES.op.HEARTBEAT) · [`SES.C.any.THb`](#SES.C.any.THb) · [`SES.state`](#SES.state)

<a id="SES.op.LEAVE"></a>
#### 3-1-4. `0x0104 LEAVE` — 끝낸다   **양방향** · 응답 없음
단 1 · 출처 §3-1

**요청**
— 빈 body

**통지 · 받는 꼴**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `code` | u16 | ✔ |  |
| `name` | str | ✔ |  |
| `message` | str | — |  |
| `details` | json | — |  |

통지 body = §11-1-1 `Failure` 형에서 `permanent` 를 뺀 것 (IR 이 칸 · 형 · 필수를 대조한다)

<a id="RULE-SES-LEAVE-CARRIES-REASON"></a>**RULE-SES-LEAVE-CARRIES-REASON** · 규칙

★★**연결 종료의 사유는 이 프레임이 나른다** — WS Close 가 아니다. §10 이 MBCP 를 옮기며 ★**의미론**(사유)과 ★**전달수단**(소켓
닫힘)을 가른 것과 같은 규율이다 — 전송을 TCP 직결로 바꿔도 이 op 은 그대로 선다. WS Close 에는 코드를 두지 않는다(정상 종료 `1000` ·
그 밖은 아무 코드 — 클라는 보지 않는다). ★**채널은 이것 하나다** — 둘에 나눠 실으면 어긋난다.

→ 구조 — [`SES.S.leave`](#SES.S.leave) · [`SES.C.any.drop_final`](#SES.C.any.drop_final) · [`SES.C.any.drop`](#SES.C.any.drop)

<a id="RULE-SES-LEAVE-FORM"></a>**RULE-SES-LEAVE-FORM** · 규칙

→ 구조 — [`SES.op.LEAVE`](#SES.op.LEAVE) · [`ERR.type.Failure`](11-err.md#ERR.type.Failure) · [`SES.fact.LeaveReasons`](#SES.fact.LeaveReasons)

<a id="RULE-SES-LEAVE-ONE-NUMBER-SPACE"></a>**RULE-SES-LEAVE-ONE-NUMBER-SPACE** · 규칙

→ 구조 — [`SES.fact.LeaveReasons`](#SES.fact.LeaveReasons)

<a id="SES.http.TOKEN"></a>
#### 3-1-5. `POST /auth/token` — 토큰 발급
누가: 앱 백엔드 · 인증: key · 성공 `200` · CORS 안 붙는다

**요청**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `api_key` | str | ✔ |  |
| `api_secret` | str | ✔ |  |
| `user_id` | str | ✔ |  |
| ★`participant_type` | u8 | — (기본 0) | ★**무엇인가** — `0` 사람 · `1` 녹화 · `2` 봇(그 밖의 자동 참가자). ★**명단에 그대로 실린다**(§4-1-3) — 앱이 *"녹화 중"·"봇이 듣고 있다"* 를 보일 수 있다 |
| ★`hidden` | bool | — (기본 false) | ★**명단에 보이나** — `true` 면 명단·입퇴장 통지·정원에서 전부 빠진다(§4-1-3·§4-1-12) |
| `metadata` | json | — | ★`metadata` = **자유 JSON — 사람의 신원**(이름·프로필·소속·호출부호 …). 서명되어 명단에 그대로 실린다, §4-1-3 |
| ★`permission` | 객체 | — | ★**무엇을 할 수 있나** — `publish_audio`·`publish_video`·`publish_screen`·`floor_request`(§4-1-3). ★**입장 초기값이고 그 뒤로는 서버 밖의 결정자가 방 단위로 바꾼다**(§4-2-6 · §4-2-6 `PARTICIPANT_STATE`). ★그 방이 기억하는 갱신값이 있으면 그것이 이 초기값보다 앞선다(§4-2-6) |

**응답**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `token` | str | ✔ |  |
| `expires_in` | u32 | ✔ | `expires_in` = 초 |

**실패** — `2004`(401) `2004 INVALID_API_KEY`(401) · `2005`(400) `CLAIM_NOT_ALLOWED` — ★**종류**·**투명**이 계정 허용 밖 · `1002`(400) `INVALID_PAYLOAD` — `metadata` 상한 초과 · 모르는 `participant_type` · `5003`(500) `5003 INTERNAL_ERROR`(500)

<a id="RULE-SES-TOKEN-FORM"></a>**RULE-SES-TOKEN-FORM** · 규칙 — `POST /auth/token` — 토큰 발급

→ 구조 — [`SES.http.TOKEN`](#SES.http.TOKEN)

<a id="RULE-SES-TOKEN-CLAIMS"></a>**RULE-SES-TOKEN-CLAIMS** · 규칙

★★**참가자 종류와 투명은 다른 축이다 — 둘 다 토큰이 정한다.**

| 클레임 | 형 | 기본 | 뜻 |
|---|---|---|---|
| ★`participant_type` | u8 | `0` | ★**무엇인가** — `0` 사람 · `1` 녹화 · `2` 봇(그 밖의 자동 참가자). ★**명단에 그대로 실린다**(§4-1-3) — 앱이 *"녹화 중"·"봇이 듣고 있다"* 를 보일 수 있다 |
| ★`hidden` | bool | `false` | ★**명단에 보이나** — `true` 면 명단·입퇴장 통지·정원에서 전부 빠진다(§4-1-3·§4-1-12) |
| ★`permission` | 객체 | 넷 다 `true` | ★**무엇을 할 수 있나** — `publish_audio`·`publish_video`·`publish_screen`·`floor_request`(§4-1-3). ★**입장 초기값이고 그 뒤로는 서버 밖의 결정자가 방 단위로 바꾼다**(§4-2-6 · §4-2-6 `PARTICIPANT_STATE`). ★그 방이 기억하는 갱신값이 있으면 그것이 이 초기값보다 앞선다(§4-2-6) |

→ 구조 — [`SES.http.TOKEN`](#SES.http.TOKEN)

<a id="RULE-SES-TOKEN-AXES-SEPARATE"></a>**RULE-SES-TOKEN-AXES-SEPARATE** · 규칙

★**둘을 묶지 않는다.** 녹화가 곧 투명이 아니다 — ★**보이는 녹화**(`participant_type:1, hidden:false`)가
지령대 운용의 기본이고(대원이 녹화 중임을 안다), ★**투명한 감시**(`hidden:true`)는 그것과 다른 결정이다.
한 값에 두 뜻을 실으면 둘 중 하나를 못 고른다.

<a id="RULE-SES-TOKEN-ACCOUNT-ALLOW"></a>**RULE-SES-TOKEN-ACCOUNT-ALLOW** · 규칙

★**계정이 낼 수 있는 것만 서명된다** — 계정 목록에 허용된 종류·투명 여부가 아니면 ★**발급을 거절한다**(`2005`).
클라가 스스로 투명해지는 경로는 ★**없다** — 요청에 그런 자리를 만들지 않는다(§4-1).

→ 구조 — [`SES.http.TOKEN`](#SES.http.TOKEN)

<a id="RULE-SES-TOKEN-NO-FLOOR-POLICY"></a>**RULE-SES-TOKEN-NO-FLOOR-POLICY** · 규칙

★**토큰은 발언권 정책을 나르지 않는다.** 우선순위는 요청이 실은 값(§10-4-31 TLV `0`)이 전부다.
★★**`permission.floor_request` 는 정책이 아니라 자격이다** — *"얼마나 세게 말하나"*(우선순위)가 아니라 *"말을 걸 수 있나"* 다.
★**둘을 겸하게 하지 않는다**: 자격이 없으면 ★**긴급도 못 건다**(§10 — 우회 경로가 없다).
★**`metadata` 는 상한이 있다** — 정책 값(토큰 `metadata` 상한 · 기본 **2,048** 바이트). 넘으면 ★**여기서 거절한다**(`1002`) — 발급된 토큰이 방에 들어간 뒤에 걸리는 경로를 만들지 않는다.
응답 `200` `{ token, expires_in }` (`expires_in` = 초)

<a id="RULE-SES-TOKEN-BY-BACKEND"></a>**RULE-SES-TOKEN-BY-BACKEND** · 규칙

**절차** — ★**앱 백엔드가 부른다.** 클라가 직접 부르지 않는다(비밀키가 클라에 있으면 안 된다).
받은 토큰을 앱이 클라에 넘긴다.

<a id="RULE-SES-TOKEN-FAIL"></a>**RULE-SES-TOKEN-FAIL** · 규칙

→ 구조 — [`SES.http.TOKEN`](#SES.http.TOKEN)

<a id="DEV-SES-TOKEN-PERMISSION-REQ"></a>**DEV-SES-TOKEN-PERMISSION-REQ** · 자체 확장

`POST /auth/token` 요청은 `permission`(선택 · 기본 넷 다 `true`)을 싣는다 — 클레임 표의 입장 초기값이 요청 칸이다

*원문: §3-1-5 요청 꼴 줄 — permission 이 없다 · 근거: RULE-SES-TOKEN-CLAIMS*

<a id="SES.fact.LeaveReasons"></a>
#### 3-1-6. LEAVE 사유

| `code` | `name` | 언제 | ★다시 붙나 | ★미디어 연결 |
|---|---|---|---|---|
| `1008` | `PROTOCOL_ERROR` | 프레임 손상·크기 초과·버전 불일치(§2-1-2) | 안 한다 — 클라 버그다 | ★닫는다 |
| `2001` | `NOT_BOUND` | ★`BIND` 전에 다른 op 을 보냈다(§3-1) | 안 한다 — 클라 버그다(맞게 지은 클라는 이 사유를 받을 수 없다 — 세션 모형) | ★닫는다 |
| `4006` | `FLOW_TIMEOUT` | 응답(ACK)을 30초 안 줬다(§2-2-1) | 안 한다 — 클라가 못 따라간다 | ★닫는다 |
| `4007` | `FLOW_OVERFLOW` | 밀린 프레임 1,000 초과(§2-2-1) | 안 한다 — 같음 | ★닫는다 |
| `4008` | `HEARTBEAT_TIMEOUT` | 아무것도 안 보냈다(30초 · `BIND` 전 10초) | 백오프(§3-2-3) | ★★**유지**(§3-2-4) — WS 만 죽고 미디어는 살아 있을 수 있다. `RESUME` 이 이어받는다 |
| `2009` | `SESSION_REVOKED` | ★운영자가 이 세션을 끊었다(C 평면 `cut`) | ★**백오프로 붙어도 된다** — 단 세션은 즉시 폐기라 `session_id` 를 싣지 말고 `BIND` 만(`RESUME` 은 `2008`). ★차단이 아니다 — 새 `BIND` 가 토큰을 다시 검사하고 자격이 남아 있으면 들어간다. 막는 자리는 토큰이다 | ★닫는다 — 서버가 Peer 를 이미 회수했다 |
| `2010` | `DUPLICATE_SESSION` | 같은 신원이 다른 데서 들어왔다(§3-1 축출) | ★**붙지 마라** | ★닫는다 |
| `5004` | `SERVER_SHUTDOWN` | 서버가 내려간다 | 백오프(§3-2-3) — 같은 `{base}` 로. LB·DNS 가 산 노드로 보낸다(§2-2-2) | ★닫는다 — 그 서버가 내려간다 |
| ★**모르는 `code`** | — | 서버가 사유를 늘렸다 | ★백오프 | ★**유지** — 그래야 서버가 사유를 늘려도 클라가 안 깨진다. 위험한 것은 `2010` 하나인데 그것은 아는 코드다 |

번호 행은 오류 코드 표(§11)와 한 번호 공간이다 — 오류 코드 표에서 `leave` 표지를 단 행과 번호 · 이름이 같다(IR 이 대조한다)

<a id="RULE-SES-LEAVE-PROC"></a>**RULE-SES-LEAVE-PROC** · 규칙

**절차**

| 주체 | |
|---|---|
| 클라 | 남은 큐를 비우지 않는다 — 보내다 만 프레임은 그대로 버린다 |
| 서버 | ★**받으면 이후 프레임을 받아들이지 않고**(SSH §11.1 과 같은 규율) 방·발행·발언권·Peer 를 축출(§3-1)과 같은 경로로 정리한다: 방마다 `PARTICIPANT_EVENT{left}` 가 `seq++` 와 함께 나간다(방마다 한 번 — 스냅샷이 바뀌었으므로, §4-2-6) |
| 서버 | ★**끊기 전에 보낸다** — 프레임 먼저, 절단 나중. 순서를 바꾸면 클라가 사유를 모른 채 끊긴다 |
| 서버 | ★**best-effort 다** — `1008` 에서 헤더를 못 읽으면 `LEAVE` 만 보내고, `4007` 에서는 못 보낼 수 있다. 못 보내면 클라는 *"사유 없는 끊김"* 으로 간다(아래) |
| 서버 | ★`1008` 만 ★**응답과 `LEAVE` 를 둘 다** 낸다(`op`·`pid` 가 읽힐 때) — 층이 다르다: 응답은 *"그 요청이 왜 죽었나"*, `LEAVE` 는 *"그래서 연결을 닫는다"*. 둘 중 하나만 보내지 않는다 |

→ 구조 — [`SES.C.ACTIVE.disconnect`](#SES.C.ACTIVE.disconnect) · [`SES.C.any.disconnect`](#SES.C.any.disconnect) · [`SES.S.leave`](#SES.S.leave) · [`ROOM.S.session_gone`](04-room.md#ROOM.S.session_gone) · [`FLR.C.any.down`](10-flr.md#FLR.C.any.down) · [`SES.op.LEAVE`](#SES.op.LEAVE) · [`SES.C.any`](#SES.C.any)

<a id="RULE-SES-SOCKET-CLOSE-NORMAL"></a>**RULE-SES-SOCKET-CLOSE-NORMAL** · 규칙

★★**`LEAVE` 없이 소켓이 끊길 수 있다 — 그것이 정상 경로다.** 망 절단·서버 급사·`LEAVE` 를 못 보낸 경우가 다 여기다.
★**WS close 든 TCP RST 든 FIN 이든 클라 동작이 같다** — 이 한 줄이 전송 독립의 실체다.

→ 구조 — [`SES.C.any.drop_carry`](#SES.C.any.drop_carry) · [`SES.C.any.drop`](#SES.C.any.drop)

<a id="SES.state"></a>
#### 3-1-7. 상태 칸 · 시한

##### 상태 — 클라

| 칸 | 처음 | 뜻 |
|---|---|---|
| `cs` | `'DISCONNECTED'` | 연결 상태(§3-4-1) |
| `sock` | `'CLOSED'` | 소켓 |
| `wait` | `'NONE'` | 기다리는 응답 |
| `sid` | `0` | 보관한 session_id — 0 이면 없다(BIND 응답이 준다 · §3-2-2) |
| `sent` | `0` | 이 소켓의 BIND 에 실은 session_id — 응답과 견준다(§3-2-3) |
| `late` | `false` | 끊긴 순간부터의 경과가 resume_window_ms 를 넘었다(§3-2-3 2 · §3-2-3) |
| `tries` | `0` | 백오프 횟수(§3-2-3) |
| `hb` | `false` | 하트비트가 돈다(§3-2-2 3) |
| `hbMs` | `0` | 보관한 하트비트 주기(ms) — BIND 응답의 `heartbeat_interval`(§3-2-2 1) · 0 이면 아직 없다 |
| `winMs` | `0` | 보관한 재개 창(ms) — BIND 응답의 `resume_window_ms` · 끊긴 뒤 경과를 이것과 견준다(§3-2-3 2) |
| `token` | `'VALID'` | 앱이 준 토큰 |
| `pcAsk` | `'2pc'` | BIND 에 싣는 연결 모드 — ★브라우저 능력이라 클라가 안다 · 고르는 법은 원문이 정하지 않는다(RULE-LINK-SDP-1PC-MODE · 모르면 2pc) |
| `pcm` | `'2pc'` | ★세션 확정 연결 모드 — BIND 응답의 에코(RULE-SES-PC-MODE-SESSION) · 세션 안에서 바뀌지 않는다 · ROOM 이 사본 없이 읽는다(RULE-ROOM-JOIN-PC-MODE-CHECK) |

##### 상태 — 서버

| 칸 | 처음 | 뜻 |
|---|---|---|
| `hidden` | `false` | ★투명 사용자다 — 토큰 클레임(§3-1-5 `hidden`) · 어느 방에 들어가도 명단 밖이다(RULE-ROOM-HIDDEN) · 방은 사본 없이 읽는다 |
| `live` | `false` | 세션이 있다 — Peer 도 있다 |
| `sid` | `0` | 그 세션의 session_id — 0 이면 없다 |
| `next` | `1` | 다음에 발급할 session_id |
| `bound` | `false` | 지금 소켓이 그 세션에 물려 있다(BIND 성공 뒤 · 끊기면 풀린다) |
| `rooms` | `false` | 그 세션에 방 · 발행이 있다 — 서버가 아는 이어받을 것(RESUME 스냅샷의 출처) |
| `full` | `false` | 이 배포의 동시 세션 수 상한에 닿아 있다(BIND 의 4003) |
| `pcm` | `'2pc'` | ★그 세션의 연결 모드 — BIND 가 확정한다 · 세션 안에서 바뀌지 않는다(RULE-SES-PC-MODE-SESSION) · 미디어 서버가 사본 없이 적용한다 |
| `resumed` | `false` | ★이 소켓이 session_id 갈래로 붙었다(이어받기 · 넘겨받기) — RESUME 은 이것일 때만 선다 · 토큰 갈래의 새 세션은 2008(§3-1 RESUME) |

##### 값 — 뜻

| 값 형 | 값 | 뜻 |
|---|---|---|
| `Conn` | `DISCONNECTED` | 안 붙어 있다 |
| `Conn` | `CONNECTING` | 소켓은 붙었고 `BIND` 응답을 기다린다 |
| `Conn` | `ACTIVE` | 쓸 수 있다 — 보낼 수 있는 것은 전부 |
| `Conn` | `RESUMING` | 끊길 때 ★**이어받을 것이 있었던** 재접속 — 창을 넘겼으면 이어받기를 건너뛰고 전면 재구축으로 간다(§3-2-3 2) |

##### 전이 — 클라(동작 경우에서 짓는다)

| 지금 | 사건 | 다음 | 라벨 |
|---|---|---|---|
| `DISCONNECTED` | 앱이 접속을 요청했다(§3-2-1) | 그대로 | `SES.C.DISCONNECTED.connect_no_token` |
| `DISCONNECTED` | 앱이 접속을 요청했다(§3-2-1) | `CONNECTING` | `SES.C.DISCONNECTED.connect` |
| `DISCONNECTED` | 앱이 새 토큰을 줬다(§3-2-2 2 · §3-2-3 1 의 요구에 답한다) | 그대로 | `SES.C.DISCONNECTED.new_token` |
| `CONNECTING` | 소켓이 열렸다(§3-2-2 · §3-2-3) | 그대로 | `SES.C.CONNECTING.open` |
| `RESUMING` | 소켓이 열렸다(§3-2-2 · §3-2-3) | 그대로 | `SES.C.RESUMING.open_late` |
| `RESUMING` | 소켓이 열렸다(§3-2-2 · §3-2-3) | 그대로 | `SES.C.RESUMING.open` |
| `CONNECTING` | BIND 응답을 받았다(§3-2-2 · §3-2-2 · §3-2-3) | `ACTIVE` | `SES.C.CONNECTING.bind_ok_rebuild` |
| `CONNECTING` | BIND 응답을 받았다(§3-2-2 · §3-2-2 · §3-2-3) | `ACTIVE` | `SES.C.CONNECTING.bind_ok` |
| `CONNECTING` | BIND 응답을 받았다(§3-2-2 · §3-2-2 · §3-2-3) | `DISCONNECTED` | `SES.C.CONNECTING.bind_2002` |
| `CONNECTING` | BIND 응답을 받았다(§3-2-2 · §3-2-2 · §3-2-3) | `DISCONNECTED` | `SES.C.CONNECTING.bind_2003` |
| `CONNECTING` | BIND 응답을 받았다(§3-2-2 · §3-2-2 · §3-2-3) | `DISCONNECTED` | `SES.C.CONNECTING.bind_1xxx` |
| `CONNECTING` | BIND 응답을 받았다(§3-2-2 · §3-2-2 · §3-2-3) | 그대로 | `SES.C.CONNECTING.bind_fail` |
| `RESUMING` | BIND 응답을 받았다(§3-2-2 · §3-2-2 · §3-2-3) | `DISCONNECTED` | `SES.C.RESUMING.bind_2003` |
| `RESUMING` | BIND 응답을 받았다(§3-2-2 · §3-2-2 · §3-2-3) | `DISCONNECTED` | `SES.C.RESUMING.bind_2002` |
| `RESUMING` | BIND 응답을 받았다(§3-2-2 · §3-2-2 · §3-2-3) | 그대로 | `SES.C.RESUMING.bind_fail` |
| `RESUMING` | BIND 응답을 받았다(§3-2-2 · §3-2-2 · §3-2-3) | 그대로 | `SES.C.RESUMING.bind_same` |
| `RESUMING` | BIND 응답을 받았다(§3-2-2 · §3-2-2 · §3-2-3) | `ACTIVE` | `SES.C.RESUMING.bind_new` |
| `RESUMING` | RESUME 응답을 받았다(§3-2-3 · §3-2-3) | `ACTIVE` | `SES.C.RESUMING.resume_ok_repair` |
| `RESUMING` | RESUME 응답을 받았다(§3-2-3 · §3-2-3) | `ACTIVE` | `SES.C.RESUMING.resume_ok` |
| `RESUMING` | RESUME 응답을 받았다(§3-2-3 · §3-2-3) | `ACTIVE` | `SES.C.RESUMING.resume_2008` |
| 어느 상태 | 소켓이 끊겼다 — c 는 LEAVE 사유(0 = LEAVE 없이 끊겼다 · §3-2-4) | `DISCONNECTED · CLOSED · NONE` | `SES.C.any.drop_final` |
| 어느 상태 | 소켓이 끊겼다 — c 는 LEAVE 사유(0 = LEAVE 없이 끊겼다 · §3-2-4) | `CONNECTING · CLOSED · NONE` | `SES.C.any.drop_reclaimed` |
| 어느 상태 | 소켓이 끊겼다 — c 는 LEAVE 사유(0 = LEAVE 없이 끊겼다 · §3-2-4) | `RESUMING · CLOSED · NONE` | `SES.C.any.drop_carry` |
| 어느 상태 | 소켓이 끊겼다 — c 는 LEAVE 사유(0 = LEAVE 없이 끊겼다 · §3-2-4) | `CONNECTING · CLOSED · NONE` | `SES.C.any.drop` |
| `RESUMING` | 끊긴 순간부터의 경과가 resume_window_ms 를 넘었다(클라 시계 · §3-2-3) | 그대로 | `SES.C.RESUMING.window_passed` |
| `ACTIVE` | 앱이 연결을 끊는다(RULE-SES-LEAVE-PROC) | `DISCONNECTED` | `SES.C.ACTIVE.disconnect` |
| 어느 상태 | 앱이 연결을 끊는다(RULE-SES-LEAVE-PROC) | `DISCONNECTED · CLOSED · NONE` | `SES.C.any.disconnect` |
| 어느 상태 | 토큰이 만료됐다(시간이 흘렀다) | `EXPIRED` | `SES.C.any.token_expired` |
| 어느 상태 | T-bind 가 끝났다(§3-2-4 · §3-2-4) | `CLOSED · NONE` | `SES.C.any.TBind` |
| 어느 상태 | T-req 가 끝났다 — RESUME 응답이 안 온다 · 소켓이 살아 있어도(서버가 그 요청에서 멈췄다 — §3-2-4 · §9-2-1) | `CLOSED · NONE` | `SES.C.any.TReq` |
| 어느 상태 | 백오프 대기가 끝났다(§3-2-4 · §3-2-4 2 · §3-2-3 · §3-2-3) | `OPENING` | `SES.C.any.Backoff` |
| `RESUMING` | 백오프 대기가 끝났다(§3-2-4 · §3-2-4 2 · §3-2-3 · §3-2-3) | `DISCONNECTED` | `SES.C.RESUMING.Backoff_exhaust` |
| `CONNECTING` | 백오프 대기가 끝났다(§3-2-4 · §3-2-4 2 · §3-2-3 · §3-2-3) | `DISCONNECTED` | `SES.C.CONNECTING.Backoff_exhaust` |
| 어느 상태 | 접속에 성공한 뒤 T-stable 동안 끊기지 않았다 — 재접속 사다리를 처음 칸으로 돌린다(§3-2-3 · DEV-SES-BACKOFF-RESET-STABLE) | — | `SES.C.any.TStable` |
| 어느 상태 | 하트비트를 보낼 때가 됐다(§3-2-4) — 상태는 바뀌지 않는다 | — | `SES.C.any.THb` |

##### 시한
시한 — T-bind · T-req 는 클라 값, T-hb · 재개 창은 서버가 BIND 응답으로 주는 값의 기본(§1-2-6), T-idle 은 서버 값.
★WS 점검 시한은 값이 없다(클라 구현 기본값 — §3-2-4) — 원장에 두지 않는다.

| 이름 | 원문 이름 | 값(ms) | 모형 | 값의 근거 |
|---|---|---|---|---|
| <a id="SES.ledger.TBind"></a>`TBind` | T-bind | 10000 | 선언 |  |
| <a id="SES.ledger.THb"></a>`THb` | T-hb | 10000 | 선언 |  |
| <a id="SES.ledger.TReq"></a>`TReq` | T-req | 30000 | 선언 |  |
| <a id="SES.ledger.TIdle"></a>`TIdle` | T-idle(서버) | 30000 | 서버 시한 — 세계의 끊김 사유 4008 로 낸다(모형 밖의 시계) |  |
| <a id="SES.ledger.TWindow"></a>`TWindow` | resume_window_ms | 60000 | 선언 |  |
| <a id="SES.ledger.TStable"></a>`TStable` | T-stable | 10000 | 선언 |  |

| 횟수 | 값 |
|---|---|
| <a id="SES.ledger.CBackoff"></a>`CBackoff` | 10 |

##### 사다리 `Backoff` — 재접속 백오프 사다리(§3-2-3) — 칸마다 대기(ms) · 3번째부터 지터 0~1000ms
칸 0 · 300 · 1200 · 2700 · 4800 · 7000 · 7000 · 7000 · 7000 · 7000 ms · 지터 3번째부터 0~1000 ms · 칸 수(최대 횟수) 10 · 합계 대기만 **44초** · 지터를 더해 **44~52초**

<a id="SES.C"></a>
### 3-2. 동작

<a id="SES.C.DISCONNECTED"></a>
#### 3-2-1. 클라 · `DISCONNECTED`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="SES.C.DISCONNECTED.connect_no_token"></a>`SES.C.DISCONNECTED.connect_no_token` | 받는 상태 cs == 'DISCONNECTED' · token == 'NONE' | 효과 app.need_token | §3-2-1 1 |
| <a id="SES.C.DISCONNECTED.connect"></a>`SES.C.DISCONNECTED.connect` | 받는 상태 cs == 'DISCONNECTED' · 나머지 | cs ← 'CONNECTING'; sock ← 'OPENING'; tries ← 0; late ← false | §3-2-1 2 §3-2-1 3 |
| <a id="SES.C.DISCONNECTED.new_token"></a>`SES.C.DISCONNECTED.new_token` | 받는 상태 cs == 'DISCONNECTED' && token != 'VALID' · 나머지 | token ← 'VALID' | §3-2-2 2 |

<a id="RULE-SES-DISC-CONNECT"></a>**RULE-SES-DISC-CONNECT** · 규칙 — 앱이 접속을 요청했다



항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`SES.C.DISCONNECTED.connect_no_token`](#SES.C.DISCONNECTED.connect_no_token) |
| 2 | [`SES.C.DISCONNECTED.connect`](#SES.C.DISCONNECTED.connect) |
| 3 | [`SES.C.DISCONNECTED.connect`](#SES.C.DISCONNECTED.connect) |

<a id="SES.C.CONNECTING"></a>
#### 3-2-2. 클라 · `CONNECTING`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="SES.C.CONNECTING.open"></a>`SES.C.CONNECTING.open` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · cs == 'CONNECTING' | sock ← 'OPEN'; wait ← 'BIND'; sent ← 0 · 보낸다 BIND.req {token: token, pc_mode: pcAsk} | §3-2-2 |
| <a id="SES.C.CONNECTING.bind_ok_rebuild"></a>`SES.C.CONNECTING.bind_ok_rebuild` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · cs == 'CONNECTING' && m.part == 'res' && marked | sid ← m.session_id; hbMs ← m.heartbeat_interval; winMs ← m.resume_window_ms; pcm ← m.pc_mode; wait ← 'NONE'; hb ← true; cs ← 'ACTIVE' · 효과 rooms.rejoin → ROOM.C.NONE.rejoin · 효과 pub.reregister_all → PUB.C.any.reregister · PUB.C.any.reregister_off · 효과 app.rebuilt | §3-2-2 §3-2-3 3 §3-2-3 4 DEV-SES-FRESH-REJOIN — 세션을 버리고 새로 붙었다 — 남긴 방에 다시 들어가고 다시 발행한다(RESUMING 의 새 세션과 같은 결말) |
| <a id="SES.C.CONNECTING.bind_ok"></a>`SES.C.CONNECTING.bind_ok` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · cs == 'CONNECTING' && m.part == 'res' | sid ← m.session_id; hbMs ← m.heartbeat_interval; winMs ← m.resume_window_ms; pcm ← m.pc_mode; wait ← 'NONE'; hb ← true; cs ← 'ACTIVE' · 효과 app.connected | §3-2-2 |
| <a id="SES.C.CONNECTING.bind_2002"></a>`SES.C.CONNECTING.bind_2002` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · cs == 'CONNECTING' && m.code == 2002 | cs ← 'DISCONNECTED'; sock ← 'CLOSED'; wait ← 'NONE' · 효과 rooms.close_all → ROOM.C.any.close_all | §3-2-2 1 |
| <a id="SES.C.CONNECTING.bind_2003"></a>`SES.C.CONNECTING.bind_2003` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · cs == 'CONNECTING' && m.code == 2003 | cs ← 'DISCONNECTED'; sock ← 'CLOSED'; wait ← 'NONE'; token ← 'NONE' · 효과 app.need_token | §3-2-2 2 |
| <a id="SES.C.CONNECTING.bind_1xxx"></a>`SES.C.CONNECTING.bind_1xxx` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · cs == 'CONNECTING' && m.code < 2000 | cs ← 'DISCONNECTED'; sock ← 'CLOSED'; wait ← 'NONE' · 효과 rooms.close_all → ROOM.C.any.close_all · 효과 app.bug | §3-2-2 3 — ★**클라 버그**다 — 재시도하지 않는다(§11-2-1 2) |
| <a id="SES.C.CONNECTING.bind_fail"></a>`SES.C.CONNECTING.bind_fail` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · cs == 'CONNECTING' | sock ← 'CLOSED'; wait ← 'NONE' | §3-2-2 4 |
| <a id="SES.C.CONNECTING.Backoff_exhaust"></a>`SES.C.CONNECTING.Backoff_exhaust` | 나머지 | cs ← 'DISCONNECTED' · 효과 rooms.close_all → ROOM.C.any.close_all · 효과 app.disconnected | §3-2-4 2 DEV-SES-FRESH-REJOIN |

<a id="RULE-SES-CONN-OPEN"></a>**RULE-SES-CONN-OPEN** · 규칙 — 소켓이 열렸다

→ 구조 — [`SES.C.CONNECTING.open`](#SES.C.CONNECTING.open) · [`PROP-SES-SERVER-SPEAKS-SECOND`](#PROP-SES-SERVER-SPEAKS-SECOND) · [`PROP-SES-CONNECTING-BIND-ONLY`](#PROP-SES-CONNECTING-BIND-ONLY) · [`SES.state`](#SES.state)

<a id="RULE-SES-CONN-BIND-OK"></a>**RULE-SES-CONN-BIND-OK** · 규칙 — `BIND` 성공 응답을 받았다

→ 구조 — [`SES.C.CONNECTING.bind_ok`](#SES.C.CONNECTING.bind_ok) · [`SES.state`](#SES.state)

<a id="RULE-SES-CONN-BIND-FAIL"></a>**RULE-SES-CONN-BIND-FAIL** · 규칙 — `BIND` 실패 응답을 받았다

→ 구조 — [`SES.C.any.Backoff`](#SES.C.any.Backoff)

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`SES.C.CONNECTING.bind_2002`](#SES.C.CONNECTING.bind_2002) |
| 2 | [`SES.C.DISCONNECTED.new_token`](#SES.C.DISCONNECTED.new_token) · [`SES.C.CONNECTING.bind_2003`](#SES.C.CONNECTING.bind_2003) |
| 3 | [`SES.C.CONNECTING.bind_1xxx`](#SES.C.CONNECTING.bind_1xxx) |
| 4 | [`SES.C.CONNECTING.bind_fail`](#SES.C.CONNECTING.bind_fail) · [`SES.C.RESUMING.bind_fail`](#SES.C.RESUMING.bind_fail) |

<a id="PROP-SES-CONNECT-ENDS"></a>**PROP-SES-CONNECT-ENDS** · 성질

접속 · 재개는 끝난다 — `CONNECTING` · `RESUMING` 은 결국 `ACTIVE` 이거나 `DISCONNECTED` 다. 백오프 사다리는 결국 바닥나 포기한다(RULE-SES-BACKOFF-LADDER · RULE-SES-CONN-TBIND · ★활성 성질 — 통 · 소켓 열기 결과 · 타이머에 공정성 · 조작 · 장애는 결국 멎는다).

*식: SL1(활성 판 session live) · 깨지면: 망이 계속 실패할 때 접속 중 표시로 영영 멈춘다 — 앱이 끊김을 듣지 못해 다시 묻지도 않는다*

<a id="SES.C.RESUMING"></a>
#### 3-2-3. 클라 · `RESUMING`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="SES.C.RESUMING.open_late"></a>`SES.C.RESUMING.open_late` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · late | sock ← 'OPEN'; wait ← 'BIND'; sent ← 0 · 보낸다 BIND.req {token: token, pc_mode: pcAsk} | §3-2-3 2 §3-2-3 5 R-SES-RES-OPEN-WHY R-SES-NO-CARRY-NO-RESUMING-WHY R-SES-CLIENT-WATCHES-CLOCK-WHY |
| <a id="SES.C.RESUMING.open"></a>`SES.C.RESUMING.open` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · 나머지 | sock ← 'OPEN'; wait ← 'BIND'; sent ← sid · 보낸다 BIND.req {token: token, session_id: sid, pc_mode: pcAsk} | §3-2-3 1 §3-2-3 3 §3-2-3 4 §3-2-3 5 R-SES-RES-OPEN-WHY |
| <a id="SES.C.RESUMING.bind_2003"></a>`SES.C.RESUMING.bind_2003` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · m.code == 2003 | cs ← 'DISCONNECTED'; sock ← 'CLOSED'; wait ← 'NONE'; token ← 'NONE' · 효과 floor.down_all → FLR.C.any.down · 효과 rooms.down_all → ROOM.C.LEAVING.down · ROOM.C.any.down · 효과 media.close_all → LINK.C.any.close_all · 효과 app.need_token | §3-2-3 1 DEV-SES-ROOMS-ON-MEDIA-CLOSE R-SES-RES-BIND-RES-WHY |
| <a id="SES.C.RESUMING.bind_2002"></a>`SES.C.RESUMING.bind_2002` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · m.code == 2002 | cs ← 'DISCONNECTED'; sock ← 'CLOSED'; wait ← 'NONE' · 효과 floor.down_all → FLR.C.any.down · 효과 rooms.close_all → ROOM.C.any.close_all · 효과 media.close_all → LINK.C.any.close_all | §3-2-3 1 DEV-SES-ROOMS-ON-MEDIA-CLOSE R-SES-RES-BIND-RES-WHY |
| <a id="SES.C.RESUMING.bind_fail"></a>`SES.C.RESUMING.bind_fail` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · m.part == 'fail' | sock ← 'CLOSED'; wait ← 'NONE' | §3-2-2 4 DEV-SES-RESUMING-BIND-FAIL |
| <a id="SES.C.RESUMING.bind_same"></a>`SES.C.RESUMING.bind_same` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · sent != 0 && m.session_id == sent | sid ← m.session_id; hbMs ← m.heartbeat_interval; winMs ← m.resume_window_ms; pcm ← m.pc_mode; hb ← true; wait ← 'RESUME' · 보낸다 RESUME.req | §3-2-3 2 |
| <a id="SES.C.RESUMING.bind_new"></a>`SES.C.RESUMING.bind_new` | 받는 상태 cs == 'CONNECTING' \|\| cs == 'RESUMING' · 나머지 | sid ← m.session_id; hbMs ← m.heartbeat_interval; winMs ← m.resume_window_ms; pcm ← m.pc_mode; hb ← true; wait ← 'NONE'; cs ← 'ACTIVE'; late ← false · 효과 floor.down_all → FLR.C.any.down · 효과 rooms.down_all → ROOM.C.LEAVING.down · ROOM.C.any.down · 효과 media.close_all → LINK.C.any.close_all · 효과 rooms.rejoin → ROOM.C.NONE.rejoin · 효과 pub.reregister_all → PUB.C.any.reregister · PUB.C.any.reregister_off · 효과 app.rebuilt | §3-2-3 3 §3-2-3 1 §3-2-3 1-1 §3-2-3 2 §3-2-3 3 §3-2-3 4 |
| <a id="SES.C.RESUMING.resume_ok_repair"></a>`SES.C.RESUMING.resume_ok_repair` | 받는 상태 cs == 'RESUMING' · m.part == 'res' && dead | cs ← 'ACTIVE'; wait ← 'NONE'; late ← false · 효과 rooms.snapshot → ROOM.C.JOINED.resume_gone · ROOM.C.JOINED.resume_epoch · ROOM.C.JOINED.resume_snapshot · 효과 pub.resume_check → PUB.C.any.resume_unlisted · PUB.C.STAGED.resume_listed_unwanted · PUB.C.STAGED.resume_listed_half · PUB.C.STAGED.resume_listed · PUB.C.STAGED.resume_unlisted · PUB.C.STAGED.resume_unlisted_unwanted · PUB.C.any.resume_orphan · PUB.C.any.resume_listed · 효과 media.repair → LINK.C.any.repair_rebuild · LINK.C.any.repair_restart · LINK.C.any.repair_ok · 효과 app.resumed | §3-2-3 1 §3-2-3 2 §3-2-3 4 §3-2-3 5 §3-2-3 6 R-SES-RESUME-PROC-WHY R-SES-RES-RESUME-OK-WHY |
| <a id="SES.C.RESUMING.resume_ok"></a>`SES.C.RESUMING.resume_ok` | 받는 상태 cs == 'RESUMING' · m.part == 'res' | cs ← 'ACTIVE'; wait ← 'NONE'; late ← false · 효과 rooms.snapshot → ROOM.C.JOINED.resume_gone · ROOM.C.JOINED.resume_epoch · ROOM.C.JOINED.resume_snapshot · 효과 pub.resume_check → PUB.C.any.resume_unlisted · PUB.C.STAGED.resume_listed_unwanted · PUB.C.STAGED.resume_listed_half · PUB.C.STAGED.resume_listed · PUB.C.STAGED.resume_unlisted · PUB.C.STAGED.resume_unlisted_unwanted · PUB.C.any.resume_orphan · PUB.C.any.resume_listed · 효과 app.resumed | §3-2-3 1 §3-2-3 2 §3-2-3 3 §3-2-3 5 §3-2-3 6 R-SES-RESUME-PROC-WHY R-SES-RES-RESUME-OK-WHY |
| <a id="SES.C.RESUMING.resume_2008"></a>`SES.C.RESUMING.resume_2008` | 받는 상태 cs == 'RESUMING' · 나머지 | wait ← 'NONE'; cs ← 'ACTIVE'; late ← false · 효과 floor.down_all → FLR.C.any.down · 효과 rooms.down_all → ROOM.C.LEAVING.down · ROOM.C.any.down · 효과 media.close_all → LINK.C.any.close_all · 효과 rooms.rejoin → ROOM.C.NONE.rejoin · 효과 pub.reregister_all → PUB.C.any.reregister · PUB.C.any.reregister_off · 효과 app.rebuilt | §3-2-3 1 §3-2-3 1-1 §3-2-3 2 §3-2-3 3 §3-2-3 4 R-SES-RESUME-2008-DEFENSIVE |
| <a id="SES.C.RESUMING.window_passed"></a>`SES.C.RESUMING.window_passed` | 받는 상태 cs == 'RESUMING' && !late · 나머지 | late ← true | §3-2-3 2 §3-2-3 R-SES-CLIENT-WATCHES-CLOCK-WHY |
| <a id="SES.C.RESUMING.Backoff_exhaust"></a>`SES.C.RESUMING.Backoff_exhaust` | cs == 'RESUMING' | cs ← 'DISCONNECTED' · 효과 floor.down_all → FLR.C.any.down · 효과 rooms.close_all → ROOM.C.any.close_all · 효과 media.close_all → LINK.C.any.close_all · 효과 app.disconnected | §3-2-3 1 §3-2-3 2 §3-2-3 3 DEV-SES-ROOMS-ON-MEDIA-CLOSE R-SES-RES-EXHAUST-WHY |

<a id="RULE-SES-RES-OPEN"></a>**RULE-SES-RES-OPEN** · 규칙 — 소켓이 열렸다

4. ★**방마다 고르지 않는다** — 어긋남은 응답의 스냅샷과 대조해 ★**서버마다** 잡는다(§3-2-3).

→ 구조 — [`SES.S.bind_resume`](#SES.S.bind_resume) · [`SES.C.RESUMING.bind_same`](#SES.C.RESUMING.bind_same) · [`SES.C.any.drop`](#SES.C.any.drop) · [`SES.C.any.drop_carry`](#SES.C.any.drop_carry)

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`SES.C.RESUMING.open`](#SES.C.RESUMING.open) · [`SES.C.any.token_expired`](#SES.C.any.token_expired) |
| 2 | [`SES.C.RESUMING.open_late`](#SES.C.RESUMING.open_late) · [`SES.C.RESUMING.window_passed`](#SES.C.RESUMING.window_passed) |
| 3 | [`SES.C.RESUMING.open`](#SES.C.RESUMING.open) |
| 4 | [`SES.C.RESUMING.open`](#SES.C.RESUMING.open) |
| 5 | [`SES.C.RESUMING.open_late`](#SES.C.RESUMING.open_late) · [`SES.C.RESUMING.open`](#SES.C.RESUMING.open) |

<a id="RULE-SES-RES-BIND-RES"></a>**RULE-SES-RES-BIND-RES** · 규칙 — `BIND` 응답을 받았다

→ 구조 — [`SES.C.RESUMING.bind_fail`](#SES.C.RESUMING.bind_fail) · [`SES.C.DISCONNECTED.new_token`](#SES.C.DISCONNECTED.new_token)

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`SES.C.RESUMING.bind_2003`](#SES.C.RESUMING.bind_2003) · [`SES.C.RESUMING.bind_2002`](#SES.C.RESUMING.bind_2002) · [`SES.S.restart`](#SES.S.restart) |
| 2 | [`SES.C.RESUMING.bind_same`](#SES.C.RESUMING.bind_same) |
| 3 | [`SES.C.RESUMING.bind_new`](#SES.C.RESUMING.bind_new) |

<a id="RULE-SES-RES-RESUME-OK"></a>**RULE-SES-RES-RESUME-OK** · 규칙 — `RESUME` 성공 응답을 받았다

1. ★`epoch` 가 다르면 그 방 보관본을 **통째로 버린다**(규칙 1 — 미디어 서버가 재기동됐다).
2. ★★**자기 미디어와 대조한다 — 서버마다** 셋을 본다.
   ① 그 서버의 미디어 연결이 ★**살아 있다**
   ② 그 서버에서 내가 들어가 있던 방이 ★**스냅샷에 다 있다**.
   이 대조는 ★**남은 방**으로 한다.
   ③ 그 서버의 내 발행(전이중·반이중 — 발행이 살아 있는 것, §3-2-3 4)이 ★**응답의 `publications[]` 에 다 있다**(`track_id`, §3-1 — 발행 층).
   트랙 층으로는 반이중 발행을 대조할 수 없으므로(§6-2-2 대응 표) 이 목록이 대조 재료다. 전이중은 그 방 `tracks` 에도 내 항목이 있다(배정표에는 줄이 없다).
   ★끊긴 동안 통로에서 사라진 `PUBLISH_TRACKS` 요청 · 응답도 이 목록으로 잡는다(DEV-PUB-RESUME-INFLIGHT).
   ★`epoch` 가 바뀐 방이 있으면 그 서버는 재기동됐다 — ③과 같이 ★**서버가 그 연결을 모르는 것**이다.
3. ★**셋 다 참인 서버는 이어 쓴다.** 그 서버의 배정표(`mline_tables[]` — §8-1-2)를 `MLINE_EVENT` 와 같이 받는다(§8-1-2).
   ★`arev` 가 그대로면 ★**미디어를 건드리지 않는다** — 재협상도 `READY` 도 하지 않는다(이어 쓰는 것이 이 op 의 목적이다).
   커졌으면 ★**구독 SDP 를 다시 조립**하고(§9-4-7 · ★`1pc` 는 §9-2-1) 협상 성공 후 `READY{arev}` — 이것은 어긋남이 아니라 놓친 표의 반영이다.
   트랙 목록(`tracks`)은 보기용이라 화면만 고친다.
4. ★★**어긋난 서버는 그 서버만 다룬다**(§9-2-1) — ②로 빠진 방은 내린 뒤,
   ③이 거짓이거나 `epoch` 가 바뀌었으면(서버가 연결을 모른다) ★**그 서버만 재구축**한다 — 남은 방만 다시 들어간다.
   ★그 서버에 남은 방이 없으면 재구축하지 않고 그 연결을 닫는다(§4-2-5 2).
   ★**다른 서버는 손대지 않는다** — 복구 단위는 미디어 서버다(성립 조건 색인). 세션 전체 재구축은 세션이 사라졌을 때만이다(§3-2-3).

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`ROOM.C.JOINED.resume_snapshot`](04-room.md#ROOM.C.JOINED.resume_snapshot) · [`SES.C.RESUMING.resume_ok_repair`](#SES.C.RESUMING.resume_ok_repair) · [`SES.C.RESUMING.resume_ok`](#SES.C.RESUMING.resume_ok) |
| 2 | [`PUB.C.any.resume_unlisted`](07-pub.md#PUB.C.any.resume_unlisted) · [`PUB.C.STAGED.resume_listed_unwanted`](07-pub.md#PUB.C.STAGED.resume_listed_unwanted) · [`PUB.C.STAGED.resume_listed_half`](07-pub.md#PUB.C.STAGED.resume_listed_half) · [`PUB.C.STAGED.resume_listed`](07-pub.md#PUB.C.STAGED.resume_listed) · [`PUB.C.STAGED.resume_unlisted`](07-pub.md#PUB.C.STAGED.resume_unlisted) · [`PUB.C.STAGED.resume_unlisted_unwanted`](07-pub.md#PUB.C.STAGED.resume_unlisted_unwanted) · [`PUB.C.any.resume_orphan`](07-pub.md#PUB.C.any.resume_orphan) · [`ROOM.C.JOINED.resume_gone`](04-room.md#ROOM.C.JOINED.resume_gone) · [`ROOM.C.JOINED.resume_epoch`](04-room.md#ROOM.C.JOINED.resume_epoch) · [`SES.C.RESUMING.resume_ok_repair`](#SES.C.RESUMING.resume_ok_repair) · [`SES.C.RESUMING.resume_ok`](#SES.C.RESUMING.resume_ok) |
| 3 | [`LINK.C.any.repair_ok`](09-link.md#LINK.C.any.repair_ok) · [`PUB.C.any.resume_listed`](07-pub.md#PUB.C.any.resume_listed) · [`SES.C.RESUMING.resume_ok`](#SES.C.RESUMING.resume_ok) · [`TRK.C.any.snapshot`](06-trk.md#TRK.C.any.snapshot) · [`TRK.C.any.snapshot_same`](06-trk.md#TRK.C.any.snapshot_same) |
| 4 | [`LINK.C.any.repair_rebuild`](09-link.md#LINK.C.any.repair_rebuild) · [`LINK.C.any.repair_restart`](09-link.md#LINK.C.any.repair_restart) · [`LINK.C.any.epoch_rebuild`](09-link.md#LINK.C.any.epoch_rebuild) · [`LINK.C.any.publost_rebuild`](09-link.md#LINK.C.any.publost_rebuild) · [`PUB.C.any.resume_unlisted`](07-pub.md#PUB.C.any.resume_unlisted) · [`ROOM.C.JOINED.resume_epoch`](04-room.md#ROOM.C.JOINED.resume_epoch) · [`SES.C.RESUMING.resume_ok_repair`](#SES.C.RESUMING.resume_ok_repair) |
| 5 | [`SES.C.RESUMING.resume_ok_repair`](#SES.C.RESUMING.resume_ok_repair) · [`SES.C.RESUMING.resume_ok`](#SES.C.RESUMING.resume_ok) |
| 6 | [`SES.C.RESUMING.resume_ok_repair`](#SES.C.RESUMING.resume_ok_repair) · [`SES.C.RESUMING.resume_ok`](#SES.C.RESUMING.resume_ok) |

<a id="RULE-SES-REBUILD"></a>**RULE-SES-REBUILD** · 규칙 — ★세션 전체 재구축 — 세션이 사라졌을 때, 방아쇠 셋이 한 절차로 온다

★★**방아쇠는 셋이고 절차는 하나다** — ①`RESUME` 이 `2008` 이다(세션이 이미 지워졌다) ②`BIND` 응답의 `session_id` 가
다르다(새 세션 — `RESUME` 을 아예 안 보낸다, §3-2-3 3) ③끊긴 뒤 경과가 `resume_window_ms` 를 넘었다(`RESUME` 을 아예
안 보낸다, §3-2-3 2). ★**셋 다 세션이 없다는 뜻이다.** ★`RESUME` 이 `5002`(그 세션의 방을 든 미디어 서버가 답하지 않았다 — §3-1 RESUME)여도 같은 절차다 — 세션은 있으나 이어받을 스냅샷을 온전히 못 받았다(방을 뺀 스냅샷은 서버가 내지 않는다). 스냅샷과의 어긋남·그 서버 미디어의 죽음은 ★**여기로 오지 않는다** —
서버마다 §9-2-1 이다(§3-2-3 4).
1. ★**이어받을 것이 없다.**
1-1. ★★**방마다 발언권을 `has no permission` 으로 내리고** 앱에 알린다(§10-4-32). ★**DC 도 함께 죽으므로 통지가
   오지 않는다** — 안 내리면 화면에 ★**말하는 중이 걸린 채로 굳는다.**
   ★내릴 때 그 방 발언권 상태를 처음 값으로 돌린다 — 화자 표시 · 표시에 쓴 일련번호 · 발행 방 · 반이중 발행까지(DEV-FLR-DOWN-FRESH — 재기동한 서버는 방 일련번호를 작은 값부터 다시 센다).
2. 방 상태를 전부 `NONE` 으로 내리고 ★**미디어 연결을 전부 닫는다** — 다른 서버 것도 닫는다.
   ★연결이 닫히면 서버가 Peer 와 함께 발행 트랙을 푼다 — 발행 상태를 `IDLE` 로 내린다(RULE-ROOM-LEAVE-RES 2). ★내리기 전에 들어가 있던 방 · 들어가려던 방은 다시 들어갈 방으로 남긴다(DEV-ROOM-REJOIN-LIST).
3. **`ROOM_JOIN` 부터 전부 다시 한다**(전면 재구축). ★**첫 `ROOM_JOIN` 은 분산해 늦추고**(§3-2-3) `4009` 를 받으면
   `retry_after_ms` 를 하한으로 기다린 뒤 다시 보낸다(§11-2-3) — 세션이 사라지는 것도 서버가 상태를 잃은 사건이라 단말 전부에 한꺼번에 온다.
   ★다시 들어간 뒤 다시 발행한다 — 그 서버 재구축과 같다(DEV-SES-REBUILD-REPUBLISH).
4. → **`ACTIVE`**. ★**`ACTIVE` 로 가는 것이 맞다** — *"이어받을 것이 없다"* 는 답이지 *"붙지 못했다"* 는 답이 아니다.

★세션을 버리고 새로 붙는 길(회수 · 재접속 중 토큰 만료)도 새로 붙은 뒤 2 에서 남긴 방에 3 을 한다 · 세션이 끝나는 길은 남기지 않는다(DEV-SES-FRESH-REJOIN).

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`SES.C.RESUMING.bind_new`](#SES.C.RESUMING.bind_new) · [`SES.C.RESUMING.resume_2008`](#SES.C.RESUMING.resume_2008) |
| 1-1 | [`SES.C.RESUMING.bind_new`](#SES.C.RESUMING.bind_new) · [`SES.C.RESUMING.resume_2008`](#SES.C.RESUMING.resume_2008) |
| 2 | [`LINK.C.any.close_all`](09-link.md#LINK.C.any.close_all) · [`ROOM.C.LEAVING.down`](04-room.md#ROOM.C.LEAVING.down) · [`ROOM.C.any.down`](04-room.md#ROOM.C.any.down) · [`ROOM.C.any.close_all`](04-room.md#ROOM.C.any.close_all) · [`SES.C.RESUMING.bind_new`](#SES.C.RESUMING.bind_new) · [`SES.C.RESUMING.resume_2008`](#SES.C.RESUMING.resume_2008) |
| 3 | [`ROOM.C.NONE.rejoin`](04-room.md#ROOM.C.NONE.rejoin) · [`SES.C.CONNECTING.bind_ok_rebuild`](#SES.C.CONNECTING.bind_ok_rebuild) · [`SES.C.RESUMING.bind_new`](#SES.C.RESUMING.bind_new) · [`SES.C.RESUMING.resume_2008`](#SES.C.RESUMING.resume_2008) |
| 4 | [`SES.C.CONNECTING.bind_ok_rebuild`](#SES.C.CONNECTING.bind_ok_rebuild) · [`SES.C.RESUMING.bind_new`](#SES.C.RESUMING.bind_new) · [`SES.C.RESUMING.resume_2008`](#SES.C.RESUMING.resume_2008) |

<a id="RULE-SES-RES-EXHAUST"></a>**RULE-SES-RES-EXHAUST** · 규칙 — 백오프를 소진했다



항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`SES.C.RESUMING.Backoff_exhaust`](#SES.C.RESUMING.Backoff_exhaust) |
| 2 | [`SES.C.RESUMING.Backoff_exhaust`](#SES.C.RESUMING.Backoff_exhaust) |
| 3 | [`SES.C.RESUMING.Backoff_exhaust`](#SES.C.RESUMING.Backoff_exhaust) |

<a id="RULE-SES-BACKOFF-LADDER"></a>**RULE-SES-BACKOFF-LADDER** · 규칙 — 재접속 백오프

★재개 창을 재는 *"끊긴 순간"* 은 그 세션에 물려 있던 소켓이 마지막으로 끊긴 순간이다(DEV-SES-LATE-CLOCK-LAST-BOUND).

★사다리의 칸은 접속에 성공했다고 처음으로 돌아가지 않는다 — 성공한 뒤 `T-stable`(10초) 동안 끊기지 않아야 처음 칸으로 돌아간다(DEV-SES-BACKOFF-RESET-STABLE).

→ 구조 — [`SES.state`](#SES.state) · [`SES.C.any.Backoff`](#SES.C.any.Backoff) · [`SES.C.RESUMING.Backoff_exhaust`](#SES.C.RESUMING.Backoff_exhaust) · [`SES.C.CONNECTING.Backoff_exhaust`](#SES.C.CONNECTING.Backoff_exhaust) · [`SES.C.any.TStable`](#SES.C.any.TStable)

<a id="COND-SES-WINDOW-OVER-BACKOFF"></a>**COND-SES-WINDOW-OVER-BACKOFF** · 성립 조건

★★**서버가 세션을 지우는 시간(`resume_window_ms`, 기본 60초)이 백오프 합계보다 길어야 한다.**
짧으면 백오프 후반에 붙는 클라는 **언제나** 이어받기에 실패한다.

<a id="RULE-SES-CLIENT-WATCHES-CLOCK"></a>**RULE-SES-CLIENT-WATCHES-CLOCK** · 규칙

| | |
|---|---|
| 무엇을 재나 | ★**소켓이 끊긴 시각부터 지금까지.** 백오프 대기 + `T-bind` 소모가 다 들어간다. ★`RESUME` 은 `BIND` 응답 뒤에 나가므로(§3-2-3) 그 RTT 만큼 창 경계에 가까워진다 — 수십 ms 라 이 산수를 안 바꾸고, 겨우 넘겨 받는 `2008` 의 결말은 어차피 갈 자리(전면 재구축)다 |

→ 구조 — [`SES.C.RESUMING.window_passed`](#SES.C.RESUMING.window_passed) · [`SES.C.RESUMING.open_late`](#SES.C.RESUMING.open_late) · [`SES.C.RESUMING.bind_new`](#SES.C.RESUMING.bind_new)

<a id="COND-SES-ARITHMETIC"></a>**COND-SES-ARITHMETIC** · 성립 조건

★**성립 조건** — 위 산수는 백오프 표의 값과 `T-bind` 값이 지금 그대로일 때만 참이다.
★**둘 중 하나를 바꾸면 이 절을 먼저 다시 계산한다.**

<a id="RULE-SES-REBUILD-JITTER"></a>**RULE-SES-REBUILD-JITTER** · 규칙

★★**재구축 칸의 첫 `ROOM_JOIN` 은 분산한다 — 원인이 아니라 칸의 무게로 가른다.** 가벼운 칸(`BIND`·`RESUME`·`ICE_RESTART`)은
위 표대로 0ms 에서 시작한다. ★`ICE_RESTART` 되풀이는 재접속과 달리 ★**최대 횟수가 없다** — 표의 칸값을 쓰고 끝 칸을 되풀이한다(§9-2-1 1·7). 재구축 칸(§3-2-3 · §9-2-1 2)은 첫 `ROOM_JOIN` 을 `0 ~ 분산 창` 균등 무작위로 늦춘다(full jitter —
창은 클라 구현 기본값). 재구축으로 들어오는 사건은 정의상 전부 ★**서버가 상태를 잃었다는 답**이라 원인 목록이 필요 없다.
★**`4009` 는 하한이다** — `retry_after_ms` 보다 먼저 다시 보내지 않는다. 횟수 제한은 없다(§11-2-3).
★**위 산수는 바뀌지 않는다** — 분산은 재구축 칸의 `ROOM_JOIN` 에만 걸리고 `RESUME` 까지의 재접속 사다리에는 걸리지 않는다.

<a id="PROP-SES-RESUME-ON-SAME"></a>**PROP-SES-RESUME-ON-SAME** · 성질

`RESUME` 이 떠 있으면 서버는 살아 있는 세션에 그 소켓을 물렸고, 그 세션이 클라가 보관한 `session_id` 다.

*식: SH2 · 깨지면: 없는 세션이나 남의 세션에 `RESUME` 이 간다 — `2008` 이 헛돌거나 다른 세션의 스냅샷을 받는다*

<a id="PROP-SES-CARRY-HAS-MEDIA"></a>**PROP-SES-CARRY-HAS-MEDIA** · 성질

통이 빈 `ACTIVE` 에서 방에 들어가 있다고 알면 미디어 연결이 있다(살았든 죽었든 — 죽은 것은 LINK 축이 되살린다).

*식: SH5 · 깨지면: 방에 있다고 알면서 미디어 연결이 없다 — 화면은 검고 되살릴 연결도 없다*

<a id="PROP-SES-DISCONNECTED-NO-MEDIA"></a>**PROP-SES-DISCONNECTED-NO-MEDIA** · 성질

`DISCONNECTED` 에는 미디어 연결이 없다.

*식: SH6 · 깨지면: 버린 세션에 STUN 을 흘리다 `failed` 로 남는다(§3-2-3)*

<a id="PROP-SES-WAIT-ENDS"></a>**PROP-SES-WAIT-ENDS** · 성질

기다리는 응답(`BIND` · `RESUME`)은 결국 끝난다 — 응답이 오거나 `T-bind` 가 끝나거나 소켓이 닫힌다(RULE-SES-RES-BIND-RES · RULE-SES-CONN-TBIND · ★활성 성질).

*식: SL2(활성 판 session live) · 깨지면: 응답을 받고도 기다림이 남아 다음 요청이 막힌다*

<a id="DEV-SES-ROOMS-ON-MEDIA-CLOSE"></a>**DEV-SES-ROOMS-ON-MEDIA-CLOSE** · 자체 확장

미디어 연결을 닫는 갈래(§3-2-4 2 · §3-2-4 3 · §3-2-3 1 의 `2003` · `2002` · §3-2-3)는 §3-2-3 1-1 · 2 와 같이 방마다 발언권을 `has no permission` 으로 내리고 방 상태를 전부 `NONE` 으로 내린다 — 다시 접속하면 방부터 다시 들어간다

*원문: §3-2-4 · §3-2-3 · §3-2-3 — 방 상태를 정하지 않는다 · 근거: RULE-SES-REBUILD RULE-ROOM-RESUMING-KEEPS PROP-SES-CARRY-HAS-MEDIA*

<a id="DEV-SES-RESUMING-BIND-FAIL"></a>**DEV-SES-RESUMING-BIND-FAIL** · 자체 확장

`RESUMING` 에서 받은 `BIND` 실패가 `2003` · `2002` 가 아니면 §3-2-2 4 와 같이 소켓을 닫고 백오프로 다시 한다(`RESUMING` 그대로)

*원문: §3-2-3 1 — 2003 · 2002 만 정한다 · 근거: RULE-SES-CONN-BIND-FAIL*

<a id="DEV-SES-LATE-CLOCK-LAST-BOUND"></a>**DEV-SES-LATE-CLOCK-LAST-BOUND** · 자체 확장

재개 창을 재는 *"끊긴 순간"* 은 그 세션에 물려 있던 소켓이 마지막으로 끊긴 순간이다 — 재접속 `BIND` 가 같은 세션에 다시 물리면(응답의 `session_id` 가 보낸 것과 같다) 시계를 새로 잰다. 서버의 재개 창도 그때 새로 돈다

*원문: §3-2-3 2 · §3-2-3 "끊긴 순간부터의 경과" — 첫 끊김인지 마지막 끊김인지 정하지 않는다 · 근거: RULE-SES-CLIENT-WATCHES-CLOCK RULE-SES-RES-OPEN*

<a id="SES.C.any"></a>
#### 3-2-4. 클라 · `any`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="SES.C.any.drop_final"></a>`SES.C.any.drop_final` | c == 1008 \|\| c == 2001 \|\| c == 4006 \|\| c == 4007 \|\| c == 2010 | cs ← 'DISCONNECTED'; sock ← 'CLOSED'; wait ← 'NONE'; hb ← false · 효과 floor.down_all → FLR.C.any.down · 효과 rooms.close_all → ROOM.C.any.close_all · 효과 media.close_all → LINK.C.any.close_all · 효과 app.disconnected | §3-2-4 1 §3-2-4 2 DEV-SES-ROOMS-ON-MEDIA-CLOSE R-SES-ON-DROP-WHY |
| <a id="SES.C.any.drop_reclaimed"></a>`SES.C.any.drop_reclaimed` | c == 2009 \|\| c == 5004 | cs ← 'CONNECTING'; sock ← 'CLOSED'; wait ← 'NONE'; hb ← false; sid ← 0; late ← false · 효과 floor.down_all → FLR.C.any.down · 효과 rooms.down_all → ROOM.C.LEAVING.down · ROOM.C.any.down · 효과 media.close_all → LINK.C.any.close_all | §3-2-4 3 DEV-SES-ROOMS-ON-MEDIA-CLOSE R-SES-ON-DROP-WHY |
| <a id="SES.C.any.drop_carry"></a>`SES.C.any.drop_carry` | carry | cs ← 'RESUMING'; sock ← 'CLOSED'; wait ← 'NONE'; hb ← false; late ← cs == 'ACTIVE' \|\| wait == 'RESUME' ? false : late | §3-2-4 4 §3-2-4 5 DEV-SES-LATE-CLOCK-LAST-BOUND R-SES-ON-DROP-WHY |
| <a id="SES.C.any.drop"></a>`SES.C.any.drop` | 나머지 | cs ← 'CONNECTING'; sock ← 'CLOSED'; wait ← 'NONE'; hb ← false | §3-2-4 4 R-SES-ON-DROP-WHY R-SES-SOCKET-CLOSE-NORMAL-WHY |
| <a id="SES.C.any.disconnect"></a>`SES.C.any.disconnect` | 받는 상태 cs != 'DISCONNECTED' · 나머지 | cs ← 'DISCONNECTED'; sock ← 'CLOSED'; wait ← 'NONE'; hb ← false · 효과 floor.down_all → FLR.C.any.down · 효과 rooms.close_all → ROOM.C.any.close_all · 효과 media.close_all → LINK.C.any.close_all · 효과 app.disconnected | §3-1 R-SES-LEAVE-PROC-WHY |
| <a id="SES.C.any.token_expired"></a>`SES.C.any.token_expired` | 나머지 | token ← 'EXPIRED' | §3-2-3 1 |
| <a id="SES.C.any.TBind"></a>`SES.C.any.TBind` | 나머지 | sock ← 'CLOSED'; wait ← 'NONE' | §3-2-4 1 §3-2-4 |
| <a id="SES.C.any.TReq"></a>`SES.C.any.TReq` | 나머지 | sock ← 'CLOSED'; wait ← 'NONE'; hb ← false | §3-2-4 §9-2-1 — 소켓을 닫는다(하트비트도 멈춘다) — 재전송하지 않는다 · 백오프로 다시 붙어 이어받는다 |
| <a id="SES.C.any.Backoff"></a>`SES.C.any.Backoff` | tries < Cap('CBackoff') | sock ← 'OPENING'; tries ← tries + 1 | §3-2-4 1 §3-2-4 2 §3-2-4 3 §3-2-4 2 §3-2-3 |
| <a id="SES.C.any.TStable"></a>`SES.C.any.TStable` | 나머지 | tries ← 0 | §3-2-3 DEV-SES-BACKOFF-RESET-STABLE R-SES-BACKOFF-RESET-STABLE |
| <a id="SES.C.any.THb"></a>`SES.C.any.THb` | 나머지 | 보낸다 HEARTBEAT.req | §3-2-4 1 §3-2-4 2 R-SES-HEARTBEAT-WHY |

<a id="DEV-SES-FRESH-REJOIN"></a>**DEV-SES-FRESH-REJOIN** · 자체 확장

세션을 버리고 새로 붙는 두 길 — 세션 회수로 끊김(`SES.C.any.drop_reclaimed`)과 재접속 중 토큰 만료(`SES.C.RESUMING.bind_2003` → 새 토큰 → 접속) — 은 방을 내리며 다시 들어갈 방 표시를 남기고(`ROOM.C.any.down`), 새로 붙으면(`SES.C.CONNECTING.bind_ok_rebuild` — 표시가 남은 방이 있다) 그 방에 다시 들어가고 다시 발행한다(`RESUMING` 의 새 세션과 같은 결말 — 앱에는 `app.rebuilt`). 세션이 끝나는 길 — 앱이 끊음 · 다시 붙어도 소용없는 사유 · 토큰 무효 · 백오프 소진 — 은 방을 내리며 표시를 지운다(`ROOM.C.any.close_all`) — 그 뒤의 접속은 새 세션이고 방은 앱이 다시 들어간다

*원문: §3-2-3 3 — 다시 들어가기를 `RESUMING` 의 새 세션에만 둔다 · 근거: R-SES-FRESH-REJOIN*

<a id="DEV-SES-BACKOFF-RESET-STABLE"></a>**DEV-SES-BACKOFF-RESET-STABLE** · 자체 확장

재접속 사다리의 칸(시도 횟수)은 접속 성공(`BIND` · `RESUME` 성공)으로 처음으로 돌리지 않는다. 성공한 뒤 `T-stable`(10초) 동안 끊기지 않으면 그때 처음 칸으로 돌린다(`SES.C.any.TStable`). 그 안에 다시 끊기면 이어서 다음 칸부터 기다린다 — 끊김이 되풀이되어 칸을 다 쓰면 소진(§3-2-3 · §3-2-4 2)으로 끝난다. 처음 접속(`DISCONNECTED` 에서)은 처음 칸에서 시작한다

*원문: §3-2-3 — 칸값과 상한만 정하고 언제 처음으로 돌리는지 정하지 않는다 · 근거: R-SES-BACKOFF-RESET-STABLE*

<a id="RULE-SES-ON-DROP"></a>**RULE-SES-ON-DROP** · 규칙 — 소켓이 끊겼다



항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`SES.C.any.drop_final`](#SES.C.any.drop_final) |
| 2 | [`LINK.C.any.close_all`](09-link.md#LINK.C.any.close_all) · [`SES.C.any.drop_final`](#SES.C.any.drop_final) |
| 3 | [`SES.C.any.drop_reclaimed`](#SES.C.any.drop_reclaimed) |
| 4 | [`SES.C.any.drop_carry`](#SES.C.any.drop_carry) · [`SES.C.any.drop`](#SES.C.any.drop) |
| 5 | [`SES.C.any.drop_carry`](#SES.C.any.drop_carry) |

<a id="RULE-SES-ON-HEARTBEAT-DUE"></a>**RULE-SES-ON-HEARTBEAT-DUE** · 규칙 — 하트비트를 보낼 때가 됐다



항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`SES.C.any.THb`](#SES.C.any.THb) |
| 2 | [`SES.C.any.THb`](#SES.C.any.THb) |

<a id="RULE-SES-CONN-TBIND"></a>**RULE-SES-CONN-TBIND** · 규칙 — `T-bind` 가 끝났다



항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`SES.C.any.TBind`](#SES.C.any.TBind) |
| 2 | [`SES.C.any.Backoff`](#SES.C.any.Backoff) · [`SES.C.CONNECTING.Backoff_exhaust`](#SES.C.CONNECTING.Backoff_exhaust) |

<a id="RULE-SES-RES-BACKOFF-END"></a>**RULE-SES-RES-BACKOFF-END** · 규칙 — 백오프 대기가 끝났다

1. 옛 소켓 핸들러를 떼고 새 소켓을 연다.
2. ★**보내다 만 프레임을 다시 보내지 않는다.** 큐를 비운다.

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`SES.C.any.Backoff`](#SES.C.any.Backoff) |
| 2 | [`SES.C.any.Backoff`](#SES.C.any.Backoff) |
| 3 | [`SES.C.any.Backoff`](#SES.C.any.Backoff) |

<a id="RULE-SES-TIMERS-CONN"></a>**RULE-SES-TIMERS-CONN** · 규칙 — 연결 시한

| 이름 | 값 | 시작 | 정상 정지 | 끝나면 |
|---|---|---|---|---|
| `T-req` | **30초** | 요청을 보냈다 | 응답을 받았다 | ★**끊는다.** 재전송하지 않는다 — ★소켓이 살아 있어도다(서버가 그 요청을 처리하지 못했다 — WS 생존 판단과 다른 고장, §9-2-1) |
| `T-idle`(서버) | **30초** | 클라에서 뭔가 받았다 | 뭔가 더 받았다 | 서버가 끊는다 |
| ★WS 점검 | 클라 구현 기본값 | ★WS 가 죽었을 수 있다는 증거가 왔다 — 연결 흔들림·망 전환, 또는 요청이 나가 있는데 이 시한 동안 아무것도 안 왔다(§9-2-1 4) — `HEARTBEAT` 를 곧바로 보낸다(윈도우 밖, §2-2-1) | ★**무엇이든** 받았다 | ★소켓을 `LEAVE` 없이 닫고 §3-2-4 4 |

→ 구조 — [`SES.state`](#SES.state) · [`SES.C.any.TBind`](#SES.C.any.TBind) · [`SES.C.any.THb`](#SES.C.any.THb) · [`SES.C.any.Backoff`](#SES.C.any.Backoff) · [`SES.op.BIND`](#SES.op.BIND)

<a id="SES.C.ACTIVE"></a>
#### 3-2-5. 클라 · `ACTIVE`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="SES.C.ACTIVE.disconnect"></a>`SES.C.ACTIVE.disconnect` | 받는 상태 cs != 'DISCONNECTED' · cs == 'ACTIVE' | cs ← 'DISCONNECTED'; sock ← 'CLOSED'; wait ← 'NONE'; hb ← false · 보낸다 LEAVE.req · 효과 floor.down_all → FLR.C.any.down · 효과 rooms.close_all → ROOM.C.any.close_all · 효과 media.close_all → LINK.C.any.close_all · 효과 app.disconnected | §3-1 R-SES-LEAVE-PROC-WHY |

<a id="SES.S"></a>
#### 3-2-6. 서버

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="SES.S.leave"></a>`SES.S.leave` | m.t == 'LEAVE' && bound | live ← false; bound ← false; resumed ← false; sid ← 0; rooms ← false | §3-1 R-SES-LEAVE-PROC-WHY |
| <a id="SES.S.bind_bound"></a>`SES.S.bind_bound` | m.t == 'BIND' && bound | 답한다 BIND.fail {code: 1008} | §3-1 |
| <a id="SES.S.bind_resume"></a>`SES.S.bind_resume` | m.t == 'BIND' && m.session_id != 0 && live && m.session_id == sid | bound ← true; resumed ← true · 답한다 BIND.res {session_id: sid, heartbeat_interval: Ms.THb, resume_window_ms: Ms.TWindow, pc_mode: pcm} | §3-1 |
| <a id="SES.S.bind_2002"></a>`SES.S.bind_2002` | m.t == 'BIND' && m.token == 'FORGED' | 답한다 BIND.fail {code: 2002} | §3-1 |
| <a id="SES.S.bind_2003"></a>`SES.S.bind_2003` | m.t == 'BIND' && m.token == 'EXPIRED' | 답한다 BIND.fail {code: 2003} | §3-1 |
| <a id="SES.S.bind_4003"></a>`SES.S.bind_4003` | m.t == 'BIND' && full | 답한다 BIND.fail {code: 4003} | §3-1 |
| <a id="SES.S.bind_new"></a>`SES.S.bind_new` | m.t == 'BIND' | live ← true; sid ← next; next ← next + 1; bound ← true; resumed ← false; rooms ← false; pcm ← m.pc_mode · 답한다 BIND.res {session_id: next, heartbeat_interval: Ms.THb, resume_window_ms: Ms.TWindow, pc_mode: m.pc_mode} | §3-1 R-SES-EVICT-OLD-WHY |
| <a id="SES.S.pre_bind_other"></a>`SES.S.pre_bind_other` | m.t != 'BIND' && !bound | — | §3-1 |
| <a id="SES.S.resume_ok"></a>`SES.S.resume_ok` | m.t == 'RESUME' && bound && live && resumed | 답한다 RESUME.res | §3-1 |
| <a id="SES.S.resume_2008"></a>`SES.S.resume_2008` | m.t == 'RESUME' | 답한다 RESUME.fail {code: 2008} | §3-1 |
| <a id="SES.S.rx_other"></a>`SES.S.rx_other` | 나머지 | — | §3-1 |
| <a id="SES.S.drop_reclaim"></a>`SES.S.drop_reclaim` | c == 2009 \|\| c == 5004 | bound ← false; resumed ← false; live ← false; sid ← 0; rooms ← false | §3-1 |
| <a id="SES.S.drop"></a>`SES.S.drop` | 나머지 | bound ← false | §3-1 |
| <a id="SES.S.restart"></a>`SES.S.restart` | 나머지 | live ← false; bound ← false; resumed ← false; sid ← 0; rooms ← false | §3-2-3 1 |
| <a id="SES.S.quota_flip"></a>`SES.S.quota_flip` | 나머지 | full ← !full | §3-1 |
| <a id="SES.S.TWindow"></a>`SES.S.TWindow` | 나머지 | live ← false; sid ← 0; rooms ← false | §1-2-6 §3-1 |

<a id="PROP-SES-PC-MODE-AGREED"></a>**PROP-SES-PC-MODE-AGREED** · 성질

통로가 빈 ACTIVE 에서 클라가 쓰는 연결 모드가 서버가 그 세션에 적용한 값이다 — BIND 응답이 에코한 값을 쓴다(RULE-SES-PC-MODE-SESSION · RULE-LINK-SDP-1PC-MODE).

*식: PM1 · 깨지면: 클라는 `1pc` 로 도는 줄 아는데 서버는 `2pc` 다 — 폴백이 일어난 것도 모른다(RULE-SES-PC-MODE-SESSION 증상)*

<a id="PROP-SES-PC-MODE-FIXED"></a>**PROP-SES-PC-MODE-FIXED** · 성질

같은 세션 안에서 연결 모드가 바뀌지 않는다 — 붙기 전에 정해지고, 그 뒤 클라가 BIND 에 다른 값을 실어도 이어받은 세션은 확정값을 에코한다(RULE-SES-PC-MODE-SESSION · RULE-SES-PC-MODE-DECIDE · RULE-LINK-SDP-1PC-MODE — 바꾸려면 세션을 다시 맺는다).

*식: PM2 · 깨지면: 이어받은 미디어 연결은 옛 모드인데 클라는 새 모드로 SDP 를 짓는다 — `BIND` 확정값과 `server_config.pc_mode` 가 갈려 연결이 서지 않는다*

<a id="PROP-SES-SERVER-SPEAKS-SECOND"></a>**PROP-SES-SERVER-SPEAKS-SECOND** · 성질

`CONNECTING` 동안 서버가 클라에게 보낸 것은 `BIND` 응답뿐이다 — 서버가 먼저 말하지 않는다.

*식: SH8 · 깨지면: 클라가 세션 전에 통지를 받아 처리할 절차가 없다 — 받은 것을 버리거나 엉뚱한 상태에서 반영한다*

#### 3-2-7. 타이머 선언

| 이름 | 주인 | 도는 동안 | 만료 | 뜻 |
|---|---|---|---|---|
| `TBind` | client | `wait == 'BIND'` | `CTBind` | BIND 응답 대기 |
| `TReq` | client | `wait == 'RESUME'` | `CTReq` | 요청 시한 — RESUME 응답 대기(BIND 는 T-bind 가 다스린다) |
| `THb` | client | `hb && sock == 'OPEN'` | `CHbDue` | 하트비트 주기 — 보관한 heartbeat_interval 마다(§3-1 HEARTBEAT) |
| `Backoff` | client | `(cs == 'CONNECTING' \|\| cs == 'RESUMING') && sock == 'CLOSED'` | `CBackoff` | 재접속 백오프 대기 — 칸마다 사다리(§3-2-3) |
| `TStable` | client | `cs == 'ACTIVE' && tries > 0` | `CStable` | 성공 뒤 안정 관찰 — 접속에 성공한 뒤 이만큼 끊기지 않아야 사다리를 처음 칸으로 돌린다 |
| `TWindow` | server | `live && !bound` | `SWindow` | 재개 창 — 소켓이 물리지 않은 세션을 거둔다 |

### 3-3. 흐름 · 검증
검증 모형 `out/session/Session.tla` · 하네스 `verify/session/harness.tla`

#### 3-3-1. 흐름 F1 — 처음 접속
세션의 줄기다. 이어받기 · 재구축은 이 흐름 끝(ACTIVE)에서 갈라진다.
모형 검사: 걸음 4 도달 · 갈래 8

1. **connect** — 앱이 접속을 요청한다. 토큰이 있으므로 소켓을 열고 CONNECTING 으로 간다.
2. **open** — 소켓이 열리면 BIND 하나만 보낸다(session_id 없이). T-bind 가 돈다. ★서버는 먼저 말하지 않는다 — BIND 응답 전에는 다른 어떤 것도 보내지 않는다.
3. **up BIND** — 서버는 새 세션을 만들고 session_id 를 준다. 소켓이 그 세션에 물린다.
4. **down BIND** — 클라는 session_id · heartbeat_interval · resume_window_ms 를 보관하고 하트비트를 시작한다 → ACTIVE.

#### 3-3-2. 흐름 F2 — 소켓만 끊겼다 — 이어받기
WS 만 끊기고 미디어가 살아 있을 때 재협상 없이 이어 쓰는 길이다(RESUME 의 목적).
모형 검사: 걸음 12 도달 · 갈래 46

1. **connect** — 앱이 접속을 요청한다. 토큰이 있으므로 소켓을 열고 CONNECTING 으로 간다.
2. **open** — 소켓이 열리면 BIND 하나만 보낸다(session_id 없이). T-bind 가 돈다. ★서버는 먼저 말하지 않는다 — BIND 응답 전에는 다른 어떤 것도 보내지 않는다.
3. **up BIND** — 서버는 새 세션을 만들고 session_id 를 준다. 소켓이 그 세션에 물린다.
4. **down BIND** — 클라는 session_id · heartbeat_interval · resume_window_ms 를 보관하고 하트비트를 시작한다 → ACTIVE.
5. **join** — 방에 들어가 미디어 연결이 선다(ROOM · LINK 의 일) — 이제 이어받을 것이 있다. 세션은 사본을 두지 않고 끊길 때 방 · 발행 상태를 읽는다.
6. **drop** — LEAVE 없이 소켓이 끊긴다. 이어받을 것이 있으므로 RESUMING 으로 가고 백오프를 시작한다. ★미디어 연결은 닫지 않는다 — WS 만 끊긴 것이다.
7. **Backoff** — 백오프 첫 칸(0ms)이 끝나 새 소켓을 연다.
8. **open** — 소켓이 열리면 token 과 함께 옛 session_id 를 실어 BIND 를 보낸다. RESUME 은 아직 보내지 않는다. ★BIND 응답의 session_id 를 봐야 이어받을 세션이 있는지 안다 — 앞질러 보내지 않는다(직렬).
9. **up BIND** — 세션이 살아 있으므로 서버는 토큰을 보지 않고 그 세션에 소켓을 다시 물린다.
10. **down BIND** — 응답의 session_id 가 보낸 것과 같다 — 하트비트를 시작하고 이제 RESUME 을 보낸다.
11. **up RESUME** — 서버가 그 세션의 스냅샷과 발행 목록을 돌려준다.
12. **down RESUME** — 방마다 보관본을 재설정하고 미디어와 대조한다. 미디어가 살아 있으므로 건드리지 않는다 → ACTIVE.

#### 3-3-3. 흐름 F3 — 창을 넘겼다 — 세션 전체 재구축
서버가 세션을 이미 거뒀을 때다. 이어받기를 건너뛰고 방부터 다시 들어간다.
모형 검사: 걸음 11 도달 · 갈래 31

1. **connect** — 앱이 접속을 요청한다. 토큰이 있으므로 소켓을 열고 CONNECTING 으로 간다.
2. **open** — 소켓이 열리면 BIND 하나만 보낸다(session_id 없이). T-bind 가 돈다. ★서버는 먼저 말하지 않는다 — BIND 응답 전에는 다른 어떤 것도 보내지 않는다.
3. **up BIND** — 서버는 새 세션을 만들고 session_id 를 준다. 소켓이 그 세션에 물린다.
4. **down BIND** — 클라는 session_id · heartbeat_interval · resume_window_ms 를 보관하고 하트비트를 시작한다 → ACTIVE.
5. **join** — 방에 들어가 미디어 연결이 선다.
6. **drop** — LEAVE 없이 소켓이 끊긴다 → RESUMING.
7. **TWindow** — 재개 창이 끝나 서버가 세션과 Peer 를 거둔다. 클라는 모른다.
8. **Backoff** — 백오프가 끝나 새 소켓을 연다.
9. **open** — 클라 시계로는 아직 창 안이라 옛 session_id 를 실어 BIND 를 보낸다.
10. **up BIND** — 서버에는 그 세션이 없다 — 토큰으로 새 세션을 만든다.
11. **down BIND** — 응답의 session_id 가 다르다 — 새 세션이다. RESUME 을 보내지 않고 발언권을 내리고 방을 내리고 미디어를 닫은 뒤 방부터 다시 들어간다 → ACTIVE. ★"이어받을 것이 없다" 는 답이지 "붙지 못했다" 는 답이 아니다 — ACTIVE 로 가는 것이 맞다.

### 3-4. 주제

#### 3-4-1. 연결 상태

<a id="RULE-SES-CONN-DIAGRAM"></a>**RULE-SES-CONN-DIAGRAM** · 규칙 — 연결 상태

→ 구조 — [`SES.state`](#SES.state) · [`SES.C`](#SES.C)

<a id="RULE-SES-RESUMING-TWO-EXITS"></a>**RULE-SES-RESUMING-TWO-EXITS** · 규칙

★**`RESUMING` → `ACTIVE` 는 두 갈래다.** `RESUME` 이 성공했을 때만이 아니라 ★**`2008
SESSION_NOT_FOUND` 로 실패했을 때도 `ACTIVE` 로 간다**(§3-2-3) — 이어받을 것이 없다는 답이지
붙지 못했다는 답이 아니기 때문이다. 그 뒤 `ROOM_JOIN` 부터 전부 다시 한다.

→ 구조 — [`SES.C.RESUMING.resume_ok`](#SES.C.RESUMING.resume_ok) · [`SES.C.RESUMING.resume_2008`](#SES.C.RESUMING.resume_2008)

<a id="RULE-SES-CONN-STATE-TABLE"></a>**RULE-SES-CONN-STATE-TABLE** · 규칙

→ 구조 — [`SES.state`](#SES.state) · [`PROP-SES-DISCONNECTED-SENDS-NOTHING`](#PROP-SES-DISCONNECTED-SENDS-NOTHING) · [`PROP-SES-CONNECTING-BIND-ONLY`](#PROP-SES-CONNECTING-BIND-ONLY) · [`PROP-SES-RESUMING-BIND-THEN-RESUME`](#PROP-SES-RESUMING-BIND-THEN-RESUME)

<a id="RULE-SES-NO-CARRY-NO-RESUMING"></a>**RULE-SES-NO-CARRY-NO-RESUMING** · 규칙

| 끊겼을 때 상황 | 어디로 | 보내는 것 |
|---|---|---|
| 방은 있었는데 ★**미디어 연결도 같이 죽었다** | **`RESUMING`** | `BIND` → ★**`RESUME` 을 보낸다** → 스냅샷과 대조해 ★**서버마다** 가른다(§3-2-3): 서버가 아는 죽은 연결은 ICE 재시작, 서버가 모르는 연결은 그 서버만 재구축(§9-2-1). 죽은 서버를 신고하는 축은 없다 — 판단은 클라가 스냅샷을 보고 한다 |

→ 구조 — [`SES.C.any.drop`](#SES.C.any.drop) · [`SES.C.any.drop_carry`](#SES.C.any.drop_carry) · [`SES.C.CONNECTING.open`](#SES.C.CONNECTING.open) · [`SES.C.RESUMING.open`](#SES.C.RESUMING.open) · [`SES.C.RESUMING.open_late`](#SES.C.RESUMING.open_late) · [`SES.C.RESUMING.bind_same`](#SES.C.RESUMING.bind_same) · [`SES.C.RESUMING.bind_new`](#SES.C.RESUMING.bind_new) · [`SES.C.RESUMING.resume_ok`](#SES.C.RESUMING.resume_ok) · [`SES.C.RESUMING.resume_ok_repair`](#SES.C.RESUMING.resume_ok_repair) · [`PROP-SES-RESUME-NEEDS-CARRY`](#PROP-SES-RESUME-NEEDS-CARRY) · [`PROP-SES-CONNECTING-BIND-ONLY`](#PROP-SES-CONNECTING-BIND-ONLY)

<a id="PROP-SES-DISCONNECTED-SENDS-NOTHING"></a>**PROP-SES-DISCONNECTED-SENDS-NOTHING** · 성질

`DISCONNECTED` 에서 보낼 수 있는 것은 없음 — 그 상태의 클라 통은 비어 있다.

*식: SH9 · 깨지면: 버린 세션의 프레임이 새 소켓에 실려 간다*

<a id="PROP-SES-RESUMING-BIND-THEN-RESUME"></a>**PROP-SES-RESUMING-BIND-THEN-RESUME** · 성질

`RESUMING` 에서 보낼 수 있는 것 — ★`BIND` 응답 뒤 `RESUME` — **둘만, 직렬**(§3-2-3). ★경과 시간이 `resume_window_ms` 를 넘었으면 `RESUME` 을 건너뛴다(§3-2-3). `HEARTBEAT` 는 윈도우 밖이라 세지 않는다(CAND-SES-RESUMING-HEARTBEAT).

*식: SH10 · 깨지면: `BIND` 응답 전에 `RESUME` 이 나가 새 세션일 때 `2008` 을 버려야 하는 자리가 생긴다*

<a id="PROP-SES-CONNECTING-BIND-ONLY"></a>**PROP-SES-CONNECTING-BIND-ONLY** · 성질

`CONNECTING` 동안 클라가 내보낸 것은 `BIND` 뿐이다.

*식: SH1 · 깨지면: 인증 전 요청이 서버에 닿아 `LEAVE` `2001` 로 끊기고 다시 붙지 않는다(§3-1-1)*

<a id="PROP-SES-RESUME-NEEDS-CARRY"></a>**PROP-SES-RESUME-NEEDS-CARRY** · 성질

`RESUME` 은 이어받을 것(방 · 발행)이 있을 때만 나간다.

*식: SH3 · 깨지면: 이어받을 것이 없는 재접속마다 빈 왕복이 하나 더 붙는다(§3-4-1 증상)*

<a id="PROP-SES-ACTIVE-BOUND"></a>**PROP-SES-ACTIVE-BOUND** · 성질

통이 빈 `ACTIVE` 에서 서버는 살아 있는 세션에 그 소켓을 물렸고, 그 세션이 클라가 보관한 `session_id` 다.

*식: SH4 · 깨지면: 클라는 쓸 수 있다고 알지만 서버에는 그 세션이 없다 — 그 소켓이 묶이지 않았으면 첫 요청에 `LEAVE` `2001` 로 끊기고(§3-1-1), 묶였으면 방 요청이 `3002` 로 돌아온다*

<a id="PROP-SES-CARRY-AGREES"></a>**PROP-SES-CARRY-AGREES** · 성질

통이 빈 `ACTIVE` 에서 클라의 방 · 발행 상태(세션이 끊길 때 읽는 이어받을 것)와 서버 세션이 가진 방 · 발행이 같다.

*식: SH7 · 깨지면: 새 세션을 이어받았다고 믿는다 — 클라는 방이 있다고 알고 서버는 그 방에 이 클라를 모른다. 미디어가 살아 있어도 통지 · 발언권이 오지 않는다*

#### 3-4-2. 상태를 앱에 알린다

<a id="PROP-SES-REJOIN-MARK-USED"></a>**PROP-SES-REJOIN-MARK-USED** · 성질

다시 들어갈 방 표시는 쓰이거나 지워진다 — 통이 빈 `ACTIVE` 에는 남은 표시가 없고, 토큰을 기다리는 때가 아닌 `DISCONNECTED` 에도 없다(DEV-SES-FRESH-REJOIN).

*식: SH11 SH12 · 깨지면: 새로 붙은 단말이 방 없이 남거나(쓰이지 않은 표시) · 앱이 끊은 뒤의 접속이 옛 방을 되살린다(지워지지 않은 표시)*

<a id="RULE-SES-APP-EXPOSE"></a>**RULE-SES-APP-EXPOSE** · 규칙

| 무엇 | 값 |
|---|---|
| 연결 상태 | §3-4-1 의 넷 |
| 끊긴 사유 | ★`LEAVE` 의 사유(§3-1). `LEAVE` 없이 끊겼으면 "없음" — 그것도 값이다 |

#### 3-4-3. 클라가 정하지 않는 것

<a id="RULE-SES-SERVER-POLICY"></a>**RULE-SES-SERVER-POLICY** · 규칙

| 무엇 | 어디서 오나 |
|---|---|
| 하트비트 주기 | `BIND` 응답의 `heartbeat_interval` |
| 이어받기 가능 시간 | `BIND` 응답의 `resume_window_ms` |

#### 3-4-4. 원천이 더한 것 — 출처 절이 없는 진술

<a id="DEV-SES-REBUILD-REPUBLISH"></a>**DEV-SES-REBUILD-REPUBLISH** · 자체 확장

세션 전체 재구축에서 다시 들어가는 갈래(`bind_new` · `resume_2008`)는 다시 들어간 뒤 다시 발행한다 — 앱이 켜 둔 트랙(`want`)만, 처음 켤 때와 같은 절차(그 서버 재구축의 `PUB.C.any.reregister` 와 같다)

*원문: §3-2-3 3 — `ROOM_JOIN` 부터 다시 한다는 것만 정한다 · 근거: R-SES-REBUILD-REPUBLISH*

### 3-5. 근거 · 기각 — 규격이 아니다

> 계약 본문(구조 · 동작 · 주제)의 규칙이 왜 그런가(R) · 무엇을 하지 않기로 했나(REJ). 소절 머리는 그 근거가 붙은 계약 자리다. 계약 판에는 싣지 않는다.

#### 3-5-1. §3-1-1 `0x0101 BIND` — 접속한다   **C→S** · 응답 있음

<a id="R-SES-WINDOW-SYMPTOM"></a>**R-SES-WINDOW-SYMPTOM** · 근거

★**증상** — 지하·터널·기내처럼 60초를 넘겨 끊긴 뒤 ★**앱이 새 토큰을 안 주면 영영 못 돌아온다.**
망은 살아났고 서버도 멀쩡한데 재접속만 `2003` 으로 반복된다. ★**앱이 할 일은 토큰을 미리 받아 두는 것**이다 —
새 토큰 요청을 받은 다음에 백엔드를 부르면 그 왕복만큼 더 늦는다.

<a id="R-SES-NO-NEW-SECRET"></a>**R-SES-NO-NEW-SECRET** · 근거

★**왜 창 밖 자격을 새로 만들지 않았나** — 장수명 재접속 토큰 같은 것을 두면 ★**새 비밀이 하나 더 생긴다**(§2-2-3
*"새 비밀을 만들지 않는다"*). 창 안 복구는 `session_id` 가 이미 덮고 있고, 창 밖은 ★**정상 인증으로 돌아가는 것이 맞다** —
그 사이 사람의 자격이 바뀌었을 수 있고, 그것을 다시 보는 자리가 토큰이다.

<a id="R-SES-BIND-SYMPTOMS"></a>**R-SES-BIND-SYMPTOMS** · 근거

★**증상**
- 질의값에 토큰을 실으면 → **액세스 로그에 남는다.**
- 하트비트를 안 시작하면 → **30초 뒤 끊긴다.**
- ★`client_ver`/`server_ver` 를 주고받으면서 불일치 실패를 안 두면 → 세대가 어긋난 클라가
  ★**붙은 채로 엉뚱한 body 를 보내다** `1002` 를 흩뿌린다. 원인이 세대라는 것을 아무도 모른다.
- ★응답 에코를 안 보면 → 자기가 `1pc` 로 도는지 `2pc` 로 도는지 ★**서버에게 확인할 방법이 없다.**

#### 3-5-2. §3-1-2 `0x0102 RESUME` — 시그널만 끊겼다 다시 붙었다   **C→S** · 응답 있음

<a id="R-SES-RESUME-PRIOR-ART"></a>**R-SES-RESUME-PRIOR-ART** · 근거

> **본뜬 곳** — LiveKit 의 `resume` / `ICE restart` / `full reconnect` 3단(`livekit §3-1`~`§7-1-2`).
> ★**요청 body 는 본뜨지 않는다** — LiveKit `SyncState`(클라가 자기 상태를 통째로 신고)는 채택하지 않는다(아래 "신고가 없는 이유").

<a id="R-SES-ICE-RESTART-WHY"></a>**R-SES-ICE-RESTART-WHY** · 근거

★**2단(ICE 재시작)이 따로 있는 이유** — 서버 후보는 host 하나라(§9-4-8·§9-4-9) 경로를 "다시 뚫을" 것은 없다.
★**되살리는 것은 경로가 아니라 브라우저 안의 연결 상태다.** 브라우저는 연결을 잃으면 후보 쌍을 지우고
`failed` 에 머문다 — W3C `RTCIceTransportState`: *"This is a terminal state until ICE is restarted"* · ★*"entering the "failed"
state does not cause DTLS transports, SCTP associations or the data channels that run over them to close"*. 같은 자격으로는
돌아오지 않고, 지명된 쌍을 바꾸는 것도 표준상 restart 로만 한다(RFC 8445 §8.1.1·§9-4-4). 3단으로 가면 DTLS·구독·발행·
발언권을 전부 버리고 다시 짓는다 — ★**2단은 그 값을 치르지 않는다**(지문·`tls-id` 가 같으면 같은 DTLS 가 새 ICE 위에서
이어진다, RFC 8829). §9-4-9 의 "자격을 계속 같은 값으로" 는 ★**restart 가 아닐 때**의 규칙이다.

<a id="R-SES-RESUME-NO-REPORT"></a>**R-SES-RESUME-NO-REPORT** · 근거

★★**신고가 없는 이유** — `rooms[]`·`publish[]` 로 *"살아 있는 것"* 을 신고하고 서버가 방마다 `resumed`/`failed` 를
가르는 형태를 쓰지 않는다. 그 신고는 ★**무증상 실패 모드**를 갖는다 — 클라가 죽은 방을 산 것으로 신고하면 서버는 `resumed` 로 답하고,
★**서버는 보내고 있다고 알고 클라는 아무것도 못 받는다.** 양쪽 계수기가 다 정상으로 보인다. 그리고 복구의 실질 입도는
방이 아니라 ★**미디어 서버**다(한 서버가 살면 그 서버의 방은 다 살고, 죽으면 다 죽는다 — 한 서버 안에서 방 하나만 죽는
경우는 `3001` 로 어차피 못 살린다). 신고를 없애면 그 실패 모드가 원리적으로 사라지고, `RESUME` 은 협상이 아니라
★**조회**가 된다 — 서버는 아는 것을 다 내고, 판단은 클라가 스냅샷을 받아 보고 한다.

<a id="R-SES-RESUME-SYMPTOMS"></a>**R-SES-RESUME-SYMPTOMS** · 근거

★**증상**
- 이어받을 것이 없는데도 보내면 → **빈 왕복이 하나 더** 붙는다(윈도우가 좁으면 첫 요청이 그만큼 늦다).
- 미디어가 살아 있는데 안 보내면 → 재접속이 **복구가 아니라 재구축**이다. 잠깐 끊겨도
  **영상이 끊겼다 붙는다.**
- ★스냅샷을 미디어와 대조하지 않으면 → 죽은 서버의 방을 산 것으로 두고 ★**서버는 보내고 있다고 알고, 클라는 아무것도
  못 받는다** — 양쪽 계수기가 다 정상으로 보인다.
- ★`snapshot` 을 안 쓰면 → 끊겨 있는 동안 들어온 사람이 **명단에 없고**, 그 사이 올라온 트랙이
  **영영 안 붙는다.** 통지는 이미 사라졌으므로 다시 오지 않는다.
- 큐를 재전송하면 → 서버는 그것을 **새 요청**으로 본다. `PUBLISH_TRACKS` 가 중복 발행돼
  상한을 소진해 `4002 TRACK_LIMIT` 이 난다.

#### 3-5-3. §3-1-3 `0x0103 HEARTBEAT` — 하트비트   **양방향** · 응답 있음

<a id="R-SES-HEARTBEAT-SYMPTOM"></a>**R-SES-HEARTBEAT-SYMPTOM** · 근거

★**증상** — 연결은 살아 있는데 미디어가 죽은 상태가 생긴다. ★**두 축을 겹쳐 봐야 갈린다.**

#### 3-5-4. §3-1-4 `0x0104 LEAVE` — 끝낸다   **양방향** · 응답 없음

<a id="R-SES-LEAVE-PRIOR-ART"></a>**R-SES-LEAVE-PRIOR-ART** · 근거

> **본뜬 곳** — LiveKit `LeaveRequest`(양방향 · `DisconnectReason` — 시그널 소켓이 그냥 닫힌 것은 그중 한 사유일 뿐이다) ·
> MQTT 5 `DISCONNECT`(양방향 · 정상 종료를 받았느냐가 세션 정리를 바꾼다) · SSH `SSH_MSG_DISCONNECT`(TCP 는 사유를 안 나른다 —
> 앱 메시지가 한다 · 받는 쪽 MUST, 보내는 쪽 SHOULD) · HTTP/2 `GOAWAY`(★best-effort 를 조문으로 둔다). 전송층 close 를 사유의
> 정본으로 쓰는 규격은 찾지 못했다. `RESUME` 은 LiveKit 을 본떴는데 `LeaveRequest` 만 빠져 있던 자리다.

#### 3-5-5. §3-1-5 `POST /auth/token` — 토큰 발급

<a id="R-SES-TOKEN-SYMPTOM"></a>**R-SES-TOKEN-SYMPTOM** · 근거

★**증상** — 클라에 비밀키를 두면 **아무나 아무 신원으로 토큰을 만든다.**

#### 3-5-6. §3-1-6 LEAVE 사유

<a id="REJ-SES-NO-WHERE-AXIS"></a>**REJ-SES-NO-WHERE-AXIS** · 기각

★**"어디에 붙나" 축은 두지 않는다** — 재접속 시각(`reconnect_after_ms`)은 §11-2-2 이 이미 기각한 형태이고(서버가 그 시각을 아는
경우가 없다), 다른 접속점(`endpoint`)은 §2-2-2 을 깬다(앱이 `{base}` 를 준다). 사유 코드만이다.

<a id="R-SES-LEAVE-IMMEDIATE-RECLAIM"></a>**R-SES-LEAVE-IMMEDIATE-RECLAIM** · 근거

★**딸려 오는 이득 — 세션 즉시 회수.** 클라 `LEAVE` 를 받은 세션은 ★**그 자리에서** 폐기되고, FIN/RST 만 온 세션은 `resume_window_ms`
(60초) 동안 보관된다. 사용자가 앱을 정상 종료했는데 서버가 60초 동안 방·명단·정원을 붙드는 일이 없어진다 — 발언권은 `T1` 이
덮지만 명단·정원은 창이 다 돌아야 비던 자리다. ★TCP 직결이면 FIN 만으로 *"의도적 종료"* 와 *"죽음"* 이 구분되지 않는다 — 서버가
그것을 알 유일한 수단이 이 프레임이다.

<a id="R-SES-LEAVE-SYMPTOMS"></a>**R-SES-LEAVE-SYMPTOMS** · 근거

★**증상**
- 사유를 WS Close 코드에 실으면 → 전송을 바꾸는 순간 사유 체계가 통째로 붕 뜬다. 채널이 둘이면 어긋난 값이 나온다.
- ★클라가 `LEAVE` 없이 소켓만 닫으면 → 서버가 60초 동안 방·명단·정원을 붙든다. 나간 사람이 명단에 남는다.
- ★`ROOM_LEAVE` 를 방마다 보내고 `LEAVE` 하면 → 왕복만 는다 — 남들 화면에는 차이가 없다(`PARTICIPANT_EVENT{left}` × N 은 같다).
- ★서버가 `LEAVE` 의 응답을 기다리고 닫으면 → 닫지도 못하고 굳는다. 보내고 닫는다.
- ★미디어 축을 사유별로 안 가르면 → 축출당한 클라가 죽은 세션의 PeerConnection 을 붙들고 STUN 을 계속 흘린다(§9-4-5 의 상시 1.32 kbps 가 헛돈다).

#### 3-5-7. §3-2-3 클라 · `RESUMING`

<a id="R-SES-RESUME-OK-SYMPTOMS"></a>**R-SES-RESUME-OK-SYMPTOMS** · 근거

★**증상** — 스냅샷이 통째로 비어 오는 것은 ★**그 세션에 방이 없을 때**뿐이다 — 그것은 §3-4-1 가 `RESUME` 을 안 보내는
경우라 정상이면 여기서 볼 일이 없다. 창을 넘겨 온 재접속은 `2008` 이지 빈 스냅샷이 아니다(§3-2-3 가 ★**보내기 전에
경과 시간을 본다** — 백오프 대기만 더하면 44~52초라 60초 안이지만 시도마다 `T-bind` 가 붙으면 그 합이 윈도우를 훌쩍
넘긴다). ★**백오프 시계는 백그라운드에서도 돈다** — 브라우저는 숨은 페이지의 타이머를 늦출 뿐(1초 정렬) 세우지 않고,
DC 가 열린 페이지는 강한 스로틀링에서 빠진다. ★**프로세스가 통째로 얼었다 깨면** 밀린 타이머가 한꺼번에 터지므로 그 만료를
시한 초과로 세지 않고 ★**경과 시간으로 처음부터 다시 판정한다**(§9-2-1 깨어남).

<a id="R-SES-TBIND-ADDS"></a>**R-SES-TBIND-ADDS** · 근거

★★**그런데 대기 시간만 더하면 안 된다.** ★**시도마다 `T-bind`(10초, §3-2-4)가 붙을 수 있다** —
서버가 죽은 것이 아니라 **느리기만 할 때**(과부하)가 정확히 그 경로다.

```
대기만       :  0+300+1200+2700+4800+7000×5           =  44초
지터 최악    :  +1,000ms × 8회(3번째부터)              =  52초
★T-bind 최악 :  +10초 × 10회                          = ★152초  →  윈도우(60초)의 두 배 넘음
```

<a id="R-SES-CLOCK-SYMPTOM"></a>**R-SES-CLOCK-SYMPTOM** · 근거

★**증상** — 경과 시간을 안 보면 → 과부하 상황에서 ★**언제나 `2008` 을 한 번 받고** 전면
재구축한다. 왕복 하나가 헛돌고, 로그에는 *"세션이 없다"* 만 남아 원인이 과부하로 안 보인다.

<a id="R-SES-REBUILD-JITTER-SYMPTOM"></a>**R-SES-REBUILD-JITTER-SYMPTOM** · 근거

★**증상** — 재구축 첫 시도가 0ms 면 → 서버 하나가 다시 뜬 순간 그 서버의 단말 전부가 같은 틱에 `ROOM_JOIN` 을 보내
★**막 뜬 서버가 그 파도에 다시 눕는다.**

<a id="R-SES-RES-OPEN-WHY"></a>**R-SES-RES-OPEN-WHY** · 근거

- `session_id` 가 유효하면 서버는 토큰을 보지 않는다.
- 백오프 대기와 `T-bind` 소모가 쌓이면 윈도우를 넘길 수 있다(§3-2-3). ★**어차피 죽은 세션을 알면서 왕복을 태우지 않는다.** (이때 토큰이 만료 상태면 `2003` 이 온다 — §3-2-3 로.)
- ★*"끊긴 순간"* 은 그 세션에 물려 있던 소켓이 마지막으로 끊긴 순간이다 — 같은 세션으로 다시 물리면 시계를 새로 잰다(DEV-SES-LATE-CLOCK-LAST-BOUND).
- 응답의 `session_id` 를 봐야 이어받을 세션이 있는지 안다(§3-2-3). ★파이프라인으로 앞질러 보내면 새 세션일 때 `2008` 응답을 버리는 자리와 서버측 인증 전 큐가 생긴다 — 1 RTT 를 사려고 예외를 파는 것이라 하지 않는다.
- ★**그 사이 미디어 생존을 다시 본다** — 백오프를 도는 동안 미디어가 죽었을 수 있다. ★발행 트랙의 생존 기준은 **발행이 살아 있는가**(`REGISTERED` 이상)이지 RTP 를 보내고 있는가가 아니다 — 반이중 마이크는 발화 밖에선 RTP 가 없다(§7-2-2).
- ★★**연결이 죽었어도 `RESUME` 을 보낸다** — Peer 는 세션과 수명이 같아(성립 조건 색인) 서버에 남아 있고, 죽은 연결은 응답 뒤 ICE 재시작으로 되살린다(§3-2-3). `RESUME` 을 거르는 것은 이어받을 것(방·발행)이 없을 때뿐이다(§3-4-1).

<a id="R-SES-RES-BIND-RES-WHY"></a>**R-SES-RES-BIND-RES-WHY** · 근거

- `session_id` 가 무효인 경우는 창 만료·운영자 절단(`2009`)·서버 재기동 셋뿐이고 셋 다 ★**서버가 Peer 를 이미 회수했다** — 붙들 미디어가 서버에 없다.
- 새 토큰이 오면 §3-2-1 부터다(새 세션 · 전면 재구축).

<a id="R-SES-RES-RESUME-OK-WHY"></a>**R-SES-RES-RESUME-OK-WHY** · 근거

- 끊겨 있는 동안의 WS 통지는 사라졌으므로 이것이 유일한 따라잡기 수단이다.
- 연결이 살아 있었으면 DC 로 계속 받아 왔다(§10-4-32). 죽어 있었으면 되살린 뒤 재동기가 따라잡는다(§9-2-1).
- 끊긴 동안 강퇴·폭파·방 닫힘이 있었고 그 통지(§4-2-6)가 사라진 것이다.

<a id="R-SES-RES-EXHAUST-WHY"></a>**R-SES-RES-EXHAUST-WHY** · 근거

- 세션을 포기한 것이다. 서버는 재개 창이 끝날 때 세션과 함께 Peer 를 거둔다(§1-2-6 — 백오프 합계가 창보다 짧아 이 순간엔 아직 남아 있을 수 있다). 열어 두면 버린 세션에 STUN 을 흘리다 `failed` 로 남는다.

<a id="R-SES-CLIENT-WATCHES-CLOCK-WHY"></a>**R-SES-CLIENT-WATCHES-CLOCK-WHY** · 근거

- ★**어차피 죽은 세션임을 알면서 이어받기 왕복을 태우지 않는다.** 제일 정직하다

<a id="R-SES-RESUME-2008-DEFENSIVE"></a>**R-SES-RESUME-2008-DEFENSIVE** · 근거

`RESUME` 의 `2008` 처리(§3-2-3 방아쇠 ①)는 방어 갈래다 — 직렬 `BIND` → `RESUME`(§3-2-3 3) 아래에서 `BIND` 가 같은 세션을 돌려준 뒤 같은 소켓의 `RESUME` 이 `2008` 을 받는 길이 없다(세션을 없애는 사건은 소켓이 물려 있는 동안 창이 돌지 않거나 소켓을 끊는다). 서버 결함 · 구현 경주에 대비해 처리는 그대로 둔다

*없으면: 서버 쪽 결함으로 2008 이 와도 받을 갈래가 없다*

#### 3-5-8. §3-2-4 클라 · `any`

<a id="R-SES-FRESH-REJOIN"></a>**R-SES-FRESH-REJOIN** · 근거

원칙 ①(전부 다시)은 세션이 다시 지어지는 모든 길에 같아야 한다 — 회수 · 토큰 만료 뒤 새로 붙는 것도 앱이 보기에 세션이 다시 지어진 것이다. 자격이 정말로 사라졌으면 그 다시 붙기가 2003 로 막힌다. 반대로 세션이 끝난 길에서 표시가 남으면 앱이 끊었다 다시 붙을 때 옛 방에 저절로 들어간다

*없으면: 회수 · 토큰 만료 뒤 새로 붙은 단말이 방 없이 남는다(남긴 표시는 쓰는 곳이 없다) · 또는 앱이 끊은 뒤의 접속이 옛 방을 되살린다*

<a id="R-SES-BACKOFF-RESET-STABLE"></a>**R-SES-BACKOFF-RESET-STABLE** · 근거

성공하자마자 칸을 처음으로 돌리면, 붙자마자 끊기는 망(재구축 도중 · 과부하 서버)에서 클라가 매번 첫 칸(0ms)부터 다시 붙는다 — 단말 전부가 쉬지 않고 붙었다 끊기를 되풀이해 백오프가 없는 것과 같다

*없으면: 붙었다 끊기는 서버에 0ms 재시도가 몰린다*

<a id="R-SES-ON-DROP-WHY"></a>**R-SES-ON-DROP-WHY** · 근거

- ★**`LEAVE` 없이 끊겼으면 사유가 없다** — 망 절단·서버 급사·서버가 못 보낸 경우가 다 여기이고, 그때는 4 로 간다(사유 없는 끊김이 정상 경로다).
- ★**`2009`(`SESSION_REVOKED`)면 이어받기를 건너뛴다** — 운영자가 세션을 폐기했으므로 서버에 이어받을 것이 **없다**(`RESUME` 을 보내면 `2008`).
- ★**미디어는 이때 닫는다** — Peer 가 서버에서 이미 회수됐다(§3-2-3 전면 재구축과 같은 자리).
- `5004`(`SERVER_SHUTDOWN`)도 같다 — 그 서버가 내려가므로 이어받을 것이 없다
- ★**미디어 연결(PeerConnection)은 닫지 않는다.** WS 만 끊긴 것이고, 살아 있는 미디어를 이어 쓰는 것이 `RESUME` 의 목적이다(★2·3 의 사유는 예외 — §3-1 표의 미디어 축).

#### 3-5-9. §3-2-6 서버

<a id="R-SES-EVICT-OLD-WHY"></a>**R-SES-EVICT-OLD-WHY** · 근거

- 방 축출(§4-1)과 같은 결이다: 단말을 바꿔 다시 로그인하는 것이 무전의 일상이라, ★**옛 좀비가 새 접속을 막으면 사용자가 잠긴다.**
- 같은 세션을 이어받는 것이라 충돌 자체가 없다.

<a id="R-SES-RESUME-PROC-WHY"></a>**R-SES-RESUME-PROC-WHY** · 근거

- 끊겨 있는 동안의 `PARTICIPANT_EVENT`·`TRACK_EVENT`·`TRACK_STATE` 는 ★**받을 소켓이 없어서 사라졌다** — 스냅샷이 그것을 대신한다
- 미디어 연결이 살아 있었으면 ★**DC 도 살아 있어 발언권 통지가 끊긴 적이 없고**, 죽어 있었으면 2단으로 되살린 뒤 ★**재동기**(§9-2-1)가 따라잡는다. 발언권 통지 권위는 DC 단일이다(§10-4-28)

<a id="R-SES-HEARTBEAT-WHY"></a>**R-SES-HEARTBEAT-WHY** · 근거

- 앞선 요청이 느려도 줄 서지 않는다.
- 미디어가 살았는지는 별도로 본다(STUN).

<a id="R-SES-LEAVE-FORM-WHY"></a>**R-SES-LEAVE-FORM-WHY** · 근거

- *"자원을 남기고 나간다"* 는 뜻을 실을 자리를 두지 않는다 — 세션이 끝나면 그 세션의 자원(방·발행·발언권·Peer)은 전부 간다(§3-1 축출 · §4-1 · §5-1-2 *트랙은 세션 것*)
- 한쪽 값만 구현 가능한 필드는 두지 않는다(`pub_room` 이 단수인 것과 같은 규율)
- 사유가 닫힌 집합이라 *"영구인가"* 불리언이 답할 것이 없다 — 재접속·미디어 두 축은 아래 표가 사유마다 답한다

<a id="R-SES-LEAVE-ONE-NUMBER-SPACE-WHY"></a>**R-SES-LEAVE-ONE-NUMBER-SPACE-WHY** · 근거

- 그래서 옛 WS Close 대역(`4000~4006`)이 사라지고, `4003`·`4004`·`4005` 가 Failure 와 Close 로 두 뜻이던 충돌도 함께 사라진다
- ★**번호 하나에 뜻 하나다.**

<a id="R-SES-LEAVE-PROC-WHY"></a>**R-SES-LEAVE-PROC-WHY** · 근거

- `CONNECTING`·`RESUMING` 에서는 그냥 닫는다 — 세션이 아직 없거나 포기한 것이다
- 응답을 기다리지 않는다(기다리면 닫지도 못하고 굳는다)
- 10방을 듣던 세션이 10왕복을 태우고 나가는 꼴을 만들지 않는다
- 퇴장이 곧 반환이다(§10-4-32)
- 고리를 만들지 않는다

<a id="R-SES-SOCKET-CLOSE-NORMAL-WHY"></a>**R-SES-SOCKET-CLOSE-NORMAL-WHY** · 근거

- 클라 버그(`1008`)에서 재시도를 태우게 되는 것은 손해가 아니다 — 망 끊김과 원래 구분되지 않고, 붙어도 같은 짓을 하다 `4008`(`BIND` 10초)에 다시 걸린다.

#### 3-5-10. §3-4-1 연결 상태

<a id="R-SES-RESUME-KEEPS-MEDIA"></a>**R-SES-RESUME-KEEPS-MEDIA** · 근거

★★**`RESUMING` 은 아무 재접속이 아니다.** ★**`RESUME` 은 미디어를 지키려고 있는 것이다** —
WS 는 시그널링일 뿐이고, WS 가 끊겨도 **WebRTC 연결(ICE·DTLS·RTP)은 살아 있다.**
`RESUME` 은 *"이 세션의 방·트랙을 그대로 두고 새 소켓에 다시 물려라"* 고 말해서
★**살아 있는 미디어를 재협상 없이 이어 쓰는 것**이다.

<a id="R-SES-RESUME-CONDITION-SYMPTOMS"></a>**R-SES-RESUME-CONDITION-SYMPTOMS** · 근거

★**증상** — 조건을 안 보고 **항상 `RESUME`** 을 보내면, 이어받을 것이 없는 재접속마다
**빈 왕복이 하나 더** 붙는다(윈도우가 좁으면 그만큼 첫 요청이 늦다). 반대로 ★**미디어가 살아 있는데
`RESUME` 을 안 보내면** 재접속이 **복구가 아니라 재구축**이 되어 **영상이 끊겼다 붙는다.**

<a id="R-SES-NO-CARRY-NO-RESUMING-WHY"></a>**R-SES-NO-CARRY-NO-RESUMING-WHY** · 근거

- Peer 는 세션과 수명이 같아 서버에 남아 있다
- ★**어차피 죽은 세션이다**(§3-2-3)

#### 3-5-11. §3-4-4 원천이 더한 것 — 출처 절이 없는 진술

<a id="R-SES-REBUILD-REPUBLISH"></a>**R-SES-REBUILD-REPUBLISH** · 근거

원칙 ①(20261011) — 전부 다시는 발행도 전부 다시다(LiveKit `Room.handleSignalRestarted` → `republishAllTracks`). 그 서버 재구축과 세션 재구축의 결말이 같아야 앱이 둘을 가르지 않는다

*없으면: 세션이 재구축된 뒤 방에는 다시 들어갔는데 마이크 · 카메라가 서버에 발행되지 않아 아무도 듣지 못한다*

<a id="ROOM"></a>
