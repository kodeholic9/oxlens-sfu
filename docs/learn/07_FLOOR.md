# 07. 발언권 — 누름 · 허가 · 줄 · 넘겨받기 · 놓음

> author: kodeholic (powered by Claude) · created: 2026-11-09.
> ★**대조한 커밋** — 서버 `oxlens-sfu` `c3642e8` · SDK `oxlens-sdk-web` `9f40557`(+탐침) · 2층 `oxlens-labs` `fc584ff`
> ★**실측 회차** — 3층 `tour-learn-20261109`(탐침 `TOUR-07`) · 2층 `e2e-20261109-070257`(`ptt_preempt` · `adv_floor_failover` · `adv_floor_multi_dest`). 값은 돌릴 때마다 바뀌고, 차례 · 모양은 같다.

## 1. 30초 요약

```mermaid
flowchart LR
  BTN["버튼 press · release"] --> SDK["SDK 발언권 상태기<br/>FLR.C"]
  SDK == "DC 위 MBCP<br/>REQUEST · RELEASE · ACK" ==> SRV["sfud 발언권 판정<br/>FLR.S · 방마다 하나"]
  SRV == "GRANTED · QINFO · DENY · REVOKE<br/>TAKEN · IDLE · ACK" ==> SDK
  SRV -- "화자 갈아 끼움" --> SLOT["방 슬롯<br/>08 장"]
  SRV -- "TAKEN · IDLE" --> LIS["듣는 사람 모두"]
```

