//! What the phone apps call: the desktop's commands (`gui/src/main.rs`), exported through UniFFI so
//! Swift and Kotlin can call them. Every wallet operation here is the one the desktop and the
//! terminal use (`crate::app`, `crate::service`); this file holds only the state an app keeps
//! between calls (the open session, the background watcher) and the types that cross over.
//! (docs/marigold/MOBILE-PLAN.md, PH-M3a.)

use crate::app;
use crate::providers::Providers;
use crate::service::{Say, WalletService};
use crate::session::{Session, SessionOptions, open_session};
use crate::tgbackup::AutoState;
use kaspa_consensus_core::network::NetworkId;
use kaspa_wallet_core::prelude::Secret;
use std::ffi::{CStr, c_char};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

static VERSION: &CStr = match CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
    Ok(v) => v,
    Err(_) => panic!("version has a NUL"),
};

/// The version of the core as a NUL-terminated C string that lives as long as the library: the one
/// plain C symbol, which the build checks for in every library it makes.
#[unsafe(no_mangle)]
pub extern "C" fn marigold_core_version() -> *const c_char {
    VERSION.as_ptr()
}

/// The version of the core.
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Why a call did not do what was asked. Every case carries the words to show the person.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum WalletError {
    #[error("no wallet is open")]
    NoWalletOpen,
    /// The node could not be reached, or stopped answering.
    #[error("{text}")]
    Network { text: String },
    /// Anything else: a wrong password, a wrong amount, a code that does not fit.
    #[error("{text}")]
    Failed { text: String },
}

impl From<String> for WalletError {
    fn from(text: String) -> Self {
        if text.contains("cannot reach") || text.contains("timed out") || text.contains("Connection refused") {
            WalletError::Network { text }
        } else {
            WalletError::Failed { text }
        }
    }
}

impl From<crate::error::Error> for WalletError {
    fn from(err: crate::error::Error) -> Self {
        err.to_string().into()
    }
}

type Reply<T> = Result<T, WalletError>;

