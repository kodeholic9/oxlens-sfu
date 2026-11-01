// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-MESH-WIRE · 정:RULE-MESH-LANE · model: claude-opus-5-5
//! 그물 wire — 메시지 열넷 · 손 부호화(varint · 길이 붙은 바이트) · 프레임 = 길이 4 + 레인 1 + 깃발 1 + 조각(16 KiB 까지) · 레인마다 조각 이어 붙이기.

pub const PROTO: u8 = 1;
pub const FRAME_MAX: usize = 16 * 1024;
pub const FRAME_HEAD: usize = 6;
pub const MSG_MAX: usize = 64 * 1024 * 1024;
pub const LANES: usize = 4;
const MORE: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lane {
    Control = 0,
    Declare = 1,
    Request = 2,
    Data = 3,
}

impl Lane {
    pub fn from_u8(v: u8) -> Option<Lane> {
        match v {
            0 => Some(Lane::Control),
            1 => Some(Lane::Declare),
            2 => Some(Lane::Request),
            3 => Some(Lane::Data),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Router = 0,
    Client = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeclKind {
    Token = 0,
    Sub = 1,
    Qabl = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenEvtKind {
    Put = 0,
    Delete = 1,
    BatchEnd = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    Shutdown = 0,
    Expired = 1,
    Fenced = 2,
    Replaced = 3,
    Refused = 4,
    Duplicate = 5,
    Protocol = 6,
    Io = 7,
}

impl CloseReason {
    pub fn from_u8(v: u8) -> CloseReason {
        match v {
            0 => CloseReason::Shutdown,
            1 => CloseReason::Expired,
            2 => CloseReason::Fenced,
            3 => CloseReason::Replaced,
            4 => CloseReason::Refused,
            5 => CloseReason::Duplicate,
            6 => CloseReason::Protocol,
            _ => CloseReason::Io,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CloseReason::Shutdown => "shutdown",
            CloseReason::Expired => "expired",
            CloseReason::Fenced => "fenced",
            CloseReason::Replaced => "replaced",
            CloseReason::Refused => "refused",
            CloseReason::Duplicate => "duplicate",
            CloseReason::Protocol => "protocol",
            CloseReason::Io => "io",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    pub proto: u8,
    pub role: Role,
    pub node: String,
    pub boot: u64,
    pub sno: u64,
    pub lease_ms: u32,
    pub listen: Vec<String>,
    pub peers: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    Hello(Hello),
    Close(CloseReason),
    KeepAlive,
    Declare { kind: DeclKind, id: u64, expr: String },
    Undeclare { kind: DeclKind, id: u64 },
    DeclareDone,
    TokenSub { id: u64, expr: String, history: bool },
    TokenUnsub { id: u64 },
    TokenEvt { id: u64, kind: TokenEvtKind, key: String },
    Peers(Vec<(String, String)>),
    Put { key: String, payload: Vec<u8> },
    Request { qid: u64, expr: String, payload: Vec<u8>, timeout_ms: u32 },
    Reply { qid: u64, key: String, ok: bool, payload: Vec<u8> },
    Final { qid: u64 },
}

impl Msg {
    pub fn lane(&self) -> Lane {
        match self {
            Msg::Hello(_) | Msg::Close(_) | Msg::KeepAlive => Lane::Control,
            Msg::Declare { .. }
            | Msg::Undeclare { .. }
            | Msg::DeclareDone
            | Msg::TokenSub { .. }
            | Msg::TokenUnsub { .. }
            | Msg::TokenEvt { .. }
            | Msg::Peers(_) => Lane::Declare,
            Msg::Request { .. } | Msg::Reply { .. } | Msg::Final { .. } => Lane::Request,
            Msg::Put { .. } => Lane::Data,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(32);
        match self {
            Msg::Hello(h) => {
                b.push(0);
                b.push(h.proto);
                b.push(h.role as u8);
                put_str(&mut b, &h.node);
                put_var(&mut b, h.boot);
                put_var(&mut b, h.sno);
                put_var(&mut b, u64::from(h.lease_ms));
                put_var(&mut b, h.listen.len() as u64);
                for l in &h.listen {
                    put_str(&mut b, l);
                }
                put_pairs(&mut b, &h.peers);
            }
            Msg::Close(r) => {
                b.push(1);
                b.push(*r as u8);
            }
            Msg::KeepAlive => b.push(2),
            Msg::Declare { kind, id, expr } => {
                b.push(3);
                b.push(*kind as u8);
                put_var(&mut b, *id);
                put_str(&mut b, expr);
            }
            Msg::Undeclare { kind, id } => {
                b.push(4);
                b.push(*kind as u8);
                put_var(&mut b, *id);
            }
            Msg::DeclareDone => b.push(5),
            Msg::TokenSub { id, expr, history } => {
                b.push(6);
                put_var(&mut b, *id);
                put_str(&mut b, expr);
                b.push(u8::from(*history));
            }
            Msg::TokenUnsub { id } => {
                b.push(7);
                put_var(&mut b, *id);
            }
            Msg::TokenEvt { id, kind, key } => {
                b.push(8);
                put_var(&mut b, *id);
                b.push(*kind as u8);
                put_str(&mut b, key);
            }
            Msg::Peers(p) => {
                b.push(9);
                put_pairs(&mut b, p);
            }
            Msg::Put { key, payload } => {
                b.push(10);
                put_str(&mut b, key);
                put_bytes(&mut b, payload);
            }
            Msg::Request { qid, expr, payload, timeout_ms } => {
                b.push(11);
                put_var(&mut b, *qid);
                put_str(&mut b, expr);
                put_bytes(&mut b, payload);
                put_var(&mut b, u64::from(*timeout_ms));
            }
            Msg::Reply { qid, key, ok, payload } => {
                b.push(12);
                put_var(&mut b, *qid);
                put_str(&mut b, key);
                b.push(u8::from(*ok));
                put_bytes(&mut b, payload);
            }
            Msg::Final { qid } => {
                b.push(13);
                put_var(&mut b, *qid);
            }
        }
        b
    }

    pub fn decode(b: &[u8]) -> Option<Msg> {
        let mut c = Cur { b, at: 0 };
        let m = match c.u8()? {
            0 => {
                let proto = c.u8()?;
                let role = match c.u8()? {
                    0 => Role::Router,
                    1 => Role::Client,
                    _ => return None,
                };
                let node = c.str()?;
                let boot = c.var()?;
                let sno = c.var()?;
                let lease_ms = u32::try_from(c.var()?).ok()?;
                let n = c.len()?;
                let mut listen = Vec::with_capacity(n.min(16));
                for _ in 0..n {
                    listen.push(c.str()?);
                }
                let peers = c.pairs()?;
                Msg::Hello(Hello { proto, role, node, boot, sno, lease_ms, listen, peers })
            }
            1 => Msg::Close(CloseReason::from_u8(c.u8()?)),
            2 => Msg::KeepAlive,
            3 => {
                let kind = c.kind()?;
                let id = c.var()?;
                Msg::Declare { kind, id, expr: c.str()? }
            }
            4 => {
                let kind = c.kind()?;
                Msg::Undeclare { kind, id: c.var()? }
            }
            5 => Msg::DeclareDone,
            6 => {
                let id = c.var()?;
                let expr = c.str()?;
                Msg::TokenSub { id, expr, history: c.u8()? != 0 }
            }
            7 => Msg::TokenUnsub { id: c.var()? },
            8 => {
                let id = c.var()?;
                let kind = match c.u8()? {
                    0 => TokenEvtKind::Put,
                    1 => TokenEvtKind::Delete,
                    2 => TokenEvtKind::BatchEnd,
                    _ => return None,
                };
                Msg::TokenEvt { id, kind, key: c.str()? }
            }
            9 => Msg::Peers(c.pairs()?),
            10 => {
                let key = c.str()?;
                Msg::Put { key, payload: c.bytes()? }
            }
            11 => {
                let qid = c.var()?;
                let expr = c.str()?;
                let payload = c.bytes()?;
                let timeout_ms = u32::try_from(c.var()?).ok()?;
                Msg::Request { qid, expr, payload, timeout_ms }
            }
            12 => {
                let qid = c.var()?;
                let key = c.str()?;
                let ok = c.u8()? != 0;
                Msg::Reply { qid, key, ok, payload: c.bytes()? }
            }
            13 => Msg::Final { qid: c.var()? },
            _ => return None,
        };
        if c.at != b.len() {
            return None;
        }
        Some(m)
    }

    pub fn frames(&self) -> (Lane, Vec<Vec<u8>>) {
        let lane = self.lane();
        let body = self.encode();
        let room = FRAME_MAX - FRAME_HEAD;
        let mut out = Vec::with_capacity(body.len() / room + 1);
        let mut chunks = body.chunks(room).peekable();
        if chunks.peek().is_none() {
            out.push(frame(lane, 0, &[]));
        }
        while let Some(ch) = chunks.next() {
            let flags = if chunks.peek().is_some() { MORE } else { 0 };
            out.push(frame(lane, flags, ch));
        }
        (lane, out)
    }
}

fn frame(lane: Lane, flags: u8, chunk: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(FRAME_HEAD + chunk.len());
    f.extend_from_slice(&((chunk.len() + 2) as u32).to_be_bytes());
    f.push(lane as u8);
    f.push(flags);
    f.extend_from_slice(chunk);
    f
}

#[derive(Default)]
pub struct Reassembler {
    parts: [Vec<u8>; LANES],
}

#[derive(Debug, PartialEq, Eq)]
pub enum Assembled {
    Partial,
    Done(Vec<u8>),
    TooBig,
}

impl Reassembler {
    pub fn push(&mut self, lane: Lane, flags: u8, chunk: &[u8]) -> Assembled {
        let p = &mut self.parts[lane as usize];
        if p.len() + chunk.len() > MSG_MAX {
            return Assembled::TooBig;
        }
        p.extend_from_slice(chunk);
        if flags & MORE != 0 {
            return Assembled::Partial;
        }
        Assembled::Done(std::mem::take(p))
    }
}

fn put_var(b: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            b.push(byte);
            return;
        }
        b.push(byte | 0x80);
    }
}

fn put_bytes(b: &mut Vec<u8>, v: &[u8]) {
    put_var(b, v.len() as u64);
    b.extend_from_slice(v);
}

fn put_str(b: &mut Vec<u8>, v: &str) {
    put_bytes(b, v.as_bytes());
}

fn put_pairs(b: &mut Vec<u8>, p: &[(String, String)]) {
    put_var(b, p.len() as u64);
    for (k, v) in p {
        put_str(b, k);
        put_str(b, v);
    }
}

struct Cur<'a> {
    b: &'a [u8],
    at: usize,
}

impl Cur<'_> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.at)?;
        self.at += 1;
        Some(v)
    }

    fn var(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.u8()?;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }

    fn len(&mut self) -> Option<usize> {
        let n = usize::try_from(self.var()?).ok()?;
        (n <= self.b.len() - self.at).then_some(n)
    }

    fn bytes(&mut self) -> Option<Vec<u8>> {
        let n = self.len()?;
        let v = self.b[self.at..self.at + n].to_vec();
        self.at += n;
        Some(v)
    }

    fn str(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?).ok()
    }

    fn kind(&mut self) -> Option<DeclKind> {
        match self.u8()? {
            0 => Some(DeclKind::Token),
            1 => Some(DeclKind::Sub),
            2 => Some(DeclKind::Qabl),
            _ => None,
        }
    }

    fn pairs(&mut self) -> Option<Vec<(String, String)>> {
        let n = self.len()?;
        let mut out = Vec::with_capacity(n.min(128));
        for _ in 0..n {
            let k = self.str()?;
            out.push((k, self.str()?));
        }
        Some(out)
    }
}
