# 09. 마이크 전원 · 상시 대기 비용 — hot · hot_standby · cold 와 1pc 대 2pc

> author: kodeholic (powered by Claude) · created: 2026-11-09.
> ★**대조한 커밋** — 서버 `oxlens-sfu` `c3642e8` · SDK `oxlens-sdk-web` `9f40557`(+탐침)
> ★**실측 회차** — 3층 `tour-learn-20261109`(탐침 `TOUR-09` · `TOUR-06` 끝 부분). 실장치 재획득 시간은 20260624 실측. 값은 돌릴 때마다 바뀌고, 차례 · 모양은 같다.

## 1. 30초 요약

```mermaid
flowchart LR
  subgraph MIC["마이크 — SDK 혼자 정한다 · 서버는 모른다"]
    HOT["hot<br/>허가 중 · 놓는 중"] -- "놓음" --> SB["hot_standby<br/>장치 쥔 채 게이트 닫힘"]
    SB -- "누름 · 허가" --> HOT
    SB -- "무발화 30초" --> COLD["cold<br/>장치 반납"]
    COLD -- "누름 → 재획득<br/>실장치 cold 360~480ms" --> HOT
  end
  subgraph NET["망 — 마이크와 무관하게 늘 흐른다"]
    STUN["STUN 동의 검사<br/>연결마다 · 약 2.7초마다"]
    PAY["RTCP · DC 심박"]
    WS["WS HEARTBEAT 10초"]
  end
```

- 마이크는 **누를 때 잡고 30초 쉬면 OS 에 돌려준다**(`cold`). 대가는 다음 첫 음절 — 실장치에서 cold 재획득이 360~480ms 걸렸다(20260624 실측 · 입장 직후 첫 획득 640ms 는 별개). `keepWarm(ms)` 이 그것을 미룬다.
- 마이크를 꺼도 **망 keepalive 는 줄지 않는다.** 그중 STUN 동의 검사는 **연결 수만큼** 나간다 — 60초 동안 1pc 22번 · 2pc 45번. 그래서 1pc 를 지향한다.
- 다만 STUN 을 뺀 나머지(RTCP · DC 심박)는 1pc 와 2pc 가 **같았다**(보냄 394B · 받음 130B). 1pc 가 줄이는 것은 STUN 몫이다.

## 2. 등장인물 · 값

| 이름 | 값 | 무엇 |
|---|---|---|
| ONE | `touridle_ONE` · 1pc · 듣기 | 연결 하나 |
| TWO | `touridle_TWO` · 2pc · 듣기 | 발행 연결(DC 만) + 구독 연결 |
| 방 | `qa_touridle_p2` · node-a | 둘 다 마이크 없음 · 발언 0 |
| P · L | `tourattr_P` 말하기 · `tourattr_L` 듣기 · 1pc | 마이크 전원 탐침(`TOUR-06` 끝 부분 · 방 `qa_tourattr_p3`) |

**상시 대기 60.015초 — candidate-pair 계수의 차(after − before)**

| | ONE(1pc) | TWO 쌍 1 | TWO 쌍 2 | TWO 합 |
|---|---|---|---|---|
| 지명된 쌍 수 | 1 | | | 2 |
| STUN 동의 검사 보냄(`consentRequestsSent`) | **22** | 22 | 23 | **45** |
| 그 응답 받음(`responsesReceived`) | 22 | 22 | 23 | 45 |
| 페이로드 보냄 `bytesSent` · 패킷 | **394B** · 14 | 130B · 2 | 264B · 12 | **394B** · 14 |
| 페이로드 받음 `bytesReceived` · 패킷 | **130B** · 2 | 130B · 2 | 0B · 0 | **130B** · 2 |

