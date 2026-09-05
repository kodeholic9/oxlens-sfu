// author: kodeholic (powered by Claude)
//! DTLS passive + SRTP 키 유도 — 정§12 DTLS·SRTP 행. 서버는 항상 수동이고 클라 지문은 대조하지 않는다(알고 하는 이탈).
//! 키 재료 60B 배치는 RFC 5764 §4.2: client_key(16)·server_key(16)·client_salt(14)·server_salt(14).

use std::sync::Arc;

use ::dtls::config::{Config, ExtendedMasterSecretType};
use ::dtls::conn::DTLSConn;
use ::dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use webrtc_util::KeyingMaterialExporter;
use webrtc_util::conn::Conn;

use super::ServerCert;

const KEY_LABEL: &str = "EXTRACTOR-dtls_srtp";
const MASTER_KEY_LEN: usize = 16;
const MASTER_SALT_LEN: usize = 14;
const MATERIAL_LEN: usize = (MASTER_KEY_LEN + MASTER_SALT_LEN) * 2;

pub type DtlsConn = DTLSConn;

/// 정§12 — 프로파일은 `AES128-CM-HMAC-SHA1-80` 하나.
pub fn server_config(cert: &ServerCert) -> Config {
    Config {
        certificates: vec![cert.cert.clone()],
        srtp_protection_profiles: vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80],
        extended_master_secret: ExtendedMasterSecretType::Require,
        insecure_skip_verify: true,
        ..Default::default()
    }
}

pub async fn accept(conn: Arc<dyn Conn + Send + Sync>, config: Config) -> Result<DtlsConn, ::dtls::Error> {
    DTLSConn::new(conn, config, false, None).await
}

/// 방향마다 키·솔트 한 쌍. inbound = client_*, outbound = server_*.
pub struct SrtpKeys {
    pub client_key: Vec<u8>,
    pub client_salt: Vec<u8>,
    pub server_key: Vec<u8>,
    pub server_salt: Vec<u8>,
}

/// RFC 5705 exporter — context 는 반드시 빈 슬라이스다(아니면 `ContextUnsupported`).
pub async fn export_srtp_keys(conn: &DTLSConn) -> Result<SrtpKeys, String> {
    let material = conn
        .connection_state()
        .await
        .export_keying_material(KEY_LABEL, &[], MATERIAL_LEN)
        .await
        .map_err(|e| format!("export_keying_material: {e:?}"))?;
    if material.len() != MATERIAL_LEN {
        return Err(format!("keying material {} bytes, want {MATERIAL_LEN}", material.len()));
    }
    let salt = MASTER_KEY_LEN * 2;
    Ok(SrtpKeys {
        client_key: material[..MASTER_KEY_LEN].to_vec(),
        server_key: material[MASTER_KEY_LEN..salt].to_vec(),
        client_salt: material[salt..salt + MASTER_SALT_LEN].to_vec(),
        server_salt: material[salt + MASTER_SALT_LEN..].to_vec(),
    })
}
