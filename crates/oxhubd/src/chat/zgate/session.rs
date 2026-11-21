// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · model: claude-opus-5-5
//! 열린 zenoh 세션 하나의 상태(입출력 없음 — 받은 전송 메시지를 넣으면 할 일을 낸다): 방향마다 받은 일련번호 · 조각 잇기 · 그 연결이 선언한 키 식 표(Sender 매핑) · 제 계정 키의 queryable 들(처음 하나 = 열림 · 마지막이 걷히면 = 닫힘) · Interest(지금 · 지금과 앞으로)에 DeclareFinal · Push 버림 · 3rd 의 Request 는 결과 없이 끝 · 우리가 보낸 질의(요청 번호 → 표) — 첫 Reply = ACK · 첫 Err = 거절 · 답 없이 끝 = 답 없음.

use std::collections::{BTreeSet, HashMap};

use zenoh_protocol::core::{Encoding, ExprId, Reliability, WireExpr};
use zenoh_protocol::network::declare::{self, DeclareFinal, QueryableId};
use zenoh_protocol::network::request::{self, RequestId};
use zenoh_protocol::network::{Declare, DeclareBody, Mapping, NetworkBody, NetworkMessage, Request, ResponseFinal, response};
use zenoh_protocol::transport::{BatchSize, TransportBody, TransportMessage, TransportSn};
use zenoh_protocol::zenoh::query::ext::QueryBodyType;
use zenoh_protocol::zenoh::{ConsolidationMode, Query, RequestBody, ResponseBody};

