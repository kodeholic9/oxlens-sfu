// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:IMPL-CFG-SYSTEM-FILE · model: claude-opus-5-5
//! zenoh 입구의 판단 — 순수 함수 + 사유: 인증서 계정(연동 허용 · 구독)이 붙을 수 있나 · InitSyn(판 0x09 · client 만)을 받나와 합의 값(해상도 · 배치 크기는 둘 중 작은 것) · queryable 선언 키가 제 계정 키인가(다른 계정 키 = 세션 닫기 · 허용 키 밖 = 무시).

use std::collections::HashMap;

use common::system::System;
use zenoh_protocol::core::resolution::Field;
use zenoh_protocol::core::{Resolution, WhatAmI};
use zenoh_protocol::transport::{BatchSize, InitSyn};

pub const VERSION: u8 = zenoh_protocol::VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    Ok,
    NoHook,
    NoSubscribe,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Accounts(HashMap<String, Standing>);

impl Accounts {
    pub fn of(sys: &System) -> Accounts {
        Accounts(
            sys.hub
                .auth
                .api_keys
                .iter()
                .map(|k| {
                    let s = if !k.hook_allowed {
                        Standing::NoHook
                    } else if !k.subscribes() {
                        Standing::NoSubscribe
                    } else {
                        Standing::Ok
                    };
                    (k.key.clone(), s)
                })
                .collect(),
        )
    }

    pub fn new(list: Vec<(String, Standing)>) -> Accounts {
        Accounts(list.into_iter().collect())
    }

    pub fn standing(&self, cn: &str) -> Option<Standing> {
        self.0.get(cn).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refuse {
    Tls(String),
    NoCn,
    UnknownAccount(String),
    NoHook(String),
    NoSubscribe(String),
    Version(u8),
    NotClient(String),
    Protocol(&'static str),
    Cookie,
    OtherKey(String),
}

impl Refuse {
    pub fn reason(&self) -> &'static str {
        match self {
            Refuse::Tls(_) => "tls",
            Refuse::NoCn => "no cn",
            Refuse::UnknownAccount(_) => "unknown account",
            Refuse::NoHook(_) => "hook not allowed",
            Refuse::NoSubscribe(_) => "no subscribe",
            Refuse::Version(_) => "zenoh version",
            Refuse::NotClient(_) => "not a client",
            Refuse::Protocol(_) => "protocol",
            Refuse::Cookie => "cookie",
            Refuse::OtherKey(_) => "other account key",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Refuse::Tls(e) => e.clone(),
            Refuse::NoCn => "-".into(),
            Refuse::UnknownAccount(c) | Refuse::NoHook(c) | Refuse::NoSubscribe(c) => format!("cn={c}"),
            Refuse::Version(v) => format!("got=0x{v:02x} want=0x{VERSION:02x}"),
            Refuse::NotClient(w) => format!("got={w}"),
            Refuse::Protocol(p) => (*p).to_string(),
            Refuse::Cookie => "-".into(),
            Refuse::OtherKey(k) => format!("key={k}"),
        }
    }
}

pub fn judge_account(cn: Option<&str>, acc: &Accounts) -> Result<String, Refuse> {
    let cn = cn.ok_or(Refuse::NoCn)?;
    match acc.standing(cn) {
        None => Err(Refuse::UnknownAccount(cn.to_string())),
        Some(Standing::NoHook) => Err(Refuse::NoHook(cn.to_string())),
        Some(Standing::NoSubscribe) => Err(Refuse::NoSubscribe(cn.to_string())),
        Some(Standing::Ok) => Ok(cn.to_string()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Agreed {
    pub resolution: Resolution,
    pub batch: BatchSize,
}

pub fn judge_init(syn: &InitSyn, mine: Resolution, mine_batch: BatchSize) -> Result<Agreed, Refuse> {
    if syn.version != VERSION {
        return Err(Refuse::Version(syn.version));
    }
    if syn.whatami != WhatAmI::Client {
        return Err(Refuse::NotClient(syn.whatami.to_str().to_string()));
    }
    let mut resolution = Resolution::default();
    for f in [Field::FrameSN, Field::RequestID] {
        resolution.set(f, syn.resolution.get(f).min(mine.get(f)));
    }
    let batch = mine_batch.min(syn.batch_size);
    Ok(Agreed { resolution, batch })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclareVerdict {
    Own,
    OtherAccount(String),
    Outside(String),
}

pub fn judge_declare(key: &str, prefix: &str, cn: &str) -> DeclareVerdict {
    match key.strip_prefix(prefix) {
        Some(k) if k == cn => DeclareVerdict::Own,
        Some(_) => DeclareVerdict::OtherAccount(key.to_string()),
        None => DeclareVerdict::Outside(key.to_string()),
    }
}
