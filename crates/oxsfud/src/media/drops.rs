// author: kodeholic (powered by Claude)
//! 정§16-2 「★조용한 drop 금지 — 버리는 전 경로가 사유별 계수를 갖는다」.
//!
//! ★같은 §16-2 의 「핫패스 카운팅 억제」와 부딪히지 않는다. 억제가 이름한 것은
//! **packets/bytes/jitter** 셋이고 그 권위는 클라 `getStats` 양단 비교다. drop 사유는
//! 그 셋이 아니고 ★**클라가 볼 수 없다** — 정§7-4 가 못박은 대로 *"수신자에겐 망 손실과
//! 구별 불가"* 다. 그래서 세는 자리가 서버밖에 없다.
//!
//! ★그리고 정§7-4 는 세야 할 이유까지 말한다 — gate 드롭은 *"서버가 만든 드롭이 자기
//! 판단으로 되돌아올 수 있다"*(자동 레이어가 RR 손실을 입력으로 쓴다). 제가 낸 구멍을
//! 서버가 모르면 그 구멍을 망 손실로 읽고 단을 내린다.
//!
//! # 가름 — 이 파일의 전부
//!
//! ★**버린 것**만 센다. **안 보낸 것**은 버린 게 아니다. 아래 셋은 정상 경로에서
//! 패킷마다 성립하므로, 세면 그것이 곧 핫패스 카운터다:
//!
//! | 안 세는 것 | 왜 |
//! |---|---|
//! | simulcast 비선택 단 | 그 구독자 몫이 아니다 — 애초에 배달 대상이 아니다 |
//! | 화자 본인 제외(슬롯 fan-out) | fan-out 집합에서 뺀 것이지 버린 게 아니다 |
//! | 이미 사라진 구독자(`Weak` 끊김) | 받을 사람이 없다 |
//!
//! 나머지 — ★**배달 대상인데 못/안 보냈다** — 만 센다. 정상 흐름에서는 하나도 안 오른다.

use std::sync::atomic::{AtomicU64, Ordering};

/// 스트림을 짓기 **전에** 버린 것 — 누구 것인지 아직 모르는 자리다.
#[derive(Debug, Default)]
pub struct SfuDrops {
    /// 등록된 전송로가 없는 주소에서 왔다.
    pub no_session: AtomicU64,
    /// 구독 연결로 RTP 가 왔다 — 받기만 하는 자리다(연§9-6).
    pub not_publisher: AtomicU64,
    pub srtp_decrypt: AtomicU64,
    /// ★`on_srtp` 주석이 *"미해소는 계수하고 버린다"* 고 적어 두고 계수가 없었다.
    pub srtcp_decrypt: AtomicU64,
    /// RTP 로 안 읽힌다.
    pub malformed: AtomicU64,
    /// 그 발행자가 약속(연§4-1)한 적 없는 ssrc — simulcast 학습으로도 안 붙었다.
    pub unknown_ssrc: AtomicU64,
}

/// 발행 축에서 버린 것 — 스트림은 아는데 내보내지 않기로 한 자리다.
#[derive(Debug, Default)]
pub struct PubDrops {
    pub muted: AtomicU64,
    /// 발행자가 어느 방에도 안 붙어 있다(`pub_room` 없음).
    pub no_room: AtomicU64,
    /// ★정§7-3 발언권 강제 — 이 계수가 0 이 아니면 클라 게이트가 새고 있다는 뜻이다.
    pub no_floor: AtomicU64,
    /// 방 슬롯에 안 물린다(kind·코덱 불일치, 정§8-1).
    pub slot_unbound: AtomicU64,
    /// 재기록기가 거절했다.
    pub rewrite_skip: AtomicU64,
}

