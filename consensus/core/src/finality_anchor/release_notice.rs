//! # Release notices
//!
//! A trustee-signed statement of the oldest wallet release that still follows the
//! current consensus (PLAN P9.4f, 2026-09-22). Not a consensus rule and never carried
//! on chain: it is a JSON document the wallet fetches from the project's website and
//! verifies against the same pinned trustee keys as the finality anchors. A wallet whose
//! `major.release` is below the notice's minimum stops with a message and the download
//! link instead of following a chain the trustees no longer certify.
//!
//! The statement is deliberately small: network, minimum release, issue time. It says
//! nothing about *which* rule changed — that is the release's job — and it carries no
//! URL, so a notice can never send anyone anywhere; the download link is compiled into
//! the wallet. A quorum of trustees signs, like an anchor, because a single leaked key
//! must not be able to shut every wallet on the network down.

use super::{ANCHOR_QUORUM, FinalityAnchorError, TRUSTEE_COUNT, TrusteeKeys};
use crate::Hash;
use borsh::{BorshDeserialize, BorshSerialize};
use kaspa_hashes::{HasherBase, ReleaseNoticeSigningHash};

/// The signed statement. `network` is the network id as the wallet prints it
/// (`testnet-10`, `mainnet`), so a testnet notice cannot be replayed against mainnet
/// wallets. `min_major`/`min_release` are the first two numbers of the wallet version
/// (`2.58` of `2.58.257`): builds of one release all follow the same rules, so the
/// build number never matters here.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ReleaseNotice {
    pub network: String,
    pub min_major: u32,
    pub min_release: u32,
    /// Unix seconds when the trustees signed. A wallet keeps the newest notice it
    /// has verified; an older one never replaces it.
    pub issued_at: u64,
}

/// A notice plus a quorum of trustee signatures, bitmap-indexed like an anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedReleaseNotice {
    pub notice: ReleaseNotice,
    pub signer_bitmap: u8,
    pub signatures: Vec<[u8; 64]>,
}

impl ReleaseNotice {
    /// The message every signature covers: `H("ReleaseNotice" || borsh(self))`.
    pub fn signing_hash(&self) -> Hash {
        let mut hasher = ReleaseNoticeSigningHash::new();
        hasher.update(borsh::to_vec(self).expect("borsh serialization of ReleaseNotice cannot fail"));
        hasher.finalize()
    }

    /// One trustee's signature over this notice.
    pub fn sign(&self, keypair: &secp256k1::Keypair) -> [u8; 64] {
        let msg = secp256k1::Message::from_digest(self.signing_hash().into());
        *secp256k1::SECP256K1.sign_schnorr_no_aux_rand(&msg, keypair).as_ref()
    }

    /// Whether a wallet of `major.release` is older than this notice allows.
    pub fn refuses(&self, major: u32, release: u32) -> bool {
        (major, release) < (self.min_major, self.min_release)
    }
}

impl SignedReleaseNotice {
    /// Assembles a signed notice from individually collected `(trustee index,
    /// signature)` pairs; duplicates and out-of-range indices are refused. Verification
    /// is a separate step, so a bad signature is reported as such rather than as a
    /// malformed document.
    pub fn assemble(notice: ReleaseNotice, mut signed: Vec<(u8, [u8; 64])>) -> Result<Self, FinalityAnchorError> {
        signed.sort_unstable_by_key(|(index, _)| *index);
        let mut signer_bitmap = 0u8;
        let mut signatures = Vec::with_capacity(signed.len());
        for (index, signature) in signed {
            if index as usize >= TRUSTEE_COUNT {
                return Err(FinalityAnchorError::TrusteeIndexOutOfRange(index));
            }
            if signer_bitmap & (1 << index) != 0 {
                // Two signatures by one key count once; the bitmap cannot express
                // otherwise, so the second is dropped rather than counted.
                continue;
            }
            signer_bitmap |= 1 << index;
            signatures.push(signature);
        }
        Ok(Self { notice, signer_bitmap, signatures })
    }

