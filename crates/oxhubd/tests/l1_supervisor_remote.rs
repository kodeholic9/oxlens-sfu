// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-OPS-SURFACE · 정:RULE-OPS-SUPERVISOR · 운영:RULE-CTL-SUPERVISOR · model: claude-opus-5-5
//! 1층 감독자 — remote 유닛(같은 기계의 sfud 에 붙기만 한다). 실제 버스 위에서 감독자가 가짜 Handle(기동 신원을 바꿀 수 있다)에 붙는다: 포트를 기다리지 않고 붙는다 · 붙은 동안 `Hello` 로 끊김과 기동 신원 바뀜을 보고 `Down` 으로 간 뒤 다시 붙는다 · `stop` 은 떼기(`Stopped` · 로컬 Handle 비움) · `kill` 은 떼고 다시 붙기(context 202611/20261124b 수정 갈래 B I12).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::envelope::Reply;
use common::envelope::handle::handle_server::{Handle, HandleServer};
use common::envelope::handle::{CallReq, CallRes, HelloReq, HelloRes};
use common::system::{Unit, UnitKind};
use oxhubd::bus::{Bus, BusCfg};
use oxhubd::ops::supervisor::{Role, SupCfg, Supervisor, UnitState};

struct Fake {
    epoch: Arc<Mutex<String>>,
}

#[tonic::async_trait]
impl Handle for Fake {
    async fn hello(&self, _req: tonic::Request<HelloReq>) -> Result<tonic::Response<HelloRes>, tonic::Status> {
        let epoch = self.epoch.lock().map(|e| e.clone()).unwrap_or_default();
        Ok(tonic::Response::new(HelloRes { epoch, build: "fake".into() }))
    }

    async fn call(&self, _req: tonic::Request<CallReq>) -> Result<tonic::Response<CallRes>, tonic::Status> {
        Ok(tonic::Response::new(CallRes { reply: Reply::ok(serde_json::json!({})).encode() }))
    }
}

fn serve(addr: std::net::SocketAddr, epoch: Arc<Mutex<String>>) -> tokio::sync::oneshot::Sender<()> {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(tonic::transport::Server::builder().add_service(HandleServer::new(Fake { epoch })).serve_with_shutdown(addr, async {
        let _ = rx.await;
    }));
    tx
}

fn set_epoch(epoch: &Mutex<String>, v: &str) {
    if let Ok(mut e) = epoch.lock() {
        e.clear();
        e.push_str(v);
    }
}