use super::judge::{DeclareVerdict, Refuse, judge_declare};
use super::wire::{self, Defrag, Piece, RxSn, SnVerdict, TxSn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Ack,
    Refused(Vec<u8>),
    NoReply,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Send(Box<NetworkMessage>),
    Open,
    Shut,
    Answer { ticket: u64, answer: Answer },
    Ignored(String),
    PushDropped,
    Dropped(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    Peer(u8),
    Refused(Refuse),
}

#[derive(Debug, Clone, Copy)]
struct Asked {
    ticket: u64,
    decided: bool,
}

pub struct Rx {
    sn: RxSn,
    defrag: Defrag,
}

pub struct Session {
    pub cn: String,
    own_key: String,
    prefix: String,
    exprs: HashMap<ExprId, String>,
    own: BTreeSet<QueryableId>,
    reliable: Rx,
    best_effort: Rx,
    pub tx: TxSn,
    pub batch: BatchSize,
    rid_mask: RequestId,
    next_rid: RequestId,
    asked: HashMap<RequestId, Asked>,
}

pub struct Opened {
    pub cn: String,
    pub own_key: String,
    pub prefix: String,
    pub rx_initial: TransportSn,
    pub tx_initial: TransportSn,
    pub sn_mask: TransportSn,
    pub rid_mask: RequestId,
    pub batch: BatchSize,
}

fn net(body: NetworkBody) -> NetworkMessage {
    NetworkMessage { body, reliability: Reliability::Reliable }
}

impl Session {
    pub fn new(o: Opened) -> Session {
        let rx = || Rx { sn: RxSn::new(o.rx_initial, o.sn_mask), defrag: Defrag::default() };
        Session {
            cn: o.cn,
            own_key: o.own_key,
            prefix: o.prefix,
            exprs: HashMap::new(),
            own: BTreeSet::new(),
            reliable: rx(),
            best_effort: rx(),
            tx: TxSn::new(o.tx_initial, o.sn_mask),
            batch: o.batch,
            rid_mask: o.rid_mask,
            next_rid: 0,
            asked: HashMap::new(),
        }
    }

    pub fn is_open(&self) -> bool {
        !self.own.is_empty()
    }

    pub fn on_transport(&mut self, m: TransportMessage) -> Result<Vec<Effect>, End> {
        let mut out = Vec::new();
        match m.body {
            TransportBody::Frame(f) => {
                let rx = self.rx(f.reliability);
                if rx.sn.judge(f.sn) == SnVerdict::Stale {
                    out.push(Effect::Dropped("stale frame sn"));
                    return Ok(out);
                }
                rx.defrag.clear();
                for n in f.payload {
                    self.on_net(n, &mut out)?;
                }
            }
            TransportBody::Fragment(f) => {
                let rx = self.rx(f.reliability);
                let v = rx.sn.judge(f.sn);
                if v == SnVerdict::Stale {
                    out.push(Effect::Dropped("stale fragment sn"));
                    return Ok(out);
                }
                match rx.defrag.push(v, f.more, &f.payload) {
                    Piece::More => {}
                    Piece::Dropped(why) => out.push(Effect::Dropped(why)),
                    Piece::Whole(bytes) => match wire::decode_net(&bytes, f.reliability) {
                        Ok(n) => self.on_net(n, &mut out)?,
                        Err(_) => out.push(Effect::Dropped("fragment did not decode")),
                    },
                }
            }
            TransportBody::KeepAlive(_) | TransportBody::OAM(_) => {}
            TransportBody::Close(c) => return Err(End::Peer(c.reason)),
            _ => return Err(End::Refused(Refuse::Protocol("unexpected transport message after open"))),
        }
        Ok(out)
    }

    fn rx(&mut self, r: Reliability) -> &mut Rx {
        match r {
            Reliability::Reliable => &mut self.reliable,
            Reliability::BestEffort => &mut self.best_effort,
        }
    }

    fn resolve(&self, w: &WireExpr) -> Option<String> {
        if w.scope == 0 {
            return Some(w.suffix.to_string());
        }
        match w.mapping {
            Mapping::Sender => self.exprs.get(&w.scope).map(|p| format!("{p}{}", w.suffix)),
            Mapping::Receiver => None,
        }
    }

    fn on_net(&mut self, n: NetworkMessage, out: &mut Vec<Effect>) -> Result<(), End> {
        match n.body {
            NetworkBody::Declare(d) => self.on_declare(d, out)?,
            NetworkBody::Interest(i) => {
                if i.mode.is_current() {
                    out.push(Effect::Send(Box::new(net(NetworkBody::Declare(Declare {
                        interest_id: Some(i.id),
                        ext_qos: declare::ext::QoSType::DECLARE,
                        ext_tstamp: None,
                        ext_nodeid: declare::ext::NodeIdType::DEFAULT,
                        body: DeclareBody::DeclareFinal(DeclareFinal),
                    })))));
                }
            }
            NetworkBody::Push(_) => out.push(Effect::PushDropped),
            NetworkBody::Request(r) => out.push(Effect::Send(Box::new(net(NetworkBody::ResponseFinal(ResponseFinal {
                rid: r.id,
                ext_qos: response::ext::QoSType::DEFAULT,
                ext_tstamp: None,
            }))))),
            NetworkBody::Response(r) => {
                if let Some(a) = self.asked.get_mut(&r.rid)
                    && !a.decided
                {
                    a.decided = true;
                    let answer = match r.payload {
                        ResponseBody::Reply(_) => Answer::Ack,
                        ResponseBody::Err(e) => Answer::Refused(wire::zbuf_bytes(&e.payload)),
                    };
                    out.push(Effect::Answer { ticket: a.ticket, answer });
                }
            }
            NetworkBody::ResponseFinal(f) => {
                if let Some(a) = self.asked.remove(&f.rid)
                    && !a.decided
                {
                    out.push(Effect::Answer { ticket: a.ticket, answer: Answer::NoReply });
                }
            }
            NetworkBody::OAM(_) => {}
        }
        Ok(())
    }

    fn on_declare(&mut self, d: Declare, out: &mut Vec<Effect>) -> Result<(), End> {
        match d.body {
            DeclareBody::DeclareKeyExpr(k) => {
                if let Some(s) = self.resolve(&k.wire_expr) {
                    self.exprs.insert(k.id, s);
                }
            }
            DeclareBody::UndeclareKeyExpr(k) => {
                self.exprs.remove(&k.id);
            }
            DeclareBody::DeclareQueryable(q) => {
                let key = self.resolve(&q.wire_expr).unwrap_or_else(|| format!("?{}:{}", q.wire_expr.scope, q.wire_expr.suffix));
                match judge_declare(&key, &self.prefix, &self.cn) {
                    DeclareVerdict::Own => {
                        let first = self.own.is_empty();
                        self.own.insert(q.id);
                        if first {
                            out.push(Effect::Open);
                        }
                    }
                    DeclareVerdict::OtherAccount(k) => return Err(End::Refused(Refuse::OtherKey(k))),
                    DeclareVerdict::Outside(k) => out.push(Effect::Ignored(k)),
                }
            }
            DeclareBody::UndeclareQueryable(q) => {
                if self.own.remove(&q.id) && self.own.is_empty() {
                    out.push(Effect::Shut);
                }
            }
            DeclareBody::DeclareSubscriber(s) => out.push(Effect::Ignored(self.resolve(&s.wire_expr).unwrap_or_default())),
            DeclareBody::DeclareToken(t) => out.push(Effect::Ignored(self.resolve(&t.wire_expr).unwrap_or_default())),
            DeclareBody::UndeclareSubscriber(_) | DeclareBody::UndeclareToken(_) | DeclareBody::DeclareFinal(_) => {}
        }
        Ok(())
    }

    pub fn query(&mut self, ticket: u64, body: Vec<u8>) -> NetworkMessage {
        let rid = self.next_rid & self.rid_mask;
        self.next_rid = self.next_rid.wrapping_add(1) & self.rid_mask;
        self.asked.insert(rid, Asked { ticket, decided: false });
        net(NetworkBody::Request(Request {
            id: rid,
            wire_expr: WireExpr::from(self.own_key.clone()),
            ext_qos: request::ext::QoSType::REQUEST,
            ext_tstamp: None,
            ext_nodeid: request::ext::NodeIdType::DEFAULT,
            ext_target: request::ext::QueryTarget::DEFAULT,
            ext_budget: None,
            ext_timeout: None,
            ext_ts_stack: None,
            payload: RequestBody::Query(Query {
                consolidation: ConsolidationMode::None,
                parameters: String::new(),
                ext_sinfo: None,
                ext_body: Some(QueryBodyType { encoding: Encoding::empty(), payload: wire::zbuf(body) }),
                ext_attachment: None,
                ext_unknown: Vec::new(),
            }),
        }))
    }

    pub fn forget(&mut self, ticket: u64) {
        self.asked.retain(|_, a| a.ticket != ticket);
    }

    pub fn asked(&self) -> usize {
        self.asked.len()
    }

    pub fn end(&mut self) -> Vec<Effect> {
        let mut out: Vec<Effect> =
            self.asked.drain().filter(|(_, a)| !a.decided).map(|(_, a)| Effect::Answer { ticket: a.ticket, answer: Answer::Closed }).collect();
        if !self.own.is_empty() {
            self.own.clear();
            out.push(Effect::Shut);
        }
        out
    }
}
