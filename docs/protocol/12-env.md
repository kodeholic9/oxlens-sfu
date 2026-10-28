[← 차례](README.md)

## 12. ENV — 바깥 사건

장애 · 운영 · 환경 · 우리 동작의 결과 — 영역들이 처음 반응하는 바깥 사실의 목록. 곳(클라 · 미디어 서버 · 게이트웨이)마다 그곳에 사는 영역이 무엇으로 처음 반응하나

### 12-1. 구조

<a id="ENV.backing"></a>
#### 12-1-1. 가정-보증 대장 — 가정 · 성립 조건마다 받침

| 가정 · 성립 조건 | 받침 | 무엇 |
|---|---|---|
| `ASM-FLR-SPEAKER-RTP` | 성질 | `PROP-FLR-SEND-NEEDS-FLOOR` · `PROP-FLR-FLOOR-SENDS` |
| `ASM-FLR-T1-OVER-T132` | 산수(원장) | `T1(4000) > T132(2000)` — 참 |
| `ASM-FLR-T20-OVER-RTP` | 환경 | 승계 화자의 첫 RTP 가 T20(1초) 안에 서버에 관측된다 — 망 지연과 서버의 RTP 관측 주기(매 목록 M2 · F-3 — 250ms) |
| `ASM-FLR-TIMER-EMPTY` | 환경 | 가장 짧은 타이머(T101 · T100 · T104 0.5초)가 RTT 보다 길다 — 망의 사실 |
| `ASM-FLR-RETX-BEFORE-EXPIRE` | 산수(원장) | `T3(3000) >= T8(1000) * C8(3) && T1(4000) >= T20(1000) * C20(3)` — 참 |
| `ASM-FLR-T8-OVER-T132` | 산수(원장) | `C8(3) * T8(1000) > T132(2000)` — 참 |
| `ASM-FLR-RTP-INSTANT` | 환경 | has permission 클라의 RTP 가 곧바로 서버 참가자에 닿는다 — 망 지연 · 서버 관측 주기 |
| `ASM-FLR-HUMAN-SPEED` | 환경 | 사람 속도 — 한 사용자의 두 동작 사이에 통이 한 번은 빈다 |
| `ASM-FLR-STALE-GUARD` | 환경 | 앱 · 사람이 자기 REQUEST 가 떠 있는 동안 지난 주기의 뗌을 보내지 않는다 |
| `COND-LINK-REBUILD` | 문서 규율 | 서버 동작이다 — egress 다시 짓기(PT 는 구독자 표 · 헤더 확장은 통째로 버리고 구독자 ID 로 새로 쓴다 · mid 확장 = 구독 m-line mid)는 서버 정의서가 서버 동작으로 정하고 이 성립 조건을 위로 인용한다 · 연동규격서 모형 밖 |
| `COND-ROOM-BUDGET` | 산수(원장) | `Sub1pc(96) == BundleCap(128) - ClientMlines(32) && Sub2pc(128) == BundleCap(128) && SlotBudget(40) == DispatchCh(20) * 2 && SlotBudget(40) <= Sub1pc(96) && SlotBudget(40) < MaxRooms(100)` — 참 |
| `COND-LINK-SFU-ID-UNIQUE` | 환경 | 서버의 기동 단위(epoch)는 서버마다 고유하다 — 배포의 사실(서버 구현이 기동마다 고유 값을 낸다) · 다판 two_servers 는 epoch = 서버 번호로 둔다 |
| `COND-ROOM-SIMPLE-U64` | 환경 | 방이 서버 사이를 옮겨 다니지 않는다 — 배치(1 방 1 미디어 서버)의 사실 |
| `COND-SES-WINDOW-OVER-BACKOFF` | 산수(원장) | `TWindow(60000) > BackoffSum(44000) + BackoffJitterMax(8000)` — 참 |
| `COND-SES-ARITHMETIC` | 산수(원장) | `BackoffSum(44000) + BackoffJitterMax(8000) + CBackoff(10) * TBind(10000) > 2 * TWindow(60000)` — 참 |
| `COND-TRN-FIFO-IN-LANE` | 환경 | WS 는 TCP 위 — 한 연결 안 순서가 지켜진다 |
| `COND-TRN-NAMING` | 문서 규율 | 이름 짓기 규율 — 분류 축이 바뀌면 깨진다(동작이 아니다) |
| `COND-TRN-CORS-HEADER-AUTH` | 문서 규율 | 인증이 헤더 기반이라는 설계 선택 — 쿠키 인증을 들이면 깨진다 |

