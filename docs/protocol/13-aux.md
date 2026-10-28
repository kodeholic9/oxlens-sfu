[← 차례](README.md)

## 13. AUX — 부가 통신

문자(MESSAGE) · 진단(TASK) — 방 상태를 나르지 않는다

### 13-1. 구조

<a id="AUX.type.ProbeResult"></a>
#### 13-1-1. 형 `ProbeResult` — `type:"probe"` 의 `result`

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `error` | string? | — | ★**수집 자체가 실패했을 때 이것만** 보낸다. 정상이면 없다 |
| `pub_tracks[]` | — | — | 내가 **보내는** 트랙마다 — `track_id` · `ssrc[]`(시뮬캐스트면 여러 개) · `kind` · `codec` · `pt` · `packets_sent` · `bytes_sent` · `rid` · `active` |
| `sub_tracks[]` | — | — | 내가 **받는** 트랙마다 — `track_id` · `ssrc` · `kind` · `codec` · `pt` · `mid` · `packets_received` · `packets_lost` · `jitter` · `frames_decoded` · `freeze_count` · ★`spatial`/`temporal`(내가 요청한 상한) · ★`paused` |
| `env` | — | — | 브라우저·OS·SDK 판 |
| `devices[]` | — | — | 마이크·카메라·스피커 목록(`kind` · `label` · `device_id`) |
| `permissions` | — | — | 마이크·카메라 권한 상태 |
| `state` | — | — | 연결 상태(§3-4-1) · 방 상태(§4-4-1) · `pc_mode` · 발언권 상태(§10-4-1) |
| `network` | — | — | ICE 상태 · 후보 쌍 · RTT · 추정 대역 |

<a id="RULE-AUX-PROBE-NO-INVENT"></a>**RULE-AUX-PROBE-NO-INVENT** · 규칙

★**모르는 값은 넣지 않는다.** `null` 로 채우면 *"쟀는데 0"* 과 *"못 쟀다"* 가 구분되지 않는다.
★**이 보고는 진단용이고 판단 근거가 되므로, 없는 것을 지어내면 잘못된 쪽으로 수렴한다.**

<a id="RULE-AUX-TASK-PROC"></a>**RULE-AUX-TASK-PROC** · 규칙

**절차**
- 받은 쪽 ★**받자마자 ACK 부터** 하고, 일은 그 다음에 해서 `phase:"report"` 로 보낸다.
- ★**ACK 과 보고는 다른 것이다.** ACK 은 *"프레임을 받았다"* 일 뿐 작업 결과가 아니다.
- ★**`report` 는 응답이 아니다.** `flags=00` **새 프레임**이고 **자기 `pid`** 를 가진다.
  그것에도 받은 쪽이 `01` 응답을 준다 — *"요청 하나에 응답 하나"* 규칙은 여기서도 같다(§2-1-2).
- `req_id` 는 ★**시킨 쪽이 만든다.** `u32` 이고 ★**자기 안에서 하나씩 올린다**(넘치면 `0` 으로 감긴다).
  ★**발급자가 하나뿐이라 충돌이 성립하지 않는다** — 받은 쪽은 **만들지 않고 그대로 되돌린다.**
- ★**`pid` 로 잇지 못하는 이유** — `report` 는 응답이 아니라 **새 프레임**이라 자기 `pid` 를 갖는다
  (§2-1-2). 그래서 시킨 것과 보고를 잇는 것은 ★**`req_id` 뿐**이다.
- ★**모르는 `type` 은 조용히 버리지 않는다** — 실패 응답으로 답한다. ★코드는 `1002`(모르는 `type` = body 가 형식에 안 맞는다) · `1003`(`req_id`·`type` 없음).
  버리면 시킨 쪽이 타임아웃까지 기다린다.
- 기다리는 시간은 **서버 정책**이다. 클라가 정하지 않는다.

<a id="AUX.op.MESSAGE"></a>
#### 13-1-2. `0x0501 MESSAGE` — 문자   **양방향** · 응답 있음
단 2 · 출처 §13-1-2

**요청**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ |  |
| `content` | str | ✔ |  |

**응답**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `msg_id` | str | ✔ | uuid |

**통지 · 받는 꼴**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ |  |
| `user_id` | str | ✔ | `user_id` = 보낸 사람 |
| `content` | str | ✔ |  |

**실패** — `3002` 방에 없음 · `1002` · `1003` `room_id`·`content` 없음

<a id="RULE-AUX-MESSAGE-FORM"></a>**RULE-AUX-MESSAGE-FORM** · 규칙

