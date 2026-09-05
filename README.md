# oxlens-sfu

OxLens 서버 코어 — Cargo workspace. `crates/oxsig` = 연동규격서 wire 계층(프레임·op·코드·스키마·DC·MBCP). 공유 벡터 정본은 `oxlens-spec/vectors/`.

`crates/common` = 설정(strict)·JWT·흐름 큐·B 평면(tonic). `crates/oxhubd` = 세션·라우팅·HTTP. `crates/oxsfud` = 방·Peer·version 상태 마스터(미디어는 이행 중).
개발 실증: `dev/system.dev.toml` 로 hub 를, `--grpc-listen 127.0.0.1:50061` 로 sfud 를 절대 경로로 띄운다(`cargo build --offline`).
