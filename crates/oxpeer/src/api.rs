// author: kodeholic (powered by Claude)
//! 앱 표면 — 핸들(`Client`) · 사건 통(`Events`) · 미디어 통 둘(`Media` 패킷 · `Frames` 프레임 — 트랙마다 고른 단위대로). 콜백을 받지 않는다 — 앱이 통에서 꺼내 간다.
//! 사건 이름은 SDK 정의서 효과 표의 것이다(SDK:IMPL-API-EVENT-MAP — 플랫폼 공유 계약) · 서버사이드에 해당하지 않는 것(장치 · 재생 · 발행)은 없다(SDK:RULE-PLAT-SERVER).

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::core::{self, Cmd};
use crate::error::Error;
pub use crate::generated::ws_wire::{Duplex, Kind, MemberInfo, Permission, TokenReq, TrackEntry, TrackEntryType};
pub use crate::link::recv::{MediaFrame, Unit};
pub use crate::link::send::OutFrame;
pub use crate::link::transport::{MediaPacket, Source};

/// 접속 설정 — `base` 는 앱이 준다(연:RULE-TRN-BASE-GIVEN) · `token` 은 앱이 발급받은 것(연:SES.http.TOKEN)
#[derive(Clone, Debug)]
pub struct Config {
    /// `http://host:port/경로` — WS 는 `ws://host:port/경로/ws` · `https://` 면 `wss://`(context 202611/20261108d E2)
    pub base: String,
    /// `https://` hub 의 사설 CA 뿌리(PEM) — 주면 이것만 믿는다 · 없으면 webpki 뿌리
    pub tls_root_pem: Option<Vec<u8>>,
    /// `https://` hub 의 인증서 하나(PEM) — 주면 받은 인증서가 이것과 같을 때만 믿는다(이름 검사 없음 · `tls_root_pem` 보다 앞선다 · 같은 기계 hub 에 루프백으로 붙는 녹음 유닛 몫)
    pub tls_pin_pem: Option<Vec<u8>>,
    pub token: String,
    /// 미디어 통 크기(패킷) — 앱이 늦게 꺼내 가득 차면 버리고 센다(`Stats::media_dropped`)
    pub media_capacity: usize,
    /// 프레임 통 크기 — 가득 차면 버리고 센다(`Stats::frames_dropped`)
    pub frames_capacity: usize,
    /// 받는 단위 기본(SDK:RULE-PLAT-SERVER 수신 단위 · 시각) — 트랙마다는 `Client::set_unit`
    pub unit: Unit,
    /// 녹음 원시 기록(SDK:RULE-PLAT-REC-TAP) — 주면 받는 줄 전부의 정리층 출구 RTP 와 제어 사건을 MCAP 파트에 적는다
    #[cfg(feature = "record")]
    pub record: Option<std::sync::Arc<crate::record::Recorder>>,
    /// 운영 로그(SDK:RULE-PLAT-SERVER 「운영 로그」) — 기본은 켬 · 표준 출력 · 같은 장비면 `socket` 에 hub 로거 소켓
    pub log: crate::log::LogConfig,
}

impl Config {
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Config {
        Config {
            base: base.into(),
            tls_root_pem: None,
            tls_pin_pem: None,
            token: token.into(),
            media_capacity: 4_096,
            frames_capacity: 1_024,
            unit: Unit::Packet,
            #[cfg(feature = "record")]
            record: None,
            log: crate::log::LogConfig::default(),
        }
    }
}

/// 받을 층(SDK:IMPL-TRK-RECEIVE `RemoteTrack.setLayer`) — ★값은 상한이고 실제 선택은 서버다 · 생략한 칸은 안 바꾼다(연:RULE-SUB-LAYER-PROC)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Layer {
    pub spatial: Option<u8>,
    pub temporal: Option<u8>,
    pub paused: Option<bool>,
    pub priority: Option<u8>,
}

