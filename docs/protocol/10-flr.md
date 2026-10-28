[← 차례](README.md)

## 10. FLR — 발언권

무전에서 지금 누가 말할 수 있는가 — 3GPP TS 24.380 MBCP, DC 위

| 정체성 | 누가 내나 | 무엇마다 | 수명 | 상태 |
|---|---|---|---|---|
| 방 | server | 방마다 | 입장 → 퇴장 | Start-stop · has no permission · pending Request · has permission · pending Release · queued |

| 의존 | 기댐 | 다중도 | 뜻 |
|---|---|---|---|
| PUB | FLR | N:1 | 반이중 트랙은 발언권을 쥔 동안에만 SENDING(§7-2-2) |
| FLR | ROOM | 1:1 | 발언권은 방마다 — 참여자만 갖는다(§10-4-1) |
| FLR | AFF | N:1 | 발언 요청은 발행 방에서만(§10-4-32) |
| FLR | LINK | N:1 | MBCP 는 그 방을 맡은 서버 연결의 DC 위(§10) |

### 10-1. 구조

<a id="FLR.fact.FlrUnadopted"></a>
#### 10-1-1. 미채택 메시지 — 원문 번호 · 비워 둔다

| 원문 subtype | 값 | 이름 | 방향 |
|---|---|---|---|
| `00111` | `7` | *(Floor Revoke Request)* | C→S |
| `x1011` | `11` | *(Unicast Media Flow Control)* | C→S |
| `x1110` | `14` | *(Queued Floor Requests)* | 양방향 |
| `01111` | `15` | *(Floor Release Multi Talker)* | S→C |

<a id="FLR.wire"></a>
#### 10-1-2. 메시지 · TLV · 사유 · 원장

##### 메시지 머리

```
 byte 0                          byte 1        byte 2 …
+---+---+---+---+---+---+---+---+ +-----------+ +----------+
| V   V | R | A | T   T   T   T | | TLV 개수  | |  TLV …   |
+---+---+---+---+---+---+---+---+ +-----------+ +----------+
 bit7  6   5   4   3   2   1   0
```

| 비트 | 폭 | 위치 | 마스크 | 뜻 |
|---|---|---|---|---|
| `V` | 2 | bit7-6 | `0xC0` | 버전 — 00 고정. 다르면 읽지 않는다 |
| `R` | 1 | bit5 | `0x20` | 원문 P(padding) 자리 — 0 으로 보낸다(원문 8.1.2). 받을 때는 보지 않는다(원문에 받는 쪽 규칙이 없다 · 8.1.4) |
| `A` | 1 | bit4 | `0x10` | ACK 을 요구한다 |
| `Type` | 4 | bit3-0 | `0x0F` | 메시지 종류 |
| `count` | u8 | byte 1 | — | TLV 개수 — 개수로 센다, 끝 표시가 없다 |
| TLV | `[id(1)][len(1)][값(len)]` | byte 2 … | — | 개수만큼 이어진다 |

##### TLV 규칙

- `len` 이 1바이트라 ★**값은 255바이트 이하**다. ★**문자열 값은 UTF-8** 이다(`SPEAKER` · `PREV` · `REASON` · `ROOM` — 꼴 utf8 · user)
- ★**모르는 `id` 는 무시한다** — 담아 두고 쓰는 쪽이 무시한다(나중에 필드를 더해도 구버전이 안 깨진다)
- ★**순서는 상관없다** — 순서를 계약으로 삼지 않는다
- 잘렸으면 ★**거기까지 읽고 멈춘다** — 그때까지 읽은 것은 유효하다
- 다시 쓰지 않는 번호 — `7` · `0x1C` · `0x1E`

##### TLV

| ID | 이름 | 꼴 | 원문 | 원문 절 | 실리는 메시지(`?` 선택) | 값 |
|---|---|---|---|---|---|---|
| `0` | `PRIORITY` | u8spare | Floor Priority | 8.2.3.2 | `REQUEST`? · `GRANTED` | 0 최저 ~ 255 최고 · 기본 0 · 값 + 예비(Table 8.2.3.2-1) |
| `1` | `DURATION` | u16be | Duration | 8.2.3.3 | `GRANTED` |  |
| `2` | `CAUSE` | u8 | Reject Cause | 8.2.3.4 | `DENY` · `REVOKE` | 원문은 16bit + 구문 — u8 + 0x1B 로 가른다(DEV-FLR-CAUSE-U8) |
| `3` | `QINFO` | pair(position u8 · priority u8) | Queue Info | 8.2.3.5 | `QINFO` | 순번 1부터 · 254 큐에 없음 · 255 큐에 있으나 순번 미상 · byte1 = 허가된 우선순위(원문 8.2.3.5) |
| `4` | `SPEAKER` | user | Granted Party's Identity | 8.2.3.6 | `TAKEN` |  |
| `8` | `SEQ` | u16be | Message Sequence Number | 8.2.3.10 | `GRANTED` · `TAKEN` · `IDLE` · `ACK`? | 보낼 때마다 1 증가 · 0~65535 감김 |
| `12` | `ACKTYPE` | u8spare | Message Type | 8.2.3.14 | `ACK` | 무엇에 대한 ACK 인가(상대 Type 값) · 값 + 예비(Table 8.2.3.14-1) |
| `26` | `PREV` | user | 직전에 말한 사람 | DEV-FLR-TLV-EXT | `IDLE`? |  |
| `27` | `REASON` | utf8 | 사유 설명 | DEV-FLR-TLV-EXT | `DENY`? · `REVOKE`? |  |
| `29` | `ROOM` | utf8 | 방(room_id) | DEV-FLR-TLV-EXT | `REQUEST` · `GRANTED` · `TAKEN` · `DENY` · `RELEASE` · `IDLE` · `REVOKE` · `QPOS_REQ` · `QINFO` · `ACK` | UTF-8 255바이트 이하 |

##### 메시지

| 종류 | 원문 subtype | 이름 | 원문 | 방향 | 원문 `A` | ★우리 `A` | 칸 | 뜻 |
|---|---|---|---|---|---|---|---|---|
| `0` | `00000` | `REQUEST` · `FLOOR_REQUEST` | Floor Request 8.2.4 | C→S | 못 쓴다 | — | priority(PRIORITY?) · room(ROOM — gate) | 0 — 말하게 해달라. 우선순위는 보통(0)과 다를 때만 싣는다(원문 6.2.4 · 8.2.3.2) |
| `1` | `x0001` | `GRANTED` · `FLOOR_GRANTED` | Floor Granted 8.2.5 | S→C | 쓸 수 있다 | ★세운다 | priority(PRIORITY) · duration(DURATION) · seq(SEQ — seq) · room(ROOM — gate) | 1 — 말해라 |
| `2` | `x0010` | `TAKEN` · `FLOOR_TAKEN` | Floor Taken 8.2.9 | S→C | 쓸 수 있다 | ★세운다 | speaker(SPEAKER) · seq(SEQ — seq) · room(ROOM — gate) | 2 — 누가 말하고 있다 |
| `3` | `x0011` | `DENY` · `FLOOR_DENY` | Floor Deny 8.2.6 | S→C | 쓸 수 있다 | ★세운다 | cause(CAUSE) · reason(REASON?) · room(ROOM — gate) | 3 — 안 된다 |
| `4` | `x0100` | `RELEASE` · `FLOOR_RELEASE` | Floor Release 8.2.7 | C→S | 쓸 수 있다 | 안 세운다 | room(ROOM — gate) | 4 — 다 말했다 |
| `5` | `x0101` | `IDLE` · `FLOOR_IDLE` | Floor Idle 8.2.8 | S→C | 쓸 수 있다 | 안 세운다 | prev(PREV?) · seq(SEQ — seq) · room(ROOM — gate) | 5 — 아무도 안 말한다 |
| `6` | `00110` | `REVOKE` · `FLOOR_REVOKE` | Floor Revoke 8.2.10 | S→C | 못 쓴다 | — | cause(CAUSE) · reason(REASON?) · room(ROOM — gate) | 6 — 그만 말해라(원문이 A 비트를 못 쓰게 한다) |
| `8` | `01000` | `QPOS_REQ` · `FLOOR_QUEUE_POS_REQUEST` | Floor Queue Position Request 8.2.11 | C→S | 못 쓴다 | — | room(ROOM — gate) | 8 — 내 차례는 몇 번째인가 |
| `9` | `x1001` | `QINFO` · `FLOOR_QUEUE_INFO` | Floor Queue Position Info 8.2.12 | S→C | 쓸 수 있다 | 안 세운다 | info(QINFO) · room(ROOM — gate) | 9 — 대기 정보 |
| `10` | `01010` | `ACK` · `FLOOR_ACK` | Floor Ack 8.2.13 | 양방향 | 못 쓴다 | — | of(ACKTYPE) · seq(SEQ?) · room(ROOM — gate) | 10 — 받았다 |

##### 거절 사유

| 값 | 이름 | 원문 |
|---|---|---|
| `3` | `ONLY_ONE` | 8.2.6.2 — Cause #3 - Only one participant |
| `4` | `RETRY_AFTER` | 8.2.6.2 — Cause #4 - Retry-after timer has not expired |
| `5` | `RECV_ONLY` | 8.2.6.2 — Cause #5 - Receive only |
| `7` | `QUEUE_FULL` | 8.2.6.2 — Cause #7 – Queue full |
| `101` | `NOT_PUBROOM` | DEV-FLR-DENY-101 |
| `102` | `NO_PERMISSION` | DEV-FLR-DENY-102 |
| `255` | `OTHER_REASON` | 8.2.6.2 — Cause #255 - Other reason |

##### 회수 사유

| 값 | 이름 | 원문 |
|---|---|---|
| `2` | `TOO_LONG` | 8.2.10.2 — Cause #2 – Media burst too long |
| `3` | `NO_PERMISSION` | 8.2.10.2 — Cause #3 - No permission to send a Media Burst |
| `4` | `PREEMPTED` | 8.2.10.2 — Cause #4 - Media Burst pre-empted |

##### 대기 순번 특수값

| 값 | 이름 | 원문 |
|---|---|---|
| `254` | `NOT_IN_QUEUE` | 8.2.3.5 — has the value '254' if the MCPTT client is not queued |
| `255` | `UNKNOWN` | 8.2.3.5 — has the max value ('255') if the MCPTT client is queued but the MCPTT server is unable to determine the queue position |

내지 않는 거절 사유 — [1, 2, 6] (1 = R-FLR-QUEUE-ALWAYS · 2·6 = DEV-FLR-DENY-2-6)

##### 타이머 · 횟수
타이머 — 클라 0.5초 재전송 값은 원문 값이 아니라 설정이다(원문은 합계 6초 미만만 정한다 — DEV-FLR-CLIENT-RETX)

| 이름 | 원문 이름 | 값 | 재시도 | 쪽 | 도는 동안 | 값의 근거 |
|---|---|---|---|---|---|---|
| `T101` | Floor Request | **0.5초** | `C101` **3회** | 클라 | `cs == 'PREQ'` |  |
| `T100` | Floor Release | **0.5초** | `C100` **3회** | 클라 | `cs == 'PREL'` |  |
| `T104` | Floor Queue Position Request | **0.5초** | `C104` **3회** | 클라 | `cs == 'QUEUED' && sub == 'QUERY'` |  |
| `T132` | Queued granted user action | **2초** | — | 클라 | `cs == 'QUEUED' && sub == 'GRANTED'` |  |
| `T1` | End of RTP media | **4초** | — | 서버 | `g == 'TAKEN' \|\| g == 'PREVOKE'` |  |
| `T1Max` | End of RTP media — 최대 | **6초** | — | — | — | `T1` 의 최대 |
| `T2` | Stop talking | **30초** | — | 서버 | `g == 'TAKEN' && firstRtp` |  |
| `T3` | Stop talking grace | **3초** | — | 서버 | `g == 'PREVOKE'` | 원문 기본값 |
| `T7` | Floor Idle | **1초** | `C7` **10회** | 서버 | `g == 'IDLE' && idleBcast && c7 < Cap('C7')` | `C7`=10 은 원문 Table 11.2.3-1 기본값. ★`T7` 은 원문에 기본값이 **없다**("RAN 특성에 따름") → **우리 값 1초** |
| `T8` | Floor Revoke | **1초** | `C8` **3회** | 서버(참가자마다) | `(part[u] == 'PREVOKE' \|\| part[u] == 'NPSENDS') && c8[u] < Cap('C8')` | 원문 기본값 · ★`C8` **3회**(우리 값) |
| `T20` | Floor Granted | **1초** | `C20` **3회** | 서버 | `g == 'TAKEN' && succ && !rtpSeen && c20 <= Cap('C20')` | 둘 다 원문 기본값 |
| `T9` | Retry-after | **3초** | — | 서버(참가자마다) | `retry[u]` | ★**원문에 값이 없다**(§10-4-32) |
| `T21` | TAKEN 재전송(자체 — DEV-FLR-TAKEN-ACK) | **1초** | `C21` **3회** | 서버(참가자마다) | `part[u] == 'NP_TAKEN' && c21[u] >= 1 && c21[u] <= Cap('C21')` |  |

| 이름 | 횟수 |
|---|---|
| `C101` | 3 |
| `C100` | 3 |
| `C104` | 3 |
| `C7` | 10 |
| `C8` | 3 |
| `C20` | 3 |
| `C21` | 3 |

<a id="FLR.state"></a>
#### 10-1-3. 상태 칸 · 시한

##### 상태 — 서버

| 칸 | 처음 | 뜻 |
|---|---|---|
| `g` | `'IDLE'` | 판정 로직 상태(6.3.4) |
| `speaker` | `none` | 허가 보유자(G: Floor Taken) |
| `sprio` | `0` | 화자의 허가된 우선순위 |
| `revoking` | `none` | 회수 중인 사람(G: pending Floor Revoke) |
| `rprio` | `0` | 회수 중인 사람의 허가된 우선순위 — 회수 유예 동안의 선점 판정 기준 |
| `rcauseG` | `0` | 판정 로직이 낸 회수 사유 |
| `queue` | `[]` | 대기줄 — 원소 {u, p, pre} |
| `succ` | `false` | 이번 허가가 큐 승계인가(T20) |
| `rtpSeen` | `false` | 이번 허가 뒤 화자의 RTP 를 받았나 — 받으면 T20 이 멈춘다 |
| `firstRtp` | `false` | T2 가 첫 RTP 부터 센다(R-FLR-T2-FIRST-RTP) |
| `c20` | `0` | T20(승계 GRANTED 재전송) 횟수 |
| `c7` | `0` | T7(IDLE 재전송) 횟수 |
| `idleBcast` | `false` | IDLE 을 방송했나 — T7 이 도는 조건 |
| `prev` | `none` | 직전 화자(TLV 0x1A) |
| `members` | `Users` | 방 참가자 |
| `pub` | `!EnAffil` (참가자마다) | 발행 방이 이 방인가(DEV-FLR-PUBROOM) |
| `half` | `!EnAffil` (참가자마다) | 반이중 발행 트랙이 있나(DEV-FLR-RECVONLY-TRIGGER) |
| `perm` | `true` (참가자마다) | 권한 비트(DEV-FLR-PERM-REVOKE) |
| `retry` | `false` (참가자마다) | T9 도는 중(DEV-FLR-T9) |
| `part` | `'NP_IDLE'` (참가자마다) | 참가자 상태(6.3.5) |
| `released` | `false` (참가자마다) | 방금 반환했나 — 반환 뒤 Idle 에서 RTP 를 보내면 사유 3 으로 회수한다(6.3.5.3.8) |
| `gTaken` | `false` (참가자마다) | NPSENDS 가 아는 판정 로직 상태 |
| `c8` | `0` (참가자마다) | T8(REVOKE 재전송) 횟수 |
| `c21` | `0` (참가자마다) | T21(TAKEN 재전송) 횟수 — 0 이면 멈춤(DEV-FLR-TAKEN-ACK) |
| `tq` | `0` (참가자마다) | 이 참가자에게 마지막으로 보낸 TAKEN 의 일련번호 — T21 재전송이 같은 번호를 다시 싣는다(DEV-FLR-T21-SAME-SEQ) |
| `rcause` | `0` (참가자마다) | 이 참가자에게 낸 회수 사유 — T8 재전송이 다시 싣는다 |
| `seq` | `0` | TLV 8 일련번호 |

##### 상태 — 클라

| 칸 | 처음 | 뜻 |
|---|---|---|
| `cs` | `'NOPERM'` | 클라 상태(6.2.4) |
| `sub` | `'WAIT'` | queued 세부 |
| `want` | `false` | 사용자가 말하려 한다 |
| `c101` | `0` | T101(REQUEST 재전송) 횟수 |
| `c100` | `0` | T100(RELEASE 재전송) 횟수 |
| `c104` | `0` | T104(순번 질의 재전송) 횟수 |
| `presses` | `0` (검증 모형만) | 누름 횟수 — 검사 상한(MaxPress)용 |
| `prio` | `0` | 요청에 실은 우선순위 |
| `grant` | `0` | 허가된 시간(초) |
| `isPub` | `!EnAffil` | 이 방이 발행 방인가(전환 응답을 받았다) |
| `hasHalf` | `!EnAffil` | 반이중 발행 트랙을 발행했나 |
| `left` | `false` | 방을 나갔다 |
| `shown` | `none` | 화자 표시 — 화면에 보이는 화자(내가 말하는 중이면 없음) |
| `seen` | `0` | 화자 표시에 마지막으로 쓴 TAKEN · IDLE 의 일련번호(DEV-FLR-SEQ-DISPLAY) |

##### 값 — 원문이 부르는 이름

| 값 형 | 값 | 이름 |
|---|---|---|
| `CState` | `NOPERM` | `has no permission` |
| `CState` | `PREQ` | `pending Request` |
| `CState` | `PERM` | `has permission` |
| `CState` | `PREL` | `pending Release` |
| `CState` | `QUEUED` | `queued` |
| `GState` | `IDLE` | `G: Floor Idle` |
| `GState` | `TAKEN` | `G: Floor Taken` |
| `GState` | `PREVOKE` | `G: pending Floor Revoke` |
| `PState` | `NP_IDLE` | `U: not permitted and Floor Idle` |
| `PState` | `NP_TAKEN` | `U: not permitted and Floor Taken` |
| `PState` | `PERMITTED` | `U: permitted` |
| `PState` | `PREVOKE` | `U: pending Floor Revoke` |
| `PState` | `NPSENDS` | `U: not permitted but sends media` |
| `PState` | `RELEASING` | `U: releasing — 나간 사람(원문 6.3.5.8.2 · 원문에 상태 이름이 없어 자체 이름)` |

<a id="FLR.C"></a><a id="FLR.S"></a>
### 10-2. 동작

<a id="FLR.C.NOPERM"></a>
#### 10-2-1. 클라 · `NOPERM`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.C.NOPERM.press"></a>`FLR.C.NOPERM.press` | cs == 'NOPERM' | cs ← 'PREQ'; want ← true; presses ← presses + 1; prio ← p; c101 ← 1 · 보낸다 REQUEST | 6.2.4.3.5 |
| <a id="FLR.C.NOPERM.rtp_rx"></a>`FLR.C.NOPERM.rtp_rx` | cs == 'NOPERM' | 효과 media.render | 6.2.4.3.4 |
| <a id="FLR.C.NOPERM.rx_TAKEN"></a>`FLR.C.NOPERM.rx_TAKEN` | cs == 'NOPERM' && m.t == 'TAKEN' && Newer(m.q, seen) | shown ← m.a; seen ← m.q · ack TAKEN m.q · 효과 app.speaker | 6.2.4.3.3 |
| <a id="FLR.C.NOPERM.rx_TAKEN_stale"></a>`FLR.C.NOPERM.rx_TAKEN_stale` | cs == 'NOPERM' && m.t == 'TAKEN' | ack TAKEN m.q | 6.2.4.3.3 DEV-FLR-SEQ-DISPLAY |
| <a id="FLR.C.NOPERM.rx_IDLE"></a>`FLR.C.NOPERM.rx_IDLE` | cs == 'NOPERM' && m.t == 'IDLE' && Newer(m.q, seen) | shown ← none; seen ← m.q · 효과 app.speaker_none | 6.2.4.3.2 R-FLR-NO-T103 |
| <a id="FLR.C.NOPERM.rx_IDLE_stale"></a>`FLR.C.NOPERM.rx_IDLE_stale` | cs == 'NOPERM' && m.t == 'IDLE' | — | 6.2.4.3.2 DEV-FLR-SEQ-DISPLAY |
| <a id="FLR.C.NOPERM.rx_other"></a>`FLR.C.NOPERM.rx_other` | cs == 'NOPERM' | — | 6.2.4.1 |

<a id="FLR.C.any"></a>
#### 10-2-2. 클라 · `any`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.C.any.press_drop"></a>`FLR.C.any.press_drop` | 나머지 | — | 6.2.4.4 6.2.4.5 6.2.4.6 6.2.4.9 DEV-FLR-PRESS-DROP — 요청 중 · 발언 중 · 반환 중 · 대기 중의 누름은 버린다 — 보내는 것도 바뀌는 것도 없다 |
| <a id="FLR.C.any.reg_half"></a>`FLR.C.any.reg_half` | 받는 효과 PUB floor.reg_half · 나머지 | hasHalf ← true | DEV-FLR-RECVONLY-TRIGGER R-FLR-DENY-RECVONLY |
| <a id="FLR.C.any.pub_select"></a>`FLR.C.any.pub_select` | 받는 효과 AFF floor.pub_select · 나머지 | isPub ← true | DEV-FLR-PUBROOM R-FLR-PUBROOM-ORDER |
| <a id="FLR.C.any.pub_away"></a>`FLR.C.any.pub_away` | 받는 효과 AFF floor.pub_away · 나머지 | cs ← 'NOPERM'; want ← false; isPub ← false · 효과 mic.close → PUB.C.SENDING.floor_close · PUB.C.any.floor_close_other | DEV-FLR-PUB-AWAY |
| <a id="FLR.C.any.leave"></a>`FLR.C.any.leave` | 받는 효과 ROOM floor.leave · 나머지 | cs ← 'NOPERM'; want ← false; isPub ← false; shown ← none; left ← true · 효과 mic.close → PUB.C.SENDING.floor_close · PUB.C.any.floor_close_other | DEV-FLR-PUB-AWAY — 발언권은 방에 든 동안만 있다(관계 FLR → ROOM) — 떠난 방의 화자 표시를 지운다 · 일련번호는 둔다(서버 번호가 이어진다 — 지우는 것은 재구축 · DEV-FLR-DOWN-FRESH) |
| <a id="FLR.C.any.join"></a>`FLR.C.any.join` | 받는 효과 ROOM floor.join · 나머지 | sub ← 'WAIT'; want ← false; prio ← 0; grant ← 0; isPub ← false; shown ← none; left ← false | DEV-FLR-JOIN-FRESH R-FLR-JOIN-FRESH — 처음부터 든 사람과 같은 칸 — 요청 없음 · 발행 방 아님(전환 응답이 따로 온다) · 화자 표시는 READY 의 첫 알림이 채운다 · 일련번호는 둔다(서버 번호가 이어진다 — CLeave 와 같은 까닭) · 상태는 떠날 때 이미 NOPERM 이다(CLeave) |
| <a id="FLR.C.any.down"></a>`FLR.C.any.down` | 받는 효과 SES floor.down_all · LINK floor.down_server · 나머지 | cs ← 'NOPERM'; want ← false; isPub ← false; hasHalf ← false; shown ← none; seen ← 0 · 효과 mic.close → PUB.C.SENDING.floor_close · PUB.C.any.floor_close_other | DEV-FLR-DOWN-FRESH R-FLR-DOWN-FRESH — DC 가 없으니 통지가 오지 않는다 — 클라가 스스로 내린다(§3-2-3 1-1) |
| <a id="FLR.C.any.dc_down"></a>`FLR.C.any.dc_down` | 받는 효과 LINK floor.dc_down · 나머지 | 효과 app.floor_unreliable | DEV-FLR-DC-DOWN R-FLR-DC-ONLY-DOWN |
| <a id="FLR.C.any.dc_reopen"></a>`FLR.C.any.dc_reopen` | 받는 효과 LINK floor.dc_up · 나머지 | 효과 app.floor_reliable | DEV-FLR-DC-DOWN R-FLR-DC-ONLY-DOWN |

<a id="FLR.C.PREQ"></a>
#### 10-2-3. 클라 · `PREQ`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.C.PREQ.release"></a>`FLR.C.PREQ.release` | 받는 상태 cs == 'PREQ' \|\| cs == 'PERM' \|\| cs == 'QUEUED' · cs == 'PREQ' | cs ← 'PREL'; want ← false; c100 ← 1 · 보낸다 RELEASE | 6.2.4.4.8 R-FLR-TAP-RELEASE |
| <a id="FLR.C.PREQ.rtp_rx"></a>`FLR.C.PREQ.rtp_rx` | cs == 'PREQ' | 효과 media.render | 6.2.4.4.7 |
| <a id="FLR.C.PREQ.rx_GRANTED"></a>`FLR.C.PREQ.rx_GRANTED` | cs == 'PREQ' && m.t == 'GRANTED' | cs ← 'PERM'; grant ← m.d; shown ← none; seen ← Newer(m.q, seen) ? m.q : seen · ack GRANTED m.q · 효과 app.granted · 효과 mic.open → PUB.C.REGISTERED.floor_open · PUB.C.any.floor_open_other | 6.2.4.4.2 DEV-FLR-GRANTED-SEQ |
| <a id="FLR.C.PREQ.rx_DENY"></a>`FLR.C.PREQ.rx_DENY` | cs == 'PREQ' && m.t == 'DENY' | cs ← 'NOPERM'; want ← false · ack DENY · 효과 app.denied | 6.2.4.4.4 R-FLR-CAUSE-SPLIT |
| <a id="FLR.C.PREQ.rx_QINFO"></a>`FLR.C.PREQ.rx_QINFO` | cs == 'PREQ' && m.t == 'QINFO' | cs ← 'QUEUED'; sub ← 'WAIT' · 효과 app.queued | 6.2.4.4.9 |
| <a id="FLR.C.PREQ.rx_TAKEN"></a>`FLR.C.PREQ.rx_TAKEN` | cs == 'PREQ' && m.t == 'TAKEN' && Newer(m.q, seen) | shown ← m.a; seen ← m.q · ack TAKEN m.q · 효과 app.speaker | 6.2.4.4.11 DEV-FLR-PREQ-TAKEN-STAY |
| <a id="FLR.C.PREQ.rx_TAKEN_stale"></a>`FLR.C.PREQ.rx_TAKEN_stale` | cs == 'PREQ' && m.t == 'TAKEN' | ack TAKEN m.q | 6.2.4.4.11 DEV-FLR-SEQ-DISPLAY |
| <a id="FLR.C.PREQ.rx_IDLE"></a>`FLR.C.PREQ.rx_IDLE` | cs == 'PREQ' && m.t == 'IDLE' && Newer(m.q, seen) | shown ← none; seen ← m.q · 효과 app.speaker_none | 6.2.4.4 DEV-FLR-PREQ-IDLE-DISPLAY |
| <a id="FLR.C.PREQ.rx_IDLE_stale"></a>`FLR.C.PREQ.rx_IDLE_stale` | cs == 'PREQ' && m.t == 'IDLE' | — | 6.2.4.4 DEV-FLR-SEQ-DISPLAY |
| <a id="FLR.C.PREQ.rx_other"></a>`FLR.C.PREQ.rx_other` | cs == 'PREQ' | — | 6.2.4.1 |
| <a id="FLR.C.PREQ.T101"></a>`FLR.C.PREQ.T101` | c101 < Cap('C101') | c101 ← c101 + 1 · 보낸다 REQUEST | 6.2.4.4.5 DEV-FLR-CLIENT-RETX |
| <a id="FLR.C.PREQ.T101_exhaust"></a>`FLR.C.PREQ.T101_exhaust` | 나머지 | cs ← 'NOPERM' · 효과 app.request_timeout | 6.2.4.4.6 |

<a id="FLR.C.PERM"></a>
#### 10-2-4. 클라 · `PERM`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.C.PERM.release"></a>`FLR.C.PERM.release` | 받는 상태 cs == 'PREQ' \|\| cs == 'PERM' \|\| cs == 'QUEUED' · cs == 'PERM' | cs ← 'PREL'; want ← false; c100 ← 1 · 보낸다 RELEASE · 효과 mic.close → PUB.C.SENDING.floor_close · PUB.C.any.floor_close_other | 6.2.4.5.3 |
| <a id="FLR.C.PERM.rtp_tx"></a>`FLR.C.PERM.rtp_tx` | 나머지 | — | 6.2.4.5.2 |
| <a id="FLR.C.PERM.rtp_rx"></a>`FLR.C.PERM.rtp_rx` | cs == 'PERM' | 효과 media.render | 6.2.4.5.6 |
| <a id="FLR.C.PERM.rx_REVOKE"></a>`FLR.C.PERM.rx_REVOKE` | cs == 'PERM' && m.t == 'REVOKE' | cs ← 'PREL'; want ← false; c100 ← 1 · 효과 mic.close → PUB.C.SENDING.floor_close · PUB.C.any.floor_close_other · 효과 app.revoked · 보낸다 RELEASE | 6.2.4.5.4 R-FLR-REVOKE-MIC R-FLR-CAUSE-SPLIT |
| <a id="FLR.C.PERM.rx_GRANTED"></a>`FLR.C.PERM.rx_GRANTED` | cs == 'PERM' && m.t == 'GRANTED' | grant ← m.d; seen ← Newer(m.q, seen) ? m.q : seen · ack GRANTED m.q | 6.2.4.5.5 DEV-FLR-GRANTED-SEQ R-FLR-REGRANT |
| <a id="FLR.C.PERM.rx_IDLE"></a>`FLR.C.PERM.rx_IDLE` | cs == 'PERM' && m.t == 'IDLE' | — | 6.2.4.5.7 |
| <a id="FLR.C.PERM.rx_TAKEN"></a>`FLR.C.PERM.rx_TAKEN` | cs == 'PERM' && m.t == 'TAKEN' | — | 6.2.4.5.8 |
| <a id="FLR.C.PERM.rx_other"></a>`FLR.C.PERM.rx_other` | cs == 'PERM' | — | 6.2.4.1 |

