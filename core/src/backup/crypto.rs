use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use pbkdf2::pbkdf2_hmac_array;
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;
use zeroize::Zeroizing;

use super::Envelope;
use crate::error::{CoreError, CoreResult};

pub const KEY_LEN: usize = 32;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 12;
pub const DEFAULT_ITERATIONS: u32 = 600_000;

pub fn random_bytes(len: usize) -> Zeroizing<Vec<u8>> {
    let mut bytes = Zeroizing::new(vec![0u8; len]);
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

pub fn derive_key(
    passphrase: &SecretString,
    salt: &[u8],
    iterations: u32,
) -> Zeroizing<[u8; KEY_LEN]> {
    Zeroizing::new(pbkdf2_hmac_array::<Sha256, KEY_LEN>(
        passphrase.expose_secret().as_bytes(),
        salt,
        iterations,
    ))
}

pub fn encrypt(
    plaintext: &[u8],
    passphrase: &SecretString,
    salt: &[u8],
    nonce: &[u8],
    header_aad: &[u8],
    iterations: u32,
) -> CoreResult<Vec<u8>> {
    if salt.len() != SALT_LEN {
        return Err(CoreError::Validation("加密盐长度无效".into()));
    }
    if nonce.len() != NONCE_LEN {
        return Err(CoreError::Validation("加密 nonce 长度无效".into()));
    }
    let key = derive_key(passphrase, salt, iterations);
    cipher(&key)?
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad: header_aad,
            },
        )
        .map_err(|_| CoreError::Internal("加密失败".into()))
}

pub fn decrypt(envelope: &Envelope, passphrase: &SecretString) -> CoreResult<Zeroizing<Vec<u8>>> {
    let salt = decode_base64(&envelope.kdf.salt, "盐")?;
    if salt.len() != SALT_LEN {
        return Err(CoreError::Validation("备份文件中的盐长度无效".into()));
    }
    let nonce = decode_base64(&envelope.cipher.nonce, "nonce")?;
    if nonce.len() != NONCE_LEN {
        return Err(CoreError::Validation("备份文件中的 nonce 长度无效".into()));
    }
    let sealed = decode_base64(&envelope.payload, "密文")?;
    let key = derive_key(passphrase, &salt, envelope.kdf.iterations);
    let plaintext = cipher(&key)?
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &sealed,
                aad: &envelope.aad()?,
            },
        )
        .map_err(|_| CoreError::Validation("口令错误或文件已损坏".into()))?;
    Ok(Zeroizing::new(plaintext))
}

fn cipher(key: &[u8; KEY_LEN]) -> CoreResult<Aes256Gcm> {
    Aes256Gcm::new_from_slice(key).map_err(|_| CoreError::Internal("加密密钥长度无效".into()))
}

fn decode_base64(value: &str, field: &str) -> CoreResult<Vec<u8>> {
    BASE64.decode(value).map_err(|error| {
        CoreError::Validation(format!("备份文件中的 {field} 不是有效的 base64：{error}"))
    })
}
