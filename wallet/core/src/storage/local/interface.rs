//!
//! Storage interface implementation capable of storing wallet data
//! in a local file system, web browser localstorage and chrome
//! extension storage.
//!

use crate::imports::*;
use crate::storage::interface::{
    AddressBookStore, CreateArgs, NoteKeyStore, OpenArgs, StorageDescriptor, StorageStream, WalletDescriptor, WalletExportOptions,
};
use crate::storage::local::Payload;
use crate::storage::local::Storage;
use crate::storage::local::cache::*;
use crate::storage::local::notevault::NoteVault;
use crate::storage::local::streams::*;
use crate::storage::local::transaction::*;
use crate::storage::local::wallet::WalletStorage;
use crate::storage::notekeys::NotesChangedApplyResult;
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::notepool::DenominationTag;
use kaspa_rpc_core::message::NotesChangedNotification;
use rand::RngCore;
use slugify_rs::slugify;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use workflow_core::runtime::is_web;
use workflow_store::fs;

pub fn make_filename(title: &Option<String>, filename: &Option<String>) -> String {
    if let Some(filename) = filename {
        filename.to_string()
    } else if let Some(title) = title {
        slugify!(title)
    } else {
        super::default_wallet_file().to_string()
    }
}

#[derive(Clone)]
pub enum Store {
    Resident,
    Storage(Storage),
}

impl Store {
    fn filename(&self) -> Option<String> {
        match self {
            Store::Resident => None,
            Store::Storage(storage) => Some(storage.filename_as_string()),
        }
    }

    pub fn is_resident(&self) -> bool {
        matches!(self, Store::Resident)
    }
}

pub(crate) struct LocalStoreInner {
    pub cache: Arc<RwLock<Cache>>,
    pub store: RwLock<Arc<Store>>,
    pub transactions: Arc<dyn TransactionRecordStore>,
    /// Note key storage (FORK-PLAN P7.6): the file-per-note vault, replacing
    /// P7.1's single-blob `note_key_data`/`note_key_info` cache fields (which the
    /// `Cache`/`Payload` types still carry only for `PaymentRequestKey` storage —
    /// per DECISIONS.md, explicitly not migrated). Constructed the same way as
    /// `transactions` above (folder/filename convention), including for resident
    /// wallets — see [`fsio::TransactionStore`]'s own precedent for why a
    /// "resident" wallet still gets real file-backed storage for this data.
    pub notevault: Arc<NoteVault>,
    pub is_modified: AtomicBool,
}

impl LocalStoreInner {
    async fn try_create(wallet_secret: &Secret, folder: &str, args: CreateArgs, is_resident: bool) -> Result<Self> {
        let (store, wallet_title, filename) = if is_resident {
            (Store::Resident, Some("Resident Wallet".to_string()), "resident".to_string())
        } else {
            // log_info!("LocalStoreInner::try_create: folder: {}, args: {:?}, is_resident: {}", folder, args, is_resident);

            let title = args.title.clone();
            let filename = make_filename(&title, &args.filename);

            let storage = Storage::try_new_with_folder(folder, &format!("{filename}.wallet"))?;
            if storage.exists().await? && !args.overwrite_wallet {
                return Err(Error::WalletAlreadyExists);
            }
            (Store::Storage(storage), title, filename)
        };

        let payload = Payload::default();
        let cache =
            Arc::new(RwLock::new(Cache::from_payload(wallet_title, args.user_hint, payload, wallet_secret, args.encryption_kind)?));
        let is_modified = AtomicBool::new(false);
        let transactions: Arc<dyn TransactionRecordStore> = if !is_web() {
            Arc::new(fsio::TransactionStore::new(folder, &filename))
        } else {
            Arc::new(indexdb::TransactionStore::new(&filename))
        };
        let notevault = if is_resident {
            // A "resident" wallet has no fixed identity — every instance shares the
            // literal filename "resident" (above), which is harmless for `store()`
            // (a no-op for `Store::Resident`) and for `transactions` (content-addressed
            // by transaction ID, so concurrent resident wallets merely interleave
            // harmlessly in the same folder). It is NOT harmless for the vault: two
            // resident `Wallet`s in the same process — or two unrelated processes on
            // the same machine, since `folder` defaults to `~/.marigold` even for
            // resident wallets — would fight over one `vault.key` wrapped under
            // whichever wallet's secret created it last. Give every resident instance
            // its own random, ephemeral location instead (OS temp dir — matches
            // "resident" meaning "nothing outlives this process" far better than a
            // real, git-adjacent storage folder would).
            let mut suffix = [0u8; 8];
            rand::thread_rng().fill_bytes(&mut suffix);
            let resident_folder = std::env::temp_dir().join(format!("kaspa-wallet-resident-{}", suffix.as_slice().to_hex()));
            NoteVault::new(resident_folder, &filename)
        } else {
            NoteVault::new(folder, &filename)
        };
        // A fresh `try_create` may land on a folder/filename that hosted an older
        // same-named wallet (`overwrite_wallet: true`) — that wallet's `vault.key`
        // would be wrapped under a secret this new wallet knows nothing about, so
        // any leftover vault state here must not survive into the new wallet.
        notevault.reset().await?;
        let notevault = Arc::new(notevault);

        Ok(Self { cache, store: RwLock::new(Arc::new(store)), is_modified, transactions, notevault })
    }

