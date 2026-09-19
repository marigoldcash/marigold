use crate::core::model::{CompactUtxoCollection, CompactUtxoEntry, UtxoSetByScriptPublicKey};

use kaspa_consensus_core::tx::{
    ScriptPublicKey, ScriptPublicKeyVersion, ScriptPublicKeys, ScriptVec, TransactionIndexType, TransactionOutpoint,
};
use kaspa_core::debug;
use kaspa_database::prelude::{CachePolicy, CachedDbAccess, DB, DirectDbWriter, StoreResult};
use kaspa_database::registry::DatabaseStorePrefixes;
use kaspa_hashes::Hash;
use kaspa_index_core::indexed_utxos::BalanceByScriptPublicKey;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt::Display;
use std::sync::Arc;

pub const VERSION_TYPE_SIZE: usize = size_of::<ScriptPublicKeyVersion>(); // Const since we need to re-use this a few times.

/// [`ScriptPublicKeyBucket`].
/// Consists of 2 bytes of little endian [VersionType] bytes, followed by the script length (8) and by a variable size of [ScriptVec].
#[derive(Eq, Hash, PartialEq, Debug, Clone)]
struct ScriptPublicKeyBucket(Vec<u8>);

impl From<&ScriptPublicKey> for ScriptPublicKeyBucket {
    fn from(script_public_key: &ScriptPublicKey) -> Self {
        // version (2) + length (8) + dynamic script
        let mut bytes: Vec<u8> = Vec::with_capacity(VERSION_TYPE_SIZE + size_of::<u64>() + script_public_key.script().len());
        bytes.extend_from_slice(&script_public_key.version().to_le_bytes());
        bytes.extend_from_slice(&(script_public_key.script().len() as u64).to_le_bytes()); // TODO: Consider using a smaller integer
        bytes.extend_from_slice(script_public_key.script());
        Self(bytes)
    }
}

impl From<ScriptPublicKeyBucket> for ScriptPublicKey {
    fn from(bucket: ScriptPublicKeyBucket) -> Self {
        let version = ScriptPublicKeyVersion::from_le_bytes(
            <[u8; VERSION_TYPE_SIZE]>::try_from(&bucket.0[..VERSION_TYPE_SIZE]).expect("expected version size"),
        );

        let script_size =
            u64::from_le_bytes(bucket.0[VERSION_TYPE_SIZE..VERSION_TYPE_SIZE + size_of::<u64>()].try_into().unwrap()) as usize;
        let script =
            ScriptVec::from_slice(&bucket.0[VERSION_TYPE_SIZE + size_of::<u64>()..VERSION_TYPE_SIZE + size_of::<u64>() + script_size]);

        Self::new(version, script)
    }
}

impl AsRef<[u8]> for ScriptPublicKeyBucket {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

// Keys:

// TransactionOutpoint:
/// Size of the [TransactionOutpointKey] in bytes.
pub const TRANSACTION_OUTPOINT_KEY_SIZE: usize = kaspa_hashes::HASH_SIZE + size_of::<TransactionIndexType>();

/// [TransactionOutpoint] key which references the [CompactUtxoEntry] within a [ScriptPublicKeyBucket]
/// Consists of 32 bytes of [TransactionId], followed by 4 bytes of little endian [TransactionIndexType]
#[derive(Eq, Hash, PartialEq, Debug, Copy, Clone)]
struct TransactionOutpointKey([u8; TRANSACTION_OUTPOINT_KEY_SIZE]);

impl From<TransactionOutpointKey> for TransactionOutpoint {
    fn from(key: TransactionOutpointKey) -> Self {
        let transaction_id = Hash::from_slice(&key.0[..kaspa_hashes::HASH_SIZE]);
        let index = TransactionIndexType::from_le_bytes(
            <[u8; size_of::<TransactionIndexType>()]>::try_from(&key.0[kaspa_hashes::HASH_SIZE..]).expect("expected index size"),
        );
        Self::new(transaction_id, index)
    }
}

impl From<&TransactionOutpoint> for TransactionOutpointKey {
    fn from(outpoint: &TransactionOutpoint) -> Self {
        let mut bytes = [0; TRANSACTION_OUTPOINT_KEY_SIZE];
        bytes[..kaspa_hashes::HASH_SIZE].copy_from_slice(&outpoint.transaction_id.as_bytes());
        bytes[kaspa_hashes::HASH_SIZE..].copy_from_slice(&outpoint.index.to_le_bytes());
        Self(bytes)
    }
}

impl AsRef<[u8]> for TransactionOutpointKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Full [CompactUtxoEntry] access key.
/// Consists of variable amount of bytes of [ScriptPublicKeyBucket], and 36 bytes of [TransactionOutpointKey]
#[derive(Eq, Hash, PartialEq, Debug, Clone, Serialize, Deserialize)]
struct UtxoEntryFullAccessKey(Arc<Vec<u8>>);

impl Display for UtxoEntryFullAccessKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self) // TODO: Deserialize first
    }
}

