# 00. OxLens 학습 문서 — 차례 · 읽는 길 · 용어

> author: kodeholic (powered by Claude) · created: 2026-11-09.
> ★**대조한 커밋** — 서버 `oxlens-sfu` `c3642e8` · SDK `oxlens-sdk-web` `9f40557`(+탐침) · 2층 `oxlens-labs` `fc584ff` · oxpeer `93df1c6`
> ★**실측 회차** — 3층 탐침 `tour-learn-20261109` · 2층 회차 `e2e-20261109-070257`. 값은 돌릴 때마다 바뀌고 차례 · 모양은 같다.
> OxLens 를 처음 보는 사람이 서버 · 웹 SDK · 서버사이드 SDK 가 실제로 무엇을 주고받는지 따라가도록 쓴 문서다. 계약(무엇이 맞나)은 [클라 통신 규격](../protocol/README.md)이 정하고, 이 문서는 그 계약이 실제로 돌아가는 모양과 깨지는 모양을 보여 준다.

## 이 묶음이 지키는 것

- ★**값은 실측에서만** — 시각 · SSRC · id · 바이트 수는 위 두 회차(3층 탐침 · 2층 봇 덤프)와 서버 로그에서 거둔 것이다. 재지 않은 것은 「이 회차로 재지 않았다」로 밝힌다.
- ★**계약은 규격 라벨로 가리킨다** — `연:<라벨>` 은 공개 연동규격서의 머리다. `정:` · `SDK:` 라벨은 안쪽 정의서라 공개본에서 걷힌다. 규격과 이 문서가 다르면 규격이 맞다.
- ★**사실마다 집이 하나다** — 한 장이 풀어 쓴 것을 다른 장은 한 줄 + 링크로 가리킨다(아래 용어 표의 「집」).
- ★**정상 길만 쓰지 않는다** — 장마다 같은 장 안에서 고장 두세 갈래를 실측으로 그리고, 다른 SFU 와 견준다. 다른 SFU 칸은 ★저쪽 소스를 정적으로 읽은 기록이다 — 실측이 아니다.

## 읽는 길 둘

| 길 | 차례 | 얼마나 |
|---|---|---|
| **처음 읽기** | [01](01_OVERVIEW.md) 전부 → 02 ~ 17 의 「1. 30초 요약」만 | 한 장에 30초 · 그림 하나와 세 줄 |
| **깊이 읽기** | 01 → 02 → … → 17 차례대로, 장마다 아홉 칸 전부 | 부를 건너뛰어도 된다 — 각 부는 앞 부의 「1. 30초 요약」만 알면 읽힌다 |

막혔을 때는 [01 §4](01_OVERVIEW.md) 의 고장 지도에서 증상으로 장을 찾는다.

## 차례 — 다섯 부