/// 세션 연결 상태(SDK:IMPL-SES-API `session.state`) — 위 상태 넷 그대로
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Disconnected,
    Connecting,
    Active,
    Resuming,
}

/// 파생 요약(SDK:IMPL-SES-API `client.health`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Ok,
    Degraded,
    Recovering,
    Offline,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionInfo {
    pub state: SessionState,
    pub recovering: bool,
    /// 마지막으로 끊긴 사유 — 서버가 준 사유면 그 code · name, 사유 없는 끊김은 code 0 + `TRANSPORT` · `BIND_TIMEOUT` · `BIND_FAILED`
    pub reason: Option<(u16, String)>,
    pub user_id: Option<String>,
    /// 접속 응답의 확정값 `1pc` · `2pc`
    pub pc_mode: Option<String>,
    pub health: Health,
}

/// 서버마다 연결 상태(SDK:IMPL-LINK-PROJECTION)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Connecting,
    Up,
    Wobbly,
    Restarting,
    Rebuilding,
    Closed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkInfo {
    pub server: String,
    pub state: LinkState,
    pub rung: u32,
    pub attempt: u32,
    /// 재구축 원인 · `path_unreachable`
    pub cause: Option<String>,
}

/// 사건 하나 — 같은 방의 사건은 서버 도착 순서대로(SDK:RULE-API-ASYNC)
#[derive(Clone, Debug)]
pub enum Event {
    /// 상태 · recovering · reason · health 가 바뀌었다
    Session(SessionInfo),
    /// 재접속 중 위가 토큰을 요구했다 — `Client::set_token` 으로 답한다(SDK:IMPL-SES-EVENTS)
    TokenRequired,
    /// 앱이 끊지 않았는데 세션이 끝났다 — retryable = 다시 `connect` 할 보람이 있다(백오프 소진)
    Closed { code: u16, name: String, retryable: bool },
    /// 새 세션으로 다시 붙어 방을 다시 세운다(이어받기 실패 · 회수 · 토큰 만료 뒤)
    Rebuilt,
    Link(LinkInfo),
    Joined { room: String },
    JoinFailed { room: String, error: Error },
    Left { room: String },
    /// 서버가 방을 닫았다(`kick` · `room_closed` · 이어받기 스냅샷에서 빠짐 `missing`)
    Forced { room: String, cause: String },
    /// 명단이 바뀌었다(투명 참가자는 없다)
    Participants { room: String, participants: Vec<MemberInfo> },
    /// 받는 트랙 목록이 바뀌었다 — 이 방의 받을 트랙 전부(내 개인 트랙 제외)
    Tracks { room: String, tracks: Vec<TrackEntry> },
    Muted { room: String, track_id: String, muted: bool },
    /// 그 트랙의 첫 복호 RTP 패킷(SDK:RULE-PLAT-SERVER `receiving`)
    Receiving { room: String, track_id: String },
    /// 그 방의 화자 — None = 아무도 말하지 않는다(SDK:IMPL-FLR-EXPOSE `speaker`)
    Speaker { room: String, user_id: Option<String> },
    /// 발언권 표시를 믿을 수 있나(DC 가 살아 있나 — `ptt.state.trusted`)
    FloorTrusted { room: String, trusted: bool },
    /// 재동기를 반영했다
    Resync { room: String },
    /// 방 · 구독의 요청이 실패했다(내 요청이 아닌 것)
    Error { room: Option<String>, error: Error },
    // ── 말하는 쪽(20261107c) ──
    /// 내 발행이 섰다(SDK 효과 표 `published`) · 실패했다 · 끝났다(서버가 지웠다 — `ended`)
    Published { room: String, track_id: String },
    PublishFailed { room: String },
    TrackEnded { room: String, track_id: String },
    /// 서버가 내 발행에 키프레임을 청했다(PLI) — 앱이 인코더에 키프레임을 시킨다(SDK 는 인코딩하지 않는다)
    KeyframeRequested { room: String, track_id: String, layer: Option<u8> },
    /// 그 발행 연결의 목표 비트레이트가 바뀌었다(TWCC 손실 · 지연 기울기 · REMB 의 최솟값 — 앞서 알린 값과 5% 넘게 다를 때 · context 202611/20261108d E4).
    /// ★SDK 는 인코딩하지 않는다 — 인코더를 맞추는 것은 앱 몫이고 SDK 는 버리거나 늦추지 않는다
    TargetBitrate { server: String, bps: u64 },
    /// 발언권 누름 쪽(SDK:IMPL-FLR-EXPOSE) — event = granted · denied · queued · revoked · state · `ptt` = 그 걸음 뒤 값(사유 · 허가 시간 · 순번 — `denied` 면 `last_deny` · `revoked` 면 `last_revoke` 가 이 사건의 사유)
    Ptt { room: String, event: String, ptt: PttInfo },
    /// 발언 방이 바뀌었다(소속 응답 — None = 없다)
    SpeakingRoom { room: Option<String> },
    /// 채팅 메시지(연:CHAT.op.MESSAGE 실시간 · 연:CHAT.op.MESSAGES 묶음 — 공간 멤버면 방 입장과 무관하게 온다 · 같은 `msg_id` 가 두 번 올 수 있다 — 연:RULE-CHAT-RECV).
    /// `source` 는 출처 — 실시간 `MESSAGE` 와 혼잡할 때 묶인 `MESSAGES{live:true}` 는 [`ChatSource::Live`] · `live` 칸 없는 묶음은 [`ChatSource::Catchup`](SDK:IMPL-CHAT-CATCHUP · 연:RULE-CHAT-CATCHUP-BUNDLE).
    /// `kind` 는 글이면 없다 · 관계 메시지 `edit` · `react` · `delete`(연:RULE-CHAT-KIND — 접지 않고 그대로) · 멤버 변경 `join` · `leave` · `add` · `remove`(연:RULE-CHAT-MEMBER-NOTICE) · 모르는 종류도 문자열 그대로(연:RULE-CHAT-UNKNOWN-KIND).
    /// `target` 은 추가 · 뺌의 대상 · `reply_to` 는 가리키는 원본(wire `ref`) · `keys` 는 반응 전체 · `ext` 는 앱 몫(연:RULE-CHAT-EXT)
    Chat {
        space: String,
        msg_id: String,
        user_id: String,
        t: u64,
        content: String,
        kind: Option<String>,
        target: Option<String>,
        reply_to: Option<String>,
        keys: Option<Vec<String>>,
        ext: Option<serde_json::Value>,
        source: ChatSource,
    },
    /// 내 권한이 바뀌었다(PARTICIPANT_STATE — 투명 참가자도 본인에겐 온다)
    Permission { room: String, permission: Option<Permission> },
    /// 녹음 원시 기록이 멈췄다(SDK:RULE-PLAT-REC-PART 디스크 하한 — `disk_low`)
    RecordStopped { reason: String },
    /// 라이브 레코드 통이 넘쳐 멈췄다 — 원시 기록을 끈 구성이라 갈아탈 꼬리가 없다(SDK:RULE-PLAT-REC-API 역압 · `overflow`)
    RecordConsumerStopped { reason: String },
}

