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
        mine: None,
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

/// Hands notes over: a code whoever holds it can take.
#[tauri::command]
async fn give(state: State<'_, App>, amount: String) -> Result<Given, String> {
    let petals =
        try_kaspa_str_to_sompi(amount.trim()).map_err(|e| e.to_string())?.filter(|p| *p > 0).ok_or("that is not an amount")?;
    with_service(&state, |s| async move {
        let paid = s.pay(petals).await?;
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
        .manage(App { session: tokio::sync::Mutex::new(None), shutdown: Arc::new(AtomicBool::new(false)) })
        .invoke_handler(tauri::generate_handler![
            version,
            wallets,
            create_wallet,
            open,
            close,
            balance,
            status,
            history,
            classify,
            pay,
            take,
            give,
            request,
            wait_request,
            qr
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let state: State<'_, App> = window.state();
                state.shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        })
        .run(tauri::generate_context!())
        .expect("the Marigold window could not be opened");
}