<a id="ENV.event.EV-WS-DROP"></a>
#### 12-1-2. `EV-WS-DROP` — WS 만 끊겼다(클라 ↔ 게이트웨이 경로)

종류 — 장애

| 곳 | 영역 | 칸 | 무엇 | 효과를 따라 닿을 수 있는 경우 |
|---|---|---|---|---|
| 클라 | SES | 입구 | `CDrop` | SES: `SES.C.any.drop_final` · `SES.C.any.drop_reclaimed` · `SES.C.any.drop_carry` · `SES.C.any.drop` / FLR: `FLR.C.any.down` / ROOM: `ROOM.C.any.close_all` · `ROOM.C.LEAVING.down` · `ROOM.C.any.down` / LINK: `LINK.C.any.close_all` / PUB: `PUB.C.SENDING.floor_close` · `PUB.C.any.floor_close_other` · `PUB.C.any.drop_server` / TRK: `TRK.C.any.store_drop` / AFF: `AFF.C.any.left` · `AFF.C.any.left_join` · `AFF.C.any.left_want` · `AFF.C.any.left_other` / SUB: `SUB.C.any.drop` |
| 클라 | LINK | 무관 | WS 만 끊긴 것 — 미디어 연결은 닫지 않는다(RULE-SES-ON-DROP 5 · R-SES-RESUME-KEEPS-MEDIA) |  |
| 클라 | ROOM | 무관 | 연결이 RESUMING 이면 방 상태는 그대로 둔다(RULE-ROOM-RESUMING-KEEPS) |  |
| 클라 | AFF | 무관 | 소속은 세션 것 — 재접속 복원은 RESUME 스냅샷이 전담한다(REJ-AFF-NO-RECONNECT-RESTORE) |  |
| 클라 | PUB | 무관 | 발행은 RESUME 이 이어받는다(RULE-SES-RES-RESUME-OK 2 ③) |  |
| 클라 | SUB | 무관 | 연결이 RESUMING 이면 방 보관본(구독 포함)은 그대로 — RESUME 스냅샷으로 정리한다(RULE-ROOM-RESUMING-KEEPS · RULE-TRK-FOUR-PATHS) |  |
| 클라 | TRK | 무관 | 연결이 RESUMING 이면 보관본은 그대로 — 끊긴 동안 사라진 TRACK_EVENT 는 RESUME 스냅샷이 trk.snapshot 으로 대신한다(RULE-TRK-FOUR-PATHS · RULE-ROOM-RESUMING-KEEPS) |  |
| 클라 | FLR | 무관 | WS 가 끊겨도 발언권은 유지된다 — DC 위(RULE-FLR-TRANSPORT-DC) |  |
| 게이트웨이 | SES | 입구 | `SDrop` | SES: `SES.S.drop_reclaim` · `SES.S.drop` / ROOM: `ROOM.S.session_gone_hidden` · `ROOM.S.session_gone` · `ROOM.S.session_gone_other` · `ROOM.S.seq_up_track` / FLR: `FLR.S.ws.leave` / AFF: `AFF.S.left` · `AFF.S.left_other` / PUB: `PUB.S.room_gone` · `PUB.S.room_gone_other` · `PUB.S.off_pub` / SUB: `SUB.S.drop` / TRK: `TRK.S.pub_same` · `TRK.S.pub_full` · `TRK.S.half_to_full` · `TRK.S.pub_half` · `TRK.S.full_to_half` · `TRK.S.unpub_full` · `TRK.S.unpub_resting` · `TRK.S.unpub_half` / LINK: `LINK.S.track_on_full` · `LINK.S.track_on` · `LINK.S.track_off` · `LINK.S.track_off_norow` |

<a id="ENV.event.EV-HUB-RESTART"></a>
#### 12-1-3. `EV-HUB-RESTART` — 게이트웨이가 재기동했다 — 세션이 다 사라진다

종류 — 장애

