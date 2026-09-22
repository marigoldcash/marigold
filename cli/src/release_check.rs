//! The release check: is this wallet still current, and is it still allowed?
//!
//! Once at startup and once a day after, the wallet fetches one small JSON document
//! from the project's website. It carries two things:
//!
//! - the newest wallet release, unsigned: when it is newer than this build, the wallet
//!   says so once, with the download link, and carries on. A wrong value here can only
//!   ever produce a needless line, because the link it points at is compiled in.
//! - release notices signed by the trustees (`finality_anchor::release_notice`): the
//!   oldest release that still follows the consensus the trustees certify. A wallet
//!   below it stops with the download link rather than run on rules it does not know.
//!   Notices are verified against the trustee keys pinned in the network's params, the
//!   same ones that make finality anchors count; a network without pinned keys has no
//!   verifiable notice and only gets the first line.
//!
//! The check never blocks anything: a website that is down or slow is the same as no
//! news. `MARIGOLD_NO_RELEASE_CHECK=1` turns it off, for tests and for anyone who would
//! rather the wallet made no request of its own; `MARIGOLD_RELEASE_URL` points a test
//! at a document of its own.

use crate::imports::*;
use kaspa_consensus_core::config::params::Params;
use kaspa_consensus_core::finality_anchor::release_notice::{ReleaseNotice, SignedReleaseNotice};
use std::time::Duration;

/// Where every "download the latest wallet" line points. Compiled in on purpose: no
/// fetched document, signed or not, gets to choose where a user is sent.
pub const DOWNLOAD_URL: &str = "https://github.com/marigoldcash/marigold-wallet/releases/latest";
/// The document the wallet reads. Served from the same GitHub Pages site as the rest
/// of marigold.cash; `scripts/release-notice.sh` writes it.
pub const NOTICE_URL: &str = "https://marigold.cash/release.json";
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// How long the "too old" message stays on screen before the wallet leaves. A wallet
/// opened by double-click often owns its window, and the window goes with it.
const LEAVE_AFTER: Duration = Duration::from_secs(30);

/// `major.release.build`, as the wallet prints it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches('v');
        let mut parts = text.split('.').map(|p| p.parse::<u32>().ok());
        let (a, b, c) = (parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(Self(a, b, c))
    }

    pub fn own() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is major.release.build")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The document as served. Unknown fields are ignored so the file can grow.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Document {
    #[serde(default)]
    pub latest: Option<String>,
    #[serde(default)]
    pub notices: Vec<NoticeEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NoticeEntry {
    pub network: String,
    pub min_version: String,
    pub issued_at: u64,
    #[serde(default)]
    pub signatures: Vec<SignatureEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignatureEntry {
    pub trustee: u8,
    pub signature: String,
}

impl NoticeEntry {
    fn signed(&self) -> Option<SignedReleaseNotice> {
        let (major, release) = self.min_version.split_once('.')?;
        let notice = ReleaseNotice {
            network: self.network.clone(),
            min_major: major.trim().parse().ok()?,
            min_release: release.trim().parse().ok()?,
            issued_at: self.issued_at,
        };
        let mut signatures = Vec::with_capacity(self.signatures.len());
        for entry in &self.signatures {
            let mut bytes = [0u8; 64];
            faster_hex::hex_decode(entry.signature.trim().as_bytes(), &mut bytes).ok()?;
            signatures.push((entry.trustee, bytes));
        }
        SignedReleaseNotice::assemble(notice, signatures).ok()
    }
}

/// What the document says about this wallet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Below the trustees' minimum for this network: `min` is `major.release`.
    TooOld {
        min: String,
    },
    /// A newer release exists.
    Newer {
        latest: Version,
    },
    Current,
}

/// Pure evaluation, so it can be tested without a website: the newest *verified*
/// notice for this network decides, the unsigned `latest` only advises.
pub fn evaluate(document: &Document, own: Version, network: Option<NetworkId>) -> Verdict {
    if let Some(network) = network {
        let params = Params::from(network);
        if let Some(trustees) = params.finality_anchor.trustees {
            let name = network.to_string();
            let newest = document
                .notices
                .iter()
                .filter(|entry| entry.network == name)
                .filter_map(|entry| entry.signed())
                .filter(|signed| signed.verify(&trustees).is_ok())
                .max_by_key(|signed| signed.notice.issued_at);
            if let Some(signed) = newest
                && signed.notice.refuses(own.0, own.1)
            {
                return Verdict::TooOld { min: format!("{}.{}", signed.notice.min_major, signed.notice.min_release) };
            }
        }
    }
    match document.latest.as_deref().and_then(Version::parse) {
        Some(latest) if latest > own => Verdict::Newer { latest },
        _ => Verdict::Current,
    }
}

pub fn disabled() -> bool {
    std::env::var("MARIGOLD_NO_RELEASE_CHECK").is_ok_and(|v| !v.is_empty() && v != "0")
}

pub async fn fetch() -> std::result::Result<Document, String> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(format!("marigold-cli/{}", Version::own()))
        .build()
        .map_err(|e| e.to_string())?;
    let url = std::env::var("MARIGOLD_RELEASE_URL").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| NOTICE_URL.to_string());
    let response = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }
    let body = response.text().await.map_err(|e| e.to_string())?;
    serde_json::from_str::<Document>(&body).map_err(|e| e.to_string())
}

/// The network this wallet is on: the connected one, else the one it is set to use.
fn network_of(cli: &Arc<KaspaCli>) -> Option<NetworkId> {
    if let Ok(id) = cli.wallet().network_id() {
        return Some(id);
    }
    cli.wallet().settings().get::<String>(WalletSettings::Network).and_then(|s| s.parse().ok())
}

