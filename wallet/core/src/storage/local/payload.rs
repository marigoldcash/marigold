//!
//! Encrypted wallet payload storage.
//!

use crate::imports::*;
use crate::storage::{AddressBookEntry, PaymentRequestKey, PrvKeyData, PrvKeyDataId};
use kaspa_bip32::Mnemonic;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Payload {
    pub prv_key_data: Vec<PrvKeyData>,
    pub accounts: Vec<AccountStorage>,
    pub address_book: Vec<AddressBookEntry>,
    pub encrypt_transactions: Option<EncryptionKind>,
    // Note key database rows lived here as `note_key_data: Vec<NoteKeyEntry>`
    // through FORK-PLAN P7.1-P7.5; P7.6 moved them out into the file-per-note
    // vault (`storage::local::notevault::NoteVault`) entirely — they no longer
    // round-trip through this blob or through `wallet_export`/`wallet_import` at
    // all. Note backup/restore is the vault's own mechanism (DECISIONS.md's
    // "Note vault, backup, and restore-rotation policy").
    /// Outstanding payment-request keys (FORK-PLAN P7.3, POOL-SPEC.md P5.5b) —
    /// explicitly NOT migrated to the vault (still short-lived, unpaid-invoice
    /// state rather than held bearer value); unchanged by P7.6.
    pub payment_request_keys: Vec<PaymentRequestKey>,
}

impl Payload {
    const STORAGE_MAGIC: u32 = 0x41544144;
    const STORAGE_VERSION: u32 = 0;

    pub fn new(prv_key_data: Vec<PrvKeyData>, accounts: Vec<AccountStorage>, address_book: Vec<AddressBookEntry>) -> Self {
        Self { prv_key_data, accounts, address_book, encrypt_transactions: None, payment_request_keys: Vec::new() }
    }
}

impl ZeroizeOnDrop for Payload {}

impl Zeroize for Payload {
    fn zeroize(&mut self) {
        self.prv_key_data.zeroize();
        self.payment_request_keys.iter_mut().for_each(|key| key.zeroize());
    }
}

impl Payload {
    pub fn add_prv_key_data(
        &mut self,
        mnemonic: Mnemonic,
        payment_secret: Option<&Secret>,
        encryption_kind: EncryptionKind,
    ) -> Result<PrvKeyData> {
        let prv_key_data = PrvKeyData::try_new_from_mnemonic(mnemonic, payment_secret, encryption_kind)?;

        if !self.prv_key_data.iter().any(|existing_key_data| prv_key_data.id == existing_key_data.id) {
            self.prv_key_data.push(prv_key_data.clone());
            Ok(prv_key_data)
        } else {
            Err(Error::custom("private key data id already exists in the wallet"))
        }
    }

    pub fn find_prv_key_data(&self, id: &PrvKeyDataId) -> Option<&PrvKeyData> {
        self.prv_key_data.iter().find(|prv_key_data| prv_key_data.id == *id)
    }
}

impl BorshSerialize for Payload {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        StorageHeader::new(Self::STORAGE_MAGIC, Self::STORAGE_VERSION).serialize(writer)?;
        BorshSerialize::serialize(&self.prv_key_data, writer)?;
        BorshSerialize::serialize(&self.accounts, writer)?;
        BorshSerialize::serialize(&self.address_book, writer)?;
        BorshSerialize::serialize(&self.encrypt_transactions, writer)?;
        BorshSerialize::serialize(&self.payment_request_keys, writer)?;

        Ok(())
    }
}

impl BorshDeserialize for Payload {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> IoResult<Self> {
        let StorageHeader { version: _, .. } =
            StorageHeader::deserialize_reader(reader)?.try_magic(Self::STORAGE_MAGIC)?.try_version(Self::STORAGE_VERSION)?;
        let prv_key_data = BorshDeserialize::deserialize_reader(reader)?;
        let accounts = BorshDeserialize::deserialize_reader(reader)?;
        let address_book = BorshDeserialize::deserialize_reader(reader)?;
        let encrypt_transactions = BorshDeserialize::deserialize_reader(reader)?;
        let payment_request_keys = BorshDeserialize::deserialize_reader(reader)?;

        Ok(Self { prv_key_data, accounts, address_book, encrypt_transactions, payment_request_keys })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::*;

    #[test]
    fn test_storage_wallet_payload() -> Result<()> {
        let storable_in = Payload::new(vec![], vec![], vec![]);
        let guard = StorageGuard::new(&storable_in);
        let _storable_out = guard.validate()?;

        Ok(())
    }
}
