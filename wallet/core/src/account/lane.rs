//!
//! The lane registry (docs/marigold/LANE-REGISTRY.md): a company claims a
//! tag of up to five letters — its own user-lane subnetwork, where its anchoring
//! transactions live — with one transaction that pays the registration fee
//! to the registry address and carries the claim in its payload. The chain
//! is the registry: the first valid claim of a tag holds it, and anyone
//! with an archival node can list the claims by filtering the registry lane.
//!

use crate::imports::*;
use crate::tx::{Fees, Generator, GeneratorSettings, PaymentDestination, PaymentOutputs, Signer};
use kaspa_addresses::{Address, Prefix};
use kaspa_consensus_core::config::params::Params;
use kaspa_consensus_core::network::NetworkType;
use kaspa_consensus_core::subnets::{SUBNETWORK_ID_LANE_REGISTRY, SUBNETWORK_NAMESPACE_LEN, SUBNETWORK_TAG_LEN, SubnetworkId};
use kaspa_hashes::Hash;
use secp256k1::{Keypair, Message, SECP256K1, SecretKey};

/// What a claim costs, in petals: 100 MAGLD (founder, 2026-09-21).
pub const LANE_REGISTRATION_FEE_PETALS: u64 = 100 * 100_000_000;

/// Payload version of a claim: 2 carries a five-byte tag; 1, the first
/// testnet claims, a four-byte one.
pub const LANE_CLAIM_VERSION: u8 = 2;

/// The longest label a claim may carry, in bytes.
pub const LANE_LABEL_MAX: usize = 64;

/// Tags that cannot be claimed: the lanes the network itself uses and the
/// registry's own. Nobody else's is reserved: every company, the first
/// integration partner included, claims its lane the same way (founder,
/// 2026-09-22).
pub const RESERVED_TAGS: [&str; 3] = ["POOL", "ANCR", "LANE"];

/// Where the registration fee goes, per network. Testnet: an address of the
/// project's, so claims can be made today; mainnet: set at the parameter
/// freeze (PLAN P9.5).
pub fn registry_address(network: NetworkType) -> Result<Address> {
    match network {
        NetworkType::Testnet => Address::try_from("marigoldtest:qqfc69eulu3v8qamcasxqcx3wxvsfzjkv73fau4exk5kem70nqjsqgm3wm9g8")
            .map_err(|e| Error::Custom(format!("registry address: {e}"))),
        NetworkType::Mainnet => Err(Error::Custom("the mainnet registry address is not set yet".to_string())),
        other => {
            // Development networks: any address of the right prefix will do,
            // the claim is only ever looked at by its own tools.
            let prefix = Prefix::from(other);
            Ok(Address::new(prefix, kaspa_addresses::Version::PubKey, &[0u8; 32]))
        }
    }
}

/// A claim on a lane: the tag (one to five letters or digits, zero-padded),
/// the company's key, a label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneClaim {
    pub tag: [u8; SUBNETWORK_TAG_LEN],
    pub pk: [u8; 32],
    pub label: String,
}