<a id="FLR.C.QUEUED"></a>
#### 10-2-5. 클라 · `QUEUED`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.C.QUEUED.release"></a>`FLR.C.QUEUED.release` | 받는 상태 cs == 'PREQ' \|\| cs == 'PERM' \|\| cs == 'QUEUED' · 나머지 | cs ← 'PREL'; want ← false; c100 ← 1 · 보낸다 RELEASE | 6.2.4.9.6 DEV-FLR-T132-SCOPE |
| <a id="FLR.C.QUEUED.intent"></a>`FLR.C.QUEUED.intent` | 받는 상태 cs == 'QUEUED' && sub == 'GRANTED' · 나머지 | cs ← 'PERM'; shown ← none · 효과 mic.open → PUB.C.REGISTERED.floor_open · PUB.C.any.floor_open_other | 6.2.4.9.12 R-FLR-T132 |
| <a id="FLR.C.QUEUED.qpos"></a>`FLR.C.QUEUED.qpos` | 받는 상태 cs == 'QUEUED' && sub == 'WAIT' · 나머지 | sub ← 'QUERY'; c104 ← 1 · 보낸다 QPOS_REQ | 6.2.4.9.9 |
| <a id="FLR.C.QUEUED.rtp_rx"></a>`FLR.C.QUEUED.rtp_rx` | 나머지 | 효과 media.render | 6.2.4.9.2 |
| <a id="FLR.C.QUEUED.rx_TAKEN"></a>`FLR.C.QUEUED.rx_TAKEN` | m.t == 'TAKEN' && Newer(m.q, seen) | shown ← m.a; seen ← m.q · ack TAKEN m.q · 효과 app.speaker | 6.2.4.9.3 |
| <a id="FLR.C.QUEUED.rx_TAKEN_stale"></a>`FLR.C.QUEUED.rx_TAKEN_stale` | m.t == 'TAKEN' | ack TAKEN m.q | 6.2.4.9.3 DEV-FLR-SEQ-DISPLAY |
| <a id="FLR.C.QUEUED.rx_GRANTED"></a>`FLR.C.QUEUED.rx_GRANTED` | m.t == 'GRANTED' && sub != 'GRANTED' | sub ← 'GRANTED'; grant ← m.d; shown ← none; seen ← Newer(m.q, seen) ? m.q : seen · ack GRANTED m.q · 효과 app.granted_queued | 6.2.4.9.4 DEV-FLR-T132-DISPLAY DEV-FLR-GRANTED-SEQ R-FLR-T132 |
| <a id="FLR.C.QUEUED.rx_GRANTED_again"></a>`FLR.C.QUEUED.rx_GRANTED_again` | m.t == 'GRANTED' | seen ← Newer(m.q, seen) ? m.q : seen · ack GRANTED m.q | 6.2.4.9.4 DEV-FLR-GRANTED-SEQ R-FLR-T132 |
| <a id="FLR.C.QUEUED.rx_DENY"></a>`FLR.C.QUEUED.rx_DENY` | m.t == 'DENY' | cs ← 'NOPERM'; want ← false · ack DENY · 효과 app.denied | 6.2.4.9.5 DEV-FLR-T132-SCOPE R-FLR-CAUSE-SPLIT |
| <a id="FLR.C.QUEUED.rx_IDLE"></a>`FLR.C.QUEUED.rx_IDLE` | m.t == 'IDLE' && Newer(m.q, seen) | cs ← 'NOPERM'; shown ← none; seen ← m.q · 효과 app.speaker_none | 6.2.4.9.8 DEV-FLR-T132-SCOPE |
| <a id="FLR.C.QUEUED.rx_IDLE_stale"></a>`FLR.C.QUEUED.rx_IDLE_stale` | m.t == 'IDLE' | — | 6.2.4.9.8 DEV-FLR-STALE-IDLE-STAY — 낡은 번호의 IDLE — 대기를 떠나지 않고 화자 표시도 그대로(서버 대기열과 어긋나지 않게) |
| <a id="FLR.C.QUEUED.rx_QINFO_answer"></a>`FLR.C.QUEUED.rx_QINFO_answer` | m.t == 'QINFO' && sub == 'QUERY' | sub ← 'WAIT' · 효과 app.queued | 6.2.4.9.7 |
| <a id="FLR.C.QUEUED.rx_QINFO"></a>`FLR.C.QUEUED.rx_QINFO` | m.t == 'QINFO' | 효과 app.queued | 6.2.4.9.7 |
| <a id="FLR.C.QUEUED.rx_other"></a>`FLR.C.QUEUED.rx_other` | 나머지 | — | 6.2.4.1 |
| <a id="FLR.C.QUEUED.T104"></a>`FLR.C.QUEUED.T104` | c104 < Cap('C104') | c104 ← c104 + 1 · 보낸다 QPOS_REQ | 6.2.4.9.10 DEV-FLR-CLIENT-RETX |
| <a id="FLR.C.QUEUED.T104_exhaust"></a>`FLR.C.QUEUED.T104_exhaust` | 나머지 | cs ← 'PREL'; c100 ← 1 · 보낸다 RELEASE · 효과 app.queue_timeout | 6.2.4.9.11 |
| <a id="FLR.C.QUEUED.T132"></a>`FLR.C.QUEUED.T132` | 나머지 | cs ← 'NOPERM'; want ← false · 보낸다 RELEASE · 효과 app.grant_expired | 6.2.4.9.13 R-FLR-T132 |

<a id="FLR.C.PREL"></a>
#### 10-2-6. 클라 · `PREL`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.C.PREL.rtp_rx"></a>`FLR.C.PREL.rtp_rx` | cs == 'PREL' | 효과 media.render | 6.2.4.6.6 |
| <a id="FLR.C.PREL.rx_IDLE"></a>`FLR.C.PREL.rx_IDLE` | cs == 'PREL' && m.t == 'IDLE' | cs ← 'NOPERM'; shown ← none; seen ← m.q · 효과 app.speaker_none | 6.2.4.6.4 |
| <a id="FLR.C.PREL.rx_TAKEN"></a>`FLR.C.PREL.rx_TAKEN` | cs == 'PREL' && m.t == 'TAKEN' && Newer(m.q, seen) | cs ← 'NOPERM'; shown ← m.a; seen ← m.q · ack TAKEN m.q · 효과 app.speaker | 6.2.4.6.5 |
| <a id="FLR.C.PREL.rx_TAKEN_seen"></a>`FLR.C.PREL.rx_TAKEN_seen` | cs == 'PREL' && m.t == 'TAKEN' | ack TAKEN m.q | 6.2.4.6.5 DEV-FLR-T21-SAME-SEQ — 이미 본 번호의 TAKEN(T21 재전송) — 반환 확인으로 쓰지 않는다 |
| <a id="FLR.C.PREL.rx_REVOKE"></a>`FLR.C.PREL.rx_REVOKE` | cs == 'PREL' && m.t == 'REVOKE' | 효과 app.revoked | 6.2.4.6.7 |
| <a id="FLR.C.PREL.rx_GRANTED"></a>`FLR.C.PREL.rx_GRANTED` | cs == 'PREL' && m.t == 'GRANTED' | seen ← Newer(m.q, seen) ? m.q : seen · ack GRANTED m.q | 6.2.4.6.8 DEV-FLR-GRANTED-SEQ |
| <a id="FLR.C.PREL.rx_other"></a>`FLR.C.PREL.rx_other` | cs == 'PREL' | — | 6.2.4.1 |
| <a id="FLR.C.PREL.T100"></a>`FLR.C.PREL.T100` | c100 < Cap('C100') | c100 ← c100 + 1 · 보낸다 RELEASE | 6.2.4.6.2 DEV-FLR-CLIENT-RETX |
| <a id="FLR.C.PREL.T100_exhaust"></a>`FLR.C.PREL.T100_exhaust` | 나머지 | cs ← 'NOPERM' | 6.2.4.6.3 |

<a id="FLR.S.G"></a>
#### 10-2-7. 서버 · `G`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.S.G.TAKEN.enter"></a>`FLR.S.G.TAKEN.enter` | 나머지 | idleBcast ← false · partA speaker GRANTED sprio Ms.T2 / 1000 seq · fan TAKEN speaker speaker | 6.3.4.4.2 |
| <a id="FLR.S.G.IDLE.enter_empty"></a>`FLR.S.G.IDLE.enter_empty` | len(queue) == 0 | g ← 'IDLE'; speaker ← none; revoking ← none; firstRtp ← false; idleBcast ← true; c7 ← 1 · fan IDLE prev none | 6.3.4.3.2 R-FLR-NO-IDLE-WHEN-QUEUE |
| <a id="FLR.S.G.IDLE.enter_succession"></a>`FLR.S.G.IDLE.enter_succession` | 나머지 | g ← 'TAKEN'; speaker ← w.u; sprio ← w.p; revoking ← none; queue ← tail(queue); succ ← true; rtpSeen ← false; firstRtp ← false; c20 ← 1 · call EnterTaken | 6.3.4.3.2 R-FLR-NO-IDLE-WHEN-QUEUE |
| <a id="FLR.S.G.PREVOKE.enter"></a>`FLR.S.G.PREVOKE.enter` | 나머지 | g ← 'PREVOKE'; revoking ← u; rprio ← sprio; rcauseG ← cause; speaker ← none · partA u REVOKE cause | 6.3.4.5.2 R-FLR-T3-GRACE |
| <a id="FLR.S.G.any.req_discard"></a>`FLR.S.G.any.req_discard` | g != 'IDLE' | — | 6.3.4.1 |
| <a id="FLR.S.G.IDLE.req_alone"></a>`FLR.S.G.IDLE.req_alone` | Alone() | partA u DENY Deny.ONLY_ONE | 6.3.4.3.3 |
| <a id="FLR.S.G.IDLE.req_retry_after"></a>`FLR.S.G.IDLE.req_retry_after` | retry[u] | partA u DENY Deny.RETRY_AFTER | 6.3.4.3.3 DEV-FLR-T9 R-FLR-T9 |
| <a id="FLR.S.G.IDLE.req_grant"></a>`FLR.S.G.IDLE.req_grant` | 나머지 | g ← 'TAKEN'; speaker ← u; sprio ← p; succ ← false; rtpSeen ← false; firstRtp ← false; c20 ← 0 · call EnterTaken | 6.3.4.3.3 |
| <a id="FLR.S.G.TAKEN.req_regrant"></a>`FLR.S.G.TAKEN.req_regrant` | g == 'TAKEN' && speaker == u | partA u GRANTED sprio T2Left() seq | 6.3.4.4.8 R-FLR-REGRANT |
| <a id="FLR.S.G.any.regrant_discard"></a>`FLR.S.G.any.regrant_discard` | 나머지 | — | 6.3.4.1 |
| <a id="FLR.S.G.TAKEN.preempt"></a>`FLR.S.G.TAKEN.preempt` | g == 'TAKEN' | queue ← cons({u: u, p: p, pre: true}, queue) · call PrevokeEnter · partA u QINFO 1 p | 6.3.4.4.7 DEV-FLR-PREEMPT-RELATIVE R-FLR-PREEMPT-OPEN |
| <a id="FLR.S.G.PREVOKE.preempt_discard"></a>`FLR.S.G.PREVOKE.preempt_discard` | 나머지 | — | 6.3.4.1 |
| <a id="FLR.S.G.TAKEN.rel_speaker"></a>`FLR.S.G.TAKEN.rel_speaker` | g == 'TAKEN' && u == speaker | prev ← u · call IdleEnter | 6.3.4.4.6 |
| <a id="FLR.S.G.PREVOKE.rel_revoked"></a>`FLR.S.G.PREVOKE.rel_revoked` | g == 'PREVOKE' && u == revoking | prev ← u · call IdleEnter | 6.3.4.5.4 R-FLR-REVOKE-RELEASE |
| <a id="FLR.S.G.any.rel_discard"></a>`FLR.S.G.any.rel_discard` | 나머지 | — | 6.3.4.1 |
| <a id="FLR.S.G.TAKEN.rtp"></a>`FLR.S.G.TAKEN.rtp` | g == 'TAKEN' && u == speaker | rtpSeen ← true; firstRtp ← true | 6.3.4.4.5 R-FLR-T2-FIRST-RTP R-FLR-RTP-GATED |
| <a id="FLR.S.G.PREVOKE.rtp"></a>`FLR.S.G.PREVOKE.rtp` | g == 'PREVOKE' && u == revoking | — | 6.3.4.5.3 R-FLR-T3-GRACE |
| <a id="FLR.S.G.any.rtp_discard"></a>`FLR.S.G.any.rtp_discard` | 나머지 | — | 6.3.4.1 |
| <a id="FLR.S.G.TAKEN.client_release"></a>`FLR.S.G.TAKEN.client_release` | g == 'TAKEN' && speaker == u | prev ← u · call IdleEnter | 6.3.4.4.11 x:6.3.5.8.2 DEV-FLR-PUB-AWAY |
| <a id="FLR.S.G.PREVOKE.client_release"></a>`FLR.S.G.PREVOKE.client_release` | g == 'PREVOKE' && revoking == u | — | 6.3.4.1 DEV-FLR-PUB-AWAY |
| <a id="FLR.S.G.TAKEN.T1"></a>`FLR.S.G.TAKEN.T1` | g == 'TAKEN' | prev ← speaker · call IdleEnter | 6.3.4.4.3 |
| <a id="FLR.S.G.PREVOKE.T1"></a>`FLR.S.G.PREVOKE.T1` | 나머지 | prev ← revoking · call IdleEnter | 6.3.4.5.6 |
| <a id="FLR.S.G.TAKEN.T2"></a>`FLR.S.G.TAKEN.T2` | 나머지 | retry[u] ← true · call PrevokeEnter | 6.3.4.4.4 DEV-FLR-T9 R-FLR-T9 |
| <a id="FLR.S.G.PREVOKE.T3"></a>`FLR.S.G.PREVOKE.T3` | 나머지 | prev ← revoking · call IdleEnter | 6.3.4.5.5 R-FLR-T3-GRACE |
| <a id="FLR.S.G.IDLE.T7"></a>`FLR.S.G.IDLE.T7` | 나머지 | c7 ← c7 + 1 · fan IDLE prev none | 6.3.4.3.4 DEV-FLR-T7-VALUE R-FLR-NO-T103 |
| <a id="FLR.S.G.TAKEN.T20"></a>`FLR.S.G.TAKEN.T20` | c20 < Cap('C20') | c20 ← c20 + 1 · partA speaker GRANTED sprio Ms.T2 / 1000 seq | 6.3.4.4.9 |
| <a id="FLR.S.G.TAKEN.T20_exhaust"></a>`FLR.S.G.TAKEN.T20_exhaust` | 나머지 | c20 ← c20 + 1 | 6.3.4.4.10 |

<a id="FLR.S.gate"></a>
#### 10-2-8. 서버 · `gate`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.S.gate.ack_drop"></a>`FLR.S.gate.ack_drop` | m.t == 'ACK' && part[u] != 'NP_TAKEN' | — | x:6.3.5.3.5 DEV-FLR-ACK-DROP R-FLR-ACK-NO-REPLY |
| <a id="FLR.S.gate.not_in_room"></a>`FLR.S.gate.not_in_room` | !(u in members) && m.t == 'REQUEST' | send u DENY Deny.OTHER_REASON | DEV-FLR-DENY-255 |
| <a id="FLR.S.gate.not_in_room_drop"></a>`FLR.S.gate.not_in_room_drop` | !(u in members) | — | x:6.3.5.8.2 DEV-FLR-DENY-255 |
| <a id="FLR.S.gate.recv_only"></a>`FLR.S.gate.recv_only` | m.t == 'REQUEST' && !half[u] | send u DENY Deny.RECV_ONLY | x:6.3.4.3.3 DEV-FLR-RECVONLY-TRIGGER R-FLR-DENY-RECVONLY |
| <a id="FLR.S.gate.not_pubroom"></a>`FLR.S.gate.not_pubroom` | m.t == 'REQUEST' && !pub[u] | send u DENY Deny.NOT_PUBROOM | DEV-FLR-DENY-101 DEV-FLR-PUBROOM R-FLR-PUBROOM-ORDER |
| <a id="FLR.S.gate.no_permission"></a>`FLR.S.gate.no_permission` | m.t == 'REQUEST' && !perm[u] | send u DENY Deny.NO_PERMISSION | DEV-FLR-DENY-102 DEV-FLR-PERM-REVOKE |

<a id=""></a>
#### 10-2-9. 서버

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| `` | 나머지 | call PartC |  |
| `` | 나머지 | rcause[u] ← Revoke.NO_PERMISSION · send u REVOKE Revoke.NO_PERMISSION NPSENDS |  |

<a id="FLR.S.P"></a>
#### 10-2-10. 서버 · `P`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.S.P.join.self"></a>`FLR.S.P.join.self` | 받는 효과 SUB floor.ready · u == Holder() | — | DEV-FLR-FIRST-NOTIFY R-FLR-FIRST-NOTIFY |
| <a id="FLR.S.P.join.taken"></a>`FLR.S.P.join.taken` | 받는 효과 SUB floor.ready · Holder() != none | send u TAKEN Holder() | 6.3.5.2.2 DEV-FLR-FIRST-NOTIFY R-FLR-FIRST-NOTIFY |
| <a id="FLR.S.P.join.idle"></a>`FLR.S.P.join.idle` | 받는 효과 SUB floor.ready · 나머지 | send u IDLE prev | 6.3.5.2.2 DEV-FLR-FIRST-NOTIFY R-FLR-FIRST-NOTIFY |
| <a id="FLR.S.P.any.a_QINFO"></a>`FLR.S.P.any.a_QINFO` | ev.t == 'QINFO' | send u QINFO ev.n ev.d | 6.3.5.4.7 |
| <a id="FLR.S.P.any.a_DENY"></a>`FLR.S.P.any.a_DENY` | ev.t == 'DENY' | send u DENY ev.n | 6.3.5.3.6 |
| <a id="FLR.S.P.NP_IDLE.a_GRANTED"></a>`FLR.S.P.NP_IDLE.a_GRANTED` | part[u] == 'NP_IDLE' && ev.t == 'GRANTED' | send u GRANTED ev.n ev.d ev.q PERMITTED | 6.3.5.3.5 |
| <a id="FLR.S.P.NP_IDLE.a_TAKEN"></a>`FLR.S.P.NP_IDLE.a_TAKEN` | part[u] == 'NP_IDLE' && ev.t == 'TAKEN' | tq[u] ← ev.q · send u TAKEN ev.a ev.q NP_TAKEN | 6.3.5.3.3 |
| <a id="FLR.S.P.NP_IDLE.a_IDLE"></a>`FLR.S.P.NP_IDLE.a_IDLE` | part[u] == 'NP_IDLE' && ev.t == 'IDLE' | send u IDLE ev.a ev.q | 6.3.5.3.10 |
| <a id="FLR.S.P.NP_IDLE.a_other"></a>`FLR.S.P.NP_IDLE.a_other` | part[u] == 'NP_IDLE' | — | 6.3.5.1 |
| <a id="FLR.S.P.NP_TAKEN.a_GRANTED"></a>`FLR.S.P.NP_TAKEN.a_GRANTED` | part[u] == 'NP_TAKEN' && ev.t == 'GRANTED' | send u GRANTED ev.n ev.d ev.q PERMITTED | 6.3.5.4.9 |
| <a id="FLR.S.P.NP_TAKEN.a_TAKEN"></a>`FLR.S.P.NP_TAKEN.a_TAKEN` | part[u] == 'NP_TAKEN' && ev.t == 'TAKEN' | tq[u] ← ev.q; c21[u] ← 1 · send u TAKEN ev.a ev.q | 6.3.5.4.10 DEV-FLR-TAKEN-ACK |
| <a id="FLR.S.P.NP_TAKEN.a_IDLE"></a>`FLR.S.P.NP_TAKEN.a_IDLE` | part[u] == 'NP_TAKEN' && ev.t == 'IDLE' | send u IDLE ev.a ev.q NP_IDLE | 6.3.5.4.3 |
| <a id="FLR.S.P.NP_TAKEN.a_other"></a>`FLR.S.P.NP_TAKEN.a_other` | part[u] == 'NP_TAKEN' | — | 6.3.5.1 |
| <a id="FLR.S.P.PERMITTED.a_GRANTED"></a>`FLR.S.P.PERMITTED.a_GRANTED` | part[u] == 'PERMITTED' && ev.t == 'GRANTED' | send u GRANTED ev.n ev.d ev.q | x:6.3.4.4.8 x:6.3.4.4.9 DEV-FLR-REGRANT-FWD R-FLR-REGRANT |
| <a id="FLR.S.P.PERMITTED.a_IDLE"></a>`FLR.S.P.PERMITTED.a_IDLE` | part[u] == 'PERMITTED' && ev.t == 'IDLE' | enter u NP_IDLE | 6.3.5.5.4 |
| <a id="FLR.S.P.PERMITTED.a_TAKEN"></a>`FLR.S.P.PERMITTED.a_TAKEN` | part[u] == 'PERMITTED' && ev.t == 'TAKEN' | tq[u] ← ev.q · send u TAKEN ev.a ev.q NP_TAKEN | 6.3.5.5.9 |
| <a id="FLR.S.P.PERMITTED.a_REVOKE"></a>`FLR.S.P.PERMITTED.a_REVOKE` | part[u] == 'PERMITTED' && ev.t == 'REVOKE' | rcause[u] ← ev.n · send u REVOKE ev.n PREVOKE | 6.3.5.5.5 R-FLR-T3-GRACE |
| <a id="FLR.S.P.PERMITTED.a_other"></a>`FLR.S.P.PERMITTED.a_other` | part[u] == 'PERMITTED' | — | 6.3.5.1 |
| <a id="FLR.S.P.PREVOKE.a_IDLE"></a>`FLR.S.P.PREVOKE.a_IDLE` | part[u] == 'PREVOKE' && ev.t == 'IDLE' | send u IDLE ev.a ev.q NP_IDLE | 6.3.5.6.6 |
| <a id="FLR.S.P.PREVOKE.a_TAKEN"></a>`FLR.S.P.PREVOKE.a_TAKEN` | part[u] == 'PREVOKE' && ev.t == 'TAKEN' | tq[u] ← ev.q · send u TAKEN ev.a ev.q NP_TAKEN | 6.3.5.6.7 |
| <a id="FLR.S.P.PREVOKE.a_other"></a>`FLR.S.P.PREVOKE.a_other` | part[u] == 'PREVOKE' | — | 6.3.5.1 |
| <a id="FLR.S.P.RELEASING.a_any"></a>`FLR.S.P.RELEASING.a_any` | part[u] == 'RELEASING' | — | 6.3.5.8.2 |
| <a id="FLR.S.P.NPSENDS.a_IDLE"></a>`FLR.S.P.NPSENDS.a_IDLE` | ev.t == 'IDLE' | gTaken[u] ← false | 6.3.5.7.1 |
| <a id="FLR.S.P.NPSENDS.a_TAKEN"></a>`FLR.S.P.NPSENDS.a_TAKEN` | ev.t == 'TAKEN' | gTaken[u] ← true | 6.3.5.7.5 |
| <a id="FLR.S.P.NPSENDS.a_other"></a>`FLR.S.P.NPSENDS.a_other` | 나머지 | — | 6.3.5.1 |
| <a id="FLR.S.P.NP_IDLE.rx_REQUEST"></a>`FLR.S.P.NP_IDLE.rx_REQUEST` | part[u] == 'NP_IDLE' && ev.t == 'REQUEST' | call ArbReqIdle | 6.3.5.3.4 |
| <a id="FLR.S.P.NP_IDLE.rx_RELEASE"></a>`FLR.S.P.NP_IDLE.rx_RELEASE` | part[u] == 'NP_IDLE' && ev.t == 'RELEASE' | dequeue u · send u IDLE prev | 6.3.5.3.7 |
| <a id="FLR.S.P.NP_IDLE.rx_RTP_after_release"></a>`FLR.S.P.NP_IDLE.rx_RTP_after_release` | part[u] == 'NP_IDLE' && ev.t == 'RTP' && released[u] | call Revoke3 | 6.3.5.3.8 |
| <a id="FLR.S.P.NP_IDLE.rx_RTP_drop"></a>`FLR.S.P.NP_IDLE.rx_RTP_drop` | part[u] == 'NP_IDLE' && ev.t == 'RTP' | — | 6.3.5.3.8 6.3.5.1 |
| <a id="FLR.S.P.NP_IDLE.rx_other"></a>`FLR.S.P.NP_IDLE.rx_other` | part[u] == 'NP_IDLE' | — | 6.3.5.1 |
| <a id="FLR.S.P.NP_TAKEN.rx_REQUEST"></a>`FLR.S.P.NP_TAKEN.rx_REQUEST` | part[u] == 'NP_TAKEN' && ev.t == 'REQUEST' | call TakenReq | 6.3.5.4.4 |
| <a id="FLR.S.P.NP_TAKEN.rx_RELEASE"></a>`FLR.S.P.NP_TAKEN.rx_RELEASE` | part[u] == 'NP_TAKEN' && ev.t == 'RELEASE' | tq[u] ← seq + 1 == SeqWrap ? 0 : seq + 1; c21[u] ← 1 · dequeue u · send u TAKEN Holder() | 6.3.5.4.5 DEV-FLR-TAKEN-ACK DEV-FLR-T21-SAME-SEQ |
| <a id="FLR.S.P.NP_TAKEN.rx_ACK_taken"></a>`FLR.S.P.NP_TAKEN.rx_ACK_taken` | part[u] == 'NP_TAKEN' && ev.t == 'ACK' && ev.n == 2 && Same(ev.q, tq[u]) | c21[u] ← 0 · — | x:6.3.5.3.3 x:8.2.3.14 DEV-FLR-TAKEN-ACK DEV-FLR-ACK-SEQ |
| <a id="FLR.S.P.NP_TAKEN.rx_ACK_taken_stale"></a>`FLR.S.P.NP_TAKEN.rx_ACK_taken_stale` | part[u] == 'NP_TAKEN' && ev.t == 'ACK' && ev.n == 2 | — | x:6.3.5.3.3 x:8.2.3.14 DEV-FLR-ACK-SEQ |
| <a id="FLR.S.P.NP_TAKEN.rx_ACK_other"></a>`FLR.S.P.NP_TAKEN.rx_ACK_other` | part[u] == 'NP_TAKEN' && ev.t == 'ACK' | — | 6.3.5.1 DEV-FLR-ACK-DROP |
| <a id="FLR.S.P.NP_TAKEN.rx_QPOS_REQ"></a>`FLR.S.P.NP_TAKEN.rx_QPOS_REQ` | part[u] == 'NP_TAKEN' && ev.t == 'QPOS_REQ' | send u QINFO Pos(u) QPrio(u) | 6.3.5.4.7 x:8.2.3.5 |
| <a id="FLR.S.P.NP_TAKEN.rx_RTP"></a>`FLR.S.P.NP_TAKEN.rx_RTP` | part[u] == 'NP_TAKEN' && ev.t == 'RTP' | call Revoke3 | 6.3.5.4.6 |
| <a id="FLR.S.P.NP_TAKEN.rx_other"></a>`FLR.S.P.NP_TAKEN.rx_other` | part[u] == 'NP_TAKEN' | — | 6.3.5.1 |
| <a id="FLR.S.P.PERMITTED.rx_RELEASE"></a>`FLR.S.P.PERMITTED.rx_RELEASE` | part[u] == 'PERMITTED' && ev.t == 'RELEASE' | released[u] ← true · call ArbRel | 6.3.5.5.3 |
| <a id="FLR.S.P.PERMITTED.rx_REQUEST"></a>`FLR.S.P.PERMITTED.rx_REQUEST` | part[u] == 'PERMITTED' && ev.t == 'REQUEST' | call ArbReqPermitted | 6.3.5.5.7 |
| <a id="FLR.S.P.PERMITTED.rx_RTP"></a>`FLR.S.P.PERMITTED.rx_RTP` | part[u] == 'PERMITTED' && ev.t == 'RTP' | call ArbRtp | 6.3.5.5.6 |
| <a id="FLR.S.P.PERMITTED.rx_other"></a>`FLR.S.P.PERMITTED.rx_other` | part[u] == 'PERMITTED' | — | 6.3.5.1 |
| <a id="FLR.S.P.PREVOKE.rx_RELEASE"></a>`FLR.S.P.PREVOKE.rx_RELEASE` | part[u] == 'PREVOKE' && ev.t == 'RELEASE' | call ArbRel | 6.3.5.6.5 R-FLR-REVOKE-RELEASE |
| <a id="FLR.S.P.PREVOKE.rx_RTP"></a>`FLR.S.P.PREVOKE.rx_RTP` | part[u] == 'PREVOKE' && ev.t == 'RTP' | call ArbRtp | 6.3.5.6.4 R-FLR-T3-GRACE |
| <a id="FLR.S.P.PREVOKE.rx_other"></a>`FLR.S.P.PREVOKE.rx_other` | part[u] == 'PREVOKE' | — | 6.3.5.1 |
| <a id="FLR.S.P.NPSENDS.rx_RELEASE_taken"></a>`FLR.S.P.NPSENDS.rx_RELEASE_taken` | ev.t == 'RELEASE' && gTaken[u] | tq[u] ← seq + 1 == SeqWrap ? 0 : seq + 1 · send u TAKEN Holder() NP_TAKEN | 6.3.5.7.4 6.3.5.7.5 DEV-FLR-T21-SAME-SEQ |
| <a id="FLR.S.P.NPSENDS.rx_RELEASE_idle"></a>`FLR.S.P.NPSENDS.rx_RELEASE_idle` | ev.t == 'RELEASE' | send u IDLE prev NP_IDLE | 6.3.5.7.4 |
| <a id="FLR.S.P.NPSENDS.rx_RTP"></a>`FLR.S.P.NPSENDS.rx_RTP` | ev.t == 'RTP' | — | 6.3.5.7.2 |
| <a id="FLR.S.P.NPSENDS.rx_other"></a>`FLR.S.P.NPSENDS.rx_other` | 나머지 | — | 6.3.5.1 |
| <a id="FLR.S.P.NP_IDLE.enter"></a>`FLR.S.P.NP_IDLE.enter` | p == 'NP_IDLE' | part[u] ← p; gTaken[u] ← false; c21[u] ← 0 | 6.3.5.3.2 |
| <a id="FLR.S.P.NP_TAKEN.enter"></a>`FLR.S.P.NP_TAKEN.enter` | p == 'NP_TAKEN' | part[u] ← p; gTaken[u] ← true; c21[u] ← 1 | 6.3.5.4.2 DEV-FLR-TAKEN-ACK |
| <a id="FLR.S.P.PERMITTED.enter"></a>`FLR.S.P.PERMITTED.enter` | p == 'PERMITTED' | part[u] ← p; released[u] ← false; c21[u] ← 0 | 6.3.5.5.2 |
| <a id="FLR.S.P.PREVOKE.enter"></a>`FLR.S.P.PREVOKE.enter` | p == 'PREVOKE' | part[u] ← p; c8[u] ← 1; c21[u] ← 0 | 6.3.5.6.2 R-FLR-T3-GRACE |
| <a id="FLR.S.P.NPSENDS.enter"></a>`FLR.S.P.NPSENDS.enter` | 나머지 | part[u] ← p; c8[u] ← 1; c21[u] ← 0 | 6.3.5.7.2 |
| <a id="FLR.S.P.NP_TAKEN.req_retry_after"></a>`FLR.S.P.NP_TAKEN.req_retry_after` | retry[u] | send u DENY Deny.RETRY_AFTER | 6.3.5.4.4 DEV-FLR-T9 R-FLR-T9 |
| <a id="FLR.S.P.NP_TAKEN.req_renotify"></a>`FLR.S.P.NP_TAKEN.req_renotify` | InQ(u) | send u QINFO Pos(u) QPrio(u) | 6.3.5.4.4 |
| <a id="FLR.S.P.NP_TAKEN.req_preempt"></a>`FLR.S.P.NP_TAKEN.req_preempt` | p > HolderPrio() && !PreQ() | call ArbPreempt | 6.3.5.4.4 DEV-FLR-PREEMPT-RELATIVE R-FLR-PREEMPT-OPEN |
| <a id="FLR.S.P.NP_TAKEN.req_preempt_blocked"></a>`FLR.S.P.NP_TAKEN.req_preempt_blocked` | p > HolderPrio() | — | 6.3.5.4.4 6.3.5.1 DEV-FLR-PREEMPT-RELATIVE |
| <a id="FLR.S.P.NP_TAKEN.req_full"></a>`FLR.S.P.NP_TAKEN.req_full` | len(queue) >= QMax | send u DENY Deny.QUEUE_FULL | 6.3.5.4.4 DEV-FLR-QUEUE-MAX R-FLR-CAUSE-SPLIT |
| <a id="FLR.S.P.NP_TAKEN.req_enqueue"></a>`FLR.S.P.NP_TAKEN.req_enqueue` | 나머지 | queue ← insert(queue, {u: u, p: p, pre: false}) · send u QINFO Pos(u) p | 6.3.5.4.4 R-FLR-QUEUE-ALWAYS |
| <a id="FLR.S.P.PREVOKE.T8"></a>`FLR.S.P.PREVOKE.T8` | part[u] == 'PREVOKE' | c8[u] ← c8[u] + 1 · send u REVOKE rcause[u] | 6.3.5.6.3 |
| <a id="FLR.S.P.NPSENDS.T8"></a>`FLR.S.P.NPSENDS.T8` | 나머지 | c8[u] ← c8[u] + 1 · send u REVOKE Revoke.NO_PERMISSION | 6.3.5.7.3 |
| <a id="FLR.S.P.NP_TAKEN.T21"></a>`FLR.S.P.NP_TAKEN.T21` | c21[u] < Cap('C21') | c21[u] ← c21[u] + 1 · send u TAKEN Holder() tq[u] | DEV-FLR-TAKEN-ACK R-FLR-TAKEN-ACK |
| <a id="FLR.S.P.NP_TAKEN.T21_exhaust"></a>`FLR.S.P.NP_TAKEN.T21_exhaust` | 나머지 | c21[u] ← c21[u] + 1 | DEV-FLR-TAKEN-ACK |

