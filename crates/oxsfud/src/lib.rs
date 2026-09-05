// author: kodeholic (powered by Claude)
//! sfud — 정§1 "두 프로세스" 중 미디어 서버. 방·Peer·트랙·발언권·version 의 상태 마스터(정§2-1).
//! 이 판: 방·명단·version·소속·통지(정§4·§5·§14) + 전송 수립(정§12·§13·§17). 트랙·발언권은 뒤 판.

pub mod emit;
pub mod handlers;
pub mod media;
pub mod peer;
pub mod room;
pub mod service;
pub mod transport;
pub mod version;
