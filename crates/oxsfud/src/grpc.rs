// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§15-0 · §15-4 · §15-5 · §16-1 · model: claude-opus-5

//! B 평면 서버 — ★**node 안의 유일한 길**이다.
//!
//! ★**`Hello` 가 답하는 순간이 그 유닛의 `Running`** 이다(정§16-1). 그 전은 `Starting` 이고,
//! ★**둘을 합치면** *"띄웠다"* 와 *"붙었다"* 가 한 값이 되어 급사를 못 본다.

use common::b::sfu_service_server::{SfuService, SfuServiceServer};
use common::b::{Envelope, HelloReply, HelloRequest, RoomLedger, RoomView, SubscribeRequest};
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, Mutex};
use tonic::{Request, Response, Status};

use crate::handle::{self, Ingress, Node};
use crate::room::{Room, Ttl};

/// 이 프로세스의 신원. ★**기동마다 새 값**이라 재기동을 가릴 수 있다.
#[derive(Debug, Clone)]
pub struct Identity {
    pub epoch: String,
    pub build: String,
}

#[derive(Debug)]
pub struct Sfu {
    id: Identity,
    node: Mutex<Node>,
    /// 통지 방송. ★**hub 마다 하나씩 받아 간다** — 한 node 에 hub 는 하나이지만
    /// 재접속 창에 둘이 겹칠 수 있어 방송으로 둔다.
    tx: broadcast::Sender<Envelope>,
    /// 전송 루프에 거는 손 — ★**회수는 통로를 닫는 것**이다(정§12).
    udp: mpsc::Sender<crate::transport::udp::Cmd>,
}

/// ★**밀린 통지의 상한** — 넘으면 그 구독자만 갭을 본다(`seq` 로 드러나고 §14-3 이 메운다).
/// ★**막지 않는다**(정§15-4 `Drop`) — 이벤트 하나를 지키려고 스트림을 죽이지 않는다.
const NOTICE_LAG: usize = 1024;

impl Sfu {
    pub fn new(id: Identity, node: Node, udp: mpsc::Sender<crate::transport::udp::Cmd>) -> Self {
        let (tx, _) = broadcast::channel(NOTICE_LAG);
        Self { id, node: Mutex::new(node), tx, udp }
    }

    pub fn into_server(self: Arc<Self>) -> SfuServiceServer<Self> {
        SfuServiceServer::from_arc(self)
    }

    /// ★**미디어가 죽은 Peer 를 거둔다**(정§2-2 reaper) — 주기는 5초 하나다.
    pub fn spawn_reaper(self: &Arc<Self>) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(std::time::Duration::from_millis(
                crate::reaper::TICK_MS,
            ));
            loop {
                iv.tick().await;
                let reaped = {
                    let mut node = me.node.lock().await;
                    handle::reaper_tick(&mut node, now_ms())
                };
                for r in reaped {
                    for n in r.notices {
                        me.emit(n);
                    }
                    // ★**태스크 종료까지가 회수다**(정§17-2 ④ · 실사고 20260814).
                    let _ = me.udp.send(crate::transport::udp::Cmd::DropSession(r.session_id)).await;
                }
            }
        });
    }

    /// 통지 한 장을 스트림으로. ★**보낼 곳이 없으면 버린다** — 막지 않는다(정§15-4 `Drop`).
    fn emit(&self, n: handle::Notice) {
        let _ = self.tx.send(Envelope {
            room_id: n.room_id,
            exclude: n.exclude,
            target: n.target.unwrap_or_default(),
            wire: n.wire,
            ..Default::default()
        });
    }
}

#[tonic::async_trait]
impl SfuService for Sfu {
    /// ★**기동 신원을 건넨다** — hub 가 이것을 받아 들고 `Running` 으로 적는다.
    async fn hello(&self, req: Request<HelloRequest>) -> Result<Response<HelloReply>, Status> {
        let who = req.into_inner().node_id;
        eprintln!("[b] hello ← hub {who}");
        Ok(Response::new(HelloReply {
            epoch: self.id.epoch.clone(),
            build: self.id.build.clone(),
        }))
    }

