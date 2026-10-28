[← 차례](README.md)

## 5. AFF — 소속

어느 방을 구독하고(sub_rooms) 어디로 발행하나(pub_room) — 방 입도의 pub/sub

| 정체성 | 누가 내나 | 무엇마다 | 수명 | 상태 |
|---|---|---|---|---|
| 세션 | server | 세션 하나 | 첫 입장 → 세션 끝 | — |

| 의존 | 기댐 | 다중도 | 뜻 |
|---|---|---|---|
| AFF | SES | 1:1 | 소속은 세션 하나에 하나 |
| AFF | ROOM | 1:N | sub_rooms = 입장한 방들 · pub_room 은 그중 하나(§5-1-1) |
| PUB | AFF | N:1 | 발행 트랙은 발행 방으로 — 발행 방 전환이 트랙에 하는 일(§5-1-2) |
| FLR | AFF | N:1 | 발언 요청은 발행 방에서만(§10-4-32) |

### 5-1. 구조

<a id="AFF.type.Affiliation"></a>
#### 5-1-1. 형 `Affiliation` — 어느 방을 구독하고 어디로 발행하나

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `sub_rooms` | str[] | ✔ | 내가 듣는 방들 = ★그 서버에 입장한 방들 (입장이 곧 구독, §4-1) |
| `pub_room` | str \| null | ✔ | ★내가 발행하는 방. **하나뿐**이라 단수다. 없으면 null |

<a id="RULE-AFF-S2C-ONLY"></a>**RULE-AFF-S2C-ONLY** · 규칙 — `Affiliation` — 어느 방을 구독하고 어디로 발행하나

★**서버가 클라에게 주는 것이다**(S→C 전용). ★**요청 형상이 아니다.**

