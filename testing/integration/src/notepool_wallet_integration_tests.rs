//! FORK-PLAN P7.2's verify criterion: "on local testnet: mint from mined funds, redeem
//! back, transparent balance reconciles minus fees" — proven here against a genuine
//! live daemon and a real [`kaspa_wallet_core::wallet::Wallet`] instance, using the same
//! non-interactive `WalletApi` bootstrap path the CLI/wasm bindings use (not a hand-rolled
//! storage shortcut).
//!
//! The wallet is connected over wRPC (`KaspaRpcClient`), not gRPC: `Wallet`'s
//! `UtxoProcessor` only wires its connect-state and `UtxosChanged` listener machinery
//! through `RpcCtl`, which `GrpcClient` does not drive the way `KaspaRpcClient` does.
//! `common::daemon::ClientManager` already configures a wRPC-borsh listener
//! (`rpc_borsh_port`) alongside the daemon's gRPC listener; this test connects to that,
//! while still using a plain `GrpcClient` (`Daemon::start`'s return value) to mine blocks,
//! mirroring every other daemon test in `daemon_integration_tests.rs`.

use crate::common::daemon::Daemon;
use crate::common::utils::wait_for;
use kaspa_addresses::Version;
use kaspa_alloc::init_allocator_with_default_settings;
use kaspa_consensus::params::SIMNET_PARAMS;
use kaspa_consensus_core::notepool::{DENOMINATION_PETALS, DenominationTag};
use kaspa_hashes::Hash;
use kaspa_rpc_core::api::rpc::RpcApi;
use kaspa_wallet_core::account::notepool::RedeemSelection;
use kaspa_wallet_core::prelude::*;
use kaspa_wallet_core::rpc::Rpc;
use kaspa_wallet_core::storage::keydata::PrvKeyDataVariantKind;
use kaspa_wallet_core::storage::{NoteProvenance, NoteStatus};
use kaspad_lib::args::Args;
use std::sync::Arc;
use workflow_core::abortable::Abortable;

/// Mint 1.11 MAGLD (111,000,000 petals) — deliberately not a single-denomination
/// amount, so it decomposes (P1.6 ladder, greedy largest-first) into D1 + D0_1 + D0_01,
/// exercising more than one denomination in a single mint.
const MINT_AMOUNT_PETALS: u64 = 111_000_000;

/// Connect a fresh resident `Wallet` to the daemon over wRPC and bootstrap a BIP32
/// account through the same non-interactive `WalletApi` calls the real CLI/wasm
/// bindings use (see `wallet_notepool_mint_redeem_test`'s comments for why wRPC and
/// why this path). Shared by every notepool wallet test; P7.3+ tests need several
/// wallets against one daemon.
async fn connect_and_bootstrap_wallet(kaspad: &Daemon, wallet_secret: &Secret) -> (Arc<Wallet>, Arc<dyn kaspa_wallet_core::account::Account>) {
    let wrpc_client = Arc::new(kaspad.new_wrpc_client());
    let rpc_ctl = wrpc_client.ctl().clone();
    let rpc_api: Arc<DynRpcApi> = wrpc_client.clone();
    let rpc = Rpc::new(rpc_api, rpc_ctl);
    let wallet = Arc::new(
        Wallet::try_with_rpc(Some(rpc), Wallet::resident_store().expect("resident store"), Some(kaspad.network))
            .expect("failed to construct Wallet"),
    );

    wrpc_client.connect(None).await.expect("wallet wRPC client failed to connect to the daemon");
    wallet.start().await.expect("wallet task failed to start");

    wallet
        .clone()
        .wallet_create(
            wallet_secret.clone(),
            WalletCreateArgs {
                title: None,
                filename: None,
                encryption_kind: EncryptionKind::XChaCha20Poly1305,
                user_hint: None,
                overwrite_wallet_storage: true,
            },
        )
        .await
        .expect("wallet_create failed");

    let mnemonic = Mnemonic::random(WordCount::Words12, Language::default()).expect("mnemonic generation failed");
    let prv_key_data_id = wallet
        .clone()
        .prv_key_data_create(
            wallet_secret.clone(),
            PrvKeyDataCreateArgs::new(None, None, Secret::from(mnemonic.phrase()), PrvKeyDataVariantKind::Mnemonic),
        )
        .await
        .expect("prv_key_data_create failed");

    let account_descriptor = wallet
        .clone()
        .accounts_create(wallet_secret.clone(), AccountCreateArgs::new_bip32(prv_key_data_id, None, None, None))
        .await
        .expect("accounts_create failed");
    let account_id = account_descriptor.account_id;

    // Activating the account (rather than just selecting it) mirrors `WalletApi`'s own
    // documented activation flow: it performs the initial UTXO discovery scan and
    // registers the account's address window for `UtxosChanged` notifications.
    wallet.clone().accounts_activate(Some(vec![account_id])).await.expect("accounts_activate failed");
    let account = wallet.active_accounts().get(&account_id).expect("account should be active after accounts_activate");

    (wallet, account)
}

