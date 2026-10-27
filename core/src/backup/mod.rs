pub mod crypto;
pub mod import;
pub mod payload;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{CoreError, CoreResult};
use crate::storage::Db;
use payload::Payload;

pub const FILE_EXTENSION: &str = "yitoken";
pub const FILE_FORMAT_NAME: &str = "yi-token 备份";
pub const MIN_PASSPHRASE_CHARS: usize = 8;
pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

const FORMAT: &str = "yi-token.backup";
const FORMAT_VERSION: u32 = 1;
const KDF_ALGORITHM: &str = "pbkdf2-hmac-sha256";
const CIPHER_ALGORITHM: &str = "aes-256-gcm";
const ITERATIONS: std::ops::RangeInclusive<u32> = 10_000..=10_000_000;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub(crate) format: String,
    pub(crate) version: u32,
    pub(crate) kdf: KdfParams,
    pub(crate) cipher: CipherParams,
    pub(crate) payload: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KdfParams {
    pub(crate) algorithm: String,
    pub(crate) iterations: u32,
    pub(crate) salt: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CipherParams {
    pub(crate) algorithm: String,
    pub(crate) nonce: String,
}

#[derive(Serialize)]
struct Header<'a> {
    format: &'a str,
    version: u32,
    kdf: &'a KdfParams,
    cipher: &'a CipherParams,
}

impl Envelope {
    pub(crate) fn aad(&self) -> CoreResult<Vec<u8>> {
        Ok(serde_json::to_vec(&Header {
            format: FORMAT,
            version: FORMAT_VERSION,
            kdf: &self.kdf,
            cipher: &self.cipher,
        })?)
    }
}

pub fn export(db: &Db, passphrase: &SecretString) -> CoreResult<Vec<u8>> {
    let payload = Payload::read(db)?;
    encode(&payload, passphrase)
}

pub fn encode(payload: &Payload, passphrase: &SecretString) -> CoreResult<Vec<u8>> {
    encode_with_iterations(payload, passphrase, crypto::DEFAULT_ITERATIONS)
}

pub fn encode_with_iterations(
    payload: &Payload,
    passphrase: &SecretString,
    iterations: u32,
) -> CoreResult<Vec<u8>> {
    payload.validate()?;
    if !ITERATIONS.contains(&iterations) {
        return Err(CoreError::Validation(format!(
            "迭代次数超出支持范围：{iterations}"
        )));
    }
    let passphrase = normalize_passphrase(passphrase)?;
    let salt = crypto::random_bytes(crypto::SALT_LEN);
    let nonce = crypto::random_bytes(crypto::NONCE_LEN);
    let kdf = KdfParams {
        algorithm: KDF_ALGORITHM.into(),
        iterations,
        salt: BASE64.encode(&salt),
    };
    let cipher = CipherParams {
        algorithm: CIPHER_ALGORITHM.into(),
        nonce: BASE64.encode(&nonce),
    };
    let header = Header {
        format: FORMAT,
        version: FORMAT_VERSION,
        kdf: &kdf,
        cipher: &cipher,
    };
    let aad = serde_json::to_vec(&header)?;
    let plaintext = Zeroizing::new(serde_json::to_vec(payload)?);
    let sealed = crypto::encrypt(&plaintext, &passphrase, &salt, &nonce, &aad, iterations)?;
    let envelope = Envelope {
        format: FORMAT.into(),
        version: FORMAT_VERSION,
        kdf,
        cipher,
        payload: BASE64.encode(sealed),
    };
    let mut bytes = serde_json::to_vec_pretty(&envelope)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn decode(text: &str, passphrase: &SecretString) -> CoreResult<Payload> {
    if text.len() > MAX_FILE_BYTES {
        return Err(CoreError::Validation("备份文件过大".into()));
    }
    let passphrase = normalize_passphrase(passphrase)?;
    let envelope: Envelope = serde_json::from_str(text)
        .map_err(|error| CoreError::Validation(format!("文件格式无法识别：{error}")))?;
    if envelope.format != FORMAT || envelope.version != FORMAT_VERSION {
        return Err(CoreError::Validation(
            "不是 yi-token 备份文件或版本不受支持".into(),
        ));
    }
    if envelope.kdf.algorithm != KDF_ALGORITHM {
        return Err(CoreError::Validation(format!(
            "不支持的密钥派生算法：{}",
            envelope.kdf.algorithm
        )));
    }
    if envelope.cipher.algorithm != CIPHER_ALGORITHM {
        return Err(CoreError::Validation(format!(
            "不支持的加密算法：{}",
            envelope.cipher.algorithm
        )));
    }
    if !ITERATIONS.contains(&envelope.kdf.iterations) {
        return Err(CoreError::Validation(format!(
            "备份文件中的迭代次数超出支持范围：{}",
            envelope.kdf.iterations
        )));
    }
    let plaintext = crypto::decrypt(&envelope, &passphrase)?;
    let payload: Payload = serde_json::from_slice(&plaintext)
        .map_err(|error| CoreError::Validation(format!("备份内容无法解析：{error}")))?;
    payload.validate()?;
    Ok(payload)
}

fn normalize_passphrase(passphrase: &SecretString) -> CoreResult<SecretString> {
    let trimmed = passphrase.expose_secret().trim();
    if trimmed.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(CoreError::Validation(format!(
            "加密字符串至少需要 {MIN_PASSPHRASE_CHARS} 个字符"
        )));
    }
    Ok(SecretString::from(trimmed.to_owned()))
}
