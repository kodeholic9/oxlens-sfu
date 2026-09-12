// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§18-1 · §3-4 · §15-0 · §16-1 · §16-1-1 · model: claude-opus-5

//! 시스템 파일 — ★**hub 전용.** 프로세스가 둘 떠도 **같아야 하는 값**이 여기다.
//!
//! 담는 것: `listen`·TLS·JWT 비밀·★계정 목록·라우팅·supervisor 유닛 목록·★zenoh·로그 디렉터리.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemError(pub String);

impl std::fmt::Display for SystemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "system: {}", self.0)
    }
}

impl std::error::Error for SystemError {}

/// 계정 하나 — ★**이 계정이 무엇을 서명할 수 있나.** 켜는 것이 명시적 결정이다.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKey {
    pub key: String,
    pub secret: String,
    #[serde(default)]
    pub name: String,
    /// 이 계정이 낼 수 있는 종류. 밖의 값을 요구하면 발급이 `2005` 다.
    #[serde(default)]
    pub participant_types: Vec<u8>,
    /// ★투명(`hidden`) 토큰을 낼 수 있나.
    #[serde(default)]
    pub hidden_allowed: bool,
    /// ★**운영 토큰**(`ops`)을 이 비밀로 서명할 수 있나 — hub 는 발급하지 않고 검증만 한다.
    #[serde(default)]
    pub ops_allowed: bool,
    /// ★**연동 토큰**(`hook`) 자격 — 대외 `ctl`·사실 구독.
    #[serde(default)]
    pub hook_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Auth {
    /// A 평면 사용자 토큰의 서명 비밀. ★**운영·연동 토큰은 이것으로 서명하지 않는다**(계정 비밀이다).
    pub jwt_secret: String,
    pub api_keys: Vec<ApiKey>,
}


/// 유닛 종류(정§16-1). ★**판단기는 하나다** — 실행기에 갈래가 하나 더 있을 뿐이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnitKind {
    /// 실행 명령이 있다 — 띄운다. ★**부모 생존 채널을 받는다.**
    Process,
    /// 명령 없이 붙는다.
    Remote,
    /// hub 프로세스 안에서 연다(zenoh 가 이것이다).
    Inproc,
}

/// supervisor 가 쥐는 유닛 하나.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub id: String,
    pub kind: UnitKind,
    /// ★**기동 순서. 정지는 역순이다** — zenoh 는 sfu 보다 먼저 서고 나중에 닫힌다(정§15-6).
    pub order: u8,
    #[serde(default)]
    pub role: String,
    /// `process` 면 실행 명령.
    #[serde(default)]
    pub cmd: Vec<String>,
    /// `remote`·`process` 의 gRPC 주소.
    #[serde(default)]
    pub addr: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// zenoh — ★**세션을 열 때 굳는 값**이라 정책 파일이 아니라 여기다.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Zenoh {
    /// hub 는 `router`, sfud 는 `client`. ★**`peer` 는 미사용**(관문이 무너진다).
    pub mode: String,
    pub listen: Vec<String>,
    /// seed — ★**기존 node 는 설정을 안 고친다**(이 목록만 는다).
    pub connect: Vec<String>,
    pub multicast_scouting: bool,
    /// ★**§15-0 전제 둘** — 기본값이지만 안 적으면 성능 튜닝 중에 꺼진다.
    pub qos_enabled: bool,
    pub lowlatency: bool,
    /// ★배포 이름공간 `{c}` — 한 버스에 여러 배포가 섞일 때 가르는 최상위 조각.
    pub namespace: String,
}

