// author: kodeholic (powered by Claude)
// spec: v1.14 · 정:IMPL-OPS-REC-UNIT · model: claude-opus-5-5
//! 녹음 유닛 실행 파일 — 기본 등록부(`talk` · `tracks` · `raw`)로 `oxtapd::unit` 을 띄운다.

#[tokio::main]
async fn main() {
    oxtapd::unit::main_with(oxtapd::Registry::defaults()).await
}
