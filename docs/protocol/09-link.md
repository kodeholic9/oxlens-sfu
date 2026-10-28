[← 차례](README.md)

## 9. LINK — 미디어 서버 연결

세션이 미디어 서버 하나와 맺은 WebRTC 연결 — 1pc 면 하나, 2pc 면 보내기 · 받기 둘. 복구 단위

| 정체성 | 누가 내나 | 무엇마다 | 수명 | 상태 |
|---|---|---|---|---|
| sfu_id(서버) · ufrag(세대) | server | 세션 × 미디어 서버 | 그 서버의 첫 ROOM_JOIN 응답(server_config) → 세션 끝 — Peer 수명 = 세션 수명 | — |

| 의존 | 기댐 | 다중도 | 뜻 |
|---|---|---|---|
| LINK | SES | N:1 | 연결은 세션 안에서만 산다 — Peer 수명 = 세션 수명 |
| ROOM | LINK | N:1 | 방은 미디어 서버 하나가 맡는다(sfu_id) — 입장 응답이 그 서버 연결의 재료를 준다(§9-1-1) |
| PUB | LINK | N:1 | 트랙은 그 서버 연결의 트랜시버로 나간다(§9-4-10) |
| SUB | LINK | N:1 | 받기 m-line 은 그 서버 연결 위(§9-4-11) |
| FLR | LINK | N:1 | MBCP 는 그 방을 맡은 서버 연결의 DC 위(§10) |

### 9-1. 구조

<a id="LINK.type.ServerConfig"></a>
#### 9-1-1. 형 `ServerConfig` — 미디어를 붙이는 재료

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| ★`sfu_id` | str | ✔ | ★**그 미디어 서버의 신원**(불투명). 프로세스 기동마다 새로 발급된다. ★**뜻을 해석하지 않고 같은지만 본다.** `version.epoch`(§4-1-6)와 같은 값 |
| ★`pc_mode` | "1pc" / "2pc" | ✔ | ★**그 서버가 적용한 값**(`"1pc"`/`"2pc"`). 세션 확정값(§3-1 `BIND` 응답)과 ★**반드시 같다** — 다르면 규격 위반이고 클라는 **연결을 세우지 않는다** |
| `ice` | {ip, port, publish_ufrag, publish_pwd, subscribe_ufrag, subscribe_pwd, tcp_port} | ✔ |  |
| `dtls` | {fingerprint, setup} | ✔ |  |
| `codecs[]` | `{kind, name, rtcp_fb[]}` | ✔ | 발행에 쓸 코덱. ★**PT·클럭·fmtp 는 없다** |
| `codecs_sub[]` | 같음 | — | 있을 때만. **구독 전용 정책** |
| `extmap[]` | `{id, uri}` | ✔ | 확장 헤더 번호 |
| `max_bitrate_bps` | u64 | ✔ | 발행 상한 |

`ice` 안:

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `ice.ip` · `ice.port` | string · u16 | ✔ | 서버 미디어 주소 (§9-4-6 후보 줄 재료) |
| `ice.port` | u16 | ✔ |  |
| `ice.publish_ufrag` · `ice.publish_pwd` | str | ✔ | ★**발행 연결**의 ICE 자격 |
| `ice.publish_pwd` | str | ✔ |  |
| `ice.subscribe_ufrag` · `ice.subscribe_pwd` | str | ✔ | ★**구독 연결**의 ICE 자격. 재협상 때 **같은 값을 계속 쓴다** — ★`ICE_RESTART` 응답(§7-1-2)이 새 값을 줄 때만 바뀐다. ★`1pc` 에서는 **쓰지 않는다**(§9-4-18) |
| `ice.subscribe_pwd` | str | ✔ |  |
| ★`ice.tcp_port` | u16? | — | ★**있을 때만** — 서버의 TCP 미디어 포트(안드로이드 축 · UDP 가 막힌 망의 폴백). 오면 클라는 §9-4-9 의 TCP passive 후보 줄을 하나 더 낸다. 없으면 후보는 UDP 한 줄이다 |

`dtls` 안:

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `dtls.fingerprint` | str | ✔ | ★**`"sha-256 AB:CD:…"`** — 해시명 소문자 + 공백 + 대문자 hex 콜론. ★**클라는 `a=fingerprint:` 뒤에 가공 없이 그대로 치환한다**(§9-4-9) |
| `dtls.setup` | "passive" | ✔ | ★**서버 역할.** 항상 `"passive"` → **클라는 `active`**(§9-4-6) |

<a id="RULE-LINK-SERVER-CONFIG-SESSION"></a>**RULE-LINK-SERVER-CONFIG-SESSION** · 규칙 — `ServerConfig` — 미디어를 붙이는 재료

`ROOM_JOIN` 응답에만 온다. ★**이 값들은 그 세션 것**이고 서버 공통 설정이 아니다.

