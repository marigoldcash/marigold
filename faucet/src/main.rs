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
use kaspa_consensus_core::notepool::{DENOMINATION_PETALS, DenominationTag};
use kaspa_hashes::Hash;
use kaspa_wallet_core::account::Account;
use kaspa_wallet_core::account::notepool;
use kaspa_wallet_core::api::WalletApi;
use kaspa_wallet_core::prelude::*;
use kaspa_wallet_core::rpc::Rpc;
use kaspa_wallet_core::storage::NoteStatus;
use kaspa_wallet_core::storage::keydata::PrvKeyDataVariantKind;
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
/// One claim: 10 + 1 + 0.1 MAGLD, minted together — 11.10 decomposes to
/// exactly that shape, greedy largest-first.
///
/// It used to be 10 + two 0.01 stamps, the idea being that a fresh vault needs
/// stamps for its first rotations. But a 0.01 note costs 0.01 to rotate, so it
/// arrives worth precisely nothing (founder's call, 2026-09-07). A 1 and a 0.1
/// are both genuinely spendable AND serve the same bootstrapping purpose, since
/// rotating either yields change in smaller denominations.
///
/// 10 rather than 1 for the headline, per the founder's earlier call
/// (2026-09-04): enough to genuinely exercise split, merge and rotate.
const BUNDLE_MINT_PETALS: u64 = 1_110_000_000;
const BUNDLE_TAGS: [DenominationTag; 3] = [DenominationTag::D10, DenominationTag::D1, DenominationTag::D0_1];

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
        /// Origins allowed to call the API from a browser (the Telegram Mini App
        /// at marigold.cash). Repeatable.
        #[arg(long = "cors-origin", default_values_t = vec!["https://marigold.cash".to_string()])]
        cors_origins: Vec<String>,
        /// File holding the Telegram bot token (BotFather). With it, a claim
        /// from the Mini App is rate-limited per Telegram user instead of per
        /// IP — a phone on a carrier's NAT shares its address with thousands.
        /// Missing file: per-IP only, and a warning at start.
        #[arg(long)]
        telegram_bot_token_file: Option<std::path::PathBuf>,
        /// The Mini App the bot's /start answer opens.
        #[arg(long, default_value = "https://marigold.cash/app/")]
        mini_app_url: String,
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
    /// One queue per denomination in [`BUNDLE_TAGS`], same order. A claim takes
    /// the front of each, so a bundle is only "ready" when every queue has one.
    by_tag: [VecDeque<Hash>; BUNDLE_TAGS.len()],
}

impl ReadyNotes {
    fn new() -> Self {
        Self { by_tag: std::array::from_fn(|_| VecDeque::new()) }
    }

    /// How many complete bundles can be handed out: the shortest queue.
    fn bundles(&self) -> usize {
        self.by_tag.iter().map(|q| q.len()).min().unwrap_or(0)
    }
}

struct FaucetState {
    account: Arc<dyn Account>,
    wallet_secret: Secret,
    ready: Mutex<ReadyNotes>,
    confirmed: Mutex<HashSet<Hash>>,
    // "tg:<user id>" for a claim from the Mini App, "ip:<addr>" otherwise.
    last_claim: Mutex<HashMap<String, Instant>>,
    // (utc day number, claims so far that day)
    daily: Mutex<(u64, u64)>,
    claims_served: Mutex<u64>,
    per_ip_cooldown: Duration,
    daily_cap: u64,
    buffer_bundles: usize,
    cors_origins: Vec<String>,
    telegram_bot_token: Option<String>,
}

/// Who is claiming: the Telegram user behind a Mini App launch when the launch
/// data verifies against the bot token, else the IP. Telegram signs the launch
/// data (`initData`) with HMAC-SHA256 keyed by HMAC-SHA256("WebAppData", token),
/// so a valid signature is proof the user id came from Telegram, not from the
/// page.
struct Claimant {
    /// The rate-limit key: "tg:<user id>" or "ip:<addr>".
    key: String,
    /// Set when Telegram vouched for the user — the bot can then message them.
    telegram_user: Option<i64>,
}

fn claimant(state: &FaucetState, headers: &HeaderMap, ip: IpAddr) -> Claimant {
    if let (Some(token), Some(init)) =
        (state.telegram_bot_token.as_deref(), headers.get("x-telegram-init-data").and_then(|v| v.to_str().ok()))
    {
        if let Some(user_id) = telegram_user_id(init, token) {
            return Claimant { key: format!("tg:{user_id}"), telegram_user: Some(user_id) };
        }
    }
    Claimant { key: format!("ip:{ip}"), telegram_user: None }
}

