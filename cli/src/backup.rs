//! One encrypted file holding everything a wallet needs to come back.
//!
//! WHY A SEPARATE PASSPHRASE
//!
//! The wallet's own files are already encrypted: `<name>.wallet` under the
//! wallet password, and every note file under the vault key `K`, which is
//! itself under the wallet password. So why wrap them again?
//!
//! Because `manifest.tsv` is plaintext. It lists every note's serial, public
//! key, denomination and status — that is your balance, in the clear, in a
//! file you were about to hand to a chat server. The keys stay safe without
//! this layer; your holdings do not. The archive passphrase closes that.
//!
//! It also means a leaked archive is two independent secrets away from
//! spendable money rather than one, which is the property that makes it
//! reasonable to keep a copy somewhere you do not control.
//!
//! FORMAT
//!
//! ```text
//! [ "MGB1" 4 ][ version u8 = 1 ]
//! [ "MGS2" | salt 32 | nonce 24 | XChaCha20-Poly1305( deflate(body) ‖ tag ) ]
//! ```
//!
//! and `body` is
//!
//! ```text
//! [ entry_count u32 LE ]
//! entry_count × [ path_len u16 LE | path utf8 | data_len u32 LE | data ]
//! ```
//!
//! Nothing outside the AEAD but the five header bytes: not the wallet name,
//! not the file count, not the size of any one file. The inner container is
//! the same `MGS2` the note vault and the paper/phone exports use, so there is
//! one encryption format in this project rather than four, and the Mini App's
//! existing decoder is most of a reader for this.
//!
//! Paths are relative and always `/`-separated, so an archive written on one
//! platform restores on another.
//!
//! Since 2026-10-06 a backup covers every wallet in the folder, each sealed
//! to a key pair derived from its own 24 words — see "Sealed to a wallet's
//! own words" below. The passphrase form above is what a backup from before
//! then is, and `unpack` still reads it.

use crate::imports::*;
use kaspa_bip32::secp256k1;
use kaspa_wallet_core::encryption::{
    decrypt_salted_or_legacy, decrypt_xchacha20poly1305_raw_key, encrypt_salted, encrypt_xchacha20poly1305_raw_key,
};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Outer marker. Read before anything else so a wrong file is rejected by name
/// rather than by a decryption failure the user would read as a bad password.
const ARCHIVE_MAGIC: &[u8; 4] = b"MGB1";
const ARCHIVE_VERSION: u8 = 1;

/// Refuse to load an archive claiming an implausible size before allocating for
/// it. A real one is a few megabytes; this is four orders of magnitude of room.
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;

/// A file on its way into or out of an archive.
#[derive(Clone)]
pub struct ArchiveEntry {
    /// Relative, `/`-separated.
    pub path: String,
    pub data: Vec<u8>,
}

/// Serialize entries and deflate them: the plaintext every sealed form of
/// an archive carries, whether it is behind a passphrase or a public key.
fn pack_body(entries: &[ArchiveEntry]) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    body.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in entries {
        let path = entry.path.as_bytes();
        if path.len() > u16::MAX as usize {
            return Err(Error::custom(format!("path too long to archive: {}", entry.path)));
        }
        body.extend_from_slice(&(path.len() as u16).to_le_bytes());
        body.extend_from_slice(path);
        body.extend_from_slice(&(entry.data.len() as u32).to_le_bytes());
        body.extend_from_slice(&entry.data);
    }

    // Compression before encryption, which is the only order that does
    // anything: ciphertext has no structure left to compress.
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&body).map_err(|e| Error::custom(format!("compressing the archive failed: {e}")))?;
    encoder.finish().map_err(|e| Error::custom(format!("compressing the archive failed: {e}")))
}

/// Inflate and parse a body. Past the AEAD the bytes are authenticated, so a
/// malformed read means a bug on the writing side rather than a hostile file.
/// Still bounds-checked: "authenticated" is not "correct".
fn unpack_body(compressed: &[u8]) -> Result<Vec<ArchiveEntry>> {
    let mut body = Vec::new();
    flate2::read::DeflateDecoder::new(compressed)
        .read_to_end(&mut body)
        .map_err(|e| Error::custom(format!("the archive decrypted but would not decompress: {e}")))?;

    let mut cursor = 0usize;
    let mut take = |n: usize| -> Result<&[u8]> {
        let end = cursor.checked_add(n).ok_or_else(|| Error::custom("the archive is malformed"))?;
        if end > body.len() {
            return Err(Error::custom("the archive is truncated"));
        }
        let slice_start = cursor;
        cursor = end;
        Ok(&body[slice_start..end])
    };

    let count = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
    let mut entries = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let path_len = u16::from_le_bytes(take(2)?.try_into().unwrap()) as usize;
        let path = String::from_utf8(take(path_len)?.to_vec()).map_err(|_| Error::custom("the archive holds a non-UTF-8 path"))?;
        let data_len = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let data = take(data_len)?.to_vec();
        entries.push(ArchiveEntry { path, data });
    }
    Ok(entries)
}

