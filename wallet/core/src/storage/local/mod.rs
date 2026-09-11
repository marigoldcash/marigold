//! Local storage implementation for the wallet SDK.
//! This module provides a local storage implementation
//! that functions uniformly in native and JS environments.
//! In native and NodeJS environments, this subsystem
//! will use the native file system IO. In the browser
//! environment, if called from the web page context
//! this will use `localStorage` and if invoked in the
//! chromium extension context it will use the
//! `chrome.storage.local` API. The implementation
//! is backed by the [`workflow_store`](https://docs.rs/workflow-store/)
//! crate.

pub mod cache;
pub mod collection;
pub mod interface;
pub mod notevault;
pub mod payload;
pub mod storage;
pub mod streams;
pub mod transaction;
pub mod wallet;

pub use collection::Collection;
pub use payload::Payload;
pub use storage::Storage;
pub use wallet::{ClientMetadata, WalletStorage};

/// A wallet is one directory. `<name>.wallet/` holds everything that wallet
/// owns:
///
/// ```text
/// XYZ.wallet/
///     XYZ.keys        the account keys, encrypted under the wallet password
///     notes/          the note vault — the money
///     transactions/   history, not needed to restore anything
///     .lock           held while the wallet is open
/// ```
///
/// It used to be a file and two sibling directories, which meant three things
/// to copy and two ways to copy them wrong. One directory is one thing to
/// move to a USB stick, sync to a cloud folder, or hand to `wallet backup`.
/// The directory is created with the wallet, before there are any notes in it.
pub fn wallet_dir_name(name: &str) -> String {
    format!("{name}.wallet")
}

/// The keys file inside that directory. Named `.keys` rather than `.wallet`
/// so that nothing is called `XYZ.wallet/XYZ.wallet`.
pub fn keys_file_name(name: &str) -> String {
    format!("{name}.keys")
}

/// Where the keys live, relative to the storage folder. Returns a *path*, not
/// a file name, so every `folder.join(wallet_file_name(name))` in the codebase
/// keeps resolving correctly.
pub fn wallet_file_name(name: &str) -> String {
    format!("{}/{}", wallet_dir_name(name), keys_file_name(name))
}

/// The vault directory for a wallet, relative to the storage folder.
pub fn notes_dir_name(name: &str) -> String {
    format!("{}/notes", wallet_dir_name(name))
}

/// The transaction history directory, relative to the storage folder.
pub fn transactions_dir_name(name: &str) -> String {
    format!("{}/transactions", wallet_dir_name(name))
}

/// Move any wallet still in the old layout into its own directory.
///
/// Every step is a rename within one filesystem, so nothing is copied and a
/// transactions folder of several million files moves instantly.
///
/// The new directory is assembled under a temporary name and put in place
/// last. If this is interrupted, `<name>.wallet.migrating` is left behind with
/// the parts already moved into it, and the next run finishes the job — rather
/// than leaving a wallet half in one layout and half in the other.
#[cfg(not(target_arch = "wasm32"))]
pub fn migrate_wallet_layout(folder: &std::path::Path) -> Vec<String> {
    use std::fs;

    let mut migrated = Vec::new();
    if !folder.exists() {
        return migrated;
    }

    // Names to consider: anything with an interrupted migration, plus any
    // `<name>.wallet` that is still a plain file.
    let mut names: Vec<String> = Vec::new();
    let Ok(entries) = fs::read_dir(folder) else { return migrated };
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if let Some(name) = file_name.strip_suffix(".wallet.migrating") {
            names.push(name.to_string());
        } else if let Some(name) = file_name.strip_suffix(".wallet")
            && entry.path().is_file()
        {
            names.push(name.to_string());
        }
    }
    names.sort();
    names.dedup();

    for name in names {
        let staging = folder.join(format!("{name}.wallet.migrating"));
        let old_keys = folder.join(format!("{name}.wallet"));
        let old_notes = folder.join(format!("{name}.notes"));
        let old_transactions = folder.join(format!("{name}.transactions"));

        if fs::create_dir_all(&staging).is_err() {
            continue;
        }
        // The keys file first: once it has moved, `<name>.wallet` is free for
        // the directory to take, and a second run sees the staging folder
        // rather than a file and resumes.
        if old_keys.is_file() && fs::rename(&old_keys, staging.join(keys_file_name(&name))).is_err() {
            continue;
        }
        if old_notes.is_dir() {
            let _ = fs::rename(&old_notes, staging.join("notes"));
        }
        if old_transactions.is_dir() {
            let _ = fs::rename(&old_transactions, staging.join("transactions"));
        }
        // Leftovers from the old layout that nothing reads any more.
        let _ = fs::remove_file(folder.join(format!("{name}.wallet.lock")));

        let destination = folder.join(wallet_dir_name(&name));
        if destination.exists() {
            // Only possible if a directory-layout wallet of the same name was
            // created alongside the old one. Leave the staging folder for a
            // person to look at rather than merging two wallets blindly.
            continue;
        }
        if fs::rename(&staging, &destination).is_ok() {
            migrated.push(name);
        }
    }

    migrated
}