// --- The bot -----------------------------------------------------------------
//
// Two things and no more: send a claimant their codes into their own chat, so
// they have them somewhere a wallet can read them from, and answer /start with
// the button that opens the Mini App. The Bot API takes every method as a GET
// with query parameters, which is what the workspace's HTTP client does.

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

async fn telegram_call(token: &str, method: &str, params: &[(&str, String)]) -> anyhow::Result<serde_json::Value> {
    let query = params.iter().map(|(k, v)| format!("{k}={}", url_encode(v))).collect::<Vec<_>>().join("&");
    let url = format!("https://api.telegram.org/bot{token}/{method}?{query}");
    let value: serde_json::Value = workflow_http::get_json(url).await.map_err(|e| anyhow::anyhow!("{method}: {e}"))?;
    if value.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        anyhow::bail!("{method}: {}", value.get("description").and_then(|d| d.as_str()).unwrap_or("not ok"));
    }
    Ok(value)
}

/// The claimed codes, into the claimant's own chat with the bot: one message,
/// each code tap-to-copy. Sent after the claim is answered; a failure is
/// logged, never shown — the page already has the notes.
async fn telegram_send_codes(token: String, user_id: i64, notes: Vec<(String, String)>) {
    let mut text = String::from(
        "Your notes. Each line is a bearer note: whoever holds it owns it, so take them into your wallet with <code>receive &lt;code&gt;</code>, the big one first.\n",
    );
    for (denomination, payload) in &notes {
        text.push_str(&format!("\n<b>{} MAGLD</b>\n<code>{}</code>\n", html_escape(denomination), html_escape(payload)));
    }
    let params = [("chat_id", user_id.to_string()), ("text", text), ("parse_mode", "HTML".to_string())];
    if let Err(e) = telegram_call(&token, "sendMessage", &params).await {
        log::warn!("could not send codes to Telegram user {user_id}: {e}");
    }
}

