// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SIM-SWITCH · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! dependency-descriptor(AV1 RTP 규격 부록 A) 읽기 — 틀 묶음(키프레임이 싣는다) · 패킷마다 틀 번호 → 시간 층 · DTI · 전환점 시간 층. libwebrtc rtp_dependency_descriptor_reader.cc 그대로. 원천 server/areas/sim/behavior/dd.ts. 돌려주는 틀 묶음은 새로 실린 것만(없으면 들던 것을 그대로 쓴다).

pub const DTI_SWITCH: u8 = 2;
pub const DD_MAX_TEMPLATES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdTemplate {
    pub sid: u8,
    pub tid: u8,
    pub dtis: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdStructure {
    pub offset: u8,
    pub dts: usize,
    pub templates: Vec<DdTemplate>,
    pub dt_layer: Vec<(u8, u8)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdFrame {
    pub start: bool,
    pub end: bool,
    pub frame: u16,
    pub sid: u8,
    pub tid: u8,
    pub dtis: Vec<u8>,
}

struct Bits<'a> {
    b: &'a [u8],
    i: usize,
    ok: bool,
}

impl Bits<'_> {
    fn read(&mut self, n: usize) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = self.i >> 3;
            let Some(x) = self.b.get(byte) else {
                self.ok = false;
                return 0;
            };
            v = (v << 1) | ((x >> (7 - (self.i & 7))) & 1) as u32;
            self.i += 1;
        }
        v
    }

    fn ns(&mut self, n: u32) -> u32 {
        let w = 32 - n.leading_zeros() as usize;
        let m = (1u32 << w) - n;
        let v = self.read(w - 1);
        if v < m {
            return v;
        }
        (v << 1) + self.read(1) - m
    }
}

pub fn dt_layers(dts: usize, templates: &[DdTemplate]) -> Vec<(u8, u8)> {
    (0..dts)
        .map(|d| {
            let (mut s, mut t) = (0u8, 0u8);
            for x in templates {
                if x.dtis.get(d).copied().unwrap_or(0) != 0 {
                    s = s.max(x.sid);
                    t = t.max(x.tid);
                }
            }
            (s, t)
        })
        .collect()
}

fn read_structure(r: &mut Bits<'_>) -> DdStructure {
    let offset = r.read(6) as u8;
    let dts = r.read(5) as usize + 1;
    let mut templates: Vec<DdTemplate> = Vec::new();
    let (mut t, mut s) = (0u8, 0u8);
    loop {
        if templates.len() == DD_MAX_TEMPLATES {
            r.ok = false;
            break;
        }
        templates.push(DdTemplate { sid: s, tid: t, dtis: Vec::with_capacity(dts) });
        let idc = r.read(2);
        if idc == 1 {
            t = t.saturating_add(1);
        } else if idc == 2 {
            t = 0;
            s = s.saturating_add(1);
        }
        if idc == 3 || !r.ok {
            break;
        }
    }
    for x in templates.iter_mut() {
        x.dtis = (0..dts).map(|_| r.read(2) as u8).collect();
    }
    for _ in 0..templates.len() {
        while r.ok && r.read(1) == 1 {
            r.read(4);
        }
    }
    let chains = r.ns(dts as u32 + 1);
    if chains > 0 {
        for _ in 0..dts {
            r.ns(chains);
        }
        for _ in 0..templates.len() {
            for _ in 0..chains {
                r.read(4);
            }
        }
    }
    if r.read(1) == 1 {
        let last = templates.last().map(|x| x.sid).unwrap_or(0);
        for _ in 0..=last {
            r.read(16);
            r.read(16);
        }
    }
    let dt_layer = dt_layers(dts, &templates);
    DdStructure { offset, dts, templates, dt_layer }
}

pub fn dd_read(raw: &[u8], held: Option<&DdStructure>) -> (Option<DdFrame>, Option<DdStructure>) {
    if raw.len() < 3 {
        return (None, None);
    }
    let mut r = Bits { b: raw, i: 0, ok: true };
    let start = r.read(1) == 1;
    let end = r.read(1) == 1;
    let tid6 = r.read(6) as usize;
    let frame = r.read(16) as u16;
    let mut attached: Option<DdStructure> = None;
    let (mut active, mut custom_dtis, mut custom_fdiffs) = (false, false, false);
    if raw.len() > 3 {
        let present = r.read(1) == 1;
        active = r.read(1) == 1;
        custom_dtis = r.read(1) == 1;
        custom_fdiffs = r.read(1) == 1;
        let _custom_chains = r.read(1);
        if present {
            attached = Some(read_structure(&mut r));
        }
    }
    if !r.ok {
        return (None, None);
    }
    let Some(st) = attached.as_ref().or(held) else { return (None, None) };
    if active {
        r.read(st.dts);
    }
    let idx = (tid6 + DD_MAX_TEMPLATES - st.offset as usize) % DD_MAX_TEMPLATES;
    let Some(tpl) = st.templates.get(idx) else { return (None, attached) };
    let dtis = if custom_dtis { (0..st.dts).map(|_| r.read(2) as u8).collect() } else { tpl.dtis.clone() };
    if custom_fdiffs {
        loop {
            let n = r.read(2) as usize;
            if n == 0 || !r.ok {
                break;
            }
            r.read(4 * n);
        }
    }
    if !r.ok {
        return (None, attached);
    }
    let out = DdFrame { start, end, frame, sid: tpl.sid, tid: tpl.tid, dtis };
    (Some(out), attached)
}

pub fn dd_sync(f: &DdFrame, st: &DdStructure) -> u8 {
    let mut m = 0u8;
    for d in 0..st.dts {
        if f.dtis.get(d).copied() == Some(DTI_SWITCH)
            && let Some(&(s, t)) = st.dt_layer.get(d)
            && s == 0
            && t < 8
        {
            m |= 1 << t;
        }
    }
    m
}
