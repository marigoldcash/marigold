//!
//! Encrypted wallet payload storage.
//!

use crate::imports::*;
use crate::storage::{AddressBookEntry, Otp, PaymentRequestKey, PrvKeyData, PrvKeyDataId};
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
    /// The enrolled authenticator, if there is one (v1). It lives in here
    /// rather than beside the file so that a copied wallet carries its own
    /// session protection, and so that the secret is never at rest in the
    /// clear. What that does and does not defend is set out in
    /// [`crate::storage::otp`].
    pub otp: Option<Otp>,
}

impl Payload {
    pub(crate) const STORAGE_MAGIC: u32 = 0x41544144;
    /// v1 appended `otp`. A v0 payload stops after `payment_request_keys`
    /// and is read back as having no authenticator.
    const STORAGE_VERSION: u32 = 1;

    pub fn new(prv_key_data: Vec<PrvKeyData>, accounts: Vec<AccountStorage>, address_book: Vec<AddressBookEntry>) -> Self {
        Self { prv_key_data, accounts, address_book, encrypt_transactions: None, payment_request_keys: Vec::new(), otp: None }
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
        // v1 tail — readers gate on the header version.
        BorshSerialize::serialize(&self.otp, writer)?;

        Ok(())
    }
}

impl BorshDeserialize for Payload {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> IoResult<Self> {
        let StorageHeader { version, .. } =
            StorageHeader::deserialize_reader(reader)?.try_magic(Self::STORAGE_MAGIC)?.try_version(Self::STORAGE_VERSION)?;
        let prv_key_data = BorshDeserialize::deserialize_reader(reader)?;
        let accounts = BorshDeserialize::deserialize_reader(reader)?;
        let address_book = BorshDeserialize::deserialize_reader(reader)?;
        let encrypt_transactions = BorshDeserialize::deserialize_reader(reader)?;
        let payment_request_keys = BorshDeserialize::deserialize_reader(reader)?;
        // Version-gated: a v0 payload ends above. Borsh is positional with no
        // length prefix, so reading this unconditionally would fail every
        // wallet written before v1.
        let otp = if version >= 1 { BorshDeserialize::deserialize_reader(reader)? } else { None };

        Ok(Self { prv_key_data, accounts, address_book, encrypt_transactions, payment_request_keys, otp })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::*;

    /// Every wallet in the field has a v0 payload. If this breaks, those
    /// wallets stop opening — which is to say the money stops opening.
    #[test]
    fn a_v0_payload_still_reads_and_reports_no_authenticator() -> Result<()> {
        let payload = Payload::new(vec![], vec![], vec![]);

        // A v0 payload is the same five fields under a version-0 header, with
        // no tail at all.
        let mut v0 = Vec::new();
        StorageHeader::new(Payload::STORAGE_MAGIC, 0).serialize(&mut v0)?;
        BorshSerialize::serialize(&payload.prv_key_data, &mut v0)?;
        BorshSerialize::serialize(&payload.accounts, &mut v0)?;
        BorshSerialize::serialize(&payload.address_book, &mut v0)?;
        BorshSerialize::serialize(&payload.encrypt_transactions, &mut v0)?;
        BorshSerialize::serialize(&payload.payment_request_keys, &mut v0)?;

        let read: Payload = BorshDeserialize::try_from_slice(&v0)?;
        assert!(read.otp.is_none(), "a wallet from before this feature has no authenticator, not a broken one");
        Ok(())
    }

    #[test]
    fn an_enrolled_authenticator_survives_a_round_trip() -> Result<()> {
        let mut payload = Payload::new(vec![], vec![], vec![]);
        let otp = crate::storage::Otp::generate();
        payload.otp = Some(otp.clone());

        let bytes = borsh::to_vec(&payload)?;
        let read: Payload = BorshDeserialize::try_from_slice(&bytes)?;
        let read = read.otp.expect("the authenticator comes back");
        assert_eq!(read.secret, otp.secret);
        assert_eq!(read.grace_secs, otp.grace_secs);
        Ok(())
    }

    #[test]
    fn test_storage_wallet_payload() -> Result<()> {
        let storable_in = Payload::new(vec![], vec![], vec![]);
        let guard = StorageGuard::new(&storable_in);
        let _storable_out = guard.validate()?;

        Ok(())
    }
}
