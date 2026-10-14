//! What the desktop app and the phone apps do around an open wallet, written once: finding the
//! wallets folder, listing and creating wallets, choosing the node, reading a pasted code,
//! restoring from backups, changing a password. Each returns plain values and a user-facing error
//! text; none of them prints. (docs/marigold/MOBILE-PLAN.md, PH-M3a: the phone's interface and
//! the desktop's commands call these, so there is no second implementation of an operation.)

use crate::{backup as archive, bundle, policy};
use kaspa_consensus_core::network::{NetworkId, NetworkType};
use kaspa_wallet_core::account::notepool::{
    BEARER_NOTE_PREFIX, HANDOVER_PREFIX, LOCKED_HANDOVER_PREFIX, PAYMENT_RECEIPT_PREFIX, PaymentRequest,
};
use kaspa_wallet_core::prelude::*;
use kaspa_wallet_core::utils::{sompi_to_kaspa_string, try_kaspa_str_to_sompi};
use kaspa_wallet_core::wallet::Wallet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// A wallet of the folder, as a list shows it.
#[derive(Debug, Clone)]
pub struct WalletEntry {
    pub filename: String,
    pub title: String,
}

/// A wallet just made.
#[derive(Debug, Clone)]
pub struct Created {
    pub filename: String,
    /// The 24 words, space-separated: the whole wallet comes back from them.
    pub words: String,
}

/// What a pasted code is, before anything is done with it.
#[derive(Debug, Clone)]
pub struct Classified {
    /// "request", "handover", "note", "receipt" or "unknown".
    pub kind: String,
    /// A pinned request's amount; empty when the payer chooses or it is no request.
    pub amount: String,
}

/// The testnet, until there is another network to choose.
pub fn default_network() -> NetworkId {
    NetworkId::with_suffix(NetworkType::Testnet, 10)
}

/// The wallets folder and the network. Named, they are used as given (a phone names its own
/// folder); not named, the wallet settings decide, as everywhere else.
pub async fn probe(folder: Option<&str>, network: Option<NetworkId>) -> Result<(Arc<Wallet>, String, NetworkId), String> {
    let wallet =
        Arc::new(Wallet::try_with_rpc(None, Wallet::local_store().map_err(|e| e.to_string())?, None).map_err(|e| e.to_string())?);
    let settings_needed = folder.is_none() || network.is_none();
    if settings_needed {
        wallet.load_settings().await.ok();
    }
    let folder = match folder {
        Some(folder) => folder.to_string(),
        None => wallet
            .settings()
            .get::<String>(WalletSettings::Folder)
            .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string()),
    };
    let network_id = match network {
        Some(id) => id,
        None => wallet
            .settings()
            .get::<String>(WalletSettings::Network)
            .and_then(|s| s.parse::<NetworkId>().ok())
            .unwrap_or_else(default_network),
    };
    wallet.store().set_storage_folder(&folder).map_err(|e| e.to_string())?;
    Ok((wallet, folder, network_id))
}

/// The wallets in the folder.
pub async fn list_wallets(folder: Option<&str>, network: Option<NetworkId>) -> Result<Vec<WalletEntry>, String> {
    let (wallet, _, _) = probe(folder, network).await?;
    let list = wallet.store().wallet_list().await.map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|d| WalletEntry { title: d.title.clone().unwrap_or_else(|| d.filename.clone()), filename: d.filename })
        .collect())
}