| 곳 | 영역 | 칸 | 무엇 | 효과를 따라 닿을 수 있는 경우 |
|---|---|---|---|---|
| 게이트웨이 | SES | 입구 | `SRestart` | SES: `SES.S.restart` |
| 클라 | SES | 입구 | `CDrop` — LEAVE 없이 끊긴다 — 다시 붙으면 RESUME 2008 · 세션 전체 재구축(RULE-SES-REBUILD) | SES: `SES.C.any.drop_final` · `SES.C.any.drop_reclaimed` · `SES.C.any.drop_carry` · `SES.C.any.drop` / FLR: `FLR.C.any.down` / ROOM: `ROOM.C.any.close_all` · `ROOM.C.LEAVING.down` · `ROOM.C.any.down` / LINK: `LINK.C.any.close_all` / PUB: `PUB.C.SENDING.floor_close` · `PUB.C.any.floor_close_other` · `PUB.C.any.drop_server` / TRK: `TRK.C.any.store_drop` / AFF: `AFF.C.any.left` · `AFF.C.any.left_join` · `AFF.C.any.left_want` · `AFF.C.any.left_other` / SUB: `SUB.C.any.drop` |
| 클라 | LINK | 무관 | 세션 축이 먼저다 — 재구축이면 SES 가 media.close_all 로 알린다(①) |  |
| 클라 | ROOM | 무관 | SES 가 rooms.down_all · rooms.rejoin 으로 알린다(①) |  |
| 클라 | AFF | 무관 | 방을 내릴 때 ROOM 이 aff.leave 로 알린다(①) |  |
| 클라 | PUB | 무관 | SES 가 media.close_all 로 연결을 닫아 LINK 가 pub.drop_server 로 IDLE · 다시 들어간 뒤 pub.reregister_all 로 다시 발행(DEV-SES-REBUILD-REPUBLISH · 원칙 ①) |  |
| 클라 | SUB | 무관 | 방을 내릴 때 ROOM 이 str.drop 으로 · 다시 입장 응답이 ready.tracks 로 알린다(①) |  |
| 클라 | TRK | 무관 | 방을 내릴 때 ROOM 이 store.drop 으로 · 다시 입장 응답이 trk.snapshot 으로 알린다(①) |  |
| 클라 | FLR | 무관 | SES 가 floor.down_all 로 알린다(①) |  |
| 미디어 서버 | LINK | 무관 | 같다 — Peer 를 거둔다(원칙 ③) |  |
| 미디어 서버 | ROOM | 무관 | 같다 — 명단에서 빠진다(방마다 PARTICIPANT_EVENT{leave}) |  |
| 미디어 서버 | AFF | 무관 | 같다 — 발행 방 기록이 사라진다 |  |
| 미디어 서버 | PUB | 무관 | 같다 — 발행이 정리된다 |  |
| 미디어 서버 | SUB | 무관 | 같다 — 구독 게이트 기록이 사라진다 |  |
| 미디어 서버 | TRK | 무관 | 서버 정의서 몫 — 게이트웨이가 재기동해 세션이 사라지면 미디어 서버도 그 세션의 Peer 를 세션 폐기와 같은 길로 거둔다(어떻게 아는가는 서버 정의서) — 트랙이 빠진다 |  |
| 미디어 서버 | FLR | 무관 | 같다 — 발언권 반환 · 큐 제거 |  |

<a id="ENV.event.EV-NET-SHAKE"></a>
#### 12-1-4. `EV-NET-SHAKE` — 망이 흔들렸다 — ICE 경로가 나빠졌다

종류 — 장애

