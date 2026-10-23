// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-LINK-TRANSPORT · 정:RULE-LINK-PAIR-LIVENESS · 정:IMPL-LINK-PEER-STATE · 정:IMPL-LINK-ICE-RESTART · 정:RULE-GC-LEAVE-SEVEN · 연:RULE-LINK-SDP-DC-OPENER · 연:TRN.frame.DC · 정:RULE-DOC-FIXED-PARTS · model: claude-opus-5-5
//! 연결 I/O — UDP 하나 · TCP passive(RFC 4571) · STUN 은 무결성 검증 뒤에만 · DTLS passive → SRTP 키 · SCTP/DCEP `"unreliable"` 하나 · 쌍별 생존 tick.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use bytes::Bytes;
use dtls::conn::DTLSConn;
use dtls::crypto::{Certificate, CryptoPrivateKey};
use dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use sctp_proto::{Association, AssociationHandle, DatagramEvent, Endpoint, EndpointConfig, Event, Payload, PayloadProtocolIdentifier, ReliabilityType, ServerConfig, StreamEvent};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;
use webrtc_srtp::context::Context;
use webrtc_srtp::protection_profile::ProtectionProfile;

use super::core::{self as lc, Edge, Kind, PeerState, Proto};
use super::{lock, Cert, Links, Transport};

pub fn make_cert() -> Result<Cert, String> {
    let mut params = rcgen::CertificateParams::new(vec!["oxlens-sfu".to_string()]).map_err(|e| e.to_string())?;
    let mut dn = rcgen::DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, "oxlens-sfu");
    params.distinguished_name = dn;
    let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| e.to_string())?;
    let cert = params.self_signed(&kp).map_err(|e| e.to_string())?;
    let der = cert.der().to_vec();
    let digest = Sha256::digest(&der);
    let hex: Vec<String> = digest.iter().map(|b| format!("{b:02X}")).collect();
    let private_key = CryptoPrivateKey::from_key_pair(&kp).map_err(|e| e.to_string())?;
    Ok(Cert { dtls: Certificate { certificate: vec![cert.der().to_owned()], private_key }, fingerprint: format!("sha-256 {}", hex.join(":")) })
}

pub async fn bind_udp(ip: IpAddr, port: u16) -> Result<UdpSocket, String> {
    match UdpSocket::bind(SocketAddr::new(ip, port)).await {
        Ok(s) => Ok(s),
        Err(_) => UdpSocket::bind(SocketAddr::new(IpAddr::from([0, 0, 0, 0]), port)).await.map_err(|e| format!("udp {port}: {e}")),
    }
}