/// `cargo test --release --package kaspa-testing-integration --lib -- notepool_wallet_integration_tests::wallet_notepool_mint_redeem_test --ignored --nocapture`
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn wallet_notepool_mint_redeem_test() {
    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO,kaspa_testing_integration=trace,kaspa_wallet_core=debug");

    let args =
        Args { simnet: true, unsafe_rpc: true, enable_unsynced_mining: true, disable_upnp: true, utxoindex: true, ..Default::default() };
    let total_fd_limit = 10;
    let mut kaspad = Daemon::new_random_with_args(args, total_fd_limit);
    // `miner_client` plays the role every other daemon test's `rpc_client1` plays: submit
    // blocks/transactions "from the network side". It is intentionally NOT what the
    // `Wallet` connects with (see module doc comment).
    let miner_client = kaspad.start().await;

    // --- Connect a real `Wallet` to the daemon over wRPC and bootstrap an account ---
    let wallet_secret = Secret::from("test-wallet-password");
    let (wallet, account) = connect_and_bootstrap_wallet(&kaspad, &wallet_secret).await;

    let receive_address = account.receive_address().expect("account should have a receive address");
    println!("wallet account receive address: {receive_address}");

    // Every subsequent "mine N blocks to bury this transaction" round needs *some*
    // address to pay the block reward to, but must NOT pay it to the wallet's own
    // receive address: this account's balance is being diffed before/after mint and
    // redeem, and a coinbase reward landing in the same account on every confirmation
    // round would swamp the (much smaller) mint/redeem deltas being asserted on. Only
    // the initial funding round below intentionally pays the wallet.
    let miner_reward_address = Address::new(kaspad.network.into(), Version::PubKey, &[7u8; 32]);

    // --- Mine a mature coinbase straight to the account's receive address ---
    //
    // Exactly ONE block goes to the wallet's own address; the remaining
    // `coinbase_maturity` blocks needed to mature it go to the throwaway
    // `miner_reward_address` instead. Mining all of them to `receive_address` would
    // leave hundreds of coinbases sitting in the wallet's own `pending`/`stasis`
    // state at the moment `balance_before_mint` is captured below (the wait_for only
    // waits for *some* mature balance, not for every mined block to mature) — those
    // would keep trickling into `mature` on their own during later confirmation
    // mining, independent of anything the mint/redeem calls do, silently masking a
    // real balance drop behind unrelated ongoing maturation. One tracked coinbase
    // keeps the balance delta attributable to mint/redeem alone.
    let coinbase_maturity = SIMNET_PARAMS.coinbase_maturity();
    let template = miner_client.get_block_template(receive_address.clone(), vec![]).await.unwrap();
    miner_client.submit_block(template.block, false).await.unwrap();
    for _ in 0..coinbase_maturity + 20 {
        let template = miner_client.get_block_template(miner_reward_address.clone(), vec![]).await.unwrap();
        miner_client.submit_block(template.block, false).await.unwrap();
    }

    // Wait for the wallet's UtxoProcessor (subscribed for UtxosChanged over the wRPC
    // connection established above) to observe the matured balance.
    let poll_account = account.clone();
    wait_for(
        200,
        150,
        move || {
            let account = poll_account.clone();
            Box::pin(async move { account.balance().map(|b| b.mature).unwrap_or(0) > 0 })
        },
        "wallet did not observe a mature transparent balance after mining",
    )
    .await;

    let balance_before_mint = account.balance().expect("balance should be populated by now").mature;
    assert!(balance_before_mint > 0, "expected a mature transparent balance from mining");
    println!("transparent balance before mint: {balance_before_mint} sompi");

    // Independent (daemon-side) confirmation that the pool starts empty, so later
    // `get_pool_stats()` deltas are unambiguous.
    let stats_before_mint = miner_client.get_pool_stats().await.unwrap();
    assert_eq!(stats_before_mint[DenominationTag::D1 as usize], 0);
    assert_eq!(stats_before_mint[DenominationTag::D0_1 as usize], 0);
    assert_eq!(stats_before_mint[DenominationTag::D0_01 as usize], 0);

    // --- Mint: 1.11 MAGLD -> D1 + D0_1 + D0_01 ---
    let abortable = Abortable::default();
    let mint_result = account
        .clone()
        .mint(wallet_secret.clone(), None, MINT_AMOUNT_PETALS, None, &abortable)
        .await
        .expect("mint failed");

    assert_eq!(mint_result.notes.len(), 3, "1.11 MAGLD should decompose into exactly 3 notes (D1 + D0_1 + D0_01)");
    let mut denominations: Vec<DenominationTag> = mint_result.notes.iter().map(|n| n.d).collect();
    denominations.sort();
    assert_eq!(denominations, vec![DenominationTag::D0_01, DenominationTag::D0_1, DenominationTag::D1]);
    for note in &mint_result.notes {
        assert_eq!(note.provenance, NoteProvenance::Cold, "wallet-generated note keys must be Cold provenance");
    }
    let mint_serials: Vec<Hash> = mint_result.notes.iter().map(|n| n.sn).collect();
    println!("mint tx(s): {:?}, serials: {:?}", mint_result.transaction_ids, mint_serials);

    // Mine a few more blocks (to a throwaway address, not the wallet's own -- see
    // `miner_reward_address`'s doc comment) to confirm the mint transaction.
    for _ in 0..10 {
        let template = miner_client.get_block_template(miner_reward_address.clone(), vec![]).await.unwrap();
        miner_client.submit_block(template.block, false).await.unwrap();
    }

    // Independent (daemon-side) check: the pool now holds exactly these three notes.
    let stats_after_mint = miner_client.get_pool_stats().await.unwrap();
    assert_eq!(stats_after_mint[DenominationTag::D1 as usize], 1);
    assert_eq!(stats_after_mint[DenominationTag::D0_1 as usize], 1);
    assert_eq!(stats_after_mint[DenominationTag::D0_01 as usize], 1);

    // The note key store persisted every minted note as Cold/Active.
    let note_key_store = account.wallet().store().as_note_key_store().expect("note key store");
    for note in &mint_result.notes {
        let info = note_key_store.load_info(&note.sn).await.unwrap().expect("minted serial should be in the note key store");
        assert_eq!(info.provenance, NoteProvenance::Cold);
        assert_eq!(info.status, NoteStatus::Active);
        assert_eq!(info.d, note.d);
    }

    // Wait for the transparent balance to reflect the mint.
    let poll_account = account.clone();
    wait_for(
        200,
        150,
        move || {
            let account = poll_account.clone();
            Box::pin(async move { account.balance().map(|b| b.mature).unwrap_or(u64::MAX) < balance_before_mint })
        },
        "transparent balance did not drop after mint",
    )
    .await;
    let balance_after_mint = account.balance().unwrap().mature;
    println!("transparent balance after mint: {balance_after_mint} sompi");
    assert!(balance_after_mint < balance_before_mint, "mint must reduce the transparent balance");
    let mint_cost = balance_before_mint - balance_after_mint;
    // Fees::SenderPays(amount_petals) (see account::notepool's doc comment) adds
    // amount_petals on top of the real, mass-computed network fee, so the balance drop
    // is exactly (minted value + real fee) -- never less than the minted value, and the
    // real fee on this tiny transaction (1 input, 0 real outputs, small payload) should
    // stay well under 0.01 MAGLD.
    assert!(mint_cost >= MINT_AMOUNT_PETALS, "mint must withhold at least the minted value from change");
    let mint_fee = mint_cost - MINT_AMOUNT_PETALS;
    println!("mint real network fee: {mint_fee} sompi");
    assert!(mint_fee < 1_000_000, "mint fee ({mint_fee} sompi) is unexpectedly large for a single-input mint");

    // --- Redeem: hand back the exact serials just minted ---
    let redeem_result = account
        .clone()
        .redeem(wallet_secret.clone(), RedeemSelection::Serials(mint_serials.clone()))
        .await
        .expect("redeem failed");
    assert_eq!(redeem_result.redeemed_value_petals, MINT_AMOUNT_PETALS);
    assert_eq!(redeem_result.serials, mint_serials);
    println!(
        "redeem tx: {}, redeemed_value_petals: {}, fee_sompi: {}",
        redeem_result.transaction_id, redeem_result.redeemed_value_petals, redeem_result.fee_petals
    );

    // `redeem()` marks the redeemed serials Superseded immediately on successful RPC
    // submission (see account::notepool::redeem's doc comment) -- no need to wait for
    // confirmation for this assertion.
    for sn in &mint_serials {
        let info = note_key_store.load_info(sn).await.unwrap().expect("redeemed serial should still exist (tombstoned)");
        assert_eq!(info.status, NoteStatus::Superseded);
    }

    // Mine enough blocks (to the same throwaway address as above) to confirm the
    // redeem transaction *and* clear the wallet's ordinary (non-outgoing) UTXO
    // maturity delay. Unlike mint's change output -- which the wallet force-matures
    // immediately because `redeem()` bypasses `Generator`/`PendingTransaction` and so
    // never registers itself as an "outgoing" transaction (see account::notepool's doc
    // comment on why redeem is hand-built) -- the wallet has no way to know the
    // redeem payout is its own doing, so it treats the new output like any external
    // deposit: `UtxoEntryReferenceExtension::maturity`'s ordinary (non-coinbase)
    // branch, gated on `user_transaction_maturity_period_daa` (100 DAA-score units by
    // default, `wallet/core/src/utxo/settings.rs`), not the ~10 blocks originally
    // mined here.
    let user_maturity = wallet.utxo_processor().network_params().unwrap().user_transaction_maturity_period_daa();
    for _ in 0..user_maturity + 20 {
        let template = miner_client.get_block_template(miner_reward_address.clone(), vec![]).await.unwrap();
        miner_client.submit_block(template.block, false).await.unwrap();
    }

    // Independent (daemon-side) check: the pool is back to empty for these denominations.
    let stats_after_redeem = miner_client.get_pool_stats().await.unwrap();
    assert_eq!(stats_after_redeem[DenominationTag::D1 as usize], 0);
    assert_eq!(stats_after_redeem[DenominationTag::D0_1 as usize], 0);
    assert_eq!(stats_after_redeem[DenominationTag::D0_01 as usize], 0);

    let poll_account = account.clone();
    wait_for(
        200,
        150,
        move || {
            let account = poll_account.clone();
            Box::pin(async move { account.balance().map(|b| b.mature).unwrap_or(0) > balance_after_mint })
        },
        "transparent balance did not rise after redeem",
    )
    .await;
    let balance_after_redeem = account.balance().unwrap().mature;
    println!("transparent balance after redeem: {balance_after_redeem} sompi");
    let redeemed_gain = balance_after_redeem - balance_after_mint;
    assert_eq!(
        redeemed_gain,
        redeem_result.redeemed_value_petals - redeem_result.fee_petals,
        "transparent balance must rise by exactly (redeemed value - real fee)"
    );

    // --- Full reconciliation: net balance change across mint + redeem is exactly
    // -(mint_fee + redeem_fee) -- the pool round-trip cost nothing but the two real
    // network fees. ---
    let net_change = balance_before_mint - balance_after_redeem;
    assert_eq!(net_change, mint_fee + redeem_result.fee_petals, "net balance change should equal exactly the two real network fees");
    println!(
        "reconciliation: before={balance_before_mint} after_mint={balance_after_mint} (fee {mint_fee}) after_redeem={balance_after_redeem} (fee {}) net_change={net_change}",
        redeem_result.fee_petals
    );

    if let Some(client) = wallet.try_wrpc_client() {
        client.disconnect().await.ok();
    }
    miner_client.disconnect().await.unwrap();
    kaspad.shutdown();
}

