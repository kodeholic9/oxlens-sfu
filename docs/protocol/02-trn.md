[← 차례](README.md)

## 2. TRN — 운반

WS 프레임 · 흐름 제어 · DC 프레임 · HTTP 공통 — 무엇을 나르든 같은 형식

<a id="RULE-TRN-THREE-CARRIERS"></a>**RULE-TRN-THREE-CARRIERS** · 규칙 — 전송 셋

| | 무엇을 나르나 | 형식 | 방향 |
|---|---|---|---|
| **HTTP** | 토큰 발급 · 방 조회/생성 | JSON | 클라·앱백엔드 → 서버 |
| **WS** | 실시간 신호 | 8B 헤더 + JSON | 양방향 |
| **DC** | ★**발언권**(§10) | 4B 헤더 + TLV(바이트) | 양방향 |

<a id="RULE-TRN-OPS-PATHS-OUT"></a>**RULE-TRN-OPS-PATHS-OUT** · 규칙

★**운영 경로(`/admin/*` · `/healthz/*`)는 이 문서 밖이다.** 클라가 쓰지 않는다.

### 2-1. 구조

<a id="TRN.oplist"></a>
#### 2-1-1. WS 메시지 — 전량

| 번호 | 이름 | 방향 |
|---|---|---|
| `0x0101` | BIND | C→S |
| `0x0102` | RESUME | C→S |
| `0x0103` | HEARTBEAT | 양방향 |
| `0x0104` | LEAVE | 양방향 |
| `0x0201` | ROOM_JOIN | C→S |
| `0x0202` | ROOM_LEAVE | C→S |
| `0x0301` | PUBLISH_TRACKS | C→S |
| `0x0302` | READY | C→S |
| `0x0303` | SUBSCRIBE_LAYER | C→S |
| `0x0304` | TRACK_SET | C→S |
| `0x0305` | ICE_RESTART | C→S |
| `0x0401` | AFFILIATION | C→S |
| `0x0501` | MESSAGE | 양방향 |
| `0x0601` | TASK | 양방향 |
| `0x0701` | PARTICIPANT_EVENT | S→C |
| `0x0702` | TRACK_EVENT | S→C |
| `0x0703` | TRACK_STATE | S→C |
| `0x0704` | ROOM_EVENT | S→C |
| `0x0705` | PARTICIPANT_STATE | S→C |
| `0x0706` | LINK_EVENT | S→C |
| `0x0707` | MLINE_EVENT | S→C |

<a id="TRN.frame.WS"></a>
#### 2-1-2. WS 프레임

```
 0        1        2        3        4        5        6        7
+--------+--------+-----------------+-----------------------------------+
| ver=1  | flags  |   op (u16 BE)   |            pid (u32 BE)           |
+--------+--------+-----------------+-----------------------------------+
| body = JSON (길이 = 프레임 길이 − 8)                                    |
```
body — JSON(UTF-8). ★**빈 body 는 0바이트**(헤더 8B 만). 받는 쪽은 `{}`(2바이트)도 같게 받는다

| 항목 | 바이트 | 규격 |
|---|---|---|
| `ver` | 1 | ★**0x01 고정.** 다르면 **끊는다** |
| `flags` | 1 | — |
| `flags` bit0-1 | 2 비트 | 아래 표 |
| `flags` bit2-7 | 6 비트 | 예약. ★**파서가 원본을 보존한다**(나중에 늘려도 안 깨지게) |
| `op` | 2 | 메시지 종류 |
| `pid` | 4 | 보내는 쪽이 매긴다 — `u32`, ★**자기 안에서 하나씩 올린다**(넘치면 `0` 으로 감긴다). ★**응답은 같은 번호를 되돌려준다.** ★**두 방향이 각자 매기므로 번호가 겹칠 수 있다** — 짝은 **방향까지 보고** 짓는다(내가 보낸 `00` ↔ 상대가 준 `01`/`10`) |
| WS opcode | — | ★**binary 고정.** text 프레임은 받지 않는다 |

<a id="RULE-TRN-UNKNOWN-JSON-FIELD"></a>**RULE-TRN-UNKNOWN-JSON-FIELD** · 규칙

★**JSON 의 모르는 필드는 무시한다.** 서버가 필드를 늘려도 구버전 클라가 안 깨진다
(DC TLV 의 같은 규율은 §10-4-29).

<a id="RULE-TRN-TWO-FRAMES"></a>**RULE-TRN-TWO-FRAMES** · 규칙 — ★프레임은 두 갈래뿐이다

| `flags` bit0-1 | 무엇 | `op`·`pid` |
|---|---|---|
| `00` | **요청 · 통지** — 보내는 쪽이 먼저 말한다 | 보내는 쪽이 매긴다 |
| `01` | **응답(성공)** | 받은 것을 **그대로 되돌린다** |
| `10` | **응답(실패)** — body 는 `Failure`(§11-1-1) | 같음 |
| `11` | 예약 — 받으면 **끊는다** | — |

