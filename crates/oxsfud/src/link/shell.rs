// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-LINK-TRANSPORT · 정:RULE-LINK-PAIR-LIVENESS · 정:CAND-LINK-DTLS-VALUES · 정:RULE-GC-LEAVE-SEVEN · 연:RULE-LINK-SDP-DC-OPENER · model: claude-opus-5-5
//! LINK 쉘 — UDP 수신 고리 · ICE-TCP passive(RFC 4571 · 신원 확인 10s · keepalive 20s) · 첫 바이트 demux · DTLS passive · SRTP 키 · SCTP/DCEP(`unreliable` 하나 · 재전송 한도 없음).

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use bytes::Bytes;
use sctp_proto::{Association, AssociationHandle, DatagramEvent, Endpoint, EndpointConfig, Event, Payload, PayloadProtocolIdentifier, ReliabilityType, ServerConfig, StreamEvent};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use super::core::{Deframer, Frame, PathKey, TCP_IDENTIFY_MS, TCP_KEEPALIVE_MS};
use super::stun::{self, Class};
use super::{lock, Net, Transport};

pub const DTLS_HANDSHAKE_SECS: u64 = 30;
pub const DTLS_MTU: usize = 1200;
pub const CERT_NAME: &str = "ox-sfu";
const DCEP_OPEN: u8 = 0x03;
const DCEP_ACK: u8 = 0x02;

pub fn make_cert() -> Result<(dtls::crypto::Certificate, String), String> {
    let mut params = rcgen::CertificateParams::new(vec![CERT_NAME.to_string()]).map_err(|e| e.to_string())?;
    let mut dn = rcgen::DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, CERT_NAME);
    params.distinguished_name = dn;
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| e.to_string())?;
    let cert = params.self_signed(&key).map_err(|e| e.to_string())?;
    let der = cert.der().clone();
    let digest = Sha256::digest(der.as_ref());
    let fp = digest.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
    let private_key = dtls::crypto::CryptoPrivateKey::from_key_pair(&key).map_err(|e| e.to_string())?;
    Ok((dtls::crypto::Certificate { certificate: vec![der], private_key }, format!("sha-256 {fp}")))
}

