[← 차례](README.md)

## 7. PUB — 발행

발행자 × 트랙 — 내가 서버에 올린 트랙의 등록(codec · ssrc · duplex · simulcast)과 발행 상태 · 업링크 스트림 · 발행 m-line

| 정체성 | 누가 내나 | 무엇마다 | 수명 | 상태 |
|---|---|---|---|---|
| track_id | server | 세션 × 연결 | 앱이 켬(IDLE → STAGED) → 앱이 끔 · 서버가 등록을 지움 | IDLE · STAGED · REGISTERED · SENDING |

| 의존 | 기댐 | 다중도 | 뜻 |
|---|---|---|---|
| PUB | LINK | N:1 | 트랙은 그 서버 연결의 트랜시버로 나간다(§9-4-10) |
| PUB | AFF | N:1 | 발행 트랙은 발행 방으로 — 발행 방 전환이 트랙에 하는 일(§5-1-2) |
| PUB | ROOM | N:1 | 전이중 발행은 등록한 방의 것 — 방을 잃으면(퇴장 · 강퇴 · 방 닫힘) 서버가 그 사람의 그 방 발행을 정리한다(원칙 ③ · RULE-ROOM-SEQ-UP) |
| PUB | FLR | N:1 | 반이중 트랙은 발언권을 쥔 동안에만 SENDING(§7-2-2) |

### 7-1. 구조

<a id="PUB.type.Publication"></a>
#### 7-1-1. 형 `Publication` — 발행 층

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `track_id` | str | ✔ |  |
| `room_id` | str | ✔ |  |
| `kind` | "audio" / "video" | ✔ |  |
| `duplex` | "full" / "half" | ✔ |  |
| `codec` | str \| null | ✔ |  |
| `fmtp` | str | — |  |
| `simulcast` | bool | ✔ |  |

<a id="PUB.type.PublishTrack"></a>
#### 7-1-2. 형 `PublishTrack` — `tracks[]` 원소

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `kind` | "audio" / "video" | ✔ | `"audio"` / `"video"` |
| `ssrc` | u32 | ✔ | ★**내 offer 의 그 m-line `a=ssrc` 에서 읽는다**(브라우저가 정한 값이다 — 클라가 짓지 않는다). ★**시뮬캐스트면 `0`** 이 정상(그때는 SDP 에 `a=ssrc` 가 없다) — 시뮬캐스트가 아닌데 `0` 이면 `1003` · ★`simulcast:true` 인데 `0` 이 아니면 `1003`(전체 거절) |
| `mid` | str | ✔ | SDP m-line MID(§6-1-1 과 같은 형). 트랙마다 |
| `codec` | string? | ★video | `"VP8"` / `"H264"` / `"VP9"`. ★**video 는 필수** — 없으면 전체 `1005` |
| ★`fmtp` | string? | ★있으면 | ★★**협상 확정본(answer)의 그 m-line `a=fmtp:{pt}` 파라미터 원문**(PT 제외). ★**kind 를 가리지 않는다** — audio 든 video 든 **확정본에 있으면 반드시 싣는다**(없으면 싣지 않는다 — 지어내지 않는다). ★서버는 이 값으로 방 코덱 검사를 한다(§4-1-1 — H264 는 `packetization-mode` · profile 만 견준다). ★구독 SDP 의 fmtp 는 이 값이 아니라 배정표 줄의 고정 선언값이다(§8-1-1 — 줄을 돌려써서 발행자를 따라가지 않는다). ★**왜 offer 가 아니라 answer 인가** — video 는 서버가 offer 를 그대로 옮기므로(§9-4-10 PT·rtpmap·fmtp) 두 값이 같지만, ★**opus 는 §9-4-10 예외로 answer 가 받는 쪽 선호를 정한다**(`useinbandfec`·`minptime` 등). answer 가 적는 받는 쪽 선호는 방 코덱의 고정 opus 값이다 — 그래야 발행자가 빚는 업링크와 구독 줄의 선언이 같다. ★`1pc` 의 `ROOM_JOIN` 번호표(§4-1)에 실리는 `fmtp` 는 offer 것이고 PT 키 재료일 뿐이라 이 값의 출처가 아니다 — 출처는 이것 하나다. ★**쓰임은 방 코덱 검사와 보기용 목록(`TrackEntry.fmtp`)이다** |
| `pt` | u8 | ✔ | 클라 실사용 PT(내 offer 기준). ★**audio 도 필수다** — 폴백은 없다(§6-1-1 규칙과 같다: 폴백은 무음을 조용히 만든다). 없으면 `1003` |
| `rtx_pt` | u8? | — | 재전송 PT |
| `rtx_ssrc` | u32? | — | 재전송 SSRC (시뮬캐스트가 아닐 때만) |
| `source` | string? | — (기본 "camera") | ★**video 만** — `"camera"`(기본) / `"screen"` ★**둘뿐이다**(닫힌 집합). 그 밖의 값 · audio 항목에 실린 값 → `1002`. ★**클라 신고값이고 서버는 검증하지 않는다** — 권한 비트의 판정 재료다(§4-2-6 사상표) |
| `duplex` | string? | — (기본 "full") | `"full"`(기본) / `"half"`(무전) |
| `simulcast` | bool? | — | ★**없으면 `false`(일반)** — duplex 와 무관하다. 추론하지 않는다. ★실은 값을 `ssrc` 와 대조한다 — `true` 인데 `ssrc ≠ 0` 이면 `1003`, `false`(또는 없음)인데 `ssrc = 0` 이면 `1003`. ★**audio 에 `true` 면 `1002`**(오디오에 시뮬캐스트는 없다) |

<a id="RULE-PUB-REMOVE-FORM"></a>**RULE-PUB-REMOVE-FORM** · 규칙

★**`action:"remove"` 의 요청은 형상이 다르다** — `{ action:"remove", room_id, track_ids:[ "…" ] }`.
★**발행 응답이 준 `track_id` 로 지목한다**(`mid`·`ssrc` 가 아니다 — 위 `tracks[]` 원소 표는 `add` 것이다).
★이미 지워진 발행을 지목한 `remove` 는 `3005 TRACK_NOT_FOUND` 로 실패한다(DEV-PUB-REMOVE-FAIL-3005).

<a id="RULE-PUB-PUBLISH-RES"></a>**RULE-PUB-PUBLISH-RES** · 규칙

응답 — add `{ action:"add", tracks:[ {mid, track_id} ] }`
· remove `{ action:"remove" }` (★`tracks` 필드 자체가 없다. ★`intent` 필드는 없다 — 값이 `true` 하나뿐인 필드라 두지 않는다. 불변이면 필드로 만들지 않는다)

