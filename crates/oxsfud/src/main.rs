// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§14-1 · §15-6 · §18-1 · model: claude-opus-5

//! `oxsfud` — 미디어 서버 프로세스.
//!
//! ★**기동 신원**(`epoch` = `sfu_id` = §15-1 `{inst}`)을 여기서 발급해 hub 에 알린다 —
//! ★**기동마다 새 값**이라야 재기동을 가릴 수 있고, 운영 `kill` 의 확인값이 이 값이다.

use common::Args;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    // `--no-lifeline` 은 사람이 손으로 띄울 때만 쓴다 — supervisor 는 항상 채널을 준다.
    let no_lifeline = argv.iter().any(|a| a == "--no-lifeline");
    let argv: Vec<String> = argv.into_iter().filter(|a| a != "--no-lifeline").collect();

    // ★이 바이너리만 아는 인자 — 정§18-1 CLI 칸(포트·public-ip)의 sfud 몫.
    const MINE: &[&str] = &["--grpc-listen", "--udp-port", "--public-ip"];
    let args = match Args::parse_allowing(&argv, MINE) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return std::process::ExitCode::from(2);
        }
    };
    if args.version {
        println!("{}", common::BuildId::new(args.build.clone()).line());
        return std::process::ExitCode::SUCCESS;
    }
    let Some(node_id) = args.id.clone() else {
        eprintln!("args: --id 가 없다 — node_id 는 안정 배치 키라 필수다");
        return std::process::ExitCode::from(2);
    };

    // ★정책은 배포 하나에 하나다 — hub 와 ★**같은 파일**을 읽는다(두 벌을 두면 값이 갈린다).
    let Some(policy_path) = args.policy.clone() else {
        eprintln!("args: --policy 가 없다 — `[media]` 값을 지어낼 수는 없다");
        return std::process::ExitCode::from(2);
    };
    let policy = match std::fs::read_to_string(&policy_path)
        .map_err(|e| format!("policy {policy_path}: {e}"))
        .and_then(|s| common::Policy::parse(&s).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return std::process::ExitCode::from(2);
        }
    };

    // ★기동마다 새 값 — 설정의 고정 별칭을 쓰지 않는다(어기면 클라가 `seq` 재시작을 못 가린다).
    let epoch = format!("sfu-{}", uuid::Uuid::new_v4().simple());

    eprintln!(
        "[boot] node_id={node_id} epoch={epoch} build={} lifeline={} grpc={} udp={} ip={}",
        common::BuildId::new(args.build.clone()).line(),
        if no_lifeline { "off" } else { "on" },
        args.extra.get("--grpc-listen").map(String::as_str).unwrap_or("-"),
        args.extra.get("--udp-port").map(String::as_str).unwrap_or("-"),
        args.extra.get("--public-ip").map(String::as_str).unwrap_or("-"),
    );

    if !no_lifeline {
        // ★부모가 어떤 방식으로 죽든 성립한다 — 부모가 죽이는 방식은 강제 종료를 못 덮는다.
        oxsfud::lifeline::spawn_watch_stdin();
    }

    // ★B 평면을 연다 — 이것이 답하는 순간이 `Running` 이다(정§16-1).
    let Some(addr) = args.extra.get("--grpc-listen").cloned() else {
        eprintln!("args: --grpc-listen 이 없다 — B 평면 없이는 hub 가 이 유닛을 못 본다");
        return std::process::ExitCode::from(2);
    };
    let listen: std::net::SocketAddr = match addr.parse() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("args: --grpc-listen {addr}: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    // ★**지문은 프로세스 것이다** — 재협상마다 바뀌면 클라가 연결을 새로 세운다.
    let dtls = match oxsfud::identity::Dtls::bake() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return std::process::ExitCode::from(1);
        }
    };
    // ★`--public-ip` 가 없으면 루프백이다 — 개발 형상이고, 상용은 설정이 준다.
    let ip = args.extra.get("--public-ip").cloned().unwrap_or_else(|| "127.0.0.1".into());
    let udp: u16 = match args.extra.get("--udp-port").map(|s| s.parse()) {
        Some(Ok(v)) => v,
        Some(Err(e)) => {
            eprintln!("args: --udp-port {e}");
            return std::process::ExitCode::from(2);
        }
        None => {
            eprintln!("args: --udp-port 가 없다 — 미디어를 받을 자리가 없다");
            return std::process::ExitCode::from(2);
        }
    };
    let node = oxsfud::handle::Node::new(
        epoch.clone(),
        dtls.clone(),
        ip.clone(),
        udp,
        policy.media.max_bitrate_bps as u64,
    );

    // ★★**광고하는 IP 로 바인드한다.** 응답의 **출발 주소**가 요청의 **목적 주소**와 같아야
    //   하고(RFC 5389 §7.3.1), `0.0.0.0` 으로 열면 커널이 경로를 보고 다른 IP 를 고른다 —
    //   클라는 그것을 ★**source address mismatch** 로 읽고 그 후보를 버린다(실측 20260912).
    //
    //   ★**못 붙으면 `0.0.0.0`** 이다 — 광고값이 이 기계의 주소가 아닌 배치(NAT 뒤 공인 IP)가
    //   그 경우이고, 거기서는 NAT 이 출발 주소를 도로 공인 IP 로 바꿔 준다. 어느 쪽인지
    //   로그가 말한다(조용히 갈리지 않는다).
    let sock = match tokio::net::UdpSocket::bind((ip.as_str(), udp)).await {
        Ok(v) => {
            eprintln!("[udp] listen {ip}:{udp}");
            std::sync::Arc::new(v)
        }
        Err(e) => match tokio::net::UdpSocket::bind(("0.0.0.0", udp)).await {
            Ok(v) => {
                eprintln!("[udp] listen 0.0.0.0:{udp} — 광고값 {ip} 는 이 기계 것이 아니다({e}). NAT 배치로 본다");
                std::sync::Arc::new(v)
            }
            Err(e) => {
                eprintln!("udp {udp}: {e}");
                return std::process::ExitCode::from(1);
            }
        },
    };
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(64);
    let (dc_tx, dc_rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(oxsfud::transport::udp::serve(
        sock,
        node.ice.clone(),
        dtls.cert,
        cmd_tx.clone(),
        cmd_rx,
        dc_tx,
    ));
    let svc = std::sync::Arc::new(oxsfud::grpc::Sfu::new(
        oxsfud::grpc::Identity {
            epoch,
            build: common::BuildId::new(args.build.clone()).line(),
        },
        node,
        cmd_tx,
    ));
    svc.spawn_reaper();
    svc.spawn_floor(dc_rx);
    eprintln!("[b] listen {listen}");
    if let Err(e) = tonic::transport::Server::builder()
        .add_service(svc.into_server())
        .serve(listen)
        .await
    {
        eprintln!("[b] {e}");
        return std::process::ExitCode::from(1);
    }
    std::process::ExitCode::SUCCESS
}
