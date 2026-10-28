[← 차례](README.md)

## 6. TRK — 트랙

방에 있는 트랙 하나 — 방 전원이 같은 값으로 보는 트랙 목록 항목(TrackEntry 본체). 개인 트랙(발행자의 카메라 · 마이크 · 화면) · 슬롯 트랙(무전 — 주인이 방). 목록 알림은 TRACK_EVENT · TRACK_STATE

| 정체성 | 누가 내나 | 무엇마다 | 수명 | 상태 |
|---|---|---|---|---|
| track_id | server | 방 × 트랙 | TRACK_EVENT{add} · 입장 응답 tracks[] → TRACK_EVENT{remove} · 퇴장 | — |

| 의존 | 기댐 | 다중도 | 뜻 |
|---|---|---|---|
| TRK | ROOM | N:1 | 트랙 목록은 참여한 방의 것 — 방 version.seq 로 정합한다(§4-1-6 · §4-2-6) |

### 6-1. 구조

<a id="TRK.type.TrackEntry"></a>
#### 6-1-1. 형 `TrackEntry` — 받을 트랙 하나

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| ★`type` | "individual" / "slot" | individual: `"individual"` · slot: `"slot"` | ★**트랙 종류.** 바뀌지 않는다 — 회의↔무전 전환은 `duplex`(발행 속성)가 바뀌는 것이고 트랙은 쉬거나(`active`) 슬롯으로 흐를 뿐이다 |
| ★`room_id` | str | individual: ✔ · slot: ✔ | ★**이 트랙이 어느 방 것인가.** 항상 온다 |
| `track_id` | str | individual: ✔ · slot: ✔ | ★**식별 키 — 보관본의 키.** 슬롯은 `"ptt-{room_id}-audio"` / `"…-video"`. **파싱하지 않는다** |
| `kind` | "audio" / "video" | individual: ✔ · slot: ✔ | `"audio"` / `"video"` |
| `codec` | string? | individual: video · slot: video 슬롯 | `"VP8"` / `"H264"` / `"VP9"` — ★서버가 실제로 지원하는 목록은 `1005` 의 `details.supported` 가 정본이다(1차 = VP8·H264). ★**이 값은 그 방의 미디어 코덱(방 속성 `media_codecs.video`, §4-1-1)이다** — 전이중 · 슬롯 모두 |
| ★`fmtp` | string? | individual: 있으면 · slot: video 슬롯만 | ★**보기용** — 발행자가 `PUBLISH_TRACKS` 에 실은 `fmtp`(§7-1-2) 그대로. ★**SDP 재료가 아니다** — 구독 SDP 의 `a=fmtp` 는 배정표 줄의 고정 선언값이다(§8-1-1 · §4-1-1) |
| `user_id` | str | individual: ✔ · slot: ★없다 | 보내는 사람. 슬롯은 ★**여러 사람이 돌려쓰는 한 m-line**이라 주인이 없다 — 화자는 발언권 통지로 안다(§10-4-35) |
| `source` | string? | individual: video · slot: ★없다 | ★**`"camera"` / `"screen"` 둘뿐이다**(닫힌 집합, §7-1-2) — ★**video 만**(audio 개인 트랙에는 없다). 발행자가 `PUBLISH_TRACKS` 에 신고한 값 그대로다 — 서버는 검증하지 않는다(§4-2-6) |
| `active` | bool? | individual: ✔ · slot: ★없다 | ★`false` 면 지금은 안 온다 — 회의→무전 전환으로 ★**쉬는** individual 트랙이다(`TRACK_SET{duplex}` 가 적용되면 그 방 전원에게 `TRACK_EVENT{add}` 로 항목이 교체된다, §4-2-6). ★★**쉬는 트랙은 배정표에 줄이 없다** — full → half 순간 서버가 그 자리를 놓고(PROP-LINK-ROW-ONLY-RECEIVED) 그 줄은 빈 줄이 된다(§8-3-1). 서버가 그 줄을 같은 `kind` 의 다른 트랙에 주면 다음 표가 자리를 채운다. 복귀(`true`)는 `TRACK_EVENT{add}` 와 ★**그 트랙의 줄이 붙은 표**(`MLINE_EVENT`)로 온다. ★*"지워라"* 가 아니다 — m-line 을 두고 화면만 내린다 |
| ★`muted` | bool? | individual: ✔ · slot: ★없다 | ★**그 트랙이 음소거인가** — `TRACK_STATE{muted}`(§4-2-6)와 같은 값이다. 슬롯은 주인이 없어 *"누구의 음소거인가"* 가 정의되지 않는다(반이중에는 mute 가 없다, §7-1-2). ★**스냅샷에 있어야 복구된다**(아래 ★) |
| `simulcast` | bool? | individual: video · slot: ★없다 | 시뮬캐스트인가 |
| ★`scalability` | string? | individual: video · slot: ★없다 | ★**`"L{spatial}T{temporal}"`** — 레이어가 몇인가(예: `"L2T3"` = 공간 레이어 둘 · 시간 레이어 셋). ★**시뮬캐스트/SVC 일 때만.** 클라는 이 값으로 §7-1-2 의 상한을 고른다. ★이 세대의 시뮬캐스트는 업링크 스트림 둘(`l`·`h`) 고정(§9-4-10)이라 값은 **`"L2T1"`** 이다 |

<a id="RULE-TRK-FOUR-PATHS"></a>**RULE-TRK-FOUR-PATHS** · 규칙 — `TrackEntry` — 받을 트랙 하나

