# 06. 트랙 속성 — 음소거 · 영상 멈춤 · 트랙 단위 duplex(회의↔무전)

> author: kodeholic (powered by Claude) · created: 2026-11-09.
> ★**대조한 커밋** — 서버 `oxlens-sfu` `c3642e8` · SDK `oxlens-sdk-web` `9f40557`(+탐침) · 2층 `oxlens-labs` `fc584ff`
> ★**실측 회차** — 3층 탐침 `tour-learn-20261109` 의 `TOUR-06` · 2층 run-all `e2e-20261109-070257`(시나리오 `conf_duplex` · `conf_simulcast` · `adv_wire_reject`). 값은 돌릴 때마다 바뀌고 차례 · 모양은 같다.

3층 시각은 로컬 시각(초.밀리초), 2층 시각은 봇이 시작한 뒤 경과 초다. 3층 브라우저는 가짜 장치로 영상을 낸다.

## 1. 30초 요약

```mermaid
flowchart LR
  subgraph PUBSIDE["발행자의 속성 — 방 전원에게 알린다 · 방 seq 가 오른다"]
    M["setMuted(true)<br/>TRACK_SET muted"] --> TS["TRACK_STATE type muted<br/>줄은 그대로"]
    D["setDuplex('half')<br/>TRACK_SET duplex"] --> TE["TRACK_EVENT add · active false<br/>같은 track_id 항목 교체 · ★줄을 놓는다"]
  end
  subgraph SUBSIDE["구독자의 속성 — 나만의 일"]
    P["setLayer({paused:true})<br/>SUBSCRIBE_LAYER paused"] --> ROW["내 배정표에서 그 줄만 빈다<br/>발행자 · 다른 구독자는 모른다"]
  end
  subgraph SDKSIDE["SDK 안 — wire 에 없다"]
    MIC["무전 마이크 전원 hot · hot_standby · cold"]
  end
```

- **누구의 속성인가가 길을 정한다.** 음소거 · 회의↔무전은 발행자의 사실이라 방 전원에게 통지가 가고 방 `seq` 가 오른다. 영상 멈춤은 구독자 자신의 선택이라 내 배정표에만 일어난다.
- ★**회의↔무전은 같은 트랙 하나의 속성이다** — 트랙을 내리고 새로 올리지 않는다. `TRACK_EVENT{add}` 한 번(같은 `track_id` 의 `active` 교체)으로 끝나고 SDP 재협상이 없다.
- 무전으로 쉬는 트랙은 구독자 줄을 놓는다 — 돌아오면 줄이 다시 실리고 서버가 키프레임을 청한다.

## 2. 등장인물 · 값

