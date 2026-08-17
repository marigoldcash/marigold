//!
//! Wallet data cache retained in memory during the wallet session.
//!

use crate::imports::*;
use crate::storage::local::wallet::WalletStorage;
use crate::storage::local::*;
use std::collections::HashMap;

pub struct Cache {
    pub wallet_title: Option<String>,
    pub user_hint: Option<Hint>,
    pub encryption_kind: EncryptionKind,
    pub prv_key_data: Encrypted,
    pub prv_key_data_info: Collection<PrvKeyDataId, PrvKeyDataInfo>,
    pub accounts: Collection<AccountId, AccountStorage>,
    pub metadata: Collection<AccountId, AccountMetadata>,
    pub address_book: Vec<AddressBookEntry>,
    // Note key storage lived here as an encrypted `note_key_data: Encrypted` map
    // plus a plaintext `note_key_info: Collection<Hash, NoteKeyInfo>` index through
    // P7.1-P7.5; P7.6 moved it out to `storage::local::notevault::NoteVault`
    // (file-per-note, its own in-memory index) — `LocalStoreInner`'s `NoteKeyStore`
    // impl now delegates there directly instead of touching this cache.
    /// Outstanding payment-request keys (FORK-PLAN P7.3) — encrypted `pk -> key` map.
    pub payment_request_data: Encrypted,
    /// Plaintext index alongside `payment_request_data` (the pk is the QR's own
    /// public content; the amount is invoice metadata). Plain Vec, not a
    /// `Collection` — requests are few and short-lived, linear scans are fine.
    pub payment_request_info: Vec<PaymentRequestInfo>,
}

fn payment_request_map_and_info(keys: Vec<PaymentRequestKey>) -> Result<(PaymentRequestMap, Vec<PaymentRequestInfo>)> {
    let mut map = PaymentRequestMap::new();
    let mut info = Vec::with_capacity(keys.len());
    for key in keys {
        let pk = key.derive_pk()?;
        info.push(PaymentRequestInfo { pk, amount_petals: key.amount_petals });
        map.insert(pk, key);
    }
    Ok((map, info))
}

impl Cache {
    pub fn from_wallet(wallet: WalletStorage, secret: &Secret) -> Result<Self> {
        let payload = wallet.payload(secret)?;

        let prv_key_data_info =
            payload.0.prv_key_data.iter().map(|pkdata| pkdata.into()).collect::<Vec<PrvKeyDataInfo>>().try_into()?;

        let prv_key_data_map = payload.0.prv_key_data.into_iter().map(|pkdata| (pkdata.id, pkdata)).collect::<HashMap<_, _>>();
        let prv_key_data: Decrypted<PrvKeyDataMap> = Decrypted::new(prv_key_data_map);
        let encryption_kind = wallet.encryption_kind;
        let prv_key_data = prv_key_data.encrypt(secret, encryption_kind)?;
        let accounts: Collection<AccountId, AccountStorage> = payload.0.accounts.try_into()?;
        let metadata: Collection<AccountId, AccountMetadata> = wallet.metadata.try_into()?;
        let user_hint = wallet.user_hint;
        let wallet_title = wallet.title;
        let address_book = payload.0.address_book.into_iter().collect();

        let (payment_request_map, payment_request_info) = payment_request_map_and_info(payload.0.payment_request_keys.clone())?;
        let payment_request_data = Decrypted::new(payment_request_map).encrypt(secret, encryption_kind)?;

        Ok(Cache {
            wallet_title,
            user_hint,
            encryption_kind,
            prv_key_data,
            prv_key_data_info,
            accounts,
            metadata,
            address_book,
            payment_request_data,
            payment_request_info,
        })
    }

    pub fn from_payload(
        wallet_title: Option<String>,
        user_hint: Option<Hint>,
        payload: Payload,
        secret: &Secret,
        encryption_kind: EncryptionKind,
    ) -> Result<Self> {
        let prv_key_data_info = payload.prv_key_data.iter().map(|pkdata| pkdata.into()).collect::<Vec<PrvKeyDataInfo>>().try_into()?;

        let prv_key_data_map = payload.prv_key_data.into_iter().map(|pkdata| (pkdata.id, pkdata)).collect::<HashMap<_, _>>();
        let prv_key_data: Decrypted<PrvKeyDataMap> = Decrypted::new(prv_key_data_map);
        let prv_key_data = prv_key_data.encrypt(secret, encryption_kind)?;
        let accounts: Collection<AccountId, AccountStorage> = payload.accounts.try_into()?;
        let metadata: Collection<AccountId, AccountMetadata> = Collection::default();
        let address_book = payload.address_book.into_iter().collect();

        let (payment_request_map, payment_request_info) = payment_request_map_and_info(payload.payment_request_keys)?;
        let payment_request_data = Decrypted::new(payment_request_map).encrypt(secret, encryption_kind)?;

        Ok(Cache {
            wallet_title,
            user_hint,
            encryption_kind,
            prv_key_data,
            prv_key_data_info,
            accounts,
            metadata,
            address_book,
            payment_request_data,
            payment_request_info,
        })
    }

    pub fn to_wallet(
        &self,
        transactions: Option<Encryptable<HashMap<AccountId, Vec<TransactionRecord>>>>,
        secret: &Secret,
    ) -> Result<WalletStorage> {
        let prv_key_data: Decrypted<PrvKeyDataMap> = self.prv_key_data.decrypt(secret)?;
        let prv_key_data = prv_key_data.values().cloned().collect::<Vec<_>>();
        let accounts: Vec<AccountStorage> = (&self.accounts).try_into()?;
        let metadata: Vec<AccountMetadata> = (&self.metadata).try_into()?;
        let address_book = self.address_book.clone();
        let payment_request_keys: Decrypted<PaymentRequestMap> = self.payment_request_data.decrypt(secret)?;
        let payment_request_keys = payment_request_keys.values().cloned().collect::<Vec<_>>();
        let mut payload = Payload::new(prv_key_data, accounts, address_book);
        payload.payment_request_keys = payment_request_keys;
        let payload = Decrypted::new(payload).encrypt(secret, self.encryption_kind)?;

        Ok(WalletStorage {
            encryption_kind: self.encryption_kind,
            payload,
            metadata,
            user_hint: self.user_hint.clone(),
            title: self.wallet_title.clone(),
            transactions,
        })
    }
}
