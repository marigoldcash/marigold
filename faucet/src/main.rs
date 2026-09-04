//! Marigold testnet faucet (LAUNCH-PLAN T7).
//!
//! Dispenses REAL bearer notes, not transparent balance — a claimer's first
//! contact with Marigold is the actual product: they receive `marigoldnote:`
//! payloads, import them, and their wallet rotates the locks. Each claim is a
//! bundle of one 1-MAGLD note plus two 0.01-MAGLD fee notes, so the recipient's
//! mandatory import-rotation runs in stamp mode and the headline note keeps its
//! denomination (see notepool.rs slack-mode docs and the integration test's
//! single-note-rotation assertions for why the fee notes matter).
//!
//! Shape: one long-lived wallet (local storage, its own vault), wRPC-borsh to a
//! local node, a background task that keeps a buffer of freshly minted notes
//! (every minted note is born on a solo Cold key, so `bearer_export` is
//! instant, free, and needs no isolation transaction), and a tiny axum server
//! bound to loopback behind nginx. Rate limits per IP plus a global daily cap;
//! no captcha until abuse shows up (testnet coins — the worst case is a refill).
//!
//! Usage:
//!   marigold-faucet init    # once: create the wallet, print mnemonic + address
//!   marigold-faucet serve   # run the faucet
//! The wallet secret comes from FAUCET_WALLET_SECRET (systemd EnvironmentFile).

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::{Parser, Subcommand};
use futures::TryStreamExt;
use kaspa_consensus_core::notepool::{DenominationTag, DENOMINATION_PETALS};
use kaspa_hashes::Hash;
use kaspa_wallet_core::account::notepool;
use kaspa_wallet_core::account::Account;
use kaspa_wallet_core::api::WalletApi;
use kaspa_wallet_core::prelude::*;
use kaspa_wallet_core::rpc::Rpc;
use kaspa_wallet_core::storage::keydata::PrvKeyDataVariantKind;
use kaspa_wallet_core::storage::{NoteStatus};
use kaspa_wallet_core::utils::sompi_to_kaspa_string;
use kaspa_wrpc_client::prelude::{ConnectOptions, ConnectStrategy, NetworkId};
use kaspa_wrpc_client::{KaspaRpcClient, WrpcEncoding};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use workflow_core::abortable::Abortable;

const WALLET_FILENAME: &str = "marigold-faucet";
/// One claim: 1 x 1-MAGLD + 2 x 0.01-MAGLD, minted together (1.02 MAGLD
/// decomposes to exactly that shape, greedy largest-first).
const BUNDLE_MINT_PETALS: u64 = 102_000_000;
const HEADLINE_TAG: DenominationTag = DenominationTag::D1;
const FEE_TAG: DenominationTag = DenominationTag::D0_01;
const FEE_NOTES_PER_BUNDLE: usize = 2;

#[derive(Parser)]
#[command(name = "marigold-faucet", about = "Marigold testnet faucet — dispenses real bearer notes")]
struct Cli {
    /// Network id the local node runs, e.g. testnet-10
    #[arg(long, default_value = "testnet-10")]
    network: String,
    /// wRPC (borsh) URL of the local node (ws:// scheme)
    #[arg(long, default_value = "ws://127.0.0.1:27210")]
    wrpc_url: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create the faucet wallet (once); prints the mnemonic and funding address
    Init,
    /// Run the faucet service
    Serve {
        /// HTTP listen address — keep on loopback, nginx fronts it
        #[arg(long, default_value = "127.0.0.1:8590")]
        listen: SocketAddr,
        /// Bundles to keep pre-minted and confirmed, ready to hand out
        #[arg(long, default_value_t = 10)]
        buffer_bundles: usize,
        /// Seconds an IP must wait between claims
        #[arg(long, default_value_t = 3600)]
        per_ip_cooldown_secs: u64,
        /// Maximum claims per UTC day across all IPs
        #[arg(long, default_value_t = 500)]
        daily_cap: u64,
    },
}

fn wallet_secret_from_env() -> Secret {
    match std::env::var("FAUCET_WALLET_SECRET") {
        Ok(s) if !s.is_empty() => Secret::from(s),
        _ => {
            eprintln!("FAUCET_WALLET_SECRET must be set (the faucet wallet's password)");
            std::process::exit(2);
        }
    }
}

