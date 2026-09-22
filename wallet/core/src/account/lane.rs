//!
//! The lane registry (docs/marigold/LANE-REGISTRY.md): a company claims a
//! four-byte namespace — its own user-lane subnetwork, where its anchoring
//! transactions live — with one transaction that pays the registration fee
//! to the registry address and carries the claim in its payload. The chain
//! is the registry: the first valid claim of a tag holds it, and anyone
//! with an archival node can list the claims by filtering the registry lane.
//!

use crate::imports::*;
use crate::tx::{Fees, Generator, GeneratorSettings, PaymentDestination, PaymentOutputs, Signer};
use kaspa_addresses::{Address, Prefix};
use kaspa_consensus_core::network::NetworkType;
use kaspa_consensus_core::subnets::SUBNETWORK_ID_LANE_REGISTRY;
use kaspa_hashes::Hash;

/// What a claim costs, in petals: 100 MAGLD (founder, 2026-09-21).
pub const LANE_REGISTRATION_FEE_PETALS: u64 = 100 * 100_000_000;

/// Payload version of a claim.
pub const LANE_CLAIM_VERSION: u8 = 1;

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

/// A claim on a lane: the four-byte tag, the company's key, a label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneClaim {
    pub tag: [u8; 4],
    pub pk: [u8; 32],
    pub label: String,
}

impl LaneClaim {
    /// A tag is exactly four ASCII capitals or digits and not reserved.
    pub fn parse_tag(text: &str) -> Result<[u8; 4]> {
        let upper = text.trim().to_ascii_uppercase();
        if upper.len() != 4 || !upper.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
            return Err(Error::Custom(format!("a lane tag is four letters or digits, like ACME — '{text}' is not")));
        }
        if RESERVED_TAGS.contains(&upper.as_str()) {
            return Err(Error::Custom(format!("'{upper}' is taken")));
        }
        Ok(upper.as_bytes().try_into().expect("four bytes"))
    }

    pub fn new(tag: [u8; 4], pk: [u8; 32], label: &str) -> Result<Self> {
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
        String::from_utf8_lossy(&self.tag).to_string()
    }

    /// `version (1) ‖ tag (4) ‖ pk (32) ‖ label length (1) ‖ label`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(38 + self.label.len());
        out.push(LANE_CLAIM_VERSION);
        out.extend_from_slice(&self.tag);
        out.extend_from_slice(&self.pk);
        out.push(self.label.len() as u8);
        out.extend_from_slice(self.label.as_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 38 || bytes[0] != LANE_CLAIM_VERSION {
            return Err(Error::Custom("not a lane claim".to_string()));
        }
        let tag: [u8; 4] = bytes[1..5].try_into().unwrap();
        let pk: [u8; 32] = bytes[5..37].try_into().unwrap();
        let len = bytes[37] as usize;
        if bytes.len() != 38 + len || len > LANE_LABEL_MAX {
            return Err(Error::Custom("a lane claim's label does not match its length".to_string()));
        }
        let label = std::str::from_utf8(&bytes[38..]).map_err(|_| Error::Custom("a lane claim's label is not text".to_string()))?;
        if !tag.iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
            return Err(Error::Custom("a lane claim's tag is not four letters or digits".to_string()));
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
    let network = account.wallet().network_id()?.network_type();
    let to = registry_address(network)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_round_trips_and_a_bad_tag_is_refused() {
        let tag = LaneClaim::parse_tag("acme").unwrap();
        assert_eq!(&tag, b"ACME");
        let claim = LaneClaim::new(tag, [7u8; 32], "Acme Ltd").unwrap();
        let bytes = claim.encode();
        assert_eq!(bytes.len(), 38 + 8);
        assert_eq!(LaneClaim::decode(&bytes).unwrap(), claim);
        assert!(LaneClaim::decode(&bytes[..40]).is_err(), "a truncated claim is refused");
        assert!(LaneClaim::parse_tag("pool").is_err(), "the pool's lane is not for claiming");
        assert!(LaneClaim::parse_tag("t360").is_ok(), "nobody's lane is reserved in advance");
        assert!(LaneClaim::parse_tag("ab").is_err());
        assert!(LaneClaim::parse_tag("ab-1").is_err());
        assert!(LaneClaim::new(tag, [0u8; 32], &"x".repeat(65)).is_err());
    }
}
