// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · model: claude-opus-5-5
//! zenoh 1.10.1 wire 의 우리 쪽 — 일련번호 마스크는 해상도마다 varint 크기에 맞춘 값(U32 = 28비트 · zenoh-transport `seq_num.rs` 와 같다) · 배치 하나를 전송 메시지들로 읽기 · 보낼 망 메시지를 Frame(하나에 하나) 또는 Fragment 줄로 짓기(배치 몸은 합의한 배치 크기 − 길이 2 바이트 이하 — 받는 쪽 버퍼가 길이까지 배치 크기 하나다 · zenoh-transport `unicast/link.rs` recv_batch) · 배치로 묶기 · 방향마다 받은 일련번호 판정(앞선 것만 · 처음은 알린 첫 번호) · 조각 잇기(번호가 이어져야 · 상한) — 형과 인코딩은 `zenoh-protocol` · `zenoh-codec` 그대로.

use zenoh_buffers::buffer::SplitBuffer;
use zenoh_buffers::reader::{HasReader, Reader};
use zenoh_buffers::writer::HasWriter;
use zenoh_buffers::{ZBuf, ZSlice};
use zenoh_codec::{RCodec, WCodec, Zenoh080, Zenoh080Reliability};
use zenoh_protocol::core::{Bits, Reliability, Resolution};
use zenoh_protocol::core::resolution::Field;
use zenoh_protocol::network::NetworkMessage;
use zenoh_protocol::transport::{BatchSize, Fragment, Frame, TransportMessage, TransportSn, frame, fragment};

pub const FRAME_HEAD_MAX: usize = 6;
pub const LEN_PREFIX: usize = 2;
pub const DEFRAG_MAX: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    Decode,
    Encode,
    TooLarge(usize),
}

pub fn decode_batch(batch: &[u8]) -> Result<Vec<TransportMessage>, WireError> {
    let mut r = batch.reader();
    let mut out = Vec::new();
    while r.can_read() {
        let m: TransportMessage = Zenoh080::new().read(&mut r).map_err(|_| WireError::Decode)?;
        out.push(m);
    }
    Ok(out)
}

pub fn encode(m: &TransportMessage) -> Result<Vec<u8>, WireError> {
    let mut v = Vec::new();
    let mut w = (&mut v).writer();
    Zenoh080::new().write(&mut w, m).map_err(|_| WireError::Encode)?;
    Ok(v)
}

pub fn encode_net(m: &NetworkMessage) -> Result<Vec<u8>, WireError> {
    let mut v = Vec::new();
    let mut w = (&mut v).writer();
    Zenoh080::new().write(&mut w, m).map_err(|_| WireError::Encode)?;
    Ok(v)
}

pub fn decode_net(bytes: &[u8], reliability: Reliability) -> Result<NetworkMessage, WireError> {
    let mut r = bytes.reader();
    let m: NetworkMessage = Zenoh080Reliability::new(reliability).read(&mut r).map_err(|_| WireError::Decode)?;
    if r.can_read() {
        return Err(WireError::Decode);
    }
    Ok(m)
}

pub fn sn_mask(res: &Resolution) -> TransportSn {
    match res.get(Field::FrameSN) {
        Bits::U8 => (u8::MAX >> 1) as TransportSn,
        Bits::U16 => (u16::MAX >> 2) as TransportSn,
        Bits::U32 => u32::MAX >> 4,
        Bits::U64 => TransportSn::MAX >> 1,
    }
}

#[derive(Debug, Clone)]
pub struct TxSn {
    next: TransportSn,
    mask: TransportSn,
}

impl TxSn {
    pub fn new(initial: TransportSn, mask: TransportSn) -> TxSn {
        TxSn { next: initial & mask, mask }
    }

    pub fn take(&mut self) -> TransportSn {
        let sn = self.next;
        self.next = self.next.wrapping_add(1) & self.mask;
        sn
    }
}

pub fn room(batch: BatchSize) -> usize {
    (batch as usize).saturating_sub(LEN_PREFIX)
}

