// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§12 · model: claude-opus-5

//! DTLS passive + SRTP 키 뽑기.
//!
//! ★**서버는 항상 passive** 다(정§12) — 그래서 클라가 `active` 이고 ClientHello 를 먼저 낸다.
//!
//! RFC 5764 §4.2 — 뽑은 60바이트의 자리(`AES_CM_128_HMAC_SHA1_80`)
//! ```text
//!   [ 0..16] client_write_key    받기(클라 → 서버)
//!   [16..32] server_write_key    보내기(서버 → 클라)
//!   [32..46] client_write_salt
//!   [46..60] server_write_salt
//! ```

use std::sync::Arc;

use dtls::config::{Config, ExtendedMasterSecretType};
use dtls::conn::DTLSConn;
use dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use webrtc_util::conn::Conn;
use webrtc_util::KeyingMaterialExporter;

/// 이 프로세스가 내미는 인증서 — ★**이름 하나로 부른다**(부르는 쪽이 크레이트 경로를 몰라도 된다).
pub type Certificate = dtls::crypto::Certificate;

const SRTP_LABEL: &str = "EXTRACTOR-dtls_srtp";
const KEY_LEN: usize = 16;
const SALT_LEN: usize = 14;
const MATERIAL_LEN: usize = (KEY_LEN + SALT_LEN) * 2;

/// ★**프로파일 하나다**(정§12) — 고르는 축을 두면 양쪽이 다른 것을 고르는 창이 생긴다.
pub fn server_config(cert: &dtls::crypto::Certificate) -> Config {
    Config {
        certificates: vec![cert.clone()],
        srtp_protection_profiles: vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80],
        extended_master_secret: ExtendedMasterSecretType::Require,
        // ★**클라 지문은 대조하지 않는다** — 신원은 토큰뿐이다(정§12 *"알고 하는 이탈"*).
        insecure_skip_verify: true,
        ..Default::default()
    }
}

/// ★`is_client = false` — passive 다.
pub async fn accept(conn: Arc<dyn Conn + Send + Sync>, config: Config) -> Result<DTLSConn, dtls::Error> {
    DTLSConn::new(conn, config, false, None).await
}

/// SRTP 열쇠 넉 장.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrtpKeys {
    pub client_key: Vec<u8>,
    pub server_key: Vec<u8>,
    pub client_salt: Vec<u8>,
    pub server_salt: Vec<u8>,
}

/// 선 DTLS 에서 SRTP 재료를 뽑는다(RFC 5705 → RFC 5764 §4.2).
///
/// ★`context` 는 **빈 것**이어야 한다 — 주면 `ContextUnsupported` 로 죽는다.
pub async fn export_srtp(conn: &DTLSConn) -> Result<SrtpKeys, String> {
    let state = conn.connection_state().await;
    let m: Vec<u8> = state
        .export_keying_material(SRTP_LABEL, &[], MATERIAL_LEN)
        .await
        .map_err(|e| format!("srtp 재료를 못 뽑았다: {e:?}"))?;
    if m.len() != MATERIAL_LEN {
        return Err(format!("srtp 재료 {}바이트 — {MATERIAL_LEN} 이어야 한다", m.len()));
    }
    Ok(SrtpKeys {
        client_key: m[0..KEY_LEN].to_vec(),
        server_key: m[KEY_LEN..KEY_LEN * 2].to_vec(),
        client_salt: m[KEY_LEN * 2..KEY_LEN * 2 + SALT_LEN].to_vec(),
        server_salt: m[KEY_LEN * 2 + SALT_LEN..].to_vec(),
    })
}