| 부 | 장 | 무엇 | 재료(3층 탐침 · 2층 시나리오) |
|---|---|---|---|
| 0 지도 | [00](00_INDEX.md) | 차례 · 읽는 길 · 용어 · 재현 · 도구 | — |
| | [01](01_OVERVIEW.md) | 한눈에 — 구성요소 · 선 셋 · 접속 한 번의 일생 · 고장 지도 · 우리만의 것 · 다른 SFU 와 큰 차이 | 02 ~ 17 이 인용한 값(새로 재지 않았다) |
| 1 회의 미디어 | [02](02_CONNECT_JOIN.md) | 접속과 입장 — 토큰 · `BIND` · `ROOM_JOIN` · 서버가 SDP 를 안 만든다 · 1pc 와 2pc · UDP 와 TCP | `TOUR-02` · `02P` · `02T` · `adv_wire_reject` |
| | [03](03_PUBLISH_SUBSCRIBE.md) | 발행과 구독 — 협상 먼저 · 배정표 · `arev` · `READY` · 방 목록 `seq` · 줄 상한과 재배정 | `TOUR-02` · `02P` · `peer_record_reassign` · `adv_wire_reject` · `adv_resource` · 연기 시험 `ops_history` |
| | [04](04_PACKET_PATH.md) | 서버 안의 패킷 길 — 수신 스레드 · 미디어 워커 · 재기록기 · RTCP 종단 | `TOUR-02` · `adv_loss` · `adv_rtcp` · `onepc_adv_loss` · `csrc_adv_loss` · `ptt_stale_nack` · `peer_agent_loss` |
| | [05](05_SIMULCAST_BWE.md) | 시뮬캐스트 층 — 손 상한 · 서버의 자동 판단 · 키프레임에서 바뀐다 | `sim_manual_layer` · `sim_bwe_updown` · `onepc_sim_bwe_updown` · `sim_bwe_nack` · `sim_bwe_probe_reject` · `conf_simulcast_repub` |
| | [06](06_TRACK_ATTRS.md) | 트랙 속성 — 음소거 · 영상 멈춤 · 트랙 단위 duplex(회의↔무전) | `TOUR-06` · `conf_duplex` · `conf_simulcast` · `adv_wire_reject` |
| 2 무전 | [07](07_FLOOR.md) | 발언권 — MBCP over DC · 누름 · 허가 · 줄 · 넘겨받기 · 놓음 · 선점 · 회수 | `TOUR-07` · `ptt_preempt` · `adv_floor_failover` · `adv_floor_multi_dest` |
| | [08](08_SLOT_SPEAKER.md) | 슬롯 · 화자 — 줄 하나로 여러 화자 · 출처 표지(CSRC) · 다방 청취 · 발언 방 바꾸기 | `TOUR-08` · `ptt_voice_seam` · `csrc_ptt_av` · `csrc_conf_av` · `ptt_stale_nack` · `ptt_multiroom` · `ptt_scope_relay` |
| | [09](09_MIC_COST.md) | 마이크 전원 · 상시 대기 비용 1pc 대 2pc | `TOUR-09` · `TOUR-06` 끝 부분 |
| 3 고장과 복구 | [10](10_RECOVERY.md) | 끊김과 복구 — 사다리 일곱 칸(신호만 — 브라우저 · 서버사이드 SDK · 얼림 · 재개 창 밖 · 운영 절단 · 유닛 죽음 · 무발화) | `TOUR-10` · `peer_recorder_*` · `peer_agent_*` · `adv_idle_survives` |
| | [11](11_STALL_RESYNC.md) | 전달 정체 — `sync_required` · 방 다시 받기 | `TOUR-11` · `adv_stall_resync` · `adv_stall_detect` |
| | [12](12_ERRORS.md) | 실패 코드 — 번호 하나에 뜻 하나 · 첫 숫자가 할 일 | `adv_wire_reject` · `adv_authz` · `adv_resource` · `adv_node_down` · `peer_recorder_unit_down` |
| 4 여러 node | [13](13_MULTI_NODE.md) | 다른 node 의 방 — 요청 · 통지 · 미디어 길 · 분산 선택(방 한 node · HRW · 중계 없음) | `TOUR-13` · `peer_recorder_unit_down` · `adv_node_down` · 실증 `bus_node_down` |
| | [14](14_OXMESH.md) | oxmesh — 그물 · 선언 · 레인 · 고립 · 울타리 · zenoh 를 갈아 끼운 까닭 | 세 node 서버 로그 · `/admin/bus` · 실증 `bus_decisions` · `adv_room_reclaim` |
| 5 짓는 법과 운영 | [15](15_SDK_INSIDE.md) | SDK 안쪽 — 생성 상태기 · 결합 · 집행 · 이음매 | `TOUR-02` · `07` · `10` · SDK 1층 |
| | [16](16_OXPEER.md) | 서버사이드 SDK oxpeer — 녹음 · 녹화 · 에이전트가 같은 계약으로 | 2층 oxpeer 시나리오 24 · 3층 `peer_mixed` |
| | [17](17_OPERATIONS.md) | 운영자가 보는 것 — 로그 줄 · 통화 번호 · 사건 이력 · 판정기 · 깨짐 일곱 | `TOUR-10` · `11` · `13` · `adv_node_down` · `peer_recorder_unit_down` · `ptt_preempt` · `peer_recorder_freeze_long` · `adv_wire_reject` · `adv_loss` · 사건 이력 |

## 용어 — 한 표

