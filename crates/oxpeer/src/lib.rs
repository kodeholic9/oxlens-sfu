// author: kodeholic (powered by Claude)
//! oxpeer — OxLens 서버사이드 SDK(SDK:RULE-PLAT-SERVER).
//! 녹음 · 녹화 · AI 에이전트 같은 서버 쪽 프로그램이 보통 클라와 같은 계약(연동규격서)으로 방에 들어가 미디어를 받고 발언권 · 방 사건을 듣는다.
//! 서버 코어는 이 SDK 를 모른다 — 참가자 종류(녹음 · 봇)와 투명은 앱이 받은 토큰의 클레임이 정한다.
//!
//! ```no_run
//! # async fn f() -> Result<(), oxpeer::Error> {
//! let req = oxpeer::TokenReq { api_key: "k".into(), api_secret: "s".into(), user_id: "rec-1".into(), participant_type: Some(1), hidden: Some(true), metadata: None, permission: None };
//! let token = oxpeer::issue_token("http://127.0.0.1:19745/media", &req).await?;
//! let (client, mut events, mut media, _frames) = oxpeer::Client::open(oxpeer::Config::new("http://127.0.0.1:19745/media", token)).await?;
//! client.join("ch-1").await?;
//! while let Some(p) = media.next().await { let _ = (p.track_id, p.payload); }
//! # let _ = events.next().await; Ok(()) }
//! ```
//! 범위(context 202611/20261106c A3 · 20261107a · 20261107c): 세션 · 연결 · 방 · 받는 트랙 · 구독 + 발언권 듣기 · 수신 정리층(재정렬 · NACK · RTX · PLI · RR · SR) ·
//! 프레임 단위 수신 · 층 고르기 · ★말하기 — 발행(앱이 인코딩한 프레임 · 송신 시계 · 음량 표시 · SR · RTX 응답 · PLI 사건) · 발언 방(소속) · 발언 누름 · 메시지 · 지금 값.

mod api;
mod core;
mod error;
mod generated;
mod http;
mod link;
pub mod log;
#[cfg(feature = "record")]
pub mod record;
mod sig;
pub mod tls;
mod token;
mod wire;

#[cfg(all(test, feature = "serde"))]
mod vectors_test;

pub use api::{floor, new_msg_id, ChatSource, Client, Config, Duplex, Event, Events, FloorCause, Frames, Health, Kind, Layer, LinkInfo, LinkState, LocalTrack, Media, MediaFrame, MediaPacket, MemberInfo, OutFrame, Permission, PttInfo, RoomView, SessionInfo, SessionState, Snapshot, Source, Stats, TokenReq, TrackEntry, TrackEntryType, TrackSpec, Transition, Transitions, Unit};
pub use error::{Category, Error};
pub use token::{issue_token, issue_token_with_root};
pub use generated::session_client::{LADDER as SESSION_LADDER, Ladder};
