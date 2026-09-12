// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§7-3 · §7-4 · §8-1 · model: claude-opus-5

//! fan-out 판정 — ★★**발언권의 강제 권위는 여기다.**
//!
//! ★**통지(DC)가 늦거나 유실돼도 미디어는 새지 않는다** — 권위는 이 게이트 하나다.
//! ★**핫패스라 판정만 하고 할당하지 않는다**(H3).

/// 발행 갈래.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Duplex {
    /// 회의 — 상시 흐른다.
    Full,
    /// 무전 — ★**발언권이 있을 때만** 흐른다.
    Half,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Audio,
    Video,
}

/// 한 패킷의 방 결정 재료.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet<'a> {
    pub publisher: &'a str,
    pub duplex: Duplex,
    pub kind: Kind,
    /// 등록 방 — ★**전이중이 보는 값.**
    pub registered_room: &'a str,
    /// ★**반이중이 보는 값** — 매 패킷 RCU 로 읽는다(전환이 값 교체 하나로 끝난다).
    pub pub_room: Option<&'a str>,
}

/// 그 방의 발언권 상태에서 판정에 쓰는 것.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloorView<'a> {
    pub speaker: Option<&'a str>,
}

/// 구독자 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subscriber<'a> {
    pub user_id: &'a str,
    pub room: &'a str,
    /// ★**전이중 video 에만** 걸리는 게이트(첫 키프레임 전 막아 둔 자리).
    pub video_gate_open: bool,
}

/// 왜 안 보냈나 — ★**사유별로 센다**(조용한 drop 금지).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drop {
    /// ★**발언권이 없다** — 이것이 강제 권위의 실현이다.
    NoFloor,
    /// 발행자의 `pub_room` 이 그 방이 아니다.
    NotPubRoom,
    /// ★**슬롯을 화자 본인이 구독한다** — 빼지 않으면 자기 목소리가 되돌아온다.
    SelfEcho,
    /// 그 방 구독자가 아니다.
    OtherRoom,
    /// ★**전이중 video 게이트** — audio 는 여기로 죽지 않는다.
    VideoGate,
}

/// 이 패킷이 흐를 방. ★**갈래마다 다르다.**
pub fn target_room<'a>(p: &Packet<'a>) -> Option<&'a str> {
    match p.duplex {
        // ★반이중은 매 패킷 `pub_room` 이다 — 등록 방이 아니다.
        Duplex::Half => p.pub_room,
        Duplex::Full => Some(p.registered_room),
    }
}

/// ★**prefan** — 반이중이 흐를 자격이 있나.
///
/// 산출 조건은 둘이다: ★**그 방 발언권 화자 == 발행자** ∧ ★**방 == `pub_room`**.
pub fn prefan_ok(p: &Packet<'_>, floor: FloorView<'_>) -> Result<(), Drop> {
    if p.duplex != Duplex::Half {
        return Ok(());
    }
    let Some(room) = p.pub_room else { return Err(Drop::NotPubRoom) };
    if target_room(p) != Some(room) {
        return Err(Drop::NotPubRoom);
    }
    if floor.speaker != Some(p.publisher) {
        return Err(Drop::NoFloor);
    }
    Ok(())
}