| 곳 | 영역 | 칸 | 무엇 | 효과를 따라 닿을 수 있는 경우 |
|---|---|---|---|---|
| 클라 | SES | 무관 | WS 가 살아 있으면 세션 축은 모른다 — LINK 가 WS 부터 확인한다(RULE-LINK-RECOVERY-UNIT) |  |
| 클라 | LINK | 입구 | `CIceDisc` | LINK: `LINK.C.any.ice_disc_leaving` · `LINK.C.OK.ice_disc` · `LINK.C.any.ice_disc_absorbed` |
| 클라 | ROOM | 무관 | LINK 가 확정하면 store.drop_server · rooms.rejoin_server 로 알린다(①) |  |
| 클라 | AFF | 무관 | 소속은 연결 흔들림과 무관하다 — 재구축이 확정되면 방이 aff.leave 로 알린다(①) |  |
| 클라 | PUB | 무관 | LINK 가 확정하면 pub.reregister_server 로 알린다(①) |  |
| 클라 | SUB | 무관 | LINK 가 확정하면 방을 내리며 str.drop · 다시 입장 응답이 ready.tracks 로 알린다(①) |  |
| 클라 | TRK | 무관 | LINK 가 확정하면 방을 내리며 store.drop · 다시 입장 응답이 trk.snapshot 으로 알린다(①) |  |
| 클라 | FLR | 무관 | 발언권은 T1 · T2 만 본다 — 되살아나면 재동기가 알린다(RULE-FLR-SERVER-EVENTS) |  |
| 미디어 서버 | LINK | 입구 | `SSuspect` — 서버 판정은 보강이다 — 경로의 생사는 클라가 먼저 안다(RULE-DOC-COND-INDEX) | LINK: `LINK.S.suspect` |
| 미디어 서버 | ROOM | 무관 | 서버는 미디어 침묵으로 방에서 빼지 않는다 — Peer 수명 = 세션 수명(RULE-DOC-COND-INDEX) |  |
| 미디어 서버 | AFF | 무관 | 소속은 연결 흔들림과 무관하다 |  |
| 미디어 서버 | PUB | 무관 | 같다 — 발행은 연결 흔들림과 무관하다 |  |
| 미디어 서버 | SUB | 무관 | 서버 구독 게이트는 READY 와 T-gate 로만 바뀐다(RULE-SUB-TGATE) |  |
| 미디어 서버 | TRK | 무관 | 트랙 층은 발행 층만 따른다 — 연결 흔들림과 무관하다(RULE-TRK-PUB-TO-TRACK) |  |
| 미디어 서버 | FLR | 무관 | 서버 발언권은 RTP 를 보고(T1) 거둔다 — 발언권 축의 세계 사실이다 |  |

<a id="ENV.event.EV-ASSOC-LOST"></a>
#### 12-1-5. `EV-ASSOC-LOST` — SCTP 연관이 사라졌다 · DTLS 가 닫혔다

종류 — 장애

| 곳 | 영역 | 칸 | 무엇 | 효과를 따라 닿을 수 있는 경우 |
|---|---|---|---|---|
| 클라 | SES | 무관 | 세션은 미디어 연결의 사본을 두지 않는다 — RESUME 응답 때 LINK 상태를 읽어 되살림을 고른다(CResumeRes 의 reads · CAND-SES-CARRY-COPY) |  |
| 클라 | LINK | 입구 | `CAssocLost` | LINK: `LINK.C.any.assoc_lost_leaving` · `LINK.C.REBUILD.assoc_lost_absorbed` · `LINK.C.any.assoc_lost` · `LINK.C.any.renego_queued` · `LINK.C.any.renego` / FLR: `FLR.C.any.down` / ROOM: `ROOM.C.LEAVING.down` · `ROOM.C.any.down` · `ROOM.C.NONE.rejoin` / PUB: `PUB.C.any.reregister` · `PUB.C.any.reregister_off` · `PUB.C.SENDING.floor_close` · `PUB.C.any.floor_close_other` / TRK: `TRK.C.any.store_drop` / AFF: `AFF.C.any.left` · `AFF.C.any.left_join` · `AFF.C.any.left_want` · `AFF.C.any.left_other` / SUB: `SUB.C.any.drop` |
| 클라 | ROOM | 무관 | LINK 가 확정하면 store.drop_server · rooms.rejoin_server 로 알린다(①) |  |
| 클라 | AFF | 무관 | 방을 내릴 때 ROOM 이 aff.leave 로 알린다(①) |  |
| 클라 | PUB | 무관 | LINK 가 확정하면 pub.reregister_server 로 알린다(①) |  |
| 클라 | SUB | 무관 | 방을 내릴 때 ROOM 이 str.drop 으로 · 다시 입장 응답이 ready.tracks 로 알린다(①) |  |
| 클라 | TRK | 무관 | 방을 내릴 때 ROOM 이 store.drop 으로 · 다시 입장 응답이 trk.snapshot 으로 알린다(①) |  |
| 클라 | FLR | 무관 | LINK 가 확정하면 floor.down_server 로 알린다(①) |  |