    async fn try_load(wallet_secret: &Secret, folder: &str, args: OpenArgs) -> Result<Self> {
        let filename = make_filename(&None, &args.filename);
        let storage = Storage::try_new_with_folder(folder, &format!("{filename}.wallet"))?;

        let wallet = WalletStorage::try_load(&storage).await?;
        let cache = Arc::new(RwLock::new(Cache::from_wallet(wallet, wallet_secret)?));
        let is_modified = AtomicBool::new(false);

        let transactions: Arc<dyn TransactionRecordStore> = if !is_web() {
            Arc::new(fsio::TransactionStore::new(folder, &filename))
        } else {
            Arc::new(indexdb::TransactionStore::new(&filename))
        };
        let notevault = Arc::new(NoteVault::new(folder, &filename));

        Ok(Self { cache, store: RwLock::new(Arc::new(Store::Storage(storage))), is_modified, transactions, notevault })
    }

    async fn try_import(wallet_secret: &Secret, folder: &str, serialized_wallet_storage: &[u8]) -> Result<Self> {
        // (see `try_create` above for the note vault reset rationale)
        let wallet = WalletStorage::try_from_slice(serialized_wallet_storage)?;
        // Try to decrypt the wallet payload with the provided
        // secret. This will block import if the secret is
        // not correct.
        let _ = wallet.payload(wallet_secret)?;

        let filename = make_filename(&wallet.title, &None);
        let storage = Storage::try_new_with_folder(folder, &format!("{filename}.wallet"))?;
        if storage.exists_sync()? {
            return Err(Error::WalletAlreadyExists);
        }

        let cache = Arc::new(RwLock::new(Cache::from_wallet(wallet, wallet_secret)?));
        let is_modified = AtomicBool::new(false);

        let transactions: Arc<dyn TransactionRecordStore> = if !is_web() {
            Arc::new(fsio::TransactionStore::new(folder, &filename))
        } else {
            Arc::new(indexdb::TransactionStore::new(&filename))
        };
        let notevault = NoteVault::new(folder, &filename);
        notevault.reset().await?;
        let notevault = Arc::new(notevault);

        Ok(Self { cache, store: RwLock::new(Arc::new(Store::Storage(storage))), is_modified, transactions, notevault })
    }

    async fn try_export(&self, wallet_secret: &Secret, _options: WalletExportOptions) -> Result<Vec<u8>> {
        let wallet = self.cache.read().unwrap().to_wallet(None, wallet_secret)?;
        Ok(borsh::to_vec(&wallet)?)
    }

    fn storage(&self) -> Arc<Store> {
        self.store.read().unwrap().clone()
    }

    fn rename(&self, filename: &str) -> Result<()> {
        let store = (**self.store.read().unwrap()).clone();
        let filename = make_filename(&None, &Some(filename.to_string()));
        match store {
            Store::Resident => Err(Error::ResidentWallet),
            Store::Storage(mut storage) => {
                storage.rename_sync(filename.as_str())?;
                *self.store.write().unwrap() = Arc::new(Store::Storage(storage));
                Ok(())
            }
        }
    }

    async fn change_secret(&self, old_secret: &Secret, new_secret: &Secret) -> Result<()> {
        match &*self.storage() {
            Store::Resident => {
                let mut cache = self.cache.write().unwrap();
                let old_prv_key_data: Decrypted<PrvKeyDataMap> = cache.prv_key_data.decrypt(old_secret)?;
                let new_prv_key_data = Decrypted::new(old_prv_key_data.unwrap()).encrypt(new_secret, cache.encryption_kind)?;
                cache.prv_key_data.replace(new_prv_key_data);

                Ok(())
            }
            Store::Storage(storage) => {
                let wallet = {
                    let mut cache = self.cache.write().unwrap();
                    let old_prv_key_data: Decrypted<PrvKeyDataMap> = cache.prv_key_data.decrypt(old_secret)?;
                    let new_prv_key_data = Decrypted::new(old_prv_key_data.unwrap()).encrypt(new_secret, cache.encryption_kind)?;
                    cache.prv_key_data.replace(new_prv_key_data);

                    cache.to_wallet(None, new_secret)?
                };
                wallet.try_store(storage).await?;
                self.set_modified(false);
                Ok(())
            }
        }
    }