/// Makes a wallet the way the terminal wallet's wizard does: 24 words, a ledger account derived
/// from them, the wallet keyed by them. With `words`, the same wallet is rebuilt from a
/// written-down set (its notes still need the wallet's files, restored from a backup).
pub async fn create_wallet(
    folder: Option<&str>,
    network: Option<NetworkId>,
    name: &str,
    password: &str,
    words: Option<&str>,
    allow_empty: bool,
) -> Result<Created, String> {
    use kaspa_wallet_core::storage::keydata::PrvKeyDataVariantKind;
    use kaspa_wallet_core::storage::local::notevault::{account_mnemonic_from_vault_words, new_vault_words};
    let name = name.trim();
    let name = if name.is_empty() { "marigold" } else { name };
    match policy::password_verdict(password, allow_empty || policy::empty_password_allowed_by_env()) {
        policy::PasswordVerdict::EmptyRefused => {
            return Err(format!(
                "a password is needed — at least {} characters; a PIN will do. No password at all is the advanced box below.",
                policy::MIN_WALLET_PASSWORD_CHARS
            ));
        }
        policy::PasswordVerdict::TooShort => {
            return Err(format!("at least {} characters, please — a PIN will do", policy::MIN_WALLET_PASSWORD_CHARS));
        }
        _ => {}
    }
    let (wallet, folder, network_id) = probe(folder, network).await?;
    wallet.set_network_id(&network_id).map_err(|e| e.to_string())?;
    let bare = kaspa_wallet_core::storage::make_filename(&Some(name.to_string()), &None);
    if wallet.store().exists(Some(&bare)).await.unwrap_or(false) {
        return Err(format!("a wallet named '{bare}' already exists on this machine"));
    }
    let vault_words = match words {
        Some(given) => {
            let normalised: Vec<String> = given.split_whitespace().map(|w| w.to_lowercase()).collect();
            if normalised.len() != 24 {
                return Err(format!("that is {} words; a wallet has 24", normalised.len()));
            }
            let phrase = normalised.join(" ");
            kaspa_bip32::Mnemonic::new(phrase.clone(), kaspa_bip32::Language::English)
                .map_err(|_| "those are not 24 wallet words".to_string())?;
            phrase
        }
        None => new_vault_words().map_err(|e| e.to_string())?,
    };
    let secret = Secret::from(password.to_string());
    let account = account_mnemonic_from_vault_words(&vault_words).map_err(|e| e.to_string())?;
    let prv = PrvKeyDataCreateArgs::new(None, None, Secret::from(account.phrase_string()), PrvKeyDataVariantKind::Mnemonic);
    wallet.store().batch().await.map_err(|e| e.to_string())?;
    let args = WalletCreateArgs::new(Some(name.to_string()), None, EncryptionKind::XChaCha20Poly1305, None, true);
    let (descriptor, _) = wallet.create_wallet(&secret, args).await.map_err(|e| e.to_string())?;
    let key_id = wallet.create_prv_key_data(&secret, prv).await.map_err(|e| e.to_string())?;
    wallet.create_account_bip32(&secret, key_id, None, AccountCreateArgsBip32::new(None, None)).await.map_err(|e| e.to_string())?;
    wallet.store().flush(&secret).await.map_err(|e| e.to_string())?;
    let store = wallet.store().as_note_key_store().map_err(|e| e.to_string())?;
    store.vault_restore_from_words(&vault_words, &secret).await.map_err(|e| e.to_string())?;
    // The public half of the wallet's backup key pair, beside the wallet.
    if let Ok(vault_folder) = store.vault_folder().await
        && let Some(wallet_dir) = vault_folder.parent()
    {
        archive::write_recipient(wallet_dir, &vault_words).map_err(|e| e.to_string())?;
    }
    if let Ok(dir) = wallet_dir_of(&folder, &descriptor.filename) {
        restrict_to_owner(&dir);
    }
    Ok(Created { filename: descriptor.filename, words: vault_words })
}

/// Makes everything a wallet keeps in its folder the owner's alone: folders `0700`, files `0600`. The keys file is
/// encrypted and the rest is public or a lock, but nothing about a wallet is for anyone else to read, and a phone's
/// folder should say so as plainly as the desktop's. Run when a wallet is made and each time it is opened, so a wallet
/// made by an older program is brought in line. Where there are no such permissions (Windows) it does nothing.
pub fn restrict_to_owner(wallet_dir: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut stack = vec![wallet_dir.to_path_buf()];
        while let Some(path) = stack.pop() {
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            if meta.file_type().is_symlink() {
                continue;
            }
            let wanted = if meta.is_dir() { 0o700 } else { 0o600 };
            if meta.permissions().mode() & 0o777 != wanted
                && let Err(err) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(wanted))
            {
                log::warn!("could not restrict {}: {err}", path.display());
            }
            if meta.is_dir()
                && let Ok(entries) = std::fs::read_dir(&path)
            {
                stack.extend(entries.flatten().map(|e| e.path()));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = wallet_dir;
}

/// The folder a wallet of this file name keeps its files in, inside the wallets folder.
pub fn wallet_dir_of(folder: &str, filename: &str) -> Result<PathBuf, String> {
    let base = workflow_store::fs::resolve_path(folder).map_err(|e| e.to_string())?;
    Ok(base.join(kaspa_wallet_core::storage::local::wallet_dir_name(filename)))
}

/// The node an open screen's choice names: "public" for a public computer, "local" for a node on
/// this machine, "own" for a sync inside the program (`None`), or a ws:// address.
pub fn resolve_node(choice: &str, network_id: NetworkId) -> Result<Option<String>, String> {
    Ok(match choice {
        "own" => None,
        "local" => Some(format!("ws://127.0.0.1:{}", network_id.default_borsh_rpc_port())),
        "public" => Some(
            kaspa_wrpc_client::resolver::public_nodes(network_id)
                .into_iter()
                .next()
                .ok_or_else(|| "no public computer is known for this network".to_string())?,
        ),
        url => Some(url.to_string()),
    })
}

/// The node's own words tell a terminal user to type a command; a screen has a choice to point at
/// instead (founder, 2026-09-23).
pub fn open_error_text(text: String, choice: &str) -> String {
    if text.contains("already in use") {
        "Looks like there is already a Marigold network running on this computer. Select \"A network running on this computer\" to connect to it.".to_string()
    } else if choice == "local" && (text.contains("cannot reach") || text.contains("Connection refused") || text.contains("timed out"))
    {
        "No Marigold program is answering on this computer right now; the one that was running has stopped. Choose \"Sync here, inside the app\" or a public computer instead.".to_string()
    } else {
        text
    }
}

/// How the wallet reaches the network, in the words the open screen used.
pub fn access_words(choice: &str) -> &'static str {
    match choice {
        "own" => "your own sync",
        "local" => "a network on this computer",
        "public" => "a public computer",
        _ => "the node you named",
    }
}

