//! # Release manifests
//!
//! A trustee-signed list of a wallet release's files and their digests, which is
//! what lets the wallet update itself (founder, 2026-10-08: "people just click OK on
//! 'there is a new version' and it installs"). Like a release notice it is a JSON
//! document on the project's website, verified against the pinned trustee keys and
//! never carried on chain. A quorum signs, so no single key can hand every wallet a
//! binary of its choosing; the download address is compiled into the wallet, so the
//! manifest can only ever say *what* a file must hash to, never where to fetch it.

use super::{ANCHOR_QUORUM, FinalityAnchorError, TRUSTEE_COUNT, TrusteeKeys};
use crate::Hash;
use borsh::{BorshDeserialize, BorshSerialize};
use kaspa_hashes::{HasherBase, ReleaseManifestSigningHash};

/// The signed statement: a release version (`major.release.build`) and, per asset
/// name as the release carries it, the SHA-256 of its bytes. Assets are kept sorted by
/// name so two signers serialise the same bytes.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ReleaseManifest {
    pub version: String,
    /// Unix seconds when the trustees signed.
    pub issued_at: u64,
    pub assets: Vec<(String, [u8; 32])>,
}

/// A manifest plus a quorum of trustee signatures, bitmap-indexed like an anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedReleaseManifest {
    pub manifest: ReleaseManifest,
    pub signer_bitmap: u8,
    pub signatures: Vec<[u8; 64]>,
}

impl ReleaseManifest {
    pub fn new(version: String, issued_at: u64, mut assets: Vec<(String, [u8; 32])>) -> Self {
        assets.sort_by(|a, b| a.0.cmp(&b.0));
        Self { version, issued_at, assets }
    }

    /// The message every signature covers: `H("ReleaseManifest" || borsh(self))`.
    pub fn signing_hash(&self) -> Hash {
        let mut hasher = ReleaseManifestSigningHash::new();
        hasher.update(borsh::to_vec(self).expect("borsh serialization of ReleaseManifest cannot fail"));
        hasher.finalize()
    }

    /// One trustee's signature over this manifest.
    pub fn sign(&self, keypair: &secp256k1::Keypair) -> [u8; 64] {
        let msg = secp256k1::Message::from_digest(self.signing_hash().into());
        *secp256k1::SECP256K1.sign_schnorr_no_aux_rand(&msg, keypair).as_ref()
    }

    /// The digest an asset must have, if the manifest lists it.
    pub fn digest_of(&self, asset: &str) -> Option<[u8; 32]> {
        self.assets.iter().find(|(name, _)| name == asset).map(|(_, digest)| *digest)
    }
}

impl SignedReleaseManifest {
    /// Assembles from individually collected `(trustee index, signature)` pairs;
    /// duplicates count once and out-of-range indices are refused.
    pub fn assemble(manifest: ReleaseManifest, mut signed: Vec<(u8, [u8; 64])>) -> Result<Self, FinalityAnchorError> {
        signed.sort_unstable_by_key(|(index, _)| *index);
        let mut signer_bitmap = 0u8;
        let mut signatures = Vec::with_capacity(signed.len());
        for (index, signature) in signed {
            if index as usize >= TRUSTEE_COUNT {
                return Err(FinalityAnchorError::TrusteeIndexOutOfRange(index));
            }
            if signer_bitmap & (1 << index) != 0 {
                continue;
            }
            signer_bitmap |= 1 << index;
            signatures.push(signature);
        }
        Ok(Self { manifest, signer_bitmap, signatures })
    }

    /// At least a quorum of distinct trustees, every signature verifying against its
    /// pinned key.
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
        let msg_hash = self.manifest.signing_hash();
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

    fn manifest() -> ReleaseManifest {
        ReleaseManifest::new(
            "2.79.322".into(),
            7,
            vec![("marigold-cli-windows-x86_64.exe".into(), [2u8; 32]), ("marigold-cli-linux-x86_64".into(), [1u8; 32])],
        )
    }

    #[test]
    fn assets_are_sorted_so_every_signer_hashes_the_same_bytes() {
        let m = manifest();
        assert_eq!(m.assets[0].0, "marigold-cli-linux-x86_64");
        assert_eq!(m.digest_of("marigold-cli-linux-x86_64"), Some([1u8; 32]));
        assert_eq!(m.digest_of("marigold-cli-macos-arm64"), None);
    }

    #[test]
    fn a_quorum_verifies_and_less_does_not() {
        let (keypairs, keys) = keys();
        let m = manifest();
        let signed =
            SignedReleaseManifest::assemble(m.clone(), [0u8, 2, 4].iter().map(|&i| (i, m.sign(&keypairs[i as usize]))).collect())
                .unwrap();
        assert_eq!(signed.verify(&keys), Ok(()));
        let two = SignedReleaseManifest::assemble(m.clone(), [0u8, 2].iter().map(|&i| (i, m.sign(&keypairs[i as usize]))).collect())
            .unwrap();
        assert!(matches!(two.verify(&keys), Err(FinalityAnchorError::BelowQuorum(2))));
    }

    #[test]
    fn a_changed_digest_breaks_every_signature() {
        let (keypairs, keys) = keys();
        let m = manifest();
        let sigs: Vec<_> = [0u8, 1, 2].iter().map(|&i| (i, m.sign(&keypairs[i as usize]))).collect();
        let mut tampered = m.clone();
        tampered.assets[0].1 = [9u8; 32];
        let signed = SignedReleaseManifest::assemble(tampered, sigs).unwrap();
        assert!(signed.verify(&keys).is_err());
    }

    #[test]
    fn a_stranger_cannot_sign() {
        let (keypairs, keys) = keys();
        let m = manifest();
        let stranger = secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &[99; 32]).unwrap();
        let signed = SignedReleaseManifest::assemble(
            m.clone(),
            vec![(0, m.sign(&keypairs[0])), (1, m.sign(&keypairs[1])), (2, m.sign(&stranger))],
        )
        .unwrap();
        assert!(matches!(signed.verify(&keys), Err(FinalityAnchorError::BadSignature(2))));
    }
}