/// No-op on wasm, which has no filesystem to migrate.
#[cfg(target_arch = "wasm32")]
pub fn migrate_wallet_layout(_folder: &std::path::Path) -> Vec<String> {
    Vec::new()
}

use crate::error::Error;
use crate::result::Result;
use wasm_bindgen::prelude::*;
use workflow_store::fs::create_dir_all_sync;

static mut DEFAULT_STORAGE_FOLDER: Option<String> = None;
static mut DEFAULT_WALLET_FILE: Option<String> = None;
static mut DEFAULT_SETTINGS_FILE: Option<String> = None;

pub fn default_storage_folder() -> &'static str {
    // SAFETY: This operation is initializing a static mut variable,
    // however, the actual variable is accessible only through
    // this function.
    #[allow(static_mut_refs)]
    unsafe {
        DEFAULT_STORAGE_FOLDER.get_or_insert("~/.marigold".to_string()).as_str()
    }
}

pub fn default_wallet_file() -> &'static str {
    // SAFETY: This operation is initializing a static mut variable,
    // however, the actual variable is accessible only through
    // this function.
    #[allow(static_mut_refs)]
    unsafe {
        DEFAULT_WALLET_FILE.get_or_insert("marigold".to_string()).as_str()
    }
}

pub fn default_settings_file() -> &'static str {
    // SAFETY: This operation is initializing a static mut variable,
    // however, the actual variable is accessible only through
    // this function.
    #[allow(static_mut_refs)]
    unsafe {
        DEFAULT_SETTINGS_FILE.get_or_insert("marigold".to_string()).as_str()
    }
}

/// Set a custom storage folder for the wallet SDK
/// subsystem.  Encrypted wallet files and transaction
/// data will be stored in this folder. If not set
/// the storage folder will default to `~/.marigold`
/// (note that the folder is hidden).
///
/// This must be called before using any other wallet
/// SDK functions.
///
/// NOTE: This function will create a folder if it
/// doesn't exist. This function will have no effect
/// if invoked in the browser environment.
///
/// # Safety
///
/// This function is unsafe because it is setting a static
/// mut variable, meaning this function is not thread-safe.
/// However the function must be used before any other
/// wallet operations are performed. You must not change
/// the default storage folder once the wallet has been
/// initialized.
///
pub unsafe fn set_default_storage_folder(folder: String) -> Result<()> {
    create_dir_all_sync(&folder).map_err(|err| Error::custom(format!("Failed to create storage folder: {err}")))?;
    unsafe {
        DEFAULT_STORAGE_FOLDER = Some(folder);
    }
    Ok(())
}

/// Set a custom storage folder for the wallet SDK
/// subsystem.  Encrypted wallet files and transaction
/// data will be stored in this folder. If not set
/// the storage folder will default to `~/.marigold`
/// (note that the folder is hidden).
///
/// This must be called before using any other wallet
/// SDK functions.
///
/// NOTE: This function will create a folder if it
/// doesn't exist. This function will have no effect
/// if invoked in the browser environment.
///
/// @param {String} folder - the path to the storage folder
///
/// @category Wallet API
#[wasm_bindgen(js_name = setDefaultStorageFolder, skip_jsdoc)]
pub fn js_set_default_storage_folder(folder: String) -> Result<()> {
    // SAFETY: This is unsafe because we are setting a static mut variable
    // meaning this function is not thread-safe. However the function
    // must be used before any other wallet operations are performed.
    unsafe { set_default_storage_folder(folder) }
}

/// Set the name of the default wallet file name
/// or the `localStorage` key.  If `Wallet::open`
/// is called without a wallet file name, this name
/// will be used.  Please note that this name
/// will be suffixed with `.wallet` suffix.
///
/// This function should be called before using any
/// other wallet SDK functions.
///
/// # Safety
///
/// This function is unsafe because it is setting a static
/// mut variable, meaning this function is not thread-safe.
///
pub unsafe fn set_default_wallet_file(folder: String) -> Result<()> {
    unsafe {
        DEFAULT_WALLET_FILE = Some(folder);
    }
    Ok(())
}

/// Set the name of the default wallet file name
/// or the `localStorage` key.  If `Wallet::open`
/// is called without a wallet file name, this name
/// will be used.  Please note that this name
/// will be suffixed with `.wallet` suffix.
///
/// This function should be called before using any
/// other wallet SDK functions.
///
/// @param {String} folder - the name to the wallet file or key.
///
/// @category Wallet API
#[wasm_bindgen(js_name = setDefaultWalletFile)]
pub fn js_set_default_wallet_file(folder: String) -> Result<()> {
    // SAFETY: This is unsafe because we are setting a static mut variable
    // meaning this function is not thread-safe.
    unsafe {
        DEFAULT_WALLET_FILE = Some(folder);
    }
    Ok(())
}