/// FORK-PLAN P7.3's verify criterion: "both flows succeed on local testnet between two
/// wallet instances; imported key is never left unrotated after confirmation." Two
/// real `Wallet` instances (A = payer, B = receiver) against one live daemon:
///
/// (a) bearer import — A hands one note's `(sn, sk, d)` to B via the text/QR payload;
///     B verifies the serial's on-chain pk against the handed-over key, stores it Hot,
///     and immediately rotates it to fresh Cold keys (slack mode here: B holds nothing
///     else, so the fee comes out of the rotated value itself — the bootstrap case).
///
/// (b) sign-to-fresh-pk — B creates a pinned-amount payment request (persisted before
///     display), A pays it with exact denominations plus a fee-stamp note, and B
///     claims the landed serials via its `NotesChanged` pk-subscription (which fires
///     on *confirmation*, so arrival is settlement).
///
/// `cargo test --release --package kaspa-testing-integration --lib -- notepool_wallet_integration_tests::wallet_notepool_receive_flows_test --ignored --nocapture`
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn wallet_notepool_receive_flows_test() {
    use kaspa_wallet_core::account::notepool::{BearerNote, PaymentRequest, await_payment_request, create_payment_request};
    use kaspa_wallet_core::storage::NoteProvenance;

    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO,kaspa_testing_integration=trace");

    let args =
        Args { simnet: true, unsafe_rpc: true, enable_unsynced_mining: true, disable_upnp: true, utxoindex: true, ..Default::default() };
    let total_fd_limit = 10;
    let mut kaspad = Daemon::new_random_with_args(args, total_fd_limit);
    let miner_client = kaspad.start().await;

    let secret_a = Secret::from("payer-wallet-password");
    let secret_b = Secret::from("receiver-wallet-password");
    let (wallet_a, account_a) = connect_and_bootstrap_wallet(&kaspad, &secret_a).await;
    let (wallet_b, account_b) = connect_and_bootstrap_wallet(&kaspad, &secret_b).await;

    let receive_a = account_a.receive_address().expect("payer receive address");
    let throwaway = Address::new(kaspad.network.into(), Version::PubKey, &[7u8; 32]);

    // Fund A: one tracked coinbase, then maturity blocks to the throwaway address
    // (same reasoning as the mint/redeem test's funding comment).
    let coinbase_maturity = SIMNET_PARAMS.coinbase_maturity();
    let template = miner_client.get_block_template(receive_a.clone(), vec![]).await.unwrap();
    miner_client.submit_block(template.block, false).await.unwrap();
    for _ in 0..coinbase_maturity + 20 {
        let template = miner_client.get_block_template(throwaway.clone(), vec![]).await.unwrap();
        miner_client.submit_block(template.block, false).await.unwrap();
    }
    let poll_account = account_a.clone();
    wait_for(
        200,
        150,
        move || {
            let account = poll_account.clone();
            Box::pin(async move { account.balance().map(|b| b.mature).unwrap_or(0) > 0 })
        },
        "payer wallet did not observe a mature transparent balance after mining",
    )
    .await;

    // A mints 0.28 MAGLD -> 2x0.1 + 8x0.01: a 0.1 to hand over as a bearer note,
    // exact denominations for a 0.05 payment, and small notes to stamp fees with.
    let mint = account_a.clone().mint(secret_a.clone(), None, 28_000_000, None, &Abortable::default()).await.expect("mint failed");
    assert_eq!(mint.notes.len(), 10);
    for _ in 0..10 {
        let template = miner_client.get_block_template(throwaway.clone(), vec![]).await.unwrap();
        miner_client.submit_block(template.block, false).await.unwrap();
    }

    // ---------- (a) bearer import ----------
    let handover = mint.notes.iter().find(|n| n.d == DenominationTag::D0_1).expect("mint produced a 0.1 note").clone();
    let bearer_text = BearerNote { sn: handover.sn, sk: handover.sk, d: handover.d }.to_text();

    // A's side of a bearer handover: the key has left the wallet; tombstone the row
    // (P7.4's bearer-export command formalizes this "handed over" bookkeeping).
    let store_a = wallet_a.store().as_note_key_store().expect("payer note key store");
    store_a.mark_status(&handover.sn, NoteStatus::Superseded).await.unwrap();

    let bearer = BearerNote::from_text(&bearer_text).expect("bearer text round-trip");
    let import_result = account_b.clone().bearer_import(secret_b.clone(), bearer).await.expect("bearer import failed");
    assert_eq!(import_result.imported_sn, handover.sn);

    // B held nothing else, so the rotation ran in slack mode: produced value =
    // rotated value - fee, all under fresh Cold keys, none reusing the imported key.
    let rotation = &import_result.rotation;
    let produced_value: u64 = rotation.own_notes.iter().map(|n| DENOMINATION_PETALS[n.d as usize]).sum();
    assert_eq!(produced_value + rotation.fee_petals, 10_000_000, "rotation must conserve value minus the fee");
    assert!(rotation.fee_petals >= 1_000_000 && rotation.fee_petals % 1_000_000 == 0, "fee must be a whole number of 0.01 quanta");
    for note in &rotation.own_notes {
        assert_ne!(note.sk, handover.sk, "no rotated note may reuse the imported (Hot) key");
        assert_eq!(note.provenance, NoteProvenance::Cold);
    }

    for _ in 0..10 {
        let template = miner_client.get_block_template(throwaway.clone(), vec![]).await.unwrap();
        miner_client.submit_block(template.block, false).await.unwrap();
    }

    // On-chain: the handed-over serial is gone, the rotated serials exist.
    let old = miner_client.get_notes_by_serial(vec![handover.sn]).await.unwrap();
    assert!(!old.iter().any(|entry| entry.sn == handover.sn), "the imported serial must be rotated away on-chain");
    let new_serials: Vec<Hash> = rotation.own_notes.iter().map(|n| n.sn).collect();
    let found = miner_client.get_notes_by_serial(new_serials.clone()).await.unwrap();
    assert_eq!(found.len(), new_serials.len(), "every rotated serial must exist in the pool");

    // B's books: imported row Hot + Superseded (never left unrotated), new rows Cold + Active.
    let store_b = wallet_b.store().as_note_key_store().expect("receiver note key store");
    let imported_info = store_b.load_info(&handover.sn).await.unwrap().expect("imported row present");
    assert_eq!(imported_info.provenance, NoteProvenance::Hot);
    assert_eq!(imported_info.status, NoteStatus::Superseded);
    for sn in &new_serials {
        let info = store_b.load_info(sn).await.unwrap().expect("rotated row present");
        assert_eq!(info.provenance, NoteProvenance::Cold);
        assert_eq!(info.status, NoteStatus::Active);
    }
    println!(
        "bearer flow: imported {} -> rotated into {} note(s), fee {} petals, tx {}",
        handover.sn,
        rotation.own_notes.len(),
        rotation.fee_petals,
        rotation.transaction_id
    );

    // ---------- (b) sign-to-fresh-pk ----------
    const PAYMENT_PETALS: u64 = 5_000_000; // 0.05 MAGLD -> 5x0.01 from A's minted notes

    let request = create_payment_request(&wallet_b, &secret_b, Some(PAYMENT_PETALS)).await.expect("create_payment_request failed");
    let request_text = request.to_text();
    assert_eq!(wallet_b.store().as_note_key_store().unwrap().payment_requests().await.unwrap().len(), 1);

    // B watches for the payment (subscribes by pk BEFORE A pays — the notification
    // fires on confirmation, which happens strictly after the mining below).
    let awaiter = {
        let wallet_b = wallet_b.clone();
        let secret_b = secret_b.clone();
        let pk = request.pk;
        tokio::spawn(async move { await_payment_request(&wallet_b, &secret_b, pk, std::time::Duration::from_secs(90)).await })
    };
    // Let the awaiter register its subscription before the payment can confirm.
    workflow_core::task::sleep(std::time::Duration::from_millis(1_000)).await;

    let parsed = PaymentRequest::from_text(&request_text).expect("request text round-trip");
    assert_eq!(parsed.amount_petals, Some(PAYMENT_PETALS));
    let pay = account_a.clone().pay_payment_request(secret_a.clone(), parsed, None).await.expect("pay_payment_request failed");
    assert_eq!(pay.external_serials.len(), 5, "0.05 MAGLD pays as 5x0.01 notes");
    assert!(pay.fee_petals >= 1_000_000 && pay.fee_petals % 1_000_000 == 0);

    for _ in 0..10 {
        let template = miner_client.get_block_template(throwaway.clone(), vec![]).await.unwrap();
        miner_client.submit_block(template.block, false).await.unwrap();
    }

    let claimed = awaiter.await.expect("awaiter task panicked").expect("await_payment_request failed");
    assert_eq!(claimed.total_petals, PAYMENT_PETALS, "claimed value must equal the requested amount exactly");
    assert_eq!(claimed.notes.len(), 5);
    let claimed_serials: Vec<Hash> = claimed.notes.iter().map(|n| n.sn).collect();
    assert_eq!(claimed_serials.iter().collect::<std::collections::HashSet<_>>().len(), 5);
    for note in &claimed.notes {
        let info = store_b.load_info(&note.sn).await.unwrap().expect("claimed row present");
        assert_eq!(info.provenance, NoteProvenance::Cold, "a request key never left the wallet — its notes are Cold");
        assert_eq!(info.status, NoteStatus::Active);
        assert_eq!(info.pk, request.pk, "claimed notes land on the request pk (transient landing pad)");
    }
    // The request retired on claim.
    assert!(wallet_b.store().as_note_key_store().unwrap().payment_requests().await.unwrap().is_empty());

    // On-chain agreement: the claimed serials exist and are owned by the request pk;
    // the payer's consumed serials (payment notes + fee stamp) are gone.
    let found = miner_client.get_notes_by_serial(claimed_serials.clone()).await.unwrap();
    assert_eq!(found.len(), 5);
    assert!(found.iter().all(|entry| entry.pk == request.pk));
    let consumed = miner_client.get_notes_by_serial(pay.consumed_serials.clone()).await.unwrap();
    assert!(consumed.is_empty(), "the payer's consumed serials must be gone from the pool");
    for sn in &pay.consumed_serials {
        let info = store_a.load_info(sn).await.unwrap().expect("payer row present");
        assert_eq!(info.status, NoteStatus::Superseded);
    }
    println!(
        "payment flow: request {} petals -> paid as {} note(s) (fee {} petals, tx {}), claimed {} petals",
        PAYMENT_PETALS,
        pay.external_serials.len(),
        pay.fee_petals,
        pay.transaction_id,
        claimed.total_petals
    );

    for wallet in [&wallet_a, &wallet_b] {
        if let Some(client) = wallet.try_wrpc_client() {
            client.disconnect().await.ok();
        }
    }
    miner_client.disconnect().await.unwrap();
    kaspad.shutdown();
}