async fn connect_wallet(network: &str, wrpc_url: &str) -> anyhow::Result<Arc<Wallet>> {
    let network_id = NetworkId::from_str(network).map_err(|e| anyhow::anyhow!("bad --network: {e}"))?;
    let client = Arc::new(KaspaRpcClient::new_with_args(WrpcEncoding::Borsh, Some(wrpc_url), None, Some(network_id), None)?);
    let rpc = Rpc::new(client.clone(), client.ctl().clone());
    let wallet = Arc::new(Wallet::try_with_rpc(Some(rpc), Wallet::local_store()?, Some(network_id))?);
    // Fallback strategy: fail fast on a bad URL/downed node instead of the
    // default silent retry-forever (which reads as a hang).
    let options = ConnectOptions { block_async_connect: true, strategy: ConnectStrategy::Fallback, ..Default::default() };
    client.connect(Some(options)).await.map_err(|e| anyhow::anyhow!("wRPC connect to {wrpc_url} failed: {e}"))?;
    wallet.clone().start().await?;
    Ok(wallet)
}

async fn cmd_init(network: &str, wrpc_url: &str) -> anyhow::Result<()> {
    let wallet_secret = wallet_secret_from_env();
    let wallet = connect_wallet(network, wrpc_url).await?;

    let args = WalletCreateArgs {
        title: Some("Marigold faucet".to_string()),
        filename: Some(WALLET_FILENAME.to_string()),
        encryption_kind: EncryptionKind::XChaCha20Poly1305,
        user_hint: None,
        overwrite_wallet_storage: false,
    };
    wallet.clone().wallet_create(wallet_secret.clone(), args).await?;

    let mnemonic = Mnemonic::random(WordCount::Words12, Language::default())?;
    let prv_key_data_id = wallet
        .clone()
        .prv_key_data_create(
            wallet_secret.clone(),
            PrvKeyDataCreateArgs::new(None, None, Secret::from(mnemonic.phrase()), PrvKeyDataVariantKind::Mnemonic),
        )
        .await?;
    let descriptor =
        wallet.clone().accounts_create(wallet_secret.clone(), AccountCreateArgs::new_bip32(prv_key_data_id, None, None, None)).await?;
    wallet.clone().accounts_activate(Some(vec![descriptor.account_id])).await?;
    let account = wallet.active_accounts().get(&descriptor.account_id).expect("account active after activate");

    println!("faucet wallet created (storage file: {WALLET_FILENAME})");
    println!();
    println!("RECOVERY MNEMONIC — record it, it is shown exactly once:");
    println!("  {}", mnemonic.phrase_string());
    println!();
    println!("Fund this address (the faucet mints notes from its transparent balance):");
    println!("  {}", account.receive_address()?);
    Ok(())
}

struct ReadyNotes {
    headline: VecDeque<Hash>,
    fee: VecDeque<Hash>,
}

struct FaucetState {
    account: Arc<dyn Account>,
    wallet_secret: Secret,
    ready: Mutex<ReadyNotes>,
    confirmed: Mutex<HashSet<Hash>>,
    last_claim_by_ip: Mutex<HashMap<IpAddr, Instant>>,
    // (utc day number, claims so far that day)
    daily: Mutex<(u64, u64)>,
    claims_served: Mutex<u64>,
    per_ip_cooldown: Duration,
    daily_cap: u64,
    buffer_bundles: usize,
}

fn utc_day() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

