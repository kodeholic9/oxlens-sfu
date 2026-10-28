[← 차례](README.md)

## 8. SUB — 구독

구독자 × 트랙 — 구독 m-line 배정표(MlineTable — 연결마다 하나) · READY · 레이어 상한(SUBSCRIBE_LAYER) · 받기 게이트 · 다운링크 스트림

| 정체성 | 누가 내나 | 무엇마다 | 수명 | 상태 |
|---|---|---|---|---|
| (구독자, track_id) · 배정(mid · pt — 구독자마다) | server | 방 참여 × 남의 트랙(또는 방 공용 슬롯) | TRACK_EVENT{add} · 입장 응답 tracks[] → TRACK_EVENT{remove} · 퇴장 | — |

| 의존 | 기댐 | 다중도 | 뜻 |
|---|---|---|---|
| SUB | ROOM | N:1 | 구독은 참여한 방의 것 — 방을 떠나면 그 방의 구독을 버린다(sub.drop) |
| SUB | LINK | N:1 | 받기 m-line 은 그 서버 연결 위(§9-4-11) |

### 8-1. 구조

<a id="SUB.type.MlineEntry"></a>
#### 8-1-1. 형 `MlineEntry` — 배정표의 줄 — 구독 m-line 하나

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `mid` | str | ✔ | ★**내 구독 m-line 번호.** 서버가 이 연결에 정한 값 — **십진 정수 문자열**이다. ★**SDP 에서는 문자열로 다루고, 정렬은 수치로 한다**(§9-4-11). ★**발급 규칙이 계약이다**: ①새 줄의 mid 는 그 연결에서 지금까지 준 **최대값보다 크다** ②비게 된 줄(`track_id` 없음)은 ★**같은 `kind` 의 트랙에만 다시 쓴다**(m-line 의 media type 을 바꾸지 않는다 — RFC 3264 §8.3.3 은 바꿔도 된다고 하나 ★브라우저(libwebrtc)가 재활용되지 않는 m-section 의 media type 변경을 거부한다. 근거는 표준이 아니라 구현이다) ③★**같은 `kind` 의 빈 줄이 있으면 그것부터 쓴다 — 새 줄은 빈 줄이 없을 때만**(의무). 안 그러면 전환이 잦을수록 m-line 수가 새 값으로만 자라 BUNDLE 천장(§4-2-6)에 닿는다. ④★**전이중 줄은 `kind` 마다 상한까지만 생긴다** — 넘으면 새 줄 대신 재배정이다(§8-1-2). ★**줄은 지우지 않는다** — 한 번 생긴 m-line 은 그 연결이 끝날 때까지 BUNDLE 에 남는다(§9-4-11 안 쓰는 m-line) |
| `kind` | "audio" / "video" | ✔ | `"audio"` / `"video"` — 줄이 생길 때 정해지고 바뀌지 않는다 |
| `pt` | u8 | ✔ | ★**서버가 이 연결(구독 BUNDLE)에 배정한 PT 다**(7비트) — ★`codec` · `fmtp` 와 함께 줄의 튜플이고 줄이 생길 때 정해진다(다른 튜플의 트랙을 실을 때만 바뀐다 — 그때는 SDP 가 바뀐다, §8-1-2) — 발행자 값이 아니다. 같은 튜플(코덱 이름·클럭·채널 + 구성/패킷화 파라미터 — `fmtp` 전부가 아니다, §9-4-2 ①)은 이 연결에서 늘 같은 PT 이고 다른 튜플은 다른 PT 다. 그래서 한 BUNDLE 안의 PT 충돌이 구조적으로 없다. ★`1pc` 는 내 발행 PT 도 같은 BUNDLE 이라 `ROOM_JOIN` 요청에 실은 번호표(§4-1)를 피해 배정된다. ★**배정 범위는 `96~127` 을 먼저 쓰고 모자라면 `35~63` 이다**(§9-4-2 ①). ★빈 줄도 이 값을 그대로 갖는다 |
| `codec` | str | ✔ | `"opus"` / `"VP8"` / `"H264"` / `"VP9"` — 클럭 · 채널은 이 이름에서 정해진다(§9-1-7). ★그 방의 미디어 코덱이다(§4-1-1) |
| `fmtp` | string? | — | ★**그 코덱의 고정 선언값**(§4-1-1 — 파라미터 부분만, PT 제외 · H264 = `level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f` · opus = `minptime=10;useinbandfec=1`) — 발행자의 fmtp 가 아니다(줄을 돌려쓰므로 발행자를 따라가지 않는다). ★**오면 반드시 SDP 에 넣는다**(§8-3-1) · 안 오면 그 줄을 넣지 않는다(지어내지 않는다 — VP8 은 없다) |
| `rtx_pt` | u8? | — | 재전송 PT. ★`rtx_ssrc` 와 짝으로 온다 — 하나만 오면 실패로 다룬다 |
| `track_id` | string? | — | ★**이 줄에 실린 트랙**(§6-1-1 의 키). ★**없으면 빈 줄이다** — 그 m-line 은 그대로 남고 오는 패킷이 없을 뿐이다(§8-3-1). 화면 쪽은 이 값으로 트랙 목록과 잇는다 — ★**SDP 재료가 아니다**(이 값만 바뀌면 SDP 는 그대로 — 재배정, §8-1-2) |
| `room_id` | string? | — | 실린 트랙의 방 — `track_id` 와 함께 온다(다방 청취의 화면 가르기 · READY 의 방) · ★SDP 재료가 아니다 |
| `ssrc` | u32 | ✔ | ★**다운링크 SSRC**(§9-4-2) — 서버가 줄마다 하나 내고 줄이 사는 동안 바꾸지 않는다(빈 줄 · 재배정 · 출처 전환에도). 발행자 SSRC 가 아니다 |
| `rtx_ssrc` | u32? | — | 재전송 다운링크 SSRC — `rtx_pt` 와 짝 · 줄이 사는 동안 그대로 |
| `stream_id` | str | ✔ | `a=msid` 의 stream-id — ★**줄마다 고정**: 전이중 줄 = `ox-m{mid}`(줄마다 따로 — A/V 를 묶지 않는다) · 무전 슬롯 = `ox-ptt`(§9-4-11). 실린 트랙이 바뀌어도 그대로 |