- ★W3C 의 candidate-pair `bytesSent` · `bytesReceived` 는 **IP · UDP 머리와 STUN 연결 검사를 빼고 센다.** 그래서 STUN 은 바이트가 아니라 **횟수로** 견준다. 이 회차는 STUN 바이트를 재지 않았다 — kbps 총량은 이 표로 낼 수 없다.
- 페이로드의 종류(RTCP · DC 심박)는 이 회차가 가르지 않았다. TWO 의 쌍 1 은 2통을 주고받았고 쌍 2 는 12통을 보내기만 했다 — 1pc 는 그 둘이 한 쌍에 실렸다.
- STUN 동의 검사 간격 = 60.015초 ÷ 22 ≈ **2.7초**(브라우저가 정한다 — 앱이 못 바꾼다).
- WS 는 이 표 밖이다 — 두 사람 다 `HEARTBEAT` 를 10초마다 6번 주고받았다(`heartbeat_interval 10000` · [연:RULE-SES-HEARTBEAT](../protocol/03-ses.md#RULE-SES-HEARTBEAT)). 1pc 와 2pc 가 같다.

| 기록 값(20260624 · 옛 서버 · 2pc · 무발화 32.7초 pcap) | |
|---|---|
| 1 사람 합계 | **1.32 kbps** ≈ 13.5 MB/일 ≈ **406 MB/월**(host 후보만 — **하한**) |
| 나눔 | STUN **61%** · RTCP 31% · DTLS/DC 7% · RTP 음성 1% |
| cold 재획득(허가 → 첫 음성) | **480 · 360 · 460ms**(무발화 32.7 · 46.4 · 60.0초 뒤) = cold 실측 360~480ms · hot_standby 는 20~60ms · 입장 직후 첫 획득(gen1)은 640ms — cold 가 아니라 별개 |

## 3. 정상 길

### 3-1. 마이크 전원 세 상태 — 실측 시각

`TOUR-06` 끝 부분(07:38 ~ 07:39 KST · 초.밀리초). `mic` 은 탐침이 250ms 마다 `ptt.state.mic` 를 읽은 것이고, 정확한 전이 시각은 SDK 전이 기록(`SDK.cold` · `SDK.warm`)이다.

```mermaid
sequenceDiagram
  participant APP as 앱 P
  participant SDK as P SDK
  participant S as sfud
  participant L as L 듣기
  Note over SDK: 01.635 mic = hot_standby — 아직 장치를 안 잡았다
  APP->>SDK: 01.635 press
  SDK->>SDK: 01.648 마이크 잡기 PUB IDLE → STAGED
  SDK->>S: 01.651 PUBLISH_TRACKS half
  SDK->>S: 01.653 REQUEST
  S->>SDK: 01.653 GRANTED · mic.open — hot
  S->>L: 01.653 TAKEN 화자 P
  APP->>SDK: 02.744 release
  SDK->>S: 02.747 RELEASE+A · mic.close
  S->>SDK: 02.751 ACK 0400 — hot_standby
  Note over SDK: 32.763 SDK.cold — 놓은 지 30.016초 · 장치 반납
  APP->>SDK: 34.106 press
  SDK->>SDK: 34.140 SDK.warm 재획득 — 가짜 장치 34ms
  SDK->>S: 34.140 REQUEST
  S->>SDK: 34.140 GRANTED · 34.141 mic.open
  APP->>SDK: 35.328 release · 35.334 keepWarm 120000
  Note over SDK: 39:07.337 32초 뒤에도 hot_standby
```

| 상태 | 언제 | 장치 | RTP |
|---|---|---|---|
| `hot` | 허가 중 · 놓는 중(`has_permission` · `pending_release`) | 쥠 · 게이트 열림 | 나감 |
| `hot_standby` | 그 밖 — 입장 직후(아직 안 잡음) · 놓음 · 거절 뒤 | 쥠 · 게이트 닫힘 | 안 나감 |
| `cold` | 그 서버의 발언권이 **전부** `no_permission` 인 채 30초 | ★반납(`MediaStreamTrack.stop()` — 표시등 꺼짐) | 안 나감 |

- `cold` 시계는 누름 응답을 기다리거나 **줄 서 있는 동안은 반납에 이르지 않는다** — 곧 말할 사람이기 때문이다. 거절 · 철회로 `no_permission` 에 돌아온 순간부터 30초를 다시 센다.
- 30초는 정책 값 `micColdAfterMs`(SDK 기본값 30000)이고, `keepWarm(ms)` 은 그 시계를 `ms` 로 다시 건다 — 지령대처럼 늘 말하는 곳에서 쓴다. 이 회차는 `keepWarm(120000)` 뒤 32초가 지나도 `hot_standby` 였다.
- 앱 · 외부가 소유한 트랙(`owner` 가 `sdk` 가 아님)은 내려가지 않는다 — SDK 가 앱의 오디오 파이프라인을 끊지 않는다.

### 3-2. 서버는 모른다

전원 세 상태 어느 전이에도 WS 프레임이 없다. 발행 트랙은 `REGISTERED` 그대로이고 발언권 · 줄도 그대로다. 반이중은 게이트가 열릴 때만 sender 에 소스를 얹는다(`replaceTrack` — 재발행 없음 · SSRC 그대로). 그래서 `cold` 에서 다시 누르면 서버가 보는 것은 평소와 같은 `REQUEST` 한 통이다(34.140) — 차이는 그 앞, 클라 안에서 재획득을 기다린 시간뿐이다.

### 3-3. 상시 대기 60초 — 1pc 와 2pc 를 한 방에서

`TOUR-09` 는 ONE(1pc) · TWO(2pc)를 같은 방에 듣기로 넣고 5초 기다린 뒤 60초 동안 아무것도 하지 않는다. 그 앞뒤에 각자 모든 candidate-pair 의 계수를 읽었다.

```mermaid
sequenceDiagram
  participant ONE as ONE 1pc
  participant TWO as TWO 2pc
  participant S as sfud node-a
  Note over ONE,S: 35:46.540 idle.start — 계수 읽음
  ONE->>S: 쌍 하나로 STUN 동의 검사 22번
  TWO->>S: 발행 연결 쌍 STUN 22번
  TWO->>S: 구독 연결 쌍 STUN 23번
  ONE->>S: 페이로드 14통 394B
  TWO->>S: 페이로드 2통 + 12통 = 394B
  ONE->>S: WS HEARTBEAT 10초마다 6번
  TWO->>S: WS HEARTBEAT 10초마다 6번
  Note over ONE,S: 36:46.555 idle.end — 계수 읽음 · 60.015초
```

- STUN 동의 검사(ICE consent freshness)는 **연결마다** 돈다 — 2pc 는 연결이 둘이라 두 벌이다. 이것이 1pc 를 지향하는 이유의 실측이다([연:RULE-LINK-SDP-PC-COUNT](../protocol/09-link.md#RULE-LINK-SDP-PC-COUNT)).
- STUN 을 뺀 페이로드는 두 꼴이 바이트까지 같았다(보냄 394B · 받음 130B). 연결이 하나든 둘이든 같은 RTCP · DC 심박이 나가고, 1pc 는 그것을 한 쌍에 실을 뿐이다.

## 4. 깨지는 길

### 4-1. cold 에서 누르면 첫 음절이 잘린다

이 회차(`TOUR-06`)의 재획득은 34ms(누름 34.106 → `SDK.warm` 34.140)였다 — 가짜 장치라 **이 숫자를 믿지 말 것.** 실장치는 20260624 의 서버 로그 · pcap 이 답한다.

```mermaid
sequenceDiagram
  participant U as U03 단말
  participant S as sfud
  participant L as 듣는 사람
  Note over U: 32.7초 무발화 — cold · 장치 반납
  U->>S: 누름 · REQUEST
  S->>U: 22:57:06.603 GRANTED
  S->>L: 예열 무음 20ms 마다 24 프레임
  U->>S: 06.603 STUN 3통 — 망은 살아 있다
  Note over U: 06.607 ~ 07.067 아무 패킷도 없음 460ms<br/>getUserMedia 재호출 · 인코더 재생성
  U->>S: 07.067 첫 음성
  Note over L: 첫 음절은 막 깨어난 인코더의 첫 프레임 — 잘려 들린다
```

- **무엇이 깨졌나** — 허가는 즉시 났는데 단말이 보낼 소리가 없다. 장치 재획득 + 인코더 재생성 + 첫 프레임이 460ms.
- **누가 먼저 아나** — 서버는 예열 프레임 수로 안다(허가 → 첫 실제 음성 사이를 20ms 마다 메운 수 — 24 프레임 ≈ 480ms, 18 ≈ 360ms, 23 ≈ 460ms). 같은 날 hot_standby 에서 누른 허가는 1~3 프레임(20~60ms)이었다.
- **사용자는 무엇을 겪나** — 듣는 사람은 예열 무음으로 공백 없이 이어 듣지만 화자의 첫 음절이 잘린다. 예열로는 못 살린다 — 잘리는 것은 단말 안에서다.
- **처방** — 자주 말하는 곳은 `keepWarm(ms)` 으로 `cold` 를 미룬다(전력과 맞바꾼다). `ptt.enable()` 은 다른 일이다 — 입장 뒤 **처음** 누를 때의 마이크 잡기 · 반이중 발행을 미리 해 둔다.

### 4-2. 마이크를 꺼도 망 비용은 안 준다

`TOUR-09` 의 두 사람은 **마이크가 아예 없다** — 그래도 60초에 STUN 22 · 45번과 페이로드 394B 를 보냈다. 20260624 의 나눔도 같은 말을 한다 — 마이크 몫(RTP 음성)은 1% 이고 99% 가 keepalive 다.

- **왜 STUN 을 못 끄나** — 브라우저는 동의 응답이 끊기면 그 연결로 미디어를 멈춘다(ICE consent). 서버도 확인된 STUN 이 30초 안 오면 그 연결을 `Suspect` 로 본다(소스 `oxsfud/src/link/mod.rs` `UDP_PAIR_EXPIRE_MS` 30000 · `ops/timers.rs` `liveness`). 2층 `adv_floor_failover` 에서 봇이 죽고 27.2초 뒤 서버 로그에 그 줄이 났다:
  ```
  07:32:28.863 node=node-c proc=sfu kind=ev spec=SV-STATE-PEER-STATE user=botA~adv_floor_failover area=LINK key=publish from=Alive to=Suspect cause=pair_expired
  ```
- **사용자는 무엇을 겪나** — 종일 켜둔 단말에서 말을 안 해도 데이터 요금이 나간다. 웹은 동의 검사 간격 · 후보 수 · RTCP 간격을 못 만진다 — 연결 수를 줄이는 것(1pc)이 웹에서 쥘 수 있는 유일한 손잡이다.

### 4-3. 줄 서는 동안 cold 에 들어가면 — 이 회차로 재지 않았다

규격이 적은 증상이다: 줄 서는 동안 30초가 지나 장치를 반납하면 **허가를 받는 순간** 재획득이 걸려 첫 음절이 잘린다(— — 20261103 코드 읽기에서 드러났다). 규격이 적은 360~640ms 는 20260624 표 전체 범위다(입장 직후 첫 획득 640ms 포함) — cold 재획득 실측만 보면 360~480ms 다. 지금 SDK 는 반납하기 직전에 그 서버의 발언권 중 하나라도 「곧 말할」 상태(누름 응답 대기 · 줄)면 반납하지 않고 시계를 다시 건다(`domain/pub/local.ts` `goCold`).

## 5. 왜 이 모양인가

> ★**우리만의 것 — 무전 상시 대기를 1pc 로**
> - **선택 배경** — 무전 단말은 종일 켜두는 물건이라 말하는 시간보다 **기다리는 시간의 비용**이 지배한다. 그 비용의 대부분이 연결마다 나가는 keepalive 다(20260624 2pc 실측 — STUN 61% · RTCP 31% · DTLS/DC 7% · 음성 1%). 그래서 발행 · 구독을 연결 하나에 싣는 1pc 를 지향점으로, 2pc 를 호환 기준선으로 둔다([연:RULE-LINK-SDP-PC-COUNT](../protocol/09-link.md#RULE-LINK-SDP-PC-COUNT)). 1pc 와 2pc 의 방향 · SDP 는 [02](02_CONNECT_JOIN.md).
> - **얻은 것** — 60초 무발화에 **STUN 동의 검사 22번 대 45번** — 반이다(`TOUR-09`). 2pc 에서 가장 큰 몫(61%)이 STUN 이었으니 그 몫이 그대로 반이 된다.
> - **잃은 것** — ①**STUN 밖의 몫은 줄지 않았다** — 페이로드는 1pc · 2pc 모두 보냄 394B · 받음 130B 로 같았다. 「1pc 는 STUN · DTLS · RTCP 를 한 벌만 흘린다」는 규격 문장 가운데 이 회차가 확인한 것은 STUN 하나다. ②**그래도 무전 전용 프로토콜보다 크다** — STUN 과 IP · UDP 머리를 빼고 페이로드만 세도 시간당 약 31kB((394 + 130)B × 60)로, Zello 가 밝힌 대기 소비(시간당 20kB · 0.044 kbps)보다 많다. Zello 는 ICE · RTCP · DTLS 가 아예 없다. ③마이크 절전의 대가는 첫 음절(실장치 cold 재획득 360~480ms · 입장 직후 첫 획득 640ms 는 별개).

| 다른 길 | 버린 이유 |
|---|---|
| `cold` 때 연결도 닫는다 | keepalive 는 0 이 되지만 말할 때 ICE + DTLS 를 다시 세워야 한다 — 이미 460ms 인 재획득에 수백ms~초가 더 붙는다 |
| 동의 검사 · RTCP 간격을 늘린다 | 웹에서는 브라우저 몫이라 못 한다. 늘리면 끊김을 늦게 안다 — 네이티브 SDK 에서 쥘 손잡이다 |
| 마이크를 늘 `hot_standby` 로 | 첫 음절은 지키지만 장치 점유 · 전력이 계속 나간다 — 앱이 원하면 `keepWarm` 으로 고른다 |
| 서버에 마이크 상태를 알린다 | 서버가 쓸 데가 없다 — 발언권 · 발행 상태가 그대로라 서버 판정에 영향이 없다 |


## 6. 다른 SFU 는

★「그쪽 방식」은 소스를 정적으로 읽은 대장과 공개 문서 조사에서 옮겼다 — **「저쪽에는 이렇게 적혀 있다」이지 실측이 아니다.**

| 상대 | 그쪽 방식 | 우리 | 우리 쪽 장점 | 우리 쪽 대가 |
|---|---|---|---|---|
| Zello Work | 자체 프로토콜 — ICE · RTCP · DTLS 없음 · 대기 시간당 20kB(0.044 kbps) · 관리자가 keep-alive 간격(모바일 · WiFi 따로) · 「기기를 깨워 연결 유지」 · 「모바일은 TCP 만」(데이터를 줄이지만 지연이 는다)을 원격으로 민다(조사 기록 §5-2 · §5-3 · §5-4) | WebRTC 표준 그대로 · 1pc 로 연결 수만 줄인다 | 브라우저에서 설치 없이 돈다 · 암호 · 혼잡 제어를 표준으로 얻는다 | 대기 비용이 크다 · 단말 전력 손잡이(기기 깨우기)가 웹에는 없다 |
| mediasoup | ICE Lite 서버 · 동의 검사 시한을 앱이 정한다(`iceConsentTimeout` 기본 30초 · 범위 10~60, `mediasoup §8-2`) | 서버는 확인된 STUN 30초로 연결 생존을 본다(소스 상수) | 같은 축 | 값이 설정이 아니라 상수다 |
| Janus | ICE 표면이 가장 넓다 — `ice_consent_freshness` · `ice_keepalive_conncheck` 를 설정으로 켠다(`Janus §1-3`) | 브라우저가 보내는 동의 검사에 답한다 | — | 서버 쪽 keepalive 손잡이가 없다 |
| Mumble | 클라가 발화를 판정한다(VAD · PTT 키 · 짧게 두 번 누르면 떼도 유지) — 마이크 전원 축은 대장에 없다(`Mumble §9-1`) | 서버 권위 발언권 + SDK 마이크 전원 셋 | 전원이 발언권 상태를 보고 내려간다(줄 서는 동안 안 내려감) | — |


## 7. 운영에서 보는 법

| 보고 싶은 것 | 어디서 | 실제 모양(이 회차) |
|---|---|---|
| 마이크 전원 전이 | SDK 전이 기록 — 영역 `PUB`, 라벨 `SDK.cold` · `SDK.warm`, 원인 `mic_power` | `SDK P PUB te0e99fb9c6adc75b REGISTERED → REGISTERED \| mic_power \| SDK.cold` |
| 지금 전원 | `ptt.state.mic`(`hot` · `hot_standby` · `cold`) | 위 3-1 |
| 서버 쪽 | ★**없다** — 전원 전이에 서버 로그 줄이 없다. 서버는 발언권 줄(`FLR.S.…`)과 발행 첫 RTP(`SV-STATE-PUBLISH-STATE … cause=first_rtp`)만 본다 | |
| 연결 생존(keepalive 가 끊겼을 때) | 서버 로그 `SV-STATE-PEER-STATE … from=Alive to=Suspect cause=pair_expired` · 진단 `SV-IMPL-LINK-PEER-STATE … in.dwell_ms` | 4-2 의 줄 |
| keepalive 계수 | 브라우저 `getStats()` 의 candidate-pair(`consentRequestsSent` · `bytesSent` · `bytesReceived`) | 위 2 절 표 |

- 「첫 음절이 잘린다」는 서버 로그만으로는 못 본다 — 허가 줄과 그 사람의 `first_rtp` 줄 사이 간격이 단서다(예열이 그만큼 돌았다). 로그 줄 꼴 · 사람 하나의 사슬은 [17](17_OPERATIONS.md).

## 8. 소스 · 규격 좌표

| 무엇 | SDK(`oxlens-sdk-web/src`) | 서버(`oxlens-sfu/crates`) | 규격 |
|---|---|---|---|
| 전원 시계 · 반납 · 데우기 | `domain/pub/local.ts`(`armCold` · `keepWarm` · `goCold` · `warm`) | — | — |
| `ptt.state.mic` · `keepWarm` 손잡이 | `domain/flr/core.ts`(`mic`) · `api/handles.ts`(`keepWarm`) · `domain/pub/registry.ts`(`halfMicCold`) | — | — |
| 30초 기본값 | `domain/gen/sdk_defaults.ts`(`micColdAfterMs: 30000`) | — | 정책서 |
| 연결 생존(STUN 30초) | — | `oxsfud/src/link/mod.rs`(`UDP_PAIR_EXPIRE_MS` · `touch_pair` · `path_alive`) · `oxsfud/src/fwd/rt.rs`(검증된 STUN 만 `touch_pair`) · `oxsfud/src/ops/timers.rs`(`liveness`) | |
| WS 심박 | — | `oxhubd/src/ses/mod.rs`(`heartbeat_interval`) | [연:RULE-SES-HEARTBEAT](../protocol/03-ses.md#RULE-SES-HEARTBEAT) |
| 1pc 지향 · 대기 비용 | — | — | [연:RULE-LINK-SDP-PC-COUNT](../protocol/09-link.md#RULE-LINK-SDP-PC-COUNT) · [연:RULE-LINK-SDP-1PC](../protocol/09-link.md#RULE-LINK-SDP-1PC) |
| 탐침 계수 | `qa/qa.js`(`icePairs`) · `qa/live/tests/probe/tour_capture.spec.ts`(`TOUR-09`) | — | |

## 9. 스스로 확인하는 질문

<details><summary>1. P 가 놓고 30초 쉬자 cold 가 됐다. 그 사이 서버로 무엇이 갔나?</summary>

아무것도 — 전원 전이는 SDK 안의 일이다. 서버에게 P 는 여전히 반이중 트랙을 등록한 채 발언권이 없는 사람이다. 다시 누르면 서버는 평소와 같은 `REQUEST` 한 통을 본다(34.140).
</details>

<details><summary>2. 1pc 로 바꾸면 상시 대기 비용이 반이 되나?</summary>

STUN 동의 검사 횟수는 반이 된다(60초 22번 대 45번). 그러나 STUN 을 뺀 페이로드는 같았다(보냄 394B · 받음 130B). 줄어드는 것은 STUN 몫이고, 그 몫이 2pc 합계의 61%(20260624)였다.
</details>

<details><summary>3. cold 에서 누른 사람의 첫 음절이 잘렸다. 서버 쪽 무엇으로 짐작하나?</summary>

허가에서 그 화자의 첫 RTP 까지의 간격 — 예열 무음이 그동안 20ms 마다 돌았다. 20260624 에 cold 재획득은 18~24 프레임 — 실측 360~480ms(입장 직후 첫 획득 640ms 는 별개) · hot_standby 는 1~3 프레임(20~60ms)이었다. 서버는 전원 상태를 모르니 이 간격이 유일한 단서다.
</details>

<details><summary>4. 데이터를 아끼려고 STUN 동의 검사를 멈추면 무엇이 깨지나?</summary>

브라우저는 동의가 끊기면 미디어를 멈추고, 서버는 확인된 STUN 이 30초 안 오면 그 연결을 `Suspect` 로 본다(`pair_expired`). 2층에서 봇이 죽은 뒤 27.2초 만에 그 줄이 났다. 웹에서는 간격을 바꿀 수도 없다 — 연결 수를 줄이는 것이 쥘 수 있는 손잡이다.
</details>