    pub async fn update_stored_metadata(&self) -> Result<()> {
        match &*self.storage() {
            Store::Resident => Ok(()),
            Store::Storage(storage) => {
                // take current metadata, load wallet, replace metadata, store wallet
                // this bypasses the cache payload and wallet encryption
                let metadata: Vec<AccountMetadata> = (&self.cache.read().unwrap().metadata).try_into()?;
                let mut wallet = WalletStorage::try_load(storage).await?;
                wallet.replace_metadata(metadata);
                wallet.try_store(storage).await?;
                Ok(())
            }
        }
    }

    // pub fn cache(&self) -> &Cache {
    //     &self.cache
    // }

    // pub fn cache_read(&self) -> RwLockReadGuard<Cache> {
    //     self.cache.read().unwrap()
    // }

    // pub fn cache_write(&self) -> RwLockWriteGuard<Cache> {
    //     self.cache.write().unwrap()
    // }

    pub async fn store(&self, wallet_secret: &Secret) -> Result<()> {
        match &*self.storage() {
            Store::Resident => Ok(()),
            Store::Storage(storage) => {
                let wallet = self.cache.read().unwrap().to_wallet(None, wallet_secret)?;
                wallet.try_store(storage).await?;
                self.set_modified(false);
                Ok(())
            }
        }
    }

    #[inline]
    pub fn set_modified(&self, modified: bool) {
        match &*self.storage() {
            Store::Resident => (),
            Store::Storage(_) => {
                self.is_modified.store(modified, Ordering::SeqCst);
            }
        }
    }

