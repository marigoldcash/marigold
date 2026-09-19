//!
//! Wallet data encryption module.
//!

use crate::imports::*;
use crate::result::Result;
use argon2::Argon2;
use chacha20poly1305::{
    Key, XChaCha20Poly1305,
    aead::{AeadCore, AeadInPlace, KeyInit, OsRng},
};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::ops::{Deref, DerefMut};
use zeroize::Zeroize;

/// Encryption algorithms supported by the Wallet framework.
#[derive(Default, Clone, Copy, Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum EncryptionKind {
    #[default]
    XChaCha20Poly1305,
}

/// Abstract data container that can contain either plain or encrypted data and
/// transform the data between the two states.
#[derive(Clone, Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(tag = "encryptable", content = "payload")]
pub enum Encryptable<T> {
    #[serde(rename = "plain")]
    Plain(T),
    #[serde(rename = "xchacha20poly1305")]
    XChaCha20Poly1305(Encrypted),
}

impl<T> Zeroize for Encryptable<T>
where
    T: Zeroize,
{
    fn zeroize(&mut self) {
        match self {
            Self::Plain(t) => t.zeroize(),
            Self::XChaCha20Poly1305(e) => e.zeroize(),
        }
    }
}

impl<T> Encryptable<T>
where
    T: Clone + Zeroize + BorshDeserialize + BorshSerialize,
{
    pub fn is_encrypted(&self) -> bool {
        !matches!(self, Self::Plain(_))
    }

    pub fn decrypt(&self, secret: Option<&Secret>) -> Result<Decrypted<T>> {
        match self {
            Self::Plain(v) => Ok(Decrypted::new(v.clone())),
            Self::XChaCha20Poly1305(v) => {
                if let Some(secret) = secret {
                    Ok(v.decrypt(secret)?)
                } else {
                    Err("Decryption secret is 'None' when the data is encrypted!".into())
                }
            }
        }
    }

    pub fn encrypt(&self, secret: &Secret, encryption_kind: EncryptionKind) -> Result<Encrypted> {
        match self {
            Self::Plain(v) => Ok(Decrypted::new(v.clone()).encrypt(secret, encryption_kind)?),
            Self::XChaCha20Poly1305(v) => match encryption_kind {
                EncryptionKind::XChaCha20Poly1305 => Ok(v.clone()),
            },
        }
    }

    pub fn into_encrypted(&self, secret: &Secret, encryption_kind: EncryptionKind) -> Result<Self> {
        match self {
            Self::Plain(v) => Ok(Self::XChaCha20Poly1305(Decrypted::new(v.clone()).encrypt(secret, encryption_kind)?)),
            Self::XChaCha20Poly1305(v) => Ok(Self::XChaCha20Poly1305(v.clone())),
        }
    }

    pub fn into_decrypted(self, secret: &Secret) -> Result<Self> {
        match self {
            Self::Plain(v) => Ok(Self::Plain(v)),
            Self::XChaCha20Poly1305(v) => Ok(Self::Plain(v.decrypt::<T>(secret)?.unwrap())),
        }
    }
}

impl<T> From<T> for Encryptable<T> {
    fn from(value: T) -> Self {
        Encryptable::Plain(value)
    }
}

/// Abstract decrypted data container.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct Decrypted<T>(pub(crate) T)
where
    T: BorshSerialize + BorshDeserialize;

impl<T> AsRef<T> for Decrypted<T>
where
    T: BorshSerialize + BorshDeserialize,
{
    fn as_ref(&self) -> &T {
        &self.0
    }
}

impl<T> Deref for Decrypted<T>
where
    T: BorshSerialize + BorshDeserialize,
{
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Decrypted<T>
where
    T: BorshSerialize + BorshDeserialize,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> AsMut<T> for Decrypted<T>
where
    T: BorshSerialize + BorshDeserialize,
{
    fn as_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> Decrypted<T>
where
    T: BorshSerialize + BorshDeserialize,
{
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn encrypt(&self, secret: &Secret, encryption_kind: EncryptionKind) -> Result<Encrypted> {
        let bytes = borsh::to_vec(&self.0)?;
        let encrypted = match encryption_kind {
            EncryptionKind::XChaCha20Poly1305 => encrypt_salted(bytes.as_slice(), secret)?,
        };
        Ok(Encrypted::new(encryption_kind, encrypted))
    }

    pub fn unwrap(self) -> T {
        self.0
    }
}

/// Encrypted data container (wraps an encrypted payload)
#[derive(Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Encrypted {
    encryption_kind: EncryptionKind,
    payload: Vec<u8>,
}

impl Zeroize for Encrypted {
    fn zeroize(&mut self) {
        self.payload.zeroize();
    }
}

impl std::fmt::Debug for Encrypted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encrypted").field("encryption_kind", &self.encryption_kind).field("payload", &self.payload.to_hex()).finish()
    }
}