| 이름 | 이 회차 값 | 무엇 |
|---|---|---|
| A | `tourattr_A` · 방 `qa_tourattr_p1` · 1pc · 마이크 `t0bb302b0c1af6965`(SSRC 2508349306) · 카메라 `t953589e887e8f9ca`(SSRC 2392994742 · H264 · 시뮬캐스트 아님) | 속성을 바꾸는 발행자 |
| B | `tourattr_B` · 같은 방 · adaptiveStream 끔 | 받는 쪽 — 줄 32 · 33(무전 슬롯) · 34(A 마이크 · SSRC 2329595648) · 35(A 카메라 · SSRC 3435589755) |
| P · L | `tourattr_P` · `tourattr_L` · 무전 방 `qa_tourattr_p3` | 마이크 전원 — [09](09_MIC_COST.md) |
| 2층 botA · botB | `conf_duplex` — botA 영상 SSRC 2952790017(0xB0000001) · track `t923d51655b2ef27b` · botB 의 줄 mid 3 · SSRC 3111925893 | RTP 가 실제로 멎고 다시 흐르는지 |
| 네 축 | `TRACK_SET{muted}` · `SUBSCRIBE_LAYER{paused}` · `TRACK_SET{duplex}` · SDK 안의 마이크 전원 | 앞 셋은 wire([연:PUB.op.TRACK_SET](../protocol/07-pub.md#PUB.op.TRACK_SET) · [연:SUB.op.SUBSCRIBE_LAYER](../protocol/08-sub.md#SUB.op.SUBSCRIBE_LAYER)) · 넷째는 SDK 안 |

## 3. 정상 길

### 3-1. 음소거 — `TRACK_SET{muted}` → `TRACK_STATE`

```mermaid
sequenceDiagram
  participant A as A SDK
  participant H as oxhubd
  participant S as oxsfud
  participant B as B SDK
  Note over A: 50.825 setMuted(true) — 브라우저 track.enabled=false
  A->>H: 50.829 TRACK_SET track_id t0bb… · muted true
  H->>S: gRPC
  Note over S: 50.831 CP-OP-TRACK-STATE room seq=5
  S-->>A: 50.834 TRACK_STATE type muted · muted true · seq 5 — 본인에게도
  S-->>B: 50.834 TRACK_STATE 같은 것
  H-->>A: 50.835 TRACK_SET ok · muted true · ssrc 2508349306
  Note over B: 50.833 TRK.C.any.track_state → 50.839 단계 B.sawMuted
  Note over A: 51.848 setMuted(false) — 같은 왕복 · 51.851 seq 6
```

| 무엇 | 값 |
|---|---|
| 요청 → 통지 → 응답 | 50.829 → 50.834 → 50.835(해제는 51.848 → 51.851 → 51.851) |
| 통지 대상 | ★방 전원 — 본인 A 도 받는다(SDK 전이 `TRK.C.any.track_state` 가 A · B 같은 ms) |
| 방 `seq` | 4 → 5(음소거) → 6(해제) |
| 배정표 · RTP | 그대로 — 줄 34 는 남는다. 서버는 음소거 트랙의 RTP 를 거르지 않는다(브라우저가 `enabled=false` 로 무음 프레임을 보낸다) |

- 통지가 응답보다 먼저 나간다 — 서버는 방 전원에게 알린 뒤 요청자에게 답한다. `TRACK_STATE` 의 `type` 은 `muted` 하나다 — 「받는 쪽이 스스로 알 수 없는 것」만 통지다([연:TRK.op.TRACK_STATE](../protocol/06-trk.md#TRK.op.TRACK_STATE)).
- ★음소거는 **전이중 트랙에만** 있다 — 반이중은 발언권 게이트가 송출을 정하므로 음소거가 없다([연:PROP-TRK-MUTED-FULL-ONLY](../protocol/06-trk.md#PROP-TRK-MUTED-FULL-ONLY) · §4-2).

### 3-2. 영상 멈춤 — `SUBSCRIBE_LAYER{paused}` · 내 배정표만

```
52.853 단계 B.pauseVideo
52.858 B→ SUBSCRIBE_LAYER {targets:[{track_id t953589e887e8f9ca, paused:true}]}
52.861 SRV CP-OP-MLINE-EVENT user=tourattr_B arev=4
52.865 B← MLINE_EVENT arev 4 — 줄 35: track_id 없음 · SSRC 3435589755 는 그대로     ← 빈 줄
52.867 B← SUBSCRIBE_LAYER ok   ·   B→ READY arev 4(m-line 수가 그대로라 SDP 재협상 없음 · SDK SUB.C.any.table_rebound)
55.371 B→ SUBSCRIBE_LAYER {paused:false}
55.377 B← MLINE_EVENT arev 5 — 줄 35 에 t953589e887e8f9ca 가 다시 실린다 · READY arev 5
55.380 SRV SV-STATE-SUBSCRIBE-STATE user=tourattr_B key=t953589e887e8f9ca from=Created to=Active cause=ready
```

- **멈춤은 줄을 비운다.** 멈춘 트랙은 배정표 후보에서 빠지고(`sub/mod.rs` `set_pref`) 그 줄은 빈 줄이 되어 같은 kind 의 다른 트랙이 쓸 수 있다([연:RULE-SUB-FILL](../protocol/08-sub.md#RULE-SUB-FILL)). 빈 줄 · 재배정의 집은 [03](03_PUBLISH_SUBSCRIBE.md).
- **A 는 모른다.** 52.8 ~ 57.8 사이 A 의 WS 에는 프레임이 하나도 없었고 A 의 송신은 그대로다. 방 `seq` 도 6 그대로.
- 와이어의 멈춤은 `paused` 라는 따로 된 칸이다 — 층 상한(`spatial`)과 섞지 않는다([연:RULE-SUB-LAYER-PAUSED-AXIS](../protocol/08-sub.md#RULE-SUB-LAYER-PAUSED-AXIS)). 2층 봇은 이 칸을 아직 못 보낸다([05 §4-5](05_SIMULCAST_BWE.md)).

### 3-3. 회의 → 무전 → 회의 — `TRACK_SET{duplex}` · 항목 교체 하나

```mermaid
sequenceDiagram
  participant A as A SDK PUB 상태기
  participant S as oxsfud
  participant B as B SDK
  Note over A: 57.885 setDuplex('half') — PUB.C.any.set_duplex · 응답 전엔 아무것도 안 바꾼다
  A->>S: 57.886 TRACK_SET track_id t9535… · duplex half
  Note over S: 57.888 CP-OP-TRACK-EVENT seq=7 · B 에게 CP-OP-MLINE-EVENT arev=6
  S-->>A: 57.892 TRACK_EVENT add active false seq 7 — 본인에게도 TRK.C.any.mine_other
  S-->>B: 57.892 TRACK_EVENT add active false seq 7 — TRK.C.any.track_event
  S-->>B: 57.893 MLINE_EVENT arev 6 — 줄 35 빈 줄 · B READY arev 6
  S-->>A: 57.893 TRACK_SET ok duplex half · ssrc 2392994742 — PUB SENDING → REGISTERED set_half
  Note over A: 59.402 setDuplex('full')
  A->>S: 59.403 TRACK_SET duplex full
  Note over S: 59.405 CP-OP-TRACK-EVENT seq=8 · CP-OP-MLINE-EVENT arev=7
  S-->>A: 59.410 TRACK_EVENT add active true seq 8 · PUB REGISTERED → SENDING set_full
  S-->>B: 59.413 TRACK_EVENT add active true · 59.414 MLINE_EVENT arev 7 — 줄 35 다시
  Note over S: 59.414 줄 35 Created → Active cause=ready
```

| 무엇 | half 로 | full 로 |
|---|---|---|
| 통지 | `TRACK_EVENT{add, active:false}` 하나(seq 7) | `TRACK_EVENT{add, active:true}` 하나(seq 8) |
| B 의 배정표 | arev 6 — 줄 35 빈 줄 | arev 7 — 줄 35 에 다시(같은 mid · 같은 SSRC) |
| 요청 → 서버 통지 → 응답 | 57.886 → .888 → .893 | 59.403 → .405 → .414 |
| A 의 발행 상태기 | `SENDING → REGISTERED`(`PUB.C.SENDING.set_half`) — 반이중은 발언권이 열 때만 보낸다 | `REGISTERED → SENDING`(`PUB.C.REGISTERED.set_full`) |
| B 가 보는 것 | `active=false` — 화면을 내린다 · m-line 은 그대로 | `active=true` · 키프레임 뒤 영상 |

실제 프레임(B 가 받은 half 통지):
```
TRACK_EVENT {"action":"add","room_id":"qa_tourattr_p1","tracks":[{"active":false,"codec":"H264","csrc":202,…,
             "kind":"video","muted":false,…,"source":"camera","track_id":"t953589e887e8f9ca","type":"individual","user_id":"tourattr_A"}],
             "version":{"epoch":"8701a55ebcdd350f","seq":7}}
```

- **전환은 프레임 하나다** — `remove` + `add` 둘이 아니라 같은 `track_id` 항목의 `active` 만 바꿔 교체한다([연:RULE-ROOM-DUPLEX-ONE-ADD](../protocol/04-room.md#RULE-ROOM-DUPLEX-ONE-ADD) · [연:RULE-TRK-FIVE-RULES](../protocol/06-trk.md#RULE-TRK-FIVE-RULES)). 트랙 `type` 은 `individual` 그대로다.
- **쉬는 트랙은 줄이 없다** — half 가 되는 순간 서버가 B 의 줄 35 를 놓는다. 되돌리면 이 회차처럼 같은 줄에, 또는 다른 줄에 다시 실린다([연:RULE-PUB-SET-PROC](../protocol/07-pub.md#RULE-PUB-SET-PROC)).
- SDK 는 통지가 아니라 **응답**(`set_half` · `set_full`)이 와야 게이트 · 전원을 바꾼다.
- `duplex` 가 적용되면 `muted` 는 `false` 로 돌아간다(`ops/media.rs` `set_duplex_of` 의 `st.muted.store(false)`) — 음소거된 카메라를 무전으로 바꾼 뒤 허가를 받아 검은 화면이 나가는 결말을 막는다.

**RTP 가 실제로 멎고 다시 흐르는 것**(2층 `conf_duplex`):

| 1초 칸 | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|---|
| botA 가 보낸 영상 | 21 | 29 | 29 | 29 | 28 | 29 | 28 | 29 |
| botB 가 받은 영상 | — | 20 | 8 | 0 | 0 | 21 | 28 | 29 |

```
2.275 botA → TRACK_SET {ssrc 2952790017, duplex: half}
2.281 botB ← TRACK_EVENT active:false seq 6 · MLINE_EVENT arev 4(mid 3 빈 줄) · → READY arev 4
5.275 botA → TRACK_SET {duplex: full}
5.280 botB ← MLINE_EVENT arev 5(mid 3 다시) · → READY · 5.283 TRACK_EVENT active:true seq 7
5.290 botB ← 첫 영상
```

botA 는 그동안 보내기를 멈추지 않았다 — 서버가 botB 의 줄로 안 보낸 것이다.

### 3-4. 마이크 전원

무전 마이크의 세 상태(hot · hot_standby · cold — 무발화 30초 뒤 장치 반납)는 서버가 모르는 SDK 안의 일이다. 이 회차도 P 의 `mic` 가 07:38:02.899 hot_standby → 07:38:32.853 cold 로 바뀌는 동안 P 의 WS 에는 10초 `HEARTBEAT` 셋뿐 · 발행 · 배정표 프레임은 없었다. 비용 · 실측은 [09](09_MIC_COST.md) 가 집이다.

## 4. 깨지는 길

### 4-1. muted 와 duplex 를 한 요청에 — `1007 FIELD_CONFLICT`(`adv_wire_reject`)

```
4.692 botA → TRACK_SET {room_id, track_id "tr-아무거나", muted: true, duplex: "half"}
      botA ← 실패 1007 FIELD_CONFLICT · permanent true
```

- 둘은 배타다 — 둘 다 없거나 둘 다 있으면 거절([연:RULE-PUB-SET-EXCLUSIVE](../protocol/07-pub.md#RULE-PUB-SET-EXCLUSIVE)). ★없는 `track_id` 인데 `3005 TRACK_NOT_FOUND` 가 아니라 `1007` 이 왔다 — 서버가 방 확인 다음 **배타 검사를 트랙 찾기보다 먼저** 한다(`ops/media.rs` `track_set`). 꼴이 틀린 요청은 대상을 찾기 전에 막힌다.
- **사용자는** 아무것도 바뀌지 않은 채 실패를 받는다. `permanent` 라 같은 요청을 다시 보내도 같다 — 앱이 둘을 따로 보내야 한다. 실패 번호 체계는 [12](12_ERRORS.md).

### 4-2. 그 트랙엔 안 되는 조작 — `3006 TRACK_OP_UNSUPPORTED`(`conf_simulcast`)

```
10.284 botA → TRACK_SET {track_id <시뮬캐스트 영상>, duplex: "half"}
       botA ← 실패 3006 TRACK_OP_UNSUPPORTED · permanent true
```

- 시뮬캐스트 트랙은 무전으로 바꿀 수 없다(`set_duplex_of` — `simulcast duplex`). ★같은 번호가 **반이중 트랙에 `muted` 를 보낼 때**도 나온다(`track_set` — `muted on half`) — 그 갈래는 이 회차로 일으키지 않았다(소스 경로로 적은 것).
- ★계약 — `3006` 을 받으면 「바뀌지 않았다」로 확정하고 시뮬캐스트를 끄기 전에는 다시 시도하지 않는다. 다른 실패와 못 가르면 영원히 재시도한다([연:R-PUB-SET-SYMPTOMS](../protocol/07-pub.md#R-PUB-SET-SYMPTOMS)).
- 탐침은 시뮬캐스트가 이미 흐르는 판의 끝(10.284)에 붙여, 거절이 미디어에 아무 영향도 없음을 같은 회차의 정합 판정(초록)으로 함께 보였다.

### 4-3. 다시 실린 줄의 첫 패킷이 키프레임이 아니었다(관찰 · 결함 단정 아님)

`conf_duplex` 에서 full 로 돌아온 뒤 botB 가 받은 영상을 seq 로 이어 보면:

| 시각 | 나간 seq | VP8 머리 바이트(맨 아래 비트 P — 1 = delta) | 뜻 |
|---|---|---|---|
| 2.259 | 57 | `11` | half 직전 마지막 — delta |
| 5.290 | ★144 | `11` | full 뒤 첫 패킷 — ★delta · seq 58 ~ 143(86개)이 비었다 |
| 5.501 | 150 | `10` | 첫 키프레임 |

```mermaid
sequenceDiagram
  participant A as botA 발행 — 쉬지 않고 보냄
  participant S as oxsfud
  participant B as botB 구독
  Note over S: 2.28 ~ 5.28 botB 의 줄 3 이 비었다 — 재기록기는 손대지 않음
  A->>S: 5.275 TRACK_SET duplex full
  S->>A: 5.279 PLI — 전환 PLI 묶음 첫째
  S-->>B: 5.280 MLINE_EVENT arev 5 — 같은 줄 3 에 같은 출처
  A->>S: seq 144 delta
  S->>B: 5.290 나간 seq 144 — 출처가 같아 오프셋 0 그대로 · 키프레임 기다림 없음
  A->>S: seq 150 키프레임 — 봇의 30프레임 주기
  S->>B: 5.501 seq 150
  S->>A: 6.285 · 7.290 PLI — 묶음의 +500 · +1500ms 몫이 스로틀 대기로
```

- **무엇이 일어났나** — 재기록기는 「출처가 바뀔 때」만 오프셋을 다시 세고 키프레임을 기다린다([04 §3-3](04_PACKET_PATH.md)). 쉬었다 돌아온 줄은 출처(같은 트랙)가 그대로라 둘 다 하지 않았다 — 그래서 seq 구멍 86개를 남긴 채 delta 가 먼저 나갔다. 3층 TOUR-06 끝의 B 줄 35 재기록 상태도 멈춤 · 해제와 half · full 을 다 지난 뒤 `switches 0 · seq_off 0` 이었다(같은 모양).
- **누가 먼저 아나** — 구독자 디코더다(참조 프레임 없는 delta). 서버는 full 로 돌아올 때 PLI 묶음(0 · 500 · 1,500ms — `rtcp::pli_burst`)을 내지만 그 키프레임이 오기 전 delta 를 막지는 않는다. 구독자 쪽에서는 86개 구멍이 망 손실과 구별되지 않는다.
- **사용자는** 2층에서는 211ms(5.290 → 5.501) 동안 쓸 수 없는 영상을 받았다. 3층 B 의 영상 수신 통계는 끝에 `freezeCount 2 · keyFramesDecoded 8 · nackCount 1 · pliCount 0` — 멈춤 둘이 줄 다시 싣기 둘(§3-2 · §3-3)과 수는 같지만 인과는 이 회차로 가르지 않았다. 브라우저 발행자는 PLI 에 곧바로 키프레임을 내 창이 짧다.
- **규격과 견주기** — — 는 「출처 전환 뒤 첫 패킷은 키프레임」이고 같은 출처의 다시 싣기는 말하지 않는다. — 가 「구멍을 남기는 예외는 게이트 드롭 하나」라고 하는데 이 구멍은 그 밖이다. 옛 학습 문서가 「되돌리면 키프레임부터」라고 적은 것과 실측이 어긋난다 — 기록으로 남기고 판단은 규격 쪽 몫이다.

## 5. 왜 이 모양인가

- **누구의 속성인가로 길을 나눴다.** 발행자의 사실(음소거 · 회의↔무전)은 다른 사람 화면이 알아야 하므로 방 통지 + `seq++`, 구독자의 선택(멈춤)은 발행자도 다른 구독자도 알 필요가 없으므로 내 배정표 하나, 장치 전원은 서버가 알 것이 없으므로 SDK 안. 그래서 음소거는 「줄은 두고 표시만」, 멈춤 · 무전은 「줄을 비운다」로 결말이 갈린다.
- **음소거는 전이중에만.** 반이중의 송출은 발언권 게이트 하나가 정한다 — 둘째 스위치를 두면 「허가를 받았는데 무음」이라는 상태가 생긴다([연:PROP-TRK-MUTED-FULL-ONLY](../protocol/06-trk.md#PROP-TRK-MUTED-FULL-ONLY)). 같은 까닭으로 duplex 를 바꾸면 `muted` 를 지운다.

> ★**우리만의 것 — 트랙 단위 duplex: 회의와 무전을 같은 트랙 하나로**
> - **선택 배경** — 옛 판에는 방이 「회의 방」 · 「무전 방」 모드를 들었다. 방 모드를 걷고 반이중 · 전이중을 트랙의 종류로 내리자, 같은 방 안에서 한 트랙을 무전 ↔ 회의로 바꾸는 일이 「트랙 속성 하나 바꾸기」가 되었다 — 새 전달 로직 없이 그 트랙이 타는 길(슬롯 · 개별 줄)만 바뀐다. 기존 무전 시스템은 1:1 승격을 별도 세션으로 열거나(3GPP MCPTT Private Call) 채널을 떠나야 하고(TETRA · P25) 몇몇은 지원하지 않는다는 것이 우리 기록의 조사였다(실측 아님). 처음에는 전환을 `TRACK_STATE{duplex}` 로 알렸으나 같은 사실을 번호를 안 올린 채 한 번 더 보내는 프레임이라 받는 쪽 규칙에 늘 버려졌고, 지금은 `TRACK_EVENT{add}` 하나로 알린다([연:RULE-ROOM-DUPLEX-ONE-ADD](../protocol/04-room.md#RULE-ROOM-DUPLEX-ONE-ADD)).
> - **얻은 것** — 전환이 프레임 하나 · `seq` 한 칸 · 요청에서 서버 통지까지 2ms(57.886 → .888 · 59.403 → .405) · 응답까지 7 ~ 11ms. `track_id` · 발행 트랜시버 · SDP 가 그대로다 — A 의 발행 상태기는 `SENDING ↔ REGISTERED` 만 오가고 B 는 `MLINE_EVENT` · `READY` 만 오갔다. 무전으로 쉬는 동안 같은 방 청취는 그대로 이어진다.
> - **잃은 것** — 시뮬캐스트 트랙은 바꿀 수 없다(`3006` · §4-2 실측). 쉬는 트랙은 구독자 줄을 놓으므로 돌아올 때 재배정 · `READY` · 키프레임 기다림이 다시 든다 — 그리고 같은 출처의 다시 싣기라 첫 패킷이 delta 일 수 있다(§4-3 · 2층 211ms). 상태가 얽혀 규칙이 늘었다 — 배타(`1007`) · 반이중 음소거 금지(`3006`) · duplex 적용 시 `muted` 초기화 · 전환 응답을 기다리는 사이 방을 잃으면 응답으로 남길지 정하는 규칙([연:RULE-PUB-SET-PROC](../protocol/07-pub.md#RULE-PUB-SET-PROC) 마지막 항).

## 6. 다른 SFU 는

★아래 「그쪽 방식」은 저쪽 소스를 정적으로 읽은 기록이다 — 「저쪽 소스에는 이렇게 적혀 있다(실측 아님)」.

| 축 | 그쪽 방식 | 우리 | 우리 쪽 장점 | 우리 쪽 대가 |
|---|---|---|---|---|
| 구독자가 영상을 안 받기 | mediasoup — Consumer 를 `paused: true` 로 만들라고 API 주석이 권한다(`mediasoup §5-4`) | `SUBSCRIBE_LAYER{paused}` → 그 줄을 비운다 | 빈 줄을 다른 트랙이 쓴다 — 줄 상한 안에서 받을 것을 고른다 | 해제 때 줄 다시 싣기 · 키프레임 기다림 |
| 발행자 음소거 | LiveKit — 전달기가 멈춤 사유를 `MUTED`(구독자) · `PUB_MUTED`(발행자)로 가른다(`livekit §9-3`) · 음소거 중 패딩 · 공백 프레임(`livekit §9-4`) / mediasoup — 멈춘 Producer 도 통계는 쌓고 전달만 안 한다(`mediasoup §4-4`) | 서버는 RTP 를 거르지 않고 통지만 · 브라우저가 무음 프레임 | 서버 상태가 하나 적다 | 음소거 중에도 대역을 쓴다 · 공백 프레임으로 디코더를 정리하지 않는다 |
| 회의 ↔ 무전 | LiveKit · mediasoup 에는 발언권 개념이 없다(대장 G4 — 누구나 발행) / Janus audiobridge 는 `mute` 와 `suspend` 를 따로 둔다(`Janus §7-5` — 믹서) | 같은 트랙의 `duplex` 속성 · `TRACK_EVENT{add}` 하나 | 한 방에서 회의와 무전을 트랙마다 섞는다 · 재협상 없음 | 시뮬캐스트와 함께 못 쓴다 · 다시 싣기 비용 |

## 7. 운영에서 보는 법

| 사건 | 서버 로그 줄(이 회차) |
|---|---|
| 음소거 · 해제 | `proc=sfu kind=ev spec=CP-OP-TRACK-STATE room=qa_tourattr_p1 seq=5` · hub `spec=CP-OP-TRACK-SET session=976e00c6 user=tourattr_A msg=ok` |
| 영상 멈춤 · 해제 | `spec=CP-OP-MLINE-EVENT session=c6f7c7f3 user=tourattr_B arev=4` · hub `spec=CP-OP-SUBSCRIBE-LAYER … msg=ok` — 멈춤에는 방 줄(`TRACK-STATE` · `TRACK-EVENT`)이 없다 |
| 회의 ↔ 무전 | `spec=CP-OP-TRACK-EVENT room=qa_tourattr_p1 seq=7` · `spec=CP-OP-MLINE-EVENT … arev=6` · hub `spec=CP-OP-TRACK-SET … msg=ok` |
| 줄이 다시 열림 | `spec=SV-STATE-SUBSCRIBE-STATE user=tourattr_B key=t953589e887e8f9ca from=Created to=Active cause=ready` |
| 줄의 재기록 상태 | `GET /admin/users/{user}/media` 의 `rewrite`(`switches` · `seq_off`) — §4-3 관찰을 서버 쪽에서 보는 곳 |

- SDK 쪽은 전이 라벨 `PUB.C.SENDING.set_half` · `PUB.C.REGISTERED.set_full` · `TRK.C.any.track_state` · `TRK.C.any.mine_other` · `SUB.C.any.table_rebound` 로 같은 사건이 찍힌다(라벨이 어디서 오는지는 [15](15_SDK_INSIDE.md)). 로그 줄 꼴과 사건 이력은 [17](17_OPERATIONS.md).
- 거절(§4-1 · §4-2)은 응답 실패라 버림 계수가 아니라 실패 번호 계수(`code_1007` · `code_3006` 꼴)로 센다(— 머리).

## 8. 소스 · 규격 좌표

| 무엇 | 서버(`oxsfud/src/`) | SDK(`oxlens-sdk-web/src/`) | 규격 |
|---|---|---|---|
| 음소거 | `ops/media.rs` `track_set`(배타 → 트랙 찾기 → `muted on half` → `TRACK_STATE` 방 전원) | `api/handles.ts` `setMuted` → `domain/pub/local.ts` `setMuted` · 받는 쪽 `domain/trk/remote.ts` | [연:PUB.op.TRACK_SET](../protocol/07-pub.md#PUB.op.TRACK_SET) · [연:TRK.op.TRACK_STATE](../protocol/06-trk.md#TRK.op.TRACK_STATE) · [연:PROP-TRK-MUTED-FULL-ONLY](../protocol/06-trk.md#PROP-TRK-MUTED-FULL-ONLY) · [연:RULE-PUB-SET-EXCLUSIVE](../protocol/07-pub.md#RULE-PUB-SET-EXCLUSIVE) |
| 영상 멈춤 · 상한 · 우선순위 | `ops/media.rs` `subscribe_layer` → `sub/mod.rs` `set_pref` · `sync` | `api/handles.ts` `setLayer` → `domain/trk/remote.ts` `setLayer` · `adapt` | [연:SUB.op.SUBSCRIBE_LAYER](../protocol/08-sub.md#SUB.op.SUBSCRIBE_LAYER) · [연:RULE-SUB-LAYER-PAUSED-AXIS](../protocol/08-sub.md#RULE-SUB-LAYER-PAUSED-AXIS) · [연:RULE-SUB-FILL](../protocol/08-sub.md#RULE-SUB-FILL) |
| 회의 ↔ 무전 | `ops/media.rs` `set_duplex_of`(시뮬캐스트 거절 · `muted` 초기화 · `TRACK_EVENT{add}` · `resync_room` · full 이면 `rtcp::pli_burst`) | `domain/pub/local.ts` `setDuplex` · `sendDuplex` · `duplexApplied` | [연:RULE-PUB-SET-PROC](../protocol/07-pub.md#RULE-PUB-SET-PROC) · [연:RULE-ROOM-DUPLEX-ONE-ADD](../protocol/04-room.md#RULE-ROOM-DUPLEX-ONE-ADD) · [연:RULE-TRK-FIVE-RULES](../protocol/06-trk.md#RULE-TRK-FIVE-RULES) |
| 줄을 놓고 다시 싣기 | `ops/media.rs`(줄의 출처를 비움 — `src_track.store(None)`) · 재기록기는 그대로 `fwd/core.rs` `Rewriter` | `domain/link`(배정표 다시 받기) | [연:LINK.op.MLINE_EVENT](../protocol/09-link.md#LINK.op.MLINE_EVENT) |
| 마이크 전원 | — | `domain/pub/local.ts` | — — [09](09_MIC_COST.md) |

## 9. 스스로 확인하는 질문

<details><summary>1. 음소거와 영상 멈춤은 둘 다 「안 보인다 · 안 들린다」인데 왜 하나는 방 전원에게 알리고 하나는 아무에게도 안 알리나?</summary>

음소거는 발행자의 사실이다 — 다른 사람 화면에 「음소거」를 그려야 하고 받는 쪽이 스스로 알 수 없다. 멈춤은 구독자 자신의 선택이다 — 발행자는 계속 보내고 다른 구독자와 무관하다. 그래서 앞은 `TRACK_STATE` + `seq++`(5 → 6), 뒤는 내 배정표의 `MLINE_EVENT` 뿐이고 방 `seq` 는 그대로다.
</details>

<details><summary>2. 카메라를 무전으로 바꿨다 되돌렸다. B 의 SDP 는 바뀌었나? 트랙 id 는?</summary>

둘 다 그대로다. 줄 35 가 비었다 다시 실렸을 뿐 m-line 수가 같아 `MLINE_EVENT` · `READY` 만 오갔다. 통지는 같은 `track_id`(`t953589e887e8f9ca`)의 항목 교체(`TRACK_EVENT{add}` · `active` false → true) 둘이다.
</details>

<details><summary>3. muted 와 duplex 를 한 번에 보냈더니 없는 track_id 인데도 1007 이 왔다. 왜 3005 가 아닌가?</summary>

서버가 요청의 꼴(둘은 배타)을 대상 트랙을 찾기 전에 검사하기 때문이다. 꼴이 틀린 요청은 무엇을 가리키든 같은 답이다 — `permanent` 이므로 다시 보내지 말고 둘로 나눠 보낸다.
</details>

<details><summary>4. 무전으로 쉬던 카메라가 회의로 돌아왔다. 구독자가 받은 첫 패킷은 키프레임인가?</summary>

이 회차(2층 `conf_duplex`)에서는 아니었다 — seq 144 delta 가 먼저 나가고 키프레임은 211ms 뒤(seq 150)였다. 재기록기는 출처가 바뀔 때만 키프레임을 기다리는데, 같은 트랙이 같은 줄에 돌아온 것은 출처가 바뀐 것이 아니어서다. 서버는 PLI 묶음을 내지만 그 앞의 delta 를 막지 않는다 — §4-3 의 관찰이다.
</details>