/// Where the wallets live and which network they are on.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Probe {
    pub folder: String,
    pub network: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct WalletEntry {
    pub filename: String,
    pub title: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Created {
    pub filename: String,
    /// The 24 words, space-separated: the whole wallet comes back from them.
    pub words: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Opened {
    pub wallet: String,
    pub network: String,
    /// How the wallet reaches the network, in the words the open screen used.
    pub access: String,
    pub ticker: String,
}

/// What a pasted code is: "request", "handover", "note", "receipt" or "unknown", and a request's
/// pinned amount (empty when the payer chooses).
#[derive(Debug, Clone, uniffi::Record)]
pub struct Classified {
    pub kind: String,
    pub amount: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PaidOut {
    pub receipt: String,
    pub value: String,
    pub fee: String,
    pub notes: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Given {
    pub code: String,
    pub value: String,
    pub fee: String,
    pub notes: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Requested {
    pub code: String,
    pub amount: String,
}

/// The state of the wallet's backups, for the screen that shows them.
#[derive(Debug, Clone, uniffi::Record)]
pub struct BackupInfo {
    pub destination: String,
    pub bot: bool,
    pub paired: bool,
    pub pairing_code: Option<String>,
    pub automatic: String,
    pub offer: bool,
    pub checkpoint_at: u64,
    pub checkpoint_bytes: u64,
    pub deltas: u32,
    pub last_post_at: u64,
    pub wallets: Vec<String>,
    pub uncovered: Vec<String>,
    pub folder: Option<String>,
}

/// The wallet's own copy of the notes, checked against the trustees' anchors.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LightInfo {
    /// A copy is being kept; false when the wallet reads from a node of its own.
    pub active: bool,
    pub notes: u64,
    pub anchored_daa: u64,
    pub checked_ms: u64,
}

/// What the app hears from the core while a wallet is open.
#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    /// A line for the person, in plain words.
    fn on_say(&self, line: String);
    /// The balance or the notes changed.
    fn on_notes_changed(&self);
    /// The connection to the node came or went.
    fn on_connection(&self, connected: bool);
}

type Listener = Arc<Mutex<Option<Arc<dyn EventListener>>>>;

/// The core an app holds: one wallets folder, at most one open wallet.
#[derive(uniffi::Object)]
pub struct MarigoldCore {
    folder: String,
    network: NetworkId,
    shutdown: Arc<AtomicBool>,
    session: tokio::sync::Mutex<Option<Session>>,
    listener: Listener,
    watcher: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    backup_state: Arc<Mutex<AutoState>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl MarigoldCore {
    /// `data_dir`: where the wallets are kept (the app's own folder). `network`: "testnet-10" when
    /// empty.
    #[uniffi::constructor]
    pub fn new(data_dir: String, network: Option<String>) -> Reply<Arc<Self>> {
        let network = match network.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            Some(text) => text.parse::<NetworkId>().map_err(|_| WalletError::Failed { text: format!("{text} is not a network") })?,
            None => app::default_network(),
        };
        Ok(Arc::new(Self {
            folder: data_dir,
            network,
            shutdown: Arc::new(AtomicBool::new(false)),
            session: tokio::sync::Mutex::new(None),
            listener: Arc::new(Mutex::new(None)),
            watcher: tokio::sync::Mutex::new(None),
            backup_state: Arc::new(Mutex::new(AutoState::default())),
        }))
    }

    /// Where the wallets live and which network they are on.
    pub fn probe(&self) -> Probe {
        Probe { folder: self.folder.clone(), network: self.network.to_string() }
    }

    /// Who hears the core's events; `None` stops them.
    pub fn set_listener(&self, listener: Option<Arc<dyn EventListener>>) {
        *self.listener.lock().unwrap() = listener;
    }

    pub async fn list_wallets(&self) -> Reply<Vec<WalletEntry>> {
        let list = app::list_wallets(Some(&self.folder), Some(self.network)).await?;
        Ok(list.into_iter().map(|d| WalletEntry { filename: d.filename, title: d.title }).collect())
    }

    /// Makes a wallet with 24 new words. The password must pass the policy (a PIN of four will do);
    /// `allow_empty` is the advanced choice of none.
    pub async fn create_wallet(&self, name: String, password: String, allow_empty: bool) -> Reply<Created> {
        let made = app::create_wallet(Some(&self.folder), Some(self.network), &name, &password, None, allow_empty).await?;
        Ok(Created { filename: made.filename, words: made.words })
    }

    /// Rebuilds a wallet from its 24 words. Its notes come back with `restore_telegram`.
    pub async fn restore_from_words(&self, name: String, password: String, words: String) -> Reply<Created> {
        let made = app::create_wallet(Some(&self.folder), Some(self.network), &name, &password, Some(&words), false).await?;
        Ok(Created { filename: made.filename, words: made.words })
    }

    /// Restores a wallet from the backups forwarded to its Telegram bot. Progress arrives as `on_say`.
    pub async fn restore_telegram(&self, token: String, words: String, name: String) -> Reply<String> {
        let listener = self.listener.clone();
        let progress = move |line: String| say_to(&listener, line);
        Ok(app::restore_from_telegram(Some(&self.folder), Some(self.network), &token, &words, &name, &progress).await?)
    }

    /// Opens a wallet. `node`: "public" for a public computer, or a ws:// address.
    pub async fn open(&self, wallet: String, password: String, node: String) -> Reply<Opened> {
        let node_url = app::resolve_node(&node, self.network)?;
        let listener = self.listener.clone();
        let say: Say = Arc::new(move |line: String| say_to(&listener, line));
        let options = SessionOptions {
            wallet: wallet.clone(),
            password: Secret::from(password),
            node: node_url,
            mine: None,
            network: Some(self.network),
            folder: Some(self.folder.clone()),
            origin: "phone",
        };
        let session = open_session(&options, self.shutdown.clone(), say, &Providers::default())
            .await
            .map_err(|e| WalletError::from(app::open_error_text(e.to_string(), &node)))?;
        self.close_wallet().await;
        let opened = Opened {
            wallet,
            network: self.network.to_string(),
            access: app::access_words(&node).to_string(),
            ticker: session.service.ticker().to_string(),
        };
        let (service, wallet) = (session.service.clone(), session.wallet.clone());
        *self.session.lock().await = Some(session);
        *self.watcher.lock().await = Some(self.watch(service, wallet));
        Ok(opened)
    }

    /// Closes the open wallet properly: the last backup change goes out and the watcher stops. (Not called `close`: the Kotlin
    /// binding gives its object a `close()` of its own.)
    pub async fn close_wallet(&self) {
        if let Some(watcher) = self.watcher.lock().await.take() {
            watcher.abort();
        }
        if let Some(session) = self.session.lock().await.take() {
            session.service.backup_flush().await;
            session.close().await;
        }
    }

    pub async fn balance(&self) -> Reply<String> {
        Ok(self.service().await?.balance_text().await)
    }

    pub async fn status(&self) -> Reply<String> {
        Ok(self.service().await?.status_text().await)
    }

    pub async fn history(&self) -> Reply<String> {
        Ok(self.service().await?.history_text(12))
    }

    /// Takes a code somebody handed over: a hand-over, a locked hand-over, or a bare note.
    pub async fn receive(&self, code: String) -> Reply<String> {
        Ok(self.service().await?.receive(code.trim()).await?)
    }

    /// A request for payment; an empty amount lets the payer choose.
    pub async fn request(&self, amount: String) -> Reply<Requested> {
        let petals = app::optional_amount(&amount)?;
        let code = self.service().await?.request(petals).await?;
        Ok(Requested { code, amount: petals.map(app::amount_text).unwrap_or_default() })
    }

    /// Waits up to `seconds` for a request to be paid; `None` when it was not yet.
    pub async fn wait_request(&self, code: String, seconds: u64) -> Reply<Option<String>> {
        let wait = Duration::from_secs(seconds.clamp(1, 120));
        Ok(self.service().await?.await_request(code.trim(), wait).await?)
    }

    /// What a pasted code is, before anything is done with it.
    pub fn pay_preview(&self, code: String) -> Reply<Classified> {
        let found = app::classify(&code)?;
        Ok(Classified { kind: found.kind, amount: found.amount })
    }

    /// Pays a request code. `amount` is used only when the request pins none.
    pub async fn pay(&self, code: String, amount: String) -> Reply<PaidOut> {
        let chosen = app::optional_amount(&amount)?;
        let paid = self.service().await?.pay_request_with(code.trim(), chosen).await?;
        Ok(PaidOut {
            receipt: paid.code,
            value: app::amount_text(paid.value_petals),
            fee: app::amount_text(paid.fee_petals),
            notes: paid.notes as u32,
        })
    }

    /// Your share key: give it to someone so their hand-over can be made for you alone.
    pub async fn share_key(&self) -> Reply<String> {
        Ok(self.service().await?.share_key("phone").await?)
    }

    /// Hands notes over: a code whoever holds it can take or, with `key`, one only that key's holder
    /// can take, within `minutes`.
    pub async fn give(&self, amount: String, key: String, minutes: u64) -> Reply<Given> {
        let petals = app::amount(&amount)?;
        let key = key.trim().to_string();
        let service = self.service().await?;
        let paid = if key.is_empty() {
            service.pay(petals).await?
        } else {
            if !WalletService::is_share_key(&key) {
                return Err(WalletError::Failed { text: "that is not a share key (marigoldkey:…)".to_string() });
            }
            service.pay_locked(petals, &key, minutes.clamp(1, 7 * 24 * 60) * 60).await?
        };
        Ok(Given {
            code: paid.code,
            value: app::amount_text(paid.value_petals),
            fee: app::amount_text(paid.fee_petals),
            notes: paid.notes as u32,
        })
    }

    /// The open wallet's 24 words, shown again only against its password.
    pub async fn recovery_words(&self, password: String) -> Reply<String> {
        Ok(self.service().await?.recovery_words(Secret::from(password)).await?)
    }

    /// A new password for the open wallet.
    pub async fn change_password(&self, old: String, new: String, allow_empty: bool) -> Reply<()> {
        if let Some(problem) = app::new_password_problem(&new, allow_empty) {
            return Err(WalletError::Failed { text: problem });
        }
        let wallet = self.session.lock().await.as_ref().map(|s| s.wallet.clone()).ok_or(WalletError::NoWalletOpen)?;
        app::change_password(&wallet, &Secret::from(old), &Secret::from(new))
            .await
            .map_err(|e| WalletError::Failed { text: e.to_string() })
    }

    pub async fn backup_status(&self) -> Reply<BackupInfo> {
        let s = self.service().await?.backup_status().await?;
        Ok(BackupInfo {
            destination: s.destination,
            bot: s.bot,
            paired: s.paired,
            pairing_code: s.pairing_code,
            automatic: s.automatic,
            offer: s.offer,
            checkpoint_at: s.checkpoint_at,
            checkpoint_bytes: s.checkpoint_bytes,
            deltas: s.deltas,
            last_post_at: s.last_post_at,
            wallets: s.wallets,
            uncovered: s.uncovered,
            folder: s.folder,
        })
    }

    pub async fn backup_now(&self) -> Reply<String> {
        Ok(self.service().await?.backup_now().await?)
    }

    pub async fn backup_automatic(&self, on: bool) -> Reply<()> {
        Ok(self.service().await?.backup_automatic(on).await?)
    }

    pub async fn backup_decline(&self) -> Reply<()> {
        Ok(self.service().await?.backup_decline().await?)
    }

    /// A wallet of this folder without a backup key yet, covered with its 24 words typed once.
    pub async fn backup_cover(&self, name: String, words: String) -> Reply<()> {
        Ok(self.service().await?.backup_cover(&name, &words).await?)
    }

    /// Sets the wallet's Telegram bot up; the answer is the code to send the bot to pair it.
    pub async fn telegram_setup(&self, token: String, pin: String) -> Reply<String> {
        Ok(self.service().await?.telegram_setup(&token, &pin)?)
    }

    /// Whether the wallet follows the notes in a copy of its own, and how fresh the copy is.
    pub async fn light_status(&self) -> Reply<LightInfo> {
        let wallet = self.session.lock().await.as_ref().map(|s| s.wallet.clone()).ok_or(WalletError::NoWalletOpen)?;
        Ok(match wallet.light() {
            Some(light) => {
                let st = light.status();
                LightInfo { active: true, notes: st.notes as u64, anchored_daa: st.anchored_daa, checked_ms: st.checked_ms }
            }
            None => LightInfo { active: false, notes: 0, anchored_daa: 0, checked_ms: 0 },
        })
    }
}

impl MarigoldCore {
    async fn service(&self) -> Reply<Arc<WalletService>> {
        self.session.lock().await.as_ref().map(|s| s.service.clone()).ok_or(WalletError::NoWalletOpen)
    }

    /// While a wallet is open: tells the app when the balance or the connection changes, and keeps
    /// the automatic backup going.
    fn watch(&self, service: Arc<WalletService>, wallet: Arc<kaspa_wallet_core::wallet::Wallet>) -> tokio::task::JoinHandle<()> {
        let (listener, state) = (self.listener.clone(), self.backup_state.clone());
        tokio::spawn(async move {
            let (mut balance, mut connected, mut round) = (String::new(), None::<bool>, 0u64);
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                round += 1;
                let now = wallet.is_connected();
                if connected != Some(now) {
                    connected = Some(now);
                    if let Some(l) = listener.lock().unwrap().clone() {
                        l.on_connection(now);
                    }
                }
                let text = service.balance_text().await;
                if text != balance {
                    let first = balance.is_empty();
                    balance = text;
                    if !first && let Some(l) = listener.lock().unwrap().clone() {
                        l.on_notes_changed();
                    }
                }
                if round % 7 == 0 {
                    service.backup_tick(&state).await;
                }
            }
        })
    }
}

/// The terminal paints its remarks; the app shows them plain.
fn say_to(listener: &Listener, line: String) {
    if let Some(l) = listener.lock().unwrap().clone() {
        l.on_say(app::strip_ansi(&line));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core(dir: &tempfile::TempDir) -> Arc<MarigoldCore> {
        MarigoldCore::new(dir.path().display().to_string(), None).unwrap()
    }

    #[test]
    fn the_core_names_its_version_and_its_network() {
        assert_eq!(core_version(), env!("CARGO_PKG_VERSION"));
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(core(&dir).probe().network, "testnet-10");
        assert!(MarigoldCore::new(dir.path().display().to_string(), Some("nowhere".into())).is_err());
    }

    /// The wallet is made, listed and refused a second time through the same calls the apps make;
    /// nothing is open, so the calls that need a wallet say so; a node that is not there is a
    /// network error in words, not a hang.
    #[tokio::test]
    async fn a_wallet_is_made_listed_and_a_missing_node_is_said() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(&dir);
        assert!(core.list_wallets().await.unwrap().is_empty());

        let refused = core.create_wallet("ph".into(), "12".into(), false).await.unwrap_err();
        assert!(matches!(refused, WalletError::Failed { .. }), "{refused}");

        let made = core.create_wallet("ph".into(), "hunter2".into(), false).await.unwrap();
        assert_eq!(made.words.split_whitespace().count(), 24);
        assert_eq!(core.list_wallets().await.unwrap().len(), 1);
        assert!(core.create_wallet("ph".into(), "hunter2".into(), false).await.is_err(), "a name is not used twice");

        assert!(matches!(core.status().await, Err(WalletError::NoWalletOpen)));
        assert!(matches!(core.balance().await, Err(WalletError::NoWalletOpen)));
        assert!(matches!(core.change_password("a".into(), "bbbb".into(), false).await, Err(WalletError::NoWalletOpen)));

        let failed = core.open("ph".into(), "hunter2".into(), "ws://127.0.0.1:9".into()).await.unwrap_err();
        assert!(matches!(failed, WalletError::Network { .. }), "{failed}");

        let again = core.restore_from_words("copy".into(), "hunter2".into(), made.words.clone()).await.unwrap();
        assert_eq!(again.words, made.words, "the words come back as given");
        assert_eq!(core.list_wallets().await.unwrap().len(), 2);
        assert!(core.restore_from_words("bad".into(), "hunter2".into(), "one two three".into()).await.is_err());
    }

    /// What a wallet leaves on a phone is the owner's alone: nothing in the wallets folder can be read, written or
    /// entered by anyone else, as on the desktop (docs/marigold/MOBILE-PLAN.md, PH-M5).
    #[cfg(unix)]
    #[tokio::test]
    async fn the_files_of_a_new_wallet_are_the_owners_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let core = core(&dir);
        core.create_wallet("ph".into(), "hunter2".into(), false).await.unwrap();
        let mut seen = 0;
        let mut open_to_others = Vec::new();
        let mut stack = vec![dir.path().to_path_buf()];
        while let Some(path) = stack.pop() {
            for entry in std::fs::read_dir(&path).unwrap() {
                let entry = entry.unwrap();
                let mode = entry.metadata().unwrap().permissions().mode() & 0o777;
                seen += 1;
                if mode & 0o077 != 0 {
                    open_to_others.push(format!("{:o} {}", mode, entry.path().display()));
                }
                if entry.file_type().unwrap().is_dir() {
                    stack.push(entry.path());
                }
            }
        }
        assert!(seen >= 3, "a wallet leaves files behind; saw {seen}");
        assert!(open_to_others.is_empty(), "files others can reach: {open_to_others:?}");
    }

    #[test]
    fn a_pasted_code_is_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(&dir);
        assert_eq!(core.pay_preview("hello".into()).unwrap().kind, "unknown");
        assert_eq!(core.pay_preview("  ".into()).unwrap().kind, "unknown");
    }

    /// The whole run, against a public node: make a wallet, open it, read its status, change the
    /// password, close, and open with the new one. Needs the network, so it runs on request:
    /// `cargo test -p kaspa-wallet-service --features ffi -- --ignored`.
    #[tokio::test]
    #[ignore = "needs a public node"]
    async fn a_wallet_opens_on_a_public_node_and_closes() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(&dir);
        let heard = Arc::new(Mutex::new(Vec::<String>::new()));
        struct Ears(Arc<Mutex<Vec<String>>>);
        impl EventListener for Ears {
            fn on_say(&self, line: String) {
                self.0.lock().unwrap().push(line);
            }
            fn on_notes_changed(&self) {}
            fn on_connection(&self, _: bool) {}
        }
        core.set_listener(Some(Arc::new(Ears(heard.clone()))));
        core.create_wallet("ph".into(), "hunter2".into(), false).await.unwrap();

        let opened = core.open("ph".into(), "hunter2".into(), "public".into()).await.unwrap();
        assert_eq!(opened.access, "a public computer");
        let status = core.status().await.unwrap();
        assert!(!status.is_empty());
        let light = core.light_status().await.unwrap();
        assert!(light.active && light.notes > 0, "the copy of the notes is kept: {light:?}");
        assert!(core.balance().await.is_ok());
        assert!(core.history().await.is_ok());
        assert!(core.backup_status().await.is_ok());
        assert!(core.recovery_words("wrong".into()).await.is_err());
        assert_eq!(core.recovery_words("hunter2".into()).await.unwrap().split_whitespace().count(), 24);

        assert!(core.change_password("not it".into(), "newpass".into(), false).await.is_err());
        core.change_password("hunter2".into(), "newpass".into(), false).await.unwrap();
        core.close_wallet().await;
        assert!(matches!(core.status().await, Err(WalletError::NoWalletOpen)));

        let wrong = core.open("ph".into(), "hunter2".into(), "public".into()).await.unwrap_err();
        assert_eq!(wrong.to_string(), "That is not this wallet's password.", "the old password no longer opens it");
        core.open("ph".into(), "newpass".into(), "public".into()).await.unwrap();
        core.close_wallet().await;
        assert!(heard.lock().unwrap().iter().any(|l| l.contains("Opening the wallet")), "{:?}", heard.lock().unwrap());
    }
}
