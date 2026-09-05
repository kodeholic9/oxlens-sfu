// author: kodeholic (powered by Claude)
//! hub — 정§1 "두 프로세스" 중 게이트웨이. 세션(정§3)·클라 WS 접속 축(연§3·§6-1·§10-3)·운영 HTTP(연§5).
//! 방·Peer·트랙·발언권의 상태 마스터는 sfud 다 — hub 는 세션과 배치·라우팅만 든다.

pub mod backend;
pub mod conn;
pub mod rest;
pub mod session;
pub mod ws;
