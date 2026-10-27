// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-CFG-THREE-PLACES · 정:IMPL-CFG-SYSTEM-FILE · 운영:RULE-BOOT-ARGS · 운영:RULE-BOOT-PORTS · model: claude-opus-5-5
//! hub 값의 자리 셋 — 기동 인자(인자 > 파일 > 코드 기본값) · 시스템 파일 · 정책 파일. 부팅 1회 로드 · 배너 한 줄.

use common::args::{Args, Spec, flag, opt};
use common::banner::Banner;
use common::policy::{Policy, Reader};
use common::system::SystemFile;

pub const ARGS: &[Spec] = &[opt("system"), opt("policy"), opt("id"), opt("build"), opt("listen"), opt("log-dir"), opt("log-socket"), flag("version")];

#[derive(Debug, Clone)]
pub struct HubCfg {
    pub system: SystemFile,
    pub policy: Policy,
    pub node_id: String,
    pub listen: String,
    pub base_path: String,
    pub log_dir: Option<String>,
    pub log_socket: Option<String>,
    pub build_label: Option<String>,
    pub build: String,
    pub args: Args,
}

impl HubCfg {
    pub fn load(args: Args) -> Result<HubCfg, String> {
        let system_path = args.get("system").unwrap_or("system.toml").to_string();
        let system = SystemFile::load(&system_path)?;
        let policy = match args.get("policy") {
            Some(p) => Policy::load(p).map_err(|e| e.to_string())?,
            None => Policy::default(),
        };
        let node_id = match args.get("id") {
            Some(id) => id.to_string(),
            None => system.first_sfu().map(|u| u.id.clone()).ok_or("node_id 를 정할 수 없다 — --id 도 sfu 유닛도 없다")?,
        };
        common::bus::keys::check_fragment("--id", &node_id)?;
        let listen = args.get("listen").map(str::to_string).unwrap_or_else(|| system.hub.listen.clone());
        let log_dir = args.get("log-dir").map(str::to_string).or_else(|| system.hub.log_dir.clone());
        let log_socket = args.get("log-socket").map(str::to_string).or_else(|| system.hub.log_socket.clone());
        let base_path = system.hub.base_path.trim_end_matches('/').to_string();
        let build_label = args.get("build").map(str::to_string);
        let build = common::build::identity(build_label.as_deref());
        Ok(HubCfg { system, policy, node_id, listen, base_path, log_dir, log_socket, build_label, build, args })
    }

    pub fn banner(&self) -> Banner {
        let mut b = Banner::new();
        for name in ["system", "policy", "id", "build", "listen", "log-dir", "log-socket"] {
            b.put_opt(name, self.args.get(name));
        }
        self.system.banner(&mut b);
        self.policy.banner(&mut b, Reader::Hub);
        b
    }
}
