[← 차례](README.md)

## 4. ROOM — 방 참여

세션이 방 하나에 들어가 있는 것. 명단 · 권한 비트 · 방 통지 · version 정합의 주인

| 정체성 | 누가 내나 | 무엇마다 | 수명 | 상태 |
|---|---|---|---|---|
| (세션, room_id) | app | 세션 × 방 | ROOM_JOIN → ROOM_LEAVE · 세션 끝 | NONE · JOINING · JOINED · LEAVING |

| 의존 | 기댐 | 다중도 | 뜻 |
|---|---|---|---|
| ROOM | SES | N:1 | 참여는 세션의 것 — RESUMING 동안 그대로 두고 snapshot 으로 정리한다(§4-4-1) |
| ROOM | LINK | N:1 | 방은 미디어 서버 하나가 맡는다(sfu_id) — 입장 응답이 그 서버 연결의 재료를 준다(§9-1-1) |
| AFF | ROOM | 1:N | sub_rooms = 입장한 방들 · pub_room 은 그중 하나(§5-1-1) |
| PUB | ROOM | N:1 | 전이중 발행은 등록한 방의 것 — 방을 잃으면(퇴장 · 강퇴 · 방 닫힘) 서버가 그 사람의 그 방 발행을 정리한다(원칙 ③ · RULE-ROOM-SEQ-UP) |
| TRK | ROOM | N:1 | 트랙 목록은 참여한 방의 것 — 방 version.seq 로 정합한다(§4-1-6 · §4-2-6) |
| SUB | ROOM | N:1 | 구독은 참여한 방의 것 — 방을 떠나면 그 방의 구독을 버린다(sub.drop) |
| FLR | ROOM | 1:1 | 발언권은 방마다 — 참여자만 갖는다(§10-4-1) |

### 4-1. 구조

<a id="ROOM.type.MediaCodecs"></a>
#### 4-1-1. 형 `MediaCodecs` — 그 방의 미디어 코덱

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `audio` | string? | — (기본 "opus") | audio 코덱 이름(전이중 · 반이중 모두) — ★기본 `"opus"` · 방 생성 요청에서 비우면 기본값 · ★응답에는 늘 채워 싣는다 |
| `video` | string? | — (기본 "H264") | video 코덱 이름(전이중 · 반이중 모두) — ★기본 `"H264"` · 방 생성 요청에서 비우면 기본값 · ★응답에는 늘 채워 싣는다 · H264 는 constrained baseline(`42e01f`) 하나 · 구독 줄의 fmtp 는 고정 선언값이다(§4-1-1) |

<a id="RULE-ROOM-MEDIA-CODECS"></a>**RULE-ROOM-MEDIA-CODECS** · 규칙 — `MediaCodecs` — 그 방의 미디어 코덱

★★**그 방의 미디어 코덱은 방 속성이다 — 그 방의 발행 전부(전이중 · 반이중)가 이 코덱이다.** 방 생성(§4-1-13)의 `media_codecs` 로 정하고, 비운 칸은 기본값(audio `opus` · video `H264`)이다. ★**지원 밖 · 모르는 이름이면 방을 만들지 않는다** — `1005` · `details.supported`(발행의 코덱 검사와 같은 지원 코덱 표 — 말없이 기본값으로 바꾸면 앱이 고른 코덱과 다른 방이 선다). ★**바꾸는 기능은 없다** — 방이 사는 동안 그대로다.
- ★**왜 발행 전부인가** — 구독 m-line 은 모두 돌려쓴다. 무전 슬롯은 화자가 돌려쓰고, 전이중 줄은 상한을 넘으면 서버가 다른 트랙을 싣는다(재배정, §8-1-2). 재협상 없이 돌려쓰려면 한 m-line 에 오는 트랙들의 코덱 · PT · 클럭이 같아야 한다(갈래 (a)).
- ★**구독 줄의 fmtp 는 코덱마다 하나로 고정된 서버 선언값이다** — H264 = `level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f` · VP8 = 없음 · opus = `minptime=10;useinbandfec=1`(LiveKit 의 같은 변형 줄과 글자 그대로 같다). 한 m-line 에 오는 트랙끼리 프로파일이 다르면 디코딩이 깨질 수 있다(§8-3-1). ★H264 level 은 상한이 아니다 — `level-asymmetry-allowed=1` 이면 서버에서 구독자로 가는 방향의 level 은 구독자 브라우저가 받을 수 있는 최고 level 이다(RFC 6184 §8.2.2).
- ★★**발행 검사는 문자열이 아니라 파라미터로 견준다** — H264 는 `packetization-mode` 와 profile(`profile-level-id` 앞 네 자리)만 · ★level 과 그 밖의 칸(`level-asymmetry-allowed` 등)은 보지 않는다(mediasoup `isSameProfile` 과 같다) · opus · VP8 은 이름만. 다르면 `1006`(§7-1-2). ★문자열로 견주면 Chrome 발행이 전부 거절된다 — Chrome 의 fmtp 에는 `level-asymmetry-allowed=1` 이 붙는다.
- ★**profile 은 협상 산물이다** — 클라는 발행 m-line 에 방 profile 의 줄을 offer 에서 골라 적는다(PT · fmtp 는 offer 원문 그대로 — §9-4-10 코덱 고르기). 서버 고정값으로 PT 를 맞추지 않는다(서버 고정 `42e01f` 와 Chrome 이 고른 `42001f` 가 어긋나 정책 PT 가 남아 검은 화면 · 표 철거).
- 발행 방 옮김도 같은 검사 — `pub_select` 로 옮겨 갈 방과 다르면 같은 실패(§5-1-2).
- ★**무전 슬롯은 audio · video 둘 다 방이 생길 때부터 늘 있다**(§6-2-2) — 코덱이 처음부터 정해져 있어 값 없는 항목이 없다. 쓰고 안 쓰고는 화자 재량이다.
- 어디에 실리나: 방 생성 응답 · 방 상세(§4-1-14) · ★입장 응답(§4-1) — 발행하기 전에 읽고 맞춘다.