impl UtxoEntryFullAccessKey {
    /// Creates a new [UtxoEntryFullAccessKey] from a [ScriptPublicKeyBucket] and [TransactionOutpointKey].
    pub fn new(script_public_key_bucket: ScriptPublicKeyBucket, transaction_outpoint_key: TransactionOutpointKey) -> Self {
        let mut bytes = Vec::with_capacity(script_public_key_bucket.as_ref().len() + TRANSACTION_OUTPOINT_KEY_SIZE);
        bytes.extend_from_slice(script_public_key_bucket.as_ref());
        bytes.extend_from_slice(transaction_outpoint_key.as_ref());
        Self(Arc::new(bytes))
    }

    pub fn extract_outpoint(&self) -> TransactionOutpoint {
        TransactionOutpoint::from(TransactionOutpointKey(self.0[(self.0.len() - TRANSACTION_OUTPOINT_KEY_SIZE)..].try_into().unwrap()))
    }
}

impl AsRef<[u8]> for UtxoEntryFullAccessKey {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

// Traits:

pub trait UtxoSetByScriptPublicKeyStoreReader {
    /// Get [UtxoSetByScriptPublicKey] set by queried [ScriptPublicKeys],
    fn get_utxos_from_script_public_keys(&self, script_public_keys: ScriptPublicKeys) -> StoreResult<UtxoSetByScriptPublicKey>;
    /// One page of a single script public key's UTXOs, in key order: up to `limit` entries strictly after `after` (or from the start).
    /// This is the chunked retrieval the TODO on `get_utxos_from_script_public_keys` asks for; it costs one seek, not a scan.
    fn get_utxos_page_from_script_public_key(
        &self,
        script_public_key: &ScriptPublicKey,
        after: Option<&TransactionOutpoint>,
        limit: usize,
    ) -> StoreResult<Vec<(TransactionOutpoint, CompactUtxoEntry)>>;
    fn get_balance_from_script_public_keys(&self, script_public_keys: ScriptPublicKeys) -> StoreResult<BalanceByScriptPublicKey>;
    fn get_all_outpoints(&self) -> StoreResult<HashSet<TransactionOutpoint>>; // This can have a big memory footprint, so it should be used only for tests.
}

pub trait UtxoSetByScriptPublicKeyStore: UtxoSetByScriptPublicKeyStoreReader {
    /// remove [UtxoSetByScriptPublicKey] from the [UtxoSetByScriptPublicKeyStore].
    fn remove_utxo_entries(&mut self, utxo_entries: &UtxoSetByScriptPublicKey) -> StoreResult<()>;

    /// add [UtxoSetByScriptPublicKey] into the [UtxoSetByScriptPublicKeyStore].
    fn add_utxo_entries(&mut self, utxo_entries: &UtxoSetByScriptPublicKey) -> StoreResult<()>;