/// Serialize entries, deflate, encrypt under a passphrase, and prepend the
/// outer header. The original single-wallet form; `seal_to` is what the
/// automatic backups use now.
pub fn pack(entries: &[ArchiveEntry], passphrase: &Secret) -> Result<Vec<u8>> {
    let compressed = pack_body(entries)?;
    let sealed = encrypt_salted(&compressed, passphrase)?;
    let mut out = Vec::with_capacity(ARCHIVE_MAGIC.len() + 1 + sealed.len());
    out.extend_from_slice(ARCHIVE_MAGIC);
    out.push(ARCHIVE_VERSION);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// The reverse. A wrong passphrase surfaces as an authentication failure from
/// the AEAD, never as garbage entries — that is what the tag is for.
pub fn unpack(archive: &[u8], passphrase: &Secret) -> Result<Vec<ArchiveEntry>> {
    if !archive.starts_with(ARCHIVE_MAGIC) {
        return Err(Error::custom("that file is not a Marigold backup archive"));
    }
    let version = *archive.get(ARCHIVE_MAGIC.len()).ok_or_else(|| Error::custom("the archive is truncated"))?;
    if version != ARCHIVE_VERSION {
        return Err(Error::custom(format!(
            "this archive is version {version}; this wallet reads version {ARCHIVE_VERSION}. Use a newer wallet to restore it."
        )));
    }

    let sealed = &archive[ARCHIVE_MAGIC.len() + 1..];
    let (plain, _legacy) =
        decrypt_salted_or_legacy(sealed, passphrase).map_err(|_| Error::custom("wrong passphrase, or the archive is damaged"))?;
    unpack_body(plain.as_ref())
}

// ---------------------------------------------------------------------------
// Sealed to a wallet's own words
//
// Every wallet's 24 words also name a key pair: the private half is a hash of
// the words, the public half sits in the clear in `<name>.wallet/backup.pub`.
// A backup of that wallet is sealed to the public half with a fresh ephemeral
// key (ECDH on secp256k1, then XChaCha20-Poly1305), so the machine can back
// up every wallet it holds — open or not — and only that wallet's words open
// its part. The private half is never written anywhere.
//
// ```text
// blob:   [ "MGBR" 4 ][ v u8 = 1 ][ ephemeral pubkey 33 ][ recipient fingerprint 8 ]
//         [ nonce 24 | XChaCha20-Poly1305( deflate(body) ‖ tag ) ]
// bundle: [ "MGBB" 4 ][ v u8 = 1 ][ count u32 LE ]
//         count × [ name_len u16 LE | wallet name utf8 | blob_len u32 LE | blob ]
// ```
//
// The bundle names its wallets in the clear on purpose: at restore time the
// person has to know which words to type, and "the file holds reserve,
// savings and marigold" is nothing a wallet's directory listing does not
// already say. Balances and keys stay inside the AEAD as before.
// ---------------------------------------------------------------------------

const SEALED_MAGIC: &[u8; 4] = b"MGBR";
const SEALED_VERSION: u8 = 1;
const BUNDLE_MAGIC: &[u8; 4] = b"MGBB";
const BUNDLE_VERSION: u8 = 1;
const RECIPIENT_DOMAIN: &[u8] = b"marigold-backup-recipient-v1";
const SEAL_DOMAIN: &[u8] = b"marigold-backup-seal-v1";

/// The file in a wallet directory holding the public half, as 66 hex characters.
pub const RECIPIENT_FILE: &str = "backup.pub";

/// A wallet's name and its sealed part of a bundle.
pub struct BundleItem {
    pub name: String,
    pub blob: Vec<u8>,
}

fn normalise_words(words: &str) -> String {
    words.split_whitespace().map(|w| w.to_lowercase()).collect::<Vec<_>>().join(" ")
}

/// The private half of a wallet's backup key pair, from its words. Derived
/// whenever it is needed and dropped right after; never stored.
pub fn recipient_secret(words: &str) -> secp256k1::SecretKey {
    use sha2::{Digest, Sha256};
    let normalised = normalise_words(words);
    // A hash lands outside the curve order with probability ~2^-128; the
    // counter keeps the function total without making the common case differ.
    for counter in 0u8..=255 {
        let mut h = Sha256::new();
        h.update(RECIPIENT_DOMAIN);
        h.update(normalised.as_bytes());
        if counter > 0 {
            h.update([counter]);
        }
        if let Ok(sk) = secp256k1::SecretKey::from_slice(&h.finalize()) {
            return sk;
        }
    }
    unreachable!("256 consecutive hashes outside the secp256k1 order")
}

/// The public half, which is what a backup is sealed to.
pub fn recipient_public(words: &str) -> secp256k1::PublicKey {
    recipient_secret(words).public_key(secp256k1::SECP256K1)
}

/// Eight bytes that name a recipient without being it: written into every
/// blob so a restore can say "those words belong to a different wallet"
/// instead of "decryption failed".
pub fn recipient_fingerprint(pk: &secp256k1::PublicKey) -> [u8; 8] {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(pk.serialize());
    digest[..8].try_into().unwrap()
}

/// Where a wallet directory keeps its public half.
pub fn recipient_pub_path(wallet_dir: &Path) -> PathBuf {
    wallet_dir.join(RECIPIENT_FILE)
}

/// Record a wallet's public half next to its files. Idempotent; the words
/// decide the key, so writing it twice writes the same bytes.
pub fn write_recipient(wallet_dir: &Path, words: &str) -> Result<secp256k1::PublicKey> {
    let pk = recipient_public(words);
    let hex = faster_hex::hex_string(&pk.serialize());
    std::fs::create_dir_all(wallet_dir).map_err(|e| Error::custom(format!("cannot create {}: {e}", wallet_dir.display())))?;
    let path = recipient_pub_path(wallet_dir);
    std::fs::write(&path, format!("{hex}\n")).map_err(|e| Error::custom(format!("cannot write {}: {e}", path.display())))?;
    Ok(pk)
}

/// The public half a wallet directory holds, if it has one yet.
pub fn read_recipient(wallet_dir: &Path) -> Option<secp256k1::PublicKey> {
    let text = std::fs::read_to_string(recipient_pub_path(wallet_dir)).ok()?;
    let mut bytes = [0u8; 33];
    faster_hex::hex_decode(text.trim().as_bytes(), &mut bytes).ok()?;
    secp256k1::PublicKey::from_slice(&bytes).ok()
}

/// Covers a wallet that is not open with its words, typed once: the vault key
/// is the words' entropy, so decrypting any one of its note files proves the
/// words are this wallet's; then the public half is written. A wallet without
/// a note cannot be checked and is refused — opening it once covers it.
pub fn cover_with_words(wallet_dir: &Path, words: &str) -> Result<()> {
    let mnemonic = kaspa_bip32::Mnemonic::new(normalise_words(words), kaspa_bip32::Language::English)
        .map_err(|_| Error::custom("those are not 24 wallet words"))?;
    let k: [u8; 32] =
        mnemonic.entropy().as_slice().try_into().map_err(|_| Error::custom("a wallet has 24 words; these encode a shorter key"))?;
    let mut probe = Vec::new();
    collect_tree(&wallet_dir.join("notes"), "", &mut probe).ok();
    let Some(note) = probe.iter().find(|e| e.path.ends_with(".note")) else {
        return Err(Error::custom(
            "this wallet holds no note yet, so the words cannot be checked against it — open it once with its password instead",
        ));
    };
    if decrypt_xchacha20poly1305_raw_key(&note.data, &k).is_err() {
        return Err(Error::custom("those are not this wallet's words"));
    }
    write_recipient(wallet_dir, words)?;
    Ok(())
}

/// The symmetric key one ephemeral/recipient pair agrees on.
fn seal_key(shared: &secp256k1::ecdh::SharedSecret, ephemeral: &secp256k1::PublicKey) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(SEAL_DOMAIN);
    h.update(shared.secret_bytes());
    h.update(ephemeral.serialize());
    h.finalize().into()
}