/// 이 구독자에게 보낼까. ★**판정만 한다** — 보내는 것은 부르는 쪽이다.
pub fn deliver_to(p: &Packet<'_>, floor: FloorView<'_>, sub: &Subscriber<'_>) -> Result<(), Drop> {
    prefan_ok(p, floor)?;
    let Some(room) = target_room(p) else { return Err(Drop::NotPubRoom) };
    if sub.room != room {
        return Err(Drop::OtherRoom);
    }
    match p.duplex {
        // ★슬롯은 N:1 이라 화자도 그 m-line 을 구독한다 — 본인을 빼야 한다.
        Duplex::Half if sub.user_id == p.publisher => Err(Drop::SelfEcho),
        Duplex::Half => Ok(()),
        Duplex::Full => {
            // ★게이트는 video 에만. ★**audio 는 어떤 경우에도 게이트로 죽지 않는다.**
            if p.kind == Kind::Video && !sub.video_gate_open {
                return Err(Drop::VideoGate);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn half<'a>(pubr: Option<&'a str>) -> Packet<'a> {
        Packet {
            publisher: "spk",
            duplex: Duplex::Half,
            kind: Kind::Audio,
            registered_room: "r-reg",
            pub_room: pubr,
        }
    }

    fn full(kind: Kind) -> Packet<'static> {
        Packet {
            publisher: "pubr",
            duplex: Duplex::Full,
            kind,
            registered_room: "r1",
            pub_room: Some("r1"),
        }
    }

    fn sub<'a>(user: &'a str, room: &'a str, gate: bool) -> Subscriber<'a> {
        Subscriber { user_id: user, room, video_gate_open: gate }
    }

    #[test]
    fn 발언권이_없으면_안_나간다() {
        // ★통지가 늦거나 유실돼도 미디어는 새지 않는다 — 권위는 이 게이트 하나다.
        let p = half(Some("r1"));
        let none = FloorView { speaker: None };
        assert_eq!(deliver_to(&p, none, &sub("u2", "r1", true)), Err(Drop::NoFloor));
        let other = FloorView { speaker: Some("남") };
        assert_eq!(deliver_to(&p, other, &sub("u2", "r1", true)), Err(Drop::NoFloor));
        let mine = FloorView { speaker: Some("spk") };
        assert_eq!(deliver_to(&p, mine, &sub("u2", "r1", true)), Ok(()));
    }

    #[test]
    fn 반이중은_매_패킷_pub_room_을_본다() {
        // ★등록 방을 읽으면 pub_select 뒤에도 소리가 옛 방으로 간다.
        let p = half(Some("r2"));
        assert_eq!(target_room(&p), Some("r2"), "★등록 방 r-reg 가 아니다");
        let f = FloorView { speaker: Some("spk") };
        assert_eq!(deliver_to(&p, f, &sub("u2", "r-reg", true)), Err(Drop::OtherRoom));
        assert_eq!(deliver_to(&p, f, &sub("u2", "r2", true)), Ok(()));
    }

    #[test]
    fn 발행_방이_없으면_흐를_곳이_없다() {
        let p = half(None);
        let f = FloorView { speaker: Some("spk") };
        assert_eq!(deliver_to(&p, f, &sub("u2", "r1", true)), Err(Drop::NotPubRoom));
    }

    #[test]
    fn 슬롯은_화자_본인을_뺀다() {
        // ★안 빼면 자기 목소리가 되돌아온다(N:1 슬롯을 화자도 구독한다).
        let p = half(Some("r1"));
        let f = FloorView { speaker: Some("spk") };
        assert_eq!(deliver_to(&p, f, &sub("spk", "r1", true)), Err(Drop::SelfEcho));
    }

    #[test]
    fn 전이중은_등록_방이다() {
        let p = full(Kind::Audio);
        assert_eq!(target_room(&p), Some("r1"));
        let f = FloorView { speaker: None };
        // ★전이중은 발언권과 무관하다.
        assert_eq!(deliver_to(&p, f, &sub("u2", "r1", true)), Ok(()));
    }

    #[test]
    fn 게이트는_video_에만_걸린다() {
        let f = FloorView { speaker: None };
        // ★audio 는 어떤 경우에도 게이트로 죽지 않는다.
        assert_eq!(deliver_to(&full(Kind::Audio), f, &sub("u2", "r1", false)), Ok(()));
        assert_eq!(
            deliver_to(&full(Kind::Video), f, &sub("u2", "r1", false)),
            Err(Drop::VideoGate)
        );
    }

    #[test]
    fn 사유를_가려_센다() {
        // ★합만 내면 어느 관문에서 막혔는지를 잃는다(정§16-2).
        let f = FloorView { speaker: Some("spk") };
        let p = half(Some("r1"));
        let drops = [
            deliver_to(&p, FloorView { speaker: None }, &sub("u2", "r1", true)),
            deliver_to(&p, f, &sub("spk", "r1", true)),
            deliver_to(&p, f, &sub("u2", "다른방", true)),
        ];
        assert_eq!(drops[0], Err(Drop::NoFloor));
        assert_eq!(drops[1], Err(Drop::SelfEcho));
        assert_eq!(drops[2], Err(Drop::OtherRoom));
    }
}