<a id="ENV.event.EV-DC-CLOSED"></a>
#### 12-1-6. `EV-DC-CLOSED` — DC 채널만 닫혔다(연관은 산다)

종류 — 장애

| 곳 | 영역 | 칸 | 무엇 | 효과를 따라 닿을 수 있는 경우 |
|---|---|---|---|---|
| 클라 | SES | 무관 | 미디어 연결은 산다 — 세션 축과 무관하다 |  |
| 클라 | LINK | 입구 | `CDcClosed` | LINK: `LINK.C.any.dc_closed_leaving` · `LINK.C.OK.dc_closed` · `LINK.C.any.dc_closed_absorbed` / FLR: `FLR.C.any.dc_down` |
| 클라 | ROOM | 무관 | DC 는 발언권만 싣는다(RULE-FLR-TRANSPORT-DC) |  |
| 클라 | AFF | 무관 | DC 는 발언권만 싣는다(RULE-FLR-TRANSPORT-DC) |  |
| 클라 | PUB | 무관 | DC 는 발언권만 싣는다(RULE-FLR-TRANSPORT-DC) |  |
| 클라 | SUB | 무관 | DC 는 발언권만 싣는다(RULE-FLR-TRANSPORT-DC) |  |
| 클라 | TRK | 무관 | DC 는 발언권만 싣는다(RULE-FLR-TRANSPORT-DC) |  |
| 클라 | FLR | 무관 | LINK 가 floor.dc_down 으로 알린다(① · RULE-FLR-PROC-DC-DOWN) |  |

<a id="ENV.event.EV-SFU-RESTART"></a>
#### 12-1-7. `EV-SFU-RESTART` — 미디어 서버가 재기동했다 — 그 서버의 상태가 다 사라진다

종류 — 장애

| 곳 | 영역 | 칸 | 무엇 | 효과를 따라 닿을 수 있는 경우 |
|---|---|---|---|---|
| 미디어 서버 | LINK | 입구 | `SRestartSrv` | LINK: `LINK.S.server_restart` |
| 미디어 서버 | ROOM | 입구 | `SRestart` — 그 방의 서버 상태가 사라지고 epoch 가 바뀐다 — 결말은 LINK 길(주도 · 원칙 ②)이 정하고 클라는 epoch 가 다른 델타 · 스냅샷(규칙 1)으로도 안다 | ROOM: `ROOM.S.restart` |
| 미디어 서버 | AFF | 무관 | 소속 기록이 없다 — 다시 들어간 뒤 pub_select(RULE-LINK-RECOVERY-REBUILD · 원칙 ①) |  |
| 미디어 서버 | PUB | 무관 | 발행이 없다 — 클라가 전부 다시 발행한다(원칙 ① · pub.reregister_server) |  |
| 미디어 서버 | SUB | 무관 | 구독 게이트 기록이 없다 — 다시 입장 · READY 가 다시 세운다(원칙 ①) |  |
| 미디어 서버 | TRK | 무관 | 트랙 층 기록이 없다 — 다시 발행하면 대응 표대로 다시 생긴다(원칙 ① · RULE-TRK-PUB-TO-TRACK) |  |
| 미디어 서버 | FLR | 무관 | 발언권 상태가 없다 — 새 입장이 처음 알리기로 새 상태를 준다 · 클라는 처음 값으로(DEV-FLR-DOWN-FRESH · 원칙 ①) |  |
| 클라 | SES | 무관 | 게이트웨이 WS 는 산다 — 세션 축은 모른다 |  |
| 클라 | LINK | 입구 | `CIceDisc` — ICE 가 끊기고 ICE_RESTART 가 3010 — 그 서버만 재구축(흐름 F3) | LINK: `LINK.C.any.ice_disc_leaving` · `LINK.C.OK.ice_disc` · `LINK.C.any.ice_disc_absorbed` |
| 클라 | ROOM | 무관 | room_closed 는 운영의 방 닫힘 전용이다 — 재기동은 LINK 길이 정한다(원칙 ② · R-ROOM-CLOSED-OPS-ONLY) · 방은 LINK 가 store.drop_server 로 내리고 rooms.rejoin_server 로 다시 들어간다(①) |  |
| 클라 | AFF | 무관 | LINK 재구축이 방을 내릴 때 ROOM 이 aff.leave 로 알린다(①) |  |
| 클라 | PUB | 무관 | LINK 가 확정하면 pub.reregister_server 로 알린다(①) |  |
| 클라 | SUB | 무관 | LINK 재구축이 방을 내릴 때 ROOM 이 str.drop 으로 알린다(①) |  |
| 클라 | TRK | 무관 | LINK 재구축이 방을 내릴 때 ROOM 이 store.drop 으로 · 다시 입장 응답이 trk.snapshot 으로 알린다(①) |  |
| 클라 | FLR | 무관 | LINK 가 floor.down_server 로 알린다(① — 처음 값으로, DEV-FLR-DOWN-FRESH) |  |