impl Default for Zenoh {
    fn default() -> Self {
        Self {
            mode: "router".into(),
            listen: Vec::new(),
            connect: Vec::new(),
            multicast_scouting: false,
            qos_enabled: true,
            lowlatency: false,
            namespace: "ox".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Dirs {
    /// ★**로그 디렉터리의 정본**(운영 규격서 §7) — `--log-dir` 이 덮는다.
    pub log: String,
}


#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HubSection {
    pub listen: String,
    pub base_path: String,
    pub auth: Auth,
}

impl Default for HubSection {
    fn default() -> Self {
        Self { listen: "0.0.0.0:1974".into(), base_path: "/media".into(), auth: Auth::default() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct System {
    pub hub: HubSection,
    #[serde(rename = "unit")]
    pub units: Vec<Unit>,
    pub zenoh: Zenoh,
    pub dirs: Dirs,
}


impl System {
    pub fn parse(text: &str) -> Result<System, SystemError> {
        let s: System = toml::from_str(text).map_err(|e| SystemError(e.to_string()))?;
        s.validate()?;
        Ok(s)
    }

    pub fn validate(&self) -> Result<(), SystemError> {
        if self.hub.auth.jwt_secret.is_empty() {
            return Err(SystemError("hub.auth.jwt_secret 이 비었다".into()));
        }
        let mut ids = std::collections::BTreeSet::new();
        for u in &self.units {
            if !ids.insert(u.id.as_str()) {
                return Err(SystemError(format!("유닛 id 중복: {}", u.id)));
            }
            // ★띄울 유닛인데 명령이 없으면 exec 가 매번 실패한다 — 기동 때 잡는다.
            if u.kind == UnitKind::Process && u.cmd.is_empty() {
                return Err(SystemError(format!("유닛 {} 는 process 인데 cmd 가 없다", u.id)));
            }
            if u.kind == UnitKind::Remote && u.addr.is_empty() {
                return Err(SystemError(format!("유닛 {} 는 remote 인데 addr 가 없다", u.id)));
            }
        }
        if self.zenoh.mode == "peer" {
            // ★peer 는 clique 강제라 node N 대면 세션 N(N−1)/2 순증하고 관문이 무너진다(정§15-0).
            return Err(SystemError("zenoh.mode=peer 는 미사용이다(정§15-0) — router/client 뿐".into()));
        }
        if !self.zenoh.qos_enabled || self.zenoh.lowlatency {
            // ★우선순위 큐가 합쳐지면 이벤트가 몰린 순간 토큰이 함께 죽는다.
            return Err(SystemError(
                "zenoh 는 qos_enabled=true · lowlatency=false 가 전제다(정§15-0)".into(),
            ));
        }
        Ok(())
    }

    /// 기동 순서 — ★**정지는 이 역순이다.**
    pub fn start_order(&self) -> Vec<&Unit> {
        let mut v: Vec<&Unit> = self.units.iter().filter(|u| u.enabled).collect();
        v.sort_by_key(|u| (u.order, u.id.as_str()));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: &str = r#"
[hub]
listen = "127.0.0.1:1974"
[hub.auth]
jwt_secret = "s"
"#;

    #[test]
    fn 최소_파일이_선다() {
        let s = System::parse(MIN).expect("parse");
        assert_eq!(s.hub.listen, "127.0.0.1:1974");
        assert!(s.units.is_empty());
        assert!(s.zenoh.qos_enabled, "★전제는 기본값이라도 명시적으로 참이어야 한다");
    }

    #[test]
    fn 모르는_키는_기동_실패다() {
        let e = System::parse("[hub]\nlisten_addr = \"x\"\n").unwrap_err();
        assert!(e.0.contains("listen_addr"), "{}", e.0);
    }

    #[test]
    fn peer_모드는_막는다() {
        let t = format!("{MIN}\n[zenoh]\nmode = \"peer\"\n");
        let e = System::parse(&t).unwrap_err();
        assert!(e.0.contains("peer"), "{}", e.0);
    }

    #[test]
    fn 전제_둘을_끄면_선다() {
        let t = format!("{MIN}\n[zenoh]\nlowlatency = true\n");
        let e = System::parse(&t).unwrap_err();
        assert!(e.0.contains("lowlatency"), "{}", e.0);
    }

    #[test]
    fn 명령_없는_process_유닛은_선다() {
        let t = format!("{MIN}\n[[unit]]\nid = \"sfu-1\"\nkind = \"process\"\norder = 2\n");
        let e = System::parse(&t).unwrap_err();
        assert!(e.0.contains("cmd"), "{}", e.0);
    }

    #[test]
    fn 기동_순서는_order_고_정지는_역순이다() {
        let t = format!(
            "{MIN}\n[[unit]]\nid = \"sfu-1\"\nkind = \"remote\"\norder = 2\naddr = \"a\"\n\
             [[unit]]\nid = \"zenoh\"\nkind = \"inproc\"\norder = 1\n"
        );
        let s = System::parse(&t).expect("parse");
        let ids: Vec<&str> = s.start_order().iter().map(|u| u.id.as_str()).collect();
        assert_eq!(ids, vec!["zenoh", "sfu-1"], "★zenoh 가 sfu 보다 먼저 선다");
    }

    #[test]
    fn 계정_허용은_켜는_것이_결정이다() {
        let t = format!(
            "{MIN}\n[[hub.auth.api_keys]]\nkey = \"k\"\nsecret = \"p\"\n"
        );
        let s = System::parse(&t).expect("parse");
        let a = &s.hub.auth.api_keys[0];
        assert!(!a.ops_allowed && !a.hidden_allowed && !a.hook_allowed, "★부재는 허용이 아니다");
    }
}