/// Seal entries so that only the words behind `recipient` open them.
pub fn seal_to(recipient: &secp256k1::PublicKey, entries: &[ArchiveEntry]) -> Result<Vec<u8>> {
    use rand::RngCore;
    let ephemeral_secret = loop {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        if let Ok(sk) = secp256k1::SecretKey::from_slice(&bytes) {
            break sk;
        }
    };
    let ephemeral = ephemeral_secret.public_key(secp256k1::SECP256K1);
    let shared = secp256k1::ecdh::SharedSecret::new(recipient, &ephemeral_secret);
    let key = seal_key(&shared, &ephemeral);
    let compressed = pack_body(entries)?;
    let ciphertext = encrypt_xchacha20poly1305_raw_key(&compressed, &key)?;

    let mut out = Vec::with_capacity(4 + 1 + 33 + 8 + ciphertext.len());
    out.extend_from_slice(SEALED_MAGIC);
    out.push(SEALED_VERSION);
    out.extend_from_slice(&ephemeral.serialize());
    out.extend_from_slice(&recipient_fingerprint(recipient));
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Whether bytes are a sealed blob.
pub fn is_sealed(blob: &[u8]) -> bool {
    blob.starts_with(SEALED_MAGIC)
}

fn sealed_parts(blob: &[u8]) -> Result<(secp256k1::PublicKey, [u8; 8], &[u8])> {
    if !is_sealed(blob) {
        return Err(Error::custom("that is not a sealed Marigold backup"));
    }
    let version = *blob.get(4).ok_or_else(|| Error::custom("the backup is truncated"))?;
    if version != SEALED_VERSION {
        return Err(Error::custom(format!(
            "this backup is version {version}; this wallet reads version {SEALED_VERSION}. Use a newer wallet to restore it."
        )));
    }
    if blob.len() < 4 + 1 + 33 + 8 {
        return Err(Error::custom("the backup is truncated"));
    }
    let ephemeral = secp256k1::PublicKey::from_slice(&blob[5..38]).map_err(|_| Error::custom("the backup header is damaged"))?;
    let fingerprint: [u8; 8] = blob[38..46].try_into().unwrap();
    Ok((ephemeral, fingerprint, &blob[46..]))
}

/// The fingerprint of the wallet a blob was sealed to.
pub fn sealed_recipient(blob: &[u8]) -> Result<[u8; 8]> {
    sealed_parts(blob).map(|(_, fp, _)| fp)
}

/// Whether these words are the ones a blob was sealed to. Cheap, and it
/// lets a restore tell "wrong wallet's words" from "damaged file".
pub fn words_fit(words: &str, blob: &[u8]) -> bool {
    match sealed_recipient(blob) {
        Ok(fp) => fp == recipient_fingerprint(&recipient_public(words)),
        Err(_) => false,
    }
}

/// Open a sealed blob with the wallet's words.
pub fn unseal_with(words: &str, blob: &[u8]) -> Result<Vec<ArchiveEntry>> {
    let (ephemeral, fingerprint, ciphertext) = sealed_parts(blob)?;
    let secret = recipient_secret(words);
    let public = secret.public_key(secp256k1::SECP256K1);
    if recipient_fingerprint(&public) != fingerprint {
        return Err(Error::custom("those are not this wallet's 24 words"));
    }
    let shared = secp256k1::ecdh::SharedSecret::new(&ephemeral, &secret);
    let key = seal_key(&shared, &ephemeral);
    let plain = decrypt_xchacha20poly1305_raw_key(ciphertext, &key).map_err(|_| Error::custom("the backup is damaged"))?;
    unpack_body(plain.as_ref())
}

/// Several wallets' sealed parts in one file.
pub fn pack_bundle(items: &[BundleItem]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(BUNDLE_MAGIC);
    out.push(BUNDLE_VERSION);
    out.extend_from_slice(&(items.len() as u32).to_le_bytes());
    for item in items {
        let name = item.name.as_bytes();
        if name.len() > u16::MAX as usize {
            return Err(Error::custom(format!("wallet name too long to bundle: {}", item.name)));
        }
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&(item.blob.len() as u32).to_le_bytes());
        out.extend_from_slice(&item.blob);
    }
    Ok(out)
}