pub async fn udp_loop(net: Arc<Net>) {
    let mut buf = vec![0u8; 2048];
    loop {
        match net.udp.recv_from(&mut buf).await {
            Ok((n, from)) => on_packet(&net, PathKey::Udp(from), &buf[..n], Some(from)),
            Err(e) => {
                tracing::warn!(kind = "diag", "udp recv: {e}");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    }
}

pub fn on_packet(net: &Arc<Net>, k: PathKey, b: &[u8], from: Option<SocketAddr>) {
    match stun::classify(b) {
        Class::Stun => on_stun(net, k, b, from),
        Class::Dtls => match net.by_path(&k) {
            Some(t) => {
                if t.dtls_in.try_send(b.to_vec()).is_err() {
                    net.counters.unknown_path.fetch_add(1, Ordering::Relaxed);
                }
            }
            None => {
                net.counters.unknown_path.fetch_add(1, Ordering::Relaxed);
            }
        },
        Class::Rtp => {
            let Some(t) = net.by_path(&k) else {
                net.counters.unknown_path.fetch_add(1, Ordering::Relaxed);
                return;
            };
            let rtcp = stun::is_rtcp(b);
            let plain = {
                let mut g = lock(&t.srtp_rx);
                let Some(ctx) = g.as_mut() else { return };
                let r = if rtcp { ctx.decrypt_rtcp(b) } else { ctx.decrypt_rtp(b) };
                match r {
                    Ok(p) => p,
                    Err(_) => {
                        net.counters.srtp_fail.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                }
            };
            if let Some(up) = net.up() {
                up.media(&t, &plain, rtcp);
            }
        }
        Class::Other => {
            net.counters.demux_unknown.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn on_stun(net: &Arc<Net>, k: PathKey, b: &[u8], from: Option<SocketAddr>) {
    let Some(req) = stun::parse_request(b) else {
        net.counters.stun_malformed.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let ufrag = req.local_ufrag().to_string();
    let Some(t) = net.by_ufrag(&ufrag) else {
        net.counters.stun_unknown_ufrag.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(pwd) = lock(&t.creds).pwd_of(&ufrag).map(str::to_string) else {
        net.counters.stun_unknown_ufrag.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if !stun::integrity_ok(b, &req, &pwd) || !stun::fingerprint_ok(b) {
        net.counters.stun_integrity_fail.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let now = common::time::mono_ms();
    t.on_check(net, k, &ufrag, req.use_candidate, now);
    let mapped = from.unwrap_or_else(|| SocketAddr::new(net.ip, net.port));
    let res = stun::success(&req, mapped, &pwd);
    net.send_to(k, &res);
}

pub async fn tcp_loop(net: Arc<Net>, l: TcpListener) {
    loop {
        match l.accept().await {
            Ok((s, from)) => {
                let _ = s.set_nodelay(true);
                let id = net.next_id();
                tokio::spawn(tcp_conn(net.clone(), s, from, id));
            }
            Err(e) => {
                tracing::warn!(kind = "diag", "tcp accept: {e}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

async fn tcp_conn(net: Arc<Net>, s: TcpStream, from: SocketAddr, id: u64) {
    let (mut rd, mut wr) = s.into_split();
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(1024);
    net.add_tcp(id, tx);
    let writer = tokio::spawn(async move {
        while let Some(b) = rx.recv().await {
            if wr.write_all(&b).await.is_err() {
                break;
            }
        }
    });
    let k = PathKey::Tcp(id);
    let mut d = Deframer::default();
    let mut buf = vec![0u8; 8192];
    let started = Instant::now();
    loop {
        let identified = net.by_path(&k).is_some();
        let wait = if identified { Duration::from_secs(3600) } else { Duration::from_millis(TCP_IDENTIFY_MS).saturating_sub(started.elapsed()) };
        let n = match tokio::time::timeout(wait, rd.read(&mut buf)).await {
            Err(_) if !identified => {
                net.counters.tcp_unidentified.fetch_add(1, Ordering::Relaxed);
                break;
            }
            Err(_) => continue,
            Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(n)) => n,
        };
        d.push(&buf[..n]);
        let mut bad = false;
        while let Some(f) = d.next_frame() {
            match f {
                Frame::Keepalive => {}
                Frame::Packet(p) => {
                    if stun::classify(&p) == Class::Other {
                        net.counters.tcp_bad_frame.fetch_add(1, Ordering::Relaxed);
                        bad = true;
                        break;
                    }
                    on_packet(&net, k, &p, Some(from));
                }
            }
        }
        if bad {
            break;
        }
    }
    if let Some(t) = net.by_path(&k) {
        lock(&t.pairs).tcp_closed(id);
    }
    net.drop_tcp(id);
    writer.abort();
}

pub async fn keepalive_loop(net: Arc<Net>) {
    let mut iv = tokio::time::interval(Duration::from_millis(TCP_KEEPALIVE_MS));
    loop {
        iv.tick().await;
        for id in net.tcp_ids() {
            if net.by_path(&PathKey::Tcp(id)).is_some() {
                net.send_to(PathKey::Tcp(id), &[]);
            }
        }
    }
}

struct ChanConn {
    rx: AsyncMutex<mpsc::Receiver<Vec<u8>>>,
    t: Weak<Transport>,
    local: SocketAddr,
}

#[async_trait::async_trait]
impl webrtc_util::Conn for ChanConn {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let mut g = self.rx.lock().await;
        match g.recv().await {
            Some(p) => {
                let n = p.len().min(buf.len());
                buf[..n].copy_from_slice(&p[..n]);
                Ok(n)
            }
            None => Err(webrtc_util::Error::ErrClosedListener),
        }
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let n = self.recv(buf).await?;
        Ok((n, self.local))
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        match self.t.upgrade() {
            Some(t) => {
                t.send_raw(buf);
                Ok(buf.len())
            }
            None => Err(webrtc_util::Error::ErrClosedListener),
        }
    }

    async fn send_to(&self, buf: &[u8], _target: SocketAddr) -> webrtc_util::Result<usize> {
        self.send(buf).await
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.local)
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

pub async fn dtls_task(t: Weak<Transport>, rx: mpsc::Receiver<Vec<u8>>, net: Arc<Net>) {
    let conn = Arc::new(ChanConn { rx: AsyncMutex::new(rx), t: t.clone(), local: SocketAddr::new(net.ip, net.port) });
    let cfg = dtls::config::Config {
        certificates: vec![net.cert.clone()],
        srtp_protection_profiles: vec![dtls::extension::extension_use_srtp::SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80],
        client_auth: dtls::config::ClientAuthType::RequireAnyClientCert,
        insecure_skip_verify: true,
        extended_master_secret: dtls::config::ExtendedMasterSecretType::Request,
        mtu: DTLS_MTU,
        ..Default::default()
    };
    let started = tokio::time::timeout(Duration::from_secs(DTLS_HANDSHAKE_SECS), dtls::conn::DTLSConn::new(conn, cfg, false, None)).await;
    let Some(tr) = t.upgrade() else { return };
    let dc = match started {
        Ok(Ok(c)) => Arc::new(c),
        Ok(Err(e)) => {
            tracing::warn!(kind = "diag", user = %tr.user, session = %tr.session, role = tr.role.as_str(), "dtls handshake failed: {e}");
            return;
        }
        Err(_) => {
            tracing::warn!(kind = "diag", user = %tr.user, session = %tr.session, role = tr.role.as_str(), "dtls handshake timed out ({DTLS_HANDSHAKE_SECS}s)");
            return;
        }
    };
    let mut keys = webrtc_srtp::config::Config { profile: webrtc_srtp::protection_profile::ProtectionProfile::Aes128CmHmacSha1_80, ..Default::default() };
    let state = dc.connection_state().await;
    if let Err(e) = keys.extract_session_keys_from_dtls(state, false).await {
        tracing::warn!(kind = "diag", user = %tr.user, "srtp keys: {e}");
        return;
    }
    let profile = keys.profile;
    let rx_ctx = webrtc_srtp::context::Context::new(&keys.keys.remote_master_key, &keys.keys.remote_master_salt, profile, None, None);
    let tx_ctx = webrtc_srtp::context::Context::new(&keys.keys.local_master_key, &keys.keys.local_master_salt, profile, None, None);
    match (rx_ctx, tx_ctx) {
        (Ok(r), Ok(x)) => {
            let _ = lock(&tr.srtp_rx).replace(r);
            let _ = lock(&tr.srtp_tx).replace(x);
        }
        _ => {
            tracing::warn!(kind = "diag", user = %tr.user, "srtp context build failed");
            return;
        }
    }
    tr.dtls_up.store(true, Ordering::Release);
    tracing::info!(kind = "ev", spec = "RULE-LINK-TRANSPORT", user = %tr.user, session = %tr.session, role = tr.role.as_str(), "dtls up — srtp keys exported");
    let (app_tx, app_rx) = mpsc::channel::<Vec<u8>>(1024);
    let reader = {
        let dc = dc.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65_536];
            while let Ok(n) = dc.read(&mut buf, None).await {
                if app_tx.send(buf[..n].to_vec()).await.is_err() {
                    break;
                }
            }
        })
    };
    tr.add_task(reader);
    let stop = tr.stop.subscribe();
    drop(tr);
    sctp_loop(t, dc, app_rx, stop).await;
}

fn dcep_open(b: &[u8]) -> Option<(u8, String)> {
    if b.len() < 12 || b[0] != DCEP_OPEN {
        return None;
    }
    let chan_type = b[1];
    let label_len = u16::from_be_bytes([b[8], b[9]]) as usize;
    let label = b.get(12..12 + label_len).map(|l| String::from_utf8_lossy(l).into_owned())?;
    Some((chan_type, label))
}

fn reliability(chan_type: u8) -> (bool, ReliabilityType) {
    let unordered = chan_type & 0x80 != 0;
    let rel = match chan_type & 0x7f {
        0x01 => ReliabilityType::Rexmit,
        0x02 => ReliabilityType::Timed,
        _ => ReliabilityType::Reliable,
    };
    (unordered, rel)
}

async fn flush(assoc: &mut Association, dc: &dtls::conn::DTLSConn) {
    let now = Instant::now();
    while let Some(tx) = assoc.poll_transmit(now) {
        if let Payload::RawEncode(chunks) = tx.payload {
            for c in chunks {
                let _ = dc.write(&c, None).await;
            }
        }
    }
}

async fn sctp_loop(t: Weak<Transport>, dc: Arc<dtls::conn::DTLSConn>, mut app_rx: mpsc::Receiver<Vec<u8>>, mut stop: tokio::sync::watch::Receiver<bool>) {
    let mut ep = Endpoint::new(Arc::new(EndpointConfig::new()), Some(Arc::new(ServerConfig::new())));
    let remote: SocketAddr = SocketAddr::from(([0, 0, 0, 0], 5000));
    let mut assoc: Option<(AssociationHandle, Association)> = None;
    let mut chan: Option<u16> = None;
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(1024);
    let mut buf = vec![0u8; 65_536];
    loop {
        let deadline = assoc.as_ref().and_then(|(_, a)| a.poll_timeout());
        let sleep = async {
            match deadline {
                Some(d) => tokio::time::sleep_until(tokio::time::Instant::from_std(d)).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = stop.changed() => break,
            got = app_rx.recv() => {
                let Some(pkt) = got else { break };
                let Some((ch, ev)) = ep.handle(Instant::now(), remote, None, None, Bytes::from(pkt)) else { continue };
                match ev {
                    DatagramEvent::NewAssociation(a) => assoc = Some((ch, a)),
                    DatagramEvent::AssociationEvent(e) => {
                        if let Some((_, a)) = assoc.as_mut() {
                            a.handle_event(e);
                        }
                    }
                }
            }
            got = out_rx.recv() => {
                let Some(frame) = got else { continue };
                if let (Some((_, a)), Some(id)) = (assoc.as_mut(), chan)
                    && let Ok(mut s) = a.stream(id)
                {
                    let _ = s.write_with_ppi(&frame, PayloadProtocolIdentifier::Binary);
                }
            }
            _ = sleep => {
                if let Some((_, a)) = assoc.as_mut() {
                    a.handle_timeout(Instant::now());
                }
            }
        }
        let Some((ch, a)) = assoc.as_mut() else { continue };
        while let Some(ev) = a.poll_endpoint_event() {
            if let Some(back) = ep.handle_event(*ch, ev) {
                a.handle_event(back);
            }
        }
        while let Some(ev) = a.poll() {
            match ev {
                Event::Stream(StreamEvent::Readable { id }) | Event::Stream(StreamEvent::Opened { id }) => {
                    while a.accept_stream().is_some() {}
                    loop {
                        let chunks = match a.stream(id).and_then(|mut s| s.read_sctp()) {
                            Ok(Some(c)) => c,
                            _ => break,
                        };
                        let n = chunks.read(&mut buf).unwrap_or(0);
                        let data = buf[..n].to_vec();
                        match chunks.ppi {
                            PayloadProtocolIdentifier::Dcep => {
                                if let Some((chan_type, label)) = dcep_open(&data) {
                                    let Some(tr) = t.upgrade() else { break };
                                    if label == oxsig::dc::CHANNEL_LABEL {
                                        if let Ok(mut s) = a.stream(id) {
                                            let (unordered, rel) = reliability(chan_type);
                                            let val = if data.len() >= 8 { u32::from_be_bytes([data[4], data[5], data[6], data[7]]) } else { 0 };
                                            let _ = s.set_reliability_params(unordered, rel, val);
                                            let _ = s.write_with_ppi(&[DCEP_ACK], PayloadProtocolIdentifier::Dcep);
                                        }
                                        chan = Some(id);
                                        tr.dc.opened(out_tx.clone());
                                        tracing::info!(kind = "ev", spec = "RULE-LINK-TRANSPORT", user = %tr.user, session = %tr.session, label = %label, chan_type, "data channel open");
                                        if let Some(net) = tr.net.upgrade()
                                            && let Some(up) = net.up()
                                        {
                                            up.dc_open(&tr);
                                        }
                                    } else {
                                        tracing::warn!(kind = "diag", user = %tr.user, label = %label, "data channel label rejected");
                                        if let Ok(mut s) = a.stream(id) {
                                            let _ = s.close();
                                        }
                                    }
                                }
                            }
                            PayloadProtocolIdentifier::Binary | PayloadProtocolIdentifier::String => {
                                if Some(id) == chan
                                    && let Some(tr) = t.upgrade()
                                    && let Some(net) = tr.net.upgrade()
                                    && let Some(up) = net.up()
                                {
                                    up.dc(&tr, &data);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::Connected => {
                    if let Some(tr) = t.upgrade() {
                        tracing::info!(kind = "ev", spec = "RULE-LINK-TRANSPORT", user = %tr.user, session = %tr.session, "sctp association up");
                    }
                }
                _ => {}
            }
        }
        flush(a, &dc).await;
    }
    let _ = dc.close().await;
}