/// 구독 축에서 버린 것 — ★받을 사람이 정해졌는데 그에게 못/안 갔다.
#[derive(Debug, Default)]
pub struct SubDrops {
    /// ★정§7-4 `READY` 게이트. 출력 seq 에 구멍이 남는다 — 서버가 만든 구멍이다.
    pub gate: AtomicU64,
    pub paused: AtomicU64,
    /// 목표 단인데 키프레임을 기다린다(정§10-2) — *"왜 안 올라가나"* 의 입력.
    pub awaiting_keyframe: AtomicU64,
    pub rewrite_skip: AtomicU64,
    /// 전송로가 아직/이미 없다.
    pub no_transport: AtomicU64,
    pub encrypt: AtomicU64,
    /// ★`send_to` 가 실패했다 — 종전엔 `is_ok()` 뒤에서 조용히 사라졌다.
    pub send_err: AtomicU64,
    /// 정§11-1 하향 — 재전송 요구를 거절한 사유 셋. ★한 번의 NACK 안에서만 살고 debug 로그로
    /// 흘러가 버렸다 — 사유가 사라지면 조용한 drop 과 같다.
    pub rtx_gate: AtomicU64,
    pub rtx_miss: AtomicU64,
    pub rtx_budget: AtomicU64,
}

/// 정§13 — DataChannel 에서 버린 것. ★종전엔 계수 하나에 두 사유가 뭉쳐 있었다.
#[derive(Debug, Default)]
pub struct DcDrops {
    /// 개통은 됐는데 보낼 큐가 찼다 — 느린 상대.
    pub full: AtomicU64,
    /// 아직 개통 전이라 쌓아 두다 64 를 넘겨 ★가장 오래된 것을 버렸다 — 영영 안 여는 상대.
    pub pending_overflow: AtomicU64,
}

/// 정§16-2 「★상태가 아니라 판단의 입력을 낸다 — *"왜 안 내려가나"* 를 밖에서 볼 수 있게」.
///
/// ★이것은 버린 패킷이 아니라 **판정을 건너뛴 사유**다 — 축이 달라서 `SubDrops` 와 섞지 않는다.
/// 정§14-3 정체 판정은 *"안 흐르는 게 정상인 창"* 을 전부 건너뛴다. 그런데 어느 창에서
/// 건너뛰었는지가 안 보이면, 통지가 안 왔을 때 ★**감지가 죽은 것인지 정상 유예인지 가를 수 없다.**
#[derive(Debug, Default)]
pub struct StallSkips {
    /// `READY` 전이라 아직 흐를 자리가 아니다(정§7-4).
    pub gate: AtomicU64,
    pub paused: AtomicU64,
    /// 발행 자체가 없다 — 슬롯 껍데기만 남았다.
    pub no_stream: AtomicU64,
    pub muted: AtomicU64,
    pub no_room: AtomicU64,
    /// 반이중인데 아무도 말하지 않는다 — 조용한 무전은 정상이다.
    pub no_speaker: AtomicU64,
    /// 화자 본인의 슬롯 구독 — 0 이 계약이다(정§7-3 ①).
    pub self_speaker: AtomicU64,
    /// 화자의 발언 방이 여기가 아니다.
    pub wrong_room: AtomicU64,
    /// 화자가 이 슬롯에 결합돼 있지 않다(코덱·kind).
    pub unbound: AtomicU64,
}

impl StallSkips {
    pub fn snapshot(&self) -> Vec<(&'static str, u64)> {
        pack(&[
            ("gate", &self.gate),
            ("paused", &self.paused),
            ("no_stream", &self.no_stream),
            ("muted", &self.muted),
            ("no_room", &self.no_room),
            ("no_speaker", &self.no_speaker),
            ("self_speaker", &self.self_speaker),
            ("wrong_room", &self.wrong_room),
            ("unbound", &self.unbound),
        ])
    }
}