주도(미디어 서버) — LINK

<a id="ENV.event.EV-KICK"></a>
#### 12-1-8. `EV-KICK` — 운영 · 앱 백엔드가 그 사람을 방에서 강퇴했다

종류 — 운영

| 곳 | 영역 | 칸 | 무엇 | 효과를 따라 닿을 수 있는 경우 |
|---|---|---|---|---|
| 미디어 서버 | LINK | 무관 | ROOM 이 link.room_gone 으로 알린다 — 그 서버의 마지막 방이면 Peer 를 거둔다(RULE-LINK-SERVER-REAP · 원칙 ③) |  |
| 미디어 서버 | ROOM | 입구 | `SKick` | ROOM: `ROOM.S.kick_hidden` · `ROOM.S.kick` · `ROOM.S.seq_up_track` / FLR: `FLR.S.ws.leave` / AFF: `AFF.S.left` · `AFF.S.left_other` / PUB: `PUB.S.room_gone` · `PUB.S.room_gone_other` · `PUB.S.off_pub` · `PUB.S.peer_gone` · `PUB.S.peer_gone_other` / SUB: `SUB.S.drop` / LINK: `LINK.S.peer_reaped` · `LINK.S.room_gone_other` · `LINK.S.track_on_full` · `LINK.S.track_on` · `LINK.S.track_off` · `LINK.S.track_off_norow` / TRK: `TRK.S.pub_same` · `TRK.S.pub_full` · `TRK.S.half_to_full` · `TRK.S.pub_half` · `TRK.S.full_to_half` · `TRK.S.unpub_full` · `TRK.S.unpub_resting` · `TRK.S.unpub_half` |
| 미디어 서버 | AFF | 무관 | 방을 내릴 때 ROOM 이 aff.leave 로 알린다(①) |  |
| 미디어 서버 | PUB | 무관 | ROOM 이 pub.room_gone 으로 — 서버가 그 사람의 그 방 전이중 발행을 정리한다 · 반이중은 그 서버 마지막 방까지(원칙 ③) |  |
| 미디어 서버 | SUB | 무관 | ROOM 이 str.drop 으로 구독 게이트를 처음으로(원칙 ③) |  |
| 미디어 서버 | TRK | 무관 | ROOM 이 pub.room_gone 으로 · PUB 가 전이중 발행을 정리하며 trk.pub_remove 로 알린다 — 남들에게 TRACK_EVENT{remove}(①) |  |
| 미디어 서버 | FLR | 무관 | ROOM 이 floor.leave 로 알린다(① · CAND-FLR-ROOM-GONE) |  |
| 클라 | SES | 무관 | 세션은 그대로다 |  |
| 클라 | LINK | 무관 | 방이 link.room_left 로 알린다 — 그 서버의 마지막 방이면 연결을 닫는다(RULE-ROOM-LEAVE-RES 2 · RULE-SES-RES-RESUME-OK 4) |  |
| 클라 | ROOM | 입구 | `CRoomEvent` — ROOM_EVENT{affiliation, kick} — ROOM.C.JOINED.kicked | ROOM: `ROOM.C.any.room_event_not_joined` · `ROOM.C.JOINED.kicked` · `ROOM.C.JOINED.sync_required_inflight` · `ROOM.C.JOINED.sync_required` / LINK: `LINK.C.any.renego_queued` · `LINK.C.any.renego` · `LINK.C.any.last_room_left` · `LINK.C.any.room_left` / TRK: `TRK.C.any.store_drop` / FLR: `FLR.C.any.leave` / AFF: `AFF.C.any.left` · `AFF.C.any.left_join` · `AFF.C.any.left_want` · `AFF.C.any.left_other` / SUB: `SUB.C.any.drop` / PUB: `PUB.C.any.room_gone` · `PUB.C.any.room_gone_other` · `PUB.C.SENDING.floor_close` · `PUB.C.any.floor_close_other` · `PUB.C.any.drop_server` |
| 클라 | AFF | 무관 | 방을 내릴 때 ROOM 이 aff.leave 로 알린다(①) |  |
| 클라 | PUB | 무관 | ROOM 이 pub.room_gone 으로 — 그 방의 전이중 발행을 IDLE 로(서버가 정리했다 · 원칙 ③) · 마지막 방이면 LINK 가 pub.drop_server 로 |  |
| 클라 | SUB | 무관 | 방을 내릴 때 ROOM 이 str.drop 으로 · 다시 입장 응답이 ready.tracks 로 알린다(①) |  |
| 클라 | TRK | 무관 | ROOM 이 store.drop 으로 알린다(①) |  |
| 클라 | FLR | 무관 | ROOM 이 floor.leave 로 알린다(①) |  |