/// 채팅 사건의 출처(SDK:IMPL-CHAT-API `messages` 의 `source` 와 같은 이름 — oxpeer 에는 `history` 가 없다)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatSource {
    /// 실시간 — `MESSAGE` 한 건 · 혼잡할 때 묶인 `MESSAGES{live:true}`(연:RULE-CHAT-RECV)
    Live,
    /// 따라잡기 — `live` 칸 없는 `MESSAGES`(연:RULE-CHAT-CATCHUP)
    Catchup,
}

/// 발행할 트랙 — 코덱은 그 방의 미디어 코덱이다(연:RULE-PUB-PUBLISH-PROC — 앱이 고르는 칸이 없다)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackSpec {
    pub kind: Kind,
    /// `Full` = 전이중(늘 보낸다) · `Half` = 반이중(발언권 허가 사이에만 — 무전)
    pub duplex: Duplex,
    /// 영상 시뮬캐스트 — 업링크 둘(`l` = 0 · `h` = 1 · 연:RULE-PUB-RID-TO-SPATIAL) · 층마다 앱이 인코딩한 프레임을 `LocalTrack::write_layer` 로 넣는다(context 202611/20261108d E3)
    pub simulcast: bool,
}

/// 지금 값(SDK:IMPL-SES-API 상태 읽기 — 사건을 놓친 앱이 다시 맞춘다)
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub session: Option<SessionInfo>,
    pub speaking_room: Option<String>,
    pub rooms: Vec<RoomView>,
    /// 게이트가 닫혀 버린 프레임 · 보낸 프레임(발행 전부)
    pub gated: u64,
    pub sent: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoomView {
    pub room: String,
    pub joined: bool,
    pub server: Option<String>,
    pub speaker: Option<String>,
    pub ptt: PttInfo,
    pub permission: Option<Permission>,
    /// 이 방의 내 발행(track_id)
    pub published: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PttInfo {
    /// 위 클라 상태 이름(NOPERM · PREQ · PERM · QUEUED …)
    pub state: String,
    pub trusted: bool,
    /// 허가에 실린 발언 시간(초 — SDK:IMPL-FLR-EXPOSE `remainingSec` · 표의 기본값이 아니다) · 허가를 쥐었을 때만
    pub remaining_sec: Option<u32>,
    /// 요청 · 허가 우선순위(`priority`) · 허가를 쥐었거나 기다릴 때만
    pub priority: Option<u8>,
    /// 큐 순번(`queue.position` — 254 = 큐에 없음 · 255 = 미상) · 기다릴 때만
    pub queue_position: Option<u8>,
    /// 마지막 거절 · 회수 사유(`lastDeny` · `lastRevoke` — 회수 2 와 4 는 처방이 반대 · 연:RULE-FLR-APP-EXPOSE) — 다음 거절 · 회수가 덮는다
    pub last_deny: Option<FloorCause>,
    pub last_revoke: Option<FloorCause>,
}

/// 발언권 사유(연:RULE-FLR-TLV 사유 칸 · 글) — 번호는 `floor::deny` · `floor::revoke`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloorCause {
    pub cause: u16,
    pub text: Option<String>,
}