→ 구조 — [`LINK.type.ServerConfig`](#LINK.type.ServerConfig) · [`ROOM.op.ROOM_JOIN`](04-room.md#ROOM.op.ROOM_JOIN)

<a id="RULE-LINK-SFU-ID-IDENTITY"></a>**RULE-LINK-SFU-ID-IDENTITY** · 규칙

★★**미디어 서버의 신원은 `sfu_id` 다.** ★**주소가 아니다** — `ice.ip`+`ice.port` 로는
**프로세스를 가릴 수 없다.**

| 언제 깨지나 | 무슨 일이 나나 |
|---|---|
| ★**운영이 미디어 서버를 재기동** | 같은 주소에 ★**다른 프로세스**가 뜬다. DTLS 지문·ICE 자격이 전부 새것인데 클라는 *"같은 서버"* 로 보고 기존 연결을 재사용하려 든다 |
| 앞에 LB·TURN | 여러 노드가 ★**한 주소로 보인다** |

<a id="RULE-LINK-SFU-ID-IS-EPOCH"></a>**RULE-LINK-SFU-ID-IS-EPOCH** · 규칙

★★**`sfu_id` 는 `version.epoch`(§4-1-6)와 같은 값이다.** 한 필드가 두 일을 한다 —
*"어느 서버인가"* 와 *"그 서버가 재기동됐는가"* 는 같은 물음이기 때문이다.

<a id="RULE-LINK-PER-SERVER-UNITS"></a>**RULE-LINK-PER-SERVER-UNITS** · 규칙

이 문서 곳곳이 *"미디어 서버 하나당"* 을 전제하는데(§5-1-2 `AFFILIATION` · §4-2 연결 생성·정리),
클라는 ★**`sfu_id` 를 키로 방을 묶어** 그 단위를 만든다.

| 무엇 | 단위 |
|---|---|
| WebRTC 연결(발행·구독 한 쌍) | ★**서버 키마다 하나** — 방이 늘어도 연결은 하나다 |
| `READY` | ★**방마다**(§7-1-2) — 협상은 서버 단위지만 이 신호는 방 단위다 |
| `AFFILIATION` | ★**서버 키마다 따로** |
| `server_config` 보관 | ★`sfu_id` 마다 하나. 같은 `sfu_id` 의 방들은 **같은 값을 받는다** |

<a id="COND-LINK-SFU-ID-UNIQUE"></a>**COND-LINK-SFU-ID-UNIQUE** · 성립 조건

★**성립 조건** — `sfu_id`(= `version.epoch`, RULE-LINK-SFU-ID-IS-EPOCH)가 서버를 가리키려면 epoch 가 한 배포의 서버 사이에서도 겹치지 않아야 한다(기동 단위가 서버마다 고유하다). 겹치면 다른 서버의 방을 같은 연결로 알고 그 서버 효과가 엉뚱한 방에 간다(PROP-LINK-PER-SERVER).

<a id="LINK.type.CodecPolicy"></a>
#### 9-1-2. 형 `CodecPolicy` — 코덱 정책

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `kind` | "audio" / "video" | ✔ |  |
| `name` | str | ✔ | `"opus"` / `"VP8"` / `"H264"` … |
| `rtcp_fb` | str[] | ✔ |  |

<a id="LINK.type.ExtMap"></a>
#### 9-1-3. 형 `ExtMap` — 확장 헤더 번호

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `id` | u8 | ✔ |  |
| `uri` | str | ✔ |  |

<a id="LINK.op.ICE_RESTART"></a>
#### 9-1-4. `0x0305 ICE_RESTART` — 그 서버 연결을 다시 뚫는다   **C→S** · 응답 있음
단 1 · 출처 §7-1-2

**요청**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `sfu_id` | str | ✔ | ★필수 — 다시 뚫을 연결의 서버(§9-1-1). 게이트웨이는 이 값으로 그 서버를 찾는다 |

**응답**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `publish` | {ufrag, pwd} | ✔ | 발행 연결의 새 자격 — ★1pc 면 이것 하나가 그 연결 전부다(§9-4-18) |
| `subscribe` | {ufrag, pwd} | ★2pc 만 | 구독 연결의 새 자격 — ★2pc 만 |

`publish` 안:

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `publish.ufrag` | str | ✔ |  |
| `publish.pwd` | str | ✔ |  |

`subscribe` 안:

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `subscribe.ufrag` | str | ✔ |  |
| `subscribe.pwd` | str | ✔ |  |

**실패** — `3010` `PEER_NOT_FOUND`(그 서버에 이 세션의 연결이 없다 — 서버가 재기동됐거나 게이트웨이가 그 `sfu_id` 를 모른다. ★**치료 불가 확정**이다 → 그 서버 재구축, §9-2-1) · `1002` · `5xxx`(★같은 치료를 다시 — §9-2-1)

<a id="RULE-LINK-ICE-RESTART-SCOPE"></a>**RULE-LINK-ICE-RESTART-SCOPE** · 규칙

★**복구 2단**(§3-1)이다. ★**서버 하나의 연결만** 다룬다 — 언제 보내는지는 §9-2-1.

→ 구조 — [`LINK.C.RESTART.restart_ok`](#LINK.C.RESTART.restart_ok)

<a id="EX-LINK-ICE-RESTART-RES"></a>**EX-LINK-ICE-RESTART-RES** · 예시(비규범)

→ 구조 — [`LINK.example.EX-LINK-ICE-RESTART-RES`](#EX-LINK-ICE-RESTART-RES) · [`LINK.op.ICE_RESTART`](#LINK.op.ICE_RESTART)

예시 — `ICE_RESTART` 응답(형과 대조된 값 · 주석은 칸 설명)

```jsonc
{ "publish": {"ufrag":"…","pwd":"…"},      // 발행 연결의 새 자격 — ★1pc 면 이것 하나가 그 연결 전부다(§9-4-18)
  "subscribe": {"ufrag":"…","pwd":"…"} }   // 구독 연결의 새 자격 — ★2pc 만
```

<a id="RULE-LINK-UFRAG-IS-GENERATION"></a>**RULE-LINK-UFRAG-IS-GENERATION** · 규칙

★**응답의 `ufrag` 가 곧 세대다** — 이후 그 연결에 대한 `LINK_EVENT`(§4-2-6)가 이 값을 싣는다.

→ 구조 — [`LINK.C.any.link_event_stale`](#LINK.C.any.link_event_stale) · [`LINK.C.RESTART.restart_ok`](#LINK.C.RESTART.restart_ok)

<a id="RULE-LINK-ICE-RESTART-PROC"></a>**RULE-LINK-ICE-RESTART-PROC** · 규칙

**절차**

| 주체 | |
|---|---|
| 클라 | ★**WS 가 살아 있음을 먼저 확인한 뒤** 보낸다(§9-2-1 WS 점검) — 죽은 WS 위에서 기다리지 않는다 |
| 서버 | ★**자격 한 벌(연결 하나)마다** 새 ufrag·pwd 를 발급한다. ★**옛 것과 새 것을 함께 받는다** — 새 ufrag 로 검증된 첫 검사가 오면 옛 것을 내린다. ★**벌마다 최대 둘**이다 — 새 것이 쓰이기 전에 요청이 또 오면 그 대기분을 새 것으로 바꾼다. 상한의 단위가 세션이 아니라 벌인 것은 `2pc` 에서 한 연결의 재시작이 다른 연결의 대기분을 밀어내지 않게 하려는 것이다 |
| 서버 | ★**DTLS·SCTP·구독·발행·발언권을 그대로 둔다** — 바뀌는 것은 ICE 자격뿐이다 |
| 서버 | ★검사 응답의 MESSAGE-INTEGRITY 는 ★**요청 USERNAME 에 쓰인 ufrag 의 pwd** 로 계산한다 |
| 클라 | 새 자격으로 ★**다시 협상한다**(§9-4-9 restart 조립) — 협상 큐의 한 항목이다(§9-2-1). 바뀌는 재료는 ICE 자격 줄 둘뿐이다 |
| 클라 | ★**그 요청의 응답만 적용한다** — 재시도가 겹쳤을 때 늦게 온 응답이 새 자격을 덮지 않는다(세대) |

<a id="RULE-LINK-ICE-LITE"></a>**RULE-LINK-ICE-LITE** · 규칙

★**ICE-lite 서버는 검사를 보내지 않는다** — 클라의 새 자격을 서버에 알릴 필요가 없어 왕복 하나로 끝난다.

<a id="RULE-LINK-ICE-RESTART-FAIL"></a>**RULE-LINK-ICE-RESTART-FAIL** · 규칙

→ 구조 — [`LINK.op.ICE_RESTART`](#LINK.op.ICE_RESTART)

<a id="LINK.op.LINK_EVENT"></a>
#### 9-1-5. `0x0706 LINK_EVENT` — 그 서버 연결이 흔들린다   **S→C** · ACK · ★**신설** · unicast
단 1 · 출처 §4-2-6

**통지 · 받는 꼴**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `type` | "suspect" | ✔ |  |
| `sfu_id` | str | ✔ | ★그 연결의 서버(§9-1-1) |
| `ufrag` | str | ✔ | ★세대 — 서버가 검사를 못 받고 있는 그 연결의 현재 자격 |

<a id="EX-LINK-EVENT"></a>**EX-LINK-EVENT** · 예시(비규범)

→ 구조 — [`LINK.example.EX-LINK-EVENT`](#EX-LINK-EVENT) · [`LINK.op.LINK_EVENT`](#LINK.op.LINK_EVENT)

예시 — `LINK_EVENT` 통지(형과 대조된 값 · 주석은 칸 설명)

```jsonc
{ "type": "suspect",
  "sfu_id": "sfu-7f3a…",   // ★그 연결의 서버(§9-1-1)
  "ufrag": "…" }           // ★세대 — 서버가 검사를 못 받고 있는 그 연결의 현재 자격
```

<a id="RULE-LINK-EVENT-CONTRACT"></a>**RULE-LINK-EVENT-CONTRACT** · 규칙

| 항목 | 규격 |
|---|---|
| 방향 | S→C. ★**나에게만 unicast** |
| ★`version` | ★**없다** — 방 상태가 아니다 |
| 언제 | 그 서버에서 이 세션의 ★**연결 하나**(자격 한 벌 — `2pc` 면 발행·구독이 따로)의 ★**경로 쌍**(클라가 지명한 쌍, §9-4-9)이 생존을 잃었다 — UDP 쌍은 검증된 검사가 시한 안에 안 왔고 TCP 쌍은 연결이 없다(판정과 시한은 서버 몫, §1-2-6). ★**통지는 연결마다다** — `ufrag` 가 어느 연결인지를 가른다 |
| 클라 의무 | ACK + ★**`ufrag` 가 그 서버 연결들(발행·구독)의 지금 세대 어느 것과도 같지 않으면 버린다.** 맞으면 그 서버 연결의 ★**복구 방아쇠로 넣는다**(§9-2-1 — 이미 복구 중이면 흡수된다) |

→ 구조 — [`LINK.C.any.link_event_stale`](#LINK.C.any.link_event_stale) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event)

<a id="RULE-LINK-EVENT-TYPES"></a>**RULE-LINK-EVENT-TYPES** · 규칙

| `type` | 뜻 |
|---|---|
| `suspect` | ★현재 정의된 값은 이것 하나. 값 추가는 additive — 모르는 `type` 은 ACK 만 한다 |

→ 구조 — [`LINK.op.LINK_EVENT`](#LINK.op.LINK_EVENT)

<a id="LINK.op.MLINE_EVENT"></a>
#### 9-1-6. `0x0707 MLINE_EVENT` — 구독 m-line 배정표가 **바뀌었다**   **S→C** · ACK
단 1 · 출처 §8-1-2

**통지 · 받는 꼴**

| 칸 | 형 | 필수 | 뜻 |
|---|---|---|---|
| `table` | MlineTable | ✔ | ★**표 전체**(§8-1-2) — 바뀐 줄만이 아니다 |

<a id="RULE-LINK-MLINE-EVENT-FORM"></a>**RULE-LINK-MLINE-EVENT-FORM** · 규칙

★**`_EVENT` 인 이유** — 받은 클라가 구독 SDP 를 다시 짓는다. 구조를 고친다(각 장의 구조 이름 규율). ★**연결 하나의 것**이라 `room_id` 가 없다 — 방이 여럿이어도 표는 연결마다 하나다(§9-1-5 와 같은 축). ★그래서 표의 상태와 판단도 연결 개체(LINK — 서버마다)가 갖는다 — 방마다인 트랙 목록(TRK)은 트랙이 받는 트랙이 되거나 그칠 때 연결 표에 알릴 뿐이다.
★**1단이다**(§2-2-1). 다만 차례가 상태를 정하지 않는다 — 새것은 `arev` 가 가른다(§8-1-2).

→ 구조 — [`LINK.op.MLINE_EVENT`](#LINK.op.MLINE_EVENT)

<a id="PROP-LINK-TABLE-AGREES"></a>**PROP-LINK-TABLE-AGREES** · 성질

통로가 비면 클라 배정표가 서버 배정표다 — 그 서버의 방 전부의 트랙이 한 표에 든다 · 받는 트랙 · 쉬는 트랙 · 줄 없음 · 줄 수 전부(RULE-SUB-TABLE-PROC · RULE-SUB-ASSIGN-RECOVERED).

*식: CT2 · 깨지면: 서버는 mid 3 으로 보내는데 클라는 mid 2 의 m-line 에 붙여 둔다 — 화면이 검거나 남의 영상이 뜬다*

<a id="PROP-LINK-MID-UNIQUE"></a>**PROP-LINK-MID-UNIQUE** · 성질

내 연결 안에서 mid 하나는 한 트랙만 쥔다 — ★방 둘의 슬롯 줄 · 전이중 줄이 한 공간이다 · 표가 통째로 오므로 클라가 짐작할 것이 없다(RULE-SUB-TABLE-PROC · PROP-LINK-SERVER-MID-UNIQUE).

*식: CT3 · 깨지면: 한 m-line 에 트랙 둘이 걸린다 — 한 방의 슬롯 영상이 다른 방 슬롯 자리에 뜬다*

<a id="PROP-LINK-SERVER-MID-UNIQUE"></a>**PROP-LINK-SERVER-MID-UNIQUE** · 성질

서버도 한 mid 를 한 트랙에만 준다 — 방이 달라도 연결이 같으면 한 공간이다 · 빈 줄은 같은 kind 의 다음 트랙이 다시 쓴다(RULE-SUB-FILL · RULE-SUB-ASSIGN-RECOVERED).

*식: CT4 · 깨지면: 서버가 한 m-line 으로 트랙 둘을 보낸다*

<a id="PROP-LINK-TABLE-FOLLOWS-TRACKS"></a>**PROP-LINK-TABLE-FOLLOWS-TRACKS** · 성질

연결 표가 줄을 원하는 트랙은 그 서버의 방 어딘가에서 받는 트랙인 것과 같다 — 트랙 층이 바뀌는 걸음이 같은 걸음에 표에 알린다(PROP-TRK-TABLE-SIGNAL · RULE-SUB-FILL).

*식: CT21 · 깨지면: 떠난 트랙이 줄을 기다리다 재배정으로 줄을 빼앗거나 · 받는 트랙이 줄을 영영 못 받는다*

<a id="PROP-LINK-ROW-ONLY-RECEIVED"></a>**PROP-LINK-ROW-ONLY-RECEIVED** · 성질

받는 트랙이 아닌 트랙은 줄을 쥐지 않는다 — 쉬거나(full → half) 빠지거나 다른 서버로 옮긴 트랙의 줄은 빈 줄이 된다(RULE-TRK-PUB-TO-TRACK · RULE-SUB-RESTING-HAS-PT · RULE-SUB-REMOVE-KEEPS-MID).

*식: CT8 · CT9(통로가 비면 클라 표도) · 깨지면: 쉬는 트랙이 줄을 붙잡아 받는 트랙이 상한에 막힌다*

<a id="PROP-LINK-ROWS-REBUILD"></a>**PROP-LINK-ROWS-REBUILD** · 성질

줄이 바뀌는 걸음만 구독 SDP 재조립을 부른다 — 실린 트랙만 바뀌면(재배정 · 빈 줄 · 다시 채움) 다시 짓지 않고 READY 만 부른다 · 트랙 목록(TRACK_EVENT · 스냅샷)은 표를 건드리지 않는다(표가 TRK 에 없다 — 구조)(RULE-SUB-TABLE-PROC · RULE-SUB-TABLE-ONE-SOURCE).

*식: CT10 · CT16 · 깨지면: RESUME 으로 놓친 줄을 표에 넣고도 구독 SDP 에 안 실어 그 트랙이 영영 안 붙는다 · 재배정마다 재협상이 돈다 · HTTP 스냅샷이 m-line 을 되감는다*

<a id="PROP-LINK-ROWS-BOUNDED"></a>**PROP-LINK-ROWS-BOUNDED** · 성질

줄은 상한까지만 생기고 지우지 않는다 — 전이중 트랙은 슬롯 줄을 쥐지 않는다(RULE-SUB-FILL · RULE-SUB-TABLE-SERVER).

*식: CT17 · CT18 · 깨지면: 큰 방에서 m-line 이 BUNDLE 천장에 닿거나 · 지운 m-line 자리에 다른 kind 가 와 브라우저가 거부한다*

<a id="PROP-LINK-EGRESS-ACKED"></a>**PROP-LINK-EGRESS-ACKED** · 성질

서버가 그 트랙을 보내는 mid 는 늘 클라 표에 있고 그 줄은 적용된 SDP 에 있다 — 확인된 차례(READY{arev}) 이후 안 바뀐 줄로만 보낸다(RULE-SUB-EGRESS-ACKED).

*식: CT15 · 깨지면: mid 가 A 에서 B 로 넘어가는 사이에 서버가 B 를 그 mid 로 보낸다 — 클라는 아직 A 의 m-line 이라 남의 영상이 뜨거나 디코딩이 깨진다 · 아직 협상하지 않은 m-line 으로 보낸다*

<a id="LINK.fact.SdpClock"></a>
#### 9-1-7. 코덱 클럭 — `a=rtpmap` 뒤에 붙는 것

| 코덱 | `a=rtpmap` 뒤에 붙는 것 |
|---|---|
| `opus` | `opus/48000/2` |
| `VP8` · `VP9` · `H264` · `AV1` | `{name}/90000` |
| rtx | `rtx/{그 코덱의 clockrate}` |

<a id="LINK.fact.ServerExtMap"></a>
#### 9-1-8. 서버가 선언하는 확장 헤더 번호
**서버가 선언하는 extmap 번호** (`server_config.extmap`)

| id | uri | 어디에 | 무엇 |
|---|---|---|---|
| 1 | `urn:ietf:params:rtp-hdrext:sdes:mid` | 양쪽 | MID (BUNDLE 구분) |
| 4 | `urn:ietf:params:rtp-hdrext:ssrc-audio-level` | 오디오 | 오디오 음량 |
| 5 | `http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time` | 양쪽 | abs-send-time |
| 6 | `http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01` | 양쪽 | transport-cc |
| 10 | `urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id` | 시뮬캐스트 | rid (시뮬캐스트) |
| 11 | `urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id` | 시뮬캐스트 | 복구용 rid |

<a id="RULE-LINK-SDES-MID-BOTH"></a>**RULE-LINK-SDES-MID-BOTH** · 규칙

★**`sdes:mid`(1) 는 양쪽이다** — 발행은 브라우저가 자기 mid 를 싣는 자리이고, 구독에서는 서버가 확장 값을 그 m-line 의
배정표 줄의 `mid`(§8-1-1) 로 새로 쓴다(§9-4-2 ③ · §9-4-11).

→ 구조 — [`LINK.sdp.send`](#LINK.sdp.send) · [`LINK.sdp.recv`](#LINK.sdp.recv)

<a id="LINK.state"></a>
#### 9-1-9. 상태 칸 · 시한

##### 상태 — 클라

| 칸 | 처음 | 뜻 |
|---|---|---|
| `ls` | `'OK'` | 복구 단계 |
| `ice` | `'OK'` | ICE 층의 브라우저 사실 |
| `assoc` | `true` | SCTP 연관 · DTLS 가 살아 있다 |
| `dc` | `true` | DC 채널이 열려 있다 |
| `gen` | `1` | 적용한 ICE 자격의 세대(ufrag) |
| `pgen` | `0` | ICE_RESTART 응답이 준 새 세대 — 재협상으로 적용 중 |
| `req` | `0` | 지금 기다리는 ICE_RESTART 요청의 pid(0 = 없다) — 그 요청의 응답만 적용한다(§7-1-2) |
| `rid` | `0` | 마지막으로 매긴 요청 pid |
| `sync` | `true` | 끊긴 뒤 발언권을 다시 맞췄다(READY — §9-2-1 5) |
| `nego` | `false` | 재협상(구독 · 발행 SDP 다시 조립)이 진행 중이다(RULE-LINK-SDP-RENEGOTIATE) |
| `dirty` | `false` | 협상 중에 방아쇠가 또 왔다 — 끝나면 한 번 더 조립한다(협상 큐 · 연달아 온 것은 한 번에 — RULE-LINK-SDP-RENEGOTIATE) |
| `everUp` | `true` | 이 연결이 한 번이라도 섰다(ICE 가 열렸다) — 막힌 망이면 서지 못한 채 되살리기가 되풀이된다(RULE-LINK-RECOVERY-NEVER-UP) |
| `cause` | `'NONE'` | 재구축을 확정한 사건 |
| `mid` | `{A: 0, B: 0, Sa: 1, Sb: 2}` | 배정표의 사본 — 그 트랙이 실린 줄의 mid(0 = 줄이 없다) · ★연결마다 하나(방 둘의 트랙이 한 표에) · MLINE_EVENT 와 입장 · RESUME 응답만 고친다 |
| `rows` | `2` | 배정표의 SDP 재료 — 줄이 늘거나 줄 하나의 튜플이 바뀌면 바뀐다(같은지만 본다 · 모형은 줄이 늘 때만 바뀌는 수로 줄인다 — 튜플 바뀜도 SDP 를 바꾼다 §8-1-1) |
| `arev` | `0` | 받은 배정표의 차례 번호(arev) — 이보다 큰 표만 받는다 |

##### 상태 — 서버

| 칸 | 처음 | 뜻 |
|---|---|---|
| `known` | `true` | 그 서버가 이 세션의 연결을 안다(재기동하면 모른다 → 3010) |
| `use` | `1` | 쓰는 자격의 세대(ufrag) — 검증된 검사를 받는 것 |
| `pend` | `0` | 대기 자격의 세대(0 = 없다) — ICE_RESTART 가 낸 새 것 · 새 요청이 오면 바뀐다 · 첫 검사가 오면 쓰는 것이 된다(§7-1-2) |
| `next` | `2` | 다음에 낼 세대 |
| `busy` | `false` | 서버 사정(5xxx) |
| `mid` | `{A: 0, B: 0, Sa: 1, Sb: 2}` | 이 연결의 배정표 — 그 트랙이 실린 줄의 mid(0 = 줄이 없다) · ★연결마다 하나 · 슬롯 줄은 방에 들어갈 때 생긴다(세계: 방 둘에 들어가 있다) |
| `rows` | `2` | 줄 수 — 늘기만 한다 · 전이중 줄은 kind 마다 상한까지(RULE-SUB-FILL) |
| `arev` | `0` | 배정표의 차례 번호(arev) — 표가 바뀔 때마다 하나 오른다 |
| `since` | `{A: 0, B: 0, Sa: 0, Sb: 0}` | 그 트랙의 줄이 마지막으로 바뀐 차례 — 확인된 차례(ack) 이후 안 바뀐 줄만 보낸다(RULE-SUB-EGRESS-ACKED) |
| `ack` | `0` | 클라가 READY{arev} 로 확인한 차례 |
| `want` | `{A: false, B: false, Sa: true, Sb: true}` | 줄을 원하는 트랙 — 이 연결이 받는 방의 받는 트랙(TRK 가 link.track_on · link.track_off 로 알린다 · 슬롯은 늘) |

##### 전이 — 클라(동작 경우에서 짓는다)

| 지금 | 사건 | 다음 | 라벨 |
|---|---|---|---|
| 어느 상태 | ICE 가 disconnected 가 됐다(브라우저) | — | `LINK.C.any.ice_disc_leaving` |
| `OK` | ICE 가 disconnected 가 됐다(브라우저) | `WAIT` | `LINK.C.OK.ice_disc` |
| 어느 상태 | ICE 가 disconnected 가 됐다(브라우저) | `DISC` | `LINK.C.any.ice_disc_absorbed` |
| `WAIT` | ICE 가 스스로 회복했다(기다리는 동안) | `OK` | `LINK.C.WAIT.ice_back` |
| 어느 상태 | disconnected 기다림이 끝나도 안 풀렸다 · 또는 failed 가 됐다 | — | `LINK.C.any.wait_over_leaving` |
| `WAIT` | disconnected 기다림이 끝나도 안 풀렸다 · 또는 failed 가 됐다 | `CHECK` | `LINK.C.WAIT.wait_over` |
| `OK` | disconnected 기다림이 끝나도 안 풀렸다 · 또는 failed 가 됐다 | `CHECK` | `LINK.C.OK.wait_over` |
| 어느 상태 | disconnected 기다림이 끝나도 안 풀렸다 · 또는 failed 가 됐다 | — | `LINK.C.any.wait_over_absorbed` |
| 어느 상태 | LINK_EVENT{suspect} 를 받았다(§4-2-6) — 세대를 견준다 | — | `LINK.C.any.link_event_stale` |
| `OK` | LINK_EVENT{suspect} 를 받았다(§4-2-6) — 세대를 견준다 | `CHECK` | `LINK.C.OK.link_event` |
| 어느 상태 | LINK_EVENT{suspect} 를 받았다(§4-2-6) — 세대를 견준다 | — | `LINK.C.any.link_event_absorbed` |
| `CHECK` | WS 점검의 HEARTBEAT 응답 — 무엇이든 받았다(WS 가 산다) | `RESTART` | `LINK.C.CHECK.ws_alive` |
| 어느 상태 | WS 점검의 HEARTBEAT 응답 — 무엇이든 받았다(WS 가 산다) | — | `LINK.C.any.hb_res` |
| 어느 상태 | ICE_RESTART 응답을 받았다(§7-1-2) — 앞자리 뜻으로 받는다(§11-2-3) | — | `LINK.C.any.restart_res_other` |
| `RESTART` | ICE_RESTART 응답을 받았다(§7-1-2) — 앞자리 뜻으로 받는다(§11-2-3) | `APPLY` | `LINK.C.RESTART.restart_ok_queued` |
| `RESTART` | ICE_RESTART 응답을 받았다(§7-1-2) — 앞자리 뜻으로 받는다(§11-2-3) | `APPLY` | `LINK.C.RESTART.restart_ok` |
| `RESTART` | ICE_RESTART 응답을 받았다(§7-1-2) — 앞자리 뜻으로 받는다(§11-2-3) | `REBUILD` | `LINK.C.RESTART.restart_3xxx` |
| `RESTART` | ICE_RESTART 응답을 받았다(§7-1-2) — 앞자리 뜻으로 받는다(§11-2-3) | `CHECK` | `LINK.C.RESTART.restart_5xxx` |
| `APPLY` | 새 자격을 적용했는데 시한 안에 안 열렸다(적용 뒤 시한 만료 — 방아쇠) | `CHECK` | `LINK.C.APPLY.apply_timeout_blocked` |
| `APPLY` | 새 자격을 적용했는데 시한 안에 안 열렸다(적용 뒤 시한 만료 — 방아쇠) | `CHECK` | `LINK.C.APPLY.apply_timeout` |
| `APPLY` | 새 자격의 재협상이 끝나 ICE 가 섰다 — 새 자격을 실은 협상이 끝난 뒤다(협상 큐 — RULE-LINK-SDP-RENEGOTIATE) | `OK` | `LINK.C.APPLY.applied` |
| 어느 상태 | SCTP 연관이 사라졌다 · DTLS 가 닫혔다 | — | `LINK.C.any.assoc_lost_leaving` |
| `REBUILD` | SCTP 연관이 사라졌다 · DTLS 가 닫혔다 | 그대로 | `LINK.C.REBUILD.assoc_lost_absorbed` |
| 어느 상태 | SCTP 연관이 사라졌다 · DTLS 가 닫혔다 | `REBUILD · ASSOC` | `LINK.C.any.assoc_lost` |
| 어느 상태 | DC 채널만 닫혔다(연관은 산다) | — | `LINK.C.any.dc_closed_leaving` |
| `OK` | DC 채널만 닫혔다(연관은 산다) | `REOPEN` | `LINK.C.OK.dc_closed` |
| 어느 상태 | DC 채널만 닫혔다(연관은 산다) | — | `LINK.C.any.dc_closed_absorbed` |
| `REOPEN` | DC 다시 열기의 결과 | `OK` | `LINK.C.REOPEN.reopen_ok` |
| `REOPEN` | DC 다시 열기의 결과 | 그대로 | `LINK.C.REOPEN.reopen_retry` |
| `REOPEN` | DC 다시 열기의 결과 | `REBUILD` | `LINK.C.REOPEN.reopen_exhaust` |
| `REBUILD` | 그 서버 재구축이 끝났다 — 새 연결 · 다시 입장 · 다시 발행(g = 새 연결의 세대 · stood = 새 연결의 ICE 가 열렸다) | `CHECK · NONE` | `LINK.C.REBUILD.rebuilt_unstood` |
| `REBUILD` | 그 서버 재구축이 끝났다 — 새 연결 · 다시 입장 · 다시 발행(g = 새 연결의 세대 · stood = 새 연결의 ICE 가 열렸다) | `OK · NONE` | `LINK.C.REBUILD.rebuilt` |
| 어느 상태 | 세션이 미디어 연결을 전부 닫는다(RULE-SES-REBUILD 2 · RULE-SES-ON-DROP 2·3) | `NONE` | `LINK.C.any.close_all` |
| 어느 상태 | 그 서버의 방 하나를 떠났다 — last = 그 서버에 남은 방이 없다(RULE-ROOM-LEAVE-RES 2) | `NONE` | `LINK.C.any.last_room_left` |
| 어느 상태 | 그 서버의 방 하나를 떠났다 — last = 그 서버에 남은 방이 없다(RULE-ROOM-LEAVE-RES 2) | — | `LINK.C.any.room_left` |
| 어느 상태 | RESUME 뒤 죽은 연결을 층별 판정대로 되살린다(RULE-SES-RES-RESUME-OK 4 · RULE-LINK-RECOVERY-UNIT — 처음부터 다시 판정) | `REBUILD · ASSOC` | `LINK.C.any.repair_rebuild` |
| 어느 상태 | RESUME 뒤 죽은 연결을 층별 판정대로 되살린다(RULE-SES-RES-RESUME-OK 4 · RULE-LINK-RECOVERY-UNIT — 처음부터 다시 판정) | `RESTART` | `LINK.C.any.repair_restart` |
| 어느 상태 | RESUME 뒤 죽은 연결을 층별 판정대로 되살린다(RULE-SES-RES-RESUME-OK 4 · RULE-LINK-RECOVERY-UNIT — 처음부터 다시 판정) | — | `LINK.C.any.repair_ok` |
| `REBUILD` | 그 서버의 방 version.epoch 가 바뀌었다 — 서버가 재기동돼 그 연결을 모른다 · 그 서버만 재구축한다(RULE-SES-RES-RESUME-OK 4 · RULE-LINK-RECOVERY-UNIT) | 그대로 | `LINK.C.REBUILD.epoch_absorbed` |
| 어느 상태 | 그 서버의 방 version.epoch 가 바뀌었다 — 서버가 재기동돼 그 연결을 모른다 · 그 서버만 재구축한다(RULE-SES-RES-RESUME-OK 4 · RULE-LINK-RECOVERY-UNIT) | `REBUILD · EPOCH` | `LINK.C.any.epoch_rebuild` |
| `REBUILD` | RESUME 응답에 내 발행이 없다 — 서버가 그 연결을 모른다 · 그 서버만 재구축한다(RULE-SES-RES-RESUME-OK 2 ③ · 4) | 그대로 | `LINK.C.REBUILD.publost_absorbed` |
| 어느 상태 | RESUME 응답에 내 발행이 없다 — 서버가 그 연결을 모른다 · 그 서버만 재구축한다(RULE-SES-RES-RESUME-OK 2 ③ · 4) | `REBUILD · PUB_LOST` | `LINK.C.any.publost_rebuild` |
| 어느 상태 | 구독 · 발행 SDP 를 다시 조립해 협상한다(RULE-LINK-SDP-RENEGOTIATE · 그 서버 전체 — RULE-TRK-TRACK-EVENT-PROC) | — | `LINK.C.any.renego_queued` |
| 어느 상태 | 구독 · 발행 SDP 를 다시 조립해 협상한다(RULE-LINK-SDP-RENEGOTIATE · 그 서버 전체 — RULE-TRK-TRACK-EVENT-PROC) | — | `LINK.C.any.renego` |
| 어느 상태 | 재협상이 끝났다 — ok = 성공 | — | `LINK.C.any.nego_ok_next` |
| 어느 상태 | 재협상이 끝났다 — ok = 성공 | — | `LINK.C.any.nego_ok` |
| 어느 상태 | 재협상이 끝났다 — ok = 성공 | — | `LINK.C.any.nego_fail_next` |
| 어느 상태 | 재협상이 끝났다 — ok = 성공 | — | `LINK.C.any.nego_fail` |
| 어느 상태 | MLINE_EVENT 를 받았다 — 그 서버 연결의 배정표 전체다(SUB.type.MlineTable) · 보관한 차례(arev)보다 크면 통째로 갈아 끼운다(RULE-SUB-TABLE-PROC) · ★표는 연결의 것이라 그 서버의 방 전부가 한 표를 쓴다 | — | `LINK.C.any.table_same` |
| 어느 상태 | MLINE_EVENT 를 받았다 — 그 서버 연결의 배정표 전체다(SUB.type.MlineTable) · 보관한 차례(arev)보다 크면 통째로 갈아 끼운다(RULE-SUB-TABLE-PROC) · ★표는 연결의 것이라 그 서버의 방 전부가 한 표를 쓴다 | — | `LINK.C.any.table` |
| 어느 상태 | MLINE_EVENT 를 받았다 — 그 서버 연결의 배정표 전체다(SUB.type.MlineTable) · 보관한 차례(arev)보다 크면 통째로 갈아 끼운다(RULE-SUB-TABLE-PROC) · ★표는 연결의 것이라 그 서버의 방 전부가 한 표를 쓴다 | — | `LINK.C.any.table_rebound` |
| 어느 상태 | MLINE_EVENT 를 받았다 — 그 서버 연결의 배정표 전체다(SUB.type.MlineTable) · 보관한 차례(arev)보다 크면 통째로 갈아 끼운다(RULE-SUB-TABLE-PROC) · ★표는 연결의 것이라 그 서버의 방 전부가 한 표를 쓴다 | — | `LINK.C.any.table_old` |
| 어느 상태 | 입장 응답 · RESUME 응답이 실어 온 그 서버의 배정표 — MLINE_EVENT 와 같게 받는다(RULE-SUB-TABLE-PROC · 어느 길로 와도 새 arev 가 이긴다) | — | `LINK.C.any.table_carried_same` |
| 어느 상태 | 입장 응답 · RESUME 응답이 실어 온 그 서버의 배정표 — MLINE_EVENT 와 같게 받는다(RULE-SUB-TABLE-PROC · 어느 길로 와도 새 arev 가 이긴다) | — | `LINK.C.any.table_carried` |
| 어느 상태 | 입장 응답 · RESUME 응답이 실어 온 그 서버의 배정표 — MLINE_EVENT 와 같게 받는다(RULE-SUB-TABLE-PROC · 어느 길로 와도 새 arev 가 이긴다) | — | `LINK.C.any.table_carried_rebound` |
| 어느 상태 | 입장 응답 · RESUME 응답이 실어 온 그 서버의 배정표 — MLINE_EVENT 와 같게 받는다(RULE-SUB-TABLE-PROC · 어느 길로 와도 새 arev 가 이긴다) | — | `LINK.C.any.table_carried_old` |

<a id="LINK.C"></a>
### 9-2. 동작

<a id="LINK.C.any"></a>
#### 9-2-1. 클라 · `any`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="LINK.C.any.ice_disc_leaving"></a>`LINK.C.any.ice_disc_leaving` | leaving && ls != 'REBUILD' | — | §4-2-5 2 §9-2-1 3 — 그 서버의 마지막 방을 나가는 중 — 서버가 Peer 를 거두며 닫는 사실이다 · 재구축 방아쇠로 받지 않는다(20261023 — 퇴장 응답보다 먼저 와 재구축으로 가 leave() 가 안 끝나던 퇴장 경주) |
| <a id="LINK.C.any.ice_disc_absorbed"></a>`LINK.C.any.ice_disc_absorbed` | 나머지 | ice ← 'DISC' | §9-2-1 3 — 받은 브라우저 사실은 적는다(ice) — 진행 중인 복구는 그대로 |
| <a id="LINK.C.any.wait_over_leaving"></a>`LINK.C.any.wait_over_leaving` | leaving && ls != 'REBUILD' | — | §4-2-5 2 §9-2-1 3 — 그 서버의 마지막 방을 나가는 중 — 서버가 Peer 를 거두며 닫는 사실이다 · 재구축 방아쇠로 받지 않는다(20261023 — 퇴장 응답보다 먼저 와 재구축으로 가 leave() 가 안 끝나던 퇴장 경주) |
| <a id="LINK.C.any.wait_over_absorbed"></a>`LINK.C.any.wait_over_absorbed` | 나머지 | ice ← failed ? 'FAILED' : ice | §9-2-1 3 — failed 사실이면 적는다(ice) — 진행 중인 복구는 그대로 |
| <a id="LINK.C.any.link_event_stale"></a>`LINK.C.any.link_event_stale` | m.ufrag != gen | — | §4-2-6 §4-2-3 — 지금 세대가 아니다 — 버린다(끝난 복구를 다시 시작하지 않는다) |
| <a id="LINK.C.any.link_event_absorbed"></a>`LINK.C.any.link_event_absorbed` | 나머지 | — | §9-2-1 3 R-LINK-RECOVERY-ONE-OWNER-WHY — 이미 복구 중이면 흡수된다 |
| <a id="LINK.C.any.hb_res"></a>`LINK.C.any.hb_res` | 나머지 | — | §3-2-4 |
| <a id="LINK.C.any.restart_res_other"></a>`LINK.C.any.restart_res_other` | ls != 'RESTART' \|\| m.pid != req | — | §7-1-2 — 그 요청의 응답만 적용한다 — 재시도가 겹쳤을 때 늦게 온 응답이 새 자격을 덮지 않는다(세대) |
| <a id="LINK.C.any.assoc_lost_leaving"></a>`LINK.C.any.assoc_lost_leaving` | leaving && ls != 'REBUILD' | — | §4-2-5 2 §9-2-1 3 — 그 서버의 마지막 방을 나가는 중 — 서버가 Peer 를 거두며 닫는 사실이다 · 재구축 방아쇠로 받지 않는다(20261023 — 퇴장 응답보다 먼저 와 재구축으로 가 leave() 가 안 끝나던 퇴장 경주) |
| <a id="LINK.C.any.assoc_lost"></a>`LINK.C.any.assoc_lost` | 나머지 | ls ← 'REBUILD'; cause ← 'ASSOC'; assoc ← false; dc ← false; sync ← false; pgen ← 0; req ← 0 · 효과 floor.down_server → FLR.C.any.down · 효과 store.drop_server → ROOM.C.LEAVING.down · ROOM.C.any.down · 효과 conn.close · 효과 rooms.rejoin_server → ROOM.C.NONE.rejoin · 효과 pub.reregister_server → PUB.C.any.reregister · PUB.C.any.reregister_off · 효과 app.link_rebuilding | §9-2-1 2 — 치료 불가 확정 ② — 브라우저는 같은 연결 위에 연관을 다시 세우지 않는다 |
| <a id="LINK.C.any.dc_closed_leaving"></a>`LINK.C.any.dc_closed_leaving` | leaving && ls != 'REBUILD' | — | §4-2-5 2 §9-2-1 3 — 그 서버의 마지막 방을 나가는 중 — 서버가 Peer 를 거두며 닫는 사실이다 · 재구축 방아쇠로 받지 않는다(20261023 — 퇴장 응답보다 먼저 와 재구축으로 가 leave() 가 안 끝나던 퇴장 경주) |
| <a id="LINK.C.any.dc_closed_absorbed"></a>`LINK.C.any.dc_closed_absorbed` | 나머지 | dc ← false · 효과 floor.dc_down → FLR.C.any.dc_down | §9-2-1 3 |
| <a id="LINK.C.any.close_all"></a>`LINK.C.any.close_all` | 받는 효과 SES media.close_all · 나머지 | ls ← 'NONE'; dc ← false; sync ← false; nego ← false; dirty ← false; pgen ← 0; req ← 0 · 효과 pub.drop_server → PUB.C.any.drop_server · 효과 conn.close | §3-2-3 2 §3-2-4 2 — 연결이 닫히면 서버가 Peer 와 함께 발행 트랙을 푼다 — 발행 상태를 IDLE 로(RULE-ROOM-LEAVE-RES 2) |
| <a id="LINK.C.any.last_room_left"></a>`LINK.C.any.last_room_left` | 받는 효과 ROOM link.room_left · last | ls ← 'NONE'; dc ← false; sync ← false; nego ← false; dirty ← false; pgen ← 0; req ← 0 · 효과 pub.drop_server → PUB.C.any.drop_server · 효과 conn.close | §4-2-5 2 — 서버의 세션 창을 기다리지 않는다 · 클라는 PUBLISH_TRACKS{remove} 를 보내지 않고 그 서버 발행을 IDLE 로 내린다 |
| <a id="LINK.C.any.room_left"></a>`LINK.C.any.room_left` | 받는 효과 ROOM link.room_left · 나머지 | — | §4-2-5 1 — 남은 방이 있다 — 구독 SDP 를 다시 조립한다(ROOM 이 sdp.rebuild 로 따로 알린다) |
| <a id="LINK.C.any.repair_rebuild"></a>`LINK.C.any.repair_rebuild` | 받는 효과 SES media.repair · !assoc | ls ← 'REBUILD'; cause ← 'ASSOC'; dc ← false; sync ← false; pgen ← 0; req ← 0 · 효과 floor.down_server → FLR.C.any.down · 효과 store.drop_server → ROOM.C.LEAVING.down · ROOM.C.any.down · 효과 conn.close · 효과 rooms.rejoin_server → ROOM.C.NONE.rejoin · 효과 pub.reregister_server → PUB.C.any.reregister · PUB.C.any.reregister_off · 효과 app.link_rebuilding | §3-2-3 4 §9-2-1 2 — DTLS · SCTP 가 닫혔다 — 치료 불가 확정 ②와 같다(RULE-LINK-RECOVERY-REBUILD) |
| <a id="LINK.C.any.repair_restart"></a>`LINK.C.any.repair_restart` | 받는 효과 SES media.repair · ice != 'OK' | ls ← 'RESTART'; rid ← rid + 1; req ← rid + 1 · 보낸다 ICE_RESTART.req · 효과 app.link_reviving | §3-2-3 4 §9-2-1 1 — ICE 층이면 재시작 — WS 는 방금 RESUME 으로 산 것이 확인됐다(점검을 건너뛴다) |
| <a id="LINK.C.any.repair_ok"></a>`LINK.C.any.repair_ok` | 받는 효과 SES media.repair · 나머지 | — | §3-2-3 3 — 층이 다 서 있다 — 이어 쓴다 |
| <a id="LINK.C.any.epoch_rebuild"></a>`LINK.C.any.epoch_rebuild` | 받는 효과 ROOM link.epoch_changed · 나머지 | ls ← 'REBUILD'; cause ← 'EPOCH'; dc ← false; sync ← false; pgen ← 0; req ← 0 · 효과 floor.down_server → FLR.C.any.down · 효과 store.drop_server → ROOM.C.LEAVING.down · ROOM.C.any.down · 효과 conn.close · 효과 rooms.rejoin_server → ROOM.C.NONE.rejoin · 효과 pub.reregister_server → PUB.C.any.reregister · PUB.C.any.reregister_off · 효과 app.link_rebuilding | §3-2-3 4 §9-2-1 2 — 서버가 연결을 모른다 — 치료 불가 확정과 같다(RULE-LINK-RECOVERY-REBUILD) |
| <a id="LINK.C.any.publost_rebuild"></a>`LINK.C.any.publost_rebuild` | 받는 효과 PUB link.pub_lost · 나머지 | ls ← 'REBUILD'; cause ← 'PUB_LOST'; dc ← false; sync ← false; pgen ← 0; req ← 0 · 효과 floor.down_server → FLR.C.any.down · 효과 store.drop_server → ROOM.C.LEAVING.down · ROOM.C.any.down · 효과 conn.close · 효과 rooms.rejoin_server → ROOM.C.NONE.rejoin · 효과 pub.reregister_server → PUB.C.any.reregister · PUB.C.any.reregister_off · 효과 app.link_rebuilding | §3-2-3 4 §9-2-1 2 — 서버가 연결을 모른다 — epoch 가 바뀐 것과 같다(RULE-LINK-RECOVERY-REBUILD) |
| <a id="LINK.C.any.renego_queued"></a>`LINK.C.any.renego_queued` | 받는 효과 ROOM sdp.rebuild · PUB sdp.rebuild · SUB sdp.rebuild · nego | dirty ← true | §9-2-1 §9-2-1 1 — 앞선 협상이 진행 중이다 — 다시 조립할 것이 생겼다고만 적는다 · 끝나면 한 번 더 조립한다(협상 큐 · 연달아 온 것은 한 번에 — RULE-LINK-SDP-RENEGOTIATE · RULE-LINK-SDP-1PC-PROC 1) |
| <a id="LINK.C.any.renego"></a>`LINK.C.any.renego` | 받는 효과 ROOM sdp.rebuild · PUB sdp.rebuild · SUB sdp.rebuild · 나머지 | nego ← true | §9-2-1 §4-2-6 — 연달아 온 것은 다 반영한 뒤 한 번 조립해도 된다 · 1pc 면 협상 큐(RULE-LINK-SDP-1PC-PROC) |
| <a id="LINK.C.any.nego_ok_next"></a>`LINK.C.any.nego_ok_next` | ok && dirty | dirty ← false · 효과 sub.nego_ok → SUB.C.WANT.nego_ok · SUB.C.any.nego_ok_other · 효과 pub.nego_ok → PUB.C.STAGED.nego_ok_unwanted · PUB.C.STAGED.nego_ok · PUB.C.STAGED.nego_fail | §9-2-1 §9-2-1 1 — 이번 협상은 끝났다 — 그 사이 적어 둔 것을 싣고 한 번 더 조립한다(협상은 이어서 진행 중 · RULE-LINK-SDP-RENEGOTIATE) |
| <a id="LINK.C.any.nego_ok"></a>`LINK.C.any.nego_ok` | ok | nego ← false · 효과 sub.nego_ok → SUB.C.WANT.nego_ok · SUB.C.any.nego_ok_other · 효과 pub.nego_ok → PUB.C.STAGED.nego_ok_unwanted · PUB.C.STAGED.nego_ok · PUB.C.STAGED.nego_fail | §9-2-1 §4-2-6 |
| <a id="LINK.C.any.nego_fail_next"></a>`LINK.C.any.nego_fail_next` | dirty | dirty ← false · 효과 sub.nego_fail → SUB.C.WANT.nego_fail · SUB.C.any.nego_fail_other · 효과 pub.nego_fail → PUB.C.STAGED.nego_ok_unwanted · PUB.C.STAGED.nego_ok · PUB.C.STAGED.nego_fail | §7-1-2 §9-2-1 — 이번 협상은 실패했다(READY 를 보내지 않는다) — 그 사이 적어 둔 것은 아직 반영되지 않았으니 한 번 더 조립한다(실패했다고 모드를 바꾸지 않는다 — RULE-LINK-SDP-1PC-PROC 5) |
| <a id="LINK.C.any.nego_fail"></a>`LINK.C.any.nego_fail` | 나머지 | nego ← false · 효과 sub.nego_fail → SUB.C.WANT.nego_fail · SUB.C.any.nego_fail_other · 효과 pub.nego_fail → PUB.C.STAGED.nego_ok_unwanted · PUB.C.STAGED.nego_ok · PUB.C.STAGED.nego_fail | §7-1-2 — 협상이 실패했으면 READY 를 보내지 않는다(RULE-SUB-READY-PROC) |
| <a id="LINK.C.any.table_same"></a>`LINK.C.any.table_same` | m.table.arev > arev && m.table.mid == mid && m.table.rows == rows | arev ← m.table.arev | §8-1-2 — 새 차례지만 표 내용이 그대로다 — 차례만 올린다 · SDP 를 건드리지 않는다(재협상도 READY 도 없다) |
| <a id="LINK.C.any.table"></a>`LINK.C.any.table` | m.table.arev > arev && m.table.rows != rows | mid ← m.table.mid; rows ← m.table.rows; arev ← m.table.arev · 효과 sub.table_changed → SUB.C.any.table_changed | §8-1-2 §8-1-2 — 줄이 늘었다 — 표 전체를 갈아 끼운다 · 그 서버의 방마다 구독이 SDP 를 이 표만으로 다시 짓고 협상이 끝나면 READY{arev} |
| <a id="LINK.C.any.table_rebound"></a>`LINK.C.any.table_rebound` | m.table.arev > arev | mid ← m.table.mid; arev ← m.table.arev · 효과 sub.table_rebound → SUB.C.WANT.table_rebound · SUB.C.NONE.table_rebound · SUB.C.any.table_rebound | §8-1-2 §8-1-2 — 줄은 그대로 · 실린 트랙만 바뀌었다(재배정 · 빈 줄 · 다시 채움) — SDP 재료가 그대로라 다시 짓지 않는다 · 화면만 다시 잇고 READY{arev}(구독이 보낸다) |
| <a id="LINK.C.any.table_old"></a>`LINK.C.any.table_old` | 나머지 | — | §8-1-2 — 보관한 것보다 낡은(같은) 표 — 버린다(입장 응답 · RESUME 응답 · 통지가 어떤 차례로 닿아도 새것이 이긴다) |
| <a id="LINK.C.any.table_carried_same"></a>`LINK.C.any.table_carried_same` | 받는 효과 ROOM link.table · tArev > arev && tMid == mid && tRows == rows | arev ← tArev | §8-1-2 — 새 차례지만 표 내용이 그대로다 — 차례만 올린다 |
| <a id="LINK.C.any.table_carried"></a>`LINK.C.any.table_carried` | 받는 효과 ROOM link.table · tArev > arev && tRows != rows | mid ← tMid; rows ← tRows; arev ← tArev · 효과 sub.table_changed → SUB.C.any.table_changed | §8-1-2 §8-1-2 — 줄이 바뀌었다 — 표 전체를 갈아 끼운다 · 구독 SDP 를 다시 짓는다 |
| <a id="LINK.C.any.table_carried_rebound"></a>`LINK.C.any.table_carried_rebound` | 받는 효과 ROOM link.table · tArev > arev | mid ← tMid; arev ← tArev · 효과 sub.table_rebound → SUB.C.WANT.table_rebound · SUB.C.NONE.table_rebound · SUB.C.any.table_rebound | §8-1-2 §8-1-2 — 줄은 그대로 · 실린 트랙만 바뀌었다 — 다시 짓지 않는다 · READY{arev} |
| <a id="LINK.C.any.table_carried_old"></a>`LINK.C.any.table_carried_old` | 받는 효과 ROOM link.table · 나머지 | — | §8-1-2 — 이미 그보다 새 표를 받았다 — 버린다 |

<a id="RULE-LINK-RECOVERY-UNIT"></a>**RULE-LINK-RECOVERY-UNIT** · 규칙 — ★그 서버의 미디어 연결 — 흔들림·되살림·재구축 (`JOINED`)

★**단위는 미디어 서버 하나의 연결이다**(성립 조건 색인 — 복구 단위). ★**다른 서버의 방·발행은 건드리지 않는다.**
WS 는 살아 있다는 전제다 — WS 가 죽었으면 세션 층(§3-2-4 · §3-2-3)이 먼저이고, 그동안 이 절은 멈췄다가 `RESUME` 응답 뒤
★**처음부터 다시 판정한다**(§3-2-3 4).

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.RESTART.restart_ok`](#LINK.C.RESTART.restart_ok) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.OK.dc_closed`](#LINK.C.OK.dc_closed) · [`LINK.C.REOPEN.reopen_retry`](#LINK.C.REOPEN.reopen_retry) · [`LINK.C.any.repair_restart`](#LINK.C.any.repair_restart) |
| 2 | [`LINK.C.RESTART.restart_3xxx`](#LINK.C.RESTART.restart_3xxx) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost`](#LINK.C.any.assoc_lost) · [`LINK.C.REOPEN.reopen_exhaust`](#LINK.C.REOPEN.reopen_exhaust) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) · [`LINK.C.any.repair_rebuild`](#LINK.C.any.repair_rebuild) · [`LINK.C.any.epoch_rebuild`](#LINK.C.any.epoch_rebuild) · [`LINK.C.any.publost_rebuild`](#LINK.C.any.publost_rebuild) · [`LINK.S.new_conn`](#LINK.S.new_conn) · [`PUB.C.any.reregister`](07-pub.md#PUB.C.any.reregister) · [`PUB.C.any.reregister_off`](07-pub.md#PUB.C.any.reregister_off) · [`ROOM.C.any.down`](04-room.md#ROOM.C.any.down) · [`ROOM.C.NONE.rejoin`](04-room.md#ROOM.C.NONE.rejoin) |
| 3 | [`LINK.C.any.ice_disc_leaving`](#LINK.C.any.ice_disc_leaving) · [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.any.ice_disc_absorbed`](#LINK.C.any.ice_disc_absorbed) · [`LINK.C.any.wait_over_leaving`](#LINK.C.any.wait_over_leaving) · [`LINK.C.any.wait_over_absorbed`](#LINK.C.any.wait_over_absorbed) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.any.link_event_absorbed`](#LINK.C.any.link_event_absorbed) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost_leaving`](#LINK.C.any.assoc_lost_leaving) · [`LINK.C.REBUILD.assoc_lost_absorbed`](#LINK.C.REBUILD.assoc_lost_absorbed) · [`LINK.C.any.dc_closed_leaving`](#LINK.C.any.dc_closed_leaving) · [`LINK.C.any.dc_closed_absorbed`](#LINK.C.any.dc_closed_absorbed) · [`LINK.C.REBUILD.epoch_absorbed`](#LINK.C.REBUILD.epoch_absorbed) · [`LINK.C.REBUILD.publost_absorbed`](#LINK.C.REBUILD.publost_absorbed) |
| 4 | [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.CHECK.ws_alive`](#LINK.C.CHECK.ws_alive) |
| 5 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REOPEN.reopen_ok`](#LINK.C.REOPEN.reopen_ok) |
| 7 | [`LINK.C.APPLY.apply_timeout_blocked`](#LINK.C.APPLY.apply_timeout_blocked) · [`LINK.C.REBUILD.rebuilt_unstood`](#LINK.C.REBUILD.rebuilt_unstood) |
| 8 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) |

<a id="RULE-LINK-RECOVERY-TIERS"></a>**RULE-LINK-RECOVERY-TIERS** · 규칙

1. ★★**층마다 사실을 따로 보고, 가장 아래에서 고장 난 층만 고친다.** 아래층이 고장이면 윗층 관찰은 믿을 수 없다 —
   아래층이 서면 윗층을 다시 판정한다.

   | 층 | 무엇을 보나(브라우저의 사실) | 고치는 법 | 고칠 수 없다고 확정되는 것 |
   |---|---|---|---|
   | ICE | `iceConnectionState` | `disconnected` → ★**기다린다**(스스로 풀릴 수 있다 — W3C 정의가 *"transient"*) · 기다림이 끝나도 안 풀리거나 `failed` 거나 `LINK_EVENT`(§4-2-6) → ★**ICE 재시작**(`ICE_RESTART`, §7-1-2 — `failed` 는 재시작 전까지 종착이다) | `ICE_RESTART` 가 `3xxx`(`3010` — 서버가 그 연결을 모른다) |
   | DTLS | DTLS 전송 상태 | 없다 — ICE 가 서면 이어진다(지문 불변, §9-4-9) | 닫힘 |
   | SCTP 연관 | 데이터 채널 전송의 연관 상태 | 없다 — 브라우저는 같은 연결 위에 연관을 다시 세우지 않는다 | 닫힘 |
   | DC 채널 | `readyState` | 같은 연결 위에 ★**다시 연다**(§10-4-23) | 되풀이 소진 |

   ★`2pc` 면 연결이 둘이다 — 연결마다 사실을 받아 ★**더 나쁜 쪽으로 합친다.** ICE 재시작은 한 요청으로 둘을 함께 한다(응답이 두 벌).
   ★**기다림은 ICE `disconnected` 에만 돈다** — DC 채널만 닫혔으면 다시 열기만 한다(ICE 가 멀쩡한데 재시작이 나가지 않게).
   ★**응답은 코드마다 칸을 두지 않고 앞자리 뜻으로 받는다**(§11-2-3) — `3xxx` 는 치료 불가 확정 · `5xxx` 는 같은 치료를 간격을
   벌려 다시 · 답 없음은 WS 의 증상(4) · `1xxx` 는 클라 버그다. 코드가 늘어도 칸이 비지 않는다.
   ★기다리는 시간(ICE `disconnected`)·DC 다시 열기의 횟수와 간격은 이 문서가 정하지 않는다 — 클라 구현의 기본값이다.
   ★**같은 치료를 다시 할 때의 간격은 §3-2-3 사다리 칸값**을 쓰고, 칸을 다 쓰면 ★**마지막 칸을 되풀이한다** — 횟수 상한이 없다(7).

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.RESTART.restart_ok`](#LINK.C.RESTART.restart_ok) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.OK.dc_closed`](#LINK.C.OK.dc_closed) · [`LINK.C.REOPEN.reopen_retry`](#LINK.C.REOPEN.reopen_retry) · [`LINK.C.any.repair_restart`](#LINK.C.any.repair_restart) |
| 2 | [`LINK.C.RESTART.restart_3xxx`](#LINK.C.RESTART.restart_3xxx) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost`](#LINK.C.any.assoc_lost) · [`LINK.C.REOPEN.reopen_exhaust`](#LINK.C.REOPEN.reopen_exhaust) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) · [`LINK.C.any.repair_rebuild`](#LINK.C.any.repair_rebuild) · [`LINK.C.any.epoch_rebuild`](#LINK.C.any.epoch_rebuild) · [`LINK.C.any.publost_rebuild`](#LINK.C.any.publost_rebuild) · [`LINK.S.new_conn`](#LINK.S.new_conn) · [`PUB.C.any.reregister`](07-pub.md#PUB.C.any.reregister) · [`PUB.C.any.reregister_off`](07-pub.md#PUB.C.any.reregister_off) · [`ROOM.C.any.down`](04-room.md#ROOM.C.any.down) · [`ROOM.C.NONE.rejoin`](04-room.md#ROOM.C.NONE.rejoin) |
| 3 | [`LINK.C.any.ice_disc_leaving`](#LINK.C.any.ice_disc_leaving) · [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.any.ice_disc_absorbed`](#LINK.C.any.ice_disc_absorbed) · [`LINK.C.any.wait_over_leaving`](#LINK.C.any.wait_over_leaving) · [`LINK.C.any.wait_over_absorbed`](#LINK.C.any.wait_over_absorbed) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.any.link_event_absorbed`](#LINK.C.any.link_event_absorbed) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost_leaving`](#LINK.C.any.assoc_lost_leaving) · [`LINK.C.REBUILD.assoc_lost_absorbed`](#LINK.C.REBUILD.assoc_lost_absorbed) · [`LINK.C.any.dc_closed_leaving`](#LINK.C.any.dc_closed_leaving) · [`LINK.C.any.dc_closed_absorbed`](#LINK.C.any.dc_closed_absorbed) · [`LINK.C.REBUILD.epoch_absorbed`](#LINK.C.REBUILD.epoch_absorbed) · [`LINK.C.REBUILD.publost_absorbed`](#LINK.C.REBUILD.publost_absorbed) |
| 4 | [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.CHECK.ws_alive`](#LINK.C.CHECK.ws_alive) |
| 5 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REOPEN.reopen_ok`](#LINK.C.REOPEN.reopen_ok) |
| 7 | [`LINK.C.APPLY.apply_timeout_blocked`](#LINK.C.APPLY.apply_timeout_blocked) · [`LINK.C.REBUILD.rebuilt_unstood`](#LINK.C.REBUILD.rebuilt_unstood) |
| 8 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) |

<a id="RULE-LINK-RECOVERY-REBUILD"></a>**RULE-LINK-RECOVERY-REBUILD** · 규칙

2. ★★**치료 불가가 확정되면 그 서버만 재구축한다.** ★**시한·횟수는 확정 근거가 아니다** — 적용했는데 안 열리는 것은 경로가
   막힌 것이고, 재구축은 같은 경로에 새 연결을 만들 뿐 고치지 못한다. 그때는 ★**같은 칸에서 간격을 벌려 다시 한다**
   (`ICE_RESTART` 의 `5xxx` 도 같다). 확정 사건은 넷이다 — ①`ICE_RESTART` 응답 `3xxx` ②SCTP 연관
   소멸·DTLS 닫힘(연관이 사라지면 브라우저는 같은 연결 위에 다시 세우지 않는다) ③DC 다시 열기 소진(연관이 산 채로 채널만 안 서는 것은 ★**서버 쪽 채널 고장**이고 재구축이 유일한 수단이다)
   ④`RESUME` 스냅샷이 그 서버 연결을 모른다(§3-2-3 4).
   ★**재구축 순서** — 그 서버 방들의 발언권을 `has no permission` 으로 내리고 앱에 알린다(DC 가 없으니 통지가 오지 않는다 —
   §3-2-3 1-1 과 같은 이유) → 그 서버 방 보관본을 버린다 → 그 서버 연결을 닫는다 → 그 서버 방을 ★**입장 순서대로**
   `ROOM_JOIN`(§4-2-1 — 새 연결) → 그 서버 다시 발행(§7-2-3). 발행 방(`pub_room`)이 그 서버였으면 다시 들어간 뒤
   `pub_select`(§5-1-2). ★**발언권 내림·보관본 버림은 들어갈 때 한 번이다** — 재시도는 입장·발행만 다시 한다.
   ★**첫 `ROOM_JOIN` 은 분산해 늦춘다**(§3-2-3) — 재구축은 서버가 상태를 잃은 사건이라 단말 전부에 한꺼번에 온다.
   `4009` 를 받으면 `retry_after_ms` 를 ★**하한**으로 기다린 뒤 다시 보낸다(§11-2-3). ★재구축 중 앱이 그 서버의 마지막 방을
   나가면 멈추고 연결을 닫는다(§4-2-5).
   ★발언권은 처음 값으로 내리고(DEV-FLR-DOWN-FRESH) · 다시 들어갈 방은 내릴 때 남긴 목록이다(DEV-ROOM-REJOIN-LIST).

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.RESTART.restart_ok`](#LINK.C.RESTART.restart_ok) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.OK.dc_closed`](#LINK.C.OK.dc_closed) · [`LINK.C.REOPEN.reopen_retry`](#LINK.C.REOPEN.reopen_retry) · [`LINK.C.any.repair_restart`](#LINK.C.any.repair_restart) |
| 2 | [`LINK.C.RESTART.restart_3xxx`](#LINK.C.RESTART.restart_3xxx) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost`](#LINK.C.any.assoc_lost) · [`LINK.C.REOPEN.reopen_exhaust`](#LINK.C.REOPEN.reopen_exhaust) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) · [`LINK.C.any.repair_rebuild`](#LINK.C.any.repair_rebuild) · [`LINK.C.any.epoch_rebuild`](#LINK.C.any.epoch_rebuild) · [`LINK.C.any.publost_rebuild`](#LINK.C.any.publost_rebuild) · [`LINK.S.new_conn`](#LINK.S.new_conn) · [`PUB.C.any.reregister`](07-pub.md#PUB.C.any.reregister) · [`PUB.C.any.reregister_off`](07-pub.md#PUB.C.any.reregister_off) · [`ROOM.C.any.down`](04-room.md#ROOM.C.any.down) · [`ROOM.C.NONE.rejoin`](04-room.md#ROOM.C.NONE.rejoin) |
| 3 | [`LINK.C.any.ice_disc_leaving`](#LINK.C.any.ice_disc_leaving) · [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.any.ice_disc_absorbed`](#LINK.C.any.ice_disc_absorbed) · [`LINK.C.any.wait_over_leaving`](#LINK.C.any.wait_over_leaving) · [`LINK.C.any.wait_over_absorbed`](#LINK.C.any.wait_over_absorbed) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.any.link_event_absorbed`](#LINK.C.any.link_event_absorbed) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost_leaving`](#LINK.C.any.assoc_lost_leaving) · [`LINK.C.REBUILD.assoc_lost_absorbed`](#LINK.C.REBUILD.assoc_lost_absorbed) · [`LINK.C.any.dc_closed_leaving`](#LINK.C.any.dc_closed_leaving) · [`LINK.C.any.dc_closed_absorbed`](#LINK.C.any.dc_closed_absorbed) · [`LINK.C.REBUILD.epoch_absorbed`](#LINK.C.REBUILD.epoch_absorbed) · [`LINK.C.REBUILD.publost_absorbed`](#LINK.C.REBUILD.publost_absorbed) |
| 4 | [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.CHECK.ws_alive`](#LINK.C.CHECK.ws_alive) |
| 5 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REOPEN.reopen_ok`](#LINK.C.REOPEN.reopen_ok) |
| 7 | [`LINK.C.APPLY.apply_timeout_blocked`](#LINK.C.APPLY.apply_timeout_blocked) · [`LINK.C.REBUILD.rebuilt_unstood`](#LINK.C.REBUILD.rebuilt_unstood) |
| 8 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) |

<a id="RULE-LINK-RECOVERY-ONE-OWNER"></a>**RULE-LINK-RECOVERY-ONE-OWNER** · 규칙

3. 방아쇠는 ICE `disconnected`·`failed` · DC 닫힘 · SCTP 연관 소멸 · `LINK_EVENT`(§4-2-6 — 세대가 맞을 때만) · `ICE_RESTART` 응답 · 적용 뒤 시한 만료 · 깨어남(6)이다.
   ★**서버끼리는 따로 돈다.**

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.RESTART.restart_ok`](#LINK.C.RESTART.restart_ok) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.OK.dc_closed`](#LINK.C.OK.dc_closed) · [`LINK.C.REOPEN.reopen_retry`](#LINK.C.REOPEN.reopen_retry) · [`LINK.C.any.repair_restart`](#LINK.C.any.repair_restart) |
| 2 | [`LINK.C.RESTART.restart_3xxx`](#LINK.C.RESTART.restart_3xxx) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost`](#LINK.C.any.assoc_lost) · [`LINK.C.REOPEN.reopen_exhaust`](#LINK.C.REOPEN.reopen_exhaust) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) · [`LINK.C.any.repair_rebuild`](#LINK.C.any.repair_rebuild) · [`LINK.C.any.epoch_rebuild`](#LINK.C.any.epoch_rebuild) · [`LINK.C.any.publost_rebuild`](#LINK.C.any.publost_rebuild) · [`LINK.S.new_conn`](#LINK.S.new_conn) · [`PUB.C.any.reregister`](07-pub.md#PUB.C.any.reregister) · [`PUB.C.any.reregister_off`](07-pub.md#PUB.C.any.reregister_off) · [`ROOM.C.any.down`](04-room.md#ROOM.C.any.down) · [`ROOM.C.NONE.rejoin`](04-room.md#ROOM.C.NONE.rejoin) |
| 3 | [`LINK.C.any.ice_disc_leaving`](#LINK.C.any.ice_disc_leaving) · [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.any.ice_disc_absorbed`](#LINK.C.any.ice_disc_absorbed) · [`LINK.C.any.wait_over_leaving`](#LINK.C.any.wait_over_leaving) · [`LINK.C.any.wait_over_absorbed`](#LINK.C.any.wait_over_absorbed) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.any.link_event_absorbed`](#LINK.C.any.link_event_absorbed) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost_leaving`](#LINK.C.any.assoc_lost_leaving) · [`LINK.C.REBUILD.assoc_lost_absorbed`](#LINK.C.REBUILD.assoc_lost_absorbed) · [`LINK.C.any.dc_closed_leaving`](#LINK.C.any.dc_closed_leaving) · [`LINK.C.any.dc_closed_absorbed`](#LINK.C.any.dc_closed_absorbed) · [`LINK.C.REBUILD.epoch_absorbed`](#LINK.C.REBUILD.epoch_absorbed) · [`LINK.C.REBUILD.publost_absorbed`](#LINK.C.REBUILD.publost_absorbed) |
| 4 | [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.CHECK.ws_alive`](#LINK.C.CHECK.ws_alive) |
| 5 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REOPEN.reopen_ok`](#LINK.C.REOPEN.reopen_ok) |
| 7 | [`LINK.C.APPLY.apply_timeout_blocked`](#LINK.C.APPLY.apply_timeout_blocked) · [`LINK.C.REBUILD.rebuilt_unstood`](#LINK.C.REBUILD.rebuilt_unstood) |
| 8 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) |

<a id="RULE-LINK-RECOVERY-WS-FIRST"></a>**RULE-LINK-RECOVERY-WS-FIRST** · 규칙

4. ★★**ICE 재시작 앞에서 WS 부터 확인한다.** 연결이 흔들린 것·요청에 답이 없는 것·브라우저의 `online` 사건·망 종류가 바뀐 것은 ★**WS 도 죽었을 수
   있다는 증거**다(망이 바뀌면 옛 TCP 가 반쯤 열린 채 남는다). `HEARTBEAT` 를 곧바로 보내고(윈도우 밖, §2-2-1) 점검 시한(§3-2-4) 안에
   ★**무엇이든 받으면** WS 가 산 것이다 — 그때 `ICE_RESTART` 를 보낸다. 못 받으면 ★**소켓을 닫고**(★`LEAVE` 를 보내지 않는다 —
   `LEAVE` 는 세션을 즉시 폐기한다, §3-1) 사유 없는 끊김으로 §3-2-4 4 를 탄다. 이 절은 멈춘다.
   ★**WS 생존 판단에는 따로 시계를 두지 않는다** — ★**WS 생존 = 마지막으로 무엇이든 받은 뒤 흐른 시간** 하나다. 요청이 나가 있는데
   점검 시한 동안 아무것도 안 왔으면 그것이 무응답이고, ★**곧 WS 점검의 방아쇠**다.
   ★**요청 하나의 시한(`T-req`)은 다른 고장을 잰다** — 소켓이 살아 통지는 오는데 그 요청의 응답만 안 오면(서버가 그 요청을 처리하지 못했다)
   WS 점검은 서지 않는다. 윈도우가 차면 뒤의 요청이 모두 막히므로 `T-req` 가 소켓을 닫는다(§3-2-4).
   ★**요청에 답이 없는 것은 WS 의 증상이지 치료 불가가 아니다** — 이 절이 스스로 재구축으로 올라가지 않는다.

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.RESTART.restart_ok`](#LINK.C.RESTART.restart_ok) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.OK.dc_closed`](#LINK.C.OK.dc_closed) · [`LINK.C.REOPEN.reopen_retry`](#LINK.C.REOPEN.reopen_retry) · [`LINK.C.any.repair_restart`](#LINK.C.any.repair_restart) |
| 2 | [`LINK.C.RESTART.restart_3xxx`](#LINK.C.RESTART.restart_3xxx) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost`](#LINK.C.any.assoc_lost) · [`LINK.C.REOPEN.reopen_exhaust`](#LINK.C.REOPEN.reopen_exhaust) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) · [`LINK.C.any.repair_rebuild`](#LINK.C.any.repair_rebuild) · [`LINK.C.any.epoch_rebuild`](#LINK.C.any.epoch_rebuild) · [`LINK.C.any.publost_rebuild`](#LINK.C.any.publost_rebuild) · [`LINK.S.new_conn`](#LINK.S.new_conn) · [`PUB.C.any.reregister`](07-pub.md#PUB.C.any.reregister) · [`PUB.C.any.reregister_off`](07-pub.md#PUB.C.any.reregister_off) · [`ROOM.C.any.down`](04-room.md#ROOM.C.any.down) · [`ROOM.C.NONE.rejoin`](04-room.md#ROOM.C.NONE.rejoin) |
| 3 | [`LINK.C.any.ice_disc_leaving`](#LINK.C.any.ice_disc_leaving) · [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.any.ice_disc_absorbed`](#LINK.C.any.ice_disc_absorbed) · [`LINK.C.any.wait_over_leaving`](#LINK.C.any.wait_over_leaving) · [`LINK.C.any.wait_over_absorbed`](#LINK.C.any.wait_over_absorbed) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.any.link_event_absorbed`](#LINK.C.any.link_event_absorbed) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost_leaving`](#LINK.C.any.assoc_lost_leaving) · [`LINK.C.REBUILD.assoc_lost_absorbed`](#LINK.C.REBUILD.assoc_lost_absorbed) · [`LINK.C.any.dc_closed_leaving`](#LINK.C.any.dc_closed_leaving) · [`LINK.C.any.dc_closed_absorbed`](#LINK.C.any.dc_closed_absorbed) · [`LINK.C.REBUILD.epoch_absorbed`](#LINK.C.REBUILD.epoch_absorbed) · [`LINK.C.REBUILD.publost_absorbed`](#LINK.C.REBUILD.publost_absorbed) |
| 4 | [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.CHECK.ws_alive`](#LINK.C.CHECK.ws_alive) |
| 5 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REOPEN.reopen_ok`](#LINK.C.REOPEN.reopen_ok) |
| 7 | [`LINK.C.APPLY.apply_timeout_blocked`](#LINK.C.APPLY.apply_timeout_blocked) · [`LINK.C.REBUILD.rebuilt_unstood`](#LINK.C.REBUILD.rebuilt_unstood) |
| 8 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) |

<a id="RULE-LINK-RECOVERY-RESYNC"></a>**RULE-LINK-RECOVERY-RESYNC** · 규칙

5. ★★**되살린 뒤 발언권을 다시 맞춘다(재동기).** 재구축 없이 되살아났으면 — ICE 가 스스로 회복했거나 · ICE 재시작으로
   · `RESUME` 뒤 재시작으로 — 끊긴 동안의 DC 통지가 사라졌을 수 있다(DC 는 재전송하지 않는다, §9-4-13). ★**방마다
   `READY` 를 다시 보낸다** — 서버가 처음 알리기로 `TAKEN` 또는 `IDLE` 을 그 사람에게 보낸다(§10-4-30). 대기
   중이었으면 `QUEUE_POS_REQUEST`(§10-4-30). ★**재전송 타이머를 두지 않는다** — 연결이 또 끊기면 이 절이 다시 돌고 재동기도
   다시 돈다. 건강한 연결에서 답 한 통만 잃으면 다음 발언권 사건이나 다음 요청의 응답이 바로잡는다.

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.RESTART.restart_ok`](#LINK.C.RESTART.restart_ok) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.OK.dc_closed`](#LINK.C.OK.dc_closed) · [`LINK.C.REOPEN.reopen_retry`](#LINK.C.REOPEN.reopen_retry) · [`LINK.C.any.repair_restart`](#LINK.C.any.repair_restart) |
| 2 | [`LINK.C.RESTART.restart_3xxx`](#LINK.C.RESTART.restart_3xxx) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost`](#LINK.C.any.assoc_lost) · [`LINK.C.REOPEN.reopen_exhaust`](#LINK.C.REOPEN.reopen_exhaust) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) · [`LINK.C.any.repair_rebuild`](#LINK.C.any.repair_rebuild) · [`LINK.C.any.epoch_rebuild`](#LINK.C.any.epoch_rebuild) · [`LINK.C.any.publost_rebuild`](#LINK.C.any.publost_rebuild) · [`LINK.S.new_conn`](#LINK.S.new_conn) · [`PUB.C.any.reregister`](07-pub.md#PUB.C.any.reregister) · [`PUB.C.any.reregister_off`](07-pub.md#PUB.C.any.reregister_off) · [`ROOM.C.any.down`](04-room.md#ROOM.C.any.down) · [`ROOM.C.NONE.rejoin`](04-room.md#ROOM.C.NONE.rejoin) |
| 3 | [`LINK.C.any.ice_disc_leaving`](#LINK.C.any.ice_disc_leaving) · [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.any.ice_disc_absorbed`](#LINK.C.any.ice_disc_absorbed) · [`LINK.C.any.wait_over_leaving`](#LINK.C.any.wait_over_leaving) · [`LINK.C.any.wait_over_absorbed`](#LINK.C.any.wait_over_absorbed) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.any.link_event_absorbed`](#LINK.C.any.link_event_absorbed) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost_leaving`](#LINK.C.any.assoc_lost_leaving) · [`LINK.C.REBUILD.assoc_lost_absorbed`](#LINK.C.REBUILD.assoc_lost_absorbed) · [`LINK.C.any.dc_closed_leaving`](#LINK.C.any.dc_closed_leaving) · [`LINK.C.any.dc_closed_absorbed`](#LINK.C.any.dc_closed_absorbed) · [`LINK.C.REBUILD.epoch_absorbed`](#LINK.C.REBUILD.epoch_absorbed) · [`LINK.C.REBUILD.publost_absorbed`](#LINK.C.REBUILD.publost_absorbed) |
| 4 | [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.CHECK.ws_alive`](#LINK.C.CHECK.ws_alive) |
| 5 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REOPEN.reopen_ok`](#LINK.C.REOPEN.reopen_ok) |
| 7 | [`LINK.C.APPLY.apply_timeout_blocked`](#LINK.C.APPLY.apply_timeout_blocked) · [`LINK.C.REBUILD.rebuilt_unstood`](#LINK.C.REBUILD.rebuilt_unstood) |
| 8 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) |

<a id="RULE-LINK-RECOVERY-WAKE"></a>**RULE-LINK-RECOVERY-WAKE** · 규칙

6. ★**깨어남은 사건이다.** 프로세스가 얼었다 깨면 밀린 타이머가 한꺼번에 터진다 — 그 만료를 시한 초과로 세지 않고
   ★**처음부터 다시 판정한다.**

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.RESTART.restart_ok`](#LINK.C.RESTART.restart_ok) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.OK.dc_closed`](#LINK.C.OK.dc_closed) · [`LINK.C.REOPEN.reopen_retry`](#LINK.C.REOPEN.reopen_retry) · [`LINK.C.any.repair_restart`](#LINK.C.any.repair_restart) |
| 2 | [`LINK.C.RESTART.restart_3xxx`](#LINK.C.RESTART.restart_3xxx) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost`](#LINK.C.any.assoc_lost) · [`LINK.C.REOPEN.reopen_exhaust`](#LINK.C.REOPEN.reopen_exhaust) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) · [`LINK.C.any.repair_rebuild`](#LINK.C.any.repair_rebuild) · [`LINK.C.any.epoch_rebuild`](#LINK.C.any.epoch_rebuild) · [`LINK.C.any.publost_rebuild`](#LINK.C.any.publost_rebuild) · [`LINK.S.new_conn`](#LINK.S.new_conn) · [`PUB.C.any.reregister`](07-pub.md#PUB.C.any.reregister) · [`PUB.C.any.reregister_off`](07-pub.md#PUB.C.any.reregister_off) · [`ROOM.C.any.down`](04-room.md#ROOM.C.any.down) · [`ROOM.C.NONE.rejoin`](04-room.md#ROOM.C.NONE.rejoin) |
| 3 | [`LINK.C.any.ice_disc_leaving`](#LINK.C.any.ice_disc_leaving) · [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.any.ice_disc_absorbed`](#LINK.C.any.ice_disc_absorbed) · [`LINK.C.any.wait_over_leaving`](#LINK.C.any.wait_over_leaving) · [`LINK.C.any.wait_over_absorbed`](#LINK.C.any.wait_over_absorbed) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.any.link_event_absorbed`](#LINK.C.any.link_event_absorbed) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost_leaving`](#LINK.C.any.assoc_lost_leaving) · [`LINK.C.REBUILD.assoc_lost_absorbed`](#LINK.C.REBUILD.assoc_lost_absorbed) · [`LINK.C.any.dc_closed_leaving`](#LINK.C.any.dc_closed_leaving) · [`LINK.C.any.dc_closed_absorbed`](#LINK.C.any.dc_closed_absorbed) · [`LINK.C.REBUILD.epoch_absorbed`](#LINK.C.REBUILD.epoch_absorbed) · [`LINK.C.REBUILD.publost_absorbed`](#LINK.C.REBUILD.publost_absorbed) |
| 4 | [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.CHECK.ws_alive`](#LINK.C.CHECK.ws_alive) |
| 5 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REOPEN.reopen_ok`](#LINK.C.REOPEN.reopen_ok) |
| 7 | [`LINK.C.APPLY.apply_timeout_blocked`](#LINK.C.APPLY.apply_timeout_blocked) · [`LINK.C.REBUILD.rebuilt_unstood`](#LINK.C.REBUILD.rebuilt_unstood) |
| 8 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) |

<a id="RULE-LINK-RECOVERY-NEVER-UP"></a>**RULE-LINK-RECOVERY-NEVER-UP** · 규칙

7. ★**한 번도 연결이 서지 못한 채 되살리기가 되풀이되면 드러낸다** — 경로가 막힌 망이다(웹은 UDP 가 막히면 복구 수단이
   없다, §9-4-9). 숨기지 않고 앱에 알린다. ★**포기하지 않는다** — 간격을 벌려 계속한다. 최초 수립(§4-2-2)에서 ICE 가 안
   열리는 것도 같은 칸이다 — 재입장으로 되돌리지 않는다(막힌 망에서 재입장 루프가 된다).

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.RESTART.restart_ok`](#LINK.C.RESTART.restart_ok) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.OK.dc_closed`](#LINK.C.OK.dc_closed) · [`LINK.C.REOPEN.reopen_retry`](#LINK.C.REOPEN.reopen_retry) · [`LINK.C.any.repair_restart`](#LINK.C.any.repair_restart) |
| 2 | [`LINK.C.RESTART.restart_3xxx`](#LINK.C.RESTART.restart_3xxx) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost`](#LINK.C.any.assoc_lost) · [`LINK.C.REOPEN.reopen_exhaust`](#LINK.C.REOPEN.reopen_exhaust) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) · [`LINK.C.any.repair_rebuild`](#LINK.C.any.repair_rebuild) · [`LINK.C.any.epoch_rebuild`](#LINK.C.any.epoch_rebuild) · [`LINK.C.any.publost_rebuild`](#LINK.C.any.publost_rebuild) · [`LINK.S.new_conn`](#LINK.S.new_conn) · [`PUB.C.any.reregister`](07-pub.md#PUB.C.any.reregister) · [`PUB.C.any.reregister_off`](07-pub.md#PUB.C.any.reregister_off) · [`ROOM.C.any.down`](04-room.md#ROOM.C.any.down) · [`ROOM.C.NONE.rejoin`](04-room.md#ROOM.C.NONE.rejoin) |
| 3 | [`LINK.C.any.ice_disc_leaving`](#LINK.C.any.ice_disc_leaving) · [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.any.ice_disc_absorbed`](#LINK.C.any.ice_disc_absorbed) · [`LINK.C.any.wait_over_leaving`](#LINK.C.any.wait_over_leaving) · [`LINK.C.any.wait_over_absorbed`](#LINK.C.any.wait_over_absorbed) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.any.link_event_absorbed`](#LINK.C.any.link_event_absorbed) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost_leaving`](#LINK.C.any.assoc_lost_leaving) · [`LINK.C.REBUILD.assoc_lost_absorbed`](#LINK.C.REBUILD.assoc_lost_absorbed) · [`LINK.C.any.dc_closed_leaving`](#LINK.C.any.dc_closed_leaving) · [`LINK.C.any.dc_closed_absorbed`](#LINK.C.any.dc_closed_absorbed) · [`LINK.C.REBUILD.epoch_absorbed`](#LINK.C.REBUILD.epoch_absorbed) · [`LINK.C.REBUILD.publost_absorbed`](#LINK.C.REBUILD.publost_absorbed) |
| 4 | [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.CHECK.ws_alive`](#LINK.C.CHECK.ws_alive) |
| 5 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REOPEN.reopen_ok`](#LINK.C.REOPEN.reopen_ok) |
| 7 | [`LINK.C.APPLY.apply_timeout_blocked`](#LINK.C.APPLY.apply_timeout_blocked) · [`LINK.C.REBUILD.rebuilt_unstood`](#LINK.C.REBUILD.rebuilt_unstood) |
| 8 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) |

<a id="RULE-LINK-RECOVERY-END"></a>**RULE-LINK-RECOVERY-END** · 규칙



항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.RESTART.restart_ok`](#LINK.C.RESTART.restart_ok) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.OK.dc_closed`](#LINK.C.OK.dc_closed) · [`LINK.C.REOPEN.reopen_retry`](#LINK.C.REOPEN.reopen_retry) · [`LINK.C.any.repair_restart`](#LINK.C.any.repair_restart) |
| 2 | [`LINK.C.RESTART.restart_3xxx`](#LINK.C.RESTART.restart_3xxx) · [`LINK.C.RESTART.restart_5xxx`](#LINK.C.RESTART.restart_5xxx) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost`](#LINK.C.any.assoc_lost) · [`LINK.C.REOPEN.reopen_exhaust`](#LINK.C.REOPEN.reopen_exhaust) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) · [`LINK.C.any.repair_rebuild`](#LINK.C.any.repair_rebuild) · [`LINK.C.any.epoch_rebuild`](#LINK.C.any.epoch_rebuild) · [`LINK.C.any.publost_rebuild`](#LINK.C.any.publost_rebuild) · [`LINK.S.new_conn`](#LINK.S.new_conn) · [`PUB.C.any.reregister`](07-pub.md#PUB.C.any.reregister) · [`PUB.C.any.reregister_off`](07-pub.md#PUB.C.any.reregister_off) · [`ROOM.C.any.down`](04-room.md#ROOM.C.any.down) · [`ROOM.C.NONE.rejoin`](04-room.md#ROOM.C.NONE.rejoin) |
| 3 | [`LINK.C.any.ice_disc_leaving`](#LINK.C.any.ice_disc_leaving) · [`LINK.C.OK.ice_disc`](#LINK.C.OK.ice_disc) · [`LINK.C.any.ice_disc_absorbed`](#LINK.C.any.ice_disc_absorbed) · [`LINK.C.any.wait_over_leaving`](#LINK.C.any.wait_over_leaving) · [`LINK.C.any.wait_over_absorbed`](#LINK.C.any.wait_over_absorbed) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.any.link_event_absorbed`](#LINK.C.any.link_event_absorbed) · [`LINK.C.APPLY.apply_timeout`](#LINK.C.APPLY.apply_timeout) · [`LINK.C.any.assoc_lost_leaving`](#LINK.C.any.assoc_lost_leaving) · [`LINK.C.REBUILD.assoc_lost_absorbed`](#LINK.C.REBUILD.assoc_lost_absorbed) · [`LINK.C.any.dc_closed_leaving`](#LINK.C.any.dc_closed_leaving) · [`LINK.C.any.dc_closed_absorbed`](#LINK.C.any.dc_closed_absorbed) · [`LINK.C.REBUILD.epoch_absorbed`](#LINK.C.REBUILD.epoch_absorbed) · [`LINK.C.REBUILD.publost_absorbed`](#LINK.C.REBUILD.publost_absorbed) |
| 4 | [`LINK.C.WAIT.wait_over`](#LINK.C.WAIT.wait_over) · [`LINK.C.OK.wait_over`](#LINK.C.OK.wait_over) · [`LINK.C.OK.link_event`](#LINK.C.OK.link_event) · [`LINK.C.CHECK.ws_alive`](#LINK.C.CHECK.ws_alive) |
| 5 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REOPEN.reopen_ok`](#LINK.C.REOPEN.reopen_ok) |
| 7 | [`LINK.C.APPLY.apply_timeout_blocked`](#LINK.C.APPLY.apply_timeout_blocked) · [`LINK.C.REBUILD.rebuilt_unstood`](#LINK.C.REBUILD.rebuilt_unstood) |
| 8 | [`LINK.C.WAIT.ice_back`](#LINK.C.WAIT.ice_back) · [`LINK.C.APPLY.applied`](#LINK.C.APPLY.applied) · [`LINK.C.REBUILD.rebuilt`](#LINK.C.REBUILD.rebuilt) |

<a id="RULE-LINK-SDP-RENEGOTIATE"></a>**RULE-LINK-SDP-RENEGOTIATE** · 규칙 — 다시 협상해야 하는 때

| 언제 | 어느 연결 |
|---|---|
| 내가 트랙을 더하거나 뺐다 | 발행 |
| ★`TRACK_EVENT` 를 받았다 | 구독 |
| `ROOM_JOIN` 응답의 `tracks[]` 를 처음 받았다 | 구독 |
| 방을 나가서 받을 것이 줄었다 | 구독 |
| ★`ICE_RESTART` 응답을 받았다(§7-1-2) | ★**그 서버의 연결 전부**(`2pc` 면 둘 — 응답이 두 벌이다) |

★★**SDP 갱신은 연결마다 큐로 하나씩 한다 — 두 모드 공통이다.** 한 연결 위에서 협상 둘을 동시에
못 돌린다. ★`2pc` 는 큐가 **둘**(발행·구독)이라 서로 안 막고, ★`1pc` 은 큐가 **하나**라 위 다섯이
서로 기다린다(그것이 `1pc` 이 치르는 값이다, §9-4-5).

★**시작하는 쪽은 언제나 클라다** — pub 은 클라가 `createOffer` 를 부르고 sub 은 클라가 offer 를
합성한다. 그래서 ★**겹치지 않게 하는 것도 클라 몫**이고, 겹치지만 않으면 `rollback` 은 쓸 일이 없다.
★`rollback` 은 ★**협상이 중간에 깨졌을 때 `stable` 로 되돌리는 용도**다 — 두 모드 다 그때만 쓴다.

★**구독 협상이 성공하면 반드시 `READY` 를 보낸다.** 실패했으면 보내지 않는다.

---

<a id="RULE-LINK-SDP-1PC-PROC"></a>**RULE-LINK-SDP-1PC-PROC** · 규칙 — 절차

1. ★**협상 큐에 넣는다**(§9-2-1). 앞선 협상이 끝나기를 기다린다.
2. 연결이 없으면 만든다 — ★**첫 협상.** 여기서 생기는 것은 ★**첫 협상 확정본**(번호표의 실체) 하나다.
   ① 데이터 채널을 연다(§9-4-13) — ★m-line 순서는 이 호출 순서가 아니라 브라우저가 정한다: audio 0 · video 1 · application 2(RFC 8829 §5.2.1 — data 는 마지막). §9-4-15 이 그 모양이다
   ② ★**audio 1 + video 1 발행 트랜시버를 `inactive` 로 세운다** — 번호표(PT·확장 ID)를 얻는 데 필요한 최소다. ★**자리 확보 트랜시버(둘째 video)는 두지 않는다** — 트랙이 늘면 합성 offer 로 발행 m-line 을 붙인다(§9-4-19 · §9-4-17, M5 실측). 규칙 2(무중단 불변)는 새 절을 더하는 것을 막지 않는다 — 무관한 절이 한 글자도 안 바뀌면 된다
   ③ 브라우저 offer 를 만들어 `setLocalDescription` 까지 한다 — ★**여기까지는 서버 재료가 필요 없다.** 이 offer 의 PT 표·확장 ID 표가 ★**내 번호표**이고 `ROOM_JOIN` 요청에 싣는다(§4-1 — 이 절차가 `ROOM_JOIN` 을 보내기 **전**에 여기까지 온다, §4-2-1)
   ④ 응답의 `server_config` 로 answer 를 조립해 `setRemoteDescription` 한다(§9-4-6) ← ★**이 answer 가 첫 협상 확정본이다**(§9-4-19). ★서버 지원 코덱을 전부 남긴다(§9-4-10 — 좁히지 않는다). 응답의 `sfu_id` 가 이미 붙은 서버면 이 연결을 버린다(§4-1)
   완성 모양은 §9-4-17.
   ★**왜 번호표를 먼저 신고하나** — 한 BUNDLE 안에서 PT 와 확장 ID 는 ★**코덱·URI 마다 하나**여야 한다.
   내 번호를 먼저 알려야 서버가 구독 PT 를 겹치지 않게 배정한다(§9-4-2 ④). 옛 `READY{transport}` 갈래는 확정본 뒤에 신고해 첫 응답의 배정을 되돌리는 재배정이 필요했다 — 없어졌다.
3. ★**통합 offer 를 조립한다**(§9-4-19) — pub 사건이든 sub 사건이든 같은 길이다. 발행 m-line 은
   ★**확정본에서**, 구독 m-line 은 보관본에서 짓는다. 무관한 절은 한 글자도 안 바뀐다(규칙 2).
4. 성공하면 `READY`(§7-1-2). ★**실패했으면 보내지 않는다.**
5. ★**실패했다고 곧바로 모드를 바꾸지 않는다** — 모드 선택은 §9-4-20 다.

★★**`negotiationneeded` 이벤트는 협상 방아쇠가 아니다.** 규칙 1 아래 협상을 시작하는 것은 §9-2-1 의 넷뿐이고, 클라가 큐에 넣어 태운다.
2 의 ①②가 `createDataChannel`·`addTransceiver` 를 잇달아 부르므로 그 자리에서 이벤트가 여러 번 뜬다 — ★**이 이벤트를 물리면 브라우저
offer 가 나가 규칙 1·2 를 그 자리에서 어긴다**(§9-4-19 의 코덱 되살아남이 바로 그 사고다). ★`2pc` 도 같다 — 발행 연결은 클라 offer 가
정상 경로지만 큐를 거치지 않으면 §9-2-1 직렬화가 깨진다.

★**증상**
- ★**직렬화를 빠뜨리면** — 내가 카메라를 켜는 순간 남이 들어오면 두 협상이 겹쳐
  ★**`have-local-offer` 에 offer 가 도착**한다. 한쪽이 통째로 버려지고, 버려진 쪽이 내 트랙이면
  **서버는 발행됐다고 아는데 RTP 가 안 나간다.**
- ★**발행 절을 협상 전 offer 목록에서 가져오면** — 걸러냈던 코덱이 되살아나 ★**보고한 것과 다른
  코덱으로 보낸다.** 서버가 `[PT:MISMATCH]` 로 잡거나, 못 잡으면 **검은 화면**이다.
- ★★**첫 협상에서 `inactive` 트랜시버를 안 세우면** — 내 PT·확장 ID 가 없는 채로 서버가 구독 번호를
  배정한다. 뒤늦게 마이크를 켤 때 ★**한 BUNDLE 안에서 번호가 부딪힌다.**
- ★**구독 m-line 에 `subscribe_*` 자격을 쓰면** — 한 BUNDLE 에 자격이 둘이 되어 **전송로가 안 선다.**
- 비게 된 m-line 을 없애면 → **BUNDLE 태그가 옮겨가 전송이 깨진다**(§9-4-11 와 같다).
- ★`negotiationneeded` 를 협상 큐에 물리면 → 첫 협상 자리에서 브라우저 offer 가 여러 번 나가고, 걸러냈던 코덱이 되살아나 보고한 것과 다른 코덱으로 보낸다.

★**규칙 둘과 값 셋이 `1pc` 의 전부다**(§9-4-18). 얻는 것은 ★**전송로 한 벌** — 상시 keepalive 가 반으로
준다(§9-4-5). `2pc` 는 브라우저가 `1pc` 를 못 받을 때의 기준선이다.


---

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.any.renego_queued`](#LINK.C.any.renego_queued) · [`LINK.C.any.nego_ok_next`](#LINK.C.any.nego_ok_next) |

<a id="PROP-LINK-PER-SERVER"></a>**PROP-LINK-PER-SERVER** · 성질

서버 하나의 일(그 연결의 고장 · 재구축 · 그 서버 방의 응답)은 다른 서버의 방 · 연결을 건드리지 않는다 — 효과는 주소(at.sameServer)로 그 서버 개체만 고르고, 방이 어느 서버인지는 입장 응답의 sfu_id 로 클라가 배운 칸이다(RULE-LINK-PER-SERVER-UNITS · RULE-LINK-RECOVERY-UNIT).

*식: MS1(다판 two_servers) · 깨지면: 한 서버의 연관 소멸이 멀쩡한 다른 서버의 방까지 내려 그 방 미디어 · 발언권이 끊긴다*

<a id="PROP-LINK-SERVER-ROOMS-DOWN"></a>**PROP-LINK-SERVER-ROOMS-DOWN** · 성질

그 서버를 재구축하면 그 서버의 방은 내린다(다시 들어간다) — 같은 서버의 방을 빠뜨리지 않는다(RULE-LINK-PER-SERVER-UNITS · RULE-LINK-RECOVERY-REBUILD).

*식: MS1b(다판 two_servers) · 깨지면: 새 연결로 옮겼는데 그 방은 옛 연결의 것으로 남아 받는 것이 없다*

<a id="PROP-LINK-BLOCKED-SHOWN"></a>**PROP-LINK-BLOCKED-SHOWN** · 성질

한 번도 서지 못한 연결의 되살리기 시한이 끝나면 앱에 드러낸다 — 막힌 망이다 · 숨기지 않는다(RULE-LINK-RECOVERY-NEVER-UP).

*식: NU1 · 깨지면: 웹에서 UDP 가 막힌 망이 조용히 되풀이만 돌아 앱은 연결 중으로만 안다*

<a id="PROP-LINK-BLOCKED-NO-REJOIN"></a>**PROP-LINK-BLOCKED-NO-REJOIN** · 성질

막힌 망에서 재입장 루프로 가지 않는다 — 적용 시한 만료와 서지 못한 재구축은 같은 복구 칸(WS 점검 → ICE 재시작)으로 · 포기하지 않는다(RULE-LINK-RECOVERY-NEVER-UP · 최초 수립에서 ICE 가 안 열리는 것도 같은 칸).

*식: NU2 · 깨지면: 막힌 망에서 재입장이 되풀이돼 방 명단이 들락거리고 서버가 헛 정리를 돈다*

<a id="PROP-LINK-RECOVERY-ENDS"></a>**PROP-LINK-RECOVERY-ENDS** · 성질

복구는 끝난다 — 망이 결국 정상이면(장애 · 실패 결과가 결국 멎으면) 그 서버 연결은 결국 선다 · 되살림이든 재구축이든(RULE-LINK-RECOVERY-UNIT · ★활성 성질 — 통 · 시한 · 세계의 성공 결과에 공정성).

*식: LL1(활성 판 link live) · 깨지면: 망이 돌아왔는데 서버가 버린 자격을 붙들고 되살리기만 되풀이한다 · 재구축이 끝나지 않는다*

<a id="PROP-LINK-NEGO-ENDS"></a>**PROP-LINK-NEGO-ENDS** · 성질

재협상은 끝난다 — 협상 큐가 결국 빈다(RULE-LINK-SDP-RENEGOTIATE · ★활성 성질).

*식: LL2(활성 판 link live) · 깨지면: 협상 중 표시가 남아 다음 방아쇠가 영영 큐에만 적힌다*

<a id="PROP-LINK-NEGO-NO-LOST-TRIGGER"></a>**PROP-LINK-NEGO-NO-LOST-TRIGGER** · 성질

협상이 쉬면 마지막 방아쇠까지 SDP 에 실렸다 — 마지막 협상이 실패했으면 적어도 그것을 싣고 시도했다(RULE-LINK-SDP-RENEGOTIATE 의 큐 · RULE-LINK-SDP-1PC-PROC 1 · 연달아 온 것은 한 번에 실어도 된다).

*식: NQ1 · 깨지면: 협상 중에 온 `TRACK_EVENT` · 내 트랙 변경이 흡수돼 사라진다 — 남의 트랙이 영영 안 붙거나 켠 카메라가 안 실린다*

<a id="PROP-LINK-NEGO-ONE-AT-A-TIME"></a>**PROP-LINK-NEGO-ONE-AT-A-TIME** · 성질

한 연결 위에서 협상 둘이 겹치지 않는다 — 진행 중이면 새 협상을 시작하지 않고 큐에 적는다(RULE-LINK-SDP-RENEGOTIATE · `1pc` 는 큐가 하나다). ICE 재시작의 협상도 같은 큐를 탄다.

*식: NQ2 · 깨지면: `have-local-offer` 에 offer 가 닿아 한쪽이 통째로 버려진다 — 버려진 쪽이 내 트랙이면 서버는 발행됐다고 아는데 RTP 가 안 나간다(RULE-LINK-SDP-1PC-PROC 증상)*

<a id="PROP-LINK-RESTART-AFTER-NEGO"></a>**PROP-LINK-RESTART-AFTER-NEGO** · 성질

새 ICE 자격은 그것을 실은 협상이 끝난 뒤에만 선다(RULE-LINK-SDP-RENEGOTIATE — ICE_RESTART 응답도 큐에 드는 방아쇠).

*식: LH8 · 깨지면: 앞선 협상이 진행 중일 때 받은 새 자격이 어느 협상에도 실리지 않은 채 적용된 것으로 보고된다 — ICE 가 옛 자격으로 남아 되살림이 시한 만료로 되풀이된다*

<a id="PROP-LINK-NO-NEEDLESS-RECOVERY"></a>**PROP-LINK-NO-NEEDLESS-RECOVERY** · 성질

복구 단계가 OK 가 아니면 그 연결에 복구가 필요하다(세계의 사실) — 또는 서버가 지금 쓰는 세대로 보낸 LINK_EVENT 로 시작했다(RACE-LINK-STALE-SUSPECT).

*식: LH3 · 깨지면: 낡은 통지가 끝난 복구를 한 번 더 시작한다(§4-2-6 증상)*

<a id="PROP-LINK-RESYNCED"></a>**PROP-LINK-RESYNCED** · 성질

통로가 빈 OK 에서는 끊긴 뒤 발언권을 다시 맞췄다(READY).

*식: LH4 · 깨지면: 끊긴 동안 사라진 DC 통지 때문에 화자 표시가 낡은 채 남는다(§9-2-1 5)*

<a id="PROP-LINK-RESTART-ON-ICE"></a>**PROP-LINK-RESTART-ON-ICE** · 성질

ICE_RESTART 를 보내는 순간 ICE 경로가 실제로 나쁘다 — 또는 서버가 지금 쓰는 세대로 보낸 LINK_EVENT 로 시작했다. DC 채널만 닫힌 것으로는 나가지 않는다.

*식: LH5 · 깨지면: ICE 가 멀쩡한데 재시작이 나간다(§9-2-1 1)*

<a id="PROP-LINK-REBUILD-CONFIRMED"></a>**PROP-LINK-REBUILD-CONFIRMED** · 성질

재구축은 치료 불가 확정 사건이 있을 때만이다(시한 · 횟수 · 5xxx 는 확정이 아니다).

*식: LH6 · 깨지면: 흔들릴 때마다 재구축해 발언권 · 구독 · 발행이 매번 리셋되고 멀쩡한 방까지 끊긴다(§9-2-1 증상)*

<a id="LINK.C.OK"></a>
#### 9-2-2. 클라 · `OK`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="LINK.C.OK.ice_disc"></a>`LINK.C.OK.ice_disc` | ls == 'OK' | ls ← 'WAIT'; sync ← false; ice ← 'DISC' · 효과 app.link_shaky | §9-2-1 1 §9-2-1 3 — disconnected → 기다린다(스스로 풀릴 수 있다) |
| <a id="LINK.C.OK.wait_over"></a>`LINK.C.OK.wait_over` | ls == 'OK' | ls ← 'CHECK'; sync ← false; ice ← failed ? 'FAILED' : ice · 보낸다 HEARTBEAT.req | §9-2-1 1 §9-2-1 4 — disconnected 를 거치지 않고 곧바로 failed — WAIT 와 같다(칸 전수가 드러냄) |
| <a id="LINK.C.OK.link_event"></a>`LINK.C.OK.link_event` | ls == 'OK' | ls ← 'CHECK'; sync ← false · 보낸다 HEARTBEAT.req | §9-2-1 1 §9-2-1 3 §9-2-1 4 — 다른 쪽 끝의 관찰 — 방아쇠로 넣는다 · WS 부터 확인한다 |
| <a id="LINK.C.OK.dc_closed"></a>`LINK.C.OK.dc_closed` | ls == 'OK' | ls ← 'REOPEN'; dc ← false; sync ← false · 효과 dc.reopen · 효과 floor.dc_down → FLR.C.any.dc_down | §9-2-1 1 §10-4-23 — DC 채널만 닫혔으면 다시 열기만 한다 — ICE 재시작을 내지 않는다 |

<a id="LINK.C.WAIT"></a>
#### 9-2-3. 클라 · `WAIT`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="LINK.C.WAIT.ice_back"></a>`LINK.C.WAIT.ice_back` | 받는 상태 ls == 'WAIT' && ice == 'DISC' · 나머지 | ls ← 'OK'; sync ← true; ice ← 'OK' · 보낸다 READY.req · 효과 app.link_ok | §9-2-1 5 §9-2-1 8 — 재구축 없이 되살아났다 — 방마다 READY 로 발언권을 다시 맞춘다 |
| <a id="LINK.C.WAIT.wait_over"></a>`LINK.C.WAIT.wait_over` | ls == 'WAIT' | ls ← 'CHECK'; sync ← false; ice ← failed ? 'FAILED' : ice · 보낸다 HEARTBEAT.req | §9-2-1 1 §9-2-1 4 — ICE 재시작 앞에서 WS 부터 확인한다 — HEARTBEAT 를 곧바로(윈도우 밖) |

<a id="LINK.C.CHECK"></a>
#### 9-2-4. 클라 · `CHECK`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="LINK.C.CHECK.ws_alive"></a>`LINK.C.CHECK.ws_alive` | ls == 'CHECK' | ls ← 'RESTART'; rid ← rid + 1; req ← rid + 1 · 보낸다 ICE_RESTART.req · 효과 app.link_reviving | §9-2-1 4 |

<a id="LINK.C.RESTART"></a>
#### 9-2-5. 클라 · `RESTART`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="LINK.C.RESTART.restart_ok_queued"></a>`LINK.C.RESTART.restart_ok_queued` | m.part == 'res' && nego | ls ← 'APPLY'; pgen ← m.publish.ufrag; req ← 0; dirty ← true | §7-1-2 §9-2-1 — 앞선 협상이 진행 중이다 — 새 자격은 큐에 넣어 다음 조립에 싣는다(한 연결 위에 협상 둘을 동시에 못 돌린다 · rollback 으로 앞지르지 않는다 — RULE-LINK-SDP-RENEGOTIATE) |
| <a id="LINK.C.RESTART.restart_ok"></a>`LINK.C.RESTART.restart_ok` | m.part == 'res' | ls ← 'APPLY'; pgen ← m.publish.ufrag; req ← 0; nego ← true · 효과 sdp.restart | §7-1-2 §9-2-1 1 §9-2-1 — 새 자격으로 다시 협상한다(§9-4-9 restart 조립) — 협상 큐를 탄다(RULE-LINK-SDP-RENEGOTIATE) |
| <a id="LINK.C.RESTART.restart_3xxx"></a>`LINK.C.RESTART.restart_3xxx` | m.code >= 3000 && m.code < 4000 | ls ← 'REBUILD'; cause ← 'RESTART_3XXX'; req ← 0 · 효과 floor.down_server → FLR.C.any.down · 효과 store.drop_server → ROOM.C.LEAVING.down · ROOM.C.any.down · 효과 conn.close · 효과 rooms.rejoin_server → ROOM.C.NONE.rejoin · 효과 pub.reregister_server → PUB.C.any.reregister · PUB.C.any.reregister_off · 효과 app.link_rebuilding | §9-2-1 2 — 치료 불가 확정 ① — 그 서버만 재구축 |
| <a id="LINK.C.RESTART.restart_5xxx"></a>`LINK.C.RESTART.restart_5xxx` | 나머지 | ls ← 'CHECK'; req ← 0 · 보낸다 HEARTBEAT.req | §9-2-1 1 §9-2-1 2 — 5xxx — 같은 치료를 간격을 벌려 다시(§3-2-3 사다리 · 끝 칸 되풀이) |

<a id="LINK.C.APPLY"></a>
#### 9-2-6. 클라 · `APPLY`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="LINK.C.APPLY.apply_timeout_blocked"></a>`LINK.C.APPLY.apply_timeout_blocked` | 받는 상태 ls == 'APPLY' · !everUp | ls ← 'CHECK'; pgen ← 0 · 보낸다 HEARTBEAT.req · 효과 app.link_blocked | §9-2-1 7 — 한 번도 연결이 서지 못한 채 되살리기가 되풀이된다 — 경로가 막힌 망이다 · 숨기지 않고 앱에 알린다 · 포기하지 않는다(간격을 벌려 계속 · 재입장하지 않는다) |
| <a id="LINK.C.APPLY.apply_timeout"></a>`LINK.C.APPLY.apply_timeout` | 받는 상태 ls == 'APPLY' · 나머지 | ls ← 'CHECK'; pgen ← 0 · 보낸다 HEARTBEAT.req | §9-2-1 3 §9-2-1 2 — 시한 · 횟수는 치료 불가 확정이 아니다 — 같은 칸에서 간격을 벌려 다시(재구축하지 않는다) |
| <a id="LINK.C.APPLY.applied"></a>`LINK.C.APPLY.applied` | 받는 상태 ls == 'APPLY' && !nego · 나머지 | ls ← 'OK'; ice ← 'OK'; gen ← pgen; pgen ← 0; sync ← true; everUp ← true · 보낸다 READY.req · 효과 app.link_ok | §9-2-1 5 §9-2-1 8 — 재구축 없이 되살아났다 — READY 로 발언권을 다시 맞춘다 |

<a id="LINK.C.REBUILD"></a>
#### 9-2-7. 클라 · `REBUILD`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="LINK.C.REBUILD.assoc_lost_absorbed"></a>`LINK.C.REBUILD.assoc_lost_absorbed` | ls == 'REBUILD' | assoc ← false; dc ← false | §9-2-1 3 |
| <a id="LINK.C.REBUILD.rebuilt_unstood"></a>`LINK.C.REBUILD.rebuilt_unstood` | 받는 상태 ls == 'REBUILD' · !stood | ls ← 'CHECK'; ice ← 'FAILED'; everUp ← false; assoc ← true; gen ← g; pgen ← 0; sync ← false; cause ← 'NONE'; nego ← false; dirty ← false · 보낸다 HEARTBEAT.req | §9-2-1 7 — 새 연결의 ICE 가 안 열린다(막힌 망 — 웹은 UDP 가 막히면 복구 수단이 없다) — 재입장으로 되돌리지 않고 같은 복구 칸(WS 점검 → ICE 재시작)으로 · 이 연결은 한 번도 서지 않았다 |
| <a id="LINK.C.REBUILD.rebuilt"></a>`LINK.C.REBUILD.rebuilt` | 받는 상태 ls == 'REBUILD' · 나머지 | ls ← 'OK'; ice ← 'OK'; assoc ← true; dc ← true; gen ← g; pgen ← 0; sync ← true; cause ← 'NONE'; nego ← false; dirty ← false; everUp ← true · 효과 app.link_ok | §9-2-1 2 §9-2-1 8 — 발언권 내림 · 보관본 버림은 들어갈 때 한 번이었다 — 새 입장이 발언권을 처음부터 받는다 · 옛 연결의 협상 큐는 연결과 함께 사라졌다(새 연결은 지금 보관본으로 짓는다) |
| <a id="LINK.C.REBUILD.epoch_absorbed"></a>`LINK.C.REBUILD.epoch_absorbed` | 받는 효과 ROOM link.epoch_changed · ls == 'REBUILD' | — | §9-2-1 3 — 이미 재구축 중 — 방아쇠는 넣기만 한다(레벨로 받는다) |
| <a id="LINK.C.REBUILD.publost_absorbed"></a>`LINK.C.REBUILD.publost_absorbed` | 받는 효과 PUB link.pub_lost · ls == 'REBUILD' | — | §9-2-1 3 — 이미 재구축 중 — 방아쇠는 넣기만 한다(레벨로 받는다) |

<a id="LINK.C.REOPEN"></a>
#### 9-2-8. 클라 · `REOPEN`

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="LINK.C.REOPEN.reopen_ok"></a>`LINK.C.REOPEN.reopen_ok` | 받는 상태 ls == 'REOPEN' · r == 'ok' | ls ← 'OK'; dc ← true; sync ← true · 보낸다 READY.req · 효과 app.link_ok · 효과 floor.dc_up → FLR.C.any.dc_reopen | §9-2-1 5 §10-4-23 — DC 가 다시 섰다 — 발언권을 다시 맞춘다 |
| <a id="LINK.C.REOPEN.reopen_retry"></a>`LINK.C.REOPEN.reopen_retry` | 받는 상태 ls == 'REOPEN' · r == 'fail' | 효과 dc.reopen | §9-2-1 1 |
| <a id="LINK.C.REOPEN.reopen_exhaust"></a>`LINK.C.REOPEN.reopen_exhaust` | 받는 상태 ls == 'REOPEN' · 나머지 | ls ← 'REBUILD'; cause ← 'DC_EXHAUST' · 효과 floor.down_server → FLR.C.any.down · 효과 store.drop_server → ROOM.C.LEAVING.down · ROOM.C.any.down · 효과 conn.close · 효과 rooms.rejoin_server → ROOM.C.NONE.rejoin · 효과 pub.reregister_server → PUB.C.any.reregister · PUB.C.any.reregister_off · 효과 app.link_rebuilding | §9-2-1 2 — 치료 불가 확정 ③ — 연관이 산 채로 채널만 안 서는 것은 서버 쪽 채널 고장 |

<a id="LINK.S"></a>
#### 9-2-9. 서버

| 라벨 | 언제 | 하는 일 | 원문 · 근거 |
|---|---|---|---|
| <a id="LINK.S.restart_3010"></a>`LINK.S.restart_3010` | m.t == 'ICE_RESTART' && !known | 답한다 ICE_RESTART.fail {code: 3010} | §7-1-2 |
| <a id="LINK.S.restart_5xxx"></a>`LINK.S.restart_5xxx` | m.t == 'ICE_RESTART' && busy | 답한다 ICE_RESTART.fail {code: 5003} | §7-1-2 §11-2-3 |
| <a id="LINK.S.restart"></a>`LINK.S.restart` | m.t == 'ICE_RESTART' | pend ← next; next ← next + 1 · 답한다 ICE_RESTART.res {publish: {ufrag: next, pwd: 0}} | §7-1-2 |
| <a id="LINK.S.heartbeat"></a>`LINK.S.heartbeat` | m.t == 'HEARTBEAT' | 답한다 HEARTBEAT.res | §3-1 |
| <a id="LINK.S.rx_other"></a>`LINK.S.rx_other` | 나머지 | — | §7-1-2 |
| <a id="LINK.S.suspect"></a>`LINK.S.suspect` | 나머지 | 답한다 LINK_EVENT.body {type: 'suspect', ufrag: use} | §4-2-6 §1-2-6 |
| <a id="LINK.S.server_restart"></a>`LINK.S.server_restart` | 나머지 | known ← false | §7-1-2 |
| <a id="LINK.S.new_conn"></a>`LINK.S.new_conn` | 나머지 | known ← true; use ← next; pend ← 0; next ← next + 1 | §9-2-1 2 |
| <a id="LINK.S.checked_new"></a>`LINK.S.checked_new` | g == pend | use ← pend; pend ← 0 | §7-1-2 |
| <a id="LINK.S.checked_current"></a>`LINK.S.checked_current` | 나머지 | — | §7-1-2 |
| <a id="LINK.S.busy_flip"></a>`LINK.S.busy_flip` | 나머지 | busy ← !busy | §11-2-3 |
| <a id="LINK.S.track_on_full"></a>`LINK.S.track_on_full` | 받는 효과 TRK link.track_on · free == 0 | want[t] ← true | §8-1-2 |
| <a id="LINK.S.track_on"></a>`LINK.S.track_on` | 받는 효과 TRK link.track_on · 나머지 | want[t] ← true; mid[t] ← free; rows ← free > rows ? free : rows; arev ← arev + 1; since[t] ← arev + 1 · 답한다 MLINE_EVENT.body {table: {arev: arev + 1, mid: {A: t == 'A' ? free : mid['A'], B: t == 'B' ? free : mid['B'], Sa: t == 'Sa' ? free : mid['Sa'], Sb: t == 'Sb' ? free : mid['Sb']}, rows: free > rows ? free : rows}} | §8-1-2 §8-1-2 |
| <a id="LINK.S.track_off"></a>`LINK.S.track_off` | 받는 효과 TRK link.track_off · mid[t] != 0 | want[t] ← false; mid[t] ← 0; arev ← arev + 1 · 답한다 MLINE_EVENT.body {table: {arev: arev + 1, mid: {A: t == 'A' ? 0 : mid['A'], B: t == 'B' ? 0 : mid['B'], Sa: t == 'Sa' ? 0 : mid['Sa'], Sb: t == 'Sb' ? 0 : mid['Sb']}, rows: rows}} | §8-1-2 §8-3-1 |
| <a id="LINK.S.track_off_norow"></a>`LINK.S.track_off_norow` | 받는 효과 TRK link.track_off · 나머지 | want[t] ← false | §8-1-2 |
| <a id="LINK.S.reissue"></a>`LINK.S.reissue` | 받는 상태 want[t] && mid[t] == 0 · 나머지 | mid[t] ← free; rows ← free > rows ? free : rows; arev ← arev + 1; since[t] ← arev + 1 · 답한다 MLINE_EVENT.body {table: {arev: arev + 1, mid: {A: t == 'A' ? free : mid['A'], B: t == 'B' ? free : mid['B'], Sa: t == 'Sa' ? free : mid['Sa'], Sb: t == 'Sb' ? free : mid['Sb']}, rows: free > rows ? free : rows}} | §8-3-1 §8-1-2 |
| <a id="LINK.S.reassign"></a>`LINK.S.reassign` | 받는 상태 want[t] && mid[t] == 0 && u != t && u != 'Sa' && u != 'Sb' && mid[u] != 0 · 나머지 | mid[t] ← mid[u]; mid[u] ← 0; arev ← arev + 1; since[t] ← arev + 1 · 답한다 MLINE_EVENT.body {table: {arev: arev + 1, mid: {A: t == 'A' ? mid[u] : u == 'A' ? 0 : mid['A'], B: t == 'B' ? mid[u] : u == 'B' ? 0 : mid['B'], Sa: mid['Sa'], Sb: mid['Sb']}, rows: rows}} | §8-1-2 §8-1-2 |
| <a id="LINK.S.ack"></a>`LINK.S.ack` | 받는 효과 SUB link.ready · a > ack | ack ← a | §8-1-3 |
| <a id="LINK.S.ack_old"></a>`LINK.S.ack_old` | 받는 효과 SUB link.ready · 나머지 | — | §8-1-3 |
| <a id="LINK.S.peer_reaped"></a>`LINK.S.peer_reaped` | 받는 효과 ROOM link.room_gone · last | known ← false | §9-2-9 |
| <a id="LINK.S.room_gone_other"></a>`LINK.S.room_gone_other` | 받는 효과 ROOM link.room_gone · 나머지 | — | §9-2-9 |

<a id="RULE-LINK-SERVER-REAP"></a>**RULE-LINK-SERVER-REAP** · 규칙

★★**그 사람이 그 서버의 마지막 방을 잃으면 서버가 Peer 를 거둔다** — 떠남(`ROOM_LEAVE`) · 강퇴 · 방 닫힘 셋이 같은 길이다(원칙 ③). 반이중 발행은 *"그 서버의 마지막 방까지"* 살므로 이때 함께 사라지고, 그 연결의 배정표(§8-1-2)도 연결과 함께 사라진다. 그 뒤 그 연결의 `ICE_RESTART` 는 `3010` 다. 클라는 같은 순간 그 연결을 닫는다(§4-2-5 2) — 강퇴 · 방 닫힘은 `ROOM_EVENT{affiliation}` 로 알고 같은 길을 탄다.
- 없으면: 마지막 방에서 쫓겨나거나 방이 닫힌 사람의 Peer 가 서버에 남아 ★반이중 발행이 주인 없이 슬롯에 남고, 다시 들어오면 재입장 축출이 돌기 전까지 옛 연결 자원이 남는다

항목 → 경우(경우가 든 원문 항목 좌표에서 짓는다 · 경우의 메모는 동작 표에)

| 항목 | 경우 |
|---|---|
| 1 | [`LINK.C.any.room_left`](#LINK.C.any.room_left) · [`ROOM.C.LEAVING.leave_ok`](04-room.md#ROOM.C.LEAVING.leave_ok) |
| 2 | [`LINK.C.any.ice_disc_leaving`](#LINK.C.any.ice_disc_leaving) · [`LINK.C.any.wait_over_leaving`](#LINK.C.any.wait_over_leaving) · [`LINK.C.any.assoc_lost_leaving`](#LINK.C.any.assoc_lost_leaving) · [`LINK.C.any.dc_closed_leaving`](#LINK.C.any.dc_closed_leaving) · [`LINK.C.any.last_room_left`](#LINK.C.any.last_room_left) · [`PUB.C.any.drop_server`](07-pub.md#PUB.C.any.drop_server) · [`ROOM.C.LEAVING.leave_ok`](04-room.md#ROOM.C.LEAVING.leave_ok) |
| 3 | [`ROOM.C.LEAVING.leave_ok`](04-room.md#ROOM.C.LEAVING.leave_ok) |

<a id="PROP-LINK-SERVER-REAP"></a>**PROP-LINK-SERVER-REAP** · 성질

그 서버에 그 사람의 방이 있으면 Peer 가 있고 없으면 없다 — 떠남 · 강퇴 · 방 닫힘 셋이 같은 길이다 · 방을 다 잃으면 발행이 하나도 안 남는다(반이중까지) · 전이중 발행은 그 방에 있는 동안만이다(RULE-LINK-SERVER-REAP).

*식: RP1 · RP2 · RP3(다판 reap) · 깨지면: 마지막 방에서 쫓겨난 사람의 Peer 와 반이중 발행이 서버에 남는다 — 슬롯에 주인 없는 발행이 남고 다시 들어오면 옛 자원이 겹친다*

<a id="RULE-LINK-SERVER-POLICY"></a>**RULE-LINK-SERVER-POLICY** · 규칙

| 무엇 | 어디서 오나 |
|---|---|
| ★서버가 연결을 의심하는 시간 | 서버가 잰다(쌍마다) — 결과가 `LINK_EVENT{type:"suspect"}` 로 온다(§4-2-6). ★**방도 연결도 거두지 않는다** — 미디어 침묵으로는 아무것도 회수하지 않는다(성립 조건 색인 — Peer 수명) |

### 9-3. 흐름 · 검증
검증 모형 `out/link/Link.tla` · 하네스 `verify/link/harness.tla`

#### 9-3-1. 흐름 F1 — 흔들렸다 — ICE 재시작으로 되살린다
가장 아래에서 고장 난 층(ICE)만 고친다 — DTLS · 구독 · 발행 · 발언권은 그대로다.
모형 검사: 걸음 8 도달 · 갈래 10

1. **ice_disc** — ICE 가 disconnected 가 된다. 스스로 풀릴 수 있으므로 기다린다.
2. **wait_over** — 기다림이 끝나도 안 풀렸다. ICE 재시작 앞에서 WS 부터 확인한다 — HEARTBEAT 를 곧바로 보낸다(윈도우 밖).
3. **up HEARTBEAT** — 서버가 하트비트에 답한다.
4. **down HEARTBEAT** — 무엇이든 받았으니 WS 가 산다 — ICE_RESTART 를 보낸다(그 요청의 pid 를 든다). ★못 받았으면 소켓을 LEAVE 없이 닫고 세션 축으로 넘긴다.
5. **up ICE_RESTART** — 서버가 새 ufrag · pwd 를 대기분으로 낸다. 옛 자격도 함께 받는다.
6. **down ICE_RESTART** — 그 요청의 응답이다 — 새 자격으로 다시 협상한다. 협상 큐를 탄다 — 앞선 협상이 진행 중이면 그것이 끝난 뒤 다음 조립에 싣는다.
7. **nego_done** — 새 자격을 실은 협상(restart 조립)이 끝났다. ★협상이 끝나기 전에는 새 자격으로 ICE 가 서지 않는다 — 협상 둘을 한 연결 위에 겹치지 않는다.
8. **applied** — 새 자격의 첫 검사가 서버에 닿아 대기분이 쓰는 것이 된다. 클라는 방마다 READY 로 발언권을 다시 맞춘다.

#### 9-3-2. 흐름 F2 — DC 채널만 닫혔다 — 다시 연다
연관이 살아 있으면 같은 연결 위에 채널만 다시 연다 — ICE 재시작을 내지 않는다.
모형 검사: 걸음 2 도달 · 갈래 4

1. **dc_closed** — DC 채널이 닫혔다. 연관은 산다 — 다시 열기만 한다.
2. **reopen** — 채널이 다시 섰다 — READY 로 발언권을 다시 맞춘다.

#### 9-3-3. 흐름 F3 — 서버가 그 연결을 모른다 — 그 서버만 재구축
치료 불가가 확정되면(ICE_RESTART 3xxx) 그 서버만 다시 짓는다 — 다른 서버의 방 · 발행은 건드리지 않는다.
모형 검사: 걸음 8 도달 · 갈래 8

1. **srv_restart** — 미디어 서버가 재기동한다 — 그 연결을 모른다.
2. **ice_disc** — ICE 가 disconnected 가 된다.
3. **wait_over** — 안 풀린다 — WS 부터 확인한다.
4. **up HEARTBEAT** — 게이트웨이 WS 는 산다.
5. **down HEARTBEAT** — ICE_RESTART 를 보낸다.
6. **up ICE_RESTART** — 서버가 그 연결을 모른다 — 3010.
7. **down ICE_RESTART** — 치료 불가 확정 — 그 서버 방들의 발언권을 내리고 보관본을 버리고 연결을 닫고 입장 순서대로 다시 들어간다. ★첫 ROOM_JOIN 은 분산해 늦춘다 — 서버 하나가 다시 뜬 순간 단말 전부가 같은 틱에 몰린다.
8. **rebuilt** — 새 연결이 섰다 — 새 입장이 발언권을 처음부터 받는다.

### 9-4. 주제

#### 9-4-1. 상태를 앱에 알린다

<a id="RULE-LINK-APP-EXPOSE"></a>**RULE-LINK-APP-EXPOSE** · 규칙

| 무엇 | 값 |
|---|---|
| ★**서버마다 미디어 연결 상태** | ★**그 서버 연결이 흔들리나·되살리는 중인가·다시 짓는 중인가**(§9-2-1). 클라가 ICE 상태로 스스로 알고, 서버도 검사가 끊기면 `LINK_EVENT`(§4-2-6)로 알린다 — ★**소켓이 살아 있는 채로** 온다. 방은 그 방을 맡은 서버의 값을 따른다(§9-1-1 `sfu_id`) |

#### 9-4-2. ★서버가 egress 에서 다시 짓는다 — 구독자 표

<a id="RULE-LINK-EGRESS-REWRITE"></a>**RULE-LINK-EGRESS-REWRITE** · 규칙 — ★서버가 egress 에서 다시 짓는다 — 구독자 표

★★**내가 보낸 PT·확장 ID 가 그대로 남에게 가지 않는다.** 서버는 구독자마다 표를 하나 갖고
egress 에서 PT 를 그 표로 배정하고 헤더 확장을 ★**다시 짓는다.** ★**클라가 알아야 하는 것은 넷**이고, §9-4-4 의 여러 자리가 이것을 딛는다.

| | 사실 | 딛는 자리 |
|---|---|---|
| ① | ★**구독 `pt` 는 서버가 이 연결(구독 BUNDLE)에 배정한 값**이다 — 발행자 값이 아니다. ★**튜플** = media type · encoding name · clock rate · channels · ★**구성/패킷화 파라미터**(H264 `packetization-mode`·profile(`profile-level-id` 앞 네 자리 — ★level 은 키가 아니다: `level-asymmetry-allowed=1` 이면 방향마다 받는 쪽이 정한다, RFC 6184 §8.2.2) · opus `sprop-stereo` · rtx `apt`)다 — RFC 8843 §9.1.1 이 요구하는 것은 *"코덱 구성과 패킷화에 영향을 주는 파라미터"* 이지 `fmtp` 전부가 아니다. ★**받는 쪽 선호**(opus `usedtx`·`useinbandfec`·`maxaveragebitrate`·`minptime`·`stereo`·`maxplaybackrate` — RFC 7587 §6.1 이 decoder/receiver 의 선호로 정의)는 ★**키에 넣지 않는다.** 같은 튜플은 이 연결에서 늘 같은 PT 이고 다른 튜플은 다른 PT 다. 그래서 ★**한 BUNDLE 안의 PT 충돌이 구조적으로 없다.** ★**배정 범위는 `96~127` 을 먼저 쓰고 모자라면 `35~63` 을 쓴다.** `64~95` 는 쓰지 않는다 — rtcp-mux 에서 RTCP 패킷 종류와 겹치고(RFC 5761 §4 *"MUST NOT be used"*) Chromium·Firefox 가 `setRemoteDescription` 에서 거부한다. `0~34` 도 쓰지 않는다 — RFC 3551 의 정적 배정 구간이고 Chromium 이 그 m-line 을 **포트 0 으로 조용히 거절한다**(오류 없이 검은 화면). 실측. ★**예산은 61**(32 + 29)이고 `1pc` 은 `ROOM_JOIN` 번호표의 번호를 뺀 나머지다(Chromium 이 31개를 먹어 34 남는다) — PT 는 트랙이 아니라 튜플마다 하나라 이 폭이 문제가 되지 않는다 | §8-1-1 `pt` · §9-4-10 · §9-4-11 |
| ② | ★**발행 패킷의 확장 ID 는 브라우저 번호로 나간다** — 클라는 그 번호를 `PUBLISH_TRACKS` 의 `*_extmap_id` 로 신고한다. ★**구독자에게는 그 구독자와 협상한 확장만 그 구독자의 확장 ID 로 간다** — 발행자의 확장 영역이 그대로 흘러오지 않는다(transport-cc 일련번호는 구독자 전송로 기준 · 음량 등 값은 발행자 것 · 확장 ID 는 구독자 것). 서버가 ingress 에서 무엇을 읽고 egress 에서 어떻게 다시 짓는가는 서버 정의서의 것이다. ★**근거는 RFC 8843 §12** — *"the identifier used for a given extension MUST identify the same extension across all the bundled media descriptions"*. 두 브라우저가 이것을 집행한다(한 BUNDLE 안에서 확장 ID 하나를 다른 URI 로 바꾼 offer 를 Chromium 149·Firefox 155 가 거부 — 실측 20260912) | §9-4-10 `a=extmap` · §9-4-11 `a=extmap` |
| ③ | ★**구독 m-line 에도 mid 확장을 선언한다**(RFC 8843 §9.1 — *"The RTP MID header extension MUST be enabled, by including an SDP 'extmap' attribute … in each bundled RTP-based m= section in every offer and answer"*). ★**확장 값은 서버가 그 m-line 의 배정표 줄 `mid`(§8-1-1) 로 새로 쓴다** — 발행자의 mid 값(발행자 m-line 번호)은 ②에서 버려져 새지 않는다. 그래서 브라우저는 SSRC 로도 mid 확장으로도 같은 m-line 을 고른다 | §9-4-11 `sdes:mid` · §9-4-16 |
| ④ | ★`1pc` 는 ★**내 번호표를 먼저 신고한다** — 새로 만든 연결의 브라우저 offer 가 정한 PT 표·확장 ID 표를 `ROOM_JOIN` 요청의 `extmap`·`codecs`(§4-1)로 싣고, 서버는 그 표를 피해 구독 PT 를 배정하고 그 표의 번호로 확장을 새로 쓴다. ★**그 서버에 이 세션의 연결이 이미 있으면 요청의 표는 버린다** — 기존 연결의 표를 피해 배정하고 기존 연결의 번호로 확장을 쓴다(클라는 새 연결을 버리고 기존 연결을 쓴다 — 브라우저가 연결마다 같은 번호를 붙인다는 보장이 없다). 첫 협상 확정본은 그 offer 의 부분집합이라 offer 표를 신고하면 충분하다 | §9-4-19 · §9-2-1 |

<a id="RULE-LINK-DOWNLINK-SSRC"></a>**RULE-LINK-DOWNLINK-SSRC** · 규칙

★★**구독 m-line 의 SSRC 는 늘 다운링크 SSRC 다** — 서버가 배정표 줄마다 하나 내고(§8-1-1 `ssrc`) ★**줄이 사는 동안 바꾸지 않는다**(빈 줄 · 재배정 · 출처 전환에도). 발행자의 업링크 SSRC 는 구독자에게 새지 않는다 — 시뮬캐스트 · 무전 슬롯 · 전이중 모두 같다.

→ 구조 — [`SUB.type.MlineEntry`](08-sub.md#SUB.type.MlineEntry)

<a id="RULE-LINK-SOURCE-SWITCH"></a>**RULE-LINK-SOURCE-SWITCH** · 규칙

★★**출처 전환 — 다운링크 스트림 밑의 업링크 스트림이 바뀌는 일은 셋이고, 서버가 똑같이 다룬다**: 레이어 전환(시뮬캐스트 — 같은 트랙의 다른 업링크 스트림) · 화자 전환(무전 슬롯 — 발언권) · 재배정(전이중 줄 — §8-1-2). 클라에게는 SSRC 도 SDP 도 그대로다(§9-4-2).

★**클라가 볼 것 — 이것이 계약의 전부다.** 다운링크 스트림의 seq 는 전환을 건너 하나씩 오르고 timestamp 는 흐른 시간만큼 오른다(한 다운링크 스트림으로 이어진다 — 한 방의 발행이 같은 코덱이라 클럭 · PT 가 같다, §4-1-1). 영상은 전환 순간의 짧은 손실 뒤 ★**새 출처의 키프레임부터** 온다. 전환 경계 앞 seq 의 NACK 에는 재전송이 오지 않는다(audio 는 NACK 을 협상하지 않는다 — §9-4-11 `a=rtcp-fb`). 받는 SR 의 SSRC 는 다운링크 SSRC 이고 NTP ↔ RTP 대응은 발행자의 것 그대로다. 재배정이면 표(`MLINE_EVENT`)가 먼저 오고, 확인(`READY{arev}`) 뒤에 새 출처가 온다(§8-1-3).
★서버가 그것을 어떻게 잇는가(오프셋 · 키프레임 요청 · NACK · PLI · SR 번역)는 서버 정의서의 것이다 — 상대편 구현이 몰라도 붙는다.

<a id="COND-LINK-REBUILD"></a>**COND-LINK-REBUILD** · 성립 조건

★**성립 조건** — 이 넷은 *"서버가 egress 에서 다시 짓는다"* 가 참인 동안만 참이다. 다시 짓기를
그만두면 §9-4-10 의 *"번호는 offer 것"* 과 §9-4-11 의 *"mid 확장을 선언한다"* 가 함께 무너진다.

#### 9-4-3. 미디어

<a id="PROP-LINK-RESTART-CREDS-ACCEPTED"></a>**PROP-LINK-RESTART-CREDS-ACCEPTED** · 성질

통로가 비고 서버가 연결을 아는 동안, 클라가 적용 중인 새 ICE 자격은 서버가 받는 자격(쓰는 것 또는 대기분)이다 — 서버는 자격 한 벌마다 옛 것과 새 것을 함께 받고 · 새 것으로 검증된 첫 검사가 오면 옛 것을 내리고 · 새 것이 쓰이기 전에 요청이 또 오면 대기분을 새 것으로 바꾼다(벌마다 최대 둘 — RULE-LINK-ICE-RESTART-PROC). ★검사 응답의 MESSAGE-INTEGRITY 계산(요청 USERNAME 의 ufrag 의 pwd)은 STUN 구현 세부라 이 모형이 재지 않는다(e2e).

*식: LH10 · 깨지면: 재시도가 겹치면 클라는 서버가 버린 자격을 적용하려다 시한만 되풀이한다 — 되살리지 못한다*

<a id="PROP-LINK-CREDS-AGREE"></a>**PROP-LINK-CREDS-AGREE** · 성질

통로가 빈 OK 에서 클라가 쓰는 자격의 세대가 서버가 받는 자격의 세대다.

*식: LH7 · 깨지면: 클라는 되살렸다고 아는데 서버는 그 자격의 검사를 받지 않는다*

#### 9-4-4. SDP 조립

<a id="RULE-LINK-SDP-FREE"></a>**RULE-LINK-SDP-FREE** · 규칙 — SDP 조립

★★**이 절이 SDP-Free 의 전부다.** 서버는 SDP 를 주지도 받지도 않는다.
클라가 **서버 쪽 SDP 를 직접 지어서** 자기 브라우저에 먹인다.

★**이 절은 두 층이다** — ★**규범**은 각 절의 **표(서버가 준 재료 → SDP 줄 매핑)와 불변**
(어기면 증상 칸의 것이 보인다)이고, ★**§9-4-14 완성 예시는 참고(비규범)**다. SDP 는 클라 내부
사정이 아니라 ★**wire 계약의 실체**다 — 서버가 SDP 를 안 주는 대신 "클라가 조립할 서버 쪽
SDP" 의 규칙이 곧 프로토콜이고, ★**서버 구현자도 이 절로 자기 기대를 안다.** 그래서 이 절은
소스 참조로 대체하지 않는다.

#### 9-4-5. 무엇을 조립하는가 — ★연결마다 방향이 다르다

<a id="RULE-LINK-SDP-DIRECTION"></a>**RULE-LINK-SDP-DIRECTION** · 규칙 — 무엇을 조립하는가 — ★연결마다 방향이 다르다

서버는 SDP 를 주지도 받지도 않는다. 그래서 **서버 쪽 SDP 를 클라가 대신 조립해서**
자기 브라우저에 먹인다. ★★**그 "서버 쪽 SDP" 가 answer 인지 offer 인지는 연결마다 다르다.**

| 연결 | 누가 offer 인가 | 클라가 조립하는 것 | 브라우저 호출 순서 |
|---|---|---|---|
| **발행(publish)** | **브라우저** | 서버의 ★**answer** | `createOffer` → `setLocalDescription(offer)` → **answer 조립** → `setRemoteDescription(answer)` |
| **구독(subscribe)** | ★★**서버**(클라가 대신 조립) | 서버의 ★**offer** | **offer 조립** → `setRemoteDescription(offer)` → `createAnswer` → `setLocalDescription(answer)` |

★★**구독이 반대인 것이 이 절에서 가장 틀리기 쉬운 곳이다.** 받을 트랙은 서버가 아는 것이고
(배정표, §8-1-2) 브라우저는 모른다. 브라우저에게 offer 를 만들게 하면 ★**m-line 이 하나도 없는
offer** 가 나온다 — 받을 것을 아무것도 선언하지 못한다.

그래서 구독 쪽은 ★**클라가 `tracks[]` 로 서버의 offer 를 조립**해서 `setRemoteDescription` 에
넣는다. 그러면 브라우저가 **그 offer 를 보고 구독 트랜시버를 스스로 만들고** answer 를 낸다.

★**구독 트랜시버를 클라가 미리 만들지 않는다.** `addTransceiver` 를 부르지 않는다 —
합성한 offer 가 m-line 을 선언하고 `setRemoteDescription` 이 트랜시버를 만든다.
그래서 배정표 줄의 `mid`(§8-1-1) 가 곧 그 m-line 의 `a=mid:` 가 된다(§9-4-11 정렬 규칙이 성립하는 이유다).

양쪽 다 클라가 조립하는 SDP 에는 **서버 값**이 들어간다 — 서버 지문, 서버 ICE 자격, 서버 주소,
그리고 ★**`a=setup:passive`**(서버가 수동이라는 뜻). 클라 자신은 능동이 되고 그건 브라우저가 알아서 한다.

<a id="RULE-LINK-SDP-PC-COUNT"></a>**RULE-LINK-SDP-PC-COUNT** · 규칙 — ★연결을 몇 개로 하나 — `1pc` 가 지향점, `2pc` 가 기준선

| 모드 | 무엇 | 언제 |
|---|---|---|
| ★**`1pc`** | 연결 **하나**가 발행·구독을 겸한다 | ★**지향점.** 무전의 상시 데이터 비용을 반으로 줄인다 |
| **`2pc`** | 발행·구독 **둘** | ★**호환 기준선.** 브라우저가 `1pc` 를 못 받으면 여기로 |

★★**왜 `1pc` 를 지향하나 — 무전은 종일 켜두는 물건이라 상시 비용이 지배한다.**

무발화 구간에도 연결을 유지하려면 keepalive 가 계속 나간다. 그리고 그것은 ★**전송로마다** 흐른다.

| 1 USER 무발화 상시대기 (실측 20260624, `2pc`) | |
|---|---|
| 합계 | **1.32 kbps** ≈ 13.5 MB/일 ≈ ★**406 MB/월** (host 후보만 잰 **하한**) |
| STUN(ICE consent) | ★**61%** — 안 보내면 연결이 끊긴다 |
| RTCP | 31% |
| DTLS·데이터 채널 | 7% |
| RTP 음성 | 1% (마이크는 절전이 듣는다) |

★**99%가 keepalive 다.** 마이크를 꺼도 안 줄고, ★**전송로를 하나로 줄여야 준다.**
`1pc` 는 STUN·DTLS·RTCP 를 **한 벌만** 흘린다.

#### 9-4-6. 발행 — 브라우저가 offer, 클라가 answer

<a id="RULE-LINK-SDP-ORDER-PUB"></a>**RULE-LINK-SDP-ORDER-PUB** · 규칙 — 발행 — 브라우저가 offer, 클라가 answer

1. 데이터 채널을 먼저 연다(§9-4-13). ★**offer 를 만들기 전이어야** `m=application` 이 생긴다.
   ★★**트랙이 하나도 없어도 이 협상을 한다** — `m=application` 하나로 협상이 성립하고,
   ★**청취 전용의 정상 경로가 그것이다**(§9-4-13 · 완성 예시 §9-4-17).
2. 브라우저에게 offer 를 만들게 한다(`createOffer` → `setLocalDescription`).
3. ★**그 offer 를 읽어** m-line 목록과 `a=group:BUNDLE` 줄을 꺼낸다.
4. offer 의 m-line **순서 그대로** answer 의 m-line 을 조립한다. ★**개수와 순서가 같아야 한다.**
5. 완성한 answer 를 `setRemoteDescription({type:"answer"})` 에 넣는다.

★**증상** — 4를 어기면(m-line 을 빼거나 순서를 바꾸면) **브라우저가 answer 를 거부한다.**

#### 9-4-7. 구독 — 클라가 offer, 브라우저가 answer

<a id="RULE-LINK-SDP-ORDER-SUB"></a>**RULE-LINK-SDP-ORDER-SUB** · 규칙 — 구독 — 클라가 offer, 브라우저가 answer

1. 보관한 ★**배정표**(§8-1-2)의 줄을 ★**`mid` 수치 오름차순으로 정렬**한다(§9-4-11). ★트랙 목록(§6-1-1)은 보지 않는다.
2. 그 순서대로 구독 m-line 을 조립한다(§9-4-11). ★**`a=mid:` 는 줄의 `mid` 를 그대로 쓴다.**
3. 완성한 offer 를 `setRemoteDescription({type:"offer"})` 에 넣는다.
4. `createAnswer` → `setLocalDescription(answer)`.
5. 협상이 성공하면 `READY` 를 보낸다(§7-1-2). ★**실패했으면 보내지 않는다.**

★**재협상일 때**는 3 앞에 `setLocalDescription({type:"rollback"})` 이 필요할 수 있다
(브라우저가 이미 협상 중이면). ★**보관한 배열로 처음부터 다시 조립한다** — 붙어 있는 SDP 를
읽어 고치지 않는다(§9-4-11).

★**증상** — 1의 정렬을 어기면 재협상 때 **엉뚱한 m-line 에 붙는다.**

→ 구조 — [`LINK.sdp.recv`](#LINK.sdp.recv) · [`LINK.sdp.EX-LINK-SDP-SUB-OFFER`](#EX-LINK-SDP-SUB-OFFER)

#### 9-4-8. 세션 헤더

<a id="RULE-LINK-SDP-SESSION-HEADER"></a>**RULE-LINK-SDP-SESSION-HEADER** · 규칙 — 세션 헤더

| 줄 | 규칙 |
|---|---|
| `o=` 의 `{session-id}` | ★**한 번 정해 그 연결 내내 같은 값**을 쓴다. **발행**(answer 조립) = ★offer 의 값을 그대로 옮긴다 · **구독**(offer 조립) = 클라가 정한다. ★★**숫자 문자열이라야 한다**(RFC 4566 §5.2) — 서버 신원(`sfu_id`) 같은 문자열을 넣으면 ★**Firefox 가 SDP 를 통째로 거부한다**(`SDP Parse Error: Invalid owner session id specified for o=`). Chrome 은 받아 주므로 ★Chrome 만으로는 안 드러난다 |
| ★`o=` 의 `{version}` | ★**다시 조립할 때마다 +1** — 같은 세션의 새 판이라는 뜻이다(RFC 4566 `o=`). 발행은 offer 값 그대로 |
| `a=group:BUNDLE` | **발행**(answer 조립) = ★offer 에서 읽은 목록 **그대로**. **구독**(offer 조립) = ★내가 만든 m-line 의 `mid` 를 **순서대로**, ★**`inactive` 인 것도 남긴다**(§9-4-11) |
| ★`a=ice-lite` | ★★**반드시 넣는다.** 서버는 **ICE-lite** 다 — 후보를 모으지도 연결성 검사를 보내지도 않는다. 이 줄이 있어야 브라우저가 **controlling** 이 되어 검사를 자기가 주도한다 |

★**증상** — `a=ice-lite` 를 빼면 브라우저가 서버의 검사 요청을 기다린다. 서버는 보내지 않으므로
★**ICE 가 `checking` 에서 안 넘어간다.** §9-4-9 이 *"후보 한 줄이 전부"* 인 것이 이 줄의 결과다.

→ 구조 — [`LINK.sdp.session`](#LINK.sdp.session)

<a id="LINK.sdp.session"></a>SDP 줄 틀 — 세션 헤더(조립기가 이 틀로 짓는다 · 재료 → 줄은 이 규칙의 표)

```
v=0
o=- {session-id} {version} IN IP4 127.0.0.1   ← 발행 = offer 의 값 그대로 · 구독 = 클라가 정한다(숫자 문자열 · 다시 조립할 때마다 version +1)
s=-
t=0 0
a=group:BUNDLE {mid 목록}                       ← 발행 = offer 의 목록 그대로 · 구독 = 배정표 줄의 mid 를 순서대로(빈 줄도 남긴다)
a=msid-semantic: WMS *
a=ice-lite                                    ← ★반드시 — 서버는 ICE-lite 다
```

#### 9-4-9. m-line 마다 공통으로 들어가는 것

<a id="RULE-LINK-SDP-COMMON"></a>**RULE-LINK-SDP-COMMON** · 규칙 — m-line 마다 공통으로 들어가는 것

| 줄 | 규칙 |
|---|---|
| `a=mid` | **발행** = ★offer 의 그 m-line `mid`. **구독** = ★★**배정표 줄의 `mid` 를 그대로**(§8-1-1 — 내가 offer 를 쓰므로 내가 정한다, §9-4-5) |
| `a=rtcp-mux` | ★**미디어 m-line 에 붙인다** — `m=application` 에는 붙이지 않는다(§9-4-12 이 그 m-line 의 전량이다). video 는 ★`a=rtcp-rsize` 도 함께 |
| `a=candidate` | ★**UDP 한 줄이 기본이고 전부다.** 후보가 서버 host 뿐이라 수집도 검사도 없다(서버가 ICE-lite, §9-4-8). 우선순위 `2113937151` 은 호스트 후보 고정값. ★**`server_config.ice.tcp_port`(§9-1-1)가 왔을 때만 TCP passive 줄을 하나 더 낸다** — 안드로이드 축의 폴백 경로다. ★**상용 웹은 UDP 한 줄**이다(웹의 TCP 후보는 QA 스위치). `{prio}` 는 UDP 줄보다 작으면 된다(RFC 6544 §4.2 — 서버는 ICE-lite 라 값을 쓰지 않는다). 둘 다 붙으면 ★**서버는 클라가 지명한 쌍을 따른다** — `USE-CANDIDATE` 가 실린 검증된 검사가 마지막으로 온 쌍이다(서버가 ICE-lite, RFC 8445 §7.3.2). 클라는 두 후보를 다 올리고 둘 다 consent 를 유지한다 |
| `a=end-of-candidates` | ★**반드시 붙인다** — 후보가 하나뿐임을 형식으로 닫는다(RFC 8840). ★libwebrtc 는 이 줄을 아직 처리하지 않는다(`sdp_offer_answer.cc` TODO) — 동작을 이 줄에 기대지 않는다, 후보 한 줄이 곧 끝이다 |
| ICE 자격 | ★**연결마다 다르다** — 발행은 `publish_*`, 구독은 `subscribe_*`. ★**`ICE_RESTART` 응답(§7-1-2)이 새 값을 줄 때만 바뀐다**(아래 재시작 조립) |
| `a=setup` | ★**항상 `passive`** |

★**두 번째 협상부터 ICE·DTLS 가 다시 돌지 않는 원리** — 브라우저는 새 SDP 의 전송로 수(BUNDLE 하나 = 전송로 하나)와
ICE 자격·지문을 직전 확정본과 견주고, 같으면 ICE 수집·DTLS 핸드셰이크를 건너뛴다(RFC 8829 §5.2.2 — 후속 offer 에서
`a=ice-ufrag`·`a=ice-pwd` 는 ICE 재시작 옵션이 아니면 그대로여야 한다). 그래서 §9-4-18 의 재협상이 미디어를 끊지 않는다 —
자격을 바꾸면 그 순간 재시작으로 읽힌다(아래 증상).

★**증상** — ICE 자격을 재협상 때 **다른 값으로 바꾸면** 브라우저가 **ICE 재시작으로 오인**한다.
★**같은 값을 계속 쓴다.**

★★**ICE 재시작 조립 — 자격이 바뀌는 경우는 이것 하나다**(§7-1-2 `ICE_RESTART`, 절차 §9-2-1). ★**바뀌는 재료는 ICE 자격 두 줄뿐이다** —
`mid`·m-line·PT·`extmap` 은 그대로이고 ★**`a=fingerprint`·`a=setup` 은 한 글자도 안 바뀐다.** 지문과 `a=tls-id` 가 그대로면
★**같은 DTLS 가 새 ICE 위에서 이어진다**(RFC 8829 §5.11). ★우리 SDP 에는 `a=tls-id` 가 없으므로 ★**지문·`a=setup` 불변이 DTLS 연속의
유일한 근거다.** 방향은 평소 조립 그대로다.

| 연결 | offer 를 쓰는 쪽 | 조립 |
|---|---|---|
| `2pc` 발행(§9-4-6) | 브라우저 | `createOffer({iceRestart:true})` → 새 서버 자격(`publish_*`)으로 answer 를 짓는다 |
| `2pc` 구독(§9-4-7) · `1pc`(§9-4-18 규칙 1) | 클라 합성 | 새 서버 자격으로 offer 를 짓는다 → 브라우저 answer 가 ★**새 로컬 자격을 스스로 만든다**(RFC 8829 §5.3.3 — answer 에는 재시작 옵션이 필요 없다) |

★`pc.restartIce()` 를 쓰지 않는 이유 — ★**협상을 시작하는 곳이 협상 큐 하나**라서다(§9-2-1). 둘은 같은 결과를 내고
`negotiationneeded` 가 나도 해롭지 않다 — 가르는 것은 소유 구조다. ★서버가 ICE-lite 라 검사를 보내지 않으므로
★**클라의 새 자격을 서버에 알릴 필요가 없다** — 왕복 하나로 끝난다.

★**증상** — 재시작에서 지문이나 `a=setup` 까지 새로 지으면 → 브라우저가 DTLS 를 허물고 다시 세운다. SCTP·DC 가 함께
사라져 ★**재시작이 재구축과 같은 값을 치른다.**

→ 구조 — [`LINK.sdp.common_head`](#LINK.sdp.common_head) · [`LINK.sdp.common_tail`](#LINK.sdp.common_tail)

<a id="LINK.sdp.common_head"></a>SDP 줄 틀 — m-line 공통 — 머리(조립기가 이 틀로 짓는다 · 재료 → 줄은 이 규칙의 표)

```
c=IN IP4 {ice.ip}
a=ice-ufrag:{ufrag}   ← 발행 연결 = ice.publish_ufrag · 구독 연결 = ice.subscribe_ufrag
a=ice-pwd:{pwd}       ← 발행 연결 = ice.publish_pwd · 구독 연결 = ice.subscribe_pwd
a=fingerprint:{dtls.fingerprint}
a=setup:passive
a=mid:{mid}           ← 발행 = offer 의 그 m-line mid · 구독 = 배정표 줄의 mid(SUB.type.MlineEntry)
a=rtcp-mux            ← audio·video 만
a=rtcp-rsize          ← video 만
```

<a id="LINK.sdp.common_tail"></a>SDP 줄 틀 — m-line 공통 — 꼬리(조립기가 이 틀로 짓는다 · 재료 → 줄은 이 규칙의 표)

```
a=candidate:1 1 udp 2113937151 {ice.ip} {ice.port} typ host generation 0
a=candidate:2 1 tcp {prio} {ice.ip} {ice.tcp_port} typ host tcptype passive generation 0   ← ★ice.tcp_port 가 왔을 때만
a=end-of-candidates
```

#### 9-4-10. 발행 m-line (내 트랙)

<a id="RULE-LINK-SDP-PUB-MLINE"></a>**RULE-LINK-SDP-PUB-MLINE** · 규칙 — 발행 m-line (내 트랙)

★**발행은 answer 조립이다**(§9-4-5). offer 의 `sendonly`/`sendrecv` m-line 에 하나씩 대응한다.


| 규칙 | 내용 |
|---|---|
| **PT·rtpmap·fmtp** | ★★**offer 의 원문 줄을 그대로 옮긴다.** 서버 값으로 **다시 쓰지 않는다** — 클럭·채널·프로파일이 전부 협상 산물이다. SFU 는 디코드하지 않으므로 PT 를 정할 이유가 없다. ★**예외 하나** — opus `fmtp` 의 **받는 쪽 선호 파라미터**(`usedtx`·`useinbandfec`·`maxaveragebitrate`·`stereo`·`maxplaybackrate` — RFC 7587 §6.1 이 decoder/receiver 의 선호로 정의)는 ★**answer 가 정한다**(answer = 서버의 수신 선호 = 클라가 자기 업링크를 빚는 자리). 코덱 이름·클럭·PT·프로파일은 그대로다. ★★**answer 의 바탕은 방 코덱의 고정 opus 값이다**(`minptime=10;useinbandfec=1` — §4-1-1) — 구독 줄의 선언도 그 값이다. ★**그 위의 받는 쪽 선호(`usedtx` 등)는 클라 설정이 더한다** — 무엇을 더하나는 SDK 정의서가 정하고 서버는 보지 않는다(아래 — PT 튜플 밖). ★DTX(`usedtx=1`)를 더하면 발행자는 무음에 패킷을 듬성하게 낸다(400ms 에 하나) — 흐름이 멎는 것이 아니다. 클라는 확정본(answer)의 값을 `PUBLISH_TRACKS.fmtp` 로 신고한다(§7-1-2). ★**이 예외의 결과값은 PT 튜플에 들어가지 않는다**(§9-4-2 ① — 받는 쪽 선호는 키가 아니다). 방향마다 값이 달라도 PT 는 같다 |
| **코덱 고르기** | `server_config.codecs[].name` 에 있는 것만 남긴다. 없는 코덱은 그 PT 줄째로 뺀다. ★**첫 협상 확정본(`1pc`, §9-2-1)은 그 교집합을 전부 남긴다 — 코덱 선택으로 좁히지 않는다.** 좁히면 뒤에 쓰는 코덱·프로파일의 PT 를 서버가 이미 다른 튜플에 배정했을 수 있다(§9-4-2). 이후 발행 m-line 은 확정본에 있는 코덱만, PT 는 확정본 번호 그대로 쓰고(§9-4-19), 코덱 선택은 발행 시점에 합성 offer 의 새 발행 m-line 에 그 코덱 줄만 적는 것으로 한다. ★★**그 줄은 그 방 미디어 코덱의 줄이다**(§4-1-1) — H264 는 `packetization-mode` · profile 이 방 값과 같은 줄(level 은 보지 않는다 · Chrome 은 H264 를 profile 별로 여러 줄 낸다 — 아무거나 고르면 `1006`). ★확정본에 그 줄이 없으면 보내지 않고 앱 오류로 돌린다 |
| ★**남는 코덱이 0개면** | ★**조용히 넘어가지 않는다.** m-line 이 코덱 없이 나가면 검은 화면이 된다 — **협상 실패로 다룬다** |
| `a=rtcp-fb` | ★**여기만 서버가 정한다.** `server_config.codecs[].rtcp_fb` 를 쓴다 — *"나한테 이 피드백을 보내라"* 는 서버의 요구다 |
| `a=extmap` | ★**offer 와 서버 선언(`server_config.extmap`)의 교집합**만. ★★**번호는 offer 것**을 쓴다. 새 번호를 제안하면 상대가 그것을 따라가 **와이어 확장 번호가 조용히 바뀐다**(rid/mid/twcc 파싱이 죽는다). ★그래서 발행 패킷은 브라우저 번호로 나가고 구독자 쪽 번호와 다르다 — ★**서버가 egress 에서 헤더 확장을 구독자 확장 ID 로 다시 짓는다**(§9-4-2 ②). 클라는 그 번호를 `PUBLISH_TRACKS` 의 `*_extmap_id` 로 신고한다. ★`sdes:mid` 도 포함한다 — 서버가 ingress 에서 SSRC 학습 재료로 읽는다(시뮬캐스트는 층별 SSRC 가 SDP 에 없어 mid·rid 확장으로 배운다) |
| RTX | offer 에 rtx PT 가 있으면 ★`a=rtpmap:{rtx_pt} rtx/{clockrate}` + ★`a=fmtp:{rtx_pt} apt={pt}` 를 **짝으로** 넣는다 |
| ★**시뮬캐스트** | offer 그 m-line 에 `a=simulcast:send` 가 있을 때만 ★`a=rid:l recv` · `a=rid:h recv` · `a=simulcast:recv l;h` 을 넣는다. ★**방향을 `recv` 로 뒤집는다**(서버가 받는 쪽이다). ★★**rid 목록의 순서가 품질을 말하고 이름은 말하지 않는다** — 브라우저는 목록 **앞쪽**을 축소(1/2), **마지막**을 원본 해상도로 잡는다(Chromium 149·Firefox 155 실측 — RFC 8853 은 rid 를 불투명 식별자로 둔다). 그래서 ★**낮은 품질부터** `l;h` 로 적고, 브라우저 offer 경로에서는 클라가 `sendEncodings` 를 같은 순서로 만든다 |
| 방향 | ★**`recvonly`**(서버 입장에서 받는다). offer 가 `inactive` 면 `a=inactive` |

★**증상**
- 서버가 모르는 코덱을 남기면 → 그 PT 로 보낸 패킷을 **서버가 못 알아본다.**
- 서버 고정 PT 로 다시 쓰면 → 번호만 겹치고 프로파일이 어긋나 ★**검은 화면**이 된다.
- `a=simulcast` 를 빼면 → 브라우저가 **업링크 스트림 하나만 보낸다.** `rid` extmap 이 있어도 소용없다.
- extmap 번호를 새로 매기면 → ★**rid·mid·twcc 파싱이 죽는다**(검은 화면 계열).

→ 구조 — [`LINK.sdp.send`](#LINK.sdp.send) · [`LINK.sdp.common_head`](#LINK.sdp.common_head)

<a id="LINK.sdp.send"></a>SDP 줄 틀 — 발행 m-line(조립기가 이 틀로 짓는다 · 재료 → 줄은 이 규칙의 표)

```
m={offer 의 kind} {ice.port} UDP/TLS/RTP/SAVPF {PT 목록}   ← PT 목록 — offer 의 그 m-line 것(남긴 코덱 · rtx)
… m-line 공통 — 머리 …
a={방향}                                                  ← 서버가 받는다 — offer 가 보내면 recvonly · offer 가 inactive 면 inactive
a=rtpmap:{pt} {offer 의 rtpmap 그대로}                      ← offer 의 rtpmap 그대로 · 되풀이 codec
a=fmtp:{pt} {offer 의 fmtp 그대로}                          ← offer 의 fmtp 그대로 · 되풀이 codec
a=rtcp-fb:{pt} {server_config 의 rtcp_fb}                ← 되풀이 codec.server_config 의 rtcp_fb
a=rtpmap:{rtx_pt} rtx/{clockrate}                       ← offer 에 rtx 가 있을 때만 · 되풀이 codec
a=fmtp:{rtx_pt} apt={pt}                                ← 되풀이 codec · rtx 일 때
a=extmap:{extmap.id} {extmap.uri}                       ← ★offer 와 server_config.extmap 의 교집합 · 번호는 offer 것 · 되풀이 extmap
a=rid:l recv                                            ← ★시뮬캐스트일 때만 — 낮은 품질부터
a=rid:h recv                                            ← simulcast 일 때
a=simulcast:recv l;h                                    ← simulcast 일 때
… m-line 공통 — 꼬리 …
```

#### 9-4-11. 구독 m-line (남의 트랙)

<a id="RULE-LINK-SDP-SUB-MLINE"></a>**RULE-LINK-SDP-SUB-MLINE** · 규칙 — 구독 m-line (남의 트랙)

★**구독은 offer 조립이다**(§9-4-5). ★**배정표의 줄 하나가 m-line 하나다**(§8-1-2) — 재료는 그 줄뿐이다.


**★클럭·채널**(`{clockrate}`·`{channels}`) — 서버는 이 값을 주지 않는다(§9-1-1). ★**코덱 이름에서 정해지는 상수**이므로
아래 표를 쓴다. ★이것은 §8-1-1 이 금지한 *"PT 정책표 폴백"* 과 **다른 것**이다 — PT 는 협상 산물이라
추측하면 안 되지만, 클럭은 코덱 정의상 고정이다.


| 규칙 | 내용 |
|---|---|
| **정렬** | ★★**`mid` 수치 오름차순**(사전순이 아니다 — `"10"` 은 `"2"` 뒤다). m-line 위치 ↔ `mid` 가 나중 SDP 에서 바뀌면 안 된다 — ★서버의 mid 발급 규칙(신규는 최대 뒤·재사용은 같은 kind · 줄은 지우지 않는다, §8-1-1)이 그것을 보장한다: 새 m-line 은 언제나 **끝에 붙고**, 재사용은 **있던 자리를 되살린다** |
| **PT** | ★줄의 `pt` 를 쓴다. **없으면 실패로 다룬다** — 정책표로 폴백하지 않는다. ★`pt` 는 서버가 이 연결에 배정한 값이라(§8-1-1) 한 BUNDLE 안에서 충돌하지 않는다. `1pc` 는 `ROOM_JOIN` 요청에 내 번호표를 먼저 실어(§4-1) 서버가 그 PT 를 피해 배정한다(§9-4-2 ④) |
| `a=rtcp-fb` | ★**`codecs_sub` 가 오면 그것의 `rtcp_fb`**, 없으면 `codecs` 의 것. ★★**video 에서 PLI/NACK 을 빠뜨리면 브라우저가 키프레임을 요청하지 않는다.** ★**audio 에는 NACK 을 협상하지 않는다**(오디오 NACK 전면 미채택 — 반이중 슬롯은 화자 교대마다 seq 가 건너뛰어 NACK 이 폭주한다) |
| ★**구독 코덱 권위** | ★`codecs_sub` 는 **`rtcp_fb` 만** 지배한다. **PT·코덱 이름·클럭·★`fmtp` 는 배정표의 줄**이 지배한다(§8-1-1). 둘이 겹치지 않는다 |
| ★**`fmtp`** | ★★**줄의 `fmtp` 가 오면 반드시 넣는다**(서버 선언값 — §4-1-1). *"필요할 때"* 가 아니다 — ★**H264 는 `profile-level-id`·`packetization-mode` 가 사실상 필수**이고, 없으면 브라우저가 `packetization-mode=0`·baseline 을 가정한다. 안 오면 그 줄을 넣지 않는다(지어내지 않는다) |
| `a=extmap` | `server_config.extmap` 에서 ★**그 m-line 의 종류에 해당하는 것만** — §9-1-1 표의 "어디에" 열대로(audio: `1`·`4`·`5`·`6` / video: `1`·`5`·`6`. `10`·`11` 은 발행 시뮬캐스트용이라 구독엔 없다). §9-4-16 예시가 그 모양이다. `1pc` 는 §9-4-19(확정본 번호). ★transport-cc 를 빠뜨리면 **대역 추정이 죽는다** |
| **`sdes:mid` 확장** | ★★**선언한다**(RFC 8843 §9.1 — 모든 bundled m= section 에 mid 확장 선언). 구독은 ★**서버가 보내는 방향**이라 브라우저가 RTP 헤더의 mid 로 m-line 을 고르는데, ★**서버가 egress 에서 확장 값을 그 m-line 의 배정표 줄 `mid` 로 새로 쓰므로**(§9-4-2 ③ 다시 짓기) SSRC 로 고르나 mid 로 고르나 같은 m-line 이다. 발행자의 mid 값은 새지 않는다(발행자 확장 영역은 통째로 버려진다, §9-4-2 ②). ★옛 규격은 이 확장을 빼고 SSRC 로만 갈랐다(알고 하는 이탈) — 12차 뒤 다시 짓기로 이탈이 없어졌다 |
| **cname** | ★`"ox-sfu"` 고정 |
| **`a=msid`** | ★**줄의 `stream_id` 와 `ox-m{mid}`** — 둘 다 줄마다 고정이다. stream-id 는 전이중 줄 = `ox-m{mid}` · ★**무전 = `ox-ptt`**(§8-1-1). ★**`track_id` 를 넣지 않는다** — 실린 트랙이 바뀌어도(재배정, §8-1-2) SDP 가 그대로여야 한다 |
| RTX | ★**줄의 `rtx_pt`·`rtx_ssrc` 는 짝으로 온다**(§8-1-1). 오면 m= 줄의 PT 목록 · `a=rtpmap` · `a=fmtp apt=` · `a=ssrc` 두 줄 · ★**`a=ssrc-group:FID {ssrc} {rtx_ssrc}`** 를 ★**전부 함께** 넣는다. 하나만 왔으면 실패로 다룬다 |
| ★**빈 줄** | ★**다른 줄과 똑같이 짓는다** — 방향(`a=sendonly`) · `a=msid` · `a=ssrc` 그대로, BUNDLE 목록에도 그대로(§8-3-1). 그 줄로 오는 패킷이 없을 뿐이다. ★옛 꼴(`a=inactive` · `port=7` · `a=msid`·`a=ssrc` 뺌)은 걷어 냈다 — 빈 줄이 생기고 채워질 때마다 재협상이 났다 |
| **다시 조립할 때** | ★★**보관한 배열로 처음부터 다시 조립한다.** 붙어 있는 SDP 를 읽어 고치지 않는다 |

★**무전이 한 stream-id 를 쓰는 이유** — 화자들이 같은 m-line 을 돌려쓰므로 입술 동기가 한 묶음이어야 한다.

★**전이중 줄은 묶지 않는다** — 같은 사람의 audio · video 가 서로 다른 줄에 실리고(audio 는 서버가 음량으로 · video 는 클라가 고른다, §8-1-2) 재배정마다 묶음(msid)을 바꾸면 SDP 가 바뀐다. 그래서 브라우저의 A/V 동기 묶음을 쓰지 않는다 — 저지연 회의에서는 두 지터 버퍼가 다 최소 근처에서 돌아 어긋남이 작다.
선례가 같다 — Jitsi(SSRC rewriting 에서 SSRC 마다 msid 를 따로 준다 — `remote-audio-N` · `remote-video-N`) · LiveKit(`syncStreams` 를 켜면 같은 stream 으로 묶는 대신 transceiver 재사용을 끈다 — 재생 지연이 200ms 를 넘을 때 도움이라 적는다). 재사용과 묶음은 함께 못 쓴다.

★**증상**
- `port=0` 을 쓰면 → **BUNDLE 태그가 옮겨가 전송이 깨진다.**
- 정렬을 어기면 → 재협상 때 **엉뚱한 m-line 에 붙는다.**
- `pt` 없이 정책표로 채우면 → ★**조용히 검은 화면**이 된다.
- ★`fmtp` 가 왔는데 안 넣으면 → 코덱 이름은 맞는데 프로파일이 어긋나 ★**패킷은 오는데
  디코딩이 0** 이다. 0613 검은 화면이 ★**이름만 맞춘 채로 재발한다.**
- ★`a=rtcp-fb` 에 PLI 가 없으면 → 브라우저가 키프레임을 안 부른다. ★**`pliCount=0` 이 그 서명이다.**
- `a=ssrc-group:FID` 를 빠뜨리면 → **재전송 패킷을 원본으로 오인**해 영상이 깨진다.
- transport-cc extmap 을 빠뜨리면 → **대역 추정이 안 돌아 화질이 안 올라간다.**

→ 구조 — [`LINK.sdp.recv`](#LINK.sdp.recv) · [`LINK.sdp.common_head`](#LINK.sdp.common_head) · [`LINK.fact.SdpClock`](#LINK.fact.SdpClock)

<a id="LINK.sdp.recv"></a>SDP 줄 틀 — 구독 m-line(조립기가 이 틀로 짓는다 · 재료 → 줄은 이 규칙의 표)

```
m={row.kind} {ice.port} UDP/TLS/RTP/SAVPF {PT 목록}   ← PT 목록 — row.pt[ row.rtx_pt]
… m-line 공통 — 머리 …
a=sendonly                                          ← 서버가 보낸다
a=rtpmap:{row.pt} {row.codec}/{클럭[/채널]}             ← 코덱 클럭 표
a=fmtp:{row.pt} {row.fmtp}                          ← ★row.fmtp 가 오면 반드시
a=rtcp-fb:{row.pt} {rtcp_fb}                        ← codecs_sub 가 오면 그것의 rtcp_fb · 없으면 codecs 의 것 · 되풀이 rtcp_fb
a=rtpmap:{row.rtx_pt} rtx/{clockrate}               ← rtx 짝이 왔을 때만(§8-1-2)
a=fmtp:{row.rtx_pt} apt={row.pt}                    ← rtx 일 때
a=extmap:{extmap.id} {extmap.uri}                   ← ★server_config.extmap 에서 그 m-line 종류의 것만 — sdes:mid 포함(§9-4-2 ③) · 되풀이 extmap
a=msid:{row.stream_id} ox-m{mid}                    ← ★줄마다 고정 — stream-id 는 전이중 = ox-m{mid} · 무전 = ox-ptt · track_id 를 넣지 않는다(재배정이 SDP 를 안 바꾼다)
a=ssrc:{row.ssrc} cname:{cname}                     ← cname = ox-sfu
a=ssrc:{row.rtx_ssrc} cname:{cname}                 ← rtx 짝이 왔을 때만(§8-1-2)
a=ssrc-group:FID {row.ssrc} {row.rtx_ssrc}          ← rtx 일 때
… m-line 공통 — 꼬리 …
```

#### 9-4-12. 데이터 채널 m-line

<a id="RULE-LINK-SDP-DC-MLINE"></a>**RULE-LINK-SDP-DC-MLINE** · 규칙 — 데이터 채널 m-line

offer 에 `m=application` 이 있으면 그것에 대응해 조립한다.


★**데이터 채널은 발행 연결에만 만든다.** 발언권이 그 위로 간다. ★`1pc` 면 연결이 하나뿐이므로 그것에 만든다.

→ 구조 — [`LINK.sdp.dc`](#LINK.sdp.dc)

<a id="LINK.sdp.dc"></a>SDP 줄 틀 — 데이터 채널 m-line(조립기가 이 틀로 짓는다 · 재료 → 줄은 이 규칙의 표)

```
m=application {ice.port} UDP/DTLS/SCTP webrtc-datachannel
… m-line 공통 — 머리 …
a=sendrecv
a=sctp-port:5000
a=max-message-size:65536
… m-line 공통 — 꼬리 …
```

#### 9-4-13. 데이터 채널을 여는 주체

<a id="RULE-LINK-SDP-DC-OPENER"></a>**RULE-LINK-SDP-DC-OPENER** · 규칙 — 데이터 채널을 여는 주체

★**클라가 연다.** 이름은 `"unreliable"` 하나다.

★★**청취 전용이어도 발행 연결을 세운다.** ★**트랙이 없어도 세운다**(track-less) —
★**데이터 채널이 그 위에 있고 발언권이 그리로 가기 때문이다.** 서버는 이것을 지원한다.

| | |
|---|---|
| 무엇을 협상하나 | ★**`m=application` 하나로 협상한다.** 미디어 m-line 이 없어도 된다 — 완성 예시는 §9-4-17 |
| 왜 필요한가 | ★**무전은 청취 전용이 절대다수**(§4-1-3)이고, 이들이 DC 를 못 가지면 `FLOOR_TAKEN`(2)·`FLOOR_IDLE`(5)·`FLOOR_GRANTED`(1)을 ★**하나도 못 받는다** |
| ★`1pc` | 연결이 하나뿐이므로 그것에 만든다. §9-2-1 의 첫 협상(2①)이 이 자리다 |

★★**미디어 정상 ≠ 제품 정상.** 청취 전용 참가자는 ★**미디어 축이 100% 정상인 채로 발언권 축이
0 인 상태가 성립한다** — `packetsReceived`·`framesDecoded`·`jitter`·ICE 상태 ★**어떤 미디어
지표로도 안 잡힌다.** 발언권·문자·진단은 ★**DC/WS 축**이므로 그 축의 지표로 봐야 한다.

| 항목 | 값 |
|---|---|
| 언제 | ★**발행 연결의 offer 를 만들기 전.** 만들어 둬야 offer 에 `m=application` 이 생긴다. ★`1pc` 도 같다 — 전송로를 먼저 세운다(§9-4-20) |
| 이름 | `"unreliable"` — ★다른 이름은 서버가 거부한다 |
| 순서 보장 | 끈다(`ordered: false`) |
| 재전송 | 하지 않는다(`maxRetransmits: 0`) |

★**증상**
- 안 열면 offer 에 `m=application` 이 없고, 그러면 ★**발언권 요청이 아무 데도 안 간다.**
  버튼을 눌러도 반응이 없다.
- ★★**청취 전용이라고 발행 연결을 안 세우면** → 소리는 정상으로 나는데 ★**누가 말하는지
  영영 안 뜬다.** §10-4-35 슬롯 매핑이 통째로 죽고, ★**미디어 계수기로는 안 잡힌다.**

#### 9-4-14. 완성 예시 — ★**참고(비규범)**

<a id="RULE-LINK-SDP-EXAMPLES-NONNORMATIVE"></a>**RULE-LINK-SDP-EXAMPLES-NONNORMATIVE** · 규칙 — 완성 예시 — ★**참고(비규범)**

★**이 절은 규범이 아니다.** 규범은 §9-4-8~§9-2-1·§9-4-18 의 **표(재료→SDP 매핑)와 불변**이고,
아래는 그것을 한 번에 보여 주는 **투영**이다 — 여기와 표가 어긋나면 ★**표가 이긴다.**
값은 예시이고, `{}` 없이 **그대로 나가는 모양**이다.

#### 9-4-15. 발행 answer (오디오 1 + 비디오 1 + 데이터 채널 — `1pc` 첫 협상 모양)

<a id="EX-LINK-SDP-PUB-ANSWER"></a>**EX-LINK-SDP-PUB-ANSWER** · 예시(비규범) — 발행 answer (오디오 1 + 비디오 1 + 데이터 채널 — `1pc` 첫 협상 모양)

브라우저 offer 가 `mid:0`(audio, opus PT 111) · `mid:1`(video, H264 PT 102 + rtx 103) · `mid:2`(application) 을 냈을 때 클라가 조립하는
**서버의 answer**(첫 협상 — 트랙이 아직 없어 둘 다 `inactive`, §9-2-1 2②). ★**데이터 채널이 마지막인 것은 브라우저
규칙이다** — RFC 8829 §5.2.1 은 data 의 m= section 을 미디어 뒤에 마지막으로 만든다. `createDataChannel` 을 먼저 불러도 순서는
안 바뀐다(§9-2-1 2①이 DC 를 먼저 여는 것과 무관하다). 그래서 BUNDLE 태그(첫 m-line)는 audio 절이 쥔다 — §9-4-18 무중단 ③의
*"첫 m-line 은 남긴다"* 가 그 위험을 막는다. ★뒤에 붙는 발행 m-line(카메라·화면공유·시뮬캐스트)은 합성 offer 로 온다(§9-4-19) — 이 예시엔 없다.


★**읽는 법** — `a=rtpmap`·`a=fmtp` 는 **offer 원문 그대로**이고(§9-4-10), 클라가 정한 것은
`a=rtcp-fb`(서버 정책) 와 `a=extmap` 걸러내기뿐이다. 첫 협상이라 두 미디어 절이 `a=inactive` 다(offer 가 `inactive` — §9-4-10 방향 행) — 번호표를
얻기 위한 협상이고 트랙은 아직 없다. 트랙이 붙으면 그 절은 합성 offer 에서 `a=recvonly`(**서버가 받는다**)가 된다(§9-4-19).

→ 구조 — [`LINK.sdp.EX-LINK-SDP-PUB-ANSWER`](#EX-LINK-SDP-PUB-ANSWER) · [`LINK.sdp.send`](#LINK.sdp.send) · [`LINK.sdp.dc`](#LINK.sdp.dc)

조립 결과 — 서버의 answer(보내기) · 조립기(gen/sdp.mjs)가 줄 틀로 짓는다

```
v=0
o=- 4611731400430051336 2 IN IP4 127.0.0.1
s=-
t=0 0
a=group:BUNDLE 0 1 2
a=msid-semantic: WMS *
a=ice-lite
m=audio 7000 UDP/TLS/RTP/SAVPF 111
c=IN IP4 203.0.113.10
a=ice-ufrag:pubUf1
a=ice-pwd:pubPw1
a=fingerprint:sha-256 AB:CD:EF:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D
a=setup:passive
a=mid:0
a=rtcp-mux
a=inactive
a=rtpmap:111 opus/48000/2
a=fmtp:111 minptime=10;useinbandfec=1
a=rtcp-fb:111 transport-cc
a=extmap:1 urn:ietf:params:rtp-hdrext:sdes:mid
a=extmap:4 urn:ietf:params:rtp-hdrext:ssrc-audio-level
a=extmap:6 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01
a=candidate:1 1 udp 2113937151 203.0.113.10 7000 typ host generation 0
a=end-of-candidates
m=video 7000 UDP/TLS/RTP/SAVPF 102 103
c=IN IP4 203.0.113.10
a=ice-ufrag:pubUf1
a=ice-pwd:pubPw1
a=fingerprint:sha-256 AB:CD:EF:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D
a=setup:passive
a=mid:1
a=rtcp-mux
a=rtcp-rsize
a=inactive
a=rtpmap:102 H264/90000
a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f
a=rtcp-fb:102 nack
a=rtcp-fb:102 nack pli
a=rtcp-fb:102 ccm fir
a=rtcp-fb:102 transport-cc
a=rtpmap:103 rtx/90000
a=fmtp:103 apt=102
a=extmap:1 urn:ietf:params:rtp-hdrext:sdes:mid
a=extmap:5 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time
a=extmap:6 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01
a=candidate:1 1 udp 2113937151 203.0.113.10 7000 typ host generation 0
a=end-of-candidates
m=application 7000 UDP/DTLS/SCTP webrtc-datachannel
c=IN IP4 203.0.113.10
a=ice-ufrag:pubUf1
a=ice-pwd:pubPw1
a=fingerprint:sha-256 AB:CD:EF:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D
a=setup:passive
a=mid:2
a=sendrecv
a=sctp-port:5000
a=max-message-size:65536
a=candidate:1 1 udp 2113937151 203.0.113.10 7000 typ host generation 0
a=end-of-candidates
```

#### 9-4-16. 구독 offer (일반 화자 1 + 무전 슬롯 1 + 빈 m-line 1)

<a id="EX-LINK-SDP-SUB-OFFER"></a>**EX-LINK-SDP-SUB-OFFER** · 예시(비규범) — 구독 offer (일반 화자 1 + 무전 슬롯 1 + 빈 m-line 1)

`tracks[]` 가 아래 셋일 때 클라가 조립하는 **서버의 offer**.



★**읽는 법**
- ★**입력은 배정표 하나다**(§8-1-2) — 트랙 목록을 보지 않는다. ★**`a=mid:` 가 줄의 `mid` 그대로**다. 내가 offer 를 쓰므로 내가 정한다(§9-4-5).
- ★**`sdes:mid`(id 1) 가 구독 m-line 에도 있다**(§9-4-11). 서버가 egress 에서 확장 값을 그 m-line 의 `mid` 로 새로 쓴다(§9-4-2 ③) — 발행자 mid 값은 오지 않는다.
- ★**`a=fmtp` 가 줄의 `fmtp` 원문 그대로**다(§8-1-1). H264 는 이 줄이 없으면
  브라우저가 baseline·`pm=0` 을 가정해 ★**패킷은 오는데 디코딩이 0** 이다.
- 무전 슬롯은 stream-id 가 ★`ox-ptt` 고정이고, 일반 화자는 `ox-{user_id}` 다 — 서버가 줄의 `stream_id` 로 싣는다.
- `mid:2` 는 ★빈 줄이다 — 다른 줄과 똑같이 지어진다(방향 · `a=msid` · `a=ssrc` 그대로). 그 줄로 오는 패킷이 없을 뿐이고, 서버가 같은 튜플의 트랙을 실어도 SDP 는 그대로다.
- 배정표는 연결마다 다르다(§8-1-2) — 이 표의 `mid`·`pt` 는 나에게만 다른 값이다.
- 이 offer 를 `setRemoteDescription` 에 넣으면 브라우저가 ★**구독 트랜시버 3개를 스스로 만든다.**

→ 구조 — [`LINK.sdp.EX-LINK-SDP-SUB-OFFER`](#EX-LINK-SDP-SUB-OFFER) · [`LINK.sdp.recv`](#LINK.sdp.recv)

배정표(입력 — §8-1-2 형과 대조된 값 · `sfu_id:"sfu-7f3a…"` · `arev:7`)

| `mid` | `kind` | `pt` | `codec` | ★`fmtp` | `track_id` | `ssrc` | `stream_id` | 비고 |
|---|---|---|---|---|---|---|---|---|
| `0` | audio | 111 | opus | `minptime=10;useinbandfec=1` | `t-u2-mic` | 2147484649 | `ox-m0` |  |
| `1` | video | 102 | H264 | `level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f` | `ptt-r1-video` | 2147485649 | `ox-ptt` | `rtx_pt:103` · `rtx_ssrc:2147485650` · 무전 슬롯 |
| `2` | video | 102 | H264 | `level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f` | — | 2147486649 | `ox-m2` | `rtx_pt:103` · `rtx_ssrc:2147486650` · ★빈 줄(`track_id` 없음) — 쉬는 트랙이 줄을 놓았다 · 다른 줄과 똑같이 짓는다(튜플 · 다운링크 SSRC · stream_id 가 줄의 것) · 서버가 같은 튜플의 트랙을 실어도 SDP 는 그대로다 |

조립 결과 — 서버의 offer(받기) · 조립기(gen/sdp.mjs)가 줄 틀로 짓는다

```
v=0
o=- 1756000000000 1 IN IP4 127.0.0.1
s=-
t=0 0
a=group:BUNDLE 0 1 2
a=msid-semantic: WMS *
a=ice-lite
m=audio 7000 UDP/TLS/RTP/SAVPF 111
c=IN IP4 203.0.113.10
a=ice-ufrag:subUf1
a=ice-pwd:subPw1
a=fingerprint:sha-256 AB:CD:EF:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D
a=setup:passive
a=mid:0
a=rtcp-mux
a=sendonly
a=rtpmap:111 opus/48000/2
a=fmtp:111 minptime=10;useinbandfec=1
a=rtcp-fb:111 transport-cc
a=extmap:1 urn:ietf:params:rtp-hdrext:sdes:mid
a=extmap:4 urn:ietf:params:rtp-hdrext:ssrc-audio-level
a=extmap:5 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time
a=extmap:6 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01
a=msid:ox-m0 ox-m0
a=ssrc:2147484649 cname:ox-sfu
a=candidate:1 1 udp 2113937151 203.0.113.10 7000 typ host generation 0
a=end-of-candidates
m=video 7000 UDP/TLS/RTP/SAVPF 102 103
c=IN IP4 203.0.113.10
a=ice-ufrag:subUf1
a=ice-pwd:subPw1
a=fingerprint:sha-256 AB:CD:EF:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D
a=setup:passive
a=mid:1
a=rtcp-mux
a=rtcp-rsize
a=sendonly
a=rtpmap:102 H264/90000
a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f
a=rtcp-fb:102 nack
a=rtcp-fb:102 nack pli
a=rtcp-fb:102 ccm fir
a=rtcp-fb:102 transport-cc
a=rtpmap:103 rtx/90000
a=fmtp:103 apt=102
a=extmap:1 urn:ietf:params:rtp-hdrext:sdes:mid
a=extmap:5 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time
a=extmap:6 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01
a=msid:ox-ptt ox-m1
a=ssrc:2147485649 cname:ox-sfu
a=ssrc:2147485650 cname:ox-sfu
a=ssrc-group:FID 2147485649 2147485650
a=candidate:1 1 udp 2113937151 203.0.113.10 7000 typ host generation 0
a=end-of-candidates
m=video 7000 UDP/TLS/RTP/SAVPF 102 103
c=IN IP4 203.0.113.10
a=ice-ufrag:subUf1
a=ice-pwd:subPw1
a=fingerprint:sha-256 AB:CD:EF:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D
a=setup:passive
a=mid:2
a=rtcp-mux
a=rtcp-rsize
a=sendonly
a=rtpmap:102 H264/90000
a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f
a=rtcp-fb:102 nack
a=rtcp-fb:102 nack pli
a=rtcp-fb:102 ccm fir
a=rtcp-fb:102 transport-cc
a=rtpmap:103 rtx/90000
a=fmtp:103 apt=102
a=extmap:1 urn:ietf:params:rtp-hdrext:sdes:mid
a=extmap:5 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time
a=extmap:6 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01
a=msid:ox-m2 ox-m2
a=ssrc:2147486649 cname:ox-sfu
a=ssrc:2147486650 cname:ox-sfu
a=ssrc-group:FID 2147486649 2147486650
a=candidate:1 1 udp 2113937151 203.0.113.10 7000 typ host generation 0
a=end-of-candidates
```

#### 9-4-17. ★청취 전용 발행 연결 (`m=application` 단독)

<a id="EX-LINK-SDP-LISTEN-ONLY"></a>**EX-LINK-SDP-LISTEN-ONLY** · 예시(비규범) — ★청취 전용 발행 연결 (`m=application` 단독)

★**트랙이 하나도 없는 발행 연결**이다. ★**청취 전용 참가자의 정상 경로**이고(§9-4-13),
★`1pc` 의 첫 협상(§9-2-1 2)은 여기에 audio · video `inactive` 둘을 더한다(아래).

브라우저가 데이터 채널만 열고 낸 offer 가 `mid:0`(application) 하나일 때 —





★**읽는 법**
- ★**미디어 m-line 이 하나도 없다.** 그래도 협상은 성립하고 ★**DTLS·SCTP 가 선다.**
- `a=group:BUNDLE 0` 에 그 하나만 들어간다. ★**빈 BUNDLE 은 만들지 않는다.**
- ★**이 연결이 서야 발언권이 산다.** 청취 전용은 여기까지가 전부이고, 받는 것은 구독 연결
  (`2pc`) 또는 같은 연결(`1pc`)이 따로 나른다.

★★**`1pc` 는 여기서 한 걸음 더 간다** — 이 협상을 할 때 ★**audio 1 + video 1 발행 트랜시버를
`inactive` 로 함께 세운다**(§9-2-1 2②). 번호표(§4-1)를 얻는 데 필요한 최소이고, 나중에 마이크·카메라를 켤 때 ★**가져올 코덱 확정본이
생긴다.** 청취 전용으로 끝나면 둘은 `inactive` 인 채로 남고, ★**비용은 m-line 둘뿐**이다. 번호는 구조(단일/시뮬캐스트)와 무관하다.

★★**자리 확보 트랜시버는 두지 않는다.** ★**새 발행 m-line 은 클라가 합성 offer 에 새 절로
붙인다**(§9-4-19) — 브라우저는 그 절에 트랜시버를 새로 만들고(RFC 8829 §5.10 — `addTrack` 으로 만든 것만 기존 절에 잇고, 없으면 새로 만든다),
클라가 `direction` 을 보내기로 돌려 `replaceTrack` 한다. ★**시뮬캐스트도 이 길로 열린다** — 합성 offer 가 `a=simulcast:recv l;h` 와
`a=rid:<id> recv` 를 선언하면 answer 가 `a=simulcast:send` 로 답하고 인코딩이 rid 마다 선다(실측 Chromium 149 · Firefox 155,
Chromium 은 두 층이 실제로 흘렀다, Firefox 의 RTP 단계는 미측정). ★**`a=rid` 의 방향은 `recv`** 다 — `send` 로 적으면
Chromium 은 조용히 단일로 떨어지고 Firefox 는 SDP 를 거부한다. ★**그래서 클라 축 mid 규칙이 필요하다** — 새 발행 m-line 의 `mid` 는
★**0~31 중 비어 있는 가장 작은 값**(십진 정수 문자열)이다. 구독 `mid` 가 32 부터라(§8-1-1) 겹치지 않고, 색인의 *"클라 m-line 32 미만"* 이 그
예산이다. 비게 된 절(`stop`)은 같은 kind·같은 구조의 다음 트랙에 되쓴다(§8-1-1 `mid` 행과 같은 규칙).

★**증상**
- 데이터 채널을 안 열고 이 협상을 하면 → offer 에 `m=application` 이 없어 ★**아무것도 안 선다.**
- 트랙이 없다고 이 연결을 아예 안 세우면 → §9-4-13 의 증상 그대로 ★**누가 말하는지 영영 안 뜬다.**
- 새 발행 m-line 의 `mid` 를 32 이상으로 짓거나 구독 `mid` 와 겹치게 하면 → 한 BUNDLE 안에서 `a=mid` 가 충돌해 ★**협상이 깨진다**(RFC 5888).
- 합성 offer 의 `a=rid` 를 `send` 로 적으면 → Chromium 은 ★**단일 층으로 조용히 떨어지고** Firefox 는 `setRemoteDescription` 을 거부한다.

→ 구조 — [`LINK.sdp.EX-LINK-SDP-LISTEN-ONLY`](#EX-LINK-SDP-LISTEN-ONLY) · [`LINK.sdp.dc`](#LINK.sdp.dc)

브라우저 offer(입력)

```
v=0
o=- 4611731400430051336 2 IN IP4 127.0.0.1
s=-
t=0 0
a=group:BUNDLE 0
a=msid-semantic: WMS *
m=application 9 UDP/DTLS/SCTP webrtc-datachannel
c=IN IP4 0.0.0.0
a=ice-ufrag:cliUf9
a=ice-pwd:cliPw9
a=fingerprint:sha-256 11:22:…
a=setup:actpass
a=mid:0
a=sctp-port:5000
a=max-message-size:262144
```

조립 결과 — 서버의 answer(보내기) · 조립기(gen/sdp.mjs)가 줄 틀로 짓는다

```
v=0
o=- 4611731400430051336 2 IN IP4 127.0.0.1
s=-
t=0 0
a=group:BUNDLE 0
a=msid-semantic: WMS *
a=ice-lite
m=application 7000 UDP/DTLS/SCTP webrtc-datachannel
c=IN IP4 203.0.113.10
a=ice-ufrag:pubUf1
a=ice-pwd:pubPw1
a=fingerprint:sha-256 AB:CD:EF:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D
a=setup:passive
a=mid:0
a=sendrecv
a=sctp-port:5000
a=max-message-size:65536
a=candidate:1 1 udp 2113937151 203.0.113.10 7000 typ host generation 0
a=end-of-candidates
```

#### 9-4-18. `1pc` — 두 축을 한 벌에 담는다

<a id="RULE-LINK-SDP-1PC"></a>**RULE-LINK-SDP-1PC** · 규칙 — `1pc` — 두 축을 한 벌에 담는다

`BIND` 의 `pc_mode:"1pc"`(§3-1) 일 때다. ★**세션 단위**라 ★**붙기 전에 정해지고**
세션 안에서 바뀌지 않는다 — ★**그 세션이 붙는 모든 미디어 서버가 같은 값**이고,
각 서버가 적용한 값이 `server_config.pc_mode`(§9-1-1) 로 되돌아온다.
연결이 하나이므로 ★**전송로도 하나**다 — 포트·ICE·DTLS·SCTP 를 한 벌만 쓴다.
★**그것이 목적이다** — 무전의 상시 keepalive 가 전송로 벌수에 비례한다(§9-4-5).

★**달라지는 것은 SDP 가 한 벌이라는 것뿐**이고, 거기서 규칙 둘이 나온다. 나머지(§9-4-8~9-4-13)는 그대로다.

| # | 무엇 | `1pc` 규격 |
|---|---|---|
| ★**1** | **협상은 언제나 서버 offer** | ★★**클라가 합성한 offer → 브라우저 answer.** 내 트랙 변경도 이 길이다 — §9-4-6 의 클라 offer 경로는 ★**첫 협상 한 번**(§9-2-1 2, 구독 m-line 이 아직 없을 때)에만 쓴다. 한 벌이라 그 offer 에 ★**발행 m-line 이 함께 실린다** — 채우는 법은 §9-4-19 |
| ★★**2** | **무중단 불변 — 상대 축은 확정본 그대로** | ★**이번 사건과 무관한 절은 한 글자도 바뀌지 않아야 한다.** 값을 새로 짓지 않는다 — PT·`rtpmap`·`fmtp` 는 ★**직전 협상이 확정한 것** 그대로, `extmap` 번호도 그대로, `rid`·`simulcast` 는 ★**방향만** 뒤집는다. 채우는 법은 §9-4-19 |

★**무중단은 넷이 함께 지킨다** — ①상대 축은 확정본 그대로(위) ②ICE 자격을 바꾸지 않는다(§9-4-9 — `ICE_RESTART` 때만 예외)
③m-line 을 지우거나 자리를 바꾸지 않는다(`inactive` + 포트 유지, §9-4-11 — ★**첫 m-line 은 무슨 일이 있어도 남긴다**: BUNDLE 태그가 첫 절에
걸리고, `1pc` 발행에서 그것은 audio 절이다(DC 는 마지막, §9-4-15). 지우면 태그가 옮겨가 전송이 깨진다) ④PT·`extmap` 번호를 다시
매기지 않는다(§9-4-10·§9-4-2). ★★**재협상 자체는 끊지 않는다** — 끊는 것은 언제나 이 넷 중 하나를 어긴 것이다.

★**mid 공간은 `1pc` 특례가 아니다** — 서버는 구독 mid 를 `32` 부터 발급하고 클라는 자기 m-line 을
32개 미만으로 유지한다(§8-1-1). `1pc` 은 내 mid(브라우저 배정)와 ★**한 공간을 쓰므로** 그 오프셋이 있어야 겹치지 않는다.

★**`1pc` 이 치르는 값 셋** — 규칙이 아니라 성질이다.

| 값 | 무엇 |
|---|---|
| ★**지연** | 협상 큐가 하나라(§9-2-1) pub 과 sub 이 서로 기다린다. `2pc` 는 큐가 둘이라 안 막는다 |
| ★**사고 반경** | 매번 SDP 한 벌 전체를 다시 내므로 ★**무관한 트랙까지 반경 안**이다. 규칙 2 가 그것을 막는다 |
| ★**공유 번호 공간** | ★**한 BUNDLE 이라 아래 표의 전부가 하나다.** `2pc` 에서 둘씩이라 겹칠 일이 없던 것들이 `1pc` 에서는 서로 비켜야 한다 — 그 여파가 SDP 층을 넘어 시그널링까지 간다(`ROOM_JOIN` 요청에 내 번호표를 싣는 이유다, §4-1) |

★**무엇이 하나가 되나 — 전량**

| 항목 | `2pc` | `1pc` | 비켜 주는 수단 |
|---|---|---|---|
| PT 공간 | 둘 | 하나 | 구독자 표 + `ROOM_JOIN` 번호표(§9-4-2 ④) |
| 확장 번호(`extmap`) | 둘 | 하나 | 확정본 번호 그대로(§9-4-19) + 서버가 egress 에서 다시 짓는다(§9-4-2 ②) |
| mid 공간 | 둘 | 하나 | 구독 오프셋 32(§8-1-1) |
| ★**SSRC 공간** | 둘 | 하나 | `PUBLISH_TRACKS` 에서 서버가 자기 발급분과 대조해 `3008` 로 거절(§7-1-2 — 아래) |
| BUNDLE 태그 | 둘 | 하나 | m-line 을 지우지 않는다(§9-4-11) |
| ICE 자격 · DTLS 세션 | 둘 | 하나 | `publish_*` 한 벌(§9-1-1) |
| 협상 큐 | 둘 | 하나 | 큐 하나(§9-2-1) — "지연" 의 자리 |
| 혼잡 제어 | 상·하행 분리 | 합쳐짐 | 없다 |

★★**SSRC** — RFC 8843 §9.1 대로 한 BUNDLE 의 RTP 미디어는 ★**한 RTP 세션**이고, 그래서 SSRC 도 한 공간이다. 다운링크 SSRC 는
서버가 만들고(§9-4-2 — §8-1-1 `ssrc` 행) 업링크 SSRC 는 브라우저가 만든다. ★서버가 만드는 값은 최상위 비트를 세운
난수(`0x80000000` 이상)이고 브라우저 값은 32비트 전역 난수라 ★**겹칠 수 있다** — 서버가 브라우저 값을 아는 시점은
`PUBLISH_TRACKS.ssrc`(§7-1-2)뿐이다. 확률은 한 연결에 SSRC 가 수백 개일 때 2 의 −32 승 급이지만 *"안 겹친다"* 가 아니라 *"거의 안 겹친다"* 이고,
겹치면 브라우저가 두 m-line 에서 같은 SSRC 를 보게 되어 ★**한쪽이 조용히 죽는다**(그 트랙의 `packetsReceived` 가 0 이다) — 기록만으로는
안 잡히는 사고다. 그래서 ★**서버가 `PUBLISH_TRACKS` 의 `ssrc`·`rtx_ssrc` 를 이 연결에 발급한 다운링크 SSRC 와 대조해 `3008` 로 거절한다**(§7-1-2). 판정은 집합 조회 하나이고 클라는 새 트랜시버로 한 번 재발행한다. 서버가 뒤에 발급하는 다운링크 SSRC 가 이미 신고된
발행 SSRC 와 겹치는 반대 방향은 서버가 발급 때 피한다(자기 연결의 발행 SSRC 집합을 안다).

★★**왜 브라우저에게 offer 를 시키지 않나.** 브라우저의 offer 는 ★**협상 전 목록**이라 할 수 있는
코덱이 전부 들어 있다. 그것으로 협상을 돌리면 ★**answer 가 걸러냈던 코덱과 번호가 되살아나고**,
`PUBLISH_TRACKS` 로 이미 서버에 보고한 합의가 조용히 갈린다 — 규칙 2(무중단 불변)를 그 자리에서
어긴다. 근거는 §9-4-19 의 "확정본에서 가져온다" 와 ★**같은 하나**다.

★**`2pc` 는 구독 PC 가 offer 를 낼 일이 없어** 이 자리가 아예 없다. `1pc` 만 한 벌이라 발행
사건이 구독 절까지 끌고 들어온다.

★**증상**
- ★**이번 사건과 무관한 절을 다시 쓰면** → 활성 트랙이 끊긴다. 그 트랙의 `packetsReceived` 가 멈추거나
  `freezeCount` 가 늘고, 코덱 줄이 바뀌었으면 ★**검은 화면**이다.

#### 9-4-19. 상대 축 m-line 채우기

<a id="RULE-LINK-SDP-1PC-FILL"></a>**RULE-LINK-SDP-1PC-FILL** · 규칙 — 상대 축 m-line 채우기

한 벌의 SDP 에 **발행 m-line 과 구독 m-line 이 같이** 들어간다. 방향은 언제나 ★**서버 시각**으로 적는다.

| m-line 갈래 | 무엇을 보고 짓나 | 방향 |
|---|---|---|
| **발행(내 트랙)** | ★**직전 협상 확정본**의 그 절 | ★**`recvonly`**(서버가 받는다) |
| **구독(남의 트랙)** | ★배정표(§8-1-2) | ★**`sendonly`**(서버가 보낸다) |
| **데이터 채널** | 확정본의 `m=application` | `sendrecv` |
| **비게 된 m-line** | 있던 자리 그대로 | ★`a=inactive`. **없애지 않는다** |
| ★**새 발행 m-line**(트랙 추가) | ★**클라가 새 절로 붙인다** — `mid` 는 0~31 중 비어 있는 가장 작은 값(§9-4-17) · 코덱 줄은 첫 협상 확정본의 그 kind 코덱 중 고른 것(PT·`rtpmap`·`fmtp` 는 확정본 번호 그대로, §9-4-10 코덱 선택) · `extmap` 은 확정본 번호 · 시뮬캐스트면 `a=rid:<id> recv` + `a=simulcast:recv l;h`(낮은 품질부터, §9-4-10) | ★**`recvonly`**(서버가 받는다). 브라우저가 새 트랜시버를 만든다(RFC 8829 §5.10) — 클라가 `direction` 을 보내기로 돌리고 `replaceTrack` |

★★**발행 m-line 은 "협상이 끝난 것"에서 가져온다.**

| 무엇 | 어디서 |
|---|---|
| **PT·`rtpmap`·`fmtp`** | ★★**직전 협상이 확정한 answer** — 협상 전의 offer 목록이 **아니다** |
| **`extmap`** | ★**확정본의 번호 그대로** |
| **`rid`·`simulcast`** | 확정본의 것을 ★**`recv` 로 뒤집는다** |

★**왜 확정본인가** — 브라우저의 offer 는 **협상 전 목록**이라 할 수 있는 코덱이 전부 들어 있다
(VP8·VP9·AV1 + H264 여러 프로파일). 그것을 그대로 되비추면 ★**answer 가 걸러냈던 코덱이 되살아나고**,
브라우저가 거기에 자기 answer 를 새로 써서 ★**`PUBLISH_TRACKS` 로 이미 서버에 보고한 합의를 조용히
갈아치운다.** 번호도 같다 — 새 `extmap` 번호를 제안하면 브라우저가 따라가 ★**와이어의 확장 헤더
번호가 조용히 바뀐다**(rid·mid·transport-cc 파싱이 죽는다, §9-4-10 와 같은 규율).

★**브라우저 offer 에 답하는 경로는 첫 협상(§9-2-1 2④) 한 번뿐이고 그때는 구독 m-line 이 아직 없다.** 규칙 1 아래
구독 m-line 이 딸린 브라우저 offer 는 존재하지 않는다 — 그런 offer 에 답하는 절차를 두지 않는다(실브라우저 실측에서 발행 · 구독 증설 둘 다 합성 offer 로 지나감을 확인했다).

**구독 m-line 의 `extmap`** — 한 BUNDLE 이라 ★**URI 마다 번호가 하나**여야 한다. ★**규칙은 둘이고 클라와 서버가 같은 계산을 한다** —
서버는 SDP 를 보지 않으므로 이 계산이 곧 계약이다. ①`ROOM_JOIN` 번호표(§4-1 — 브라우저 offer 의 확장 ID 표, 발행 절 전부의 합집합)에
그 URI 가 있으면 ★**그 번호**(`sdes:mid` 포함 — §9-4-11 가 선언한다) ②없으면 `server_config.extmap` 의 번호를 쓰되, 그 번호가 번호표에서
★**다른 URI 로 이미 쓰였으면** 번호표와 `server_config.extmap` 어디에도 없는 ★**가장 작은 번호**(1~14). 이 표는 번호표로 클라가 미리
계산할 수 있고, 서버도 같은 표로 송신 확장을 새로 쓴다(§9-4-2 ②). 두 문장이 다르면 그 확장은 구독자가 못 읽고 아무 오류도 없다.

★**확정본의 구독 표를 `READY{transport}` 로 신고하고 구독에서 `sdes:mid` 를 빼는 방식을 쓰지 않는다.** 발행 절을 합쳐
신고하면 그 `sdes:mid` 번호가 구독자 표에 들어가 서버가 발행자 mid 값을 그 번호로 옮겨 적고, 받는 쪽이 그것을 자기 발행
m-line 의 mid 로 읽어 SSRC 결속이 옮겨가 `Failed to apply demuxer criteria` 로 거부된다(실측 20260909). ★**다시 짓기**(발행자 확장
영역을 버리고 구독자 mid 를 새로 쓴다, §9-4-2)가 그 경로를 없애고, 신고는 브라우저 offer 표를 `ROOM_JOIN` 에 싣는다.

#### 9-4-20. 어느 모드로 붙나 — ★붙기 전에 정해진다

<a id="RULE-LINK-SDP-1PC-MODE"></a>**RULE-LINK-SDP-1PC-MODE** · 규칙 — 어느 모드로 붙나 — ★붙기 전에 정해진다

★★**`pc_mode` 는 세션 단위다.** `BIND`(§3-1)에 실려 확정되고 ★**세션 안에서 바뀌지 않는다.**

| | |
|---|---|
| 언제 정해지나 | ★**붙기 전.** `BIND` 요청이 값을 싣고 응답이 확정한다 |
| 어디까지 같나 | ★**그 세션이 붙는 모든 미디어 서버가 같은 값**이고, 각 서버가 적용한 값이 `server_config.pc_mode`(§9-1-1)로 되돌아온다. ★**둘이 다르면 규격 위반이고 클라는 연결을 세우지 않는다** |
| 바꾸려면 | ★**세션을 다시 맺는다** — 그 서버의 방에서 나갔다 다시 들어오는 값을 치른다(§4-1 축출·재생성) |
| ★무엇으로 고르나 | ★★**이 문서는 정하지 않는다.** 브라우저가 `1pc` 를 받는지는 ★**클라가 아는 것**이고, 모르면 기준선인 `2pc` 로 붙는다(§9-4-5) |

★**증상** — 세션 중에 모드를 바꾸려 들면 → `BIND` 확정값과 `server_config.pc_mode` 가 갈려
★**연결이 서지 않는다.** 모드를 바꾸는 유일한 길은 세션 재수립이다.

#### 9-4-21. 원천이 더한 것 — 출처 절이 없는 진술

<a id="RACE-LINK-STALE-SUSPECT"></a>**RACE-LINK-STALE-SUSPECT** · 알려진 경주

ICE 가 흔들리는 동안 서버가 보낸 `LINK_EVENT{suspect}` 가, ICE 가 스스로 회복한 뒤(ICE 재시작이 없어 `ufrag` 가 그대로다) 닿으면 세대 견주기를 통과해 멀쩡한 연결에 복구(WS 점검 → `ICE_RESTART`)를 다시 시작한다 — 세대는 ICE 재시작을 거친 복구만 가른다. 결과는 쓸데없는 ICE 재시작 한 번이다(DTLS · 구독 · 발행 · 발언권은 그대로라 상태는 깨지지 않는다)

*비고: 미디어 연결 모형 core 판 LH3 반례(ice_disc → suspect → ice_back → LINK_EVENT 도착). 모형은 이 경주로 시작한 복구를 검사 보조 byEvent 로 표시하고 그 밖의 길을 계속 본다 · CAND-LINK-STALE-SUSPECT-SAME-GEN*

### 9-5. 근거 · 기각 — 규격이 아니다

> 계약 본문(구조 · 동작 · 주제)의 규칙이 왜 그런가(R) · 무엇을 하지 않기로 했나(REJ). 소절 머리는 그 근거가 붙은 계약 자리다. 계약 판에는 싣지 않는다.

#### 9-5-1. §9-1-1 형 `ServerConfig` — 미디어를 붙이는 재료

<a id="R-LINK-SERVER-CONFIG-SYMPTOMS"></a>**R-LINK-SERVER-CONFIG-SYMPTOMS** · 근거

★**증상**
- 방마다 연결을 따로 만들면 → ★**같은 서버에 연결이 중복**되어 ICE 자격이 충돌하고
  (§9-4-9 은 같은 자격을 계속 쓰라고 한다) 브라우저가 **ICE 재시작으로 오인**한다.
- ★**주소를 신원으로 쓰면** → 미디어 서버가 재기동된 뒤에도 **옛 연결을 재사용**하려 한다.
  지문이 달라 DTLS 가 안 붙는데 ★**클라는 "같은 서버니 될 것" 이라고 알아** 원인을 못 찾는다.

#### 9-5-2. §9-1-4 `0x0305 ICE_RESTART` — 그 서버 연결을 다시 뚫는다   **C→S** · 응답 있음

<a id="R-LINK-ICE-RESTART-SYMPTOMS"></a>**R-LINK-ICE-RESTART-SYMPTOMS** · 근거

★**증상**
- 새 자격을 받고도 다시 협상하지 않으면 → 서버는 새 자격의 검사를 기다리고 브라우저는 옛 자격으로 `failed` 에 머문다. ★**되살아나지 않는다.**
- ★서버가 새 ufrag 를 새 연결로 다루면 → DTLS 를 처음부터 기다리는데 브라우저는 DTLS 를 이어 간다(지문 불변이면 같은 DTLS, RFC 8829). ★**검사는 통과하는데 미디어·DC 가 멎는다.**
- 옛 ufrag 를 곧바로 내리면 → 새 자격이 적용되기 전의 검사가 버려져 ★**아직 살아 있던 경로까지 끊긴다.**

#### 9-5-3. §9-1-5 `0x0706 LINK_EVENT` — 그 서버 연결이 흔들린다   **S→C** · ACK · ★**신설** · unicast

<a id="R-LINK-EVENT-WHY"></a>**R-LINK-EVENT-WHY** · 근거

★**왜 있나** — 경로의 생사는 ★**클라(ICE controlling)가 먼저 안다**(§9-2-1). 이 통지는 ★**다른 쪽 끝의 관찰**이라는 보강이다 —
플랫폼이 표준 상태를 늦게 올리는 경우처럼 클라가 못 알아챈 때의 방아쇠가 된다.
★**`ROOM_EVENT` 와 가르는 이유** — 방과의 관계가 아니라 ★**서버 하나의 연결 상태**다. 방마다 보내면 방 스물을 듣는 사람에게 같은 사실이 스무 번 간다.
★**`_EVENT` 인 이유** — 받은 클라가 재협상(ICE 재시작)을 한다. 구조를 고친다(각 장의 구조 이름 규율).

<a id="R-LINK-EVENT-SYMPTOMS"></a>**R-LINK-EVENT-SYMPTOMS** · 근거

★**증상**
- `ufrag`(세대)를 안 보면 → 복구가 막 끝난 뒤 늦게 도착한 통지가 ★**끝난 복구를 한 번 더 시작한다.**
- 이 통지로 복구를 곧바로 시작하면(방아쇠로 넣지 않고) → 진행 중인 복구와 ★**겹친다**(§9-2-1 — 연결마다 복구 주체는 하나).

#### 9-5-4. §9-2-1 클라 · `any`

<a id="R-LINK-RECOVERY-SYMPTOMS"></a>**R-LINK-RECOVERY-SYMPTOMS** · 근거

★**증상** — 이 절이 없으면 → 클라는 하트비트가 정상이라 `JOINED` 로 믿는데 그 서버 연결은 죽어 있다. ★**소리도 화면도
멎었는데 상태는 전부 정상**이고, 사용자는 앱을 껐다 켜야 돌아온다.
★**증상** — 흔들릴 때마다 재구축하면 → 발언권·구독·발행이 매번 리셋되고 ★**멀쩡한 방까지 끊긴다.** 되살리는 값(ICE
재시작 — DTLS·구독·발행 유지)보다 훨씬 비싸다.
★**증상** — 복구 주체가 둘이면(플래그로 막으면) → 확인을 빠뜨린 경로 하나로 복구가 겹치고, 같은 서버에 `ROOM_JOIN` 이
두 번 가 ★**첫 것이 두 번째에 축출된다**(§4-1).

<a id="R-LINK-RECOVERY-ONE-OWNER-WHY"></a>**R-LINK-RECOVERY-ONE-OWNER-WHY** · 근거

- 방 스물을 듣던 서버 하나가 흔들릴 때 복구가 스무 번 다시 시작되지 않는다.

#### 9-5-5. §9-4-2 ★서버가 egress 에서 다시 짓는다 — 구독자 표

<a id="R-LINK-DOWNLINK-SSRC-NAMES"></a>**R-LINK-DOWNLINK-SSRC-NAMES** · 근거

★옛 이름 *"가상 SSRC"*(시뮬캐스트) · *"슬롯 SSRC"*(무전)는 다운링크 SSRC 하나로 합쳤다 — 같은 성질에 한 이름. ★옛 꼴의 *"그 외엔 원본"*(전이중은 발행자 SSRC 를 그대로)은 걷어 냈다 — 재배정이 줄의 출처를 바꾸므로 SSRC 가 출처를 따라가면 SDP 가 바뀐다.

<a id="REJ-LINK-NO-SWAP"></a>**REJ-LINK-NO-SWAP** · 기각

★**바꿔치기를 하지 않는다** — 발행자 mid 확장의 ID 를 *구독자 표에 없는 번호로 바꿔치기*해 읽히지 않게 하고 구독에서 `sdes:mid` 를 빼는 방식은
서버가 구독자 SDP 의 선언을 전부 안다는 전제가 필요하고, `1pc` 에서는 구독자 자신의 발행 m-line 이 선언한 확장 ID 까지
알아야 해 깨진다(발행자 mid 값이 새어 받는 쪽 demuxer 결속이 옮겨가고 `Failed to apply demuxer criteria` 로 거부된다 — 실측 20260909).
다시 짓기의 근거는 관행이 아니라 ★**RFC 8843 §9.1(선언 MUST)과 RFC 8843 §12(ID 일관 MUST)** 다 — 확장 ID 를 구독자 기준으로 다시 쓰는 것은 mediasoup·LiveKit·Janus 가 같지만, subscribe 방향에 mid 확장 값을 싣는 것이 확인된 곳은 mediasoup 하나다(LiveKit `downtrack.go`·Janus `rtp.c` 는 안 싣는다 — 2차 자료). 그 자리가 원리적으로 없다.

<a id="R-LINK-REBUILD-SYMPTOMS"></a>**R-LINK-REBUILD-SYMPTOMS** · 근거

★**증상** — ①을 어겨 정책표로 PT 를 채우면 ★**조용히 검은 화면**이 되고, ②를 어겨 발행자 확장 영역을
그대로 흘리면 ★**구독자가 rid·twcc 를 자기 번호에서 못 읽어 대역 추정·음량 표시가 조용히 죽고**, 발행자 mid 값이
새면 ★**엉뚱한 m-line 에 붙는다.** ③을 어겨 mid 확장을 안 선언하면 RFC 8843 이탈이고, 확장 값을 구독자 mid 로 안 쓰면 ②와 같다.

#### 9-5-6. 통지 — 전부 **S→C**, 공통 의무는 ★**ACK 하나**

<a id="R-LINK-STALE-SUSPECT-ACCEPTED"></a>**R-LINK-STALE-SUSPECT-ACCEPTED** · 근거

같은 세대의 늦은 `LINK_EVENT{suspect}`(RACE-LINK-STALE-SUSPECT)는 받아들인다 — 결과는 쓸데없는 ICE 재시작 한 번이고 DTLS · 구독 · 발행 · 발언권은 그대로다. 막으려면 통지에 세대 밖의 순번이 필요해 서버 · SDK · 봇의 wire 가 바뀐다 — 값이 이득보다 크다

*없으면: 같은 세대 의심을 막으려 wire 를 늘린다*

#### 9-5-7. §9-4-5 무엇을 조립하는가 — ★연결마다 방향이 다르다

<a id="R-LINK-SDP-WHY-2PC"></a>**R-LINK-SDP-WHY-2PC** · 근거 — ★그런데 왜 다들 `2pc` 로 도나

WebRTC 의 offer/answer 는 ★**한 번에 한 방향만** 돈다(RFC 3264). 그런데 협상을 시작하는 사건은 둘이다.

| 사건 | 누가 시작하나 | 언제 |
|---|---|---|
| 내가 카메라·마이크를 켜고 껐다 | **클라** | 내 사정 |
| 남이 들어오고 나갔다 | **서버**(클라가 대신 조립) | 남의 사정 |

★**보통 SFU 는 여기서 `glare` 가 난다** — 서버가 제 판단으로 offer 를 밀어 넣으므로 내 offer 를 낸 채
서버 offer 가 도착하고, 푸는 길은 `rollback` 뿐이라 진행 중이던 내 변경이 날아간다.

★★**우리는 다르다 — 서버가 SDP 를 만들지 않는다.** 두 offer 를 다 클라가 시작하므로 ★**겹칠지 말지를
클라가 쥔다.** 큐로 하나씩 태우면 glare 는 나지 않는다(§9-2-1) — ★**`1pc` 이든 `2pc` 이든 같다.**

★**그래도 `2pc` 가 구조로 얻는 것이 있다.**

| 연결 | offer 는 항상 | 언제만 협상하나 |
|---|---|---|
| 발행 | ★**클라** | 내가 트랙을 더하거나 뺄 때 |
| 구독 | ★**서버** | 남이 들고날 때 |

★**사고 반경이 절반**이다 — pub 을 고칠 때 sub SDP 를 아예 열지 않는다 ·
★**큐가 둘**이라 두 축이 서로 안 막는다(§9-2-1) ·
ICE·DTLS 자격이 갈려 한쪽 경로가 죽어도 다른 쪽이 산다(§9-1-1 의 `publish_*`·`subscribe_*`) ·
상·하향 혼잡 제어가 분리된다.

★★**여기에 *"청취만 하는 참가자는 발행 연결을 아예 만들지 않는다"* 를 넣으면 안 된다.**
★**사실이 아니다** — 데이터 채널이 발행 연결 위에 있고(§9-4-12·§9-4-13) ★**발언권이 그 위로 간다.**
청취 전용도 ★**track-less 발행 연결을 세운다**(§9-4-13). 그래서 `2pc` 에서는 ★**청취자도
전송로 둘을 다 쓰고**, 상시 keepalive 가 전송로 벌수에 비례하므로 ★**`1pc` 논거가 오히려 강해진다.**

★그래서 SFU 들이 `1pc` 를 시도한 흔적은 있어도 **실제로는 죄다 `2pc` 로 돈다.**

<a id="R-LINK-SDP-WE-1PC"></a>**R-LINK-SDP-WE-1PC** · 근거 — ★우리는 그 값을 치르고 `1pc` 를 쓴다

`1pc` 은 ★**두 축을 한 SDP 에 담은 것**이다. 더 필요한 것은 ★**규칙 둘**뿐이다 —
★**협상은 언제나 서버 offer**(브라우저가 자기 offer 를 내면 구독 audio m-line 이 둘 이상일 때
demuxer 기준이 부딪힌다, §9-4-18) · ★**무중단 불변**(이번 사건과 무관한 절은 확정본 그대로).
규격은 ★**§9-4-18** 이다.

치르는 값은 셋이다 — ★**지연**(큐가 하나라 두 축이 서로 기다린다) · ★**사고 반경**(매번 SDP 한 벌
전체를 다시 낸다) · ★**공유 번호 공간**(PT·확장 번호·mid·SSRC·BUNDLE 태그·ICE·DTLS·큐·혼잡 제어가 한 벌 — §9-4-18 표).
★**옛날에 겪은 사고**(시뮬캐스트 업링크 스트림 소실·코덱 되살아남·확장 번호 바뀜)는
전부 그 둘째 값을 어긴 것이었다 — ★**무중단 불변이 그것을 막는다.** `ROOM_JOIN` 요청에 내 번호표를 싣는 이유(§4-1)가 셋째 값이다.

★**`1pc` 가 안 서면 `2pc` 로 간다.** 그래서 `2pc` 를 버리지 않는다 — 기준선으로 남긴다.
`pc_mode` 는 ★**세션 단위**라(§3-1) ★**붙기 전에 정하고**(§9-4-20) 세션 안에서 바뀌지 않는다.

★**증상**
- 구독에서 `createOffer` 를 부르면 → **m-line 0개**. 협상은 성공한 것처럼 보이는데 ★**아무것도 안 온다.**
- 이걸 *"내 SDP 를 만든다"* 로 읽고 `setup:active` 를 쓰면 → **양쪽이 다 능동이 되어 DTLS 가 안 붙는다.**

<a id="R-LINK-1PC-BUNDLE"></a>**R-LINK-1PC-BUNDLE** · 근거

무전은 종일 켜두는 물건이라 상시 비용이 지배한다. 무발화 구간에도 keepalive(STUN · DTLS · RTCP)가 전송로마다 흐르고, 1pc 는 그것을 한 벌만 흘리므로 전송로 유지 비용을 크게 줄인다.

*없으면: 2pc 로 상시 대기하면 무발화 중에도 1 USER 1.32 kbps(월 406 MB, 하한) 중 99% 가 keepalive 다*

<a id="FLR"></a>