    /// Full verification: at least a quorum of distinct trustees, every signature
    /// verifying against its pinned key.
    pub fn verify(&self, trustee_keys: &TrusteeKeys) -> Result<(), FinalityAnchorError> {
        if self.signer_bitmap >> TRUSTEE_COUNT != 0 {
            return Err(FinalityAnchorError::BitmapOutOfRange(self.signer_bitmap));
        }
        let signer_count = self.signer_bitmap.count_ones() as usize;
        if signer_count != self.signatures.len() {
            return Err(FinalityAnchorError::SignatureCountMismatch { expected: signer_count, actual: self.signatures.len() });
        }
        if signer_count < ANCHOR_QUORUM {
            return Err(FinalityAnchorError::BelowQuorum(signer_count));
        }
        let msg_hash = self.notice.signing_hash();
        for (signature, trustee_index) in self.signatures.iter().zip(super::bitmap_signers(self.signer_bitmap)) {
            super::verify_one(&trustee_keys[trustee_index as usize], trustee_index, msg_hash, signature)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> (Vec<secp256k1::Keypair>, TrusteeKeys) {
        let keypairs: Vec<_> = (1..=TRUSTEE_COUNT as u8)
            .map(|seed| secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &[seed; 32]).unwrap())
            .collect();
        let keys: Vec<[u8; 32]> = keypairs.iter().map(|kp| kp.public_key().x_only_public_key().0.serialize()).collect();
        (keypairs, keys.try_into().unwrap())
    }

    fn notice() -> ReleaseNotice {
        ReleaseNotice { network: "testnet-10".into(), min_major: 2, min_release: 58, issued_at: 1_758_500_000 }
    }

    fn signed(signers: &[u8]) -> (SignedReleaseNotice, TrusteeKeys) {
        let (keypairs, keys) = keys();
        let notice = notice();
        let sigs = signers.iter().map(|&i| (i, notice.sign(&keypairs[i as usize]))).collect();
        (SignedReleaseNotice::assemble(notice, sigs).unwrap(), keys)
    }

    #[test]
    fn quorum_verifies_and_orders_itself() {
        let (n, keys) = signed(&[4, 0, 2]);
        assert_eq!(n.signer_bitmap, 0b10101);
        assert_eq!(n.verify(&keys), Ok(()));
    }

    #[test]
    fn two_signers_are_not_enough() {
        let (n, keys) = signed(&[0, 1]);
        assert_eq!(n.verify(&keys), Err(FinalityAnchorError::BelowQuorum(2)));
    }

    #[test]
    fn a_repeated_signer_counts_once() {
        let (n, keys) = signed(&[1, 1, 3]);
        assert_eq!(n.verify(&keys), Err(FinalityAnchorError::BelowQuorum(2)));
    }

    #[test]
    fn a_changed_statement_fails() {
        let (mut n, keys) = signed(&[0, 1, 2]);
        n.notice.min_release = 99;
        assert_eq!(n.verify(&keys), Err(FinalityAnchorError::BadSignature(0)));
        let (mut n, keys) = signed(&[0, 1, 2]);
        n.notice.network = "mainnet".into();
        assert!(n.verify(&keys).is_err());
    }

    #[test]
    fn a_stranger_cannot_sign() {
        let (keypairs, keys) = keys();
        let stranger = secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &[99; 32]).unwrap();
        let notice = notice();
        let sigs = vec![(0, notice.sign(&keypairs[0])), (1, notice.sign(&keypairs[1])), (2, notice.sign(&stranger))];
        let n = SignedReleaseNotice::assemble(notice, sigs).unwrap();
        assert_eq!(n.verify(&keys), Err(FinalityAnchorError::BadSignature(2)));
    }

    #[test]
    fn refuses_only_older_releases() {
        let n = notice();
        assert!(n.refuses(2, 57));
        assert!(n.refuses(1, 99));
        assert!(!n.refuses(2, 58));
        assert!(!n.refuses(2, 59));
        assert!(!n.refuses(3, 0));
    }
}