/// 발언권 상수 — 원장 생성본 그대로(손 상수로 옮겨 적지 않는다)
pub mod floor {
    pub use crate::generated::floor_consts::{deny, revoke};
    /// 서버 `T1`(End of RTP media) — 허가한 뒤 · RTP 마다 다시 걸리고 만료하면 화자에게 알리지 않고 회수한다(연:RULE-FLR-PROC-TAKEN-IDLE 5)
    pub const T1_MS: u64 = crate::generated::floor_consts::timers::T1_MS;
}

/// 수치 — 미디어 경로와 수신 정리층의 계수
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub media_dropped: u64,
    pub media_unrouted: u64,
    pub decrypt_fail: u64,
    pub frames_dropped: u64,
    /// 보낸 NACK 묶음 수 · RTX 로 메운 패킷 · 다 묻고도 못 받아 넘긴 패킷
    pub nack_sent: u64,
    pub rtx_recovered: u64,
    pub lost: u64,
    pub pli_sent: u64,
    pub rr_sent: u64,
    /// 조각이 빠져 버린 프레임 · 키프레임을 기다리느라 버린 프레임
    pub frames_broken: u64,
    pub frames_skipped: u64,
    /// QA 스위치가 버린 영상 패킷(`OXPEER_QA_DROP_VIDEO_EVERY` — 시험 수단 · 평소 0)
    pub qa_dropped: u64,
    /// 보내는 쪽 — RTX 로 다시 보낸 패킷 · 서버가 준 대역 상한(REMB) · 보낸 프레임 · 게이트가 닫혀 버린 프레임
    pub rtx_sent: u64,
    pub remb_bps: u64,
    /// 대역 추정 목표(bps · 발행 연결 중 가장 큰 것) · 마지막 1초 창 손실률(TWCC — 천분율)
    pub target_bps: u64,
    pub send_loss_permille: u64,
    pub frames_sent: u64,
    pub frames_gated: u64,
    /// QA 스위치가 보내지 않은 영상 패킷(`OXPEER_QA_DROP_SEND_EVERY` — 시험 수단 · 평소 0)
    pub qa_send_dropped: u64,
}