/// The terminal paints its remarks; a window shows them plain.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// An amount typed by a person, in whole units, to petals; nothing typed is `None`.
pub fn optional_amount(text: &str) -> Result<Option<u64>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(amount(text)?))
}

/// An amount typed by a person, which must be more than nothing.
pub fn amount(text: &str) -> Result<u64, String> {
    try_kaspa_str_to_sompi(text.trim())
        .map_err(|e| e.to_string())?
        .filter(|p| *p > 0)
        .ok_or_else(|| "that is not an amount".to_string())
}

/// An amount of petals as the units a person reads.
pub fn amount_text(petals: u64) -> String {
    sompi_to_kaspa_string(petals)
}

/// What a pasted code is, before anything is done with it.
pub fn classify(code: &str) -> Result<Classified, String> {
    let code = code.trim();
    if let Ok(request) = PaymentRequest::from_text(code) {
        request.verify().map_err(|e| e.to_string())?;
        return Ok(Classified {
            kind: "request".into(),
            amount: request.amount_petals.map(sompi_to_kaspa_string).unwrap_or_default(),
        });
    }
    let kind = if code.starts_with(HANDOVER_PREFIX) || code.starts_with(LOCKED_HANDOVER_PREFIX) {
        "handover"
    } else if code.starts_with(BEARER_NOTE_PREFIX) {
        "note"
    } else if code.starts_with(PAYMENT_RECEIPT_PREFIX) {
        "receipt"
    } else {
        "unknown"
    };
    Ok(Classified { kind: kind.into(), amount: String::new() })
}

/// Opens the archives with the words and writes the wallet they hold into the folder.
fn restore_archives(
    archives: std::collections::BTreeMap<String, Vec<u8>>,
    words: &str,
    folder: &str,
    name: &str,
    rotate_advice: bool,
) -> Result<String, String> {
    use bundle::Unlock;
    let names = bundle::bundle_wallets(&archives).map_err(|e| e.to_string())?;
    let merged = if names.is_empty() {
        bundle::merge(&archives, Unlock::Legacy(&archive::key_from_words(words)))
    } else {
        let fits = bundle::wallets_for_words(&archives, words).map_err(|e| e.to_string())?;
        let Some(mine) = fits.first() else {
            return Err(format!(
                "those words open none of the wallets in this backup ({}) — each wallet opens with its own 24 words",
                names.join(", ")
            ));
        };
        let mut map = std::collections::BTreeMap::new();
        map.insert(mine.clone(), words.to_string());
        bundle::merge(&archives, Unlock::Words(&map))
    }
    .map_err(|e| e.to_string())?;
    let folder = workflow_store::fs::resolve_path(folder).map_err(|e| e.to_string())?;
    let new_name = if name.trim().is_empty() { None } else { Some(name.trim().to_string()) };
    let mut done = Vec::new();
    let mut restored_names = Vec::new();
    for (wallet, entries) in merged.wallets {
        let restored = archive::install_restored(entries, &folder, new_name.clone()).map_err(|e| e.to_string())?;
        restored_names.push(wallet);
        done.push(format!("Restored {} files as '{}'", restored.written, restored.name));
    }
    let others: Vec<String> = names.into_iter().filter(|n| !restored_names.contains(n)).collect();
    Ok(format!(
        "{} from the full copy of {} with {} change set(s) after it. Open it with the password it had when the backup was made.{}{}",
        done.join("; "),
        bundle::checkpoint_moment(&merged.stamp),
        merged.deltas,
        if rotate_advice {
            " If any other copy of this backup could exist, rotate its notes to fresh keys once ('note rotate all' in the terminal wallet)."
        } else {
            ""
        },
        if others.is_empty() {
            String::new()
        } else {
            format!(" The backup also holds {}: run this again with its own words.", bundle::wallets_phrase(&others))
        }
    ))
}