/// FORK-PLAN P7.4's verify criterion: "spends of amounts requiring splits succeed;
/// bearer-exporting a shared-key note demonstrably isolates first (two txs
/// on-chain)."
///
/// - Split spend: A holds exactly ONE 0.1 note and pays a 0.03 request — the
///   covering planner must consume the 0.1 and emit payment (3x0.01 to B), change
///   (back to A under fresh keys), and the fee in ONE `TransferOp` ("split then pay
///   is one transaction", POOL-SPEC.md P5.6).
/// - Shared-key export: B's claimed notes share the request pk (landing pad), so
///   exporting one MUST auto-isolate onto a fresh solo key first (tx 1); the
///   receiving wallet's import-rotation is tx 2 — the exported serial's journey is
///   demonstrably two on-chain transactions, vs. a solo export which skips
///   isolation entirely (also asserted).
///
/// `cargo test --release --package kaspa-testing-integration --lib -- notepool_wallet_integration_tests::wallet_notepool_spend_flows_test --ignored --nocapture`
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn wallet_notepool_spend_flows_test() {
    use kaspa_wallet_core::account::notepool::{BearerNote, await_payment_request, create_payment_request};
    use kaspa_wallet_core::storage::NoteProvenance;

    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO,kaspa_testing_integration=trace");

    let args =
        Args { simnet: true, unsafe_rpc: true, enable_unsynced_mining: true, disable_upnp: true, utxoindex: true, ..Default::default() };
    let total_fd_limit = 10;
    let mut kaspad = Daemon::new_random_with_args(args, total_fd_limit);
    let miner_client = kaspad.start().await;

    let secret_a = Secret::from("payer-wallet-password");
    let secret_b = Secret::from("receiver-wallet-password");
    let (wallet_a, account_a) = connect_and_bootstrap_wallet(&kaspad, &secret_a).await;
    let (wallet_b, account_b) = connect_and_bootstrap_wallet(&kaspad, &secret_b).await;

    let receive_a = account_a.receive_address().expect("payer receive address");
    let throwaway = Address::new(kaspad.network.into(), Version::PubKey, &[7u8; 32]);
    let mut mine = |n: usize, to: Address| {
        let miner_client = miner_client.clone();
        async move {
            for _ in 0..n {
                let template = miner_client.get_block_template(to.clone(), vec![]).await.unwrap();
                miner_client.submit_block(template.block, false).await.unwrap();
            }
        }
    };

    // Fund A (one tracked coinbase + maturity), then mint exactly ONE 0.1 note — the
    // smallest holding that forces every later spend through the split planner.
    let coinbase_maturity = SIMNET_PARAMS.coinbase_maturity();
    mine(1, receive_a.clone()).await;
    mine((coinbase_maturity + 20) as usize, throwaway.clone()).await;
    let poll_account = account_a.clone();
    wait_for(
        200,
        150,
        move || {
            let account = poll_account.clone();
            Box::pin(async move { account.balance().map(|b| b.mature).unwrap_or(0) > 0 })
        },
        "payer wallet did not observe a mature transparent balance after mining",
    )
    .await;

    let mint = account_a.clone().mint(secret_a.clone(), None, 10_000_000, None, &Abortable::default()).await.expect("mint failed");
    assert_eq!(mint.notes.len(), 1, "0.1 MAGLD mints as exactly one D0_1 note");
    assert_eq!(mint.notes[0].d, DenominationTag::D0_1);
    mine(10, throwaway.clone()).await;

    // ---------- split-requiring spend ----------
    const PAYMENT_PETALS: u64 = 3_000_000; // 0.03 — no exact representation from one 0.1

    let request = create_payment_request(&wallet_b, &secret_b, Some(PAYMENT_PETALS)).await.expect("create_payment_request failed");
    let awaiter = {
        let wallet_b = wallet_b.clone();
        let secret_b = secret_b.clone();
        let pk = request.pk;
        tokio::spawn(async move { await_payment_request(&wallet_b, &secret_b, pk, std::time::Duration::from_secs(90)).await })
    };
    workflow_core::task::sleep(std::time::Duration::from_millis(1_000)).await;

    let pay = account_a.clone().pay_payment_request(secret_a.clone(), request, None).await.expect("split-requiring pay failed");
    // ONE transaction: consumed the single 0.1, produced 3x0.01 payment + change,
    // with the fee withheld — payment, split, change, fee all in the same TransferOp.
    assert_eq!(pay.consumed_serials.len(), 1, "the covering planner must consume the single 0.1 note");
    assert_eq!(pay.consumed_serials[0], mint.notes[0].sn);
    assert_eq!(pay.external_serials.len(), 3, "0.03 pays as 3x0.01");
    let change_value: u64 = pay.own_notes.iter().map(|n| DENOMINATION_PETALS[n.d as usize]).sum();
    assert_eq!(
        change_value + PAYMENT_PETALS + pay.fee_petals,
        10_000_000,
        "consumed value must exactly split into payment + change + fee"
    );
    println!(
        "split spend: 1 note in -> {} payment + {} change notes + {} petals fee, tx {}",
        pay.external_serials.len(),
        pay.own_notes.len(),
        pay.fee_petals,
        pay.transaction_id
    );

    mine(10, throwaway.clone()).await;
    let claimed = awaiter.await.expect("awaiter task panicked").expect("await_payment_request failed");
    assert_eq!(claimed.total_petals, PAYMENT_PETALS);
    assert_eq!(claimed.notes.len(), 3);

    // ---------- bearer export: shared key isolates first (two txs on-chain) ----------
    let store_b = wallet_b.store().as_note_key_store().expect("receiver note key store");
    let exported_origin = claimed.notes[0].clone();

    let export = account_b.clone().bearer_export(secret_b.clone(), exported_origin.sn).await.expect("shared-key export failed");
    let isolation = export.isolation.as_ref().expect("a landing-pad note's key is shared — export MUST isolate first");
    assert_ne!(export.bearer.sn, exported_origin.sn, "the exported serial is the freshly isolated one");
    assert_eq!(export.bearer.d, exported_origin.d, "isolation preserves the denomination");
    assert_ne!(export.bearer.sk, exported_origin.sk, "the isolated key is fresh — the shared key is never handed out");
    // The isolation consumed the exported note plus a same-key sibling as fee stamp —
    // one SignedGroup covering both serials under the shared landing-pad key.
    assert_eq!(isolation.consumed_serials.len(), 2);
    assert!(isolation.consumed_serials.contains(&exported_origin.sn));

    mine(10, throwaway.clone()).await;
    // Isolation (tx 1) on-chain: origin serial gone, isolated serial live.
    let gone = miner_client.get_notes_by_serial(vec![exported_origin.sn]).await.unwrap();
    assert!(gone.is_empty(), "the shared-key origin serial must be rotated away before handover");
    let live = miner_client.get_notes_by_serial(vec![export.bearer.sn]).await.unwrap();
    assert_eq!(live.len(), 1, "the isolated serial must be live for the receiver to verify");

    // B's books: origin + stamp Superseded, isolated note HandedOver.
    assert_eq!(store_b.load_info(&exported_origin.sn).await.unwrap().unwrap().status, NoteStatus::Superseded);
    assert_eq!(store_b.load_info(&export.bearer.sn).await.unwrap().unwrap().status, NoteStatus::HandedOver);

    // The receiver (A) imports the handover — tx 2 of the exported note's journey.
    let bearer = BearerNote::from_text(&export.bearer.to_text()).expect("bearer text round-trip");
    let import = account_a.clone().bearer_import(secret_a.clone(), bearer).await.expect("import of exported note failed");
    mine(10, throwaway.clone()).await;
    let handover_gone = miner_client.get_notes_by_serial(vec![export.bearer.sn]).await.unwrap();
    assert!(handover_gone.is_empty(), "the handed-over serial must be rotated away by the receiver (second on-chain tx)");
    let received: Vec<Hash> = import.rotation.own_notes.iter().map(|n| n.sn).collect();
    assert_eq!(miner_client.get_notes_by_serial(received).await.unwrap().len(), import.rotation.own_notes.len());
    println!(
        "shared-key export journey: isolation tx {} -> handover -> receiver rotation tx {}",
        isolation.transaction_id, import.rotation.transaction_id
    );

    // ---------- solo export skips isolation ----------
    // Any of A's change notes that is still Active (the bearer import above consumed
    // one of them as its rotation's fee stamp — skip that one).
    let store_a = wallet_a.store().as_note_key_store().expect("payer note key store");
    let mut solo = None;
    for note in &pay.own_notes {
        if store_a.load_info(&note.sn).await.unwrap().unwrap().status == NoteStatus::Active {
            solo = Some(note.clone());
            break;
        }
    }
    let solo = solo.expect("at least one change note remains active");
    let solo_export = account_a.clone().bearer_export(secret_a.clone(), solo.sn).await.expect("solo export failed");
    assert!(solo_export.isolation.is_none(), "a fresh solo Cold key needs no isolation");
    assert_eq!(solo_export.bearer.sn, solo.sn, "a solo note is handed over as-is");
    assert_eq!(store_a.load_info(&solo.sn).await.unwrap().unwrap().status, NoteStatus::HandedOver);
    assert_eq!(store_a.load_info(&solo.sn).await.unwrap().unwrap().provenance, NoteProvenance::Cold);

    for wallet in [&wallet_a, &wallet_b] {
        if let Some(client) = wallet.try_wrpc_client() {
            client.disconnect().await.ok();
        }
    }
    miner_client.disconnect().await.unwrap();
    kaspad.shutdown();
}