    #[inline]
    pub fn is_modified(&self) -> bool {
        match &*self.storage() {
            Store::Resident => false,
            Store::Storage(_) => self.is_modified.load(Ordering::SeqCst),
        }
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    fn descriptor(&self) -> WalletDescriptor {
        let filename = self
            .storage()
            .filename()
            .and_then(|f| PathBuf::from(f).file_stem().and_then(|f| f.to_str().map(String::from)))
            .unwrap_or_else(|| "resident".to_string());
        WalletDescriptor { title: self.cache.read().unwrap().wallet_title.clone(), filename }
    }

    fn location(&self) -> Result<StorageDescriptor> {
        let store = self.storage();
        match &*store {
            Store::Resident => Ok(StorageDescriptor::Resident),
            Store::Storage(storage) => Ok(StorageDescriptor::Internal(storage.filename_as_string())),
        }
    }

    /// Auto-provision the note vault (24-word `K` ceremony) the first time any
    /// note-storing call reaches it without one already existing on disk. This
    /// keeps every pre-P7.6 flow (mint/receive/import — none of which know about
    /// vault setup) working unmodified; a wallet that explicitly ran the vault
    /// creation command first (FORK-PLAN P7.6/P7.9's `note vault create`) never
    /// hits this path since `notevault.exists()` is already true by then. The
    /// words are only `log_warn!`'d here as a stopgap — this is not a substitute
    /// for the real interactive ceremony, which the CLI command surfaces properly.
    async fn ensure_note_vault(&self, wallet_secret: &Secret) -> Result<()> {
        if !self.notevault.exists().await? {
            let words = self.notevault.create(wallet_secret).await?;
            log_warn!(
                "note vault: no vault existed yet for this wallet — auto-created one. \
                 Run `note vault backup` soon to record these 24 recovery words properly; \
                 they will not be shown again automatically: {words}"
            );
        }
        Ok(())
    }
}

impl Drop for LocalStoreInner {
    fn drop(&mut self) {
        if self.is_modified() {
            panic!("LocalStoreInner::drop called while modified flag is true");
        }
    }
}

pub struct Location {
    pub folder: String,
}

impl Location {
    pub fn new(folder: &str) -> Self {
        Self { folder: folder.to_string() }
    }
}

impl Default for Location {
    fn default() -> Self {
        Self { folder: super::default_storage_folder().to_string() }
    }
}

#[derive(Clone)]
pub(crate) struct LocalStore {
    location: Arc<Mutex<Option<Arc<Location>>>>,
    inner: Arc<Mutex<Option<Arc<LocalStoreInner>>>>,
    is_resident: bool,
    batch: Arc<AtomicBool>,
}

impl LocalStore {
    pub fn try_new(is_resident: bool) -> Result<Self> {
        Ok(Self {
            location: Arc::new(Mutex::new(Some(Arc::new(Location::default())))),
            inner: Arc::new(Mutex::new(None)),
            is_resident,
            batch: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn inner(&self) -> Result<Arc<LocalStoreInner>> {
        self.inner.lock().unwrap().as_ref().cloned().ok_or(Error::WalletNotOpen)
    }

    fn location(&self) -> Option<Arc<Location>> {
        self.location.lock().unwrap().clone()
    }

    #[allow(dead_code)]
    async fn wallet_export_impl(&self, wallet_secret: &Secret, _options: WalletExportOptions) -> Result<Vec<u8>> {
        self.inner()?.try_export(wallet_secret, _options).await
    }

    async fn wallet_import_impl(&self, wallet_secret: &Secret, serialized_wallet_storage: &[u8]) -> Result<WalletDescriptor> {
        let location = self.location().expect("initialized wallet storage location");
        let inner = LocalStoreInner::try_import(wallet_secret, &location.folder, serialized_wallet_storage).await?;
        inner.store(wallet_secret).await?;
        let wallet_descriptor = inner.descriptor();
        Ok(wallet_descriptor)
    }
}

#[async_trait]
impl Interface for LocalStore {
    fn as_prv_key_data_store(&self) -> Result<Arc<dyn PrvKeyDataStore>> {
        Ok(self.inner()?)
    }

    fn as_account_store(&self) -> Result<Arc<dyn AccountStore>> {
        Ok(self.inner()?)
    }

    fn as_address_book_store(&self) -> Result<Arc<dyn AddressBookStore>> {
        Ok(self.inner()?)
    }

    fn as_transaction_record_store(&self) -> Result<Arc<dyn TransactionRecordStore>> {
        Ok(self.inner()?.transactions.clone())
    }

    fn as_note_key_store(&self) -> Result<Arc<dyn NoteKeyStore>> {
        Ok(self.inner()?)
    }

    fn descriptor(&self) -> Option<WalletDescriptor> {
        self.inner.lock().unwrap().as_ref().map(|inner| inner.descriptor())
    }

    fn encryption_kind(&self) -> Result<EncryptionKind> {
        Ok(self.inner()?.cache.read().unwrap().encryption_kind)
    }

    async fn rename(&self, wallet_secret: &Secret, title: Option<&str>, filename: Option<&str>) -> Result<()> {
        let inner = self.inner.lock().unwrap().clone().ok_or(Error::WalletNotOpen)?;
        if let Some(title) = title {
            inner.cache.write().unwrap().wallet_title = Some(title.to_string());
            self.commit(wallet_secret).await?;
        }

        if let Some(filename) = filename {
            inner.rename(filename)?;
        }
        Ok(())
    }

    /// change the secret of the currently open wallet
    async fn change_secret(&self, old_wallet_secret: &Secret, new_wallet_secret: &Secret) -> Result<()> {
        let inner = self.inner.lock().unwrap().clone().ok_or(Error::WalletNotOpen)?;
        inner.change_secret(old_wallet_secret, new_wallet_secret).await?;
        Ok(())
    }

    async fn exists(&self, name: Option<&str>) -> Result<bool> {
        let location = self.location.lock().unwrap().clone().unwrap();
        let store =
            Storage::try_new_with_folder(&location.folder, &format!("{}.wallet", name.unwrap_or(super::default_wallet_file())))?;
        store.exists().await
    }

    async fn create(&self, wallet_secret: &Secret, args: CreateArgs) -> Result<WalletDescriptor> {
        let location = self.location().expect("initialized wallet storage location");

        let inner = Arc::new(LocalStoreInner::try_create(wallet_secret, &location.folder, args, self.is_resident).await?);
        let descriptor = inner.descriptor();
        self.inner.lock().unwrap().replace(inner);

        Ok(descriptor)
    }

    async fn open(&self, wallet_secret: &Secret, args: OpenArgs) -> Result<()> {
        if let Some(inner) = self.inner.lock().unwrap().as_ref()
            && inner.is_modified()
        {
            panic!("LocalStore::open called while modified flag is true!");
        }

        let location = self.location.lock().unwrap().clone().unwrap();
        let inner = Arc::new(LocalStoreInner::try_load(wallet_secret, &location.folder, args).await?);
        self.inner.lock().unwrap().replace(inner);
        Ok(())
    }

    async fn wallet_list(&self) -> Result<Vec<WalletDescriptor>> {
        let location = self.location.lock().unwrap().clone().unwrap();

        let folder = fs::resolve_path(&location.folder)?;
        let files = fs::readdir(folder.clone(), false).await?;
        let wallets = files
            .iter()
            .filter_map(|de| {
                let file_name = de.file_name();
                file_name.ends_with(".wallet").then(|| file_name.trim_end_matches(".wallet").to_string())
            })
            .collect::<Vec<_>>();

        let mut descriptors = vec![];
        for filename in wallets.into_iter() {
            let path = folder.join(format!("{}.wallet", filename));
            // TODO - refactor on native to read directly from file (skip temporary buffer creation)
            let wallet_data = fs::read(&path).await;
            let title =
                wallet_data.ok().and_then(|data| WalletStorage::try_from_slice(data.as_slice()).ok()).and_then(|wallet| wallet.title);
            descriptors.push(WalletDescriptor { title, filename });
        }

        Ok(descriptors)
    }

    async fn client_metadata(&self, filename: &str) -> Result<Option<crate::storage::local::wallet::ClientMetadata>> {
        let location = self.location.lock().unwrap().clone().unwrap();
        let path = fs::resolve_path(&location.folder)?.join(format!("{filename}.wallet"));
        if !fs::exists(&path).await? {
            return Err(Error::NoWalletInStorage(filename.to_string()));
        }
        let data = fs::read(&path).await?;
        let wallet = WalletStorage::try_from_slice(data.as_slice())?;
        Ok(wallet.client_metadata)
    }

    async fn set_client_metadata(&self, filename: &str, metadata: Option<crate::storage::local::wallet::ClientMetadata>) -> Result<()> {
        let location = self.location.lock().unwrap().clone().unwrap();
        let path = fs::resolve_path(&location.folder)?.join(format!("{filename}.wallet"));
        if !fs::exists(&path).await? {
            return Err(Error::NoWalletInStorage(filename.to_string()));
        }
        // Plaintext-tier edit: the encrypted payload round-trips untouched, so
        // no password is needed — same access level as wallet_list's title read.
        let data = fs::read(&path).await?;
        let mut wallet = WalletStorage::try_from_slice(data.as_slice())?;
        wallet.client_metadata = metadata.clone();
        let serialized = borsh::to_vec(&wallet)?;
        fs::write(&path, serialized.as_slice()).await?;
        // If this wallet is currently open, its in-memory cache would clobber
        // the file on the next full save — keep it in step.
        if let Some(inner) = self.inner.lock().unwrap().clone() {
            if inner.descriptor().filename == filename {
                inner.cache.write().unwrap().client_metadata = metadata;
            }
        }
        Ok(())
    }

    fn set_storage_folder(&self, folder: &str) -> Result<()> {
        if self.inner.lock().unwrap().is_some() {
            return Err(Error::Custom("cannot change the storage folder while a wallet is open".to_string()));
        }
        *self.location.lock().unwrap() = Some(Arc::new(Location::new(folder)));
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.inner.lock().unwrap().is_some()
    }

    fn location(&self) -> Result<StorageDescriptor> {
        self.inner()?.location()
    }

    async fn batch(&self) -> Result<()> {
        self.batch.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn flush(&self, wallet_secret: &Secret) -> Result<()> {
        if !self.batch.load(Ordering::SeqCst) {
            panic!("flush() called while not in batch mode");
        }

        self.batch.store(false, Ordering::SeqCst);
        self.commit(wallet_secret).await?;
        Ok(())
    }

    async fn commit(&self, wallet_secret: &Secret) -> Result<()> {
        if !self.batch.load(Ordering::SeqCst) {
            self.inner()?.store(wallet_secret).await?;
        }
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        if self.inner()?.is_modified() {
            panic!("LocalStore::close called while modified flag is true");
        }

        if !self.is_open() {
            panic!("LocalStore::close called while wallet is not open");
        }

        let inner = self.inner.lock().unwrap().take().unwrap();
        inner.close().await?;

        Ok(())
    }

    async fn get_user_hint(&self) -> Result<Option<Hint>> {
        Ok(self.inner()?.cache.read().unwrap().user_hint.clone())
    }

    async fn set_user_hint(&self, user_hint: Option<Hint>) -> Result<()> {
        self.inner()?.cache.write().unwrap().user_hint = user_hint;
        Ok(())
    }

    async fn wallet_export(&self, wallet_secret: &Secret, options: WalletExportOptions) -> Result<Vec<u8>> {
        self.wallet_export_impl(wallet_secret, options).await
    }

    async fn wallet_import(&self, wallet_secret: &Secret, serialized_wallet_storage: &[u8]) -> Result<WalletDescriptor> {
        self.wallet_import_impl(wallet_secret, serialized_wallet_storage).await
    }
}

#[async_trait]
impl PrvKeyDataStore for LocalStoreInner {
    async fn is_empty(&self) -> Result<bool> {
        Ok(self.cache.read().unwrap().prv_key_data_info.is_empty())
    }

    async fn iter(&self) -> Result<StorageStream<Arc<PrvKeyDataInfo>>> {
        Ok(Box::pin(PrvKeyDataInfoStream::new(self.cache.clone())))
    }

    async fn load_key_info(&self, prv_key_data_id: &PrvKeyDataId) -> Result<Option<Arc<PrvKeyDataInfo>>> {
        Ok(self.cache.read().unwrap().prv_key_data_info.map.get(prv_key_data_id).cloned())
    }

    async fn load_key_data(&self, wallet_secret: &Secret, prv_key_data_id: &PrvKeyDataId) -> Result<Option<PrvKeyData>> {
        let prv_key_data_map: Decrypted<PrvKeyDataMap> = self.cache.read().unwrap().prv_key_data.decrypt(wallet_secret)?;
        Ok(prv_key_data_map.get(prv_key_data_id).cloned())
    }

    async fn store(&self, wallet_secret: &Secret, prv_key_data: PrvKeyData) -> Result<()> {
        let mut cache = self.cache.write().unwrap();
        let encryption_kind = cache.encryption_kind;
        let mut prv_key_data_map: Decrypted<PrvKeyDataMap> = cache.prv_key_data.decrypt(wallet_secret)?;
        let prv_key_data_info = Arc::new((&prv_key_data).into());
        cache.prv_key_data_info.insert(prv_key_data.id, prv_key_data_info)?;
        prv_key_data_map.insert(prv_key_data.id, prv_key_data);
        cache.prv_key_data.replace(prv_key_data_map.encrypt(wallet_secret, encryption_kind)?);
        self.set_modified(true);
        Ok(())
    }

    async fn remove(&self, wallet_secret: &Secret, prv_key_data_id: &PrvKeyDataId) -> Result<()> {
        let mut cache = self.cache.write().unwrap();
        let encryption_kind = cache.encryption_kind;
        let mut prv_key_data_map: Decrypted<PrvKeyDataMap> = cache.prv_key_data.decrypt(wallet_secret)?;
        prv_key_data_map.remove(prv_key_data_id);
        cache.prv_key_data.replace(prv_key_data_map.encrypt(wallet_secret, encryption_kind)?);
        self.set_modified(true);
        Ok(())
    }
}

#[async_trait]
impl NoteKeyStore for LocalStoreInner {
    async fn is_empty(&self) -> Result<bool> {
        self.notevault.is_empty().await
    }

    async fn iter(&self) -> Result<StorageStream<Arc<NoteKeyInfo>>> {
        self.notevault.iter().await
    }

    async fn load_info(&self, sn: &Hash) -> Result<Option<Arc<NoteKeyInfo>>> {
        self.notevault.load_info(sn).await
    }

    async fn load_key(&self, wallet_secret: &Secret, sn: &Hash) -> Result<Option<NoteKeyEntry>> {
        self.notevault.load_key(wallet_secret, sn).await
    }

    async fn store(&self, wallet_secret: &Secret, entry: NoteKeyEntry) -> Result<()> {
        self.ensure_note_vault(wallet_secret).await?;
        self.notevault.store(wallet_secret, entry).await?;
        self.set_modified(true);
        Ok(())
    }

    async fn remove(&self, wallet_secret: &Secret, sn: &Hash) -> Result<()> {
        self.notevault.remove(wallet_secret, sn).await?;
        self.set_modified(true);
        Ok(())
    }

    async fn import_bearer_key(&self, wallet_secret: &Secret, sn: Hash, sk: [u8; 32], d: DenominationTag) -> Result<()> {
        self.ensure_note_vault(wallet_secret).await?;
        self.notevault.import_bearer_key(wallet_secret, sn, sk, d).await?;
        self.set_modified(true);
        Ok(())
    }

    async fn mark_status(&self, sn: &Hash, status: NoteStatus) -> Result<()> {
        self.notevault.mark_status(sn, status).await?;
        self.set_modified(true);
        Ok(())
    }

    async fn apply_notes_changed(
        &self,
        wallet_secret: Option<&Secret>,
        notification: &NotesChangedNotification,
    ) -> Result<NotesChangedApplyResult> {
        let result = self.notevault.apply_notes_changed(wallet_secret, notification).await?;
        if !result.superseded.is_empty() || !result.added.is_empty() {
            self.set_modified(true);
        }
        Ok(result)
    }

    async fn vault_exists(&self) -> Result<bool> {
        self.notevault.exists().await
    }

    async fn vault_create(&self, wallet_secret: &Secret) -> Result<String> {
        self.notevault.create(wallet_secret).await
    }

    async fn vault_restore_from_words(&self, words: &str, wallet_secret: &Secret) -> Result<()> {
        self.notevault.restore_key_from_words(words, wallet_secret).await
    }

    async fn vault_words_match(&self, words: &str, wallet_secret: &Secret) -> Result<bool> {
        self.notevault.words_match_existing_key(words, wallet_secret).await
    }

    async fn vault_folder(&self) -> Result<std::path::PathBuf> {
        Ok(self.notevault.folder().to_path_buf())
    }

    async fn store_payment_request(&self, wallet_secret: &Secret, key: PaymentRequestKey) -> Result<PaymentRequestInfo> {
        let pk = key.derive_pk()?;
        let info = PaymentRequestInfo { pk, amount_petals: key.amount_petals };
        let mut cache = self.cache.write().unwrap();
        let encryption_kind = cache.encryption_kind;
        let mut map: Decrypted<PaymentRequestMap> = cache.payment_request_data.decrypt(wallet_secret)?;
        map.insert(pk, key);
        cache.payment_request_data.replace(map.encrypt(wallet_secret, encryption_kind)?);
        cache.payment_request_info.retain(|existing| existing.pk != pk);
        cache.payment_request_info.push(info.clone());
        self.set_modified(true);
        Ok(info)
    }

    async fn payment_requests(&self) -> Result<Vec<PaymentRequestInfo>> {
        Ok(self.cache.read().unwrap().payment_request_info.clone())
    }

    async fn load_payment_request_key(&self, wallet_secret: &Secret, pk: &[u8; 32]) -> Result<Option<PaymentRequestKey>> {
        let map: Decrypted<PaymentRequestMap> = self.cache.read().unwrap().payment_request_data.decrypt(wallet_secret)?;
        Ok(map.get(pk).cloned())
    }

    async fn remove_payment_request(&self, wallet_secret: &Secret, pk: &[u8; 32]) -> Result<()> {
        let mut cache = self.cache.write().unwrap();
        let encryption_kind = cache.encryption_kind;
        let mut map: Decrypted<PaymentRequestMap> = cache.payment_request_data.decrypt(wallet_secret)?;
        map.remove(pk);
        cache.payment_request_data.replace(map.encrypt(wallet_secret, encryption_kind)?);
        cache.payment_request_info.retain(|existing| existing.pk != *pk);
        self.set_modified(true);
        Ok(())
    }
}

#[async_trait]
impl AccountStore for LocalStoreInner {
    async fn is_empty(&self) -> Result<bool> {
        Ok(self.cache.read().unwrap().accounts.is_empty())
    }

    async fn iter(
        &self,
        prv_key_data_id_filter: Option<PrvKeyDataId>,
    ) -> Result<StorageStream<(Arc<AccountStorage>, Option<Arc<AccountMetadata>>)>> {
        Ok(Box::pin(AccountStream::new(self.cache.clone(), prv_key_data_id_filter)))
    }

    async fn len(&self, prv_key_data_id_filter: Option<PrvKeyDataId>) -> Result<usize> {
        let len = match prv_key_data_id_filter {
            Some(filter) => {
                self.cache.read().unwrap().accounts.vec.iter().filter(|account| account.prv_key_data_ids.contains(&filter)).count()
            }
            None => self.cache.read().unwrap().accounts.vec.len(),
        };

        Ok(len)
    }

    async fn load_single(&self, ids: &AccountId) -> Result<Option<(Arc<AccountStorage>, Option<Arc<AccountMetadata>>)>> {
        let cache = self.cache.read().unwrap();
        if let Some(account) = cache.accounts.load_single(ids)? {
            Ok(Some((account, cache.metadata.load_single(ids)?)))
        } else {
            Ok(None)
        }
    }

    async fn load_multiple(&self, ids: &[AccountId]) -> Result<Vec<(Arc<AccountStorage>, Option<Arc<AccountMetadata>>)>> {
        let cache = self.cache.read().unwrap();
        let accounts = cache.accounts.load_multiple(ids)?;
        accounts
            .into_iter()
            .map(|account| {
                cache.metadata.load_single(account.id()).map(|metadata| (account.clone(), metadata)).or_else(|_| Ok((account, None)))
            })
            .collect::<Result<Vec<_>>>()
    }

    async fn store_single(&self, account: &AccountStorage, metadata: Option<&AccountMetadata>) -> Result<()> {
        let mut cache = self.cache.write().unwrap();
        cache.accounts.store_single(account)?;
        if let Some(metadata) = metadata {
            cache.metadata.store_single(metadata)?;
        }
        self.set_modified(true);
        Ok(())
    }

    async fn store_multiple(&self, data: Vec<(AccountStorage, Option<AccountMetadata>)>) -> Result<()> {
        let mut cache = self.cache.write().unwrap();
        let (accounts, metadata): (Vec<_>, Vec<_>) = data.into_iter().unzip();
        cache.accounts.store_multiple(accounts)?;
        cache.metadata.store_multiple(metadata.into_iter().flatten().collect())?;
        self.set_modified(true);
        Ok(())
    }

    async fn remove(&self, ids: &[&AccountId]) -> Result<()> {
        let mut cache = self.cache.write().unwrap();
        cache.accounts.remove(ids)?;
        cache.metadata.remove(ids)?;

        self.set_modified(true);

        Ok(())
    }

    async fn update_metadata(&self, metadata: Vec<AccountMetadata>) -> Result<()> {
        self.cache.write().unwrap().metadata.store_multiple(metadata)?;
        self.update_stored_metadata().await?;
        Ok(())
    }
}

#[async_trait]
impl AddressBookStore for LocalStoreInner {
    async fn iter(&self) -> Result<StorageStream<Arc<AddressBookEntry>>> {
        Ok(Box::pin(AddressBookEntryStream::new(self.cache.clone())))
    }

    async fn search(&self, search: &str) -> Result<Vec<Arc<AddressBookEntry>>> {
        let matches = self
            .cache
            .read()
            .unwrap()
            .address_book
            .iter()
            .filter_map(|entry| if entry.alias.contains(search) { Some(Arc::new(entry.clone())) } else { None })
            .collect();

        Ok(matches)
    }
}

#[cfg(test)]
mod note_key_store_tests {
    use super::*;
    use kaspa_rpc_core::message::RpcNoteEntry;

    async fn resident_store(wallet_secret: &Secret) -> LocalStoreInner {
        let args = CreateArgs::new(None, None, EncryptionKind::XChaCha20Poly1305, None, false);
        LocalStoreInner::try_create(wallet_secret, "", args, true).await.unwrap()
    }

    #[tokio::test]
    async fn test_note_key_store_round_trip() -> Result<()> {
        let wallet_secret = Secret::from("note-key-db-test-secret");
        let store = resident_store(&wallet_secret).await;

        let sn = Hash::from([0x11u8; 32]);
        let sk = [0x22u8; 32];
        let entry = NoteKeyEntry::new(sn, sk, DenominationTag::D1, NoteProvenance::Cold);

        assert!(NoteKeyStore::is_empty(&store).await?);
        NoteKeyStore::store(&store, &wallet_secret, entry.clone()).await?;
        assert!(!NoteKeyStore::is_empty(&store).await?);

        let loaded = NoteKeyStore::load_key(&store, &wallet_secret, &sn).await?.expect("entry round-trips");
        assert_eq!(loaded, entry);

        let info = NoteKeyStore::load_info(&store, &sn).await?.expect("info round-trips");
        assert_eq!(info.provenance, NoteProvenance::Cold);
        assert_eq!(info.status, NoteStatus::Active);
        assert_eq!(info.d, DenominationTag::D1);
        assert_eq!(info.pk, entry.derive_pk()?);

        NoteKeyStore::remove(&store, &wallet_secret, &sn).await?;
        assert!(NoteKeyStore::load_key(&store, &wallet_secret, &sn).await?.is_none());
        assert!(NoteKeyStore::load_info(&store, &sn).await?.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn test_hot_keys_flagged_at_import() -> Result<()> {
        let wallet_secret = Secret::from("note-key-db-test-secret");
        let store = resident_store(&wallet_secret).await;

        let sn = Hash::from([0x33u8; 32]);
        let sk = [0x44u8; 32];

        // Even though nothing here claims Cold, a bearer import is always Hot
        // (POOL-SPEC.md P5.6's same-key-in-two-wallets hazard) — the entry point
        // itself doesn't accept a provenance argument.
        NoteKeyStore::import_bearer_key(&store, &wallet_secret, sn, sk, DenominationTag::D0_1).await?;

        let info = NoteKeyStore::load_info(&store, &sn).await?.expect("imported entry present");
        assert_eq!(info.provenance, NoteProvenance::Hot);

        Ok(())
    }

    #[tokio::test]
    async fn test_rotation_observed_on_chain_updates_status() -> Result<()> {
        let wallet_secret = Secret::from("note-key-db-test-secret");
        let store = resident_store(&wallet_secret).await;

        let sn_old = Hash::from([0x55u8; 32]);
        let sn_new = Hash::from([0x66u8; 32]);
        let sk = [0x77u8; 32];
        let entry = NoteKeyEntry::new(sn_old, sk, DenominationTag::D1, NoteProvenance::Cold);
        let pk = entry.derive_pk()?;
        NoteKeyStore::store(&store, &wallet_secret, entry).await?;

        let notification = kaspa_rpc_core::message::NotesChangedNotification {
            added: Arc::new(vec![RpcNoteEntry { sn: sn_new, denomination: DenominationTag::D1 as u8, pk }]),
            removed: Arc::new(vec![RpcNoteEntry { sn: sn_old, denomination: DenominationTag::D1 as u8, pk }]),
        };

        // Without a secret: the removed half (plaintext-only) always applies; the
        // added half (needs to write a new encrypted row) is deferred.
        let result = NoteKeyStore::apply_notes_changed(&store, None, &notification).await?;
        assert_eq!(result.superseded, vec![sn_old]);
        assert!(result.added.is_empty());
        assert_eq!(result.deferred, vec![sn_new]);

        let old_info = NoteKeyStore::load_info(&store, &sn_old).await?.expect("old row retained as tombstone");
        assert_eq!(old_info.status, NoteStatus::Superseded);
        assert!(NoteKeyStore::load_info(&store, &sn_new).await?.is_none());

        // With a secret: the deferred row is now written too, inheriting sk/provenance
        // from the row that shares its pk.
        let result = NoteKeyStore::apply_notes_changed(&store, Some(&wallet_secret), &notification).await?;
        assert!(result.deferred.is_empty());
        assert_eq!(result.added, vec![sn_new]);

        let new_info = NoteKeyStore::load_info(&store, &sn_new).await?.expect("new row inserted");
        assert_eq!(new_info.status, NoteStatus::Active);
        assert_eq!(new_info.provenance, NoteProvenance::Cold);
        let new_key = NoteKeyStore::load_key(&store, &wallet_secret, &sn_new).await?.expect("new row's key stored");
        assert_eq!(new_key.sk, sk);

        Ok(())
    }
}
