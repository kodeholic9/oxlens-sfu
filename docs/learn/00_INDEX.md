# OxLens 학습 문서 — 차례

> author: kodeholic (powered by Claude) · created: 2026-11-03.
> OxLens 를 처음 보는 사람이 서버와 웹 SDK 가 실제로 무엇을 주고받는지 따라가도록 쓴 문서다. 계약(무엇이 맞나)은 [클라 통신 규격](../protocol/README.md)이 정하고, 이 문서는 그 계약이 실제로 돌아가는 모양을 보여 준다.

## 이 묶음이 지키는 것

- ★**흐름과 구조만 쓴다** — 누가 누구를 어떤 차례로 부르나 · 무엇이 키인가 · 왜 그렇게 나뉘었나. ★**값과 계약은 쓰지 않고 규격 라벨로 가리킨다**(같은 사실을 두 곳에 적지 않는다). 규격과 다르면 규격이 맞다.
- ★**그림은 실측으로 그린다** — 장마다 탐침 시험을 한 번 돌려 거둔 WS 프레임 · SDP · SDK 전이 · 서버 로그의 실제 값과 차례를 쓴다. 지어낸 예가 없다.
- ★**장마다 「대조한 커밋」과 「실측 회차」를 머리에 적는다** — 소스가 바뀌면 그 장을 다시 돌려 맞댄다.
- 재현: 웹 SDK 저장소의 브라우저 탐침 하나(판정 없음) — `oxlens-sdk-web/qa/live/tests/probe/tour_capture.spec.ts`. 서버는 개발 형상(`oxlens-sfu/dev/` — `dev/gen_nodes.py` 가 짓는 node 설정)으로 띄운다 — 탐침이 그 형상의 로그 위치(`/tmp/oxlens/<node>/`)에서 서버 로그를 자른다. 산출은 SDK 저장소 안의 보관소(`.archive` 아래 `tour/tour-<시각>/tour.json` · 커밋하지 않는다)에 남는다.

```bash
cd oxlens-sdk-web/qa/live && npx playwright test --project=chromium-probe tests/probe/tour_capture.spec.ts
```

## 차례

| 장 | 사건 | 재료 |
|---|---|---|
| [01](01_JOIN_TO_VIDEO.md) | 큰 그림 · 접속 · 방 입장 · 발행 · 구독 · 영상이 보이기까지 · §3-2b 길이 둘일 때(UDP · TCP) | 3층 탐침 `TOUR-01` · `TOUR-01T` |
| [02](02_RTCP.md) | RTCP — 서버가 두 세션의 끝점 · PLI · NACK → RTX · RR · SR 번역 | 2층 봇 덤프(`adv_loss` · `adv_rtcp`) |
| [03](03_PTT_FLOOR.md) | 무전 — 누름 · 허가 · 줄 서기 · 넘겨받기 · 놓음 | 3층 탐침 `TOUR-03` |
| [04](04_MULTI_ROOM.md) | 다방 청취 · 발언 방 바꾸기 | 3층 탐침 `TOUR-04` |
| [05](05_RECOVERY.md) | 끊김과 복구 — 신호가 끊겼다 돌아온다(RESUME) | 3층 탐침 `TOUR-05` |
| [06](06_MULTI_NODE.md) | 다른 node 의 방 — hub 사이 oxmesh | 3층 탐침 `TOUR-06` |
| [07](07_SDK_INSIDE.md) | SDK 안쪽 — 생성 상태기 · 결합 · 집행 | 01 · 03 · 05 의 SDK 전이 |
| [08](08_FAILURES.md) | 실패와 거절 — 번호 하나에 뜻 하나 · hub 가 거르는 것과 sfud 가 판정하는 것 · 3001 과 5001 | 2층 봇 덤프(`adv_wire_reject` · `adv_authz` · `adv_resource` · `adv_node_down`) |
| [09](09_SIMULCAST_LAYER.md) | 시뮬캐스트 층 — 손 상한 · 자동 판단(대역 추정 · 프로브) · 키프레임에서 바뀐다 | 2층 봇 덤프(`sim_manual_layer` · `sim_bwe_updown`) |
| [10](10_STALL_RESYNC.md) | 전달 정체 — 서버가 언제 알리고 SDK 가 방을 다시 받는다(`sync_required` · `GET /rooms`) | 2층 봇 덤프(`adv_stall_resync` · `adv_stall_detect`) + 3층 탐침 `TOUR-10` |
| [11](11_TRACK_ATTRS.md) | 트랙 속성을 산 채로 — 음소거 · 영상 멈춤 · 회의↔무전 전환 · 마이크 전원 | 3층 탐침 `TOUR-11` + 2층 `conf_duplex` |
| [12](12_OXMESH.md) | oxmesh — node 사이 버스의 관리 구조 · 관리 항목(`/admin/bus` · 로그 줄 · 설정) · 실측(재기동 · 유닛 급사 · 방 소멸) | 서버 로그 세 node · 운영 표면 |


장마다 같은 틀 — 한 줄 목적 · 등장인물 · 구조도 · 콜 플로우(실측 시각) · 실제 프레임 · 로그 · 소스 좌표 · 규격 라벨 · 스스로 확인하는 질문.