/// Rescan the vault + chain and refill the ready queues; mint when low.
async fn buffer_tick(state: &FaucetState) -> anyhow::Result<()> {
    let wallet = state.account.wallet();
    let note_key_store = wallet.store().as_note_key_store()?;

    // Active notes by denomination, from the vault (survives restarts).
    let mut active_headline: Vec<Hash> = Vec::new();
    let mut active_fee: Vec<Hash> = Vec::new();
    let mut stream = note_key_store.iter().await?;
    while let Some(info) = stream.try_next().await? {
        if info.status == NoteStatus::Active {
            match info.d {
                d if d == HEADLINE_TAG => active_headline.push(info.sn),
                d if d == FEE_TAG => active_fee.push(info.sn),
                _ => {}
            }
        }
    }

    // Only hand out notes whose serial the chain already knows — the claimer's
    // `bearer_import` verifies on-chain and would reject an unconfirmed note.
    let unconfirmed: Vec<Hash> = {
        let confirmed = state.confirmed.lock().await;
        active_headline.iter().chain(active_fee.iter()).filter(|sn| !confirmed.contains(sn)).copied().collect()
    };
    if !unconfirmed.is_empty() {
        let on_chain = wallet.rpc_api().get_notes_by_serial(unconfirmed).await?;
        let mut confirmed = state.confirmed.lock().await;
        for entry in on_chain {
            confirmed.insert(entry.sn);
        }
    }

    {
        let confirmed = state.confirmed.lock().await;
        let mut ready = state.ready.lock().await;
        ready.headline = active_headline.iter().filter(|sn| confirmed.contains(sn)).copied().collect();
        ready.fee = active_fee.iter().filter(|sn| confirmed.contains(sn)).copied().collect();
    }

    // Mint one bundle per tick while below target and funded (mint waits for
    // nothing: minted serials are known instantly; confirmation is picked up by
    // the next tick's get_notes_by_serial pass).
    let (ready_headline, ready_fee) = {
        let ready = state.ready.lock().await;
        (ready.headline.len(), ready.fee.len())
    };
    let bundles_ready = ready_headline.min(ready_fee / FEE_NOTES_PER_BUNDLE);
    if bundles_ready < state.buffer_bundles {
        let mature = state.account.balance().map(|b| b.mature).unwrap_or(0);
        if mature > BUNDLE_MINT_PETALS * 2 {
            let abortable = Abortable::default();
            let result =
                notepool::mint(state.account.clone(), state.wallet_secret.clone(), None, BUNDLE_MINT_PETALS, None, &abortable).await?;
            log::info!(
                "minted bundle ({} notes, tx {})",
                result.notes.len(),
                result.transaction_ids.last().map(|h| h.to_string()).unwrap_or_default()
            );
        } else {
            log::warn!(
                "buffer low ({bundles_ready}/{} bundles) but mature balance is only {} — fund the faucet wallet",
                state.buffer_bundles,
                sompi_to_kaspa_string(mature)
            );
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct ClaimNote {
    denomination_magld: String,
    payload: String,
    qr_svg: String,
}

#[derive(Serialize)]
struct ClaimResponse {
    notes: Vec<ClaimNote>,
    message: String,
}

fn qr_svg(text: &str) -> String {
    use qrcode::render::svg;
    match qrcode::QrCode::new(text.as_bytes()) {
        Ok(code) => code.render::<svg::Color<'_>>().min_dimensions(180, 180).quiet_zone(true).build(),
        Err(_) => String::new(),
    }
}

fn client_ip(headers: &HeaderMap, connect: IpAddr) -> IpAddr {
    // Behind nginx (loopback), trust X-Real-IP; direct hits fall back to the
    // socket address. Only ever trusted from loopback by construction — the
    // service refuses to bind anything else without an explicit flag change.
    if connect.is_loopback() {
        if let Some(v) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            if let Ok(ip) = IpAddr::from_str(v.trim()) {
                return ip;
            }
        }
    }
    connect
}

async fn claim(
    State(state): State<Arc<FaucetState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ip = client_ip(&headers, peer.ip());

    // Global daily cap.
    {
        let mut daily = state.daily.lock().await;
        let today = utc_day();
        if daily.0 != today {
            *daily = (today, 0);
        }
        if daily.1 >= state.daily_cap {
            return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({"error": "The faucet reached its daily limit — try again tomorrow."}))).into_response();
        }
        daily.1 += 1;
    }

    // Per-IP cooldown.
    {
        let mut last = state.last_claim_by_ip.lock().await;
        let now = Instant::now();
        if let Some(prev) = last.get(&ip) {
            let elapsed = now.duration_since(*prev);
            if elapsed < state.per_ip_cooldown {
                let wait = (state.per_ip_cooldown - elapsed).as_secs().max(1);
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({"error": format!("Easy there — one claim per hour per address. Try again in {} minutes.", wait.div_ceil(60))})),
                )
                    .into_response();
            }
        }
        last.insert(ip, now);
        last.retain(|_, t| now.duration_since(*t) < state.per_ip_cooldown * 2);
    }

    // Pop a bundle.
    let (headline, fees) = {
        let mut ready = state.ready.lock().await;
        if ready.headline.is_empty() || ready.fee.len() < FEE_NOTES_PER_BUNDLE {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "The faucet is out of ready notes right now — more are being minted, try again in a minute."})),
            )
                .into_response();
        }
        let headline = ready.headline.pop_front().unwrap();
        let fees: Vec<Hash> = (0..FEE_NOTES_PER_BUNDLE).map(|_| ready.fee.pop_front().unwrap()).collect();
        (headline, fees)
    };

    // Hand them over. Every minted note sits on a solo Cold key, so this is
    // instant (no isolation transaction) — it just reveals the key and marks
    // the row HandedOver.
    let mut notes = Vec::new();
    for sn in std::iter::once(headline).chain(fees) {
        match notepool::bearer_export(state.account.clone(), state.wallet_secret.clone(), sn).await {
            Ok(result) => {
                let payload = result.bearer.to_text();
                notes.push(ClaimNote {
                    denomination_magld: sompi_to_kaspa_string(DENOMINATION_PETALS[result.bearer.d as usize]),
                    qr_svg: qr_svg(&payload),
                    payload,
                });
            }
            Err(e) => {
                log::error!("bearer_export of {sn} failed mid-claim: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Internal faucet error — please report this."})))
                    .into_response();
            }
        }
    }

    {
        let mut served = state.claims_served.lock().await;
        *served += 1;
    }
    log::info!("claim served to {ip} ({} notes)", notes.len());

    Json(ClaimResponse {
        notes,
        message: "These are bearer notes: whoever holds the key owns them. Import them into your wallet NOW — anyone who sees these codes can take the money.".to_string(),
    })
    .into_response()
}