<a id="ENV.event.EV-ROOM-CLOSE"></a>
#### 12-1-9. `EV-ROOM-CLOSE` — 방이 닫혔다(운영)

종류 — 운영

| 곳 | 영역 | 칸 | 무엇 | 효과를 따라 닿을 수 있는 경우 |
|---|---|---|---|---|
| 미디어 서버 | LINK | 무관 | ROOM 이 link.room_gone 으로 알린다 — 그 서버의 마지막 방이면 Peer 를 거둔다(RULE-LINK-SERVER-REAP · 원칙 ③) |  |
| 미디어 서버 | ROOM | 입구 | `SClose` | ROOM: `ROOM.S.close_notify` · `ROOM.S.close` · `ROOM.S.seq_up_track` / FLR: `FLR.S.ws.leave` / AFF: `AFF.S.left` · `AFF.S.left_other` / PUB: `PUB.S.room_gone` · `PUB.S.room_gone_other` · `PUB.S.off_pub` · `PUB.S.peer_gone` · `PUB.S.peer_gone_other` / SUB: `SUB.S.drop` / LINK: `LINK.S.peer_reaped` · `LINK.S.room_gone_other` · `LINK.S.track_on_full` · `LINK.S.track_on` · `LINK.S.track_off` · `LINK.S.track_off_norow` / TRK: `TRK.S.pub_same` · `TRK.S.pub_full` · `TRK.S.half_to_full` · `TRK.S.pub_half` · `TRK.S.full_to_half` · `TRK.S.unpub_full` · `TRK.S.unpub_resting` · `TRK.S.unpub_half` |
| 미디어 서버 | AFF | 무관 | 방을 내릴 때 ROOM 이 aff.leave 로 알린다(①) |  |
| 미디어 서버 | PUB | 무관 | ROOM 이 pub.room_gone 으로 — 서버가 그 사람의 그 방 전이중 발행을 정리한다 · 반이중은 그 서버 마지막 방까지(원칙 ③) |  |
| 미디어 서버 | SUB | 무관 | ROOM 이 str.drop 으로 구독 게이트를 처음으로(원칙 ③) |  |
| 미디어 서버 | TRK | 무관 | 방이 닫혀 그 방 트랙 층이 통째로 사라진다 — 남은 사람이 없다(전원에게 ROOM_EVENT{room_closed}) |  |
| 미디어 서버 | FLR | 무관 | ROOM 이 floor.leave 로 알린다(① · CAND-FLR-ROOM-GONE) |  |
| 클라 | SES | 무관 | 세션은 그대로다 |  |
| 클라 | LINK | 무관 | 방이 link.room_left 로 알린다 — 그 서버의 마지막 방이면 연결을 닫는다(RULE-ROOM-LEAVE-RES 2 · RULE-SES-RES-RESUME-OK 4) |  |
| 클라 | ROOM | 입구 | `CRoomEvent` — ROOM_EVENT{affiliation, room_closed} — ROOM.C.JOINED.kicked | ROOM: `ROOM.C.any.room_event_not_joined` · `ROOM.C.JOINED.kicked` · `ROOM.C.JOINED.sync_required_inflight` · `ROOM.C.JOINED.sync_required` / LINK: `LINK.C.any.renego_queued` · `LINK.C.any.renego` · `LINK.C.any.last_room_left` · `LINK.C.any.room_left` / TRK: `TRK.C.any.store_drop` / FLR: `FLR.C.any.leave` / AFF: `AFF.C.any.left` · `AFF.C.any.left_join` · `AFF.C.any.left_want` · `AFF.C.any.left_other` / SUB: `SUB.C.any.drop` / PUB: `PUB.C.any.room_gone` · `PUB.C.any.room_gone_other` · `PUB.C.SENDING.floor_close` · `PUB.C.any.floor_close_other` · `PUB.C.any.drop_server` |
| 클라 | AFF | 무관 | 방을 내릴 때 ROOM 이 aff.leave 로 알린다(①) |  |
| 클라 | PUB | 무관 | ROOM 이 pub.room_gone 으로 — 그 방의 전이중 발행을 IDLE 로(서버가 정리했다 · 원칙 ③) · 마지막 방이면 LINK 가 pub.drop_server 로 |  |
| 클라 | SUB | 무관 | 방을 내릴 때 ROOM 이 str.drop 으로 · 다시 입장 응답이 ready.tracks 로 알린다(①) |  |
| 클라 | TRK | 무관 | ROOM 이 store.drop 으로 알린다(①) |  |
| 클라 | FLR | 무관 | ROOM 이 floor.leave 로 알린다(①) |  |

