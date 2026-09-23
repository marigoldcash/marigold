//! The Marigold desktop wallet: a window over the wallet service the Telegram
//! bot uses (`kaspa_cli_lib::serve::WalletService`), so every screen does exactly
//! what the terminal wallet does, through the same code. Phase one (2026-09-23,
//! founder-asked after five testers wanted a GUI): open a wallet, see the
//! balance, pay a request code, make a request and watch it get paid. The
//! terminal wallet stays the light build for servers, mining and the bot.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use kaspa_cli_lib::serve::{Session, SessionOptions, open_session};
use kaspa_consensus_core::network::NetworkId;
use kaspa_wallet_core::prelude::*;
use kaspa_wallet_core::utils::{sompi_to_kaspa_string, try_kaspa_str_to_sompi};
use kaspa_wallet_core::wallet::Wallet;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

struct App {
    session: tokio::sync::Mutex<Option<Session>>,
    shutdown: Arc<AtomicBool>,
    /// "own", "local", "public" or an address: what the open screen chose.
    access: std::sync::Mutex<String>,
}

#[derive(Serialize)]
struct SyncState {
    synced: bool,
    own_node: bool,
    blocks: u64,
    headers: u64,
    daa: u64,
}

#[derive(Serialize)]
struct Machine {
    memory: String,
    mining: String,
    can_mine: bool,
}

#[derive(Serialize)]
struct WalletEntry {
    filename: String,
    title: String,
}

#[derive(Serialize)]
struct Opened {
    wallet: String,
    network: String,
    own_node: bool,
    /// How the wallet reaches the network, in the words the open screen used.
    access: String,
    ticker: String,
}

#[derive(Serialize)]
struct PaidOut {
    receipt: String,
    value: String,
    fee: String,
    notes: usize,
}

#[derive(Serialize)]
struct Created {
    filename: String,
    /// The 24 vault words, space-separated: the whole wallet comes back from them.
    words: String,
}

#[derive(Serialize)]
struct Classified {
    /// "request", "handover", "note", "receipt" or "unknown".
    kind: String,
    /// A pinned request's amount; empty when the payer chooses or it is no request.
    amount: String,
}

#[derive(Serialize)]
struct Given {
    code: String,
    value: String,
    fee: String,
    notes: usize,
    qr: String,
}

#[derive(Serialize)]
struct Requested {
    code: String,
    amount: String,
    qr: String,
}

/// The wallet settings decide the folder and the network, as everywhere else.
async fn probe() -> Result<(Arc<Wallet>, String, NetworkId), String> {
    let wallet =
        Arc::new(Wallet::try_with_rpc(None, Wallet::local_store().map_err(|e| e.to_string())?, None).map_err(|e| e.to_string())?);
    wallet.load_settings().await.ok();
    let folder = wallet
        .settings()
        .get::<String>(WalletSettings::Folder)
        .unwrap_or_else(|| kaspa_wallet_core::storage::local::default_storage_folder().to_string());
    let network_id = wallet
        .settings()
        .get::<String>(WalletSettings::Network)
        .and_then(|s| s.parse::<NetworkId>().ok())
        .unwrap_or(NetworkId::with_suffix(kaspa_consensus_core::network::NetworkType::Testnet, 10));
    wallet.store().set_storage_folder(&folder).map_err(|e| e.to_string())?;
    Ok((wallet, folder, network_id))
}

/// Makes a wallet the way the terminal wallet's wizard does: 24 vault words, a
/// ledger account derived from them, the vault keyed by them. With `words`, the
/// same wallet is rebuilt from a written-down set (its notes still need the
/// wallet's files, restored with the terminal wallet's `backup restore`).
#[tauri::command]
async fn create_wallet(name: String, password: String, words: Option<String>) -> Result<Created, String> {
    use kaspa_wallet_core::storage::keydata::PrvKeyDataVariantKind;
    use kaspa_wallet_core::storage::local::notevault::{account_mnemonic_from_vault_words, new_vault_words};
    let name = name.trim();
    let name = if name.is_empty() { "marigold" } else { name };
    if password.chars().count() < 8 {
        return Err("a password needs at least eight characters".to_string());
    }
    let (wallet, _, network_id) = probe().await?;
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
    let secret = Secret::from(password);
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
    Ok(Created { filename: descriptor.filename, words: vault_words })
}

