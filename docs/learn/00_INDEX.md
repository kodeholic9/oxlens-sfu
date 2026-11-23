# 00. OxLens 학습 문서 — 차례 · 읽는 길 · 용어

> author: kodeholic (powered by Claude) · created: 2026-11-09.
> ★**대조한 커밋** — 서버 `oxlens-sfu` `1ef7265` · SDK `oxlens-sdk-web` `de8c4a9`(+탐침) · 2층 `oxlens-labs` `a68d2de` · oxpeer `740f172`
> ★**실측 회차** — 3층 탐침 `tour-learn` 20261109 둘째 판 · 2층 회차 `e2e-20261109-132317`. 값은 돌릴 때마다 바뀌고 차례 · 모양은 같다.
> OxLens 를 처음 보는 사람이 서버 · 웹 SDK · 서버사이드 SDK 가 실제로 무엇을 주고받는지 따라가도록 쓴 문서다. 계약(무엇이 맞나)은 [클라 통신 규격](../protocol/README.md)이 정한다. 이 문서는 그 계약이 실제로 돌아가는 모양과 깨지는 모양을 보여 준다.

## 이 묶음이 지키는 것

- **값은 실측에서만 가져온다.** 시각 · SSRC · id · 바이트 수는 위 두 회차(3층 탐침 · 2층 봇 덤프)와 서버 로그에서 거뒀다. 재지 않은 것은 「이 회차로 재지 않았다」고 밝힌다.
- **계약은 규격 라벨로 가리킨다.** `연:<라벨>` 은 공개 연동규격서의 머리다. 규격과 이 문서가 다르면 규격이 맞다.
- **표준 용어를 쓴다.** RFC · W3C · 3GPP 에 이름이 있으면 그 이름을 쓴다. 우리 와이어 이름은 백틱으로 그대로 둔다. 대응은 아래 [용어](#용어) 절에 있다.
- **사실마다 집이 하나다.** 한 장이 풀어 쓴 것을 다른 장은 한 줄과 링크로 가리킨다. 집은 [우리 설계 용어](#우리-설계-용어) 표의 「집」 칸이다.
- **정상 길만 쓰지 않는다.** 장마다 고장 두세 갈래를 실측으로 그린다. 다른 SFU 와도 견준다. 다른 SFU 칸은 저쪽 소스를 정적으로 읽은 기록이고, 실측이 아니다.

## 읽는 길 둘

| 길 | 차례 | 얼마나 |
|---|---|---|
| **처음 읽기** | [01](01_OVERVIEW.md) 전부 → 02 ~ 21 의 「1. 30초 요약」만 | 한 장에 30초. 그림 하나와 세 줄 |
| **깊이 읽기** | 01 → 02 → … → 21 차례대로, 장마다 아홉 칸 전부 | 부를 건너뛰어도 된다. 각 부는 앞 부의 「1. 30초 요약」만 알면 읽힌다 |

막혔을 때는 [01 §4](01_OVERVIEW.md) 의 고장 지도에서 증상으로 장을 찾는다. 낯선 말은 [용어](#용어) 절에서 찾는다.

## 차례 — 여섯 부

| 부 | 장 | 무엇 | 재료(3층 탐침 · 2층 시나리오) |
|---|---|---|---|
| 0 지도 | [00](00_INDEX.md) | 차례 · 읽는 길 · 용어 · 재현 · 도구 | — |
| | [01](01_OVERVIEW.md) | 한눈에 — 구성요소 · 선 셋 · 접속 한 번의 일생 · 고장 지도 · 우리만의 것 · 다른 SFU 와 큰 차이 | `TOUR-02` 와 02 ~ 17 이 쓴 회차 |
| 1 회의 미디어 | [02](02_CONNECT_JOIN.md) | 접속과 입장 — 토큰 · `BIND` · `ROOM_JOIN` · 서버가 SDP 를 안 만든다 · 1pc 와 2pc · UDP 와 TCP | `TOUR-02` · `02P` · `02T` · `adv_wire_reject` |
| | [03](03_PUBLISH_SUBSCRIBE.md) | 발행과 구독 — 협상 먼저 · `mline_table`(m-line 표) · `arev` · `READY` · 방 목록 `seq` · m-line 상한과 재배정 | `TOUR-02` · `02P` · `peer_record_reassign` · `adv_wire_reject` · `adv_resource` · 연기 시험 `ops_history` |
| | [04](04_PACKET_PATH.md) | 서버 안의 패킷 길 — 수신 스레드 · 미디어 워커 · RTP 헤더 재작성 · RTCP 종단(PLI · NACK · RTX · RR · SR) | `TOUR-02` · `adv_loss` · `adv_rtcp` · `onepc_adv_loss` · `csrc_adv_loss` · `ptt_stale_nack` · `peer_agent_loss` |
| | [05](05_SIMULCAST_BWE.md) | 시뮬캐스트 레이어 — 손 상한 · BWE(TWCC · GCC) · 키프레임에서 바뀐다 | `sim_manual_layer` · `sim_bwe_updown` · `onepc_sim_bwe_updown` · `sim_bwe_nack` · `sim_bwe_probe_reject` · `conf_simulcast_repub` |
| | [06](06_TRACK_ATTRS.md) | 트랙 속성 — 음소거 · 영상 멈춤 · 트랙 단위 duplex(full ↔ half) | `TOUR-06` · `conf_duplex` · `conf_simulcast` · `adv_wire_reject` |
| 2 무전 | [07](07_FLOOR.md) | floor control(발언권 제어) — Floor Request · Granted · 대기열 · 승계 · Release · 선점 · Revoke | `TOUR-07` · `ptt_preempt` · `adv_floor_failover` · `adv_floor_multi_dest` |
| | [08](08_SLOT_SPEAKER.md) | PTT 슬롯 · 화자 — m-line 하나로 여러 화자 · CSRC · 다방 청취 · 발언 방 바꾸기 | `TOUR-08` · `ptt_voice_seam` · `csrc_ptt_av` · `csrc_conf_av` · `ptt_stale_nack` · `ptt_multiroom` · `ptt_scope_relay` |
| | [09](09_MIC_COST.md) | 마이크 전원 · 상시 대기 비용 1pc 대 2pc | `TOUR-09` · `TOUR-06` 끝 부분 |
| 3 고장과 복구 | [10](10_RECOVERY.md) | 끊김과 복구 — 세션 재개(`RESUME`) · ICE 재시작 · 재연결(rebuild) · 유닛 죽음 · 무발화 | `TOUR-10` · `peer_recorder_*` · `peer_agent_*` · `adv_idle_survives` |
| | [11](11_STALL_RESYNC.md) | 미디어 정체(stall) — `sync_required` · 방 다시 받기 | `TOUR-11` · `adv_stall_resync` · `adv_stall_detect` |
| | [12](12_ERRORS.md) | 실패 코드 — 번호 하나에 뜻 하나 · 첫 숫자가 할 일 · 거절마다 `CP-CODE` 줄 | `adv_wire_reject` · `adv_authz` · `adv_resource` · `adv_node_down` · `peer_recorder_unit_down` |
| 4 여러 node | [13](13_MULTI_NODE.md) | 다른 node 의 방 — 요청 · 통지 · 미디어 길 · 분산 선택(방 한 node · rendezvous hashing(HRW) · 서버 간 미디어 중계 없음) | `TOUR-13` · `peer_recorder_unit_down` · `adv_node_down` · 실증 `bus_node_down` |
| | [14](14_OXMESH.md) | oxmesh — 풀 메시 · liveliness token 선언 · 우선순위 큐(lane) · 네트워크 분할 · fencing · zenoh 를 갈아 끼운 까닭 | 세 node 서버 로그 · `/admin/bus` · 실증 `bus_decisions` · `adv_room_reclaim` |
| 5 짓는 법과 운영 | [15](15_SDK_INSIDE.md) | SDK 안쪽 — 생성 상태기 · 결합 · 집행 · 이음매 | `TOUR-02` · `07` · `10` · SDK 1층 |
| | [16](16_OXPEER.md) | 서버사이드 SDK oxpeer — 녹음 · 녹화 · 에이전트가 같은 계약으로 | 2층 oxpeer 시나리오 24 · 3층 `peer_mixed` |
| | [17](17_OPERATIONS.md) | 운영자가 보는 것 — 로그 줄 · 세션 지문 · 이벤트 이력 · 이력 판정기 · 깨짐 일곱 | `TOUR-10` · `11` · `13` · `adv_node_down` · `peer_recorder_unit_down` · `ptt_preempt` · `peer_recorder_freeze_long` · `adv_wire_reject` · `adv_loss` · 이벤트 이력 |
| | [18](18_RECORDING.md) | 녹음 · 녹화 파이프라인 — 원시 기록(MCAP) · 레코드 흐름 · 파이프라인 · Ogg · Matroska · HLS | 2층 `peer_record_*`(HLS 판 포함) · 3층 `peer_hls` |
| | [19](19_AGENT_BRIDGE.md) | AI 에이전트 브리지 — 턴은 발언권이 정한다 · WS 이음매(JSON 사건 · PCM) · reply 와 Floor Request · 무음 상한(`T1` 거울) · 선점 · 느린 AI · 끊김 | 2층 `bridge_*` 열하나 · 3층 `PEER-BRIDGE-01` |
| 6 채팅 | [20](20_CHAT.md) | 채팅 — 공간 · 멤버 · `MESSAGE` 와 `t` · 실시간 통지와 ACK · 미전송 기록 · `CATCHUP` 묶음 · 답장 · 고침 · 반응 · 지움 · 안 읽은 수 · 다른 node 로 가는 실시간 · hub 안 모듈 · 저장소 꼴 셋 · 혼잡 묶기 | `TOUR-20` · 2층 smoke `chat_fanout` · `chat_refused` · `chat_handover_reach` · `chat_live_bundle` · 통지 넘침 · 처리량 · 저장소 꼴 측정 |
| | [21](21_TALK_RECORD.md) | 무전 발언 기록 · 녹취 — 발언권 허가 하나 = `talk` 하나 · TLV `TALK` 발언 id · 상태 여섯과 `deadline` · 원본을 가리키는 갱신 · 녹음 유닛 · `GET /rec` | 2층 smoke `chat_talk_record` · `rec_unit_down` · 같은 형상 손 측정(20261119) · SDK 1층 |

## 용어

이 묶음은 표준에 이름이 있는 것은 그 이름으로 쓴다. 장마다 처음 나올 때 한 번만 우리말 풀이를 괄호로 단다. 우리 와이어 이름(`mline_table` · `arev` 꼴)은 백틱으로 그대로 둔다. 다른 장은 이 절을 `[용어](00_INDEX.md#용어)` 로 가리킨다.

### 표준 용어

| 표준 이름 | 우리말 풀이 | 출처 |
|---|---|---|
| SFU | 받은 RTP 를 섞지 않고 골라서 넘기는 미디어 서버. 우리 것은 `oxsfud` | 통용 |
| 시그널링 게이트웨이 | 클라가 WS 로 붙는 관문. 우리 것은 `oxhubd`(줄여 hub) | 통용 |
| PeerConnection(RTCPeerConnection) | 브라우저의 미디어 연결 하나. 1pc 는 하나에 발행 · 구독을 함께 싣고, 2pc 는 발행 · 구독을 둘로 나눈다 | W3C WebRTC |
| 트랜스포트 | 서버 쪽에서 본 연결 하나 — ICE · DTLS · SRTP 한 벌 | RFC 8445 · RFC 5764 |
| m-line | SDP 의 `m=` 줄 하나. 미디어 하나(소리 하나 · 영상 하나 · 데이터 채널)를 기술한다 | RFC 8866 · JSEP(RFC 8829) |
| 트랙 · 발행 · 구독 | 소리 · 영상 하나 · 내 트랙을 서버로 올림 · 남의 트랙을 서버에서 받음 | W3C WebRTC |
| 스트림(업링크 · 다운링크) | 한 SSRC 의 패킷 흐름. 발행자 → 서버가 업링크, 서버 → 구독자가 다운링크 | RFC 3550 |
| 시뮬캐스트 · 레이어 · `rid` | 한 트랙을 해상도 갈래 여럿으로 동시에 보내기 · 그 갈래 하나 · 갈래 이름 | RFC 8853 · RFC 8851 |
| PT 매핑 | payload type 번호와 코덱의 짝 | RFC 3551 · JSEP |
| RTP 헤더 재작성 | SSRC · 시퀀스 번호 · 타임스탬프를 받는 m-line 의 값으로 바꿔 쓰기(rewriting) | SFU 통용어 |
| CSRC | contributing source. 패킷에 싣는 원 발행자 번호 | RFC 3550 §5.1 |
| PLI · FIR | 키프레임 요청 | RFC 4585 · RFC 5104 |
| NACK · RTX | 잃은 패킷 알림 · 재전송 스트림 | RFC 4585 · RFC 4588 |
| SR · RR | 송신 보고 · 수신 보고 | RFC 3550 |
| BWE · TWCC · GCC | 대역 추정 · 전송 단위 혼잡 피드백 · 구글 혼잡 제어 | draft-holmer-rmcat-transport-wide-cc · draft-ietf-rmcat-gcc |
| full-duplex · half-duplex | 전이중(동시에 말함) · 반이중(한 번에 한 사람) | 통용 |
| floor control · MBCP | 발언권 제어 — 한 번에 한 사람만 말하게 하는 서버 판정 · 그 메시지 형식 | 3GPP TS 24.380 |
| Floor Request · Floor Release | 누름 · 놓음 | TS 24.380 |
| Floor Granted · Taken · Idle · Revoke · Deny · Queue Position Info | 허가 · 남이 말하는 중 · 쉼 · 회수 · 거절 · 대기 순번 | TS 24.380 |
| 대기열 · 대기열 승계 | 줄 서기 · 놓는 순간 대기열 맨 앞에게 바로 허가 | TS 24.380 §6.3.5 |
| 미디어 정체(stall) | 흘러야 할 미디어가 멎음 | 통용 |
| 전달 자격 · 전달 게이트(forwarding eligibility) | 서버가 이 패킷을 이 구독자에게 넘길지의 판정 | SFU 통용어 |
| 풀 메시(full mesh) | 모든 node 가 서로 직접 잇는 연결 모양 | 통용 |
| liveliness token · 선언(declare) | 주체 세션에 묶인 「있다」 표시 · 그것을 내놓기. 주체가 죽으면 저절로 사라진다 | zenoh 용어 |
| 우선순위 큐(lane) | 급한 메시지가 큰 메시지 뒤에 막히지 않게 나눈 큐 | 통용 |
| 네트워크 분할(partition) · 고립(isolation) | node 사이가 끊겨 무리가 갈림 · 나 혼자 떨어짐 | 분산 시스템 |
| fencing | 옛 주체(옛 기동)가 보낸 것을 막는 장치 | 분산 시스템 |
| rendezvous hashing(HRW) | 이름의 해시로 node 를 고르는 배치. 외부 저장소가 필요 없다 | 통용 |
| 정리(reclaim) · 축출(eviction) | 주인이 사라진 자원 거두기 · 새 접속이 옛 접속을 밀어내기 | 통용 |
| MCAP · Chunk · Message · Footer | 여러 채널의 메시지를 시각 차례로 한 파일에 적는 컨테이너 · 메시지 묶음(닫힌 단위) · 메시지 레코드 하나 · 파일 끝 레코드(있으면 정상으로 닫힌 파일) | mcap.dev 명세 |
| fMP4 · CMAF · init 세그먼트 · 미디어 조각 | 조각(`moof` + `mdat`)으로 나눈 ISO BMFF · 그 HLS · DASH 공용 꼴 · 트랙 · 코덱 머리(`ftyp` + `moov`) · 재생 단위 하나(`styp` + `moof` + `mdat`) | ISO/IEC 14496-12 · ISO/IEC 23000-19 |
| HLS · `EXT-X-MAP` · `EXT-X-DISCONTINUITY` | HTTP 위 조각 재생 목록(m3u8) · 조각을 풀 init 세그먼트 지정 · 「여기서 시각 · 코덱 머리가 끊긴다」 표시 | RFC 8216 |
| Opus PLC(packet loss concealment) | 디코더가 잃은 프레임을 은닉하는 것. 본문 없는 TOC 한 바이트 패킷을 주면 디코더가 손실로 보고 은닉한다 | RFC 6716 |
| fan-out | 메시지 하나를 받을 사람(또는 node) 여럿에게 퍼뜨리기 | 통용 |
| 적어도 한 번 전달(at-least-once) · 중복 거르기 | 빠지지 않는 대신 같은 것이 두 번 올 수 있는 전달 · 받는 쪽이 id 로 두 번째를 버리기 | 분산 시스템 |
| 멱등 키(idempotency key) | 같은 요청을 다시 보내도 결과가 한 번 보낸 것과 같게 하는 요청 id. 우리 것은 `msg_id` | 통용 |
| relation · redaction(tombstone) | 원본을 가리키는 새 메시지(고침 · 반응) · 지운 흔적 — 본문을 비우고 봉투만 남김 | Matrix · XEP-0424 |
| 안 읽은 수(unread count) | 공간의 글 수(무전 발언 기록 포함) − 내가 읽은 수 | 통용 |
| 범위 요청(Range) | 파일의 일부만 받는 HTTP 요청. 긴 녹취를 이어 받는다 | RFC 9110 §14 |

### 우리 설계 용어

표준에 없는 우리 것이다. 「집」은 그 사실을 풀어 쓴 장이다.

| 이름 | 뜻 | 집 |
|---|---|---|
| `mline_table` · `arev` | 서버가 고른 「받는 m-line(mid) → 트랙」 표 · 그 판 번호. 구독하는 PeerConnection 마다 하나 | [03](03_PUBLISH_SUBSCRIBE.md) |
| `READY` | 「이 `arev` 의 표로 협상을 끝냈다」는 확인. 서버는 확인된 m-line 으로만 보낸다 | [03](03_PUBLISH_SUBSCRIBE.md) |
| 방 목록 `seq` | 방의 사람 · 트랙 목록 차례(`version` 의 `epoch` · `seq`). 방마다 하나 | [03](03_PUBLISH_SUBSCRIBE.md) |
| 1pc 의 PT 매핑 | 1pc 가 `ROOM_JOIN` 의 `codecs` · `extmap` 에 싣는 브라우저의 PT · 확장 번호 표 | [02](02_CONNECT_JOIN.md) |
| 1pc · 2pc | PeerConnection 하나에 발행 · 구독을 함께 / 둘로. `BIND` 에서 정하는 세션 값 | [02](02_CONNECT_JOIN.md) |
| 미디어 워커 | 수신 스레드 하나가 받은 패킷을 넘기는 일꾼. 트랜스포트 하나는 워커 하나에 고정 | [04](04_PACKET_PATH.md) |
| 단일 스칼라 offset 재작성 · 재작성기 | m-line 마다 재작성기(`Rewriter`) 하나가 seq · ts 오프셋 하나를 든다. 출처가 바뀔 때만 다시 센다 | [04](04_PACKET_PATH.md) |
| 트랙 단위 duplex | 트랙 속성 — PTT 슬롯으로 흐르나(half) · 제 m-line 으로 흐르나(full) | [06](06_TRACK_ATTRS.md) |
| PTT 슬롯 | 방마다 화자 한 명 몫으로 고정한 오디오 · 비디오 m-line. 화자가 바뀌어도 SSRC · mid 가 그대로다 | [08](08_SLOT_SPEAKER.md) |
| `source_tag` | `BIND` 에서 켜는 세션 값. 켜면 그 세션의 다운링크 패킷마다 CSRC 를 싣는다 | [08](08_SLOT_SPEAKER.md) |
| 다방 청취 · 말할 방 | 여러 방을 듣고(`sub_rooms`) 말할 방 하나를 고른다(`pub_room` · `AFFILIATION`) | [08](08_SLOT_SPEAKER.md) |
| 마이크 전원(hot · hot_standby · cold) | SDK 혼자 정하는 장치 상태. 서버는 모른다 | [09](09_MIC_COST.md) |
| 재개 창 · `RESUME` | 소켓이 끊겨도 서버가 세션을 쥐는 시간(60초) · 그 안에서 지금 상태를 다시 받는 요청(세션 재개) | [10](10_RECOVERY.md) |
| 재연결(rebuild) | 세션 · PeerConnection 을 버리고 `ROOM_JOIN` 부터 다시 짓기 | [10](10_RECOVERY.md) |
| `sync_required` | 미디어 정체 때 구독자에게 가는 「방을 다시 받아라」 | [11](11_STALL_RESYNC.md) |
| 실패 번호 · 첫 숫자 | `{code, name, permanent}` · 첫 숫자가 할 일(1 클라 버그 ~ 5 서버 사정) | [12](12_ERRORS.md) |
| `sfu_id` = `epoch` | SFU 기동 신원. 기동마다 새 값 | [04](04_PACKET_PATH.md) · [13](13_MULTI_NODE.md) |
| node · 유닛 | hub 하나 + SFU 하나의 묶음(방은 한 node 에만 산다) · hub 가 띄우고 돌보는 SFU 프로세스 | [13](13_MULTI_NODE.md) · [14](14_OXMESH.md) |
| oxmesh · 방 토큰 · node 토큰 | node 사이 버스 · 방 위치와 node 생존을 알리는 liveliness token | [14](14_OXMESH.md) |
| 생성 상태기 · 전이 라벨 · 이음매 | 규격 원천에서 지은 판단 함수 · 그 걸음 이름(`SUB.C.WANT.nego_ok` 꼴) · 한 영역이 다른 영역 입구를 부르는 곳 | [15](15_SDK_INSIDE.md) |
| oxpeer · 투명 참가자 | 서버사이드 SDK · 토큰 클레임 `hidden` 이 참인 참가자(명단에 안 보임) | [16](16_OXPEER.md) |
| 세션 지문(session fingerprint) | `SHA-256(session_id)` 앞 8자. 서버 로그 `session=` 과 같다. 사용자가 문의할 때 불러 주는 번호로도 쓴다 | [17](17_OPERATIONS.md) |
| 로그 줄 `ev` · `diag` · `stat` | 규격에 이름 있는 상태 변화 · 판단 근거 · 5초 계수 | [17](17_OPERATIONS.md) |
| 이벤트 이력 · 이력 판정기 | `/admin/history` · `oxadmin history` · 그 사슬을 규격에 대는 `judge-history` | [17](17_OPERATIONS.md) |
| 원시 파트 · 파트 머리 · 사슬 | 녹음 원시 기록 MCAP 파일 하나(크기 · 시간으로 회전) · 파트 첫 메시지의 해석 상태 스냅숏 · `prev_last_sequence` + 1 = 첫 `sequence` | [18](18_RECORDING.md) |
| 레코드 흐름 · 지점 셋 | `sequence` · `log_time` · 채널 · 본문의 흐름(라이브 통 · 원시 파트가 같은 값) · 앱이 꺼내는 ① 레코드 ② 프레임 ③ 세그먼트 사건 | [18](18_RECORDING.md) |
| 파이프라인 · 분할 정책 | 레코드열 → 결과물을 결정론으로 짓는 순수 상태기(분기 · 조립 · 시간축 · 분할) · 발언(출처가 바뀌면 자름) / 길이(T · S 뒤 영상은 키프레임 · 음성은 아무 프레임) | [18](18_RECORDING.md) |
| 묶음(HLS) | HLS 스트림 하나 — 개인 트랙은 주인마다 · 무전 슬롯은 방마다 · 종류마다 트랙 하나 | [18](18_RECORDING.md) |
| 공간(`space_id`) · 멤버 | 채팅의 단위(채널 · 1:1 · 그룹) · 접속 · 방 입장과 무관하게 그 공간 메시지를 받는 사람. 무전 방은 `room_id` 가 곧 `space_id` | [20](20_CHAT.md) |
| `t` | 서버가 메시지를 저장할 때 매긴 ms. 공간 안의 차례. 유일하지 않아 `(t, msg_id)` 로 줄 세운다 | [20](20_CHAT.md) |
| 미전송 기록 | 받을 사람마다 `(user_id, msg_id, t)` 한 줄. 저장과 같은 트랜잭션에 쓰고 ACK 로 지운다. 남은 것이 못 받은 것 | [20](20_CHAT.md) |
| 따라잡기 · `MESSAGES` 묶음 | 앱이 부르는 `CATCHUP` · 서버가 남은 기록을 이어 보내는 통지(동시에 하나 · ACK 뒤 다음) | [20](20_CHAT.md) |
| 관계 메시지 · 접기 | 원본을 `ref` 로 가리키는 `edit` · `react` · `delete` · 받는 쪽이 그것을 원본 글에 합쳐 보이는 것 | [20](20_CHAT.md) |
| 멤버 변경 메시지 | 들어옴 · 나감 · 추가 · 뺌을 서버가 공간에 남기는 메시지. 행위자가 사용자가 아니면 `user_id` 가 `$sys` | [20](20_CHAT.md) |
| 저장소 꼴 | `[chat] db` 값의 꼴 — `memory:` · `sqlite:<경로>` · `postgres://…`. 계약은 같고 잠금이 다르다 | [20](20_CHAT.md) |
| 혼잡 묶음 | 통지 창이 찬 세션에서 줄 끝의 실시간 채팅 통지를 합친 `MESSAGES{live:true}`. 창에 빈 칸이 있으면 묶지 않는다 | [20](20_CHAT.md) |
| 발언 기록 · 발언 id | 발언권 허가 하나마다 서버가 공간에 남기는 메시지(`talk` · `user_id` = 화자)와 그 갱신(`talk_update`) · 허가 때 SFU 가 짓는 id(TLV `TALK` · 기록의 `msg_id` · 녹취 파일 이름에 같은 값) | [21](21_TALK_RECORD.md) |
| `deadline` | 허가 + T1 + T2 + T3. 끝 갱신 없이 지나면 받는 쪽이 그 발언을 `lost` 로 본다 | [21](21_TALK_RECORD.md) |
| 녹음 유닛(`oxtapd`) | node 마다 하나 · SFU 와 다른 프로세스. `record:true` 방에 녹화 참가자로 들어 발언마다 녹취 파일을 남긴다. 준비 상태(ready)에 세지 않는다 | [21](21_TALK_RECORD.md) |

## 틀 — 장마다 아홉 칸

| 칸 | 무엇 | 읽는 법 |
|---|---|---|
| 1. 30초 요약 | 그림 하나 + 세 줄 | 바쁘면 이것만 |
| 2. 등장인물 · 값 | 이 회차의 사람 · 방 · SSRC · 잰 값 표 | 아래 칸의 숫자가 다 여기서 나온다 |
| 3. 정상 길 | 실측 시각을 단 콜 플로우 · 실제 프레임 · 로그 발췌 · 해설 | 시각은 대개 초 아래 ms 만 적는다 |
| 4. 깨지는 길 | 같은 장의 고장 두세 갈래 — 무엇이 깨졌나 → 누가 먼저 아나 → 무엇이 오가나 → 사용자는 무엇을 겪나 | 「관찰」은 판정 전 기록이다. 고쳐진 것은 「20261109 고침」과 새 회차의 모습으로 적는다 |
| 5. 왜 이 모양인가 | 버린 길과 그 이유 · `★우리만의 것` 상자(선택 배경 · 얻은 것 · 잃은 것) | 잃은 것도 숫자로 적는다 |
| 6. 다른 SFU 는 | LiveKit · mediasoup · Janus · JVB · 무전 쪽과 견준 표 | 저쪽 칸은 소스를 정적으로 읽은 것이다. 실측이 아니다 |
| 7. 관측하는 법 | 이 장의 사건이 찍히는 로그 줄 · 운영 표면 · SDK 전이 라벨 | 줄 읽는 법은 [17](17_OPERATIONS.md) |
| 8. 소스 · 규격 좌표 | 서버 · SDK 함수 이름과 규격 라벨 | 지금 소스에 있는 이름만 |
| 9. 스스로 확인하는 질문 | 접힌 질문 셋 이상. 깨지는 길에서 하나 이상 | 답을 열기 전에 장을 다시 짚는다 |

## 재현 — 다시 재 보기

3층 탐침은 판정하지 않는다. 장의 재료를 탐침 하나마다 파일 하나로 거둔다. 재료는 WS 프레임 · SDP · ICE · DataChannel 바이트 · SDK 전이 · 서버 표면 · 서버 로그다. 탐침 이름이 장 번호다(`TOUR-02` · `02P` · `02T` · `06` · `07` · `08` · `09` · `10` · `11` · `13` · `20`). 21 장은 탐침이 없다 — 2층 smoke 와 같은 형상의 손 측정이 재료다. 서버는 개발 형상(`oxlens-sfu/dev/` — 한 기계 위 세 node)으로 띄워 둔다.

```bash
cd oxlens-sdk-web/qa/live && npx playwright test --project=chromium-probe tests/probe/tour_capture.spec.ts          # 전량
cd oxlens-sdk-web/qa/live && npx playwright test --project=chromium-probe tests/probe/tour_capture.spec.ts -g "TOUR-07"  # 하나(-g 는 제목 안 글자 일치라 "TOUR-02" 는 02P · 02T 도 고른다)
```

2층은 2층 하네스 `oxe2epy` 가 서버를 짓고 띄워, 헤드리스 봇으로 시나리오 전량을 돈다. 봇이 보내고 받은 WS · RTP · RTCP 를 바이트째 덤프로 남긴다. 끝에는 회차 구간의 서버 로그 전량을 사람마다 · 방마다 이력 판정기에 넣는다. 사슬 끊김 · 줄 꼴 어긋남 · 이른 재전송이 하나라도 있으면 빨강이다.

```bash
cd oxlens-labs && python -m oxe2epy run-all
```

회차 파일은 저장소의 보관소에 남고 커밋하지 않는다. 보기 · 모양 견주기 도구는 안쪽 저장소에 있다.