<a id="ENV.event.EV-SERVER-STALL"></a>
#### 12-1-10. `EV-SERVER-STALL` — 서버가 어긋남을 감지했다(미디어가 멎었다 등)

종류 — 환경

<a id="ENV.event.EV-TOKEN-AGED"></a>
#### 12-1-11. `EV-TOKEN-AGED` — 토큰이 만료됐다(시간)

종류 — 환경

<a id="ENV.event.EV-LATE"></a>
#### 12-1-12. `EV-LATE` — 끊긴 뒤 경과가 재개 창을 넘었다(클라 시계)

종류 — 환경

<a id="ENV.event.EV-RETRY-DUE"></a>
#### 12-1-13. `EV-RETRY-DUE` — 4009 의 retry_after_ms 하한이 지났다

종류 — 환경

<a id="ENV.event.EV-LOAD"></a>
#### 12-1-14. `EV-LOAD` — 정원 · 과부하 · 상한 · 서버 사정이 바뀌었다

종류 — 환경

<a id="ENV.event.EV-FILL-RANK"></a>
#### 12-1-15. `EV-FILL-RANK` — 받을 트랙의 순위가 바뀌었다(video = 클라 우선순위 · paused · audio = 음량)

종류 — 환경

<a id="ENV.event.EV-OTHERS"></a>
#### 12-1-16. `EV-OTHERS` — 다른 사람이 그 방 공통 스냅샷을 바꿨다

종류 — 환경

<a id="ENV.event.EV-PERM"></a>
#### 12-1-17. `EV-PERM` — 결정자가 그 방에서 그 사람의 권한 비트를 바꿨다(서버 밖의 결정자 — RULE-ROOM-PERMISSION-AXIS)

종류 — 환경

<a id="ENV.event.EV-R-SOCK"></a>
#### 12-1-18. `EV-R-SOCK` — 소켓 열기의 결과

종류 — 우리 동작의 결과

<a id="ENV.event.EV-R-NEGO"></a>
#### 12-1-19. `EV-R-NEGO` — 협상(offer/answer)의 결과

종류 — 우리 동작의 결과

<a id="ENV.event.EV-R-ATTACH"></a>
#### 12-1-20. `EV-R-ATTACH` — 트랙 붙이기의 결과

종류 — 우리 동작의 결과

<a id="ENV.event.EV-R-APPLY"></a>
#### 12-1-21. `EV-R-APPLY` — 새 자격 적용(재협상)의 결과

종류 — 우리 동작의 결과

<a id="ENV.event.EV-R-REOPEN"></a>
#### 12-1-22. `EV-R-REOPEN` — DC 다시 열기의 결과

종류 — 우리 동작의 결과

<a id="ENV.event.EV-R-CHECK"></a>
#### 12-1-23. `EV-R-CHECK` — 새 자격의 첫 검증된 검사

종류 — 우리 동작의 결과

<a id="AUX"></a>