    /// removes all entries in the cache and db, besides prefixes themselves.
    fn delete_all(&mut self) -> StoreResult<()>;
}

// Implementations:

#[derive(Clone)]
pub struct DbUtxoSetByScriptPublicKeyStore {
    db: Arc<DB>,
    access: CachedDbAccess<UtxoEntryFullAccessKey, CompactUtxoEntry>,
}

impl DbUtxoSetByScriptPublicKeyStore {
    pub fn new(db: Arc<DB>, cache_policy: CachePolicy) -> Self {
        Self { db: Arc::clone(&db), access: CachedDbAccess::new(db, cache_policy, DatabaseStorePrefixes::UtxoIndex.into()) }
    }
}

impl UtxoSetByScriptPublicKeyStoreReader for DbUtxoSetByScriptPublicKeyStore {
    // compared to go-kaspad this gets transaction outpoints from multiple script public keys at once.
    // TODO: probably ideal way to retrieve is to return a chained iterator which can be used to chunk results and propagate utxo entries
    // to the rpc via pagination, this would alleviate the memory footprint of script public keys with large amount of utxos.
    fn get_utxos_from_script_public_keys(&self, script_public_keys: ScriptPublicKeys) -> StoreResult<UtxoSetByScriptPublicKey> {
        let script_count = script_public_keys.len();
        let mut entries_count: usize = 0;
        let mut utxos_by_script_public_keys = UtxoSetByScriptPublicKey::new();
        for script_public_key in script_public_keys.into_iter() {
            let script_public_key_bucket = ScriptPublicKeyBucket::from(&script_public_key);
            let utxos_by_script_public_keys_inner = CompactUtxoCollection::from_iter(
                self.access.seek_iterator(Some(script_public_key_bucket.as_ref()), None, usize::MAX, false).map(|res| {
                    let (key, entry) = res.unwrap();
                    (TransactionOutpointKey(<[u8; TRANSACTION_OUTPOINT_KEY_SIZE]>::try_from(&key[..]).unwrap()).into(), entry)
                }),
            );
            entries_count += utxos_by_script_public_keys_inner.len();
            utxos_by_script_public_keys.insert(script_public_key, utxos_by_script_public_keys_inner);
        }
        debug!("IDXPRC, Executed a query for the utxo set of {} script public keys yielding {} entries", script_count, entries_count);
        Ok(utxos_by_script_public_keys)
    }

    fn get_utxos_page_from_script_public_key(
        &self,
        script_public_key: &ScriptPublicKey,
        after: Option<&TransactionOutpoint>,
        limit: usize,
    ) -> StoreResult<Vec<(TransactionOutpoint, CompactUtxoEntry)>> {
        // "Up to `limit`" includes zero. Without this, `limit + 1` below seeks
        // one entry and the length check runs after the push, so a zero limit
        // returned one entry — the opposite of what was asked.
        if limit == 0 {
            return Ok(Vec::new());
        }
        let bucket = ScriptPublicKeyBucket::from(script_public_key);
        // Seek to the cursor rather than past it, and drop it by comparison if
        // it is still there. RocksDB's seek lands on the first key >= target;
        // if the cursor entry was spent between two pages, that first key is a
        // real, unseen entry — `skip_first` would have thrown it away.
        let seek_from = after.map(|outpoint| UtxoEntryFullAccessKey::new(bucket.clone(), TransactionOutpointKey::from(outpoint)));
        let mut page = Vec::with_capacity(limit);
        for res in self.access.seek_iterator(Some(bucket.as_ref()), seek_from, limit + 1, false) {
            let (key, entry) = res.unwrap();
            let outpoint: TransactionOutpoint =
                TransactionOutpointKey(<[u8; TRANSACTION_OUTPOINT_KEY_SIZE]>::try_from(&key[..]).unwrap()).into();
            if after == Some(&outpoint) {
                continue;
            }
            page.push((outpoint, entry));
            if page.len() == limit {
                break;
            }
        }
        Ok(page)
    }

    fn get_balance_from_script_public_keys(&self, script_public_keys: ScriptPublicKeys) -> StoreResult<BalanceByScriptPublicKey> {
        let script_count = script_public_keys.len();
        let mut entries_count: usize = 0;
        let mut balance_by_script_public_keys = BalanceByScriptPublicKey::new();
        for script_public_key in script_public_keys.into_iter() {
            let script_public_key_bucket = ScriptPublicKeyBucket::from(&script_public_key);
            let balance: u64 = self
                .access
                .seek_iterator(Some(script_public_key_bucket.as_ref()), None, usize::MAX, false)
                .map(|res| {
                    entries_count += 1;
                    let (_, entry) = res.unwrap();
                    entry.amount
                })
                .sum();
            balance_by_script_public_keys.insert(script_public_key, balance);
        }
        debug!("IDXPRC, Executed a query for the balance of {} script public keys involving {} entries", script_count, entries_count);
        Ok(balance_by_script_public_keys)
    }

