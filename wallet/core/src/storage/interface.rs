//!
//! Wallet storage subsystem traits.
//!

use crate::imports::*;
use async_trait::async_trait;
use downcast::{AnySync, downcast_sync};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::notepool::DenominationTag;

#[derive(Debug, Clone)]
pub struct WalletExportOptions {
    pub include_transactions: bool,
}

#[wasm_bindgen(typescript_custom_section)]
const TS_WALLET_DESCRIPTOR: &'static str = r#"
/**
 * Wallet storage information.
 * 
 * @category Wallet API
 */
export interface IWalletDescriptor {
    title?: string;
    filename: string;
}
"#;

/// @category Wallet API
#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[wasm_bindgen(inspectable)]
pub struct WalletDescriptor {
    #[wasm_bindgen(getter_with_clone)]
    pub title: Option<String>,
    #[wasm_bindgen(getter_with_clone)]
    pub filename: String,
}

impl WalletDescriptor {
    pub fn new(title: Option<String>, filename: String) -> Self {
        Self { title, filename }
    }
}

#[wasm_bindgen(typescript_custom_section)]
const TS_STORAGE_DESCRIPTOR: &'static str = r#"
/**
 * Wallet storage information.
 */
export interface IStorageDescriptor {
    kind: string;
    data: string;
}
"#;

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "kind", content = "data")]
pub enum StorageDescriptor {
    Resident,
    Internal(String),
    Other(String),
}

impl std::fmt::Display for StorageDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageDescriptor::Resident => write!(f, "memory(resident)"),
            StorageDescriptor::Internal(path) => write!(f, "{path}"),
            StorageDescriptor::Other(other) => write!(f, "{other}"),
        }
    }
}

pub type StorageStream<T> = Pin<Box<dyn Stream<Item = Result<T>> + Send>>;

#[async_trait]
pub trait PrvKeyDataStore: Send + Sync {
    async fn is_empty(&self) -> Result<bool>;
    async fn iter(&self) -> Result<StorageStream<Arc<PrvKeyDataInfo>>>;
    async fn load_key_info(&self, id: &PrvKeyDataId) -> Result<Option<Arc<PrvKeyDataInfo>>>;
    async fn load_key_data(&self, wallet_secret: &Secret, id: &PrvKeyDataId) -> Result<Option<PrvKeyData>>;
    async fn store(&self, wallet_secret: &Secret, data: PrvKeyData) -> Result<()>;
    async fn remove(&self, wallet_secret: &Secret, id: &PrvKeyDataId) -> Result<()>;
}

#[async_trait]
pub trait NoteKeyStore: Send + Sync {
    async fn is_empty(&self) -> Result<bool>;
    async fn iter(&self) -> Result<StorageStream<Arc<NoteKeyInfo>>>;
    async fn load_info(&self, sn: &Hash) -> Result<Option<Arc<NoteKeyInfo>>>;
    async fn load_key(&self, wallet_secret: &Secret, sn: &Hash) -> Result<Option<NoteKeyEntry>>;
    /// Store a row with caller-supplied provenance (used for locally-generated `Cold`
    /// keys, and rotation-derived rows that inherit an existing key's provenance).
    async fn store(&self, wallet_secret: &Secret, entry: NoteKeyEntry) -> Result<()>;
    async fn remove(&self, wallet_secret: &Secret, sn: &Hash) -> Result<()>;
    /// A refund key for a note paid away under a lock (P8.0g), kept `Offered`
    /// with the DAA score its lock lapses at.
    async fn store_offered(&self, wallet_secret: &Secret, entry: NoteKeyEntry, lock_until: u64) -> Result<()>;
    /// Every offered note and the score its lock lapses at.
    async fn offered_notes(&self) -> Result<Vec<(Arc<NoteKeyInfo>, u64)>>;
    /// The standing receiving keys (P8.0g), public halves and labels.
    async fn share_keys(&self) -> Result<Vec<crate::storage::notekeys::ShareKeyInfo>>;
    async fn share_secret(&self, wallet_secret: &Secret, index: u32) -> Result<[u8; 32]>;
    async fn add_share_key(&self, wallet_secret: &Secret, label: &str) -> Result<crate::storage::notekeys::ShareKeyInfo>;
    /// Import a key that crossed a wallet boundary (bearer handover, cross-device
    /// export, backup restore) — always recorded `Hot` regardless of the imported
    /// key's prior state (POOL-SPEC.md P5.6's same-key-in-two-wallets hazard).
    async fn import_bearer_key(&self, wallet_secret: &Secret, sn: Hash, sk: [u8; 32], d: DenominationTag) -> Result<()>;
    /// Serials written within the last `within_secs` seconds.
    ///
    /// A note is stored the moment its creating transaction is SUBMITTED — the
    /// serial comes from the transaction id, so the wallet knows it before the
    /// chain has accepted it. A very young note the pool does not have yet is
    /// therefore in flight, not lost, and reconciliation must not call it a
    /// phantom.
    async fn recently_written(&self, within_secs: u64) -> Result<Vec<Hash>>;