impl Encrypted {
    pub fn new(encryption_kind: EncryptionKind, payload: Vec<u8>) -> Self {
        Encrypted { encryption_kind, payload }
    }

    pub fn replace(&mut self, from: Encrypted) {
        self.payload = from.payload;
    }

    pub fn kind(&self) -> EncryptionKind {
        self.encryption_kind
    }

    /// True while this payload still uses the password-derived Argon2 salt.
    pub fn is_legacy(&self) -> bool {
        is_legacy_encryption(&self.payload)
    }

    pub fn decrypt<T>(&self, secret: &Secret) -> Result<Decrypted<T>>
    where
        T: BorshSerialize + BorshDeserialize,
    {
        match self.encryption_kind {
            EncryptionKind::XChaCha20Poly1305 => {
                let (decrypted, _legacy) = decrypt_salted_or_legacy(&self.payload, secret)?;
                Ok(Decrypted(T::try_from_slice(decrypted.as_ref())?))
            }
        }
    }
}

/// Produces `SHA256` hash of the given data.
#[inline]
pub fn sha256_hash(data: &[u8]) -> Secret {
    let mut sha256 = Sha256::default();
    sha256.update(data);
    Secret::new(sha256.finalize().to_vec())
}

/// Produces `SHA256d` hash of the given data.
#[inline]
pub fn sha256d_hash(data: &[u8]) -> Secret {
    let mut sha256 = Sha256::default();
    sha256.update(data);
    sha256_hash(sha256.finalize().as_slice())
}

/// Argon2 with an explicit, caller-supplied salt.
///
/// [`argon2_sha256iv_hash`] derives its salt from the password itself
/// (`sha256(password)`), which makes the whole derivation deterministic: the
/// same password yields the same key in every wallet on earth, so an attacker
/// can precompute once and test against every vault ever made. A salt exists
/// precisely to stop that, and a salt that is a function of the secret is not
/// a salt. Callers pass 32 random bytes, stored in the clear beside the
/// ciphertext — a salt is not secret, only unique.
pub fn argon2_hash_with_salt(data: &[u8], salt: &[u8], byte_length: usize) -> Result<Secret> {
    let mut key = vec![0u8; byte_length];
    Argon2::default().hash_password_into(data, salt, &mut key)?;
    Ok(Secret::new(key))
}

/// Encrypts under a password with an explicit random salt. The salt is NOT
/// stored here — the caller owns the container format and writes it alongside.
pub fn encrypt_xchacha20poly1305_with_salt(data: &[u8], secret: &Secret, salt: &[u8]) -> Result<Vec<u8>> {
    let private_key_bytes = argon2_hash_with_salt(secret.as_ref(), salt, 32)?;
    let key = Key::from_slice(private_key_bytes.as_ref());
    let cipher = XChaCha20Poly1305::new(key);
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut buffer = data.to_vec();
    buffer.reserve(16);
    cipher.encrypt_in_place(&nonce, &[], &mut buffer)?;
    buffer.splice(0..0, nonce.iter().cloned());
    Ok(buffer)
}

/// Decrypts data produced by [`encrypt_xchacha20poly1305_with_salt`].
pub fn decrypt_xchacha20poly1305_with_salt(data: &[u8], secret: &Secret, salt: &[u8]) -> Result<Secret> {
    if data.len() < 24 {
        return Err("ciphertext shorter than the nonce prefix".into());
    }
    let private_key_bytes = argon2_hash_with_salt(secret.as_ref(), salt, 32)?;
    let key = Key::from_slice(private_key_bytes.as_ref());
    let cipher = XChaCha20Poly1305::new(key);
    let nonce = &data[0..24];
    let mut buffer = data[24..].to_vec();
    cipher.decrypt_in_place(nonce.into(), &[], &mut buffer)?;
    Ok(Secret::new(buffer))
}

/// Produces `argon2sha256iv` hash of the given data.
pub fn argon2_sha256iv_hash(data: &[u8], byte_length: usize) -> Result<Secret> {
    let salt = sha256_hash(data);
    let mut key = vec![0u8; byte_length];
    Argon2::default().hash_password_into(data, salt.as_ref(), &mut key)?;
    Ok(key.into())
}