    // This can have a big memory footprint, so it should be used only for tests.
    fn get_all_outpoints(&self) -> StoreResult<HashSet<TransactionOutpoint>> {
        Ok(HashSet::from_iter(
            self.access.iterator().map(|res| UtxoEntryFullAccessKey(Arc::new(res.unwrap().0.to_vec())).extract_outpoint()),
        ))
    }
}

impl UtxoSetByScriptPublicKeyStore for DbUtxoSetByScriptPublicKeyStore {
    fn remove_utxo_entries(&mut self, utxo_entries: &UtxoSetByScriptPublicKey) -> StoreResult<()> {
        if utxo_entries.is_empty() {
            return Ok(());
        }

        let mut writer = DirectDbWriter::new(&self.db);

        let mut to_remove = utxo_entries.iter().flat_map(move |(script_public_key, compact_utxo_collection)| {
            compact_utxo_collection.keys().map(move |transaction_outpoint| {
                UtxoEntryFullAccessKey::new(
                    ScriptPublicKeyBucket::from(script_public_key),
                    TransactionOutpointKey::from(transaction_outpoint),
                )
            })
        });

        self.access.delete_many(&mut writer, &mut to_remove)?;

        Ok(())
    }

    fn add_utxo_entries(&mut self, utxo_entries: &UtxoSetByScriptPublicKey) -> StoreResult<()> {
        if utxo_entries.is_empty() {
            return Ok(());
        }

        let mut writer = DirectDbWriter::new(&self.db);

        let mut to_add = utxo_entries.iter().flat_map(move |(script_public_key, compact_utxo_collection)| {
            compact_utxo_collection.iter().map(move |(transaction_outpoint, compact_utxo)| {
                (
                    UtxoEntryFullAccessKey::new(
                        ScriptPublicKeyBucket::from(script_public_key),
                        TransactionOutpointKey::from(transaction_outpoint),
                    ),
                    *compact_utxo,
                )
            })
        });

        self.access.write_many(&mut writer, &mut to_add)?;

        Ok(())
    }

    /// Removes all entries in the cache and db, besides prefixes themselves.
    fn delete_all(&mut self) -> StoreResult<()> {
        self.access.delete_all(DirectDbWriter::new(&self.db))
    }
}

#[cfg(test)]
mod paging_tests {
    use super::*;
    use kaspa_database::create_temp_db;
    use kaspa_database::prelude::ConnBuilder;

    fn spk(tag: u8) -> ScriptPublicKey {
        ScriptPublicKey::from_vec(0, vec![tag; 34])
    }

    fn outpoint(n: u64) -> TransactionOutpoint {
        TransactionOutpoint::new(Hash::from_u64_word(n), (n % 3) as TransactionIndexType)
    }

    fn entry(n: u64) -> CompactUtxoEntry {
        CompactUtxoEntry::new(n * 1_000, n, false, None)
    }

    fn store_with(spks: &[(ScriptPublicKey, &[u64])]) -> (Box<dyn std::any::Any>, DbUtxoSetByScriptPublicKeyStore) {
        let (lifetime, db) = create_temp_db!(ConnBuilder::default().with_files_limit(10));
        let mut store = DbUtxoSetByScriptPublicKeyStore::new(db, CachePolicy::Empty);
        let mut set = UtxoSetByScriptPublicKey::new();
        for (spk, ns) in spks {
            let collection: CompactUtxoCollection = ns.iter().map(|&n| (outpoint(n), entry(n))).collect();
            set.insert(spk.clone(), collection);
        }
        store.add_utxo_entries(&set).unwrap();
        (Box::new(lifetime), store)
    }

    fn key_bytes(o: &TransactionOutpoint) -> [u8; TRANSACTION_OUTPOINT_KEY_SIZE] {
        TransactionOutpointKey::from(o).0
    }

    /// Walk an address a page at a time the way the RPC does: resume after
    /// the last outpoint of the previous page until a page comes back short.
    fn walk(store: &DbUtxoSetByScriptPublicKeyStore, spk: &ScriptPublicKey, page: usize) -> Vec<Vec<TransactionOutpoint>> {
        let mut pages = Vec::new();
        let mut after: Option<TransactionOutpoint> = None;
        loop {
            let got: Vec<TransactionOutpoint> =
                store.get_utxos_page_from_script_public_key(spk, after.as_ref(), page).unwrap().into_iter().map(|(o, _)| o).collect();
            let short = got.len() < page;
            after = got.last().copied();
            pages.push(got);
            if short {
                break;
            }
        }
        pages
    }