async fn until(s: &Supervisor, id: &str, within: Duration, want: impl Fn(UnitState, Option<String>) -> bool) -> (bool, Vec<UnitState>, Duration) {
    let t0 = Instant::now();
    let mut seen = Vec::new();
    while t0.elapsed() < within {
        let u = s.unit(id).expect("unit");
        let (st, ep) = (u.state(), u.epoch());
        if seen.last() != Some(&st) {
            seen.push(st);
        }
        if want(st, ep) {
            return (true, seen, t0.elapsed());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (false, seen, t0.elapsed())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remote_unit_attaches_watches_and_detaches() {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("port");
    let addr = probe.local_addr().expect("addr");
    drop(probe);
    let epoch = Arc::new(Mutex::new("sfu-e1".to_string()));
    let mut stop_server = Some(serve(addr, epoch.clone()));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (bus, _events) = Bus::open("n1", "e1", 1_000, BusCfg { listen: vec!["127.0.0.1:0".into()], advertise: vec![], connect: vec![], namespace: "tremote".into() }).await.expect("bus");
    let (fatal, _f) = tokio::sync::mpsc::unbounded_channel();
    let cfg = SupCfg {
        node: "n1".into(),
        hub_inst: "i1".into(),
        build_label: None,
        backoff_ms: vec![200],
        intensity_max: 50,
        intensity_window: Duration::from_secs(60),
        hello_timeout: Duration::from_secs(2),
        extra_args: Vec::new(),
        sfu_args: Vec::new(),
        rec: None,
    };
    let r = Unit { id: "r1".into(), kind: UnitKind::Remote, order: 1, role: "sfu".into(), addr: Some(addr.to_string()), cmd: Vec::new(), enabled: true };
    let s = Supervisor::new(&[r], cfg, fatal);
    s.mark_state(Role::Bus, UnitState::Running, "l1");
    s.start_sfu(bus.clone());

    let (ok, seen, took) = until(&s, "r1", Duration::from_secs(20), |st, ep| st == UnitState::Running && ep.as_deref() == Some("sfu-e1")).await;
    assert!(ok, "stand check — the remote unit attaches: {seen:?}");
    assert!(took < Duration::from_secs(3), "S-sym-RULE-OPS-SURFACE-1③ a remote unit does not wait for the port to free — its sfud is already listening ({took:?} · {seen:?})");
    assert!(bus.local().is_some_and(|l| l.epoch == "sfu-e1"), "stand check — the attached unit is the hub's local Handle");

    set_epoch(&epoch, "sfu-e2");
    let (ok, seen, _) = until(&s, "r1", Duration::from_secs(6), |st, ep| st == UnitState::Running && ep.as_deref() == Some("sfu-e2")).await;
    assert!(ok && seen.contains(&UnitState::Down), "S-sym-RULE-OPS-SURFACE-1② a changed boot identity behind the same address takes the unit Down, then it attaches to the new identity: {seen:?}");

    if let Some(tx) = stop_server.take() {
        let _ = tx.send(());
    }
    let (ok, seen, took) = until(&s, "r1", Duration::from_secs(5), |st, _| st == UnitState::Down).await;
    assert!(ok, "S-sym-RULE-OPS-SURFACE-1① a remote sfud that went away takes the unit Down — Running would keep ready at 200 ({seen:?} · {took:?})");
    set_epoch(&epoch, "sfu-e3");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let _back = serve(addr, epoch.clone());
    let (ok, seen, _) = until(&s, "r1", Duration::from_secs(8), |st, ep| st == UnitState::Running && ep.as_deref() == Some("sfu-e3")).await;
    assert!(ok, "S-sym-RULE-OPS-SURFACE-1① the sfud that came back is attached again with its new identity: {seen:?}");

    let got = tokio::time::timeout(Duration::from_secs(2), s.request("r1", "stop")).await;
    assert!(matches!(got, Ok(Ok((UnitState::Running, UnitState::Stopped)))), "S-sym-RULE-OPS-SURFACE-1④ stop on a remote unit answers Running → Stopped: {got:?}");
    assert!(bus.local().is_none(), "S-sym-RULE-OPS-SURFACE-1④ stop detaches — the local Handle is emptied");
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert_eq!(s.unit("r1").map(|u| u.state()), Some(UnitState::Stopped), "S-sym-RULE-OPS-SURFACE-1④ a stopped remote unit stays Stopped — it is not attached again");

    let got = tokio::time::timeout(Duration::from_secs(2), s.request("r1", "load")).await;
    assert!(matches!(got, Ok(Ok((UnitState::Stopped, UnitState::Starting)))), "stand check — load attaches again: {got:?}");
    let (ok, seen, _) = until(&s, "r1", Duration::from_secs(5), |st, _| st == UnitState::Running).await;
    assert!(ok, "stand check — Running after load: {seen:?}");

    let got = tokio::time::timeout(Duration::from_secs(2), s.request("r1", "kill")).await;
    assert!(matches!(got, Ok(Ok((UnitState::Running, UnitState::Down)))), "S-sym-RULE-OPS-SURFACE-1⑤ kill on a remote unit answers Running → Down: {got:?}");
    let (ok, seen, _) = until(&s, "r1", Duration::from_secs(5), |st, ep| st == UnitState::Running && ep.as_deref() == Some("sfu-e3")).await;
    assert!(ok && seen.contains(&UnitState::Down), "S-sym-RULE-OPS-SURFACE-1⑤ kill detaches and attaches again — the same sfud, nothing was killed: {seen:?}");
}