/// Whether bytes are a bundle.
pub fn is_bundle(bytes: &[u8]) -> bool {
    bytes.starts_with(BUNDLE_MAGIC)
}

/// Split a bundle into its wallets. Nothing here is encrypted, so this is
/// bounds-checking and nothing more; each blob still has to be unsealed.
pub fn parse_bundle(bytes: &[u8]) -> Result<Vec<BundleItem>> {
    if !is_bundle(bytes) {
        return Err(Error::custom("that file is not a Marigold backup bundle"));
    }
    let version = *bytes.get(4).ok_or_else(|| Error::custom("the bundle is truncated"))?;
    if version != BUNDLE_VERSION {
        return Err(Error::custom(format!(
            "this bundle is version {version}; this wallet reads version {BUNDLE_VERSION}. Use a newer wallet to restore it."
        )));
    }
    let mut cursor = 5usize;
    let mut take = |n: usize| -> Result<&[u8]> {
        let end = cursor.checked_add(n).ok_or_else(|| Error::custom("the bundle is malformed"))?;
        if end > bytes.len() {
            return Err(Error::custom("the bundle is truncated"));
        }
        let start = cursor;
        cursor = end;
        Ok(&bytes[start..end])
    };
    let count = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
    let mut items = Vec::with_capacity(count.min(64));
    for _ in 0..count {
        let name_len = u16::from_le_bytes(take(2)?.try_into().unwrap()) as usize;
        let name = String::from_utf8(take(name_len)?.to_vec()).map_err(|_| Error::custom("the bundle holds a non-UTF-8 name"))?;
        let blob_len = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let blob = take(blob_len)?.to_vec();
        items.push(BundleItem { name, blob });
    }
    Ok(items)
}