→ 구조 — [`ROOM.type.MediaCodecs`](#ROOM.type.MediaCodecs) · [`ROOM.http.CREATE_ROOM`](#ROOM.http.CREATE_ROOM)

<a id="ROOM.type.RoomSummary"></a>
#### 4-1-2. 형 `RoomSummary` — 방 목록의 한 줄

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ |  |
| `name` | str | ✔ |  |
| `capacity` | u32 | ✔ | 정원. 기본·최대 **1000** |
| `user_count` | u32 | ✔ | 현재 인원 (★`hidden` 제외) |
| `created_at` | u64 | ✔ | epoch ms |
| `rec` | bool | ✔ | 녹화 참가자(`participant_type:1`) 존재 여부 — ★`hidden` 이어도 참이다(녹화 사실은 감추지 않는다) |

<a id="ROOM.type.MemberInfo"></a>
#### 4-1-3. 형 `MemberInfo` — `MemberInfo`

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `user_id` | str | ✔ |  |
| `role` | u8 | — (기본 255) |  |
| `select` | bool | ✔ |  |
| `participant_type` | 0 / 1 / 2 | ✔ |  |
| `metadata` | json | — |  |
| `permission` | Permission | — |  |

<a id="RULE-ROOM-MEMBERINFO-SHAPE"></a>**RULE-ROOM-MEMBERINFO-SHAPE** · 규칙 — `MemberInfo`

`{ "user_id": "u1", "role": 255, "select": false, "participant_type": 0, "metadata": { … }, "permission": { … } }`
`role` 은 `u8`, 기본 `255`. `select` 는 bool. ★`participant_type` 은 `u8` — ★**서버가 토큰에서 채운다**(§3-1-5).
★`metadata` 는 **있을 때만**(토큰에 없으면 필드도 없다).
★`permission` 도 ★**기본과 다를 때만**(아래) — 없으면 ★**전부 허용**이다.

→ 구조 — [`ROOM.type.MemberInfo`](#ROOM.type.MemberInfo)

<a id="RULE-ROOM-PERMISSION-AXIS"></a>**RULE-ROOM-PERMISSION-AXIS** · 규칙

★★**`permission` 은 그 사람이 무엇을 할 수 있나다 — `role` 과 다른 축이다.**

`{ "publish_audio": true, "publish_video": true, "publish_screen": true, "floor_request": true }`

| 계약 | |
|---|---|
| 누가 정하나 | ★**클라가 아니다.** 토큰이 입장 초기값을 주고(§3-1-5), 그 뒤로는 ★**서버 밖의 결정자**가 ★**방 단위로** 바꾼다(§4-2-6). ★클라가 보내는 자리가 **없다** |
| ★**어느 방 것인가** | ★**방 것**이다 — 같은 사람이 방마다 다른 비트를 가진다. 갱신값은 그 방이 기억한다(§4-2-6) |
| 서버가 무엇에 쓰나 | ★**집행한다** — `role` 과 정반대다. 비트가 내려간 발행은 거절하고, 이미 올라온 것은 ★**회수한다.** 무엇을 막는지는 §4-2-6 사상표 |
| 기본 | 넷 다 `true`. ★**기본이면 명단에 안 싣는다** — 정원 1000 의 명단이 `RESUME` 스냅샷마다 통째로 나간다(§3-1-5 `metadata` 상한과 같은 이유) |
| 갱신 통지 | ★**있다** — `PARTICIPANT_STATE{permission}`(§4-2-6). `role`·`select` 와 갈리는 자리다 |

<a id="RULE-ROOM-SELECT"></a>**RULE-ROOM-SELECT** · 규칙

★★**`select` 가 청취/참여 구분이다.** 그 사람이 `ROOM_JOIN` 에 실어 보낸 `select`(§4-1)를
서버가 ★**그대로 에코**한 값이다 — `true` = 이 방을 발언 방으로 삼겠다는 **입장 의도**(참여),
`false` = 듣기만 하겠다는 입장 의도(청취).

| 계약 | |
|---|---|
| 뜻 | ★**입장 시점의 의도**다. 입장 후 발언 방을 옮겨도(`pub_select`, §5-1-2) 이 값은 안 바뀐다 — *"지금 누가 말하는가"* 는 이 필드가 아니라 ★**발언권 축(`FLOOR_TAKEN`, §10)** 이 준다. 두 축을 섞지 않는다 |
| 화면 | ★**표시할지는 앱이 정한다.** 규격은 사실만 나른다 — 지령대는 청취석을 가려 보이고, 단말 앱은 숨겨도 된다 |
| 갱신 통지 | 없다. 입장 시점 고정값이라 `PARTICIPANT_EVENT{joined}` 에 실려 오는 것으로 끝이다(§4-2-6) |

→ 구조 — [`ROOM.type.MemberInfo`](#ROOM.type.MemberInfo) · [`ROOM.op.ROOM_JOIN`](#ROOM.op.ROOM_JOIN)

<a id="RULE-ROOM-ROLE-LABEL"></a>**RULE-ROOM-ROLE-LABEL** · 규칙

★★**`role` 은 권한이 아니다. 앱이 붙이는 라벨이다.**

| | |
|---|---|
| 누가 정하나 | ★**클라가 `ROOM_JOIN`(§4-1) 에 실어 보낸다.** 서버는 그대로 저장한다 |
| 서버가 무엇에 쓰나 | ★**아무것도 안 한다.** 저장하고 명단·통지로 되돌려줄 뿐이다 — **크기를 비교하지도 않는다** |
| 뜻 | ★**앱이 정한다**(지휘관/대원, 상담원/고객 …). 이 문서는 뜻을 정하지 않는다 |

<a id="RULE-ROOM-ROLE-BLOCKS-NOTHING"></a>**RULE-ROOM-ROLE-BLOCKS-NOTHING** · 규칙

★★**이 값으로 무엇도 막지 않는다.** 클라가 스스로 선언하는 값이라 ★**누구나 아무 값이나 쓸 수 있다.**
권한 판정에 쓰면 ★**자기가 자기에게 권한을 주는 것**이 된다.

<a id="RULE-ROOM-SIGNED-AXES-APART"></a>**RULE-ROOM-SIGNED-AXES-APART** · 규칙

★**서명된 축과 섞지 않는다** — 참가자 종류(`participant_type`)·투명(`hidden`)·신원(`metadata`)은
★**앱 백엔드가 비밀키로 서명**하므로 클라가 못 바꾼다(§3-1-5). 이 `role` 은 그 축이 아니다 —
★**클라가 선언하는 방 배역 라벨** 하나뿐이고, 토큰에는 같은 이름의 필드가 ★**없다.**

<a id="RULE-ROOM-PARTICIPANT-TYPE"></a>**RULE-ROOM-PARTICIPANT-TYPE** · 규칙

★★**`participant_type` 은 서버가 채운다 — 클라가 선언하지 않는다.**

| | |
|---|---|
| 누가 정하나 | ★**토큰**(§3-1-5). ★`ROOM_JOIN` body 에 자리가 없다 — 요청에 실어도 무시한다(§4-1) |
| 값 | `0` 사람(기본) · `1` 녹화 · `2` 봇 |
| 서버가 무엇에 쓰나 | ★**명단에 싣는 것뿐**이다. 정원·통지를 가르는 것은 이 값이 아니라 ★**`hidden`**(아래) |
| 뜻 | 앱이 *"녹화 중"·"봇이 듣고 있다"* 를 화면에 보이게 하는 재료다 |

<a id="RULE-ROOM-HIDDEN"></a>**RULE-ROOM-HIDDEN** · 규칙

★★**`hidden` — 명단에서 빠지는 축.** ★**명단에 안 오르고**(그래서 `MemberInfo` 가 없다) ★**그 사람의**
입퇴장 통지가 남에게 안 돌고 정원도 안 먹는다(§4-1-12 `user_count`). ★★**방향이 한쪽이다** — 투명 참가자
★**자신은 남들의 통지를 그대로 받는다**(명단·`PARTICIPANT_EVENT`·`TRACK_EVENT` 전부). 안 그러면
★**녹화 봇이 누구를 녹화하는지 모른다.** 선례도 같다(LiveKit — hidden 에게도 배달, 남에게만 안 뿌린다). ★**토큰이 정하고**, 계정 목록에 허용된 계정만 켤 수 있다(§3-1-5).

<a id="RULE-ROOM-HIDDEN-ONLY-FOUR"></a>**RULE-ROOM-HIDDEN-ONLY-FOUR** · 규칙

★★**감춰지는 것은 그 넷뿐이다 — 방에는 남들과 똑같이 들어와 있다.** 투명 참가자도 ★**구독하고
받는다**(입장이 곧 구독이다, §4-4-1 — 안 그러면 투명 녹화가 아무것도 못 받는다). ★**발행할 수 있고
발언권도 쥘 수 있다.** 투명은 ★**명단 축**이지 미디어 축이 아니다.

| ★**남들에게** 감춰지는 것 | 감춰지지 않는 것 |
|---|---|
| ①`MemberInfo`(명단 응답 전량 — §4-1-14·§3-1·§4-1) ②`PARTICIPANT_EVENT`(§4-2-6) — ★**그 사람의 입퇴장이 남에게 안 간다는 뜻이다. 그 사람이 받는 남들의 통지는 그대로다** ③`user_count`(§4-1-12) ④★`PARTICIPANT_STATE`(§4-2-6 — 그 사람의 `permission` 변경은 ★**남에게** 안 간다: 명단 밖이라 방 공통 스냅샷이 안 바뀌고 `seq` 도 안 오른다. ★**본인에게는 `version` 없이 간다**(나에게만 — DEV-ROOM-HIDDEN-SELF-PSTATE) · 입장 · 재접속 때의 내 권한은 `me` 가 준다(DEV-ROOM-ME-APART). ★집행은 그대로다 — 감추는 것은 명단이지 자격이 아니다) | ★`rec`(녹화 사실 — §4-1-12) · ★비-무전 트랙의 `user_id`(§6-1-1) · ★`FLOOR_TAKEN` 의 화자(§10-4-30) |

<a id="RULE-ROOM-HIDDEN-SPEAKER-NAME"></a>**RULE-ROOM-HIDDEN-SPEAKER-NAME** · 규칙

★★**투명 화자의 이름은 클라에 없다.** `metadata` 는 명단 경로로만 오고(아래) 투명 참가자는 그 경로에서 빠지므로,
`FLOOR_TAKEN` 의 화자가 투명이면 클라가 쥔 것은 ★**`user_id` 뿐**이다. ★**이름을 얻을 경로를 만들지 않는다** —
감추기로 한 축이 명단이다. 화면에는 `user_id` 또는 앱이 정한 대체 표기가 뜬다(§10-4-35).

<a id="RULE-ROOM-HIDDEN-NO-SEQ"></a>**RULE-ROOM-HIDDEN-NO-SEQ** · 규칙

★★**그래서 입퇴장으로는 `seq` 가 안 오른다.** 명단에 안 오르므로 방 공통 스냅샷이 안 바뀐다 — ★**`seq++` ⟺ 스냅샷 변경**
(§4-2-6). 투명 입퇴장에 `seq` 를 올리면 방 전원이 갭을 보고 ★**헛 재동기를 돈다.** ★단 그 사람이 `individual` 트랙을 든 채
나가면 그 `remove` 는 트랙 층 변경이라 오른다(§4-2-6) — 감춰지는 것은 명단이지 트랙이 아니다.

<a id="RULE-ROOM-TYPE-HIDDEN-APART"></a>**RULE-ROOM-TYPE-HIDDEN-APART** · 규칙

★★**두 축을 묶지 않는다.** 녹화가 곧 투명이 아니다 — ★**보이는 녹화**(`1` + `hidden:false`)가 기본이고,
투명은 따로 켜는 결정이다. 선례도 같다(LiveKit `kind` ↔ `Video.Hidden` — 유도 관계가 없다).

<a id="RULE-ROOM-METADATA"></a>**RULE-ROOM-METADATA** · 규칙

★★**`metadata` 는 사람의 신원이다 — 앱 백엔드가 서명한다.**

| | |
|---|---|
| 누가 정하나 | ★**앱 백엔드가 `/auth/token`(§3-1-5)에 실어 서명한다.** 클라가 못 바꾼다 |
| 뜻 | ★**앱이 정한다** — 이름·프로필·소속·호출부호 …. 서버는 ★**해석하지 않는다**(불투명 JSON) |
| 서버가 하는 일 | 토큰에서 꺼내 세션에 보관하고, 그 사람이 든 방의 ★**명단·통지에 그대로 싣는다.** 저장·중계뿐이다 |
| 어디에 실리나 | ★**명단을 나르는 곳 전부** — `ROOM_JOIN` 응답(§4-1) · `GET /rooms/{id}`(§4-1-14) · `PARTICIPANT_EVENT{joined}`(§4-2-6) · `RESUME` 스냅샷(§3-1) |
| ★**갱신 통지** | ★**없다.** `select` 와 같은 규칙 — ★**토큰 발급 시점의 고정값**이다. 바뀌면 새 토큰으로 ★**새 세션을 연다**(옛 `session_id` 를 싣지 않는다) — ★`session_id` 로 이어받으면 서버가 ★**토큰을 아예 보지 않으므로**(§3-1 인증 두 갈래) 옛 값이 그대로다 |
| 상한 | ★**정책 값(토큰 `metadata` 상한), 기본 2,048 바이트.** 넘으면 ★**발급에서 거절**(`1002`, §3-1-5) |

<a id="RULE-ROOM-NAME-NOT-ROLE"></a>**RULE-ROOM-NAME-NOT-ROLE** · 규칙

★★**이름을 `role` 에 넣지 않는다.** 두 축은 다른 것이다 — ★**`metadata` = 그 사람이 누구인가**
(서명됨 · 세션 전역) ↔ ★**`role` = 이 방에서 무슨 배역인가**(클라 선언 · 방마다 다를 수 있다).
섞으면 ★**사칭이 라벨로 들어온다** — `role` 은 누구나 아무 값이나 쓸 수 있기 때문이다(위).

<a id="RULE-ROOM-LISTENERS-IN-ROSTER"></a>**RULE-ROOM-LISTENERS-IN-ROSTER** · 규칙

★**듣기만 하는 참가자도 명단에 들어간다.** 방에 들어가 있으면(§4-4-1 `JOINED`) 발행 트랙이 하나도
없어도 `participants` 에 나오고(`select:false` 로 구분될 뿐) `PARTICIPANT_EVENT` 도 뜬다.
★**빠지는 것은 `hidden` 참가자뿐**이다(§3-1-5). 사람도 녹화도 봇도 `hidden` 이 아니면 전원 오른다. 무전은 청취 전용이
절대다수라 이들이 안 보이면 ★**방이 비어 보인다** — ★**"이 방을 누가 듣고 있는가"가 명단의
존재 이유**이고, `SUBSCRIBED`(명단 밖 청취)를 기각한 근거다(§4-4-1).

→ 구조 — [`ROOM.S.join`](#ROOM.S.join)

<a id="PROP-ROOM-HIDDEN-NO-SEQ"></a>**PROP-ROOM-HIDDEN-NO-SEQ** · 성질

투명 참가자 자신의 입퇴장 · 강퇴 · 권한 변경으로는 그 방 seq 가 오르지 않는다.

*식: HD1 · 깨지면: 방 전원이 갭을 보고 헛 재동기를 돈다(RULE-ROOM-HIDDEN-NO-SEQ)*

<a id="PROP-ROOM-HIDDEN-HEARS"></a>**PROP-ROOM-HIDDEN-HEARS** · 성질

투명 참가자도 방에 든 동안 남들의 변경 통지를 받는다.

*식: HD2 · 깨지면: 투명 녹화가 누구를 녹화하는지 모른다(RULE-ROOM-HIDDEN)*

<a id="PROP-ROOM-HIDDEN-PSTATE-BARE"></a>**PROP-ROOM-HIDDEN-PSTATE-BARE** · 성질

투명 참가자에게 가는 PARTICIPANT_STATE 는 version 이 없는 본인 것뿐이다.

*식: HD3 · 깨지면: version 을 실으면 오르지 않은 seq 를 싣게 되어 견주기가 어긋난다(RULE-ROOM-HIDDEN-NO-SEQ)*

<a id="PROP-ROOM-HIDDEN-KNOWS-OWN"></a>**PROP-ROOM-HIDDEN-KNOWS-OWN** · 성질

투명 참가자도 방에 든 동안 자기 권한 비트가 바뀌면 그 통지를 받는다.

*식: HD5 · 깨지면: 자기 권한이 바뀐 것을 실패(2006 · DENY)로 부딪혀 보고서야 안다 — 찍어 보고 배우지 않는다(RULE-PUB-PUBLISH-PROC)*

<a id="PROP-ROOM-HIDDEN-NO-CAP"></a>**PROP-ROOM-HIDDEN-NO-CAP** · 성질

투명 참가자는 정원으로 막히지 않는다.

*식: HD4 · 깨지면: 정원이 찬 방에 투명 녹화 · 안내 봇이 못 들어간다 — 정원을 안 먹는다는 계약이 깨진다(RULE-ROOM-HIDDEN)*

<a id="DEV-ROOM-HIDDEN-SELF-PSTATE"></a>**DEV-ROOM-HIDDEN-SELF-PSTATE** · 자체 확장

투명 참가자의 권한 비트가 바뀌면 `PARTICIPANT_STATE{permission}` 을 ★**본인에게만 `version` 없이** 보낸다 — 남에게는 안 간다 · `seq` 는 오르지 않는다. 클라는 `version` 없는 델타를 견주지 않고 반영한다(보관 `seq` 그대로)

*원문: RULE-ROOM-HIDDEN-ONLY-FOUR ④ — 본인에게도 안 간다 · 본인은 2006 · DENY 로 안다 · 근거: CAND-ROOM-HIDDEN-SELF-PSTATE RULE-ROOM-VERSION-NOT-CARRIED*

<a id="DEV-ROOM-ME-APART"></a>**DEV-ROOM-ME-APART** · 자체 확장

입장 응답(`ROOM_JOIN`) · RESUME 의 방 스냅샷 · 입장 중인 사람의 `GET /rooms/{id}` 는 ★**나(`me` — `MemberInfo`)를 명단과 따로 싣는다** — 투명이든 아니든 늘. ★**내 권한의 출처는 `me` 다**(명단의 내 항목이 아니다). 그 뒤의 변경은 `PARTICIPANT_STATE` 가 준다(투명이면 `version` 없이 나에게만 — DEV-ROOM-HIDDEN-SELF-PSTATE)

*원문: RULE-ROOM-HIDDEN-ONLY-FOUR ④ — 본인은 2006 · DENY 로 안다(자기 권한을 받는 경로가 없다) · 근거: CAND-ROOM-ME-APART RULE-ROOM-BITS-OUTLIVE*

<a id="ROOM.type.Permission"></a>
#### 4-1-4. 형 `Permission` — 권한 비트

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `publish_audio` | bool | ✔ (기본 true) |  |
| `publish_video` | bool | ✔ (기본 true) |  |
| `publish_screen` | bool | ✔ (기본 true) |  |
| `floor_request` | bool | ✔ (기본 true) |  |

<a id="ROOM.type.RoomSnapshot"></a>
#### 4-1-5. 형 `RoomSnapshot` — 방 스냅샷

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `participants` | MemberInfo[] | ✔ | MemberInfo[] §4-1-3 |
| ★`me` | MemberInfo | ✔ | ★**나** — 명단과 따로 싣는다(투명이든 아니든 늘 · 권한 포함 — ★내 권한의 출처는 이것이다 · DEV-ROOM-ME-APART) |
| `tracks` | TrackEntry[] | ✔ |  |
| `version` | Version | ✔ |  |

<a id="ROOM.type.Version"></a>
#### 4-1-6. 형 `Version` — 무엇이 최신인가

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `epoch` | str | ✔ | ★**미디어 서버 프로세스 기동 단위 불투명 값.** 재기동하면 바뀐다. ★`server_config.sfu_id`(§9-1-1)와 **같은 값** |
| `seq` | u64 | ✔ | ★**방마다의 단조증가 일련번호.** ★**그 방 공통 스냅샷이 바뀔 때마다** 하나씩 오른다 |

<a id="EX-ROOM-VERSION"></a>**EX-ROOM-VERSION** · 예시(비규범) — ★`Version` — 무엇이 최신인가

→ 구조 — [`ROOM.example.EX-ROOM-VERSION`](#EX-ROOM-VERSION) · [`ROOM.type.Version`](#ROOM.type.Version)

예시 — 형 `Version`(형과 대조된 값 · 주석은 칸 설명)

```jsonc
{ "epoch": "sfu-7f3a…",   // ★**미디어 서버 프로세스 기동 단위 불투명 값.** 재기동하면 바뀐다. ★`server_config.sfu_id`(§9-1-1)와 **같은 값**
  "seq": 10245 }          // ★**방마다의 단조증가 일련번호.** ★**그 방 공통 스냅샷이 바뀔 때마다** 하나씩 오른다
```

<a id="PROP-ROOM-CONVERGE"></a>**PROP-ROOM-CONVERGE** · 성질

통로(WS · HTTP)가 비고 재동기가 없고 따라잡지 못한 유실이 없으면, JOINED 방의 보관본은 서버의 그 방 스냅샷과 같다.

*식: RH1 · 깨지면: 통지가 다 지나간 뒤에도 보관본이 낡아 있다 — 그 사이 올라온 트랙이 영영 안 붙는다(§4-2-3 증상)*

<a id="PROP-ROOM-NO-REWIND"></a>**PROP-ROOM-NO-REWIND** · 성질

JOINED 동안 보관본의 seq 는 그 입장 뒤 가장 큰 값 아래로 내려가지 않는다.

*식: RH2 · 깨지면: 낡은 응답이 최신 통지를 되감는다(§4-1-14 증상)*

<a id="ROOM.op.ROOM_JOIN"></a>
#### 4-1-7. `0x0201 ROOM_JOIN` — 방 입장   **C→S** · 응답 있음
단 1 · 출처 §4-1

**요청**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ | 들어갈 방 |
| `role` | u8 | — (기본 255) | 기본 `255`. ★**앱이 붙이는 라벨이고 권한이 아니다**(§4-1-3) — 서버는 저장만 한다 |
| `select` | bool | — (기본 true) | 기본 `true` = **이 방에서 말한다**. `false` = 듣기만. ★**명단에 그대로 에코된다**(`MemberInfo.select`, §4-1-3) — 청취/참여 구분이 이 값 하나다 |
| ★`extmap` | `{id, uri}[]` | ★`1pc` 필수 | ★**내 번호표 — 확장 ID 표.** 이 요청을 위해 새로 만든 연결의 브라우저 offer 가 선언한 확장 ID 전량(§9-2-1 2③). `2pc` 는 싣지 않는다(구독 연결에 내 발행 번호가 없다) — 실으면 `1002` |
| ★`codecs` | `{kind, pt, name, fmtp?, rtx_pt?}[]` | ★`1pc` 필수 | ★**내 번호표 — PT 표.** 같은 offer 의 코덱 줄 전량(내 발행 m-line 의 PT 표 — `fmtp` 는 offer 것이고 PT 튜플 키의 구성 파라미터를 뽑는 재료다, §9-4-2 ①). 서버는 이 표를 피해 구독 PT 를 배정한다(§9-4-2 ④) |

`codecs` 안:

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `codecs.kind` | "audio" / "video" | ✔ | 그 코덱 줄이 있는 m-line 의 media type — 튜플의 첫 칸이다(§9-4-2 ①). 이름만으로는 갈리지 않는다(`red` 는 audio · video 둘 다 있다 · CAND-ROOM-JOIN-CODEC-KIND) |
| `codecs.pt` | u8 | ✔ |  |
| `codecs.name` | str | ✔ |  |
| `codecs.fmtp` | str | — |  |
| `codecs.rtx_pt` | u8 | — |  |

**응답**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ |  |
| `participants` | MemberInfo[] | ✔ |  |
| ★`me` | MemberInfo | ✔ | ★**나** — 명단과 따로 싣는다(투명이든 아니든 늘 · 권한 포함 — ★내 권한의 출처는 이것이다 · DEV-ROOM-ME-APART) |
| `affiliation` | Affiliation | ✔ | pub_room 없으면 null |
| `server_config` | ServerConfig | ✔ |  |
| `tracks` | TrackEntry[] | ✔ | §6-1-1 — ★보기용 목록 |
| `version` | Version | ✔ | ★§4-1-6 |
| ★`media_codecs` | MediaCodecs | ✔ | ★**그 방의 미디어 코덱**(§4-1-1) — 발행하기 전에 읽고 맞춘다(전이중 · 반이중 모두 — §7-1-2) |
| ★`mline_table` | MlineTable | ✔ | ★**그 서버 연결의 구독 m-line 배정표 전체**(§8-1-2) — 이 방의 트랙이 실린 뒤의 표다. 받는 법은 `MLINE_EVENT` 와 같다(§8-1-2) |

**실패** — `3001` 방 없음 · `4001` 정원 초과 · `4009` ★자체 확장 — §4-1 실패 줄에 없다. §11-2-3 · §4-2-2 6 이 ROOM_JOIN 실패로 정한다(DEV-ROOM-JOIN-FAIL-4009) · `4004` 동시 입장 방 수 초과 · `4005` 구독 m-section 몫 초과 · `1002` body 오류(★`2pc` 에 번호표 · `1003` `1pc` 에 번호표 없음

<a id="RULE-ROOM-LISTEN-ONLY-NORMAL"></a>**RULE-ROOM-LISTEN-ONLY-NORMAL** · 규칙

★**청취 전용 입장(`select:false`)이 다방 모니터링의 정상 경로다**(§4-4-1). 특례가 없다 —
★**명단에 오르고, 정원(`4001`)을 먹고, 입퇴장 통지가 돈다.** 지령대 10~20 채널 규모에서
이 비용은 계산할 것이 없다.

<a id="RULE-ROOM-JOIN-NO-PC-MODE"></a>**RULE-ROOM-JOIN-NO-PC-MODE** · 규칙

★**`pc_mode` 는 여기 없다.** ★**세션 단위**라 `BIND`(§3-1)가 나른다 — 클라는 요청 시점에
★**그 방이 어느 미디어 서버인지 모르므로**(§9-1-1) 방마다 정할 방법이 원리적으로 없다.

<a id="RULE-ROOM-JOIN-NUMBER-TAGS"></a>**RULE-ROOM-JOIN-NUMBER-TAGS** · 규칙

★★**번호표(`extmap`·`codecs`)는 트랙 발행이 아니다.** 트랙 발행(`PUBLISH_TRACKS` — *"이 트랙을 보내겠다"*)과 달리 여기 싣는 것은
★**내 브라우저를 통해 습득한 번호 정보**(*"내 브라우저가 이 번호를 쓴다"*)다. 트랙이 없어도(청취 전용이어도) 성립한다.
★**요청에 실을 수 있는 이유** — 브라우저 PT·확장 ID 는 `RTCPeerConnection` 생성 → DC + audio 1 · video 1 `inactive` 트랜시버 →
`createOffer` → `setLocalDescription` 시점에 정해지고, 거기까지는 ★**서버 재료가 필요 없다.** 서버 재료(`server_config`)가 필요한 것은
answer 조립뿐이다. 첫 협상 확정본(answer)의 번호는 그 offer 의 부분집합이므로 offer 표를 신고하면 충분하다 — 서버가 피할 번호가
조금 많아질 뿐 안전하다(opus `fmtp` 의 받는 쪽 선호는 answer 에서 바뀌지만 PT 키가 아니다, §9-4-2 ①). ★**확정본 뒤에 신고하는 갈래(`READY{transport}`)를 두지 않는다** — 첫 입장이 한 바퀴 헛돌고 첫 응답의 배정을 되돌리는 재배정 `TRACK_EVENT` 예외가 생긴다.
★**걸리는 것 하나 — 요청 시점에 그 방이 어느 서버인지 모른다**(§9-1-1). 연결은 서버마다 하나이므로 ★**`ROOM_JOIN` 마다 새로 만든
연결의 표를 싣고, 응답의 `sfu_id` 가 이미 붙은 서버면 새 연결을 버린다**(그 서버는 기존 연결의 표를 이미 안다 — `setLocalDescription`
까지만 한 연결이라 비용이 작다). ★**기각한 대안** — 표 하나를 `BIND` 에 싣는 안은 *"브라우저가 연결마다 같은 PT 를 붙인다"* 에 기대는데
표준이 보장하지 않는다(추정)라 기대지 않는다. `setLocalDescription(offer)` 뒤 원격 answer 가 `ROOM_JOIN` 왕복만큼 늦어도 문제없는지는
정상 offer/answer 지연 범위라 괜찮다고 보되 실측으로 확인한다.

★**코덱 줄은 media type(`kind`)을 함께 싣는다** — PT 튜플의 첫 칸이고(§9-4-2 ①) 이름만으로는 갈리지 않는다(`red` 는 audio · video 둘 다 있다 · CAND-ROOM-JOIN-CODEC-KIND).

<a id="EX-ROOM-JOIN-RES"></a>**EX-ROOM-JOIN-RES** · 예시(비규범)

→ 구조 — [`ROOM.example.EX-ROOM-JOIN-RES`](#EX-ROOM-JOIN-RES) · [`ROOM.op.ROOM_JOIN`](#ROOM.op.ROOM_JOIN) · [`LINK.type.ServerConfig`](09-link.md#LINK.type.ServerConfig) · [`LINK.type.CodecPolicy`](09-link.md#LINK.type.CodecPolicy)

예시 — `ROOM_JOIN` 응답(형과 대조된 값 · 주석은 칸 설명)

```jsonc
{ "room_id": "…",
  "participants": [ {"user_id":"u1","role":255,"select":false,"participant_type":0,"metadata":{"name":"…"}} ],
  "me": {"user_id":"u1","role":255,"select":false,"participant_type":0,"metadata":{"name":"…"}},   // ★**나** — 명단과 따로 싣는다(투명이든 아니든 늘 · 권한 포함 — ★내 권한의 출처는 이것이다 · DEV-ROOM-ME-APART)
  "affiliation": { "sub_rooms": [ "r1" ],               // 내가 듣는 방들 = ★그 서버에 입장한 방들 (입장이 곧 구독, §4-1)
                   "pub_room": "r1" },                  // ★내가 발행하는 방. **하나뿐**이라 단수다. 없으면 null · pub_room 없으면 null
  "server_config": { "ice": {"ip":"…","port":7000,"publish_ufrag":"…","publish_pwd":"…","subscribe_ufrag":"…","subscribe_pwd":"…"},
                     "dtls": {"fingerprint":"sha-256 AB:CD:…","setup":"passive"},
                     "codecs": [ {"kind":"audio","name":"opus","rtcp_fb":["transport-cc"]}, {"kind":"video","name":"H264","rtcp_fb":["nack","nack pli"]} ],   // 발행에 쓸 코덱. ★**PT·클럭·fmtp 는 없다**
                     "codecs_sub": [ /* CodecPolicy… §9-1-2 */ ],   // 있을 때만. **구독 전용 정책**
                     "extmap": [ {"id":1,"uri":"…"} ],   // 확장 헤더 번호
                     "max_bitrate_bps": 800000,         // 발행 상한
                     "sfu_id": "sfu-7f3a…",             // ★**그 미디어 서버의 신원**(불투명). 프로세스 기동마다 새로 발급된다. ★**뜻을 해석하지 않고 같은지만 본다.** `version.epoch`(§4-1-6)와 같은 값
                     "pc_mode": "1pc" },                // ★**그 서버가 적용한 값**(`"1pc"`/`"2pc"`). 세션 확정값(§3-1 `BIND` 응답)과 ★**반드시 같다** — 다르면 규격 위반이고 클라는 **연결을 세우지 않는다**
  "tracks": [ /* TrackEntry… §6-1-1 */ ],   // §6-1-1 — ★보기용 목록
  "version": { "epoch": "sfu-7f3a…",                    // ★**미디어 서버 프로세스 기동 단위 불투명 값.** 재기동하면 바뀐다. ★`server_config.sfu_id`(§9-1-1)와 **같은 값**
               "seq": 10245 },                          // ★**방마다의 단조증가 일련번호.** ★**그 방 공통 스냅샷이 바뀔 때마다** 하나씩 오른다 · ★§4-1-6
  "media_codecs": { "audio": "opus",                    // audio 코덱 이름(전이중 · 반이중 모두) — ★기본 `"opus"` · 방 생성 요청에서 비우면 기본값 · ★응답에는 늘 채워 싣는다
                    "video": "H264" },                  // video 코덱 이름(전이중 · 반이중 모두) — ★기본 `"H264"` · 방 생성 요청에서 비우면 기본값 · ★응답에는 늘 채워 싣는다 · H264 는 constrained baseline(`42e01f`) 하나 · 구독 줄의 fmtp 는 고정 선언값이다(§4-1-1) · ★**그 방의 미디어 코덱**(§4-1-1) — 발행하기 전에 읽고 맞춘다(전이중 · 반이중 모두 — §7-1-2)
  "mline_table": { "sfu_id": "sfu-7f3a…",               // ★**어느 미디어 서버의 연결 표인가**(§9-1-1 — 재기동하면 바뀐다)
                   "arev": 3,                           // ★**표의 차례 번호** — 그 서버가 이 세션의 표를 바꿀 때마다 하나 오른다(재구축해도 이어진다 · 재기동하면 `sfu_id` 가 바뀐다). ★**같은 `sfu_id` 에서 보관값보다 큰 것만 받는다** — 표 전체라 낡은 것은 버리면 된다(사이를 메울 것이 없다)
                   "mlines": [ /* MlineEntry… §8-1-1 */ ] } }   // ★**표 전체** — 그 연결의 구독 m-line 전부(빈 줄 포함 · mid 순서는 받는 쪽이 수치로 세운다) · ★**그 서버 연결의 구독 m-line 배정표 전체**(§8-1-2) — 이 방의 트랙이 실린 뒤의 표다. 받는 법은 `MLINE_EVENT` 와 같다(§8-1-2)
```

<a id="RULE-ROOM-JOIN-PROC"></a>**RULE-ROOM-JOIN-PROC** · 규칙

**절차**
- 클라 ★`server_config` **로만** 로컬 SDP 를 조립한다. 그 전에 미디어를 보내지 않는다.
- 클라 ★`codecs` 에 **PT·클럭·fmtp 가 없다.** 그 셋은 협상 산물이라 클라가 자기 offer 값을 쓴다.
  서버가 정하는 것은 **코덱 이름과 받고 싶은 `rtcp_fb`** 뿐이다.
- 클라 ★`codecs_sub` 가 오면 **구독 m-line 에는 그것을 쓴다**(발행은 `codecs`).
- ★**발언권 상태는 여기 오지 않는다.** 이 시점엔 미디어도 DC 도 아직 없어서 **쓸 수가 없고**,
  DC 가 설 때까지 흐르는 동안 ★**이미 낡는다.** 발언권은 ★**DC 가 서면 서버가 보낸다**(§10-4-30).
  ★**권위는 DC 단일이다**(§10).
- 클라 `ice` 의 네 자격은 ★**이 세션 것**이다. 재협상 때 **같은 값을 계속 쓴다**(§9-4-9) — ★`ICE_RESTART` 응답(§7-1-2)이 새 값을 주면 그때부터 그 값이다.
- 서버 ★**이미 그 방에 있으면 그 서버의 옛 Peer 를 통째로 축출하고 새 Peer 로 들어온다**(실패로 오지 않는다). Peer 는 서버당 하나라(§9-1-1) ★**그 서버의 다른 방 멤버십·구독·발행·DC 도 함께 사라진다** — 응답 `affiliation.sub_rooms` 가 진실이다.
- 클라 응답의 `server_config.ice` 자격이 보관값과 다르면 그 서버 연결을 새로 세운다. ★**응답 `sub_rooms` 에 없는 그 서버의 방은 `NONE` 으로 내리고 다시 입장한다**(§9-2-1 이 이 경로를 쓴다).

<a id="RULE-ROOM-JOIN-PC-MODE-CHECK"></a>**RULE-ROOM-JOIN-PC-MODE-CHECK** · 규칙

- 클라 ★**응답의 `server_config.pc_mode` 가 세션 확정값(§3-1)과 같은지 본다.** 다르면
  ★**연결을 세우지 않고 실패로 다룬다** — 서버가 조용히 다른 모드로 돌리는 경로는 없다. ★서버가 이미 받아들인 입장은 `ROOM_LEAVE` 로 되돌린다(DEV-ROOM-PC-MISMATCH-UNDO).
- 클라 ★**`version` 을 보관한다** — ★**이 응답은 스냅샷이라 견주지 않는다**(§4-2-3 규칙 2). 이후 그 방 **델타 통지**는 이 값과 견주어 받아들인다.
- ★`1pc` — 응답의 `tracks[].pt` 는 요청의 번호표를 피해 배정된 값이다(§9-4-2 ④). ★그 서버에 이 세션의 연결이 이미 있으면 서버는 요청의 번호표를 버리고 ★**기존 연결의 표**를 피해 배정한다 — 클라가 버릴 연결의 표이기 때문이다. 응답의 `server_config` 로 answer 를 조립해 첫 협상을 닫고(§9-2-1), 그 다음 구독 m-line 을 통합 offer 로 조립한다.

<a id="ROOM.op.ROOM_LEAVE"></a>
#### 4-1-8. `0x0202 ROOM_LEAVE` — 방 퇴장   **C→S** · 응답 있음
단 1 · 출처 §4-1

**요청**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ |  |

**응답**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ |  |
| `affiliation` | Affiliation | ✔ |  |

**실패** — `3002` 그 방에 없음

<a id="RULE-ROOM-LEAVE-FORM"></a>**RULE-ROOM-LEAVE-FORM** · 규칙

**형식** — 요청 `{ room_id }` / 응답 `{ room_id, affiliation:{sub_rooms, pub_room} }`
(★**`version` 은 없다** — 종결이라 재설정할 스냅샷이 없고, 소속은 방 공통 스냅샷 밖의 값이다, §5-1-1·§4-2-3)

→ 구조 — [`ROOM.op.ROOM_LEAVE`](#ROOM.op.ROOM_LEAVE)

<a id="RULE-ROOM-LEAVE-PROC"></a>**RULE-ROOM-LEAVE-PROC** · 규칙

**절차** — 서버: **그 방만** 뺀다(다른 방은 유지). 그 방이 발행 방이었으면 `pub_room` 이 `null` 이 된다.
★**남을 내보내는 필드는 없다.** 강제 퇴장은 운영 경로이고 이 문서 밖이다.

→ 구조 — [`ROOM.S.leave`](#ROOM.S.leave) · [`AFF.C.any.left`](05-aff.md#AFF.C.any.left)

<a id="RULE-ROOM-LEAVE-FAIL"></a>**RULE-ROOM-LEAVE-FAIL** · 규칙

→ 구조 — [`ROOM.op.ROOM_LEAVE`](#ROOM.op.ROOM_LEAVE)

<a id="ROOM.op.PARTICIPANT_EVENT"></a>
#### 4-1-9. `0x0701 PARTICIPANT_EVENT` — 누가 들고났다   **S→C** · ACK
단 1 · 출처 §4-2-6

**통지 · 받는 꼴**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `type` | "joined" / "left" | ✔ |  |
| `room_id` | str | ✔ |  |
| `user_id` | str | ✔ |  |
| `role` | u8 | joined |  |
| `select` | bool | joined |  |
| `participant_type` | 0 / 1 / 2 | joined |  |
| `metadata` | json | joined |  |
| `permission` | Permission | joined |  |
| `version` | Version | ✔ |  |

<a id="RULE-ROOM-PARTICIPANT-EVENT-FORM"></a>**RULE-ROOM-PARTICIPANT-EVENT-FORM** · 규칙

**형식** — `{ type:"joined"|"left", room_id, user_id, role?, select?, participant_type?, metadata?, permission?, version }`
(`role`·`select`·`participant_type`·`metadata`·`permission` 은 `joined` 만 — ★`permission` 은 기본과 다를 때만(§4-1-3). ★**재입장자는 그 방이 기억하는 갱신값을 이고 들어온다**(§4-2-6) — 여기 안 실으면 남들 명단에 넷 다 허용으로 오르고 `PARTICIPANT_STATE` 는 바뀔 때만 와서 고쳐질 계기가 없다 — ★`hidden` 참가자는 애초에 이 통지가 돌지 않는다 — ★`select` 는 그 사람의 청취/참여 구분이고, ★`metadata` 는 토큰이 서명한 신원이다, §4-1-3. ★**`updated` 형은 없다** — 갱신 축이 없다)
**절차** — ACK + `version` 견주기 + 참가자 목록 갱신.
★**증상** — *방이 생겼다/없어졌다* 는 **클라에게 오지 않는다.** 그건 서버끼리 쓰는 것이다.

→ 구조 — [`ROOM.op.PARTICIPANT_EVENT`](#ROOM.op.PARTICIPANT_EVENT)

<a id="ROOM.op.PARTICIPANT_STATE"></a>
#### 4-1-10. `0x0705 PARTICIPANT_STATE` — 참가자 **속성**이 바뀌었다   **S→C** · ACK
단 1 · 출처 §4-2-6

**통지 · 받는 꼴**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `type` | "permission" | ✔ |  |
| `user_id` | str | ✔ |  |
| `room_id` | str | ✔ |  |
| `version` | Version | ★투명 참가자 본인에게 가는 자기 권한 통지에는 없다 — 명단 밖이라 seq 가 안 오른다 · 나에게만 온다(DEV-ROOM-HIDDEN-SELF-PSTATE) |  |
| `permission` | Permission | ✔ |  |

<a id="RULE-ROOM-PSTATE-DISPLAY-ONLY"></a>**RULE-ROOM-PSTATE-DISPLAY-ONLY** · 규칙

★**`_STATE` 는 표시만 고치는 자리다**(각 장의 구조 이름 규율). ★**여기서 트랙이 사라지지 않는다** — 회수는 `TRACK_EVENT{remove}`(§4-2-6)와 발언권 회수(§10)가 ★**따로 온다.**

<a id="EX-ROOM-PSTATE"></a>**EX-ROOM-PSTATE** · 예시(비규범)

→ 구조 — [`ROOM.example.EX-ROOM-PSTATE`](#EX-ROOM-PSTATE) · [`ROOM.op.PARTICIPANT_STATE`](#ROOM.op.PARTICIPANT_STATE)

예시 — `PARTICIPANT_STATE` 통지(형과 대조된 값 · 주석은 칸 설명)

```jsonc
{ "type": "permission",
  "user_id": "u1",
  "room_id": "r1",
  "version": {"epoch":"…","seq":10249},   // ★투명 참가자 본인에게 가는 자기 권한 통지에는 없다 — 명단 밖이라 seq 가 안 오른다 · 나에게만 온다(DEV-ROOM-HIDDEN-SELF-PSTATE)
  "permission": {"publish_audio":true,"publish_video":false,"publish_screen":false,"floor_request":true} }
```

<a id="RULE-ROOM-PSTATE-TYPES"></a>**RULE-ROOM-PSTATE-TYPES** · 규칙

| `type` | 언제 | 클라가 할 일 |
|---|---|---|
| `permission` | 그 사람이 할 수 있는 것이 바뀌었다(§4-1-3) | 표시 갱신 — ★**버튼을 잠그고 푼다.** 본인이면 자동 발행을 멈춘다 |

→ 구조 — [`ROOM.op.PARTICIPANT_STATE`](#ROOM.op.PARTICIPANT_STATE)

<a id="RULE-ROOM-PSTATE-CONTRACT"></a>**RULE-ROOM-PSTATE-CONTRACT** · 규칙

| 항목 | 규격 |
|---|---|
| 방향 | S→C. ★★**그 방 전원이다** — 본인만이 아니다. `permission` 은 명단에 실리는 값(§4-1-3)이라 ★**전원의 보관본이 바뀐다**(`muted` 와 같은 축, §4-2-6) |
| `version` | ★**있다** — 명단이 바뀌었으므로 방 공통 스냅샷의 값이다(§4-1-6) |
| 클라 의무 | ACK + `version` 견주기 + 보관본 갱신 + 표시 |
| ★**본인일 때 더 할 일** | ★**내려간 비트의 자동 발행을 멈춘다.** 안 멈추면 다음 발행이 거절된다 — 거절을 보고 멈추는 것보다 ★**통지를 보고 멈추는 것이 빠르다** |
| ★**싣는 값** | ★**전량이다**(넷 다). 델타로 보내지 않는다 — 넷뿐이라 전량이 더 싸고, ★**"안 온 비트는 그대로"** 라는 규칙을 안 만든다 |
| ★**어느 방의 값인가** | ★**`room_id` 의 것**이다 — 비트는 방 것(§4-2-6). 같은 사람의 다른 방은 안 바뀐다 |
| ★`hidden` 참가자 | ★**남에게 안 간다**(§4-1-3 감춰지는 넷). 명단 밖이라 방 공통 스냅샷이 안 바뀌고 `seq` 도 안 오른다. ★**본인에게는 `version` 없이 간다**(나에게만 — DEV-ROOM-HIDDEN-SELF-PSTATE) · 입장 · 재접속 때의 내 권한은 `me` 가 준다(DEV-ROOM-ME-APART) |

→ 구조 — [`ROOM.S.perm_pub_down`](#ROOM.S.perm_pub_down) · [`ROOM.S.perm_floor_down`](#ROOM.S.perm_floor_down) · [`ROOM.S.perm_pub_down_hidden`](#ROOM.S.perm_pub_down_hidden)

<a id="RULE-ROOM-PSTATE-ORDER"></a>**RULE-ROOM-PSTATE-ORDER** · 규칙

★★**순서가 계약이다 — 서버는 적용하고 나서 알린다.** 통지가 먼저 가면 클라가 그것을 보고 발행을 거는
★**그 순간에 서버 비트가 아직 안 내려가 있어** 통과해 버린다(또는 반대로 거절된다). ★**알림은 이미 끝난
일의 보고**이지 예고가 아니다.

<a id="ROOM.op.ROOM_EVENT"></a>
#### 4-1-11. `0x0704 ROOM_EVENT` — 나와 방의 관계·정합성   **S→C** · ACK · ★**신설** · unicast
단 1 · 출처 §4-2-6

**통지 · 받는 꼴**

`type` = `affiliation`

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ | ★이 변화를 일으킨 방 |
| `affiliation` | Affiliation | ✔ | §5-1-1 |
| `cause` | "kick" / "room_closed" | ✔ | kick · room_closed |

`type` = `sync_required`

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ | ★이 변화를 일으킨 방 |
| `reason` | str | ✔ | ★현재 정의된 값은 이것 하나. 로그·사람용 — 클라 처리는 type 으로만 가른다(값 추가는 additive) |

<a id="EX-ROOM-ROOM-EVENT"></a>**EX-ROOM-ROOM-EVENT** · 예시(비규범)

→ 구조 — [`ROOM.example.EX-ROOM-ROOM-EVENT`](#EX-ROOM-ROOM-EVENT) · [`ROOM.op.ROOM_EVENT`](#ROOM.op.ROOM_EVENT)

예시 — `ROOM_EVENT` 통지(형과 대조된 값 · 주석은 칸 설명)

```jsonc
// `type:"affiliation"` 일 때
{ "type": "affiliation",
  "room_id": "r1",                                      // ★이 변화를 일으킨 방
  "affiliation": {"sub_rooms":["r2"],"pub_room":null},   // §5-1-1
  "cause": "kick" }                                     // kick · room_closed
// `type:"sync_required"` 일 때
{ "type": "sync_required",
  "room_id": "r1",              // ★이 변화를 일으킨 방
  "reason": "no_media_flow" }   // ★현재 정의된 값은 이것 하나. 로그·사람용 — 클라 처리는 type 으로만 가른다(값 추가는 additive)
```

<a id="RULE-ROOM-ROOM-EVENT-CONTRACT"></a>**RULE-ROOM-ROOM-EVENT-CONTRACT** · 규칙

| 항목 | 규격 |
|---|---|
| 방향 | S→C. ★**나에게만 unicast** — ★**방 broadcast 가 아니다** |
| ★`version` | ★**없다**(§4-2-3) — `affiliation` 은 방 공통 스냅샷 밖의 값이고 `sync_required` 는 payload 가 없다. 견줄 것이 없다 |
| 클라 의무 | ACK + ★**견주지 않고 반영**(§4-2-3 — 종결이면 곧바로 방을 내리고, 아니면 부분 목록으로 갱신) + 앱 통보 |

→ 구조 — [`ROOM.C.JOINED.kicked`](#ROOM.C.JOINED.kicked) · [`ROOM.C.JOINED.sync_required`](#ROOM.C.JOINED.sync_required)

<a id="RULE-ROOM-ROOM-EVENT-TYPES"></a>**RULE-ROOM-ROOM-EVENT-TYPES** · 규칙

| `type` | 언제 | 클라가 할 일 |
|---|---|---|
| ★`affiliation` | ★**서버가 소속을 강제로 바꿨다** — ★**방에서 축출**(`cause:"kick"`) · 방이 닫힘(`cause:"room_closed"` — ★운영의 방 닫힘이다. 미디어 서버 재기동은 LINK 길(`3010` · `2008`)이 정한다, R-ROOM-CLOSED-OPS-ONLY). ★미디어 연결이 죽은 것은 소속 변화가 아니다 — 서버는 미디어 침묵으로 방에서 빼지 않는다(성립 조건 색인 — Peer 수명 = 세션 수명). 그 연결 상태는 `LINK_EVENT`(아래)다 | ACK + 보관본 갱신 + 앱 통보 |
| ★`sync_required` | ★**서버가 어긋남을 감지했다**(미디어가 멎었다 등) | ACK + ★**`GET /rooms/{room_id}?tracks=1`(§4-1-14)** 로 통짜로 다시 받는다. 같은 방 재통보는 **30초 쿨다운**(§4-2-3) |

→ 구조 — [`ROOM.op.ROOM_EVENT`](#ROOM.op.ROOM_EVENT)

<a id="RULE-ROOM-ROOM-EVENT-PARTIAL"></a>**RULE-ROOM-ROOM-EVENT-PARTIAL** · 규칙

★★**`affiliation` 의 목록은 부분 목록이다.** §5-1-1 과 같다 — ★**그 미디어 서버가 아는 만큼**이고
★**전체로 덮어쓰지 않는다.** 덮어쓰면 다른 서버의 듣는 방이 사라진다.

→ 구조 — [`ROOM.C.JOINED.kicked`](#ROOM.C.JOINED.kicked)

<a id="ROOM.http.LIST_ROOMS"></a>
#### 4-1-12. `GET /rooms` — 방 목록
누가: 클라 · 인증: bearer · session · 성공 `200` · CORS 붙는다

**응답**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `rooms` | RoomSummary[] | ✔ |  |
| `total` | u32 | ✔ |  |

**실패** — (401) `401`

<a id="RULE-ROOM-LIST-FORM"></a>**RULE-ROOM-LIST-FORM** · 규칙 — `GET /rooms` — 방 목록

**형식** — 질의 없음.

→ 구조 — [`ROOM.http.LIST_ROOMS`](#ROOM.http.LIST_ROOMS)

<a id="RULE-ROOM-LIST-NO-VAD"></a>**RULE-ROOM-LIST-NO-VAD** · 규칙

★**발성 감지 플래그는 여기 없다.** 확장이라 이 문서 밖이다(§1-2-1·§10-4-34).

<a id="RULE-ROOM-LIST-PROC"></a>**RULE-ROOM-LIST-PROC** · 규칙

**절차** — 목록엔 **참가자 이름이 없다.** 누가 있는지는 §4-1-14(방 상세, `participants`).

<a id="RULE-ROOM-LIST-FAIL"></a>**RULE-ROOM-LIST-FAIL** · 규칙

→ 구조 — [`ROOM.http.LIST_ROOMS`](#ROOM.http.LIST_ROOMS)

<a id="ROOM.http.CREATE_ROOM"></a>
#### 4-1-13. `POST /rooms` — 방 생성
누가: 클라 · ★POST 는 앱 백엔드도(방 재생성, §4-2-6 `room_closed`) · 인증: bearer · session · 성공 `201` · CORS 붙는다

**요청**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | string? | — | 없거나 빈 문자열이면 서버가 uuid 발급. ★**UTF-8 255바이트 이하** — DC TLV `0x1D` 의 `len` 1바이트가 전 규격의 상한이다(§10-4-29). ★**`/` `*` `$` `?` `#` 를 쓰지 않는다** — 서버가 방 이름을 서버끼리 방 위치를 찾는 키의 조각으로 쓰고, 이 다섯은 그 키의 구분자·와일드카드다. ★**이 형식은 `room_id` 를 싣는 모든 요청에 같다**(`ROOM_JOIN`·`GET /rooms/{room_id}` 등) — 벗어나면 `1002` |
| `name` | str | ✔ |  |
| `capacity` | int? | — (기본 1000) | 기본·최대 1000. ★**초과 요청은 `400`(`1002`) 으로 거절한다** — 조용히 자르지 않는다 |
| `unused_ttl_secs` | int? | — | 아무도 안 들어온 채로 유지할 초. 없으면 ★서버 설정을 따른다(상용 기본 = 영구 · 시험 형상은 짧게) |
| `departure_ttl_secs` | int? | — | 마지막 참가자가 나간 뒤 유지할 초. 없으면 ★서버 설정의 기본값(위와 같다). `0` = 즉시 |
| ★`media_codecs` | MediaCodecs? | — | ★**그 방의 미디어 코덱**(§4-1-1 — 그 방의 발행 전부) — 없거나 빈 값인 칸은 기본값(audio `opus` · video `H264`). ★**방이 사는 동안 바뀌지 않는다**(바꾸는 기능이 없다) |

**응답**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ |  |
| `name` | str | ✔ |  |
| `capacity` | u32 | ✔ |  |
| `created_at` | u64 | ✔ |  |
| ★`media_codecs` | MediaCodecs | ✔ | ★정해진 값(기본값을 채운 뒤) |

**실패** — `1003` `1003 MISSING_FIELD` 이름 필요 · `1002`(400) `capacity` 초과·`room_id` 255바이트 초과·★`room_id` 금지 문자(`/` `*` `$` `?` `#`) · `4003` `QUOTA_EXCEEDED` ★**이 배포**의 방 생성 수 상한 · `1005` ★`media_codecs` 가 지원 밖 · 모르는 이름 — 발행의 코덱 검사와 같은 지원 코덱 표 · `details.supported` 로 알린다(말없이 기본값으로 바꾸지 않는다). HTTP 실패 body 는 전부 §11-1-1 `Failure` 형이다

<a id="RULE-ROOM-CREATE-FORM"></a>**RULE-ROOM-CREATE-FORM** · 규칙 — `POST /rooms` — 방 생성

→ 구조 — [`ROOM.http.CREATE_ROOM`](#ROOM.http.CREATE_ROOM)

<a id="RULE-ROOM-CREATE-PROC"></a>**RULE-ROOM-CREATE-PROC** · 규칙

**절차** — ★**같은 `room_id` 로 다시 부르면 기존 방을 돌려준다**(멱등).
★★**방에 주인을 두지 않는다** — ★**한 배포는 한 고객**이라 방이 전부 그 고객의 것이고, 가를 축이 없다. 방을 계정에 묶으면 ★**있지도 않은 경계를 지키느라 키 교체가 방 주인을 잃게 만든다.**

<a id="RULE-ROOM-CREATE-FAIL"></a>**RULE-ROOM-CREATE-FAIL** · 규칙

→ 구조 — [`ROOM.http.CREATE_ROOM`](#ROOM.http.CREATE_ROOM)

<a id="ROOM.http.GET_ROOM"></a>
#### 4-1-14. `GET /rooms/{room_id}` — 방 상세
누가: 클라 · 인증: bearer · session · 성공 `200` · CORS 붙는다

**경로 칸**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ |  |

**질의**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| ★`?tracks` | 0 / 1 | — (기본 0) | ★**기본 `0`.** `1` 이면 그 방 트랙 전량 |

**응답**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ |  |
| `name` | str | ✔ |  |
| `capacity` | u32 | ✔ |  |
| `user_count` | u32 | ✔ |  |
| `participants` | MemberInfo[] | ✔ | MemberInfo[] §4-1-3 |
| ★`me` | MemberInfo | ★내가 그 방에 입장 중일 때만 | ★**나** — 명단과 따로 싣는다(투명이든 아니든 늘 · 권한 포함 — ★내 권한의 출처는 이것이다 · DEV-ROOM-ME-APART) |
| `created_at` | u64 | ✔ |  |
| ★`media_codecs` | MediaCodecs | ✔ | §4-1-1 |
| ★`version` | Version | ✔ | ★**항상 온다.** ★**스냅샷이라 갭으로 보지 않고 보관값을 재설정한다**(§4-2-3 규칙 2) — ★단 같은 `epoch` 에서 보관값보다 낮으면 버린다(DEV-ROOM-SNAPSHOT-BEHIND) |
| `tracks` | TrackEntry[] | ★tracks=1 일 때만 |  |

**실패** — `3001`(404) `404` = `3001` · `2002`(401) `401` = `2002`/`2003` · `2003`(401) `401` = `2002`/`2003` · `2008`(401) 세션 헤더 무효도 `2008` — body 는 §11-1-1 `Failure` 형. ★상태 코드가 아니라 `code` 로 판단한다

<a id="RULE-ROOM-DETAIL-TWO-USES"></a>**RULE-ROOM-DETAIL-TWO-USES** · 규칙 — `GET /rooms/{room_id}?tracks=0|1` — 방 상세

★★**한 경로가 두 쓰임을 겸한다** — ①**입장 전 미리보기** ②★**재동기.** 쿼리 하나로 가른다.

<a id="EX-ROOM-DETAIL-FORM"></a>**EX-ROOM-DETAIL-FORM** · 예시(비규범)

→ 구조 — [`ROOM.example.EX-ROOM-DETAIL-FORM`](#EX-ROOM-DETAIL-FORM) · [`ROOM.http.GET_ROOM`](#ROOM.http.GET_ROOM) · [`TRK.type.TrackEntry`](06-trk.md#TRK.type.TrackEntry)

예시 — `GET_ROOM` 응답(형과 대조된 값 · 주석은 칸 설명)

```jsonc
{ "room_id": "…",
  "name": "…",
  "capacity": 1000,
  "user_count": 12,
  "participants": [ {"user_id":"u1","role":255,"select":false,"participant_type":0,"metadata":{"name":"…"}} ],   // MemberInfo[] §4-1-3
  "created_at": 1756000000000,
  "media_codecs": {"audio":"opus","video":"H264"},      // §4-1-1
  "version": {"epoch":"sfu-7f3a…","seq":10245},         // ★**항상 온다.** ★**스냅샷이라 갭으로 보지 않고 보관값을 재설정한다**(§4-2-3 규칙 2) — ★단 같은 `epoch` 에서 보관값보다 낮으면 버린다(DEV-ROOM-SNAPSHOT-BEHIND)
  "tracks": [ {"room_id":"r1","user_id":"u2","kind":"audio","track_id":"t-u2-mic","type":"individual"} ] }   // ★tracks=1 일 때만
```

<a id="RULE-ROOM-DETAIL-FIELDS"></a>**RULE-ROOM-DETAIL-FIELDS** · 규칙

| 항목 | 규격 |
|---|---|
| ★`?tracks` | ★**기본 `0`.** `1` 이면 그 방 트랙 전량 |
| ★배정표 | ★**싣지 않는다** — 이 응답은 보기용 목록이다. 구독 SDP 의 재료(배정표)는 WS 로만 온다(§8-1-2) |
| ★`version` | ★**항상 온다.** ★**스냅샷이라 갭으로 보지 않고 보관값을 재설정한다**(§4-2-3 규칙 2) — ★단 같은 `epoch` 에서 보관값보다 낮으면 버린다(DEV-ROOM-SNAPSHOT-BEHIND) |
| 발언권 | ★**여기 없다** — 아래 |

→ 구조 — [`ROOM.http.GET_ROOM`](#ROOM.http.GET_ROOM)

<a id="RULE-ROOM-DETAIL-PROC"></a>**RULE-ROOM-DETAIL-PROC** · 규칙

**절차**
- ★**방에 안 들어가 있어도 부를 수 있다.** 들어갈지 정하려고 보는 것이다.
- 클라 ★★**이 응답은 목록만 고친다** — HTTP 는 WS 와 순서가 없으므로 SDP 재료를 싣지 않는다(§8-1-2). 갭으로 알게 된 새 트랙의 m-line 은 서버가 `MLINE_EVENT` 로 따로 준다.
- 클라 반영한 뒤 구독 SDP 를 ★**지금 보관한 배정표로** 다시 조립하고(§9-4-7) 협상 성공 후 `READY{arev}` — 재동기의 끝을 서버에 알리는 신호다(키프레임 · 처음 알리기, §8-1-3). 표가 그대로면 SDP 도 그대로다.
- 클라 ★**여러 방을 다시 받았으면 다 반영한 뒤 한 번 조립한다.**

→ 구조 — [`ROOM.http.GET_ROOM`](#ROOM.http.GET_ROOM) · [`ROOM.C.JOINED.snapshot`](#ROOM.C.JOINED.snapshot) · [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) · [`ROOM.C.JOINED.sync_required`](#ROOM.C.JOINED.sync_required) · [`ROOM.C.JOINED.snapshot_short`](#ROOM.C.JOINED.snapshot_short)

<a id="RULE-ROOM-DETAIL-WHEN"></a>**RULE-ROOM-DETAIL-WHEN** · 규칙

★**언제 부르나 — 셋뿐이다**

| # | 언제 | 왜 |
|---|---|---|
| ① | 입장 전 미리보기 | *"이 채널에 지금 누가 있나"* |
| ② | ★`ROOM_EVENT{sync_required}`(§4-2-6) 를 받았다 | 서버가 어긋남을 감지했다. 같은 방 재통보는 **30초 쿨다운**(§4-2-3) |
| ③ | ★`version.seq` 가 건너뛰었다(§4-2-3 규칙 5) | 내가 스스로 유실을 감지했다. ★**그 방 재동기가 진행 중이면 새로 던지지 않는다**(§4-2-3) |

→ 구조 — [`ROOM.C.JOINED.sync_required`](#ROOM.C.JOINED.sync_required) · [`ROOM.C.JOINED.sync_required_inflight`](#ROOM.C.JOINED.sync_required_inflight) · [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) · [`ROOM.C.JOINED.delta_gap_inflight`](#ROOM.C.JOINED.delta_gap_inflight)

<a id="RULE-ROOM-DETAIL-IMPL-BUDGET"></a>**RULE-ROOM-DETAIL-IMPL-BUDGET** · 규칙

★★**던지는 개수와 간격은 규격이 아니라 구현 책임이다.** 몇 개를 언제 던지든
★**`version` 을 견주기만 하면 상태가 안 깨지고**, 그것은 자기 대역을 쓰는 문제라 남의 상태를
망치지 않는다. ★**계약은 "낡은 응답은 버린다" 하나다.**

<a id="RULE-ROOM-DETAIL-NO-FLOOR"></a>**RULE-ROOM-DETAIL-NO-FLOOR** · 규칙

★**발언권은 여기 없다.** 발언권은 ★**DC 로만** 흐르고(§10) DC 는 WS·HTTP 와 수명이 다르다 —
미디어가 살아 있으면 통지가 계속 오므로 **다시 받을 것이 없고**, 미디어가 죽었으면
★**상태를 알아봐야 발언권을 못 쓴다**(복구는 미디어 연결을 되살리는 것이고 그 뒤 재동기에 서버가 DC 로 답한다, §9-2-1 5).
★HTTP 로 주면 **도착하는 순간 이미 낡은 값**이고, ★**통지 권위가 둘로 갈린다**(DC ↔ HTTP).

<a id="RULE-ROOM-DETAIL-FAIL"></a>**RULE-ROOM-DETAIL-FAIL** · 규칙

→ 구조 — [`ROOM.http.GET_ROOM`](#ROOM.http.GET_ROOM)

<a id="DEV-ROOM-SNAPSHOT-BEHIND"></a>**DEV-ROOM-SNAPSHOT-BEHIND** · 자체 확장

재동기 응답(`GET /rooms` — HTTP)의 `version` 이 같은 `epoch` 에서 보관값보다 낮으면 버린다 — 그 응답이 지어진 뒤의 WS 델타를 이미 반영했다. 같거나 크면 규칙 2 대로 견주지 않고 재설정한다(큰 점프를 갭으로 보지 않는다)

*원문: §4-2-3 규칙 2 · §4-1-14 — 스냅샷은 견주지 않는다 · 근거: RULE-ROOM-SEQ-GAP PROP-ROOM-NO-REWIND*

<a id="ROOM.fact.PermissionMap"></a>
#### 4-1-15. 비트마다 무엇을 막나 — 사상표

| 비트 | 막는 것 | 판정 자리 |
|---|---|---|
| `publish_audio` | 오디오 발행 전부 — 전이중 마이크 · 반이중 마이크(슬롯 발화도 발행이 관문이다) | `PUBLISH_TRACKS` 의 `kind:"audio"` 항목(§7-1-2) · 반이중 발화는 `FLOOR_REQUEST`(§10-4-30) |
| `publish_video` | 카메라 영상 | `kind:"video"` ∧ `source:"camera"`(§7-1-2) · 반이중 영상은 `FLOOR_REQUEST`(§10-4-30) |
| `publish_screen` | 화면공유 영상 | `kind:"video"` ∧ `source:"screen"`(§7-1-2) |
| `floor_request` | 발언 요청 | `FLOOR_REQUEST` → `DENY(102)`(§10-4-31). ★**긴급도 같다** — 우회 경로가 없다 |

<a id="RULE-ROOM-SOURCE-TRUSTED"></a>**RULE-ROOM-SOURCE-TRUSTED** · 규칙

★★**`source` 는 클라 신고값이고 서버는 그것을 믿는다.** 서버는 SDP 를 보지 않고(§6-1-1), 본다 해도 SDP·RTP 어디에도
*"이것은 화면이다"* 를 검증할 표식이 없다. 그래서 `publish_screen` 이 막는 것은 ★**협조하는 클라의 화면공유**다 —
3rd 앱은 우리 SDK 로 만들고 SDK 가 라벨을 맞게 신고한다. 유효 토큰을 쥔 악의 클라는 어차피 카메라 라벨로 무엇이든
보낸다 — 그것은 이 비트가 아니라 토큰 발급(§3-1-5)이 막는 층이다. 선례(LiveKit)도 같은 자리에 서 있다 — 클라
`AddTrackRequest.source` 를 그대로 대조한다. ★**그래서 `source` 는 닫힌 집합이다** — 모르는 값은 어느 비트인지 답할 수
없으므로 `1002` 다(§7-1-2).

<a id="RULE-ROOM-BITS-PER-ROOM"></a>**RULE-ROOM-BITS-PER-ROOM** · 규칙

★★**비트는 방 것이다.** 같은 사람이 방마다 다른 비트를 가진다 — 토큰(§3-1-5)이 **입장 초기값**을 주고, 그 뒤로는 서버 밖의
결정자가 ★**방 하나를 지목해** 바꾼다(`PARTICIPANT_STATE` 가 `room_id` 를 싣는 이유다, §4-2-6). ★**내 발행(전이중 · 반이중)은
`pub_room` 의 비트**를 따른다 — 발행 RTP 는 거기서만 흐른다(§5-1-2 전환 규칙 · §10-4-30).

<a id="RULE-ROOM-BITS-OUTLIVE"></a>**RULE-ROOM-BITS-OUTLIVE** · 규칙

★★**갱신값은 그 방이 기억한다 — 멤버십보다 오래 산다.** 나갔다 들어와도(§4-1 재입장), 새 토큰으로 다시 붙어도(§3-1)
★**그 방의 값이 토큰값보다 앞선다.** 지워지는 것은 둘뿐이다 — 방이 소멸했거나, 결정자가 기본값(넷 다 `true`)으로 되돌렸을
때. ★**방에 없는 사람의 비트는 못 바꾼다** — 미리 정하려면 토큰에 싣는다(§3-1-5).

<a id="ROOM.state"></a>
#### 4-1-16. 상태 칸 · 시한

##### 상태 — 클라

| 칸 | 처음 | 뜻 |
|---|---|---|
| `sfu` | `0` | ★그 방이 붙은 미디어 서버 — 입장 응답 server_config.sfu_id(0 = 아직) · 같은 방의 SUB · TRK · FLR 은 사본 없이 이것으로 서버를 안다(축 B) |
| `rs` | `'NONE'` | 방 상태(§4-4-1) |
| `cseq` | `0` | 보관본의 version.seq — 이 번호의 내용을 들고 있다(§4-2-3) |
| `cepoch` | `0` | 보관본의 version.epoch — 이 기동의 내용이다(0 = 보관본 없음 · 규칙 1) |
| `rsync` | `false` | 그 방 재동기(GET /rooms)가 진행 중이다(§4-2-3 재동기 in-flight) |
| `ahead` | `0` | 재동기 중에 버린 델타의 가장 큰 seq — 응답이 이보다 낮으면 따라잡지 못한 것이다(DEV-ROOM-INFLIGHT-AHEAD) |
| `retry` | `false` | 4009 를 받아 retry_after_ms 를 기다린다(§4-2-2 6) |
| `quit` | `false` | 입장 요청이 나가 있는 동안 앱이 나가려 했다 — 응답을 기다렸다가 성공이면 ROOM_LEAVE 를 보낸다(DEV-ROOM-LEAVE-WHILE-JOINING) |
| `rejoin` | `false` | 재구축이 다시 들어갈 방이다 — 내릴 때 들어가 있었거나 들어가려던 방(DEV-ROOM-REJOIN-LIST · 원칙 ①) |

##### 상태 — 서버

| 칸 | 처음 | 뜻 |
|---|---|---|
| `open` | `true` | 방이 있다(닫히면 3001) |
| `member` | `false` | 이 클라가 명단에 있다 |
| `seq` | `1` | 그 방 공통 스냅샷의 seq(§4-2-6) |
| `epoch` | `1` | 그 미디어 서버의 기동 단위(§4-1-6 epoch) — 재기동하면 바뀐다 |
| `full` | `false` | 정원이 찼다(4001) |
| `pubBit` | `true` | ★그 사람의 그 방 권한 비트 — 발행 · 발언(§4-2-6) · 방이 기억한다 — 멤버십보다 오래 산다(RULE-ROOM-BITS-OUTLIVE) |
| `floorBit` | `true` |  |
| `busy` | `false` | 과부하(4009) |
| `cool` | `false` | 같은 방 sync_required 재통보 쿨다운 중(T-stall · §4-2-3) |
| `sel` | `true` | ★그 사람의 select — 입장 요청값 그대로 명단에 에코된다(§4-1 · ROOM.type.MemberInfo). true = 이 방에서 말한다 → 입장이 곁들여 발행 방을 바꾼다(aff.join) |

##### 전이 — 클라(동작 경우에서 짓는다)

| 지금 | 사건 | 다음 | 라벨 |
|---|---|---|---|
| `NONE` | 앱이 방에 들어가려 한다(§4-2-1) | `JOINING` | `ROOM.C.NONE.join` |
| `JOINING` | ROOM_JOIN 응답을 받았다(§4-2-2 · §4-2-2) | `LEAVING` | `ROOM.C.JOINING.join_ok_quit` |
| `JOINING` | ROOM_JOIN 응답을 받았다(§4-2-2 · §4-2-2) | `NONE` | `ROOM.C.JOINING.join_fail_quit` |
| `JOINING` | ROOM_JOIN 응답을 받았다(§4-2-2 · §4-2-2) | `LEAVING` | `ROOM.C.JOINING.join_pc_mismatch` |
| `JOINING` | ROOM_JOIN 응답을 받았다(§4-2-2 · §4-2-2) | `JOINED` | `ROOM.C.JOINING.join_ok` |
| `JOINING` | ROOM_JOIN 응답을 받았다(§4-2-2 · §4-2-2) | 그대로 | `ROOM.C.JOINING.join_4009` |
| `JOINING` | ROOM_JOIN 응답을 받았다(§4-2-2 · §4-2-2) | `NONE` | `ROOM.C.JOINING.join_1xxx` |
| `JOINING` | ROOM_JOIN 응답을 받았다(§4-2-2 · §4-2-2) | `NONE` | `ROOM.C.JOINING.join_fail` |
| `JOINING` | 4009 의 retry_after_ms 하한이 지났다(서버가 준 값 — 원장 시한이 아니다) | 그대로 | `ROOM.C.JOINING.retry_join` |
| `JOINING` | 앱이 나가려 한다(§4-2-3) · JOINING 이면 4009 대기를 멈춘다 | `NONE` | `ROOM.C.JOINING.app_leave` |
| `JOINING` | 앱이 나가려 한다(§4-2-3) · JOINING 이면 4009 대기를 멈춘다 | 그대로 | `ROOM.C.JOINING.app_leave_wait` |
| `JOINED` | 앱이 나가려 한다(§4-2-3) · JOINING 이면 4009 대기를 멈춘다 | `LEAVING` | `ROOM.C.JOINED.leave` |
| 어느 상태 | ROOM_LEAVE 응답을 받았다(§4-2-5) | — | `ROOM.C.any.leave_res_stale` |
| `LEAVING` | ROOM_LEAVE 응답을 받았다(§4-2-5) | `NONE` | `ROOM.C.LEAVING.leave_ok` |
| `LEAVING` | ROOM_LEAVE 응답을 받았다(§4-2-5) | `NONE` | `ROOM.C.LEAVING.leave_fail` |
| 어느 상태 | version 을 실은 통지를 받았다 — ACK · 판정(§4-2-3 규칙 3~5) · 반영(§4-2-3) | — | `ROOM.C.any.delta_not_joined` |
| `JOINED` | version 을 실은 통지를 받았다 — ACK · 판정(§4-2-3 규칙 3~5) · 반영(§4-2-3) | 그대로 | `ROOM.C.JOINED.delta_bare` |
| `JOINED` | version 을 실은 통지를 받았다 — ACK · 판정(§4-2-3 규칙 3~5) · 반영(§4-2-3) | 그대로 | `ROOM.C.JOINED.delta_epoch` |
| `JOINED` | version 을 실은 통지를 받았다 — ACK · 판정(§4-2-3 규칙 3~5) · 반영(§4-2-3) | 그대로 | `ROOM.C.JOINED.delta_old` |
| `JOINED` | version 을 실은 통지를 받았다 — ACK · 판정(§4-2-3 규칙 3~5) · 반영(§4-2-3) | 그대로 | `ROOM.C.JOINED.delta_apply` |
| `JOINED` | version 을 실은 통지를 받았다 — ACK · 판정(§4-2-3 규칙 3~5) · 반영(§4-2-3) | 그대로 | `ROOM.C.JOINED.delta_gap_inflight` |
| `JOINED` | version 을 실은 통지를 받았다 — ACK · 판정(§4-2-3 규칙 3~5) · 반영(§4-2-3) | 그대로 | `ROOM.C.JOINED.delta_gap` |
| 어느 상태 | ROOM_EVENT 를 받았다 — version 없음 · 견주지 않는다(§4-2-3 · §4-2-3 종결) | — | `ROOM.C.any.room_event_not_joined` |
| `JOINED` | ROOM_EVENT 를 받았다 — version 없음 · 견주지 않는다(§4-2-3 · §4-2-3 종결) | `NONE` | `ROOM.C.JOINED.kicked` |
| `JOINED` | ROOM_EVENT 를 받았다 — version 없음 · 견주지 않는다(§4-2-3 · §4-2-3 종결) | 그대로 | `ROOM.C.JOINED.sync_required_inflight` |
| `JOINED` | ROOM_EVENT 를 받았다 — version 없음 · 견주지 않는다(§4-2-3 · §4-2-3 종결) | 그대로 | `ROOM.C.JOINED.sync_required` |
| 어느 상태 | GET /rooms/{room_id}?tracks=1 응답을 받았다(§4-2-3 3 · §4-1-14 · §4-2-3 규칙 2) | — | `ROOM.C.any.snapshot_stale` |
| `JOINED` | GET /rooms/{room_id}?tracks=1 응답을 받았다(§4-2-3 3 · §4-1-14 · §4-2-3 규칙 2) | 그대로 | `ROOM.C.JOINED.snapshot_epoch` |
| `JOINED` | GET /rooms/{room_id}?tracks=1 응답을 받았다(§4-2-3 3 · §4-1-14 · §4-2-3 규칙 2) | 그대로 | `ROOM.C.JOINED.snapshot_short` |
| `JOINED` | GET /rooms/{room_id}?tracks=1 응답을 받았다(§4-2-3 3 · §4-1-14 · §4-2-3 규칙 2) | 그대로 | `ROOM.C.JOINED.snapshot_behind` |
| `JOINED` | GET /rooms/{room_id}?tracks=1 응답을 받았다(§4-2-3 3 · §4-1-14 · §4-2-3 규칙 2) | 그대로 | `ROOM.C.JOINED.snapshot` |
| `JOINED` | 같은 서버 방의 입장 응답이 왔다 — 응답 affiliation.sub_rooms 가 그 서버의 진실이다(RULE-ROOM-JOIN-PROC) · listed = 이 방이 그 목록에 있다 | `JOINING` | `ROOM.C.JOINED.not_in_sub_rooms` |
| 어느 상태 | 같은 서버 방의 입장 응답이 왔다 — 응답 affiliation.sub_rooms 가 그 서버의 진실이다(RULE-ROOM-JOIN-PROC) · listed = 이 방이 그 목록에 있다 | — | `ROOM.C.any.sub_rooms_ok` |
| `LEAVING` | 재구축으로 방을 내린다 — 세션 전체면 전부 · 그 서버면 그 서버 방(§3-2-3 2 · RULE-LINK-RECOVERY-REBUILD) | `NONE` | `ROOM.C.LEAVING.down` |
| 어느 상태 | 재구축으로 방을 내린다 — 세션 전체면 전부 · 그 서버면 그 서버 방(§3-2-3 2 · RULE-LINK-RECOVERY-REBUILD) | `NONE` | `ROOM.C.any.down` |
| 어느 상태 | 세션이 끝났다 — 방을 전부 내리고 다시 들어갈 방 표시를 지운다(DEV-SES-FRESH-REJOIN · 앱이 다시 들어간다) | `NONE` | `ROOM.C.any.close_all` |
| `NONE` | 재구축 — 내렸던 방에 다시 들어간다(§3-2-3 3 · RULE-LINK-RECOVERY-REBUILD) | `JOINING` | `ROOM.C.NONE.rejoin` |
| `JOINED` | RESUME 성공 — 응답 스냅샷으로 방 보관본을 다시 맞춘다(§3-2-3 1 · 2 ②) | `NONE` | `ROOM.C.JOINED.resume_gone` |
| `JOINED` | RESUME 성공 — 응답 스냅샷으로 방 보관본을 다시 맞춘다(§3-2-3 1 · 2 ②) | 그대로 | `ROOM.C.JOINED.resume_epoch` |
| `JOINED` | RESUME 성공 — 응답 스냅샷으로 방 보관본을 다시 맞춘다(§3-2-3 1 · 2 ②) | 그대로 | `ROOM.C.JOINED.resume_snapshot` |

##### 시한
T-stall — 같은 방에 sync_required 를 보낸 뒤 다시 알릴 수 있을 때까지(서버 · 30초)

| 이름 | 원문 이름 | 값(ms) | 모형 | 값의 근거 |
|---|---|---|---|---|
| <a id="ROOM.ledger.TStall"></a>`TStall` | T-stall | 30000 | 선언 |  |

| 횟수 | 값 |
|---|---|
| <a id="ROOM.ledger.BundleCap"></a>`BundleCap` | 128 |
| <a id="ROOM.ledger.ClientMlines"></a>`ClientMlines` | 32 |
| <a id="ROOM.ledger.Sub1pc"></a>`Sub1pc` | 96 |
| <a id="ROOM.ledger.Sub2pc"></a>`Sub2pc` | 128 |
| <a id="ROOM.ledger.DispatchCh"></a>`DispatchCh` | 20 |
| <a id="ROOM.ledger.SlotBudget"></a>`SlotBudget` | 40 |
| <a id="ROOM.ledger.FullAudio"></a>`FullAudio` | 25 |
| <a id="ROOM.ledger.FullVideo"></a>`FullVideo` | 25 |
| <a id="ROOM.ledger.MaxRooms"></a>`MaxRooms` | 100 |

<a id="ROOM.C"></a>
### 4-2. 동작

<a id="ROOM.C.NONE"></a>
#### 4-2-1. 클라 · `NONE`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="ROOM.C.NONE.join"></a>`ROOM.C.NONE.join` | 받는 상태 rs == 'NONE' · 나머지 | rs ← 'JOINING'; cseq ← 0; cepoch ← 0; rsync ← false; ahead ← 0; retry ← false; quit ← false · 보낸다 ROOM_JOIN.req | §4-2-1 1 §4-2-1 4 §4-2-1 5 |
| <a id="ROOM.C.NONE.rejoin"></a>`ROOM.C.NONE.rejoin` | 받는 효과 SES rooms.rejoin · LINK rooms.rejoin_server · 받는 상태 rs == 'NONE' && rejoin · 나머지 | rs ← 'JOINING'; cseq ← 0; cepoch ← 0; rsync ← false; ahead ← 0; retry ← false; rejoin ← false · 보낸다 ROOM_JOIN.req | §3-2-3 3 §9-2-1 2 DEV-ROOM-REJOIN-LIST — 입장 순서대로 · 첫 ROOM_JOIN 은 0 ~ 분산 창으로 늦춘다(§3-2-3) — 모형에 시간 · 방 여럿이 없어 싣지 않는다. 다시 들어갈 방은 내릴 때 rejoin 칸에 남긴다(DEV-ROOM-REJOIN-LIST) |

<a id="RULE-ROOM-JOIN-START"></a>**RULE-ROOM-JOIN-START** · 규칙 — 앱이 방에 들어가려 한다 (`NONE`)

1. 연결이 `ACTIVE` 가 아니면 **기다린다.**
2. ★**이 방에서 말하려는데(`select:true`) 이미 발행 방이 있으면, `select:false` 로 들어가고 입장 응답 뒤 소속 전환으로 옮긴다**(§5-2-2 — 같은 서버면 요청 하나 · 서버를 건너면 옛 서버 해제 먼저 · 해제가 안 되면 새 서버에 선택을 보내지 않는다). *"발행 방은 하나"*(§5-1-1)는 ★**전역
   계약인데 서버는 자기 것만 안다** — 전역 단일성은 클라가 지킨다. ★**요청 시점에는 그 방의 서버를 모른다**(서버는 응답의 `server_config.sfu_id` 로 드러난다) — 그래서 `select:true` 로 곧장 들어가면 다른 서버의 옛 발행 방이 남아 발행 방이 둘이 된다. ★**전환이 실패해도 입장은 그대로다** — 듣기와 발행 방은 다른 축이라 입장을 되돌리지 않고 전환 실패만 알린다. 발행 방이 없고 소속 전환도 떠 있지 않으면 `select:true` 로 곧장 들어간다(입장이 곁들여 정한다) — ★그 입장도 떠 있는 전환으로 친다(그 응답이 올 때까지 다른 전환을 막는다 — 응답 전에 다른 서버의 방을 고르면 두 서버가 발행 방을 갖는다 · 20261023 TLC). ★그 기다림은 소속 응답이 아니라 그 입장이 푼다 — 성공 응답의 `affiliation` · 실패 응답 · 응답 전에 그 방이 내려감(나가기 · 재구축) 셋 다(안 풀면 다음 전환이 영영 막힌다).
3. ★`1pc` 면 ★**연결을 먼저 만든다**(§9-2-1 2①~③ — 데이터 채널 + `inactive` 트랜시버 + 브라우저 offer → `setLocalDescription`). 그 offer 의 번호표(`extmap`·`codecs`)를 요청에 싣는다(§4-1). 요청 시점에 그 방의 서버를 모르므로 ★**입장마다 새 연결로 한다** — 응답에서 이미 붙은 서버로 드러나면 버린다(§4-2-2).

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`ROOM.C.NONE.join`](#ROOM.C.NONE.join) |
| 4 | [`ROOM.C.NONE.join`](#ROOM.C.NONE.join) |
| 5 | [`ROOM.C.NONE.join`](#ROOM.C.NONE.join) |

<a id="ROOM.C.JOINING"></a>
#### 4-2-2. 클라 · `JOINING`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="ROOM.C.JOINING.join_ok_quit"></a>`ROOM.C.JOINING.join_ok_quit` | 받는 상태 rs == 'JOINING' · m.part == 'res' && quit | rs ← 'LEAVING'; quit ← false · 보낸다 ROOM_LEAVE.req · 효과 aff.leave → AFF.C.any.left · AFF.C.any.left_join · AFF.C.any.left_want · AFF.C.any.left_other | §4-2-3 1 DEV-ROOM-LEAVE-WHILE-JOINING — 기다리는 동안 앱이 나가려 했다 — 서버는 받아들였으니 연결을 세우지 않고 ROOM_LEAVE 로 나간다(응답 뒤 처리는 퇴장과 같다) |
| <a id="ROOM.C.JOINING.join_fail_quit"></a>`ROOM.C.JOINING.join_fail_quit` | 받는 상태 rs == 'JOINING' · m.part != 'res' && quit && m.code >= 2000 | rs ← 'NONE'; quit ← false; retry ← false · 효과 aff.join_failed → AFF.C.any.join_failed · AFF.C.any.join_failed_other · 효과 app.left | §4-2-2 1 DEV-ROOM-LEAVE-WHILE-JOINING — 기다리는 동안 앱이 나가려 했다 — 실패면 들어간 적이 없으니 보낼 것 없이 끝낸다(4009 도 되풀이하지 않는다) |
| <a id="ROOM.C.JOINING.join_pc_mismatch"></a>`ROOM.C.JOINING.join_pc_mismatch` | 받는 상태 rs == 'JOINING' · m.part == 'res' && m.server_config.pc_mode != pcm | rs ← 'LEAVING' · 보낸다 ROOM_LEAVE.req · 효과 aff.leave → AFF.C.any.left · AFF.C.any.left_join · AFF.C.any.left_want · AFF.C.any.left_other · 효과 app.join_failed | §4-1 DEV-ROOM-PC-MISMATCH-UNDO — 응답의 server_config.pc_mode 가 세션 확정값과 다르다 — 규격 위반 · 연결을 세우지 않고 실패로 다룬다(RULE-ROOM-JOIN-PC-MODE-CHECK · 서버가 조용히 다른 모드로 돌리는 경로는 없다). ★서버는 이미 받아들였다 — ROOM_LEAVE 로 되돌린다(응답 뒤 처리는 퇴장과 같다 — 세우던 연결은 link.room_left 로 버린다) |
| <a id="ROOM.C.JOINING.join_ok"></a>`ROOM.C.JOINING.join_ok` | 받는 상태 rs == 'JOINING' · m.part == 'res' | rs ← 'JOINED'; cseq ← m.version.seq; cepoch ← m.version.epoch; sfu ← m.server_config.sfu_id · 효과 trk.snapshot → TRK.C.any.snapshot · TRK.C.any.snapshot_same · 효과 link.table → LINK.C.any.table_carried_same · LINK.C.any.table_carried · LINK.C.any.table_carried_rebound · LINK.C.any.table_carried_old · 효과 sdp.rebuild → LINK.C.any.renego_queued · LINK.C.any.renego · 효과 ready.tracks → SUB.C.any.ready_want · 효과 app.joined · 효과 room.sub_rooms · 효과 aff.implicit → AFF.C.any.implicit_joined · AFF.C.any.implicit_join_unselected · AFF.C.any.implicit_busy · AFF.C.any.implicit_pub · AFF.C.any.implicit_away · AFF.C.any.implicit_same · 효과 floor.join → FLR.C.any.join | §4-2-3 2 §4-2-2 1 §4-2-2 2 §4-2-2 3 §4-2-2 4 §4-2-2 5 §4-2-2 6 — ★스냅샷이다 — version 은 견주지 않고 재설정 |
| <a id="ROOM.C.JOINING.join_4009"></a>`ROOM.C.JOINING.join_4009` | 받는 상태 rs == 'JOINING' · m.code == 4009 | retry ← true | §4-2-2 6 R-ROOM-ON-JOIN-FAIL-WHY |
| <a id="ROOM.C.JOINING.join_1xxx"></a>`ROOM.C.JOINING.join_1xxx` | 받는 상태 rs == 'JOINING' · m.code < 2000 | rs ← 'NONE' · 효과 aff.join_failed → AFF.C.any.join_failed · AFF.C.any.join_failed_other · 효과 app.bug | §4-2-2 3 |
| <a id="ROOM.C.JOINING.join_fail"></a>`ROOM.C.JOINING.join_fail` | 받는 상태 rs == 'JOINING' · 나머지 | rs ← 'NONE' · 효과 aff.join_failed → AFF.C.any.join_failed · AFF.C.any.join_failed_other · 효과 app.join_failed | §4-2-2 1 §4-2-2 2 §4-2-2 4 §4-2-2 5 R-ROOM-ON-JOIN-FAIL-WHY |
| <a id="ROOM.C.JOINING.retry_join"></a>`ROOM.C.JOINING.retry_join` | 받는 상태 rs == 'JOINING' && retry · 나머지 | retry ← false · 보낸다 ROOM_JOIN.req | §4-2-2 6 |
| <a id="ROOM.C.JOINING.app_leave"></a>`ROOM.C.JOINING.app_leave` | 받는 상태 rs == 'JOINED' \|\| (rs == 'JOINING' && !quit) · rs == 'JOINING' && retry | rs ← 'NONE'; retry ← false · 효과 aff.join_failed → AFF.C.any.join_failed · AFF.C.any.join_failed_other · 효과 app.left | §4-2-2 6 — 4009 대기 중에 앱이 그 방을 나가면 되풀이를 멈춘다(보낸 요청이 없다) |
| <a id="ROOM.C.JOINING.app_leave_wait"></a>`ROOM.C.JOINING.app_leave_wait` | 받는 상태 rs == 'JOINED' \|\| (rs == 'JOINING' && !quit) · rs == 'JOINING' | quit ← true | §4-2-3 1 DEV-ROOM-LEAVE-WHILE-JOINING — 입장 요청이 나가 있다 — 지금 보내지 않고 응답을 기다린다(성공이면 ROOM_LEAVE · 실패면 끝) |

<a id="RULE-ROOM-JOIN-OK"></a>**RULE-ROOM-JOIN-OK** · 규칙 — `ROOM_JOIN` 성공 응답을 받았다 (`JOINING`)

1. `server_config` 를 **그 서버 것으로 보관한다.**
2. 그 서버의 연결이 아직 없으면 **만든다** — 발행·구독 SDP 를 §9-4-4 대로 조립한다.
   ★`1pc` 면 **연결이 하나**다 — 응답의 `sfu_id` 가 ★**처음 붙는 서버면** §4-2-1 3 에서 만든 연결에 `server_config` 로 answer 를 조립해
   첫 협상을 닫고(§9-2-1 2④ — 이 answer 가 확정본), ★**이미 붙은 서버면 그 연결을 버린다**(그 서버는 기존 연결의 번호표를 이미 안다).
3. `tracks[]` 와 ★`version`(§4-1-6)을 **통째로 보관한다**(§6-1-1 — 보기용 목록).
   ★`mline_table` 은 배정표로 받는다(§8-1-2 — `arev` 가 보관값보다 클 때만 갈아 끼운다).
4. 구독 SDP 를 ★**배정표로** 조립하고 협상한다. ★`1pc` 는 ★**통합 offer**(§9-4-19)로 한 번에 한다.
5. 협상이 성공하면 `READY{arev}` 를 보낸다.

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`ROOM.C.JOINING.join_ok`](#ROOM.C.JOINING.join_ok) |
| 2 | [`ROOM.C.JOINING.join_ok`](#ROOM.C.JOINING.join_ok) |
| 3 | [`ROOM.C.JOINING.join_ok`](#ROOM.C.JOINING.join_ok) |
| 4 | [`ROOM.C.JOINING.join_ok`](#ROOM.C.JOINING.join_ok) |
| 5 | [`ROOM.C.JOINING.join_ok`](#ROOM.C.JOINING.join_ok) |
| 6 | [`ROOM.C.JOINING.join_ok`](#ROOM.C.JOINING.join_ok) |

<a id="RULE-ROOM-ON-JOIN-FAIL"></a>**RULE-ROOM-ON-JOIN-FAIL** · 규칙 — `ROOM_JOIN` 실패 응답을 받았다 (`JOINING`)

4. ★`4005`(구독 m-section 몫이 다 찼다) → 앱에 알리고 `NONE`.
   `details.current` 로 지금 몇 m-line 을 물고 있는지 앱에 알린다.
5. ★`4004`(동시 입장 방 수 100 초과, §4-1) → 앱에 알리고 `NONE`.
   `details.limit`·`details.current` 를 앱에 알린다.

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`ROOM.C.JOINING.join_fail_quit`](#ROOM.C.JOINING.join_fail_quit) · [`ROOM.C.JOINING.join_fail`](#ROOM.C.JOINING.join_fail) |
| 2 | [`ROOM.C.JOINING.join_fail`](#ROOM.C.JOINING.join_fail) |
| 3 | [`ROOM.C.JOINING.join_1xxx`](#ROOM.C.JOINING.join_1xxx) |
| 4 | [`ROOM.C.JOINING.join_fail`](#ROOM.C.JOINING.join_fail) |
| 5 | [`ROOM.C.JOINING.join_fail`](#ROOM.C.JOINING.join_fail) |
| 6 | [`ROOM.C.JOINING.join_4009`](#ROOM.C.JOINING.join_4009) · [`ROOM.C.JOINING.retry_join`](#ROOM.C.JOINING.retry_join) · [`ROOM.C.JOINING.app_leave`](#ROOM.C.JOINING.app_leave) |

<a id="ROOM.C.JOINED"></a>
#### 4-2-3. 클라 · `JOINED`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="ROOM.C.JOINED.leave"></a>`ROOM.C.JOINED.leave` | 받는 상태 rs == 'JOINED' \|\| (rs == 'JOINING' && !quit) · 나머지 | rs ← 'LEAVING'; rsync ← false; ahead ← 0 · 보낸다 ROOM_LEAVE.req · 효과 http.abandon · 효과 floor.leave → FLR.C.any.leave · 효과 aff.leave → AFF.C.any.left · AFF.C.any.left_join · AFF.C.any.left_want · AFF.C.any.left_other · 효과 sub.drop → SUB.C.any.drop · 효과 pub.room_gone → PUB.C.any.room_gone · PUB.C.any.room_gone_other | §4-2-3 1 §4-2-3 2 DEV-ROOM-RESYNC-ABANDON |
| <a id="ROOM.C.JOINED.delta_bare"></a>`ROOM.C.JOINED.delta_bare` | bare | 효과 app.members | §4-2-3 §4-1-3 DEV-ROOM-HIDDEN-SELF-PSTATE — bare = version 없이 나에게만 온 델타(투명 참가자 본인의 권한 — RULE-ROOM-VERSION-NOT-CARRIED) — 견주지 않고 반영한다 · 보관 seq 는 그대로 |
| <a id="ROOM.C.JOINED.delta_epoch"></a>`ROOM.C.JOINED.delta_epoch` | m.version.epoch != cepoch | 효과 link.epoch_changed → LINK.C.REBUILD.epoch_absorbed · LINK.C.any.epoch_rebuild | §4-2-3 1 R-ROOM-EPOCH-WHY — 규칙 1 — epoch 가 다르다 · 그 서버가 재기동됐다. 반영하지 않고 그 서버만 재구축한다(보관본은 재구축이 버린다 · RULE-SES-RES-RESUME-OK 4) |
| <a id="ROOM.C.JOINED.delta_old"></a>`ROOM.C.JOINED.delta_old` | m.version.seq <= cseq | — | §4-2-3 §4-1-6 §4-2-3 3 — seq ≤ 보관값 — 낡은 것이라 버린다 |
| <a id="ROOM.C.JOINED.delta_apply"></a>`ROOM.C.JOINED.delta_apply` | m.version.seq == cseq + 1 | cseq ← m.version.seq · 효과 app.members | §4-2-3 §4-1-6 §4-2-3 4 — seq = 보관값 + 1 — 반영하고 보관값을 갱신한다 |
| <a id="ROOM.C.JOINED.delta_gap_inflight"></a>`ROOM.C.JOINED.delta_gap_inflight` | rsync | ahead ← m.version.seq > ahead ? m.version.seq : ahead | §4-2-3 §4-2-3 §4-2-3 5 DEV-ROOM-INFLIGHT-AHEAD — 건너뛰었는데 그 방 재동기가 진행 중이면 새로 던지지 않는다(§4-2-3) · 버린 seq 를 기억한다 |
| <a id="ROOM.C.JOINED.delta_gap"></a>`ROOM.C.JOINED.delta_gap` | 나머지 | rsync ← true · 보낸다 GET_ROOM.req {tracks: 1} | §4-2-3 1 §4-2-3 2 §4-2-3 4 §4-2-3 5 R-ROOM-SEQ-GAP-WHY — 건너뛰었다 — GET /rooms/{room_id}?tracks=1 로 통짜로 다시 받는다 |
| <a id="ROOM.C.JOINED.kicked"></a>`ROOM.C.JOINED.kicked` | m.type == 'affiliation' | rs ← 'NONE'; cseq ← 0; cepoch ← 0; rsync ← false; ahead ← 0 · 효과 sdp.rebuild → LINK.C.any.renego_queued · LINK.C.any.renego · 효과 store.drop → TRK.C.any.store_drop · 효과 http.abandon · 효과 floor.leave → FLR.C.any.leave · 효과 app.room_gone · 효과 aff.leave → AFF.C.any.left · AFF.C.any.left_join · AFF.C.any.left_want · AFF.C.any.left_other · 효과 sub.drop → SUB.C.any.drop · 효과 link.room_left → LINK.C.any.last_room_left · LINK.C.any.room_left · 효과 pub.room_gone → PUB.C.any.room_gone · PUB.C.any.room_gone_other | §4-2-3 DEV-ROOM-RESYNC-ABANDON — 종결 — cause kick · room_closed 면 그 방을 NONE 으로 |
| <a id="ROOM.C.JOINED.sync_required_inflight"></a>`ROOM.C.JOINED.sync_required_inflight` | rsync | — | §4-2-3 §4-2-3 |
| <a id="ROOM.C.JOINED.sync_required"></a>`ROOM.C.JOINED.sync_required` | 나머지 | rsync ← true · 보낸다 GET_ROOM.req {tracks: 1} | §4-2-3 |
| <a id="ROOM.C.JOINED.snapshot_epoch"></a>`ROOM.C.JOINED.snapshot_epoch` | m.version.epoch != cepoch | rsync ← false; ahead ← 0 · 효과 link.epoch_changed → LINK.C.REBUILD.epoch_absorbed · LINK.C.any.epoch_rebuild | §4-2-3 1 §4-1-14 R-ROOM-EPOCH-WHY — 규칙 1 — 스냅샷이어도 epoch 가 다르면 재설정하지 않는다(다른 기동의 내용) · 이 재동기는 끝났다 · 그 서버만 재구축한다 |
| <a id="ROOM.C.JOINED.snapshot_short"></a>`ROOM.C.JOINED.snapshot_short` | m.version.seq < ahead | cseq ← m.version.seq > cseq ? m.version.seq : cseq; ahead ← 0 · 보낸다 GET_ROOM.req {tracks: 1} | §4-2-3 3 §4-2-3 2 DEV-ROOM-INFLIGHT-AHEAD — 재동기 중에 버린 델타가 응답보다 새것이다 — 응답을 반영하고 한 번 더 받는다 |
| <a id="ROOM.C.JOINED.snapshot_behind"></a>`ROOM.C.JOINED.snapshot_behind` | m.version.seq < cseq | rsync ← false; ahead ← 0 | §4-1-14 §4-2-3 2 DEV-ROOM-SNAPSHOT-BEHIND — HTTP 응답이 그 뒤의 WS 델타보다 늦게 닿았다 — 재설정하면 되감는다 |
| <a id="ROOM.C.JOINED.snapshot"></a>`ROOM.C.JOINED.snapshot` | 나머지 | cseq ← m.version.seq; rsync ← false; ahead ← 0 · 효과 trk.snapshot → TRK.C.any.snapshot · TRK.C.any.snapshot_same · 효과 sdp.rebuild → LINK.C.any.renego_queued · LINK.C.any.renego · 효과 ready.tracks → SUB.C.any.ready_want | §4-2-3 3 §4-2-3 4 §4-2-3 2 R-ROOM-DETAIL-PROC-WHY R-ROOM-SEQ-GAP-WHY — 스냅샷이다 — 견주지 않고 보관값을 재설정한다 · 큰 점프를 갭으로 보지 않는다 |
| <a id="ROOM.C.JOINED.not_in_sub_rooms"></a>`ROOM.C.JOINED.not_in_sub_rooms` | 받는 효과 ROOM room.sub_rooms · rs == 'JOINED' && !listed | rs ← 'JOINING'; cseq ← 0; cepoch ← 0; rsync ← false; ahead ← 0; retry ← false · 보낸다 ROOM_JOIN.req · 효과 store.drop → TRK.C.any.store_drop · 효과 http.abandon · 효과 floor.leave → FLR.C.any.leave · 효과 aff.leave → AFF.C.any.left · AFF.C.any.left_join · AFF.C.any.left_want · AFF.C.any.left_other · 효과 sub.drop → SUB.C.any.drop · 효과 pub.room_gone → PUB.C.any.room_gone · PUB.C.any.room_gone_other | §4-1 — 들어가 있다고 아는데 그 서버의 sub_rooms 에 없다 — 서버가 옛 Peer 를 축출했다(다시 입장) · NONE 으로 내리고 다시 입장한다(RULE-ROOM-JOIN-PROC) |
| <a id="ROOM.C.JOINED.resume_gone"></a>`ROOM.C.JOINED.resume_gone` | 받는 효과 SES rooms.snapshot · 받는 상태 rs == 'JOINED' · !present | rs ← 'NONE'; cseq ← 0; cepoch ← 0; rsync ← false; ahead ← 0; retry ← false · 효과 store.drop → TRK.C.any.store_drop · 효과 http.abandon · 효과 floor.leave → FLR.C.any.leave · 효과 app.room_gone · 효과 aff.leave → AFF.C.any.left · AFF.C.any.left_join · AFF.C.any.left_want · AFF.C.any.left_other · 효과 sub.drop → SUB.C.any.drop · 효과 link.room_left → LINK.C.any.last_room_left · LINK.C.any.room_left · 효과 pub.room_gone → PUB.C.any.room_gone · PUB.C.any.room_gone_other | §3-2-3 2 R-SES-RES-RESUME-OK-WHY — 스냅샷에 없는 방 — 끊긴 동안 강퇴 · 폭파 · 방 닫힘이 있었고 그 통지가 사라졌다. 재입장하지 않는다 |
| <a id="ROOM.C.JOINED.resume_epoch"></a>`ROOM.C.JOINED.resume_epoch` | 받는 효과 SES rooms.snapshot · 받는 상태 rs == 'JOINED' · epoch != cepoch | 효과 link.epoch_changed → LINK.C.REBUILD.epoch_absorbed · LINK.C.any.epoch_rebuild | §3-2-3 2 §3-2-3 4 §4-2-3 1 R-ROOM-EPOCH-WHY — 규칙 1 — RESUME 스냅샷이어도 epoch 가 다르면 재설정하지 않는다(다른 기동의 내용) · 서버가 재기동돼 그 연결을 모른다 — 그 서버만 재구축한다(RULE-SES-RES-RESUME-OK 2 · 4 · 재동기의 snapshot_epoch 와 같다) |
| <a id="ROOM.C.JOINED.resume_snapshot"></a>`ROOM.C.JOINED.resume_snapshot` | 받는 효과 SES rooms.snapshot · 받는 상태 rs == 'JOINED' · 나머지 | cseq ← seq · 효과 trk.snapshot → TRK.C.any.snapshot · TRK.C.any.snapshot_same · 효과 link.table → LINK.C.any.table_carried_same · LINK.C.any.table_carried · LINK.C.any.table_carried_rebound · LINK.C.any.table_carried_old | §3-2-3 1 §4-2-3 2 — 스냅샷이다 — 견주지 않고 재설정(epoch 가 같다) · 트랙 목록도 스냅샷으로(RULE-TRK-FOUR-PATHS) |

<a id="RULE-ROOM-VERSION-CARRIED"></a>**RULE-ROOM-VERSION-CARRIED** · 규칙 — `version` 을 싣는 곳 / 안 싣는 곳

**싣는다** — 그 스냅샷 또는 그 델타를 나르는 프레임

| 갈래 | 프레임 |
|---|---|
| ★**스냅샷** | `ROOM_JOIN` 응답 · `RESUME` 의 `snapshot[].version` · `GET /rooms/{id}` |
| ★**델타** | `PARTICIPANT_EVENT` · `TRACK_EVENT` · `TRACK_STATE`(`muted` 하나) · ★`PARTICIPANT_STATE`(`permission`) — 넷 전부 |

→ 구조 — [`ROOM.type.Version`](#ROOM.type.Version) · [`ROOM.op.PARTICIPANT_EVENT`](#ROOM.op.PARTICIPANT_EVENT) · [`TRK.op.TRACK_EVENT`](06-trk.md#TRK.op.TRACK_EVENT) · [`TRK.op.TRACK_STATE`](06-trk.md#TRK.op.TRACK_STATE) · [`ROOM.op.PARTICIPANT_STATE`](#ROOM.op.PARTICIPANT_STATE)

<a id="RULE-ROOM-VERSION-NOT-CARRIED"></a>**RULE-ROOM-VERSION-NOT-CARRIED** · 규칙

**안 싣는다** — ★**겹치는 payload 를 나르는 broadcast 가 없으면 되감길 대상이 없다.**

| 프레임 | 왜 |
|---|---|
| ★`AFFILIATION` 응답 | `sub_rooms`·`pub_room` 은 그 payload 를 나르는 broadcast 가 **없다**. ★**`versions{}` 필드는 폐기했다** |
| ★`ROOM_EVENT{affiliation}` | 같음. ★**`version` 필드 없음** |
| ★`ROOM_EVENT{sync_required}` | ★**payload 자체가 없다**(`reason` 뿐) |
| ★`LINK_EVENT` | ★**방 상태가 아니다** — 서버 하나의 연결 상태다(§4-2-6). 견주는 것은 `version` 이 아니라 ★**세대(`ufrag`)** 다 |
| ★`MLINE_EVENT` | ★**방 상태가 아니다** — 연결 하나의 배정표다(§8-1-2). 견주는 것은 `version` 이 아니라 ★**`arev`** 다(§8-1-2) |
| ★투명 참가자 본인의 `PARTICIPANT_STATE{permission}` | ★**명단 밖이라 `seq` 가 안 오른다**(RULE-ROOM-HIDDEN-NO-SEQ) — `version` **없이** 나에게만 온다 · 남에게는 안 간다(DEV-ROOM-HIDDEN-SELF-PSTATE) |
| ★`ROOM_LEAVE` 응답 | ★**종결이다** — 명단도 트랙도 안 싣고 그 방 보관본은 버린다(§4-2-3). 재설정할 스냅샷이 없다 |

<a id="RULE-ROOM-AFF-FRESHNESS"></a>**RULE-ROOM-AFF-FRESHNESS** · 규칙

★**`AFFILIATION` 응답의 신선도는 전송 계층이 지킨다** — §2-2-1 가 윈도우 기본 1 이고 *"서버는 받은 순서대로
처리한다"* 를 못박았으며, `pub_room` 은 ★**출처가 나 하나**다. 경주가 성립할 두 경로가 없다.

<a id="RULE-ROOM-VERSION-CLIENT-RULES"></a>**RULE-ROOM-VERSION-CLIENT-RULES** · 규칙 — ★클라 규칙 — 다섯 줄

1. epoch 가 다르다 → ★그 방 보관본을 통째로 버리고 재구축한다

→ 구조 — [`ROOM.http.GET_ROOM`](#ROOM.http.GET_ROOM)

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`ROOM.C.JOINED.delta_epoch`](#ROOM.C.JOINED.delta_epoch) · [`ROOM.C.JOINED.snapshot_epoch`](#ROOM.C.JOINED.snapshot_epoch) · [`ROOM.C.JOINED.resume_epoch`](#ROOM.C.JOINED.resume_epoch) · [`ROOM.S.restart`](#ROOM.S.restart) |
| 2 | [`ROOM.C.JOINING.join_ok`](#ROOM.C.JOINING.join_ok) · [`ROOM.C.JOINED.snapshot_short`](#ROOM.C.JOINED.snapshot_short) · [`ROOM.C.JOINED.snapshot_behind`](#ROOM.C.JOINED.snapshot_behind) · [`ROOM.C.JOINED.snapshot`](#ROOM.C.JOINED.snapshot) · [`ROOM.C.JOINED.resume_snapshot`](#ROOM.C.JOINED.resume_snapshot) |
| 3 | [`ROOM.C.JOINED.delta_old`](#ROOM.C.JOINED.delta_old) |
| 4 | [`ROOM.C.JOINED.delta_apply`](#ROOM.C.JOINED.delta_apply) |
| 5 | [`ROOM.C.JOINED.delta_gap_inflight`](#ROOM.C.JOINED.delta_gap_inflight) · [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) |

<a id="RULE-ROOM-REJOIN-RESET"></a>**RULE-ROOM-REJOIN-RESET** · 규칙

★**방을 나갔다 다시 들어오면** 그 방 카운터를 ★**버리고** `ROOM_JOIN` 응답 값으로 시작한다.

→ 구조 — [`ROOM.C.LEAVING.leave_ok`](#ROOM.C.LEAVING.leave_ok)

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`ROOM.C.JOINED.delta_epoch`](#ROOM.C.JOINED.delta_epoch) · [`ROOM.C.JOINED.snapshot_epoch`](#ROOM.C.JOINED.snapshot_epoch) · [`ROOM.C.JOINED.resume_epoch`](#ROOM.C.JOINED.resume_epoch) · [`ROOM.S.restart`](#ROOM.S.restart) |
| 2 | [`ROOM.C.JOINING.join_ok`](#ROOM.C.JOINING.join_ok) · [`ROOM.C.JOINED.snapshot_short`](#ROOM.C.JOINED.snapshot_short) · [`ROOM.C.JOINED.snapshot_behind`](#ROOM.C.JOINED.snapshot_behind) · [`ROOM.C.JOINED.snapshot`](#ROOM.C.JOINED.snapshot) · [`ROOM.C.JOINED.resume_snapshot`](#ROOM.C.JOINED.resume_snapshot) |
| 3 | [`ROOM.C.JOINED.delta_old`](#ROOM.C.JOINED.delta_old) |
| 4 | [`ROOM.C.JOINED.delta_apply`](#ROOM.C.JOINED.delta_apply) |
| 5 | [`ROOM.C.JOINED.delta_gap_inflight`](#ROOM.C.JOINED.delta_gap_inflight) · [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) |

<a id="RULE-ROOM-TERMINAL-NO-COMPARE"></a>**RULE-ROOM-TERMINAL-NO-COMPARE** · 규칙

★**종결은 견주지 않는다** — `ROOM_EVENT{affiliation}` 의 `cause` 가 `kick`·`room_closed` 인 경우(둘이 전부다, §4-2-6).
★**갱신이 아니라 폐기라 되감길 뒤가 없다.**

→ 구조 — [`ROOM.C.JOINED.kicked`](#ROOM.C.JOINED.kicked)

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`ROOM.C.JOINED.delta_epoch`](#ROOM.C.JOINED.delta_epoch) · [`ROOM.C.JOINED.snapshot_epoch`](#ROOM.C.JOINED.snapshot_epoch) · [`ROOM.C.JOINED.resume_epoch`](#ROOM.C.JOINED.resume_epoch) · [`ROOM.S.restart`](#ROOM.S.restart) |
| 2 | [`ROOM.C.JOINING.join_ok`](#ROOM.C.JOINING.join_ok) · [`ROOM.C.JOINED.snapshot_short`](#ROOM.C.JOINED.snapshot_short) · [`ROOM.C.JOINED.snapshot_behind`](#ROOM.C.JOINED.snapshot_behind) · [`ROOM.C.JOINED.snapshot`](#ROOM.C.JOINED.snapshot) · [`ROOM.C.JOINED.resume_snapshot`](#ROOM.C.JOINED.resume_snapshot) |
| 3 | [`ROOM.C.JOINED.delta_old`](#ROOM.C.JOINED.delta_old) |
| 4 | [`ROOM.C.JOINED.delta_apply`](#ROOM.C.JOINED.delta_apply) |
| 5 | [`ROOM.C.JOINED.delta_gap_inflight`](#ROOM.C.JOINED.delta_gap_inflight) · [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) |

<a id="COND-ROOM-SIMPLE-U64"></a>**COND-ROOM-SIMPLE-U64** · 성립 조건

★**왜 단순 `u64` 로 되나** — 방 하나는 미디어 서버 하나가 맡고 ★**방이 서버 사이를 옮겨 다니지
않는다.** 그래서 발급자가 하나이고 총순서가 잡힌다. ★**성립 조건이 그것이다** — 한 방이 여러
서버에 걸치는 구조가 생기면 ★**이 축이 먼저 깨진다**(그때는 논리시계가 필요해진다).

<a id="RULE-ROOM-LEAVE-START"></a>**RULE-ROOM-LEAVE-START** · 규칙 — 앱이 나가려 한다 (`JOINED`)



항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`ROOM.C.JOINING.join_ok_quit`](#ROOM.C.JOINING.join_ok_quit) · [`ROOM.C.JOINING.app_leave_wait`](#ROOM.C.JOINING.app_leave_wait) · [`ROOM.C.JOINED.leave`](#ROOM.C.JOINED.leave) |
| 2 | [`ROOM.C.JOINED.leave`](#ROOM.C.JOINED.leave) |

<a id="RULE-ROOM-NOTE-ORDER"></a>**RULE-ROOM-NOTE-ORDER** · 규칙 — 통지를 받았다 (`ACTIVE` · 방은 `JOINED`)

★★**WS 통지는 순서가 같다** — ①ACK ②★`version` 판정(§4-2-3) ③반영.
★**②를 건너뛰면 낡은 것이 최신을 되감는다.**
★떠난 방(퇴장 · 강퇴 · 방 닫힘)에 진행 중이던 재동기의 응답은 반영하지 않는다(DEV-ROOM-RESYNC-ABANDON).

<a id="RULE-ROOM-JUDGE-TIER"></a>**RULE-ROOM-JUDGE-TIER** · 규칙

★★**판정은 내용과 무관한 층이다** — ★**`version` 이 있으면 그 다섯 줄로만 가른다.** 무엇이 실렸는지는
반영 단계가 볼 일이다. ★예외는 **종결 하나**(그 방을 내리는 `ROOM_EVENT`) — 그것은 애초에 `version` 이
없어(§4-2-3) 판정할 것이 없다. ★`version` 이 없는 프레임은 견주지 않는다: `ROOM_EVENT` 둘.
`MLINE_EVENT` 는 이 판정 밖이다 — `arev` 로 가른다(§8-1-2).

→ 구조 — [`ROOM.C.JOINED.delta_old`](#ROOM.C.JOINED.delta_old) · [`ROOM.C.JOINED.delta_apply`](#ROOM.C.JOINED.delta_apply) · [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) · [`ROOM.C.JOINED.delta_bare`](#ROOM.C.JOINED.delta_bare)

<a id="RULE-ROOM-FLOOR-NOT-HERE"></a>**RULE-ROOM-FLOOR-NOT-HERE** · 규칙

★**발언권(DC)은 여기 없다.** 축이 다르다 — ★**§10-4-3** 이고, 견주는 것도
`version` 이 아니라 ★**메시지 일련번호(`8`, §10-4-31)** 다.

<a id="RULE-ROOM-NOTE-TABLE"></a>**RULE-ROOM-NOTE-TABLE** · 규칙

| 통지 | 무엇을 하나 | 상태 |
|---|---|---|
| ★`TRACK_EVENT` | ACK + 판정 + `tracks[]` 를 보관본에 반영 + 표시 갱신. ★**구독 SDP 는 다시 짓지 않는다** — m-line 은 `MLINE_EVENT` 가 따로 고친다(§8-1-2). ★**같은 `track_id` 의 항목 교체**(회의↔무전 전환 — `active:false` 면 개인 화면에서 내리고, `active:true` 면 다시 건다, §4-2-6) | 그대로 |
| ★`MLINE_EVENT` | ACK + ★**`arev` 가 보관값보다 크면 배정표를 통째로 갈아 끼우고** 구독 SDP 를 다시 조립한다(§9-4-7 · ★`1pc` 는 §9-2-1) → 협상 성공하면 `READY{arev}`(§8-1-2) | 그대로 |
| `TRACK_STATE{muted}` | ACK + 판정 + ★**보관본의 `TrackEntry.muted` 갱신** + 표시 갱신 | 그대로 |
| ★`PARTICIPANT_STATE{permission}` | ACK + 판정 + ★**보관본의 `MemberInfo.permission` 갱신**(기본이면 필드를 지운다, §4-1-3) + 표시 갱신(버튼 잠금·해제). ★본인이면 내려간 비트의 자동 발행을 멈춘다(§4-2-6) | 그대로 |
| ★`ROOM_EVENT{affiliation}` | ACK + ★**결말을 먼저 본다**(§4-2-3 종결 — `version` 이 없어 견줄 것이 없다) — ★`cause` 는 `kick`·`room_closed` 둘뿐이고 둘 다 종결이다(§4-2-3) — ★그 방을 곧바로 `NONE` 으로 내리고 앱에 알린다. 실린 `sub_rooms` 는 그 서버가 아는 부분 목록이라 ★**전체로 덮어쓰지 않는다**. ★`room_closed` 는 방 자체가 없어진 것이라(서버 급사 포함) 재입장하면 `3001` 이다 — 방을 다시 만들지는 앱이 정한다(재생성은 앱 백엔드 `POST /rooms`, §4-1-13). 전면 재구축(§3-2-3)은 그 재생성 뒤에 선다 | ★방 상태가 바뀔 수 있다 |
| ★`ROOM_EVENT{sync_required}` | ACK + ★**`GET /rooms/{room_id}?tracks=1`(§4-1-14)** 로 통짜로 다시 받아 ★**보관값을 재설정한 뒤**(§4-2-3 규칙 2 — 낡은 응답은 버린다, DEV-ROOM-SNAPSHOT-BEHIND) 반영 → 달라졌으면 구독 SDP 재조립 + `READY`. 같은 방 재통보는 **30초 쿨다운** | 그대로 |
| ★`LINK_EVENT` | ACK + ★**세대(`ufrag`)를 견준다** — 지금 쓰는 세대가 아니면 버린다. 맞으면 그 서버 연결의 복구 방아쇠로 넣는다(§9-2-1). `version` 이 없어 방 보관본과 무관하다 | 그대로(방) |
| `MESSAGE` | ACK + 앱에 전달 | 그대로 |
| `TASK{phase:"request"}` | ★**ACK 부터**, 그 다음 일을 하고 `phase:"report"` 로 보낸다 | 그대로 |

→ 구조 — [`ROOM.C.JOINED.delta_old`](#ROOM.C.JOINED.delta_old) · [`ROOM.C.JOINED.delta_apply`](#ROOM.C.JOINED.delta_apply) · [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) · [`ROOM.C.JOINED.delta_gap_inflight`](#ROOM.C.JOINED.delta_gap_inflight) · [`ROOM.op.PARTICIPANT_EVENT`](#ROOM.op.PARTICIPANT_EVENT)

<a id="RULE-ROOM-SEQ-GAP"></a>**RULE-ROOM-SEQ-GAP** · 규칙 — ★`version.seq` 가 건너뛰었다

3. `tracks` 가 달라졌으면 구독 SDP 재조립 + `READY`.

→ 구조 — [`ROOM.C.JOINED.delta_gap_inflight`](#ROOM.C.JOINED.delta_gap_inflight) · [`ROOM.http.GET_ROOM`](#ROOM.http.GET_ROOM)

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) |
| 2 | [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) |
| 3 | [`ROOM.C.JOINED.snapshot_short`](#ROOM.C.JOINED.snapshot_short) · [`ROOM.C.JOINED.snapshot`](#ROOM.C.JOINED.snapshot) |
| 4 | [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) · [`ROOM.C.JOINED.snapshot`](#ROOM.C.JOINED.snapshot) |

<a id="RULE-ROOM-MANY-ROOMS-ONCE"></a>**RULE-ROOM-MANY-ROOMS-ONCE** · 규칙

★**여러 방이 한꺼번에 어긋났을 때** — 다 반영한 뒤 ★**한 번만 조립한다**(§4-1-14).
몇 개를 언제 던지느냐는 ★**구현이 정한다** — `version` 을 견주기만 하면 상태가 안 깨진다.

→ 구조 — [`LINK.C.any.renego`](09-link.md#LINK.C.any.renego)

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) |
| 2 | [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) |
| 3 | [`ROOM.C.JOINED.snapshot_short`](#ROOM.C.JOINED.snapshot_short) · [`ROOM.C.JOINED.snapshot`](#ROOM.C.JOINED.snapshot) |
| 4 | [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap) · [`ROOM.C.JOINED.snapshot`](#ROOM.C.JOINED.snapshot) |

<a id="RULE-ROOM-TIMERS-MEDIA"></a>**RULE-ROOM-TIMERS-MEDIA** · 규칙 — 미디어

| 이름 | 시작 | 정상 정지 | 끝나면 |
|---|---|---|---|
| `T-stall`(서버) | ★같은 방에 `ROOM_EVENT{sync_required}` 를 보냈다 | — | 다시 알릴 수 있다(쿨다운) |
| ★재동기 in-flight(클라) | ★그 방 `GET /rooms/{id}?tracks=1` 을 던졌다 | 응답이 왔다 | ★**그 방에 새 재동기를 안 던진다**(§4-2-3 규칙 5). 갭은 한 번에 여러 통지로 이어지므로 억제가 없으면 ★**한 번 놓친 것이 왕복 여러 번**이 된다. 값이 아니라 **상태**다 |
★재동기 in-flight 의 예외 둘 — 버린 델타가 응답보다 앞서면 한 번 더 받는다(DEV-ROOM-INFLIGHT-AHEAD) · 방을 떠나면 그 재동기를 버린다(DEV-ROOM-RESYNC-ABANDON).

→ 구조 — [`ROOM.state`](#ROOM.state)

<a id="PROP-ROOM-ONE-RESYNC"></a>**PROP-ROOM-ONE-RESYNC** · 성질

한 방에 진행 중인 재동기는 하나다(지금 입장의 것).

*식: RH3 · 깨지면: 한 번 놓친 것이 왕복 여러 번이 된다(§4-2-3)*

<a id="DEV-ROOM-INFLIGHT-AHEAD"></a>**DEV-ROOM-INFLIGHT-AHEAD** · 자체 확장

재동기가 진행 중일 때 버린 갭 델타의 가장 큰 `seq` 를 기억한다. 응답의 `seq` 가 그보다 낮으면 응답을 반영하고 한 번 더 받는다 — 진행 중 재동기는 여전히 하나다

*원문: §4-2-3 규칙 5 · RULE-ROOM-TIMERS-MEDIA(재동기 in-flight 행) — 진행 중이면 새로 던지지 않는다 · 근거: RULE-ROOM-SEQ-GAP RULE-ROOM-TIMERS-MEDIA PROP-ROOM-CONVERGE*

<a id="DEV-ROOM-RESYNC-ABANDON"></a>**DEV-ROOM-RESYNC-ABANDON** · 자체 확장

방을 떠나면(`ROOM_LEAVE` 를 보냈다 · 강퇴 · 방 닫힘) 그 방에 진행 중인 재동기를 버린다 — 그 요청의 응답이 와도 반영하지 않는다(HTTP 응답은 자기 요청에 묶인다). 다시 들어간 방의 재동기는 새로 센다

*원문: RULE-ROOM-TIMERS-MEDIA(재동기 in-flight 행) · §4-2-3 · §4-2-3 — 떠날 때의 재동기를 정하지 않는다 · 근거: RULE-ROOM-TIMERS-MEDIA PROP-ROOM-ONE-RESYNC*

<a id="ROOM.C.any"></a>
#### 4-2-4. 클라 · `any`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="ROOM.C.any.leave_res_stale"></a>`ROOM.C.any.leave_res_stale` | rs != 'LEAVING' | — | §4-2-5 — LEAVING 이 아닌데 온 응답 — 원문이 정하지 않는다(무시) |
| <a id="ROOM.C.any.delta_not_joined"></a>`ROOM.C.any.delta_not_joined` | rs != 'JOINED' | — | §4-2-3 — §4-2-3 은 JOINED 방의 것 — 그 밖(LEAVING 등)에서 받은 통지는 ACK 만(원문이 정하지 않는다) |
| <a id="ROOM.C.any.room_event_not_joined"></a>`ROOM.C.any.room_event_not_joined` | rs != 'JOINED' | — | §4-2-3 |
| <a id="ROOM.C.any.snapshot_stale"></a>`ROOM.C.any.snapshot_stale` | rs != 'JOINED' \|\| !rsync | — | §4-1-14 — 그 재동기를 던진 입장이 아니다 — 원문이 정하지 않는다(무시) |
| <a id="ROOM.C.any.sub_rooms_ok"></a>`ROOM.C.any.sub_rooms_ok` | 받는 효과 ROOM room.sub_rooms · 나머지 | — | §4-1 — 목록에 있다 · 아직 들어가 있지 않다(입장 중이면 제 응답이 따로 온다) — 할 일이 없다 |
| <a id="ROOM.C.any.down"></a>`ROOM.C.any.down` | 받는 효과 SES rooms.down_all · LINK store.drop_server · 나머지 | rs ← 'NONE'; cseq ← 0; cepoch ← 0; rsync ← false; ahead ← 0; retry ← false; rejoin ← rs == 'JOINED' \|\| (rs == 'JOINING' && !quit) · 효과 store.drop → TRK.C.any.store_drop · 효과 http.abandon · 효과 ws.abandon · 효과 aff.leave → AFF.C.any.left · AFF.C.any.left_join · AFF.C.any.left_want · AFF.C.any.left_other · 효과 sub.drop → SUB.C.any.drop | §3-2-3 2 §9-2-1 2 DEV-ROOM-RESYNC-ABANDON — 방 상태를 NONE 으로 · 보관본을 버린다 · 떠 있는 재동기는 버린다(방을 떠난 것과 같다) |
| <a id="ROOM.C.any.close_all"></a>`ROOM.C.any.close_all` | 받는 효과 SES rooms.close_all · 나머지 | rs ← 'NONE'; cseq ← 0; cepoch ← 0; rsync ← false; ahead ← 0; retry ← false; rejoin ← false · 효과 store.drop → TRK.C.any.store_drop · 효과 http.abandon · 효과 ws.abandon · 효과 aff.leave → AFF.C.any.left · AFF.C.any.left_join · AFF.C.any.left_want · AFF.C.any.left_other · 효과 sub.drop → SUB.C.any.drop | §3-2-3 2 DEV-SES-FRESH-REJOIN DEV-ROOM-RESYNC-ABANDON — 방 상태를 NONE 으로 · 보관본을 버린다 · 떠 있는 재동기는 버린다 · 다시 들어갈 방으로 남기지 않는다 |

<a id="ROOM.C.LEAVING"></a>
#### 4-2-5. 클라 · `LEAVING`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="ROOM.C.LEAVING.leave_ok"></a>`ROOM.C.LEAVING.leave_ok` | m.part == 'res' | rs ← 'NONE'; cseq ← 0; cepoch ← 0; rsync ← false · 효과 sdp.rebuild → LINK.C.any.renego_queued · LINK.C.any.renego · 효과 store.drop → TRK.C.any.store_drop · 효과 app.left · 효과 link.room_left → LINK.C.any.last_room_left · LINK.C.any.room_left | §4-2-5 1 §4-2-5 2 §4-2-5 3 |
| <a id="ROOM.C.LEAVING.leave_fail"></a>`ROOM.C.LEAVING.leave_fail` | 나머지 | rs ← 'NONE'; cseq ← 0; cepoch ← 0; rsync ← false · 효과 sdp.rebuild → LINK.C.any.renego_queued · LINK.C.any.renego · 효과 store.drop → TRK.C.any.store_drop · 효과 app.left · 효과 link.room_left → LINK.C.any.last_room_left · LINK.C.any.room_left | §4-2-5 — 3001 · 3002 는 성공과 같이 · 그 밖도 NONE 으로 내린다 |
| <a id="ROOM.C.LEAVING.down"></a>`ROOM.C.LEAVING.down` | 받는 효과 SES rooms.down_all · LINK store.drop_server · rs == 'LEAVING' | rs ← 'NONE'; cseq ← 0; cepoch ← 0; rsync ← false; ahead ← 0; retry ← false; rejoin ← false · 효과 store.drop → TRK.C.any.store_drop · 효과 http.abandon · 효과 ws.abandon · 효과 aff.leave → AFF.C.any.left · AFF.C.any.left_join · AFF.C.any.left_want · AFF.C.any.left_other · 효과 sub.drop → SUB.C.any.drop · 효과 app.left | §3-2-3 2 §4-2-5 — 나가던 방이 재구축으로 내려갔다 — 나가는 일은 끝난 것이다(서버는 연결과 함께 그 방의 Peer 를 거둔다) · ★앱의 퇴장을 끝낸다(app.left — 20261023: 이것이 없어 leave() 가 영영 안 끝났다) · 다시 들어가지 않는다 |

<a id="RULE-ROOM-LEAVE-RES"></a>**RULE-ROOM-LEAVE-RES** · 규칙 — `ROOM_LEAVE` 응답을 받았다 (`LEAVING`)

1. 그 방의 구독 m-line 을 정리하고 **구독 SDP 를 다시 조립한다.**
   ★**m-line 을 없애지 않는다** — `a=inactive` 로 두고 BUNDLE 목록엔 남긴다(§9-4-11).
   ★`1pc` 면 §9-2-1 절차를 탄다.

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.any.room_left`](09-link.md#LINK.C.any.room_left) · [`ROOM.C.LEAVING.leave_ok`](#ROOM.C.LEAVING.leave_ok) |
| 2 | [`LINK.C.any.ice_disc_leaving`](09-link.md#LINK.C.any.ice_disc_leaving) · [`LINK.C.any.wait_over_leaving`](09-link.md#LINK.C.any.wait_over_leaving) · [`LINK.C.any.assoc_lost_leaving`](09-link.md#LINK.C.any.assoc_lost_leaving) · [`LINK.C.any.dc_closed_leaving`](09-link.md#LINK.C.any.dc_closed_leaving) · [`LINK.C.any.last_room_left`](09-link.md#LINK.C.any.last_room_left) · [`PUB.C.any.drop_server`](07-pub.md#PUB.C.any.drop_server) · [`ROOM.C.LEAVING.leave_ok`](#ROOM.C.LEAVING.leave_ok) |
| 3 | [`ROOM.C.LEAVING.leave_ok`](#ROOM.C.LEAVING.leave_ok) |

<a id="RULE-ROOM-ON-LEAVE-FAIL"></a>**RULE-ROOM-ON-LEAVE-FAIL** · 규칙

★**실패 응답이면** — `3001`·`3002`(이미 없는 방 · 미입장)는 ★**성공과 같이 다룬다**(방 `NONE`, m-line 비움 — 서버에 없는 방을 붙들 이유가 없다). 그 밖(`5xxx`)은 §11-2-1 의 재시도 뒤 앱에 알리되 ★방은 `LEAVING` 에 두지 않고 `NONE` 으로 내린다 — 서버 회수(세션 창, §1-2-6)가 뒤를 맡는다.

→ 구조 — [`ROOM.C.LEAVING.leave_fail`](#ROOM.C.LEAVING.leave_fail)

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.any.room_left`](09-link.md#LINK.C.any.room_left) · [`ROOM.C.LEAVING.leave_ok`](#ROOM.C.LEAVING.leave_ok) |
| 2 | [`LINK.C.any.ice_disc_leaving`](09-link.md#LINK.C.any.ice_disc_leaving) · [`LINK.C.any.wait_over_leaving`](09-link.md#LINK.C.any.wait_over_leaving) · [`LINK.C.any.assoc_lost_leaving`](09-link.md#LINK.C.any.assoc_lost_leaving) · [`LINK.C.any.dc_closed_leaving`](09-link.md#LINK.C.any.dc_closed_leaving) · [`LINK.C.any.last_room_left`](09-link.md#LINK.C.any.last_room_left) · [`PUB.C.any.drop_server`](07-pub.md#PUB.C.any.drop_server) · [`ROOM.C.LEAVING.leave_ok`](#ROOM.C.LEAVING.leave_ok) |
| 3 | [`ROOM.C.LEAVING.leave_ok`](#ROOM.C.LEAVING.leave_ok) |

<a id="ROOM.S"></a>
#### 4-2-6. 서버

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="ROOM.S.join_3001"></a>`ROOM.S.join_3001` | m.t == 'ROOM_JOIN' && !open | 답한다 ROOM_JOIN.fail {code: 3001} | §4-1 |
| <a id="ROOM.S.join_4009"></a>`ROOM.S.join_4009` | m.t == 'ROOM_JOIN' && busy && !member | 답한다 ROOM_JOIN.fail {code: 4009} | §11-2-3 |
| <a id="ROOM.S.join_4001"></a>`ROOM.S.join_4001` | m.t == 'ROOM_JOIN' && full && !member && !hidden | 답한다 ROOM_JOIN.fail {code: 4001} | §4-1 §4-1-3 |
| <a id="ROOM.S.join_again"></a>`ROOM.S.join_again` | m.t == 'ROOM_JOIN' && member | sel ← m.select · 답한다 ROOM_JOIN.res {version: {epoch: epoch, seq: seq}, server_config: {pc_mode: pcm, sfu_id: epoch}} | §4-1 |
| <a id="ROOM.S.join_hidden"></a>`ROOM.S.join_hidden` | m.t == 'ROOM_JOIN' && hidden | member ← true; sel ← m.select · 답한다 ROOM_JOIN.res {version: {epoch: epoch, seq: seq}, server_config: {pc_mode: pcm, sfu_id: epoch}} | §4-1 §4-1-3 |
| <a id="ROOM.S.join"></a>`ROOM.S.join` | m.t == 'ROOM_JOIN' | member ← true; seq ← seq + 1; sel ← m.select · 답한다 ROOM_JOIN.res {version: {epoch: epoch, seq: seq + 1}, server_config: {pc_mode: pcm, sfu_id: epoch}} | §4-1 §4-1-6 |
| <a id="ROOM.S.leave_3002"></a>`ROOM.S.leave_3002` | m.t == 'ROOM_LEAVE' && !member | 답한다 ROOM_LEAVE.fail {code: 3002} | §4-1 |
| <a id="ROOM.S.leave_hidden"></a>`ROOM.S.leave_hidden` | m.t == 'ROOM_LEAVE' && hidden | member ← false · 답한다 ROOM_LEAVE.res | §4-1 §4-1-3 |
| <a id="ROOM.S.leave"></a>`ROOM.S.leave` | m.t == 'ROOM_LEAVE' | member ← false; seq ← seq + 1 · 답한다 ROOM_LEAVE.res | §4-1 §4-1-6 |
| <a id="ROOM.S.rx_other"></a>`ROOM.S.rx_other` | 나머지 | — | §4-1 |
| <a id="ROOM.S.session_gone_hidden"></a>`ROOM.S.session_gone_hidden` | 받는 효과 SES rooms.session_gone · member && hidden | member ← false | §3-1 §4-1-3 |
| <a id="ROOM.S.session_gone"></a>`ROOM.S.session_gone` | 받는 효과 SES rooms.session_gone · member | member ← false; seq ← seq + 1 | §3-1 §4-1-6 |
| <a id="ROOM.S.session_gone_other"></a>`ROOM.S.session_gone_other` | 받는 효과 SES rooms.session_gone · 나머지 | — | §3-1 |
| <a id="ROOM.S.peer_evicted_self"></a>`ROOM.S.peer_evicted_self` | 받는 효과 ROOM room.peer_evicted · self | — | §4-1 |
| <a id="ROOM.S.peer_evicted_hidden"></a>`ROOM.S.peer_evicted_hidden` | 받는 효과 ROOM room.peer_evicted · member && hidden | member ← false | §4-1 §4-1-3 |
| <a id="ROOM.S.peer_evicted"></a>`ROOM.S.peer_evicted` | 받는 효과 ROOM room.peer_evicted · member | member ← false; seq ← seq + 1 | §4-1 §4-1-6 |
| <a id="ROOM.S.peer_evicted_other"></a>`ROOM.S.peer_evicted_other` | 받는 효과 ROOM room.peer_evicted · 나머지 | — | §4-1 |
| <a id="ROOM.S.change_notify"></a>`ROOM.S.change_notify` | member | seq ← seq + 1 · 답한다 PARTICIPANT_EVENT.body {version: {epoch: epoch, seq: seq + 1}} | §4-1-6 §4-2-6 |
| <a id="ROOM.S.change"></a>`ROOM.S.change` | 나머지 | seq ← seq + 1 | §4-1-6 |
| <a id="ROOM.S.kick_hidden"></a>`ROOM.S.kick_hidden` | hidden | member ← false · 답한다 ROOM_EVENT.body {type: 'affiliation', cause: 'kick'} | §4-2-6 §4-1-3 |
| <a id="ROOM.S.kick"></a>`ROOM.S.kick` | 나머지 | member ← false; seq ← seq + 1 · 답한다 ROOM_EVENT.body {type: 'affiliation', cause: 'kick'} | §4-2-6 §4-1-6 |
| <a id="ROOM.S.close_notify"></a>`ROOM.S.close_notify` | member | open ← false; member ← false · 답한다 ROOM_EVENT.body {type: 'affiliation', cause: 'room_closed'} | §4-2-6 |
| <a id="ROOM.S.close"></a>`ROOM.S.close` | 나머지 | open ← false | §4-2-6 |
| <a id="ROOM.S.reopen"></a>`ROOM.S.reopen` | 나머지 | open ← true; seq ← seq + 1 | §4-2-6 |
| <a id="ROOM.S.sync_required"></a>`ROOM.S.sync_required` | 나머지 | cool ← true · 답한다 ROOM_EVENT.body {type: 'sync_required'} | §4-2-6 §4-2-3 |
| <a id="ROOM.S.unbusy"></a>`ROOM.S.unbusy` | busy | busy ← false | §11-2-3 |
| <a id="ROOM.S.busy"></a>`ROOM.S.busy` | 나머지 | busy ← true | §11-2-3 |
| <a id="ROOM.S.full_flip"></a>`ROOM.S.full_flip` | 나머지 | full ← !full | §4-1 |
| <a id="ROOM.S.restart"></a>`ROOM.S.restart` | 나머지 | epoch ← epoch + 1; seq ← 1; member ← false; open ← false; cool ← false; pubBit ← true; floorBit ← true | §4-2-3 1 |
| <a id="ROOM.S.perm_absent"></a>`ROOM.S.perm_absent` | !member | — | §4-2-6 |
| <a id="ROOM.S.perm_floor_down_hidden"></a>`ROOM.S.perm_floor_down_hidden` | hidden && bit == 'floor' && floorBit | floorBit ← false · 답한다 PARTICIPANT_STATE.body {type: 'permission'} | §4-2-6 §4-1-3 |
| <a id="ROOM.S.perm_floor_up_hidden"></a>`ROOM.S.perm_floor_up_hidden` | hidden && bit == 'floor' | floorBit ← true · 답한다 PARTICIPANT_STATE.body {type: 'permission'} | §4-2-6 §4-1-3 |
| <a id="ROOM.S.perm_pub_down_hidden"></a>`ROOM.S.perm_pub_down_hidden` | hidden && pubBit | pubBit ← false · 답한다 PARTICIPANT_STATE.body {type: 'permission'} | §4-2-6 §4-1-3 |
| <a id="ROOM.S.perm_pub_up_hidden"></a>`ROOM.S.perm_pub_up_hidden` | hidden | pubBit ← true · 답한다 PARTICIPANT_STATE.body {type: 'permission'} | §4-2-6 §4-1-3 |
| <a id="ROOM.S.perm_floor_down"></a>`ROOM.S.perm_floor_down` | bit == 'floor' && floorBit | floorBit ← false; seq ← seq + 1 · 답한다 PARTICIPANT_STATE.body {type: 'permission', version: {epoch: epoch, seq: seq + 1}} | §4-2-6 §4-2-6 §4-2-6 |
| <a id="ROOM.S.perm_floor_up"></a>`ROOM.S.perm_floor_up` | bit == 'floor' | floorBit ← true; seq ← seq + 1 · 답한다 PARTICIPANT_STATE.body {type: 'permission', version: {epoch: epoch, seq: seq + 1}} | §4-2-6 §4-2-6 §4-2-6 |
| <a id="ROOM.S.perm_pub_down"></a>`ROOM.S.perm_pub_down` | pubBit | pubBit ← false; seq ← seq + 1 · 답한다 PARTICIPANT_STATE.body {type: 'permission', version: {epoch: epoch, seq: seq + 1}} | §4-2-6 §4-2-6 §4-2-6 |
| <a id="ROOM.S.perm_pub_up"></a>`ROOM.S.perm_pub_up` | 나머지 | pubBit ← true; seq ← seq + 1 · 답한다 PARTICIPANT_STATE.body {type: 'permission', version: {epoch: epoch, seq: seq + 1}} | §4-2-6 §4-2-6 §4-2-6 |
| <a id="ROOM.S.seq_up_track"></a>`ROOM.S.seq_up_track` | 받는 효과 TRK room.seq_up · 나머지 | seq ← seq + 1 | §4-1-6 §6-2-2 |
| <a id="ROOM.S.TStall"></a>`ROOM.S.TStall` | 나머지 | cool ← false | §4-2-3 |

<a id="RULE-ROOM-PERMISSION-MAP"></a>**RULE-ROOM-PERMISSION-MAP** · 규칙 — ★권한 비트 — 사상표 · 라벨 · 방 단위 · 수명

★★**비트마다 무엇을 막나 — 사상표.** 서버와 클라가 같은 표를 본다(`details.denied` 의 뜻이 여기서 선다).

<a id="RULE-ROOM-SEQ-CONTENT"></a>**RULE-ROOM-SEQ-CONTENT** · 규칙 — ★`seq++` 의 조건 — 배달이 아니라 **내용**이다

```
seq++  ⟺  ★그 방 공통 스냅샷이 바뀌었다
           = participants[] (MemberInfo, §4-1-3)
           + tracks[]       (TrackEntry 의 ★트랙 층 — type 불문, §6-2-2)
```

★★**보관본 내용에 매단다 — 누가 몇 장 받느냐가 아니다.** ★**발원도 안 가린다**(클라 요청이든
서버 정리든 각 장의 구조 이름 규율과 같다). 그래서 *"올렸으면 몇 명에게 보내라"* 같은 배달 규정이
★**필요 없어진다** — 공통 스냅샷이 바뀌면 그 방 전원의 보관본이 바뀌므로 델타가 저절로 전원에게 있다.

<a id="RULE-ROOM-SEQ-UP"></a>**RULE-ROOM-SEQ-UP** · 규칙

**오른다**

| 사건                                         | 무엇이 바뀌나                                                                                                                      |
| ------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------- |
| `ROOM_JOIN`                                | `participants[]` +1                                                                                                          |
| `ROOM_LEAVE`                               | `participants[]` −1 + 그 사람 트랙 정리                                                                                             |
| ★전이중 `PUBLISH_TRACKS{add/remove}`          | `tracks[]` 증감                                                                                                                |
| ★`TRACK_SET{duplex}`                       | `TrackEntry` 의 `active`(·`source`·`mid` 자리) — ★**`TRACK_EVENT{add}` 로 항목이 교체되는 것 한 번**이다(§4-2-6). `TRACK_STATE{duplex}` 는 폐기했다 |
| ★전이중 트랙을 든 `AFFILIATION`(같은 서버 안 발행 방 옮김) | 옛 방 `tracks[]` −1 · 새 방 +1 — ★**두 방 다** 오른다(§4-2-6 · §5-1-2) |
| ★`TRACK_SET{muted}`                        | `TrackEntry` 의 `muted`(§6-1-1)                                                                                                 |
| ★`permission` 변경(§4-2-6 `PARTICIPANT_STATE`) | `MemberInfo.permission`(§4-1-3) — 명단 필드다. ★`hidden` 참가자의 것은 명단에 없어 안 오른다(§4-1-3)                                                 |
| ★**서버 정리 전량**                              | 세션 폐기(창 만료·운영자 절단 — Peer 가 함께 간다, §3-1) · 세션 축출(`LEAVE` `2010`, §3-1) · 방 재입장 축출(§4-1) · 강퇴                   |

<a id="RULE-ROOM-SEQ-NOT-UP"></a>**RULE-ROOM-SEQ-NOT-UP** · 규칙

**안 오른다**

| 사건                                                       | 왜                                                                                             |
| -------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `READY`                                          | 게이트 해제·키프레임 요청 · 정체 판정 시작뿐 — 방 공통 스냅샷을 안 바꾼다(§7-1-2)                                             |
| ★반이중 `PUBLISH_TRACKS{add/remove}`                        | ★**반이중 개인 트랙은 구독자 스냅샷에 없다**(§6-2-2) · ★슬롯은 방이 생길 때부터 늘 있어 반이중 발행이 슬롯을 만들지도 지우지도 않는다(§6-2-2) |
| ★배정표만 바뀌는 일(`MLINE_EVENT` — 빈 줄 채움 · 재배정) | ★**연결의 배정표만 바뀐다** — 방 공통 스냅샷이 아니다(§8-1-2) |
| ★반이중 · 트랙 없는 `AFFILIATION`(`pub_select`·`pub_deselect`) | ★**스냅샷에 `pub_room` 이 없다** — 그 값은 §5-1-1 이 **내 세션 것**이라 정했고 남이 볼 자리가 없다 ★전이중 트랙을 들고 옮기면 위 '오른다' 표다 |
| `SUBSCRIBE_LAYER`                                        | 내 수신 설정뿐                                                                                      |
| ★`hidden` 참가자 입퇴장·`permission` 변경(§4-1-3) — ★**트랙이 없을 때** | 명단에 안 오른다 — 공통 스냅샷이 안 바뀐다. ★그 사람의 `individual` 트랙이 있었으면 그 `remove` 는 오른다(트랙 층)              |
| `BIND`·`RESUME`·`HEARTBEAT`·`MESSAGE`·`TASK`             | 상태를 안 바꾼다 — ★단 같은 신원의 새 세션 `BIND` 는 옛 세션을 축출하고, 그 정리가 옛 세션 방들의 seq 를 올린다(위 '오른다' 표의 서버 정리 전량 · 세션 축출)                                                                                     |

→ 구조 — [`ROOM.S.seq_up_track`](#ROOM.S.seq_up_track) · [`SES.S.bind_new`](03-ses.md#SES.S.bind_new) · [`TRN.oplist`](02-trn.md#TRN.oplist)

<a id="RULE-ROOM-PUBROOM-NOT-IN-SNAPSHOT"></a>**RULE-ROOM-PUBROOM-NOT-IN-SNAPSHOT** · 규칙

★★**`pub_room` 이 안 오르는 근거는 "스냅샷에 그 값이 없어서"다** — *"남에게 갈 프레임이 없어서"*(배달 축)가
아니다. 그래서 ★**`version` 을 실을 이유도 없다**(§4-2-3).

<a id="RULE-ROOM-HALF-FULL-SPLIT"></a>**RULE-ROOM-HALF-FULL-SPLIT** · 규칙

★★**반이중과 전이중은 여기서 갈린다**(§5-1-2 전환표의 근거) —
★**전이중**은 `PUBLISH_TRACKS{remove}`(옛 방) → `{add}`(새 방)라 ★**두 방 다 `tracks[]` 가 바뀌어 둘 다 오른다.**
★**반이중**은 `AFFILIATION` 하나뿐이라 ★**두 방 다 안 오른다** — ★단 쉬던 전이중 트랙(`TRACK_SET{duplex:"half"}` 로 돌린 것 — 쉬는 항목이 있다)을 들고 옮기면 그 트랙도 따라가 옛 방 `remove` · 새 방 `add`(`active:false`)로 ★**두 방 다 오른다**(§5-1-2 — 든 트랙 전부가 새 방으로 간다). 무전 트랙은 개인 것이 아니라 ★**방의 것**이고
(`ptt-{room}-audio` 는 화자가 떠나도 그대로다) ★**내 마이크 트랙은 애초에 남의 `tracks[]` 에 나타난 적이 없다.**
남이 볼 변화는 ★**발언권 축(DC)** 에 있고 그것은 `version` 이 아니라 일련번호 `8` 을 달고 간다(§10-4-31).

<a id="RULE-ROOM-ONE-COUNTER"></a>**RULE-ROOM-ONE-COUNTER** · 규칙

★★**네 통지 op 은 방마다 한 카운터를 공유한다** — `PARTICIPANT_EVENT`·`TRACK_EVENT`·`TRACK_STATE`·`PARTICIPANT_STATE` 가
★**같은 번호줄**을 쓴다. ★**op 별 카운터로 구현하면 안 된다**: `PARTICIPANT_EVENT` 만 보면 `11 → 14` 처럼
건너뛴 것으로 보여 ★**화자가 말할 때마다 방 전원이 재동기를 던진다.**

→ 구조 — [`ROOM.C.JOINED.delta_apply`](#ROOM.C.JOINED.delta_apply) · [`ROOM.C.JOINED.delta_gap`](#ROOM.C.JOINED.delta_gap)

<a id="RULE-ROOM-DUPLEX-ONE-ADD"></a>**RULE-ROOM-DUPLEX-ONE-ADD** · 규칙

★★**회의↔무전 전환은 `TRACK_EVENT{add}` 하나다** — 잔존 항목 교체(`active:false`) 또는 복귀(`active:true`, 새 `mid` 일 수 있다)로
한 번 나가고 `seq` 도 한 번 오른다. ★`TRACK_STATE{duplex}` 를 두지 않는다 — 같은 사실을 `version` 을 실은 채 번호를 안 올려 두 번 보내는
프레임이라(규칙 3 에 걸려 늘 버려진다).

<a id="RULE-ROOM-MAX-100"></a>**RULE-ROOM-MAX-100** · 규칙

★**동시에 들어가 있을 수 있는 방은 최대 100 개다.** 넘으면 ★`4004 LISTEN_LIMIT`
(`details.limit`·`details.current`)으로 거절된다. ★**미디어 서버마다 따로 센다** — 근거가
한 BUNDLE(=서버당 연결)의 m-line 이기 때문이다(아래). 세션 전역 상한은 두지 않는다. ★**설계 하중의 기준은 지령대 10~20 채널**이고
(§4-4-1), 100 은 방 수의 안전 상한이다. ★**m-section 예산의 보증이 아니다** — 먼저 닿는 것은 슬롯 몫(서버 정책, 기본 40)의 `4005` 다. ★**슬롯 방은 늘 2 를 먹는다**(audio · video — 슬롯 둘은 방이 생길 때부터 늘 있다 · 쓰고 안 쓰고는 화자 재량이다, §6-2-2). 그래서 ★**입장 뒤에 몫이 넘치는 길이 없다** — `4005` 은 입장에서만 난다.

→ 구조 — [`ROOM.ledger.MaxRooms`](#ROOM.ledger.MaxRooms) · [`ROOM.ledger.SlotBudget`](#ROOM.ledger.SlotBudget) · [`ROOM.op.ROOM_JOIN`](#ROOM.op.ROOM_JOIN)

<a id="RULE-ROOM-BUNDLE-128"></a>**RULE-ROOM-BUNDLE-128** · 규칙

★★**진짜 천장은 방 수가 아니라 한 BUNDLE 의 m-section 개수이고, 값은 128 이다.** Firefox 155 실측 — 128 절까지 받고
129 절부터 초과분을 ★**BUNDLE 밖으로 내보낸다.** Chromium 149 는 4,100 절까지 무결이지만 ★**좁은 쪽이 계약이다**(SDK 능력표가 두 브라우저를
`1pc` 로 둔다). Safari 는 미측정이라 `2pc` 다.

→ 구조 — [`ROOM.ledger.BundleCap`](#ROOM.ledger.BundleCap)

<a id="RULE-ROOM-BUDGET-MATH"></a>**RULE-ROOM-BUDGET-MATH** · 규칙

★**예산 산수** — 구독 몫 = 천장 128 − 클라 몫. `1pc` 는 클라 m-line 이 같은 BUNDLE 에 있고 32 미만으로 유지되므로(성립 조건 색인 · §6-1-1)
★**`1pc` 96 · `2pc` 128**(구독 연결에 클라 m-line 이 없다). `4005` 는 mid 값이 아니라 ★**이 개수로 난다** — `details.limit` 이 그 몫, `current` 가 지금
개수다(§11-2-3). 값 폭 255(§6-1-1)는 이제 먼저 닿는 벽이 아니다.

→ 구조 — [`ROOM.ledger.BundleCap`](#ROOM.ledger.BundleCap) · [`ROOM.ledger.ClientMlines`](#ROOM.ledger.ClientMlines) · [`ROOM.ledger.Sub1pc`](#ROOM.ledger.Sub1pc) · [`ROOM.ledger.Sub2pc`](#ROOM.ledger.Sub2pc) · [`ROOM.op.ROOM_JOIN`](#ROOM.op.ROOM_JOIN)

<a id="RULE-ROOM-BUDGET-SPLIT"></a>**RULE-ROOM-BUDGET-SPLIT** · 규칙

★**예산 배분은 고정 분할이다** — 서버는 구독 몫을 ★**무전 슬롯 몫**(값은 서버 정책 — 기본은 지령대 20채널 × 2)과 ★**전이중 줄 상한**(연결마다 audio 25 · video 25 — §8-1-2)으로 나눈다. 선착순이면
통화 방 하나가 지령대 채널을 굶긴다. 슬롯 몫이 차면 그 뒤의 무전 방 입장이 `4005` 이다. ★전이중은 `4005` 가 없다 — 상한에 닿으면 줄이 더 생기지 않고 서버가 고른 트랙을 있는 줄에 싣는다(재배정).

→ 구조 — [`ROOM.ledger.DispatchCh`](#ROOM.ledger.DispatchCh) · [`ROOM.ledger.SlotBudget`](#ROOM.ledger.SlotBudget) · [`ROOM.ledger.FullAudio`](#ROOM.ledger.FullAudio) · [`ROOM.ledger.FullVideo`](#ROOM.ledger.FullVideo) · [`ROOM.ledger.Sub1pc`](#ROOM.ledger.Sub1pc) · [`ROOM.op.ROOM_JOIN`](#ROOM.op.ROOM_JOIN)

<a id="RULE-ROOM-JOIN-FAIL"></a>**RULE-ROOM-JOIN-FAIL** · 규칙

→ 구조 — [`ROOM.op.ROOM_JOIN`](#ROOM.op.ROOM_JOIN)

<a id="RULE-ROOM-JOIN-NO-TYPE-CHECK"></a>**RULE-ROOM-JOIN-NO-TYPE-CHECK** · 규칙

★**참가자 종류·투명은 여기서 판정하지 않는다** — 토큰이 이미 정했다(§3-1-5·§4-1-3). 요청에 자리가 없으므로 거절할 것도 없다.

<a id="RULE-ROOM-NOTIFY-VERSION-FOUR"></a>**RULE-ROOM-NOTIFY-VERSION-FOUR** · 규칙 — 통지 — 전부 **S→C**, 공통 의무는 ★**ACK 하나**

★★**`version` 을 싣는 것은 넷이다** — `PARTICIPANT_EVENT`·`TRACK_EVENT`·`TRACK_STATE`·`PARTICIPANT_STATE`.
이 넷이 ★**방 공통 스냅샷의 델타**를 나르고, ★**방마다 한 카운터**를 공유한다(§4-2-6).
클라는 §4-2-3 다섯 줄로 받는다 — `epoch` 가 다르면 버리고, `≤ 보관값` 이면 버리고,
`보관값 + 1` 이면 반영하고, 그 밖이면 §4-1-14 로 통짜로 다시 받는다.

<a id="RULE-ROOM-ROOM-EVENT-NO-VERSION"></a>**RULE-ROOM-ROOM-EVENT-NO-VERSION** · 규칙

★**`ROOM_EVENT` 는 `version` 을 안 싣는다**(§4-2-3) — `affiliation` 은 방 공통 스냅샷 밖의 값이고
`sync_required` 는 payload 자체가 없다.

→ 구조 — [`ROOM.op.ROOM_EVENT`](#ROOM.op.ROOM_EVENT)

<a id="COND-ROOM-BUDGET"></a>**COND-ROOM-BUDGET** · 성립 조건

★**성립 조건 — m-line 예산의 수가 서로 맞는다.** 구독 몫은 천장에서 클라 몫을 뺀 것(`1pc`) · 천장 그대로(`2pc`) · 슬롯 몫 기본은 지령대 채널 × 2 · ★슬롯 몫과 전이중 줄 상한(audio · video)의 합이 `1pc` 구독 몫 안에 든다 · 방 수 상한보다 슬롯 몫이 먼저 닿는다(방마다 슬롯 m-line 이 적어도 하나라서 — RULE-ROOM-MAX-100 의 "보증이 아니다").

<a id="PROP-ROOM-PSTATE-ENFORCED"></a>**PROP-ROOM-PSTATE-ENFORCED** · 성질

서버가 알린 권한 비트(그 방 서버 비트 · 같은 걸음의 PARTICIPANT_STATE)는 이미 발언권 · 발행이 집행하고 있다.

*식: PO1 · 깨지면: 알림을 보고 발행을 건 그 순간 서버 비트가 아직 안 내려가 통과한다(또는 반대로 거절된다)(RULE-ROOM-PSTATE-ORDER)*

<a id="DEV-ROOM-JOIN-FAIL-4009"></a>**DEV-ROOM-JOIN-FAIL-4009** · 자체 확장

`ROOM_JOIN` 의 실패에 `4009 OVERLOADED` 가 있다 — §11-2-3 · §4-2-2 6 이 정하고 §4-1 실패 줄에는 빠져 있다

*원문: §4-1 실패 줄 · 근거: RULE-ROOM-ON-JOIN-FAIL*

<a id="PROP-ROOM-PC-MODE-JOINED"></a>**PROP-ROOM-PC-MODE-JOINED** · 성질

다른 연결 모드를 되돌린 서버의 방으로는 들어가지 않는다 — JOINING 에서 JOINED 로 가는 응답의 `server_config.pc_mode` 는 세션 확정값이다(RULE-ROOM-JOIN-PC-MODE-CHECK).

*식: PC1 · 깨지면: 클라는 세션 모드로 SDP 를 짓는데 서버는 다른 모드로 돈다 — 연결이 서지 않거나 서버가 조용히 다른 모드로 돌린다(RULE-SES-PC-MODE-SESSION 증상)*

<a id="DEV-ROOM-PC-MISMATCH-UNDO"></a>**DEV-ROOM-PC-MISMATCH-UNDO** · 자체 확장

`ROOM_JOIN` 응답의 `server_config.pc_mode` 가 세션 확정값과 다르면 연결을 세우지 않고 실패로 다루되(앱에 입장 실패), ★서버가 이미 받아들인 입장을 `ROOM_LEAVE` 로 되돌린다 — `LEAVING` 으로 가고 퇴장 응답 뒤는 퇴장과 같다(세우던 연결은 그 서버에 남은 방이 없으면 닫는다)

*원문: RULE-ROOM-JOIN-PC-MODE-CHECK — 실패로 다룬다고만 한다(서버 쪽 명단은 정하지 않는다) · 근거: RULE-ROOM-JOIN-PC-MODE-CHECK PROP-ROOM-MEMBER-AGREES PROP-ROOM-PC-MODE-JOINED CAND-ROOM-PC-MISMATCH-UNDO*

#### 4-2-7. 타이머 선언

| 이름 | 주인 | 도는 동안 | 만료 | 뜻 |
|---|---|---|---|---|
| `TStall` | server | `cool` | `STStall` | sync_required 재통보 쿨다운 |

### 4-3. 흐름 · 검증
검증 모형 `out/room/Room.tla` · 하네스 `verify/room/harness.tla`

#### 4-3-1. 흐름 F1 — 입장하고 통지 하나 받기
방의 줄기다. 입장 응답의 version 으로 보관본을 시작하고, 그 뒤의 델타를 하나씩 붙인다.
모형 검사: 걸음 5 도달 · 갈래 14

1. **join** — 앱이 방에 들어가려 한다. ROOM_JOIN 을 보내고 JOINING 으로 간다.
2. **up ROOM_JOIN** — 서버가 명단에 올린다 — 명단이 바뀌었으므로 그 방 seq 가 오른다. 응답은 오른 뒤의 스냅샷이다.
3. **down ROOM_JOIN** — 클라는 tracks 와 version 을 통째로 보관한다 — 견주지 않고 재설정한다(규칙 2). 구독 SDP 를 짓고 READY 를 보낸다 → JOINED. ★READY 를 빠뜨리면 받는 영상이 영구히 검다.
4. **change** — 남이 그 방 공통 스냅샷을 바꾼다 — seq 가 오르고 그 방 전원에게 델타가 간다.
5. **down PARTICIPANT_EVENT** — 보관값 + 1 이므로 반영하고 보관값을 갱신한다(규칙 4).

#### 4-3-2. 흐름 F2 — 통지 하나를 잃었다 — 갭과 재동기
version.seq 가 건너뛰면 클라가 스스로 유실을 알고 그 방을 통째로 다시 받는다.
모형 검사: 걸음 9 도달 · 갈래 33

1. **join** — 앱이 방에 들어가려 한다. ROOM_JOIN 을 보내고 JOINING 으로 간다.
2. **up ROOM_JOIN** — 서버가 명단에 올린다 — 명단이 바뀌었으므로 그 방 seq 가 오른다. 응답은 오른 뒤의 스냅샷이다.
3. **down ROOM_JOIN** — 클라는 tracks 와 version 을 통째로 보관한다 — 견주지 않고 재설정한다(규칙 2). 구독 SDP 를 짓고 READY 를 보낸다 → JOINED. ★READY 를 빠뜨리면 받는 영상이 영구히 검다.
4. **change** — 남의 변경 하나 — 델타 N+1.
5. **change** — 또 하나 — 델타 N+2.
6. **lose** — 델타 N+1 이 사라진다(끊긴 동안의 유실 같은 것). 다시 오지 않는다.
7. **down PARTICIPANT_EVENT** — N+2 를 받으니 보관값 + 1 이 아니다 — 갭이다. GET /rooms/{room_id}?tracks=1 을 던지고 그 방 재동기를 진행 중으로 둔다. ★재동기가 진행 중이면 새로 던지지 않는다(규칙 5) — 갭은 여러 통지로 이어진다.
8. **http_serve** — 서버가 그 순간의 스냅샷으로 답한다. 이 응답은 HTTP 라 WS 와 순서가 없다.
9. **http** — 응답의 version 으로 보관값을 재설정하고(규칙 2) 구독 SDP 를 다시 짓는다. ★응답이 그 사이 반영한 델타보다 낡았으면 버린다 — 견주지 않고 재설정하면 되감긴다(DEV-ROOM-SNAPSHOT-BEHIND).

#### 4-3-3. 흐름 F3 — 강퇴당했다
서버가 소속을 강제로 바꾸면 응답이 아니라 ROOM_EVENT 로 온다. version 이 없어 견주지 않는다.
모형 검사: 걸음 5 도달 · 갈래 12

1. **join** — 앱이 방에 들어가려 한다. ROOM_JOIN 을 보내고 JOINING 으로 간다.
2. **up ROOM_JOIN** — 서버가 명단에 올린다 — 명단이 바뀌었으므로 그 방 seq 가 오른다. 응답은 오른 뒤의 스냅샷이다.
3. **down ROOM_JOIN** — 클라는 tracks 와 version 을 통째로 보관한다 — 견주지 않고 재설정한다(규칙 2). 구독 SDP 를 짓고 READY 를 보낸다 → JOINED. ★READY 를 빠뜨리면 받는 영상이 영구히 검다.
4. **kick** — 서버가 그 사람을 명단에서 뺀다 — 남에게는 델타가, 그 사람에게는 ROOM_EVENT{affiliation, cause:"kick"} 가 간다.
5. **down ROOM_EVENT** — 종결이다 — 견주지 않고 그 방을 NONE 으로 내리고 보관본을 버린다.

### 4-4. 주제

#### 4-4-1. 방 상태 — 방마다 따로 갖는다

<a id="RULE-ROOM-STATES"></a>**RULE-ROOM-STATES** · 규칙 — 방 상태 — 방마다 따로 갖는다

| 상태 | 뜻 |
|---|---|
| `NONE` | 그 방에 없다 |
| `JOINING` | `ROOM_JOIN` 을 보내고 응답을 기다린다 |
| `JOINED` | 들어가 있다. 미디어를 붙일 수 있다 |
| `LEAVING` | `ROOM_LEAVE` 를 보내고 응답을 기다린다 |

→ 구조 — [`ROOM.C.NONE.join`](#ROOM.C.NONE.join) · [`ROOM.C.JOINING.join_ok`](#ROOM.C.JOINING.join_ok) · [`ROOM.C.JOINED.leave`](#ROOM.C.JOINED.leave) · [`ROOM.C.LEAVING.leave_ok`](#ROOM.C.LEAVING.leave_ok)

<a id="RULE-ROOM-LISTEN-IS-JOINED"></a>**RULE-ROOM-LISTEN-IS-JOINED** · 규칙

★★**청취 전용도 같은 `JOINED` 다.** 참여의 입구는 ★**`ROOM_JOIN` 하나**이고, 듣기만 할 방은
★**`select:false`**(§4-1)로 들어간다 — 지령대가 10~20 채널을 상시 모니터링하는 것이 이 경로의
기준 시나리오다. 청취라고 특례를 두지 않는다: ★**명단에 오르고, 정원을 먹고, 통지를 받는다.**
청취인지 참여인지는 ★**명단의 `select`**(§4-1-3)로 가른다 — 화면에 어떻게 보일지는 앱이 정한다.

→ 구조 — [`ROOM.C.JOINING.join_ok`](#ROOM.C.JOINING.join_ok)

<a id="RULE-ROOM-RESUMING-KEEPS"></a>**RULE-ROOM-RESUMING-KEEPS** · 규칙

★**연결이 `RESUMING` 이면 방 상태는 그대로 둔다.** `RESUME` 응답의 `snapshot` 을 반영하고 미디어와 대조한 결과(§3-2-3)로 정리한다.

→ 구조 — [`ROOM.C.JOINED.resume_snapshot`](#ROOM.C.JOINED.resume_snapshot) · [`ROOM.C.JOINED.resume_gone`](#ROOM.C.JOINED.resume_gone)

<a id="PROP-ROOM-MEMBER-AGREES"></a>**PROP-ROOM-MEMBER-AGREES** · 성질

통로가 비면 클라가 JOINED 인 것과 서버 명단에 있는 것이 같다.

*식: RH4 · 깨지면: 클라는 방에 있다고 알지만 서버 명단에 없다(통지가 안 온다) · 또는 그 반대(남의 화면에 유령이 남는다)*

#### 4-4-2. 상태를 앱에 알린다

<a id="RULE-ROOM-APP-EXPOSE"></a>**RULE-ROOM-APP-EXPOSE** · 규칙

| 무엇 | 값 |
|---|---|
| 방마다의 상태 | §4-4-1 의 **넷** |
| ★**방마다 청취인가 참여인가** | ★**내 `select`**(§4-1). 명단의 남들 것과 같은 축이다(§4-1-3) |
| ★**방마다 내가 할 수 있는 것** | ★**내 `permission`**(§4-2-6) — ★`me`(명단의 내 항목이 아니다 — DEV-ROOM-ME-APART)와 `PARTICIPANT_STATE{permission}`(§4-2-6). ★**버튼을 잠그고 푸는 재료**다 — 거절을 보고 잠그는 것보다 통지를 보고 잠그는 것이 빠르다 |
| ★**어느 방이 어긋났다** | `ROOM_EVENT{sync_required}`(§4-2-6) 를 받으면 ★**그 방**을 앱에 알린다 — 서버 쪽 전달이 멎은 것이다. ★**미디어 연결이 죽은 것은 다른 축이다** — 아래 행(서버마다 연결 상태) |

#### 4-4-3. `ServerConfig` — 미디어를 붙이는 재료

<a id="PROP-ROOM-SFU-ID-IS-EPOCH"></a>**PROP-ROOM-SFU-ID-IS-EPOCH** · 성질

입장 응답의 `server_config.sfu_id` 는 그 응답 `version.epoch` 와 같다 — 한 필드가 "어느 서버인가" 와 "그 서버가 재기동됐는가" 를 함께 답한다(RULE-LINK-SFU-ID-IS-EPOCH · 재기동하면 둘이 함께 바뀐다).

*식: PC2 · 깨지면: 이미 붙은 서버로 알고 새 연결을 버렸는데 그 서버는 재기동해 그 연결을 모른다 · 또는 같은 서버를 다른 서버로 알아 연결을 하나 더 세운다*

#### 4-4-4. ★`TrackEntry` 의 세 층 — `seq` 가 셀 수 있는 범위

<a id="RULE-ROOM-SEQ-TRACK-TIER-ONLY"></a>**RULE-ROOM-SEQ-TRACK-TIER-ONLY** · 규칙 — ★`TrackEntry` 의 세 층 — `seq` 가 셀 수 있는 범위

`seq` 는 ★**방 단위**이므로 원리적으로 ★**트랙 층만 셀 수 있다.** 배정은 연결마다의 배정표라 방 번호로 못 세고(`arev` 가 센다, §8-1-2) 발행 층은 스냅샷 밖이다 —
층 구분과 근거는 §6-1-1·§6-2-2 에 있다.

→ 구조 — [`ROOM.C.JOINED.delta_bare`](#ROOM.C.JOINED.delta_bare)

#### 4-4-5. 원천이 더한 것 — 출처 절이 없는 진술

<a id="DEV-ROOM-REJOIN-LIST"></a>**DEV-ROOM-REJOIN-LIST** · 자체 확장

재구축으로 방을 내릴 때(`ROOM.C.any.down`) 들어가 있던 방 · 들어가려던 방(`JOINED` · `JOINING`)을 다시 들어갈 방으로 남긴다(`rejoin`). 재구축의 다시 들어가기(`ROOM.C.NONE.rejoin`)는 그 방에만 가고, 들어간 뒤 지운다. 앱이 그동안 나간 방은 남기지 않는다 — 입장 응답을 기다리는 동안 앱이 나가려 한 방(DEV-ROOM-LEAVE-WHILE-JOINING)도 남기지 않는다. 세션이 끝나면 표시를 지운다(`ROOM.C.any.close_all` · DEV-SES-FRESH-REJOIN)

*원문: §3-2-3 3 · RULE-LINK-RECOVERY-REBUILD — "입장 순서대로" 만 정하고 목록을 둘 곳을 정하지 않는다 · 근거: R-ROOM-REJOIN-LIST*

<a id="DEV-ROOM-LEAVE-WHILE-JOINING"></a>**DEV-ROOM-LEAVE-WHILE-JOINING** · 자체 확장

입장 요청이 나가 있는 동안(`JOINING` · 4009 대기 아님) 앱이 그 방을 나가려 하면 클라는 지금 보내지 않고 응답을 기다린다(`ROOM.C.JOINING.app_leave_wait`). 응답이 성공이면 연결을 세우지 않고 곧바로 `ROOM_LEAVE` 를 보낸다 — 그 뒤는 퇴장과 같다(`ROOM.C.JOINING.join_ok_quit` → §4-2-5). 실패면 들어간 적이 없으니 보낼 것 없이 `NONE` 으로 끝낸다(`ROOM.C.JOINING.join_fail_quit` · 4009 도 되풀이하지 않는다). 앱에는 어느 쪽이든 나감(`app.left`)이 한 번 간다

*원문: §4-2-2 6 — 4009 대기 중의 퇴장만 정한다 · 근거: R-ROOM-LEAVE-WHILE-JOINING*

<a id="PROP-ROOM-RESYNC-ENDS"></a>**PROP-ROOM-RESYNC-ENDS** · 성질

재동기는 끝난다 — 갭 · `sync_required` 로 연 재동기는 결국 닫힌다(스냅샷 · 버림 · 방 떠남 · RULE-ROOM-SEQ-GAP · ★활성 성질 — 통(WS · HTTP) · 재시도 시한 · 재구축 · 다시 들어가기 · 타이머에 공정성 · 조작 · 장애는 결국 멎는다).

*식: RL1(활성 판 room live) · 깨지면: 재동기 중 표시가 영영 남아 그 뒤 델타를 모두 "재동기 중" 으로 미룬다*

<a id="PROP-ROOM-CONVERGE-ENDS"></a>**PROP-ROOM-CONVERGE-ENDS** · 성질

조작이 멎으면 결국 늘 JOINED 방의 보관본 = 서버 판 — ★유실 뒤에도(PROP-ROOM-CONVERGE 의 활성판 · 그쪽은 "따라잡지 못한 유실이 없으면" 을 건다 · RULE-ROOM-VERSION-CLIENT-RULES). 유실은 끊긴 동안의 것이라 그 뒤 RESUME 결과가 결국 온다.

*식: RL2(활성 판 room live) · 깨지면: 끊겼다 이어진 뒤 보관본이 낡은 채 남는다 — 그 사이 올라온 트랙이 영영 안 붙는다*

<a id="PROP-ROOM-JOIN-ENDS"></a>**PROP-ROOM-JOIN-ENDS** · 성질

입장 · 퇴장은 끝난다 — `JOINING` · `LEAVING` 은 결국 `JOINED` 이거나 `NONE` 이다(4009 는 횟수 제한 없이 재시도 · RULE-ROOM-JOIN-FAIL · RULE-ERR-4009-SUPPRESS · ★활성 성질 — 혼잡은 결국 풀린다).

*식: RL3(활성 판 room live) · 깨지면: 서버가 잠깐 바빴을 뿐인데 입장 중 표시로 영영 멈춘다*

<a id="PROP-ROOM-SUB-ROOMS-RECONCILE"></a>**PROP-ROOM-SUB-ROOMS-RECONCILE** · 성질

같은 방에 다시 입장해 서버가 옛 Peer 를 축출하면 그 서버의 다른 방 멤버십도 사라진다 — 클라는 입장 응답의 sub_rooms 와 대조해 빠진 방을 내리고 다시 입장한다 · 통로가 비면 들어가 있다고 아는 방은 그 서버 명단에 있다(RULE-ROOM-JOIN-PROC).

*식: MS3(다판 two_servers evict — 떠남 응답 유실 · 다시 입장 · 축출) · 깨지면: 축출된 방을 들어가 있다고 아는 채 그 방 미디어 · 발언권을 기다린다 — 서버는 이미 그 사람을 모른다*

<a id="PROP-ROOM-FULL-MOVE-BOTH-UP"></a>**PROP-ROOM-FULL-MOVE-BOTH-UP** · 성질

전이중 트랙을 든 채 발행 방을 옮기면 두 방 다 seq 가 오른다 — 옛 방 remove · 새 방 add(RULE-ROOM-HALF-FULL-SPLIT · RULE-ROOM-SEQ-UP).

*식: PM3(다판 pub_move) · 깨지면: 한쪽 방 사람들이 트랙이 바뀐 것을 갭으로 알아채지 못하고 낡은 목록을 붙든다*

### 4-5. 근거 · 기각 — 규격이 아니다

> 계약 본문(구조 · 동작 · 주제)의 규칙이 왜 그런가(R) · 무엇을 하지 않기로 했나(REJ). 소절 머리는 그 근거가 붙은 계약 자리다. 계약 판에는 싣지 않는다.

#### 4-5-1. §4-1-3 형 `MemberInfo` — `MemberInfo`

<a id="R-ROOM-ROLE-SYMPTOM"></a>**R-ROOM-ROLE-SYMPTOM** · 근거

★**증상** — 이것을 권한으로 쓰면 → 클라가 `role:0` 을 실어 보내는 것만으로 ★**무엇이든 통과한다.**

<a id="R-ROOM-HIDDEN-SYMPTOMS"></a>**R-ROOM-HIDDEN-SYMPTOMS** · 근거

★**증상**
- 클라가 `hidden` 을 선언하게 두면 → ★**누구나 은신 청취**가 된다. *"이 방을 누가 듣고 있는가"* 라는 명단의 존재 이유(§4-4-1)가 무너진다.
- 종류와 투명을 한 값에 묶으면 → ★**보이는 녹화를 표현할 수 없다.** 녹화를 켜는 순간 대원 화면에서 사라진다.
- ★투명을 "방에 없는 것"으로 다루면 → ★**투명 녹화가 아무것도 못 받는다.** 구독 대상이 아니니 흘릴 자리가 없다.
- ★`hidden` 을 *익명*으로 읽으면 → 투명 봇이 말하는 순간 ★**`FLOOR_TAKEN` 화자로 이름이 나간다.** 안내방송 봇을 투명으로 두는 것은 되지만 ★**감춰지는 것은 명단이지 목소리가 아니다.**

<a id="R-ROOM-METADATA-SYMPTOMS"></a>**R-ROOM-METADATA-SYMPTOMS** · 근거

★**증상**
- 프로필을 클라 선언으로 두면 → ★**아무나 남의 이름으로 명단에 뜬다.** 무전에서 이름은 곧 지시 대상이라 사칭이 바로 오작동이다.
- 상한이 없으면 → ★**명단이 폭탄이 된다.** 정원 1000 인 방의 스냅샷이 통째로 다시 나가고(§3-1 `RESUME` · §4-1-14 재동기), 재접속이 몰릴 때 제일 크게 터진다.
- ★투명을 **양방향**으로 읽으면 → ★**녹화 봇이 누구를 녹화하는지 모른다.** 감추는 것은 *남들이 보는 쪽* 하나다(위).
- ★이어받기 재접속으로 프로필이 바뀔 것이라 기대하면 → ★**옛 이름이 그대로 남는다.** `session_id` 갈래는 토큰을 안 본다(§3-1) — 새 이름은 **새 세션**에서만 선다.

#### 4-5-2. §4-1-10 `0x0705 PARTICIPANT_STATE` — 참가자 **속성**이 바뀌었다   **S→C** · ACK

<a id="R-ROOM-PSTATE-SYMPTOMS"></a>**R-ROOM-PSTATE-SYMPTOMS** · 근거

★**증상**
- ★본인에게만 보내면 → 남들 화면에 ★**말할 수 없는 사람이 말할 수 있는 것으로 남는다.** 지령대가 그 사람을 계속 지목한다.
- ★적용보다 통지가 먼저면 → 자동 발행이 ★**막 내려간 비트를 통과한다.** 막았다고 답한 뒤에도 소리가 난다.
- 델타로 보내면 → 놓친 통지 하나가 ★**영구히 어긋난 비트**를 남긴다. `version` 재동기가 있어도 그 사이가 틀린다.
- ★투명 참가자의 것을 방에 보내면 → 명단에 없는 `user_id` 가 ★**전원 화면에 도착한다.** 감추기로 한 축이 한 프레임으로 무너진다.
- ★투명 참가자 본인에게도 안 보내면 → 자기 권한이 바뀐 것을 ★**실패(`2006` · `DENY(102)`)로 부딪혀 보고서야 안다** — 명단 밖이라 스냅샷에도 자기 항목이 없다(DEV-ROOM-HIDDEN-SELF-PSTATE · DEV-ROOM-ME-APART).

#### 4-5-3. §4-1-11 `0x0704 ROOM_EVENT` — 나와 방의 관계·정합성   **S→C** · ACK · ★**신설** · unicast

<a id="REJ-ROOM-NO-KICK-TYPE"></a>**REJ-ROOM-NO-KICK-TYPE** · 기각

★**축출을 별도 `type` 으로 두지 않는다.** 방에서 쫓겨나는 것은 ★**소속 변화의 한 사유**이고
`cause:"kick"` + `room_id` 가 이미 그 자리다.

<a id="R-ROOM-ROOM-EVENT-ONE-OP"></a>**R-ROOM-ROOM-EVENT-ONE-OP** · 근거

★**왜 `type` 둘을 한 op 에 두나** — 축이 같기 때문이다: ①같은 자료형(`room_id` + 나와 그 방의 관계 —
`version` 없이)을 나르고 ②클라 처리기가 같으며(견주지 않고 → 보관본 교체 → 앱 통보) ③한쪽만 구현하는
일이 없다. ★**`type` 은 "무엇을 고칠지" 만 가른다.**

<a id="R-ROOM-ROOM-EVENT-SYMPTOMS"></a>**R-ROOM-ROOM-EVENT-SYMPTOMS** · 근거

★**증상**
- ★이 op 이 없으면 → 강퇴·방 닫힘을 ★**알릴 경로가 아예 없다.** 서버는 소속을
  바꿨는데 클라는 **옛 목록으로 계속 말하려 하고**, 소리가 안 나가는 이유를 못 찾는다.
- `sync_required` 를 무시하면 → ★**멎은 채로 굳는다.** 서버는 알렸고 클라가 안 움직였으므로
  **양쪽 계수기가 다 정상**으로 보인다.
- ★`sync_required` 는 **연결이 살아 있는 채로** 온다. `RESUME`(§3-1)의 `snapshot` 은
  **재접속 때만** 오므로 여기서는 안 온다 — ★**§4-1-14 가 이때의 유일한 재동기 수단이다.**
- ★**부분 목록을 전체로 덮어쓰면** → 다른 서버의 듣는 방이 목록에서 사라진다. 소리는 나는데
  그 방이 없다.

#### 4-5-4. §4-1-13 `POST /rooms` — 방 생성

<a id="R-ROOM-CREATE-SYMPTOM"></a>**R-ROOM-CREATE-SYMPTOM** · 근거

★**증상** — TTL 을 안 준 방은 서버 설정을 따르고 상용 기본은 영구다 — ★**앱이 TTL 을 빠뜨리면 빈 방이 영원히 남는다.**

#### 4-5-5. §4-1-14 `GET /rooms/{room_id}` — 방 상세

<a id="R-ROOM-DETAIL-SYMPTOMS"></a>**R-ROOM-DETAIL-SYMPTOMS** · 근거

★**증상**
- ★`version` 을 안 보고 덮어쓰면 → **낡은 응답이 최신 통지를 되감는다.** 그 사이 올라온 트랙은
  통지가 이미 지나가서 ★**영영 안 붙는다.**
- ★이 응답의 목록으로 m-line 을 지으면 → 배정표와 다른 대응으로 ★**엉뚱한 자리에 붙는다** — SDP 는 표로만 짓는다.
- 여러 방을 방마다 재조립하면 → 왕복이 방 수만큼 나고 ★**중간 상태로 협상이 돈다.**
- 입장한 방만 조회를 허용하면 → 사용자가 **채널을 눈감고 고른다.**

<a id="R-ROOM-DETAIL-PROC-WHY"></a>**R-ROOM-DETAIL-PROC-WHY** · 근거

- 미리보기가 더 잦고 payload 가 훨씬 크다. 재동기 쪽은 ★**명시적으로 요구하는 것이 안전하다.**
- ★견주면 방을 나갔다 돌아온 뒤나 재접속 직후의 큰 점프가 갭으로 보여 ★**받자마자 또 받는다.**
- 통지 권위는 DC 단일이다(§1-2-1·§10).
- 방마다 조립하면 ★**왕복이 방 수만큼 나고 중간 상태로 협상이 돈다.**
- 상태는 통지로 흐르는 것이 정상이고, 이것은 **어긋났을 때의 복구 수단**이다. 폴링하면 통지를 안 듣는 구현이 된다.

#### 4-5-6. §4-1-15 비트마다 무엇을 막나 — 사상표

<a id="R-ROOM-BITS-SYMPTOMS"></a>**R-ROOM-BITS-SYMPTOMS** · 근거

★**증상**
- 사상표가 없으면 → 서버와 클라가 `publish_screen` 을 다르게 읽어 ★**`details.denied` 가 뜻을 잃는다.**
- `source` 를 열린 집합으로 두면 → 모르는 라벨이 어느 비트인지 서버가 답을 못 해 ★**조용히 통과하거나 조용히 막힌다.**
- 갱신값을 세션에 두거나 멤버십과 함께 버리면 → 비트를 내린 사람이 ★**방을 나갔다 들어오는 것만으로 풀린다.** 결정자는 자기 결정이 왜 안 먹는지 모른다.
- 비트를 세션 것으로 읽으면 → 한 방에서 내린 비트가 ★**다른 방의 명단과 어긋난 채 산다** — 통지는 한 방에만 가기 때문이다.

#### 4-5-7. §4-2-2 클라 · `JOINING`

<a id="R-ROOM-JOIN-OK-SYMPTOM"></a>**R-ROOM-JOIN-OK-SYMPTOM** · 근거

★**증상** — 5를 빠뜨리면 ★**받는 영상이 영구히 검다.**

<a id="R-ROOM-ON-JOIN-FAIL-WHY"></a>**R-ROOM-ON-JOIN-FAIL-WHY** · 근거

- ★**다른 방을 나가기 전에는 재시도해도 같다** — `4xxx` 앞자리 규칙("줄이거나 기다린다")에서 *기다린다* 가 안 듣는 곳이다(아래 5 와 둘).
- ★**`4005` 와 같은 성질**이다 — 다른 방을 나가기 전에는 재시도해도 같으므로 §11-2-1 4 의 재시도를 태우지 않는다.
- `4xxx` 가운데 ★**클라가 스스로 기다리는 유일한 코드**다.
- 재구축의 입장이든 앱이 부른 입장이든 같다 — ★**서버는 둘을 가를 수 없다.**

#### 4-5-8. §4-2-3 클라 · `JOINED`

<a id="R-ROOM-RULE2-WHY"></a>**R-ROOM-RULE2-WHY** · 근거

★★**규칙 2 가 있어야 하는 이유** — 스냅샷의 `seq` 는 ★**현재값**이라 보관값보다 훨씬 클 수 있다.
방을 나갔다 3분 뒤 다시 들어오면 `11 → 847` 이고 ★**형태로는 완벽한 갭**이다. 견주면 ★**방금 스냅샷을
받았는데 스냅샷을 또 받는다.** `RESUME` 이 특히 나쁘다 — 재접속 직후 방 여럿이 동시에 큰 점프라
★**재접속이 몰릴 때 제일 크게 터진다.**

<a id="R-ROOM-EPOCH-WHY"></a>**R-ROOM-EPOCH-WHY** · 근거

★★**`epoch` 가 없으면 재기동 사고가 난다.** 미디어 서버가 죽었다 살아나 `seq` 가 `0` 부터
다시 시작하면, `10245` 를 들고 있는 클라는 ★**이후 모든 통지를 영원히 버린다.** 소리도 화면도
멀쩡히 도착하는데 ★**아무것도 갱신되지 않는다.**

<a id="R-ROOM-VERSION-SYMPTOMS"></a>**R-ROOM-VERSION-SYMPTOMS** · 근거

★**증상**
- ★`version` 을 안 보고 덮어쓰면 → **낡은 응답이 최신 통지를 되감는다.** 그 사이 올라온 트랙은
  통지가 이미 지나가서 ★**영영 안 붙는다.**
- ★`seq` 갭을 안 보면 → 통지 하나를 놓친 것을 **알 방법이 없다.** 미디어가 멎어야 뜨는
  `sync_required`(§4-2-6)만 기다리게 되는데, ★**참가자 통지 하나를 놓친 것은 미디어를 안 멈춘다.**
- `epoch` 를 안 보면 → 서버 재기동 뒤 ★**모든 통지를 버린다.**
- ★스냅샷이 안 바뀌었는데 `seq` 를 올리면(투명 참가자 입퇴장, §4-1-3) → 갭이 ★**'유실'과 '투명 참가자' 두 뜻**이 되어 규칙 5 가 죽는다. 정원 1000 방에 봇 하나 붙는 것이 ★**전원 스냅샷 재전송**이다.
- ★**스냅샷을 견주면**(규칙 2 없이) → 재입장·재접속마다 ★**받자마자 또 받는다.** 재접속이 몰릴 때 제일 크다.
- ★**op 별 카운터로 구현하면** → 화자가 말할 때마다 ★**방 전원이 헛 재동기**한다. 상태는 안 깨져 조용히 비싸다.
---

<a id="R-ROOM-SEQ-GAP-WHY"></a>**R-ROOM-SEQ-GAP-WHY** · 근거

- ★**뭔가 놓쳤다.** 통지 하나가 사라졌고 ★**다시 오지 않는다.**
- 견주면 응답의 `seq` 가 또 갭으로 보여 ★**무한 재동기**가 된다

★**이것이 없으면** — 유실을 알 방법이 ★**`sync_required` 하나뿐**인데, 그것은
★**미디어가 멎어야** 뜬다. `PARTICIPANT_EVENT` 하나를 놓친 것은 미디어를 안 멈추므로
★**영영 안 잡힌다.**

#### 4-5-9. §4-2-6 서버

<a id="R-ROOM-SILENT-OVERFLOW"></a>**R-ROOM-SILENT-OVERFLOW** · 근거

★**깨지는 모습이 조용하다** — 브라우저는 초과분을 거절(포트 0)하지 않고 ★**제 `a=ice-ufrag` 를 가진 독립 전송로 절**로 답한다. 서버는 BUNDLE 하나만
주므로 그 m-line 은 ICE 가 붙지 않고 ★**오류 없이 그 트랙만 안 나온다.** 그래서 서버가 개수를 세어 `4005` 로 막는 것이 유일한 방어다.

<a id="R-ROOM-WORST-SHAPE"></a>**R-ROOM-WORST-SHAPE** · 근거

★★**최악 형상은 무전 방과 통화 방의 공존이다** — 한 세션이 지령대로 20채널을 듣는 채로 지휘부 통화 방에 들어간다(성립 조건
색인). 무전 방은 슬롯 2 로 고정이고, 통화 방은 ★**인원이 늘어도 전이중 줄 상한에서 멈춘다**(audio 25 · video 25 — §8-1-2). 그래서 정적 예산이 인원과 무관하게 닫힌다.

| 형상 | 구독 m-line N | `1pc` 구독 몫 96 대비 | `2pc` 128 대비 |
|---|---|---|---|
| 무전 20방 + 통화 15명 | 40 + 30 = 70 | 73% | 55% |
| 무전 20방 + 통화 50명 | 40 + 50(상한 — 재배정) = 90 | 94% | 70% |
| 무전 20방 + 통화 1000명 | 40 + 50(상한 — 재배정) = 90 | 94% | 70% |
| 무전 20방(video 슬롯을 쓰든 안 쓰든) | 40 | 슬롯 몫 기본 40 ★**천장**(21번째 무전 방 `4005`) | 같음 — 슬롯 몫은 모드 무관 |
| 무전 100방(색인의 방 수 상한) | 200 | ★**성립하지 않는다** | ★**성립하지 않는다** |

<a id="R-ROOM-JOIN-SYMPTOMS"></a>**R-ROOM-JOIN-SYMPTOMS** · 근거

★**증상**
- `server_config` 를 안 쓰고 자체 SDP 를 조립하면 → **DTLS·ICE 가 안 붙는다.**
- 고정 PT 표를 쓰면 → **화면이 검고 키프레임 요청이 0** 이다(그게 서명이다).

<a id="R-ROOM-LIMIT-KINDS"></a>**R-ROOM-LIMIT-KINDS** · 근거

★**상한마다 종류가 다르다** — 바뀌는 조건이 다르므로 섞어 읽지 않는다(제3자 검토 20261012).

| 값 | 종류 | 바뀌는 조건 |
|---|---|---|
| 천장 128 | ★**브라우저 실측**(Firefox 155 — 좁은 쪽이 계약) · 표준 제약이 아니다 | 브라우저가 바뀌면 다시 잰다(e2e 회귀) |
| 클라 몫 32 | 우리 설계(`1pc` 의 발행 m-line 예약) | 규격 개정 |
| 구독 몫 `1pc` 96 · `2pc` 128 | 산수(천장 − 클라 몫 · 천장) — 위 둘에서 나온다 | 위 둘이 바뀌면 함께(COND-ROOM-BUDGET 가 건다) |
| 슬롯 몫 기본 40 | ★**서버 정책**(지령대 20채널 × 2) | 배포 설정 |
| 전이중 줄 상한 audio 25 · video 25 | ★**우리 설계**(연결마다 — §8-1-2) · video 는 한 화면 타일 25(Jitsi 선례) · audio 도 같은 수 | 규격 개정 |
| 방 100 | 서버 안전 상한(운영) — m-section 예산의 보증이 아니다 | 배포 설정 |
| 구독 mid 값 폭 255 | ★**서버 자료형**(`u8`) — 표준 제약이 아니다 · 먼저 닿는 벽이 아니다 | 서버 구현 |

#### 4-5-10. §4-4-1 방 상태 — 방마다 따로 갖는다

<a id="REJ-ROOM-NO-SUBSCRIBED"></a>**REJ-ROOM-NO-SUBSCRIBED** · 기각

★**"입장 없는 구독"(별도 `SUBSCRIBED` 상태)은 두지 않는다.** ★**누가 듣고 있는지가
명단에 보여야 하는 것**이 제품 요구인데(§4-1-3), 명단 밖 청취를 만들면 그 요구와 어긋나고 —
★**재료의 출처가 갈라진다.** `server_config` 는 `ROOM_JOIN` 응답에만 오므로(§9-1-1), 입장하지
않은 방을 받으려면 재료를 나를 두 번째 경로가 필요해진다. 입구가 하나면 그 경로 자체가 없다.

<a id="R-ROOM-LISTEN-PATH-SYMPTOM"></a>**R-ROOM-LISTEN-PATH-SYMPTOM** · 근거

★**증상** — 청취를 `ROOM_JOIN` 밖의 별도 경로로 만들면 → 그 경로로만 듣는 방은
★**`server_config` 를 받은 적이 없어** 연결·DC 를 세울 재료가 없고, 명단·`version.seq` 의
통지 범위가 두 갈래로 갈려 ★**어느 방은 참가자 통지가 안 와 `seq` 가 계속 건너뛴다.**

#### 4-5-11. §4-4-5 원천이 더한 것 — 출처 절이 없는 진술

<a id="R-ROOM-REJOIN-LIST"></a>**R-ROOM-REJOIN-LIST** · 근거

원칙 ①(20261011) — 전부 다시는 받은 상태를 버리되 "어디에 다시 들어가나"는 남겨야 한다. 우리는 연결 하나에 방이 여럿이라(다방 청취) LiveKit 처럼 연결 하나 = 방 하나로 기억할 수 없다

*없으면: 재구축이 방을 먼저 `NONE` 으로 내리고 보관본을 버려, 어느 방에 다시 들어갈지 원천 어디에도 남지 않는다*

<a id="R-ROOM-LEAVE-WHILE-JOINING"></a>**R-ROOM-LEAVE-WHILE-JOINING** · 근거

기다리지 않고 곧바로 `ROOM_LEAVE` 를 보내면 입장이 실패했을 때 그 퇴장이 오류를 받는다 — 클라는 회원 아닌 사람의 퇴장을 서버가 어떻게 다루는지에 기대게 된다. 응답을 기다리면 `ROOM_LEAVE` 는 늘 "들어가 있던 방을 나감" 하나의 뜻이다

*없으면: 입장 중에 나간 방이 구현에 따라 남거나(응답 뒤 `JOINED`) 오류 처리에 기댄다*

<a id="R-ROOM-CLOSED-OPS-ONLY"></a>**R-ROOM-CLOSED-OPS-ONLY** · 근거

원칙 ②(20261011) — `ROOM_EVENT{affiliation, cause:"room_closed"}` 는 운영의 방 닫힘 전용이다. 미디어 서버 재기동은 재접속 응답 하나(`ICE_RESTART` `3010` · `RESUME` `2008`)가 결말을 정한다 — LiveKit 은 이어받기를 시도한 참가자를 모르면 `LeaveRequest{RECONNECT, STATE_MISMATCH}` 로 답하고 방 삭제에만 `ROOM_DELETED` 를 쓴다

*없으면: 재기동 때 입구가 둘(LINK 길 · `room_closed`)이 되어 한쪽은 다시 들어가고 다른 쪽은 쫓겨난 채 끝난다*

<a id="AFF"></a>