#[tauri::command]
async fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
async fn wallets() -> Result<Vec<WalletEntry>, String> {
    let (wallet, _, _) = probe().await?;
    let list = wallet.store().wallet_list().await.map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|d| WalletEntry { title: d.title.clone().unwrap_or_else(|| d.filename.clone()), filename: d.filename })
        .collect())
}

/// `node`: "public" for a public computer, "local" for a node on this machine,
/// "own" for a sync inside the app, or a ws:// address.
#[tauri::command]
async fn open(app: AppHandle, state: State<'_, App>, wallet: String, password: String, node: String) -> Result<Opened, String> {
    let (_, _, network_id) = probe().await?;
    let node_url = match node.as_str() {
        "own" => None,
        "local" => Some(format!("ws://127.0.0.1:{}", network_id.default_borsh_rpc_port())),
        "public" => Some(
            kaspa_wrpc_client::resolver::public_nodes(network_id)
                .into_iter()
                .next()
                .ok_or_else(|| "no public computer is known for this network".to_string())?,
        ),
        url => Some(url.to_string()),
    };
    let say_app = app.clone();
    let say: kaspa_cli_lib::serve::Say = Arc::new(move |line: String| {
        let _ = say_app.emit("say", line);
    });
    let options = SessionOptions {
        wallet: wallet.clone(),
        password: Secret::from(password),
        node: node_url,
        // A miner host is made whenever the wallet has a ledger address, so the
        // Status screen can start it; nothing mines until asked.
        mine: Some(50),
        network: Some(network_id),
        origin: "desktop",
    };
    let session = open_session(&options, state.shutdown.clone(), say).await.map_err(|e| e.to_string())?;
    let access = match node.as_str() {
        "own" => "your own sync",
        "local" => "a node on this machine",
        "public" => "a public computer",
        _ => "the node you named",
    }
    .to_string();
    let opened = Opened {
        wallet,
        network: network_id.to_string(),
        own_node: session.own_node(),
        access,
        ticker: session.service.ticker().to_string(),
    };
    let mut guard = state.session.lock().await;
    if let Some(old) = guard.take() {
        old.close().await;
    }
    *guard = Some(session);
    *state.access.lock().unwrap() = node;
    Ok(opened)
}

#[tauri::command]
async fn close(state: State<'_, App>) -> Result<(), String> {
    if let Some(session) = state.session.lock().await.take() {
        session.close().await;
    }
    Ok(())
}

async fn with_service<T, F, Fut>(state: &State<'_, App>, f: F) -> Result<T, String>
where
    F: FnOnce(Arc<kaspa_cli_lib::serve::WalletService>) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let service = state.session.lock().await.as_ref().map(|s| s.service.clone()).ok_or("no wallet is open")?;
    f(service).await
}

#[tauri::command]
async fn balance(state: State<'_, App>) -> Result<String, String> {
    with_service(&state, |s| async move { Ok(s.balance_text().await) }).await
}

#[tauri::command]
async fn status(state: State<'_, App>) -> Result<String, String> {
    with_service(&state, |s| async move { Ok(s.status_text().await) }).await
}

/// Where the node the wallet uses stands: for a sync of its own, the figures
/// that move while it catches up.
#[tauri::command]
async fn sync_state(state: State<'_, App>) -> Result<SyncState, String> {
    let (rpc, own_node) = state.session.lock().await.as_ref().map(|s| (s.rpc.clone(), s.own_node())).ok_or("no wallet is open")?;
    let synced = matches!(rpc.get_server_info().await, Ok(info) if info.is_synced);
    let dag = rpc.get_block_dag_info().await.map_err(|e| e.to_string())?;
    Ok(SyncState { synced, own_node, blocks: dag.block_count, headers: dag.header_count, daa: dag.virtual_daa_score })
}