/// Collect a directory tree into archive entries under `prefix`.
///
/// Symlinks are read through rather than recorded: an archive is meant to
/// survive the machine it came from, and a link into a directory that no longer
/// exists restores as nothing at all.
pub fn collect_tree(root: &Path, prefix: &str, out: &mut Vec<ArchiveEntry>) -> Result<()> {
    let mut dirs = vec![(root.to_path_buf(), prefix.to_string())];
    while let Some((dir, rel)) = dirs.pop() {
        let listing = std::fs::read_dir(&dir).map_err(|e| Error::custom(format!("cannot read {}: {e}", dir.display())))?;
        for entry in listing {
            let entry = entry.map_err(|e| Error::custom(format!("cannot read {}: {e}", dir.display())))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            // metadata() follows symlinks; file_type() would not.
            let meta = entry.path().metadata().map_err(|e| Error::custom(format!("cannot read {}: {e}", entry.path().display())))?;
            if meta.is_dir() {
                dirs.push((entry.path(), child_rel));
            } else {
                let data =
                    std::fs::read(entry.path()).map_err(|e| Error::custom(format!("cannot read {}: {e}", entry.path().display())))?;
                out.push(ArchiveEntry { path: child_rel, data });
            }
        }
    }
    Ok(())
}

/// Reject a path that would write outside the destination. The archive is
/// authenticated, so this cannot trigger on a file we wrote — it is here
/// because "the tag verified" says the bytes are unmodified, not that they were
/// benign when they were written.
fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let mut path = base.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains('\\') || part.contains(':') {
            return Err(Error::custom(format!("the archive holds an unsafe path: {rel}")));
        }
        path.push(part);
    }
    Ok(path)
}

/// Write entries under `base`. Never overwrites: the caller has already decided
/// what may exist, and silently replacing a live wallet with an old copy of it
/// is the one mistake this whole command exists to prevent.
pub fn extract(entries: &[ArchiveEntry], base: &Path) -> Result<usize> {
    for entry in entries {
        let target = safe_join(base, &entry.path)?;
        if target.exists() {
            return Err(Error::custom(format!("{} already exists — nothing was written", target.display())));
        }
    }
    for entry in entries {
        let target = safe_join(base, &entry.path)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::custom(format!("cannot create {}: {e}", parent.display())))?;
        }
        write_owner_only(&target, &entry.data).map_err(|e| Error::custom(format!("cannot write {}: {e}", target.display())))?;
    }
    Ok(entries.len())
}

/// Read an archive off disk, refusing an implausible one before allocating.
pub fn read_file(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).map_err(|e| Error::custom(format!("cannot read {}: {e}", path.display())))?;
    if meta.len() > MAX_ARCHIVE_BYTES {
        return Err(Error::custom(format!("{} is {} bytes — too large to be a wallet backup", path.display(), meta.len())));
    }
    std::fs::read(path).map_err(|e| Error::custom(format!("cannot read {}: {e}", path.display())))
}

/// Rewrite the wallet name that leads every path in an archive.
///
/// Every file a wallet owns is either `<name>.wallet` or lives under
/// `<name>.notes/`, so restoring under a different name is a prefix swap and
/// nothing more. Returns an error rather than a partial rename if some path
/// does not fit that shape, because a half-renamed wallet would not open.
pub fn rename_entries(entries: Vec<ArchiveEntry>, from: &str, to: &str) -> Result<Vec<ArchiveEntry>> {
    entries
        .into_iter()
        .map(|entry| {
            // Every path is `<from>.wallet/...`; the keys file inside also
            // carries the name, so both have to move together or the restored
            // directory would not contain a keys file matching its own name.
            let Some(rest) = entry.path.strip_prefix(&format!("{from}.wallet/")) else {
                return Err(Error::custom(format!("'{}' does not belong to the wallet '{from}' — cannot rename", entry.path)));
            };
            let rest = if rest == format!("{from}.keys") { format!("{to}.keys") } else { rest.to_string() };
            let renamed = format!("{to}.wallet/{rest}");
            Ok(ArchiveEntry { path: renamed, data: entry.data })
        })
        .collect()
}