pub fn parts(net: &NetworkMessage, sn: &mut TxSn, batch: BatchSize) -> Result<Vec<Vec<u8>>, WireError> {
    let limit = room(batch);
    let one = TransportMessage::from(Frame {
        reliability: Reliability::Reliable,
        sn: sn.next,
        ext_qos: frame::ext::QoSType::DEFAULT,
        payload: vec![net.clone()],
    });
    let bytes = encode(&one)?;
    if bytes.len() <= limit {
        sn.take();
        return Ok(vec![bytes]);
    }
    let body = encode_net(net)?;
    let chunk = limit.saturating_sub(FRAME_HEAD_MAX);
    if chunk == 0 {
        return Err(WireError::TooLarge(body.len()));
    }
    let chunks: Vec<&[u8]> = body.chunks(chunk).collect();
    let last = chunks.len() - 1;
    let mut out = Vec::with_capacity(chunks.len());
    for (i, c) in chunks.into_iter().enumerate() {
        let f = TransportMessage::from(Fragment {
            reliability: Reliability::Reliable,
            more: i < last,
            sn: sn.take(),
            payload: ZSlice::from(c.to_vec()),
            ext_qos: fragment::ext::QoSType::DEFAULT,
            ext_first: None,
            ext_drop: None,
        });
        let b = encode(&f)?;
        if b.len() > limit {
            return Err(WireError::TooLarge(b.len()));
        }
        out.push(b);
    }
    Ok(out)
}

pub fn pack(parts: Vec<Vec<u8>>, batch: BatchSize) -> Vec<Vec<u8>> {
    let limit = room(batch);
    let mut out: Vec<Vec<u8>> = Vec::new();
    for p in parts {
        match out.last_mut() {
            Some(b) if b.len() + p.len() <= limit => b.extend_from_slice(&p),
            _ => out.push(p),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnVerdict {
    Next,
    Gap,
    Stale,
}

#[derive(Debug, Clone)]
pub struct RxSn {
    last: TransportSn,
    mask: TransportSn,
}

impl RxSn {
    pub fn new(initial: TransportSn, mask: TransportSn) -> RxSn {
        RxSn { last: initial.wrapping_sub(1) & mask, mask }
    }

    pub fn judge(&mut self, sn: TransportSn) -> SnVerdict {
        if sn & !self.mask != 0 {
            return SnVerdict::Stale;
        }
        let gap = sn.wrapping_sub(self.last) & self.mask;
        if gap == 0 || gap & !(self.mask >> 1) != 0 {
            return SnVerdict::Stale;
        }
        self.last = sn;
        if gap == 1 { SnVerdict::Next } else { SnVerdict::Gap }
    }
}

#[derive(Debug, Default)]
pub struct Defrag {
    buf: Vec<u8>,
    open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    More,
    Whole(Vec<u8>),
    Dropped(&'static str),
}

impl Defrag {
    pub fn push(&mut self, verdict: SnVerdict, more: bool, payload: &ZSlice) -> Piece {
        if verdict == SnVerdict::Gap && self.open {
            self.buf.clear();
            self.open = false;
            return Piece::Dropped("fragment sn gap");
        }
        if self.buf.len() + payload.len() > DEFRAG_MAX {
            self.buf.clear();
            self.open = false;
            return Piece::Dropped("fragment over the defrag limit");
        }
        self.buf.extend_from_slice(payload.as_slice());
        if more {
            self.open = true;
            return Piece::More;
        }
        self.open = false;
        Piece::Whole(std::mem::take(&mut self.buf))
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.open = false;
    }
}

pub fn zbuf(bytes: Vec<u8>) -> ZBuf {
    ZBuf::from(bytes)
}

pub fn zbuf_bytes(z: &ZBuf) -> Vec<u8> {
    z.contiguous().to_vec()
}

pub fn err_body(bytes: Vec<u8>) -> zenoh_protocol::zenoh::Err {
    zenoh_protocol::zenoh::Err { encoding: zenoh_protocol::core::Encoding::empty(), ext_sinfo: None, ext_unknown: Vec::new(), payload: zbuf(bytes) }
}