<a id="FLR.S.perm"></a>
#### 10-2-11. 서버 · `perm`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.S.perm.speaker"></a>`FLR.S.perm.speaker` | 받는 효과 ROOM floor.perm_down · g == 'TAKEN' && speaker == u | perm[u] ← false · call PrevokeEnter | x:6.3.4.4.4 DEV-FLR-PERM-REVOKE R-FLR-PERM-AXIS |
| <a id="FLR.S.perm.queued"></a>`FLR.S.perm.queued` | 받는 효과 ROOM floor.perm_down · InQ(u) | perm[u] ← false · dequeue u · partA u DENY Deny.NO_PERMISSION | DEV-FLR-PERM-REVOKE |
| <a id="FLR.S.perm.other"></a>`FLR.S.perm.other` | 받는 효과 ROOM floor.perm_down · 나머지 | perm[u] ← false | DEV-FLR-PERM-REVOKE |
| <a id="FLR.S.perm.up"></a>`FLR.S.perm.up` | 받는 효과 ROOM floor.perm_up · 나머지 | perm[u] ← true | DEV-FLR-PERM-REVOKE R-FLR-PERM-AXIS |

<a id="FLR.S.away"></a>
#### 10-2-12. 서버 · `away`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.S.away.queued"></a>`FLR.S.away.queued` | InQ(u) | dequeue u | DEV-FLR-PUB-AWAY |
| <a id="FLR.S.away.other"></a>`FLR.S.away.other` | 나머지 | — | DEV-FLR-PUB-AWAY |

<a id="FLR.S.ws"></a>
#### 10-2-13. 서버 · `ws`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.S.ws.pub_select"></a>`FLR.S.ws.pub_select` | 받는 효과 AFF floor.pub_select · 나머지 | pub[u] ← true | DEV-FLR-PUBROOM R-FLR-PUBROOM-ORDER |
| <a id="FLR.S.ws.reg_half"></a>`FLR.S.ws.reg_half` | 나머지 | half[u] ← true | DEV-FLR-RECVONLY-TRIGGER R-FLR-DENY-RECVONLY |
| <a id="FLR.S.ws.pub_away"></a>`FLR.S.ws.pub_away` | 받는 효과 AFF floor.pub_away · 나머지 | pub[u] ← false · call Away | DEV-FLR-PUB-AWAY |
| <a id="FLR.S.ws.leave"></a>`FLR.S.ws.leave` | 받는 효과 ROOM floor.leave · 나머지 | pub[u] ← false · call Away · members ← without(members, u); retry[u] ← false; part[u] ← 'RELEASING' | x:6.3.5.8.2 DEV-FLR-PUB-AWAY |
| <a id="FLR.S.ws.join_revoking"></a>`FLR.S.ws.join_revoking` | 받는 효과 ROOM floor.join · g == 'PREVOKE' && revoking == u | prev ← revoking · call IdleEnter · members ← adding(members, u); part[u] ← Holder() != none ? 'NP_TAKEN' : 'NP_IDLE'; gTaken[u] ← Holder() != none; pub[u] ← false; half[u] ← half; perm[u] ← perm; retry[u] ← false; released[u] ← false; tq[u] ← 0; rcause[u] ← 0 | 6.3.4.5.5 6.3.5.2.2 |
| <a id="FLR.S.ws.join"></a>`FLR.S.ws.join` | 받는 효과 ROOM floor.join · 나머지 | members ← adding(members, u); part[u] ← Holder() != none ? 'NP_TAKEN' : 'NP_IDLE'; gTaken[u] ← Holder() != none; pub[u] ← false; half[u] ← half; perm[u] ← perm; retry[u] ← false; released[u] ← false; tq[u] ← 0; rcause[u] ← 0 | 6.3.5.2.2 |

<a id="FLR.S.T9"></a>
#### 10-2-14. 서버 · `T9`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="FLR.S.T9.expire"></a>`FLR.S.T9.expire` | 나머지 | retry[u] ← false | DEV-FLR-T9 R-FLR-T9 |

#### 10-2-15. 타이머 선언

| 이름 | 주인 | 도는 동안 | 만료 | 뜻 |
|---|---|---|---|---|
| `T101` | client | `cs == 'PREQ'` | `CT101` | 요청 재전송 |
| `T100` | client | `cs == 'PREL'` | `CT100` | 반환 재전송 |
| `T104` | client | `cs == 'QUEUED' && sub == 'QUERY'` | `CT104` | 순번 질의 재전송 |
| `T132` | client | `cs == 'QUEUED' && sub == 'GRANTED'` | `CT132` | 승계 의지 대기 |
| `T8` | participant | `(part[u] == 'PREVOKE' \|\| part[u] == 'NPSENDS') && c8[u] < Cap('C8')` | `SrvT8` | REVOKE 재전송 |
| `T9` | participant | `retry[u]` | `SrvT9` | Retry-after |
| `T21` | participant | `part[u] == 'NP_TAKEN' && c21[u] >= 1 && c21[u] <= Cap('C21')` | `SrvT21` | TAKEN 재전송(ACK 없음) |
| `T1` | server | `g == 'TAKEN' \|\| g == 'PREVOKE'` | `SrvT1` | RTP 없음 |
| `T2` | server | `g == 'TAKEN' && firstRtp` | `SrvT2` | 발화 상한 |
| `T3` | server | `g == 'PREVOKE'` | `SrvT3` | 회수 유예 |
| `T7` | server | `g == 'IDLE' && idleBcast && c7 < Cap('C7')` | `SrvT7` | IDLE 재전송 |
| `T20` | server | `g == 'TAKEN' && succ && !rtpSeen && c20 <= Cap('C20')` | `SrvT20` | 승계 GRANTED 재전송 |

### 10-3. 흐름 · 검증
검증 모형 `out/floor/Floor.tla` · 하네스 `verify/floor/harness.tla`

#### 10-3-1. 흐름 F3 — 선점
선점은 세 걸음이다 — 화자에게 회수, 선점자는 큐 맨 앞, 화자가 놓으면(또는 T3 가 지나면) 승계. 회수와 승계 사이에 회수 유예(T3)가 있다.
모형 검사: 걸음 18 도달 · 갈래 144

1. **A press** — A 가 누른다. 클라는 REQUEST 를 보내고 요청 중(pending Request)으로 간다. T101 이 돈다. ★마이크는 아직 열지 않는다 — 허가(GRANTED)가 와야 연다.
2. **A up REQUEST** — 서버가 REQUEST 를 받는다. 판정 로직이 비어 있으므로 A 에게 허가하고, 나머지에게 TAKEN 을 방송한다. ★허가와 방송은 판정 로직이 직접 보내지 않고 참가자 상태기를 거친다 — 참가자 상태가 보낼지를 정한다.
3. **A down GRANTED** — A 가 GRANTED 를 받는다. 말하는 중(has permission)으로 가고 마이크를 연다. A 비트가 섰으므로 ACK 을 보낸다.
4. **A up ACK** — 서버는 ACK 을 받고 아무것도 하지 않는다.
5. **B down TAKEN** — B 가 TAKEN 을 받는다. 화자 표시를 A 로 바꾸고, A 비트가 섰으므로 ACK 을 보낸다. 상태는 그대로(has no permission)다.
6. **B up ACK** — 서버의 B 참가자가 ACK(TAKEN)을 받고 T21 을 멈춘다. 답하지는 않는다. ★TAKEN 이 사라지면 이 ACK 이 오지 않아 서버가 T21 마다 TAKEN 을 다시 보낸다 — 듣는 사람 화면의 화자 표시를 지키는 장치다.
7. **A rtp** — A 의 목소리(RTP)가 서버에 닿는다. 서버는 이때부터 T2(발화 상한)를 센다.
8. **B press** — B 가 우선순위 1 을 실어 누른다. 클라는 자기 요청이 선점인지 모른다 — 판정은 서버가 한다.
9. **B up REQUEST** — 서버가 1 > 0(화자의 허가된 우선순위)을 보고 선점한다. A 에게 REVOKE(사유 4), B 는 큐 맨 앞에 넣고 QINFO 순번 1. 판정 로직은 회수 유예(pending Floor Revoke)로 가고 T3 가, A 참가자에는 T8 이 돈다. ★이 순간 B 에게 허가하지 않는다. A 가 놓기 전까지 채널은 아직 A 의 것이다.
10. **A down REVOKE** — A 가 REVOKE 를 받는다. 마이크를 닫고 RELEASE 를 보내고 반환 중으로 간다. ★회수 동안에도 서버는 A 의 목소리를 계속 흘린다. 회수가 닿았다는 확인은 A 의 RELEASE 다.
11. **B down QINFO** — B 가 QINFO(순번 1)를 받고 대기 중으로 간다.
12. **A up RELEASE** — 서버가 A 의 RELEASE 를 받는다. 회수 유예를 끝내고(T3 · T8 멈춤) 큐 맨 앞의 B 에게 곧바로 승계한다.
13. **B down GRANTED** — B 가 승계 GRANTED 를 받는다. F2 와 같이 의지 표시 창(T132)이 돈다.
14. **B up ACK** — 서버는 ACK 에 답하지 않는다.
15. **B intent** — B 가 의지를 보이고 말하는 중으로 간다.
16. **A down TAKEN** — A 가 TAKEN 을 받아 반환 확인을 얻고 말할 수 없음으로 간다.
17. **A up ACK** — A 의 ACK(TAKEN)이 서버에 닿아 A 참가자의 T21 이 멈춘다.
18. **B rtp** — B 의 목소리가 서버에 닿는다. ★선점자도 같은 상대 비교로 다시 선점될 수 있다 — 우선순위 2 가 오면 B 가 회수된다.

#### 10-3-2. 흐름 F2 — 줄 서서 이어받기
큐를 항상 켜는 우리 규격의 기본 경로다. 점유 중의 요청은 거절이 아니라 대기이고, 승계에는 IDLE 이 끼지 않으며, 승계 허가 뒤에 의지 표시 창(T132)이 있다.
모형 검사: 걸음 18 도달 · 갈래 149

1. **A press** — A 가 누른다. 클라는 REQUEST 를 보내고 요청 중(pending Request)으로 간다. T101 이 돈다. ★마이크는 아직 열지 않는다 — 허가(GRANTED)가 와야 연다.
2. **A up REQUEST** — 서버가 REQUEST 를 받는다. 판정 로직이 비어 있으므로 A 에게 허가하고, 나머지에게 TAKEN 을 방송한다. ★허가와 방송은 판정 로직이 직접 보내지 않고 참가자 상태기를 거친다 — 참가자 상태가 보낼지를 정한다.
3. **A down GRANTED** — A 가 GRANTED 를 받는다. 말하는 중(has permission)으로 가고 마이크를 연다. A 비트가 섰으므로 ACK 을 보낸다.
4. **A up ACK** — 서버는 ACK 을 받고 아무것도 하지 않는다.
5. **B down TAKEN** — B 가 TAKEN 을 받는다. 화자 표시를 A 로 바꾸고, A 비트가 섰으므로 ACK 을 보낸다. 상태는 그대로(has no permission)다.
6. **B up ACK** — 서버의 B 참가자가 ACK(TAKEN)을 받고 T21 을 멈춘다. 답하지는 않는다. ★TAKEN 이 사라지면 이 ACK 이 오지 않아 서버가 T21 마다 TAKEN 을 다시 보낸다 — 듣는 사람 화면의 화자 표시를 지키는 장치다.
7. **A rtp** — A 의 목소리(RTP)가 서버에 닿는다. 서버는 이때부터 T2(발화 상한)를 센다.
8. **B press** — A 가 말하는 중에 B 가 누른다. B 도 REQUEST 를 보내고 요청 중(pending Request)으로 간다.
9. **B up REQUEST** — 서버의 B 참가자(Floor Taken)가 요청을 대기줄에 넣고 순번을 QINFO 로 알린다. ★점유 중의 요청에 거절 사유 1(남이 말하는 중)을 내지 않는다 — 늘 줄을 세운다.
10. **B down QINFO** — B 가 QINFO 를 받고 대기 중(queued)으로 간다. 화면에 순번을 보인다.
11. **A release** — A 가 뗀다. RELEASE 를 보내고 반환 중(pending Release)으로 간다.
12. **A up RELEASE** — 서버가 RELEASE 를 받는다. 대기줄이 있으므로 Idle 에 머물지 않고 곧바로 B 에게 승계한다 — B 에게 GRANTED, A 에게 TAKEN. T20 이 돈다. ★승계 사이에 IDLE 을 보내지 않는다. 보내면 화면이 "아무도 안 말함"으로 깜빡인다.
13. **B down GRANTED** — B 가 GRANTED 를 받는다. 아직 말하는 중이 아니다 — 대기 중 안에서 승계 허가를 받은 상태로 가고, 의지 표시 창 T132 가 돈다. ACK 을 보낸다. ★큐 승계 허가에는 마이크를 바로 열지 않는다. 사용자가 기기 앞에 있는지 모르기 때문이다.
14. **B up ACK** — 서버는 ACK 에 답하지 않는다.
15. **B intent** — B 가 의지를 보인다(hold 면 버튼이 눌린 채 · toggle 이면 다시 누름). 말하는 중(has permission)으로 가고 마이크를 연다.
16. **A down TAKEN** — A 가 TAKEN 을 받는다. 이것이 A 의 반환 확인이다 — 말할 수 없음으로 가고 T100 을 멈춘다. ★F1 과 달리 직전 화자는 IDLE 이 아니라 승계의 TAKEN 으로 반환 확인을 받는다.
17. **A up ACK** — A 의 ACK(TAKEN)이 서버에 닿아 A 참가자의 T21 이 멈춘다.
18. **B rtp** — B 의 목소리가 서버에 닿는다. 서버는 T20(승계 GRANTED 재전송)을 멈추고 T2 를 센다.

#### 10-3-3. 흐름 F1 — 빈 방에서 말하고 놓기
발언권의 줄기다. 다른 흐름은 모두 이 흐름의 어느 걸음에서 갈라진다.
모형 검사: 걸음 13 도달 · 갈래 108

1. **A press** — A 가 누른다. 클라는 REQUEST 를 보내고 요청 중(pending Request)으로 간다. T101 이 돈다. ★마이크는 아직 열지 않는다 — 허가(GRANTED)가 와야 연다.
2. **A up REQUEST** — 서버가 REQUEST 를 받는다. 판정 로직이 비어 있으므로 A 에게 허가하고, 나머지에게 TAKEN 을 방송한다. ★허가와 방송은 판정 로직이 직접 보내지 않고 참가자 상태기를 거친다 — 참가자 상태가 보낼지를 정한다.
3. **A down GRANTED** — A 가 GRANTED 를 받는다. 말하는 중(has permission)으로 가고 마이크를 연다. A 비트가 섰으므로 ACK 을 보낸다.
4. **A up ACK** — 서버는 ACK 을 받고 아무것도 하지 않는다.
5. **B down TAKEN** — B 가 TAKEN 을 받는다. 화자 표시를 A 로 바꾸고, A 비트가 섰으므로 ACK 을 보낸다. 상태는 그대로(has no permission)다.
6. **B up ACK** — 서버의 B 참가자가 ACK(TAKEN)을 받고 T21 을 멈춘다. 답하지는 않는다. ★TAKEN 이 사라지면 이 ACK 이 오지 않아 서버가 T21 마다 TAKEN 을 다시 보낸다 — 듣는 사람 화면의 화자 표시를 지키는 장치다.
7. **A rtp** — A 의 목소리(RTP)가 서버에 닿는다. 서버는 이때부터 T2(발화 상한)를 센다.
8. **A release** — A 가 뗀다. 마이크를 닫고 RELEASE 를 보내고 반환 중(pending Release)으로 간다. T100 이 돈다.
9. **A up RELEASE** — 서버가 RELEASE 를 받는다. 대기줄이 비었으므로 판정 로직은 Idle 로 가고 IDLE 을 방송한다. T7 이 돈다. ★방송은 A 에게 가지 않는다 — A 의 참가자(permitted)는 IDLE 을 보내지 않고 상태만 바꾼다(원문 6.3.5.5.4).
10. **B down IDLE** — B 가 IDLE 을 받는다. 화자 표시를 내린다.
11. **A T100** — A 는 IDLE 을 받지 못했으므로 T100 이 만료하고 RELEASE 를 다시 보낸다. ★직전 화자의 반환 확인은 늘 이 재전송으로 받는다. 첫 RELEASE 의 답은 오지 않는 것이 정상이다.
12. **A up RELEASE** — 서버의 A 참가자(Idle)가 다시 온 RELEASE 에 IDLE 을 A 에게만 보낸다.
13. **A down IDLE** — A 가 IDLE 을 받는다. 말할 수 없음(has no permission)으로 돌아가고 T100 을 멈춘다. 흐름이 처음으로 돌아왔다.

### 10-4. 주제

#### 10-4-1. 발언권 상태

<a id="RULE-FLR-CLIENT-STATE-NAMES"></a>**RULE-FLR-CLIENT-STATE-NAMES** · 규칙 — 발언권 상태

3GPP TS 24.380 의 이름을 그대로 쓴다 — `Start-stop` · `has no permission` · `pending Request` ·
`has permission` · `pending Release` · `queued`. 전이는 §10-4-30.

<a id="RULE-FLR-SERVER-STATE-SEPARATE"></a>**RULE-FLR-SERVER-STATE-SEPARATE** · 규칙

★**서버 쪽 상태는 따로다** — 전체 상태 `G: Floor Idle` · `G: Floor Taken` · `G: pending Floor Revoke` 와 참가자마다의 상태
(`U: permitted` · `U: not permitted and Floor Idle/Taken` · `U: pending Floor Revoke` · `U: not permitted but sends media`) — 원문 §6.3.4·§6.3.5.
★**클라는 서버 상태를 몰라도 된다** — 클라가 보는 것은 메시지뿐이다(§10-4-3).

#### 10-4-2. 상태를 앱에 알린다

<a id="RULE-FLR-APP-EXPOSE"></a>**RULE-FLR-APP-EXPOSE** · 규칙

| 무엇 | 값 |
|---|---|
| ★**발언권 상태** | ★**방마다** §10-4-1 의 여섯. ★**바뀔 때마다 알린다** — 버튼·화자 표시가 전부 이 값에 매달린다 |
| ★**발언 남은 시간** | ★**허가(`GRANTED`)에 실려 온 값**(§1-2-6). ★**표의 기본값이 아니다** |
| ★**회수 사유** | `FLOOR_REVOKE` 의 회수 사유(`2`, §10-4-31). ★**선점(`4`)과 상한 초과(`2`)는 사용자가 할 일이 반대다** · 자격 회수(`3`)는 다시 누를 일이 없다(§4-2-6) |

#### 10-4-3. 발언권 — 상태: §10-4-1 의 여섯 (DC 축)

<a id="RULE-FLR-PROC-DC-AXIS"></a>**RULE-FLR-PROC-DC-AXIS** · 규칙 — 발언권 — 상태: §10-4-1 의 여섯 (DC 축)

★**§4-2-3 과 축이 다르다.** WS 가 아니라 ★**DC**(§10)다. ★**방마다 따로 갖는다** — 다방이면 상태기도 방마다다(§10-4-35).
★**칸은 3GPP TS 24.380 §6.2.4 그대로다** — 각 항 머리에 원문 절을 적는다. 이 절은 원문을 옮긴 것이고 새 절차를 더하지 않는다.

#### 10-4-4. 어느 상태에서나

<a id="RULE-FLR-PROC-ANY-STATE"></a>**RULE-FLR-PROC-ANY-STATE** · 규칙 — 어느 상태에서나

1. ★**`0x1D`(방)로 어느 방 것인지 먼저 가른다**(§10-4-35). ★**없으면 처리하지 않는다.**
2. ★**ACK(`10`)은 `A` 가 선 메시지(`GRANTED`·`TAKEN`·`DENY`)를 ★절차가 있는 칸에서 받았을 때만 보낸다**(§10-4-30 · 원문은 절마다 ACK 단계를 둔다). 3 으로 버리는 칸에서는 ACK 도 없다.
3. ★★**이 절에 정해진 절차가 없는 상태에서 받은 메시지는 버리고 상태를 유지한다**(원문 6.2.4.1). 칸이 비어 있는 것이 아니라 ★**이 문장이 그 칸이다.**
4. ★**`queued` 의 `T104`·`T132` 는 그 상태에 딸린다** — `queued` 를 나가는 전이는 둘을 멈춘다(원문 6.2.4.9.1 *"can be running in this state"*).
5. 상태는 아래 항마다 다르다.