→ 구조 — [`TRN.frame.WS`](#TRN.frame.WS)

<a id="RULE-TRN-ONE-REPLY"></a>**RULE-TRN-ONE-REPLY** · 규칙

★★**`00` 프레임 하나에 `01` 또는 `10` 프레임 하나가 돌아온다. 그것이 전부다.**
다른 종류는 없다. ★예외는 `LEAVE` 하나다 — 양방향 모두 응답이 없다(보내고 닫는다, §3-1-4).

→ 구조 — [`TRN.oplist`](#TRN.oplist)

<a id="RULE-TRN-CLOSE"></a>**RULE-TRN-CLOSE** · 규칙 — 끊는 조건

아래는 ★**응답을 지을 수 없거나 대화를 이어갈 수 없는 경우**다.
응답은 `op`·`pid` 를 그대로 되돌리는 물건이라, ★**그 둘을 못 믿으면 응답 프레임을 지을 수가 없다.**
그래서 응답 대신 ★**`LEAVE`(§3-1)를 보내고 연결을 닫는다** — 사유는 그 프레임이 나른다(WS Close 에는 싣지 않는다).
근거는 둘이다 — ①`op`·`pid` 를 못 믿는다 ②★**방아쇠가 된 클라 프레임이 없다**(답할 요청이 자기 소켓에 없다).

| 조건 | 경계 | 왜 응답이 아니라 `LEAVE` 인가 | `LEAVE` 사유 |
|---|---|---|---|
| 프레임이 너무 크다 | **1,048,576 byte** 초과 | ① 읽기 전에 자원을 먹는다 | `1008` |
| 8바이트가 안 된다 / `ver`≠1 / `flags` 하위 2비트가 `11` | — | ① ★`op`·`pid` 를 못 믿는다 — best-effort | `1008` |
| body 가 JSON 이 아니다 | — | ① ★`op`·`pid` 는 읽히나 대화가 이미 어긋났다 — 응답과 `LEAVE` 를 둘 다 낸다 | `1008` |
| ★이미 묶인 소켓에 두 번째 `BIND` | — | ① 같음 — 응답과 `LEAVE` 를 둘 다 낸다(§3-1-1) | `1008` |
| 아무것도 안 보낸다 | **30초** | ② 대상 프레임 자체가 없다 | `4008` |
| 보낸 것에 응답을 안 준다 | **30초** | ② 같음 | `4006` |
| 밀린 프레임이 너무 많다 | **1,000** 개 초과 | ② 상대가 못 따라간다 — 못 보낼 수 있다 | `4007` |
| ★운영자가 세션을 끊었다(`cut`) | — | ② 클라 프레임이 없다 | `2009` |
| ★같은 신원이 다른 소켓으로 들어왔다 | — | ② 응답은 ★**다른 소켓**에 갔다 | `2010` |
| ★서버가 내려간다 | — | ② 클라 프레임이 없다 | `5004` |

<a id="TRN.frame.DC"></a>
#### 2-1-3. DC 프레임

```
 0        1        2        3        4 …
+--------+--------+-----------------+-------------------------+
| ver=1  | svc(1) |    len(2 BE)    |   payload (len bytes)   |
+--------+--------+-----------------+-------------------------+
```
body — payload (len bytes)

| 항목 | 바이트 | 규격 |
|---|---|---|
| `ver` | 1 | ★**0x01 고정.** WS 와 첫 바이트 뜻이 같다 |
| `svc` | 1 | `0x01` ★**발언권(MBCP) — 이 문서가 정하는 유일한 svc** · `0x02` ★**발성 감지(확장, 이 문서 밖 — §1-2-1·§10-4-34)** · `0x80~` 앱 예약 |
| `len` | 2 | 최대 `65532`(= 서버 SDP `a=max-message-size` 65536 − 머리 4 — 한 프레임이 한 SCTP 메시지에 든다). ★넘으면 **보내는 쪽이 거부**한다 |
| 잘린 프레임 | — | ★**조용히 버린다.** 예외를 던지지 않는다 |
| 채널 | — | 이름 `"unreliable"` 하나. ★다른 이름은 **서버가 거부**한다 — DCEP 에는 거부 메시지가 없어(OPEN/ACK 둘뿐) 클라에겐 `open` 이 영영 안 오고 `close`/`error` 로만 보인다 |
| 어디에 붙나 | — | ★**발행용 연결**에 붙는다(`1pc` 면 그 하나뿐인 연결). 그것이 안 서면 발언권을 못 쓴다 |

### 2-2. 주제

#### 2-2-1. 흐름 제어

<a id="RULE-TRN-FLOW-CONTROL"></a>**RULE-TRN-FLOW-CONTROL** · 규칙 — 흐름 제어

| 값 | 규격 |
|---|---|
| ★**윈도우** — 동시에 응답을 기다릴 수 있는 개수 | ★**1 ~ 10.** 기본 **1** |
| 넘으면 | 큐에 쌓인다. `1,000` 넘으면 끊긴다 |
| ★**윈도우 밖** | ★**`HEARTBEAT` 하나** — 방 상태를 나르지 않으므로 윈도우를 소비하지 않고, 앞 요청이 응답을 기다리는 중에도 곧바로 나간다. WS 가 살았는지 재는 신호가 요청 통로에 막히면 느린 요청 하나로 산 WS 를 죽었다고 판정한다(§9-2-1 WS 점검). ★단 `BIND` 응답을 받은 소켓에만 나간다 — 그 전에는 다른 op 과 같이 `LEAVE` `2001` 로 끊긴다(§3-1-1) |
| 우선순위 | 4단(0 복구 · 1 세션·방·미디어 · 2 데이터 · 3 진단) — ★**op 별 배정은 아래 표.** ★**같은 단 안에서는 먼저 온 것부터** |
| 재전송 | ★**없다.** 응답이 30초 안에 안 오면 **끊는다** |

<a id="RULE-TRN-LANES"></a>**RULE-TRN-LANES** · 규칙

**op 별 우선순위 단**

| 단 | op |
|---|---|
| **0 복구** | ★**비어 있다 — 예약.** `RESUME` 은 `BIND` 응답 뒤 직렬이라(§3-1·§3-2-3) 앞지를 것이 없어 1단으로 내려왔다. 단은 값·표기를 그대로 두고 쓰는 op 을 두지 않는다(발언권은 WS 가 아니라 DC 로 간다 — §10) |
| **1 세션·방·미디어** | `BIND` · `RESUME` · `HEARTBEAT` · `ROOM_JOIN` · `ROOM_LEAVE` · `PUBLISH_TRACKS` · `READY` · `SUBSCRIBE_LAYER` · `TRACK_SET` · ★`ICE_RESTART` · `AFFILIATION` · S→C 통지 일곱(`PARTICIPANT_EVENT`·`TRACK_EVENT`·`TRACK_STATE`·`PARTICIPANT_STATE`·`ROOM_EVENT`·★`LINK_EVENT`·★`MLINE_EVENT`) · ★`LEAVE`(큐 밖 — 곧바로 나가고 남은 큐는 버린다, RULE-SES-LEAVE-PROC) |
| **2 데이터** | `MESSAGE` |
| **3 진단** | `TASK` |

→ 구조 — [`TRN.oplist`](#TRN.oplist)

<a id="RULE-TRN-ROOM-STATE-ONE-LANE"></a>**RULE-TRN-ROOM-STATE-ONE-LANE** · 규칙

★★**방 상태를 나르는 메시지는 반드시 같은 우선순위 단에 둔다.** ★**순서가 곧 상태**이기 때문이다.
`ROOM_EVENT`(0x0704) · `PARTICIPANT_EVENT`(0x0701) · `TRACK_EVENT`(0x0702) · `TRACK_STATE`(0x0703) · `PARTICIPANT_STATE`(0x0705) ·
`AFFILIATION` 응답(0x0401) 은 ★**전부 1단**이다. 급하다는 이유로 하나만 위로 올리면
★**뒤에 온 것이 먼저 도착해 상태가 되감긴다.**

→ 구조 — [`TRN.oplist`](#TRN.oplist)

<a id="COND-TRN-FIFO-IN-LANE"></a>**COND-TRN-FIFO-IN-LANE** · 성립 조건

★**성립 조건** — 이 규정은 *"같은 단 안에서는 FIFO"* 가 참인 동안만 참이다. 단을 늘리거나
단 안의 순서 규칙을 바꾸는 변경은 ★**이 줄을 먼저 깬다.**

<a id="RULE-TRN-WINDOW-UP-TO-10"></a>**RULE-TRN-WINDOW-UP-TO-10** · 규칙

★**늘려도 된다 — 최대 10.** 붙는 방이 많거나 트랙을 한꺼번에 올릴 때 왕복이 줄어든다.
늘릴 때 지킬 것은 둘이다.

| 늘렸을 때 | |
|---|---|
| ★**서버는 받은 순서대로 처리한다** | 응답이 겹쳐 돌아와도 **적용 순서는 보낸 순서**다 |
| ★**클라는 미리 고치지 않는다** | 응답이 온 것만 반영한다(§5-2-2). 낙관적으로 고치면 겹친 요청에서 상태가 갈린다 |

<a id="PROP-TRN-WINDOW-NO-OPTIMISTIC"></a>**PROP-TRN-WINDOW-NO-OPTIMISTIC** · 성질

겹쳐 보낸 요청은 응답이 온 것만 반영한다 — 클라가 들어가 있다고 아는 방은 통로가 비지 않은 동안에도 그 서버 명단에 있다 · ★없으면 서버가 축출한 것이고 그것을 알리는 입장 응답(그 방을 뺀 sub_rooms)이 이미 통로에 있다(RULE-TRN-WINDOW-UP-TO-10 · RULE-ROOM-JOIN-PROC · 같은 서버 방 둘에 입장 요청을 겹쳐 보내는 세계).

*식: MS4(다판 two_servers) · 깨지면: 클라가 응답 전에 입장했다고 고쳐 그 방에 미디어 · 발언권 요청을 보낸다 — 서버는 아직 모르는 사람이라 거절하거나 흘린다*

<a id="RULE-TRN-WINDOW-PER-DIRECTION"></a>**RULE-TRN-WINDOW-PER-DIRECTION** · 규칙

★**윈도우는 방향마다 각자다.** 서버가 보내는 통지는 내 요청 윈도우를 소비하지 않는다 —
대신 ★**서버도 같은 규칙으로 자기 `00` 프레임의 응답(ACK)을 기다린다** — ★**그 윈도우 값은 서버 정책이다**(범위 1~10 은 이 절이 정한다).
★S→C 통지 여섯은 전부 한 단(1단, 아래 표)의 FIFO 이고(서버가 보내는 `MESSAGE` · `TASK` 는 제 단이다 — 방 상태를 나르지 않는다) WS 가 순서를 지키므로 윈도우가 1 보다 커도 ★**도착 순서가 되감기지 않는다.**
그래서 클라가 ACK 을 미루면 ★**윈도우가 차는 순간 다음 통지가 그만큼 늦는다**(§2-2-5 의 증상이 이것이다).

<a id="RULE-TRN-NO-RATE-LIMIT"></a>**RULE-TRN-NO-RATE-LIMIT** · 규칙

★**속도 제한은 없다.** 작은 윈도우가 이미 그 일을 한다.

#### 2-2-2. 접속점

<a id="RULE-TRN-BASE-GIVEN"></a>**RULE-TRN-BASE-GIVEN** · 규칙 — 접속점과 HTTP

★**앱이 클라에 `{base}` 를 준다.** 클라가 호스트나 경로를 스스로 정하지 않는다.
`{base}` 는 스킴·호스트·기본 경로까지다 — 예: `https://sfu.example.com/media`.

<a id="RULE-TRN-ENDPOINTS"></a>**RULE-TRN-ENDPOINTS** · 규칙 — 접속점

| 무엇 | 주소 |
|---|---|
| HTTP | `{base}/auth/token` · `{base}/rooms` · `{base}/rooms/{room_id}` |
| ★**WS** | `{base}/ws` — 스킴을 `wss`(또는 `ws`)로 바꾼다 |

→ 구조 — [`SES.http.TOKEN`](03-ses.md#SES.http.TOKEN) · [`ROOM.http.LIST_ROOMS`](04-room.md#ROOM.http.LIST_ROOMS) · [`ROOM.http.CREATE_ROOM`](04-room.md#ROOM.http.CREATE_ROOM) · [`ROOM.http.GET_ROOM`](04-room.md#ROOM.http.GET_ROOM)

<a id="RULE-TRN-WS-NO-UPGRADE-AUTH"></a>**RULE-TRN-WS-NO-UPGRADE-AUTH** · 규칙

| 항목 | 규격 |
|---|---|
| HTTP 인증 | ★**`Authorization: Bearer {token}`** 헤더 |
| WS 서브프로토콜 | ★**쓰지 않는다.** `Sec-WebSocket-Protocol` 을 보내지 않는다 |
| ★**WS 업그레이드 인증** | ★★**하지 않는다.** 업그레이드는 무인증으로 열리고 **인증은 `BIND`(§3-1) 가 한다** |

#### 2-2-3. HTTP 공통

<a id="RULE-TRN-HTTP-AUTH-TWO"></a>**RULE-TRN-HTTP-AUTH-TWO** · 규칙 — HTTP 공통

전부 `{base}` 아래. ★**인증은 두 갈래이고 둘 중 하나면 통과한다.**

| 헤더 | 누가 쓰나 | 언제 |
|---|---|---|
| `Authorization: Bearer {token}` | 앱 백엔드 · 클라 | 토큰이 살아 있을 때. ★**미리보기**(§4-1-14)가 이쪽 |
| ★`X-OxLens-Session: {session_id}` | 클라 | ★**클라 경로 셋 전부**(§4-1-12·§4-1-13·§4-1-14 — 재동기가 이 헤더를 둔 이유이고, 교대 중 미리보기·목록도 같은 `401` 에 걸린다). WS 세션이 살아 있는 동안 유효 |

<a id="RULE-TRN-NO-NEW-SECRET"></a>**RULE-TRN-NO-NEW-SECRET** · 규칙

★**새 비밀을 만들지 않는다.** `session_id` 는 재접속 `BIND`(§3-1)의 자격증명이라 ★**이미 비밀 취급**이고
(남이 알면 세션을 가로챈다), 그것을 그대로 쓴다. ★비밀로 다룬다는 것의 뜻 — 서버는 예측할 수 없는 값으로 발급하고, 서버 · 클라 모두 `Authorization` · `X-OxLens-Session` · `session_id` 를 로그 · 진단 보고에 남기지 않는다(DEV-TRN-SESSION-SECRET).

<a id="RULE-TRN-SESSION-HEADER-SCOPE"></a>**RULE-TRN-SESSION-HEADER-SCOPE** · 규칙

| 세션 헤더로 왔을 때 | |
|---|---|
| 신원 | ★**그 세션의 신원**으로 다룬다 — `me`(§4-1-14)를 채우려면 어차피 누구인지 알아야 한다 |
| ★**범위 제한** | 그 세션이 ★**입장한 방에만** `me` 를 채운다. 그 밖의 방은 ★**`me` 없이** 나간다 · ★배정표는 어느 방이든 싣지 않는다(§8-1-2) |

<a id="RULE-TRN-HTTP-PATHS"></a>**RULE-TRN-HTTP-PATHS** · 규칙

→ 구조 — [`ROOM.http.LIST_ROOMS`](04-room.md#ROOM.http.LIST_ROOMS) · [`ROOM.http.CREATE_ROOM`](04-room.md#ROOM.http.CREATE_ROOM) · [`ROOM.http.GET_ROOM`](04-room.md#ROOM.http.GET_ROOM) · [`SES.http.TOKEN`](03-ses.md#SES.http.TOKEN)

<a id="RULE-TRN-CORS"></a>**RULE-TRN-CORS** · 규칙 — ★브라우저 클라와 origin

★★**`/rooms` 세 자리는 브라우저가 부른다.** 페이지 origin 이 `{base}` 와 다르면 ★**CORS 가 없으면 막힌다** — 그리고 ★`X-OxLens-Session` 이 안전목록 밖 헤더라 ★**매 요청 앞에 preflight 가 붙는다.**

| 항목 | 규격 |
|---|---|
| 어디에 붙나 | ★**클라 경로 셋에만** — `GET /rooms` · `POST /rooms` · `GET /rooms/{room_id}`. ★`/auth/token` 은 **앱 백엔드 몫**이고 `/admin/*`·`/healthz` 는 운영이라 붙지 않는다 |
| 허용 origin | 서버 정책 `allowed_origins`. ★**빈 목록이면 헤더를 아예 안 낸다** — 같은 origin 배포를 전제한다는 뜻이다 |
| 허용 헤더 | `authorization` · `content-type` · ★`x-oxlens-session` |
| ★**자격증명** | ★**쓰지 않는다.** 인증이 `Authorization`·`X-OxLens-Session` 이라 ambient 권한이 없다 — 쿠키를 쓰지 않으므로 `*` 로도 남의 페이지가 읽을 것이 없다 |
| WS | ★**대상이 아니다.** `{base}/ws` 는 동일출처 정책 밖이라 origin 이 달라도 붙는다 — ★그래서 서버가 핸드셰이크의 `Origin` 을 따로 본다(DEV-TRN-WS-ORIGIN) |

<a id="COND-TRN-CORS-HEADER-AUTH"></a>**COND-TRN-CORS-HEADER-AUTH** · 성립 조건

★**성립 조건** — 이 절은 인증이 헤더 기반인 동안 참이다. ★**쿠키 인증을 도입하면 이 줄이 먼저 깨진다**(그때는 `*` 를 쓸 수 없고 자격증명 허용을 따로 정해야 한다). ★WS 도 같다 — 인증이 `BIND` 안의 토큰 · `session_id` 라 남의 페이지는 그것 없이 세션을 못 얻는다(브라우저가 알아서 싣는 자격이 없다). 그래서 WS 의 `Origin` 검사는 ★**보조 방어**다 — 쿠키 인증을 들이면 그것이 주 방어가 된다(DEV-TRN-WS-ORIGIN).

<a id="RULE-TRN-NO-OTHER-PATHS"></a>**RULE-TRN-NO-OTHER-PATHS** · 규칙

★**여기 없는 경로는 클라가 부르지 않는다.** `/admin/*` · `/healthz/*` · `/telemetry/*` 는 운영이고,
★**중재 경로는 없다** — 서버는 중재를 모르고 권한 비트만 든다(§4-2-6). 비트를 바꾸는 것은 3rd 의 연동 경로이고 §1-2-1 대로 이 문서 밖이다 — 클라가 부를 자리가 아니다.

<a id="RULE-TRN-NO-TOKEN-REFRESH"></a>**RULE-TRN-NO-TOKEN-REFRESH** · 규칙

★**토큰 갱신 경로는 없다.** 접속 중에는 토큰을 보지 않는다(§3-1).

<a id="RULE-TRN-ONE-ISSUE-PATH"></a>**RULE-TRN-ONE-ISSUE-PATH** · 규칙

★**발급 경로도 이것 하나다** — ★**운영 토큰은 발급하지 않는다**(운영자가 자기 비밀키로 직접 서명한다). 클라가 볼 일이 없는 물건이다.

<a id="DEV-TRN-WS-ORIGIN"></a>**DEV-TRN-WS-ORIGIN** · 자체 확장

서버는 WS 핸드셰이크의 `Origin` 을 본다 — ★`Origin` 이 없으면(네이티브 클라) 보지 않는다 · 있으면 `{base}` 와 같은 origin 이거나 `allowed_origins` 안이어야 하고, 아니면 핸드셰이크를 거절한다(소켓이 열리지 않는다). 빈 목록이면 같은 origin 만 받는다(HTTP CORS 의 *"빈 목록 = 같은 origin 배포"* 와 같다). ★보조 방어다 — 인증은 `BIND` 의 토큰 · `session_id` 라 이 검사가 없어도 남의 페이지가 세션을 못 얻는다

*원문: RULE-TRN-CORS — WS 는 대상이 아니라고만 한다 · 근거: COND-TRN-CORS-HEADER-AUTH CAND-TRN-WS-ORIGIN*

<a id="DEV-TRN-SESSION-SECRET"></a>**DEV-TRN-SESSION-SECRET** · 자체 확장

`session_id` 를 비밀로 다룬다는 것은 둘이다 — ①서버는 예측할 수 없는 값으로 발급한다 ②서버 · 클라 모두 `Authorization` · `X-OxLens-Session` · `session_id` 를 로그 · 진단 보고에 남기지 않는다. 지켜지는지는 구현 시험 몫이다

*원문: RULE-TRN-NO-NEW-SECRET — 비밀 취급이라고만 한다 · 근거: RULE-TRN-NO-NEW-SECRET CAND-TRN-SESSION-SECRET*

#### 2-2-4. WS 메시지 — **전량 20**

<a id="RULE-TRN-OP-NUMBERS"></a>**RULE-TRN-OP-NUMBERS** · 규칙 — WS 메시지 — **전량 20**

대역은 사람이 읽기 좋으라고 나눈 것이고 ★**코드가 번호에서 무엇도 파생하지 않는다.**
★**번호 재사용 금지(문서 머리)는 이 표 안의 규율이다.** 다른 세대의 wire 와 한 소켓에 공존하지 않는다 — 세대는 `BIND` 의 `client_ver`/`server_ver` 가 가른다(§3-1, 불일치는 `1004`).

<a id="RULE-TRN-S2C-NAMING"></a>**RULE-TRN-S2C-NAMING** · 규칙 — ★S→C 이름 규율 — `{엔티티}_{EVENT|STATE}`

★**S→C 통지의 이름은 예외 없이 이 꼴이다.** 접미사가 ★**클라 처리와 1:1** 이라,
이름만 보고 *"재협상이 도나"* 를 안다.

| 접미사 | 뜻 | 클라가 하는 일 |
|---|---|---|
| ★**`_EVENT`** | ★**존재·관계**가 바뀌었다 | ★**구조를 고친다** — 재협상 / 보관본 교체 |
| ★**`_STATE`** | ★**속성**이 바뀌었다 | ★**표시만 고친다** |

| 엔티티 \ 처리 | 구조를 고친다 | 표시만 고친다 |
|---|---|---|
| **PARTICIPANT** | `PARTICIPANT_EVENT` (joined/left) | ★`PARTICIPANT_STATE` (permission) |
| **TRACK** | `TRACK_EVENT` (add/remove) | `TRACK_STATE` (muted) |
| ★**ROOM** | ★`ROOM_EVENT` (affiliation/sync_required) | — |
| ★**LINK** | ★`LINK_EVENT` (suspect) — 서버 하나의 연결 | — |
| ★**MLINE** | ★`MLINE_EVENT` (배정표 — §8-1-2) — 서버 하나의 연결 | — |
| **SESSION** | ★`LEAVE`(0x0104, §3-1) — ★**이 규율 밖의 이름**(아래) | — |

<a id="RULE-TRN-SESSION-AXIS-LEAVE"></a>**RULE-TRN-SESSION-AXIS-LEAVE** · 규칙

★**`SESSION` 축의 op 은 `LEAVE` 하나다.** 존재·관계·속성의 변화가 아니라 ★**끝**이고 양방향이라
`{엔티티}_{EVENT|STATE}` 꼴에 억지로 맞추지 않는다(`SESSION_EVENT` 로 하면 규율엔 맞고 뜻은 흐려진다).
★*"세션 통지는 WS Close 가 담당하므로 op 이 없다"* 로 두지 않는다 — 종료 사유를 전송층에 눌러 붙이는 자리다(§3-1 `LEAVE`).

<a id="RULE-TRN-OUTSIDE-NAMING"></a>**RULE-TRN-OUTSIDE-NAMING** · 규칙

★**`LEAVE`(0x0104) · `MESSAGE`(0x0501) · `TASK`(0x0601) 는 이 규율 밖이다** — 종료·데이터·진단 계열이라
★**방 상태를 나르지 않는다.** §1-2-1 의 *"핵심/확장"* 가르기와 같은 층위 구분이다.

<a id="COND-TRN-NAMING"></a>**COND-TRN-NAMING** · 성립 조건

★**성립 조건** — 이 규율은 ★**"클라가 받아서 무엇을 하나"가 분류 축인 동안만** 참이다.
발원(클라 포워딩이냐 서버 자체 감지냐)으로 가르려는 시도는 ★**하지 않는다** —
`PARTICIPANT_EVENT{left}` 와 `TRACK_EVENT{remove}` 가 **두 발원을 다 타므로**(남의 요청, 그리고
서버의 정리) 이름에 쓰면 ★**거짓 신호**가 된다. 클라는 어느 쪽이든 똑같이 처리한다.

<a id="RULE-TRN-NOTIFY-ACK"></a>**RULE-TRN-NOTIFY-ACK** · 규칙

★**S→C 통지의 클라 공통 의무는 ACK 이다**(= body 가 빈 `flags=01` 응답, §2-1-2).
안 보내면 서버가 30초 뒤 끊는다. ★**통지에 대해 돌아가는 것은 이 ACK 하나뿐이고, 그 밖에
보낼 것은 없다.** ★서버의 `LEAVE` 는 통지가 아니라 끝냄이다 — ACK 하지 않는다(§3-1-4).

#### 2-2-5. 서버가 보낸 통지를 받았다

<a id="RULE-TRN-NOTIFY-ACK-STEPS"></a>**RULE-TRN-NOTIFY-ACK-STEPS** · 규칙 — 서버가 보낸 통지를 받았다

1. ★**먼저 ACK 을 보낸다** — `op`·`pid` 를 받은 그대로, `flags=01`, **빈 body**.
   ★ACK 은 별개 메시지가 아니라 **body 가 빈 성공 응답**이다(§2-1-2).
2. 그 다음에 내용을 처리한다.
3. ★**처리 결과를 따로 보내지 않는다.** 통지에 대해 돌아가는 것은 이 ACK 하나뿐이다.
4. 상태는 통지마다 다르다(§4-2-3).

#### 2-2-6. 클라가 정하지 않는 것

<a id="RULE-TRN-SERVER-POLICY"></a>**RULE-TRN-SERVER-POLICY** · 규칙

| 무엇 | 어디서 오나 |
|---|---|
| ★서버 쪽 송신 윈도우 | 서버가 정한다 — 범위 1~10 은 §2-2-1 |

### 2-3. 근거 · 기각 — 규격이 아니다

> 계약 본문(구조 · 동작 · 주제)의 규칙이 왜 그런가(R) · 무엇을 하지 않기로 했나(REJ). 소절 머리는 그 근거가 붙은 계약 자리다. 계약 판에는 싣지 않는다.

#### 2-3-1. §2-1-2 WS 프레임

<a id="REJ-TRN-MISREADINGS"></a>**REJ-TRN-MISREADINGS** · 기각

| 흔한 오해 | 실제 |
|---|---|
| *"`ERROR` 라는 op 이 따로 있다"* | ★**없다.** 실패도 응답이라 **같은 `op`·`pid` 에 `flags=10`** 으로 온다(§11) |
| *"`ACK` 은 응답과 다른 물건이다"* | ★**같은 물건이다.** **`ACK` = body 가 빈 `01` 응답**을 부르는 말이다. 통지(§4-2-6)에 이것을 보낸다 |
| *"`TASK` 의 `report` 는 세 번째 종류다"* | ★**아니다.** `report` 는 **새 `00` 프레임**이고 자기 `pid` 를 가지며, 그것에도 `01` 응답이 온다(§13-1-3) |
| *"연결 종료 사유는 WS Close 가 나른다"* | ★**아니다.** 사유는 ★**`LEAVE` 프레임**(§3-1)이 나른다 — 프레임 두 갈래 밖의 제3의 것이 아니라 **op 하나**다. WS Close 는 전송층의 것이라 코드를 두지 않는다(정상 종료 `1000`, 그 밖은 아무 코드) — 전송을 바꿔도 사유 체계가 남는다 |

#### 2-3-2. §2-2-1 흐름 제어

<a id="R-TRN-SMALL-WINDOW"></a>**R-TRN-SMALL-WINDOW** · 근거

★**작게 두는 이유** — 요청 순서가 곧 상태다. `AFFILIATION` 처럼 순서가 결과를 바꾸는 요청이
뒤섞이면 양쪽 상태가 갈린다. 무전 제어는 초당 몇 건이라 **처리량보다 순서가 값지다.**
★**1 이면 순서가 전송 계층에서 저절로 지켜진다** — 그래서 기본이 1 이다.

#### 2-3-3. §2-2-2 접속점

<a id="R-TRN-WS-AUTH-IN-BIND"></a>**R-TRN-WS-AUTH-IN-BIND** · 근거

★**왜 업그레이드에서 인증하지 않는가** — 업그레이드 시점에 토큰을 검사하려면 **질의값에 실어야**
하고, 질의값은 ★**액세스 로그·프록시·리퍼러에 그대로 남는다.** 그래서 토큰은 `BIND` 의 **body** 로
보낸다. 그 대신 서버는 ★**`BIND` 전의 다른 op 을 받지 않고**, ★**10초** 안에 `BIND` 가 안 오면 끊는다(`LEAVE` `4008 HEARTBEAT_TIMEOUT`, §3-1).

<a id="R-TRN-WS-AUTH-SYMPTOMS"></a>**R-TRN-WS-AUTH-SYMPTOMS** · 근거

★**증상**
- 토큰을 질의값에 실으면 → **액세스 로그에 남는다.**
- 업그레이드가 열렸다고 인증된 것으로 다루면 → ★**아무나 소켓을 열어 둘 수 있다**(op 은 못 보내지만).

#### 2-3-4. §2-2-3 HTTP 공통

<a id="R-TRN-SESSION-HEADER-WHY"></a>**R-TRN-SESSION-HEADER-WHY** · 근거

★★**왜 세션 헤더가 필요한가** — §3-1 이 *"토큰은 접속 중에 만료되지 않는다"* 라고 한 것은
★**WS 세션 얘기**다. HTTP 는 그 밖이라 **매 요청 검사**하므로, 12시간 켜둔 클라의 재동기는
`401` 이다. ★**재동기 수단이 통째로 죽는다.**

<a id="R-TRN-SESSION-HEADER-SYMPTOM"></a>**R-TRN-SESSION-HEADER-SYMPTOM** · 근거

★**증상** — 세션 헤더가 없으면 → 종일 켜둔 클라가 `sync_required`(§4-2-6)를 받고도
★**`401` 이라 아무것도 못 한다.** 멎은 채로 굳는다.

<a id="R-TRN-CORS-SYMPTOMS"></a>**R-TRN-CORS-SYMPTOMS** · 근거

★**증상**
- CORS 를 안 두면 → 다른 origin 의 앱에서 ★**미리보기·방 목록·재동기가 통째로 죽는다.** 미디어와 발언권은 멀쩡해서 원인이 안 보인다.
- ★**재동기가 죽으면 `seq` 갭에서 복구 수단이 없다**(§4-1-14 ②③). 그 방 이벤트가 멈춘 채로 굳는다.
- ★**웹 SDK 는 고객 앱에 들어가는 물건**이라 *"같은 origin 이면 된다"* 로 끝나지 않는다 — 고객 도메인 ≠ 서버 도메인이 기본이다.
- 네이티브(안드로이드·iOS)는 CORS 가 없어 ★**이 구멍을 원리적으로 못 본다.** 브라우저를 붙여야 드러난다.
- ★WS `Origin` 이 목록 밖이면 → 핸드셰이크가 거절돼 ★**소켓이 안 열리고 백오프만 돈다**(§3-2-3) — HTTP 는 되는데 WS 만 안 붙으면 `allowed_origins` 를 먼저 본다.

#### 2-3-5. §2-2-5 서버가 보낸 통지를 받았다

<a id="R-TRN-NOTIFY-ACK-SYMPTOM"></a>**R-TRN-NOTIFY-ACK-SYMPTOM** · 근거

★**증상** — ACK 을 미루면 **다음 프레임이 못 나간다**(윈도우가 좁다). 30초 넘으면 끊긴다.

<a id="SES"></a>
