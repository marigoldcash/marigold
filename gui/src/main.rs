//! The Marigold desktop wallet: a window over the wallet service the Telegram
//! bot uses (`kaspa_cli_lib::serve::WalletService`), so every screen does exactly
//! what the terminal wallet does, through the same code. Phase one (2026-09-23,
//! founder-asked after five testers wanted a GUI): open a wallet, see the
//! balance, pay a request code, make a request and watch it get paid. The
//! terminal wallet stays the light build for servers, mining and the bot.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use kaspa_cli_lib::app;
use kaspa_cli_lib::serve::{Session, SessionOptions, WalletService, open_session};
use kaspa_cli_lib::tgbackup::{AutoState, BackupStatus};
use kaspa_consensus_core::network::NetworkId;
use kaspa_wallet_core::prelude::*;
use kaspa_wallet_core::utils::sompi_to_kaspa_string;
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
    /// The wallet's Telegram bot, answered while the wallet is open — as the
    /// terminal wallet does — so the phone works and a pairing can happen.
    bot: tokio::sync::Mutex<Option<tokio::task::AbortHandle>>,
    /// The automatic backup's tick.
    ticker: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    backup_state: Arc<std::sync::Mutex<AutoState>>,
}

/// Starts the bot (when one is set up) and the backup tick for an open wallet.
async fn start_background(app: &App, service: Arc<WalletService>) {
    stop_background(app).await;
    if let Some(cfg) = service.telegram_config() {
        let path = service.telegram_path();
        let bot = tokio::spawn(kaspa_cli_lib::telegram::run_bot(service.clone(), path, cfg));
        let abort = bot.abort_handle();
        let say = service.say_handle();
        tokio::spawn(async move {
            match bot.await {
                Ok(()) => say("The Telegram bot stopped answering; close and open the wallet to start it again.".to_string()),
                Err(err) if err.is_panic() => {
                    say("The Telegram bot crashed; close and open the wallet to start it again.".to_string())
                }
                Err(_) => {}
            }
        });
        *app.bot.lock().await = Some(abort);
    }
    let state = app.backup_state.clone();
    *app.ticker.lock().await = Some(tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            service.backup_tick(&state).await;
        }
    }));
}

async fn stop_background(app: &App) {
    if let Some(bot) = app.bot.lock().await.take() {
        bot.abort();
    }
    if let Some(ticker) = app.ticker.lock().await.take() {
        ticker.abort();
    }
}

/// Closes the open wallet properly: the last backup change goes out, the bot
/// and the tick stop, a node inside the app is stopped.
async fn close_session(app: &App) {
    stop_background(app).await;
    if let Some(session) = app.session.lock().await.take() {
        session.service.backup_flush().await;
        session.close().await;
    }
}