| 용어 | 뜻 | 집 |
|---|---|---|
| 트랙 | 발행자가 올린 소리 · 영상 하나. 서버가 `track_id` 로 이름 짓는다 | [03](03_PUBLISH_SUBSCRIBE.md) |
| 발행 · 구독 | 내 트랙을 서버로 올림 · 남의 트랙을 서버에서 받음 | [03](03_PUBLISH_SUBSCRIBE.md) |
| 스트림(업링크 · 다운링크) | 한 SSRC 의 패킷 흐름. 발행자 → 서버가 업링크, 서버 → 구독자가 다운링크 | [04](04_PACKET_PATH.md) |
| 레이어(층) | 시뮬캐스트 발행의 해상도 갈래 — 이 판은 h · l 둘 | [05](05_SIMULCAST_BWE.md) |
| 줄(m-line) | SDP 의 `m=` 절 하나. 받는 줄은 서버가 정한 mid · 다운링크 SSRC 를 갖는다 | [03](03_PUBLISH_SUBSCRIBE.md) |
| 배정표 · `arev` | 서버가 고른 「받는 줄 mid → 트랙」 표와 그 차례 번호 — 연결마다 하나 | [03](03_PUBLISH_SUBSCRIBE.md) |
| `READY` | 「이 `arev` 의 표로 협상을 끝냈다」 확인. 서버는 확인된 줄로만 보낸다 | [03](03_PUBLISH_SUBSCRIBE.md) |
| 방 목록 `seq` | 방의 사람 · 트랙 목록 차례(`version` 의 `epoch` · `seq`) — 방마다 하나 | [03](03_PUBLISH_SUBSCRIBE.md) |
| 번호표 | 1pc 가 `ROOM_JOIN` 에 싣는 브라우저의 PT · 확장 번호 표 | [02](02_CONNECT_JOIN.md) |
| 1pc · 2pc | 연결(RTCPeerConnection) 하나에 보내기 · 받기를 함께 / 발행 · 구독 연결 둘. `BIND` 에서 정하는 세션 값 | [02](02_CONNECT_JOIN.md) |
| 재기록기 | 줄마다 seq · ts 오프셋 하나를 들고 출처가 바뀔 때만 다시 세는 서버 부품 | [04](04_PACKET_PATH.md) |
| 미디어 워커 · 전송 객체 | 수신 스레드 하나가 받은 패킷을 넘기는 일꾼 · 연결 하나의 서버 쪽 몫(워커 하나에 고정) | [04](04_PACKET_PATH.md) |
| duplex(반이중 · 전이중) | 트랙의 속성 — 무전 슬롯으로 흐르나(half) · 제 줄로 흐르나(full) | [06](06_TRACK_ATTRS.md) |
| 발언권 · MBCP | 무전에서 한 번에 한 사람만 말하게 하는 서버 판정 · 그 메시지(3GPP TS 24.380) — DC 로 오간다 | [07](07_FLOOR.md) |
| 슬롯 | 무전 방의 오디오 · 비디오 줄 하나씩 — 화자가 바뀌어도 SSRC · mid 가 그대로 | [08](08_SLOT_SPEAKER.md) |
| 출처 표지(CSRC · `source_tag`) | 받는 연결이 켜면 다운링크 패킷마다 실리는 발행 출처 번호 | [08](08_SLOT_SPEAKER.md) |
| 다방 청취 · 말할 방 | 여러 방을 듣고(`sub_rooms`) 말할 방 하나를 고른다(`pub_room` · `AFFILIATION`) | [08](08_SLOT_SPEAKER.md) |
| 마이크 전원(hot · hot_standby · cold) | SDK 혼자 정하는 장치 상태 — 서버는 모른다 | [09](09_MIC_COST.md) |
| 재개 창 · `RESUME` | 소켓이 끊겨도 세션을 쥐는 시간(60초) · 그 안에서 지금 상태를 다시 받는 요청 | [10](10_RECOVERY.md) |
| 재구축 | 세션 · 연결을 버리고 `ROOM_JOIN` 부터 다시 짓기 | [10](10_RECOVERY.md) |
| 전달 정체 · `sync_required` | 흘릴 자격이 있는데 안 흐름 · 그때 구독자에게 가는 「방을 다시 받아라」 | [11](11_STALL_RESYNC.md) |
| 실패 번호 · 첫 숫자 | `{code, name, permanent}` · 첫 숫자가 할 일(1 클라 버그 ~ 5 서버 사정) | [12](12_ERRORS.md) |
| hub(`oxhubd`) | 클라가 붙는 관문 — 세션 · 방 위치 표 · oxmesh router | [02](02_CONNECT_JOIN.md) · [13](13_MULTI_NODE.md) |
| sfud(`oxsfud`) · 미디어 서버 | 방 · 발행 · 배정표 · 전달 · 발언권을 드는 프로세스. 기동 신원 `sfu_id` = `epoch` | [04](04_PACKET_PATH.md) · [13](13_MULTI_NODE.md) |
| node | hub 하나 + sfud 하나의 묶음 — 방은 한 node 에만 산다 | [13](13_MULTI_NODE.md) |
| 유닛 | hub 가 띄우고 돌보는 sfud 프로세스(`SV-STATE-UNIT-STATE`) | [14](14_OXMESH.md) |
| HRW | 방 이름의 순수 함수로 node 를 고르는 배치 | [13](13_MULTI_NODE.md) |
| oxmesh | node 사이 버스 — router 그물 · 선언(토큰) · 레인 · 고립 · 울타리 | [14](14_OXMESH.md) |
| 방 토큰 · node 토큰 | 주체 세션에 묶인 선언 키 — 주체가 죽으면 저절로 사라진다. 방 토큰이 곧 방 → node 표 | [14](14_OXMESH.md) |
| 생성 상태기 · 전이 라벨 | 규격 원천에서 지은 판단 함수 · 그 걸음 이름(`SUB.C.WANT.nego_ok` 꼴) | [15](15_SDK_INSIDE.md) |
| 이음매 | 한 영역의 걸음이 다른 영역의 입구를 부르는 지점(`trk.delta` 꼴) | [15](15_SDK_INSIDE.md) |
| oxpeer · 투명 참가자 | 서버사이드 SDK · 토큰 클레임 `hidden` 이 참인 참가자(명단에 안 보임) | [16](16_OXPEER.md) |
| 세션 지문(통화 번호) | `SHA-256(session_id)` 앞 8자 — 서버 로그 `session=` 과 같은 값 | [17](17_OPERATIONS.md) |
| 로그 줄 `ev` · `diag` · `stat` | 규격 이름 있는 상태 변화 · 판단 근거 · 5초 계수 | [17](17_OPERATIONS.md) |

