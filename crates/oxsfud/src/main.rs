// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§14-1 · §15-6 · §18-1 · model: claude-opus-5

//! `oxsfud` — 미디어 서버 프로세스.
//!
//! ★**기동 신원**(`epoch` = `sfu_id` = §15-1 `{inst}`)을 여기서 발급해 hub 에 알린다 —
//! ★**기동마다 새 값**이라야 재기동을 가릴 수 있고, 운영 `kill` 의 확인값이 이 값이다.

use common::Args;

fn main() -> std::process::ExitCode {
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

    // 미디어 축은 다음 걸음이다 — 지금은 서서 채널만 지킨다.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3_600));
    }
}