impl LaneClaim {
    /// A tag is one to five ASCII capitals or digits, like a ticker symbol,
    /// and not reserved. Zero-padded to five bytes.
    pub fn parse_tag(text: &str) -> Result<[u8; SUBNETWORK_TAG_LEN]> {
        let upper = text.trim().to_ascii_uppercase();
        if upper.is_empty() || upper.len() > SUBNETWORK_TAG_LEN || !upper.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return Err(Error::Custom(format!("a lane tag is one to five letters or digits, like ACME — '{text}' is not")));
        }
        if RESERVED_TAGS.contains(&upper.as_str()) {
            return Err(Error::Custom(format!("'{upper}' is taken")));
        }
        let mut tag = [0u8; SUBNETWORK_TAG_LEN];
        tag[..upper.len()].copy_from_slice(upper.as_bytes());
        Ok(tag)
    }

    /// Whether a tag needs wide lanes: five letters do, fewer do not, since
    /// a shorter tag padded with zeros is a lane under the old rule as well.
    pub fn needs_wide_lanes(tag: &[u8; SUBNETWORK_TAG_LEN]) -> bool {
        tag[SUBNETWORK_NAMESPACE_LEN] != 0
    }

    /// The lane this claim is for.
    pub fn lane(&self) -> SubnetworkId {
        SubnetworkId::from_tag(self.tag_text().as_bytes()).expect("a parsed tag is a lane")
    }

    pub fn new(tag: [u8; SUBNETWORK_TAG_LEN], pk: [u8; 32], label: &str) -> Result<Self> {
        let label = label.trim().to_string();
        if label.len() > LANE_LABEL_MAX {
            return Err(Error::Custom(format!("a label is at most {LANE_LABEL_MAX} bytes")));
        }
        if label.contains(|c: char| c.is_control()) {
            return Err(Error::Custom("a label is plain text".to_string()));
        }
        Ok(Self { tag, pk, label })
    }

    pub fn tag_text(&self) -> String {
        let end = self.tag.iter().rposition(|b| *b != 0).map(|i| i + 1).unwrap_or(0);
        String::from_utf8_lossy(&self.tag[..end]).to_string()
    }

    /// `version (1) ‖ tag (5) ‖ pk (32) ‖ label length (1) ‖ label`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(39 + self.label.len());
        out.push(LANE_CLAIM_VERSION);
        out.extend_from_slice(&self.tag);
        out.extend_from_slice(&self.pk);
        out.push(self.label.len() as u8);
        out.extend_from_slice(self.label.as_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        // Version 1 carried a four-byte tag; version 2 a five-byte one.
        let tag_len = match bytes.first() {
            Some(1) => SUBNETWORK_NAMESPACE_LEN,
            Some(2) => SUBNETWORK_TAG_LEN,
            _ => return Err(Error::Custom("not a lane claim".to_string())),
        };
        let head = 1 + tag_len + 32 + 1;
        if bytes.len() < head {
            return Err(Error::Custom("not a lane claim".to_string()));
        }
        let mut tag = [0u8; SUBNETWORK_TAG_LEN];
        tag[..tag_len].copy_from_slice(&bytes[1..1 + tag_len]);
        let pk: [u8; 32] = bytes[1 + tag_len..1 + tag_len + 32].try_into().unwrap();
        let len = bytes[head - 1] as usize;
        if bytes.len() != head + len || len > LANE_LABEL_MAX {
            return Err(Error::Custom("a lane claim's label does not match its length".to_string()));
        }
        let label = std::str::from_utf8(&bytes[head..]).map_err(|_| Error::Custom("a lane claim's label is not text".to_string()))?;
        let used = tag.iter().rposition(|b| *b != 0).map(|i| i + 1).unwrap_or(0);
        if used == 0
            || !tag[..used].iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            || tag[used..].iter().any(|b| *b != 0)
        {
            return Err(Error::Custom("a lane claim's tag is not one to five letters or digits".to_string()));
        }
        Ok(Self { tag, pk, label: label.to_string() })
    }
}

/// Make the claim: one transaction in the registry lane paying the fee to
/// the registry address, the claim in its payload. Returns the transaction
/// id, which is the claim's own reference.
pub async fn claim_lane(
    account: Arc<dyn Account>,
    wallet_secret: Secret,
    payment_secret: Option<Secret>,
    claim: &LaneClaim,
) -> Result<Hash> {
    let network_id = account.wallet().network_id()?;
    let network = network_id.network_type();
    let to = registry_address(network)?;
    if LaneClaim::needs_wide_lanes(&claim.tag) {
        let activation = Params::from(network_id).wide_lanes_activation;
        let now = account.wallet().rpc_api().get_server_info().await?.virtual_daa_score;
        if !activation.is_active(now) {
            return Err(Error::Custom(format!(
                "five-letter lanes open on this network at DAA score {} — it is {now} now; a tag of up to four letters can be claimed today",
                activation.daa_score()
            )));
        }
    }
    let keydata = account.prv_key_data(wallet_secret).await?;
    let signer = Arc::new(Signer::new(account.clone(), keydata, payment_secret));
    let settings = GeneratorSettings::try_new_with_account(
        account.clone(),
        PaymentDestination::PaymentOutputs(PaymentOutputs::from((to, LANE_REGISTRATION_FEE_PETALS))),
        None,
        Fees::SenderPays(0),
        Some(claim.encode()),
    )?
    .with_subnetwork_id(SUBNETWORK_ID_LANE_REGISTRY);
    let generator = Generator::try_new(settings, Some(signer), None)?;
    let mut stream = generator.stream();
    let mut last = None;
    while let Some(transaction) = stream.try_next().await? {
        transaction.try_sign()?;
        last = Some(transaction.try_submit(&account.wallet().rpc_api()).await?);
    }
    last.ok_or_else(|| Error::Custom("the claim produced no transaction".to_string()))
}

/// Payload version of a signed anchor. Version 1 (33 bytes, the root alone)
/// was the gateway contract's first draft; it carries no proof of who wrote
/// it, and a lane is not exclusive at the chain level — anyone can put a
/// transaction in any subnetwork — so from version 2 an anchor is signed by
/// the lane's registered key, and a verifier checks that before the root.
pub const ANCHOR_VERSION: u8 = 2;