/// The machine's memory and the miner, for the Status screen.
#[tauri::command]
async fn machine(state: State<'_, App>) -> Result<Machine, String> {
    let memory = match kaspa_cli_lib::memory::read() {
        Some(m) => format!("{} free of {}", kaspa_cli_lib::memory::gigabytes(m.available), kaspa_cli_lib::memory::gigabytes(m.total)),
        None => "not measured".to_string(),
    };
    let access = state.access.lock().unwrap().clone();
    let can_mine = access != "public";
    let mining = with_service(&state, |s| async move { Ok(s.mine(None).await) }).await?;
    Ok(Machine { memory, mining, can_mine })
}

/// "start" or "stop". Mining through a public computer is refused: the block
/// template it hands out carries this wallet's payout address, and its operator
/// would see it — the terminal wallet refuses the same way.
#[tauri::command]
async fn mine(state: State<'_, App>, action: String) -> Result<String, String> {
    let access = state.access.lock().unwrap().clone();
    if access == "public" && action == "start" {
        return Err("Mining through a public computer would tell its operator where your rewards go. Open the wallet with a node on this machine or a sync of your own to mine.".to_string());
    }
    let session_rpc = state.session.lock().await.as_ref().map(|s| s.rpc.clone()).ok_or("no wallet is open")?;
    if action == "start" {
        let synced = matches!(session_rpc.get_server_info().await, Ok(info) if info.is_synced);
        if !synced {
            return Err("The sync has to finish first; blocks built on an unfinished copy are accepted by nobody.".to_string());
        }
    }
    with_service(&state, |s| async move { Ok(s.mine(Some(action.as_str())).await) }).await
}

#[tauri::command]
async fn history(state: State<'_, App>) -> Result<String, String> {
    with_service(&state, |s| async move { Ok(s.history_text(12)) }).await
}