/// Restores from a folder of backup files, the way the Telegram restore does, with the words of
/// the wallet wanted.
pub async fn restore_from_folder(
    folder: Option<&str>,
    network: Option<NetworkId>,
    backups: &str,
    words: &str,
    name: &str,
) -> Result<String, String> {
    if !archive::looks_like_words(words) {
        return Err("that is not 24 words".to_string());
    }
    let dir = PathBuf::from(backups.trim());
    if !dir.is_dir() {
        return Err(format!("{} is not a folder", dir.display()));
    }
    let archives = bundle::collect_dir(&dir).map_err(|e| e.to_string())?;
    let (_, folder, _) = probe(folder, network).await?;
    restore_archives(archives, words, &folder, name, false)
}

#[cfg(not(target_arch = "wasm32"))]
/// Restores a wallet from the backups forwarded to its bot: the newest checkpoint and the deltas
/// after it. A backup holds every wallet of its computer, each sealed with its own 24 words; the
/// words given open one of them, and the answer names the others. Progress goes to `progress` as
/// the parts arrive.
pub async fn restore_from_telegram(
    folder: Option<&str>,
    network: Option<NetworkId>,
    token: &str,
    words: &str,
    name: &str,
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<String, String> {
    if !archive::looks_like_words(words) {
        return Err("that is not 24 words".to_string());
    }
    let token = token.trim().to_string();
    if !token.contains(':') || token.len() < 20 {
        return Err("that does not look like a bot token".to_string());
    }
    let (_, folder, _) = probe(folder, network).await?;
    let sets = crate::telegram::collect_backup_sets(&token, Duration::from_secs(600), progress).await?;
    let mut archives = std::collections::BTreeMap::new();
    for (backup, parts) in &sets {
        let mut bytes = Vec::new();
        for part in parts {
            bytes.extend(crate::telegram::download_file(&token, &part.file_id).await?);
        }
        progress(format!("Received {backup} ({} bytes).", bytes.len()));
        archives.insert(backup.clone(), bytes);
    }
    restore_archives(archives, words, &folder, name, true)
}

/// What is wrong with a new password under the policy, in words for the person; `None` if it is fine.
pub fn new_password_problem(password: &str, allow_empty: bool) -> Option<String> {
    match policy::password_verdict(password, allow_empty || policy::empty_password_allowed_by_env()) {
        policy::PasswordVerdict::EmptyRefused => {
            Some(format!("A password is needed — at least {} characters; a PIN will do.", policy::MIN_WALLET_PASSWORD_CHARS))
        }
        policy::PasswordVerdict::TooShort => {
            Some(format!("At least {} characters, please — a PIN will do.", policy::MIN_WALLET_PASSWORD_CHARS))
        }
        _ => None,
    }
}

/// Why a password change did not happen.
#[derive(Debug)]
pub enum PasswordChange {
    /// The old password is not this wallet's.
    WrongPassword,
    Failed(String),
}

impl std::fmt::Display for PasswordChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PasswordChange::WrongPassword => write!(f, "That is not this wallet's password; nothing was changed."),
            PasswordChange::Failed(text) => write!(f, "{text}"),
        }
    }
}

/// A new password for the open wallet; the policy was checked by the caller. The notes' key first,
/// since that write is atomic; then the keys file, and the notes' key goes back if the keys file
/// refuses. The backups are sealed with the words, never the password, so they are not touched.
pub async fn change_password(wallet: &Arc<Wallet>, old: &Secret, new: &Secret) -> Result<(), PasswordChange> {
    let store = wallet.store().clone();
    let notes = store.as_note_key_store().map_err(|e| PasswordChange::Failed(e.to_string()))?;
    if let Err(err) = notes.vault_change_secret(old, new).await {
        let text = err.to_string();
        if text.contains("decrypt") || text.contains("aead") || text.contains("Decrypt") {
            return Err(PasswordChange::WrongPassword);
        }
        return Err(PasswordChange::Failed(text));
    }
    if let Err(err) = store.change_secret(old, new).await {
        let _ = notes.vault_change_secret(new, old).await;
        return Err(PasswordChange::Failed(format!("the keys file could not be rewritten ({err}); nothing was changed")));
    }
    Ok(())
}