    /// 요청 하나 → 응답 wire 하나. ★**통지는 응답에 섞지 않고 스트림으로 간다.**
    async fn handle(&self, req: Request<Envelope>) -> Result<Response<Envelope>, Status> {
        let env = req.into_inner();
        let (header, body) = oxsig::frame::decode(&env.wire)
            // ★hub 가 이미 읽은 프레임이다 — 여기서 깨졌으면 우리 버그다.
            .map_err(|e| Status::invalid_argument(format!("wire: {e:?}")))?;
        let ing = Ingress {
            session_id: env.session_id.clone(),
            user_id: env.user_id.clone(),
            participant_type: env.participant_type as u8,
            hidden: env.hidden,
            metadata: serde_json::from_str(&env.metadata).ok(),
            pc_mode: match env.pc_mode.as_str() {
                "1pc" => oxsig::body::session::PcMode::One,
                _ => oxsig::body::session::PcMode::Two,
            },
        };
        let out = {
            let mut node = self.node.lock().await;
            handle::dispatch(&mut node, &ing, header, body)
        };
        for n in out.notices {
            self.emit(n);
        }
        Ok(Response::new(Envelope { wire: out.reply, ..Default::default() }))
    }

    /// 방 대장 반영 + 현황. ★**zenoh 토큰의 대역**이다(정§15-2).
    async fn rooms(&self, req: Request<RoomLedger>) -> Result<Response<RoomView>, Status> {
        let led = req.into_inner();
        let now = now_ms();
        let mut node = self.node.lock().await;
        for r in led.put {
            // ★**멱등** — 있으면 그 방이 그대로다(명단·`seq` 를 지우지 않는다).
            node.rooms.create(
                r.room_id,
                r.name,
                r.capacity,
                Ttl { unused_secs: r.unused_ttl_secs, departure_secs: r.departure_ttl_secs },
                now,
            );
        }
        for id in led.drop {
            node.rooms.remove(&id);
        }
        // ★**한 tick 에 한 걸음** — 비는 것을 본 tick 에는 만료를 판정하지 않는다.
        let expired = node.rooms.sweep(now);
        for id in &expired {
            node.rooms.remove(id);
        }
        let rooms = node.rooms.iter().map(|r| view_of(r, &node.epoch)).collect();
        Ok(Response::new(RoomView { epoch: node.epoch.clone(), rooms, expired }))
    }

    type SubscribeStream = tokio_stream::wrappers::ReceiverStream<Result<Envelope, Status>>;

    /// ★**sfud 가 직접 낸다** — hub 가 재발행하지 않는다(정§15-4).
    async fn subscribe(
        &self,
        req: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let who = req.into_inner().hub_id;
        eprintln!("[b] subscribe ← hub {who}");
        let mut rx = self.tx.subscribe();
        let (tx, out) = tokio::sync::mpsc::channel(NOTICE_LAG);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(e) => {
                        if tx.send(Ok(e)).await.is_err() {
                            break;
                        }
                    }
                    // ★밀려서 버려졌다 — ★**스트림은 살려 둔다.** 갭은 `seq` 가 드러낸다.
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("[b] 통지 {n} 건 밀려 버렸다 — seq 갭으로 드러난다");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(out)))
    }
}

/// 연§5-4 목록 항목 형 — ★**hub 가 대장에 덧씌운다.**
fn view_of(r: &Room, epoch: &str) -> String {
    // ★**`assign` 이 없다** — 배정은 수신자별 층이고(연§4-1-1) 이 현황은 방 것이라
    //   받는 사람이 정해져 있지 않다. 연§4-1 배포표도 HTTP 조회의 `assign` 을
    //   *"입장 중일 때만"* 으로 둔다 — 없는 자리를 지어내지 않는다.
    let slot = serde_json::json!({
        "type": "slot",
        "room_id": r.id,
        "track_id": format!("ptt-{}-audio", r.id),
        "kind": "audio",
        "ssrc": r.slot_audio_ssrc,
        "codec": "opus",
    });
    serde_json::json!({
        "room_id": r.id,
        "tracks": [slot],
        // ★보이는 수다(투명 제외).
        "user_count": r.user_count(),
        // ★녹화 사실은 감추지 않는다 — `hidden` 이어도 참이다.
        "rec": r.rec(),
        "participants": r.participants(),
        "version": r.version(epoch),
    })
    .to_string()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