/// Encrypts the given data using `XChaCha20Poly1305` algorithm.
pub fn encrypt_xchacha20poly1305(data: &[u8], secret: &Secret) -> Result<Vec<u8>> {
    let private_key_bytes = argon2_sha256iv_hash(secret.as_ref(), 32)?;
    let key = Key::from_slice(private_key_bytes.as_ref());
    let cipher = XChaCha20Poly1305::new(key);
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng); // 96-bits; unique per message
    let mut buffer = data.to_vec();
    buffer.reserve(16);
    cipher.encrypt_in_place(&nonce, &[], &mut buffer)?;
    buffer.splice(0..0, nonce.iter().cloned());
    Ok(buffer)
}

/// Decrypts the given data using `XChaCha20Poly1305` algorithm.
pub fn decrypt_xchacha20poly1305(data: &[u8], secret: &Secret) -> Result<Secret> {
    let private_key_bytes = argon2_sha256iv_hash(secret.as_ref(), 32)?;
    let key = Key::from_slice(private_key_bytes.as_ref());
    let cipher = XChaCha20Poly1305::new(key);
    let nonce = &data[0..24];
    let mut buffer = data[24..].to_vec();
    cipher.decrypt_in_place(nonce.into(), &[], &mut buffer)?;
    Ok(Secret::new(buffer))
}

/// Marker for the salted container. A payload that does not begin with it was
/// written by software that derived its Argon2 salt from the password itself
/// (`sha256(password)`) — deterministic, so one precomputation attacks every
/// wallet ever made with that password. The bytes are opaque to Borsh, so
/// versioning them here needs no change to any file format that carries them:
/// the wallet payload, its transaction records, and the private key data
/// wrapped under a bip39 passphrase all pass through this one container.
const SALTED_MAGIC: &[u8; 4] = b"MGS2";
const SALT_LEN: usize = 32;

