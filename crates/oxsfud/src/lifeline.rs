// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§15-6 · §16-1 · model: claude-opus-5

//! 부모 생존 채널 — ★★**hub 가 죽으면 sfu 도 죽는다.**
//!
//! ★**부모가 죽이는 방식은 강제 종료를 못 덮는다**(규격이 그것을 미리 적어 뒀다).
//! 그래서 ★**자식이 스스로 본다** — 부모가 쥔 파이프의 쓰기 끝이 닫히면 읽기 끝이 EOF 가 되고,
//! 그것은 ★**부모가 어떤 방식으로 죽든**(정상 종료·패닉·`SIGKILL`) 성립한다.
//!
//! ★**채널은 자식의 표준입력이다** — hub 는 거기에 아무것도 쓰지 않는다. 쓰지 않는 것이 계약이라
//! *"무엇이 오나"* 를 정할 필요가 없고, 자식은 ★**EOF 하나만** 본다.

use std::io::Read;

/// 채널이 끊긴 뒤 할 일.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fate {
    /// 부모가 살아 있다(채널이 열려 있다).
    Alive,
    /// ★**부모가 갔다 — 스스로 끝낸다.**
    ParentGone,
}

/// 읽기 끝을 지켜본다. ★**한 바이트도 기대하지 않는다 — EOF 만 본다.**
pub fn watch<R: Read>(mut r: R) -> Fate {
    let mut buf = [0u8; 64];
    loop {
        match r.read(&mut buf) {
            // EOF — 쓰기 끝이 전부 닫혔다.
            Ok(0) => return Fate::ParentGone,
            // ★무엇이 와도 무시한다 — 이 채널은 뜻을 나르지 않는다.
            Ok(_) => continue,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // 읽을 수 없게 된 것도 끊긴 것이다.
            Err(_) => return Fate::ParentGone,
        }
    }
}

/// 표준입력을 채널로 삼아 지켜보다가, 끊기면 프로세스를 끝낸다.
///
/// ★**`--no-lifeline` 으로 끌 수 있다** — 사람이 손으로 띄워 볼 때뿐이고, supervisor 는 항상 켠다.
pub fn spawn_watch_stdin() -> std::thread::JoinHandle<()> {
    std::thread::spawn(|| {
        if watch(std::io::stdin()) == Fate::ParentGone {
            eprintln!("[lifeline] ★부모 생존 채널이 끊겼다 — 스스로 끝낸다");
            // ★고아로 남으면 시그널 없이 RTP 만 흐르고 아무도 그것을 거두지 않는다.
            std::process::exit(0);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 끝나면_부모가_간_것이다() {
        // 빈 읽기 = EOF.
        assert_eq!(watch("".as_bytes()), Fate::ParentGone);
    }

    #[test]
    fn 무엇이_와도_뜻으로_읽지_않는다() {
        // ★이 채널은 뜻을 나르지 않는다 — 다 읽고 EOF 에서만 판정한다.
        assert_eq!(watch("ping\n무엇이든".as_bytes()), Fate::ParentGone);
    }
}