    /// Flip a row's `status` — plaintext-only, never needs the wallet secret.
    async fn mark_status(&self, sn: &Hash, status: NoteStatus) -> Result<()>;

    /// Record that a synced node reported this serial missing. Returns the new
    /// consecutive-strike count.
    async fn record_missing(&self, sn: &Hash) -> Result<u32>;

    /// The serial turned up after all — reset its strikes.
    async fn clear_missing(&self, sn: &Hash) -> Result<()>;
    /// Apply a live `NotesChanged` notification (PLAN P6.9). See the trait-level
    /// doc on [`crate::storage::notekeys::NotesChangedApplyResult`] for the split
    /// between what always applies (status) and what needs `wallet_secret` (new rows).
    async fn apply_notes_changed(
        &self,
        wallet_secret: Option<&Secret>,
        notification: &kaspa_rpc_core::message::NotesChangedNotification,
    ) -> Result<crate::storage::notekeys::NotesChangedApplyResult>;

    // ~~~ payment-request keys (PLAN P7.3, sign-to-fresh-pk receive flow) ~~~

    /// Persist a freshly generated payment-request key. Returns the plaintext info
    /// (pk derived from the key). Must be called BEFORE the request's QR is shown
    /// anywhere — see `PaymentRequestKey`'s doc comment on crash safety.
    async fn store_payment_request(&self, wallet_secret: &Secret, key: PaymentRequestKey) -> Result<PaymentRequestInfo>;
    /// All outstanding (not yet claimed/removed) payment requests, plaintext half only.
    async fn payment_requests(&self) -> Result<Vec<PaymentRequestInfo>>;
    /// Does this password open the vault? A wallet that keeps notes only
    /// (PLAN P8.0b) has no account key to check a password against, and
    /// the vault key is wrapped under the same password.
    async fn verify_secret(&self, wallet_secret: &Secret) -> Result<()>;
    /// The vault's 24 recovery words, for whoever wants them on paper. The
    /// key is the words' entropy, so they can be produced at any time.
    async fn recovery_words(&self, wallet_secret: &Secret) -> Result<String>;
    async fn load_payment_request_key(&self, wallet_secret: &Secret, pk: &[u8; 32]) -> Result<Option<PaymentRequestKey>>;
    async fn remove_payment_request(&self, wallet_secret: &Secret, pk: &[u8; 32]) -> Result<()>;

    // ~~~ vault ceremony (PLAN P7.6) ~~~
    //
    // Backends that store notes some other way (there are none today, but the
    // trait stays storage-agnostic on principle) simply don't support these —
    // default to `NotImplemented` rather than forcing every implementor to
    // define vault-specific semantics.

    /// Whether the vault has already been through its 24-word creation ceremony.
    /// `store()`/`import_bearer_key()` auto-provision one silently on first use
    /// if this is false when they're called (see `LocalStoreInner::ensure_note_vault`)
    /// — `vault_create` lets a caller run the ceremony explicitly and properly
    /// beforehand instead, so the words are actually shown to the user.
    async fn vault_exists(&self) -> Result<bool> {
        Err(Error::NotImplemented)
    }
    /// Run the 24-word creation ceremony now. Returns the words — the caller MUST
    /// display/record them; they are never retrievable again. Errs if a vault
    /// already exists.
    async fn vault_create(&self, _wallet_secret: &Secret) -> Result<String> {
        Err(Error::NotImplemented)
    }
    /// Recover `K` from its 24-word encoding, re-wrapping it under `wallet_secret`
    /// for daily use afterward (PLAN P7.6 restore flow: "24 words + the
    /// files" — the files themselves are a separate, out-of-band copy step).
    async fn vault_restore_from_words(&self, _words: &str, _wallet_secret: &Secret) -> Result<()> {
        Err(Error::NotImplemented)
    }
    /// Whether `words` decode to the same `K` this vault is already wrapping under
    /// `wallet_secret` — lets a caller distinguish "this is a safe idempotent
    /// re-run of a restore that got partway through" from "this is an unrelated
    /// existing vault, refuse" (P7.8 finding: a restore that copies the files and
    /// recovers K but then hits a rotation-batch failure leaves a vault in place
    /// that `vault_exists()` alone can't tell apart from someone else's).
    async fn vault_words_match(&self, _words: &str, _wallet_secret: &Secret) -> Result<bool> {
        Err(Error::NotImplemented)
    }
    /// The vault's on-disk folder, for standalone copy-out (`note vault backup`)
    /// — the CLI does the actual file copy natively; this just says where from.
    async fn vault_folder(&self) -> Result<std::path::PathBuf> {
        Err(Error::NotImplemented)
    }
}