/// 계수 하나 올린다. ★`Relaxed` — 순서를 견주는 값이 아니라 세는 값이다.
pub fn note(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

/// ★0 인 사유는 안 낸다 — 전부 실어 보내면 읽는 사람이 0 을 헤아려야 한다(연§6-6 과 같은 규율).
fn pack(rows: &[(&'static str, &AtomicU64)]) -> Vec<(&'static str, u64)> {
    rows.iter()
        .map(|(n, c)| (*n, c.load(Ordering::Relaxed)))
        .filter(|(_, v)| *v > 0)
        .collect()
}

impl SfuDrops {
    pub fn snapshot(&self) -> Vec<(&'static str, u64)> {
        pack(&[
            ("no_session", &self.no_session),
            ("not_publisher", &self.not_publisher),
            ("srtp_decrypt", &self.srtp_decrypt),
            ("srtcp_decrypt", &self.srtcp_decrypt),
            ("malformed", &self.malformed),
            ("unknown_ssrc", &self.unknown_ssrc),
        ])
    }
}

impl PubDrops {
    pub fn snapshot(&self) -> Vec<(&'static str, u64)> {
        pack(&[
            ("muted", &self.muted),
            ("no_room", &self.no_room),
            ("no_floor", &self.no_floor),
            ("slot_unbound", &self.slot_unbound),
            ("rewrite_skip", &self.rewrite_skip),
        ])
    }
}

impl SubDrops {
    pub fn snapshot(&self) -> Vec<(&'static str, u64)> {
        pack(&[
            ("gate", &self.gate),
            ("paused", &self.paused),
            ("awaiting_keyframe", &self.awaiting_keyframe),
            ("rewrite_skip", &self.rewrite_skip),
            ("no_transport", &self.no_transport),
            ("encrypt", &self.encrypt),
            ("send_err", &self.send_err),
            ("rtx_gate", &self.rtx_gate),
            ("rtx_miss", &self.rtx_miss),
            ("rtx_budget", &self.rtx_budget),
        ])
    }

    pub fn total(&self) -> u64 {
        self.snapshot().iter().map(|(_, v)| v).sum()
    }
}

impl DcDrops {
    pub fn snapshot(&self) -> Vec<(&'static str, u64)> {
        pack(&[("full", &self.full), ("pending_overflow", &self.pending_overflow)])
    }

    pub fn total(&self) -> u64 {
        self.full.load(Ordering::Relaxed) + self.pending_overflow.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quiet_path_reports_nothing_at_all() {
        assert!(SubDrops::default().snapshot().is_empty(), "0 을 헤아리게 만들지 않는다");
        assert!(PubDrops::default().snapshot().is_empty());
        assert!(SfuDrops::default().snapshot().is_empty());
    }

    #[test]
    fn a_reason_that_fired_is_named_not_lumped() {
        let d = SubDrops::default();
        note(&d.gate);
        note(&d.gate);
        note(&d.send_err);
        assert_eq!(d.snapshot(), vec![("gate", 2), ("send_err", 1)]);
        assert_eq!(d.total(), 3, "★합만 내면 어느 관문에서 막혔는지를 잃는다");
    }

    #[test]
    fn every_field_has_a_name_in_the_snapshot() {
        // ★필드를 늘리고 `snapshot` 에 안 적으면 그 사유는 영영 조용하다 — 그것이 이 규격이 금한 것이다.
        let d = SubDrops::default();
        for c in [&d.gate, &d.paused, &d.awaiting_keyframe, &d.rewrite_skip, &d.no_transport, &d.encrypt,
                  &d.send_err, &d.rtx_gate, &d.rtx_miss, &d.rtx_budget] {
            note(c);
        }
        assert_eq!(d.snapshot().len(), 10);
        let dc = DcDrops::default();
        note(&dc.full);
        note(&dc.pending_overflow);
        assert_eq!(dc.snapshot().len(), 2);
        assert_eq!(dc.total(), 2);
        let p = PubDrops::default();
        for c in [&p.muted, &p.no_room, &p.no_floor, &p.slot_unbound, &p.rewrite_skip] {
            note(c);
        }
        assert_eq!(p.snapshot().len(), 5);
        let s = SfuDrops::default();
        for c in [&s.no_session, &s.not_publisher, &s.srtp_decrypt, &s.srtcp_decrypt, &s.malformed, &s.unknown_ssrc] {
            note(c);
        }
        assert_eq!(s.snapshot().len(), 6);
    }
}