- 발언권은 **WebSocket 이 아니라 데이터 채널(DC)** 로 오간다. 메시지 번호 · ACK · 타이머 · 사유 코드는 3GPP MCPTT 의 발언권 프로토콜(TS 24.380 · MBCP) 원문 그대로이고, 바꾼 것은 전달 수단 하나다([연:RULE-FLR-MBCP-COMPLY](../protocol/10-flr.md#RULE-FLR-MBCP-COMPLY)).
- **서버가 판정한다.** 아무도 안 말하면 곧바로 허가, 누가 말하는 중이면 줄(QINFO), 놓으면 줄 맨 앞에게 넘긴다. 이 회차에서 요청 → 허가 1ms, 놓음 → 다음 사람 허가 2ms.
- 깨지는 길은 셋을 실측했다 — 우선순위 선점(REVOKE 사유 4), 화자가 놓지 않고 사라짐(T1 4초 회수), 남의 방을 지목한 요청(DENY 255).

## 2. 등장인물 · 값

| 이름 | 값 | 무엇 |
|---|---|---|
| A | `tourptt_A` · 말하기 | 먼저 누르는 사람 |
| B | `tourptt_B` · 말하기 | A 가 말하는 중에 누르는 사람 |
| L | `tourptt_L` · 듣기(`select:false`) | 듣기만 한다 — 그래도 DC 를 연다 |
| 방 | `qa_tourptt_p2` · node-a · 1pc 셋 | |
| 시각 | 07:36:47 ~ 53(KST) — 아래는 초.밀리초만 | DC 시각은 브라우저가 찍은 것 |

| 잰 것(이 회차) | 값 |
|---|---|
| A 누름(STEP) → GRANTED | 47.484 → 47.503 = **19ms**(처음 누름이라 마이크 잡기 · 협상 · `PUBLISH_TRACKS` 포함) |
| REQUEST → GRANTED | 47.502 → 47.503 = **1ms** |
| A RELEASE → B GRANTED(넘겨받기) | 49.605 → 49.607 = **2ms** |
| B RELEASE → B 가 받은 ACK(0400) | 52.112 → 52.115 = **3ms** |
| 끝 통계 — 슬롯 오디오 받은 패킷 | A 125 · B 89 · L 213 |

| 규격 값(정본은 [연:FLR.wire](../protocol/10-flr.md#FLR.wire) 의 타이머 표) | |
|---|---|
| `T101` · `T100` · `T104` | 0.5초 · 재전송 `C` 3번 — 클라 |
| `T1` · `T2` · `T3` | 4초(RTP 없음) · 30초(발화 상한) · 3초(회수 유예) — 서버 |
| `T7` · `T8` · `T20` · `T21` | 1초 — IDLE · REVOKE · 승계 GRANTED · TAKEN 재전송. `C7` 10 · `C8` 3 · `C20` 3 · `C21` 3 |
| `T9` | 3초 — 상한 초과(사유 2)로 회수당한 사람의 재요청 차단 |

## 3. 정상 길

### 3-1. 한 장으로

```mermaid
sequenceDiagram
  autonumber
  participant A as A
  participant B as B
  participant L as L 듣기
  participant S as sfud 발언권
  Note over A,S: 입장 READY 마다 지금 상태 하나 — IDLE seq 1 · 2 · 3
  A->>S: 47.499 WS PUBLISH_TRACKS duplex half
  A->>S: 47.502 DC REQUEST
  S->>A: 47.503 GRANTED+A 우선 0 · 30초
  S->>B: 47.503 TAKEN+A 화자 A · seq 4 · SOURCES 192
  S->>L: 47.503 TAKEN+A 화자 A · seq 4
  A->>S: ACK 0100
  B->>S: ACK 0200
  L->>S: ACK 0200
  Note over B: 48.593 누름
  B->>S: 48.630 REQUEST
  S->>B: 48.630 QINFO 1번째
  Note over A: 49.601 놓음
  A->>S: 49.605 RELEASE+A
  S->>A: 49.607 ACK 0400 — A 놓음 끝
  S->>B: 49.607 GRANTED+A
  S->>A: 49.607 TAKEN+A 화자 B · seq 5
  S->>L: 49.607 TAKEN+A 화자 B · seq 5
  Note over B: 52.108 놓음
  B->>S: 52.112 RELEASE+A
  S->>B: 52.115 ACK 0400
  S->>A: 52.115 IDLE seq 6 · 앞 화자 B
  S->>L: 52.115 IDLE seq 6
  S->>A: 53.117 IDLE seq 7 T7 되풀이
```

### 3-2. 프레임 한 통 — DC 4바이트 머리 + MBCP

```
47.503 DC L recv MBCP TAKEN+A [SPEAKER=tourptt_A · SEQ=0004 · ROOM=qa_tourptt_p2 · SOURCES=000000c0]
  01 01 0026 | 12 04 | 04 09 746f75727074745f41 | 08 02 0004 | 1d 0d 71615f…7032 | 1f 04 000000c0
  │  │  │      │  │    │                        │            │                   └ TLV 31 SOURCES — 화자 출처 번호 192
  │  │  │      │  │    │                        │            └ TLV 29 ROOM 13바이트
  │  │  │      │  │    │                        └ TLV 8 SEQ = 4
  │  │  │      │  │    └ TLV 4 SPEAKER 9바이트 "tourptt_A"
  │  │  │      │  └ TLV 4개
  │  │  │      └ 0x12 = A 비트(0x10) + 종류 2 TAKEN
  │  │  └ 길이 38
  │  └ svc 1 = 발언권
  └ ver 1
```

- 종류 번호는 원문 그대로다 — `0` REQUEST · `1` GRANTED · `2` TAKEN · `3` DENY · `4` RELEASE · `5` IDLE · `6` REVOKE · `8` 순번 질의 · `9` QINFO · `10` ACK([연:RULE-FLR-TLV](../protocol/10-flr.md#RULE-FLR-TLV)).
- `ROOM`(29)은 서버가 보내는 발언권 메시지 전부에 실린다 — DC 는 미디어 서버마다 하나라 여러 방의 통지가 한 통로로 섞여 오기 때문이다(다방은 [08](08_SLOT_SPEAKER.md)).
- `SEQ` 는 `IDLE` · `TAKEN` 에만 있고 방마다 1 씩 오른다. 재전송(T7 · T21)도 새로 보내는 것이라 올린다 — **클라는 이 번호를 견주지 않는다**([연:RULE-FLR-SEQ](../protocol/10-flr.md#RULE-FLR-SEQ)).
- `SOURCES`(31)는 원문 밖 자체 확장이다 — 화자의 반이중 발행 트랙 출처 번호. 쓰임은 [08 §3-3](08_SLOT_SPEAKER.md).

### 3-3. 누름 — 마이크는 누를 때 잡는다

```
47.484 STEP A.press
47.496 SDK A PUB audio:microphone IDLE → STAGED          | PUB.C.IDLE.on            (마이크 잡기)
47.499 WS  A send PUBLISH_TRACKS {"duplex":"half", …}                               (mid 0 sendonly)
47.501 SDK A PUB t4f41943c19116e23 STAGED → REGISTERED   | PUB.C.STAGED.pub_ok
47.501 SDK A FLR NOPERM → PREQ                           | FLR.C.NOPERM.press
47.502 DC  A send REQUEST [ROOM]
47.503 DC  A recv GRANTED+A [PRIORITY=0000 · DURATION=001e]
47.503 SDK A PUB REGISTERED → SENDING                    | PUB.C.REGISTERED.floor_open  (게이트 열림)
47.503 SRV   spec=FLR.S.P.NP_IDLE.rx_REQUEST from=IDLE to=TAKEN
47.603 SRV   spec=SV-STATE-PUBLISH-STATE from=Intended to=Active cause=first_rtp
```

- 반이중 트랙은 **허가가 나야 RTP 를 보낸다** — 등록(`REGISTERED`)과 송출(`SENDING`)이 갈려 있다. 서버 쪽에도 같은 문이 있다 — 화자가 아닌 사람의 반이중 RTP 는 버린다(`prefan_gate`, [08](08_SLOT_SPEAKER.md)). 통지가 아니라 이 문이 강제 권위다([연:RULE-FLR-TRANSPORT-DC](../protocol/10-flr.md#RULE-FLR-TRANSPORT-DC)).
- 허가(47.503) 뒤 A 의 첫 실제 RTP 가 서버에 닿기까지 100ms(47.603) — 브라우저가 인코더를 띄우는 시간이다. 그 사이 듣는 쪽 슬롯에는 서버가 지은 예열 무음이 흐른다(B 의 첫 슬롯 패킷 `FLR.C.NOPERM.rtp_rx` 47.503). 예열은 [08](08_SLOT_SPEAKER.md) 몫이다.
- `GRANTED+A` 의 ACK(`0100`)을 서버는 **답하지 않고 받았다는 사실만 본다** — 로그 줄이 `FLR.S.gate.ack_drop` 이다([연:DEV-FLR-ACK-DROP](../protocol/10-flr.md#DEV-FLR-ACK-DROP)). 반면 `TAKEN+A` 의 ACK(`0200`)은 `FLR.S.P.NP_TAKEN.rx_ACK_taken` 으로 그 사람의 T21 재전송을 멈춘다.

### 3-4. 줄 서기 · 넘겨받기 — 쉬지 않고 줄 맨 앞에게

```
48.630 DC  B send REQUEST
48.630 DC  B recv QINFO [QINFO=0100]                    ← 줄 1번째 · 우선 0
48.630 SRV   spec=FLR.S.P.NP_TAKEN.rx_REQUEST from=TAKEN to=TAKEN
49.605 DC  A send RELEASE+A
49.606 SRV   spec=FLR.S.P.PERMITTED.rx_RELEASE from=TAKEN to=TAKEN      ← IDLE 을 거치지 않는다
49.607 DC  B recv GRANTED+A · A recv ACK 0400 · A · L recv TAKEN+A [SPEAKER=tourptt_B · SEQ=0005 · SOURCES=000000c1]
49.607 SDK B FLR QUEUED → QUEUED | rx.GRANTED | FLR.C.QUEUED.rx_GRANTED
49.608 SDK B FLR QUEUED → PERM   | hold       | FLR.C.QUEUED.intent      ← 버튼이 눌린 채라 곧바로 수락
49.625 SRV   spec=FLR.S.P.PERMITTED.rx_RTP (B 첫 RTP)
```

- 누가 말하는 중의 요청은 거절하지 않고 줄에 넣는다 — 큐는 늘 켠다([연:R-FLR-QUEUE-ALWAYS](../protocol/10-flr.md#R-FLR-QUEUE-ALWAYS)). 줄이 차면 그때 `DENY` 사유 7 이다.
- 놓으면 쉼(IDLE)을 거치지 않고 줄 맨 앞에게 넘긴다([연:FLR.S.G.IDLE.enter_succession](../protocol/10-flr.md#FLR.S.G.IDLE.enter_succession)). 듣는 L 은 `IDLE` 없이 `TAKEN`(화자 B)만 받는다.
- 줄에서 받은 허가는 `queued` 에 머문 채 사용자 의지를 기다린다(T132 2초) — 누르고 말하기(hold)는 버튼이 눌린 것이 곧 의지라 1ms 뒤 수락했다([연:RULE-FLR-PROC-QUEUED-ACCEPT](../protocol/10-flr.md#RULE-FLR-PROC-QUEUED-ACCEPT)). 승계 허가는 T20 으로 재전송된다.

### 3-5. 놓음 — 놓은 사람에게는 IDLE 대신 ACK

```
52.112 DC  B send RELEASE+A                      SDK B FLR PERM → PREL | FLR.C.PERM.release
52.113 SDK B PUB SENDING → REGISTERED | mic.close
52.114 SRV   spec=FLR.S.P.PERMITTED.rx_RELEASE from=TAKEN to=IDLE
52.115 DC  B recv ACK [ACKTYPE=0400]             SDK B FLR PREL → NOPERM | FLR.C.PREL.rx_ACK_release
52.115 DC  A · L recv IDLE [SEQ=0006 · PREV=tourptt_B]
53.115 SRV   spec=FLR.S.G.IDLE.T7
53.117 DC  A · B · L recv IDLE [SEQ=0007]
```

- 서버는 쉼으로 들어가며 방 사람 모두에게 `IDLE` 을 돌리지만 **놓은 사람에게는 보내지 않는다**([연:FLR.S.P.PERMITTED.a_IDLE](../protocol/10-flr.md#FLR.S.P.PERMITTED.a_IDLE) — 원문 6.3.5.5.4). 그래서 클라는 `RELEASE` 에 A 비트를 세우고, 서버의 ACK(`0400`)를 「놓음 끝」으로 다룬다([연:DEV-FLR-RELEASE-ACK](../protocol/10-flr.md#DEV-FLR-RELEASE-ACK)). 놓은 사람의 놓음 완료가 **왕복 하나**(3ms)다.
- ACK 이 사라지면(DC 는 `ordered:false` · 재전송 0 이다) T100(0.5초) 뒤 `RELEASE` 를 다시 보내고 C100(3번) 소진이면 놓은 것으로 친다. 이 ACK 이 생기기 전에는 보통 길에서도 T100 을 다 기다렸다 — 놓음 완료 512ms, 그 사이 다시 누름이 버려졌다([연:R-FLR-RELEASE-ACK](../protocol/10-flr.md#R-FLR-RELEASE-ACK)).
- 쉬는 동안 서버는 1초마다 `IDLE` 을 되풀이한다 — 늦게 들어오거나 하나를 놓친 사람도 결국 쉼을 안다. 2층 `ptt_preempt` 의 끝에서 세면 되풀이 9번 뒤 멈췄다(처음 한 통과 합쳐 10통 = `C7`).
- 끝난 뒤의 방 표면(`/admin/rooms/{id}/snapshot` 의 `floor`): `state IDLE · speaker null · queue [] · timers [T7]`.

## 4. 깨지는 길

### 4-1. 우선순위가 높은 사람이 뺏는다 — 선점 · 회수 유예 T3

2층 `ptt_preempt` — botA(우선순위 10)가 말하는 중에 botB(200)가 누른다. 메시지 줄의 앞 숫자는 봇 덤프의 경과 초, `Note` 의 숫자는 서버 로그 시각(07:28:초)이다.

```mermaid
sequenceDiagram
  participant A as botA 우선 10
  participant S as sfud
  participant B as botB 우선 200
  A->>S: 1.493 REQUEST
  S->>A: 1.496 GRANTED 우선 10
  B->>S: 4.491 REQUEST
  Note over S: 06.954 NP_TAKEN.rx_REQUEST — TAKEN → PREVOKE · T3 3초 시작
  S->>B: 4.493 QINFO 1번째 · granted_priority 200
  S->>A: 4.494 REVOKE 사유 4 선점
  S->>A: 5.495 REVOKE 다시 — 07.956 PREVOKE.T8
  S->>A: 6.498 REVOKE 다시 — 08.958 PREVOKE.T8
  A->>S: 7.491 RELEASE
  Note over S: 09.954 PREVOKE.rx_RELEASE — T3 시작에서 3.000초
  S->>A: 7.494 ACK 0400
  S->>B: 7.494 GRANTED 우선 200
  S->>A: 7.495 TAKEN 화자 B · seq 4
```

- **무엇이 깨졌나** — 말하던 사람이 말하는 도중 발언권을 잃는다. 선점은 「뺏어서 바로 주지 않는다」 — 현 화자에게 `REVOKE`(사유 4)를 보내고, 선점자는 줄 맨 앞에 세운 뒤 현 화자의 `RELEASE` 또는 T3 만료 때 넘긴다([연:RULE-FLR-PREEMPT](../protocol/10-flr.md#RULE-FLR-PREEMPT)).
- **누가 먼저 아나** — 서버(판정)와 선점자(`QINFO` 1번째). 현 화자는 `REVOKE` 로 안다.
- **무엇이 오가나** — 이 봇은 `REVOKE` 를 받아도 시나리오가 정한 7.491 까지 놓지 않는다. 그래서 서버가 `REVOKE` 를 1초마다 두 번 더 보냈고(모두 세 통 — `C8` 3), 선점자는 **3.0초** 기다렸다. SDK 는 `REVOKE` 를 받으면 곧바로 `RELEASE` 를 보내므로([연:RULE-FLR-PROC-REVOKE](../protocol/10-flr.md#RULE-FLR-PROC-REVOKE)) 정상이면 이 대기는 왕복 하나다. 3초를 다 쓰는 것은 회수가 유실됐거나 화자가 응답하지 않을 때뿐이다 — 그 3초가 「회수가 사라졌을 때 화자는 말한다고 믿는데 아무도 못 듣는」 것을 덮는 창이다([연:R-FLR-T3-GRACE](../protocol/10-flr.md#R-FLR-T3-GRACE)).
- **사용자는 무엇을 겪나** — A 는 회수 사유 4(다시 눌러도 또 뺏긴다)를 본다. 사유 2(상한 초과 — T9 3초 뒤 다시 누르면 된다)와 처방이 반대라 사유를 가른다. 선점자 B 는 `QINFO` 를 거절로 읽으면 안 된다 — 누르고 말하기면 버튼이 눌린 채라 `GRANTED` 가 오는 순간 열린다.
- 기본 배치에서는 선점이 일어나지 않는다 — 모두 우선순위 0 이면 `>` 가 성립하지 않아 전원 동급 줄이다([연:DEV-FLR-PREEMPT-RELATIVE](../protocol/10-flr.md#DEV-FLR-PREEMPT-RELATIVE)).

### 4-2. 화자가 놓지 않고 사라진다 — T1 회수 · 죽은 사람에게 가는 TAKEN

2층 `adv_floor_failover` — botA 가 허가를 쥔 채 3.727 에 프로세스째 끊긴다(`ROOM_LEAVE` 도 `RELEASE` 도 없다). botC 는 듣기만 한다. 메시지 줄의 숫자는 덤프 경과 초, `Note` 의 숫자는 서버 로그 시각(07:분:초)이다 — 서버 시각 = 덤프 경과 + 07:31:57.974.

```mermaid
sequenceDiagram
  participant A as botA
  participant H as hub
  participant S as sfud node-c
  participant B as botB
  participant C as botC 듣기
  S->>A: 1.728 GRANTED
  S->>C: 1.729 TAKEN 화자 A · seq 4
  A->>S: RTP 마지막 3.713
  Note over A: 3.727 끊김
  Note over H: 32:01.702 SES.S.drop — WS 끊김 · 재개 창 60초 시작
  Note over S: 32:05.687 FLR.S.G.TAKEN.T1 — 마지막 RTP 에서 4.0초
  S->>C: 7.716 IDLE seq 5 · 앞 화자 A
  S->>C: 7.716 ~ 7.759 해제 무음 3 프레임
  B->>S: 11.726 REQUEST
  S->>B: 11.727 GRANTED
  S->>C: 11.728 TAKEN 화자 B · seq 10
  S--xA: TAKEN 화자 B — ACK 없음
  Note over S: 32:10.701 · 32:11.704 NP_TAKEN.T21 · 32:12.704 T21_exhaust
  Note over S: 32:28.863 LINK Alive → Suspect pair_expired
  Note over H: 33:01.868 SES.S.TWindow — 재개 창 끝 · 세션 지움
```

- **무엇이 깨졌나** — 발언권을 쥔 사람이 사라졌는데 판정 상태기는 아직 그를 화자로 안다.
- **누가 먼저 아나** — hub 가 먼저 안다(WS 끊김 `SES.S.drop`, 끊김과 같은 ms). 그러나 **발언권은 WS 를 보지 않는다** — 미디어가 살아 있으면 발언권도 산다는 규칙이라([연:RULE-FLR-SERVER-EVENTS](../protocol/10-flr.md#RULE-FLR-SERVER-EVENTS)), 회수 근거는 RTP 가 4초 안 온 것(T1) 하나다. 마지막 RTP(3.713) + 4.000초 = 7.713, 듣는 쪽 `IDLE` 7.716.
- **무엇이 오가나** — 회수 뒤 B 가 허가를 받자 서버는 죽은 A 에게도 `TAKEN+A` 를 보낸다. ACK 이 안 오니 T21 로 1초마다 두 번 더 보내고 세 번째 만료에서 그만둔다(`T21_exhaust`) — **ACK 유실 재전송이 실측으로 보인 것**이다([연:DEV-FLR-TAKEN-ACK](../protocol/10-flr.md#DEV-FLR-TAKEN-ACK)). 그 재전송도 `SEQ` 를 올려서, C 가 받은 다음 `IDLE` 은 seq 11 · 12 를 건너뛴 13 이다.
- **사용자는 무엇을 겪나** — 듣는 사람은 3.7초부터 4초 동안 **화자 표시는 A 인데 소리가 없다.** 그 4초 동안 다른 사람이 누르면 줄에 선다. 회수 뒤에는 서버가 지은 무음 3 프레임이 오고(seq 92 · 93 · 94, ts 는 4초만큼 건너뜀) `IDLE` 로 쉼을 안다. WS 끊김으로 회수하지 않는 까닭은 발언권이 미디어 연결(DC) 위로 가고 WS 는 시그널링일 뿐이기 때문이다 — WS 만 끊겼다 `RESUME` 한 화자는 발언권을 그대로 쥐고 있다.

### 4-3. 남의 방을 지목하거나 방을 안 적은 요청

2층 `adv_floor_multi_dest` — botADV 는 roomP 에만 들어가 있고 roomQ 는 남의 방이다.

| 덤프 경과 | 보낸 것 | 받은 것 | 서버 로그 |
|---|---|---|---|
| 1.972 | REQUEST `ROOM=roomQ`(남의 방) | 1.975 `DENY` 사유 255 · `REASON "not_in_room"` | `FLR.S.gate.not_in_room` |
| 2.973 | REQUEST `ROOM` 없음 | **무응답** | 없음 — `/admin/drops` 의 `dc_no_room` 만 오른다 |
| 3.973 | REQUEST `ROOM=roomP` | 3.976 `GRANTED` | `FLR.S.P.NP_IDLE.rx_REQUEST` |

- 방을 고를 수 없는 요청에는 **돌려줄 방이 없어** 답하지 않는다 — 조용히 첫 방을 고르지 않는다. 클라는 T101 재전송 3번 뒤 「응답 없음」(`lastEnd:'no_response'`)을 본다.
- 셋째 줄이 있어야 「거부됐다」와 「봇이 애초에 요청을 못 보냈다」가 갈린다 — 같은 봇이 정상 요청으로는 허가를 받는다.

### 4-4. 이 회차로 재지 않은 셋

| 깨짐 | 규격이 정한 결과 | 왜 위험한가 |
|---|---|---|
| ★DC 만 끊김(미디어는 산다) | 서버는 T1 · T2 만 봐서 **회수하지 않는다** · 클라는 `trusted:false` 로 보이고 DC 를 다시 연다 · 소진하면 그 서버 재구축([연:RULE-FLR-PROC-DC-DOWN](../protocol/10-flr.md#RULE-FLR-PROC-DC-DOWN)) | 양쪽이 다르게 아는 유일한 칸 — 말한다고 믿는데 남의 화면에 이름이 안 뜬다. 미디어 지표로 안 잡힌다([연:R-FLR-DC-ONLY-DOWN](../protocol/10-flr.md#R-FLR-DC-ONLY-DOWN)) |
| 반이중 발행 없이 요청 | `DENY` 사유 5(수신 전용) — 무시하지 않는다([연:R-FLR-DENY-RECVONLY](../protocol/10-flr.md#R-FLR-DENY-RECVONLY)). 서버는 `REQUEST` 를 받는 그때 발행 쪽 사실을 읽는다(사본 없음) | 무시하면 클라가 T101 × 3 을 헛되이 태우고 사유를 못 보인다 |
| 놓음의 ACK 유실 | T100 0.5초마다 `RELEASE` 재전송 · C100 소진이면 놓은 것으로 친다 | 「놓는 중」 동안 다시 누름이 버려진다([연:R-FLR-PRESS-DROP](../protocol/10-flr.md#R-FLR-PRESS-DROP)) |


## 5. 왜 이 모양인가

> ★**우리만의 것 — WebRTC 위의 반이중 무전**
> - **선택 배경** — 업무 무전은 한 번에 한 사람 · 우선순위 · 줄 · 선점이 본질이다([연:RULE-FLR-TIMERS](../protocol/10-flr.md#RULE-FLR-TIMERS) 「우선순위·큐 기반 제어가 규율형 무전의 본질」). 소스를 읽어 본 WebRTC 서버들(LiveKit · mediasoup · Janus · JVB)에는 발언권이 없다 — 화자는 판정 대상일 뿐이다(§6). 그래서 새로 짓지 않고 공공안전 무전 표준(3GPP TS 24.380)을 메시지 번호 · ACK · 타이머 · 사유 코드까지 원문대로 옮기고, RTCP APP 패킷 대신 DC 에 싣는 것 하나만 바꿨다([연:RULE-FLR-MBCP-COMPLY](../protocol/10-flr.md#RULE-FLR-MBCP-COMPLY)).
> - **얻은 것** — 서버 권위 판정이 ms 단위로 끝난다: 요청 → 허가 1ms · 놓음 → 다음 사람 허가 2ms · 놓은 사람의 놓음 완료 3ms(이 회차 — 같은 기계라 망 왕복이 거의 0 이다. 실망에서는 왕복 하나가 더해진다). 줄 · 선점 · 회수 유예 · 사유 코드가 원문 절 번호와 한 줄씩 맞물려 원문을 옆에 놓고 검증된다.
> - **잃은 것** — ①**DC 가 없는 청취자는 발언권 통지를 못 받는다.** WS 로 발언권을 보내던 길(`FLOOR_MBCP`)을 철거하면서 그 길이 메우던 청취자 도달도 같이 버렸다 — 2층에서 다른 node 의 방을 듣는 청취자가 그 방의 `TAKEN` 을 0번 받았다([08 §4-3](08_SLOT_SPEAKER.md)). ②**영상 무전은 보류했다** — 한 연결로 음성 · 영상을 같이 보내면 화자 전환마다 영상 키프레임 묶음이 음성 송신을 밀어, 수신 NetEQ 누적 지연 p99 가 264ms 로 커졌다(영상만 끄면 42ms — 예전 실측 · 이번 회차 아님). 그 레버(브라우저 송신 페이서)는 앱이 쥘 수 없다. ③명시 요청이라 첫 음절 앞에 왕복 하나가 붙는다 — 「먼저 보내는 사람이 화자」인 암묵형(SvxLink)보다 늦다.


| 다른 길 | 버린 이유 |
|---|---|
| WS 로도 발언권을 보낸다(폴백) | 두면 「WS 가 끊겨도 발언권은 산다」와 「`RESUME` 스냅샷에 발언권이 없다」가 둘 다 거짓이 된다. 런타임 전환의 방아쇠가 망 문제와 구분되지 않는다([연:RULE-FLR-TRANSPORT-DC](../protocol/10-flr.md#RULE-FLR-TRANSPORT-DC)) |
| 먼저 보내는 사람이 화자(암묵형) | 줄 · 우선순위 · 선점을 표현할 곳이 없다 |
| 발성 감지로 화자 표시 | 화자 표시는 발언권 한 축이다 — 소리 크기로 그리지 않는다([연:RULE-FLR-NO-VAD](../protocol/10-flr.md#RULE-FLR-NO-VAD)) |
| 두 사람 동시 발화(multi-talker) | 슬롯 오디오 하나 구조가 두 화자를 못 받는다([연:REJ-FLR-MULTI-TALKER](../protocol/10-flr.md#REJ-FLR-MULTI-TALKER)) |
| 큐 없이 거절(사유 1) | 줄을 설 수 있는 사람이 포기한다([연:R-FLR-QUEUE-ALWAYS](../protocol/10-flr.md#R-FLR-QUEUE-ALWAYS)) |
| A 비트 없는 `RELEASE` | 놓은 사람이 매번 T100 을 다 기다린다 — 실측 512ms 의 「놓는 중」([연:R-FLR-RELEASE-ACK](../protocol/10-flr.md#R-FLR-RELEASE-ACK)) |

## 6. 다른 SFU 는

★아래 「그쪽 방식」은 각 프로젝트 소스를 정적으로 읽은 대장에서 옮겼다 — **「저쪽 소스에는 이렇게 적혀 있다」이지 실측이 아니다.**

| 상대 | 그쪽 방식 | 우리 | 우리 쪽 장점 | 우리 쪽 대가 |
|---|---|---|---|---|
| LiveKit | 발언권이 없다. 오디오 레벨로 화자를 **판정**한다 — 400ms 창에서 40% 넘게 -35dBov 를 넘으면 active(`livekit §4-4`) | 서버가 허가한 한 사람만 흐른다 | 동시 발화가 구조적으로 없다 · 화자 표시가 판정과 같다 | 요청 왕복 · 판정 상태기를 우리가 든다 |
| mediasoup | 관측자 둘(음량 목록 · 지배 화자 1명) — 판정일 뿐 게이트가 아니다(`mediasoup §11`) | 게이트 + 통지 | 게이트가 권위라 표시와 소리가 어긋나지 않는다 | 회의 쪽 음량 관측은 따로 든다 |
| Janus | videoroom 은 발언권 없음 · audiobridge 는 서버가 **섞는다**(N-1 믹싱 · 디코드 → 재인코드, `Janus §7-4`) | 섞지 않고 한 사람만 통과 | 코덱 해석 · 재인코드 비용이 0 | 둘이 동시에 말하는 것을 들려줄 수 없다 |
| JVB | 에너지 순위로 입구에서 자른다 — 상위 3 + 지배 화자(`JVB §4-2` — 설정 `route-loudest-only` 를 켰을 때 · 기본은 꺼짐) | 발언권으로 자른다 | 「누가 흐르나」가 정책이 아니라 계약이다 | — |
| Mumble | 클라가 판정(VAD · PTT 키) · 서버는 동시 발화를 막지 않고 우선 발언자 감쇠는 클라가 한다(`Mumble §9-1` · `§10-1`) · 발화 끝은 미디어의 `is_terminator`(`§2-4`) | 서버 권위 · 끝은 제어(`RELEASE`) | 줄 · 선점 · 회수가 있다 | 미디어(마지막 소리)와 제어(`IDLE`)가 다른 길 · 다른 속도로 온다 |
| SvxLink | 암묵 — talker 가 비어 있으면 먼저 보낸 노드가 talker(`SvxLink §2-2`) · 화자 통지는 「정보 제공 전용」이라 오디오가 먼저 올 수 있다(`§1-3`) · 3초 오디오 공백 회수 + 상한 초과 뒤 60초 차단(`§2-3`) | 명시 요청 · 통지 권위는 DC · 강제 권위는 서버 게이트 | 우선순위 · 줄 · 선점 | 첫 음절 앞 왕복 하나. 차단은 우리도 둔다 — 사유 2 뒤 T9 3초([연:RULE-FLR-T9-VALUE](../protocol/10-flr.md#RULE-FLR-T9-VALUE)) |
| Zello Work | 비상 호출 — 5초 카운트다운 뒤 10초 무중단 오디오를 지정 비상 채널로 · 기본은 발신자만 해제(조사 기록 §4-4) | 우선순위 선점(REVOKE 사유 4) | 방 안의 누구든 값으로 뺏을 수 있다 | 「무중단 10초」 같은 보장 시간은 계약에 없다 |


## 7. 운영에서 보는 법

| 보고 싶은 것 | 어디서 | 실제 모양(이 회차) |
|---|---|---|
| 판정 전이 | 서버 로그 `kind=ev spec=FLR.S.…` — 판정마다 한 줄(`from` · `to` 는 방 전체 상태 IDLE · TAKEN · PREVOKE) | `spec=FLR.S.P.NP_IDLE.rx_REQUEST … user=tourptt_A area=FLR key=qa_tourptt_p2 from=IDLE to=TAKEN cause=server_rx` |
| 타이머 만료 | 같은 줄 · `cause=t1` · `t7` · `t8` · `t21` | `spec=FLR.S.G.TAKEN.T1 room=… from=TAKEN to=IDLE cause=t1` |
| 지금 화자 · 줄 · 걸린 타이머 | `GET /admin/rooms/{id}/snapshot` 의 `floor` | `{"state":"IDLE","speaker":null,"queue":[],"timers":[{"name":"T7","at":…}],"first_rtp_at":…,"last_rtp_at":…}` |
| 버려진 발언권 프레임 | `GET /admin/drops` 의 `dc_bad_frame` · `dc_unknown_svc` · `dc_no_room` | 4-3 의 방 없는 요청은 `dc_no_room` 으로 센다(소스 `ops/floor.rs` `dc_in` — 이 회차는 계수를 읽지 않았다) |
| 클라 쪽 | SDK 전이 라벨 `FLR.C.…` · `ptt.state`(`phase` · `lastEnd` · `lastDeny` · `lastRevoke` · `trusted`) | `FLR.C.PREL.rx_ACK_release` · `FLR.C.QUEUED.intent` |

- `floor.timers` 는 「왜 아직 안 회수되나」를 밖에서 보는 판단 재료다 — `T1` 의 `at` 은 `last_rtp_at + 4초` 로 밀린 값이 나온다.
- 발언권 줄은 `ev` 종류라 늘 남는다. 로그 줄 꼴 · 세션 지문 · `/admin/history` 로 한 사람의 사슬을 거는 법은 [17](17_OPERATIONS.md).


## 8. 소스 · 규격 좌표

| 무엇 | 서버(`oxlens-sfu/crates`) | SDK(`oxlens-sdk-web/src`) | 규격 |
|---|---|---|---|
| DC 프레임 · MBCP 꼴 | `oxsig/src/dc.rs` · `oxsig/src/floor.rs`(`tlv::SOURCES` · `taken`) | `internal/wire/floor_codec.ts` · `floor_consts.ts` · `internal/flr/mbcp.ts` | [연:TRN.frame.DC](../protocol/02-trn.md#TRN.frame.DC) · [연:RULE-FLR-TLV](../protocol/10-flr.md#RULE-FLR-TLV) |
| 판정 상태기 | `oxsfud/src/flr/core.rs`(`server_rx` · `arb_preempt` · `arb_rel` · `t1` · `t21`) | — | `FLR.S.…` 표 · [연:RULE-FLR-PREEMPT](../protocol/10-flr.md#RULE-FLR-PREEMPT) |
| 집행 · 타이머 · DC 입구 | `oxsfud/src/ops/floor.rs`(`dc_in` · `timer_task` · `fire_due` · `snapshot`) | — | [연:RULE-FLR-TIMERS](../protocol/10-flr.md#RULE-FLR-TIMERS) |
| 클라 상태기 · 마이크 게이트 | — | `domain/flr/core.ts`(`FloorCore`) · `domain/pub/local.ts`(`floorOpen`) | `FLR.C.…` 표 |
| 버림 계수 | `oxsfud/src/ops/mod.rs`(`DROP_NAMES` · `drops_json`) · `ops/admin.rs`(`/admin/drops`) | — | — |

## 9. 스스로 확인하는 질문

<details><summary>1. 듣기만 하는 L 은 발행이 없는데 왜 DC 를 여나?</summary>

발언권 통지(`TAKEN` · `IDLE`)가 DC 로만 오기 때문이다. WS 로 오는 길은 철거했다. DC 가 없으면 소리는 슬롯으로 들려도 누가 말하는지 영영 모른다 — 그래서 청취 전용도 트랙 없는 발행 연결을 세우고 그 위에 DC 를 둔다([연:RULE-FLR-TRANSPORT-DC](../protocol/10-flr.md#RULE-FLR-TRANSPORT-DC)).
</details>

<details><summary>2. 놓은 B 는 IDLE 을 못 받는데 어떻게 놓음이 끝난 줄 아나? ACK 이 사라지면?</summary>

`RELEASE` 에 A 비트를 세우고 서버가 그 `RELEASE` 에 ACK(`0400`)으로 답한다 — 그것이 놓음 끝이다(이 회차 3ms). 서버는 놓은 사람에게 `IDLE` 을 보내지 않는다(원문 6.3.5.5.4). ACK 이 사라지면 T100(0.5초)마다 `RELEASE` 를 다시 보내고 3번 소진이면 놓은 것으로 친다.
</details>

<details><summary>3. 화자가 프로세스째 사라졌는데 hub 는 같은 ms 에 알았다. 발언권은 왜 4초 뒤에야 풀렸나?</summary>

발언권은 WS 를 보지 않기 때문이다 — WS 만 끊긴 사람은 미디어가 살아 있으면 계속 말하고 있다([연:RULE-FLR-SERVER-EVENTS](../protocol/10-flr.md#RULE-FLR-SERVER-EVENTS)). 회수 근거는 RTP 가 T1(4초) 동안 안 온 것 하나다. 2층 `adv_floor_failover` 에서 마지막 RTP 3.713 → 듣는 쪽 `IDLE` 7.716.
</details>

<details><summary>4. 선점당한 봇은 왜 REVOKE 를 세 번 받았고, 선점자는 왜 3초를 기다렸나?</summary>

봇이 `REVOKE` 에 곧바로 `RELEASE` 하지 않았기 때문이다. 서버는 T8(1초)마다 `REVOKE` 를 다시 보내고(C8 3번), 회수 유예 T3(3초) 동안 현 화자의 소리를 계속 흘린다. SDK 라면 받자마자 놓으므로 대기는 왕복 하나다. 3초는 회수 통지가 유실됐을 때를 덮는 창이다.
</details>

<details><summary>5. 2층에서 죽은 A 에게 간 TAKEN 은 몇 번이었고, 왜 C 의 다음 IDLE 번호가 건너뛰었나?</summary>

세 번 — 처음 한 번 + T21 재전송 둘(`NP_TAKEN.T21` 두 줄) 뒤 `T21_exhaust` 로 멈췄다. 재전송도 새 메시지라 `SEQ` 를 올리므로 seq 11 · 12 를 A 몫으로 쓰고, C 의 다음 `IDLE` 은 13 이다. 클라는 이 번호를 견주지 않으니 건너뜀은 문제가 아니다.
</details>
