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

use crate::imports::*;
use kaspa_wallet_core::encryption::{decrypt_salted_or_legacy, encrypt_salted};
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
pub struct ArchiveEntry {
    /// Relative, `/`-separated.
    pub path: String,
    pub data: Vec<u8>,
}

/// Serialize entries, deflate, encrypt, and prepend the outer header.
pub fn pack(entries: &[ArchiveEntry], passphrase: &Secret) -> Result<Vec<u8>> {
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
    let compressed = encoder.finish().map_err(|e| Error::custom(format!("compressing the archive failed: {e}")))?;

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

    let mut body = Vec::new();
    flate2::read::DeflateDecoder::new(plain.as_ref())
        .read_to_end(&mut body)
        .map_err(|e| Error::custom(format!("the archive decrypted but would not decompress: {e}")))?;

    // Past here the bytes are authenticated, so a malformed read means a bug
    // on the writing side rather than a hostile file. Still bounds-checked:
    // "authenticated" is not "correct".
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
