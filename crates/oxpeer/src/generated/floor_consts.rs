// author: kodeholic (powered by Claude)
// spec: context/spec/pilot/floor/ts — 생성 파일(gen/consts.py) · 손으로 고치지 않는다

pub mod timers {
    pub const T101_MS: u64 = 500;
    pub const T100_MS: u64 = 500;
    pub const T104_MS: u64 = 500;
    pub const T132_MS: u64 = 2000;
    pub const T1_MS: u64 = 4000;
    pub const T1MAX_MS: u64 = 6000;
    pub const T2_MS: u64 = 30000;
    pub const T3_MS: u64 = 3000;
    pub const T7_MS: u64 = 1000;
    pub const T8_MS: u64 = 1000;
    pub const T20_MS: u64 = 1000;
    pub const T9_MS: u64 = 3000;
    pub const T21_MS: u64 = 1000;
    pub const C101: u8 = 3;
    pub const C100: u8 = 3;
    pub const C104: u8 = 3;
    pub const C7: u8 = 10;
    pub const C8: u8 = 3;
    pub const C20: u8 = 3;
    pub const C21: u8 = 3;
}
pub mod msg {
    pub const REQUEST: u8 = 0;
    pub const GRANTED: u8 = 1;
    pub const TAKEN: u8 = 2;
    pub const DENY: u8 = 3;
    pub const RELEASE: u8 = 4;
    pub const IDLE: u8 = 5;
    pub const REVOKE: u8 = 6;
    pub const QPOS_REQ: u8 = 8;
    pub const QINFO: u8 = 9;
    pub const ACK: u8 = 10;
}
pub mod tlv {
    pub const PRIORITY: u8 = 0;
    pub const DURATION: u8 = 1;
    pub const CAUSE: u8 = 2;
    pub const QINFO: u8 = 3;
    pub const SPEAKER: u8 = 4;
    pub const SEQ: u8 = 8;
    pub const ACKTYPE: u8 = 12;
    pub const PREV: u8 = 26;
    pub const REASON: u8 = 27;
    pub const ROOM: u8 = 29;
    pub const TALK: u8 = 32;
    pub const SOURCES: u8 = 31;
}
pub mod deny {
    pub const ONLY_ONE: u8 = 3;
    pub const RETRY_AFTER: u8 = 4;
    pub const RECV_ONLY: u8 = 5;
    pub const QUEUE_FULL: u8 = 7;
    pub const NOT_PUBROOM: u8 = 101;
    pub const NO_PERMISSION: u8 = 102;
    pub const OTHER_REASON: u8 = 255;
}
pub mod revoke {
    pub const TOO_LONG: u8 = 2;
    pub const NO_PERMISSION: u8 = 3;
    pub const PREEMPTED: u8 = 4;
}
pub mod qpos {
    pub const NOT_IN_QUEUE: u8 = 254;
    pub const UNKNOWN: u8 = 255;
}
pub const SEQ_WRAP: u32 = 65536;