#[async_trait]
pub trait AccountStore: Send + Sync {
    async fn is_empty(&self) -> Result<bool>;
    async fn iter(
        &self,
        prv_key_data_id_filter: Option<PrvKeyDataId>,
    ) -> Result<StorageStream<(Arc<AccountStorage>, Option<Arc<AccountMetadata>>)>>;
    async fn len(&self, prv_key_data_id_filter: Option<PrvKeyDataId>) -> Result<usize>;
    async fn load_single(&self, ids: &AccountId) -> Result<Option<(Arc<AccountStorage>, Option<Arc<AccountMetadata>>)>>;
    async fn load_multiple(&self, ids: &[AccountId]) -> Result<Vec<(Arc<AccountStorage>, Option<Arc<AccountMetadata>>)>>;
    async fn store_single(&self, account: &AccountStorage, metadata: Option<&AccountMetadata>) -> Result<()>;
    async fn store_multiple(&self, data: Vec<(AccountStorage, Option<AccountMetadata>)>) -> Result<()>;
    async fn remove(&self, id: &[&AccountId]) -> Result<()>;
    async fn update_metadata(&self, metadata: Vec<AccountMetadata>) -> Result<()>;
}

#[async_trait]
pub trait AddressBookStore: Send + Sync {
    async fn is_empty(&self) -> Result<bool> {
        Err(Error::NotImplemented)
    }
    async fn iter(&self) -> Result<StorageStream<Arc<AddressBookEntry>>> {
        Err(Error::NotImplemented)
    }
    async fn search(&self, _search: &str) -> Result<Vec<Arc<AddressBookEntry>>> {
        Err(Error::NotImplemented)
    }
}

pub struct TransactionRangeResult {
    pub transactions: Vec<Arc<TransactionRecord>>,
    pub total: u64,
}

#[async_trait]
pub trait TransactionRecordStore: Send + Sync {
    async fn transaction_id_iter(&self, binding: &Binding, network_id: &NetworkId) -> Result<StorageStream<Arc<TransactionId>>>;
    async fn transaction_data_iter(&self, binding: &Binding, network_id: &NetworkId) -> Result<StorageStream<Arc<TransactionRecord>>>;
    async fn load_range(
        &self,
        binding: &Binding,
        network_id: &NetworkId,
        filter: Option<Vec<TransactionKind>>,
        range: std::ops::Range<usize>,
    ) -> Result<TransactionRangeResult>;

    async fn load_single(&self, binding: &Binding, network_id: &NetworkId, id: &TransactionId) -> Result<Arc<TransactionRecord>>;
    async fn load_multiple(
        &self,
        binding: &Binding,
        network_id: &NetworkId,
        ids: &[TransactionId],
    ) -> Result<Vec<Arc<TransactionRecord>>>;

    async fn store(&self, transaction_records: &[&TransactionRecord]) -> Result<()>;
    async fn remove(&self, binding: &Binding, network_id: &NetworkId, ids: &[&TransactionId]) -> Result<()>;

    async fn store_transaction_note(
        &self,
        binding: &Binding,
        network_id: &NetworkId,
        id: TransactionId,
        note: Option<String>,
    ) -> Result<()>;
    async fn store_transaction_metadata(
        &self,
        binding: &Binding,
        network_id: &NetworkId,
        id: TransactionId,
        metadata: Option<String>,
    ) -> Result<()>;
}

#[derive(Debug)]
pub struct CreateArgs {
    pub title: Option<String>,
    pub filename: Option<String>,
    pub encryption_kind: EncryptionKind,
    pub user_hint: Option<Hint>,
    pub overwrite_wallet: bool,
}

impl CreateArgs {
    pub fn new(
        title: Option<String>,
        filename: Option<String>,
        encryption_kind: EncryptionKind,
        user_hint: Option<Hint>,
        overwrite_wallet: bool,
    ) -> Self {
        Self { title, filename, encryption_kind, user_hint, overwrite_wallet }
    }
}

#[derive(Debug)]
pub struct OpenArgs {
    pub filename: Option<String>,
}

impl OpenArgs {
    pub fn new(filename: Option<String>) -> Self {
        Self { filename }
    }
}

#[async_trait]
pub trait Interface: Send + Sync + AnySync {
    /// enumerate all wallets available in the storage
    async fn wallet_list(&self) -> Result<Vec<WalletDescriptor>>;