/// 내 발행 하나 — 앱이 인코딩한 프레임을 넣는다. 송신 시계가 프레임 길이대로 내보내고 · 통이 차면 `write` 가 기다린다(SDK:RULE-PLAT-SERVER 송신 시계)
pub struct LocalTrack {
    id: u64,
    pub room: String,
    pub track_id: String,
    /// 발행 코덱 — 그 방의 미디어 코덱(연:RULE-PUB-PUBLISH-PROC — 앱은 이 꼴로 인코딩해 넣는다 · `opus` · `VP8` · `H264`)
    pub codec: String,
    /// 층마다 송신 통(시뮬캐스트가 아니면 하나)
    frames: Vec<mpsc::Sender<OutFrame>>,
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl LocalTrack {
    /// 프레임 하나 — 통이 차면 기다린다(역압) · 시뮬캐스트 트랙은 `write_layer` 로 넣는다
    pub async fn write(&self, f: OutFrame) -> Result<(), Error> {
        if self.frames.len() != 1 {
            return Err(Error::sdk("LAYER_REQUIRED", crate::error::Category::State));
        }
        self.frames[0].send(f).await.map_err(|_| Error::closed("CLOSED"))
    }

    /// 시뮬캐스트 층 하나(0 = `l` · 1 = `h`)에 프레임 하나 — 층마다 송신 시계가 따로다
    pub async fn write_layer(&self, layer: u8, f: OutFrame) -> Result<(), Error> {
        let tx = self.frames.get(usize::from(layer)).ok_or_else(|| Error::sdk("NO_SUCH_LAYER", crate::error::Category::State))?;
        tx.send(f).await.map_err(|_| Error::closed("CLOSED"))
    }

    /// 층 수(시뮬캐스트면 2)
    pub fn layers(&self) -> usize {
        self.frames.len()
    }

    /// 넣은 프레임을 송신 시계가 다 내보낼 때까지 기다린다 — 발언을 놓기 전에 부른다(놓으면 게이트가 닫혀 통에 남은 것은 버려진다)
    pub async fn drain(&self) {
        while self.frames.iter().any(|f| f.capacity() < f.max_capacity()) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        // 통에서 꺼낸 마지막 프레임이 제 때를 기다리고 있을 수 있다 — 한 프레임 길이(넉넉히)
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    }

    /// 반이중 ↔ 전이중(SDK:IMPL-PUB-API `setDuplex` · TRACK_SET) — 응답이 오면 돌아온다
    pub async fn set_duplex(&self, duplex: Duplex) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Cmd::SetDuplex(self.id, duplex == Duplex::Half, tx)).map_err(|_| Error::closed("CLOSED"))?;
        rx.await.map_err(|_| Error::closed("CLOSED"))?
    }

    /// 발행을 내린다(`LocalTrack.stop()`)
    pub fn unpublish(self) {
        let _ = self.cmd.send(Cmd::Unpublish(self.id));
    }
}

/// 클라이언트 핸들 — 복제해도 같은 SDK 루프다
#[derive(Clone)]
pub struct Client {
    cmd: mpsc::UnboundedSender<Cmd>,
}

/// 사건 통
pub struct Events {
    rx: mpsc::UnboundedReceiver<Event>,
}

impl Events {
    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}