    /// Every entry exactly once, in key order, across page boundaries.
    #[test]
    fn a_paged_walk_covers_the_address_exactly_once_in_key_order() {
        let a = spk(1);
        let ns: Vec<u64> = (1..=7).collect();
        let (_db, store) = store_with(&[(a.clone(), &ns)]);

        let pages = walk(&store, &a, 3);
        assert_eq!(pages.iter().map(Vec::len).collect::<Vec<_>>(), vec![3, 3, 1], "7 entries in pages of 3");

        let flat: Vec<TransactionOutpoint> = pages.concat();
        let inserted: HashSet<TransactionOutpoint> = ns.iter().map(|&n| outpoint(n)).collect();
        assert_eq!(flat.iter().copied().collect::<HashSet<_>>(), inserted, "the union is the set");
        assert_eq!(flat.len(), inserted.len(), "and nothing is returned twice");
        assert!(flat.windows(2).all(|w| key_bytes(&w[0]) < key_bytes(&w[1])), "strictly ascending by key, across pages too");
    }

    /// The bug the design avoids: if the entry the cursor points at is spent
    /// between two pages, the next page must begin with the entry after it,
    /// not the one after *that*. A seek-then-skip-first would lose one here.
    #[test]
    fn a_cursor_entry_spent_between_pages_does_not_swallow_its_successor() {
        let a = spk(2);
        let ns: Vec<u64> = (1..=6).collect();
        let (_db, mut store) = store_with(&[(a.clone(), &ns)]);

        let first: Vec<TransactionOutpoint> =
            store.get_utxos_page_from_script_public_key(&a, None, 3).unwrap().into_iter().map(|(o, _)| o).collect();
        let cursor = *first.last().unwrap();

        // Spend the cursor entry.
        let mut gone = UtxoSetByScriptPublicKey::new();
        gone.insert(a.clone(), [(cursor, entry(0))].into_iter().collect());
        store.remove_utxo_entries(&gone).unwrap();

        let rest: Vec<TransactionOutpoint> =
            store.get_utxos_page_from_script_public_key(&a, Some(&cursor), 10).unwrap().into_iter().map(|(o, _)| o).collect();

        let seen: HashSet<TransactionOutpoint> = first.iter().chain(rest.iter()).copied().collect();
        let expected: HashSet<TransactionOutpoint> = ns.iter().map(|&n| outpoint(n)).collect();
        assert_eq!(seen, expected, "everything inserted is still seen exactly once, the spent cursor included from page one");
        assert!(!rest.contains(&cursor), "the spent entry is not re-served");
        assert!(rest.iter().all(|o| key_bytes(o) > key_bytes(&cursor)), "the second page is strictly after the cursor");
        assert_eq!(rest.len(), 3, "three remained after the cursor and all three arrived");
    }

    #[test]
    fn an_empty_address_and_an_oversized_limit_both_behave() {
        let a = spk(3);
        let b = spk(4);
        let (_db, store) = store_with(&[(a.clone(), &[10, 11, 12])]);

        assert!(
            store.get_utxos_page_from_script_public_key(&b, None, 5).unwrap().is_empty(),
            "nothing under an address that holds nothing"
        );
        assert_eq!(
            store.get_utxos_page_from_script_public_key(&a, None, 1_000).unwrap().len(),
            3,
            "a limit above the count returns the lot"
        );
        assert_eq!(
            store.get_utxos_page_from_script_public_key(&a, None, 0).unwrap().len(),
            0,
            "a zero limit returns nothing rather than everything"
        );
    }

    /// Buckets are per script public key; a page of one address must never
    /// leak into the next address's entries however the keys happen to sort.
    #[test]
    fn pages_stay_inside_their_own_address() {
        let a = spk(5);
        let b = spk(6);
        let (_db, store) = store_with(&[(a.clone(), &[1, 2, 3, 4]), (b.clone(), &[5, 6, 7, 8])]);

        let a_all: HashSet<TransactionOutpoint> = walk(&store, &a, 3).concat().into_iter().collect();
        let b_all: HashSet<TransactionOutpoint> = walk(&store, &b, 3).concat().into_iter().collect();
        assert_eq!(a_all, (1..=4).map(outpoint).collect::<HashSet<_>>());
        assert_eq!(b_all, (5..=8).map(outpoint).collect::<HashSet<_>>());
        assert!(a_all.is_disjoint(&b_all));
    }
}
