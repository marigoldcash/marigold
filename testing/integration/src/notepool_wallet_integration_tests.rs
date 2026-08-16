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
use kaspa_consensus_core::notepool::DenominationTag;
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

    // --- Connect a real `Wallet` to the daemon over wRPC ---
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

    // --- Non-interactive wallet + account bootstrap, via the same `WalletApi` calls the
    // real CLI (`cli/src/wizards/account.rs`) and wasm bindings use. ---
    let wallet_secret = Secret::from("test-wallet-password");
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
        redeem_result.transaction_id, redeem_result.redeemed_value_petals, redeem_result.fee_sompi
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
        redeem_result.redeemed_value_petals - redeem_result.fee_sompi,
        "transparent balance must rise by exactly (redeemed value - real fee)"
    );

    // --- Full reconciliation: net balance change across mint + redeem is exactly
    // -(mint_fee + redeem_fee) -- the pool round-trip cost nothing but the two real
    // network fees. ---
    let net_change = balance_before_mint - balance_after_redeem;
    assert_eq!(net_change, mint_fee + redeem_result.fee_sompi, "net balance change should equal exactly the two real network fees");
    println!(
        "reconciliation: before={balance_before_mint} after_mint={balance_after_mint} (fee {mint_fee}) after_redeem={balance_after_redeem} (fee {}) net_change={net_change}",
        redeem_result.fee_sompi
    );

    wrpc_client.disconnect().await.ok();
    miner_client.disconnect().await.unwrap();
    kaspad.shutdown();
}