/// 전이 기록 한 줄 — 생성 상태기의 걸음 하나 · 받지 않는 사건이면 `impossible` (SDK:RULE-DIAG-EVENTS `transition` · SDK:IMPL-LINK-PROJECTION)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    /// 유닉스 ms
    pub at_ms: u64,
    /// 영역 — SES · ROOM · AFF · LINK · SUB · PUB · TRK · FLR
    pub area: String,
    /// 그 영역의 인스턴스 열쇠(방 · 서버 · 트랙 …)
    pub key: String,
    pub from: String,
    /// 받지 않는 사건이면 없다
    pub to: Option<String>,
    /// 규격의 경우 라벨(`SUB.C.WANT.nego_ok` 따위) · 받지 않는 사건이면 없다
    pub label: Option<String>,
    /// 까닭 — 입구 이름
    pub cause: String,
    /// 그 상태에서 받지 않는 사건이었다(조용히 버리지 않고 알린다)
    pub impossible: bool,
}

/// 전이 통 — `Client::transitions` 로 연다(앱 사건 통과 따로 · 여는 쪽만 받는다)
pub struct Transitions {
    rx: mpsc::UnboundedReceiver<Transition>,
}

impl Transitions {
    pub async fn next(&mut self) -> Option<Transition> {
        self.rx.recv().await
    }
}

/// 미디어 통 — 받은 RTP(복호한 것) · 단위가 `Packet` 인 트랙
pub struct Media {
    rx: mpsc::Receiver<MediaPacket>,
}

impl Media {
    pub async fn next(&mut self) -> Option<MediaPacket> {
        self.rx.recv().await
    }
}

/// 프레임 통 — 단위가 `Frame` 인 트랙
pub struct Frames {
    rx: mpsc::Receiver<MediaFrame>,
}

impl Frames {
    pub async fn next(&mut self) -> Option<MediaFrame> {
        self.rx.recv().await
    }
}

impl Client {
    /// 만들기 — 유일한 생성 경로(SDK:api.md `createClient`) · 아직 붙지 않는다. SDK 루프는 앱이 `Client` 를 놓을 때 끝난다
    pub fn create(cfg: Config) -> Result<(Client, Events, Media, Frames), Error> {
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let (media_tx, media_rx) = mpsc::channel(cfg.media_capacity.max(16));
        let (frames_tx, frames_rx) = mpsc::channel(cfg.frames_capacity.max(16));
        let cmd = core::spawn(cfg, ev_tx, media_tx, frames_tx)?;
        Ok((Client { cmd }, Events { rx: ev_rx }, Media { rx: media_rx }, Frames { rx: frames_rx }))
    }