★**네 경로가 이 형을 나른다** — `ROOM_JOIN` 응답의 `tracks[]`(§4-1) · `TRACK_EVENT` 의
`tracks[]`(§4-2-6) · `RESUME` 응답의 `snapshot[].tracks`(§3-1) · `GET /rooms/{id}?tracks=1`(§4-1-14).
★★**넷이 같은 보관본 하나에 들어간다.**
★**보기용 목록이다** — 구독 SDP 를 짓는 재료는 이 목록이 아니라 배정표다(§8-1-2 · §8-1-2).

<a id="RULE-TRK-THREE-TIERS"></a>**RULE-TRK-THREE-TIERS** · 규칙

★★**세 층이다.** 성격이 다른 셋을 한 형으로 부르면 조항마다 *"반이중이면?"* 을 따로 판정하게 되고 판정이 조항마다 갈린다(12차까지의 사고).

| 층 | 무엇 | 누가 보나 | 어디에 |
|---|---|---|---|
| ★**트랙** | 구독 m-line 하나로 흘러오는 것 — `type` 이 `individual`(한 사람의 트랙) / `slot`(방 공용) | ★**방 전원**(같은 값) | `TrackEntry` 본체 · `seq` 가 세는 것(§4-2-6) |
| ★**배정** | 그 트랙의 내 자리 — 구독 m-line 하나(`mid`·`pt`·코덱 …) | ★**구독자 본인**(사람마다 다르다) | ★**배정표**(§8-1-2 — 연결마다 하나 · `arev` 가 센다) · `seq` 대상 아님 · ★`TrackEntry` 에는 없다 |
| ★**발행** | 내가 서버에 올린 트랙 — `codec`·`ssrc`·`duplex`·`simulcast` | ★**서버와 발행자 본인** | ★**스냅샷 밖** — `PUBLISH_TRACKS` 응답(§7-1-2)과 `RESUME` 응답의 `publications[]`(§3-1) |

<a id="RULE-TRK-FIELDS-BY-TYPE"></a>**RULE-TRK-FIELDS-BY-TYPE** · 규칙

**트랙 층** — `type` 마다 필드가 다르다(★한 표로 *"항상 온다"* 고 읽지 않는다).