async fn status(State(state): State<Arc<FaucetState>>) -> impl IntoResponse {
    let ready = state.ready.lock().await;
    let bundles = ready.headline.len().min(ready.fee.len() / FEE_NOTES_PER_BUNDLE);
    let mature = state.account.balance().map(|b| b.mature).unwrap_or(0);
    Json(serde_json::json!({
        "ready_bundles": bundles,
        "claims_served": *state.claims_served.lock().await,
        "wallet_mature_balance_magld": sompi_to_kaspa_string(mature),
    }))
}

async fn index() -> Html<&'static str> {
    Html(include_str!("page.html"))
}

async fn cmd_serve(
    network: &str,
    wrpc_url: &str,
    listen: SocketAddr,
    buffer_bundles: usize,
    per_ip_cooldown_secs: u64,
    daily_cap: u64,
) -> anyhow::Result<()> {
    let wallet_secret = wallet_secret_from_env();
    let wallet = connect_wallet(network, wrpc_url).await?;

    let descriptors = wallet
        .clone()
        .wallet_open(wallet_secret.clone(), Some(WALLET_FILENAME.to_string()), true, false)
        .await?
        .ok_or_else(|| anyhow::anyhow!("wallet '{WALLET_FILENAME}' has no accounts — run `marigold-faucet init` first"))?;
    let account_id = descriptors.first().ok_or_else(|| anyhow::anyhow!("wallet has no accounts"))?.account_id;
    wallet.clone().accounts_activate(Some(vec![account_id])).await?;
    let account = wallet.active_accounts().get(&account_id).expect("account active after activate");

    log::info!("faucet wallet open; funding address {}", account.receive_address()?);

    let state = Arc::new(FaucetState {
        account,
        wallet_secret,
        ready: Mutex::new(ReadyNotes { headline: VecDeque::new(), fee: VecDeque::new() }),
        confirmed: Mutex::new(HashSet::new()),
        last_claim_by_ip: Mutex::new(HashMap::new()),
        daily: Mutex::new((utc_day(), 0)),
        claims_served: Mutex::new(0),
        per_ip_cooldown: Duration::from_secs(per_ip_cooldown_secs),
        daily_cap,
        buffer_bundles,
    });

    let buffer_state = state.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = buffer_tick(&buffer_state).await {
                log::warn!("buffer tick failed: {e}");
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/api/claim", post(claim))
        .route("/api/status", get(status))
        .with_state(state);

    log::info!("faucet listening on http://{listen}");
    let listener = tokio::net::TcpListener::bind(listen).await?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    kaspa_core::log::try_init_logger("info");
    let cli = Cli::parse();
    match cli.command {
        Command::Init => cmd_init(&cli.network, &cli.wrpc_url).await,
        Command::Serve { listen, buffer_bundles, per_ip_cooldown_secs, daily_cap } => {
            cmd_serve(&cli.network, &cli.wrpc_url, listen, buffer_bundles, per_ip_cooldown_secs, daily_cap).await
        }
    }
}