    /// 접속 — 세션이 ACTIVE 에 닿으면 돌아온다(SDK:IMPL-SES-API `connect`). 최초 접속의 토큰 요구 · 거절은 `Err`(auth).
    /// ★`close()` 뒤 · `Closed` 사건 뒤에 같은 객체로 다시 부를 수 있다 — 새 세션이다(방은 앱이 다시 `join` · SDK:ses.md `close()`)
    pub async fn connect(&self) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::Connect(tx))?;
        rx.await.map_err(|_| Error::closed("CLOSED"))?
    }

    /// 만들고 붙기(지름길) — `create` 다음 `connect`
    pub async fn open(cfg: Config) -> Result<(Client, Events, Media, Frames), Error> {
        let (c, e, m, f) = Client::create(cfg)?;
        c.connect().await?;
        Ok((c, e, m, f))
    }

    fn send(&self, c: Cmd) -> Result<(), Error> {
        self.cmd.send(c).map_err(|_| Error::closed("CLOSED"))
    }

    /// 방 입장 — 듣기(`select:false`)로 들어간다(SDK:IMPL-ROOM-API 기본 `listen`). 들어가 첫 협상이 끝나면 돌아온다
    pub async fn join(&self, room: &str) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::Join(room.to_string(), tx))?;
        rx.await.map_err(|_| Error::closed("CLOSED"))?
    }

    /// 그 방만 나간다
    pub async fn leave(&self, room: &str) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::Leave(room.to_string(), tx))?;
        rx.await.map_err(|_| Error::closed("CLOSED"))?
    }

    /// 새 토큰 — 재접속 중 토큰 요구로 멈췄으면 이어서 접속한다
    pub fn set_token(&self, token: impl Into<String>) -> Result<(), Error> {
        self.send(Cmd::SetToken(token.into()))
    }

    /// 끊기 — 위 입구 disconnect(방마다 따로 나가지 않는다) · 기다리지 않는다 · 이 뒤 같은 객체로 `connect` 를 다시 부를 수 있다
    pub async fn close(&self) {
        let (tx, rx) = oneshot::channel();
        if self.send(Cmd::Close(tx)).is_ok() {
            let _ = tokio::time::timeout(Duration::from_secs(2), rx).await;
        }
    }

    /// 이 방에서 말하려고 들어간다(발언 방 — select:true 입장 · 이미 발언 방이 있으면 들어간 뒤 옮긴다)
    pub async fn join_speaking(&self, room: &str) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::JoinSpeaking(room.to_string(), tx))?;
        rx.await.map_err(|_| Error::closed("CLOSED"))?
    }

    /// 발언 방을 들어가 있는 이 방으로(소속 전환 — 연:AFF)
    pub async fn set_speaking_room(&self, room: &str) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::SetSpeakingRoom(room.to_string(), tx))?;
        rx.await.map_err(|_| Error::closed("CLOSED"))?
    }

    /// 발행 — 서버가 받으면(`published`) 돌아온다. 전이중은 곧 보내고 · 반이중은 발언권 허가 사이에만 보낸다
    pub async fn publish(&self, room: &str, spec: TrackSpec) -> Result<LocalTrack, Error> {
        let n = if spec.simulcast { 2 } else { 1 };
        let (ftx, frx): (Vec<_>, Vec<_>) = (0..n).map(|_| mpsc::channel(crate::core::FRAME_QUEUE)).unzip();
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::Publish(room.to_string(), spec, frx, tx))?;
        let (id, track_id, codec) = rx.await.map_err(|_| Error::closed("CLOSED"))??;
        Ok(LocalTrack { id, room: room.to_string(), track_id, codec, frames: ftx, cmd: self.cmd.clone() })
    }

    /// 발언 누름(SDK:IMPL-FLR-BUTTON `ptt.press`) — 위 입구에 닿으면 돌아온다 · 허가는 사건 `Ptt{granted}` 로
    pub async fn ptt_press(&self, room: &str, priority: u8) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::PttPress(room.to_string(), priority, tx))?;
        rx.await.map_err(|_| Error::closed("CLOSED"))?
    }

    /// 발언 놓음(`ptt.release`)
    pub async fn ptt_release(&self, room: &str) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::PttRelease(room.to_string(), tx))?;
        rx.await.map_err(|_| Error::closed("CLOSED"))?
    }

    /// 채팅 보내기(연:CHAT.op.MESSAGE) — ★실패 뒤 같은 메시지를 다시 보내려면 처음부터 [`new_msg_id`] 로 지어 넘긴다(연:RULE-CHAT-SEND 3 · 4) — `msg_id` 는 SDK 가 짓는다(hex 36 자 — 연:RULE-CHAT-MSG-ID) · 서버가 저장할 때 매긴 `t`.
    /// 같은 메시지를 다시 보낼 때는 `msg_id` 를 넘긴다(같은 내용이면 처음 `t` — 연:RULE-CHAT-SEND)
    pub async fn send_chat(&self, space: &str, content: &str, msg_id: Option<&str>) -> Result<u64, Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::Chat(space.to_string(), content.to_string(), msg_id.map(str::to_string), tx))?;
        rx.await.map_err(|_| Error::closed("CLOSED"))?
    }

    /// 따라잡기(연:CHAT.op.CATCHUP) — 앱이 부른다 · 서버 응답에 끝나고 못 받은 것은 그 뒤 [`Event::Chat`] 으로 온다. `space` · `t` 를 넣고 빼는 것이 곧 고르기(연:RULE-CHAT-CATCHUP)
    pub async fn chat_catchup(&self, space: Option<&str>, t: Option<u64>) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::ChatCatchup(space.map(str::to_string), t, tx))?;
        rx.await.map_err(|_| Error::closed("CLOSED"))?.map(|_| ())
    }

    /// 지금 값
    pub async fn snapshot(&self) -> Result<Snapshot, Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::Snapshot(tx))?;
        rx.await.map_err(|_| Error::closed("CLOSED"))
    }

    /// 이 트랙을 받는 단위 — 다음 패킷부터
    pub fn set_unit(&self, track_id: impl Into<String>, unit: Unit) -> Result<(), Error> {
        self.send(Cmd::SetUnit(track_id.into(), unit))
    }

    /// 받을 층 — 서버가 받으면 돌아온다. 입장한 방의 받는 트랙만(`NO_TRACK`) · 다시 들어가면 SDK 가 다시 보낸다
    pub async fn set_layer(&self, track_id: &str, layer: Layer) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Cmd::SetLayer(track_id.to_string(), layer, tx))?;
        rx.await.map_err(|_| Error::closed("CLOSED"))?
    }

    /// 녹화 조각 경계처럼 주기 키프레임이 없는 발행자(브라우저)에게 쓴다. 오디오 · 모르는 트랙은 아무 일도 없다
    pub fn request_keyframe(&self, track_id: impl Into<String>) -> Result<(), Error> {
        self.send(Cmd::RequestKey(track_id.into()))
    }

    /// 전이 기록을 받는 통을 연다 — 연 뒤의 걸음 전량(SDK:RULE-DIAG-EVENTS `transition`). 여러 번 열 수 있다
    pub fn transitions(&self) -> Transitions {
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = self.send(Cmd::Diag(tx));
        Transitions { rx }
    }

    /// 라이브 레코드 통을 연다(지점 ① — SDK:RULE-PLAT-REC-API) — 연 뒤의 레코드 전량 · `capacity` 를 넘으면 원시 파트 꼬리 읽기로 갈아탄다.
    #[cfg(feature = "record")]
    pub fn records(&self, capacity: usize) -> crate::record::Records {
        let (tx, rx) = mpsc::channel(capacity.max(16));
        let shared = std::sync::Arc::new(crate::record::Shared::default());
        let _ = self.send(Cmd::Records(tx, shared.clone()));
        crate::record::Records::new(rx, shared)
    }

    /// 앱 운영 로그 한 줄 — SDK 와 같은 로거 · 같은 꼴(머리 · `session` · `user` 는 SDK 가 붙인다 · SDK:RULE-PLAT-SERVER 운영 로그) · 기다리지 않는다
    pub fn log(&self, line: crate::log::AppLine) {
        let _ = self.send(Cmd::Log(line));
    }

    pub async fn stats(&self) -> Stats {
        let (tx, rx) = oneshot::channel();
        if self.send(Cmd::Stats(tx)).is_err() {
            return Stats::default();
        }
        rx.await.unwrap_or_default()
    }
}

/// 채팅 `msg_id` 짓기 — 소문자 hex 36 자 = SHA-256(`user_id`) 앞 4 byte · unix ms 6 byte · 보안 난수 8 byte(연:RULE-CHAT-MSG-ID · 저장된 상태에 기대지 않는 값). 실패 뒤 다시 보낼 때 같은 값을 넘기려면 이것으로 먼저 짓는다
pub fn new_msg_id(user_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(user_id.as_bytes());
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let ms = (now.as_millis() as u64) & 0xffff_ffff_ffff;
    let mut r = [0u8; 8];
    if getrandom::fill(&mut r).is_err() {
        r.copy_from_slice(&(now.as_nanos() as u64).to_be_bytes());
    }
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    format!("{}{ms:012x}{}", hex(&h[..4]), hex(&r))
}
