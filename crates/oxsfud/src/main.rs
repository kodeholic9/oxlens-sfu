// author: kodeholic (powered by Claude)
//! 기동 — 정책 파일 1회 로드(정§18-1) · 프로세스 인증서·epoch 발급(정§14-1) · UDP 전송 포트(정§12) · gRPC `SfuService` · 회수 tick(정§17-1).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::bplane::SfuServiceServer;
use common::config::PolicyConfig;
use oxsfud::handlers::{AutoLayer, BweMode, MediaParams, RTCP_REPORT_INTERVAL_MS, TWCC_INTERVAL_MS, Sfu, now_ms};
use oxsfud::media::nack::RETRY_MS as NACK_TICK_MS;
use oxsfud::service::Service;
use oxsfud::peer::REAPER_TICK_MS;
use oxsfud::transport::{ServerCert, udp};
use oxsfud::version::new_epoch;
use tracing::info;

struct Args {
    policy: PathBuf,
    id: String,
    grpc_listen: String,
    udp_port: u16,
    public_ip: String,
}

fn parse_args() -> Args {
    let mut a = Args { policy: "policy.toml".into(), id: "sfu-1".into(), grpc_listen: "127.0.0.1:50061".into(), udp_port: 20000, public_ip: "127.0.0.1".into() };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let v = it.next().unwrap_or_default();
        match k.as_str() {
            "--policy" => a.policy = v.into(),
            "--id" => a.id = v,
            "--grpc-listen" => a.grpc_listen = v,
            "--udp-port" => a.udp_port = v.parse().unwrap_or(20000),
            "--public-ip" => a.public_ip = v,
            _ => {
                eprintln!("usage: oxsfud [--policy policy.toml] [--id sfu-1] [--grpc-listen 127.0.0.1:50061] [--udp-port 20000] [--public-ip 127.0.0.1]");
                std::process::exit(2);
            }
        }
    }
    a
}

#[tokio::main]
async fn main() {
    let args = parse_args();
    let policy = PolicyConfig::load(&args.policy).unwrap_or_else(|e| {
        eprintln!("policy config: {e}");
        std::process::exit(2);
    });
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::new(policy.logging.level.clone())).init();
    let cert = ServerCert::generate().unwrap_or_else(|e| {
        eprintln!("dtls certificate: {e}");
        std::process::exit(2);
    });
    let epoch = new_epoch();
    info!(id = %args.id, epoch = %epoch, grpc = %args.grpc_listen, udp = %format!("{}:{}", args.public_ip, args.udp_port),
        fingerprint = %cert.fingerprint, max_bitrate = policy.media.max_bitrate_bps, "oxsfud up");
    let cert = Arc::new(cert);
    let sfu = Arc::new(Sfu::new(epoch, MediaParams { public_ip: args.public_ip, udp_port: args.udp_port, fingerprint: cert.fingerprint.clone(), bwe_mode: BweMode::parse(&policy.media.bwe_mode), auto_layer: AutoLayer::parse(&policy.media.auto_layer), max_bitrate_bps: u64::from(policy.media.max_bitrate_bps) }, cert));

    let socket = match udp::bind(args.udp_port).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("udp bind {}: {e}", args.udp_port);
            std::process::exit(2);
        }
    };
    sfu.attach_socket(socket.clone());
    tokio::spawn(udp::run(sfu.clone(), socket));

    // 정§11-1 상향 — 재전송 요구는 눈금이 다르다(200ms). RR 과 같은 타이머에 태우면 늦다.
    let nacker = sfu.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(NACK_TICK_MS));
        loop {
            tick.tick().await;
            nacker.emit_nacks(now_ms()).await;
        }
    });

    // 정§11-2 — Ingress TWCC 는 눈금이 더 촘촘하다(100ms). RR 에 태우면 추정이 안 따라온다.
    let bwe = sfu.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(TWCC_INTERVAL_MS));
        loop {
            tick.tick().await;
            bwe.emit_transport_feedback().await;
        }
    });

    // 정§11-2 — Ingress RR 은 서버가 자체 생성한다. 소비자는 ★이 타이머 하나다.
    let reporter = sfu.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(RTCP_REPORT_INTERVAL_MS));
        loop {
            tick.tick().await;
            reporter.emit_receiver_reports(now_ms()).await;
            // 정§11-2 — REMB 는 RR 과 같은 눈금이다(1초).
            reporter.emit_remb().await;
            // 정§10-3 — 레이어 판단도 1초 눈금이다(판정 해상도 ≈2s).
            reporter.auto_layer_tick(now_ms());
        }
    });

    let reaper = sfu.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(REAPER_TICK_MS));
        loop {
            tick.tick().await;
            reaper.tick();
        }
    });

    let addr = args.grpc_listen.parse().unwrap_or_else(|e| {
        eprintln!("grpc listen: {e}");
        std::process::exit(2);
    });
    if let Err(e) = tonic::transport::Server::builder().add_service(SfuServiceServer::new(Service { sfu })).serve(addr).await {
        eprintln!("grpc server: {e}");
        std::process::exit(1);
    }
}