/// Answer /start (and anything else typed at the bot) with the button that
/// opens the Mini App. Long polling; this is the only process on the token.
async fn telegram_updates_loop(token: String, mini_app_url: String) {
    let mut offset: i64 = 0;
    loop {
        let params = [("offset", offset.to_string()), ("timeout", "25".to_string()), ("allowed_updates", "[\"message\"]".to_string())];
        let updates = match telegram_call(&token, "getUpdates", &params).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Telegram getUpdates: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let Some(list) = updates.get("result").and_then(|r| r.as_array()) else { continue };
        for update in list {
            if let Some(id) = update.get("update_id").and_then(|v| v.as_i64()) {
                offset = offset.max(id + 1);
            }
            let Some(message) = update.get("message") else { continue };
            let Some(chat_id) = message.get("chat").and_then(|c| c.get("id")).and_then(|v| v.as_i64()) else { continue };
            let text = message.get("text").and_then(|t| t.as_str()).unwrap_or("");
            let reply = if text.starts_with("/start") {
                "Marigold testnet faucet. Tap the button for three bearer notes — 10, 1 and 0.1 MAGLD. They are sent here as well, so your wallet can take them with <code>receive &lt;code&gt;</code>."
            } else {
                "Tap the button to take a note. Your wallet takes it with <code>receive &lt;code&gt;</code>."
            };
            let markup = serde_json::json!({ "inline_keyboard": [[{ "text": "Take a note", "web_app": { "url": mini_app_url } }]] })
                .to_string();
            let params = [
                ("chat_id", chat_id.to_string()),
                ("text", reply.to_string()),
                ("parse_mode", "HTML".to_string()),
                ("reply_markup", markup),
            ];
            if let Err(e) = telegram_call(&token, "sendMessage", &params).await {
                log::warn!("Telegram reply to {chat_id}: {e}");
            }
        }
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The Telegram user id in verified Mini App launch data, or None if the
/// signature does not check out or the data is older than a day.
fn telegram_user_id(init_data: &str, bot_token: &str) -> Option<i64> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut hash = None;
    for part in init_data.split('&') {
        let (k, v) = part.split_once('=')?;
        let (k, v) = (percent_decode(k), percent_decode(v));
        if k == "hash" {
            hash = Some(v);
        } else {
            pairs.push((k, v));
        }
    }
    let hash = hex::decode(hash?).ok()?;
    pairs.sort();
    let check = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n");
    let mut secret = Hmac::<Sha256>::new_from_slice(b"WebAppData").ok()?;
    secret.update(bot_token.as_bytes());
    let secret = secret.finalize().into_bytes();
    let mut mac = Hmac::<Sha256>::new_from_slice(&secret).ok()?;
    mac.update(check.as_bytes());
    mac.verify_slice(&hash).ok()?;
    let auth_date: u64 = pairs.iter().find(|(k, _)| k == "auth_date")?.1.parse().ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if now.saturating_sub(auth_date) > 86_400 {
        return None;
    }
    let user = pairs.iter().find(|(k, _)| k == "user")?;
    serde_json::from_str::<serde_json::Value>(&user.1).ok()?.get("id")?.as_i64()
}

fn utc_day() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

/// Rescan the vault + chain and refill the ready queues; mint when low.
async fn buffer_tick(state: &FaucetState) -> anyhow::Result<()> {
    let wallet = state.account.wallet();
    let note_key_store = wallet.store().as_note_key_store()?;

    // Active notes by denomination, from the vault (survives restarts).
    let mut active: [Vec<Hash>; BUNDLE_TAGS.len()] = std::array::from_fn(|_| Vec::new());
    let mut stream = note_key_store.iter().await?;
    while let Some(info) = stream.try_next().await? {
        if info.status == NoteStatus::Active {
            if let Some(slot) = BUNDLE_TAGS.iter().position(|t| *t == info.d) {
                active[slot].push(info.sn);
            }
        }
    }

    // Only hand out notes whose serial the chain already knows — the claimer's
    // `bearer_import` verifies on-chain and would reject an unconfirmed note.
    let unconfirmed: Vec<Hash> = {
        let confirmed = state.confirmed.lock().await;
        active.iter().flatten().filter(|sn| !confirmed.contains(sn)).copied().collect()
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
        for (slot, serials) in active.iter().enumerate() {
            ready.by_tag[slot] = serials.iter().filter(|sn| confirmed.contains(sn)).copied().collect();
        }
    }

    // Mint one bundle per tick while below target and funded (mint waits for
    // nothing: minted serials are known instantly; confirmation is picked up by
    // the next tick's get_notes_by_serial pass).
    let bundles_ready = state.ready.lock().await.bundles();
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
    let claimant = claimant(&state, &headers, ip);
    let who = claimant.key.clone();

    // Global daily cap.
    {
        let mut daily = state.daily.lock().await;
        let today = utc_day();
        if daily.0 != today {
            *daily = (today, 0);
        }
        if daily.1 >= state.daily_cap {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({"error": "The faucet reached its daily limit — try again tomorrow."})),
            )
                .into_response();
        }
        daily.1 += 1;
    }

    // Cooldown per claimant: a Telegram user from the Mini App, an IP otherwise.
    {
        let mut last = state.last_claim.lock().await;
        let now = Instant::now();
        if let Some(prev) = last.get(&who) {
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
        last.insert(who.clone(), now);
        last.retain(|_, t| now.duration_since(*t) < state.per_ip_cooldown * 2);
    }

    // Pop a bundle.
    let bundle: Vec<Hash> = {
        let mut ready = state.ready.lock().await;
        if ready.bundles() == 0 {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "The faucet is out of ready notes right now — more are being minted, try again in a minute."})),
            )
                .into_response();
        }
        // Largest first, so the page shows the 10 at the top — and so an
        // importer working top to bottom rotates the big note while its vault
        // still has the others to draw a fee from.
        ready.by_tag.iter_mut().map(|q| q.pop_front().unwrap()).collect()
    };

    // Hand them over. Every minted note sits on a solo Cold key, so this is
    // instant (no isolation transaction) — it just reveals the key and marks
    // the row HandedOver.
    let mut notes = Vec::new();
    for sn in bundle {
        match notepool::bearer_export(state.account.wallet(), state.wallet_secret.clone(), sn).await {
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
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "Internal faucet error — please report this."})),
                )
                    .into_response();
            }
        }
    }

    {
        let mut served = state.claims_served.lock().await;
        *served += 1;
    }
    log::info!("claim served to {who} ({} notes)", notes.len());
    if let (Some(user_id), Some(token)) = (claimant.telegram_user, state.telegram_bot_token.clone()) {
        let codes = notes.iter().map(|n| (n.denomination_magld.clone(), n.payload.clone())).collect();
        tokio::spawn(telegram_send_codes(token, user_id, codes));
    }

    Json(ClaimResponse {
        notes,
        message: "These are bearer notes: whoever holds the key owns them. Import them into your wallet NOW — anyone who sees these codes can take the money.".to_string(),
    })
    .into_response()
}