/// What a lane's transaction carries: a 32-byte root and the lane key's
/// signature over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnchorPayload {
    pub root: [u8; 32],
    pub signature: [u8; 64],
}

impl AnchorPayload {
    /// The bytes the lane key signs: a tagged hash of the lane and the root,
    /// so a signature is good for one root in one lane and nothing else.
    fn message(lane: &SubnetworkId, root: &[u8; 32]) -> Message {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"marigold-anchor-v2");
        hasher.update(lane.as_bytes());
        hasher.update(root);
        let digest: [u8; 32] = hasher.finalize().into();
        Message::from_digest(digest)
    }

    pub fn sign(lane_sk: &[u8; 32], lane: &SubnetworkId, root: [u8; 32]) -> Result<Self> {
        let secret_key = SecretKey::from_slice(lane_sk).map_err(|e| Error::Custom(format!("lane key: {e}")))?;
        let keypair = Keypair::from_secret_key(SECP256K1, &secret_key);
        let signature = *keypair.sign_schnorr(Self::message(lane, &root)).as_ref();
        Ok(Self { root, signature })
    }

    /// Whether this anchor, found in `lane`, was signed by the key registered
    /// for that lane.
    pub fn verify(&self, lane: &SubnetworkId, lane_pk: &[u8; 32]) -> Result<()> {
        let pk = secp256k1::XOnlyPublicKey::from_slice(lane_pk)
            .map_err(|_| Error::Custom("the lane's key is not a valid key".to_string()))?;
        let signature = secp256k1::schnorr::Signature::from_slice(&self.signature)
            .map_err(|_| Error::Custom("the anchor's signature is malformed".to_string()))?;
        SECP256K1.verify_schnorr(&signature, &Self::message(lane, &self.root), &pk).map_err(|_| {
            Error::Custom("this anchor was not signed by the lane's registered key — it is not the company's".to_string())
        })
    }

    /// `version (1) ‖ root (32) ‖ signature (64)`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(97);
        out.push(ANCHOR_VERSION);
        out.extend_from_slice(&self.root);
        out.extend_from_slice(&self.signature);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 97 || bytes[0] != ANCHOR_VERSION {
            return Err(Error::Custom(
                "not a signed anchor (version 2 is 97 bytes; a 33-byte version-1 anchor carries no signature and proves nothing)"
                    .to_string(),
            ));
        }
        Ok(Self { root: bytes[1..33].try_into().unwrap(), signature: bytes[33..].try_into().unwrap() })
    }
}

/// The label under which a lane's key is kept among the wallet's share keys:
/// derived from the vault key like every share key, so it is recoverable from
/// the vault words, and never leaves the wallet.
pub fn lane_key_label(tag: &str) -> String {
    format!("lane:{}", tag.to_ascii_uppercase())
}

/// The wallet's key for `tag`, if it has made one.
pub async fn lane_key(wallet: &Arc<Wallet>, tag: &str) -> Result<Option<crate::storage::notekeys::ShareKeyInfo>> {
    let label = lane_key_label(tag);
    Ok(wallet.store().as_note_key_store()?.share_keys().await?.into_iter().find(|s| s.label == label))
}

/// A key for `tag`, made now if the wallet has none.
pub async fn lane_key_or_new(
    wallet: &Arc<Wallet>,
    wallet_secret: &Secret,
    tag: &str,
) -> Result<crate::storage::notekeys::ShareKeyInfo> {
    if let Some(existing) = lane_key(wallet, tag).await? {
        return Ok(existing);
    }
    wallet.store().as_note_key_store()?.add_share_key(wallet_secret, &lane_key_label(tag)).await
}

/// What an anchoring transaction pays itself, so it has an output: a tenth.
pub const ANCHOR_SELF_PAYMENT_PETALS: u64 = 10_000_000;