## 틀 — 장마다 아홉 칸

| 칸 | 무엇 | 읽는 법 |
|---|---|---|
| 1. 30초 요약 | 그림 하나 + 세 줄 | 바쁘면 이것만 |
| 2. 등장인물 · 값 | 이 회차의 사람 · 방 · SSRC · 잰 값 표 | 아래 칸의 숫자가 다 여기서 나온다 |
| 3. 정상 길 | 실측 시각을 단 콜 플로우 · 실제 프레임 · 로그 발췌 · 해설 | 시각은 대개 초 아래 ms 만 적는다 |
| 4. 깨지는 길 | 같은 장의 고장 두세 갈래 — 무엇이 깨졌나 → 누가 먼저 아나 → 무엇이 오가나 → 사용자는 무엇을 겪나 | 「관찰」 · 「결함 의심」은 판정 전 기록이다 |
| 5. 왜 이 모양인가 | 버린 길과 그 이유 · `★우리만의 것` 상자(선택 배경 · 얻은 것 · 잃은 것) | 잃은 것도 숫자로 적는다 |
| 6. 다른 SFU 는 | LiveKit · mediasoup · Janus · JVB · 무전 쪽과 견준 표 | 저쪽 칸은 소스를 정적으로 읽은 것 — 실측 아님 |
| 7. 운영에서 보는 법 | 이 장의 사건이 찍히는 로그 줄 · 운영 표면 · SDK 전이 라벨 | 줄 읽는 법은 [17](17_OPERATIONS.md) |
| 8. 소스 · 규격 좌표 | 서버 · SDK 함수 이름과 규격 라벨 | 지금 소스에 있는 이름만 |
| 9. 스스로 확인하는 질문 | 접힌 질문 셋 이상 — 깨지는 길에서 하나 이상 | 답을 열기 전에 장을 다시 짚는다 |

## 재현 — 다시 재 보기

3층 탐침은 판정하지 않는다 — 장의 재료(WS 프레임 · SDP · ICE · DC 바이트 · SDK 전이 · 서버 표면 · 서버 로그)를 탐침 하나마다 파일 하나로 거둔다. 탐침 이름이 장 번호다(`TOUR-02` · `02P` · `02T` · `06` · `07` · `08` · `09` · `10` · `11` · `13`). 서버는 개발 형상(`oxlens-sfu/dev/` — 한 기계 위 세 node)으로 띄워 둔다.

```bash
cd oxlens-sdk-web/qa/live && npx playwright test --project=chromium-probe tests/probe/tour_capture.spec.ts          # 전량
cd oxlens-sdk-web/qa/live && npx playwright test --project=chromium-probe tests/probe/tour_capture.spec.ts -g "TOUR-07"  # 하나(-g 는 제목 안 글자 일치라 "TOUR-02" 는 02P · 02T 도 고른다)
```

2층은 2층 하네스 `oxe2epy` 가 서버를 짓고 띄워 헤드리스 봇으로 시나리오 전량을 돈다 — 봇이 보내고 받은 WS · RTP · RTCP 를 바이트째 덤프로 남긴다.

```bash
cd oxlens-labs && python -m oxe2epy run-all
```

회차 파일은 저장소의 보관소에 남고 커밋하지 않는다. 보기 · 모양 견주기 도구는 안쪽 저장소에 있다.