async fn status(State(state): State<Arc<FaucetState>>) -> impl IntoResponse {
    let ready = state.ready.lock().await;
    let bundles = ready.bundles();
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

/// Browser cross-origin rules for the Mini App at marigold.cash: the page
/// there may POST a claim here with its Telegram launch data. Origins not on
/// the list get no CORS headers, and the browser refuses on their behalf.
async fn cors(
    State(state): State<Arc<FaucetState>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let origin = req.headers().get("origin").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let mut resp = next.run(req).await;
    if let Some(origin) = origin.filter(|o| state.cors_origins.iter().any(|allowed| allowed == o)) {
        let h = resp.headers_mut();
        h.insert("access-control-allow-origin", origin.parse().expect("origin is a header value"));
        h.insert("access-control-allow-methods", "GET, POST, OPTIONS".parse().unwrap());
        h.insert("access-control-allow-headers", "content-type, x-telegram-init-data".parse().unwrap());
        h.insert("access-control-max-age", "600".parse().unwrap());
        h.insert("vary", "Origin".parse().unwrap());
    }
    resp
}

async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn cmd_serve(
    network: &str,
    wrpc_url: &str,
    listen: SocketAddr,
    buffer_bundles: usize,
    per_ip_cooldown_secs: u64,
    daily_cap: u64,
    cors_origins: Vec<String>,
    telegram_bot_token_file: Option<std::path::PathBuf>,
    mini_app_url: String,
) -> anyhow::Result<()> {
    let wallet_secret = wallet_secret_from_env();
    let telegram_bot_token = match telegram_bot_token_file {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(token) if !token.trim().is_empty() => {
                log::info!("Telegram bot token loaded; Mini App claims are rate-limited per Telegram user");
                Some(token.trim().to_string())
            }
            Ok(_) => {
                log::warn!("{} is empty — Mini App claims are rate-limited per IP", path.display());
                None
            }
            Err(err) => {
                log::warn!("cannot read {} ({err}) — Mini App claims are rate-limited per IP", path.display());
                None
            }
        },
        None => None,
    };
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
        ready: Mutex::new(ReadyNotes::new()),
        confirmed: Mutex::new(HashSet::new()),
        last_claim: Mutex::new(HashMap::new()),
        daily: Mutex::new((utc_day(), 0)),
        claims_served: Mutex::new(0),
        per_ip_cooldown: Duration::from_secs(per_ip_cooldown_secs),
        daily_cap,
        buffer_bundles,
        cors_origins,
        telegram_bot_token,
    });

    if let Some(token) = state.telegram_bot_token.clone() {
        tokio::spawn(telegram_updates_loop(token, mini_app_url));
        log::info!("Telegram bot answering /start with the Mini App button");
    }
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
        .route("/api/claim", post(claim).options(preflight))
        .route("/api/status", get(status).options(preflight))
        .layer(axum::middleware::from_fn_with_state(state.clone(), cors))
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
        Command::Serve {
            listen,
            buffer_bundles,
            per_ip_cooldown_secs,
            daily_cap,
            cors_origins,
            telegram_bot_token_file,
            mini_app_url,
        } => {
            cmd_serve(
                &cli.network,
                &cli.wrpc_url,
                listen,
                buffer_bundles,
                per_ip_cooldown_secs,
                daily_cap,
                cors_origins,
                telegram_bot_token_file,
                mini_app_url,
            )
            .await
        }
    }
}

#[cfg(test)]
mod telegram_tests {
    use super::telegram_user_id;

    // Vector computed with Python's hmac the way Telegram's docs specify:
    // secret = HMAC_SHA256("WebAppData", token); hash = HMAC_SHA256(secret,
    // sorted "k=v" pairs joined by newline, hash excluded). auth_date is in
    // 2100 so the freshness check does not expire the vector.
    const TOKEN: &str = "123456:TEST-TOKEN";
    const INIT: &str = "auth_date=4102444800&user=%7B%22id%22%3A4242%2C%22first_name%22%3A%22Test%22%7D&query_id=AAH&hash=f82dc9b3ce4adba9d4bf029727e681e78c2dfc993348f1550cc9837b0871baf3";

    #[test]
    fn verified_launch_data_yields_the_user_id() {
        assert_eq!(telegram_user_id(INIT, TOKEN), Some(4242));
    }

    #[test]
    fn a_wrong_token_or_a_tampered_field_is_refused() {
        assert_eq!(telegram_user_id(INIT, "123456:OTHER"), None);
        let tampered = INIT.replace("4242", "4243");
        assert_eq!(telegram_user_id(&tampered, TOKEN), None);
        assert_eq!(telegram_user_id("auth_date=1&user=x", TOKEN), None);
    }
}