/// What a pasted code is, before anything is done with it.
#[tauri::command]
async fn classify(code: String) -> Result<Classified, String> {
    use kaspa_wallet_core::account::notepool::{
        BEARER_NOTE_PREFIX, HANDOVER_PREFIX, LOCKED_HANDOVER_PREFIX, PAYMENT_RECEIPT_PREFIX, PaymentRequest,
    };
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

/// `amount` is used only when the request pins none.
#[tauri::command]
async fn pay(state: State<'_, App>, code: String, amount: String) -> Result<PaidOut, String> {
    let chosen = if amount.trim().is_empty() {
        None
    } else {
        Some(try_kaspa_str_to_sompi(amount.trim()).map_err(|e| e.to_string())?.filter(|p| *p > 0).ok_or("that is not an amount")?)
    };
    with_service(&state, |s| async move {
        let paid = s.pay_request_with(code.trim(), chosen).await?;
        Ok(PaidOut {
            receipt: paid.code,
            value: sompi_to_kaspa_string(paid.value_petals),
            fee: sompi_to_kaspa_string(paid.fee_petals),
            notes: paid.notes,
        })
    })
    .await
}

/// Takes a code somebody handed over: a hand-over, a locked hand-over, or a bare note.
#[tauri::command]
async fn take(state: State<'_, App>, code: String) -> Result<String, String> {
    with_service(&state, |s| async move { s.receive(code.trim()).await }).await
}

/// The open wallet's 24 words, shown again only against its password.
#[tauri::command]
async fn words(state: State<'_, App>, password: String) -> Result<String, String> {
    with_service(&state, |s| async move { s.recovery_words(Secret::from(password)).await }).await
}

/// Your share key: give it to someone so their hand-over can be made for you
/// alone, with a time lock (PLAN P8.0g). A fresh key each time, labelled.
#[tauri::command]
async fn share_key(state: State<'_, App>) -> Result<Requested, String> {
    with_service(&state, |s| async move {
        let code = s.share_key("desktop").await?;
        let qr = qr_data_url(&code);
        Ok(Requested { code, amount: String::new(), qr })
    })
    .await
}

/// Hands notes over: a code whoever holds it can take — or, with `key`, one
/// only that key's holder can take, and only within `minutes`, after which the
/// notes come back to this wallet by themselves.
#[tauri::command]
async fn give(state: State<'_, App>, amount: String, key: String, minutes: u64) -> Result<Given, String> {
    let petals =
        try_kaspa_str_to_sompi(amount.trim()).map_err(|e| e.to_string())?.filter(|p| *p > 0).ok_or("that is not an amount")?;
    let key = key.trim().to_string();
    with_service(&state, |s| async move {
        let paid = if key.is_empty() {
            s.pay(petals).await?
        } else {
            if !kaspa_cli_lib::serve::WalletService::is_share_key(&key) {
                return Err("that is not a share key (marigoldkey:…)".to_string());
            }
            s.pay_locked(petals, &key, minutes.clamp(1, 7 * 24 * 60) * 60).await?
        };
        let qr = qr_data_url(&paid.code);
        Ok(Given {
            code: paid.code,
            value: sompi_to_kaspa_string(paid.value_petals),
            fee: sompi_to_kaspa_string(paid.fee_petals),
            notes: paid.notes,
            qr,
        })
    })
    .await
}

#[tauri::command]
async fn request(state: State<'_, App>, amount: String) -> Result<Requested, String> {
    let petals = if amount.trim().is_empty() {
        None
    } else {
        Some(try_kaspa_str_to_sompi(amount.trim()).map_err(|e| e.to_string())?.filter(|p| *p > 0).ok_or("that is not an amount")?)
    };
    with_service(&state, |s| async move {
        let code = kaspa_cli_lib::serve::WalletService::request(&s, petals).await?;
        let qr = qr_data_url(&code);
        Ok(Requested { code, amount: petals.map(sompi_to_kaspa_string).unwrap_or_default(), qr })
    })
    .await
}

/// Waits up to `seconds` for a request to be paid; `None` when it was not yet.
#[tauri::command]
async fn wait_request(state: State<'_, App>, code: String, seconds: u64) -> Result<Option<String>, String> {
    with_service(&state, |s| async move { s.await_request(code.trim(), Duration::from_secs(seconds.clamp(1, 120))).await }).await
}

#[tauri::command]
async fn qr(text: String) -> Result<String, String> {
    Ok(qr_data_url(&text))
}

fn qr_data_url(text: &str) -> String {
    use base64::Engine;
    match kaspa_cli_lib::qrpng::qr_png(text) {
        Some(png) => format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)),
        None => String::new(),
    }
}

fn main() {
    // WebKitGTK's DMA-BUF renderer fails on NVIDIA's driver ("Failed to create
    // GBM buffer … Permission denied") and shows a blank window; the classic
    // renderer is fine. Set before any thread exists, as the standard requires.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
    tauri::Builder::default()
        .manage(App {
            session: tokio::sync::Mutex::new(None),
            shutdown: Arc::new(AtomicBool::new(false)),
            access: std::sync::Mutex::new(String::new()),
        })
        .invoke_handler(tauri::generate_handler![
            version,
            wallets,
            create_wallet,
            open,
            close,
            balance,
            status,
            history,
            machine,
            mine,
            sync_state,
            classify,
            pay,
            take,
            give,
            share_key,
            words,
            request,
            wait_request,
            qr
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // Close the wallet properly before the window goes: a node
                // running inside the app is stopped and its database left
                // clean, the way 'exit' does it in the terminal wallet.
                let state: State<'_, App> = window.state();
                state.shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
                tauri::async_runtime::block_on(async {
                    if let Some(session) = state.session.lock().await.take() {
                        session.close().await;
                    }
                });
            }
        })
        .run(tauri::generate_context!())
        .expect("the Marigold window could not be opened");
}