→ 구조 — [`PUB.op.PUBLISH_TRACKS`](#PUB.op.PUBLISH_TRACKS)

<a id="RULE-PUB-PUBLISH-PROC"></a>**RULE-PUB-PUBLISH-PROC** · 규칙

**절차**
- 클라 ★**세 단계로 한다**: ① 트랜시버를 만들고(협상까지, RTP 는 아직 안 보냄) ② 이 메시지로 발행
  ③ RTP 시작. ★**어느 단계에서 실패하든 되돌리기는 하나**로 한다.
- 클라 ★**②가 성공한 뒤 ③에서 실패하면 `action:"remove"` 를 보내 되돌린다.**
- 클라 ★응답의 `track_id` 를 **자기 `mid` 에 대응시켜 보관한다.** 이후 모든 식별이 이 값이다.
- 서버 상한 — **요청당 8개 / 한 사람 활성 16개**(단위 = 트랙 = 요청 항목).
  ★넘으면 **일부도 안 받고 전체를 거절**한다.
- 서버 ★**발행 트랙은 그 방의 미디어 코덱과 같아야** 한다 — 전이중 · 반이중 · audio · video 모두(★파라미터로 견준다 — H264 는 `packetization-mode` · profile 만 · level 은 보지 않는다 · opus · VP8 은 이름만 · 다르면 `1006` — §4-1-1).
- ★★**"그 방의 미디어 코덱" 은 방 속성이다**(§4-1-1 — 방 생성 때 정하고 바뀌지 않는다 · 기본 opus · H264 · 구독 줄의 fmtp 는 코덱마다 고정 선언값).
  입장 응답의 `media_codecs`(§4-1)로 안다. ★**보내기 전에 읽고 맞춘다** — 찍어 보고 `1006` 으로 배우지 않는다.
- 서버 ★★**`1006` 을 돌릴 때 `details.codec`·`details.fmtp` 에 그 방의 미디어 코덱을 싣는다** — 거절 사유의 구체화다. 서버는 고쳐 주지 않고 거절한다.
- 서버 ★★**`1pc` 에서 신고된 `ssrc`·`rtx_ssrc` 가 이 연결에 이미 발급한 다운링크 SSRC(§9-4-2)와 겹치면 전체를 `3008` 로 거절한다**(`details.ssrc`) — 한 BUNDLE 이라 SSRC 공간이 하나다(§9-4-18). 클라는 그 트랜시버를 `inactive` 로 두고 ★**새 트랜시버로 한 번 재발행**한다 — 브라우저가 새 SSRC 를 만든다. 확률이 2 의 −32 승 급이라 한 번이면 끝난다. `2pc` 는 공간이 둘이라 검사하지 않는다.

<a id="RULE-PUB-PUBLISH-FAIL"></a>**RULE-PUB-PUBLISH-FAIL** · 규칙

→ 구조 — [`PUB.op.PUBLISH_TRACKS`](#PUB.op.PUBLISH_TRACKS)

<a id="RULE-PUB-RID-TO-SPATIAL"></a>**RULE-PUB-RID-TO-SPATIAL** · 규칙

★★**`rid` → spatial 인덱스 사상은 서버가 한다.** 발행 항목은 트랙 하나이고 `rid` 를 싣지 않는다 — 서버는 ★**RTP 의 rid 확장으로 업링크 스트림을 배운다**(`rid_extmap_id`). 이 세대의 시뮬캐스트는 업링크 스트림 둘(`l`·`h`) 고정(§9-4-10)이라 ★**낮은 화질부터 `0`**(l=0, h=1)이고, ★**`TrackEntry.scalability` 는 `"L2T1"`**(§6-1-1)로 구독자에게 알린다. 구독자는 ★**그 인덱스로만 지목**하고(§7-1-2 `SUBSCRIBE_LAYER`)
★**`rid` 를 쓰지 않는다** — 보내는 쪽 이름을 받는 쪽 제어에 끌어들이지 않는다.

<a id="PUB.op.PUBLISH_TRACKS"></a>
#### 7-1-3. `0x0301 PUBLISH_TRACKS` — 보낼 트랙 발행   **C→S** · 응답 있음
단 1 · 출처 §7-1-2

**요청**

`action` = `add`

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `action` | string? | — (기본 "add") | `"add"`(기본) / `"remove"` |
| `room_id` | str | ✔ | ★필수. ★**발행 검사의 기준 방**이다(미디어 코덱 `1006` 등). ★**발행 RTP 는 그 순간의 `pub_room` 으로 흐른다** — 발행 방을 옮기면 트랙이 따라간다(§5-1-2 전환 규칙). ★**전이중은 `room_id` 가 `pub_room` 이어야 한다** — 아니면 `3011`. 반이중은 발행한 뒤 `pub_select` 할 수 있다(§10-4-5 0) |
| `tracks[]` | — | ✔ | 아래 |
| `twcc_extmap_id` · `rid_extmap_id` · `repair_rid_extmap_id` · `mid_extmap_id` · `audio_level_extmap_id` | u8 | 확정본(answer)의 그 m-line 에 그 URI 가 있으면 | ★**협상 결과 번호 — 확정본(answer)의 그 m-line 에 그 URI 가 있으면 필수, 없으면 싣지 않는다.** ★**폴백은 없다** — 와이어 번호는 offer(브라우저) 것이라(§9-4-10) 서버 선언값과 늘 다르고, 서버가 틀린 번호로 읽으면 rid·mid 학습과 twcc 가 조용히 죽는다(§6-1-1 이 금지한 고정표 폴백과 같은 형태). 협상했는데 안 실으면 `1003`. ★`abs-send-time` 은 여기 없다 — 서버가 egress 에서 새로 만드는 확장이라(§9-4-2 ②) 발행자 번호가 필요 없다 |
| `rid_extmap_id` | u8 | 확정본(answer)의 그 m-line 에 그 URI 가 있으면 |  |
| `repair_rid_extmap_id` | u8 | 확정본(answer)의 그 m-line 에 그 URI 가 있으면 |  |
| `mid_extmap_id` | u8 | 확정본(answer)의 그 m-line 에 그 URI 가 있으면 |  |
| `audio_level_extmap_id` | u8 | 확정본(answer)의 그 m-line 에 그 URI 가 있으면 |  |

`action` = `remove`

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `action` | "remove" | ✔ |  |
| `room_id` | str | ✔ |  |
| `track_ids` | str[] | ✔ | ★**발행 응답이 준 `track_id` 로 지목한다** |

**응답**

`action` = `add`

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `action` | "add" | ✔ |  |
| `tracks` | {mid, track_id}[] | ✔ |  |

`tracks` 안:

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `tracks.mid` | str | ✔ |  |
| `tracks.track_id` | str | ✔ |  |

`action` = `remove`

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `action` | "remove" | ✔ |  |

**실패** — `3005` remove 의 없는 발행(§7-2-5) — §7-1-2 실패 줄에는 없다 · `3002` 방에 없음 · `3011` 전이중인데 `room_id` 가 `pub_room` 이 아니다 · `1002` body 오류(★모르는 `source` · audio 항목의 `source` 포함) · `1003` `pt`·`ssrc`·`mid` 미신고(audio 포함) · `1005` codec 미지정/미지원 · `4002` 상한 초과 · `1003` 협상한 확장의 `*_extmap_id` 미신고 · `1006` 무전 코덱 불일치(★**`details.codec`·`details.fmtp` 에 그 방의 무전 코덱**) · `3008` `1pc` SSRC 충돌(`details.ssrc`) · `2006` 자격 없음 — `permission` 비트가 `false` 인 종류를 올렸다(★**`details.denied` = 막힌 비트 이름 배열**(`"publish_audio"`·`"publish_video"`·`"publish_screen"`) — 판정은 §4-2-6 사상표. ★**항목 하나가 막히면 요청 전체가 거절된다** — 부분 수용 없음)

<a id="PUB.op.TRACK_SET"></a>
#### 7-1-4. `0x0304 TRACK_SET` — 트랙 상태 바꾸기   **C→S** · 응답 있음
단 1 · 출처 §7-1-2

**요청**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `room_id` | str | ✔ | ★필수 |
| `track_id` | string? | — | ★**이것을 먼저 쓴다** |
| `ssrc` | u32 | `track_id` 가 없을 때 | `track_id` 가 없을 때의 폴백 |
| `muted` | bool? | — | 음소거 — ★**전이중 트랙만.** 반이중 트랙에 오면 `3006`(반이중에는 mute 가 없다 — 송출 여부는 발언권 게이트 하나가 정하고, 상대 화면에 개인 타일이 없어 표시할 자리도 없다) |
| `duplex` | string? | — | `"full"` / `"half"` — ★`"full"` 은 그 방이 `pub_room` 일 때만(아니면 `3011`) |

**응답**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `ssrc` | u32 | ✔ |  |
| `muted` | bool | — |  |
| `duplex` | "full" / "half" | — |  |
| `noop` | bool | — | `noop:true` = 이미 그 상태 |

**실패** — `3005` 트랙 없음 · `3006` **시뮬캐스트 트랙은 무전 전환 불가** · `3006` **반이중 트랙에 `muted`** · `3002` 방에 없음 · `3011` `duplex:"full"` 인데 그 방이 `pub_room` 이 아니다 · `1007` 축이 없거나 겹침

<a id="RULE-PUB-SET-EXCLUSIVE"></a>**RULE-PUB-SET-EXCLUSIVE** · 규칙

★**`muted` 와 `duplex` 는 배타다.** 둘 다 없거나 둘 다 있으면 `1007`.

→ 구조 — [`PUB.op.TRACK_SET`](#PUB.op.TRACK_SET)

<a id="RULE-PUB-SET-RES"></a>**RULE-PUB-SET-RES** · 규칙

응답 `{ ssrc, muted? , duplex? , noop? }` — 요청한 축만 돌아온다. `noop:true` = 이미 그 상태.

→ 구조 — [`PUB.op.TRACK_SET`](#PUB.op.TRACK_SET)

<a id="RULE-PUB-SET-PROC"></a>**RULE-PUB-SET-PROC** · 규칙

**절차**
- 무전↔회의 전환은 ★**이 경로로만** 한다. `0x0301` 로 우회하지 않는다.
- 서버는 **트랙 단위**로 적용한다 — 시뮬캐스트면 ★**모든 업링크 스트림이 같이 바뀐다**.
- 서버 ★**`duplex` 가 적용되면 `muted` 는 `false` 로 돌아간다**(양방향) — 서버는 트랙의 `muted` 를 초기화하고 클라는 `enabled=true` 로 둔다. 없으면 음소거된 전이중 카메라를 반이중으로 바꾼 뒤 허가를 받아도 ★**검은 화면이나 무음**이 나가고, 반이중에는 해제 수단이 없다.
- 서버 회의→무전(`full→half`): 되돌아오면 ★**같은 트랙(`track_id`)이 그대로 산다** — 구독자에게는 ★**그 방 전원에게 `TRACK_EVENT{add}`**(잔존 항목 교체 `active:false`, `seq++`, §4-2-6)로 알린다. ★**쉬는 동안 배정표에 그 줄이 없다** — 서버가 자리를 놓아 그 줄은 빈 줄이 되고 같은 kind 의 다른 트랙에 갈 수 있다(§6-1-1).
- 서버 무전→회의(`half→full`): 그 트랙의 발언권을 먼저 해제하고 ★**`TRACK_EVENT{add}`**(`active:true`)로 알리고, 그 트랙의 줄이 붙은 배정표(`MLINE_EVENT`)를 구독자마다 따로 보낸다 — 줄은 놓았던 그 자리일 수도 다른 자리일 수도 있다(§6-1-1 · §8-1-2).

<a id="RULE-PUB-SET-FAIL"></a>**RULE-PUB-SET-FAIL** · 규칙

→ 구조 — [`PUB.op.TRACK_SET`](#PUB.op.TRACK_SET)

<a id="PUB.state"></a>
#### 7-1-5. 상태 칸 · 시한

##### 상태 — 클라

| 칸 | 처음 | 뜻 |
|---|---|---|
| `sfu` | `0` | ★이 트랙이 붙은 미디어 서버 · 발행한 방 — 앱이 트랙을 만들 때 정하는 개체 신원(0 = 모형의 개체 하나 · 축 B) |
| `room` | `0` |  |
| `ps` | `'IDLE'` | 발행 상태(§7-2-2) |
| `want` | `false` | 앱이 켜 두려 한다(카메라 · 마이크를 켰다) |
| `half` | `false` | 반이중 트랙이다 — 발언권을 쥔 동안에만 SENDING(§7-2-2). 모형에서는 바뀌지 않는다 |
| `nego` | `false` | 발행 협상이 진행 중이다(브라우저 — §7-2-1 · §7-2-2) |

##### 상태 — 서버

| 칸 | 처음 | 뜻 |
|---|---|---|
| `reg` | `false` | 발행이 있다(track_id 를 줬다) |
| `perm` | `true` | 권한 비트가 그 종류를 허용한다(§4-2-6) |
| `half` | `false` | 반이중 발행이다 — 그 서버의 마지막 방을 나갈 때까지 산다(RULE-FLR-PROC-PRESS 0) |
| `full` | `false` | 트랙 상한에 닿았다(4002) |
| `atPub` | `true` | 이 트랙의 방이 그 세션의 발행 방(`pub_room`)이다 — 소속 축(AFF)이 준다 · 전이중은 여기서만(3011 · RULE-AFF-SWITCH-TRACKS) |
| `muted` | `false` | 음소거 — 전이중 트랙만(반이중에는 mute 가 없다 · duplex 가 적용되면 false 로 — RULE-PUB-SET-PROC) |

##### 전이 — 클라(동작 경우에서 짓는다)

| 지금 | 사건 | 다음 | 라벨 |
|---|---|---|---|
| `IDLE` | 앱이 카메라 · 마이크를 켰다(§7-2-1) | `STAGED` | `PUB.C.IDLE.on` |
| `STAGED` | 발행 협상이 끝났다(§7-2-2) | `IDLE` | `PUB.C.STAGED.nego_ok_unwanted` |
| `STAGED` | 발행 협상이 끝났다(§7-2-2) | 그대로 | `PUB.C.STAGED.nego_ok` |
| `STAGED` | 발행 협상이 끝났다(§7-2-2) | `IDLE` | `PUB.C.STAGED.nego_fail` |
| 어느 상태 | PUBLISH_TRACKS 응답을 받았다(§7-2-2 · §7-1-2) | — | `PUB.C.any.pub_res_other` |
| `STAGED` | PUBLISH_TRACKS 응답을 받았다(§7-2-2 · §7-1-2) | `IDLE` | `PUB.C.STAGED.pub_ok_unwanted` |
| `STAGED` | PUBLISH_TRACKS 응답을 받았다(§7-2-2 · §7-1-2) | `REGISTERED` | `PUB.C.STAGED.pub_ok_half` |
| `STAGED` | PUBLISH_TRACKS 응답을 받았다(§7-2-2 · §7-1-2) | `REGISTERED` | `PUB.C.STAGED.pub_ok` |
| `STAGED` | PUBLISH_TRACKS 응답을 받았다(§7-2-2 · §7-1-2) | `IDLE` | `PUB.C.STAGED.pub_fail` |
| `REGISTERED` | RTP 를 시작했다 — 실제 트랙을 트랜시버에 붙였다(§7-2-4) | 그대로 | `PUB.C.REGISTERED.attach_ok_half` |
| `REGISTERED` | RTP 를 시작했다 — 실제 트랙을 트랜시버에 붙였다(§7-2-4) | `SENDING` | `PUB.C.REGISTERED.attach_ok` |
| `REGISTERED` | RTP 를 시작했다 — 실제 트랙을 트랜시버에 붙였다(§7-2-4) | `IDLE` | `PUB.C.REGISTERED.attach_fail` |
| `SENDING` | 앱이 껐다(§7-2-5) | `IDLE` | `PUB.C.SENDING.off` |
| `REGISTERED` | 앱이 껐다(§7-2-5) | `IDLE` | `PUB.C.REGISTERED.off` |
| `STAGED` | 앱이 껐다(§7-2-5) | 그대로 | `PUB.C.STAGED.off` |
| `IDLE` | 앱이 껐다(§7-2-5) | 그대로 | `PUB.C.IDLE.off` |
| `SENDING` | 내 track_id 의 TRACK_EVENT{remove} — 서버가 내 발행을 지웠다(§7-2-5) | `IDLE` | `PUB.C.SENDING.revoked` |
| `REGISTERED` | 내 track_id 의 TRACK_EVENT{remove} — 서버가 내 발행을 지웠다(§7-2-5) | `IDLE` | `PUB.C.REGISTERED.revoked` |
| 어느 상태 | 내 track_id 의 TRACK_EVENT{remove} — 서버가 내 발행을 지웠다(§7-2-5) | — | `PUB.C.any.revoked_other` |
| 어느 상태 | 그 서버 재구축 — 다시 발행한다(RULE-LINK-RECOVERY-REBUILD · 절차는 §7-2-1) | `STAGED` | `PUB.C.any.reregister` |
| 어느 상태 | 그 서버 재구축 — 다시 발행한다(RULE-LINK-RECOVERY-REBUILD · 절차는 §7-2-1) | `IDLE` | `PUB.C.any.reregister_off` |
| `REGISTERED` | 발언권을 얻었다 — 반이중 트랙이 보내기 시작한다(§7-2-2) | `SENDING` | `PUB.C.REGISTERED.floor_open` |
| 어느 상태 | 발언권을 얻었다 — 반이중 트랙이 보내기 시작한다(§7-2-2) | — | `PUB.C.any.floor_open_other` |
| `SENDING` | 발언권을 놓았다 · 잃었다 — 반이중 트랙이 보내기를 멈춘다(§7-2-2) | `REGISTERED` | `PUB.C.SENDING.floor_close` |
| 어느 상태 | 발언권을 놓았다 · 잃었다 — 반이중 트랙이 보내기를 멈춘다(§7-2-2) | — | `PUB.C.any.floor_close_other` |
| 어느 상태 | RESUME 성공 — 응답의 publications[] 에 내 발행이 있나(listed)를 본다(RULE-SES-RES-RESUME-OK 2 ③ · 4) · 끊긴 동안 사라진 요청 · 응답도 이것으로 잡는다 | — | `PUB.C.any.resume_unlisted` |
| `STAGED` | RESUME 성공 — 응답의 publications[] 에 내 발행이 있나(listed)를 본다(RULE-SES-RES-RESUME-OK 2 ③ · 4) · 끊긴 동안 사라진 요청 · 응답도 이것으로 잡는다 | `IDLE` | `PUB.C.STAGED.resume_listed_unwanted` |
| `STAGED` | RESUME 성공 — 응답의 publications[] 에 내 발행이 있나(listed)를 본다(RULE-SES-RES-RESUME-OK 2 ③ · 4) · 끊긴 동안 사라진 요청 · 응답도 이것으로 잡는다 | `REGISTERED` | `PUB.C.STAGED.resume_listed_half` |
| `STAGED` | RESUME 성공 — 응답의 publications[] 에 내 발행이 있나(listed)를 본다(RULE-SES-RES-RESUME-OK 2 ③ · 4) · 끊긴 동안 사라진 요청 · 응답도 이것으로 잡는다 | `REGISTERED` | `PUB.C.STAGED.resume_listed` |
| `STAGED` | RESUME 성공 — 응답의 publications[] 에 내 발행이 있나(listed)를 본다(RULE-SES-RES-RESUME-OK 2 ③ · 4) · 끊긴 동안 사라진 요청 · 응답도 이것으로 잡는다 | 그대로 | `PUB.C.STAGED.resume_unlisted` |
| `STAGED` | RESUME 성공 — 응답의 publications[] 에 내 발행이 있나(listed)를 본다(RULE-SES-RES-RESUME-OK 2 ③ · 4) · 끊긴 동안 사라진 요청 · 응답도 이것으로 잡는다 | `IDLE` | `PUB.C.STAGED.resume_unlisted_unwanted` |
| 어느 상태 | RESUME 성공 — 응답의 publications[] 에 내 발행이 있나(listed)를 본다(RULE-SES-RES-RESUME-OK 2 ③ · 4) · 끊긴 동안 사라진 요청 · 응답도 이것으로 잡는다 | — | `PUB.C.any.resume_orphan` |
| 어느 상태 | RESUME 성공 — 응답의 publications[] 에 내 발행이 있나(listed)를 본다(RULE-SES-RES-RESUME-OK 2 ③ · 4) · 끊긴 동안 사라진 요청 · 응답도 이것으로 잡는다 | — | `PUB.C.any.resume_listed` |
| 어느 상태 | 그 서버 연결이 닫혔다 — 서버가 Peer 와 함께 발행 트랙을 풀었다(RULE-ROOM-LEAVE-RES 2) | `IDLE` | `PUB.C.any.drop_server` |
| 어느 상태 | 발행한 방을 잃었다(퇴장 · 강퇴 · 방 닫힘 · 스냅샷에 없는 방) — 서버가 그 방의 전이중 발행을 정리했다(원칙 ③) | `IDLE` | `PUB.C.any.room_gone` |
| 어느 상태 | 발행한 방을 잃었다(퇴장 · 강퇴 · 방 닫힘 · 스냅샷에 없는 방) — 서버가 그 방의 전이중 발행을 정리했다(원칙 ③) | — | `PUB.C.any.room_gone_other` |

<a id="PUB.C"></a>
### 7-2. 동작

<a id="PUB.C.IDLE"></a>
#### 7-2-1. 클라 · `IDLE`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="PUB.C.IDLE.on"></a>`PUB.C.IDLE.on` | 받는 상태 ps == 'IDLE' · 나머지 | ps ← 'STAGED'; want ← true; nego ← true · 효과 sdp.rebuild → LINK.C.any.renego_queued · LINK.C.any.renego | §7-2-1 1 §7-2-1 2 §7-2-1 3 |
| <a id="PUB.C.IDLE.off"></a>`PUB.C.IDLE.off` | 나머지 | want ← false | §7-2-5 DEV-PUB-OFF-BEFORE-SENDING — 협상이 실패해 IDLE 로 돌아온 뒤(바라는 것은 남아 있다) 끈다 — 바라는 것만 내린다 · STAGED 와 같다(칸 전수가 드러냄) |

<a id="RULE-PUB-ON"></a>**RULE-PUB-ON** · 규칙 — 앱이 카메라·마이크를 켰다 (`IDLE`)

1. 트랜시버를 만들고 **발행 SDP 를 다시 조립한다**(§9-4-6).
   ★`1pc` 면 ★**§9-2-1 절차를 탄다**(트랜시버를 늘리지 않고 `replaceTrack` 으로 붙인다).

→ 구조 — [`PUB.C.REGISTERED.attach_ok`](#PUB.C.REGISTERED.attach_ok)

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`PUB.C.IDLE.on`](#PUB.C.IDLE.on) · [`PUB.C.any.reregister`](#PUB.C.any.reregister) |
| 2 | [`PUB.C.IDLE.on`](#PUB.C.IDLE.on) |
| 3 | [`PUB.C.IDLE.on`](#PUB.C.IDLE.on) |

<a id="PUB.C.STAGED"></a>
#### 7-2-2. 클라 · `STAGED`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="PUB.C.STAGED.nego_ok_unwanted"></a>`PUB.C.STAGED.nego_ok_unwanted` | 받는 효과 LINK pub.nego_ok · LINK pub.nego_fail · 받는 상태 ps == 'STAGED' · ok && !want | ps ← 'IDLE'; nego ← false | §7-2-2 §7-1-2 DEV-PUB-OFF-BEFORE-SENDING — 원문은 SENDING 에서 끈 것만 정한다 — 되돌리기 하나(§7-2-2 · §7-1-2 절차)로 되돌린다 |
| <a id="PUB.C.STAGED.nego_ok"></a>`PUB.C.STAGED.nego_ok` | 받는 효과 LINK pub.nego_ok · LINK pub.nego_fail · 받는 상태 ps == 'STAGED' · ok | nego ← false · 보낸다 PUBLISH_TRACKS.req {action: 'add'} | §7-2-2 1 §7-2-2 2 |
| <a id="PUB.C.STAGED.nego_fail"></a>`PUB.C.STAGED.nego_fail` | 받는 효과 LINK pub.nego_ok · LINK pub.nego_fail · 받는 상태 ps == 'STAGED' · 나머지 | ps ← 'IDLE'; nego ← false · 효과 app.publish_failed | §7-2-2 — 협상 실패 — 되돌리기는 하나(§7-2-2 · §7-1-2 절차 ①~③) |
| <a id="PUB.C.STAGED.pub_ok_unwanted"></a>`PUB.C.STAGED.pub_ok_unwanted` | m.part == 'res' && !want | ps ← 'IDLE' · 보낸다 PUBLISH_TRACKS.req {action: 'remove'} | §7-1-2 §7-2-2 DEV-PUB-OFF-BEFORE-SENDING — 원문은 SENDING 에서 끈 것만 정한다 — 되돌리기 하나(§7-2-2 · §7-1-2 절차)로 되돌린다 |
| <a id="PUB.C.STAGED.pub_ok_half"></a>`PUB.C.STAGED.pub_ok_half` | m.part == 'res' && half | ps ← 'REGISTERED' · 효과 floor.reg_half → FLR.C.any.reg_half | §7-2-2 1 §7-2-2 2 §7-2-2 DEV-FLR-RECVONLY-TRIGGER — 반이중 발행 — 그 서버의 마지막 방을 나갈 때까지 산다 · 발언권에 알린다(거절 사유 5 의 방아쇠) |
| <a id="PUB.C.STAGED.pub_ok"></a>`PUB.C.STAGED.pub_ok` | m.part == 'res' | ps ← 'REGISTERED' | §7-2-2 1 §7-2-2 2 |
| <a id="PUB.C.STAGED.pub_fail"></a>`PUB.C.STAGED.pub_fail` | 나머지 | ps ← 'IDLE' · 효과 app.publish_failed | §7-2-2 §7-1-2 — 발행 실패 — 되돌리기는 하나(§7-2-2) |
| <a id="PUB.C.STAGED.off"></a>`PUB.C.STAGED.off` | ps == 'STAGED' | want ← false | §7-2-5 DEV-PUB-OFF-BEFORE-SENDING — 원문은 SENDING 에서 끈 것만 정한다 — 되돌리기 하나(§7-2-2 · §7-1-2 절차)로 되돌린다 |
| <a id="PUB.C.STAGED.resume_listed_unwanted"></a>`PUB.C.STAGED.resume_listed_unwanted` | 받는 효과 SES pub.resume_check · ps == 'STAGED' && !nego && listed && !want | ps ← 'IDLE' · 보낸다 PUBLISH_TRACKS.req {action: 'remove'} | §3-2-3 2 §7-1-2 DEV-PUB-RESUME-INFLIGHT — 발행 요청은 닿았고 응답이 사라졌다 · 그 사이 앱이 껐다 — 응답을 받은 것과 같다(PUB.C.STAGED.pub_ok_unwanted) |
| <a id="PUB.C.STAGED.resume_listed_half"></a>`PUB.C.STAGED.resume_listed_half` | 받는 효과 SES pub.resume_check · ps == 'STAGED' && !nego && listed && half | ps ← 'REGISTERED' · 효과 floor.reg_half → FLR.C.any.reg_half | §3-2-3 2 §7-2-2 1 §7-2-2 DEV-PUB-RESUME-INFLIGHT — 발행 요청은 닿았고 응답이 사라졌다 — 응답을 받은 것과 같다(PUB.C.STAGED.pub_ok_half) |
| <a id="PUB.C.STAGED.resume_listed"></a>`PUB.C.STAGED.resume_listed` | 받는 효과 SES pub.resume_check · ps == 'STAGED' && !nego && listed | ps ← 'REGISTERED' | §3-2-3 2 §7-2-2 1 DEV-PUB-RESUME-INFLIGHT — 발행 요청은 닿았고 응답이 사라졌다 — 응답을 받은 것과 같다(PUB.C.STAGED.pub_ok) |
| <a id="PUB.C.STAGED.resume_unlisted"></a>`PUB.C.STAGED.resume_unlisted` | 받는 효과 SES pub.resume_check · ps == 'STAGED' && !nego && want | 보낸다 PUBLISH_TRACKS.req {action: 'add'} | §3-2-3 2 §7-2-2 1 DEV-PUB-RESUME-INFLIGHT — 발행 요청이 닿지 않았거나 거절됐고 그 응답이 사라졌다 — 서버에 없음을 확인했으니 새 요청으로 다시 묻는다(중복 발행이 아니다 · RULE-SES-RES-BACKOFF-END 2 의 재전송과 다르다) |
| <a id="PUB.C.STAGED.resume_unlisted_unwanted"></a>`PUB.C.STAGED.resume_unlisted_unwanted` | 받는 효과 SES pub.resume_check · ps == 'STAGED' && !nego | ps ← 'IDLE' | §3-2-3 2 §7-1-2 DEV-PUB-RESUME-INFLIGHT — 서버에 없고 앱도 껐다 — 할 일 없이 IDLE |

<a id="RULE-PUB-STATES"></a>**RULE-PUB-STATES** · 규칙 — 발행 상태 — 트랙마다 따로 갖는다

| 상태 | 뜻 |
|---|---|
| `IDLE` | 없다 |
| `STAGED` | 트랜시버를 만들고 협상까지 했다. **RTP 는 아직 안 보낸다** |
| `REGISTERED` | `PUBLISH_TRACKS` 가 성공해 `track_id` 를 받았다 |
| `SENDING` | RTP 를 보내고 있다. ★반이중 트랙은 발화 중에만 여기다 — 발행된 채 `REGISTERED` 에 머무는 것이 정상이고, 그것이 "살아 있는 발행"이다(§3-2-3 4 의 생존 기준) |

<a id="RULE-PUB-ONE-ROLLBACK"></a>**RULE-PUB-ONE-ROLLBACK** · 규칙

★**`STAGED` → `REGISTERED` → `SENDING` 을 한 덩어리로 다룬다.** 어디서 실패하든 되돌리기는 하나다(§7-2-3).

→ 구조 — [`PUB.C.STAGED.nego_fail`](#PUB.C.STAGED.nego_fail) · [`PUB.C.STAGED.pub_fail`](#PUB.C.STAGED.pub_fail) · [`PUB.C.REGISTERED.attach_fail`](#PUB.C.REGISTERED.attach_fail)

<a id="RULE-PUB-NEGO-OK"></a>**RULE-PUB-NEGO-OK** · 규칙 — 협상이 성공했다 (`STAGED`)



항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`PUB.C.STAGED.nego_ok`](#PUB.C.STAGED.nego_ok) · [`PUB.C.STAGED.resume_unlisted`](#PUB.C.STAGED.resume_unlisted) |
| 2 | [`PUB.C.STAGED.nego_ok`](#PUB.C.STAGED.nego_ok) |

<a id="RULE-PUB-OK"></a>**RULE-PUB-OK** · 규칙 — `PUBLISH_TRACKS` 성공 응답을 받았다 (`STAGED`)

1. 응답의 `track_id` 를 ★**자기 `mid` 에 대응시켜 보관한다.** 이후 모든 식별이 이 값이다.

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`PUB.C.STAGED.pub_ok_half`](#PUB.C.STAGED.pub_ok_half) · [`PUB.C.STAGED.pub_ok`](#PUB.C.STAGED.pub_ok) · [`PUB.C.STAGED.resume_listed_half`](#PUB.C.STAGED.resume_listed_half) · [`PUB.C.STAGED.resume_listed`](#PUB.C.STAGED.resume_listed) |
| 2 | [`PUB.C.STAGED.pub_ok_half`](#PUB.C.STAGED.pub_ok_half) · [`PUB.C.STAGED.pub_ok`](#PUB.C.STAGED.pub_ok) |

<a id="PUB.C.any"></a>
#### 7-2-3. 클라 · `any`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="PUB.C.any.pub_res_other"></a>`PUB.C.any.pub_res_other` | ps != 'STAGED' \|\| (m.part == 'res' && m.action == 'remove') \|\| m.code == 3005 | — | §7-2-3 §7-1-2 — remove 의 응답(성공 { action:"remove" } · 3005) — 원문은 remove 응답을 기다리지 않는다(§7-2-5 3). 응답의 action 이 add 와 remove 를 가른다(§7-1-2) |
| <a id="PUB.C.any.revoked_other"></a>`PUB.C.any.revoked_other` | 받는 효과 TRK pub.revoked · 나머지 | — | §7-2-5 — IDLE · STAGED 에서 받은 내 트랙 remove — 원문이 정하지 않는다(무시) |
| <a id="PUB.C.any.reregister"></a>`PUB.C.any.reregister` | 받는 효과 LINK pub.reregister_server · SES pub.reregister_all · want | ps ← 'STAGED'; nego ← true · 효과 sdp.rebuild → LINK.C.any.renego_queued · LINK.C.any.renego | §9-2-1 2 §7-2-1 1 — 옛 연결의 트랜시버 · 발행은 연결과 함께 사라졌다 — 처음 켤 때와 같은 절차 |
| <a id="PUB.C.any.reregister_off"></a>`PUB.C.any.reregister_off` | 받는 효과 LINK pub.reregister_server · SES pub.reregister_all · 나머지 | ps ← 'IDLE'; nego ← false | §9-2-1 2 — 앱이 꺼 둔 트랙 — 다시 발행하지 않는다 |
| <a id="PUB.C.any.floor_open_other"></a>`PUB.C.any.floor_open_other` | 받는 효과 FLR mic.open · 나머지 | — | §7-2-2 — 전이중 트랙 · 발행 전 — 발언권과 무관 |
| <a id="PUB.C.any.floor_close_other"></a>`PUB.C.any.floor_close_other` | 받는 효과 FLR mic.close · 나머지 | — | §7-2-2 — 전이중 트랙 · 보내지 않는 중 — 발언권과 무관 |
| <a id="PUB.C.any.resume_unlisted"></a>`PUB.C.any.resume_unlisted` | 받는 효과 SES pub.resume_check · (ps == 'REGISTERED' \|\| ps == 'SENDING') && !listed | 효과 link.pub_lost → LINK.C.REBUILD.publost_absorbed · LINK.C.any.publost_rebuild | §3-2-3 2 §3-2-3 4 — 서버가 내 발행을 모른다(끊긴 동안 지웠고 그 통지가 사라졌다 · 서버가 연결을 모른다) — 그 서버만 재구축한다(RULE-SES-RES-RESUME-OK ③ · 4 — 다른 서버는 손대지 않는다) |
| <a id="PUB.C.any.resume_orphan"></a>`PUB.C.any.resume_orphan` | 받는 효과 SES pub.resume_check · (ps == 'IDLE' \|\| (ps == 'STAGED' && nego)) && listed | 보낸다 PUBLISH_TRACKS.req {action: 'remove'} | §3-2-3 2 §7-1-2 DEV-PUB-RESUME-INFLIGHT — 내가 갖지 않은 발행이 서버에 남았다(끊긴 동안 remove 가 사라졌다) — remove 를 다시 보낸다 · 상태는 그대로 |
| <a id="PUB.C.any.resume_listed"></a>`PUB.C.any.resume_listed` | 받는 효과 SES pub.resume_check · 나머지 | — | §3-2-3 3 — 발행이 응답에 있다 · 또는 발행 전이고 서버에도 없다 — 이어 쓴다 |
| <a id="PUB.C.any.drop_server"></a>`PUB.C.any.drop_server` | 받는 효과 LINK pub.drop_server · 나머지 | ps ← 'IDLE'; nego ← false | §4-2-5 2 — 클라는 PUBLISH_TRACKS{remove} 를 보내지 않고 발행 상태를 IDLE 로 내린다 — 트랙은 세션 것(앱이 켜 둔 의지 want 는 그대로) |
| <a id="PUB.C.any.room_gone"></a>`PUB.C.any.room_gone` | 받는 효과 ROOM pub.room_gone · !half && (ps == 'REGISTERED' \|\| ps == 'SENDING') | ps ← 'IDLE'; nego ← false · 효과 track.detach | §4-2-6 §4-2-5 — 클라는 PUBLISH_TRACKS{remove} 를 보내지 않는다 — 서버가 이미 정리했다(RULE-ROOM-SEQ-UP · RULE-ROOM-LEAVE-RES 2 와 같은 꼴) |
| <a id="PUB.C.any.room_gone_other"></a>`PUB.C.any.room_gone_other` | 받는 효과 ROOM pub.room_gone · 나머지 | — | §4-2-5 — 반이중 발행은 그 서버의 마지막 방을 나갈 때까지 산다 · 발행 전이면 할 일이 없다 |

<a id="RULE-PUB-2PC-1PC"></a>**RULE-PUB-2PC-1PC** · 규칙 — 트랙을 보내기 — 상태: 발행

★★**아래는 `2pc` 서술이다.** ★`1pc` 는 협상이 ★**언제나 서버 offer**(클라가 합성)이고
연결이 하나라 ★**§9-2-1 절차를 탄다** — 협상 큐 · `rollback` 가드 · 통합 offer.
★**바뀌는 것은 "어떻게 협상하나" 뿐이고, 상태 전이와 보내는 메시지는 같다.**

<a id="PROP-PUB-NO-GHOST"></a>**PROP-PUB-NO-GHOST** · 성질

통로가 비면 서버 발행이 있는 것과 클라가 REGISTERED · SENDING 인 것이 같다.

*식: PH1 · 깨지면: 유령 발행이 상한을 소진해 나중에 `4002` 가 잦아진다(§7-2-4 증상) · 또는 죽은 발행으로 RTP 가 나간다(§7-2-5 증상)*

<a id="PROP-PUB-ON-ENDS"></a>**PROP-PUB-ON-ENDS** · 성질

켜기는 끝난다 — STAGED 는 결국 벗어나고(협상 · 발행 요청이 끊김 뒤에도 답을 받는다) · REGISTERED(전이중)는 결국 송출하거나 되돌린다(RULE-PUB-ONE-ROLLBACK · RULE-SES-RES-RESUME-OK · ★활성 성질 — 통 · 세계 결과에 공정성 · 조작 · 장애는 결국 멎는다).

*식: PL1 · PL1b(활성 판 publish live) · 깨지면: 카메라를 켰는데 발행 중 표시로 영영 멈춘다(끊긴 동안 사라진 요청)*

<a id="PROP-PUB-WANT-ENDS"></a>**PROP-PUB-WANT-ENDS** · 성질

켜 둔 트랙은 결국 송출하거나 실패 · 회수를 듣는다 — 조용히 멈추지 않는다(PROP-PUB-NO-SILENT-DROP 의 활성판 · ★활성 성질).

*식: PL3(활성 판 publish live) · 깨지면: 앱은 켜 두었는데 아무 알림 없이 송출이 영영 시작되지 않는다*

<a id="DEV-PUB-STALL-FROM-RTP"></a>**DEV-PUB-STALL-FROM-RTP** · 자체 확장

★**발행 쪽 준비 신호는 없다 — 클라는 카메라가 첫 프레임을 냈다고 알리지 않는다.** 전이중 video 트랙의 정체 판정(`sync_required{no_media_flow}`)은 서버가 그 트랙의 업링크 스트림 도착을 스스로 관측해 시작한다 — 그래서 카메라 워밍업(300~600ms) 동안 패킷이 없어도 `sync_required` 가 오지 않고, 반이중 영상은 발언권 게이트가 닫힌 동안의 무패킷을 정체로 재지 않는다. 언제를 시작점으로 찍는지는 서버 정의서의 것이다. ★wire 에 두는 기준은 받는 쪽이 스스로 알 수 있는가다(R-TRK-MUTED-IN-SNAPSHOT) — 서버는 업링크 스트림의 도착을 스스로 관측하므로 신호를 두지 않는다

*원문: RULE-SUB-READY-WHAT — 옛 `type` 표의 `"camera"` 행 · 옛 첫 프레임 절차(동결 기준본 7-4-5 — 카메라가 첫 프레임을 내면 `READY{camera}` 를 보낸다) · 근거: R-TRK-MUTED-IN-SNAPSHOT CAND-PUB-NO-READY-CAMERA*

<a id="PUB.C.REGISTERED"></a>
#### 7-2-4. 클라 · `REGISTERED`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="PUB.C.REGISTERED.attach_ok_half"></a>`PUB.C.REGISTERED.attach_ok_half` | 받는 상태 ps == 'REGISTERED' · ok && half | 효과 app.publishing | §7-2-4 1 §7-2-2 — 반이중은 발행된 채 REGISTERED 에 머문다 — ★게이트는 닫힌 채다(소리는 허가 뒤에만 — 발언권을 얻으면 CFloorOpen 이 연다 · 여기서 track.attach 를 내 허가 전 송출이 됐다) |
| <a id="PUB.C.REGISTERED.attach_ok"></a>`PUB.C.REGISTERED.attach_ok` | 받는 상태 ps == 'REGISTERED' · ok | ps ← 'SENDING' · 효과 track.attach · 효과 app.publishing | §7-2-4 1 §7-2-4 2 |
| <a id="PUB.C.REGISTERED.attach_fail"></a>`PUB.C.REGISTERED.attach_fail` | 받는 상태 ps == 'REGISTERED' · 나머지 | ps ← 'IDLE' · 보낸다 PUBLISH_TRACKS.req {action: 'remove'} · 효과 app.publish_failed | §7-2-4 3 |
| <a id="PUB.C.REGISTERED.off"></a>`PUB.C.REGISTERED.off` | ps == 'REGISTERED' | ps ← 'IDLE'; want ← false · 보낸다 PUBLISH_TRACKS.req {action: 'remove'} | §7-2-5 §7-1-2 DEV-PUB-OFF-BEFORE-SENDING — 원문은 SENDING 에서 끈 것만 정한다 — 되돌리기 하나(§7-2-2 · §7-1-2 절차)로 되돌린다 |
| <a id="PUB.C.REGISTERED.revoked"></a>`PUB.C.REGISTERED.revoked` | 받는 효과 TRK pub.revoked · ps == 'REGISTERED' | ps ← 'IDLE' · 효과 track.detach · 효과 transceiver.inactive · 효과 app.revoked | §7-2-5 1 §7-2-5 2 §7-2-5 3 §7-2-5 4 R-PUB-REVOKED-WHY — 첫 프레임 전(REGISTERED)에 지워져도 SENDING 과 같다(칸 전수가 드러냄) |
| <a id="PUB.C.REGISTERED.floor_open"></a>`PUB.C.REGISTERED.floor_open` | 받는 효과 FLR mic.open · half && ps == 'REGISTERED' | ps ← 'SENDING' · 효과 track.attach | §7-2-2 — ★게이트를 여는 곳은 여기 하나다 — 반이중 소리는 허가 뒤에만 나간다 |

<a id="RULE-PUB-ATTACH"></a>**RULE-PUB-ATTACH** · 규칙 — RTP 를 시작했다 (`REGISTERED`)



항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`PUB.C.REGISTERED.attach_ok_half`](#PUB.C.REGISTERED.attach_ok_half) · [`PUB.C.REGISTERED.attach_ok`](#PUB.C.REGISTERED.attach_ok) |
| 2 | [`PUB.C.REGISTERED.attach_ok`](#PUB.C.REGISTERED.attach_ok) |
| 3 | [`PUB.C.REGISTERED.attach_fail`](#PUB.C.REGISTERED.attach_fail) |

<a id="PUB.C.SENDING"></a>
#### 7-2-5. 클라 · `SENDING`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="PUB.C.SENDING.off"></a>`PUB.C.SENDING.off` | ps == 'SENDING' | ps ← 'IDLE'; want ← false · 보낸다 PUBLISH_TRACKS.req {action: 'remove'} · 효과 track.detach | §7-2-5 1 §7-2-5 2 §7-2-5 3 |
| <a id="PUB.C.SENDING.revoked"></a>`PUB.C.SENDING.revoked` | 받는 효과 TRK pub.revoked · ps == 'SENDING' | ps ← 'IDLE' · 효과 track.detach · 효과 transceiver.inactive · 효과 app.revoked | §7-2-5 1 §7-2-5 2 §7-2-5 3 §7-2-5 4 R-PUB-REVOKED-WHY |
| <a id="PUB.C.SENDING.floor_close"></a>`PUB.C.SENDING.floor_close` | 받는 효과 FLR mic.close · half && ps == 'SENDING' | ps ← 'REGISTERED' · 효과 track.detach | §7-2-2 — 발행된 채 REGISTERED 에 머무는 것이 정상이다 · 게이트는 곧바로 닫는다(회수의 유예는 클라가 소리를 더 내는 창이 아니다) |

<a id="RULE-PUB-OFF"></a>**RULE-PUB-OFF** · 규칙 — 앱이 껐다 (`SENDING`)

2. 트랜시버는 두고 트랙만 뗀다(트랜시버를 없애면 협상이 또 돈다).

→ 구조 — [`PUB.C.REGISTERED.off`](#PUB.C.REGISTERED.off) · [`PUB.C.STAGED.off`](#PUB.C.STAGED.off) · [`PUB.C.STAGED.nego_ok_unwanted`](#PUB.C.STAGED.nego_ok_unwanted) · [`PUB.C.STAGED.pub_ok_unwanted`](#PUB.C.STAGED.pub_ok_unwanted)

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`PUB.C.SENDING.off`](#PUB.C.SENDING.off) |
| 2 | [`PUB.C.SENDING.off`](#PUB.C.SENDING.off) |
| 3 | [`PUB.C.SENDING.off`](#PUB.C.SENDING.off) |

<a id="RULE-PUB-REVOKED"></a>**RULE-PUB-REVOKED** · 규칙 — 서버가 내 발행을 지웠다 (`REGISTERED` · `SENDING`)

1. 반이중 트랙은 서버가 지우지 않는다(방에 묶이지 않는다, §7-1-2).
3. 다시 올리려면 비트가 올라와야 한다(`PARTICIPANT_STATE`).

→ 구조 — [`PUB.C.any.revoked_other`](#PUB.C.any.revoked_other) · [`PUB.S.perm_down_revoke`](#PUB.S.perm_down_revoke)

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`PUB.C.SENDING.revoked`](#PUB.C.SENDING.revoked) · [`PUB.C.REGISTERED.revoked`](#PUB.C.REGISTERED.revoked) |
| 2 | [`PUB.C.SENDING.revoked`](#PUB.C.SENDING.revoked) · [`PUB.C.REGISTERED.revoked`](#PUB.C.REGISTERED.revoked) |
| 3 | [`PUB.C.SENDING.revoked`](#PUB.C.SENDING.revoked) · [`PUB.C.REGISTERED.revoked`](#PUB.C.REGISTERED.revoked) |
| 4 | [`PUB.C.SENDING.revoked`](#PUB.C.SENDING.revoked) · [`PUB.C.REGISTERED.revoked`](#PUB.C.REGISTERED.revoked) |

<a id="PROP-PUB-SENDING-NEEDS-SERVER"></a>**PROP-PUB-SENDING-NEEDS-SERVER** · 성질

통로가 빈 SENDING 에서는 서버 발행이 있다.

*식: PH2 · 깨지면: 마이크 · 카메라가 켜진 채 서버가 모르는 SSRC 로 RTP 가 나가고 앱은 발행 중으로 안다(§7-2-5 증상)*

<a id="PROP-PUB-OFF-MEANS-IDLE"></a>**PROP-PUB-OFF-MEANS-IDLE** · 성질

앱이 껐으면 통로와 브라우저 쪽 일(협상 · 붙이기)이 끝난 뒤 IDLE 이다.

*식: PH3 · 깨지면: 앱은 껐는데 송출이 이어진다*

<a id="PROP-PUB-OFF-ENDS"></a>**PROP-PUB-OFF-ENDS** · 성질

끄면 결국 서버 발행이 없다(다시 켜지 않으면) — 끊긴 동안 사라진 remove 도 RESUME 대조가 다시 보낸다(RULE-PUB-REMOVE-FORM · ★활성 성질).

*식: PL2(활성 판 publish live) · 깨지면: 껐는데 서버에 유령 발행이 남아 상한을 먹는다*

<a id="DEV-PUB-OFF-BEFORE-SENDING"></a>**DEV-PUB-OFF-BEFORE-SENDING** · 자체 확장

앱이 `SENDING` 전에 끄면 되돌리기 하나로 되돌린다 — `REGISTERED` 에서 끄면 곧바로 `PUBLISH_TRACKS{remove}` → `IDLE` · `STAGED` 에서 끄면 의도만 내리고, 협상이 끝나면 `PUBLISH_TRACKS` 를 보내지 않고 `IDLE` · 발행 응답이 이미 오면 `remove` → `IDLE`

*원문: §7-2-5 — SENDING 에서 끈 것만 정한다 · 근거: RULE-PUB-ONE-ROLLBACK RULE-PUB-PUBLISH-PROC PROP-PUB-OFF-MEANS-IDLE*

<a id="DEV-PUB-REMOVE-FAIL-3005"></a>**DEV-PUB-REMOVE-FAIL-3005** · 자체 확장

`PUBLISH_TRACKS` 의 실패에 `3005 TRACK_NOT_FOUND` 가 있다 — 이미 지워진 발행의 `remove`(§7-2-5). §7-1-2 실패 줄에는 빠져 있다

*원문: §7-1-2 실패 줄 · 근거: RULE-PUB-REVOKED*

<a id="PUB.S"></a>
#### 7-2-6. 서버

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="PUB.S.remove"></a>`PUB.S.remove` | m.t == 'PUBLISH_TRACKS' && m.action == 'remove' && reg | reg ← false · 답한다 PUBLISH_TRACKS.res {action: 'remove'} | §7-1-2 |
| <a id="PUB.S.remove_3005"></a>`PUB.S.remove_3005` | m.t == 'PUBLISH_TRACKS' && m.action == 'remove' | 답한다 PUBLISH_TRACKS.fail {code: 3005} | §7-1-2 |
| <a id="PUB.S.add_2006"></a>`PUB.S.add_2006` | m.t == 'PUBLISH_TRACKS' && !perm | 답한다 PUBLISH_TRACKS.fail {code: 2006} | §7-1-2 |
| <a id="PUB.S.add_3011"></a>`PUB.S.add_3011` | m.t == 'PUBLISH_TRACKS' && !atPub && m.duplex == 'full' | 답한다 PUBLISH_TRACKS.fail {code: 3011} | §5-1-2 |
| <a id="PUB.S.add_4002"></a>`PUB.S.add_4002` | m.t == 'PUBLISH_TRACKS' && full | 답한다 PUBLISH_TRACKS.fail {code: 4002} | §7-1-2 |
| <a id="PUB.S.add"></a>`PUB.S.add` | m.t == 'PUBLISH_TRACKS' | reg ← true · 답한다 PUBLISH_TRACKS.res {action: 'add'} | §7-1-2 |
| <a id="PUB.S.rx_other"></a>`PUB.S.rx_other` | 나머지 | — | §7-1-2 |
| <a id="PUB.S.perm_down_revoke"></a>`PUB.S.perm_down_revoke` | 받는 효과 ROOM pub.perm_down · ROOM pub.perm_up · perm && reg | perm ← false; reg ← false · 답한다 TRACK_EVENT.body {action: 'remove'} | §4-2-6 §7-2-5 |
| <a id="PUB.S.perm_down"></a>`PUB.S.perm_down` | 받는 효과 ROOM pub.perm_down · ROOM pub.perm_up · perm | perm ← false | §4-2-6 |
| <a id="PUB.S.perm_up"></a>`PUB.S.perm_up` | 받는 효과 ROOM pub.perm_down · ROOM pub.perm_up · 나머지 | perm ← true | §4-2-6 |
| <a id="PUB.S.full_flip"></a>`PUB.S.full_flip` | 나머지 | full ← !full | §7-1-2 |
| <a id="PUB.S.room_gone"></a>`PUB.S.room_gone` | 받는 효과 ROOM pub.room_gone · reg && !half | reg ← false | §4-2-6 |
| <a id="PUB.S.room_gone_other"></a>`PUB.S.room_gone_other` | 받는 효과 ROOM pub.room_gone · 나머지 | — | §4-2-6 |
| <a id="PUB.S.at_pub"></a>`PUB.S.at_pub` | 받는 효과 AFF pub.at_pub · 나머지 | atPub ← true | §5-1-2 |
| <a id="PUB.S.off_pub"></a>`PUB.S.off_pub` | 받는 효과 AFF pub.off_pub · 나머지 | atPub ← false | §5-1-2 |
| <a id="PUB.S.set_3005"></a>`PUB.S.set_3005` | !reg | 답한다 TRACK_SET.fail {code: 3005} | §7-1-2 |
| <a id="PUB.S.set_3011"></a>`PUB.S.set_3011` | !atPub && m.duplex == 'full' | 답한다 TRACK_SET.fail {code: 3011} | §5-1-2 |
| <a id="PUB.S.set_noop"></a>`PUB.S.set_noop` | (m.duplex == 'half') == half | 답한다 TRACK_SET.res {noop: true} | §7-1-2 |
| <a id="PUB.S.set_duplex"></a>`PUB.S.set_duplex` | 나머지 | half ← m.duplex == 'half'; muted ← false · 답한다 TRACK_SET.res {duplex: m.duplex} | §7-1-2 |
| <a id="PUB.S.peer_gone"></a>`PUB.S.peer_gone` | 받는 효과 ROOM pub.peer_gone · LINK pub.peer_gone · reg | reg ← false | §4-1 |
| <a id="PUB.S.peer_gone_other"></a>`PUB.S.peer_gone_other` | 받는 효과 ROOM pub.peer_gone · LINK pub.peer_gone · 나머지 | — | §4-1 |
| <a id="PUB.S.mute_3005"></a>`PUB.S.mute_3005` | !reg | 답한다 TRACK_SET.fail {code: 3005} | §7-1-2 |
| <a id="PUB.S.mute_3006"></a>`PUB.S.mute_3006` | half | 답한다 TRACK_SET.fail {code: 3006} | §7-1-2 |
| <a id="PUB.S.mute_noop"></a>`PUB.S.mute_noop` | m.muted == muted | 답한다 TRACK_SET.res {noop: true} | §7-1-2 |
| <a id="PUB.S.mute"></a>`PUB.S.mute` | 나머지 | muted ← m.muted · 답한다 TRACK_SET.res {muted: m.muted} | §7-1-2 |

### 7-3. 흐름 · 검증
검증 모형 `out/publish/Publish.tla` · 하네스 `verify/publish/harness.tla`

#### 7-3-1. 흐름 F1 — 켜서 보내기
발행의 줄기다 — 트랜시버 · 협상 · 발행 · RTP 는 한 덩어리이고 어디서 실패하든 되돌리기는 하나다.
모형 검사: 걸음 5 도달 · 갈래 11

1. **on** — 앱이 카메라를 켠다. 트랜시버를 만들고 발행 SDP 를 다시 짓는다 → STAGED. ★RTP 는 아직 보내지 않는다.
2. **nego** — 협상이 성공하면 PUBLISH_TRACKS{action:"add"} 를 보낸다.
3. **up PUBLISH_TRACKS** — 서버가 받아들이고 track_id 를 준다.
4. **down PUBLISH_TRACKS** — 응답의 track_id 를 자기 mid 에 대응시켜 보관한다 → REGISTERED.
5. **attach** — 실제 트랙을 트랜시버에 붙인다 → SENDING. ★붙이기가 실패하면 remove 로 되돌린다 — 안 그러면 유령 발행이 상한을 먹는다. 첫 RTP 는 서버가 ingress 에서 스스로 안다(신호 없음).

#### 7-3-2. 흐름 F2 — 서버가 내 발행을 지웠다
권한 비트가 내려가면 서버가 전이중 트랙을 지운다 — 클라는 remove 를 보내지 않고 내린다.
모형 검사: 걸음 7 도달 · 갈래 14

1. **on** — 앱이 카메라를 켠다. 트랜시버를 만들고 발행 SDP 를 다시 짓는다 → STAGED. ★RTP 는 아직 보내지 않는다.
2. **nego** — 협상이 성공하면 PUBLISH_TRACKS{action:"add"} 를 보낸다.
3. **up PUBLISH_TRACKS** — 서버가 받아들이고 track_id 를 준다.
4. **down PUBLISH_TRACKS** — 응답의 track_id 를 자기 mid 에 대응시켜 보관한다 → REGISTERED.
5. **attach** — 실제 트랙을 트랜시버에 붙인다 → SENDING. ★붙이기가 실패하면 remove 로 되돌린다 — 안 그러면 유령 발행이 상한을 먹는다. 첫 RTP 는 서버가 ingress 에서 스스로 안다(신호 없음).
6. **perm** — 결정자가 그 종류의 권한 비트를 내린다 — 서버가 발행을 지우고 내 track_id 의 TRACK_EVENT{remove} 를 보낸다.
7. **down TRACK_EVENT** — 트랙을 떼고 트랜시버는 inactive 로 둔다 → IDLE. ★PUBLISH_TRACKS{remove} 를 보내지 않는다 — 발행은 이미 없다(보내면 3005).

### 7-4. 주제

#### 7-4-1. `RESUME` 성공 응답을 받았다

<a id="PROP-PUB-NO-SILENT-DROP"></a>**PROP-PUB-NO-SILENT-DROP** · 성질

앱이 켜 둔 트랙이 통로와 브라우저 쪽 일이 끝난 뒤 IDLE 이면 앱이 그 사실을 들었다(발행 실패 · 회수).

*식: PH4 · 깨지면: 앱은 켜 두었는데 아무 알림 없이 발행이 멎는다 — 끊긴 동안 사라진 발행 요청을 RESUME 뒤 다시 묻지 않으면 이렇게 된다(CAND-PUB-RESUME-INFLIGHT)*

<a id="DEV-PUB-RESUME-INFLIGHT"></a>**DEV-PUB-RESUME-INFLIGHT** · 자체 확장

WS 가 끊긴 동안 통로 안에서 사라진 `PUBLISH_TRACKS` 요청 · 응답도 RESUME 응답의 `publications[]` 로 잡는다. 요청을 기다리던(`STAGED` · 협상 끝) 트랙은 응답이 온 것처럼 — 실려 있으면 발행된 것(앱이 껐으면 `remove` → `IDLE`), 없으면 앱이 켜 둔 채일 때 ★**새 요청으로 다시 묻는다**(서버에 없음을 확인했으니 중복 발행이 아니다 — RULE-SES-RES-BACKOFF-END 2 의 재전송과 다르다) · 앱이 껐으면 `IDLE`. 내가 갖지 않은 발행이 서버에 남았으면(끊긴 동안 `remove` 가 사라졌다) `remove` 를 다시 보낸다

*원문: §3-2-3 2 ③ — 대조는 살아 있는 발행(`REGISTERED` 이상)만 정한다 · 근거: RULE-SES-RES-RESUME-OK RULE-PUB-ONE-ROLLBACK PROP-PUB-NO-GHOST PROP-PUB-OFF-MEANS-IDLE PROP-PUB-NO-SILENT-DROP CAND-PUB-RESUME-INFLIGHT*

### 7-5. 근거 · 기각 — 규격이 아니다

> 계약 본문(구조 · 동작 · 주제)의 규칙이 왜 그런가(R) · 무엇을 하지 않기로 했나(REJ). 소절 머리는 그 근거가 붙은 계약 자리다. 계약 판에는 싣지 않는다.

#### 7-5-1. §7-1-2 형 `PublishTrack` — `tracks[]` 원소

<a id="R-PUB-ONE-CODEC-PER-ROOM"></a>**R-PUB-ONE-CODEC-PER-ROOM** · 근거

★**왜 하나로 묶나** — 구독자 m-line 은 모두 돌려쓴다. 무전은 화자들이 ★**슬롯 m-line 하나를 돌려쓰고**, 전이중은 줄이 상한에 닿으면
서버가 다른 트랙을 그 줄에 싣는다(재배정, §8-1-2). 바뀔 때마다 재협상하지 않으려면 그 m-line 의 코덱·PT 가 고정이어야 한다.
즉 *"무전은 VP8"* 이 아니라 ★*"한 방의 발행은 서로 같은 코덱"* 이 진짜 제약이다.

<a id="R-PUB-PUBLISH-SYMPTOMS"></a>**R-PUB-PUBLISH-SYMPTOMS** · 근거

★**증상**
- `codec` 을 빼면 → 서버는 VP8 로 알고 클라는 H264 로 보낸다 → ★**패킷은 오는데 화면이 검다.**
- ③ 실패를 안 되돌리면 → **유령 발행이 상한을 소진해** 나중에 `4002` 가 잦아진다.
- 시뮬캐스트인데 `ssrc` 를 억지로 채우면 → 서버가 실제 도착 SSRC 와 못 맞춘다.

#### 7-5-2. §7-1-4 `0x0304 TRACK_SET` — 트랙 상태 바꾸기   **C→S** · 응답 있음

<a id="R-PUB-SET-SYMPTOMS"></a>**R-PUB-SET-SYMPTOMS** · 근거

★**증상**
- `ssrc` 만 보내면 → 다시 발행한 뒤 **엉뚱한 트랙**이 바뀐다.
- `0x0301` 로 우회 전환하면 → **쓰이지 않는 발행 트랜시버가 남는다.**
- `3006` 을 다른 실패와 못 가르면 → ★**영원히 재시도한다.** 시뮬캐스트를 끄기 전엔 성공할 수 없다.
  ★**계약**: `3006` 을 받으면 **바뀌지 않은 것으로 확정**하고, 시뮬캐스트를 끄기 전에는 다시 시도하지 않는다.
- ★`pub_room` 이 아닌 방에 전이중을 발행하면 → `3011` 이다(§5-1-2). 받아 주면 그 트랙 RTP 가 갈 방이 없다 — 구독자는 트랙이 있는데 RTP 가 없어 ★**검은 화면이고 어디에도 신호가 없다.**

#### 7-5-3. §7-2-3 클라 · `any`

<a id="R-PUB-STALL-SYMPTOMS"></a>**R-PUB-STALL-SYMPTOMS** · 근거

★**증상**
- 첫 RTP 전부터 정체를 재면 → 카메라 워밍업(300~600ms) 동안 패킷이 없는 것이 정상인데 ★**헛 `sync_required`** 가 온다.
- 시작점을 찍지 않으면 → 그 트랙이 ★**실제로 멎어도 `sync_required` 가 안 온다** — 구독자 화면은 검은데 계수기만 조용하다.
- ★반이중 영상의 무패킷(발언권 게이트가 닫힌 동안)을 정체로 재면 → 말하지 않는 동안 헛 `sync_required` 가 온다 — 정체 자격은 발언권에 걸린다.

#### 7-5-4. §7-2-4 클라 · `REGISTERED`

<a id="R-PUB-ATTACH-SYMPTOM"></a>**R-PUB-ATTACH-SYMPTOM** · 근거

★**증상** — 3의 되돌리기를 빠뜨리면 **유령 발행이 상한을 소진해** 나중에 `4002` 가 잦아진다.

#### 7-5-5. §7-2-5 클라 · `SENDING`

<a id="R-PUB-REVOKED-SYMPTOM"></a>**R-PUB-REVOKED-SYMPTOM** · 근거

★**증상** — 이 전이가 없으면 → 마이크·카메라가 켜진 채 죽은 발행으로 RTP 가 계속 나가고(서버는 모르는 SSRC 로 버린다) 앱은 발행 중으로 안다. 회수했다고 답한 뒤에도 표시등이 켜져 있다.

<a id="R-PUB-REVOKED-WHY"></a>**R-PUB-REVOKED-WHY** · 근거

- 발행은 이미 없다(보내면 `3005`).

<a id="SUB"></a>