→ 구조 — [`TRK.type.TrackEntry`](#TRK.type.TrackEntry)

<a id="TRK.op.TRACK_EVENT"></a>
#### 6-1-2. `0x0702 TRACK_EVENT` — 받을 트랙이 **생기거나 없어졌다**   **S→C** · ACK
단 1 · 출처 §4-2-6

**통지 · 받는 꼴**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `action` | "add" / "remove" | ✔ |  |
| `room_id` | str | ✔ |  |
| `tracks` | TrackEntry[] | ✔ | §6-1-1 |
| `version` | Version | ✔ |  |

<a id="RULE-TRK-TRACK-EVENT-FORM"></a>**RULE-TRK-TRACK-EVENT-FORM** · 규칙

→ 구조 — [`TRK.op.TRACK_EVENT`](#TRK.op.TRACK_EVENT)

<a id="RULE-TRK-TRACK-EVENT-ONE-ROOM"></a>**RULE-TRK-TRACK-EVENT-ONE-ROOM** · 규칙

★★**한 메시지는 한 방 것이다.** `version.seq` 가 ★**방마다**의 일련번호라(§4-1-6) 그것을 나르는
메시지도 방 단위여야 견줄 곳이 정해진다 — 통지 넷이 전부 같은 꼴이 된다(`room_id` + `version`).
여러 방이 한꺼번에 바뀌면 ★**방마다 한 메시지씩** 온다(같은 1단 FIFO 라 순서는 안전하다, §2-2-1).
원소가 `room_id` 를 따로 갖는 것은 §6-1-1 의 보관본 규칙 그대로다(컨테이너가 방을 정해도 원소는 갖는다).

→ 구조 — [`TRK.op.TRACK_EVENT`](#TRK.op.TRACK_EVENT)

<a id="RULE-TRK-TRACK-EVENT-PROC"></a>**RULE-TRK-TRACK-EVENT-PROC** · 규칙

**절차**
- ACK + `version` 견주기(★그 `room_id` 방의 보관값과) + 목록을 고친다 — ★**화면만 고친다.**
  ★구독 SDP 는 이 통지로 다시 짓지 않는다 — m-line 은 배정표가 바뀔 때 `MLINE_EVENT` 가 따로 알린다(§8-1-2).
- 서버 ★★**방 공통 스냅샷이 바뀌었으므로 그 방 전원의 보관본이 바뀐다** — 델타는 저절로 전원에게 있다
  (§4-2-6). ★**빈 델타는 원리적으로 안 생긴다.**
- 서버 ★**고갈 해소 · mid 재사용은 이 축이 아니다** — 배정표만 바뀌므로 `MLINE_EVENT` 로 그 클라에게만 간다(§8-3-1). `TRACK_EVENT` 가 없다.
- 서버 ★★**회의와 무전 사이의 전환도 이 op 이다**(§7-1-2 `TRACK_SET{duplex}`) — `full` 에서 `half` 로는 그 항목을 `active:false` 로 교체해 그 방 전원에게
  `add`(`seq++`), `half` 에서 `full` 로는 `active:true` 로 교체해 `add`(`seq++`). 받는 쪽은
  ★**같은 `track_id` 의 항목 교체**로 다룬다(트랙 층 통째). 쉬는 동안 놓은 줄과 복귀 때의 줄은 배정표가 따로 알린다(복귀 줄의 `mid` 는 새 값일 수 있다).
- ★**화면에 걸 때는 `room_id` 로 가른다.** 다방 청취면 방마다 화면이 다르다.
- 구독 SDP 재조립 · `READY{arev}` 는 배정표의 절차다(§8-1-2) — ★`1pc` 면 §9-2-1 절차를 탄다(협상 큐).

<a id="TRK.op.TRACK_STATE"></a>
#### 6-1-3. `0x0703 TRACK_STATE` — 트랙 **속성**이 바뀌었다   **S→C** · ACK
단 1 · 출처 §4-2-6

**통지 · 받는 꼴**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `type` | "muted" | ✔ |  |
| `user_id` | str | ✔ |  |
| `track_id` | str | ✔ | ★**`track_id` 가 식별 키다.** |
| `kind` | "audio" / "video" | ✔ |  |
| `room_id` | str | ✔ |  |
| `version` | Version | ✔ |  |
| `muted` | bool | ✔ |  |

<a id="RULE-TRK-TRACK-STATE-DISPLAY"></a>**RULE-TRK-TRACK-STATE-DISPLAY** · 규칙

★**`_STATE` 는 표시만 고치는 자리다**(각 장의 구조 이름 규율). ★**재협상도 재동기도 여기서 일어나지 않는다.**

<a id="EX-TRK-TRACK-STATE"></a>**EX-TRK-TRACK-STATE** · 예시(비규범)

→ 구조 — [`TRK.example.EX-TRK-TRACK-STATE`](#EX-TRK-TRACK-STATE) · [`TRK.op.TRACK_STATE`](#TRK.op.TRACK_STATE)

예시 — `TRACK_STATE` 통지(형과 대조된 값 · 주석은 칸 설명)

```jsonc
{ "type": "muted",
  "user_id": "u1",
  "track_id": "…",   // ★**`track_id` 가 식별 키다.**
  "kind": "video",
  "room_id": "…",
  "version": {"epoch":"…","seq":10248},
  "muted": true }
```

<a id="RULE-TRK-TRACK-STATE-TYPES"></a>**RULE-TRK-TRACK-STATE-TYPES** · 규칙

| `type` | 언제 | 클라가 할 일 |
|---|---|---|
| `muted` | 상대(전이중 개인 트랙)가 음소거/해제했다 | 표시 갱신. video 면 아바타 전환 |

★**`type` 은 `muted` 하나다.** `duplex`(회의↔무전 전환 — `TRACK_EVENT{add}` 항목 교체 하나, §4-2-6)와 `live`(프레임 도착 —
받는 쪽이 로컬에서 관측한다, §6-1-1)는 두지 않는다. ★**속성이 `_STATE` 인 조건** — 값이 오가고(`false` 로 돌아오는 경로가 있고)
받는 쪽이 스스로 알 수 없어야 한다. `muted` 만 둘 다 만족한다.

→ 구조 — [`TRK.op.TRACK_STATE`](#TRK.op.TRACK_STATE)

<a id="RULE-TRK-TRACK-STATE-PROC"></a>**RULE-TRK-TRACK-STATE-PROC** · 규칙

**절차** — ACK + `version` 견주기 + 위 표대로. ★**`track_id` 가 식별 키다.** ★`ssrc` 는 싣지 않는다 — 구독자가 받는 SSRC 는 연결마다 다른 다운링크 SSRC 라 방 전원에게 같은 값으로 줄 것이 없다(§9-4-2).

<a id="RULE-TRK-TRACK-STATE-TO-ALL"></a>**RULE-TRK-TRACK-STATE-TO-ALL** · 규칙

★★**배달은 그 방 전원이다** — 그 트랙 구독자만도, 본인 제외도 아니다. `muted` 는 ★**트랙 층**(§6-2-2)이라
그 방 전원의 보관본이 바뀌고, 델타는 저절로 전원에게 있다(§4-2-6). ★**아직 그 트랙을 구독하지 않은 사람도 받고**(다방
청취에서 구독은 나중에 붙는다) 본인도 받는다(보관본의 그 값이 바뀐다).

<a id="RULE-TRK-TRACK-STATE-NO-STALLED"></a>**RULE-TRK-TRACK-STATE-NO-STALLED** · 규칙

★★**`stalled` 는 여기 없다.** ★**`ROOM_EVENT{sync_required}`(0x0704)로 옮겼다** — 하는 일이
*"표시를 고쳐라"* 가 아니라 ★**"보관본을 통째로 다시 받아라"** 라서 축이 다르고, 무전 슬롯처럼
해석할 수 없는 경우엔 ★**`track_id` 가 `null` 이 되어** 식별 키가 없어졌다. ★**식별 키가
없어지는 것이 다른 물건이라는 증거다.**

<a id="TRK.state"></a>
#### 6-1-4. 상태 칸 · 시한

##### 상태 — 클라

| 칸 | 처음 | 뜻 |
|---|---|---|
| `lay` | `{A: 'NONE', B: 'NONE', S: 'ACTIVE'}` | 트랙 층 — 받은 항목으로 통째 교체한다 |
| `muted` | `{A: false, B: false, S: false}` | 트랙 층의 muted — TRACK_STATE 와 항목 교체가 고친다 |

##### 상태 — 서버

| 칸 | 처음 | 뜻 |
|---|---|---|
| `dup` | `{A: 'NONE', B: 'NONE', S: 'NONE'}` | 발행 층 — 그 사람 video 발행의 duplex(슬롯은 쓰지 않는다) |
| `lay` | `{A: 'NONE', B: 'NONE', S: 'ACTIVE'}` | 트랙 층 — 방 전원이 같은 값으로 본다 · seq 가 센다 |
| `muted` | `{A: false, B: false, S: false}` | 트랙 층의 muted(전이중 individual 만 — 슬롯은 없다 · duplex 가 바뀌면 false — RULE-PUB-SET-PROC) |

##### 전이 — 클라(동작 경우에서 짓는다)

| 지금 | 사건 | 다음 | 라벨 |
|---|---|---|---|
| 어느 상태 | TRACK_EVENT 를 받았다 — 트랙 목록(보기용)을 고친다(RULE-TRK-STORE-UPDATE · 키 = track_id) · mine = 내 track_id(version 은 ROOM 이 본다 — 낡은 델타는 ROOM 이 버린다 · RULE-TRK-TRACK-EVENT-PROC) · ★배정표는 건드리지 않는다 | — | `TRK.C.any.mine_removed` |
| 어느 상태 | TRACK_EVENT 를 받았다 — 트랙 목록(보기용)을 고친다(RULE-TRK-STORE-UPDATE · 키 = track_id) · mine = 내 track_id(version 은 ROOM 이 본다 — 낡은 델타는 ROOM 이 버린다 · RULE-TRK-TRACK-EVENT-PROC) · ★배정표는 건드리지 않는다 | — | `TRK.C.any.mine_other` |
| 어느 상태 | TRACK_EVENT 를 받았다 — 트랙 목록(보기용)을 고친다(RULE-TRK-STORE-UPDATE · 키 = track_id) · mine = 내 track_id(version 은 ROOM 이 본다 — 낡은 델타는 ROOM 이 버린다 · RULE-TRK-TRACK-EVENT-PROC) · ★배정표는 건드리지 않는다 | `NONE` | `TRK.C.any.remove` |
| 어느 상태 | TRACK_EVENT 를 받았다 — 트랙 목록(보기용)을 고친다(RULE-TRK-STORE-UPDATE · 키 = track_id) · mine = 내 track_id(version 은 ROOM 이 본다 — 낡은 델타는 ROOM 이 버린다 · RULE-TRK-TRACK-EVENT-PROC) · ★배정표는 건드리지 않는다 | — | `TRK.C.any.track_event` |
| 어느 상태 | TRACK_STATE 를 받았다 — track_id 가 식별 키(RULE-TRK-TRACK-STATE-PROC) · muted 는 트랙 층이라 보관본의 그 값이 바뀐다(아직 구독하지 않은 트랙도 · RULE-TRK-TRACK-STATE-TO-ALL) | — | `TRK.C.any.track_state` |
| 어느 상태 | 스냅샷(입장 응답 · RESUME · GET)의 tracks 를 받았다 — 그 방 트랙 목록 전부다(보기용 · RULE-TRK-FOUR-PATHS · RULE-TRK-PER-PATH) · ★배정표는 실려 오지 않는다(입장 응답 · RESUME 의 표는 MLINE_EVENT 와 같은 길로 받는다) | — | `TRK.C.any.snapshot` |
| 어느 상태 | 스냅샷(입장 응답 · RESUME · GET)의 tracks 를 받았다 — 그 방 트랙 목록 전부다(보기용 · RULE-TRK-FOUR-PATHS · RULE-TRK-PER-PATH) · ★배정표는 실려 오지 않는다(입장 응답 · RESUME 의 표는 MLINE_EVENT 와 같은 길로 받는다) | — | `TRK.C.any.snapshot_same` |
| 어느 상태 | 그 방을 떠났다 · 내렸다 — 그 방 트랙 목록 보관본을 버린다(RULE-ROOM-LEAVE-RES) · 배정표는 연결의 것이라 서버가 새 표(MLINE_EVENT)로 고친다 | — | `TRK.C.any.store_drop` |

<a id="TRK.C"></a>
### 6-2. 동작

<a id="TRK.C.any"></a>
#### 6-2-1. 클라 · `any`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="TRK.C.any.mine_removed"></a>`TRK.C.any.mine_removed` | mine && m.action == 'remove' | 효과 pub.revoked → PUB.C.SENDING.revoked · PUB.C.REGISTERED.revoked · PUB.C.any.revoked_other | §4-2-6 — 서버가 내 발행을 지웠다 — PUB 가 받는다(RULE-PUB-REVOKED) |
| <a id="TRK.C.any.mine_other"></a>`TRK.C.any.mine_other` | mine | — | §4-2-6 §6-2-2 — 내 트랙의 항목 — 받을 트랙이 아니다(표에 줄이 없다 · 고갈이 아니다 · RULE-SUB-NO-ASSIGN-NO-SDP) |
| <a id="TRK.C.any.remove"></a>`TRK.C.any.remove` | m.action == 'remove' | lay[m.tracks.track_id] ← 'NONE'; muted[m.tracks.track_id] ← false · 효과 app.tracks | §6-2-2 §4-2-6 — 항목을 지운다 — 화면에서 내린다 · m-line 은 배정표가 따로 다룬다(그 줄은 빈 줄이 된다) |
| <a id="TRK.C.any.track_event"></a>`TRK.C.any.track_event` | 나머지 | lay[m.tracks.track_id] ← m.tracks.active ? 'ACTIVE' : 'RESTING'; muted[m.tracks.track_id] ← m.tracks.muted · 효과 app.tracks | §6-2-2 §4-2-6 — 트랙 층은 통째 교체 — 받는 트랙(active:true) · 쉬는 트랙(active:false) · 화면만 고친다(SDP 는 배정표 몫) |
| <a id="TRK.C.any.track_state"></a>`TRK.C.any.track_state` | 나머지 | muted[m.track_id] ← m.muted · 효과 app.track_state | §4-2-6 |
| <a id="TRK.C.any.snapshot"></a>`TRK.C.any.snapshot` | 받는 효과 ROOM trk.snapshot · sLay != lay \|\| sMuted != muted | lay ← sLay; muted ← sMuted · 효과 app.tracks | §6-1-1 §6-2-2 §3-2-3 3 — 스냅샷은 목록 전부라 목록을 통째로 바꾼다 · 화면만 고친다(SDP 는 배정표 몫이라 재협상이 없다) |
| <a id="TRK.C.any.snapshot_same"></a>`TRK.C.any.snapshot_same` | 받는 효과 ROOM trk.snapshot · 나머지 | — | §3-2-3 3 — 목록 그대로 — 아무것도 하지 않는다 |
| <a id="TRK.C.any.store_drop"></a>`TRK.C.any.store_drop` | 받는 효과 ROOM store.drop · 나머지 | lay ← {A: 'NONE', B: 'NONE', S: 'NONE'}; muted ← {A: false, B: false, S: false} | §4-2-5 §6-2-2 |

<a id="TRK.S"></a>
#### 6-2-2. 서버

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="TRK.S.pub_same"></a>`TRK.S.pub_same` | 받는 효과 PUB trk.pub_add · PUB trk.pub_remove · PUB trk.pub_duplex · AFF trk.pub_move · dup[t] == d | — | §6-2-2 |
| <a id="TRK.S.pub_full"></a>`TRK.S.pub_full` | 받는 효과 PUB trk.pub_add · PUB trk.pub_remove · PUB trk.pub_duplex · AFF trk.pub_move · d == 'FULL' && dup[t] == 'NONE' | dup[t] ← 'FULL'; muted[t] ← false; lay[t] ← 'ACTIVE' · 답한다 TRACK_EVENT.body {action: 'add', tracks: {track_id: t, active: true, muted: false}, version: {epoch: epoch, seq: seq + 1}} | §6-2-2 |
| <a id="TRK.S.half_to_full"></a>`TRK.S.half_to_full` | 받는 효과 PUB trk.pub_add · PUB trk.pub_remove · PUB trk.pub_duplex · AFF trk.pub_move · d == 'FULL' | dup[t] ← 'FULL'; muted[t] ← false; lay[t] ← 'ACTIVE' · 답한다 TRACK_EVENT.body {action: 'add', tracks: {track_id: t, active: true, muted: false}, version: {epoch: epoch, seq: seq + 1}} | §6-2-2 |
| <a id="TRK.S.pub_half"></a>`TRK.S.pub_half` | 받는 효과 PUB trk.pub_add · PUB trk.pub_remove · PUB trk.pub_duplex · AFF trk.pub_move · d == 'HALF' && dup[t] == 'NONE' | dup[t] ← 'HALF' | §6-2-2 |
| <a id="TRK.S.full_to_half"></a>`TRK.S.full_to_half` | 받는 효과 PUB trk.pub_add · PUB trk.pub_remove · PUB trk.pub_duplex · AFF trk.pub_move · d == 'HALF' | dup[t] ← 'HALF'; muted[t] ← false; lay[t] ← 'RESTING' · 답한다 TRACK_EVENT.body {action: 'add', tracks: {track_id: t, active: false, muted: false}, version: {epoch: epoch, seq: seq + 1}} | §6-2-2 |
| <a id="TRK.S.unpub_full"></a>`TRK.S.unpub_full` | 받는 효과 PUB trk.pub_add · PUB trk.pub_remove · PUB trk.pub_duplex · AFF trk.pub_move · dup[t] == 'FULL' | dup[t] ← 'NONE'; muted[t] ← false; lay[t] ← 'NONE' · 답한다 TRACK_EVENT.body {action: 'remove', tracks: {track_id: t, active: true, muted: false}, version: {epoch: epoch, seq: seq + 1}} | §6-2-2 §4-2-6 |
| <a id="TRK.S.unpub_resting"></a>`TRK.S.unpub_resting` | 받는 효과 PUB trk.pub_add · PUB trk.pub_remove · PUB trk.pub_duplex · AFF trk.pub_move · lay[t] == 'RESTING' | dup[t] ← 'NONE'; muted[t] ← false; lay[t] ← 'NONE' · 답한다 TRACK_EVENT.body {action: 'remove', tracks: {track_id: t, active: false, muted: false}, version: {epoch: epoch, seq: seq + 1}} | §6-2-2 |
| <a id="TRK.S.unpub_half"></a>`TRK.S.unpub_half` | 받는 효과 PUB trk.pub_add · PUB trk.pub_remove · PUB trk.pub_duplex · AFF trk.pub_move · 나머지 | dup[t] ← 'NONE' | §6-2-2 |
| <a id="TRK.S.rest_in"></a>`TRK.S.rest_in` | 받는 효과 AFF trk.pub_move · 받는 상태 lay[t] == 'NONE' · 나머지 | dup[t] ← 'HALF'; muted[t] ← false; lay[t] ← 'RESTING' · 답한다 TRACK_EVENT.body {action: 'add', tracks: {track_id: t, active: false, muted: false}, version: {epoch: epoch, seq: seq + 1}} | §5-1-2 §4-2-6 |
| <a id="TRK.S.mute"></a>`TRK.S.mute` | 받는 효과 PUB trk.pub_muted · 나머지 | muted[t] ← v · 답한다 TRACK_STATE.body {track_id: t, muted: v, version: {epoch: epoch, seq: seq + 1}} | §4-2-6 §4-2-6 |

<a id="RULE-TRK-PUB-TO-TRACK"></a>**RULE-TRK-PUB-TO-TRACK** · 규칙 — ★층마다 규칙이 다르다 — 발행 → 트랙 대응 · 갱신 · `seq`

**발행 → 트랙 대응** — 모든 판정의 출처를 이 표 하나로 둔다.

| 사건(발행 층) | 트랙 층에 일어나는 일 |
|---|---|
| 전이중 발행(`PUBLISH_TRACKS{duplex:"full"}`) | `individual` 트랙 생성 — 그 방 전원에게 `TRACK_EVENT{add}` |
| ★반이중 발행(`duplex:"half"`) | ★**없음** — 소리 · 영상은 방 슬롯으로 흐르고 화자는 발언권 축이 준다(§10-4-35) |
| full → half(`TRACK_SET{duplex}`) | 그 `individual` 트랙 `active:false`(항목 교체 `add`) · ★배정표에서 그 줄은 빈 줄(서버가 자리를 놓는다) |
| half → full | 같은 `individual` 트랙 `active:true`(항목 교체 `add`) · 배정표에 줄이 다시 붙는다(빈 줄 · 새 줄 · 상한이면 서버가 고를 때 — §8-1-2) |
| 방 생성 | `slot` audio · video 트랙 둘(상존 — 코덱은 방 속성 `media_codecs`, §4-1-1 · SSRC 는 구독자 연결마다 배정표 줄의 다운링크 SSRC) |

<a id="RULE-TRK-FIVE-RULES"></a>**RULE-TRK-FIVE-RULES** · 규칙

★**규칙 다섯** — ①구독자는 트랙 층만 본다(발행자의 `duplex` 를 모른다) ②`seq++` ⟺ 명단 또는 트랙 층(목록·속성)이 바뀌었다(§4-2-6)
③트랙의 `type` 은 바뀌지 않는다(`duplex` 는 발행 속성) ④한 사실은 한 프레임으로 보낸다(전환은 `TRACK_EVENT{add}` 하나) ⑤★**내 발행 상태는
나에게만 온다** — `PUBLISH_TRACKS` 응답과 `RESUME` 응답의 `publications[]`(§3-1). 서버가 알려 주는 방향이라 12차가 폐기한 *"클라 신고"* 와 다르다.

<a id="RULE-TRK-STORE-UPDATE"></a>**RULE-TRK-STORE-UPDATE** · 규칙

**보관본 갱신** — ★**키는 `track_id`** 다.

| 층 | 갱신 | 근거 |
|---|---|---|
| ★트랙 | ★**통째 교체** — 같은 `track_id` 의 항목이 오면 트랙 층 필드 전부를 받은 값으로 바꾼다(필드 병합이 아니다 — 서버가 매번 전 필드를 싣는다) | 방 전원이 같은 값을 본다 — `seq` 가 센다 |
| ★배정 | ★**목록에 없다 — 배정표가 따로 갖는다**(§8-1-2). 표는 WS 로만 오고 늘 표 전체이며, `arev` 가 큰 것만 받는다(§8-1-2) | 연결마다 하나인 것을 방마다인 목록에 섞으면 차례를 셀 곳이 없다 — 표 하나에 차례 번호 하나(§8-4-1) |
| 발행 | 스냅샷에 없다 — `RESUME.publications[]` 와 내 발행 목록을 대조한다(§3-2-3) | 나와 서버만 아는 값이다 |

<a id="RULE-TRK-PER-PATH"></a>**RULE-TRK-PER-PATH** · 규칙

**경로마다 오는 것**

| | `ROOM_JOIN` 응답 · `RESUME.snapshot` | `TRACK_EVENT{add}` | `TRACK_EVENT{remove}` | ★HTTP `?tracks=1` |
|---|---|---|---|---|
| 트랙 층 | 전 필드(type 표대로) | 전 필드 | `type`·`room_id`·`track_id`·`kind` 만(코덱 정보 없음) | 전 필드 |
| ★배정표 | ✔ `ROOM_JOIN` 응답 `mline_table` · `RESUME` 응답 `mline_tables`(서버마다) | ★없다 — 표가 바뀌면 `MLINE_EVENT` 가 따로 온다 | ★없다 | ★**없다** — HTTP 는 보기용이다 |

<a id="RULE-TRK-HALF-NOT-IN-SNAPSHOT"></a>**RULE-TRK-HALF-NOT-IN-SNAPSHOT** · 규칙

- ★★**반이중 개인 트랙은 구독자 스냅샷에 없다**(대응 표). 그래서 반이중 `PUBLISH_TRACKS` 는 `seq` 와 무관하다(§4-2-6).

<a id="RULE-TRK-ROOM-ID-ONLY-MEANS"></a>**RULE-TRK-ROOM-ID-ONLY-MEANS** · 규칙

- ★★**`room_id` 가 트랙을 방에 붙이는 유일한 수단이다.** `track_id` 는 **파싱하지 않으므로**
  거기서 방을 뽑을 수 없고, `user_id` 는 슬롯에 없으며 한 사람이 여러 방에 있을 수도 있다.
  ★다방 청취에서는 **어느 방 화면에 걸 것인가**가 이 필드로만 정해진다.

<a id="RULE-TRK-SLOT-LIFE"></a>**RULE-TRK-SLOT-LIFE** · 규칙

- ★★**슬롯 트랙은 둘이고 늘 있다** — audio · video 슬롯은 방이 생길 때 생기고 방이 없어질 때까지 남는다. 입장 응답부터 목록에 있다. ★코덱 · fmtp 는 방 속성(§4-1-1)이라 처음부터 정해져 있다 — 값 없는 항목이 없다(고정표 폴백 금지와 부딪히지 않는다). 쓰고 안 쓰고는 화자 재량이다. ★반이중 발행 · `pub_select` 가 슬롯을 만들거나 지우지 않는다.

<a id="RULE-TRK-ELEMENT-HAS-ROOM-ID"></a>**RULE-TRK-ELEMENT-HAS-ROOM-ID** · 규칙

- ★★**컨테이너가 방을 정해도 원소는 `room_id` 를 갖는다.** `GET /rooms/{id}` · `RESUME.snapshot`
  · `TRACK_EVENT`(§4-2-6)에서는 잉여로 보이지만, ★**네 경로가 같은 보관본 하나에 들어가고** 그 보관본이 `room_id` 로만
  방을 가른다(§10-4-35). ★**다방을 준다는 뜻이 아니다** — 그 응답은 그 방 것뿐이다. 한 경로에서만
  빼면 클라가 ★**"어느 경로로 온 것인지 기억해 필드를 합성"** 해야 하고, 필드 하나 아끼자고
  그 분기를 만드는 것은 손해다.

→ 구조 — [`TRK.type.TrackEntry`](#TRK.type.TrackEntry)

<a id="PROP-TRK-STORE-AGREES"></a>**PROP-TRK-STORE-AGREES** · 성질

통로가 비면 보관본의 트랙 층이 서버 트랙 층이다 — 넷이 같은 보관본 하나에 들어간다(RULE-TRK-FOUR-PATHS · RULE-TRK-STORE-UPDATE).

*식: TK1 · 깨지면: 쉬는 트랙을 받는 트랙으로 알거나 없어진 트랙을 붙잡는다 — 구독 SDP 가 서버와 다른 목록으로 지어진다*

<a id="PROP-TRK-TABLE-SIGNAL"></a>**PROP-TRK-TABLE-SIGNAL** · 성질

트랙이 받는 트랙이 되는 걸음에만 연결 표에 싣자고(link.track_on) · 그치는 걸음에만 빼자고(link.track_off) 알린다 — 배정표는 연결의 것이라 LINK 가 갖고 TRK 는 트랙 층의 바뀜만 알린다(RULE-TRK-PUB-TO-TRACK).

*식: TK19 · 깨지면: 쉬는 트랙이 줄을 붙잡거나 받는 트랙이 줄을 못 받는다 — 연결 표가 방의 트랙 층과 어긋난다*

<a id="PROP-TRK-SLOT-LIFE"></a>**PROP-TRK-SLOT-LIFE** · 성질

슬롯 트랙(video)은 늘 있다 — 반이중 발행 · 전환 · 발행 방 옮김이 만들거나 지우지 않는다(RULE-TRK-SLOT-LIFE · RULE-ROOM-MEDIA-CODECS).

*식: TK5 · 깨지면: 슬롯이 없어져 다음 화자의 영상을 받을 m-line 이 없거나 · 슬롯이 생길 때마다 방 seq 가 올라 헛 재동기가 돈다*

<a id="PROP-TRK-HALF-NOT-RECEIVED"></a>**PROP-TRK-HALF-NOT-RECEIVED** · 성질

반이중 개인 트랙은 받는 트랙이 아니다 — 반이중 발행만으로는 항목이 없고 전환한 것은 쉰다(RULE-TRK-HALF-NOT-IN-SNAPSHOT · RULE-TRK-PUB-TO-TRACK).

*식: TK6 · 깨지면: 반이중 개인 트랙이 구독자 스냅샷에 실려 seq 가 반이중 발행마다 오른다*

<a id="PROP-TRK-SEQ-COUNTS-LAYER"></a>**PROP-TRK-SEQ-COUNTS-LAYER** · 성질

그 방 seq 는 트랙 층이 바뀔 때만 오른다 — 배정표만 바뀌는 재발급 · 반이중 개인 발행은 올리지 않는다(RULE-TRK-FIVE-RULES ② · RULE-TRK-STORE-UPDATE).

*식: TK7 · 깨지면: 재발급마다 남들이 헛 재동기를 돈다 · 또는 트랙 층 변화를 seq 가 못 세 재동기가 놓친다*

### 6-3. 주제

#### 6-3-1. ★`seq++` 의 조건 — 배달이 아니라 **내용**이다

<a id="PROP-TRK-SWITCH-ONE-ADD"></a>**PROP-TRK-SWITCH-ONE-ADD** · 성질

회의 ↔ 무전 전환(TRACK_SET{duplex})은 그 트랙의 version 실은 `TRACK_EVENT{add}` 하나다 — 잔존 교체(`active:false`) 또는 복귀(`active:true`) · seq 는 그 트랙 몫으로 한 번(RULE-ROOM-DUPLEX-ONE-ADD · RULE-TRK-FIVE-RULES ④). 슬롯이 생기고 없어지는 것은 다른 사실이라 다른 프레임이다.

*식: TK12 · 깨지면: 전환을 두 프레임(remove + add · TRACK_STATE{duplex})으로 보내 하나는 version 규칙에 걸려 버려지거나 seq 를 두 번 올린다*

#### 6-3-2. 미디어

<a id="PROP-TRK-MUTED-FULL-ONLY"></a>**PROP-TRK-MUTED-FULL-ONLY** · 성질

muted 는 전이중 트랙에만 있다 — 반이중에는 mute 가 없고(3006) · duplex 가 바뀌면 false 로 돌아간다(RULE-PUB-SET-PROC).

*식: TK14 · 깨지면: 음소거된 카메라를 무전으로 돌린 뒤 허가를 받아도 검은 화면 · 무음이 나간다*

#### 6-3-3. 통지 — 전부 **S→C**, 공통 의무는 ★**ACK 하나**

<a id="PROP-TRK-MUTED-AGREES"></a>**PROP-TRK-MUTED-AGREES** · 성질

통로가 비면 보관본의 muted 가 서버 트랙 층의 muted 다 — muted 는 트랙 층이라 TRACK_STATE 가 그 방 전원에게 가고 seq 가 오른다(RULE-TRK-TRACK-STATE-TO-ALL · RULE-TRK-STORE-UPDATE).

*식: TK13 · 깨지면: 말하는 사람이 음소거를 풀었는데 내 화면은 음소거 표시로 남는다 · 재동기 뒤 되돌아간다*

#### 6-3-4. `RESUME` 성공 응답을 받았다

<a id="PROP-TRK-PUB-MOVE-FOLLOWS"></a>**PROP-TRK-PUB-MOVE-FOLLOWS** · 성질

발행 방을 옮기면 든 트랙이 전부 따라간다 — 옛 방에는 그 트랙이 없고, 전이중이면 새 방에서 받는 트랙 · 쉬던 전이중이면 새 방에서도 쉬는 항목이다(RULE-AFF-SWITCH-TRACKS · AFF 서버의 trk.pub_move 를 TRK 가 옛 방 NONE · 새 방 그 duplex 로 받는다 · 쉬던 것은 SRestIn).

*식: PM2 · PM6(다판 pub_move) · 깨지면: 옛 방 사람들이 떠난 사람의 영상 자리를 계속 붙잡거나, 새 방 사람들이 말하는 사람의 영상을 못 받는다 · 쉬던 카메라가 새 방에서 사라져 돌아올 자리가 없다*

<a id="PROP-TRK-SLOT-FOLLOWS-PUB"></a>**PROP-TRK-SLOT-FOLLOWS-PUB** · 성질

발행 방을 옮겨도 두 방의 슬롯은 그대로다 — 옛 방에서 없어지지도 새 방에 새로 생기지도 않는다(RULE-TRK-SLOT-LIFE).

*식: PM5(다판 pub_move) · 깨지면: 옮길 때마다 두 방 슬롯이 지워졌다 생겨 seq 가 오르고 구독자가 재협상한다*

### 6-4. 근거 · 기각 — 규격이 아니다

> 계약 본문(구조 · 동작 · 주제)의 규칙이 왜 그런가(R) · 무엇을 하지 않기로 했나(REJ). 소절 머리는 그 근거가 붙은 계약 자리다. 계약 판에는 싣지 않는다.

#### 6-4-1. §6-1-1 형 `TrackEntry` — 받을 트랙 하나

<a id="R-TRK-MUTED-IN-SNAPSHOT"></a>**R-TRK-MUTED-IN-SNAPSHOT** · 근거

★★**`muted` 가 스냅샷에 있어야 하는 이유** — `TRACK_STATE` 로만 오던 값이었다. 그 통지를 놓쳤거나 그 전에
끊겼으면 ★**재접속해도 재동기해도 받아 올 값 자체가 없어** 음소거 표시가 ★**영영 안 맞는다.** 스냅샷에 넣어 정상 축으로 올린다.
★**wire 에 두는 기준은 "받는 쪽이 스스로 알 수 있는가"다** — `muted` 는 알 수 없다(`enabled=false` 면 검은 프레임이 계속 와서
진짜로 까만 장면과 구별되지 않는다). 반대로 ★**프레임이 오는지는 받는 쪽이 로컬에서 관측한다** — 옛 `live` 필드와
`TRACK_STATE{live}` 는 그래서 두지 않는다 — 통지가 키프레임보다 먼저 닿아 막으려던 검은 화면을 만드는 신호이고, 한 번
`true` 면 되돌아오는 경로가 없는 사건이라 스냅샷에 담을 상태가 아니다. 상대 화면의 표시 순서는 `muted`(wire) → 프레임 도착(로컬)이다.

#### 6-4-2. §6-1-2 `0x0702 TRACK_EVENT` — 받을 트랙이 **생기거나 없어졌다**   **S→C** · ACK

<a id="R-TRK-TRACK-EVENT-SYMPTOMS"></a>**R-TRK-TRACK-EVENT-SYMPTOMS** · 근거

★**증상**
- 목록이 바뀌었다고 구독 SDP 를 다시 지으면 → 배정표보다 앞선 목록으로 m-line 을 짓게 된다 — ★SDP 는 표로만 짓는다.
- ★여러 방을 한 메시지에 섞으면 → `version` 하나가 ★**어느 방 것인지 정의되지 않아**
  견주기가 통째로 죽는다 — 낡은 통지를 못 버린다.
- ★**발행자 본인에게 자기 항목을 안 보내면** → 그 사람 보관본에 그 트랙이 없어 `seq` 가 건너뛴 것으로
  보이고, ★**발행할 때마다 자기가 통짜 스냅샷을 받는다.** 본인 항목은 ★**배정표에 줄이 없을 뿐**
  (내 트랙이라 내게 배정할 자리가 없다) 트랙 층은 남들 것과 같다.

#### 6-4-3. §6-1-3 `0x0703 TRACK_STATE` — 트랙 **속성**이 바뀌었다   **S→C** · ACK

<a id="R-TRK-TRACK-STATE-SYMPTOMS"></a>**R-TRK-TRACK-STATE-SYMPTOMS** · 근거

★**증상**
- 무시하면 음소거 표시가 안 바뀐다.
- ★반이중 트랙에 이 통지를 기대하면 → 오지 않는다(반이중에는 mute 가 없다, §7-1-2) — 슬롯 타일의 화자 표시는 발언권 축(§10-4-35)이다.

<a id="PUB"></a>