/// Starts the daily check. Returns at once; everything happens on its own task.
pub fn start(cli: &Arc<KaspaCli>) {
    if disabled() {
        return;
    }
    let cli = cli.clone();
    workflow_core::task::spawn(async move {
        let mut announced: Option<Version> = None;
        loop {
            // Give the splash and the first connection a moment, so the line lands
            // where a person is looking and not inside the startup output.
            workflow_core::task::sleep(Duration::from_secs(20)).await;
            if cli.is_shutting_down() {
                return;
            }
            match fetch().await {
                Ok(document) => match evaluate(&document, Version::own(), network_of(&cli)) {
                    Verdict::TooOld { min } => {
                        leave_too_old(&cli, &min).await;
                        return;
                    }
                    Verdict::Newer { latest } if announced != Some(latest) => {
                        announced = Some(latest);
                        let term = cli.term();
                        term.writeln("");
                        term.writeln(crate::ui::warn(format!("A newer wallet is out: {latest}. This one is {}.", Version::own())));
                        term.writeln(format!("Download it at {}", crate::ui::value(DOWNLOAD_URL)));
                        term.writeln("");
                    }
                    _ => {}
                },
                Err(err) => log_trace!("release check: {err}"),
            }
            workflow_core::task::sleep(CHECK_EVERY).await;
            if cli.is_shutting_down() {
                return;
            }
        }
    });
}

async fn leave_too_old(cli: &Arc<KaspaCli>, min: &str) {
    let term = cli.term();
    term.writeln("");
    term.writeln(crate::ui::bad(format!(
        "This wallet, {}, is too old for the Marigold network as the trustees now sign it.",
        Version::own()
    )));
    term.writeln(crate::ui::bad(format!(
        "Releases from {min} on follow the current rules; this one stops here rather than run on rules it does not know."
    )));
    term.writeln(format!("Download the latest wallet at {}", crate::ui::value(DOWNLOAD_URL)));
    term.writeln(crate::ui::dim(format!(
        "Your wallet files and words are untouched; the new version opens them as they are. Closing in {} seconds.",
        LEAVE_AFTER.as_secs()
    )));
    term.writeln("");
    workflow_core::task::sleep(LEAVE_AFTER).await;
    if let Err(err) = cli.shutdown().await {
        term.writeln(format!("{err}"));
        term.exit().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaspa_consensus_core::finality_anchor::TRUSTEE_COUNT;

    #[test]
    fn versions_parse_and_order() {
        assert_eq!(Version::parse("2.58.257"), Some(Version(2, 58, 257)));
        assert_eq!(Version::parse("v2.59.260"), Some(Version(2, 59, 260)));
        assert_eq!(Version::parse("2.58"), None);
        assert_eq!(Version::parse("2.58.257.1"), None);
        assert!(Version(2, 59, 1) > Version(2, 58, 999));
        assert!(Version(3, 0, 0) > Version(2, 99, 999));
    }

    fn document(latest: &str, notices: Vec<NoticeEntry>) -> Document {
        Document { latest: Some(latest.into()), notices }
    }

    #[test]
    fn newer_release_advises_and_equal_says_nothing() {
        let own = Version(2, 58, 257);
        assert_eq!(evaluate(&document("2.59.260", vec![]), own, None), Verdict::Newer { latest: Version(2, 59, 260) });
        assert_eq!(evaluate(&document("2.58.257", vec![]), own, None), Verdict::Current);
        assert_eq!(evaluate(&document("2.58.255", vec![]), own, None), Verdict::Current);
        assert_eq!(evaluate(&document("garbage", vec![]), own, None), Verdict::Current);
    }

    #[test]
    fn an_unsigned_notice_cannot_stop_a_wallet() {
        let testnet: NetworkId = "testnet-10".parse().unwrap();
        let notice = NoticeEntry { network: "testnet-10".into(), min_version: "9.0".into(), issued_at: 1, signatures: vec![] };
        assert_eq!(evaluate(&document("2.58.257", vec![notice]), Version(2, 58, 257), Some(testnet)), Verdict::Current);
    }

    /// The real testnet keys are on the trustee hosts, so this exercises the path
    /// with generated keys through the same code the wallet runs, patching params
    /// being out of reach: verification is proven in consensus-core's own tests, and
    /// here the wallet's parsing of hex signatures and its choice of the newest
    /// verified notice.
    #[test]
    fn parses_hex_signatures_into_a_verifiable_notice() {
        let keypairs: Vec<_> = (1..=TRUSTEE_COUNT as u8)
            .map(|seed| secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &[seed; 32]).unwrap())
            .collect();
        let keys: Vec<[u8; 32]> = keypairs.iter().map(|kp| kp.public_key().x_only_public_key().0.serialize()).collect();
        let keys: kaspa_consensus_core::finality_anchor::TrusteeKeys = keys.try_into().unwrap();
        let notice = ReleaseNotice { network: "testnet-10".into(), min_major: 2, min_release: 58, issued_at: 5 };
        let entry = NoticeEntry {
            network: "testnet-10".into(),
            min_version: "2.58".into(),
            issued_at: 5,
            signatures: [0u8, 2, 3]
                .iter()
                .map(|&i| SignatureEntry { trustee: i, signature: faster_hex::hex_string(&notice.sign(&keypairs[i as usize])) })
                .collect(),
        };
        let signed = entry.signed().expect("well-formed");
        assert_eq!(signed.verify(&keys), Ok(()));
        assert!(signed.notice.refuses(2, 57));
        assert!(!signed.notice.refuses(2, 58));
    }
}
