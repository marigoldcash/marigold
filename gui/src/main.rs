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
struct Requested {
    code: String,
    amount: String,
    qr: String,
}

/// The wallet settings decide the folder and the network, as everywhere else.
async fn probe() -> Result<(Arc<Wallet>, String, NetworkId), String> {
    let wallet = Arc::new(Wallet::try_with_rpc(None, Wallet::local_store().map_err(|e| e.to_string())?, None).map_err(|e| e.to_string())?);
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
            kaspa_wrpc_client::Resolver::default()
                .get_url(kaspa_wrpc_client::WrpcEncoding::Borsh, network_id)
                .await
                .map_err(|e| format!("no public computer answers: {e}"))?,
        ),
        url => Some(url.to_string()),
    };
    let say_app = app.clone();
    let say: kaspa_cli_lib::serve::Say = Arc::new(move |line: String| {
        let _ = say_app.emit("say", line);
    });
    let options = SessionOptions { wallet: wallet.clone(), password: Secret::from(password), node: node_url, mine: None, network: Some(network_id) };
    let session = open_session(&options, state.shutdown.clone(), say).await.map_err(|e| e.to_string())?;
    let opened = Opened {
        wallet,
        network: network_id.to_string(),
        own_node: session.own_node(),
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

/// What a request code asks for, before anything is paid.
#[tauri::command]
async fn request_amount(code: String) -> Result<String, String> {
    let petals = kaspa_cli_lib::serve::WalletService::request_amount(code.trim())?;
    Ok(sompi_to_kaspa_string(petals))
}

#[tauri::command]
async fn pay(state: State<'_, App>, code: String) -> Result<PaidOut, String> {
    with_service(&state, |s| async move {
        let paid = s.pay_request(code.trim()).await?;
        Ok(PaidOut {
            receipt: paid.code,
            value: sompi_to_kaspa_string(paid.value_petals),
            fee: sompi_to_kaspa_string(paid.fee_petals),
            notes: paid.notes,
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
    tauri::Builder::default()
        .manage(App { session: tokio::sync::Mutex::new(None), shutdown: Arc::new(AtomicBool::new(false)) })
        .invoke_handler(tauri::generate_handler![
            version,
            wallets,
            open,
            close,
            balance,
            status,
            history,
            request_amount,
            pay,
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