→ 구조 — [`AFF.type.Affiliation`](#AFF.type.Affiliation)

<a id="RULE-AFF-WIRE-WORDS"></a>**RULE-AFF-WIRE-WORDS** · 규칙

★**필드는 `sub_rooms`/`pub_room` 이고, 글로 풀 때는 *"듣는다/말한다"* 로 읽는다.** 뜻은 같다 —
★**wire 어휘와 사람이 읽는 말을 나눈 것**이다(§1-2-1). 필드 이름을 `listen`/`talk` 로 바꾸지 않는다.

<a id="EX-AFF-SHAPE"></a>**EX-AFF-SHAPE** · 예시(비규범)

→ 구조 — [`AFF.example.EX-AFF-SHAPE`](#EX-AFF-SHAPE) · [`AFF.type.Affiliation`](#AFF.type.Affiliation)

예시 — 형 `Affiliation`(형과 대조된 값 · 주석은 칸 설명)

```jsonc
{ "sub_rooms": [ "r1", "r2" ],   // 내가 듣는 방들 = ★그 서버에 입장한 방들 (입장이 곧 구독, §4-1)
  "pub_room": "r1" }             // ★내가 발행하는 방. **하나뿐**이라 단수다. 없으면 null
```

<a id="RULE-AFF-CARRIERS"></a>**RULE-AFF-CARRIERS** · 규칙

| 어디에 실리나 | |
|---|---|
| `ROOM_JOIN` 응답 (§4-1) | 입장이 바꾼 결과 — ★`cause` 필드는 없다. §5-1-2 가 이 변경을 *implicit*(입장이 곁들여 바꿈)이라 부르지만 wire 값이 아니다 |
| `ROOM_LEAVE` 응답 (§4-1) | 퇴장이 바꾼 결과 — 같다. `cause` 필드도 `version` 도 없다(종결 — 재설정할 스냅샷이 없다, §4-2-3) |
| `AFFILIATION` 응답 (§5-1-2) | ★여기엔 `cause`·`change_id` 가 **더 붙는다** |
| ★`ROOM_EVENT{affiliation}` (§4-2-6) | ★**서버가 강제로 바꿨을 때.** `cause` 가 **더 붙는다** |

→ 구조 — [`AFF.type.Affiliation`](#AFF.type.Affiliation) · [`ROOM.op.ROOM_JOIN`](04-room.md#ROOM.op.ROOM_JOIN) · [`AFF.op.AFFILIATION`](#AFF.op.AFFILIATION) · [`ROOM.op.ROOM_EVENT`](04-room.md#ROOM.op.ROOM_EVENT)

<a id="RULE-AFF-NO-VERSION"></a>**RULE-AFF-NO-VERSION** · 규칙

★★**소속만 나르는 프레임에는 `version` 이 없다**(`AFFILIATION` 응답 · `ROOM_EVENT{affiliation}` · `ROOM_LEAVE` 응답 — §4-2-3. `ROOM_JOIN` 응답은 스냅샷 갈래라 싣는다). ★`pub_room` 은 ★**내 세션 것**이라 방 공통
스냅샷에 들어 있지 않고(§4-2-6), 같은 payload 를 나르는 broadcast 도 없어 ★**되감길 대상이 없다.**
신선도는 전송 계층이 지킨다 — 윈도우 기본 1 · 받은 순서대로 처리(§2-2-1) · 출처는 나 하나.

→ 구조 — [`AFF.op.AFFILIATION`](#AFF.op.AFFILIATION) · [`ROOM.op.ROOM_EVENT`](04-room.md#ROOM.op.ROOM_EVENT)

<a id="RULE-AFF-TWO-AXES"></a>**RULE-AFF-TWO-AXES** · 규칙

★★**두 축은 바꾸는 수단이 다르다.** ★**`sub_rooms` 는 `ROOM_JOIN`/`ROOM_LEAVE` 가 바꾼다** —
입장이 곧 구독이라(§4-4-1) 소속만 따로 바꾸는 sub 요청은 **없다**(`sub_add`/`sub_remove` 는
기각 — §5-1-2). ★**`pub_room` 만 `AFFILIATION`**(`pub_select`·`pub_deselect`)이 바꾼다.
요청은 증감이고 응답은 ★**적용 후 전체 목록**이다(§5-1-2).

<a id="RULE-AFF-ONE-PUB-ROOM"></a>**RULE-AFF-ONE-PUB-ROOM** · 규칙

★**`pub_room` 은 하나다** — 이름이 단수인 것이 곧 계약이다. 여러 방에서 동시에 말할 수 없다.

→ 구조 — [`AFF.C.NOPUB.select`](#AFF.C.NOPUB.select) · [`AFF.C.PUB.switch_same`](#AFF.C.PUB.switch_same) · [`AFF.C.PUB.switch_cross`](#AFF.C.PUB.switch_cross)

<a id="RULE-AFF-PARTIAL"></a>**RULE-AFF-PARTIAL** · 규칙

★★**이 값은 그 미디어 서버가 아는 만큼이다.** ★**전체가 아니다** — 방이 여러 서버에 걸쳐 있으면
서버마다 자기가 아는 목록만 돌려준다. ★**전체로 덮어쓰면 다른 서버의 듣는 방이 사라진다**(§5-2-2).

<a id="AFF.op.AFFILIATION"></a>
#### 5-1-2. `0x0401 AFFILIATION` — 어디로 발행할까   **C→S** · 응답 있음
단 1 · 출처 §5-1-2

**요청**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `pub_select` | string? | — | 이 방으로 발행한다(= 이 방에서 말한다) |
| `pub_deselect` | string? | — | 이 방으로 그만 발행한다 |
| `change_id` | string? | — | 내가 붙인 번호. 응답에 되돌아온다 |

**응답**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `sub_rooms` | str[] | ✔ | ★그 서버에 입장한 방들의 스냅샷, §5-1-1 |
| `pub_room` | str \| null | ✔ |  |
| `cause` | "user" | ✔ |  |
| `change_id` | str | — |  |

**실패** — `2001` 아직 BIND 안 함 · `1003` `pub_select` · `pub_deselect` 가 둘 다 없음 — 물을 서버가 없다(RULE-AFF-RES-FORM) · `1006` `pub_select` 대상 방의 미디어 코덱 불일치(내 트랙 하나라도 — 전환 전체 실패) · `2006` `pub_select` 대상 방의 권한 비트가 내 발행 트랙 종류를 막는다(전환 전체 실패) · `5003` 내부 오류

<a id="RULE-AFF-PUB-ONLY"></a>**RULE-AFF-PUB-ONLY** · 규칙

★**발언 방 전환 전용이다.** sub 축은 이 op 에 없다 — ★**입장이 곧 구독**이라(§4-4-1·§5-1-1)
듣는 방의 증감은 `ROOM_JOIN`/`ROOM_LEAVE` 가 한다.

→ 구조 — [`AFF.op.AFFILIATION`](#AFF.op.AFFILIATION)

<a id="RULE-AFF-RES-FORM"></a>**RULE-AFF-RES-FORM** · 규칙

응답 `{ sub_rooms: ["r1","r2"], pub_room: "r1" | null, cause, change_id? }`
(`sub_rooms` = ★그 서버에 입장한 방들의 스냅샷, §5-1-1)
★`pub_select` · `pub_deselect` 가 ★**둘 다 없는 요청은 `1003`** 다 — 응답은 ★그 서버의 스냅샷인데 대상 방이 없으면 그 서버가 정해지지 않는다. 지어낸 빈 스냅샷으로 답하면 응답만 반영하는 클라(§5-2-2)가 소속을 잃는다.

→ 구조 — [`AFF.op.AFFILIATION`](#AFF.op.AFFILIATION)

<a id="RULE-AFF-RES-NO-VERSION"></a>**RULE-AFF-RES-NO-VERSION** · 규칙

★★**`version` 을 싣지 않는다 — `versions{}` 필드는 폐기했다.** 근거는 §4-2-3 이다:
★**`sub_rooms`·`pub_room` 을 나르는 broadcast 가 없어 되감길 대상이 없다.** 소속은 §5-1-1 이 정한 대로
★**내 세션 것**이고 방 공통 스냅샷에 들어 있지 않으므로, 이 응답이 `seq` 를 올리지도 싣지도 않는다.

→ 구조 — [`AFF.op.AFFILIATION`](#AFF.op.AFFILIATION)

<a id="RULE-AFF-TWO-ROOMS-ONE-REQ"></a>**RULE-AFF-TWO-ROOMS-ONE-REQ** · 규칙

★**한 요청이 두 방을 바꿀 수 있다**(`pub_deselect`(r1) + `pub_select`(r2)) — 두 방 요청을 금지하는 대안은
★같은 서버 안 전환의 원자성(해제 후 선택을 한 요청에)을 잃어 기각했다.

→ 구조 — [`AFF.C.PUB.switch_same`](#AFF.C.PUB.switch_same)

<a id="RULE-AFF-CAUSE-USER"></a>**RULE-AFF-CAUSE-USER** · 규칙

★★**응답의 `cause` 는 `user` 하나다** — 내가 이 요청으로 바꿨다. 입·퇴장이 곁들여 바꾸는 것은(§4-1 — 이 문서가
*implicit* 이라 부르는 변경) `ROOM_JOIN`/`ROOM_LEAVE` 응답의 `affiliation` 으로 오고 ★**그 응답엔 `cause` 필드가 없다**(§5-1-1).

→ 구조 — [`AFF.op.AFFILIATION`](#AFF.op.AFFILIATION)

<a id="RULE-AFF-NO-FORCED-IN-RES"></a>**RULE-AFF-NO-FORCED-IN-RES** · 규칙

★**서버가 강제로 바꾼 것은 여기 오지 않는다.** ★**응답은 내가 보낸 요청에만 온다**는 것이
그 이유다 — 강퇴당했는데 알릴 프레임이 응답일 수는 없다. `kick` · `room_closed` 는
★**`ROOM_EVENT{type:"affiliation"}`(0x0704, §4-2-6)** 이 나른다.

<a id="RULE-AFF-PROC"></a>**RULE-AFF-PROC** · 규칙

**절차**
- 서버 적용 순서는 ★**`pub_deselect` → `pub_select`** 다 — 같은 요청에 둘 다 오면
  **해제 후 선택**이라 같은 서버 안 전환이 안전하다.
- 서버 ★★**`pub_select` 단독은 옛 `pub_room` 을 묵시로 내린다** — `pub_room` 이 단수(§5-1-1)라 그렇다. 그것은
  옛 방에 `pub_deselect` 가 온 것과 같고, 따라서 옛 방에서 발언권을 쥐고 있었으면 ★**그 방 `RELEASE` 와 같다**
  (§10-4-32 "쥔 채로 방을 옮기지 않는다"). 클라는 옛 방의 발언권 상태를 `has no permission` 으로 내린다.
- 클라 ★★**요청 하나는 미디어 서버 하나 것이다.** 서버를 옮겨 말하려면
  ★**옛 서버에 `pub_deselect` + 새 서버에 `pub_select` 두 번**이다. 필드가 방 하나짜리라
  ★**요청이 어느 서버 것인지는 그 방으로 정해진다** — 서버를 가리키는 별도 필드가 필요 없다.
- 클라 ★**말하려는 방에 먼저 입장해 있어야** 한다(§4-4-1). 아니면 그 항목만 건너뛴다.
- 서버 ★★**발행 방이 바뀌면 그 세션의 발행 트랙이 전부 따라간다**(반이중 · 전이중 모두 — 아래 전환표). ★**새 방에서 내 트랙 하나라도 받을 수 없으면 전환 전체를 거절한다** — 새 방의 미디어 코덱과 어긋나면 `1006`, 새 방의 권한 비트가 내 트랙 종류를 막으면 `2006` 이고 ★**아무것도 옮기지 않는다**(일부만 옮기면 *"내 트랙 전부 = `pub_room`"* 이 깨진다).
- 서버 ★**`pub_deselect` 만으로 발행 방이 비면 발행은 그대로 두고 RTP 는 버린다** — 트랙은 세션 것이라 다시 `pub_select` 하면 따라간다. ★방을 떠나 발행 방이 빈 것은 다르다 — 그 방의 전이중 발행은 정리된다(§7-1-2 · 방 퇴장 절차 그대로).
- 서버 ★**`pub_deselect`(또는 옛 방을 내리는 `pub_select`)는 그 방의 큐 대기도 뺀다** — 대기는 `pub_room` 에서만 걸린다(§10-4-30). 클라는 응답 뒤 그 방을 `has no permission` 으로 내린다(따로 통지는 없다).
- 서버 ★응답은 **그 서버가 아는 만큼**이다. **전체가 아니다.**
- 서버 어긋난 항목은 건너뛰고 **나머지는 적용**한다. ★**건너뛴 것은 응답의 `pub_room` 이
  안 바뀐 것으로 안다** — 항목별 실패 코드는 없다.
- 클라 ★**직접 상태를 고치지 않는다.** 응답이 온 것만 반영한다.
- ★**듣던 방을 나가면(`ROOM_LEAVE`) 발행도 함께 풀린다** — 그 방이 발행 방이었으면
  `pub_room` 이 `null` 이 된다(§4-1). ★**입장하지 않은 방으로는 말할 수 없다**는 규칙의
  역방향이다.

<a id="RULE-AFF-SINGLE-SPEAK-ROOM"></a>**RULE-AFF-SINGLE-SPEAK-ROOM** · 규칙

★**발행 방은 하나다.** 여러 방에서 동시에 말할 수 없다.

→ 구조 — [`AFF.C.PUB.switch_same`](#AFF.C.PUB.switch_same) · [`AFF.C.PUB.switch_cross`](#AFF.C.PUB.switch_cross)

<a id="RULE-AFF-PUB-IN-SUB"></a>**RULE-AFF-PUB-IN-SUB** · 규칙

★★**`pub_room` 은 언제나 `sub_rooms` 안에 있다.** 이 불변식이 깨진 응답이 오면 ★**서버 결함**이고,
클라는 그 응답을 반영하지 않는다.

<a id="RULE-AFF-SWITCH-TRACKS"></a>**RULE-AFF-SWITCH-TRACKS** · 규칙 — ★발행 방 전환이 트랙에 하는 일

★★**발행 트랙은 세션 것이고, 그 RTP 는 그 순간의 `pub_room` 으로 흐른다** — 반이중 · 전이중 모두다. 서버는 발행 RTP 를
흘릴 때마다 ★**그 순간의 `pub_room`** 으로 방을 정한다. ★**전이중은 `pub_room` 에서만 발행 · 전환된다** — `room_id` 가 `pub_room` 이 아니면
`3011` 이다(§7-1-2). 그래서 발행 방을 옮기면 ★**내가 든 트랙 전부가 새 방으로 간다**:

| 전환 | 클라가 하는 일 |
|---|---|
| ★**같은 서버 안** (r1→r3) | ★**`AFFILIATION{pub_deselect:r1, pub_select:r3}` 하나뿐** — 트랙 재발행도 발행 쪽 재협상도 없다. 반이중은 슬롯이 방 것이라 서버가 다음 패킷부터 r3 으로 흘린다. ★**전이중은 서버가 옮긴다** — r1 전원에게 `TRACK_EVENT{remove}`, r3 전원에게 `{add}`(구독자는 배정표로 받는다 — 줄이 늘 때만 재협상한다 · 두 방을 다 받는 구독자는 그 줄의 `room_id` 만 바뀐다, §8-1-2). ★쉬던 전이중(반이중으로 돌려 쉬는 항목)도 같이 옮긴다 — r3 에는 `active:false` 로 `add`. ★r1 에서 발언권을 쥐고 있었으면 **서버가 반환한다**(`pub_deselect` = 그 방 `RELEASE`, §10-4-32) — 클라는 r1 의 발언권 상태를 `has no permission` 으로 내린다. ★r1 의 큐 대기도 빠진다(§10-4-30 — 대기는 `pub_room` 에서만) |
| ★**서버를 건너** (sfu A→B) | 순서 계약 — 서버가 상태를 나누지 않으므로 ★**하나의 요청으로 묶지 않는다**(묶어도 두 서버에 걸친 원자성은 없다). ①옛 서버에 `PUBLISH_TRACKS{remove}` + `pub_deselect` 를 보내고 ★**응답을 기다린다** — ★`pub_deselect` 가 곧 그 방 발언권 반환 · 대기 철회다(§5-1-2) · `FLOOR_RELEASE` 를 따로 보내지 않는다 · 응답 뒤 그 방을 `has no permission` 으로 내린다 ②새 서버에 `pub_select` + 든 트랙 그대로 `PUBLISH_TRACKS{add}`(그 서버엔 내 업링크가 없어 재협상한다, §7-2-3) ③②가 실패하면 ★**옛 서버로 되돌린다**(같은 순서로 재발행) — 실패한 채 두면 ★**발행 축이 어디에도 없다** |

<a id="RULE-AFF-PUB-SELECT-CODEC-RECHECK"></a>**RULE-AFF-PUB-SELECT-CODEC-RECHECK** · 규칙

★★**서버는 `pub_select` 때 미디어 코덱을 재검사한다.** 발행 때(§7-1-2 `1006`)만 검사하면
H264 방에서 발행한 트랙을 `pub_select` 로 VP8 방에 끌고 가는 구멍이 생긴다 — 새 방의 미디어 코덱(방 속성 `media_codecs` — §4-1-1 · 든 트랙 전부 — 전이중 · 반이중)과
어긋나면 그 `pub_select` 를 ★**`1006`(`details.codec` 동봉)으로 거절**한다(조용한
건너뛰기가 아니다 — 클라가 코덱을 맞춰 재발행해야 풀리는 실패라 원인을 알려야 한다).

<a id="RULE-AFF-FAIL"></a>**RULE-AFF-FAIL** · 규칙

→ 구조 — [`AFF.op.AFFILIATION`](#AFF.op.AFFILIATION)

<a id="PROP-AFF-ONE-HOLDER"></a>**PROP-AFF-ONE-HOLDER** · 성질

두 서버가 동시에 그 세션의 발행 방을 갖지 않는다 — 서버를 건너는 전환 중에도(옛 서버 해제를 먼저 · RULE-AFF-ONE-PUB-ROOM · RULE-AFF-CHANGE-PROC).

*식: AH2 · 깨지면: 두 방에 동시에 말할 수 있는 상태가 된다(이중 발행)*

<a id="PROP-AFF-CLIENT-AGREES"></a>**PROP-AFF-CLIENT-AGREES** · 성질

통로가 비면 클라가 아는 발행 방이 서버들이 아는 것이다 — 응답만 반영하고(미리 고치지 않는다) · 다른 서버의 응답은 그 서버가 아는 만큼만 덮는다(RULE-AFF-PARTIAL · RULE-AFF-CHANGE-PROC). ★윈도우 1 · WS 통 하나(FIFO)에서 잰다 — 신선도를 전송 계층이 지킨다는 주장(RULE-ROOM-AFF-FRESHNESS)이 이 판의 전제다. ★소속 절차가 응답을 기다리는 동안 다음 요청을 내지 않아(대기 칸) 통 안에 요청 둘이 겹치지 않는다 — "받은 순서 처리" 는 이 판에서 갈리지 않는다(윈도우를 늘린 판은 없다).

*식: AH3 · 깨지면: 클라는 발행 방이 a 라 알고 서버는 b 로 안다 — 말하면 엉뚱한 방에 나간다*

<a id="PROP-AFF-SERVER-OWN"></a>**PROP-AFF-SERVER-OWN** · 성질

서버는 자기 방만 발행 방으로 안다 — 응답은 그 서버가 아는 만큼이다(RULE-AFF-PARTIAL).

*식: AH4 · 깨지면: 남의 서버 방을 발행 방이라 답해 클라가 그 서버 방을 덮어쓴다*

<a id="PROP-AFF-CLIENT-PUB-HELD"></a>**PROP-AFF-CLIENT-PUB-HELD** · 성질

통로가 비면 클라가 발행 방이라 아는 방은 그 서버도 발행 방으로 안다(RULE-AFF-ONE-PUB-ROOM).

*식: AH5 · 깨지면: 클라는 말할 수 있다고 알지만 서버는 그 방 발행을 거절한다*

<a id="AFF.state"></a>
#### 5-1-3. 상태 칸 · 시한

##### 상태 — 클라

| 칸 | 처음 | 뜻 |
|---|---|---|
| `pub` | `'NONE'` | 세션의 발행 방 — 방 하나 또는 없음(응답이 그렇게 말했다 — 직접 고치지 않는다 · RULE-AFF-PROC) |
| `wait` | `'NONE'` | 기다리는 응답 — 요청은 한 번에 하나(서버를 건너면 옛 서버 해제 → 응답 → 새 서버 선택 · RULE-AFF-SWITCH-TRACKS) |
| `want` | `'NONE'` | 옮겨 갈 방 — 서버를 건너는 전환에서 옛 서버 해제 응답을 기다리는 동안 |

##### 상태 — 서버

| 칸 | 처음 | 뜻 |
|---|---|---|
| `pub` | `'NONE'` | 그 서버의 방 하나 또는 없음 |

##### 전이 — 클라(동작 경우에서 짓는다)

| 지금 | 사건 | 다음 | 라벨 |
|---|---|---|---|
| `NOPUB` | 방 r 을 발행 방으로(§5-2-2 — 같은 서버면 요청 하나 · 서버를 건너면 옛 서버 해제 먼저 · RULE-AFF-SWITCH-TRACKS) | `SELECT` | `AFF.C.NOPUB.select` |
| `PUB` | 방 r 을 발행 방으로(§5-2-2 — 같은 서버면 요청 하나 · 서버를 건너면 옛 서버 해제 먼저 · RULE-AFF-SWITCH-TRACKS) | `SELECT` | `AFF.C.PUB.switch_same` |
| `PUB` | 방 r 을 발행 방으로(§5-2-2 — 같은 서버면 요청 하나 · 서버를 건너면 옛 서버 해제 먼저 · RULE-AFF-SWITCH-TRACKS) | `DESELECT` | `AFF.C.PUB.switch_cross` |
| `PUB` | 발행 방을 뺀다(§5-2-2) | `DESELECT · NONE` | `AFF.C.PUB.deselect` |
| 어느 상태 | AFFILIATION 응답 — ok = 성공 응답 · p = 응답한 서버가 아는 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub · onWant = 응답한 서버가 지금 발행 방 · 옮겨 갈 방의 서버다(§5-2-2 3 · 4) | `NONE` | `AFF.C.any.fail` |
| `PUB` | AFFILIATION 응답 — ok = 성공 응답 · p = 응답한 서버가 아는 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub · onWant = 응답한 서버가 지금 발행 방 · 옮겨 갈 방의 서버다(§5-2-2 3 · 4) | `NONE · SELECT` | `AFF.C.PUB.switch_in` |
| `PUB` | AFFILIATION 응답 — ok = 성공 응답 · p = 응답한 서버가 아는 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub · onWant = 응답한 서버가 지금 발행 방 · 옮겨 갈 방의 서버다(§5-2-2 3 · 4) | `NONE` | `AFF.C.PUB.switch_stuck` |
| `NOPUB` | AFFILIATION 응답 — ok = 성공 응답 · p = 응답한 서버가 아는 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub · onWant = 응답한 서버가 지금 발행 방 · 옮겨 갈 방의 서버다(§5-2-2 3 · 4) | `NONE` | `AFF.C.NOPUB.res_pub` |
| `PUB` | AFFILIATION 응답 — ok = 성공 응답 · p = 응답한 서버가 아는 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub · onWant = 응답한 서버가 지금 발행 방 · 옮겨 갈 방의 서버다(§5-2-2 3 · 4) | `NONE` | `AFF.C.PUB.res_moved` |
| `PUB` | AFFILIATION 응답 — ok = 성공 응답 · p = 응답한 서버가 아는 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub · onWant = 응답한 서버가 지금 발행 방 · 옮겨 갈 방의 서버다(§5-2-2 3 · 4) | `NONE` | `AFF.C.PUB.res_away` |
| 어느 상태 | AFFILIATION 응답 — ok = 성공 응답 · p = 응답한 서버가 아는 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub · onWant = 응답한 서버가 지금 발행 방 · 옮겨 갈 방의 서버다(§5-2-2 3 · 4) | `NONE` | `AFF.C.any.res_same` |
| `NOPUB` | select:true 입장을 보낸다 — 입장이 곁들여 발행 방을 바꾸므로 ★소속 전환이 떠 있는 것과 같다(RULE-ROOM-JOIN-START 2 · 그 응답이 올 때까지 다른 전환을 막는다) | `JOIN` | `AFF.C.NOPUB.join_select` |
| 어느 상태 | 입장 응답의 affiliation — 입장이 곁들여 바꾼 발행 방(RULE-AFF-CAUSE-USER · cause 없음) · r = 그 입장의 방 · sel = 그 입장 요청이 select:true 였다(응답은 pid 로 제 요청과 짝지어진다 — RULE-TRN-ONE-REPLY) · p = 그 응답이 말한 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub = 지금 발행 방이 그 응답을 낸 서버의 방 | `NONE` | `AFF.C.any.implicit_joined` |
| 어느 상태 | 입장 응답의 affiliation — 입장이 곁들여 바꾼 발행 방(RULE-AFF-CAUSE-USER · cause 없음) · r = 그 입장의 방 · sel = 그 입장 요청이 select:true 였다(응답은 pid 로 제 요청과 짝지어진다 — RULE-TRN-ONE-REPLY) · p = 그 응답이 말한 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub = 지금 발행 방이 그 응답을 낸 서버의 방 | `NONE` | `AFF.C.any.implicit_join_unselected` |
| 어느 상태 | 입장 응답의 affiliation — 입장이 곁들여 바꾼 발행 방(RULE-AFF-CAUSE-USER · cause 없음) · r = 그 입장의 방 · sel = 그 입장 요청이 select:true 였다(응답은 pid 로 제 요청과 짝지어진다 — RULE-TRN-ONE-REPLY) · p = 그 응답이 말한 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub = 지금 발행 방이 그 응답을 낸 서버의 방 | — | `AFF.C.any.implicit_busy` |
| 어느 상태 | 입장 응답의 affiliation — 입장이 곁들여 바꾼 발행 방(RULE-AFF-CAUSE-USER · cause 없음) · r = 그 입장의 방 · sel = 그 입장 요청이 select:true 였다(응답은 pid 로 제 요청과 짝지어진다 — RULE-TRN-ONE-REPLY) · p = 그 응답이 말한 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub = 지금 발행 방이 그 응답을 낸 서버의 방 | — | `AFF.C.any.implicit_pub` |
| 어느 상태 | 입장 응답의 affiliation — 입장이 곁들여 바꾼 발행 방(RULE-AFF-CAUSE-USER · cause 없음) · r = 그 입장의 방 · sel = 그 입장 요청이 select:true 였다(응답은 pid 로 제 요청과 짝지어진다 — RULE-TRN-ONE-REPLY) · p = 그 응답이 말한 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub = 지금 발행 방이 그 응답을 낸 서버의 방 | `NONE` | `AFF.C.any.implicit_away` |
| 어느 상태 | 입장 응답의 affiliation — 입장이 곁들여 바꾼 발행 방(RULE-AFF-CAUSE-USER · cause 없음) · r = 그 입장의 방 · sel = 그 입장 요청이 select:true 였다(응답은 pid 로 제 요청과 짝지어진다 — RULE-TRN-ONE-REPLY) · p = 그 응답이 말한 발행 방(그 서버의 방 하나 또는 없음 — 부분 목록) · onPub = 지금 발행 방이 그 응답을 낸 서버의 방 | — | `AFF.C.any.implicit_same` |
| 어느 상태 | 듣던 방 r 을 떠났다 — 그 방이 발행 방이면 발행도 함께 풀린다(RULE-AFF-PROC 끝 항) | `NONE` | `AFF.C.any.left` |
| 어느 상태 | 듣던 방 r 을 떠났다 — 그 방이 발행 방이면 발행도 함께 풀린다(RULE-AFF-PROC 끝 항) | `NONE` | `AFF.C.any.left_join` |
| 어느 상태 | 듣던 방 r 을 떠났다 — 그 방이 발행 방이면 발행도 함께 풀린다(RULE-AFF-PROC 끝 항) | `NONE` | `AFF.C.any.left_want` |
| 어느 상태 | 듣던 방 r 을 떠났다 — 그 방이 발행 방이면 발행도 함께 풀린다(RULE-AFF-PROC 끝 항) | — | `AFF.C.any.left_other` |
| 어느 상태 | 방 r 입장이 실패했다(응답이 실패 · 응답의 pc_mode 불일치로 되돌림) — select:true 입장이 건 기다림이면 푼다(소속 요청의 실패 응답이 AFF.C.any.fail 로 푸는 것과 같은 꼴) | `NONE` | `AFF.C.any.join_failed` |
| 어느 상태 | 방 r 입장이 실패했다(응답이 실패 · 응답의 pc_mode 불일치로 되돌림) — select:true 입장이 건 기다림이면 푼다(소속 요청의 실패 응답이 AFF.C.any.fail 로 푸는 것과 같은 꼴) | — | `AFF.C.any.join_failed_other` |

<a id="AFF.C"></a>
### 5-2. 동작

<a id="AFF.C.NOPUB"></a>
#### 5-2-1. 클라 · `NOPUB`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="AFF.C.NOPUB.select"></a>`AFF.C.NOPUB.select` | 받는 상태 wait == 'NONE' && pub != r · pub == 'NONE' | wait ← 'SELECT'; want ← r · 보낸다 AFFILIATION.req {pub_select: r} | §5-2-2 1 §5-2-2 3 — 말하려는 방에 먼저 입장해 있어야 한다(RULE-AFF-PROC) — 미리 고치지 않는다 |
| <a id="AFF.C.NOPUB.res_pub"></a>`AFF.C.NOPUB.res_pub` | p != 'NONE' && pub == 'NONE' | pub ← p; wait ← 'NONE'; want ← 'NONE' · 효과 floor.pub_select → FLR.C.any.pub_select · 효과 app.affiliation | §5-1-2 §5-2-2 3 §5-2-2 5 |
| <a id="AFF.C.NOPUB.join_select"></a>`AFF.C.NOPUB.join_select` | 받는 상태 wait == 'NONE' && pub == 'NONE' · 나머지 | wait ← 'JOIN'; want ← r | §4-2-1 2 §5-2-2 3 — 발행 방이 없을 때만 select:true 로 들어간다 — 요청은 ROOM 이 보낸다(ROOM_JOIN) · 여기는 기다림만 건다(20261023 TLC: 응답 전에 다른 서버 방을 고르면 두 서버가 발행 방을 갖던 것) |

<a id="AFF.C.PUB"></a>
#### 5-2-2. 클라 · `PUB`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="AFF.C.PUB.switch_same"></a>`AFF.C.PUB.switch_same` | 받는 상태 wait == 'NONE' && pub != r · same | wait ← 'SELECT'; want ← r · 보낸다 AFFILIATION.req {pub_deselect: pub, pub_select: r} | §5-2-2 1 §5-2-2 3 §5-1-2 — 같은 서버 안 — 요청 하나(해제 후 선택 · RULE-AFF-PROC 적용 순서 · RULE-AFF-SWITCH-TRACKS 같은 서버 행) |
| <a id="AFF.C.PUB.switch_cross"></a>`AFF.C.PUB.switch_cross` | 받는 상태 wait == 'NONE' && pub != r · 나머지 | wait ← 'DESELECT'; want ← r · 보낸다 AFFILIATION.req {pub_deselect: pub} | §5-2-2 2 §5-1-2 — 서버를 건넌다 — ★옛 서버에 해제를 먼저 보내고 응답을 기다린다(새 서버 선택은 그 뒤 · RULE-AFF-SWITCH-TRACKS 순서 계약 ② → ③ · 두 서버가 동시에 발행 방을 갖지 않는다) |
| <a id="AFF.C.PUB.deselect"></a>`AFF.C.PUB.deselect` | 받는 상태 wait == 'NONE' && pub != 'NONE' · 나머지 | wait ← 'DESELECT'; want ← 'NONE' · 보낸다 AFFILIATION.req {pub_deselect: pub} | §5-2-2 1 §5-2-2 3 |
| <a id="AFF.C.PUB.switch_in"></a>`AFF.C.PUB.switch_in` | wait == 'DESELECT' && want != 'NONE' && !onWant && p == 'NONE' | pub ← 'NONE'; wait ← 'SELECT' · 보낸다 AFFILIATION.req {pub_select: want} · 효과 floor.pub_away → FLR.C.any.pub_away · 효과 app.affiliation | §5-2-2 2 §5-1-2 — 서버를 건너는 전환 — 옛 서버가 해제를 확인했다 · 이제 새 서버에 선택을 보낸다(그 사이 발행 방은 없다) |
| <a id="AFF.C.PUB.switch_stuck"></a>`AFF.C.PUB.switch_stuck` | wait == 'DESELECT' && want != 'NONE' && !onWant | wait ← 'NONE'; want ← 'NONE' · 효과 app.aff_failed | §5-1-2 §5-2-2 3 — 옛 서버가 해제를 건너뛰었다(응답의 pub_room 이 그대로 — 건너뛴 항목은 안 바뀐 것 · RULE-AFF-PROC) — 새 서버에 선택을 보내지 않는다(두 서버가 동시에 갖게 된다) · 전환을 멈춘다 |
| <a id="AFF.C.PUB.res_moved"></a>`AFF.C.PUB.res_moved` | p != 'NONE' && pub != p | pub ← p; wait ← 'NONE'; want ← 'NONE' · 효과 floor.pub_away → FLR.C.any.pub_away · 효과 floor.pub_select → FLR.C.any.pub_select · 효과 app.affiliation | §5-1-2 §5-2-2 3 §5-2-2 5 — 같은 서버 안 전환 — 옛 방은 내려가고(그 방 RELEASE · 대기 철회와 같다 · RULE-AFF-PROC) 새 방이 발행 방이다 |
| <a id="AFF.C.PUB.res_away"></a>`AFF.C.PUB.res_away` | p == 'NONE' && onPub && pub != 'NONE' | pub ← 'NONE'; wait ← 'NONE'; want ← 'NONE' · 효과 floor.pub_away → FLR.C.any.pub_away · 효과 app.affiliation | §5-1-2 §5-2-2 3 §5-2-2 5 — pub_deselect — 그 서버의 발행 방이 없다 · 그 방 RELEASE · 대기 철회와 같다 |

<a id="RULE-AFF-CHANGE-PROC"></a>**RULE-AFF-CHANGE-PROC** · 규칙 — 소속을 바꾸려 한다 (`JOINED`)

1. ★**미디어 서버마다 따로** `AFFILIATION` 을 보낸다.
2. 서버를 옮겨 말하려면 ★**§5-1-2 「전환」 표를 따른다** — 같은 서버 안인가 서버를 건너는가로
   갈리고, 순서와 실패 복귀까지 그 표가 정본이다. ★**여기 다시 적지 않는다**(적으면 사본이고,
   §5-1-2 가 늘 때 뒤처진다).
4. ★**응답의 목록은 그 서버가 아는 만큼**이다. **전체로 덮어쓰지 않는다.**

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`AFF.C.NOPUB.select`](#AFF.C.NOPUB.select) · [`AFF.C.PUB.switch_same`](#AFF.C.PUB.switch_same) · [`AFF.C.PUB.deselect`](#AFF.C.PUB.deselect) |
| 2 | [`AFF.C.PUB.switch_cross`](#AFF.C.PUB.switch_cross) · [`AFF.C.PUB.switch_in`](#AFF.C.PUB.switch_in) |
| 3 | [`AFF.C.NOPUB.select`](#AFF.C.NOPUB.select) · [`AFF.C.PUB.switch_same`](#AFF.C.PUB.switch_same) · [`AFF.C.PUB.deselect`](#AFF.C.PUB.deselect) · [`AFF.C.any.fail`](#AFF.C.any.fail) · [`AFF.C.PUB.switch_stuck`](#AFF.C.PUB.switch_stuck) · [`AFF.C.NOPUB.res_pub`](#AFF.C.NOPUB.res_pub) · [`AFF.C.PUB.res_moved`](#AFF.C.PUB.res_moved) · [`AFF.C.PUB.res_away`](#AFF.C.PUB.res_away) · [`AFF.C.NOPUB.join_select`](#AFF.C.NOPUB.join_select) · [`AFF.C.any.implicit_busy`](#AFF.C.any.implicit_busy) · [`AFF.C.any.join_failed`](#AFF.C.any.join_failed) |
| 4 | [`AFF.C.any.res_same`](#AFF.C.any.res_same) |
| 5 | [`AFF.C.NOPUB.res_pub`](#AFF.C.NOPUB.res_pub) · [`AFF.C.PUB.res_moved`](#AFF.C.PUB.res_moved) · [`AFF.C.PUB.res_away`](#AFF.C.PUB.res_away) · [`AFF.C.any.res_same`](#AFF.C.any.res_same) · [`AFF.C.any.implicit_pub`](#AFF.C.any.implicit_pub) |

<a id="AFF.C.any"></a>
#### 5-2-3. 클라 · `any`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="AFF.C.any.fail"></a>`AFF.C.any.fail` | !ok | wait ← 'NONE'; want ← 'NONE' · 효과 app.aff_failed | §5-1-2 §5-2-2 3 — 1006 미디어 코덱 불일치 · 2006 새 방 권한(내 트랙 하나라도 — 전환 전체 실패) · 2001 · 5003(RULE-AFF-FAIL) — 상태는 고치지 않는다 · 전환도 멈춘다 |
| <a id="AFF.C.any.res_same"></a>`AFF.C.any.res_same` | 나머지 | wait ← 'NONE'; want ← 'NONE' | §5-1-2 §5-2-2 4 §5-2-2 5 — 발행 방이 그대로다 — 다른 서버의 응답은 그 서버가 아는 만큼이다(전체로 덮어쓰지 않는다 · RULE-AFF-PARTIAL) · 건너뛴 항목은 안 바뀐 것(RULE-AFF-PROC) |
| <a id="AFF.C.any.implicit_joined"></a>`AFF.C.any.implicit_joined` | 받는 효과 ROOM aff.implicit · sel && wait == 'JOIN' && want == r && p == r | pub ← p; wait ← 'NONE'; want ← 'NONE' · 효과 floor.pub_select → FLR.C.any.pub_select · 효과 app.affiliation | §4-2-1 2 §5-1-2 — select:true 입장이 발행 방을 이 방으로 정했다 — 기다림을 푼다 |
| <a id="AFF.C.any.implicit_join_unselected"></a>`AFF.C.any.implicit_join_unselected` | 받는 효과 ROOM aff.implicit · sel && wait == 'JOIN' && want == r | wait ← 'NONE'; want ← 'NONE' · 효과 app.aff_failed | §4-2-1 2 §5-1-2 — 입장은 됐는데 그 서버가 이 방을 발행 방으로 두지 않았다 — 기다림을 풀고 실패로 알린다(입장은 그대로) |
| <a id="AFF.C.any.implicit_busy"></a>`AFF.C.any.implicit_busy` | 받는 효과 ROOM aff.implicit · wait != 'NONE' | — | §5-1-2 §5-2-2 3 — 다른 전환 요청이 떠 있다 — 그 응답이 정한다(요청은 한 번에 하나 · 응답이 온 것만 반영) |
| <a id="AFF.C.any.implicit_pub"></a>`AFF.C.any.implicit_pub` | 받는 효과 ROOM aff.implicit · p != 'NONE' && pub != p | pub ← p · 효과 floor.pub_away → FLR.C.any.pub_away · 효과 floor.pub_select → FLR.C.any.pub_select · 효과 app.affiliation | §5-1-2 §5-2-2 5 — 입장 응답이 그 서버의 발행 방을 이 값으로 말한다(같은 서버의 옛 발행 방은 서버가 내렸다) |
| <a id="AFF.C.any.implicit_away"></a>`AFF.C.any.implicit_away` | 받는 효과 ROOM aff.implicit · p == 'NONE' && onPub && pub != 'NONE' | pub ← 'NONE' · 효과 floor.pub_away → FLR.C.any.pub_away · 효과 app.affiliation | §5-1-2 §5-1-1 — 그 서버가 내 발행 방을 없다고 말한다 — 그 서버의 방이었을 때만 믿는다(다른 서버의 발행 방은 그 서버가 모른다 · 전체로 덮어쓰지 않는다) |
| <a id="AFF.C.any.implicit_same"></a>`AFF.C.any.implicit_same` | 받는 효과 ROOM aff.implicit · 나머지 | — | §5-1-2 §5-1-1 — 발행 방이 그대로다 — select:false 입장 · 다른 서버의 응답 |
| <a id="AFF.C.any.left"></a>`AFF.C.any.left` | 받는 효과 ROOM aff.leave · pub == r | pub ← 'NONE' | §5-1-2 — 그 방이 발행 방이었으면 pub_room 이 null 이 된다 — 발언권은 ROOM 이 floor.leave 로 직접 알린다 |
| <a id="AFF.C.any.left_join"></a>`AFF.C.any.left_join` | 받는 효과 ROOM aff.leave · wait == 'JOIN' && want == r | wait ← 'NONE'; want ← 'NONE' · 효과 app.aff_failed | §4-2-1 2 §5-1-2 — select:true 로 들어가던 방이 응답 전에 내려갔다 — 그 입장의 응답은 오지 않는다(기다림을 풀 소속 응답도 없다) · 기다림을 풀고 실패로 알린다 |
| <a id="AFF.C.any.left_want"></a>`AFF.C.any.left_want` | 받는 효과 ROOM aff.leave · want == r | want ← 'NONE' | §5-1-2 — 옮겨 가려던 방을 떠났다 — 그 전환은 멈춘다(입장하지 않은 방으로는 말할 수 없다 · RULE-AFF-PROC) |
| <a id="AFF.C.any.left_other"></a>`AFF.C.any.left_other` | 받는 효과 ROOM aff.leave · 나머지 | — | §5-1-2 — 듣기만 하던 방 — 소속의 발행 방은 그대로 |
| <a id="AFF.C.any.join_failed"></a>`AFF.C.any.join_failed` | 받는 효과 ROOM aff.join_failed · wait == 'JOIN' && want == r | wait ← 'NONE'; want ← 'NONE' · 효과 app.aff_failed | §4-2-1 2 §5-2-2 3 — 발행 방은 그대로(서버가 입장을 받지 않았으니 곁들인 select 도 없다) |
| <a id="AFF.C.any.join_failed_other"></a>`AFF.C.any.join_failed_other` | 받는 효과 ROOM aff.join_failed · 나머지 | — | §4-2-1 2 — select:false 입장 — 소속은 그대로 |

<a id="AFF.S"></a>
#### 5-2-4. 서버

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="AFF.S.reject"></a>`AFF.S.reject` | bad | 답한다 AFFILIATION.fail {code: 1006} | §5-1-2 |
| <a id="AFF.S.reject_perm"></a>`AFF.S.reject_perm` | noperm | 답한다 AFFILIATION.fail {code: 2006} | §5-1-2 |
| <a id="AFF.S.select"></a>`AFF.S.select` | sel != 'NONE' && pub == 'NONE' | pub ← sel · 답한다 AFFILIATION.res {pub_room: sel} | §5-1-2 |
| <a id="AFF.S.select_moved"></a>`AFF.S.select_moved` | sel != 'NONE' && pub != sel | pub ← sel · 답한다 AFFILIATION.res {pub_room: sel} | §5-1-2 |
| <a id="AFF.S.away"></a>`AFF.S.away` | sel == 'NONE' && desel != 'NONE' && pub == desel | pub ← 'NONE' · 답한다 AFFILIATION.res {pub_room: 'NONE'} | §5-1-2 |
| <a id="AFF.S.same"></a>`AFF.S.same` | 나머지 | 답한다 AFFILIATION.res {pub_room: pub} | §5-1-2 |
| <a id="AFF.S.join_select"></a>`AFF.S.join_select` | 받는 효과 ROOM aff.join · sel && pub == 'NONE' | pub ← r | §4-1 §4-2-1 |
| <a id="AFF.S.join_select_moved"></a>`AFF.S.join_select_moved` | 받는 효과 ROOM aff.join · sel && pub != r | pub ← r | §4-1 §4-2-1 |
| <a id="AFF.S.join_listen"></a>`AFF.S.join_listen` | 받는 효과 ROOM aff.join · 나머지 | — | §4-1 |
| <a id="AFF.S.left"></a>`AFF.S.left` | 받는 효과 ROOM aff.leave · pub == r | pub ← 'NONE' | §5-1-2 |
| <a id="AFF.S.left_other"></a>`AFF.S.left_other` | 받는 효과 ROOM aff.leave · 나머지 | — | §5-1-2 |

### 5-3. 주제

### 5-4. 근거 · 기각 — 규격이 아니다

> 계약 본문(구조 · 동작 · 주제)의 규칙이 왜 그런가(R) · 무엇을 하지 않기로 했나(REJ). 소절 머리는 그 근거가 붙은 계약 자리다. 계약 판에는 싣지 않는다.

#### 5-4-1. §5-1-2 `0x0401 AFFILIATION` — 어디로 발행할까   **C→S** · 응답 있음

<a id="REJ-AFF-NO-SUB-AXIS"></a>**REJ-AFF-NO-SUB-AXIS** · 기각

★**`sub_add`/`sub_remove` 는 두지 않는다.** 입장 없는 구독(`SUBSCRIBED`)과 한 몸인
필드다 — 기각 근거는 §4-4-1. 같은 이름을 다른 뜻으로 재사용하지 않는다.

<a id="R-AFF-FRESHNESS-BY-TRANSPORT"></a>**R-AFF-FRESHNESS-BY-TRANSPORT** · 근거

★**신선도는 전송 계층이 지킨다** — §2-2-1 가 윈도우 기본 1 이고 *"서버는 받은 순서대로 처리한다"* 를
못박았으며 `pub_room` 은 ★**출처가 나 하나**다. 경주가 성립할 두 경로가 없다.

<a id="R-AFF-FORCED-SYMPTOM"></a>**R-AFF-FORCED-SYMPTOM** · 근거

★**증상** — 강제 변경을 응답 축에만 두면 → ★**강제 `cause` 값들이 영원히 wire 에 나타나지 않는다.**
서버는 소속을 바꿨는데 클라는 **옛 목록을 그대로 들고 있고**, 소리가 안 나는 이유를 못 찾는다.

<a id="REJ-AFF-NO-RECONNECT-RESTORE"></a>**REJ-AFF-NO-RECONNECT-RESTORE** · 기각

★**`cause` 에 `reconnect_restore` 는 없다** — 발동 절차가 어느 문서에도
없는 유령 값이 된다. 재접속 복원은 `RESUME` 스냅샷(§3-1)이 전담한다. 문자열 값이라 필요해지면
새 값으로 정의한다(재사용 아님).

<a id="R-AFF-SYMPTOMS"></a>**R-AFF-SYMPTOMS** · 근거

★**증상**
- ★**응답을 전체로 믿고 덮어쓰면 다른 서버의 듣는 방이 목록에서 사라진다** —
  소리는 나는데 그 방이 없다.
- 입장하지 않은 방에서 말하려 하면 조용히 무시된다(응답의 `pub_room` 이 안 바뀐 것으로 안다).

#### 5-4-2. §5-2-2 클라 · `PUB`

<a id="R-AFF-CHANGE-SYMPTOM"></a>**R-AFF-CHANGE-SYMPTOM** · 근거

★**증상** — 4를 어기면 ★**다른 서버의 듣는 방이 목록에서 사라진다** — 소리는 나는데 그 방이 없다.

#### 5-4-3. 소속 어휘 — `scope` 는 버리고 `pub`/`sub` 는 남긴다

<a id="REJ-AFF-SCOPE-WORD"></a>**REJ-AFF-SCOPE-WORD** · 기각 — 소속 어휘 — `scope` 는 버리고 `pub`/`sub` 는 남긴다

★**`scope` 는 대표성이 없다** — *"범위"* 라는 뜻뿐이라 **무엇의 범위인지 이름에 없다**(방인지 트랙인지 권한인지). `affiliation` 은 *"어디에 소속되어 있나"* 라 방 관계임이 드러난다.

<a id="REJ-AFF-LISTEN-TALK"></a>**REJ-AFF-LISTEN-TALK** · 기각

★★**`pub`/`sub` 를 `listen`/`talk` 로 바꾸는 안은 기각했다.**

| 기각 사유 | |
|---|---|
| ★**이 규격은 WebRTC 위에 선다** | 트랙 층이 이미 `PUBLISH_TRACKS`·`SUBSCRIBE_LAYER` 다. 방 층도 같은 낱말이라야 ★**한 어휘 체계**가 된다 |
| ★**이중 사용이 흠이 아니다** | 방과 트랙은 다른 개념이 아니라 ★**같은 축의 입도 차이**이고, 같은 이름이 그 정합을 말해 준다 |
| ★**`talk` 은 뜻을 좁힌다** | 화면공유가 실리는 방도 *"말하는 방"* 이 된다. 이 제품은 무전만 하지 않는다 |
| 선례가 약했다 | `listen` 의 근거였던 **Mumble `ChannelListener`** 는 ★**다방 청취 선례일 뿐 무전이 아니다**(동시 발화를 안 막고 전량 전달한다). `talk` 의 근거였던 **SvxLink `talker`** 는 ★*"지금 누가 말한다"* 는 **통지**이지 발행 방 선택이 아니다 — 층이 다르다 |

<a id="R-AFF-LISTEN-TALK-USER-WORDS"></a>**R-AFF-LISTEN-TALK-USER-WORDS** · 근거

★**"듣기/말하기" 는 최종 사용자 어휘다.** 필요하면 ★**SDK 공개 API 층**에서 그렇게 부르면 되고, **wire 는 개발자가 읽는 것**이라 WebRTC 어휘를 쓴다. ★**두 층을 섞지 않는다.**

<a id="R-AFF-FLOOR-NAMES-DIFFER"></a>**R-AFF-FLOOR-NAMES-DIFFER** · 근거

★**발언권 상태 이름은 다르다** — 그쪽은 3GPP TS 24.380 을 그대로 쓴다(§10-4-1). 층이 달라 출처도 다르다.

<a id="R-AFF-NOTICE-MOVED"></a>**R-AFF-NOTICE-MOVED** · 근거

★★**소속 통지는 "없앤 것"이 아니라 "옮긴 것"이다.** 응답만 남기면 ★**강제 변경(강퇴·방 닫힘)을 알릴 경로가 사라진다** — ★**응답은 내가 보낸 요청에만 오기 때문이다.** 기각된 것은 ★**방 단위 broadcast** 이지 통지 자체가 아니었다. 그래서 `ROOM_EVENT` 가 ★**unicast 로** 그 자리를 받는다(§4-2-6).

<a id="TRK"></a>
