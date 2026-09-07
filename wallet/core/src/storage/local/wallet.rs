//!
//! Wallet data storage wrapper.
//!

use crate::imports::*;
use crate::storage::Encryptable;
use crate::storage::TransactionRecord;
use crate::storage::local::Payload;
use crate::storage::local::Storage;
use crate::storage::{AccountMetadata, Decrypted, Encrypted, Hint, PrvKeyData, PrvKeyDataId};
use workflow_store::fs;

/// Client convenience metadata stored in the wallet file's PLAINTEXT tier
/// (alongside `title`/`user_hint`) — readable without the wallet password, so
/// the wallet picker and post-open connect flow work before decryption, and
/// the settings travel with the file when it is copied to another machine
/// (deliberate: a bearer-era wallet file should carry its own context).
/// Never put secrets here. `remember: false` means the wallet asked not to
/// have usage details recorded; writers must honor it by storing `None`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct ClientMetadata {
    pub network: Option<String>,
    pub server: Option<String>,
    pub last_opened: Option<u64>,
    pub remember: bool,
    /// Hidden from the `open` picker (still openable by name, still listed by
    /// `wallet list` marked as hidden). For wallets you keep but don't want in
    /// your face — archives, one-purpose stashes.
    #[serde(default)]
    pub hidden: bool,
    /// Auto-mint: turn arriving ledger balance into notes automatically once
    /// it passes `auto_mint_threshold_petals`. A preference only — it arms
    /// with the password typed at `open` and never persists any secret.
    #[serde(default)]
    pub auto_mint: bool,
    #[serde(default)]
    pub auto_mint_threshold_petals: u64,
    /// Auto-sweep: consolidate ledger coins on its own schedule, independent
    /// of auto-mint — a holder who wants to keep plain ledger balance (an
    /// exchange, say) still wants the dust kept under control.
    #[serde(default)]
    pub auto_sweep: bool,
    /// UTXO count above which a sweep is triggered (0 = use the default).
    #[serde(default)]
    pub auto_sweep_utxo_threshold: u64,
    /// Set once the user has touched the `auto` command. Until then the
    /// automation defaults are ON — housekeeping the ledger is not a thing a
    /// person should have to discover, and an untouched wallet must not be
    /// mistaken for one that opted out.
    #[serde(default)]
    pub auto_configured: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WalletStorage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_hint: Option<Hint>,
    pub encryption_kind: EncryptionKind,
    pub payload: Encrypted,
    pub metadata: Vec<AccountMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transactions: Option<Encryptable<HashMap<AccountId, Vec<TransactionRecord>>>>,
    /// v1 field — `None` for files written by v0 software (see version-gated
    /// read below; v0 files stay readable, and are upgraded to v1 on next save).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_metadata: Option<ClientMetadata>,
}

impl WalletStorage {
    pub const STORAGE_MAGIC: u32 = 0x5753414b;
    pub const STORAGE_VERSION: u32 = 1;

    pub fn try_new(
        title: Option<String>,
        user_hint: Option<Hint>,
        secret: &Secret,
        encryption_kind: EncryptionKind,
        payload: Payload,
        metadata: Vec<AccountMetadata>,
    ) -> Result<Self> {
        let payload = Decrypted::new(payload).encrypt(secret, encryption_kind)?;
        Ok(Self { title, encryption_kind, payload, metadata, user_hint, transactions: None, client_metadata: None })
    }

    pub fn payload(&self, secret: &Secret) -> Result<Decrypted<Payload>> {
        self.payload.decrypt::<Payload>(secret).map_err(|err| match err {
            Error::Chacha20poly1305(e) => Error::WalletDecrypt(e),
            _ => err,
        })
    }

    pub async fn try_load(store: &Storage) -> Result<WalletStorage> {
        if fs::exists(store.filename()).await? {
            let bytes = fs::read(store.filename()).await?;
            Ok(BorshDeserialize::try_from_slice(bytes.as_slice())?)
        } else {
            let name = store.filename().file_name().unwrap().to_str().unwrap();
            Err(Error::NoWalletInStorage(name.to_string()))
        }
    }

