// author: kodeholic (powered by Claude)
//! 기동 — 정책 파일 1회 로드(정§18-1) · 프로세스 인증서·epoch 발급(정§14-1) · gRPC `SfuService` · room sweep(정§17-1).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::bplane::SfuServiceServer;
use common::config::PolicyConfig;
use oxsfud::handlers::{MediaParams, Sfu};
use oxsfud::service::Service;
use oxsfud::transport::ServerCert;
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
    let sfu = Arc::new(Sfu::new(epoch, MediaParams { public_ip: args.public_ip, udp_port: args.udp_port, fingerprint: cert.fingerprint.clone(), max_bitrate_bps: u64::from(policy.media.max_bitrate_bps) }));

    let sweeper = sfu.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(5_000));
        loop {
            tick.tick().await;
            sweeper.sweep();
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