pub fn spawn_udp(links: Arc<Links>) {
    tokio::spawn(async move {
        let sock = links.udp.clone();
        let mut buf = vec![0u8; 2048];
        loop {
            match sock.recv_from(&mut buf).await {
                Ok((n, from)) => on_datagram(&links, &buf[..n], from, Proto::Udp, None),
                Err(e) => {
                    tracing::warn!(kind = "diag", err = %e, "udp recv");
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
        }
    });
}

pub fn spawn_tcp(links: Arc<Links>, l: TcpListener) {
    tokio::spawn(async move {
        loop {
            let Ok((s, from)) = l.accept().await else { continue };
            let _ = s.set_nodelay(true);
            let lk = links.clone();
            tokio::spawn(async move { tcp_conn(lk, s, from).await });
        }
    });
}

async fn tcp_conn(links: Arc<Links>, s: tokio::net::TcpStream, from: SocketAddr) {
    let (mut r, mut w) = s.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let writer = tokio::spawn(async move {
        while let Some(f) = rx.recv().await {
            if w.write_all(&f).await.is_err() {
                break;
            }
        }
    });
    let mut d = lc::Deframer::default();
    let mut buf = vec![0u8; 8192];
    let deadline = tokio::time::Instant::now() + Duration::from_millis(lc::TCP_UNVERIFIED_MS);
    let mut known = false;
    loop {
        let n = if known {
            r.read(&mut buf).await
        } else {
            match tokio::time::timeout_at(deadline, r.read(&mut buf)).await {
                Ok(x) => x,
                Err(_) => break,
            }
        };
        let n = match n {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for f in d.push(&buf[..n]) {
            if f.is_empty() {
                continue;
            }
            on_datagram(&links, &f, from, Proto::Tcp, Some(&tx));
            known = known || links.by_addr(&from).is_some();
        }
    }
    if let Some(t) = links.by_addr(&from) {
        t.tcp.remove(&from);
        lock(&t.pairs).tcp_gone(&from);
    }
    writer.abort();
}

fn reply(links: &Links, data: &[u8], to: SocketAddr, proto: Proto, w: Option<&mpsc::UnboundedSender<Vec<u8>>>) {
    match (proto, w) {
        (Proto::Tcp, Some(w)) => {
            if let Some(f) = lc::frame(data) {
                let _ = w.send(f);
            }
        }
        _ => {
            let _ = links.udp.try_send_to(data, to);
        }
    }
}

fn on_datagram(links: &Arc<Links>, data: &[u8], from: SocketAddr, proto: Proto, w: Option<&mpsc::UnboundedSender<Vec<u8>>>) {
    match lc::classify(data) {
        Kind::Stun => on_stun(links, data, from, proto, w),
        Kind::Dtls => {
            let Some(t) = links.by_addr(&from) else {
                links.unknown_addr.fetch_add(1, Ordering::Relaxed);
                return;
            };
            on_dtls(links, &t, data, from);
        }
        Kind::Rtp => {
            let Some(t) = links.by_addr(&from) else {
                links.unknown_addr.fetch_add(1, Ordering::Relaxed);
                return;
            };
            let plain = {
                let mut g = lock(&t.srtp_in);
                match g.as_mut() {
                    Some(c) => c.decrypt_rtp(data).ok(),
                    None => None,
                }
            };
            if let Some(p) = plain {
                t.rx_packets.fetch_add(1, Ordering::Relaxed);
                crate::fwd::shell::ingress(&t, p);
            }
        }
        Kind::Rtcp => {
            let Some(t) = links.by_addr(&from) else { return };
            let plain = {
                let mut g = lock(&t.srtp_in);
                match g.as_mut() {
                    Some(c) => c.decrypt_rtcp(data).ok(),
                    None => None,
                }
            };
            if let Some(p) = plain {
                crate::rtcp::shell::ingress(&t, &p);
            }
        }
        Kind::Other => {}
    }
}

fn on_stun(links: &Arc<Links>, data: &[u8], from: SocketAddr, proto: Proto, w: Option<&mpsc::UnboundedSender<Vec<u8>>>) {
    let Some(b) = lc::parse_binding(data) else { return };
    let ufrag = b.local_ufrag().to_string();
    let Some(t) = links.by_ufrag(&ufrag) else {
        links.stun_bad.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(pwd) = lock(&t.creds).pwd_of(&ufrag).map(str::to_string) else {
        links.stun_bad.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if !b.verify(&pwd) {
        links.stun_bad.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let resp = lc::success(&b.tid, from, &pwd);
    if proto == Proto::Tcp
        && let Some(w) = w {
            t.tcp.insert(from, w.clone());
        }
    reply(links, &resp, from, proto, w);
    links.addrs.insert(from, Arc::downgrade(&t));
    let now = common::clock::mono_ms();
    let swapped = lock(&t.creds).checked(&ufrag);
    if let Some(old) = &swapped {
        links.ufrags.remove(old);
        tracing::info!(kind = "ev", spec = "CP-OP-ICE-RESTART", user = %t.user_id, epoch = %links.sfu_id, from = %old, to = %ufrag, label = "LINK.S.checked_new", "ufrag 교체");
    }
    let prev_route = lock(&t.pairs).route;
    let edge = lock(&t.pairs).checked(from, proto, b.use_candidate, now, t.peer_state());
    if edge == Edge::Alive || (swapped.is_some() && b.use_candidate) {
        let was = t.set_state(PeerState::Alive);
        let since = t.suspect_since.swap(0, Ordering::AcqRel);
        if was == PeerState::Suspect {
            tracing::info!(kind = "ev", spec = "SV-STATE-PEER-STATE", user = %t.user_id, epoch = %links.sfu_id, from = "suspect", to = "alive", pair = %from, proto = ?proto, suspect_ms = now.saturating_sub(since), "PeerState");
        }
    }
    if b.use_candidate && prev_route != Some(from) {
        tracing::info!(kind = "ev", spec = "SV-STATE-PEER-STATE", user = %t.user_id, epoch = %links.sfu_id, from = t.peer_state().name(), to = t.peer_state().name(), pair = %from, proto = ?proto, "경로 쌍 지명");
    }
}

fn on_dtls(links: &Arc<Links>, t: &Arc<Transport>, data: &[u8], from: SocketAddr) {
    lock(&t.dtls_from).replace(from);
    let tx = {
        let mut g = lock(&t.dtls_tx);
        if g.is_none() && !t.closed.load(Ordering::Acquire) {
            let (tx, rx) = mpsc::channel::<Bytes>(256);
            g.replace(tx);
            let tt = t.clone();
            let cert = Certificate { certificate: links.cert.dtls.certificate.clone(), private_key: links.cert.dtls.private_key.clone() };
            let sfu = links.sfu_id.clone();
            let h = tokio::spawn(async move { dtls_run(tt, rx, cert, sfu).await });
            lock(&t.tasks).push(h);
        }
        g.clone()
    };
    if let Some(tx) = tx {
        let _ = tx.try_send(Bytes::copy_from_slice(data));
    }
}

struct DtlsIo {
    rx: tokio::sync::Mutex<mpsc::Receiver<Bytes>>,
    t: Weak<Transport>,
}

#[async_trait::async_trait]
impl webrtc_util::Conn for DtlsIo {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Some(b) => {
                let n = b.len().min(buf.len());
                buf[..n].copy_from_slice(&b[..n]);
                Ok(n)
            }
            None => Err(webrtc_util::Error::Other("closed".to_string())),
        }
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let n = self.recv(buf).await?;
        Ok((n, SocketAddr::from(([0, 0, 0, 0], 0))))
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        match self.t.upgrade() {
            Some(t) if t.send_raw(buf) => Ok(buf.len()),
            Some(_) => Ok(buf.len()),
            None => Err(webrtc_util::Error::Other("gone".to_string())),
        }
    }

    async fn send_to(&self, buf: &[u8], _target: SocketAddr) -> webrtc_util::Result<usize> {
        self.send(buf).await
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(SocketAddr::from(([0, 0, 0, 0], 0)))
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        None
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

async fn dtls_run(t: Arc<Transport>, rx: mpsc::Receiver<Bytes>, cert: Certificate, sfu: String) {
    let io = Arc::new(DtlsIo { rx: tokio::sync::Mutex::new(rx), t: Arc::downgrade(&t) });
    let cfg = dtls::config::Config {
        certificates: vec![cert],
        srtp_protection_profiles: vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80],
        client_auth: dtls::config::ClientAuthType::RequireAnyClientCert,
        insecure_skip_verify: true,
        mtu: 1200,
        ..Default::default()
    };
    let conn = match tokio::time::timeout(Duration::from_secs(30), DTLSConn::new(io, cfg, false, None)).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            tracing::warn!(kind = "diag", user = %t.user_id, epoch = %sfu, err = %e, "DTLS 실패");
            lock(&t.dtls_tx).take();
            return;
        }
        Err(_) => {
            tracing::warn!(kind = "diag", user = %t.user_id, epoch = %sfu, "DTLS 시한");
            lock(&t.dtls_tx).take();
            return;
        }
    };
    let state = conn.connection_state().await;
    let mut sc = webrtc_srtp::config::Config { profile: ProtectionProfile::Aes128CmHmacSha1_80, ..Default::default() };
    if let Err(e) = sc.extract_session_keys_from_dtls(state, false).await {
        tracing::warn!(kind = "diag", user = %t.user_id, err = %e, "SRTP 키");
        return;
    }
    let k = &sc.keys;
    let inn = Context::new(&k.remote_master_key, &k.remote_master_salt, ProtectionProfile::Aes128CmHmacSha1_80, None, None);
    let out = Context::new(&k.local_master_key, &k.local_master_salt, ProtectionProfile::Aes128CmHmacSha1_80, None, None);
    match (inn, out) {
        (Ok(i), Ok(o)) => {
            lock(&t.srtp_in).replace(i);
            lock(&t.srtp_out).replace(o);
        }
        _ => {
            tracing::warn!(kind = "diag", user = %t.user_id, "SRTP 맥락");
            return;
        }
    }
    t.connected_at.store(common::clock::wall_ms(), Ordering::Release);
    tracing::info!(kind = "ev", spec = "SV-STATE-PEER-STATE", user = %t.user_id, epoch = %sfu, role = ?t.role, "DTLS 수립 · SRTP 준비");
    crate::fwd::shell::on_connected(&t);
    sctp_run(t, Arc::new(conn)).await;
}

fn dcep_ack() -> Bytes {
    Bytes::from_static(&[0x02])
}

async fn sctp_run(t: Arc<Transport>, conn: Arc<DTLSConn>) {
    let (in_tx, mut in_rx) = mpsc::channel::<Bytes>(512);
    let c2 = conn.clone();
    let reader = tokio::spawn(async move {
        let mut buf = vec![0u8; 16384];
        while let Ok(n) = c2.read(&mut buf, None).await {
            if in_tx.send(Bytes::copy_from_slice(&buf[..n])).await.is_err() {
                break;
            }
        }
    });
    lock(&t.tasks).push(reader);
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    lock(&t.dc).tx.replace(out_tx);
    let remote = SocketAddr::from(([127, 0, 0, 1], 5000));
    let mut ep = Endpoint::new(Arc::new(EndpointConfig::new()), Some(Arc::new(ServerConfig::new())));
    let mut assoc: Option<(AssociationHandle, Association)> = None;
    let mut sid: Option<u16> = None;
    loop {
        let wake = assoc.as_ref().and_then(|(_, a)| a.poll_timeout());
        let sleep_until = wake.map(tokio::time::Instant::from_std).unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600));
        tokio::select! {
            pkt = in_rx.recv() => {
                let Some(pkt) = pkt else { break };
                if let Some((h, ev)) = ep.handle(Instant::now(), remote, None, None, pkt) {
                    match ev {
                        DatagramEvent::NewAssociation(a) => assoc = Some((h, a)),
                        DatagramEvent::AssociationEvent(e) => {
                            if let Some((_, a)) = assoc.as_mut() {
                                a.handle_event(e);
                            }
                        }
                    }
                }
            }
            msg = out_rx.recv() => {
                let Some(msg) = msg else { break };
                if let (Some((_, a)), Some(id)) = (assoc.as_mut(), sid)
                    && let Ok(mut s) = a.stream(id) {
                        let _ = s.write_sctp(&Bytes::from(msg), PayloadProtocolIdentifier::Binary);
                    }
            }
            _ = tokio::time::sleep_until(sleep_until) => {
                if let Some((_, a)) = assoc.as_mut() {
                    a.handle_timeout(Instant::now());
                }
            }
        }
        while let Some(tr) = ep.poll_transmit() {
            if let Payload::RawEncode(v) = tr.payload {
                for b in v {
                    let _ = conn.write(&b, None).await;
                }
            }
        }
        let Some((h, a)) = assoc.as_mut() else { continue };
        while let Some(ev) = a.poll_endpoint_event() {
            if let Some(ae) = ep.handle_event(*h, ev) {
                a.handle_event(ae);
            }
        }
        let mut inbound: Vec<(u16, PayloadProtocolIdentifier, Vec<u8>)> = Vec::new();
        while let Some(ev) = a.poll() {
            match ev {
                Event::Stream(StreamEvent::Opened { .. }) => {
                    while a.accept_stream().is_some() {}
                }
                Event::Stream(StreamEvent::Readable { id }) => {
                    if let Ok(mut s) = a.stream(id) {
                        while let Ok(Some(ch)) = s.read_sctp() {
                            let mut buf = vec![0u8; ch.len()];
                            if ch.read(&mut buf).is_ok() {
                                inbound.push((id, ch.ppi, buf));
                            }
                        }
                    }
                }
                Event::AssociationLost { .. } => {
                    tracing::info!(kind = "diag", user = %t.user_id, "SCTP 연관 끊김");
                }
                _ => {}
            }
        }
        for (id, ppi, data) in inbound {
            match ppi {
                PayloadProtocolIdentifier::Dcep => {
                    if data.first() == Some(&0x03) && data.len() >= 12 {
                        let ct = data[1];
                        let rel = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                        let ll = u16::from_be_bytes([data[8], data[9]]) as usize;
                        let label = data.get(12..12 + ll).map(|l| String::from_utf8_lossy(l).to_string()).unwrap_or_default();
                        if let Ok(mut s) = a.stream(id) {
                            if label != "unreliable" {
                                tracing::warn!(kind = "diag", user = %t.user_id, label = %label, "DC 라벨 거부");
                                let _ = s.close();
                                continue;
                            }
                            let rt = match ct & 0x7f {
                                1 => ReliabilityType::Rexmit,
                                2 => ReliabilityType::Timed,
                                _ => ReliabilityType::Reliable,
                            };
                            let _ = s.set_reliability_params(ct & 0x80 != 0, rt, rel);
                            let _ = s.write_sctp(&dcep_ack(), PayloadProtocolIdentifier::Dcep);
                        }
                        sid = Some(id);
                        let pending: Vec<Vec<u8>> = {
                            let mut d = lock(&t.dc);
                            d.open = true;
                            d.buf.drain(..).collect()
                        };
                        if let Ok(mut s) = a.stream(id) {
                            for m in pending {
                                let _ = s.write_sctp(&Bytes::from(m), PayloadProtocolIdentifier::Binary);
                            }
                        }
                        tracing::info!(kind = "diag", user = %t.user_id, stream = id, "DC open");
                    }
                }
                PayloadProtocolIdentifier::Binary | PayloadProtocolIdentifier::String => crate::aux::on_dc(&t, &data),
                _ => {}
            }
        }
        let now = Instant::now();
        while let Some(tr) = a.poll_transmit(now) {
            if let Payload::RawEncode(v) = tr.payload {
                for b in v {
                    let _ = conn.write(&b, None).await;
                }
            }
        }
    }
    lock(&t.dc).open = false;
    let _ = conn.close().await;
}

pub fn spawn_tick(links: Arc<Links>) {
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(1000));
        let mut n: u64 = 0;
        loop {
            iv.tick().await;
            n += 1;
            let now = common::clock::mono_ms();
            for p in links.all() {
                for t in p.transports() {
                    let edge = lock(&t.pairs).sweep(now, t.peer_state());
                    if edge == Edge::Suspect {
                        t.set_state(PeerState::Suspect);
                        let _ = t.suspect_since.compare_exchange(0, now, Ordering::AcqRel, Ordering::Acquire);
                        let ufrag = t.ufrag();
                        tracing::info!(kind = "ev", spec = "SV-STATE-PEER-STATE", user = %p.user_id, session = %p.session(), epoch = %links.sfu_id, from = "alive", to = "suspect", ufrag = %ufrag, "PeerState");
                        crate::ops::link_suspect(&p, &ufrag);
                    }
                    if n.is_multiple_of(lc::TCP_KEEPALIVE_MS / 1000) {
                        for w in t.tcp.iter() {
                            let _ = w.value().send(vec![0, 0]);
                        }
                    }
                }
            }
        }
    });
}