    /// read a wallet's plaintext client metadata (network/server/last-opened)
    /// without opening it — no password required. `Ok(None)` when the wallet
    /// predates the field or the backend doesn't support it.
    async fn client_metadata(&self, _filename: &str) -> Result<Option<crate::storage::local::wallet::ClientMetadata>> {
        Ok(None)
    }

    /// rewrite a wallet's plaintext client metadata in place. No password
    /// required (the encrypted payload passes through untouched). No-op on
    /// backends that don't support it.
    async fn set_client_metadata(
        &self,
        _filename: &str,
        _metadata: Option<crate::storage::local::wallet::ClientMetadata>,
    ) -> Result<()> {
        Ok(())
    }

    /// The authenticator enrolled on the currently open wallet, if any.
    /// `Ok(None)` when none is enrolled or the backend has no support.
    fn otp(&self) -> Result<Option<crate::storage::Otp>> {
        Ok(None)
    }

    /// Enrol, re-configure, or remove the authenticator, and write it. Needs
    /// the wallet password because the secret lives in the encrypted payload
    /// and the whole payload is re-sealed to store it.
    async fn set_otp(&self, _wallet_secret: &Secret, _otp: Option<crate::storage::Otp>) -> Result<()> {
        Err(Error::custom("this storage backend cannot hold an authenticator"))
    }

    /// redirect where wallet files live (the `folder` setting). Applies to
    /// wallet/vault/transaction files only — the settings file stays at the
    /// default location so the redirect itself has a fixed home. Must be
    /// called before a wallet is opened; no-op on backends without folders.
    fn set_storage_folder(&self, _folder: &str) -> Result<()> {
        Ok(())
    }

    /// check if a wallet is currently open
    fn is_open(&self) -> bool;

    /// return storage information string (file location)
    fn location(&self) -> Result<StorageDescriptor>;

    /// returns the name of the currently open wallet or none
    fn descriptor(&self) -> Option<WalletDescriptor>;

    /// encryption used by the currently open wallet
    fn encryption_kind(&self) -> Result<EncryptionKind>;

    /// rename the currently open wallet (title or the filename)
    async fn rename(&self, wallet_secret: &Secret, title: Option<&str>, filename: Option<&str>) -> Result<()>;

    /// Rename a CLOSED wallet on disk: the `<name>.wallet` file and the
    /// `<name>.notes` / `<name>.transactions` folders that belong to it move
    /// together. The wallet must not be open — its in-memory handles hold the
    /// old paths.
    async fn rename_storage(&self, from: &str, to: &str) -> Result<()>;

    /// change the secret of the currently open wallet
    async fn change_secret(&self, old_wallet_secret: &Secret, new_wallet_secret: &Secret) -> Result<()>;

    /// checks if the wallet storage is present
    async fn exists(&self, name: Option<&str>) -> Result<bool>;

    /// initialize wallet storage
    async fn create(&self, wallet_secret: &Secret, args: CreateArgs) -> Result<WalletDescriptor>;

    /// establish an open state (load wallet data cache, connect to the database etc.)
    async fn open(&self, wallet_secret: &Secret, args: OpenArgs) -> Result<()>;

    /// suspend commit operations until flush() is called
    async fn batch(&self) -> Result<()>;

    /// flush resumes commit operations previously suspended by `suspend()`
    async fn flush(&self, wallet_secret: &Secret) -> Result<()>;

    /// commit any changes changes to storage
    async fn commit(&self, wallet_secret: &Secret) -> Result<()>;

    /// stop the storage subsystem
    async fn close(&self) -> Result<()>;

    /// export the wallet data
    async fn wallet_export(&self, wallet_secret: &Secret, options: WalletExportOptions) -> Result<Vec<u8>>;

    /// import the wallet data
    async fn wallet_import(&self, wallet_secret: &Secret, serialized_wallet_storage: &[u8]) -> Result<WalletDescriptor>;

    // ~~~

    // phishing hint (user-created text string identifying authenticity of the wallet)
    async fn get_user_hint(&self) -> Result<Option<Hint>>;
    async fn set_user_hint(&self, hint: Option<Hint>) -> Result<()>;

    // ~~~
    fn as_prv_key_data_store(&self) -> Result<Arc<dyn PrvKeyDataStore>>;
    fn as_account_store(&self) -> Result<Arc<dyn AccountStore>>;
    fn as_address_book_store(&self) -> Result<Arc<dyn AddressBookStore>>;
    fn as_transaction_record_store(&self) -> Result<Arc<dyn TransactionRecordStore>>;
    fn as_note_key_store(&self) -> Result<Arc<dyn NoteKeyStore>>;
}

downcast_sync!(dyn Interface);