/// The terminal paints its remarks; the window shows them plain.
fn strip_ansi(text: &str) -> String {
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

/// Whether a Marigold program on this computer already serves the network on
/// its usual port: the terminal wallet's sync, or another Marigold program.
#[tauri::command]
async fn local_program_present() -> Result<bool, String> {
    let (_, _, network_id) = probe().await?;
    Ok(kaspa_cli_lib::modules::connect::probe_local_node(network_id).await.is_some())
}

/// The wallet settings decide the folder and the network, as everywhere else.
async fn probe() -> Result<(Arc<Wallet>, String, NetworkId), String> {
    app::probe(None, None).await
}

/// Makes a wallet the way the terminal wallet's wizard does (`app::create_wallet`). With `words`,
/// the same wallet is rebuilt from a written-down set (its notes still need the wallet's files,
/// restored with the terminal wallet's `backup restore`).
#[tauri::command]
async fn create_wallet(name: String, password: String, words: Option<String>, allow_empty: Option<bool>) -> Result<Created, String> {
    let made = app::create_wallet(None, None, &name, &password, words.as_deref(), allow_empty.unwrap_or(false)).await?;
    Ok(Created { filename: made.filename, words: made.words })
}

#[tauri::command]
async fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
async fn wallets() -> Result<Vec<WalletEntry>, String> {
    Ok(app::list_wallets(None, None).await?.into_iter().map(|d| WalletEntry { filename: d.filename, title: d.title }).collect())
}

/// `node`: "public" for a public computer, "local" for a node on this machine,
/// "own" for a sync inside the app, or a ws:// address.
#[tauri::command]
async fn open(app: AppHandle, state: State<'_, App>, wallet: String, password: String, node: String) -> Result<Opened, String> {
    let (_, _, network_id) = probe().await?;
    let node_url = app::resolve_node(&node, network_id)?;
    let say_app = app.clone();
    let say: kaspa_cli_lib::serve::Say = Arc::new(move |line: String| {
        let _ = say_app.emit("say", strip_ansi(&line));
    });
    let options = SessionOptions {
        wallet: wallet.clone(),
        password: Secret::from(password),
        node: node_url,
        // A miner host is made whenever the wallet has a ledger address, so the
        // Status screen can start it; nothing mines until asked.
        mine: Some(50),
        network: Some(network_id),
        folder: None,
        origin: "desktop",
    };
    let session = open_session(&options, state.shutdown.clone(), say).await.map_err(|e| app::open_error_text(e.to_string(), &node))?;
    let access = app::access_words(&node).to_string();
    let opened = Opened {
        wallet,
        network: network_id.to_string(),
        own_node: session.own_node(),
        access,
        ticker: session.service.ticker().to_string(),
    };
    close_session(state.inner()).await;
    let service = session.service.clone();
    *state.session.lock().await = Some(session);
    *state.access.lock().unwrap() = node;
    start_background(state.inner(), service).await;
    Ok(opened)
}

#[tauri::command]
async fn close(state: State<'_, App>) -> Result<(), String> {
    close_session(state.inner()).await;
    Ok(())
}

#[tauri::command]
async fn backup_status(state: State<'_, App>) -> Result<BackupStatus, String> {
    with_service(&state, |s| async move { s.backup_status().await }).await
}

#[tauri::command]
async fn backup_now(state: State<'_, App>) -> Result<String, String> {
    with_service(&state, |s| async move { s.backup_now().await }).await
}

#[tauri::command]
async fn backup_decline(state: State<'_, App>) -> Result<(), String> {
    with_service(&state, |s| async move { s.backup_decline().await }).await
}

/// A wallet of this folder without a backup key yet, covered with its 24 words typed once.
#[tauri::command]
async fn backup_cover(state: State<'_, App>, name: String, words: String) -> Result<(), String> {
    with_service(&state, |s| async move { s.backup_cover(&name, &words).await }).await
}

/// The backups also go to a folder on this computer — one a cloud service
/// mirrors; an empty path turns it off. Writes the first full copy at once.
#[tauri::command]
async fn backup_folder(state: State<'_, App>, path: String) -> Result<String, String> {
    with_service(&state, |s| async move { s.backup_folder(&path).await }).await
}

/// Restores from a folder of backup files — the one a 'backup folder' keeps,
/// mirrored to this computer — the way the Telegram restore does, with the
/// words of the wallet wanted.
#[tauri::command]
async fn restore_folder(path: String, words: String, name: String) -> Result<String, String> {
    app::restore_from_folder(None, None, &path, &words, &name).await
}

#[tauri::command]
async fn backup_automatic(state: State<'_, App>, on: bool) -> Result<(), String> {
    with_service(&state, |s| async move { s.backup_automatic(on).await }).await
}

/// Writes the encrypted backup file into the Downloads folder (or home) and
/// says where; never over an existing file.
#[tauri::command]
async fn backup_file(app: AppHandle, state: State<'_, App>) -> Result<String, String> {
    let (name, bytes) = with_service(&state, |s| async move { s.backup_file().await }).await?;
    let dir = app.path().download_dir().or_else(|_| app.path().home_dir()).map_err(|e| e.to_string())?;
    let path = dir.join(name);
    if path.exists() {
        return Err(format!("{} already exists; a backup never overwrites one", path.display()));
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(&path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    file.write_all(&bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// Sets the wallet's bot up and starts answering it, so the pairing can happen.
#[tauri::command]
async fn telegram_setup(state: State<'_, App>, token: String, pin: String) -> Result<String, String> {
    let code = with_service(&state, |s| async move { s.telegram_setup(&token, &pin) }).await?;
    let service = state.session.lock().await.as_ref().map(|s| s.service.clone()).ok_or("no wallet is open")?;
    start_background(state.inner(), service).await;
    Ok(code)
}

/// Restores a wallet from the backups forwarded to its bot: the newest
/// checkpoint and the deltas after it. A backup holds every wallet of its
/// computer, each sealed with its own 24 words; the words given open one of
/// them, and the answer names the others. Progress goes out as "say" lines
/// while the parts arrive.
#[tauri::command]
async fn restore_telegram(app: AppHandle, token: String, words: String, name: String) -> Result<String, String> {
    let say_app = app.clone();
    let progress = move |line: String| {
        let _ = say_app.emit("say", line);
    };
    kaspa_cli_lib::app::restore_from_telegram(None, None, &token, &words, &name, &progress).await
}

/// What the updater found at the compiled-in address, if newer than this build.
#[derive(Serialize)]
struct UpdateInfo {
    version: String,
    notes: String,
}

/// Asks the update server (the project's site) whether a newer desktop
/// wallet is out. The answer is signed with the project's updater key, which
/// is compiled in; an unsigned or wrongly signed answer is an error here and
/// nothing is offered.
#[tauri::command]
async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => Ok(Some(UpdateInfo { version: update.version.clone(), notes: update.body.clone().unwrap_or_default() })),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Installs the update the check offered: the wallet is closed first the way
/// the Close button does it (its backup goes out), the installer is
/// downloaded with progress on the "update-progress" event, verified against
/// the compiled-in key, installed, and the app starts again.
#[tauri::command]
async fn install_update(app: AppHandle, state: State<'_, App>) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Err("there is no newer version to install".to_string());
    };
    close_session(state.inner()).await;
    let progress = app.clone();
    let mut got: u64 = 0;
    update
        .download_and_install(
            move |chunk, total| {
                got += chunk as u64;
                let _ = progress.emit("update-progress", serde_json::json!({ "got": got, "total": total }));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
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
        return Err("Mining through a public computer would tell its operator where your rewards go. Open the wallet with a network running on this computer or a sync of your own to mine.".to_string());
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
    let found = app::classify(&code)?;
    Ok(Classified { kind: found.kind, amount: found.amount })
}

/// `amount` is used only when the request pins none.
#[tauri::command]
async fn pay(state: State<'_, App>, code: String, amount: String) -> Result<PaidOut, String> {
    let chosen = app::optional_amount(&amount)?;
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
    let petals = app::amount(&amount)?;
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
    let petals = app::optional_amount(&amount)?;
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(App {
            session: tokio::sync::Mutex::new(None),
            shutdown: Arc::new(AtomicBool::new(false)),
            access: std::sync::Mutex::new(String::new()),
            bot: tokio::sync::Mutex::new(None),
            ticker: tokio::sync::Mutex::new(None),
            backup_state: Arc::new(std::sync::Mutex::new(AutoState::default())),
        })
        .invoke_handler(tauri::generate_handler![
            version,
            wallets,
            local_program_present,
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
            qr,
            backup_status,
            backup_now,
            backup_automatic,
            backup_decline,
            backup_cover,
            backup_folder,
            restore_folder,
            backup_file,
            telegram_setup,
            restore_telegram,
            check_update,
            install_update
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // Close the wallet properly before the window goes: a node
                // running inside the app is stopped and its database left
                // clean, the way 'exit' does it in the terminal wallet.
                let state: State<'_, App> = window.state();
                state.shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
                tauri::async_runtime::block_on(close_session(state.inner()));
            }
        })
        .run(tauri::generate_context!())
        .expect("the Marigold window could not be opened");
}