→ 구조 — [`FLR.wire`](#FLR.wire)

#### 10-4-5. 앱이 PTT 를 눌렀다 (`has no permission`) — 원문 6.2.4.3.5

<a id="RULE-FLR-PROC-PRESS"></a>**RULE-FLR-PROC-PRESS** · 규칙 — 앱이 PTT 를 눌렀다 (`has no permission`) — 원문 6.2.4.3.5

0. ★**그 방에 내 반이중 발행 트랙이 없으면 먼저 발행한다** — `PUBLISH_TRACKS{duplex:"half"}`
   (§7-1-2 · 절차는 §7-2-3 그대로: 트랜시버 → 협상 → 발행). ★청취 전용(`select:false`)으로 들어온
   방에서 처음 말할 때가 이 경로다. 없이 요청하면 `DENY(5)` 다(§10-4-30). 한 번 발행하면 ★**그 서버의 마지막 방을 나갈 때까지** 유지된다(§5-1-2).
1. ★**그 방이 `pub_room` 이 아니면 `AFFILIATION{pub_select}` 를 먼저 보내고 ★응답을 받는다**(§5-1-2).
   ★**서버는 `FLOOR_REQUEST` 를 `pub_room` 에서만 받는다**(§10-4-30). ★다른 방에서 `has permission`·`pending Request`·`pending Release`·`queued` 였으면
   이 `pub_select` 가 그 방 `RELEASE`·대기 철회와 같다(§5-1-2·§10-4-32) — 따로 반환하지 않는다. 클라는 응답 뒤 그 방을 ★**`has no permission` 으로 내리고 그 방 타이머를 멈춘다.**
2. ★**전환이었으면 1 의 응답 뒤다** — WS(게이트웨이 경유)와 DC(직행)는 다른 전송로라 순서가 보장되지 않고, DC 가 먼저 닿으면 `DENY(101)` 이다(§10-4-30).

→ 구조 — [`FLR.C.NOPERM.press`](#FLR.C.NOPERM.press) · [`FLR.wire`](#FLR.wire)

#### 10-4-6. 허가 전에 뗐다 (`pending Request`) — 원문 6.2.4.4.8

<a id="RULE-FLR-PROC-PREQ-RELEASE"></a>**RULE-FLR-PROC-PREQ-RELEASE** · 규칙 — 허가 전에 뗐다 (`pending Request`) — 원문 6.2.4.4.8

★**증상** — 이 전이가 없으면 → 탭했다 뗀 손짓마다 채널이 허가되고 `T1` 이 다 돌 때까지 잠긴다.

→ 구조 — [`FLR.C.PREQ.release`](#FLR.C.PREQ.release) · [`FLR.wire`](#FLR.wire)

#### 10-4-7. `T101` 이 만료했다 (`pending Request`) — 원문 6.2.4.4.5 · 6.2.4.4.6

<a id="RULE-FLR-PROC-PREQ-T101"></a>**RULE-FLR-PROC-PREQ-T101** · 규칙 — `T101` 이 만료했다 (`pending Request`) — 원문 6.2.4.4.5 · 6.2.4.4.6

→ 구조 — [`FLR.C.PREQ.T101`](#FLR.C.PREQ.T101) · [`FLR.C.PREQ.T101_exhaust`](#FLR.C.PREQ.T101_exhaust) · [`FLR.wire`](#FLR.wire)

#### 10-4-8. `2 FLOOR_TAKEN` 을 받았다 (`pending Request`) — 원문 6.2.4.4.11

<a id="RULE-FLR-PROC-PREQ-TAKEN"></a>**RULE-FLR-PROC-PREQ-TAKEN** · 규칙 — `2 FLOOR_TAKEN` 을 받았다 (`pending Request`) — 원문 6.2.4.4.11

3. ★**원문과 다르다** — 원문(6.2.4.4.11 7)은 선점 우선순위로 요청했을 때만 남고 아니면 `has no permission` 으로 간다. 우리 선점 판정은 상대 비교라(§10-4-32) 클라가 자기 요청이 선점인지 모른다 — 그래서 늘 남는다(§10-4-27 관문 목록).

→ 구조 — [`FLR.C.PREQ.rx_TAKEN`](#FLR.C.PREQ.rx_TAKEN) · [`FLR.C.PREQ.rx_TAKEN_stale`](#FLR.C.PREQ.rx_TAKEN_stale) · [`FLR.C.PREQ.T101`](#FLR.C.PREQ.T101)

#### 10-4-9. `1 FLOOR_GRANTED` 를 받았다 (`pending Request` · `queued`) — 원문 6.2.4.4.2 · 6.2.4.9.4

<a id="RULE-FLR-PROC-GRANTED"></a>**RULE-FLR-PROC-GRANTED** · 규칙 — `1 FLOOR_GRANTED` 를 받았다 (`pending Request` · `queued`) — 원문 6.2.4.4.2 · 6.2.4.9.4

1. ★**시간(`1`)·우선순위(`0`)를 받은 값으로 보관하고 앱에 알린다**(§1-2-3). ★**표의 기본값을 쓰지 않는다.**
3. 같은 `GRANTED` 가 다시 오면(서버 `T20` 재전송, §10-4-24) ACK 을 보내고 `T132` 를 다시 건다.

→ 구조 — [`FLR.C.PREQ.rx_GRANTED`](#FLR.C.PREQ.rx_GRANTED) · [`FLR.C.QUEUED.rx_GRANTED`](#FLR.C.QUEUED.rx_GRANTED) · [`FLR.C.QUEUED.rx_GRANTED_again`](#FLR.C.QUEUED.rx_GRANTED_again) · [`FLR.C.QUEUED.intent`](#FLR.C.QUEUED.intent)

#### 10-4-10. 사용자가 발언 의지를 표시했다 (`queued` · `T132` 도는 중) — 원문 6.2.4.9.12

<a id="RULE-FLR-PROC-QUEUED-ACCEPT"></a>**RULE-FLR-PROC-QUEUED-ACCEPT** · 규칙 — 사용자가 발언 의지를 표시했다 (`queued` · `T132` 도는 중) — 원문 6.2.4.9.12

1. ★**"의지 표시"는 입력 방식이 해석한다** — ★**누르고말하기(hold)면 버튼이 여전히 눌려
   있는 것 자체가 표시**라 `GRANTED` 도착 즉시 이 사건이 발화하고, ★**토글이면 사용자가
   다시 누를 때**다. ★**상태기와 wire 는 두 방식에서 동일하다** — 입력 방식은 앱·SDK 층이고 이 규격 밖이다.

→ 구조 — [`FLR.C.QUEUED.intent`](#FLR.C.QUEUED.intent) · [`FLR.wire`](#FLR.wire)

#### 10-4-11. `T132` 가 만료했다 (`queued`) — 원문 6.2.4.9.13

<a id="RULE-FLR-PROC-QUEUED-T132"></a>**RULE-FLR-PROC-QUEUED-T132** · 규칙 — `T132` 가 만료했다 (`queued`) — 원문 6.2.4.9.13

→ 구조 — [`FLR.C.QUEUED.T132`](#FLR.C.QUEUED.T132)

#### 10-4-12. 앱이 대기를 철회했다 (`queued` — 버튼을 뗐다/취소했다) — 원문 6.2.4.9.6

<a id="RULE-FLR-PROC-QUEUED-CANCEL"></a>**RULE-FLR-PROC-QUEUED-CANCEL** · 규칙 — 앱이 대기를 철회했다 (`queued` — 버튼을 뗐다/취소했다) — 원문 6.2.4.9.6

3. ★**서버가 그 사람에게 `TAKEN` 을 보내 철회를 확인한다**(§10-4-30 `RELEASE` — 원문 6.3.5.4.5) — §10-4-22 이 그것을 받아 `has no permission` 으로 내린다.

→ 구조 — [`FLR.C.QUEUED.release`](#FLR.C.QUEUED.release) · [`FLR.wire`](#FLR.wire)

#### 10-4-13. `1 FLOOR_GRANTED` 를 `has permission` 에서 받았다 — 원문 6.2.4.5.5

<a id="RULE-FLR-PROC-PERM-GRANTED"></a>**RULE-FLR-PROC-PERM-GRANTED** · 규칙 — `1 FLOOR_GRANTED` 를 `has permission` 에서 받았다 — 원문 6.2.4.5.5

2. 시간(`1`)·우선순위(`0`)는 받은 값으로 갱신한다.

→ 구조 — [`FLR.C.PERM.rx_GRANTED`](#FLR.C.PERM.rx_GRANTED) · [`FLR.wire`](#FLR.wire)

#### 10-4-14. 앱이 대기 순번을 물었다 (`queued` · `T132` 안 돎) — 원문 6.2.4.9.9 · 6.2.4.9.10

<a id="RULE-FLR-PROC-QUEUED-POSREQ"></a>**RULE-FLR-PROC-QUEUED-POSREQ** · 규칙 — 앱이 대기 순번을 물었다 (`queued` · `T132` 안 돎) — 원문 6.2.4.9.9 · 6.2.4.9.10

2. ★**방아쇠는 앱의 요청이다**(원문 *"indication from the MCPTT client"* — 사용자 동작 · 재동기 §9-2-1 5).

→ 구조 — [`FLR.C.QUEUED.qpos`](#FLR.C.QUEUED.qpos) · [`FLR.wire`](#FLR.wire) · [`FLR.C.QUEUED.T104`](#FLR.C.QUEUED.T104)

#### 10-4-15. `T104` 를 소진했다 (`queued`) — 원문 6.2.4.9.11

<a id="RULE-FLR-PROC-QUEUED-T104"></a>**RULE-FLR-PROC-QUEUED-T104** · 규칙 — `T104` 를 소진했다 (`queued`) — 원문 6.2.4.9.11

→ 구조 — [`FLR.C.QUEUED.T104`](#FLR.C.QUEUED.T104) · [`FLR.C.QUEUED.T104_exhaust`](#FLR.C.QUEUED.T104_exhaust) · [`FLR.wire`](#FLR.wire)

#### 10-4-16. `3 FLOOR_DENY` 를 받았다 (`pending Request` · `queued`) — 원문 6.2.4.4.4 · 6.2.4.9.5

<a id="RULE-FLR-PROC-DENY"></a>**RULE-FLR-PROC-DENY** · 규칙 — `3 FLOOR_DENY` 를 받았다 (`pending Request` · `queued`) — 원문 6.2.4.4.4 · 6.2.4.9.5

2. ★**거절 사유(`2`)를 앱에 알린다**(§10-4-31) — `7`(큐 만석)과 `4`(retry-after)는 ★**사용자가 할 일이 반대다.** ★`102`(자격 없음)는 다시 눌러도 같다 — `PARTICIPANT_STATE`(§4-2-6)가 같이 온다.

→ 구조 — [`FLR.C.PREQ.rx_DENY`](#FLR.C.PREQ.rx_DENY) · [`FLR.C.QUEUED.rx_DENY`](#FLR.C.QUEUED.rx_DENY)

#### 10-4-17. `9 FLOOR_QUEUE_INFO` 를 받았다 (`pending Request` · `queued`) — 원문 6.2.4.4.9 · 6.2.4.9.7

<a id="RULE-FLR-PROC-QINFO"></a>**RULE-FLR-PROC-QINFO** · 규칙 — `9 FLOOR_QUEUE_INFO` 를 받았다 (`pending Request` · `queued`) — 원문 6.2.4.4.9 · 6.2.4.9.7

1. ★**`3`(대기 정보)에서 순번과 우선순위를 함께 읽어** 앱에 알린다.

→ 구조 — [`FLR.C.PREQ.rx_QINFO`](#FLR.C.PREQ.rx_QINFO) · [`FLR.C.QUEUED.rx_QINFO`](#FLR.C.QUEUED.rx_QINFO) · [`FLR.C.QUEUED.rx_QINFO_answer`](#FLR.C.QUEUED.rx_QINFO_answer) · [`FLR.wire`](#FLR.wire)

#### 10-4-18. 앱이 PTT 를 뗐다 (`has permission`) — 원문 6.2.4.5.3

<a id="RULE-FLR-PROC-PERM-RELEASE"></a>**RULE-FLR-PROC-PERM-RELEASE** · 규칙 — 앱이 PTT 를 뗐다 (`has permission`) — 원문 6.2.4.5.3

3. ★**반환한 화자에게는 `IDLE` 방송이 가지 않는다**(원문 6.3.5.5.4) — 큐가 있으면 승계의 `TAKEN` 이 바로 오고, 없으면 `T100` 재전송한 `RELEASE` 에 서버가 `IDLE` 을 unicast 로 답한다(6.3.5.3.7). 확인이 `T100` 한 번만큼 늦는 것이 원문 그대로다.

→ 구조 — [`FLR.C.PERM.release`](#FLR.C.PERM.release) · [`FLR.C.PREL.rx_IDLE`](#FLR.C.PREL.rx_IDLE) · [`FLR.C.PREL.rx_TAKEN`](#FLR.C.PREL.rx_TAKEN) · [`FLR.C.PREL.T100`](#FLR.C.PREL.T100) · [`FLR.C.PREL.T100_exhaust`](#FLR.C.PREL.T100_exhaust) · [`FLR.C.PREL.rx_GRANTED`](#FLR.C.PREL.rx_GRANTED)

#### 10-4-19. `6 FLOOR_REVOKE` 를 받았다 (`has permission`) — 원문 6.2.4.5.4

<a id="RULE-FLR-PROC-REVOKE"></a>**RULE-FLR-PROC-REVOKE** · 규칙 — `6 FLOOR_REVOKE` 를 받았다 (`has permission`) — 원문 6.2.4.5.4

2. ★**즉시 마이크를 끄고**(버퍼에 남은 것도 버린다), ★**회수 사유(`2`)와 함께 앱에 알린다**(§1-2-3). ★사유 `3` 이면 `PARTICIPANT_STATE`(§4-2-6)가 같이 올 수 있다 — 다시 눌러도 `102` 다.
3. 이 `RELEASE` 가 서버의 유예(`T3`)를 끝내고 `T8` 을 멈춘다(§10-4-30 · §10-4-24).

★**증상** — 마이크를 늦게 끄면 서버는 유예(`T3`) 동안만 흘리고 그 뒤로는 ★**송신 위반**으로 보고 `REVOKE(3)` 을 되풀이한다(§10-4-30).
`RELEASE` 를 안 보내면 서버가 유예를 꼬박 채운 뒤에야 다음 사람에게 넘긴다 — ★**선점자가 그만큼 기다린다.**

→ 구조 — [`FLR.C.PERM.rx_REVOKE`](#FLR.C.PERM.rx_REVOKE) · [`FLR.C.PREL.rx_IDLE`](#FLR.C.PREL.rx_IDLE) · [`FLR.C.PREL.rx_TAKEN`](#FLR.C.PREL.rx_TAKEN)

#### 10-4-20. ★같은 것을 `pending Release` 에서 받았다 — 원문 6.2.4.6.7

<a id="RULE-FLR-PROC-PREL-REVOKE"></a>**RULE-FLR-PROC-PREL-REVOKE** · 규칙 — ★같은 것을 `pending Release` 에서 받았다 — 원문 6.2.4.6.7

→ 구조 — [`FLR.C.PREL.rx_REVOKE`](#FLR.C.PREL.rx_REVOKE) · [`FLR.C.NOPERM.rx_other`](#FLR.C.NOPERM.rx_other) · [`FLR.C.PREQ.rx_other`](#FLR.C.PREQ.rx_other) · [`FLR.C.QUEUED.rx_other`](#FLR.C.QUEUED.rx_other)

#### 10-4-21. `2 FLOOR_TAKEN` / `5 FLOOR_IDLE` 을 받았다 (`has no permission` · `queued`) — 원문 6.2.4.3.2·.3 · 6.2.4.9.3·.8

<a id="RULE-FLR-PROC-TAKEN-IDLE"></a>**RULE-FLR-PROC-TAKEN-IDLE** · 규칙 — `2 FLOOR_TAKEN` / `5 FLOOR_IDLE` 을 받았다 (`has no permission` · `queued`) — 원문 6.2.4.3.2·.3 · 6.2.4.9.3·.8

5. ★**서버가 `T1` 으로 회수한 것은 클라에 알려지지 않는다** — 원문 성질이다(서버는 화자에게 `IDLE` 을 보내지 않는다, 6.3.5.5.4). 클라는 뗄 때(`RELEASE`) 서버의 확인으로 안다. 회수 통지는 `REVOKE`(`T2`·선점·권한 비트)뿐이다.
6. `pending Request` 의 `TAKEN` 은 §10-4-8, `IDLE` 은 §10-4-4 3(버린다)이다.

→ 구조 — [`FLR.C.NOPERM.rx_TAKEN`](#FLR.C.NOPERM.rx_TAKEN) · [`FLR.C.NOPERM.rx_IDLE`](#FLR.C.NOPERM.rx_IDLE) · [`FLR.C.QUEUED.rx_TAKEN`](#FLR.C.QUEUED.rx_TAKEN) · [`FLR.C.QUEUED.rx_IDLE`](#FLR.C.QUEUED.rx_IDLE) · [`FLR.C.PERM.rx_TAKEN`](#FLR.C.PERM.rx_TAKEN) · [`FLR.C.PERM.rx_IDLE`](#FLR.C.PERM.rx_IDLE) · [`FLR.wire`](#FLR.wire)

#### 10-4-22. 같은 것을 `pending Release` 에서 받았다 (★반환의 확인) — 원문 6.2.4.6.4 · 6.2.4.6.5

<a id="RULE-FLR-PROC-PREL-CONFIRM"></a>**RULE-FLR-PROC-PREL-CONFIRM** · 규칙 — 같은 것을 `pending Release` 에서 받았다 (★반환의 확인) — 원문 6.2.4.6.4 · 6.2.4.6.5

→ 구조 — [`FLR.C.PREL.rx_IDLE`](#FLR.C.PREL.rx_IDLE) · [`FLR.C.PREL.rx_TAKEN`](#FLR.C.PREL.rx_TAKEN) · [`FLR.C.PREL.rx_TAKEN_seen`](#FLR.C.PREL.rx_TAKEN_seen) · [`FLR.wire`](#FLR.wire)

#### 10-4-23. DC 만 끊겼다 (미디어 생존)

<a id="RULE-FLR-PROC-DC-DOWN"></a>**RULE-FLR-PROC-DC-DOWN** · 규칙 — DC 만 끊겼다 (미디어 생존)

3. ★**다시 열기의 시도·횟수·간격은 클라 재량이다**(주고받을 메시지가 없다, §10-4-32). ★**소진하면 그 서버를
   재구축한다**(§9-2-1 2) — 연관이 산 채로 채널만 안 서는 것은 서버 쪽 채널 고장이다. 연관까지 죽었으면 다시 열기를
   건너뛰고 곧바로 재구축이다.

★**증상**
- 1을 빠뜨리면(방을 안 가르면) → 다방에서 ★**엉뚱한 방의 발언권이 바뀐다.**
- §10-4-19 의 3을 빠뜨리면 → 서버가 유예(`T3`)를 꼬박 채운다 — ★**선점자가 그만큼 기다리고 `REVOKE` 가 `T8` 마다 다시 온다.**
- §10-4-23 을 모르면 → ★**말하고 있다고 믿은 채로 아무도 안 듣는다.**

→ 구조 — [`FLR.C.any.dc_down`](#FLR.C.any.dc_down) · [`FLR.C.any.dc_reopen`](#FLR.C.any.dc_reopen) · [`FLR.wire`](#FLR.wire)

#### 10-4-24. 발언권 — ★3GPP TS 24.380 이름 그대로

<a id="RULE-FLR-TIMERS"></a>**RULE-FLR-TIMERS** · 규칙 — 발언권 — ★3GPP TS 24.380 이름 그대로

★★**타이머 값의 정본은 원장이다** — 값 · 재시도 횟수 · 원문 이름 · 값의 근거는 구조의 타이머 표(`FLR.wire` — 원장 `Timers` · `Counts` 와 타이머 선언에서 짓는다)가 든다. 이 절은 ★**시작 · 정지 · 만료**를 정한다.
★**두 곳에 적으면 하나만 고쳤을 때 갈린다.**

★**자체 이름을 붙이지 않는다.** 원문 타이머를 그 이름·기본값·시작·정지 조건으로 쓴다(원문 Table 11.1.3-1 · 11.2.3-1).
★**`T4`(Inactivity)는 미채택이다** — 그 일("발언 없는 통화의 정리")을 우리는 방 수명
(참가자 기준 TTL, §4-1-13)이 맡는다. 발언권 무활동으로 방을 정리하지 않는다.
★**`T103`(End of RTP media, 클라)은 미채택이다** — 원문에서도 선택(*"optional"*)이다.
★**audio cut-in floor control 그룹 구성도 미채택이다** — 원문에서 그것은 *우선순위 무효·큐 없음·
`T3`=0* 한 덩어리인데, ★**우선순위·큐 기반 제어가 규율형 무전의 본질**이라 채택하지 않는다. ★**큐잉 미지원 그룹 구성도 미채택** — 큐는 항상 켠다.

★★**`T3`(Stop talking grace)는 원문 그대로 쓴다** — 회수를 보낸 뒤 ★**화자의 `RELEASE`(= 회수가 닿았다는 확인)가 올 때까지
화자의 목소리를 계속 흘리는 창**이다(원문 6.3.4.5.1). `T8` 재전송이 그 창 안에서 돈다. 정상이면 화자가 `REVOKE` 를 받자마자
`RELEASE` 를 보내므로(§10-4-19) ★**RTT 만에 끝나고**, 3초를 꼬박 도는 것은 화자가 응답하지 않을 때뿐이다. ★**없으면 회수가 유실될 때
화자는 말한다고 믿는데 아무도 못 듣는다** — 그래서 원문이 둔다.

**클라**

| 이름 | 시작 | 정상 정지 | 끝나면 |
|---|---|---|---|
| `T101` (Floor Request) | 발언 요청을 보냈다 | 허가·거절·대기 응답 · 남의 `TAKEN`(§10-4-8) · 뗌 | 다시 보낸다. ★`C101` 을 소진하면 알리고 **`has no permission`**(원문 6.2.4.4.6) |
| `T100` (Floor Release) | 반환을 보냈다 | ★**그 방의 `IDLE` 또는 `TAKEN` 을 받았다**(원문 6.2.4.6.4·.5 — 반환의 결과물이 곧 확인이다) | 다시 보낸다. ★`C100` 을 소진하면 **`has no permission`**(원문 6.2.4.6.3) |
| `T104` (Floor Queue Position Request) | ★**앱이** 순번을 물었다(§10-4-14) | `QUEUE_INFO` · ★`queued` 를 나감(원문 Table 11.1.1-1) | 다시 묻는다. ★`C104` 을 소진하면 알리고 **`RELEASE` + `T100` → `pending Release`**(원문 6.2.4.9.11) |
| ★`T132` (Queued granted user action) | ★**큐 승계 `GRANTED`** 를 받았다 — ★그동안 `queued` 에 남는다(§10-4-9) | ★**사용자가 발언 의지를 표시했다**(§10-4-10 — hold 면 버튼이 눌린 채인 것이 곧 표시라 즉시, 토글이면 다시 누를 때) | ★**`FLOOR_RELEASE` 를 보내고** *"이제 못 쓴다"* 를 알린다(원문 6.2.4.9.13) |

★**재전송 간격 0.5초는 원문 값이 아니라 설정값**이다. 원문은 값을 안 박고
★**"총 재전송 시간 6초 미만"** 만 못 박았다 — `0.5 × 3 = 1.5초`가 그 안이다.

**서버**

| 이름 | 시작 | 정지 | 만료하면 |
|---|---|---|---|
| `T1` (End of RTP media) | 허가한 뒤. ★**RTP 를 받을 때마다 다시 건다** — ★회수 유예(`pending Floor Revoke`) 중에도(원문 6.3.4.5.3) | 반환 · `T2` 만료 | 그 허가가 끝난 것으로 본다 → Idle(큐가 있으면 승계, 원문 6.3.4.4.3). ★유예 중 만료면 `T3` 를 멈추고 Idle(6.3.4.5.6) |
| `T2` (Stop talking) | ★**허가 시점이 아니라 첫 RTP 를 받은 순간** | 반환 · `T1` 만료 | ★**회수 사유 `2` 로 `REVOKE`** → `pending Floor Revoke` · `T3` 시작(원문 6.3.4.4.4) |
| ★`T3` (Stop talking grace) | ★**`REVOKE` 를 보냈다**(`T2` 만료 · 선점 · 권한 비트 회수) | ★**화자의 `RELEASE` 수신 · `T1` 만료** | 송신 불허 → Idle(큐가 있으면 승계, 원문 6.3.4.5.5) |
| ★`T7` / `C7` (Floor Idle) | `IDLE` 을 보냈다 | 상태 변화 | ★**`C7` 상한까지** IDLE 을 다시 보낸다(일련번호는 계속 증가) |
| ★`T8` / `C8` (Floor Revoke) | `REVOKE` 를 보냈다 — 회수(`pending Floor Revoke`) · ★송신 위반(허가 없이 RTP, `REVOKE(3)`) | ★**`T3` 만료 또는 그 사람의 `RELEASE` 수신**(원문 표) | `REVOKE` 를 다시 보낸다. ★`C8` 을 소진하면 재전송만 멈춘다 — 원문은 횟수와 포기를 구현 선택으로 두고 *"호에서 끊기"* 를 권한다(6.3.5.7.3 NOTE). 끊지 않는 이유: 반환 직후 망에 떠 있던 RTP 도 송신 위반으로 걸리는데, 그 사람은 `has no permission` 이라 `REVOKE` 를 버린다(§10-4-4 3) — 끊으면 정상 사용자가 나간다 |
| ★`T20` / `C20` (Floor Granted) | ★**큐 승계 `GRANTED`** 를 보냈다(선점자 포함 — 선점자도 큐 맨 앞에서 승계된다, §10-4-32) | ★**그 사람의 RTP 수신 · `RELEASE` · `T1` 만료**(원문 6.3.4.4.3·.6) | ★**`C20` 상한까지** 다시 보낸다 |
| ★`T9` (Retry-after) | ★**회수 사유 `2`(상한 초과)로 회수했다** | 만료 · 퇴장 | 그 사람의 재요청 차단이 풀린다 |

★★**`T132` 는 클라 타이머다 — ACK 재전송이 아니다.** 재전송은 `T7`·`T8`·`T20` 이 맡는다.
★**이름을 바꿔 쓰면 서버가 클라 타이머 이름으로 돌게 된다.**

★**타이머와 재시도 횟수는 짝이다.** `T101` 이 끝날 때마다 `C101` 을 세고, 한도를 넘으면 그때 포기한다.
타이머만 있고 횟수가 없으면 **영원히 다시 보낸다.** (`T3` 는 재전송이 아니라 기한이라 횟수가 없다.)

→ 구조 — [`FLR.wire`](#FLR.wire)

#### 10-4-25. 클라가 정하지 않는 것

<a id="RULE-FLR-SERVER-POLICY"></a>**RULE-FLR-SERVER-POLICY** · 규칙

| 무엇 | 어디서 오나 |
|---|---|
| 발언 가능 시간 | 발언권 허가에 실린 값 |
| ★발언권 대기줄 상한 | 서버가 정한다 — 넘치면 `DENY(7)`(§10-4-31) |

#### 10-4-26. ★MBCP 를 준수한다 — 데이터 채널 위에서, 그 한계 안에서

<a id="RULE-FLR-MBCP-COMPLY"></a>**RULE-FLR-MBCP-COMPLY** · 규칙 — DC — 발언권 · 방침과 전제 · ★MBCP 를 준수한다 — 데이터 채널 위에서, 그 한계 안에서

★★**새로 만드는 것은 3GPP TS 24.380(MBCP)을 준수한다.** 이름만 빌리지 않는다 —
★**메시지 번호 · ACK 규칙 · 타이머 · 사유 코드 · 상태 전이를 원문 값으로 쓴다.**

★**단, 전달수단은 데이터 채널이다.** 원문은 UDP/IP 위의 **RTCP APP 패킷**(RFC 3550 — 코딩은 원문 §8. 각 장의 동작 은 off-network 절차다)에
싣고 별도 `m=` 줄·별도 RTCP 포트·`mc_floor_ssrc` fmtp 를 쓴다. 우리는 ★**DC(SCTP) 위의
4바이트 헤더 + TLV**(§2-1-3)에 싣는다.

| 축 | 우리 | 판정 |
|---|---|---|
| **메시지 코딩**(원문 §8) — 메시지 집합·번호·ACK·TLV·사유 코드 | ★**원문 그대로** — 형이 다른 곳 둘(사유 u8 + `0x1B` · ACK Source 미탑재, §10-4-31) | ★**준수**(형 이탈 둘은 §10-4-27) |
| **클라 절차**(원문 §6.2) — 상태·전이·클라 타이머 | ★**원문 그대로**(§10-4-3 — 항마다 원문 절을 적는다) · `T103` 미채택 | ★**준수** |
| ★**서버 절차**(원문 §6.3) — 전체 상태기(§6.3.4)·참가자 상태기(§6.3.5)·서버 타이머·선점 | ★**원문 그대로** — dual floor·multi-talker·`T4` 미채택 | ★**준수**(미채택은 아래 표) |
| **전달수단** — 컨테이너·주소·다중화 | ★**DC 로 치환** | ★**대체 구현** |
| **다방(cross-room)** — TLV `0x1A`~ | 자체 | ★**자체 확장.** 원문이 안 쓰는 번호 + ★**확장 규율 4** — §10-4-31 |
| 원문 §7 off-network · multi-talker 등 | 없음 | ★**미채택**(§10-4-30 에 번호를 비워 표시) |

★★**"DC 의 한계 안에서 유연하게" 가 무슨 뜻인가** — ★**컨테이너가 강제하는 것만 바꾸고,
그 밖은 바꾸지 않는다.** 편해서 바꾸는 것이 아니다.

| 원문 | ★**DC 에서** | 왜 바꿔야만 하나 |
|---|---|---|
| RTCP APP 패킷 · PT=204 · name `"MCPT"` | ★**4B 헤더 + TLV**(§2-1-3) | RTCP 가 아니다 |
| 별도 `m=` 줄 · 별도 RTCP 포트 | ★**`"unreliable"` 채널 하나**(§9-4-13) | m-line 을 늘리지 않는다(§9-2-1) |
| RTCP SSRC 로 제어 채널 다중화 | ★**TLV `0x1D`(방)로 방을 가른다** — 아래 대응표 | ★**RTCP 헤더가 없다** |
| 4옥텟 정렬 + 패딩 (원문 §8.1.3) | ★**TLV 연속 배치** | SCTP 메시지는 정렬 요구가 없다 |
| SIP/SDP 로 세션·자격 협상 (TS 24.379) | ★**WS 시그널링**(각 장의 구조) | ★**미채택 축.** 이 문서 §1-2-1 |

#### 10-4-27. ★버린 RTCP APP 헤더 — 무엇이 대신하나

<a id="RULE-FLR-RTCP-HEADER-DROPPED"></a>**RULE-FLR-RTCP-HEADER-DROPPED** · 규칙 — ★버린 RTCP APP 헤더 — 무엇이 대신하나

★★**원문 메시지의 앞 세 워드(RTCP APP 헤더)를 통째로 버렸다.** 그 헤더가 나르던 것을
하나도 빠뜨리지 않고 다른 데로 옮겼는지 여기서 확인한다.

```
원문:  |V=2|P| Subtype | PT=APP=204 |   length   |
       |        RTCP SSRC of sending entity      |
       |              name = MCPT                |
       |            floor 필드들 …                |

우리:  | ver=1 | svc | len(2 BE) |  ← §2-1-3 DC 프레임
       | V R A Type | TLV 개수 |  ← §10-4-29 MBCP 헤더
       | TLV …                  |
```

| 원문 헤더가 나르던 것 | ★**우리는 무엇으로** |
|---|---|
| `length` | ★**DC 프레임 `len`**(§2-1-3) |
| `name = MCPT` (앱 식별) | ★**채널 이름 `"unreliable"` + `svc=0x01`**(§2-1-3) |
| `PT = 204` (RTCP 임을 표시) | ★**필요 없다** — RTCP 가 아니다 |
| `V=2` · `P`(패딩) | `ver=1` · ★**패딩 없음**(4옥텟 정렬 미적용) |
| `Subtype`(5bit) | ★**`A`(bit4) + `Type`(bit3-0)**(§10-4-29·§10-4-30). 합치면 같은 5비트 |
| ★★**`RTCP SSRC`** — ★**같은 포트 위 여러 통신의 제어 채널을 가르는 값**(원문 §7) | ★★**TLV `0x1D`(방)**(§10-4-35) |

★★**마지막 줄이 이 문서에서 제일 중요한 대체다.** 원문은 ★**통화마다 제어 채널을 따로 두고**
헤더의 SSRC 로 다중화한다. 우리는 ★**DC 하나에 방 여럿을 얹었으므로** 그 자리가 없다 —
★**`0x1D` 가 없으면 다방 청취가 통째로 성립하지 않는다.** 있으면 좋은 필드가 아니다.

★**증상** — `0x1D` 를 빼면 → 여러 방의 `TAKEN`·`IDLE` 이 ★**한 통로로 섞여 들어와 가릴 수단이
없다.** 다른 방 사람이 ★**우리 채널에서 말하는 것처럼 보인다.** 미디어는 정상이다.

★★**원문에 없는 것을 더하지 않는다.** 이 규격은 TS 24.380 의 ★**부분집합**이다 — 미채택은
명시하되(§10-4-30 메시지 · §10-4-24 타이머 · cut-in/큐잉 미지원 그룹 구성), ★**원문에 없는 절차·상태·
정책을 발명하지 않는다.** 원문과 다른 것은 ★**전달수단이 강제한 대체와 원문이 비워 둔 값·축뿐**이고 ★**전량이 이것이다** —
TLV `0x1A`·`0x1B`·`0x1D`(§10-4-31, 원문 미사용 번호 — DC 하나에 방 여럿을 얹은 대체) · `T9` 의 값(§10-4-32, 원문은 이름만 있고 값·시작 조건이 없다) ·
`T7` 의 값(§10-4-24, 원문 기본값 없음) · `T101`/`T100`/`T104` 의 값(§10-4-24, 원문은 합계 상한만) · `C8` 의 값(§10-4-24, 원문이 구현 선택으로 둔 횟수) ·
거절 사유 `101`(§10-4-31, 원문 미할당 영역 — `pub_room` 축) · 거절 사유 `102`(§10-4-31, 원문 미할당 영역 — 권한 비트 축) · 미입장 `DENY(255)`(§10-4-30) ·
권한 비트 회수(§10-4-32 — 화자는 원문 회수 절차를 사유 `3` 으로 · 대기자는 큐에서 빼고 `DENY(102)`) · `FLOOR_REQUEST` 의 `pub_room` 검사(§10-4-30) ·
대기줄 상한을 서버 정책으로 둔 것(§10-4-32 — 원문 *"maximum queue length"* 의 값) · 거절/회수 사유를 u8 + `0x1B` 로 가른 형(§10-4-31, 원문은 16비트+구문) ·
`FLOOR_ACK` 에 Source 필드를 싣지 않는 것(§10-4-31) · 처음 알리기의 방아쇠를 `READY` 로 둔 것(§10-4-30 — 원문은 호 참여 시점, 6.3.5.2.2) · ★선점 판정을 상대 비교로 둔 것(§10-4-32 — 요청 우선순위 > 화자의 허가된 우선순위 · 원문은 로컬 정책 4.1.1.4 와 *"화자가 선점 등급이 아님"* 조건 6.3.5.4.4 5) · `pending Request` 에서 남의 `TAKEN` 을 받아도 남는 것(§10-4-8) · ★`pub_select`/`pub_deselect` 가 그 방 `RELEASE`·대기 철회인 것(§10-4-5 · §10-4-32) · 사유 `5` 의 방아쇠를 반이중 발행 트랙 없음으로 둔 것(§10-4-30) · `queued` 를 나가면 `T132` 도 멈추는 것(§10-4-4) · 거절 사유 `2`·`6` 을 내지 않는 것(§10-4-31) · 서버가 `FLOOR_ACK` 에 답하지 않는 것(§10-4-30) ·
우선순위 상한 협상(`mc_priority`) 미채택(아래 표). ★**이 밖의 것이 슬쩍 늘지 않게 이 목록이 관문이다.**

★**미채택을 명시한다** — 원문에 있으나 우리가 안 쓰는 것은 아래가 전량이다(§10-4-30 메시지 · §10-4-24 타이머는 그 절에).

| 미채택 | 근거 |
|---|---|
| ★**dual floor control**(원문 §6.3.6 · `T11`·`T12` · `D:` 상태) | ★**슬롯 audio 1 구조가 안 받는다**(§10-4-35) — m-line 예산을 넘고, §10-4-35 의 1:1 가정과 §10-4-21 의 회수 처리가 함께 깨진다 |
| ★**override 선점**(원문 §6.3.4.4.7 1b·3) | 위와 같다 — 두 화자를 동시에 흘리는 구조가 없다 |
| ★**audio cut-in 그룹 구성** | 우선순위·큐 기반 제어가 본질이라 안 쓴다(§10-4-24) |
| ★**`T4`(Inactivity) · `T103`(End of RTP media)** | §10-4-24 — 방 수명이 정리를 맡고, `T103` 은 원문에서도 선택이다 |
| ★**`mc_priority` fmtp 협상**(원문 §8.2.3.2 · §14.2.3·§14.3.3) | 우선순위 상한 축이다. ★**미채택 — 요청 선언값을 쓴다**(§10-4-31). 필요해지면 토큰 클레임으로 additive |

★**바꾸지 않는 것** — 메시지 번호(§10-4-30) · ACK 비트가 붙는 메시지(§10-4-30) ·
타이머 이름과 기본값(§10-4-24) · 거절/회수 사유 값(§10-4-31) · TLV 필드 ID(§10-4-31) · ★상태와 전이(§10-4-3 · 서버는 원문 §6.3).
★★**이것들은 컨테이너와 무관하다.** 바꿀 이유가 없으므로 바꾸지 않는다.

★★**말할 수 있는 범위** — *"MCPTT 를 한다"* 가 아니다. ★**"TS 24.380 의 floor control 메시지 코딩과 클라 절차를
데이터 채널 위에 준수 구현하고, 미채택·대체 범위를 §10-4-27·§10-4-30 에 명시한다"** 까지다.
★**상호운용은 전제가 아니다** — 전달 수단이 DC 라서 실제 MCPTT 단말·서버와는 애초에 붙지 않는다. 준수에서 얻는 것은 검증된
상태기 설계와 원문을 옆에 두고 디버깅할 수 있는 어휘이고, 둘 다 메시지 코딩과 클라 절차에 있다 — 층을 뭉뚱그려 준수라 하면 매번
예외가 드러난다.
TS 24.379(SIP 호 제어)·TS 23.379(참조 구조)는 ★**미채택**이라 그 평면은 아무것도 주장하지 않는다.

★**증상** — *"따른다"* 고만 쓰고 무엇을 안 따르는지 안 적으면 → 구현자가 원문을 열어
★**`T9`(재요청 차단)·큐 만석 사유(#7)를 있는 줄 알고 기다리고, dual floor 가 있는 줄 알고 선점당한 뒤에도 말한다.** 안 된다.

→ 구조 — [`FLR.wire`](#FLR.wire)

#### 10-4-28. ★전송로 — 발언권은 어디로 가나

<a id="RULE-FLR-TRANSPORT-DC"></a>**RULE-FLR-TRANSPORT-DC** · 규칙 — ★전송로 — 발언권은 어디로 가나

★★**발언권의 전송로는 데이터 채널 단일이다.** 고를 수 있는 값이 아니다 —
★**`server_config` 에 전송로를 고르는 필드가 없다**(§9-1-1). 흩어져 있던 전제를 여기 모은다.

| 무엇 | 규격 | 원래 있던 곳 |
|---|---|---|
| 프레임 형식 | 4B 헤더 + TLV | §2-1-3 |
| 어느 연결에 붙나 | ★**발행 연결.** `1pc` 면 그 하나뿐인 연결 | §2-1-3 · §9-4-12 |
| 누가 여나 | ★**클라.** 이름은 `"unreliable"` 하나 | §9-4-13 |
| ★**언제부터 쓸 수 있나** | ★**DC 의 `open` 이벤트 뒤** — DCEP `DATA_CHANNEL_ACK` 이 돌아온 시점이다. ICE·DTLS 가 붙은 것과 다른 시점이다(SCTP 수립과 DCEP 왕복이 더 있다). 그 전에 보낸 프레임은 나가지 않으므로 클라는 `open` 을 기다린다 | §9-4-13 |
| ★**청취 전용은** | ★**track-less 발행 연결을 세운다.** DC 가 그 위에 있다 | §9-4-13 · §9-4-17 |
| ★**통지 권위** | ★**DC 단일**(§1-2-1). WS·HTTP 로는 발언권 상태가 오지 않는다 | §4-1-14 · §4-1 |
| ★**강제 권위** | ★**서버의 publisher prefan 게이트**(§1-2-1). 통지가 아니라 이것이 실제로 막고 흘린다 | §10-4-30 |
| WS 가 끊기면 | ★**발언권은 유지된다.** DC 는 미디어 연결 위에 있다 | §10-4-32 |
| `RESUME` 스냅샷 | ★**발언권이 없다.** 끊긴 적이 없으므로 따라잡을 것이 없다 | §3-1 |

★★**WS 폴백 경로는 없다.** 두면 위 표의 *"WS 가 끊겨도 발언권은 유지된다"* 와
*"`RESUME` 스냅샷에 발언권이 없다"* 가 ★**둘 다 거짓이 된다** — 한 줄을 편하게 두려다
★**두 절이 연쇄로 깨진다.** 그리고 런타임 전환은 ★**방아쇠가 망 문제와 구분되지 않는다**(§9-4-20).

★**성립 조건** — 이 절 전체는 ★**DC 가 서 있는 동안만** 참이다. DC 만 끊긴 경우는 §10-4-32 에 있다.

#### 10-4-29. 메시지 헤더

<a id="RULE-FLR-HEADER"></a>**RULE-FLR-HEADER** · 규칙 — 메시지 헤더

→ 구조 — [`FLR.wire`](#FLR.wire) · [`ROOM.http.CREATE_ROOM`](04-room.md#ROOM.http.CREATE_ROOM)

#### 10-4-30. 메시지 종류 — ★원문 Table 8.2.2.1-1 그대로

<a id="RULE-FLR-MSG-TYPES"></a>**RULE-FLR-MSG-TYPES** · 규칙 — 메시지 종류 — ★원문 Table 8.2.2.1-1 그대로

★★**subtype 은 5비트다** — 원문은 `x????` 꼴로 쓰고, ★**첫 비트 `x` 가 곧 ACK 요구 비트**다.
우리 헤더(§10-4-29)는 그것을 `A`(bit4) + `Type`(bit3-0)으로 나눠 담는다. ★**합치면 같은 5비트**다.


★★**가르는 기준 하나 — 타이머로 지켜지는 것에는 ACK 을 얹지 않는다.**

| 우리가 세우는 것 | 왜 |
|---|---|
| ★`GRANTED` | 유실되면 ★**서버는 허가로 아는데 클라는 말을 못 한다** — `T101` 재요청이 덮긴 하나 1.5초가 무전엔 길다 |
| ★`DENY` | ★**unicast 라 비용이 1개**이고, 거절 사유(§10-4-31)를 사용자에게 보여야 한다 |
| ★`TAKEN` | 유실되면 ★**그 발화 내내 듣는 사람 화면에 화자가 없다** — 대신 지켜 줄 다른 장치가 없어 ACK 을 얹고 `T21` 로 다시 보낸다(DEV-FLR-TAKEN-ACK · R-FLR-TAKEN-ACK) |

| 안 세우는 것 | 무엇이 대신 지키나 |
|---|---|
| `IDLE` | ★**`T7`/`C7` 재전송**(§10-4-32) |
| `RELEASE` | ★**`T100`/`C100` — 클라가 직접 재전송**한다 |
| `QUEUE_INFO` | ★**`T104`/`C104` — 클라가 다시 물어본다** |
| `REVOKE` | ★**`T8` 재전송.** 원문이 `A` 를 못 쓰게 한 자리다 |

★**증상** — 이 표가 없으면 → §10-4-30 의 *"`A` 비트가 선 메시지에는 반드시 ACK 을 보낸다"* 가
★**실행할 수 없는 문장**이 된다. 클라마다 다르게 구현하고, ★**서버는 안 오는 ACK 을 3번씩 기다린다.**
★★**미채택 번호는 비워 둔다.** *"안 쓰는 번호"* 가 아니라 ★**원문이 쓰는 번호인데 우리가
채택하지 않은 것**이다. ★**다른 뜻으로 재사용하지 않는다**(§문서 머리 번호 규율).

★**증상** — 번호를 구현 순서대로 매기면 → 동작은 하는데 ★**원문을 옆에 놓고 디버깅할 때
`Taken` 과 `Idle` 이 뒤바뀌어 보인다.** 사설 wire 라 사고는 안 나지만
★**"준수" 라고 쓴 문구가 거짓이 된다.**

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.fact.FlrUnadopted`](#FLR.fact.FlrUnadopted)

<a id="RULE-FLR-ACK-BIT"></a>**RULE-FLR-ACK-BIT** · 규칙 — ★`A` 비트 — 정책이 아니라 인코딩이다

원문 §8.2.2.1:

> 일부 메시지는 **subtype 의 첫 비트**로 송신자가 확인응답을 원하는지 나타낸다.
> `0` = 필요 없음, `1` = 필요함. 확인응답이 필요하면 **Floor Ack** 메시지로 응답한다.


**그러면 `REVOKE` 는 어떻게 지키나** — ★**원문은 ACK 이 아니라 타이머로 지킨다.**


★★**그래서 "IDLE·REVOKE 는 유실되면 복구 수단이 없다" 는 우리 쪽 결함이었다** —
원문에는 T7·T8 이 있고, 우리가 그 둘을 안 갖고 있었을 뿐이다. ★**ACK 비트를 옮겨 붙이는 것이
아니라 타이머를 갖추는 것이 답이다.**

★**증상** — `REVOKE` 에 `A` 를 세우면 → ★**원문에 없는 subtype(`10110`)이 나간다.**
받는 쪽이 원문대로 만들어졌으면 **모르는 메시지로 버린다.**
반대로 T8 이 없으면 → 회수가 유실될 때 ★**사용자는 말하고 있다고 믿고 아무도 안 듣는다.**

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.S.P.PREVOKE.enter`](#FLR.S.P.PREVOKE.enter) · [`FLR.S.G.IDLE.enter_empty`](#FLR.S.G.IDLE.enter_empty) · [`FLR.S.G.TAKEN.T20`](#FLR.S.G.TAKEN.T20)

#### 10-4-31. TLV 필드

<a id="RULE-FLR-TLV"></a>**RULE-FLR-TLV** · 규칙 — TLV 필드

| id | 이름 | 형 | 어디에 실리나 |
|---|---|---|---|
| `0` | 우선순위 | u8 | ★**요청에 실은 값이 곧 우선순위**다 — 서버는 그 값으로 허가하고 ★**선점 판정(§10-4-32)도 이 값으로 한다.** 상한을 씌우는 축은 없다 |
| `1` | 말할 수 있는 시간 | u16 BE(초) | 원문 Floor Request 에 Duration 필드가 없다(8.2.4) |
| `7` | 대기 인원 | — | ★**싣지 않는다** — 원문 Queue Size 는 Floor Granted 의 필드(8.2.5, 2 옥텟)이고 Floor Queue Position Info 에는 없다(8.2.12) |
| ★`8` | ★**메시지 일련번호** | u16 BE | 보낼 때마다 **1 증가**(`T21` 재전송은 처음 그 번호 그대로 — DEV-FLR-T21-SAME-SEQ) · 클라는 화자 표시에만 견준다(DEV-FLR-SEQ-DISPLAY) · ★**GRANTED 필수** — 그 순간의 마지막 번호 그대로(DEV-FLR-GRANTED-SEQ) · ★**ACK** — 받은 메시지에 실려 왔으면 되돌린다(DEV-FLR-ACK-SEQ) |
| ★`0x1D` | ★★**방** (`room_id`) | 문자열 | ★ACK(대상 것 에코, §10-4-30) |


★**성립 조건** — 우선순위(`0`)는 ★**요청이 실은 값이 전부**다. 이 규정은 상한을 씌우는 축이 없는 동안 참이다 — 상한 축(토큰 클레임·협상 값)을 다시 들이면 `GRANTED`·선점 판정이 먼저 깨진다.

★**원문 필드와 형이 다른 곳 둘 — 알고 하는 이탈** — ①`2` 거절/회수 사유: 원문(§8.2.3.4)은 **16비트 값 + 같은 필드 안의 Reject Phrase** 인데 우리는 **u8 + `0x1B` 분리**다(SCTP 라 4옥텟 정렬·패딩이 없어 짧게 간다; 값 공간은 255 안이다) ②`10` Source 필드: 원문 ACK 은 Source(참여자 0/서버 1)를 싣지만 우리는 싣지 않는다 — 신원은 연결이 준다(화자 규율과 같다). 미채택 목록(§10-4-27)에 이 둘을 더한다.

| 폐기 | 두려 했던 것 | 왜 뺐나 |
|---|---|---|
| `0x1C` | 말하는 사람의 방들 | ★**`pub_room` 이 단수**(§5-1-1)라 원소가 늘 하나이고, 그 값이 ★**`0x1D` 와 같다** |
| `0x1E` | 말할 대상 방들 | 같은 이유로 목록일 필요가 없고, ★**`0x1D` 와 같은 축**이다 — 아래 |

→ 구조 — [`FLR.wire`](#FLR.wire)

<a id="RULE-FLR-TLV-EXT-RULES"></a>**RULE-FLR-TLV-EXT-RULES** · 규칙 — ★확장 규율 — 원문 **위에** 얹는다

★★**원문에는 다방이라는 개념이 없다.** 한 통화 = 한 제어 채널이다(§10-4-26).
다방은 ★**우리 제품의 축**이고, 그래서 확장이 필요하다. 확장에는 규율이 있다.

| # | 규율 | |
|---|---|---|
| ① | ★**원문이 안 쓰는 번호에서만** | `0x1A`(26) 이상. `025` 까지는 원문이 쓴다 |
| ② | ★★**한 개념 = 한 필드** | ★**방향마다 다른 필드를 만들지 않는다** |
| ③ | ★**원문 필드의 뜻을 재정의하지 않는다** | `0`~`12` 는 원문 그대로 |
| ④ | ★★**걷어내면 원문 그대로 남는다** | ★**방이 하나면 확장 없이 돌아야 한다** |

★★**`0x1D` 가 ②의 사례다.** 방은 ★**C→S 로 올라갔다가 그대로 S→C 로 내려온다** —
클라가 `REQUEST` 에 어느 방인지 실어야 서버가 그 방의 floor 를 돌리고,
그 값이 ★**`TAKEN` 에 그대로 실려 그 방 구독자에게 간다.** ★**같은 값이 왕복하므로 같은 필드다.**

```
C→S  REQUEST { 0x1D:"r1" }        →  서버가 r1 의 floor 를 돌린다
S→C  GRANTED { 0x1D:"r1" }        →  요청자에게
S→C  TAKEN   { 0x1D:"r1", 4:"u2" } →  r1 구독자 전원에게
```

★**④의 시험** — 방이 하나뿐인 배치에서는 `0x1D` 를 빼도 원문 그대로 성립한다.
★**확장이 원문을 대체하지 않고 얹혀 있다는 증거**가 그것이다.

★★**화자는 클라가 싣지 않는다.** DC 는 ★**발행 연결 위**에 있고 그 연결은 한 사용자의
것이므로(§9-4-13), 서버는 ★**프레임이 도착한 연결로 신원을 안다.** 클라가 실어 보내면
★**자기가 자기 신원을 선언**하는 꼴이고, §4-1-3 가 기각한 형태와 같다.
`TAKEN` 의 말하는 사람(`4`)은 ★**서버가 채운다.**

★★**`0` ~ `12` 는 원문 Table 8.2.3.1-2 의 field ID 그대로다.**

| 우리 | 원문 이름 |
|---|---|
| `7` | Queue Size |

→ 구조 — [`FLR.wire`](#FLR.wire)

<a id="RULE-FLR-TLV-MATRIX"></a>**RULE-FLR-TLV-MATRIX** · 규칙 — ★화자와 지속시간은 축이 갈린다

★★**청취자는 화자의 남은 시간을 모른다.** 원문이 그렇게 정했고 우리도 따른다 —
남은 시간 게이지는 ★**발언자 화면의 것**이고, 청취자에게는 `T2` 기본값이 알려진 값이다.

★**증상** — `TAKEN` 에 지속시간을 얹으면 → 원문에 없는 필드가 나가고, ★**청취자 화면마다
카운트다운이 돌아** 실제 종료(`RELEASE`·`T2`·회수)와 어긋난 채 0 에서 멈춘다.

→ 구조 — [`FLR.wire`](#FLR.wire)

<a id="RULE-FLR-SEQ"></a>**RULE-FLR-SEQ** · 규칙 — ★`8` 메시지 일련번호 — 서버가 싣는다, 클라는 화자 표시 · 대기 이탈에만 견준다

| 규격 | |
|---|---|
| 어디에 | ★**`IDLE` · `TAKEN` 에 반드시**(원문 §6.3.4.3.2 · §6.3.4.4.2 · §6.3.5 의 unicast 확인 — 싣는 쪽(서버) 규정) · ★**`GRANTED` 에 반드시** — 그 순간 방의 마지막 번호(DEV-FLR-GRANTED-SEQ) · ★**`ACK`** — 받은 메시지에 실려 왔으면 그 번호를 되돌려 싣는다(DEV-FLR-ACK-SEQ) |
| 값 | ★**보낼 때마다 1 증가.** 방마다 센다. ★**감김은 원문 정의 그대로다** — 0~65535, 65535 에 닿으면 0 부터(§8.2.3.10). ★**올리는 것은 `IDLE` · `TAKEN` 을 새로 보낼 때뿐이다** — `GRANTED` 와 `T21` 재전송은 올리지 않는다(DEV-FLR-GRANTED-SEQ · DEV-FLR-T21-SAME-SEQ) |
| 클라 | ★**이 번호는 화자 표시와 대기 이탈에만 쓴다** — 낡은 번호의 `TAKEN` · `IDLE` 은 화자 표시를 바꾸지 않고(DEV-FLR-SEQ-DISPLAY) · 대기 중이면 대기를 떠나지 않는다(DEV-FLR-STALE-IDLE-STAY) · 반환 중에 이미 본 번호의 `TAKEN` 은 반환 확인으로 쓰지 않는다(DEV-FLR-T21-SAME-SEQ) · `GRANTED` 의 번호는 본 번호로 삼는다(DEV-FLR-GRANTED-SEQ). ★**ACK 은 낡은 번호에도 한다**(받은 번호를 되돌려 싣는다 — DEV-FLR-ACK-SEQ). 원문 on-network 클라는 이 번호를 보지 않는다(§6.2.4.3.2 — Idle 수신 시 ACK·알림·`T103` 정지뿐) — 비교 주체는 MBMS 로 중계하는 participating function 하나다(§10.2.3.2). ★원문은 데이터 채널이 순서를 바꾸는 경우를 다루지 않는다(우리 채널은 `ordered: false`, §9-4-13). `has permission` 에서는 둘을 버리고, 허가가 끝났음은 `REVOKE` 가 알린다(§10-4-3) |

★**§4-1-6 의 `version{epoch,seq}` 과 다른 축이다** — 그쪽은 ★**WS 로 흐르는 방 상태**,
이쪽은 ★**DC 로 흐르는 발언권**이다. 수명도 전송로도 다르므로 ★**서로 대신하지 못한다.**


★★**자체 확장은 `0x1A`(26) 부터 잡는다.** 원문은 ★**`000`~`025` 를 이미 다 쓴다** —
`020` List of Locations · `021` Queued Floor Requests Purpose · `022` List of Queued Users ·
`023` Response State · `024` Media Flow Control Indicator · `025` Floor Revoke Request User ID.
★**`0x14`~`0x18` 은 빈 자리가 아니다.**

| 왜 `026`~`191` 안에서 잡나 | |
|---|---|
| 아래 경계 | ★**`025` 까지 원문이 쓴다.** 밟으면 준수가 깨진다 |
| ★**위 경계** | ★**원문 §8.1.3 — field ID 가 `192` 이상이면 length 가 2옥텟**이 된다. 넘어가면 파서가 갈린다 |

★**증상** — 원문이 쓰는 번호를 자체 확장으로 쓰면 → ★**"준수" 라고 써 놓고 같은 번호에
다른 뜻을 싣는다.** 사설 wire 라 당장은 안 깨지지만, 원문을 옆에 놓고 읽는 사람이
★**`0x17` 을 `Response State` 로 읽는다.**

**거절 사유(`id 2`)** — ★**원문 §8.2.6.2 그대로**

| 값 | 원문 | 뜻 |
|---|---|---|
| `1` | Another MCPTT client has permission | 이미 다른 사람이 말하는 중 — ★**우리 서버는 내지 않는다**(큐를 항상 지원하므로 점유 중 요청은 `9`(대기) 아니면 `7`(만석)이다). 원문 표의 자리로만 둔다 |
| `2` | Internal floor control server error | 서버 내부 오류 — ★**우리 서버는 내지 않는다**(내부 실패는 세션·WS 층 실패로 드러난다). 원문도 값만 정의하고 보내는 절차가 없다. 원문 표의 자리로만 둔다 |
| `3` | Only one participant | 참가자가 나뿐이다 |
| ★`4` | Retry-after timer has not expired | ★**회수 직후 재요청 차단** — `T9`(§10-4-32) 가 도는 중이다 |
| `5` | Receive only | 청취 전용이다 |
| `6` | No resources available | 자원이 없다(혼잡) — ★**우리 서버는 내지 않는다**(`2` 와 같다). 원문 표의 자리로만 둔다 |
| ★`7` | Queue full | ★**대기줄이 찼다** |
| `255` | Other reason | 그 밖 |
| `100` | *(폐기 — 번호는 비워 둔다)* | 자체 확장이던 *"다른 방에서 발언권을 쥔 채 요청"*. `pub_room` 을 옮기면 그 방 발언권이 묵시 반환되므로(§5-1-2) 이 상태가 서버 하나 안에서 성립하지 않는다 |
| ★`101` | *(원문 미할당 영역)* | ★**자체 확장** — ★**`pub_room` 이 아닌 방에 요청했다**(§10-4-30). `pub_select` 로 옮긴 뒤 다시(§10-4-5) |
| ★`102` | *(원문 미할당 영역)* | ★**자체 확장** — ★**발언 요청 자격이 없다** — `pub_room` 의 `permission.floor_request` 가 `false` 이거나, ★내가 든 반이중 트랙 kind 의 `publish_audio`/`publish_video` 가 그 방에서 `false` 다(§4-2-6 사상표). ★큐 대기 중에 비트가 내려가면 서버가 큐에서 빼며 이 사유로 보낸다(§10-4-32). ★**긴급도 이 사유로 막힌다** — 우회 경로가 없다. ★`5`(청취 전용)와 다르다: 그쪽은 *"올린 것이 없다"* 는 **사실**이고 이쪽은 *"걸 수 없다"* 는 **자격**이다 |

★★**사유를 하나로 뭉치지 않는다.** `7`(큐 만석)을 `1`(남이 말하는 중)로 보내면
★**클라가 화면을 못 가른다** — 앞은 *"기다리면 차례가 온다"*, 뒤는 ★**"줄도 못 선다"** 라
사용자가 할 일이 정반대다.

**회수 사유(`FLOOR_REVOKE` 의 `id 2`)** — ★**원문 §8.2.10 그대로**

| 값 | 원문 | 뜻 |
|---|---|---|
| `1` | Only one MCPTT client | 남은 사람이 나뿐이다 |
| `2` | Media Burst too long | ★**발화 상한 초과**(`T2`) |
| ★`3` | No permission to send a Media Burst | ★**우리 발행처 = 권한 비트 회수**(§10-4-32) — 화자의 `floor_request`(또는 그가 든 반이중 kind 의 `publish_*`)가 그 방에서 내려갔다(§4-2-6). `T9` 는 걸지 않는다(§10-4-32). `PARTICIPANT_STATE`(§4-2-6)가 같이 온다 |
| ★`4` | Media Burst pre-empted | ★**선점당했다** |
| `6` | No resources available | 자원이 없다 |
| `7` | Revoked by another MCPTT client | 다른 사람이 회수시켰다 |
| `255` | Other reason | 그 밖 |

★★**`0x1D` 은 S→C 발언권 메시지 전부에 실린다.** ★**클라가 방마다 발언권 상태를 따로 갖기
때문**이다(§10-4-35) — `TAKEN`·`IDLE` 뿐 아니라 `GRANTED`·`DENY`·`REVOKE`·`QUEUE_INFO` 도
★**어느 방 것인지 모르면 걸 자리가 없다.** 다방에서 두 방에 요청을 걸어 두면 `DENY` 가
어느 쪽 답인지 가릴 수단이 이 필드뿐이다.

★**증상** — `0x1D` 없이 *"내 방"* 으로 가정하면 → 다방에서 ★**엉뚱한 방의 발언권 상태가 바뀐다.**
DC 는 미디어 서버당 하나라 ★**여러 방 통지가 한 통로로 섞여 온다.**

→ 구조 — [`FLR.C.NOPERM.rx_IDLE_stale`](#FLR.C.NOPERM.rx_IDLE_stale) · [`FLR.C.NOPERM.rx_TAKEN_stale`](#FLR.C.NOPERM.rx_TAKEN_stale)

#### 10-4-32. 발언권 규칙 — 자체 값과 절차

<a id="RULE-FLR-OWN-RULES"></a>**RULE-FLR-OWN-RULES** · 규칙 — 발언권 규칙 — 자체 값과 절차

★★**타이머 값은 여기 없다.** ★**§10-4-24 가 정본**이다 — 이름·기본값·시작·만료 조건이 거기 있다.
이 절은 ★**발언권에서만 뜻이 생기는 것**만 적는다.

★**무엇이 무엇을 지키나** (값은 §10-4-24)

| 지켜야 할 것 | 수단 |
|---|---|
| `FLOOR_GRANTED`(큐 승계) | `T20` / `C20` 재전송 |
| `FLOOR_REVOKE` | `T8` 재전송 (★`A` 를 못 쓰는 자리, §10-4-30) |
| `FLOOR_IDLE` | `T7` / `C7` 재전송 |
| `FLOOR_REQUEST`·`RELEASE`·`QUEUE_POS_REQUEST` | ★**클라가 직접 재전송** — `T101`/`T100`/`T104` |
| `FLOOR_GRANTED`·`FLOOR_DENY` | ★**ACK**(§10-4-30 `A` 비트 표) |

★★**타이머마다 카운터가 짝이다.** ★**카운터 없이 타이머만 두면 영원히 다시 보낸다.**

★★**`T132` 는 클라 타이머다** — ★**ACK 재전송이 아니다.** 재전송은 `T7`·`T8`·`T20` 이 맡는다.
이름을 바꿔 쓰면 ★**서버가 클라 타이머 이름으로 돌게 된다.**

**자체 값** (원문 미정의 — §10-4-26 의 *자체 확장*)

| 값 | 규격 |
|---|---|
| 기본 우선순위 | `0` (0 최저 ~ 255 최고) — ★원문 값이다(§8.2.3.2 "default priority is '0'"). 자체 값이 아니라 여기 적어 둘 뿐이다 |
| 대기줄 최대 | ★**서버 정책**(§1-2-6 와 같은 축 — 클라가 정하지 않는다). 넘치면 `DENY(7)`(§10-4-31) |
| ★`T9` (Retry-after) | 원문이 값을 정의하지 않는다 — 값은 §10-4-24, 근거는 아래 |

★★**시간 값은 상수가 아니라 기본값이다.** ★**실제 발화 시간은 허가(`1 FLOOR_GRANTED`)의
TLV `1` 에 실려 온다**(§1-2-6). 클라는 ★**받은 값을 쓰고 표의 숫자를 쓰지 않는다.**

<a id="RULE-FLR-T9-VALUE"></a>**RULE-FLR-T9-VALUE** · 규칙 — ★`T9` (Retry-after) — 값은 우리가 정한다

★★**원문이 값을 정의하지 않았다.** `T9` 는 거절 사유 `#4` 의 설명문에만 나오고
(*"permission to send media 가 회수된 뒤 `T9`(Retry-after) 가 아직 안 끝났다"*),
★**Table 11.1.3-1 에 행이 없다.** 값·시작·정지 조건이 원문에 없으므로 ★**설정 축으로 빠진 것**이고,
우리가 정한다 — §10-4-26의 **자체 값**이다.


★★**왜 필요한가 — 없으면 `T2` 가 무력해진다.** `T2`(발화 상한)로 회수해도 ★**곧바로 다시
누르면 그만**이라, 한 사람이 `T2` 만큼씩 끊어 누르며 채널을 계속 쥔다. `T9` 가 그 틈을 만든다.

★**왜 3초인가** — 남이 끼어들 수 있어야 뜻이 있다. 클라 `T101` 재전송이 ★**0.5초 × 3회 = 1.5초**
(§10-4-24)이므로 그보다 길어야 대기자의 요청이 한 바퀴 돈다. 그것이 근거의 전부다 — `T3`(회수 유예)와는 무관하다.

★**성립 조건** — 이 값은 `T101`×`C101` 이 1.5초인 동안만 뜻이 있다. ★**재전송 설정을 바꾸면
이 값을 다시 잡는다.**

★**증상** — `T9` 가 없으면 → ★**한 사람이 채널을 영구히 쥔다.** 상한은 걸리는데 곧바로
다시 잡으므로 남들은 ★**"30초마다 잠깐 비는" 틈만 보고 계속 놓친다.**

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.S.G.TAKEN.T2`](#FLR.S.G.TAKEN.T2) · [`FLR.S.G.IDLE.req_retry_after`](#FLR.S.G.IDLE.req_retry_after) · [`FLR.S.P.NP_TAKEN.req_retry_after`](#FLR.S.P.NP_TAKEN.req_retry_after) · [`FLR.S.T9.expire`](#FLR.S.T9.expire) · [`FLR.S.ws.leave`](#FLR.S.ws.leave)

<a id="RULE-FLR-PERM-REVOKE-ENFORCE"></a>**RULE-FLR-PERM-REVOKE-ENFORCE** · 규칙 — ★권한 비트 회수 — 화자는 `REVOKE(3)`, 대기자는 `DENY(102)`

결정자가 비트를 내리면(§4-2-6) 서버는 ★**그 방이 그 사람의 `pub_room` 일 때** 발언권 축에서도 즉시 집행한다.

| 그 사람이 | 서버 | 클라 |
|---|---|---|
| 그 밖 | 아무것도 안 한다 | `PARTICIPANT_STATE`(§4-2-6)로 버튼을 잠근다 |

★**원문 밖은 방아쇠뿐이다** — 원문에는 권한 비트 축이 없다(§10-4-27 관문 목록). 화자 쪽은 방아쇠만 우리 것이고 ★**절차는 원문 회수 그대로**다(사유 `3` 은 원문 §8.2.10 의 값 —
원문의 발행처는 허가 없이 보내는 사람, 6.3.5.7). 대기자를 서버가 빼는 절차는 원문에 없어 `DENY` 로 결말을 맞춘다.

→ 구조 — [`FLR.S.perm.speaker`](#FLR.S.perm.speaker) · [`FLR.S.perm.queued`](#FLR.S.perm.queued) · [`FLR.S.perm.other`](#FLR.S.perm.other) · [`FLR.S.gate.no_permission`](#FLR.S.gate.no_permission) · [`FLR.C.PERM.rx_REVOKE`](#FLR.C.PERM.rx_REVOKE) · [`FLR.C.QUEUED.rx_DENY`](#FLR.C.QUEUED.rx_DENY)

<a id="RULE-FLR-PREEMPT"></a>**RULE-FLR-PREEMPT** · 규칙 — ★선점 — 원문 §6.3.4.4.7 그대로(회수 쪽)

★★**선점은 "뺏어서 큐 맨 앞에 세운다"다.** 원문은 로컬 정책으로 ①현 화자 회수 ②둘 다 허용(dual floor) 중 하나를 고르는데,
★**우리는 ①이다** — dual floor 는 미채택이다(§10-4-27 — 슬롯 audio 1 구조가 두 화자를 못 받는다).

| 걸음 | 서버 | 누구에게 |
|---|---|---|
| ① | `T1`·`T20` 정지 · ★**`6 FLOOR_REVOKE` + 회수 사유 `4`**(Media Burst pre-empted) · `pending Floor Revoke` · ★`T3` 시작 · 화자는 `U: pending Floor Revoke`(`T8`) | 현 화자 |
| ② | ★**선점자를 큐 맨 앞에** 넣고 `9 FLOOR_QUEUE_INFO` | 선점자 |
| ③ | ★**현 화자의 `RELEASE`**(보통 RTT 뒤) ★또는 `T3` 만료 → Idle → 큐 맨 앞 승계 → `1 FLOOR_GRANTED`(`T20`) + `2 FLOOR_TAKEN` | 선점자 · 나머지 |

| 규칙 | |
|---|---|
| 언제 | ★**요청자의 요청 우선순위(TLV `0`)가 현재 화자의 허가된 우선순위보다 클 때만**— ★**높은 값 우선의 상대 비교**다(원문은 서버 로컬 정책 4.1.1.4 · 이탈은 §10-4-27). 그래서 선점으로 쥔 화자도 더 높은 값에 다시 선점된다. ★**큐에 선점 요청이 이미 있거나 회수 유예 중이면 그 선점 요청은 버린다** — 원문에 그 칸의 절이 없다(6.3.5.4.4 5·8·9 · 6.3.4.5). 클라 `T101` 재요청이 다음 상태에서 선다 |
| ★**기본 배치** | ★**선점이 일어나지 않는다** — 모두 기본 `0` 이면 `>` 가 성립하지 않아 전원 동급 FIFO 큐다. ★**선점은 앱이 값을 정해야 열린다** |
| ★**선점자가 받는 응답** | ★**누르고말하기(hold)면 버튼이 눌린 채라 곧바로 열린다.** 토글이면 한 번 더 눌러야 한다 — ★**긴급 버튼은 hold 로 만든다** |
| ★**도착 순서** | `QUEUE_INFO` 는 ①과 같은 때, `GRANTED` 는 ★**현 화자의 `RELEASE` 뒤**(RTT 이상)라 두 메시지가 맞붙지 않는다 |
| ★**지연** | ★**정상이면 RTT 하나**(현 화자의 `RELEASE` 가 `T3` 를 끝낸다). `T3`(3초)를 꼬박 기다리는 것은 ★**현 화자가 응답하지 않을 때뿐**이다 — 그 3초가 회수 유실을 덮는 창이다(§10-4-24) |

★**클라에게 사유가 큰 차이다** — `4`(선점)면 다시 눌러도 또 뺏기고, `2`(상한 초과)면 다시 누르는 것이
맞다. ★**처방이 반대인데 사유를 안 가르면 화면이 같다.**

→ 구조 — [`FLR.S.G.TAKEN.preempt`](#FLR.S.G.TAKEN.preempt) · [`FLR.S.G.PREVOKE.enter`](#FLR.S.G.PREVOKE.enter) · [`FLR.S.P.PREVOKE.enter`](#FLR.S.P.PREVOKE.enter) · [`FLR.S.G.PREVOKE.rel_revoked`](#FLR.S.G.PREVOKE.rel_revoked) · [`FLR.S.G.PREVOKE.T3`](#FLR.S.G.PREVOKE.T3) · [`FLR.S.G.IDLE.enter_succession`](#FLR.S.G.IDLE.enter_succession) · [`FLR.C.QUEUED.rx_GRANTED`](#FLR.C.QUEUED.rx_GRANTED) · [`FLR.S.G.TAKEN.T2`](#FLR.S.G.TAKEN.T2)

<a id="RULE-FLR-SERVER-EVENTS"></a>**RULE-FLR-SERVER-EVENTS** · 규칙 — ★소속과 발언권 — 쥔 채로 방을 옮기지 않는다

| 사건 | 서버 |
|---|---|
| 발언권을 쥔 방을 `pub_deselect`(§5-1-2) 또는 `ROOM_LEAVE` 했다 | ★**그 방 `RELEASE` 와 같다** — 반환하고 큐가 있으면 승계. 클라는 그 방을 `has no permission` 으로 내린다(`IDLE`/`TAKEN` 이 확인) |
| ★방을 잃었다 — 강퇴 · 방 닫힘 · `RESUME` 스냅샷에 없는 방 | ★**그 방 `ROOM_LEAVE` 와 같다**(DEV-FLR-ROOM-GONE) — 반환하고 큐가 있으면 승계 · 대기 철회. 클라는 그 방을 `has no permission` 으로 내리고 반이중 송출을 멈춘다(R-FLR-MIC-CLOSE-ON-LOSS) |
| 큐 대기 중인 방을 `pub_deselect`(또는 다른 방 `pub_select`)했다 | ★**대기도 빠진다** — 대기는 `pub_room` 에서만 걸린다(§10-4-30). 통지는 없고 클라는 응답 뒤 그 방을 `has no permission` 으로 내린다 |

★**WS 가 끊겨도 발언권은 유지된다.** 발언권은 ★**데이터 채널(미디어 연결) 위로** 가고
WS 는 시그널링일 뿐이다 — 미디어가 살아 있으면 발언권도 살아 있다.

| 무엇이 끊겼나 | 내가 쥔 발언권 |
|---|---|
| **WS 만** (미디어 생존) | ★**유지된다.** 회수 기준은 RTP 미관찰(`T1`)과 발언 상한(`T2`)뿐이다 |
| ★**DC 만** (미디어 생존) | ★★**유지되나 통지가 멎는다.** 서버는 `T1`·`T2` 만 보므로 ★**회수하지 않는다** — 아래 |
| 미디어도 (RTP 멈춤) | `T1` 뒤 ★**회수된다.** 연결이 되살아나면 재동기가 그것을 알려 준다(§9-2-1 5) |
| 세션이 지워짐 (`resume_window_ms` 초과) | ★**회수된다.** `RESUME` 이 `2008` 로 실패한다(§3-1) |

★★**"DC 만 끊김" 이 이 표에서 제일 위험하다.** ★**양쪽이 다르게 알게 되는 유일한 칸**이다 —
서버는 발언권이 살아 있다고 알고 클라는 ★**아무 통지도 못 받는다.** 미디어는 정상이라
★**어떤 미디어 지표로도 안 잡힌다**(§9-4-13).

| | |
|---|---|
| 서버가 하는 일 | ★**아무것도.** `T1`·`T2` 만 보므로 회수 조건에 안 걸린다 |
| 클라가 아는 것 | ★**끊기기 직전 상태에서 굳는다.** `TAKEN`·`IDLE`·`REVOKE` 가 안 온다 |
| ★**복구 절차** | 다시 열기의 시도·횟수·간격은 ★**클라 재량**이다 — ★**주고받을 메시지가 없다.** (§3-2-4 이 규격인 것은 `RESUME` 이라는 **주고받는 메시지**가 있어서다) ★**소진하면 그 서버를 재구축한다**(§10-4-23 · §9-2-1 2) — 연관이 산 채로 채널만 안 서는 것은 서버 쪽 채널 고장이다 |

★**증상** — 이 칸을 모르면 → 사용자가 ★**말하고 있다고 믿는 채로 아무도 안 듣는다**(반대도 성립).
버튼은 눌리고 마이크는 켜져 있고 RTP 도 나가는데 ★**남의 화면에 이름이 안 뜬다.**

★**클라 의무** — ★**WS 만 끊겼다가** `RESUME` 이 성공해도 ★**발언권을 다시 요청하지 않는다.** 쥐고 있던 것은 그대로다.
★**끊긴 동안에도 DC 로 통지가 계속 왔으므로 내가 아는 상태가 곧 현재 상태다.**
★**미디어 연결까지 끊겼다가 재구축 없이 되살아났으면 다르다** — 끊긴 동안의 통지가 사라졌을 수 있다. 그때도 발언권을
다시 요청하지 않고 ★**`READY` 로 지금 상태를 받는다**(재동기, §9-2-1 5 — 처음 알리기가 답한다, §10-4-30).

★**증상** — 재접속마다 발언권을 다시 요청하면 → 서버가 `GRANTED` 를 다시 주는데(§10-4-30 재허가 —
거절이 아니다) ★**시간(`1`)에 `T2` 남은 값이 실려**, 클라가 그것을 새 발화로 알고 카운트다운을
다시 시작한다. 화면의 남은 시간이 실제보다 길다.

#### 10-4-33. 메시지별 규격

<a id="RULE-FLR-MSG-REQUEST"></a>**RULE-FLR-MSG-REQUEST** · 규칙 — 메시지별 규격 · `0 FLOOR_REQUEST` — 말하게 해달라   **C→S**

★시간(`1`)은 싣지 않는다(원문 8.2.4 — 말할 수 있는 시간은 서버가 `GRANTED` 에 `T2` 값으로 준다)

★★**`0x1D` 를 반드시 싣는다.** DC 하나에 방 여럿이 얹혀 있으므로(§10-4-26)
★**이것이 없으면 서버가 어느 방의 floor 를 돌릴지 모른다.** 그리고 이 값이 그대로
★**`TAKEN` 에 실려 그 방 구독자에게 간다.**

★**화자는 싣지 않는다** — 서버가 연결로 안다(§10-4-31 확장 규율).
**절차**
- 클라 ★**허가(`GRANTED`)를 받은 뒤에 오디오를 보낸다**(원문 6.2.4.5.2 — 송신은 `has permission` 에서만). ★발언 방 전환이면 요청 자체를 `pub_select` 응답 뒤에 보낸다(§10-4-5 — 서버가 `pub_room` 을 검사한다).
- 서버 ★★**대상 방 검증은 둘이다 — 입장해 있는가(§4-4-1 `JOINED`) · 그 서버의 `pub_room` 인가.** `pub_room` 이 아니면
  `DENY(101)`(§10-4-31). 강제 권위(prefan, §1-2-1)가 발언권과 `pub_room` 을 둘 다 보므로 요청도 같은 조건이라야 *"허가는 났는데
  `pub_room` 이 아니라 무음"* 인 틈이 없다. 클라는 발화 전에 그 방을 `pub_room` 으로 만들고 응답 뒤 요청한다(§10-4-5).
- ★★**대기도 `pub_room` 한 방에만 건다** — `pub_room` 이 하나라 다방 대기는 없다. 다른 방에서 말하려면 `pub_select` 로
  옮긴 뒤 요청하고, 옮기는 순간 옛 방의 대기·발언권은 빠진다(§10-4-32). 상태기는 방마다지만(§10-4-3) `pending Request`·`queued` 는
  한 방에만 선다.
- 서버 ★**거절 조건이 정해져 있다**(원문 §6.3.4.3.3) — ★**방에 나뿐이면 `3`**(Only one
  participant — ★**`Floor Idle` 일 때만**, 원문 6.3.4.3.3. 화자가 있으면 그 화자의 재요청은 아래 재허가다) · ★**반이중 발행 트랙이 없는 참가자(= 청취 전용)면 `5`**(Receive only) — ★무시가 아니라 거절이다(응답 없이 버리면 `T101`×3 을 헛되이 태운다). 큐가 찼으면 ★**`7`**. ★**`0x1D` 의 방에 내가 입장해 있지 않으면 `255`**(Other reason, `0x1B`=`"not_in_room"`) — 원문에 자리가 없는 우리 쪽 오류라 `255` 다. ★**입장은 했는데 그 방이 내 `pub_room` 이 아니면 `101`**(§10-4-31). ★**`pub_room` 의 `permission.floor_request` 가 `false` 이거나 내가 든 반이중 트랙 kind 의 `publish_audio`/`publish_video` 가 그 방에서 `false` 면 `102`**(§10-4-31 · §4-2-6 사상표 — ★**우선순위 TLV 를 보기 전에 판정한다.** 긴급도 여기서 막힌다). ★**`0x1D` 자체가 없으면 응답하지 않고 계수한다**(돌려줄 방이 없어 클라가 버린다 — `QUEUE_POS_REQUEST` 와 같은 규칙).

★**증상** — 데이터 채널이 안 서 있으면 요청이 **아무 데도 안 간다.** 버튼을 눌러도 반응이 없다.

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.C.PREQ.T101`](#FLR.C.PREQ.T101) · [`FLR.C.NOPERM.press`](#FLR.C.NOPERM.press) · [`FLR.S.G.IDLE.req_grant`](#FLR.S.G.IDLE.req_grant) · [`FLR.S.P.NP_TAKEN.req_enqueue`](#FLR.S.P.NP_TAKEN.req_enqueue) · [`FLR.S.G.TAKEN.req_regrant`](#FLR.S.G.TAKEN.req_regrant) · [`FLR.S.perm.queued`](#FLR.S.perm.queued) · [`FLR.S.perm.speaker`](#FLR.S.perm.speaker) · [`FLR.S.P.PREVOKE.rx_other`](#FLR.S.P.PREVOKE.rx_other) · [`FLR.S.P.NPSENDS.rx_other`](#FLR.S.P.NPSENDS.rx_other) · [`FLR.S.P.NP_TAKEN.req_renotify`](#FLR.S.P.NP_TAKEN.req_renotify) · [`FLR.S.P.NP_TAKEN.req_preempt`](#FLR.S.P.NP_TAKEN.req_preempt) · [`FLR.S.P.NP_TAKEN.req_preempt_blocked`](#FLR.S.P.NP_TAKEN.req_preempt_blocked) · [`FLR.S.G.PREVOKE.preempt_discard`](#FLR.S.G.PREVOKE.preempt_discard) · [`FLR.S.P.NP_TAKEN.req_full`](#FLR.S.P.NP_TAKEN.req_full)

<a id="RULE-FLR-MSG-GRANTED"></a>**RULE-FLR-MSG-GRANTED** · 규칙 — `1 FLOOR_GRANTED` — 말해라   **S→C**

★★**화자(`4`)를 싣지 않는다.** 원문 Table 8.2.5-1 에 없다 — ★**나에게 오는 것**이라
내 이름을 실을 이유가 없다.(`User ID`(`6`)가 있지만 ★**off-network 전용**이다.)

★**시간(`1`)은 `T2` 의 값**이다(원문 6.3.4.4.2 1a — 재허가면 `T2` 남은 값) — ★**받은 값을 쓴다.**
★**우선순위(`0`)는 요청에 실은 값 그대로 돌아온다** — 깎는 축이 없다(§10-4-31 성립 조건).
**절차**

★**증상**
- ACK 을 안 하면 서버의 도달 관측이 빈다 — ★서버는 그것으로 다시 보내지 않는다(직접 허가의 유실은 클라 `T101` 재요청이 덮는다).
- ★`T132` 가 없으면 → 큐에서 차례가 왔는데 ★**사용자가 자리를 비운 사이 채널이 `T2` 만큼 묶인다.**
  서버는 `T2` 가 다 돌 때까지 기다린다.

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.S.G.IDLE.enter_succession`](#FLR.S.G.IDLE.enter_succession) · [`FLR.S.G.TAKEN.T20`](#FLR.S.G.TAKEN.T20) · [`FLR.C.PREQ.rx_GRANTED`](#FLR.C.PREQ.rx_GRANTED) · [`FLR.C.QUEUED.rx_GRANTED`](#FLR.C.QUEUED.rx_GRANTED) · [`FLR.C.QUEUED.intent`](#FLR.C.QUEUED.intent) · [`FLR.C.QUEUED.T132`](#FLR.C.QUEUED.T132)

<a id="RULE-FLR-MSG-DENY"></a>**RULE-FLR-MSG-DENY** · 규칙 — `3 FLOOR_DENY` — 안 된다   **S→C**

★**증상** — 사유 `101`(`pub_room` 이 아닌 방)을 못 가르면 ★**계속 눌러도 안 되는 이유를
사용자가 모른다** — 처방은 채널을 고르는 것이다(`pub_select`).

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.C.PREQ.rx_DENY`](#FLR.C.PREQ.rx_DENY) · [`FLR.C.QUEUED.rx_DENY`](#FLR.C.QUEUED.rx_DENY)

<a id="RULE-FLR-MSG-RELEASE"></a>**RULE-FLR-MSG-RELEASE** · 규칙 — `4 FLOOR_RELEASE` — 다 말했다   **C→S**

**절차**

★**증상** — 안 보내면 ★**`T2` 가 다 될 때까지 남들이 못 말한다.**

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.C.PERM.release`](#FLR.C.PERM.release) · [`FLR.C.PERM.rx_REVOKE`](#FLR.C.PERM.rx_REVOKE) · [`FLR.C.QUEUED.T132`](#FLR.C.QUEUED.T132) · [`FLR.S.P.PERMITTED.rx_RELEASE`](#FLR.S.P.PERMITTED.rx_RELEASE) · [`FLR.S.G.TAKEN.rel_speaker`](#FLR.S.G.TAKEN.rel_speaker) · [`FLR.S.P.PERMITTED.a_IDLE`](#FLR.S.P.PERMITTED.a_IDLE) · [`FLR.S.P.PERMITTED.a_TAKEN`](#FLR.S.P.PERMITTED.a_TAKEN) · [`FLR.S.P.NP_IDLE.rx_RELEASE`](#FLR.S.P.NP_IDLE.rx_RELEASE) · [`FLR.S.P.NP_TAKEN.rx_RELEASE`](#FLR.S.P.NP_TAKEN.rx_RELEASE) · [`FLR.S.P.PREVOKE.rx_RELEASE`](#FLR.S.P.PREVOKE.rx_RELEASE) · [`FLR.S.G.PREVOKE.rel_revoked`](#FLR.S.G.PREVOKE.rel_revoked) · [`FLR.S.P.NPSENDS.rx_RELEASE_taken`](#FLR.S.P.NPSENDS.rx_RELEASE_taken) · [`FLR.S.P.NPSENDS.rx_RELEASE_idle`](#FLR.S.P.NPSENDS.rx_RELEASE_idle) · [`FLR.S.G.IDLE.enter_succession`](#FLR.S.G.IDLE.enter_succession)

<a id="RULE-FLR-MSG-IDLE"></a>**RULE-FLR-MSG-IDLE** · 규칙 — `5 FLOOR_IDLE` — 아무도 안 말한다   **S→C**

**절차** — 원문 §6.3.4.3.2 · §6.3.4.3.4

★**증상** — 큐가 있는데도 `IDLE` 을 보내면 → 화면이 ★**"아무도 안 말함" 으로 한 번 깜빡였다가**
다음 화자로 바뀐다. 무전에서 이 깜빡임은 ★**끊긴 것처럼 보인다.**

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.S.G.IDLE.enter_empty`](#FLR.S.G.IDLE.enter_empty) · [`FLR.S.G.IDLE.enter_succession`](#FLR.S.G.IDLE.enter_succession) · [`FLR.S.P.PERMITTED.a_IDLE`](#FLR.S.P.PERMITTED.a_IDLE) · [`FLR.S.G.IDLE.T7`](#FLR.S.G.IDLE.T7) · [`FLR.S.P.join.idle`](#FLR.S.P.join.idle) · [`FLR.C.NOPERM.rx_IDLE`](#FLR.C.NOPERM.rx_IDLE)

<a id="RULE-FLR-JOIN-NOTIFY"></a>**RULE-FLR-JOIN-NOTIFY** · 규칙 — ★처음 알리기 — 서버가 현재 화자를 보내는 때

발언권의 권위는 ★**DC 단일**이다. 그래서 들어간 방에 ★**이미 말하고 있는 사람이 있으면 서버가
따로 알려야** 한다 — 안 그러면 클라는 **그 사람이 말을 마칠 때까지 아무것도 모른다.**


| 규격 | 내용 |
|---|---|
| 방아쇠 | **DC 가 열린 시점이 아니다** — 그때는 구독 m-line 이 아직 안 붙어 화자를 걸 곳이 없다 |
| 화자가 **없으면** | 처음 입장이면 클라의 초기 상태(*"아무도 안 말함"*)와 같아 바뀌는 것이 없지만, ★**재구축 없이 되살아난 연결의 재동기**(§9-2-1)에서는 끊긴 동안 사라진 `IDLE` 을 대신하는 ★**유일한 신호**다 — 없으면 낡은 화자 표시가 그대로 남는다 |
| ★**몇 번** | 첫 `READY` 에만 묶지 않는다. 서버가 첫 `READY` 만 쓰는 것은 정체 판정의 시작점뿐이다(정체 창이 영영 안 차지 않게). 처음 알리기를 같은 조건에 묶으면 재동기용 `READY` 에 답이 없다 |
| ★**재전송** | 잃으면 다음 발언권 사건(`TAKEN`·`IDLE`)이나 다음 `READY` 가 덮는다(§9-2-1 — 재동기 재전송 타이머는 두지 않는다) |

★★**방마다 온다.** 이 통지는 `READY` 에 실린 `room_id` 의 방에 대해서만 나가므로,
★**입장한 방마다 `READY` 를 보내야 그 방의 화자를 받는다**(§7-1-2).
키프레임 요청도 같은 범위다 — **한 신호가 두 가지를 함께 나른다.**

★**증상**
- 처음 알리기가 없으면 → 들어간 방에서 ★**이미 말하고 있는 사람이 안 보인다.** 소리는 나는데
  화면은 비어 있고, 그 사람이 말을 마쳐야(`5 FLOOR_IDLE`) 비로소 맞는다.
- ★다방에서 `READY` 를 한 방에만 보내면 → **나머지 방의 화자가 안 뜬다**(그 방 영상도 검다, §7-1-2).
- 이것을 `ROOM_JOIN` 응답으로 대신하면 → 그 시점엔 ★**DC 도 구독 m-line 도 없어 쓸 수가 없고**,
  준비될 때까지 흐르는 동안 ★**값이 이미 낡는다.** 그리고 ★**권위가 둘로 갈린다.**

→ 구조 — [`FLR.S.P.join.taken`](#FLR.S.P.join.taken) · [`FLR.S.P.join.idle`](#FLR.S.P.join.idle) · [`FLR.S.P.join.self`](#FLR.S.P.join.self) · [`FLR.wire`](#FLR.wire)

<a id="RULE-FLR-MSG-TAKEN"></a>**RULE-FLR-MSG-TAKEN** · 규칙 — `2 FLOOR_TAKEN` — 누가 말하고 있다   **S→C**

★**우선순위(`0`)를 싣지 않는다.** 원문 Table 8.2.9-1 에 Floor Priority 가 없다 —
★**청취자가 화자의 우선순위로 할 일이 없다.**
**절차**
- 클라 `0x1D` 이 **내가 보고 있는 방이면** 내 화면의 화자로, **다른 방이면** 그 방 표시로 나눈다.

★**증상** — ★서버가 `0x1D` 을 빼면 클라가 **없는 것을 "내 방"으로 읽어서**, 다른 방 사람이
**우리 채널에서 말하는 것처럼 보인다.** 미디어는 정상이라 **화면만 거짓말한다.**

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.S.G.TAKEN.enter`](#FLR.S.G.TAKEN.enter) · [`FLR.S.G.TAKEN.req_regrant`](#FLR.S.G.TAKEN.req_regrant) · [`FLR.S.P.join.taken`](#FLR.S.P.join.taken) · [`FLR.S.G.TAKEN.preempt`](#FLR.S.G.TAKEN.preempt) · [`FLR.S.G.PREVOKE.rel_revoked`](#FLR.S.G.PREVOKE.rel_revoked) · [`FLR.S.G.PREVOKE.T3`](#FLR.S.G.PREVOKE.T3)

<a id="RULE-FLR-MSG-REVOKE"></a>**RULE-FLR-MSG-REVOKE** · 규칙 — `6 FLOOR_REVOKE` — 그만 말해라   **S→C**

**절차** — 원문 §6.3.4.5 · §6.3.5.6 · §6.2.4.5.4

★★**회수에는 유예가 있다 — `T3`.** 서버는 `REVOKE` 를 보내고 `G: pending Floor Revoke` 로 가며, ★**화자의 `RELEASE` 가 오거나 `T3` 가 끝날
때까지 그 사람의 RTP 를 계속 흘린다**(원문 6.3.4.5.1·.3). 유예는 말끝을 위한 것이 아니라 ★**회수가 화자에게 닿았음을 확인하는 창**이다(§10-4-24).


- 클라 ★**사유(`2`)마다 화면이 달라야 한다** — `4`(선점)면 다시 눌러도 또 뺏기고,
  `2`(상한)면 다시 누르는 것이 맞다. ★**처방이 반대다.**
- 서버 ★**선점 절차는 §10-4-32** — 회수 뒤 선점자가 큐 맨 앞에서 승계된다.

★**증상**
- 마이크를 안 멈추면 → 유예(`T3`) 뒤로는 ★**송신 위반**으로 막히고 `REVOKE(3)` 가 되풀이된다.
- ★**`RELEASE` 를 안 보내면** → 서버가 `T3` 를 꼬박 채운다 — ★**다음 사람(선점자)이 그만큼 기다린다.**
- ★`A` 를 세우면 → ★**원문에 없는 subtype(`10110`)이 나간다.**
- ★`T3` 가 없으면(즉시 회수) → `REVOKE` 가 유실될 때 ★**화자는 말한다고 믿는데 아무도 못 듣는다** — 확인 없이 끊었기 때문이다.

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.S.G.PREVOKE.enter`](#FLR.S.G.PREVOKE.enter) · [`FLR.S.P.PREVOKE.enter`](#FLR.S.P.PREVOKE.enter) · [`FLR.S.P.PREVOKE.T8`](#FLR.S.P.PREVOKE.T8) · [`FLR.C.PERM.rx_REVOKE`](#FLR.C.PERM.rx_REVOKE) · [`FLR.S.G.PREVOKE.rel_revoked`](#FLR.S.G.PREVOKE.rel_revoked) · [`FLR.S.G.PREVOKE.T3`](#FLR.S.G.PREVOKE.T3) · [`FLR.S.P.NP_TAKEN.rx_RTP`](#FLR.S.P.NP_TAKEN.rx_RTP) · [`FLR.S.P.NP_IDLE.rx_RTP_drop`](#FLR.S.P.NP_IDLE.rx_RTP_drop)

<a id="RULE-FLR-MSG-QUEUE"></a>**RULE-FLR-MSG-QUEUE** · 규칙 — `8 FLOOR_QUEUE_POS_REQUEST` / `9 FLOOR_QUEUE_INFO` — 내 차례

★**우선순위는 따로 실리지 않는다** — ★**`3` 이 순번과 우선순위를 한 필드에** 담는다(§10-4-31).
**절차**
- 클라 ★**언제 묻나** — ★**앱이 물을 때**(원문 6.2.4.9.9 — 사용자 요청 · 재동기 §9-2-1 5, `T132` 가 돌지 않을 때만). 주기 폴링 시계는 두지 않는다. `T104`/`C104` 는 그 한 번의 재전송이다(§10-4-24).
★**증상**
- 0부터로 읽으면 화면에 **"0번째"** 가 뜬다.

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.S.P.NP_TAKEN.req_enqueue`](#FLR.S.P.NP_TAKEN.req_enqueue) · [`FLR.S.P.NP_TAKEN.rx_QPOS_REQ`](#FLR.S.P.NP_TAKEN.rx_QPOS_REQ) · [`FLR.S.G.TAKEN.preempt`](#FLR.S.G.TAKEN.preempt) · [`FLR.S.P.NP_IDLE.rx_other`](#FLR.S.P.NP_IDLE.rx_other)

<a id="RULE-FLR-MSG-ACK"></a>**RULE-FLR-MSG-ACK** · 규칙 — `10 FLOOR_ACK` — 받았다   **양방향**

★**`0x1D` 를 되돌리는 이유** — 다방에서는 `pub_room` 의 `DENY` 와 `pub_room` 이 아닌 방에 보낸 요청의 `DENY(101)` 이 동시에
떠 있을 수 있다. `12`(Type)만으로는 ★**어느 방 것의 ACK 인지 서버가 못 갈라** 재전송 정지가 어긋난다.
★**증상** — 빠뜨리면 서버의 도달 관측이 빈다(재전송은 없다 — 원문은 ACK 처리를 구현 선택으로 둔다).

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.C.PREQ.rx_GRANTED`](#FLR.C.PREQ.rx_GRANTED) · [`FLR.C.NOPERM.rx_TAKEN`](#FLR.C.NOPERM.rx_TAKEN) · [`FLR.C.PREQ.rx_DENY`](#FLR.C.PREQ.rx_DENY)

#### 10-4-34. ★발성 감지(`svc=0x02`)는 이 문서 밖이다

<a id="RULE-FLR-NO-VAD"></a>**RULE-FLR-NO-VAD** · 규칙 — ★발성 감지(`svc=0x02`)는 이 문서 밖이다

★★**무전 방에서는 발성 감지를 켜지 않는다.** 화자는 ★**발언권이 준다**(§10-4-30 · §10-4-35).

**왜** — 무전 방에서 RTP 를 내보내는 사람은 ★**발언권을 쥔 한 명뿐**이라, 발성 감지 결과는
★**정의상 발언권 화자와 같다.** 새 사실이 하나도 없고, 대신 둘이 생긴다.

| 섞으면 | |
|---|---|
| ★**통지 권위가 갈린다** | 같은 사실이 두 경로로 온다. 이 문서가 ★**이미 세 번 기각한 형태**다 — HTTP 로 발언권 주기(§4-1-14) · `ROOM_JOIN` 응답에 발언권 싣기(§4-1) · WS bearer 폴백(§10-4-28) |
| ★**첫 음절 구간에서 어긋난다** | 허가(`GRANTED`)와 첫 발성 사이에 ★*"발언권은 쥐었는데 아직 무음"* 구간이 ★**설계상 반드시 있다.** 그때 발언권 축은 *"말하는 중"*, 발성 축은 *"비었다"* 로 ★**두 화면이 다르다.** 말 중간의 짧은 무음에서도 같다 |

★**증상** — 두 축을 함께 그리면 → 말하는 내내 ★**이름이 켜졌다 꺼졌다** 한다.
미디어는 정상이고 ★**화면만 거짓말한다.**

★**회의 제품에서는 반대다.** 발언권이 없으니 ★**관측 말고 화자를 알 방법이 없어** 발성 감지가
유일한 수단이다. ★**그래서 두 경로는 배타이고, 이 문서는 무전 쪽만 정한다**(§1-2-1).
발성 감지를 쓰려면 ★**따로 규격을 본다** — `svc=0x02` 번호는 그쪽이 쓰고 있으므로 비우지 않는다.

★**클라는 `svc=0x02` 로 보내지 않는다.** 보내면 서버가 버린다.

#### 10-4-35. ★화자를 어느 슬롯에 붙이나

<a id="RULE-FLR-SLOT-SPEAKER"></a>**RULE-FLR-SLOT-SPEAKER** · 규칙 — ★화자를 어느 슬롯에 붙이나

발언권 통지는 **누가 말하는가**를 주고, `TrackEntry` 는 **어느 m-line 으로 오는가**를 준다.
★**둘을 잇는 것이 이 절이다.** 슬롯 트랙(`type:"slot"`)에는 `user_id` 가 없으므로(§6-1-1) 이 연결이 없으면
★**소리는 나는데 이름이 안 뜬다.**

<a id="RULE-FLR-ROOM-KEY"></a>**RULE-FLR-ROOM-KEY** · 규칙 — ★다방에서 방을 무엇으로 가르나 — `0x1D` 단일

★★**`0x1D`(방, `room_id` 문자열)가 유일한 방 키다.**

| 메시지 | 몇 방에서 오나 |
|---|---|
| `GRANTED` · `DENY` · `REVOKE` · `QUEUE_INFO` | ★**언제나 한 방** — 발행 방은 하나다(§5-1-1) |
| ★`TAKEN` · `IDLE` | ★**입장한 모든 방**(최대 100, §4-1) |

★**앞의 넷에도 싣는 이유** — `pub_select`(§5-1-2)로 발행 방을 바꾸는 순간
★**날아다니던 메시지가 어느 방 답인지 모호해진다.**

★★**슬롯 SSRC 를 방 키로 쓰지 않는다.** 무전 슬롯은 방마다 상존하고 그 SSRC 는 배정표 줄의 다운링크 SSRC(§8-1-1)로
오지만, ★**그 값에 발언권을 매달지 않는다.**

| 왜 | |
|---|---|
| ★**순서** | ★**발언권은 미디어보다 먼저 흐른다.** 구독 직후 `tracks` 를 아직 못 받은 순간에도 `TAKEN` 이 온다. 그때 슬롯 SSRC 를 모른다 |
| ★★**축 결합** | 슬롯 SSRC 는 ★**미디어 축의 값**이다. 발언권 축이 거기 매달리면 ★**미디어가 정상인데 발언권만 죽는** 형태를 또 만든다(§9-4-13) |
| 이름의 출처 | `room_id` 는 ★**클라·서버·앱이 함께 쓰는 이름**이고 미디어와 무관하다 |

★**원문은 이 자리를 `RTCP SSRC`(제어 채널 식별자)로 채운다** — ★**미디어 SSRC 가 아니라**
세션 수립 시 SDP `mc_floor_ssrc` 로 정해지는 값이고, 통화마다 제어 채널이 따로 있어 그것을
다중화한다(원문 §7). ★**우리는 DC 가 하나뿐이라 그 자리가 없어 TLV 로 대신한다**(§10-4-26).

**재료**

| 어디서 | 무엇 |
|---|---|
| `FLOOR_TAKEN`(`2`) | 말하는 사람 = TLV `4` · ★**방 = TLV `0x1D`** |
| `FLOOR_IDLE`(`5`) | 직전 화자 = TLV `0x1A` · ★**방 = TLV `0x1D`** |
| 보관한 `tracks[]` · 배정표 | `room_id`(§6-1-1) · ★`type:"slot"` · `kind` · `track_id` · 그 `track_id` 의 줄 `mid`(§8-1-2) |

★★**어느 보관본을 뒤지나** — ★**그 DC 가 붙어 있는 미디어 서버의 보관본**이다.
DC 는 ★**미디어 서버당 하나**(§2-1-3)이고 배정표도 ★**서버별**로 갖는다(`mid` 공간이 연결
하나에 하나라 재조립이 서버 단위, §8-1-2). ★**서버 셋에 붙어 있으면 배열도 셋**이다.

**절차**

1. 통지에서 ★**방(`0x1D`)** 을 읽는다. ★**서버는 이것을 항상 싣는다**(§10-4-30).
2. 보관한 `tracks[]` 에서 ★**`room_id` 가 그 방이고 `type` 이 `"slot"` 인 것**을 고른다(§6-1-1 — 구독자는 발행자의 `duplex` 를 모른다).
   `kind` 마다 하나씩이다(audio 1 · video 0~1).
3. 그 트랙의 `track_id` 와 배정표에서 그 `track_id` 의 줄 `mid` 가 ★**그 화자가 지금 흐르는 자리**다. 거기에 이름을 건다. ★화자가 투명 참가자면
   명단에 없어 `metadata` 가 없다 — 걸 이름은 `user_id` 뿐이다(§4-1-3).
4. `FLOOR_IDLE` 을 받으면 같은 방식으로 찾아 ★**이름을 내린다.**
5. ★**`track_id` 를 파싱하지 않는다.** `"ptt-{room_id}-audio"` 처럼 보여도 ★**`room_id` 필드로
   찾는다** — 방 이름에 `-` 가 들어가면 파싱이 깨진다.

★**다방 청취일 때** — 슬롯은 **방마다 따로 있다.** `0x1D` 이 없으면 어느 방 슬롯인지 못 고르므로,
★**`0x1D` 이 이 절차의 입구다.**

| `0x1D` 이 가리키는 방 | 화면 |
|---|---|
| ★**내가 보고 있는 방** | 그 방의 슬롯에 화자 이름을 건다 |
| 다른 방(듣고만 있는 방) | ★**그 방 표시에만** 건다. 지금 보는 화면은 건드리지 않는다 |

★**증상**
- `0x1D` 을 안 쓰고 *"내 방"* 으로 가정하면 → ★**다른 방 사람이 우리 채널에서 말하는 것처럼 보인다.**
  미디어는 정상이라 ★**화면만 거짓말한다.**
- `room_id` 없이 `type:"slot"` 만으로 고르면 → 다방일 때 ★**아무 방 슬롯이나 잡는다.**
- 매핑을 아예 안 하면 → 소리는 나는데 ★**"말하는 중" 표시가 비어 있다.**

→ 구조 — [`FLR.wire`](#FLR.wire) · [`FLR.S.gate.not_in_room`](#FLR.S.gate.not_in_room)

#### 10-4-36. 원천이 더한 것 — 출처 절이 없는 진술

<a id="DEV-FLR-TLV-EXT"></a>**DEV-FLR-TLV-EXT** · 자체 확장

TLV 0x1A 직전 화자 · 0x1B 사유 설명 · 0x1D 방 — 원문 미사용 번호(026~191). DC 하나에 방 여럿을 얹은 대체

*근거: R-FLR-ROOM-TLV*

<a id="DEV-FLR-T9"></a>**DEV-FLR-T9** · 자체 확장

T9(Retry-after) 의 값 3초와 시작 조건 — 원문은 이름만 있다(거절 사유 4 의 설명문). 회수 사유 2 로 회수한 사람에게만 건다

*근거: R-FLR-T9*

<a id="DEV-FLR-DENY-101"></a>**DEV-FLR-DENY-101** · 자체 확장

거절 사유 101 — 발행 방(pub_room)이 아닌 방에 요청(원문 미할당 영역)

*근거: R-FLR-PUBROOM-ORDER*

<a id="DEV-FLR-DENY-102"></a>**DEV-FLR-DENY-102** · 자체 확장

거절 사유 102 — 발언 요청 자격 없음(권한 비트, 원문 미할당 영역). 우선순위를 보기 전에 판정

*근거: R-FLR-PERM-AXIS*

<a id="DEV-FLR-DENY-255"></a>**DEV-FLR-DENY-255** · 자체 확장

미입장 방의 요청은 DENY(255, 0x1B = not_in_room). 그 밖의 프레임은 버린다

*근거: R-FLR-PERM-AXIS*

<a id="DEV-FLR-PERM-REVOKE"></a>**DEV-FLR-PERM-REVOKE** · 자체 확장

권한 비트가 내려가면 — 화자는 원문 회수 절차 그대로 사유 3(T9 없음) · 대기자는 큐에서 빼고 DENY(102) · 그 밖은 다음 요청이 102

*근거: R-FLR-PERM-AXIS*

<a id="DEV-FLR-PUBROOM"></a>**DEV-FLR-PUBROOM** · 자체 확장

FLOOR_REQUEST 와 대기는 발행 방에서만 받는다 — 강제 권위(prefan)가 발언권과 발행 방을 함께 보므로

*근거: R-FLR-PUBROOM-ORDER*

<a id="DEV-FLR-T7-VALUE"></a>**DEV-FLR-T7-VALUE** · 자체 확장

T7 의 값 1초(원문 기본값 없음 — RAN 특성에 따름). C7 10 은 원문 기본값

*근거: R-FLR-NO-T103*

<a id="DEV-FLR-CLIENT-RETX"></a>**DEV-FLR-CLIENT-RETX** · 자체 확장

T101·T100·T104 의 값 0.5초 — 원문은 총 재전송 6초 미만만 정한다

*근거: R-FLR-T9*

<a id="DEV-FLR-QUEUE-MAX"></a>**DEV-FLR-QUEUE-MAX** · 자체 확장

대기줄 상한을 서버 정책으로 둔다(원문 maximum queue length 의 값) — 넘치면 DENY(7)

*근거: R-FLR-CAUSE-SPLIT*

<a id="DEV-FLR-CAUSE-U8"></a>**DEV-FLR-CAUSE-U8** · 자체 확장

거절·회수 사유를 u8 + 0x1B 로 가른다(원문 16비트 + 구문) — SCTP 라 정렬·패딩이 없다

*근거: R-FLR-ROOM-TLV*

<a id="DEV-FLR-ACK-NOSRC"></a>**DEV-FLR-ACK-NOSRC** · 자체 확장

FLOOR_ACK 에 Source 필드를 싣지 않는다 — 신원은 연결이 준다

*근거: R-FLR-ACK-NO-REPLY*

<a id="DEV-FLR-FIRST-NOTIFY"></a>**DEV-FLR-FIRST-NOTIFY** · 자체 확장

처음 알리기의 방아쇠 = READY 마다(원문은 호 참여 시점, 6.3.5.2.2) · unicast 한 번 · T7 없음

*근거: R-FLR-FIRST-NOTIFY*

<a id="DEV-FLR-NO-MCPRIO"></a>**DEV-FLR-NO-MCPRIO** · 자체 확장

mc_priority 협상 미채택 — 우선순위는 요청이 실은 값 그대로

*근거: R-FLR-PREEMPT-OPEN*

<a id="DEV-FLR-PREEMPT-RELATIVE"></a>**DEV-FLR-PREEMPT-RELATIVE** · 자체 확장

선점 판정 = 요청 우선순위 > 화자의 허가된 우선순위(높은 값 우선의 상대 비교) — 원문 4.1.1.4 로컬 정책 · 6.3.5.4.4 5 의 화자가 선점 등급이 아님 조건 대신. 큐에 선점 요청이 이미 있거나 회수 유예 중이면 그 선점 요청은 버린다(원문 빈칸 — 6.3.4.5 에 절 없음)

*근거: R-FLR-PREEMPT-OPEN*

<a id="DEV-FLR-PREQ-TAKEN-STAY"></a>**DEV-FLR-PREQ-TAKEN-STAY** · 자체 확장

pending Request 에서 남의 TAKEN 을 받아도 남는다 — 클라는 자기 요청이 선점인지 모르고, 큐를 항상 켜므로 서버가 모든 요청에 답한다(원문 6.2.4.4.11 7 은 선점 요청일 때만 남는다)

*근거: R-FLR-PREQ-STAY*

<a id="DEV-FLR-PUB-AWAY"></a>**DEV-FLR-PUB-AWAY** · 자체 확장

pub_select·pub_deselect·ROOM_LEAVE = 그 방 RELEASE·대기 철회(통지 없음) · 클라는 응답 뒤 has no permission 으로 내리고 타이머를 멈춘다 · 회수 중인 사람의 이탈엔 아무것도 하지 않는다(T3·T1 이 닫는다)

*근거: R-FLR-PUBROOM-ORDER*

<a id="DEV-FLR-DC-DOWN"></a>**DEV-FLR-DC-DOWN** · 자체 확장

DC 만 끊김 — 발언권 상태 유지 · 믿을 수 없음 표시 · 다시 열기는 클라 재량, 소진하면 그 서버 재구축

*근거: R-FLR-DC-ONLY-DOWN*

<a id="DEV-FLR-RECVONLY-TRIGGER"></a>**DEV-FLR-RECVONLY-TRIGGER** · 자체 확장

거절 사유 5 의 방아쇠 = 반이중 발행 트랙 없음(원문은 수신 전용 설정·effective priority) — 무시하지 않고 거절

*근거: R-FLR-DENY-RECVONLY*

<a id="DEV-FLR-T132-SCOPE"></a>**DEV-FLR-T132-SCOPE** · 자체 확장

queued 를 나가는 전이는 T104 와 함께 T132 도 멈춘다(T104 는 원문 표 Table 11.1.1-1 그대로)

*근거: R-FLR-T132*

<a id="DEV-FLR-DENY-2-6"></a>**DEV-FLR-DENY-2-6** · 자체 확장

거절 사유 2·6 을 내지 않는다(원문도 보내는 절차가 없다) · 사유 1 도 내지 않는다(R-QUEUE-ALWAYS)

*근거: R-FLR-QUEUE-ALWAYS*

<a id="DEV-FLR-REGRANT-FWD"></a>**DEV-FLR-REGRANT-FWD** · 자체 확장

U: permitted 참가자가 판정 로직의 GRANTED(재허가 6.3.4.4.8 · T20 재전송 6.3.4.4.9)를 클라로 넘긴다 — ★원문 6.3.5.5 에 Send Floor Granted 절이 없어 기본 규칙(6.3.5.1)대로면 버려진다. 판정 로직 절이 보내라 한 뜻을 따른 빈칸 채움

*근거: R-FLR-REGRANT*

<a id="DEV-FLR-TAKEN-ACK"></a>**DEV-FLR-TAKEN-ACK** · 자체 확장

TAKEN 에 A 비트를 세우고(원문 6.3.5.3.3 의 may) ACK 이 없으면 T21(자체 확장 이름 · 1초 · C21 3번)마다 다시 보낸다. 참가자가 Floor Taken 을 떠나거나 ACK(Message Type 2)이 오면 멈춘다 — ACK 이 없을 때의 처리는 원문이 구현 선택으로 둔다(6.3.5.3.3 NOTE)

*근거: R-FLR-TAKEN-ACK*

<a id="DEV-FLR-ACK-DROP"></a>**DEV-FLR-ACK-DROP** · 자체 확장

서버는 FLOOR_ACK 에 답하지 않고 도달 관측에만 쓴다(원문은 ACK 처리를 구현 선택으로 둔다, 6.3.5.3.5 NOTE) · 직접 허가·거절은 재전송하지 않는다

*근거: R-FLR-ACK-NO-REPLY R-FLR-NO-RETX-DIRECT*

<a id="ASM-FLR-SPEAKER-RTP"></a>**ASM-FLR-SPEAKER-RTP** · 가정

has permission 클라는 RTP 를 계속 보낸다 — T1 은 그 화자 클라가 has permission 이 아닐 때만 만료한다

*식: K1 · 조건: 미디어 건강*

<a id="ASM-FLR-T1-OVER-T132"></a>**ASM-FLR-T1-OVER-T132** · 가정

T1(4초)은 승계 의지 대기 T132(2초) 동안 만료하지 않는다

*식: K1b · 조건: T1 > T132*

<a id="ASM-FLR-T20-OVER-RTP"></a>**ASM-FLR-T20-OVER-RTP** · 가정

승계 T20 은 화자가 has permission 이면 돌지 않는다 — 그 RTP 가 1초 안에 T20 을 멈춘다

*식: K1c · 조건: T20 > RTP 간격*

<a id="ASM-FLR-TIMER-EMPTY"></a>**ASM-FLR-TIMER-EMPTY** · 가정

타이머 만료는 통이 비었을 때만 일어난다 — 가장 짧은 타이머(0.5초)가 RTT 보다 길다

*식: K2 · 조건: RTT < 0.5초*

<a id="ASM-FLR-RETX-BEFORE-EXPIRE"></a>**ASM-FLR-RETX-BEFORE-EXPIRE** · 가정

T3·T1 은 같은 대상의 T8·T20 재전송이 다 돈 뒤에만 만료한다

*식: K2b · 조건: C8×T8 ≤ T3 · C20×T20 ≤ T1*

<a id="ASM-FLR-T8-OVER-T132"></a>**ASM-FLR-T8-OVER-T132** · 가정

T8 마지막 재전송은 받는 이가 승계 의지 대기 중이면 미뤄진다

*식: K2c · 조건: C8×T8(3초) > T132(2초) — 어디에도 적혀 있지 않던 성립 조건(N3). ★선점은 허가 뒤에만 오므로 첫 REVOKE 는 허가보다 늦고, 마지막 재전송은 첫 REVOKE + C8×T8 이다 — 승계 의지 대기(T132)는 GRANTED 를 받은 순간부터라 마지막 REVOKE 는 대기가 끝난 뒤 닿는다. 남는 것은 두 메시지의 지연 차이가 C8×T8 − T132(1초)보다 작다는 망의 사실이다. 값을 바꿀 때 이 산수가 생성기 검사로 걸린다(가정-보증 대장)*

<a id="ASM-FLR-RTP-INSTANT"></a>**ASM-FLR-RTP-INSTANT** · 가정

has permission 클라의 RTP 는 곧바로 서버 참가자에 닿는다

*식: K3 · 조건: RTP 지연은 범위 밖*

<a id="ASM-FLR-HUMAN-SPEED"></a>**ASM-FLR-HUMAN-SPEED** · 가정

사람 속도 — 한 사용자의 두 동작 사이에 통이 한 번은 빈다

*식: K4 · 조건: 사람 반응 > RTT (손잡이 HumanSpeed)*

<a id="ASM-FLR-STALE-GUARD"></a>**ASM-FLR-STALE-GUARD** · 가정

지난 주기 차단 — 누를 때 자기 통이 비어 있고 자기 REQUEST 가 떠 있으면 떼지 않는다

*식: K5 · 조건: 손잡이 StaleGuard — N 경주 판정 때는 끈다*

<a id="PROP-FLR-ONE-SPEAKER"></a>**PROP-FLR-ONE-SPEAKER** · 성질

두 사람이 함께 has permission 이 아니다

*식: H1 · 깨지면: 둘이 말한다고 믿는다*

<a id="PROP-FLR-SPEAKER-IS-HOLDER"></a>**PROP-FLR-SPEAKER-IS-HOLDER** · 성질

has permission 클라가 있으면 서버의 허가 보유자는 그 사람이거나 없다

*식: H2 · 깨지면: 말하는데 아무도 못 듣고 남이 말한다*

<a id="PROP-FLR-HOLDER-SPEAKS"></a>**PROP-FLR-HOLDER-SPEAKS** · 성질

서버 화자는 has permission 이거나 승계 의지 대기 중이다

*식: H3 · 깨지면: 빈 채널이 T1 까지 묶인다*

<a id="PROP-FLR-ARB-PART-AGREE"></a>**PROP-FLR-ARB-PART-AGREE** · 성질

판정 로직과 참가자 상태기가 맞는다(언제나)

*식: H4 · 깨지면: 서버 안이 갈린다*

<a id="PROP-FLR-HOLDER-EXISTS"></a>**PROP-FLR-HOLDER-EXISTS** · 성질

has permission 클라가 있으면 서버의 허가 보유자가 있다

*식: H5 · 깨지면: 원문 성질 확인용(6.3.5.5.4 · 6.3.5.3.8 — T1 회수는 클라에 알려지지 않는다)*

<a id="PROP-FLR-PERM-WANTS"></a>**PROP-FLR-PERM-WANTS** · 성질

has permission 클라는 사용자가 말하려 한다(언제나)

*식: H6 · 깨지면: 수락하지 않은 마이크가 열린다*

<a id="PROP-FLR-QUEUED-IN-QUEUE"></a>**PROP-FLR-QUEUED-IN-QUEUE** · 성질

queued 클라는 서버 큐에 있거나 서버 화자다

*식: H7 · 깨지면: 기다리는데 차례가 안 온다*

<a id="PROP-FLR-SHOWN-IS-HOLDER"></a>**PROP-FLR-SHOWN-IS-HOLDER** · 성질

듣는 클라의 화자 표시는 서버의 허가 보유자와 같다

*식: H9 · 깨지면: 말하는 사람이 있는데 화면에 없거나 다른 사람이 보인다(소리는 들린다)*

<a id="PROP-FLR-LEDGER-BOUNDS"></a>**PROP-FLR-LEDGER-BOUNDS** · 성질

내는 사유는 원장 안 · 대기 정보 순번은 1 이상(254·255 특수) · Idle 이면 큐가 비었다 · 큐는 상한 안(언제나)

*식: H8 · 깨지면: 원장 밖 값이 나간다*

<a id="PROP-FLR-SEND-NEEDS-FLOOR"></a>**PROP-FLR-SEND-NEEDS-FLOOR** · 성질

반이중 트랙이 보내는 클라는 has permission 이다(언제나) — 발언권 모형과 발행 모형(PUB)을 합친 판에서 잰다

*식: H10 · 깨지면: 발언권을 잃었는데 마이크 RTP 가 계속 나간다(서버는 허가 없는 RTP 로 REVOKE 를 되풀이한다)*

<a id="PROP-FLR-FLOOR-SENDS"></a>**PROP-FLR-FLOOR-SENDS** · 성질

반이중 발행을 마친 has permission 클라는 반이중 트랙이 보낸다(언제나) — 합친 판에서 잰다

*식: H11 · 깨지면: 발언권을 쥐었는데 RTP 가 나가지 않아 T1 이 회수한다(ASM-FLR-SPEAKER-RTP 가 무너진다)*

<a id="PROP-FLR-REQUEST-ENDS"></a>**PROP-FLR-REQUEST-ENDS** · 성질

요청은 끝난다 — Pending Request 는 결국 has permission · queued · has no permission 중 하나가 된다(재전송이 바닥나면 포기 — RULE-FLR-PROC-PREQ-T101 · DEV-FLR-CLIENT-RETX · ★활성 성질 — 통 · 타이머 · RTP · 승계 의지에 공정성 · 사람 조작 · 결정자 · 장애는 결국 멎는다)

*식: FL1(활성 판 floor live) · 깨지면: 버튼을 눌렀는데 요청 중 표시로 영영 멈춘다 — 응답도 포기도 오지 않는다*

<a id="PROP-FLR-RELEASE-ENDS"></a>**PROP-FLR-RELEASE-ENDS** · 성질

반환은 끝난다 — Pending Release 는 결국 has no permission 이 된다(재전송이 바닥나면 포기 — RULE-FLR-PROC-PREL-CONFIRM · DEV-FLR-CLIENT-RETX · ★활성 성질)

*식: FL2(활성 판 floor live) · 깨지면: 버튼을 뗐는데 반환 중 표시로 영영 멈춰 다시 누를 수 없다*

<a id="PROP-FLR-SHOWN-CONVERGES"></a>**PROP-FLR-SHOWN-CONVERGES** · 성질

조작이 멎으면 결국 늘 듣는 클라의 화자 표시 = 서버의 허가 보유자(PROP-FLR-SHOWN-IS-HOLDER 의 활성판 — ★그쪽이 서는 판에서만 잰다 · 통이 순서를 지키고 ACK 이 있고 유실이 없는 판)

*식: FL3(활성 판 floor live_show) · 깨지면: 말이 끝났는데 화면에 화자가 남는다 · 말하는 사람이 있는데 화면이 비어 있다*

<a id="RACE-FLR-TAP-OVERTAKE"></a>**RACE-FLR-TAP-OVERTAKE** · 알려진 경주

탭하고 바로 뗌 — RELEASE 가 REQUEST 를 추월하면 반환 확인 뒤 늦은 요청이 허가된다

*막는 가정: ASM-FLR-HUMAN-SPEED · 깨는 성질: PROP-FLR-HOLDER-SPEAKS*

<a id="RACE-FLR-T101-OVERTAKE"></a>**RACE-FLR-T101-OVERTAKE** · 알려진 경주

T101 재전송 REQUEST 를 뗌이 추월 — 회수 유예 중 선점 요청을 버리는 정석이 재전송을 부른다

*막는 가정: ASM-FLR-STALE-GUARD · 깨는 성질: PROP-FLR-HOLDER-SPEAKS*

<a id="RACE-FLR-LATE-GRANTED"></a>**RACE-FLR-LATE-GRANTED** · 알려진 경주

지난 요청의 늦은 GRANTED(직접 허가 또는 T20 재전송)가 새 요청의 답으로 읽힌다 — 요청 식별자가 없다

*막는 가정: ASM-FLR-HUMAN-SPEED ASM-FLR-STALE-GUARD · 깨는 성질: PROP-FLR-ARB-PART-AGREE*

<a id="RACE-FLR-UNICAST-OVERTAKES-BROADCAST"></a>**RACE-FLR-UNICAST-OVERTAKES-BROADCAST** · 알려진 경주

번호 없는 unicast(QINFO · DENY · REVOKE)가 그보다 먼저 보낸 방송(TAKEN · IDLE)을 앞지르면 대기 · 화자 표시가 서버와 어긋난다 — ★받아들인다: MBCP 는 UDP 위의 프로토콜이고 원문은 유실 · 순서 바뀜을 타이머 재전송 · 재요청 · 멱등 응답으로 견딘다. 데이터 채널은 UDP 와 같은 조건(순서 없음 · 한 번만 — RULE-LINK-SDP-DC-OPENER)이라 몇 ms 간격으로 보낸 두 패킷이 한 경로에서 뒤바뀌어야 일어난다

*비고: 검증 모형은 아무렇게나 순서를 바꾸는 통(Fifo FALSE)이라 이 경주를 찾는다(h9s_core) — CAND-FLR-UNICAST-OVERTAKES-BROADCAST ③(20261010) · 깨는 성질: PROP-FLR-QUEUED-IN-QUEUE PROP-FLR-SHOWN-IS-HOLDER*

<a id="RACE-FLR-T132-PREEMPT"></a>**RACE-FLR-T132-PREEMPT** · 알려진 경주

queued 승계 의지 대기 중의 선점 — REVOKE 를 queued 가 버린다(6.2.4.9 에 절 없음)

*막는 가정: ASM-FLR-T8-OVER-T132 · 깨는 성질: PROP-FLR-QUEUED-IN-QUEUE*

<a id="RACE-FLR-LATE-RELEASE"></a>**RACE-FLR-LATE-RELEASE** · 알려진 경주

늦은 T100 재전송 RELEASE 가 새 대기를 뺀다 — 직전 화자는 IDLE 을 안 받아(6.3.5.5.4) 반환 확인을 늘 재전송으로 받는다. 시범 F7 의 다른 입구

*막는 가정: ASM-FLR-STALE-GUARD · 깨는 성질: PROP-FLR-QUEUED-IN-QUEUE*

<a id="RACE-FLR-LATE-CONFIRM-IDLE"></a>**RACE-FLR-LATE-CONFIRM-IDLE** · 알려진 경주

늦은 확인 IDLE(T100 재전송에 대한 unicast)이 새 대기 클라를 내린다(6.2.4.9.8)

*막는 가정: ASM-FLR-STALE-GUARD · 깨는 성질: PROP-FLR-HOLDER-SPEAKS*

<a id="RACE-FLR-LATE-BCAST-IDLE"></a>**RACE-FLR-LATE-BCAST-IDLE** · 알려진 경주

대기 응답보다 먼저 보낸 방송 IDLE 이 나중에 닿으면 queued 클라가 대기를 버린다(6.2.4.9.8) — 서버 큐엔 남는다. 시범 F8 의 queued 판

*비고: (막는 가정 없음) · 깨는 성질: PROP-FLR-HOLDER-SPEAKS*

<a id="RACE-FLR-DC-STALE"></a>**RACE-FLR-DC-STALE** · 알려진 경주

DC 만 끊긴 사이 승계·선점·회수가 지나가면 다시 열린 뒤에도 클라가 옛 상태에 굳는다(queued 고착 · 회수 모른 채 has permission) — 처음 알리기는 화자만 고치고, 순번 질의의 254(큐에 없음)에도 원문은 queued 에 머문다(6.2.4.9.7)

*비고: (DC 끊김 판에서만 — R-FLR-DC-ONLY-DOWN 이 드러난 것) · 깨는 성질: PROP-FLR-QUEUED-IN-QUEUE PROP-FLR-ONE-SPEAKER*

<a id="RACE-FLR-LOST-GRANTED-PREEMPT"></a>**RACE-FLR-LOST-GRANTED-PREEMPT** · 알려진 경주

승계 GRANTED 유실 + T20 재전송 전 선점 → queued 클라가 REVOKE 를 버리고 굳는다

*비고: (유실 0 이면 없다) · 깨는 성질: PROP-FLR-QUEUED-IN-QUEUE*

<a id="DEV-FLR-T132-DISPLAY"></a>**DEV-FLR-T132-DISPLAY** · 자체 확장

대기 중(queued)에 승계 허가(GRANTED)를 받으면 화자 표시를 내린다 — 원문 6.2.4.9.4 는 표시를 정하지 않고, 직전 화자에게는 IDLE 이 가지 않으므로(6.3.5.5.4) 내리지 않으면 T132 동안 앞 화자가 화면에 남는다

*원문: 6.2.4.9.4 · 근거: R-FLR-T132*

<a id="DEV-FLR-SEQ-DISPLAY"></a>**DEV-FLR-SEQ-DISPLAY** · 자체 확장

화자 표시를 바꾸는 `TAKEN` · `IDLE` 은 메시지 일련번호(TLV `8`)가 마지막으로 표시에 쓴 번호보다 새것일 때만 표시에 쓴다(감김 0~65535 를 고려해 견준다). 상태 전이는 원문대로 둔다 — 견주는 것은 화자 표시 하나다(대기 중 낡은 `IDLE` 은 예외 — DEV-FLR-STALE-IDLE-STAY). 낡은 번호의 `TAKEN` 에도 ACK 은 한다. 데이터 채널이 순서를 보장하지 않아(`ordered: false`, §9-4-13) 승계 뒤에 옛 `TAKEN` 이 닿을 수 있다

*원문: §10-4-31 "클라는 이 번호로 메시지를 버리지 않는다" — 원문 on-network 클라는 이 번호를 보지 않는다(6.2.4.3.2). 이 확장은 메시지를 버리지 않고 표시만 가른다 · 근거: R-FLR-SEQ-DISPLAY*

<a id="DEV-FLR-T21-SAME-SEQ"></a>**DEV-FLR-T21-SAME-SEQ** · 자체 확장

`T21` 재전송은 처음 보낸 그 `TAKEN` 의 일련번호를 다시 싣는다(같은 메시지의 재전송이다 — "보낼 때마다 1 증가"의 예외). 반환 중(`pending Release`) 클라는 이미 본 번호의 `TAKEN` 을 반환 확인으로 쓰지 않는다(ACK 만 한다)

*원문: §10-4-31 일련번호 "보낼 때마다 1 증가" · 6.2.4.6.5 · 근거: R-FLR-T21-SAME-SEQ*

<a id="DEV-FLR-PREQ-IDLE-DISPLAY"></a>**DEV-FLR-PREQ-IDLE-DISPLAY** · 자체 확장

요청 중(`pending Request`)에 `IDLE` 을 받으면 상태는 그대로 두고 화자 표시만 내린다(일련번호가 새것일 때)

*원문: 6.2.4.4 — pending Request 에 Floor Idle 절이 없다(원문 그대로면 IDLE 을 버려 앞 화자가 남는다) · 근거: R-FLR-PREQ-IDLE-DISPLAY*

<a id="DEV-FLR-PRESS-DROP"></a>**DEV-FLR-PRESS-DROP** · 자체 확장

누름은 `has no permission` 에서만 요청이 된다. 그 밖의 상태 — 요청 중(`pending Request`) · 발언 중(`has permission`) · 반환 중(`pending Release`) · 대기 중(`queued`) — 에서 누르면 클라는 그 누름을 버린다(보내는 것도 상태도 바뀌지 않는다). 반환 중에 다시 말하려면 반환이 끝난 뒤(`has no permission`) 다시 누른다

*원문: 6.2.4.4 · 6.2.4.5 · 6.2.4.6 · 6.2.4.9 — 그 상태들에 누름(PTT 버튼 눌림) 절이 없다 · 근거: R-FLR-PRESS-DROP*

<a id="DEV-FLR-GRANTED-SEQ"></a>**DEV-FLR-GRANTED-SEQ** · 자체 확장

`GRANTED` 는 메시지 일련번호(TLV `8`)를 싣는다 — 그 순간 방의 마지막 번호 그대로(올리지 않는다). 클라는 그 번호가 새것이면 본 번호로 삼는다 — 그 뒤 닿는 더 옛 `TAKEN` · `IDLE` 은 낡은 것으로 걸러진다(DEV-FLR-SEQ-DISPLAY)

*원문: 8.2.5 Floor Granted 에 Message Sequence Number 필드가 없다(8.2.3.10 은 Taken · Idle 만 묶는다) · 근거: R-FLR-GRANTED-SEQ*

<a id="DEV-FLR-ACK-SEQ"></a>**DEV-FLR-ACK-SEQ** · 자체 확장

`ACK` 은 받은 메시지에 일련번호가 실려 왔으면 그 번호를 되돌려 싣는다. 서버는 `TAKEN` 의 ACK 을 그 사람에게 마지막으로 보낸 `TAKEN` 의 번호와 같을 때만 확인으로 쓴다(`T21` 을 멈춘다) — 다르면 옛 `TAKEN` 의 늦은 ACK 이라 재전송을 이어 간다

*원문: 8.2.13 Floor Ack 은 Source · Message Type · Track Info · Location 뿐이다(일련번호가 없다) · 근거: R-FLR-ACK-SEQ*

<a id="DEV-FLR-STALE-IDLE-STAY"></a>**DEV-FLR-STALE-IDLE-STAY** · 자체 확장

대기 중(`queued`) 클라는 낡은 번호의 `IDLE` 로 대기를 떠나지 않는다 — 상태도 화자 표시도 그대로 둔다(같은 번호 이후의 새 `IDLE` 은 원문대로 대기를 떠난다)

*원문: 6.2.4.9.8 — Floor Idle 을 받으면 `has no permission` 으로 간다(번호를 보지 않는다) · 근거: R-FLR-STALE-IDLE-STAY*

<a id="DEV-FLR-DOWN-FRESH"></a>**DEV-FLR-DOWN-FRESH** · 자체 확장

재구축(세션 전체 · 그 서버)으로 그 방 발언권을 내릴 때 클라의 그 방 발언권 상태를 처음 값으로 돌린다 — 상태 `has no permission` · 누름 의지 · 발행 방(`isPub`) · 반이중 발행(`hasHalf`) · 화자 표시(`shown`) · 표시에 쓴 일련번호(`seen`)

*원문: §3-2-3 1-1 · RULE-LINK-RECOVERY-REBUILD — 발언권을 `has no permission` 으로 내린다는 것만 정한다 · 근거: R-FLR-DOWN-FRESH*

<a id="DEV-FLR-JOIN-FRESH"></a>**DEV-FLR-JOIN-FRESH** · 자체 확장

방에 들 때(처음 · 다시 입장 — ROOM 효과 `floor.join`) 클라의 그 방 발언권 칸을 처음 값으로 둔다 — 요청 없음 · 누름 의지 없음 · 발행 방 아님(`isPub` — 전환 응답이 따로 온다) · 화자 표시 없음(READY 의 첫 알림이 채운다 — DEV-FLR-FIRST-NOTIFY) · ★표시에 쓴 일련번호(`seen`)는 둔다 — 같은 서버의 같은 방이라 번호가 이어진다(재구축의 DEV-FLR-DOWN-FRESH 와 다른 점). 서버는 참가자 기록을 원문 6.3.5.2.2 대로 만든다(`FLR.S.ws.join`)

*근거: R-FLR-JOIN-FRESH*

<a id="DEV-FLR-ROOM-GONE"></a>**DEV-FLR-ROOM-GONE** · 자체 확장

방을 잃는 경우 — 강퇴 · 방 닫힘 · `RESUME` 스냅샷에 없는 방 — 는 그 방 `ROOM_LEAVE` 와 같다: 반환하고 큐가 있으면 승계 · 대기 철회(서버 `FLR.S.ws.leave`) · 클라는 그 방을 `has no permission` 으로 내린다(`FLR.C.any.leave`)

*원문: RULE-FLR-SERVER-EVENTS — `pub_deselect` · `ROOM_LEAVE` 만 정한다 · 근거: R-FLR-ROOM-GONE*

### 10-5. 근거 · 기각 — 규격이 아니다

> 계약 본문(구조 · 동작 · 주제)의 규칙이 왜 그런가(R) · 무엇을 하지 않기로 했나(REJ). 소절 머리는 그 근거가 붙은 계약 자리다. 계약 판에는 싣지 않는다.

#### 10-5-1. §10-4-2 상태를 앱에 알린다

<a id="R-FLR-APP-EXPOSE-SYMPTOM"></a>**R-FLR-APP-EXPOSE-SYMPTOM** · 근거

★**발언권을 안 노출하면** — 앱이 ★**PTT 버튼을 만들 수 없다.** 누를 수 있는지, 대기 중인지,
남은 시간이 얼마인지가 전부 이 축이다. ★**미디어 지표로는 하나도 안 나온다**(§9-4-13).

#### 10-5-2. §10-4-8 `2 FLOOR_TAKEN` 을 받았다 (`pending Request`) — 원문 6.2.4.4.11

<a id="R-FLR-PROC-PREQ-TAKEN-WHY"></a>**R-FLR-PROC-PREQ-TAKEN-WHY** · 근거

- 서버가 대기(`QUEUE_INFO`)·허가·거절로 답한다(큐를 항상 켜므로 모든 요청에 답이 온다, §10-4-32).

#### 10-5-3. §10-4-11 `T132` 가 만료했다 (`queued`) — 원문 6.2.4.9.13

<a id="R-FLR-PROC-QUEUED-T132-WHY"></a>**R-FLR-PROC-QUEUED-T132-WHY** · 근거

- ★**차례가 왔는데 사용자가 자리를 비웠다.**

#### 10-5-4. §10-4-13 `1 FLOOR_GRANTED` 를 `has permission` 에서 받았다 — 원문 6.2.4.5.5

<a id="R-FLR-PROC-PERM-GRANTED-WHY"></a>**R-FLR-PROC-PERM-GRANTED-WHY** · 근거

- ★**어디서 오나** — 큐 승계 `GRANTED` 의 `T20` 재전송이 발화를 시작한 뒤에 닿은 것, 또는 내 `T101` 재요청에 대한 재허가(§10-4-30).

#### 10-5-5. §10-4-15 `T104` 를 소진했다 (`queued`) — 원문 6.2.4.9.11

<a id="R-FLR-PROC-QUEUED-T104-WHY"></a>**R-FLR-PROC-QUEUED-T104-WHY** · 근거

- ★**질의가 서버에 닿지 않았다는 뜻이다** — 서버는 발언권이 잡혀 있는 동안 큐 소속과 무관하게 답한다(큐에 없으면 순번 `254`, §10-4-30). 원문대로 대기를 접는다.

#### 10-5-6. §10-4-16 `3 FLOOR_DENY` 를 받았다 (`pending Request` · `queued`) — 원문 6.2.4.4.4 · 6.2.4.9.5

<a id="R-FLR-PROC-DENY-WHY"></a>**R-FLR-PROC-DENY-WHY** · 근거

- ★`queued` 에서 오는 `DENY` 는 서버가 큐에서 뺀 것이다(§10-4-32 권한 비트 회수).

#### 10-5-7. §10-4-21 `2 FLOOR_TAKEN` / `5 FLOOR_IDLE` 을 받았다 (`has no permission` · `queued`) — 원문 6.2.4.3.2·.3 · 6.2.4.9.3·.8

<a id="R-FLR-PROC-TAKEN-IDLE-WHY"></a>**R-FLR-PROC-TAKEN-IDLE-WHY** · 근거

- 서버는 큐가 비어 있을 때만 `IDLE` 을 보내므로(§10-4-30) 내 대기는 이미 없다.
- (원문 6.2.4.5.7·.8 — 원문은 dual floor·multi-talker 일 때만 다루고 둘 다 미채택이다, §10-4-27)

#### 10-5-8. §10-4-22 같은 것을 `pending Release` 에서 받았다 (★반환의 확인) — 원문 6.2.4.6.4 · 6.2.4.6.5

<a id="R-FLR-PROC-PREL-CONFIRM-WHY"></a>**R-FLR-PROC-PREL-CONFIRM-WHY** · 근거

- ★**이것이 반환의 확인이다** — 내가 보낸 `RELEASE` 의 결과물(`IDLE`, 또는 다음 화자의 `TAKEN`, 또는 대기 철회에 대한 서버의 `TAKEN`)이 돌아온 것이다. `RELEASE` 에는 ACK 이 없으므로(§10-4-30) ★**이 수신이 유일한 확인 수단**이다.

#### 10-5-9. §10-4-31 TLV 필드

<a id="R-FLR-TLV-MATRIX-WHY"></a>**R-FLR-TLV-MATRIX-WHY** · 근거

- 내 이름을 나에게 알릴 이유가 없다
- 남의 남은 시간이 필요 없다

#### 10-5-10. §10-4-32 발언권 규칙 — 자체 값과 절차

<a id="R-FLR-T9-VALUE-WHY"></a>**R-FLR-T9-VALUE-WHY** · 근거

- 자기 잘못이 아닌데 벌주는 꼴이 된다.

<a id="R-FLR-PERM-REVOKE-ENFORCE-WHY"></a>**R-FLR-PERM-REVOKE-ENFORCE-WHY** · 근거

- 자기 잘못이 아니다
- 대기 철회(§10-4-12)와 달리 자기 요청이 아니라 남의 결정으로 빠진 것이라 응답으로 알 길이 없다

#### 10-5-11. §10-4-33 메시지별 규격

<a id="R-FLR-MSG-REQUEST-WHY"></a>**R-FLR-MSG-REQUEST-WHY** · 근거

- 그래서 속도 제한이 따로 필요 없다
- ★**버튼을 두 번 눌러도 발언이 끊기지 않는다.**

<a id="R-FLR-MSG-GRANTED-WHY"></a>**R-FLR-MSG-GRANTED-WHY** · 근거

- ★**줄 서 놓고 자리를 비우지 않는다.**

<a id="R-FLR-MSG-RELEASE-WHY"></a>**R-FLR-MSG-RELEASE-WHY** · 근거

- 내가 재전송하므로 서버 ACK 이 필요 없다.

<a id="R-FLR-MSG-IDLE-WHY"></a>**R-FLR-MSG-IDLE-WHY** · 근거

- ★**빈 화면이 깜빡이지 않는다.**
- ★**무전은 다음 사건이 몇 분 뒤일 수 있어** 한 번만 보내면 유실 시 ★**직전 화자 이름이 화면에 그대로 남는다.**

<a id="R-FLR-MSG-QUEUE-WHY"></a>**R-FLR-MSG-QUEUE-WHY** · 근거

- 클라가 `T104` 로 다시 묻는다.

#### 10-5-12. §10-4-36 원천이 더한 것 — 출처 절이 없는 진술

<a id="R-FLR-ROOM-TLV"></a>**R-FLR-ROOM-TLV** · 근거

DC 하나에 방 여럿이 얹혀 원문의 RTCP SSRC 다중화 자리가 없다 — 0x1D 가 그 자리다

*없으면: 0x1D 를 빼면 다방에서 엉뚱한 방의 발언권이 바뀌고 다른 방 사람이 우리 채널에서 말하는 것처럼 보인다(미디어는 정상)*

<a id="R-FLR-PUBROOM-ORDER"></a>**R-FLR-PUBROOM-ORDER** · 근거

WS(게이트웨이 경유)와 DC(직행)는 다른 전송로라 순서가 없다. 서버가 발행 방을 검사하므로 전환 응답 뒤에 요청한다

*없으면: DC 가 먼저 닿으면 DENY(101)*

<a id="R-FLR-PERM-AXIS"></a>**R-FLR-PERM-AXIS** · 근거

권한 비트는 결정자가 방마다 내리고 올리는 자격이다 — 우회 경로가 없어야 긴급도 막힌다

*없으면: 비트를 안 보면 자격 없는 사람이 우선순위로 채널을 쥔다*

<a id="R-FLR-TAP-RELEASE"></a>**R-FLR-TAP-RELEASE** · 근거

허가 전에 뗐을 때의 전이가 있어야 짧은 탭이 채널을 잠그지 않는다

*없으면: 없으면 탭했다 뗀 손짓마다 채널이 허가되고 T1 이 다 돌 때까지 잠긴다*

<a id="R-FLR-T132"></a>**R-FLR-T132** · 근거

큐 승계 허가는 사용자가 자리에 있는지 모르는 상태다 — 의지 표시 창을 둔다. hold 면 버튼이 눌린 것이 표시, toggle 이면 다시 누름

*없으면: 없으면 사용자가 자리를 비운 사이 채널이 T2 만큼 묶인다*

<a id="R-FLR-REVOKE-MIC"></a>**R-FLR-REVOKE-MIC** · 근거

회수를 받으면 화자는 마이크를 끄고 RELEASE 를 보낸다(원문 6.2.4.5.4)

*없으면: 늦게 끄면 유예(T3) 뒤 송신 위반으로 REVOKE(3) 이 되풀이된다*

<a id="R-FLR-REVOKE-RELEASE"></a>**R-FLR-REVOKE-RELEASE** · 근거

회수 응답 RELEASE 가 서버 유예(T3)를 끝내고 T8 을 멈춘다

*없으면: 안 보내면 선점자가 T3 를 꼬박 기다린다*

<a id="R-FLR-CAUSE-SPLIT"></a>**R-FLR-CAUSE-SPLIT** · 근거

사유마다 사용자 처방이 반대다 — DENY 7(줄도 못 선다) 대 4(3초 뒤면 된다), REVOKE 4(또 뺏긴다) 대 2(다시 누르면 된다)

*없으면: 사유를 뭉치면 화면이 같아 사용자가 할 일을 모른다*

<a id="R-FLR-QUEUE-ALWAYS"></a>**R-FLR-QUEUE-ALWAYS** · 근거

큐는 항상 켠다 — 우선순위·큐 기반 제어가 규율형 무전의 본질이다. 점유 중 요청은 대기 아니면 만석이다

*없으면: 사유 1 을 내면 줄을 설 수 있는 사람이 포기한다*

<a id="R-FLR-NO-T4"></a>**R-FLR-NO-T4** · 근거

발언 없는 통화의 정리는 방 수명(참가자 기준 TTL)이 맡는다

*없으면: 장치가 둘이면 정리 시점이 갈린다*

<a id="R-FLR-NO-T103"></a>**R-FLR-NO-T103** · 근거

화자 표시 내림은 IDLE 의 T7·C7 재전송이 지킨다(T103 미채택)

*없으면: IDLE 을 한 번만 보내면 유실 때 직전 화자 이름이 화면에 남는다*

<a id="R-FLR-T9"></a>**R-FLR-T9** · 근거

T2 로 회수해도 곧바로 다시 누르면 한 사람이 채널을 계속 쥔다. 3초 = T101×C101(1.5초)보다 길어 대기자 요청이 한 바퀴 돈다

*없으면: 없으면 남들은 30초마다 잠깐 비는 틈만 보고 놓친다*

<a id="R-FLR-T2-FIRST-RTP"></a>**R-FLR-T2-FIRST-RTP** · 근거

T2 는 허가가 아니라 첫 RTP 부터 센다(원문 6.3.4.4.5 1) — 큐에서 늦게 마이크를 켠 사람에게도 상한이 공정하다

*없으면: 허가부터 세면 늦게 켠 사람이 짧게 말하고 회수된다*

<a id="R-FLR-RTP-GATED"></a>**R-FLR-RTP-GATED** · 근거

RTP 수신은 강제 권위(prefan 게이트)를 통과한 것만 센다

*없으면: 게이트 전 관찰이면 아무에게도 안 들리는 발화가 T2 까지 산다*

<a id="R-FLR-NO-IDLE-WHEN-QUEUE"></a>**R-FLR-NO-IDLE-WHEN-QUEUE** · 근거

큐가 있으면 IDLE 없이 곧바로 승계한다(원문 6.3.4.3.2)

*없으면: IDLE 을 보내면 화면이 아무도 안 말함으로 깜빡여 끊긴 것처럼 보인다*

<a id="R-FLR-REGRANT"></a>**R-FLR-REGRANT** · 근거

이미 쥔 사람의 재요청은 거절이 아니라 재허가다(원문 6.3.4.4.8) — 시간에는 T2 남은 값

*없으면: 거절하면 버튼을 두 번 눌렀을 때 발언이 끊긴다*

<a id="R-FLR-NO-RETX-DIRECT"></a>**R-FLR-NO-RETX-DIRECT** · 근거

직접 허가·거절의 유실은 클라 T101 재요청 + 서버 멱등 재응답이 진다 — 원문이 재전송을 승계(T20)에만 둔 이유

*없으면: 서버도 재전송하면 같은 허가가 두 갈래로 온다*

<a id="R-FLR-ACK-NO-REPLY"></a>**R-FLR-ACK-NO-REPLY** · 근거

ACK 에 답하지 않는다 — DENY 로 답하면 그 DENY 가 다시 ACK 을 부른다

*없으면: 실측 요청 한 번에 33,971통*

<a id="R-FLR-FIRST-NOTIFY"></a>**R-FLR-FIRST-NOTIFY** · 근거

통지 권위가 DC 단일이라 이미 말하는 사람이 있으면 서버가 따로 알려야 한다 — READY 마다(재동기 포함)

*없으면: 없으면 들어간 방에서 말하는 사람이 안 보이고 소리만 난다*

<a id="R-FLR-DC-ONLY-DOWN"></a>**R-FLR-DC-ONLY-DOWN** · 근거

DC 만 끊기면 서버는 T1·T2 만 보므로 회수하지 않는다 — 양쪽이 다르게 아는 유일한 칸

*없으면: 사용자가 말하고 있다고 믿는 채로 아무도 안 듣는다*

<a id="R-FLR-DENY-RECVONLY"></a>**R-FLR-DENY-RECVONLY** · 근거

청취 전용의 요청을 무시하지 않고 거절한다

*없으면: 무시하면 클라가 T101×3 을 헛되이 태우고 사유를 못 보인다*

<a id="R-FLR-T3-GRACE"></a>**R-FLR-T3-GRACE** · 근거

회수를 보낸 뒤 화자 RELEASE(= 회수가 닿았다는 확인)가 올 때까지 화자 목소리를 계속 흘린다 — T8 재전송이 그 창 안에서 돈다(원문 6.3.4.5.1)

*없으면: 없으면 회수가 유실될 때 화자는 말한다고 믿는데 아무도 못 듣는다*

<a id="R-FLR-PREEMPT-OPEN"></a>**R-FLR-PREEMPT-OPEN** · 근거

선점은 앱이 우선순위 값을 정해야 열린다 — 모두 기본 0 이면 전원 동급 선착 큐

*없으면: 상대 비교라 선점 화자도 더 높은 값에 다시 선점된다*

<a id="R-FLR-TAKEN-ACK"></a>**R-FLR-TAKEN-ACK** · 근거

TAKEN 이 사라지면 그 발화 내내 듣는 사람 화면에 화자가 없다 — 받아내는 다른 장치가 없다(흐름 검사 F1 · H9 반례)

*없으면: 소리는 들리는데 누가 말하는지 안 보인다*

<a id="R-FLR-PREQ-STAY"></a>**R-FLR-PREQ-STAY** · 근거

선점 판정이 상대 비교라 클라는 자기 요청이 선점인지 모른다 — 서버가 모든 요청에 답하므로 늘 남는다

*없으면: 내려가면 그 뒤 QUEUE_INFO·승계 GRANTED 를 버려 빈 채널이 T1 까지 묶인다(3자 검증 1 · 모델 검사 V1)*

<a id="REJ-FLR-MULTI-TALKER"></a>**REJ-FLR-MULTI-TALKER** · 기각

multi-talker 미채택 — 슬롯 audio 1 구조가 두 화자를 못 받는다

*무늬: Multi Talker|multi-talker*

<a id="REJ-FLR-UNICAST-MEDIA"></a>**REJ-FLR-UNICAST-MEDIA** · 기각

Unicast Media Flow Control 미채택

*무늬: Unicast Media*

<a id="REJ-FLR-QUEUED-REQUESTS"></a>**REJ-FLR-QUEUED-REQUESTS** · 기각

Queued Floor Requests 미채택

*무늬: Queued Floor Requests*

<a id="REJ-FLR-REVOKE-REQUEST"></a>**REJ-FLR-REVOKE-REQUEST** · 기각

Floor Revoke Request 미채택

*무늬: Floor Revoke Request*

<a id="REJ-FLR-IMPLICIT-REQUEST"></a>**REJ-FLR-IMPLICIT-REQUEST** · 기각

implicit floor request — SIP 호 제어 미채택

*무늬: implicit floor request*

<a id="REJ-FLR-T103"></a>**REJ-FLR-T103** · 기각

T103(End of RTP media, 클라) 미채택 — 원문에서도 선택

*근거: R-FLR-NO-T103 · 무늬: T103*

<a id="REJ-FLR-T4"></a>**REJ-FLR-T4** · 기각

T4 미채택

*근거: R-FLR-NO-T4 · 무늬: T4 \(Inactivity\)*

<a id="REJ-FLR-QUEUED-CANCEL"></a>**REJ-FLR-QUEUED-CANCEL** · 기각

Floor Queued Cancel — dual floor 축 미채택

*무늬: Queued Cancel*

<a id="REJ-FLR-MERGE"></a>**REJ-FLR-MERGE** · 기각

그룹 병합 — SIP 호 제어 미채택

*무늬: Merge|merge*

<a id="REJ-FLR-OVERRIDE"></a>**REJ-FLR-OVERRIDE** · 기각

overriding without revoke(dual floor) 일 때만의 절 — override 선점 미채택

*무늬: Send RTP Media \(S: media\)*

<a id="R-FLR-SEQ-DISPLAY"></a>**R-FLR-SEQ-DISPLAY** · 근거

데이터 채널이 순서를 보장하지 않아(`ordered: false`, §9-4-13) 승계 뒤에 옛 `TAKEN` 이 닿으면 듣는 사람 화면이 앞 화자로 되돌아간다(H9 반례 h9_core). 원문 클라는 일련번호를 보지 않는다(RULE-FLR-SEQ) — 견줄 것이 번호뿐이다

*없으면: 말하는 사람과 화면의 화자가 다르다(PROP-FLR-SHOWN-IS-HOLDER)*

<a id="R-FLR-T21-SAME-SEQ"></a>**R-FLR-T21-SAME-SEQ** · 근거

`T21` 재전송(DEV-FLR-TAKEN-ACK)이 떠 있는 사이 대기를 그만둔 클라가 재전송된 옛 `TAKEN` 을 반환 확인으로 읽으면 앞 화자를 보이고, 그 뒤 `IDLE` 은 직전 화자에게 오지 않는다(H9 반례 h9f_core · h9f_loss1_t7) — 재전송과 새 `TAKEN` 을 가를 것이 번호뿐이다(DEV-FLR-SEQ-DISPLAY)

*없으면: 반환한 사람 화면에 앞 화자가 남는다*

<a id="R-FLR-PREQ-IDLE-DISPLAY"></a>**R-FLR-PREQ-IDLE-DISPLAY** · 근거

요청 중 클라가 `IDLE` 을 버리면 화면에 앞 화자가 남고 `T101` 을 다 쓴 뒤에도 그대로다(H9 반례 h9f_loss1_ack_t7)

*없으면: 아무도 말하지 않는데 화자가 보인다(PROP-FLR-SHOWN-IS-HOLDER)*

<a id="R-FLR-PRESS-DROP"></a>**R-FLR-PRESS-DROP** · 근거

그 상태들의 누름을 정하지 않으면 구현마다 다르게 한다 — 요청을 다시 내거나(서버는 화자의 재요청을 재허가로 받는다 — RULE-FLR-MSG-REQUEST) 대기를 철회로 읽으면 같은 버튼이 구현에 따라 wire 에 다른 것을 낸다. 규칙 하나(버린다)면 상태기에 새 전이가 없다

*없으면: 같은 누름이 구현마다 다른 메시지가 된다*

<a id="R-FLR-GRANTED-SEQ"></a>**R-FLR-GRANTED-SEQ** · 근거

번호 없는 `GRANTED` 가 더 옛 `TAKEN` 을 앞지르면 대기 허가를 받은 클라가 화자 표시를 앞 화자로 되돌리고, `T132` 로 허가를 놓으면 서버는 반환한 사람에게 `IDLE` 을 보내지 않으므로(6.3.5.5.4) 표시가 굳는다(H9 반례 h9s_core)

*없으면: 아무도 말하지 않는데 앞 화자가 보인다 — 다음 발언권 사건까지*

<a id="R-FLR-ACK-SEQ"></a>**R-FLR-ACK-SEQ** · 근거

청취자가 `TAKEN` 에 보내는 것은 ACK 뿐이라 `T21` 은 ACK 으로 멈추는데, 번호 없는 ACK 은 옛 `TAKEN` 의 늦은 ACK 과 가를 수 없다 — 새 `TAKEN` 이 유실되면 재전송 없이 표시가 빈다(H9 반례 h9f_loss1_ack_t7). 원문 `T20` 은 허가받은 사람의 RTP 로 멈춰(6.3.4.4.5) 이 문제가 없다

*없으면: 유실 하나와 늦은 ACK 하나가 겹치면 그 발화 내내 그 청취자 화면에 화자가 없다*

<a id="R-FLR-STALE-IDLE-STAY"></a>**R-FLR-STALE-IDLE-STAY** · 근거

낡은 `IDLE` 이 원문대로 대기를 떠나게 하면 클라와 서버 대기열이 어긋난다 — 서버가 차례에 준 `GRANTED` 를 클라가 버리고(`has no permission`), 서버는 말하지 않는 사람을 화자로 쥔 채 `T20` 재전송을 다 쓰고 `T1` 까지 기다린다 — 그동안 다른 사람이 말할 수 없다(H9 반례 h9s_core)

*없으면: 순서가 바뀐 메시지 하나로 채널이 몇 초 막힌다*

<a id="R-FLR-DOWN-FRESH"></a>**R-FLR-DOWN-FRESH** · 근거

재구축 뒤의 발언권은 새 연결 위의 새 관계다. 방 일련번호는 서버가 방마다 센다(RULE-FLR-SEQ 값 칸) — 미디어 서버가 재기동하면 다시 작은 값부터 오르고, MBCP 에는 WS 의 `epoch` 같은 축이 없다. 발행 방 · 반이중 발행도 재구축이 다시 한다(RULE-LINK-RECOVERY-REBUILD — 다시 발행 · 다시 `pub_select`)

*없으면: 옛 `seen` 이 새 `TAKEN` · `IDLE` 을 낡은 것으로 읽어(DEV-FLR-SEQ-DISPLAY) 화자 표시가 굳는다 · 옛 `isPub` · `hasHalf` 로 누름 준비 단계를 건너뛰어 `DENY(101)` · `DENY(5)` 를 받는다*

<a id="R-FLR-JOIN-FRESH"></a>**R-FLR-JOIN-FRESH** · 근거

다시 입장은 같은 방 · 같은 서버라 방 일련번호가 끊기지 않는다 — 떠날 때(`FLR.C.any.leave`)도 번호를 두었다. 나머지 칸은 떠날 때 이미 내렸거나 새 입장이 다시 알린다.

*없으면: 떠난 채(`left`)인 클라 칸에 입장이 아무것도 안 해 재입장한 방의 발언권 클라가 떠난 사람으로 남는다 · `seen` 까지 지우면 다음 `TAKEN` · `IDLE` 의 번호 견주기(DEV-FLR-SEQ-DISPLAY)가 처음부터 다시 서 낡은 통지를 새것으로 읽을 수 있다*

<a id="R-FLR-ROOM-GONE"></a>**R-FLR-ROOM-GONE** · 근거

원칙 ③(20261011) — 자격(방)을 잃으면 서버가 정리하고 이유를 알린다. 3인방 공통 — LiveKit 은 참가자를 닫을 때 그 발행 트랙을 전부 내리고(`UpTrackManager.Close`) 이유를 실은 `LeaveRequest` 를, Janus 는 강퇴 때 그 사람 발행을 내리고(`leave_or_unpublish(…, kicked)`) `kicked` 를 보낸다

*없으면: 강퇴당한 화자가 서버에서 화자로 남아 남은 사람이 `T1`(4초)까지 못 말한다*

<a id="R-FLR-MIC-CLOSE-ON-LOSS"></a>**R-FLR-MIC-CLOSE-ON-LOSS** · 근거

`has permission` 을 떠나는 모든 길(뗌 · 회수 · 발행 방 이동 · 방 퇴장 · 방을 잃음 · 재구축)에서 반이중 송출을 멈춘다 — 원문은 허가 밖 RTP 를 송신 위반으로 회수한다(6.3.5.4.6 · `U: not permitted but sends media`)

*없으면: 서버가 송신 위반으로 `REVOKE(3)` 를 되풀이하고 그 RTP 는 게이트에서 버려진다*

<a id="ERR"></a>