/// Human-readable size, because "3,481,209 bytes" tells nobody whether it will
/// fit in a chat message.
pub fn human_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} bytes")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{:.1} MB", b / (KB * KB))
    }
}

/// Write a restored file for its owner alone: a restored vault is the
/// wallet, and the archive's own modes are not kept.
pub(crate) fn write_owner_only(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

/// The marker a restore leaves in the vault folder: rotate every note on the
/// first open (POOL-SPEC.md P5.6).
pub const ROTATE_ON_OPEN: &str = "rotate-on-open";

/// The wallet an archive holds: `<name>.wallet/<name>.keys` — one level down,
/// and the directory name is the authority since the keys file is named
/// after it.
pub fn wallet_name_in(entries: &[ArchiveEntry]) -> Result<String> {
    let mut found = entries.iter().filter_map(|e| {
        let (dir, file) = e.path.split_once('/')?;
        let name = dir.strip_suffix(".wallet")?;
        (file == format!("{name}.keys")).then(|| name.to_string())
    });
    let name = found.next().ok_or_else(|| Error::custom("that archive holds no wallet file"))?;
    if found.next().is_some() {
        return Err(Error::custom("that archive holds more than one wallet file"));
    }
    Ok(name)
}

/// What a restore put in place.
pub struct Restored {
    pub written: usize,
    /// The name the backup carried.
    pub original: String,
    /// The name the files have now.
    pub name: String,
}

/// Puts decrypted backup entries in place under `folder` — under `new_name`
/// if given — and marks the wallet for key rotation on its first open. The
/// shared tail of every restore; the terminal adds its questions around it.
pub fn install_restored(entries: Vec<ArchiveEntry>, folder: &Path, new_name: Option<String>) -> Result<Restored> {
    let original = wallet_name_in(&entries)?;
    let name = new_name.unwrap_or_else(|| original.clone());
    if name.to_lowercase() == "wallet" {
        return Err(Error::custom("a wallet cannot be named 'wallet'"));
    }
    let entries = if name == original { entries } else { rename_entries(entries, &original, &name)? };
    let written = extract(&entries, folder)?;
    // A backup is a copy of the keys, and any other copy of it can spend the
    // same notes. The first open of the restored wallet rotates every note to
    // fresh keys (POOL-SPEC.md P5.6), which needs the wallet open and a node:
    // this marker asks for it (threat pass, 2026-09-20).
    let marker = folder.join(kaspa_wallet_core::storage::local::wallet_dir_name(&name)).join("notes").join(ROTATE_ON_OPEN);
    if let Some(dir) = marker.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(&marker, b"restored from a backup; rotate every note on the first open\n").ok();
    Ok(Restored { written, original, name })
}

/// The key an automatic Telegram backup is sealed with: the wallet's 24
/// words, normalised, through a domain-separated hash. Nothing has to be
/// asked or remembered beyond the words, which open everything anyway.
pub fn key_from_words(words: &str) -> Secret {
    use sha2::{Digest, Sha256};
    let normalised = words.split_whitespace().map(|w| w.to_lowercase()).collect::<Vec<_>>().join(" ");
    let mut h = Sha256::new();
    h.update(b"marigold-telegram-backup-v1");
    h.update(normalised.as_bytes());
    Secret::from(h.finalize().to_vec())
}

/// Whether an answer at the restore prompt is 24 words rather than a passphrase.
pub fn looks_like_words(answer: &str) -> bool {
    let words: Vec<&str> = answer.split_whitespace().collect();
    words.len() == 24 && words.iter().all(|w| w.chars().all(|c| c.is_ascii_alphabetic()))
}

/// The key for a restore prompt's answer: the words if that is what was typed, the passphrase otherwise.
pub fn key_from_answer(answer: &str) -> Secret {
    if looks_like_words(answer) { key_from_words(answer) } else { Secret::from(answer.as_bytes().to_vec()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, data: &[u8]) -> ArchiveEntry {
        ArchiveEntry { path: path.to_string(), data: data.to_vec() }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marigold-backup-test-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trip_preserves_paths_and_bytes() {
        let pass = Secret::from(b"correct horse battery".to_vec());
        let entries = vec![
            entry("marigold.wallet/marigold.keys", &[0u8, 1, 2, 250]),
            entry("marigold.wallet/notes/vault.key", b"MGV2 and then some"),
            entry("marigold.wallet/notes/active/beef.note", &vec![7u8; 4096]),
            entry("marigold.wallet/notes/manifest.tsv", b"sn\tpk\td\tactive\n"),
        ];
        let packed = pack(&entries, &pass).unwrap();
        let out = unpack(&packed, &pass).unwrap();
        assert_eq!(out.len(), entries.len());
        for (a, b) in entries.iter().zip(out.iter()) {
            assert_eq!(a.path, b.path);
            assert_eq!(a.data, b.data);
        }
    }

    #[test]
    fn wrong_passphrase_is_rejected_not_garbled() {
        let packed = pack(&[entry("marigold.wallet/marigold.keys", b"secret")], &Secret::from(b"right one".to_vec())).unwrap();
        assert!(unpack(&packed, &Secret::from(b"wrong one".to_vec())).is_err());
    }

    #[test]
    fn a_flipped_bit_is_caught() {
        let pass = Secret::from(b"correct horse battery".to_vec());
        let mut packed = pack(&[entry("marigold.wallet/marigold.keys", b"secret")], &pass).unwrap();
        let last = packed.len() - 1;
        packed[last] ^= 0x01;
        assert!(unpack(&packed, &pass).is_err());
    }

    #[test]
    fn a_foreign_file_is_named_as_such() {
        // Deliberately not unwrap_err: that needs Debug on ArchiveEntry, and a
        // Debug impl on a struct holding decrypted key bytes is one stray
        // "{:?}" away from printing them.
        let err = match unpack(b"PK\x03\x04 not ours at all", &Secret::from(b"x".to_vec())) {
            Ok(_) => panic!("a zip file was accepted as a backup"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("not a Marigold backup"), "{err}");
    }

    #[test]
    fn nothing_leaks_outside_the_ciphertext() {
        let pass = Secret::from(b"correct horse battery".to_vec());
        let packed = pack(&[entry("distinctive-wallet-name.wallet/distinctive-wallet-name.keys", b"payload")], &pass).unwrap();
        assert!(!packed.windows(24).any(|w| w == b"distinctive-wallet-name."), "the wallet name is readable in the file");
        assert_eq!(&packed[..4], ARCHIVE_MAGIC);
        assert_eq!(packed[4], ARCHIVE_VERSION);
    }

    #[test]
    fn collect_then_extract_reproduces_the_tree() {
        let root = scratch("tree");
        let vault = root.join("src").join("marigold.wallet").join("notes");
        std::fs::create_dir_all(vault.join("active")).unwrap();
        std::fs::write(vault.join("vault.key"), b"key bytes").unwrap();
        std::fs::write(vault.join("manifest.tsv"), b"a\tb\n").unwrap();
        std::fs::write(vault.join("active").join("one.note"), b"note one").unwrap();

        let mut entries = vec![];
        collect_tree(&vault, "marigold.wallet/notes", &mut entries).unwrap();
        assert_eq!(entries.len(), 3);

        let dest = root.join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        extract(&entries, &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("marigold.wallet/notes/active/one.note")).unwrap(), b"note one");
        assert_eq!(std::fs::read(dest.join("marigold.wallet/notes/vault.key")).unwrap(), b"key bytes");

        // Twice must not silently merge into a live wallet.
        assert!(extract(&entries, &dest).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn extract_writes_nothing_when_one_file_is_in_the_way() {
        let root = scratch("collision");
        std::fs::create_dir_all(root.join("marigold.wallet")).unwrap();
        std::fs::write(root.join("marigold.wallet/marigold.keys"), b"the live one").unwrap();
        let entries = vec![entry("marigold.wallet/notes/vault.key", b"new"), entry("marigold.wallet/marigold.keys", b"old backup")];
        assert!(extract(&entries, &root).is_err());
        // The collision was the second entry; the first must not have landed.
        assert!(!root.join("marigold.wallet/notes").exists());
        assert_eq!(std::fs::read(root.join("marigold.wallet/marigold.keys")).unwrap(), b"the live one");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn traversal_is_refused() {
        let root = scratch("traversal");
        assert!(extract(&[entry("../escaped.wallet", b"x")], &root).is_err());
        assert!(extract(&[entry("marigold.wallet/../../escaped", b"x")], &root).is_err());
        assert!(!root.parent().unwrap().join("escaped.wallet").exists());
        std::fs::remove_dir_all(&root).ok();
    }

    const WORDS: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
    const OTHER: &str = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote";

    #[test]
    fn the_key_pair_follows_the_words_not_their_spelling() {
        let a = recipient_public(WORDS);
        let b = recipient_public(&WORDS.to_uppercase().replace(' ', "   "));
        assert_eq!(a, b);
        assert_ne!(a, recipient_public(OTHER));
    }

    #[test]
    fn sealed_opens_with_its_words_only() {
        let entries = vec![entry("reserve.wallet/reserve.keys", b"keys"), entry("reserve.wallet/notes/manifest.tsv", b"sn\tpk\n")];
        let blob = seal_to(&recipient_public(WORDS), &entries).unwrap();
        assert!(is_sealed(&blob));
        assert!(words_fit(WORDS, &blob));
        assert!(!words_fit(OTHER, &blob));
        let out = unseal_with(WORDS, &blob).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].data, b"sn\tpk\n");
        let err = match unseal_with(OTHER, &blob) {
            Ok(_) => panic!("another wallet's words opened the blob"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("not this wallet's"), "{err}");
        assert!(!blob.windows(5).any(|w| w == b"sn\tpk"), "the manifest is readable in the blob");
    }

    #[test]
    fn two_seals_of_the_same_bytes_differ() {
        let entries = vec![entry("reserve.wallet/reserve.keys", b"keys")];
        let pk = recipient_public(WORDS);
        assert_ne!(seal_to(&pk, &entries).unwrap(), seal_to(&pk, &entries).unwrap());
    }

    #[test]
    fn a_flipped_bit_in_a_sealed_blob_is_caught() {
        let mut blob = seal_to(&recipient_public(WORDS), &[entry("reserve.wallet/reserve.keys", b"keys")]).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        assert!(unseal_with(WORDS, &blob).is_err());
    }

    #[test]
    fn a_bundle_round_trips_and_names_its_wallets() {
        let one = seal_to(&recipient_public(WORDS), &[entry("marigold.wallet/marigold.keys", b"one")]).unwrap();
        let two = seal_to(&recipient_public(OTHER), &[entry("reserve.wallet/reserve.keys", b"two")]).unwrap();
        let bundle = pack_bundle(&[
            BundleItem { name: "marigold".into(), blob: one.clone() },
            BundleItem { name: "reserve".into(), blob: two.clone() },
        ])
        .unwrap();
        assert!(is_bundle(&bundle));
        assert!(!is_sealed(&bundle));
        let items = parse_bundle(&bundle).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "marigold");
        assert_eq!(items[0].blob, one);
        assert_eq!(items[1].name, "reserve");
        assert_eq!(unseal_with(OTHER, &items[1].blob).unwrap()[0].data, b"two");
        assert!(unseal_with(WORDS, &items[1].blob).is_err());
        assert!(parse_bundle(&bundle[..bundle.len() - 3]).is_err());
    }

    #[test]
    fn covering_a_closed_wallet_checks_the_words_against_a_note() {
        let root = scratch("cover");
        let dir = root.join("reserve.wallet");
        std::fs::create_dir_all(dir.join("notes").join("active")).unwrap();
        // No note yet: nothing to check against, so refused.
        assert!(cover_with_words(&dir, WORDS).is_err());
        let k: [u8; 32] =
            kaspa_bip32::Mnemonic::new(WORDS, kaspa_bip32::Language::English).unwrap().entropy().as_slice().try_into().unwrap();
        let note = encrypt_xchacha20poly1305_raw_key(b"a note", &k).unwrap();
        std::fs::write(dir.join("notes").join("active").join("100_ab.note"), note).unwrap();
        assert!(cover_with_words(&dir, OTHER).is_err());
        assert!(read_recipient(&dir).is_none());
        cover_with_words(&dir, WORDS).unwrap();
        assert_eq!(read_recipient(&dir), Some(recipient_public(WORDS)));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_public_half_is_written_and_read_back() {
        let root = scratch("recipient");
        let dir = root.join("reserve.wallet");
        assert!(read_recipient(&dir).is_none());
        let pk = write_recipient(&dir, WORDS).unwrap();
        assert_eq!(read_recipient(&dir), Some(pk));
        assert_eq!(std::fs::read_to_string(recipient_pub_path(&dir)).unwrap().trim().len(), 66);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn renaming_moves_every_path_or_none() {
        let entries = vec![entry("marigold.wallet/marigold.keys", b"w"), entry("marigold.wallet/notes/vault.key", b"k")];
        let renamed = rename_entries(entries, "marigold", "spare").unwrap();
        assert_eq!(renamed[0].path, "spare.wallet/spare.keys");
        assert_eq!(renamed[1].path, "spare.wallet/notes/vault.key");

        let stray = vec![entry("marigold.wallet/marigold.keys", b"w"), entry("something-else.dat", b"?")];
        assert!(rename_entries(stray, "marigold", "spare").is_err());
    }
}