/// FORK-PLAN P7.5's verify criterion: "scripted two-wallet POS demo passes; merchant
/// wallet ends with one-note-one-key state within seconds of payment." B is the
/// merchant (`note pos`), A is the customer (`note pay`, requiring a split — the
/// same "amount not a single denomination" shape as the other flows, exercising the
/// covering planner on the payer's side of a POS sale). `pos_checkout` chains
/// request -> await -> sweep in one call; the assertion that matters is that B's
/// swept notes each land under a distinct, fresh key, none of them still on the
/// checkout `pk`.
///
/// `cargo test --release --package kaspa-testing-integration --lib -- notepool_wallet_integration_tests::wallet_notepool_pos_test --ignored --nocapture`
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn wallet_notepool_pos_test() {
    use kaspa_wallet_core::account::notepool::PaymentRequest;
    use kaspa_wallet_core::storage::NoteProvenance;
    use std::collections::HashSet;

    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO,kaspa_testing_integration=trace");

    let args =
        Args { simnet: true, unsafe_rpc: true, enable_unsynced_mining: true, disable_upnp: true, utxoindex: true, ..Default::default() };
    let total_fd_limit = 10;
    let mut kaspad = Daemon::new_random_with_args(args, total_fd_limit);
    let miner_client = kaspad.start().await;

    let secret_a = Secret::from("customer-wallet-password");
    let secret_b = Secret::from("merchant-wallet-password");
    let (wallet_a, account_a) = connect_and_bootstrap_wallet(&kaspad, &secret_a).await;
    let (_wallet_b, account_b) = connect_and_bootstrap_wallet(&kaspad, &secret_b).await;

    let receive_a = account_a.receive_address().expect("customer receive address");
    let throwaway = Address::new(kaspad.network.into(), Version::PubKey, &[7u8; 32]);
    let mine = |n: usize, to: Address| {
        let miner_client = miner_client.clone();
        async move {
            for _ in 0..n {
                let template = miner_client.get_block_template(to.clone(), vec![]).await.unwrap();
                miner_client.submit_block(template.block, false).await.unwrap();
            }
        }
    };

    // Fund the customer, then mint a 0.1 note -- the same "no exact representation
    // for a smaller ask" shape P7.4's spend test used, now on the paying side of a
    // POS sale rather than a peer-to-peer payment request.
    let coinbase_maturity = SIMNET_PARAMS.coinbase_maturity();
    mine(1, receive_a.clone()).await;
    mine((coinbase_maturity + 20) as usize, throwaway.clone()).await;
    let poll_account = account_a.clone();
    wait_for(
        200,
        150,
        move || {
            let account = poll_account.clone();
            Box::pin(async move { account.balance().map(|b| b.mature).unwrap_or(0) > 0 })
        },
        "customer wallet did not observe a mature transparent balance after mining",
    )
    .await;
    let mint = account_a.clone().mint(secret_a.clone(), None, 10_000_000, None, &Abortable::default()).await.expect("mint failed");
    assert_eq!(mint.notes.len(), 1);
    mine(10, throwaway.clone()).await;

    // ---------- the sale ----------
    const CHECKOUT_PETALS: u64 = 4_000_000; // 0.04 -- no exact representation from one 0.1

    // Merchant side: one call does request -> await -> sweep. Capture the request
    // (via the on_request hook, exactly as the CLI does) so the customer side of
    // this test can pay it, and run it concurrently with the payment below.
    let (request_tx, request_rx) = tokio::sync::oneshot::channel::<PaymentRequest>();
    let checkout = {
        let account_b = account_b.clone();
        let secret_b = secret_b.clone();
        tokio::spawn(async move {
            account_b
                .pos_checkout(
                    secret_b,
                    CHECKOUT_PETALS,
                    std::time::Duration::from_secs(90),
                    Some(Box::new(move |request: &PaymentRequest| {
                        let _ = request_tx.send(*request);
                    })),
                )
                .await
        })
    };
    let request = request_rx.await.expect("checkout must publish its request before waiting for payment");

    let pay = account_a.clone().pay_payment_request(secret_a.clone(), request, None).await.expect("split-requiring pos pay failed");
    assert_eq!(pay.external_serials.len(), 4, "0.04 pays as 4x0.01 to the checkout pk");

    mine(10, throwaway.clone()).await;
    let result = checkout.await.expect("checkout task panicked").expect("pos_checkout failed");

    assert_eq!(result.claimed.total_petals, CHECKOUT_PETALS);
    assert_eq!(result.claimed.notes.len(), 4);
    // One-note-one-key: every swept note under its own fresh Cold key, no two
    // sharing an sk, none of them the checkout pk.
    let store_b = account_b.wallet().store().as_note_key_store().expect("merchant note key store");
    let mut seen_pks = HashSet::new();
    for note in &result.sweep.own_notes {
        let info = store_b.load_info(&note.sn).await.unwrap().expect("swept row present");
        assert_eq!(info.provenance, NoteProvenance::Cold);
        assert_eq!(info.status, NoteStatus::Active);
        assert_ne!(info.pk, result.request.pk, "a swept note must not remain on the landing-pad pk");
        assert!(seen_pks.insert(info.pk), "no two swept notes may share a key");
    }
    // The claimed (pre-sweep) landing-pad rows are gone from the pool; the sweep's
    // consumed set is exactly the claimed serials, one SignedGroup (spec-mandated:
    // every claimed note shares the checkout key).
    let claimed_serials: Vec<Hash> = result.claimed.notes.iter().map(|n| n.sn).collect();
    assert_eq!(result.sweep.consumed_serials.iter().collect::<HashSet<_>>(), claimed_serials.iter().collect::<HashSet<_>>());
    let landing_pad_gone = miner_client.get_notes_by_serial(claimed_serials).await.unwrap();
    assert!(landing_pad_gone.is_empty(), "the landing-pad serials must be gone once swept");

    mine(10, throwaway.clone()).await;
    let swept: Vec<Hash> = result.sweep.own_notes.iter().map(|n| n.sn).collect();
    let live = miner_client.get_notes_by_serial(swept.clone()).await.unwrap();
    assert_eq!(live.len(), swept.len(), "every swept serial must be live in the pool");
    assert!(live.iter().all(|entry| entry.pk != result.request.pk));

    println!(
        "POS sale: {} MAGLD paid as {} note(s) to the landing pad -> swept to {} note(s) under distinct fresh keys (fee {} petals), \
         checkout tx {}, sweep tx {}",
        sompi_to_kaspa_string(result.claimed.total_petals),
        result.claimed.notes.len(),
        result.sweep.own_notes.len(),
        result.sweep.fee_petals,
        pay.transaction_id,
        result.sweep.transaction_id
    );

    if let Some(client) = wallet_a.try_wrpc_client() {
        client.disconnect().await.ok();
    }
    if let Some(client) = account_b.wallet().try_wrpc_client() {
        client.disconnect().await.ok();
    }
    miner_client.disconnect().await.unwrap();
    kaspad.shutdown();
}