    pub async fn try_store(&self, store: &Storage) -> Result<()> {
        store.ensure_dir().await?;

        cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                let serialized = borsh::to_vec(self)?;
                fs::write(store.filename(), serialized.as_slice()).await?;
            } else {
                // Written to a temporary file and renamed over the original.
                // `File::create` truncates in place, so a failure part way
                // through — a full disk, a crash — left a half-written wallet
                // where the keys used to be. Rename is atomic on every
                // filesystem this runs on, so the old file stands until the
                // new one is complete on disk.
                let target = store.filename();
                let tmp = target.with_extension("wallet.tmp");
                {
                    let mut file = std::fs::File::create(&tmp)?;
                    BorshSerialize::serialize(self, &mut file)?;
                    // fsync before the rename: a rename that lands ahead of
                    // the data is how a crash produces an empty wallet.
                    file.sync_all()?;
                }
                std::fs::rename(&tmp, target)?;
            }
        }
        Ok(())
    }

    /// Obtain [`PrvKeyData`] using [`PrvKeyDataId`]
    pub async fn try_get_prv_key_data(&self, secret: &Secret, prv_key_data_id: &PrvKeyDataId) -> Result<Option<PrvKeyData>> {
        let payload = self.payload.decrypt::<Payload>(secret)?;
        let idx = payload.as_ref().prv_key_data.iter().position(|keydata| &keydata.id == prv_key_data_id);
        let keydata = idx.map(|idx| payload.as_ref().prv_key_data.get(idx).unwrap().clone());
        Ok(keydata)
    }

    pub fn replace_metadata(&mut self, metadata: Vec<AccountMetadata>) {
        self.metadata = metadata;
    }
}

impl BorshSerialize for WalletStorage {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        StorageHeader::new(Self::STORAGE_MAGIC, Self::STORAGE_VERSION).serialize(writer)?;
        BorshSerialize::serialize(&self.title, writer)?;
        BorshSerialize::serialize(&self.user_hint, writer)?;
        BorshSerialize::serialize(&self.encryption_kind, writer)?;
        BorshSerialize::serialize(&self.payload, writer)?;
        BorshSerialize::serialize(&self.metadata, writer)?;
        BorshSerialize::serialize(&self.transactions, writer)?;
        // v1 tail — readers gate on the header version.
        BorshSerialize::serialize(&self.client_metadata, writer)?;

        Ok(())
    }
}

impl BorshDeserialize for WalletStorage {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> IoResult<Self> {
        let StorageHeader { magic, version, .. } = StorageHeader::deserialize_reader(reader)?;

        if magic != Self::STORAGE_MAGIC {
            return Err(IoError::new(
                IoErrorKind::InvalidData,
                format!("This does not seem to be a kaspa wallet data file. Unknown file signature '0x{:x}'.", magic),
            ));
        }

        if version > Self::STORAGE_VERSION {
            return Err(IoError::new(
                IoErrorKind::InvalidData,
                format!(
                    "This wallet data was generated using a new version of the software. Please upgrade your software environment. Expected at most version '{}', encountered version '{}'",
                    Self::STORAGE_VERSION,
                    version
                ),
            ));
        }

        let title = BorshDeserialize::deserialize_reader(reader)?;
        let user_hint = BorshDeserialize::deserialize_reader(reader)?;
        let encryption_kind = BorshDeserialize::deserialize_reader(reader)?;
        let payload = BorshDeserialize::deserialize_reader(reader)?;
        let metadata = BorshDeserialize::deserialize_reader(reader)?;
        let transactions = BorshDeserialize::deserialize_reader(reader)?;
        // Version-gated: v0 files end here. Reading the field unconditionally
        // would error on every wallet created before v1 (positional Borsh, no
        // length prefix) — the exact backward-compat trap this fork's docs
        // warn about for this file format.
        // Tolerant tail read: a v0 file has no tail at all, and a client
        // metadata struct that gained fields in a later build would otherwise
        // make the whole wallet unreadable. Convenience data is never worth
        // failing an open over — fall back to None.
        let client_metadata = if version >= 1 { BorshDeserialize::deserialize_reader(reader).unwrap_or(None) } else { None };

        Ok(Self { title, user_hint, encryption_kind, payload, metadata, transactions, client_metadata })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::*;

    #[test]
    fn test_storage_wallet_storage() -> Result<()> {
        let storable_in = WalletStorage::try_new(
            Some("title".to_string()),
            Some(Hint::new("hint".to_string())),
            &Secret::from("secret"),
            EncryptionKind::XChaCha20Poly1305,
            Payload::new(vec![], vec![], vec![]),
            vec![],
        )?;
        let guard = StorageGuard::new(&storable_in);
        let _storable_out = guard.validate()?;

        Ok(())
    }
}