**형식**
- 보낼 때 `{ room_id, content }` → 응답 `{ msg_id }` (uuid)
- 받을 때 `{ room_id, user_id, content }` — `user_id` = 보낸 사람. ★**보낸 사람에게는 에코하지 않는다** — 자기 것은 응답으로 안다(아래)

→ 구조 — [`AUX.op.MESSAGE`](#AUX.op.MESSAGE)

<a id="RULE-AUX-MESSAGE-PROC"></a>**RULE-AUX-MESSAGE-PROC** · 규칙

**절차** — 같은 번호가 양방향이다. **보낸 사람이 자기 것을 알아보는 것은 `pid`** 다.

<a id="RULE-AUX-MESSAGE-FAIL"></a>**RULE-AUX-MESSAGE-FAIL** · 규칙

→ 구조 — [`AUX.op.MESSAGE`](#AUX.op.MESSAGE)

<a id="AUX.op.TASK"></a>
#### 13-1-3. `0x0601 TASK` — 시켜서 하고 보고한다   **양방향** · 응답 있음
단 3 · 출처 §13-1-3

**요청**

`phase` = `request`

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `phase` | "request" | ✔ |  |
| `req_id` | u32 | ✔ |  |
| `type` | "probe" | ✔ |  |
| `params` | json | ✔ |  |

`phase` = `report`

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `phase` | "report" | ✔ |  |
| `req_id` | u32 | ✔ |  |
| `type` | "probe" | ✔ |  |
| `result` | ProbeResult | ✔ |  |

**응답**
— 빈 body

**실패** — `1002` 모르는 `type` = body 가 형식에 안 맞는다 · `1003` `req_id`·`type` 없음

<a id="EX-AUX-TASK-FORM"></a>**EX-AUX-TASK-FORM** · 예시(비규범)

→ 구조 — [`AUX.example.EX-AUX-TASK-FORM`](#EX-AUX-TASK-FORM) · [`AUX.op.TASK`](#AUX.op.TASK)

예시 — `TASK` 요청(형과 대조된 값 · 주석은 칸 설명)

```jsonc
// 시키는 쪽 →
{"phase":"request","req_id":7,"type":"probe","params":{}}
// 받은 쪽 →
{"phase":"report","req_id":7,"type":"probe","result":{}}
```

<a id="RULE-AUX-TASK-TYPES"></a>**RULE-AUX-TASK-TYPES** · 규칙

| `type` | 무엇 | 누가 시키나 |
|---|---|---|
| `"probe"` | 지금 상태를 모아 보고 | 서버(운영자 진입) |

→ 구조 — [`AUX.op.TASK`](#AUX.op.TASK)

<a id="RULE-AUX-PROBE-PARAMS"></a>**RULE-AUX-PROBE-PARAMS** · 규칙

**`type:"probe"` 의 `params`** — 지금은 ★**빈 `{}`** 다. 무엇을 모을지는 클라가 정한다.

→ 구조 — [`AUX.op.TASK`](#AUX.op.TASK)

<a id="RULE-AUX-PROBE-RESULT"></a>**RULE-AUX-PROBE-RESULT** · 규칙

**`type:"probe"` 의 `result`** — ★**아래가 전부이고, 모을 수 있는 것만 넣는다**(못 넣는 칸은 뺀다).

→ 구조 — [`AUX.type.ProbeResult`](#AUX.type.ProbeResult)

### 13-2. 주제

#### 13-2-1. 클라가 정하지 않는 것

<a id="RULE-AUX-SERVER-POLICY"></a>**RULE-AUX-SERVER-POLICY** · 규칙

| 무엇 | 어디서 오나 |
|---|---|
| `TASK` 를 기다리는 시간 | 서버가 잰다 |

### 13-3. 근거 · 기각 — 규격이 아니다

> 계약 본문(구조 · 동작 · 주제)의 규칙이 왜 그런가(R) · 무엇을 하지 않기로 했나(REJ). 소절 머리는 그 근거가 붙은 계약 자리다. 계약 판에는 싣지 않는다.

#### 13-3-1. §13-1-1 형 `ProbeResult` — `type:"probe"` 의 `result`

<a id="R-AUX-TASK-SYMPTOM"></a>**R-AUX-TASK-SYMPTOM** · 근거

★**증상** — ACK 을 미루면 **다음 프레임이 못 나간다**(윈도우가 좁다).

#### 13-3-2. §13-1-2 `0x0501 MESSAGE` — 문자   **양방향** · 응답 있음

<a id="R-AUX-MESSAGE-SYMPTOM"></a>**R-AUX-MESSAGE-SYMPTOM** · 근거

★**증상** — `room_id` 를 빼면 `1003`(필수 칸 — 방을 따지기 전에 형을 본다) · 들어가지 않은 방이면 `3002`.

