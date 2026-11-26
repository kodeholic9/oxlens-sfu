// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-HOOK-DIRECT · 정:RULE-HOOK-CREDENTIAL · 정:RULE-HOOK-KEYSPACE · 정:RULE-HOOK-STATUS · 정:RULE-HOOK-PERMISSION · 정:RULE-HOOK-FACTS-SUB · model: claude-opus-5-5
//! zenoh 입구의 판단 — 순수 함수 + 사유(키 판단은 원천 `server/areas/hook/behavior/judge.ts` 의 거울 · 벡터 `hook.json`): 선언 키 → 제 계정의 갈래(채팅 · 사실) · 다른 계정 · 허용 키 밖 · get 키 → 방 목록 · 방 상세 · 권한 제어 · 그 밖(답 없이 끝) · 인증서 계정이 붙을 수 있나(연동 허용이면 붙는다 — 채팅 구독 · 사실 칸이 비어도) · InitSyn(판 0x09 · client 만)을 받나와 합의 값(해상도 · 배치 크기는 둘 중 작은 것) · queryable 선언 키가 제 계정 키인가(다른 계정 키 = 세션 닫기 · 허용 키 밖 = 무시).

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
                .map(|k| (k.key.clone(), if k.hook_allowed { Standing::Ok } else { Standing::NoHook }))
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
            Refuse::UnknownAccount(c) | Refuse::NoHook(c) => format!("cn={c}"),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lane {
    Chat,
    Facts,
}

impl Lane {
    pub fn as_str(self) -> &'static str {
        match self {
            Lane::Chat => "chat",
            Lane::Facts => "facts",
        }
    }

    pub fn of(s: &str) -> Option<Lane> {
        match s {
            "chat" => Some(Lane::Chat),
            "facts" => Some(Lane::Facts),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclareVerdict {
    Own(Lane),
    OtherAccount(String),
    Outside(String),
}

pub fn judge_declare(key: &str, prefix: &str, cn: &str) -> DeclareVerdict {
    let Some(tail) = key.strip_prefix(prefix) else { return DeclareVerdict::Outside(key.to_string()) };
    let p: Vec<&str> = tail.split('/').collect();
    let Some(lane) = p.first().and_then(|l| Lane::of(l)).filter(|_| p.len() >= 3 && p[1] == "sub") else {
        return DeclareVerdict::Outside(key.to_string());
    };
    if p.len() == 3 && p[2] == cn { DeclareVerdict::Own(lane) } else { DeclareVerdict::OtherAccount(key.to_string()) }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    List,
    Detail(String),
    Ctl(String),
    Outside,
}

impl Ask {
    pub fn reason(&self) -> &'static str {
        match self {
            Ask::List => "list",
            Ask::Detail(_) => "detail",
            Ask::Ctl(_) => "ctl",
            Ask::Outside => "outside",
        }
    }
}

fn room_piece(s: &str) -> bool {
    !s.is_empty() && !s.contains(['*', '$'])
}

pub fn judge_request(key: &str, prefix: &str) -> Ask {
    let Some(tail) = key.strip_prefix(prefix) else { return Ask::Outside };
    if tail == "q/rooms" {
        return Ask::List;
    }
    match tail.split('/').collect::<Vec<_>>().as_slice() {
        ["q", "room", r] if room_piece(r) => Ask::Detail(r.to_string()),
        ["ctl", "room", r, "permission"] if room_piece(r) => Ask::Ctl(r.to_string()),
        _ => Ask::Outside,
    }
}