/// FORK-PLAN P7.6's verify criterion: mint -> back up the vault -> restore into a
/// completely independent wallet using only "24 words + the files" -> deep-verify
/// recovers exactly the still-owned notes -> accept batched restore-time rotation
/// -> the OLD backup copy is now provably stale (light-verifiable without ever
/// restoring it) -> a corrupted vault file is caught by deep verify but not light
/// verify. `cargo test --release --package kaspa-testing-integration --lib --
/// notepool_wallet_integration_tests::wallet_notepool_vault_test --ignored --nocapture`
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn wallet_notepool_vault_test() {
    use kaspa_wallet_core::account::notepool::{deep_verify, light_verify, light_verify_vault, plan_restore_rotation};
    use kaspa_wallet_core::storage::NoteKeyInfo;
    use kaspa_wallet_core::storage::local::notevault::NoteVault;
    use kaspa_utils::hex::ToHex;
    use futures_util::TryStreamExt;

    fn copy_dir_recursive(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir_recursive(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO,kaspa_testing_integration=trace,kaspa_wallet_core=debug");

    let args =
        Args { simnet: true, unsafe_rpc: true, enable_unsynced_mining: true, disable_upnp: true, utxoindex: true, ..Default::default() };
    let total_fd_limit = 10;
    let mut kaspad = Daemon::new_random_with_args(args, total_fd_limit);
    let miner_client = kaspad.start().await;

    let secret_a = Secret::from("vault-test-wallet-a-password");
    let (wallet_a, account_a) = connect_and_bootstrap_wallet(&kaspad, &secret_a).await;

    // Run the 24-word ceremony explicitly (rather than letting the first mint
    // auto-provision it silently, per `LocalStoreInner::ensure_note_vault`) so the
    // test can capture the words for the restore step below.
    let store_a = wallet_a.store().as_note_key_store().expect("note key store");
    assert!(!store_a.vault_exists().await.unwrap(), "a fresh wallet must start with no vault");
    let words = store_a.vault_create(&secret_a).await.expect("vault_create failed");
    assert_eq!(words.split(' ').count(), 24, "the ceremony must produce exactly 24 words");
    assert!(store_a.vault_exists().await.unwrap());

    let receive_a = account_a.receive_address().expect("account should have a receive address");
    let throwaway = Address::new(kaspad.network.into(), Version::PubKey, &[7u8; 32]);
    let mine = |n: usize, to: Address| {
        let miner_client = miner_client.clone();
        async move {
            for _ in 0..n {
                let template = miner_client.get_block_template(to.clone(), vec![]).await.unwrap();
                miner_client.submit_block(template.block, false).await.unwrap();
            }
        }
    };

    let coinbase_maturity = SIMNET_PARAMS.coinbase_maturity();
    mine(1, receive_a.clone()).await;
    mine((coinbase_maturity + 20) as usize, throwaway.clone()).await;
    let poll_account = account_a.clone();
    wait_for(
        200,
        150,
        move || {
            let account = poll_account.clone();
            Box::pin(async move { account.balance().map(|b| b.mature).unwrap_or(0) > 0 })
        },
        "wallet did not observe a mature transparent balance after mining",
    )
    .await;

    // --- Mint, then back up the vault files to a standalone directory ---
    let abortable = Abortable::default();
    let mint_result =
        account_a.clone().mint(secret_a.clone(), None, MINT_AMOUNT_PETALS, None, &abortable).await.expect("mint failed");
    assert_eq!(mint_result.notes.len(), 3, "1.11 MAGLD should decompose into exactly 3 notes");
    let minted_serials: Vec<Hash> = mint_result.notes.iter().map(|n| n.sn).collect();
    mine(10, throwaway.clone()).await;

    let vault_a_folder = store_a.vault_folder().await.unwrap();
    let backup_dir = tempfile::tempdir().expect("tempdir");
    copy_dir_recursive(&vault_a_folder, backup_dir.path());
    println!("backed up vault ({} note(s)) to {:?}", minted_serials.len(), backup_dir.path());

    // --- Light-verify the backup directly, no wallet open, no secret ---
    let backup_vault = NoteVault::at(backup_dir.path());
    let rpc_a = wallet_a.rpc_api();
    let backup_report = light_verify_vault(&backup_vault, &rpc_a).await.expect("light_verify_vault failed");
    assert_eq!(backup_report.stale.len(), 0, "a fresh backup must show every note as live");
    assert_eq!(
        backup_report.live.iter().collect::<std::collections::HashSet<_>>(),
        minted_serials.iter().collect::<std::collections::HashSet<_>>(),
        "the backup's live set must be exactly what was minted"
    );

    // --- Bootstrap a completely independent, empty wallet ("wipe and get a new one") ---
    let secret_b = Secret::from("vault-test-wallet-b-password");
    let (wallet_b, account_b) = connect_and_bootstrap_wallet(&kaspad, &secret_b).await;
    let store_b = wallet_b.store().as_note_key_store().expect("note key store");
    assert!(store_b.is_empty().await.unwrap(), "the fresh wallet must start with no notes");

    // --- Restore: "24 words + the files" ---
    let vault_b_folder = store_b.vault_folder().await.unwrap();
    copy_dir_recursive(backup_dir.path(), &vault_b_folder);
    store_b.vault_restore_from_words(&words, &secret_b).await.expect("vault_restore_from_words failed");

    let deep_report = deep_verify(account_b.wallet(), secret_b.clone()).await.expect("deep_verify failed");
    assert!(deep_report.corrupted.is_empty());
    assert!(deep_report.stale.is_empty());
    assert_eq!(
        deep_report.live.iter().collect::<std::collections::HashSet<_>>(),
        minted_serials.iter().collect::<std::collections::HashSet<_>>(),
        "restore must recover exactly the still-owned minted notes"
    );

    // --- Accept the default restore-time rotation, batched ---
    //
    // `plan_restore_rotation` groups serials for spacing/mixing purposes only; it
    // has no visibility into `rotate_notes`'s own fee-source selection, which
    // freely pulls in *any* other currently-Active note as a fee stamp (P5.2's
    // mechanism) -- including one a *later* planned batch was going to target
    // directly. That note's value still gets reissued under a fresh key (as the
    // fee source's change), so it genuinely IS rotated -- just earlier than
    // planned, as a side effect ("rotation doubles as backup revocation",
    // DECISIONS.md). A later batch whose only serial already went this way has
    // nothing left to do and must be skipped rather than resubmitted.
    let batches = plan_restore_rotation(deep_report.live.clone());
    assert!((2..=5).contains(&batches.len()) || deep_report.live.len() < 2);
    let mut rotated_notes = Vec::new();
    for batch in &batches {
        let mut still_active = Vec::new();
        for sn in batch {
            if let Some(info) = store_b.load_info(sn).await.unwrap() {
                if info.status == NoteStatus::Active {
                    still_active.push(*sn);
                }
            }
        }
        if still_active.is_empty() {
            continue;
        }
        let result = account_b.clone().rotate_notes(secret_b.clone(), still_active).await.expect("rotate_notes failed");
        rotated_notes.extend(result.own_notes);
        mine(10, throwaway.clone()).await;
    }
    // Every originally-minted serial must be gone (Superseded) one way or another
    // -- either as a batch's own direct target, or consumed as another batch's
    // fee source.
    for sn in &minted_serials {
        let info = store_b.load_info(sn).await.unwrap().expect("original serial must still be a tombstoned row");
        assert_eq!(info.status, NoteStatus::Superseded, "every original note must have been rotated away by the end of the loop");
    }

    // --- The OLD backup is now provably stale, checkable without restoring it ---
    let backup_report_after_rotation = light_verify_vault(&backup_vault, &rpc_a).await.expect("light_verify_vault failed");
    assert_eq!(
        backup_report_after_rotation.live.len(),
        0,
        "every serial in the old backup must show stale once rotated -- rotation doubles as backup revocation"
    );
    assert_eq!(backup_report_after_rotation.stale.len(), minted_serials.len());

    // --- A corrupted note file is caught by deep verify but not light verify ---
    //
    // Pick any note that's actually still `Active` after the full rotation loop
    // settled -- `rotated_notes` includes every fresh row minted along the way,
    // some of which (an early batch's fee-stamp *change*) may have themselves
    // been consumed as a later batch's fee source before the loop finished.
    let mut active_after_rotation: Vec<Arc<NoteKeyInfo>> = store_b.iter().await.unwrap().try_collect().await.unwrap();
    active_after_rotation.retain(|info| info.status == NoteStatus::Active);
    assert!(!active_after_rotation.is_empty(), "the wallet must hold at least one active note after rotation");
    let corrupt_info = active_after_rotation[0].clone();
    let corrupt_note =
        rotated_notes.iter().find(|n| n.sn == corrupt_info.sn).expect("the active row must correspond to a rotated entry");
    let note_path = vault_b_folder
        .join("active")
        .join(format!("{}_{}.note", DENOMINATION_PETALS[corrupt_note.d as usize], corrupt_note.sn.to_hex()));
    assert!(note_path.exists(), "expected the rotated note's file to exist at {note_path:?}");
    std::fs::write(&note_path, b"not-valid-ciphertext-at-all").expect("failed to corrupt note file");

    let light_report_after_corruption = light_verify(account_b.wallet()).await.expect("light_verify failed");
    assert!(
        light_report_after_corruption.live.contains(&corrupt_note.sn),
        "light verify never decrypts, so it cannot see the corruption -- it must still report the note live"
    );

    let deep_report_after_corruption = deep_verify(account_b.wallet(), secret_b.clone()).await.expect("deep_verify failed");
    assert!(
        deep_report_after_corruption.corrupted.contains(&corrupt_note.sn),
        "deep verify decrypts every note and must catch the corrupted ciphertext"
    );

    println!(
        "vault test: minted {} note(s), backed up, restored into an independent wallet via words+files, deep-verified, \
         rotated in {} batch(es), confirmed the old backup went stale, and confirmed a corrupted file is caught only by deep verify",
        minted_serials.len(),
        batches.len()
    );

    if let Some(client) = wallet_a.try_wrpc_client() {
        client.disconnect().await.ok();
    }
    if let Some(client) = wallet_b.try_wrpc_client() {
        client.disconnect().await.ok();
    }
    miner_client.disconnect().await.unwrap();
    kaspad.shutdown();
}