/// Wrap `data` under `secret` with a fresh random salt: magic, salt in the
/// clear, then the XChaCha20-Poly1305 blob keyed by Argon2(secret, salt).
/// A salt is not secret — only unique.
pub fn encrypt_salted(data: &[u8], secret: &Secret) -> Result<Vec<u8>> {
    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    let body = encrypt_xchacha20poly1305_with_salt(data, secret, &salt)?;
    let mut out = Vec::with_capacity(SALTED_MAGIC.len() + SALT_LEN + body.len());
    out.extend_from_slice(SALTED_MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Unwrap either container, reporting whether the input was the legacy one so
/// a caller holding the password can rewrite it.
pub fn decrypt_salted_or_legacy(data: &[u8], secret: &Secret) -> Result<(Secret, bool)> {
    if data.starts_with(SALTED_MAGIC) {
        let body = SALTED_MAGIC.len() + SALT_LEN;
        if data.len() <= body {
            return Err("encrypted payload is truncated".into());
        }
        let salt = &data[SALTED_MAGIC.len()..body];
        Ok((decrypt_xchacha20poly1305_with_salt(&data[body..], secret, salt)?, false))
    } else {
        Ok((decrypt_xchacha20poly1305(data, secret)?, true))
    }
}

/// True if these bytes still use the password-derived salt.
pub fn is_legacy_encryption(data: &[u8]) -> bool {
    !data.starts_with(SALTED_MAGIC)
}

/// Encrypts with `XChaCha20Poly1305` using `key` directly as the cipher key — no
/// Argon2 stretching (FORK-PLAN P7.6, DECISIONS.md's "Note vault" entry). Argon2
/// exists in [`encrypt_xchacha20poly1305`] to slow down brute-forcing a *human*
/// password; it's pure waste (tens of ms per call, deliberately) when `key` is
/// already 32 bytes of CSPRNG output, as the note vault's key `K` is — every
/// per-note-file operation would otherwise pay a full Argon2 pass for no security
/// benefit. Reserved for high-entropy keys only; never call this with password
/// bytes directly.
pub fn encrypt_xchacha20poly1305_raw_key(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut buffer = data.to_vec();
    buffer.reserve(16);
    cipher.encrypt_in_place(&nonce, &[], &mut buffer)?;
    buffer.splice(0..0, nonce.iter().cloned());
    Ok(buffer)
}

/// Decrypts data produced by [`encrypt_xchacha20poly1305_raw_key`].
pub fn decrypt_xchacha20poly1305_raw_key(data: &[u8], key: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    if data.len() < 24 {
        return Err("ciphertext shorter than the nonce prefix".into());
    }
    let nonce = &data[0..24];
    let mut buffer = data[24..].to_vec();
    cipher.decrypt_in_place(nonce.into(), &[], &mut buffer)?;
    Ok(Zeroizing::new(buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wallet_argon2() {
        println!("testing argon2 hash");
        let password = b"user_password";
        let hash = argon2_sha256iv_hash(password, 32).unwrap();
        let hash_hex = hash.as_ref().to_hex();
        // println!("argon2hash: {:?}", hash_hex);
        assert_eq!(hash_hex, "a79b661f0defd1960a4770889e19da0ce2fde1e98ca040f84ab9b2519ca46234");
    }

    #[test]
    fn test_wallet_encrypt_decrypt() -> Result<()> {
        println!("testing encrypt/decrypt");

        let password = b"password";
        let original = b"hello world".to_vec();
        // println!("original: {}", original.to_hex());
        let password = Secret::new(password.to_vec());
        let encrypted = encrypt_xchacha20poly1305(&original, &password).unwrap();
        // println!("encrypted: {}", encrypted.to_hex());
        let decrypted = decrypt_xchacha20poly1305(&encrypted, &password).unwrap();
        // println!("decrypted: {}", decrypted.to_hex());
        assert_eq!(decrypted.as_ref(), original);

        Ok(())
    }
}

#[cfg(test)]
mod salted_container_tests {
    use super::*;

    /// Two wallets with the same password must not produce the same key
    /// material. The old derivation did exactly that: salt = sha256(password),
    /// so one precomputation attacked every wallet sharing a password.
    #[test]
    fn the_same_password_wraps_differently_every_time() {
        let secret = Secret::from("same-password-everywhere");
        let data = b"the private key data";
        let a = encrypt_salted(data, &secret).unwrap();
        let b = encrypt_salted(data, &secret).unwrap();
        assert_ne!(a, b, "two wraps under one password must differ");
        assert_ne!(a[4..36], b[4..36], "the salts must differ");
        assert!(a.starts_with(SALTED_MAGIC));
        assert!(!is_legacy_encryption(&a));

        for wrapped in [&a, &b] {
            let (plain, legacy) = decrypt_salted_or_legacy(wrapped, &secret).unwrap();
            assert_eq!(plain.as_ref(), data);
            assert!(!legacy);
        }
        assert!(decrypt_salted_or_legacy(&a, &Secret::from("wrong")).is_err());
    }

    /// Every wallet in the wild is the old format. It must still open, and it
    /// must say that it wants rewriting.
    #[test]
    fn legacy_payloads_still_open_and_ask_to_be_upgraded() {
        let secret = Secret::from("legacy-password");
        let data = b"the private key data";
        let old = encrypt_xchacha20poly1305(data, &secret).unwrap();
        assert!(is_legacy_encryption(&old));
        let (plain, legacy) = decrypt_salted_or_legacy(&old, &secret).unwrap();
        assert_eq!(plain.as_ref(), data);
        assert!(legacy, "a legacy payload must report itself as one");
    }

    /// The container is what `Encrypted` uses, so anything stored through it
    /// — the wallet payload, its transaction records, key data wrapped under a
    /// bip39 passphrase — is salted without knowing about it.
    #[test]
    fn the_encrypted_container_round_trips_through_the_new_format() {
        let secret = Secret::from("wallet-password");
        let value: Vec<u8> = b"payload".to_vec();
        let encrypted = Decrypted::new(value.clone()).encrypt(&secret, EncryptionKind::XChaCha20Poly1305).unwrap();
        assert!(!encrypted.is_legacy(), "a freshly encrypted payload must be salted");
        assert_eq!(encrypted.decrypt::<Vec<u8>>(&secret).unwrap().unwrap(), value);

        // ...and an Encrypted built from bytes written by older software still
        // decrypts through the same call, reporting itself as legacy.
        let old = Encrypted::new(
            EncryptionKind::XChaCha20Poly1305,
            encrypt_xchacha20poly1305(&borsh::to_vec(&value).unwrap(), &secret).unwrap(),
        );
        assert!(old.is_legacy());
        assert_eq!(old.decrypt::<Vec<u8>>(&secret).unwrap().unwrap(), value);
    }
}