<a id="RULE-SUB-ROW-ONE-MLINE"></a>**RULE-SUB-ROW-ONE-MLINE** · 규칙

★**줄 하나가 구독 m-line 하나다.** ★**SDP 를 짓는 재료 전부가 그 줄에 있다** — `mid`·`kind`·`pt`·`codec`·`fmtp`·`rtx_pt`·`ssrc`·`rtx_ssrc`·`stream_id`.
클라는 이 줄만으로 m-line 을 짓고 트랙 목록(§6-1-1)과 잇지 않는다. 화면 쪽만 `track_id` 로 목록과 잇는다.
★**`track_id` · `room_id` 는 SDP 재료가 아니다** — 실린 트랙만 바뀌는 표(빈 줄 · 다시 채움 · 재배정, §8-1-2)는 SDP 를 바꾸지 않는다.

→ 구조 — [`SUB.type.MlineEntry`](#SUB.type.MlineEntry)

<a id="SUB.type.MlineTable"></a>
#### 8-1-2. 형 `MlineTable` — 구독 m-line 배정표

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `sfu_id` | str | ✔ | ★**어느 미디어 서버의 연결 표인가**(§9-1-1 — 재기동하면 바뀐다) |
| `arev` | u64 | ✔ | ★**표의 차례 번호** — 그 서버가 이 세션의 표를 바꿀 때마다 하나 오른다(재구축해도 이어진다 · 재기동하면 `sfu_id` 가 바뀐다). ★**같은 `sfu_id` 에서 보관값보다 큰 것만 받는다** — 표 전체라 낡은 것은 버리면 된다(사이를 메울 것이 없다) |
| `mlines` | MlineEntry[] | ✔ | ★**표 전체** — 그 연결의 구독 m-line 전부(빈 줄 포함 · mid 순서는 받는 쪽이 수치로 세운다) |

<a id="RULE-SUB-TABLE-ONE-SOURCE"></a>**RULE-SUB-TABLE-ONE-SOURCE** · 규칙 — `MlineTable` — 구독 m-line 배정표(연결마다 하나)

★★**구독 SDP 의 원천은 배정표 하나다.** 미디어 연결마다 표가 하나 있고, 서버가 그 표의 주인이다. 클라는 차례 번호(`arev`)가 붙은 사본을 갖는다.
★**표는 WS 로만 온다 — 셋 다 표 전체다**: `ROOM_JOIN` 응답의 `mline_table`(§4-1) · `MLINE_EVENT`(§9-1-6) · `RESUME` 응답의 `mline_tables`(§3-1).
★**HTTP 로는 오지 않는다** — `GET /rooms/{id}`(§4-1-14)는 보기용 목록만 싣는다.
트랙 목록(`TrackEntry` — `ROOM_JOIN` 응답 · `TRACK_EVENT` · `RESUME` 스냅샷 · HTTP)은 ★**보기용**이다. 화면만 고치고 SDP 를 건드리지 않는다.

→ 구조 — [`SUB.type.MlineTable`](#SUB.type.MlineTable) · [`SUB.type.MlineEntry`](#SUB.type.MlineEntry)

<a id="RULE-SUB-TABLE-PROC"></a>**RULE-SUB-TABLE-PROC** · 규칙

**받는 법 — 셋 다 같다**
1. ★`sfu_id` 가 보관한 표와 같으면 ★**`arev` 가 보관값보다 클 때만 표를 통째로 갈아 끼운다.** 작거나 같으면 버린다 — 표 전체라 사이를 메울 것이 없다. 셋이 어떤 차례로 닿아도 새것이 이긴다.
2. `sfu_id` 가 다르면(그 서버가 재기동했다) 새 표로 받는다 — 그 서버 연결은 재구축 중이다(§9-2-1).
3. ★**SDP 재료가 바뀌었으면**(줄이 늘었다 · 줄의 튜플이 바뀌었다) 그 서버의 구독 SDP 를 ★**표만으로 처음부터 다시 짓는다**(§9-4-7). 연달아 온 표는 마지막 것 하나로 지어도 된다.
   ★**실린 트랙만 바뀌었으면**(`track_id` · `room_id` — 빈 줄 · 다시 채움 · 재배정) 다시 짓지 않는다 — 화면만 다시 잇고 바로 `READY{arev}`. ★단 협상 중이면 지금 보내지 않는다 — 협상 끝의 `READY` 가 그때의 가장 새 차례를 싣는다(지금 보내면 아직 SDP 에 없는 줄까지 확인하게 된다).
   ★새 차례인데 내용이 그대로면 차례만 올리고 SDP 를 건드리지 않는다.
4. 협상이 끝나면 `READY{arev}` — ★**적용된 SDP 와 줄이 같은 표 가운데 가장 새 것의 `arev`** 를 싣는다(§8-1-3). 실패했으면 보내지 않는다.

→ 구조 — [`LINK.op.MLINE_EVENT`](09-link.md#LINK.op.MLINE_EVENT) · [`SUB.op.READY`](#SUB.op.READY)

<a id="RULE-SUB-TABLE-SERVER"></a>**RULE-SUB-TABLE-SERVER** · 규칙

**서버가 지킬 것**
- ★**표가 바뀔 때마다 `arev` 를 하나 올리고 표 전체를 `MLINE_EVENT` 로 그 사람에게 보낸다.** 바뀐 줄만 보내지 않는다.
- ★**줄은 지우지 않는다.** 트랙이 빠지면 그 줄은 빈 줄(`track_id` 없음)이 된다. 빈 줄은 같은 `kind` 의 다음 트랙이 먼저 쓴다(§8-1-1 `mid` 발급 규칙).
- ★**줄의 튜플(`pt`·`codec`·`fmtp`) · 다운링크 SSRC · `stream_id` 는 줄이 생길 때 정해진다** — 빈 줄도 그대로 갖는다(§8-3-1). 튜플은 다른 튜플의 트랙을 실을 때만 바뀐다(§8-1-2).
- ★`arev` 는 그 서버 프로세스가 사는 동안 세션마다 이어진다 — 연결을 재구축해도 되돌리지 않는다.

<a id="DEV-SUB-MLINE-TABLE"></a>**DEV-SUB-MLINE-TABLE** · 자체 확장

구독 SDP 의 재료를 트랙 목록의 `assign` 에서 떼어 연결마다 배정표 하나로 모은다. 표는 WS 로만 오고 늘 표 전체이며, `arev` 로 새것을 가른다. `READY` 가 그 협상의 `arev` 를 싣고, 서버는 확인된 차례 이후 바뀌지 않은 줄로만 보낸다.

*원문: §6-1-1 `assign`(배정 층 — 트랙 목록 항목에 실린다 · 네 경로 · 재발급은 `version` 없이) · §4-1-14(HTTP 의 `assign` 은 보관본에 없는 것만 받는다) · 근거: R-SUB-TABLE-WHY PROP-LINK-TABLE-AGREES PROP-LINK-EGRESS-ACKED*

<a id="RULE-SUB-FILL"></a>**RULE-SUB-FILL** · 규칙

★★**전이중 줄은 `kind` 마다 상한까지만 생기고, 넘으면 서버가 고른 트랙을 있는 줄에 싣는다(재배정).** 상한은 연결마다 audio 25 · video 25 이고 방 수 · 인원과 무관하다 — ★인원으로 방식을 바꾸지 않는다(문턱 없음). 무전 슬롯 줄은 따로 센다(§4-2-6).

| | video | audio |
|---|---|---|
| 고르는 이 | ★**클라** — `SUBSCRIBE_LAYER` 의 `paused` · `priority`(§8-1-4) | ★**서버** — ingress 음량 확장(클라는 받기 전에 누가 말하는지 모른다) |
| 줄을 원하는 트랙 | `paused:false` 인 받는 트랙(`active:true`) | 같다 |
| 상한을 넘으면 | 서버가 `priority` 로 고른다 — 어느 트랙이 밀리는지는 서버 정의서의 것 | 서버가 음량으로 고른다 — 같다 |

- ★**줄이 생기는 때와 안 생기는 때** — 같은 `kind` 의 빈 줄이 있으면 그것을 쓰고(표만 온다), 없으면 상한 안에서 새 줄이 생기며(구독 SDP 가 바뀐다), 상한이면 재배정이다. 같은 튜플의 줄이 없을 때만 줄의 튜플이 바뀌어 SDP 가 바뀐다(코덱이 다른 두 방을 한 연결로 받을 때만, §4-1-1). 어느 빈 줄 · 어느 트랙을 고르는지는 서버 정의서의 것이다 — 클라는 표로 받는다.
- ★**재배정은 줄의 `track_id` · `room_id` 만 바꾼다** — `mid` · 튜플 · 다운링크 SSRC · `stream_id` 가 그대로라 구독 SDP 가 안 바뀐다. 표(`MLINE_EVENT`, `arev`+1)만 간다 — 트랙 목록은 그대로라 `TRACK_EVENT` 도 `seq` 도 없다.
- ★**서버는 확인(`READY{arev}`) 전에 새 트랙을 그 줄로 보내지 않는다**(§8-1-3) — 옛 트랙은 그 차례부터 멈춘다. 클라가 화면을 다시 잇기 전에 다른 사람이 그 칸에 보이는 일이 구조적으로 없다. 대가는 재배정마다 한 왕복이다.
- 줄을 못 받은 받는 트랙은 목록에는 있고 표에는 없다 — 앱에 *"지금 안 받는 트랙"* 으로 알린다(§8-3-1).
- 밑의 출처가 바뀌는 일(재배정 · 레이어 전환 · 화자 전환)을 서버가 어떻게 잇는지는 §9-4-2 다.

→ 구조 — [`SUB.type.MlineEntry`](#SUB.type.MlineEntry) · [`SUB.op.SUBSCRIBE_LAYER`](#SUB.op.SUBSCRIBE_LAYER) · [`ROOM.ledger.FullAudio`](04-room.md#ROOM.ledger.FullAudio) · [`ROOM.ledger.FullVideo`](04-room.md#ROOM.ledger.FullVideo)

<a id="SUB.op.READY"></a>
#### 8-1-3. `0x0302 READY` — 준비 됐다   **C→S** · 응답 있음
단 1 · 출처 §7-1-2

**요청**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ |  |
| ★`arev` | u64 | ✔ | ★**이 협상에 쓴 배정표의 차례 번호**(§8-1-2). 서버는 이 번호까지 확인된 대응으로만 보낸다(§8-1-3) |

**응답**
— 빈 body

**실패** — `3001` · `3002` · `1002` · `1003` `room_id` 없음

<a id="RULE-SUB-READY-FORM"></a>**RULE-SUB-READY-FORM** · 규칙

→ 구조 — [`SUB.op.READY`](#SUB.op.READY)

<a id="RULE-SUB-READY-WHAT"></a>**RULE-SUB-READY-WHAT** · 규칙

| 언제 | 서버가 하는 일 |
|---|---|
| ★**구독 협상이 성공한 뒤** · ★**미디어 연결을 재구축 없이 되살린 뒤**(재동기, §9-2-1) | 막아 뒀던 영상을 흘리고 **키프레임을 요청**하고, 그 방의 ★**처음 알리기**(§10-4-30)를 한다 — ★**둘 다 `READY` 마다**다. 5초 안에 안 오면 스스로 푼다 |

★**갈래가 없다** — 발행 쪽 정체 판정의 시작은 서버가 업링크 스트림의 첫 RTP 로 스스로 찍는다(DEV-PUB-STALL-FROM-RTP).

→ 구조 — [`SUB.S.ready`](#SUB.S.ready) · [`SUB.C.WANT.nego_ok`](#SUB.C.WANT.nego_ok)

<a id="RULE-SUB-READY-PROC"></a>**RULE-SUB-READY-PROC** · 규칙

**절차**
- ★★**`READY` 는 그 서버에서 내가 들어가 있는 방마다 하나씩 보낸다.** 협상은 서버 단위로
  한 번이지만, 이 신호는 **방 단위**다.
- ★★**`arev` 는 그 협상에 쓴 배정표의 차례 번호다**(§8-1-2) — 한 협상의 `READY` 들은 같은 `arev` 를 싣는다.
- ★**협상이 실패했으면 보내지 않는다.**

<a id="RULE-SUB-EGRESS-ACKED"></a>**RULE-SUB-EGRESS-ACKED** · 규칙

★★**서버는 클라가 확인한 대응으로만 보낸다.** 확인된 차례 = 받은 `READY` 의 `arev` 중 가장 큰 것.
그 트랙의 줄이 ★**확인된 차례 이후 바뀌지 않았을 때만** 그 줄의 `mid` 로 보낸다. 바뀐 줄은 다음 `READY` 까지 보내지 않는다.
- 줄 M 이 A 에서 B 로 넘어가면, A 는 넘기는 차례에서 M 으로 안 나가고 B 는 그 차례가 확인된 뒤에 나간다 — ★**클라 SDP 에 없는 대응으로 패킷이 나가는 길이 없다.**
- T-gate(§8-3-2)는 안전망으로 남는다 — `READY` 가 사라졌을 때 그 서버가 스스로 푼다.

→ 구조 — [`SUB.op.READY`](#SUB.op.READY) · [`LINK.op.MLINE_EVENT`](09-link.md#LINK.op.MLINE_EVENT)

<a id="RULE-SUB-READY-IDEMPOTENT"></a>**RULE-SUB-READY-IDEMPOTENT** · 규칙

★**여러 번 보내도 안전하다** — 이미 풀린 것은 다시 풀지 않고, 키프레임 요청도 그 방 것만 나간다.

<a id="RULE-SUB-READY-FAIL"></a>**RULE-SUB-READY-FAIL** · 규칙

→ 구조 — [`SUB.op.READY`](#SUB.op.READY)

<a id="SUB.op.SUBSCRIBE_LAYER"></a>
#### 8-1-4. `0x0303 SUBSCRIBE_LAYER` — 받을 레이어 고르기   **C→S** · 응답 있음
단 1 · 출처 §7-1-2

**요청**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ |  |
| `targets` | {track_id, spatial, temporal, paused, priority}[] | ✔ |  |

`targets` 안:

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| ★`track_id` | str | ✔ | ★**식별 키.** `user_id` 가 아니다 — 아래 |
| ★`spatial` | u8? | — | ★**공간 레이어 상한.** `0` 이 최저. 범위는 `TrackEntry.scalability`(§6-1-1)의 `L` 값 · ★생략 = 안 바꾼다 |
| ★`temporal` | u8? | — | ★**시간 레이어 상한.** `0` 이 최저. 범위는 같은 값의 `T` · ★생략 = 안 바꾼다 |
| ★`paused` | bool? | — | ★★**받을지 말지.** ★**레이어 값이 아니다** · ★별개 축 |
| ★`priority` | u8? | — (기본 128) | ★**대역 배분 우선순위** `1`~`255`(기본 `128`). 대역이 모자랄 때 ★**어느 트랙부터 채울지** |

**응답**
— 빈 body

**실패** — `3002` 방에 없음 · `1002` body 오류 ★대상별 실패는 실패가 아니다(위 절차).

<a id="RULE-SUB-LAYER-FORM"></a>**RULE-SUB-LAYER-FORM** · 규칙

→ 구조 — [`SUB.op.SUBSCRIBE_LAYER`](#SUB.op.SUBSCRIBE_LAYER)

<a id="EX-SUB-LAYER-REQ"></a>**EX-SUB-LAYER-REQ** · 예시(비규범)

→ 구조 — [`SUB.example.EX-SUB-LAYER-REQ`](#EX-SUB-LAYER-REQ) · [`SUB.op.SUBSCRIBE_LAYER`](#SUB.op.SUBSCRIBE_LAYER)

예시 — `SUBSCRIBE_LAYER` 요청(형과 대조된 값 · 주석은 칸 설명)

```jsonc
{ "room_id": "r1",
  "targets": [ { "track_id": "t-u2-cam",   // ★**식별 키.** `user_id` 가 아니다 — 아래
                 "spatial": 1,             // ★**공간 레이어 상한.** `0` 이 최저. 범위는 `TrackEntry.scalability`(§6-1-1)의 `L` 값 · ★생략 = 안 바꾼다
                 "temporal": 2,            // ★**시간 레이어 상한.** `0` 이 최저. 범위는 같은 값의 `T` · ★생략 = 안 바꾼다
                 "paused": false,          // ★★**받을지 말지.** ★**레이어 값이 아니다** · ★별개 축
                 "priority": 128 } ] }     // ★**대역 배분 우선순위** `1`~`255`(기본 `128`). 대역이 모자랄 때 ★**어느 트랙부터 채울지**
```

<a id="RULE-SUB-LAYER-BY-TRACK-ID"></a>**RULE-SUB-LAYER-BY-TRACK-ID** · 규칙

★★**대상은 `track_id` 다.** `user_id` 가 아니다 — 한 사람이 ★**카메라와 화면공유를 둘 다**
시뮬캐스트로 올리면 사람으로는 **어느 쪽 레이어인지 지정할 수 없다.** 이 문서의 다른 모든
곳에서 식별 키는 `track_id` 이고(§6-1-1 · §4-2-6), ★**여기만 어긋날 이유가 없다.**

→ 구조 — [`SUB.op.SUBSCRIBE_LAYER`](#SUB.op.SUBSCRIBE_LAYER)

<a id="RULE-SUB-LAYER-PAUSED-AXIS"></a>**RULE-SUB-LAYER-PAUSED-AXIS** · 규칙

★**`paused` 를 레이어 값에 섞지 않는다.** *"안 받는다"* 와 *"낮은 화질로 받는다"* 는
★**다른 축**이다. 한 필드에 섞으면 ★**모르는 값이 왔을 때 어느 쪽으로 떨어질지 정할 수 없다.**

→ 구조 — [`SUB.op.SUBSCRIBE_LAYER`](#SUB.op.SUBSCRIBE_LAYER)

<a id="RULE-SUB-LAYER-PROC"></a>**RULE-SUB-LAYER-PROC** · 규칙

**절차**
- 클라 ★**여러 대상을 한 번에** 보낼 수 있다. ★**생략한 필드는 안 바꾼다** — 부분 갱신이다.
- ★★**요청은 "지정"이 아니라 "상한"이다.** 그 레이어가 없거나 대역이 부족하면 ★**실제 선택은
  서버가 한다** — 무엇을 보고 언제 올리고 내리는지는 서버 정의서의 것이다(클라가 보는 것은 상한 안에서 바뀌는 레이어뿐이다).
- 클라 ★**`TrackEntry.scalability`(§6-1-1) 밖의 값을 보내지 않는다.** `"L2T3"` 이면
  `spatial` 은 `0`~`1`, `temporal` 은 `0`~`2` 다.
- 서버 ★**범위를 넘는 값은 그 축의 최대로 자른다**(거절하지 않는다). ★**상한이므로 자르는 것이 뜻에 맞다.**
- 대상별 실패는 ★**조용히 건너뛴다.** 응답은 그래도 성공이다.

<a id="RULE-SUB-LAYER-FAIL"></a>**RULE-SUB-LAYER-FAIL** · 규칙

→ 구조 — [`SUB.op.SUBSCRIBE_LAYER`](#SUB.op.SUBSCRIBE_LAYER)

<a id="SUB.state"></a>
#### 8-1-5. 상태 칸 · 시한

##### 상태 — 클라

| 칸 | 처음 | 뜻 |
|---|---|---|
| `ready` | `'NONE'` | 그 방의 READY(RULE-SUB-READY-PROC — 방 단위) |

##### 상태 — 서버

| 칸 | 처음 | 뜻 |
|---|---|---|
| `gated` | `true` | 받을 트랙을 막고 있다 — READY 가 풀고 T-gate(5초)가 안전망으로 푼다(§8-3-2) |

##### 전이 — 클라(동작 경우에서 짓는다)

| 지금 | 사건 | 다음 | 라벨 |
|---|---|---|---|
| 어느 상태 | 그 방에 READY 를 보낼 때가 됐다 — 입장 · 스냅샷 반영 뒤(RULE-SUB-READY-PROC) | `WANT` | `SUB.C.any.ready_want` |
| `WANT` | 그 서버 재협상이 끝났다 — 이번에 바뀐 방마다 READY(RULE-TRK-TRACK-EVENT-PROC) | `SENT` | `SUB.C.WANT.nego_ok` |
| 어느 상태 | 그 서버 재협상이 끝났다 — 이번에 바뀐 방마다 READY(RULE-TRK-TRACK-EVENT-PROC) | — | `SUB.C.any.nego_ok_other` |
| `WANT` | 협상이 실패했다 — READY 를 보내지 않는다(RULE-SUB-READY-PROC) | `NONE` | `SUB.C.WANT.nego_fail` |
| 어느 상태 | 협상이 실패했다 — READY 를 보내지 않는다(RULE-SUB-READY-PROC) | — | `SUB.C.any.nego_fail_other` |
| 어느 상태 | READY 응답을 받았다(RULE-SUB-READY-FAIL) | — | `SUB.C.any.ready_res_old` |
| `SENT` | READY 응답을 받았다(RULE-SUB-READY-FAIL) | `OK` | `SUB.C.SENT.ready_ok` |
| `SENT` | READY 응답을 받았다(RULE-SUB-READY-FAIL) | `NONE` | `SUB.C.SENT.ready_fail` |
| 어느 상태 | 그 서버의 배정표가 바뀌었다(MLINE_EVENT · 입장 응답 · RESUME) — 구독 SDP 를 그 표만으로 다시 짓고 협상이 끝나면 READY{arev}(RULE-SUB-TABLE-PROC) | `WANT` | `SUB.C.any.table_changed` |
| `WANT` | 그 서버의 배정표에서 줄은 그대로이고 실린 트랙만 바뀌었다(재배정 · 빈 줄 · 다시 채움 — RULE-SUB-FILL) — SDP 를 다시 짓지 않고 READY{arev}(RULE-SUB-TABLE-PROC) | 그대로 | `SUB.C.WANT.table_rebound` |
| `NONE` | 그 서버의 배정표에서 줄은 그대로이고 실린 트랙만 바뀌었다(재배정 · 빈 줄 · 다시 채움 — RULE-SUB-FILL) — SDP 를 다시 짓지 않고 READY{arev}(RULE-SUB-TABLE-PROC) | 그대로 | `SUB.C.NONE.table_rebound` |
| 어느 상태 | 그 서버의 배정표에서 줄은 그대로이고 실린 트랙만 바뀌었다(재배정 · 빈 줄 · 다시 채움 — RULE-SUB-FILL) — SDP 를 다시 짓지 않고 READY{arev}(RULE-SUB-TABLE-PROC) | `SENT` | `SUB.C.any.table_rebound` |
| 어느 상태 | SUBSCRIBE_LAYER 를 보낸다 — 지정이 아니라 상한 · 생략한 필드는 안 바꾼다(RULE-SUB-LAYER-PROC) | — | `SUB.C.any.layer` |
| 어느 상태 | SUBSCRIBE_LAYER 응답 — 대상별 실패는 조용히 건너뛴다(RULE-SUB-LAYER-FAIL) | — | `SUB.C.any.layer_res` |
| 어느 상태 | 그 방을 떠났다 · 잃었다 — 그 방의 구독을 버린다 | `NONE` | `SUB.C.any.drop` |

<a id="SUB.C"></a>
### 8-2. 동작

<a id="SUB.C.any"></a>
#### 8-2-1. 클라 · `any`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="SUB.C.any.ready_want"></a>`SUB.C.any.ready_want` | 받는 효과 ROOM ready.tracks · 나머지 | ready ← 'WANT' | §7-1-2 — 방마다 하나 · ★협상이 끝나면 보낸다 — 실패했으면 보내지 않는다(RULE-SUB-READY-PROC) · 협상 결과는 LINK 가 준다 |
| <a id="SUB.C.any.nego_ok_other"></a>`SUB.C.any.nego_ok_other` | 받는 효과 LINK sub.nego_ok · 나머지 | — | §7-1-2 — 이 방은 바뀌지 않았다 |
| <a id="SUB.C.any.nego_fail_other"></a>`SUB.C.any.nego_fail_other` | 받는 효과 LINK sub.nego_fail · 나머지 | — | §7-1-2 |
| <a id="SUB.C.any.ready_res_old"></a>`SUB.C.any.ready_res_old` | ready != 'SENT' | — | §7-1-2 §4-2-6 — 옛 READY 의 응답 — 그 뒤 더 새 READY 가 필요해졌다(WANT — 남의 트랙이 붙어 다시 협상한다 · RULE-TRK-TRACK-EVENT-PROC). 상태를 바꾸지 않는다 — 덮어쓰면 새 READY 를 빠뜨려 받는 영상이 검다(R-ROOM-JOIN-OK-SYMPTOM) · 방을 버린 뒤(NONE) 늦게 온 응답도 같다(SUB 모형이 드러냄) |
| <a id="SUB.C.any.table_changed"></a>`SUB.C.any.table_changed` | 받는 효과 LINK sub.table_changed · 나머지 | ready ← 'WANT' · 효과 sdp.rebuild → LINK.C.any.renego_queued · LINK.C.any.renego | §8-1-2 — 재조립은 그 서버 전체(표가 연결 하나에 하나) · 연달아 온 표는 마지막 것 하나로 조립해도 된다(표 전체라서) |
| <a id="SUB.C.any.table_rebound"></a>`SUB.C.any.table_rebound` | 받는 효과 LINK sub.table_rebound · 나머지 | ready ← 'SENT' · 보낸다 READY.req | §8-1-2 §8-1-3 — 적용된 SDP 가 이 표의 줄을 다 갖는다 — 바로 READY{arev} · 서버는 그 뒤에 새 트랙을 그 줄로 보낸다(확인 전에는 그 줄로 아무것도 안 보낸다) |
| <a id="SUB.C.any.layer"></a>`SUB.C.any.layer` | 나머지 | 보낸다 SUBSCRIBE_LAYER.req | §7-1-2 |
| <a id="SUB.C.any.layer_res"></a>`SUB.C.any.layer_res` | 나머지 | — | §7-1-2 |
| <a id="SUB.C.any.drop"></a>`SUB.C.any.drop` | 받는 효과 ROOM sub.drop · 나머지 | ready ← 'NONE' | §6-1-1 |

<a id="RULE-SUB-ASSIGN-TIER"></a>**RULE-SUB-ASSIGN-TIER** · 규칙

**배정 = 배정표의 줄**(§8-1-2). ★**트랙 목록 항목(`TrackEntry`)에는 배정이 없다.** 그 트랙이 내게 오는지는 표에 그 `track_id` 의 줄이 있는가로 안다.

→ 구조 — [`SUB.type.MlineEntry`](#SUB.type.MlineEntry)

<a id="SUB.C.WANT"></a>
#### 8-2-2. 클라 · `WANT`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="SUB.C.WANT.nego_ok"></a>`SUB.C.WANT.nego_ok` | 받는 효과 LINK sub.nego_ok · ready == 'WANT' | ready ← 'SENT' · 보낸다 READY.req | §7-1-2 §4-2-6 — 여러 번 보내도 안전하다(RULE-SUB-READY-IDEMPOTENT) |
| <a id="SUB.C.WANT.nego_fail"></a>`SUB.C.WANT.nego_fail` | 받는 효과 LINK sub.nego_fail · ready == 'WANT' | ready ← 'NONE' | §7-1-2 |
| <a id="SUB.C.WANT.table_rebound"></a>`SUB.C.WANT.table_rebound` | 받는 효과 LINK sub.table_rebound · ready == 'WANT' | — | §8-1-2 — 협상 중이다 — 지금 READY 를 보내면 아직 SDP 에 없는 줄까지 확인하게 된다 · 협상이 끝나면 보내는 READY 가 그때의 가장 새 차례를 싣는다 |

<a id="SUB.C.SENT"></a>
#### 8-2-3. 클라 · `SENT`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="SUB.C.SENT.ready_ok"></a>`SUB.C.SENT.ready_ok` | m.part == 'res' | ready ← 'OK' | §7-1-2 |
| <a id="SUB.C.SENT.ready_fail"></a>`SUB.C.SENT.ready_fail` | 나머지 | ready ← 'NONE' · 효과 app.bug | §7-1-2 — 3001 · 3002 · 1002 · 1003 · 3005 — 클라가 틀렸거나 방이 없다 |

<a id="SUB.C.NONE"></a>
#### 8-2-4. 클라 · `NONE`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="SUB.C.NONE.table_rebound"></a>`SUB.C.NONE.table_rebound` | 받는 효과 LINK sub.table_rebound · ready == 'NONE' | — | §8-1-2 — 적용된 구독 SDP 가 없다(입장 전 · 협상 실패) — 확인할 것이 없다 |

<a id="SUB.S"></a>
#### 8-2-5. 서버

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="SUB.S.ready"></a>`SUB.S.ready` | 나머지 | gated ← false · 답한다 READY.res | §7-1-2 §8-3-2 |
| <a id="SUB.S.drop"></a>`SUB.S.drop` | 받는 효과 ROOM sub.drop · 나머지 | gated ← true | §8-3-2 |

### 8-3. 주제

#### 8-3-1. ★층마다 규칙이 다르다 — 발행 → 트랙 대응 · 갱신 · `seq`

<a id="RULE-SUB-RESTING-HAS-PT"></a>**RULE-SUB-RESTING-HAS-PT** · 규칙

- ★★**빈 줄(`track_id` 없음)도 m-line 그대로다** — 쉬거나(`active:false` — full → half 순간 서버가 줄을 놓는다, PROP-LINK-ROW-ONLY-RECEIVED) 빠진 트랙의 줄은
  BUNDLE 에서 빠지지 않고 ★**튜플 · 다운링크 SSRC · `stream_id` · 방향을 그대로 갖는다** — 그 줄로 오는 패킷이 없을 뿐이다(§9-4-11). 그래서 빈 줄이 생기고 채워져도 SDP 가 안 바뀐다.
  §9-4-16 의 `mid:2` 가 그 모양이다. 서버가 그 줄을 같은 kind 의 다른 트랙에 주면 다음 표가 채운다 — 한 줄을 두 트랙이 쥐는 일이 없다.
  ★옛 꼴(`a=inactive` · `port=7` · `a=msid`·`a=ssrc` 뺌)은 걷어 냈다 — 빈 줄이 생기고 채워질 때마다 재협상이 났다.

<a id="RULE-SUB-REMOVE-KEEPS-MID"></a>**RULE-SUB-REMOVE-KEEPS-MID** · 규칙

- ★**`TRACK_EVENT{remove}` 는 목록에서 항목을 지울 뿐이다** — m-line 은 표가 다룬다. 서버는 그 줄을 빈 줄로 만든 표를 보내고, 같은 kind 의 새 트랙이 오면 그 줄을 다시 채운 표를 보낸다(발급 규칙은 §8-1-1 `mid`) — 같은 튜플이면 SDP 는 그대로다.

<a id="RULE-SUB-ASSIGN-RECOVERED"></a>**RULE-SUB-ASSIGN-RECOVERED** · 규칙

- ★**줄 없이 알렸던 받는 트랙에 줄이 생기면 서버는 그 트랙을 실은 표를 보낸다**(줄이 비었다 · 서버가 골랐다 — §8-1-2) — 트랙 목록은 그대로라 `TRACK_EVENT` 도 `seq` 도 없다. 받는 법은 표의 것 하나다(§8-1-2).

<a id="RULE-SUB-NO-ASSIGN-NO-SDP"></a>**RULE-SUB-NO-ASSIGN-NO-SDP** · 규칙

- ★★**구독 SDP 는 표의 줄로만 짓는다** — 목록에 있어도 표에 줄이 없는 트랙은 m-line 이 없다. 입장하지 않은 방의 트랙(§4-1-14)에는 줄이 없다 — ★**읽을 자료**다.
  ★**입장한 방의 받는 트랙(`active:true`)인데 표에 줄이 없으면** — 줄이 상한에 닿아 서버가 지금 그 트랙을 싣지 않은 것이다(§8-1-2 — `paused` 이거나 우선순위 · 음량에서 밀렸다). 트랙의 존재는 알되 지금은 안 받는다 — 앱에 "지금 안 받는 트랙"으로
  알린다(조용히 없는 것처럼 두지 않는다). ★**줄이 없는 원인은 셋이다** — `user_id` 가 나인 항목(내 트랙 — 받을 것이 아니다)과
  `active:false` 인 항목(쉬는 트랙 — 비점유)은 ★**밀린 것이 아니다.** 이 판정은 그 둘을 뺀 나머지에만 건다 — 안 가르면 발행할 때마다 자기 트랙이 "지금 안 받는 트랙"으로 뜬다.
  ★이 판정은 화면 쪽 대조다 — SDP 는 목록을 보지 않는다.

<a id="RULE-SUB-FMTP-MUST"></a>**RULE-SUB-FMTP-MUST** · 규칙

- ★★**`fmtp` 가 오면 반드시 SDP 에 넣는다.** 코덱 이름만 맞추는 것으로는 부족하다 —
  ★**H264 는 `profile-level-id`·`packetization-mode` 에서 갈린다.** 빠뜨리면 브라우저가
  `packetization-mode=0`·baseline 을 가정하는데 실제 스트림이 `pm=1` 이면
  ★**패킷은 오는데 디코딩이 0** 이다. 코덱 계약은 **이름 아래 한 층까지**가 계약이다.

→ 구조 — [`LINK.sdp.recv`](09-link.md#LINK.sdp.recv)

#### 8-3-2. 미디어

<a id="RULE-SUB-TGATE"></a>**RULE-SUB-TGATE** · 규칙

| 이름 | 값 | 시작 | 정상 정지 | 끝나면 |
|---|---|---|---|---|
| `T-gate`(서버) | **5초** | 받을 트랙을 막았다 | `READY` 를 받았다 | 서버가 스스로 푼다(안전망) |

### 8-4. 근거 · 기각 — 규격이 아니다

> 계약 본문(구조 · 동작 · 주제)의 규칙이 왜 그런가(R) · 무엇을 하지 않기로 했나(REJ). 소절 머리는 그 근거가 붙은 계약 자리다. 계약 판에는 싣지 않는다.

#### 8-4-1. §8-1-2 형 `MlineTable` — 구독 m-line 배정표

<a id="R-SUB-TABLE-WHY"></a>**R-SUB-TABLE-WHY** · 근거

★**왜 표 하나인가** — 옛 꼴은 서버 안의 연결별 배정(mid → 트랙)을 클라가 여러 길의 사건으로 다시 지었다. 그래서 어긋날 자리가 일곱이었다.
①HTTP 와 WS 의 경주 ②`version` 없는 재발급이 트랙 층까지 되감음 ③mid 유일성을 클라가 짐작 ④서버가 클라가 무엇까지 적용했는지 모름 ⑤SDP 를 두 층의 합으로 지음 ⑥지운 m-line 의 코덱을 클라가 기억 ⑦mid 공간은 연결 단위인데 순서는 방 단위.
트랙 모형이 ①②를 반례로 냈다(TK1 · TK13). 표를 원천으로 두고 한 길로, 차례 번호와 함께 보내면 일곱이 규칙이 아니라 짜임으로 없어진다.
선례: mediasoup(재료는 consumer 하나 = 한 묶음 · mid 는 서버가 늘리기만) · LiveKit(`SessionDescription.mid_to_track_id` — SDP 와 대응을 한 메시지에) · Jitsi(대응은 연결마다 바뀐 것만 · mid 는 자리에 묶임).

*없으면: 낡은 재발급이 새 스냅샷 뒤에 닿아 쉬는 트랙을 받는 트랙으로 되돌리고 그 mid 를 빼앗는다 · 서버가 클라 SDP 에 없는 대응으로 패킷을 보낸다*

<a id="R-SUB-FILL-WHY"></a>**R-SUB-FILL-WHY** · 근거

★**왜 장치 하나인가**(갈래 A) — 받는 트랙 수가 구독 m-line 수보다 많아지면 m-line 을 돌려써야 한다. 인원 문턱으로 방식을 바꾸면(갈래 B) 문턱을 넘는 순간 SSRC 가 바뀌고 모두가 재협상하는 갈래가 따로 생긴다. 처음부터 한 장치로 두면 작은 방에서는 재배정이 일어나지 않을 뿐 같은 길이다.
선례: Jitsi Videobridge SSRC rewriting — 구독자마다 SSRC 상한(audio 50 · video 50) · 오래 안 쓴 것부터 다시 쓴다 · 방 크기와 무관 · 재배정은 bridge channel 맵 통지(주인만 바꾼다 — 재협상 없음). ★Jitsi 는 통지와 패킷의 경주를 여유분(상한 50 > 동시 타일 25)으로 줄이지만 우리는 확인된 차례(`READY{arev}`)로 막는다.

*없으면: 큰 방에서 m-line 이 BUNDLE 천장(§4-2-6)에 닿아 `4005` 로 입장이 막히거나, 문턱을 넘는 순간 모든 구독자가 재협상한다*

#### 8-4-2. §8-1-3 `0x0302 READY` — 준비 됐다   **C→S** · 응답 있음

<a id="R-SUB-READY-ONE-KIND"></a>**R-SUB-READY-ONE-KIND** · 근거

★**준비 신호는 구독 쪽 하나다** — *"이제 흘려도 된다"*(구독 준비). 서버는 그 신호 전엔 흘리지 않는다.
★발행 쪽 준비 신호(옛 `READY{camera}` — *"프레임이 흐르기 시작했다"*)는 두지 않는다 — 서버가 그 트랙의 업링크 스트림 첫 RTP 를 ingress 에서 스스로 안다(DEV-PUB-STALL-FROM-RTP).
★`type:"transport"`(1pc 번호표 신고)는 없다 — 번호표는 `ROOM_JOIN` 요청의 `extmap`·`codecs` 로 간다(§4-1).
번호는 브라우저 offer 시점에 정해져 서버 재료가 필요 없으므로 첫 입장이 헛도는 바퀴 자체가 없다.

<a id="R-SUB-READY-PER-ROOM"></a>**R-SUB-READY-PER-ROOM** · 근거

★**왜 방마다인가** — 서버가 이 신호를 받고 하는 일이 **둘인데 범위가 다르다.**

| 서버가 하는 일 | 범위 |
|---|---|
| 막아 둔 것을 푼다 | ★**그 서버의 내 것 전부** — 한 번이면 족하다 |
| ★**키프레임을 요청한다** | ★**`room_id` 의 그 방에만** |
| ★**이미 말하는 사람을 알린다**(§10-4-30) | ★**`room_id` 의 그 방에만** |

<a id="R-SUB-READY-SYMPTOMS"></a>**R-SUB-READY-SYMPTOMS** · 근거

★**증상**
- `READY` 를 안 보내면 → ★**받는 영상이 영구히 검다.**
- ★**한 방에만 보내면** → 그 서버에 듣는 방이 여럿일 때 ★**나머지 방 영상이 검고**, 그 방에서
  **이미 말하고 있는 사람도 안 뜬다**(§10-4-30). 막아 둔 것은 풀렸으므로 ★**패킷은 오는데 화면만 검다.**
- 실패한 협상에 보내면 → 서버가 **적용되지도 않은 SDP** 에 대고 영상을 흘린다.

#### 8-4-3. §8-1-4 `0x0303 SUBSCRIBE_LAYER` — 받을 레이어 고르기   **C→S** · 응답 있음

<a id="REJ-SUB-LAYER-NO-RID"></a>**REJ-SUB-LAYER-NO-RID** · 기각

★★**`rid` 로 지목하지 않는다.** ★**`rid`("h"/"l")는 SDP·송신 쪽 어휘**다(§9-4-10 · §7-1-2
`PUBLISH_TRACKS`). 받는 쪽 제어는 ★**정수 인덱스**로 한다.

| 왜 | |
|---|---|
| ★**레이어가 늘면 이름이 모자란다** | 공간 레이어가 셋 · 넷이 되면 `"m"`·`"xl"` 처럼 이름을 계속 지어야 한다. 인덱스는 안 그렇다 |
| ★★**temporal 축이 `rid` 에 없다** | `rid` 는 **공간 단**만 가리킨다. 시간 레이어를 지목할 자리가 아예 없어진다 |
| 층 분리 | `rid`↔`spatial` 사상은 ★**서버가 한다** — 클라는 SDP 어휘를 제어 축에 끌어들이지 않는다 |

<a id="R-SUB-LAYER-SYMPTOMS"></a>**R-SUB-LAYER-SYMPTOMS** · 근거

★**증상**
- ★**`paused` 를 레이어 값에 섞으면** → 모르는 값이 ★**최고 화질로 떨어져 대역이 폭증한다.**
  축을 나누면 그 갈래가 아예 없어진다.
- ★**`scalability` 를 안 주면** → 클라가 ★**레이어가 몇인지 몰라 찍어 본다.** 없는 레이어를 고르면
  잘리므로 조용히 최고 화질을 받는다.
- 부분 실패인데 **응답이 성공**이라 클라는 **다 됐다고 믿는다.**
- ★`user_id` 로 지목하면 → 화면공유를 같이 켠 사람에게 ★**엉뚱한 트랙의 화질이 바뀐다.**
  대역은 줄었는데 **줄이려던 것이 안 줄어 있다.**
- ★`priority` 를 안 쓰면 → 다방 청취에서 ★**보고 있는 방과 안 보는 방이 대역을 똑같이 나눈다.**

<a id="LINK"></a>