/// Anchor `root` in the lane of `tag`, signed with the wallet's key for that
/// lane: one transaction in the lane's subnetwork, paying a tenth to this
/// wallet's own address plus the network fee. Returns the transaction id,
/// which is what a record holder is given alongside their proof.
pub async fn anchor(
    account: Arc<dyn Account>,
    wallet_secret: Secret,
    payment_secret: Option<Secret>,
    tag: &str,
    root: [u8; 32],
) -> Result<Hash> {
    let parsed = LaneClaim::parse_tag(tag)?;
    let lane = SubnetworkId::from_tag(LaneClaim { tag: parsed, pk: [0; 32], label: String::new() }.tag_text().as_bytes())
        .ok_or_else(|| Error::Custom("not a lane".to_string()))?;
    let wallet = account.wallet();
    let key = lane_key(wallet, tag)
        .await?
        .ok_or_else(|| Error::Custom(format!("this wallet has no key for lane {tag}: 'lane key {tag}' makes one")))?;
    let sk = wallet.store().as_note_key_store()?.share_secret(&wallet_secret, key.index).await?;
    let payload = AnchorPayload::sign(&sk, &lane, root)?;
    let to = account.receive_address()?;
    let keydata = account.prv_key_data(wallet_secret).await?;
    let signer = Arc::new(Signer::new(account.clone(), keydata, payment_secret));
    let settings = GeneratorSettings::try_new_with_account(
        account.clone(),
        PaymentDestination::PaymentOutputs(PaymentOutputs::from((to, ANCHOR_SELF_PAYMENT_PETALS))),
        None,
        Fees::SenderPays(0),
        Some(payload.encode()),
    )?
    .with_subnetwork_id(lane);
    let generator = Generator::try_new(settings, Some(signer), None)?;
    let mut stream = generator.stream();
    let mut last = None;
    while let Some(transaction) = stream.try_next().await? {
        transaction.try_sign()?;
        last = Some(transaction.try_submit(&account.wallet().rpc_api()).await?);
    }
    last.ok_or_else(|| Error::Custom("the anchor produced no transaction".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_round_trips_and_a_bad_tag_is_refused() {
        let tag = LaneClaim::parse_tag("acme").unwrap();
        assert_eq!(&tag, b"ACME\0");
        assert!(!LaneClaim::needs_wide_lanes(&tag));
        let wide = LaneClaim::parse_tag("acmex").unwrap();
        assert!(LaneClaim::needs_wide_lanes(&wide));
        assert_eq!(LaneClaim::new(wide, [1u8; 32], "").unwrap().tag_text(), "ACMEX");
        assert!(LaneClaim::parse_tag("acmexy").is_err(), "six is one too many");
        assert_eq!(LaneClaim::parse_tag("a").unwrap(), *b"A\0\0\0\0");
        let claim = LaneClaim::new(tag, [7u8; 32], "Acme Ltd").unwrap();
        assert_eq!(claim.lane(), kaspa_consensus_core::subnets::SubnetworkId::from_namespace(*b"ACME"));
        let bytes = claim.encode();
        assert_eq!(bytes.len(), 39 + 8);
        // A version-1 claim, four-byte tag, still decodes.
        let mut v1 = vec![1u8];
        v1.extend_from_slice(b"ACME");
        v1.extend_from_slice(&[7u8; 32]);
        v1.push(8);
        v1.extend_from_slice(b"Acme Ltd");
        assert_eq!(LaneClaim::decode(&v1).unwrap(), claim);
        assert_eq!(LaneClaim::decode(&bytes).unwrap(), claim);
        assert!(LaneClaim::decode(&bytes[..40]).is_err(), "a truncated claim is refused");
        assert!(LaneClaim::parse_tag("pool").is_err(), "the pool's lane is not for claiming");
        assert!(LaneClaim::parse_tag("t360").is_ok(), "nobody's lane is reserved in advance");
        assert!(LaneClaim::parse_tag("ab-1").is_err());
        assert!(LaneClaim::parse_tag("").is_err());
        assert!(LaneClaim::new(tag, [0u8; 32], &"x".repeat(65)).is_err());
    }

    /// A signed anchor round-trips, verifies under its lane and key, and
    /// fails under another lane, another key, or a changed root.
    #[test]
    fn an_anchor_is_good_for_one_root_in_one_lane() {
        let sk = [9u8; 32];
        let pk = Keypair::from_secret_key(SECP256K1, &SecretKey::from_slice(&sk).unwrap()).x_only_public_key().0.serialize();
        let lane = SubnetworkId::from_tag(b"ACME").unwrap();
        let root = [0xabu8; 32];
        let anchor = AnchorPayload::sign(&sk, &lane, root).unwrap();
        let bytes = anchor.encode();
        assert_eq!(bytes.len(), 97);
        let back = AnchorPayload::decode(&bytes).unwrap();
        assert_eq!(back, anchor);
        back.verify(&lane, &pk).unwrap();
        assert!(back.verify(&SubnetworkId::from_tag(b"ACMEX").unwrap(), &pk).is_err(), "another lane");
        assert!(back.verify(&lane, &[7u8; 32]).is_err(), "another key");
        let mut other = back;
        other.root[0] ^= 1;
        assert!(other.verify(&lane, &pk).is_err(), "another root");
        let mut v1 = vec![1u8];
        v1.extend_from_slice(&root);
        assert!(AnchorPayload::decode(&v1).is_err(), "an unsigned version-1 anchor is not accepted");
    }
}
