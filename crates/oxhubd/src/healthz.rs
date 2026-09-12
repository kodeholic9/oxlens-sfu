// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§16-1 · 운영 §3-1 · model: claude-opus-5

//! `/healthz` — ★**두 축을 한 이름으로 합치지 않는다.**
//!
//! ★★**`유닛 live` 는 supervisor 가 쥔 그 유닛의 상태가 `Running` 인 것**이고,
//! ★**`노드 live` 는 그 노드의 `node` 토큰이 서 있는 것**이다(B 평면).
//! `node` 토큰은 ★**hub 가 선언한다** — 그 값으로 sfud 의 생존을 판정하면
//! ★**hub 가 살아 있는 한 언제나 참**이라 유닛이 죽어도 안 보인다.

use crate::supervisor::{Supervisor, UnitState};

/// `/healthz/ready` 의 답.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    pub ok: bool,
    /// ★**어느 유닛이 빠졌는지까지 낸다** — 503 만 주면 운영자가 다시 물어봐야 한다.
    pub down: Vec<String>,
}

/// hub 가 정상이고 ★**그 hub 가 쥔 enabled 유닛 전부가 `유닛 live`** 면 200.
///
/// ★**로컬 물음이다** — 타 node 의 사정은 여기 안 들어온다.
pub fn ready(sup: &Supervisor, hub_ok: bool) -> Ready {
    let down: Vec<String> = sup
        .units
        .iter()
        // ★설정이 끈 유닛은 묻지 않는다 — 그것은 결정이지 장애가 아니다.
        .filter(|u| u.state != UnitState::Disabled && !u.state.is_live())
        .map(|u| u.id.clone())
        .collect();
    Ready { ok: hub_ok && down.is_empty(), down }
}

/// `/healthz/live` — ★**프로세스 생존 = 항상 200 · 무인증**(probe 가 부른다).
///
/// ★`ready` 와 가르는 것이 요점이다 — 이쪽이 200 인데 저쪽이 503 이면
/// *"떠는 있는데 일을 못 한다"* 가 한눈에 드러난다.
pub const fn live() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::system::{Unit, UnitKind};

    fn unit(id: &str, enabled: bool) -> Unit {
        Unit {
            id: id.into(),
            kind: UnitKind::Process,
            order: 1,
            role: String::new(),
            cmd: vec!["x".into()],
            addr: String::new(),
            enabled,
        }
    }

    #[test]
    fn 유닛이_죽으면_503_이다() {
        // ★두 축을 합치면 sfud 가 죽어도 ready 가 200 이다.
        let mut s = Supervisor::new(&[unit("sfu-1", true)]);
        s.start_all(0);
        s.on_ready("sfu-1", "e".into());
        assert!(ready(&s, true).ok);

        s.on_exit("sfu-1", 10);
        let r = ready(&s, true);
        assert!(!r.ok);
        assert_eq!(r.down, vec!["sfu-1".to_string()], "★어느 유닛이 빠졌는지까지 낸다");
    }

    #[test]
    fn 띄우는_중에는_아직_아니다() {
        // ★`Starting` 은 프로세스가 생겼다는 뜻뿐이다 — 붙었다는 뜻이 아니다.
        let mut s = Supervisor::new(&[unit("sfu-1", true)]);
        s.start_all(0);
        assert!(!ready(&s, true).ok);
    }

    #[test]
    fn 설정이_끈_유닛은_안_묻는다() {
        let s = Supervisor::new(&[unit("off", false)]);
        assert!(ready(&s, true).ok, "★결정이지 장애가 아니다");
    }

    #[test]
    fn live_와_ready_는_다른_물음이다() {
        let s = Supervisor::new(&[unit("sfu-1", true)]);
        assert!(live(), "★프로세스가 떴으면 항상 200");
        assert!(!ready(&s, true).ok, "★일은 아직 못 한다");
    }
}
