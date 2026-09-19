use crate::common::{
    client::ListeningClient,
    client_notify::ChannelNotify,
    daemon::Daemon,
    fee,
    utils::{fetch_spendable_utxos, mine_block, wait_for},
};
use kaspa_addresses::Address;
use kaspa_alloc::init_allocator_with_default_settings;
use kaspa_consensus::params::{Params, SIMNET_GENESIS, SIMNET_PARAMS};
use kaspa_consensus_core::{
    config::params::OverrideParams,
    constants::{TX_VERSION, TX_VERSION_TOCCATA},
    header::Header,
    mass::ComputeBudget,
    sign::{sign, sign_with_multiple_v2},
    subnets::{SUBNETWORK_ID_NATIVE, SubnetworkId},
    tx::{MutableTransaction, Transaction, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry},
};
use kaspa_consensusmanager::ConsensusManager;
use kaspa_core::{task::runtime::AsyncRuntime, trace};
use kaspa_grpc_client::GrpcClient;
use kaspa_hashes::Hash;
use kaspa_notify::{
    events::EventType,
    scope::{BlockAddedScope, NotesChangedScope, UtxosChangedScope, VirtualDaaScoreChangedScope},
};
use kaspa_rpc_core::{Notification, RpcNoteEntry, RpcTransaction, RpcTransactionId, api::rpc::RpcApi};
use kaspa_txscript::{
    opcodes::codes, pay_to_address_script, pay_to_script_hash_script, pay_to_script_hash_signature_script,
    script_builder::ScriptBuilder,
};
use kaspad_lib::{args::Args, daemon::Runtime as KaspadRuntime};
use rand::thread_rng;
use serde_json;
use std::{fs, path::PathBuf, sync::Arc, time::Duration};

fn load_override_params(path: &PathBuf) -> Params {
    let override_params_json = fs::read_to_string(path).unwrap();
    let override_params: OverrideParams = serde_json::from_str(&override_params_json).unwrap();
    SIMNET_PARAMS.override_params(override_params)
}

async fn walk_parent_chain(client: &GrpcClient, mut hash: Hash, steps: u64) -> Hash {
    for _ in 0..steps {
        let block = client.get_block(hash, false).await.unwrap();
        let Some(parent) = block.header.direct_parents().first() else {
            break;
        };
        hash = *parent;
    }
    hash
}

async fn is_ancestor_in_selected_parent_chain(client: &GrpcClient, mut descendant: Hash, target: Hash) -> bool {
    loop {
        if descendant == target {
            return true;
        }
        let block = client.get_block(descendant, false).await.unwrap();
        let Some(parent) = block.header.direct_parents().first() else {
            return false;
        };
        descendant = *parent;
    }
}

// Ignored since it might fail to initialize the logger if another test already initialized it. Run it specifically with `cargo test --release --package kaspa-testing-integration --lib -- daemon_integration_tests::daemon_toccata_activation_log_file_test --ignored`
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_toccata_activation_log_file_test() {
    init_allocator_with_default_settings();

    let test_dir = tempfile::tempdir().unwrap();
    let log_dir = test_dir.path().join("logs");
    let params_path = test_dir.path().join("params.json");
    fs::create_dir_all(&log_dir).unwrap();
    fs::write(&params_path, r#"{"skip_proof_of_work":true,"toccata_activation":1}"#).unwrap();

    let args = Args {
        simnet: true,
        unsafe_rpc: true,
        enable_unsynced_mining: true,
        disable_upnp: true,
        disable_dns_seeding: true,
        outbound_target: 0,
        logdir: Some(log_dir.to_string_lossy().to_string()),
        override_params_file: Some(params_path.to_string_lossy().to_string()),
        ..Default::default()
    };

    let _runtime = KaspadRuntime::from_args(&args);
    let mut kaspad = Daemon::new_random_with_args(args, 10);
    let rpc_client = kaspad.start().await;
    let log_path = log_dir.join("rusty-kaspa.log");

    let initial_log = fs::read_to_string(&log_path).unwrap_or_default();
    assert!(!initial_log.contains("[Toccata] Activated for"), "Toccata activation logs were emitted before activation");

    let miner_address = Address::new(kaspad.network.into(), kaspa_addresses::Version::PubKey, &[0; 32]);
    for target_daa_score in 1..=2 {
        let template = rpc_client.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client.submit_block(template.block, false).await.unwrap();

        let activation_check_client = rpc_client.clone();
        wait_for(
            50,
            100,
            move || {
                let client = activation_check_client.clone();
                Box::pin(async move { client.get_server_info().await.unwrap().virtual_daa_score >= target_daa_score })
            },
            "daemon did not reach Toccata activation",
        )
        .await;
    }

    rpc_client.disconnect().await.unwrap();
    drop(rpc_client);
    kaspad.shutdown();

    let log = fs::read_to_string(&log_path).unwrap();
    let header_log_count = log.matches("[Toccata] Activated for header in context validation").count();
    assert_eq!(header_log_count, 1, "Toccata activation log for header in context validation should be emitted exactly once");
    let virtual_state_log_count = log.matches("[Toccata] Activated for virtual state processing rules").count();
    assert_eq!(virtual_state_log_count, 1, "Toccata activation log for virtual state processing rules should be emitted exactly once");
    assert_eq!(
        log.matches("TOCCATA").count(),
        virtual_state_log_count,
        "Toccata ASCII art should only be emitted by the virtual state logger"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_sanity_test() {
    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO");

    // let total_fd_limit =  kaspa_utils::fd_budget::get_limit() / 2 - 128;
    let total_fd_limit = 10;
    let mut kaspad1 = Daemon::new_random(total_fd_limit);
    let rpc_client1 = kaspad1.start().await;
    assert!(rpc_client1.handle_message_id() && rpc_client1.handle_stop_notify(), "the client failed to collect server features");

    let mut kaspad2 = Daemon::new_random(total_fd_limit);
    let rpc_client2 = kaspad2.start().await;
    assert!(rpc_client2.handle_message_id() && rpc_client2.handle_stop_notify(), "the client failed to collect server features");

    tokio::time::sleep(Duration::from_secs(1)).await;
    rpc_client1.disconnect().await.unwrap();
    drop(rpc_client1);
    kaspad1.shutdown();

    rpc_client2.disconnect().await.unwrap();
    drop(rpc_client2);
    kaspad2.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_mining_test() {
    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO");

    let args = Args {
        simnet: true,
        unsafe_rpc: true,
        enable_unsynced_mining: true,
        disable_upnp: true, // UPnP registration might take some time and is not needed for this test
        ..Default::default()
    };
    // let total_fd_limit = kaspa_utils::fd_budget::get_limit() / 2 - 128;
    let total_fd_limit = 10;

    let mut kaspad1 = Daemon::new_random_with_args(args.clone(), total_fd_limit);
    let mut kaspad2 = Daemon::new_random_with_args(args, total_fd_limit);
    let rpc_client1 = kaspad1.start().await;
    let rpc_client2 = kaspad2.start().await;

    rpc_client2.add_peer(format!("127.0.0.1:{}", kaspad1.p2p_port).try_into().unwrap(), true).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await; // Let it connect
    assert_eq!(rpc_client2.get_connected_peer_info().await.unwrap().peer_info.len(), 1);

    let (sender, event_receiver) = async_channel::unbounded();
    rpc_client1.start(Some(Arc::new(ChannelNotify::new(sender)))).await;
    rpc_client1.start_notify(Default::default(), VirtualDaaScoreChangedScope {}.into()).await.unwrap();

    // Mine 10 blocks to daemon #1
    let mut last_block_hash = None;
    for i in 0..10 {
        let template = rpc_client1
            .get_block_template(Address::new(kaspad1.network.into(), kaspa_addresses::Version::PubKey, &[0; 32]), vec![])
            .await
            .unwrap();
        let header: Header = (&template.block.header).try_into().unwrap();
        last_block_hash = Some(header.hash);
        rpc_client1.submit_block(template.block, false).await.unwrap();

        while let Ok(notification) = match tokio::time::timeout(Duration::from_secs(1), event_receiver.recv()).await {
            Ok(res) => res,
            Err(elapsed) => panic!("expected virtual event before {}", elapsed),
        } {
            match notification {
                Notification::VirtualDaaScoreChanged(msg) if msg.virtual_daa_score == i + 1 => {
                    break;
                }
                Notification::VirtualDaaScoreChanged(msg) if msg.virtual_daa_score > i + 1 => {
                    panic!("DAA score too high for number of submitted blocks")
                }
                Notification::VirtualDaaScoreChanged(_) => {}
                _ => panic!("expected only DAA score notifications"),
            }
        }
    }

    tokio::time::sleep(Duration::from_secs(1)).await;
    // Expect the blocks to be relayed to daemon #2
    let dag_info = rpc_client2.get_block_dag_info().await.unwrap();
    assert_eq!(dag_info.block_count, 10);
    assert_eq!(dag_info.sink, last_block_hash.unwrap());

    // Check that acceptance data contains the expected coinbase tx ids
    let vc = rpc_client2
        .get_virtual_chain_from_block(
            SIMNET_GENESIS.hash, //
            true,
            None,
        )
        .await
        .unwrap();
    assert_eq!(vc.removed_chain_block_hashes.len(), 0);
    assert_eq!(vc.added_chain_block_hashes.len(), 10);
    assert_eq!(vc.accepted_transaction_ids.len(), 10);
    for accepted_txs_pair in vc.accepted_transaction_ids {
        assert_eq!(accepted_txs_pair.accepted_transaction_ids.len(), 1);
    }
}

/// `cargo test --release --package kaspa-testing-integration --lib -- daemon_integration_tests::daemon_utxos_propagation_test`
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_utxos_propagation_test() {
    #[cfg(feature = "heap")]
    let _profiler = dhat::Profiler::builder().file_name("kaspa-testing-integration-heap.json").build();

    kaspa_core::log::try_init_logger(
        "INFO,kaspa_testing_integration=trace,kaspa_notify=debug,kaspa_rpc_core=debug,kaspa_grpc_client=debug",
    );

    let args = Args {
        simnet: true,
        unsafe_rpc: true,
        enable_unsynced_mining: true,
        disable_upnp: true, // UPnP registration might take some time and is not needed for this test
        utxoindex: true,
        ..Default::default()
    };
    let total_fd_limit = 10;

    let coinbase_maturity = SIMNET_PARAMS.coinbase_maturity();
    let mut kaspad1 = Daemon::new_random_with_args(args.clone(), total_fd_limit);
    let mut kaspad2 = Daemon::new_random_with_args(args, total_fd_limit);
    let rpc_client1 = kaspad1.start().await;
    let rpc_client2 = kaspad2.start().await;

    // Let rpc_client1 receive virtual DAA score changed notifications
    let (sender1, event_receiver1) = async_channel::unbounded();
    rpc_client1.start(Some(Arc::new(ChannelNotify::new(sender1)))).await;
    rpc_client1.start_notify(Default::default(), VirtualDaaScoreChangedScope {}.into()).await.unwrap();

    // Connect kaspad2 to kaspad1
    rpc_client2.add_peer(format!("127.0.0.1:{}", kaspad1.p2p_port).try_into().unwrap(), true).await.unwrap();
    let check_client = rpc_client2.clone();
    wait_for(
        50,
        20,
        move || {
            async fn peer_connected(client: GrpcClient) -> bool {
                client.get_connected_peer_info().await.unwrap().peer_info.len() == 1
            }
            Box::pin(peer_connected(check_client.clone()))
        },
        "the nodes did not connect to each other",
    )
    .await;

    // Mining key and address
    let (miner_sk, miner_pk) = secp256k1::generate_keypair(&mut thread_rng());
    let miner_address =
        Address::new(kaspad1.network.into(), kaspa_addresses::Version::PubKey, &miner_pk.x_only_public_key().0.serialize());
    let miner_spk = pay_to_address_script(&miner_address);

    // User key and address
    let (_user_sk, user_pk) = secp256k1::generate_keypair(&mut thread_rng());
    let user_address =
        Address::new(kaspad1.network.into(), kaspa_addresses::Version::PubKey, &user_pk.x_only_public_key().0.serialize());

    // Some dummy non-monitored address
    let blank_address = Address::new(kaspad1.network.into(), kaspa_addresses::Version::PubKey, &[0; 32]);

    // Create a multi-listener RPC client on each node. Multi-listener subscriptions are propagated
    // upstream asynchronously, so subscribe to the streams used by mine_block before the long initial
    // mining run and later verify that notifications actually flowed through both listeners.
    let mut clients = vec![ListeningClient::connect(&kaspad2).await, ListeningClient::connect(&kaspad1).await];
    for x in clients.iter_mut() {
        x.start_notify(BlockAddedScope {}.into()).await.unwrap();
        x.start_notify(VirtualDaaScoreChangedScope {}.into()).await.unwrap();
    }

    // Mine 1000 blocks to daemon #1
    let initial_blocks = coinbase_maturity;
    let mut last_block_hash = None;
    for i in 0..initial_blocks {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        let header: Header = (&template.block.header).try_into().unwrap();
        last_block_hash = Some(header.hash);
        rpc_client1.submit_block(template.block, false).await.unwrap();

        while let Ok(notification) = match tokio::time::timeout(Duration::from_secs(1), event_receiver1.recv()).await {
            Ok(res) => res,
            Err(elapsed) => panic!("expected virtual event before {}", elapsed),
        } {
            match notification {
                Notification::VirtualDaaScoreChanged(msg) if msg.virtual_daa_score == i + 1 => {
                    break;
                }
                Notification::VirtualDaaScoreChanged(msg) if msg.virtual_daa_score > i + 1 => {
                    panic!("DAA score too high for number of submitted blocks")
                }
                Notification::VirtualDaaScoreChanged(_) => {}
                _ => panic!("expected only DAA score notifications"),
            }
        }
    }

    let check_client = rpc_client2.clone();
    wait_for(
        50,
        20,
        move || {
            async fn daa_score_reached(client: GrpcClient) -> bool {
                let virtual_daa_score = client.get_server_info().await.unwrap().virtual_daa_score;
                trace!("Virtual DAA score: {}", virtual_daa_score);
                virtual_daa_score == SIMNET_PARAMS.coinbase_maturity()
            }
            Box::pin(daa_score_reached(check_client.clone()))
        },
        "the nodes did not add and relay all the initial blocks",
    )
    .await;

    // Expect the blocks to be relayed to daemon #2
    let dag_info = rpc_client2.get_block_dag_info().await.unwrap();
    assert_eq!(dag_info.block_count, initial_blocks);
    assert_eq!(dag_info.sink, last_block_hash.unwrap());

    // Check that acceptance data contains the expected coinbase tx ids
    let vc = rpc_client2.get_virtual_chain_from_block(SIMNET_GENESIS.hash, true, None).await.unwrap();
    assert_eq!(vc.removed_chain_block_hashes.len(), 0);
    assert_eq!(vc.added_chain_block_hashes.len() as u64, initial_blocks);
    assert_eq!(vc.accepted_transaction_ids.len() as u64, initial_blocks);
    for accepted_txs_pair in vc.accepted_transaction_ids {
        assert_eq!(accepted_txs_pair.accepted_transaction_ids.len(), 1);
    }

    // Use the initial mining run as a readiness barrier for the multi-listener notification stack,
    // then consume the warm-up history through the final block and virtual DAA notifications so
    // the following mine_block calls observe only fresh notifications.
    let last_block_hash = last_block_hash.unwrap();
    let timeout_per_notification = Duration::from_secs(10);
    for x in clients.iter() {
        x.wait_for_notification(EventType::BlockAdded, timeout_per_notification, |notification| {
            matches!(notification, Notification::BlockAdded(notification) if notification.block.header.hash == last_block_hash)
        })
        .await;
        x.wait_for_notification(EventType::VirtualDaaScoreChanged, timeout_per_notification, |notification| {
            matches!(notification, Notification::VirtualDaaScoreChanged(notification) if notification.virtual_daa_score == initial_blocks)
        })
        .await;
        x.block_added_listener().unwrap().drain();
        x.virtual_daa_score_changed_listener().unwrap().drain();
    }

    // Subscribe to address-filtered UTXO notifications only after the initial maturity mining, so
    // the UTXO listener does not accumulate the 1000 coinbase notifications above.
    for x in clients.iter_mut() {
        x.start_notify(UtxosChangedScope::new(vec![miner_address.clone(), user_address.clone()]).into()).await.unwrap();
    }

    // Mine some extra blocks so the latest miner reward is added to its balance and some UTXOs reach maturity
    const EXTRA_BLOCKS: usize = 10;
    for _ in 0..EXTRA_BLOCKS {
        mine_block(blank_address.clone(), &rpc_client1, &clients).await;
    }

    // Check the balance of the miner address
    let miner_balance = rpc_client2.get_balance_by_address(miner_address.clone()).await.unwrap();
    assert_eq!(miner_balance, initial_blocks * SIMNET_PARAMS.pre_deflationary_phase_base_subsidy);
    let miner_balance = rpc_client1.get_balance_by_address(miner_address.clone()).await.unwrap();
    assert_eq!(miner_balance, initial_blocks * SIMNET_PARAMS.pre_deflationary_phase_base_subsidy);

    // Get the miner UTXOs
    let utxos = fetch_spendable_utxos(&rpc_client1, miner_address.clone(), coinbase_maturity).await;
    assert_eq!(utxos.len(), EXTRA_BLOCKS - 1);
    for utxo in utxos.iter() {
        assert!(utxo.1.is_coinbase);
        assert_eq!(utxo.1.amount, SIMNET_PARAMS.pre_deflationary_phase_base_subsidy);
        assert_eq!(utxo.1.script_public_key, miner_spk);
    }

    // Drain UTXOs and Virtual DAA score changed notification channels
    clients.iter().for_each(|x| x.utxos_changed_listener().unwrap().drain());
    clients.iter().for_each(|x| x.virtual_daa_score_changed_listener().unwrap().drain());

    // Spend some coins - sending funds from miner address to user address
    // The transaction here is later used to verify utxo return address RPC
    const NUMBER_INPUTS: u64 = 2;
    const NUMBER_OUTPUTS: u64 = 2;
    const TX_AMOUNT: u64 = SIMNET_PARAMS.pre_deflationary_phase_base_subsidy * (NUMBER_INPUTS * 5 - 1) / 5;
    let selected_utxos = &utxos[0..NUMBER_INPUTS as usize];
    let tx_script_public_key = pay_to_address_script(&user_address);
    let inputs = selected_utxos
        .iter()
        .map(|(op, _)| TransactionInput {
            previous_outpoint: *op,
            signature_script: vec![],
            sequence: 0,
            compute_commit: ComputeBudget(0).into(),
        })
        .collect();
    let outputs = (0..NUMBER_OUTPUTS)
        .map(|_| TransactionOutput {
            value: TX_AMOUNT / NUMBER_OUTPUTS,
            script_public_key: tx_script_public_key.clone(),
            covenant: None,
        })
        .collect();
    let unsigned_tx = Transaction::new(TX_VERSION_TOCCATA, inputs, outputs, 0, SUBNETWORK_ID_NATIVE, 0, vec![]);
    let signed_tx = sign_with_multiple_v2(
        MutableTransaction::with_entries(unsigned_tx, selected_utxos.iter().map(|(_, entry)| entry.clone()).collect()),
        &[miner_sk.secret_bytes()],
    )
    .unwrap();
    let mut transaction = signed_tx.tx;
    let per_input_compute_budget_commitment: u16 = 300; // ~30k-gram per-input upper bound
    transaction.inputs.iter_mut().for_each(|input| input.compute_commit = ComputeBudget(per_input_compute_budget_commitment).into());
    rpc_client1.submit_transaction((&transaction).into(), false).await.unwrap();

    let check_client = rpc_client1.clone();
    let transaction_id = transaction.id();
    wait_for(
        50,
        20,
        move || {
            async fn transaction_in_mempool(client: GrpcClient, transaction_id: RpcTransactionId) -> bool {
                let entry = client.get_mempool_entry(transaction_id, false, false).await;
                entry.is_ok()
            }
            Box::pin(transaction_in_mempool(check_client.clone(), transaction_id))
        },
        "the transaction was not added to the mempool",
    )
    .await;

    mine_block(blank_address.clone(), &rpc_client1, &clients).await;

    // Check UTXOs changed notifications
    for x in clients.iter() {
        let Notification::UtxosChanged(uc) = x.utxos_changed_listener().unwrap().receiver.recv().await.unwrap() else {
            panic!("wrong notification type")
        };
        assert!(uc.removed.iter().all(|x| x.address.is_some() && *x.address.as_ref().unwrap() == miner_address));
        assert!(uc.added.iter().all(|x| x.address.is_some() && *x.address.as_ref().unwrap() == user_address));
        assert_eq!(uc.removed.len() as u64, NUMBER_INPUTS);
        assert_eq!(uc.added.len() as u64, NUMBER_OUTPUTS);
        assert_eq!(
            uc.removed.iter().map(|x| x.utxo_entry.amount).sum::<u64>(),
            SIMNET_PARAMS.pre_deflationary_phase_base_subsidy * NUMBER_INPUTS
        );
        assert_eq!(uc.added.iter().map(|x| x.utxo_entry.amount).sum::<u64>(), TX_AMOUNT);
    }

    // Check the balance of both miner and user addresses
    for x in clients.iter() {
        let miner_balance = x.get_balance_by_address(miner_address.clone()).await.unwrap();
        assert_eq!(miner_balance, (initial_blocks - NUMBER_INPUTS) * SIMNET_PARAMS.pre_deflationary_phase_base_subsidy);

        let user_balance = x.get_balance_by_address(user_address.clone()).await.unwrap();
        assert_eq!(user_balance, TX_AMOUNT);
    }

    // UTXO Return Address Test
    // Mine another block to accept the transactions from the previous block
    // The tx above is sending from miner address to user address
    mine_block(blank_address.clone(), &rpc_client1, &clients).await;
    let new_utxos = rpc_client1.get_utxos_by_addresses(vec![user_address]).await.unwrap();
    let new_utxo = new_utxos
        .iter()
        .find(|utxo| utxo.outpoint.transaction_id == transaction.id())
        .expect("Did not find a utxo for the tx we just created but expected to");

    let utxo_return_address = rpc_client1
        .get_utxo_return_address(new_utxo.outpoint.transaction_id, new_utxo.utxo_entry.block_daa_score)
        .await
        .expect("We just created the tx and utxo here");

    assert_eq!(miner_address, utxo_return_address);

    // Terminate multi-listener clients
    for x in clients.iter() {
        x.disconnect().await.unwrap();
        x.join().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_compute_budget_relay_test() {
    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO");

    let compute_budget_relay_test_params =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/params/compute_budget_relay_test_params.json");

    let args = Args {
        simnet: true,
        unsafe_rpc: true,
        enable_unsynced_mining: true,
        disable_upnp: true,
        disable_dns_seeding: true,
        utxoindex: true,
        outbound_target: 0,
        override_params_file: Some(compute_budget_relay_test_params.to_string_lossy().to_string()),
        ..Default::default()
    };
    let total_fd_limit = 10;

    let coinbase_maturity = 0;
    let mut kaspad1 = Daemon::new_random_with_args(args.clone(), total_fd_limit);
    let mut kaspad2 = Daemon::new_random_with_args(args, total_fd_limit);
    let rpc_client1 = kaspad1.start().await;
    let rpc_client2 = kaspad2.start().await;

    rpc_client2.add_peer(format!("127.0.0.1:{}", kaspad1.p2p_port).try_into().unwrap(), true).await.unwrap();
    let check_client = rpc_client2.clone();
    wait_for(
        50,
        40,
        move || {
            async fn peer_connected(client: GrpcClient) -> bool {
                client.get_connected_peer_info().await.unwrap().peer_info.len() == 1
            }
            Box::pin(peer_connected(check_client.clone()))
        },
        "the nodes did not connect to each other",
    )
    .await;

    let (miner_sk, miner_pk) = secp256k1::generate_keypair(&mut thread_rng());
    let miner_address =
        Address::new(kaspad1.network.into(), kaspa_addresses::Version::PubKey, &miner_pk.x_only_public_key().0.serialize());
    let (_user_sk, user_pk) = secp256k1::generate_keypair(&mut thread_rng());
    let user_address =
        Address::new(kaspad1.network.into(), kaspa_addresses::Version::PubKey, &user_pk.x_only_public_key().0.serialize());

    let mut last_block_hash = None;
    for _ in 0..coinbase_maturity {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        let header: Header = (&template.block.header).try_into().unwrap();
        last_block_hash = Some(header.hash);
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    if let Some(expected_sink) = last_block_hash {
        let check_client = rpc_client2.clone();
        wait_for(
            50,
            40,
            move || {
                async fn node_synced(client: GrpcClient, expected_sink: Hash) -> bool {
                    let info = client.get_block_dag_info().await.unwrap();
                    info.sink == expected_sink
                }
                Box::pin(node_synced(check_client.clone(), expected_sink))
            },
            "node #2 did not sync to node #1 tip",
        )
        .await;
    }

    const EXTRA_BLOCKS: usize = 10;
    for _ in 0..EXTRA_BLOCKS {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    let expected_sink = rpc_client1.get_block_dag_info().await.unwrap().sink;
    let check_client = rpc_client2.clone();
    wait_for(
        50,
        200,
        move || {
            async fn node_synced(client: GrpcClient, expected_sink: Hash) -> bool {
                client.get_block_dag_info().await.unwrap().sink == expected_sink
            }
            Box::pin(node_synced(check_client.clone(), expected_sink))
        },
        "node #2 did not catch up after extra blocks",
    )
    .await;

    if rpc_client2.get_connected_peer_info().await.unwrap().peer_info.is_empty() {
        rpc_client2.add_peer(format!("127.0.0.1:{}", kaspad1.p2p_port).try_into().unwrap(), true).await.unwrap();
        let check_client = rpc_client2.clone();
        wait_for(
            50,
            200,
            move || {
                async fn peer_connected(client: GrpcClient) -> bool {
                    client.get_connected_peer_info().await.unwrap().peer_info.len() == 1
                }
                Box::pin(peer_connected(check_client.clone()))
            },
            "the nodes were disconnected before transaction submission",
        )
        .await;
    }

    let check_client1 = rpc_client1.clone();
    let check_client2 = rpc_client2.clone();
    wait_for(
        50,
        600,
        move || {
            async fn tips_aligned(client1: GrpcClient, client2: GrpcClient) -> bool {
                let tip1 = client1.get_block_dag_info().await.unwrap().sink;
                let tip2 = client2.get_block_dag_info().await.unwrap().sink;
                tip1 == tip2
            }
            Box::pin(tips_aligned(check_client1.clone(), check_client2.clone()))
        },
        "the nodes did not align to the same tip before transaction submission",
    )
    .await;

    let utxos = fetch_spendable_utxos(&rpc_client1, miner_address.clone(), coinbase_maturity).await;
    const NUMBER_INPUTS: u64 = 2;
    const NUMBER_OUTPUTS: u64 = 2;
    const PER_INPUT_COMPUTE_BUDGET: u16 = 30;
    const EXTRA_FEE: u64 = 10_000;
    let oldest_utxos_start = utxos.len() - NUMBER_INPUTS as usize;
    let selected_utxos = &utxos[oldest_utxos_start..];
    let total_in = selected_utxos.iter().map(|x| x.1.amount).sum::<u64>();
    let script_public_key = pay_to_address_script(&user_address);
    let build_transaction = |tx_output_amount: u64| {
        let inputs = selected_utxos
            .iter()
            .map(|(op, _)| TransactionInput {
                previous_outpoint: *op,
                signature_script: vec![],
                sequence: 0,
                compute_commit: ComputeBudget(0).into(),
            })
            .collect();
        let outputs = (0..NUMBER_OUTPUTS)
            .map(|_| TransactionOutput {
                value: tx_output_amount / NUMBER_OUTPUTS,
                script_public_key: script_public_key.clone(),
                covenant: None,
            })
            .collect();
        let unsigned_tx = Transaction::new(TX_VERSION_TOCCATA, inputs, outputs, 0, SUBNETWORK_ID_NATIVE, 0, vec![]);
        sign_with_multiple_v2(
            MutableTransaction::with_entries(unsigned_tx, selected_utxos.iter().map(|(_, entry)| entry.clone()).collect()),
            &[miner_sk.secret_bytes()],
        )
        .unwrap()
        .tx
    };

    let tx_fee = fee::calc_from_probe(|| {
        let mut tx = build_transaction(total_in);
        tx.inputs.iter_mut().for_each(|input| input.compute_commit = ComputeBudget(PER_INPUT_COMPUTE_BUDGET).into());
        tx
    })
    .saturating_add(EXTRA_FEE);
    let tx_amount = total_in.checked_sub(tx_fee).expect("expected enough input value for test transaction fee");

    let mut transaction = build_transaction(tx_amount);
    transaction.inputs.iter_mut().for_each(|input| input.compute_commit = ComputeBudget(PER_INPUT_COMPUTE_BUDGET).into());
    assert!(
        transaction.inputs.iter().any(|input| input.compute_commit.compute_budget().unwrap() > 0),
        "expected non-zero compute_budget commitment for v1 transaction"
    );
    let transaction_id = transaction.id();
    rpc_client1.submit_transaction((&transaction).into(), false).await.unwrap();

    let check_client = rpc_client1.clone();
    wait_for(
        50,
        200,
        move || {
            async fn transaction_in_mempool(client: GrpcClient, transaction_id: RpcTransactionId) -> bool {
                client.get_mempool_entry(transaction_id, false, false).await.is_ok()
            }
            Box::pin(transaction_in_mempool(check_client.clone(), transaction_id))
        },
        "the transaction was not added to node #1 mempool",
    )
    .await;

    let node1_entry = rpc_client1.get_mempool_entry(transaction_id, false, false).await.unwrap();
    assert_eq!(node1_entry.transaction.version, TX_VERSION_TOCCATA);
    let node1_compute_budget = node1_entry.transaction.inputs[0].compute_budget;
    assert!(node1_compute_budget > 0, "expected non-zero compute_budget on node #1 mempool tx");

    let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
    let mined_header: Header = (&template.block.header).try_into().unwrap();
    let mined_block_hash = mined_header.hash;
    rpc_client1.submit_block(template.block, false).await.unwrap();

    let check_client = rpc_client2.clone();
    wait_for(
        50,
        200,
        move || {
            async fn node_synced(client: GrpcClient, expected_sink: Hash) -> bool {
                client.get_block_dag_info().await.unwrap().sink == expected_sink
            }
            Box::pin(node_synced(check_client.clone(), mined_block_hash))
        },
        "node #2 did not receive the mined block with the transaction",
    )
    .await;

    let block2 = rpc_client2.get_block(mined_block_hash, true).await.unwrap();
    let included_tx = block2
        .transactions
        .iter()
        .find(|tx| tx.verbose_data.as_ref().is_some_and(|vd| vd.transaction_id == transaction_id))
        .expect("node #2 block does not include the submitted transaction");

    assert_eq!(included_tx.version, TX_VERSION_TOCCATA);
    let included_compute_budget = included_tx.inputs[0].compute_budget;
    assert!(included_compute_budget > 0, "expected non-zero compute_budget on propagated block tx");
    assert_eq!(included_compute_budget, node1_compute_budget);

    rpc_client1.disconnect().await.unwrap();
    rpc_client2.disconnect().await.unwrap();
    kaspad1.shutdown();
    kaspad2.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_rejects_transactions_with_inconsistent_input_mass_and_version() {
    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO");

    let compute_budget_relay_test_params =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/params/compute_budget_relay_test_params.json");
    let args = Args {
        simnet: true,
        unsafe_rpc: true,
        enable_unsynced_mining: true,
        disable_upnp: true,
        disable_dns_seeding: true,
        utxoindex: true,
        outbound_target: 0,
        override_params_file: Some(compute_budget_relay_test_params.to_string_lossy().to_string()),
        ..Default::default()
    };

    let mut kaspad = Daemon::new_random_with_args(args, 10);
    let rpc_client = kaspad.start().await;

    let (miner_sk, miner_pk) = secp256k1::generate_keypair(&mut thread_rng());
    let miner_address =
        Address::new(kaspad.network.into(), kaspa_addresses::Version::PubKey, &miner_pk.x_only_public_key().0.serialize());
    let pay_spk = pay_to_address_script(&miner_address);
    let miner_schnorr_key = secp256k1::Keypair::from_secret_key(secp256k1::SECP256K1, &miner_sk);

    for _ in 0..4 {
        let template = rpc_client.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client.submit_block(template.block, false).await.unwrap();
    }

    let utxos = fetch_spendable_utxos(&rpc_client, miner_address.clone(), 0).await;
    assert!(utxos.len() >= 2, "expected enough spendable UTXOs for malformed transaction tests");

    let build_single_input_tx = |version: u16, selected_utxo: &(TransactionOutpoint, UtxoEntry)| {
        let fee = fee::calc_for_plain_standard_tx(1, 1);
        let output_value = selected_utxo.1.amount.checked_sub(fee).expect("expected enough input value for test fee");
        let compute_commit = ComputeBudget(0).into(); // set correctly by sign below
        let tx = Transaction::new(
            version,
            vec![TransactionInput { previous_outpoint: selected_utxo.0, signature_script: vec![], sequence: 0, compute_commit }],
            vec![TransactionOutput { value: output_value, script_public_key: pay_spk.clone(), covenant: None }],
            0,
            SUBNETWORK_ID_NATIVE,
            0,
            vec![],
        );
        sign(MutableTransaction::with_entries(tx, vec![selected_utxo.1.clone()]), miner_schnorr_key).tx
    };

    let v1_tx = build_single_input_tx(TX_VERSION_TOCCATA, &utxos[0]);
    let valid_v1_rpc_tx: RpcTransaction = (&v1_tx).into();
    let mut malformed_v1_rpc_tx = valid_v1_rpc_tx.clone();
    malformed_v1_rpc_tx.inputs[0].sig_op_count = 1;
    assert!(
        rpc_client.submit_transaction(malformed_v1_rpc_tx, false).await.is_err(),
        "expected v1 transaction with non-zero sig_op_count to be rejected at the daemon boundary"
    );

    let v0_tx = build_single_input_tx(TX_VERSION, &utxos[1]);
    let valid_v0_rpc_tx: RpcTransaction = (&v0_tx).into();
    let mut malformed_v0_rpc_tx: RpcTransaction = valid_v0_rpc_tx.clone();
    malformed_v0_rpc_tx.inputs[0].compute_budget = 1;
    assert!(
        rpc_client.submit_transaction(malformed_v0_rpc_tx, false).await.is_err(),
        "expected v0 transaction with non-zero compute_budget to be rejected at the daemon boundary"
    );

    rpc_client.submit_transaction(valid_v1_rpc_tx, false).await.expect("expected the valid v1 transaction to be accepted");
    rpc_client.submit_transaction(valid_v0_rpc_tx, false).await.expect("expected the valid v0 transaction to be accepted");

    rpc_client.disconnect().await.unwrap();
    kaspad.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_pruning_seqcommit_sync_test() {
    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO,kaspa_testing_integration=trace,kaspa_rpc_core=debug");

    let override_params_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/params/seqcommit_sync_test_params.json");
    let params = load_override_params(&override_params_path);

    let args = Args {
        simnet: true,
        unsafe_rpc: true,
        enable_unsynced_mining: true,
        disable_upnp: true,
        utxoindex: true,
        override_params_file: Some(override_params_path.to_string_lossy().to_string()),
        ..Default::default()
    };

    let total_fd_limit = 10;
    let mut kaspad1 = Daemon::new_random_with_args(args.clone(), total_fd_limit);
    let rpc_client1 = kaspad1.start().await;

    let (miner_sk, miner_pk) = secp256k1::generate_keypair(&mut thread_rng());
    let miner_address =
        Address::new(kaspad1.network.into(), kaspa_addresses::Version::PubKey, &miner_pk.x_only_public_key().0.serialize());
    let miner_schnorr_key = secp256k1::Keypair::from_secret_key(secp256k1::SECP256K1, &miner_sk);

    // Step 1: advance the chain to ~1.5 * finality depth from genesis.
    // We will create a seqcommit transaction at that height, referencing a block
    // almost a full finality_depth below the tip (KIP-21 seqcommit look-back is
    // bounded by `finality_depth`).
    let finality_depth = params.finality_depth();
    let initial_blocks = finality_depth.saturating_mul(3).saturating_div(2) as usize;
    for _ in 0..initial_blocks {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    let mined_check = rpc_client1.clone();
    wait_for(
        50,
        40,
        move || {
            let client = mined_check.clone();
            Box::pin(async move { client.get_server_info().await.unwrap().virtual_daa_score >= initial_blocks as u64 })
        },
        "syncer did not reach the initial finality depth target",
    )
    .await;

    // Choose a target almost a full finality_depth below the current tip, leaving
    // a small buffer for the confirmation and spend blocks.
    let dag_info = rpc_client1.get_block_dag_info().await.unwrap();
    let remaining = finality_depth.saturating_sub(3);
    let target_block = walk_parent_chain(&rpc_client1, dag_info.sink, remaining).await;

    // Build a P2SH redeem script that exercises OpChainblockSeqCommit.
    let mut builder = ScriptBuilder::new();
    builder.add_data(&target_block.as_bytes()).unwrap();
    builder.add_op(codes::OpChainblockSeqCommit).unwrap();
    builder.add_op(codes::OpDrop).unwrap();
    builder.add_op(codes::OpTrue).unwrap();
    let redeem_script = builder.drain();
    let seqcommit_spk = pay_to_script_hash_script(&redeem_script);

    // Fund the P2SH output and confirm it on the syncer at ~1.5 * finality depth.
    let utxos = fetch_spendable_utxos(&rpc_client1, miner_address.clone(), 10).await;
    let input_utxos = &utxos[0..1];
    let total_in = input_utxos.iter().map(|x| x.1.amount).sum::<u64>();
    let fee = fee::calc_for_plain_standard_tx(input_utxos.len(), 1);
    let outputs = vec![TransactionOutput { value: total_in - fee, script_public_key: seqcommit_spk.clone(), covenant: None }];
    let inputs = input_utxos.iter().map(|(op, _)| TransactionInput::new(*op, vec![], 0, 1)).collect();
    let unsigned_tx = Transaction::new(TX_VERSION, inputs, outputs, 0, SUBNETWORK_ID_NATIVE, 0, vec![]);
    let signed_tx =
        sign(MutableTransaction::with_entries(unsigned_tx, input_utxos.iter().map(|(_, e)| e.clone()).collect()), miner_schnorr_key);
    let seqcommit_tx = signed_tx.tx.clone();
    rpc_client1.submit_transaction((&seqcommit_tx).into(), false).await.unwrap();

    let mempool_check = rpc_client1.clone();
    let seqcommit_tx_id = seqcommit_tx.id();
    wait_for(
        50,
        20,
        move || {
            let client = mempool_check.clone();
            Box::pin(async move { client.get_mempool_entry(seqcommit_tx_id, false, false).await.is_ok() })
        },
        "seqcommit transaction was not added to the mempool",
    )
    .await;

    let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
    rpc_client1.submit_block(template.block, false).await.unwrap();

    // Spend the P2SH output to trigger seqcommit validation on the syncer while the target is
    // still within the finality depth of the spending block.
    let outpoint = TransactionOutpoint::new(seqcommit_tx.id(), 0);
    let pay_spk = pay_to_address_script(&miner_address);
    let signature_script = pay_to_script_hash_signature_script(redeem_script, vec![]).expect("canonical signature script");
    let spend_fee = fee::calc_for_plain_standard_tx_with_extra_serialized_bytes(1, 1, signature_script.len() as u64);
    let spend_value = total_in - fee - spend_fee;
    let spend_tx = Transaction::new(
        TX_VERSION,
        vec![TransactionInput::new(outpoint, signature_script, 0, 1)],
        vec![TransactionOutput { value: spend_value, script_public_key: pay_spk, covenant: None }],
        0,
        SUBNETWORK_ID_NATIVE,
        0,
        vec![],
    );
    rpc_client1.submit_transaction((&spend_tx).into(), false).await.unwrap();

    let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
    rpc_client1.submit_block(template.block, false).await.unwrap();

    // Step 2: advance the pruning point so it moves off genesis and ends up above the
    // target block that the seqcommit script references.
    //
    // KIP-21: the seqcommit look-back is `finality_depth`, so the target sits at
    // depth ≈ F below the initial tip. Pruning samples space by F in blue_score, so
    // PP may need to advance to climb past the target.
    let mut dag_info = rpc_client1.get_block_dag_info().await.unwrap();
    let mut extra_blocks = 0usize;
    let extra_blocks_limit = params.pruning_depth().saturating_add(params.finality_depth()).saturating_add(30) as usize;
    while (dag_info.pruning_point_hash == SIMNET_GENESIS.hash
        || !is_ancestor_in_selected_parent_chain(&rpc_client1, dag_info.pruning_point_hash, target_block).await)
        && extra_blocks < extra_blocks_limit
    {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
        extra_blocks += 1;
        dag_info = rpc_client1.get_block_dag_info().await.unwrap();
    }
    if dag_info.pruning_point_hash == SIMNET_GENESIS.hash {
        panic!("pruning point did not advance from genesis in time");
    }
    if !is_ancestor_in_selected_parent_chain(&rpc_client1, dag_info.pruning_point_hash, target_block).await {
        panic!("pruning point did not advance above the seqcommit target in time");
    }

    // Step 3: only now start the syncee and let it sync and validate the seqcommit flow.
    let mut kaspad2 = Daemon::new_random_with_args(args, total_fd_limit);
    let rpc_client2 = kaspad2.start().await;

    rpc_client2.add_peer(format!("127.0.0.1:{}", kaspad1.p2p_port).try_into().unwrap(), true).await.unwrap();
    let check_client = rpc_client2.clone();
    wait_for(
        50,
        40,
        move || {
            let client = check_client.clone();
            Box::pin(async move { client.get_connected_peer_info().await.unwrap().peer_info.len() == 1 })
        },
        "the nodes did not connect to each other",
    )
    .await;

    let sync_check = rpc_client2.clone();
    let target_daa_score = rpc_client1.get_server_info().await.unwrap().virtual_daa_score;
    wait_for(
        100,
        60,
        move || {
            let client = sync_check.clone();
            Box::pin(async move { client.get_server_info().await.unwrap().virtual_daa_score >= target_daa_score })
        },
        "syncee did not complete IBD",
    )
    .await;

    // The spend block is already mined before the pruning point moves, so the syncee
    // should validate it while syncing historical data.

    let synced_check = rpc_client2.clone();
    let final_score = rpc_client1.get_server_info().await.unwrap().virtual_daa_score;
    wait_for(
        100,
        40,
        move || {
            let client = synced_check.clone();
            Box::pin(async move { client.get_server_info().await.unwrap().virtual_daa_score >= final_score })
        },
        "syncee did not accept seqcommit block",
    )
    .await;

    rpc_client1.disconnect().await.unwrap();
    rpc_client2.disconnect().await.unwrap();
    kaspad1.shutdown();
    kaspad2.shutdown();
}

// IBD test focused on `sync_new_smt_state` (protocol/flows/src/ibd/flow.rs:635).
// Produces a non-trivial active-lanes SMT by submitting one transaction per
// distinct subnetwork_id — each distinct subnetwork_id creates a new lane (see
// consensus/src/pipeline/virtual_processor/utxo_validation.rs:532). With the
// `test-smt-small-chunks` feature active the stream uses SMT_CHUNK_SIZE=4 and
// SMT_FLOW_CONTROL_WINDOW=2, so `SMT_LANE_COUNT = 10` forces 3 chunks and one
// flow-control round-trip — exercising both chunked streaming and the
// RequestNextPruningPointSmtChunk handshake end to end.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_ibd_smt_state_sync_test() {
    const SMT_LANE_COUNT: usize = 10;
    const SMT_ANTICONE_COUNT: usize = 4;

    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO,kaspa_testing_integration=trace,kaspa_rpc_core=debug");

    let override_params_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/params/seqcommit_sync_test_params.json");
    let params = load_override_params(&override_params_path);

    let args = Args {
        simnet: true,
        unsafe_rpc: true,
        enable_unsynced_mining: true,
        disable_upnp: true,
        utxoindex: true,
        override_params_file: Some(override_params_path.to_string_lossy().to_string()),
        ..Default::default()
    };

    let total_fd_limit = 10;
    let mut kaspad1 = Daemon::new_random_with_args(args.clone(), total_fd_limit);
    let rpc_client1 = kaspad1.start().await;

    let (miner_sk, miner_pk) = secp256k1::generate_keypair(&mut thread_rng());
    let miner_address =
        Address::new(kaspad1.network.into(), kaspa_addresses::Version::PubKey, &miner_pk.x_only_public_key().0.serialize());
    let miner_schnorr_key = secp256k1::Keypair::from_secret_key(secp256k1::SECP256K1, &miner_sk);

    // Phase 1: mine enough blocks to mature SMT_LANE_COUNT coinbase outputs.
    let coinbase_maturity = params.coinbase_maturity();
    let initial_blocks = (coinbase_maturity as usize) + SMT_LANE_COUNT + 20;
    for _ in 0..initial_blocks {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    let mined_check = rpc_client1.clone();
    wait_for(
        50,
        60,
        move || {
            let client = mined_check.clone();
            Box::pin(async move { client.get_server_info().await.unwrap().virtual_daa_score >= initial_blocks as u64 })
        },
        "syncer did not reach the initial mining target",
    )
    .await;

    let mut anticone_templates = Vec::with_capacity(SMT_ANTICONE_COUNT);
    for i in 0..SMT_ANTICONE_COUNT {
        let extra = format!("anticone-{i:02}").into_bytes();
        anticone_templates.push(rpc_client1.get_block_template(miner_address.clone(), extra).await.unwrap());
    }
    for template in anticone_templates {
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }
    // Bury the siblings under a few chain blocks so virtual's selected
    // parent is past them when the lane txs come in.
    for _ in 0..10 {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    // Phase 2: submit SMT_LANE_COUNT transactions, each on a distinct non-reserved
    // subnetwork_id, so every one populates a fresh lane in the active-lanes SMT.
    let utxos = fetch_spendable_utxos(&rpc_client1, miner_address.clone(), coinbase_maturity).await;
    assert!(utxos.len() >= SMT_LANE_COUNT, "syncer produced {} spendable utxos, need {}", utxos.len(), SMT_LANE_COUNT);

    let mut submitted_tx_ids: Vec<RpcTransactionId> = Vec::with_capacity(SMT_LANE_COUNT);
    for (i, (outpoint, entry)) in utxos.iter().take(SMT_LANE_COUNT).enumerate() {
        // Post-HF user-lane shape is `[namespace (4 bytes), 0×16]` with a
        // non-zero byte somewhere in bytes[1..4] (see
        // consensus/src/processes/transaction_validator/tx_validation_in_isolation.rs).
        // A distinct nonzero byte at position 3 keeps each lane_id unique while
        // conforming to the shape.
        let mut subnet_bytes = [0u8; 20];
        subnet_bytes[3] = (i as u8) + 1;
        let lane_subnet = SubnetworkId::from_bytes(subnet_bytes);

        let fee = fee::calc_for_plain_standard_tx(1, 1);
        assert!(entry.amount > fee, "coinbase utxo is too small to cover a tx fee");
        let out_value = entry.amount - fee;
        let unsigned_tx = Transaction::new(
            TX_VERSION_TOCCATA,
            vec![TransactionInput::new(*outpoint, vec![], 0, 1)],
            vec![TransactionOutput { value: out_value, script_public_key: pay_to_address_script(&miner_address), covenant: None }],
            0,
            lane_subnet,
            0,
            vec![],
        );
        let signed_tx = sign(MutableTransaction::with_entries(unsigned_tx, vec![entry.clone()]), miner_schnorr_key);
        let tx_id = signed_tx.tx.id();
        rpc_client1.submit_transaction((&signed_tx.tx).into(), false).await.unwrap();
        submitted_tx_ids.push(tx_id);
    }

    let mempool_check = rpc_client1.clone();
    let expected_ids = submitted_tx_ids.clone();
    wait_for(
        50,
        40,
        move || {
            let client = mempool_check.clone();
            let ids = expected_ids.clone();
            Box::pin(async move {
                for id in &ids {
                    if client.get_mempool_entry(*id, false, false).await.is_err() {
                        return false;
                    }
                }
                true
            })
        },
        "lane transactions did not reach the mempool",
    )
    .await;

    // Phase 3: mine enough additional blocks that the lane transactions land on
    // chain and the pruning point then advances off genesis. `pruning_depth` + a
    // comfortable margin guarantees the pruning point covers the lane txs.
    let finality_depth = params.finality_depth();
    let pruning_depth = params.pruning_depth();
    let blocks_after_txs = pruning_depth as usize + 60;
    for _ in 0..blocks_after_txs {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    let mut dag_info = rpc_client1.get_block_dag_info().await.unwrap();
    let mut pruning_point_blue_score = rpc_client1.get_block(dag_info.pruning_point_hash, false).await.unwrap().header.blue_score;
    let mut extra_blocks = 0usize;
    let extra_blocks_limit = finality_depth as usize + 100;
    // The pruning point's blue_score must exceed finality_depth so the IBD
    // metadata call to `inactivity_shortcut_block_for_pov(pruning_point)` takes
    // the backward chain-walk branch instead of the shallow `Ok(genesis)`
    // early-return (which triggers when blue_score < finality_depth + 1).
    while (dag_info.pruning_point_hash == SIMNET_GENESIS.hash || pruning_point_blue_score <= finality_depth)
        && extra_blocks < extra_blocks_limit
    {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
        extra_blocks += 1;
        dag_info = rpc_client1.get_block_dag_info().await.unwrap();
        pruning_point_blue_score = rpc_client1.get_block(dag_info.pruning_point_hash, false).await.unwrap().header.blue_score;
    }
    assert_ne!(dag_info.pruning_point_hash, SIMNET_GENESIS.hash, "syncer pruning point did not advance off genesis");
    assert!(
        pruning_point_blue_score > finality_depth,
        "syncer pruning point did not advance past finality depth: pp_bs={pruning_point_blue_score}, finality_depth={finality_depth}"
    );

    let target_daa_score = rpc_client1.get_server_info().await.unwrap().virtual_daa_score;
    let target_pruning_point = dag_info.pruning_point_hash;

    // Phase 4: bring up the syncee and connect it to the syncer.
    let mut kaspad2 = Daemon::new_random_with_args(args, total_fd_limit);
    let rpc_client2 = kaspad2.start().await;

    rpc_client2.add_peer(format!("127.0.0.1:{}", kaspad1.p2p_port).try_into().unwrap(), true).await.unwrap();
    let check_client = rpc_client2.clone();
    wait_for(
        50,
        40,
        move || {
            let client = check_client.clone();
            Box::pin(async move { client.get_connected_peer_info().await.unwrap().peer_info.len() == 1 })
        },
        "the nodes did not connect to each other",
    )
    .await;

    // Phase 5: wait for IBD (including `sync_new_smt_state`) to complete
    let sync_check = rpc_client2.clone();
    wait_for(
        100,
        600,
        move || {
            let client = sync_check.clone();
            Box::pin(async move {
                let server_info = client.get_server_info().await.unwrap();
                if server_info.virtual_daa_score < target_daa_score {
                    return false;
                }
                client.get_block_dag_info().await.unwrap().pruning_point_hash == target_pruning_point
            })
        },
        "syncee did not complete SMT-era IBD within timeout (suspected sync_new_smt_state stall)",
    )
    .await;

    // Phase 6: mine finality_depth + buffer blocks on the syncer and assert
    // the syncee catches up. Verifies syncer/syncee shortcut agreement for live
    // blocks whose target_bs lands in the IBD-imported lane range.
    let post_ibd_blocks = finality_depth as usize + 30;
    for _ in 0..post_ibd_blocks {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    let post_ibd_target_score = rpc_client1.get_server_info().await.unwrap().virtual_daa_score;
    let post_ibd_target_pp = rpc_client1.get_block_dag_info().await.unwrap().pruning_point_hash;
    let post_ibd_check = rpc_client2.clone();
    wait_for(
        100,
        600,
        move || {
            let client = post_ibd_check.clone();
            Box::pin(async move {
                let server_info = client.get_server_info().await.unwrap();
                if server_info.virtual_daa_score < post_ibd_target_score {
                    return false;
                }
                client.get_block_dag_info().await.unwrap().pruning_point_hash == post_ibd_target_pp
            })
        },
        "syncee did not accept post-IBD blocks",
    )
    .await;

    rpc_client1.disconnect().await.unwrap();
    rpc_client2.disconnect().await.unwrap();
    kaspad1.shutdown();
    kaspad2.shutdown();
}

// IBD test focused on `sync_new_pool_state` (FORK-PLAN P6.8): a fresh node syncing from a
// pruning point whose note pool is non-empty must download the pool state, verify it
// against the pruning point header's `pool_commitment`, and end up with a genuinely
// usable pool — proven by (a) IBD completing at all (a root mismatch aborts it), (b) the
// synced pruning point committing to a non-empty pool, (c) the syncee's OWN mempool
// accepting a rotate that consumes notes which exist only in the imported state, and
// (d) the syncee following post-IBD blocks (whose pool commitments extend the imported
// state — with a wrong import every subsequent chain block would fail commitment
// verification and virtual would never advance).
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_ibd_pool_state_sync_test() {
    use kaspa_consensus_core::notepool::{
        DenominationTag, FreshnessAnchor, MintOp, NewNote, PoolOp, SignedGroup, TransferOp, hashing as pool_hashing,
    };
    use kaspa_consensus_core::subnets::SUBNETWORK_ID_NOTE_POOL;

    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO,kaspa_testing_integration=trace");

    let override_params_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/params/seqcommit_sync_test_params.json");
    let params = load_override_params(&override_params_path);

    let args = Args {
        simnet: true,
        unsafe_rpc: true,
        enable_unsynced_mining: true,
        disable_upnp: true,
        utxoindex: true,
        override_params_file: Some(override_params_path.to_string_lossy().to_string()),
        ..Default::default()
    };

    let total_fd_limit = 10;
    let mut kaspad1 = Daemon::new_random_with_args(args.clone(), total_fd_limit);
    let rpc_client1 = kaspad1.start().await;

    let (miner_sk, miner_pk) = secp256k1::generate_keypair(&mut thread_rng());
    let miner_address =
        Address::new(kaspad1.network.into(), kaspa_addresses::Version::PubKey, &miner_pk.x_only_public_key().0.serialize());
    let miner_schnorr_key = secp256k1::Keypair::from_secret_key(secp256k1::SECP256K1, &miner_sk);
    let miner_note_pk: [u8; 32] = miner_pk.x_only_public_key().0.serialize();

    // A second key to rotate the notes to pre-IBD, and a third for the post-IBD rotate.
    let holder_key = secp256k1::Keypair::new(secp256k1::SECP256K1, &mut thread_rng());
    let holder_pk: [u8; 32] = holder_key.public_key().x_only_public_key().0.serialize();
    let final_key = secp256k1::Keypair::new(secp256k1::SECP256K1, &mut thread_rng());
    let final_pk: [u8; 32] = final_key.public_key().x_only_public_key().0.serialize();

    // Signs a one-group rotate of `serials` producing `produced` (POOL-SPEC.md P5.2's
    // signing hash; anchor 0 stays within the 36k freshness window at this test's scale).
    let build_rotate = |signer: &secp256k1::Keypair, serials: Vec<Hash>, produced: Vec<NewNote>| -> Transaction {
        let outputs_hash = pool_hashing::transparent_outputs_hash(&[]);
        let msg_hash = pool_hashing::signing_hash(1, &serials, &produced, outputs_hash, 0);
        let msg = secp256k1::Message::from_digest(msg_hash.into());
        let signature = *signer.sign_schnorr(msg).as_ref();
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup { serials, signature }],
            produced,
            freshness: FreshnessAnchor { anchor_daa_score: 0 },
        });
        Transaction::new(TX_VERSION_TOCCATA, vec![], vec![], 0, SUBNETWORK_ID_NOTE_POOL, 0, op.encode_payload())
    };

    // Phase 1: mine enough blocks for a mature coinbase output.
    let coinbase_maturity = params.coinbase_maturity();
    let initial_blocks = (coinbase_maturity as usize) + 20;
    for _ in 0..initial_blocks {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    // Phase 2: mint three notes (1 + 0.01 + 0.01 MAGLD, all to the miner key) from a real
    // coinbase input (P6.6's value binding), through the real mempool (P6.7's entry path).
    let utxos = fetch_spendable_utxos(&rpc_client1, miner_address.clone(), coinbase_maturity).await;
    let (outpoint, entry) = utxos.first().expect("mature utxo").clone();

    let mint_notes = vec![
        NewNote { d: DenominationTag::D1, pk: miner_note_pk },
        NewNote { d: DenominationTag::D0_01, pk: miner_note_pk },
        NewNote { d: DenominationTag::D0_01, pk: miner_note_pk },
    ];
    let notes_value: u64 = mint_notes.iter().map(|n| n.d.petals()).sum();
    // Generous relay-fee estimate: 1-in-1-out standard shape plus the mint payload bytes.
    let mint_fee = 2 * fee::calc_for_plain_standard_tx_with_extra_serialized_bytes(1, 1, 200);
    assert!(entry.amount > notes_value + mint_fee, "coinbase utxo too small to fund the mint");
    let change = entry.amount - notes_value - mint_fee;
    let unsigned_mint = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![TransactionInput::new(outpoint, vec![], 0, 1)],
        vec![TransactionOutput { value: change, script_public_key: pay_to_address_script(&miner_address), covenant: None }],
        0,
        SUBNETWORK_ID_NOTE_POOL,
        0,
        PoolOp::Mint(MintOp { new_notes: mint_notes }).encode_payload(),
    );
    let mint = sign(MutableTransaction::with_entries(unsigned_mint, vec![entry]), miner_schnorr_key).tx;
    let mint_id = mint.id();
    let mint_serials: Vec<Hash> = (0..3).map(|i| pool_hashing::serial_hash(&mint_id, i)).collect();
    rpc_client1.submit_transaction((&mint).into(), false).await.unwrap();

    // Mine until the mint clears the mempool (i.e. was included and accepted).
    let mint_check = rpc_client1.clone();
    let mint_check_id = mint_id;
    for _ in 0..10 {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }
    wait_for(
        50,
        40,
        move || {
            let client = mint_check.clone();
            Box::pin(async move { client.get_mempool_entry(mint_check_id.into(), false, false).await.is_err() })
        },
        "mint did not clear the syncer mempool",
    )
    .await;

    // Rotate all three notes to the holder key: consumed 1.02, produced 1.01, fee 0.01.
    // Serial-consuming ops can only enter the mempool once their producing op is on chain
    // (P6.7's documented no-unconfirmed-chaining scope), hence the burial above.
    let rotate1_produced =
        vec![NewNote { d: DenominationTag::D1, pk: holder_pk }, NewNote { d: DenominationTag::D0_01, pk: holder_pk }];
    let rotate1 = build_rotate(&miner_schnorr_key, mint_serials.clone(), rotate1_produced);
    let rotate1_id = rotate1.id();
    let holder_serials: Vec<Hash> = (0..2).map(|i| pool_hashing::serial_hash(&rotate1_id, i)).collect();
    rpc_client1.submit_transaction((&rotate1).into(), false).await.unwrap();

    let rotate1_check = rpc_client1.clone();
    for _ in 0..10 {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }
    wait_for(
        50,
        40,
        move || {
            let client = rotate1_check.clone();
            Box::pin(async move { client.get_mempool_entry(rotate1_id.into(), false, false).await.is_err() })
        },
        "rotate did not clear the syncer mempool",
    )
    .await;

    // Phase 3: mine past the pruning depth so the pruning point advances beyond the pool
    // ops; the pool state at the pruning point is then exactly the two holder-key notes.
    let finality_depth = params.finality_depth();
    let pruning_depth = params.pruning_depth();
    let blocks_after_txs = pruning_depth as usize + 60;
    for _ in 0..blocks_after_txs {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    let mut dag_info = rpc_client1.get_block_dag_info().await.unwrap();
    let mut pruning_point_blue_score = rpc_client1.get_block(dag_info.pruning_point_hash, false).await.unwrap().header.blue_score;
    let mut extra_blocks = 0usize;
    let extra_blocks_limit = finality_depth as usize + 100;
    while (dag_info.pruning_point_hash == SIMNET_GENESIS.hash || pruning_point_blue_score <= finality_depth)
        && extra_blocks < extra_blocks_limit
    {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
        extra_blocks += 1;
        dag_info = rpc_client1.get_block_dag_info().await.unwrap();
        pruning_point_blue_score = rpc_client1.get_block(dag_info.pruning_point_hash, false).await.unwrap().header.blue_score;
    }
    assert_ne!(dag_info.pruning_point_hash, SIMNET_GENESIS.hash, "syncer pruning point did not advance off genesis");

    let target_daa_score = rpc_client1.get_server_info().await.unwrap().virtual_daa_score;
    let target_pruning_point = dag_info.pruning_point_hash;

    // The pruning point must commit to a non-empty pool, or this test would pass vacuously.
    let genesis_pool_commitment = rpc_client1.get_block(SIMNET_GENESIS.hash, false).await.unwrap().header.pool_commitment;
    let pp_pool_commitment = rpc_client1.get_block(target_pruning_point, false).await.unwrap().header.pool_commitment;
    assert_ne!(
        pp_pool_commitment, genesis_pool_commitment,
        "pruning point pool commitment is the empty root — the pool ops did not make it below the pruning point"
    );

    // Phase 4: bring up the syncee and connect it to the syncer.
    let mut kaspad2 = Daemon::new_random_with_args(args, total_fd_limit);
    let rpc_client2 = kaspad2.start().await;

    rpc_client2.add_peer(format!("127.0.0.1:{}", kaspad1.p2p_port).try_into().unwrap(), true).await.unwrap();
    let check_client = rpc_client2.clone();
    wait_for(
        50,
        40,
        move || {
            let client = check_client.clone();
            Box::pin(async move { client.get_connected_peer_info().await.unwrap().peer_info.len() == 1 })
        },
        "the nodes did not connect to each other",
    )
    .await;

    // Phase 5: wait for IBD (including `sync_new_pool_state`) to complete. A tampered or
    // wrong pool download would fail the import's root check and stall this wait.
    let sync_check = rpc_client2.clone();
    wait_for(
        100,
        600,
        move || {
            let client = sync_check.clone();
            Box::pin(async move {
                let server_info = client.get_server_info().await.unwrap();
                if server_info.virtual_daa_score < target_daa_score {
                    return false;
                }
                client.get_block_dag_info().await.unwrap().pruning_point_hash == target_pruning_point
            })
        },
        "syncee did not complete pool-state-era IBD within timeout (suspected sync_new_pool_state stall)",
    )
    .await;

    // The syncee serves the same pruning point header, committing to the same non-empty pool.
    let syncee_pp_commitment = rpc_client2.get_block(target_pruning_point, false).await.unwrap().header.pool_commitment;
    assert_eq!(syncee_pp_commitment, pp_pool_commitment);

    // Phase 6a: the sharpest import proof — the syncee's own mempool validates a rotate
    // consuming notes that exist ONLY in the pool state it just imported.
    let rotate2 = build_rotate(&holder_key, holder_serials.clone(), vec![NewNote { d: DenominationTag::D1, pk: final_pk }]);
    let rotate2_id = rotate2.id();
    rpc_client2.submit_transaction((&rotate2).into(), false).await.unwrap();
    rpc_client2
        .get_mempool_entry(rotate2_id.into(), false, false)
        .await
        .expect("syncee mempool rejected a rotate of imported notes — the imported pool state is not serving mempool validation");

    // Phase 6b: mine post-IBD blocks on the syncer and assert the syncee follows — every
    // new chain block's pool commitment now builds on the imported state. (The rotate may
    // also reach the syncer via tx relay and land on-chain; not required for this assert.)
    let post_ibd_blocks = finality_depth as usize + 30;
    for _ in 0..post_ibd_blocks {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    let post_ibd_target_score = rpc_client1.get_server_info().await.unwrap().virtual_daa_score;
    let post_ibd_target_pp = rpc_client1.get_block_dag_info().await.unwrap().pruning_point_hash;
    let post_ibd_check = rpc_client2.clone();
    wait_for(
        100,
        600,
        move || {
            let client = post_ibd_check.clone();
            Box::pin(async move {
                let server_info = client.get_server_info().await.unwrap();
                if server_info.virtual_daa_score < post_ibd_target_score {
                    return false;
                }
                client.get_block_dag_info().await.unwrap().pruning_point_hash == post_ibd_target_pp
            })
        },
        "syncee did not accept post-IBD blocks on top of the imported pool state",
    )
    .await;

    rpc_client1.disconnect().await.unwrap();
    rpc_client2.disconnect().await.unwrap();
    kaspad1.shutdown();
    kaspad2.shutdown();
}

// FORK-PLAN P6.9's verify condition: a client subscribed to NotesChanged sees a notification
// when a mint lands and again when a rotate consumes/produces notes, proving the full
// consensus -> notify -> rpc-core -> grpc wiring end to end (the notify crate's own unit
// tests already cover the subscription-filtering logic in isolation). Also exercises the two
// new "get" RPC methods added alongside the notification.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_notes_changed_notification_test() {
    use kaspa_consensus_core::notepool::{
        DenominationTag, FreshnessAnchor, MintOp, NewNote, PoolOp, SignedGroup, TransferOp, hashing as pool_hashing,
    };
    use kaspa_consensus_core::subnets::SUBNETWORK_ID_NOTE_POOL;

    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO,kaspa_testing_integration=trace,kaspa_notify=debug,kaspa_rpc_core=debug");

    let args = Args {
        simnet: true,
        unsafe_rpc: true,
        enable_unsynced_mining: true,
        disable_upnp: true,
        utxoindex: true,
        ..Default::default()
    };
    let total_fd_limit = 10;
    let mut kaspad1 = Daemon::new_random_with_args(args, total_fd_limit);
    let rpc_client1 = kaspad1.start().await;

    let (miner_sk, miner_pk) = secp256k1::generate_keypair(&mut thread_rng());
    let miner_address =
        Address::new(kaspad1.network.into(), kaspa_addresses::Version::PubKey, &miner_pk.x_only_public_key().0.serialize());
    let miner_schnorr_key = secp256k1::Keypair::from_secret_key(secp256k1::SECP256K1, &miner_sk);
    let miner_note_pk: [u8; 32] = miner_pk.x_only_public_key().0.serialize();
    let holder_key = secp256k1::Keypair::new(secp256k1::SECP256K1, &mut thread_rng());
    let holder_pk: [u8; 32] = holder_key.public_key().x_only_public_key().0.serialize();

    // Mine to a mature coinbase (a margin beyond coinbase_maturity blocks so the earliest
    // coinbase output is actually spendable, mirroring daemon_ibd_pool_state_sync_test).
    let coinbase_maturity = SIMNET_PARAMS.coinbase_maturity();
    let initial_blocks = coinbase_maturity + 20;
    for _ in 0..initial_blocks {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    // Subscribe to ALL NotesChanged events: an empty scope means "all", mirroring
    // UtxosChangedScope's empty-addresses convention (see the notify crate's own
    // "None -> All" mutation test case for the underlying subscription logic).
    let mut client = ListeningClient::connect(&kaspad1).await;
    client.start_notify(NotesChangedScope::default().into()).await.unwrap();

    // Mint two notes (1 + 0.01 MAGLD) from a real mature coinbase input (P6.6's value
    // binding) through the real mempool (P6.7's entry path). Minting more than the rotate
    // below will consume leaves a real pool-value fee (D0_01) behind on the rotate, since a
    // consumed == produced rotate carries zero fee and is rejected by the standard relay
    // policy exactly like a zero-fee transparent transaction would be.
    let utxos = fetch_spendable_utxos(&rpc_client1, miner_address.clone(), coinbase_maturity).await;
    let (outpoint, entry) = utxos.first().expect("mature utxo").clone();
    let mint_notes =
        vec![NewNote { d: DenominationTag::D1, pk: miner_note_pk }, NewNote { d: DenominationTag::D0_01, pk: miner_note_pk }];
    let notes_value: u64 = mint_notes.iter().map(|n| n.d.petals()).sum();
    let mint_fee = 2 * fee::calc_for_plain_standard_tx_with_extra_serialized_bytes(1, 1, 200);
    assert!(entry.amount > notes_value + mint_fee, "coinbase utxo too small to fund the mint");
    let change = entry.amount - notes_value - mint_fee;
    let unsigned_mint = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![TransactionInput::new(outpoint, vec![], 0, 1)],
        vec![TransactionOutput { value: change, script_public_key: pay_to_address_script(&miner_address), covenant: None }],
        0,
        SUBNETWORK_ID_NOTE_POOL,
        0,
        PoolOp::Mint(MintOp { new_notes: mint_notes }).encode_payload(),
    );
    let mint = sign(MutableTransaction::with_entries(unsigned_mint, vec![entry]), miner_schnorr_key).tx;
    let mint_id = mint.id();
    let mint_serials: Vec<Hash> = (0..2).map(|i| pool_hashing::serial_hash(&mint_id, i)).collect();
    rpc_client1.submit_transaction((&mint).into(), false).await.unwrap();

    for _ in 0..10 {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    let mint_notification = client
        .wait_for_notification(
            EventType::NotesChanged,
            Duration::from_secs(30),
            |n| matches!(n, Notification::NotesChanged(msg) if msg.added.iter().any(|e| e.sn == mint_serials[0])),
        )
        .await;
    let Notification::NotesChanged(msg) = mint_notification else { unreachable!() };
    assert_eq!(msg.added.len(), 2);
    for (serial, expected_d) in mint_serials.iter().zip([DenominationTag::D1, DenominationTag::D0_01]) {
        let entry = msg.added.iter().find(|e| e.sn == *serial).unwrap();
        assert_eq!(entry.denomination, expected_d as u8);
        assert_eq!(entry.pk, miner_note_pk);
    }
    assert!(msg.removed.is_empty(), "a fresh mint must not report any removed notes");

    // The two new "get" RPC methods agree with what the notification reported.
    let stats = rpc_client1.get_pool_stats().await.unwrap();
    assert_eq!(stats[DenominationTag::D1 as usize], 1);
    assert_eq!(stats[DenominationTag::D0_01 as usize], 1);
    let fetched = rpc_client1.get_notes_by_serial(mint_serials.clone()).await.unwrap();
    assert_eq!(
        fetched,
        vec![
            RpcNoteEntry { sn: mint_serials[0], denomination: DenominationTag::D1 as u8, pk: miner_note_pk, lock: None },
            RpcNoteEntry { sn: mint_serials[1], denomination: DenominationTag::D0_01 as u8, pk: miner_note_pk, lock: None },
        ]
    );

    client.notes_changed_listener().unwrap().drain();

    // Rotate both notes to a new key, producing only the D1 note: the D0_01 difference is
    // the rotate's fee. The notification should now report both old serials as removed and
    // the new one as added.
    let outputs_hash = pool_hashing::transparent_outputs_hash(&[]);
    let rotate_produced = vec![NewNote { d: DenominationTag::D1, pk: holder_pk }];
    let sig_hash = pool_hashing::signing_hash(1, &mint_serials, &rotate_produced, outputs_hash, 0);
    let sig_msg = secp256k1::Message::from_digest(sig_hash.into());
    let signature = *miner_schnorr_key.sign_schnorr(sig_msg).as_ref();
    let rotate_op = PoolOp::Transfer(TransferOp {
        consumed: vec![SignedGroup { serials: mint_serials.clone(), signature }],
        produced: rotate_produced,
        freshness: FreshnessAnchor { anchor_daa_score: 0 },
    });
    let rotate = Transaction::new(TX_VERSION_TOCCATA, vec![], vec![], 0, SUBNETWORK_ID_NOTE_POOL, 0, rotate_op.encode_payload());
    let rotate_id = rotate.id();
    let holder_serial = pool_hashing::serial_hash(&rotate_id, 0);
    rpc_client1.submit_transaction((&rotate).into(), false).await.unwrap();

    for _ in 0..10 {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    let rotate_notification = client
        .wait_for_notification(
            EventType::NotesChanged,
            Duration::from_secs(30),
            |n| matches!(n, Notification::NotesChanged(msg) if msg.removed.iter().any(|e| e.sn == mint_serials[0])),
        )
        .await;
    let Notification::NotesChanged(msg) = rotate_notification else { unreachable!() };
    assert_eq!(msg.removed.len(), 2);
    for serial in &mint_serials {
        assert!(msg.removed.iter().any(|e| e.sn == *serial));
    }
    assert!(msg.added.iter().any(|e| e.sn == holder_serial && e.pk == holder_pk));

    rpc_client1.disconnect().await.unwrap();
    kaspad1.shutdown();
}

/// FORK-PLAN P6.10's "pool-root agreement across nodes" verify criterion: three real,
/// independent nodes (not two, unlike `daemon_ibd_pool_state_sync_test` — genuinely N,
/// not a special-cased pair) in a star topology around a miner, fed a real mix of mint,
/// split, merge, and redeem transactions relayed over P2P (not hand-imported), converge
/// on identical pool state. Cross-checked two ways: `header.pool_commitment` at the
/// shared sink (the same signal `daemon_ibd_pool_state_sync_test` uses) AND
/// `get_pool_stats()` (P6.9's own RPC surface) at the live tip — the latter is a more
/// direct "the actual pool contents agree" signal than a commitment hash alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_notepool_multi_node_agreement_test() {
    use kaspa_consensus_core::notepool::{
        DenominationTag, FreshnessAnchor, MintOp, NewNote, PoolOp, SignedGroup, TransferOp, hashing as pool_hashing,
    };
    use kaspa_consensus_core::subnets::SUBNETWORK_ID_NOTE_POOL;

    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO,kaspa_testing_integration=trace");

    let args = Args {
        simnet: true,
        unsafe_rpc: true,
        enable_unsynced_mining: true,
        disable_upnp: true,
        utxoindex: true,
        ..Default::default()
    };
    let total_fd_limit = 10;
    let mut kaspad1 = Daemon::new_random_with_args(args.clone(), total_fd_limit);
    let mut kaspad2 = Daemon::new_random_with_args(args.clone(), total_fd_limit);
    let mut kaspad3 = Daemon::new_random_with_args(args, total_fd_limit);
    let rpc_client1 = kaspad1.start().await;
    let rpc_client2 = kaspad2.start().await;
    let rpc_client3 = kaspad3.start().await;

    // Star topology: kaspad2 and kaspad3 both peer directly to kaspad1, the miner.
    for client in [&rpc_client2, &rpc_client3] {
        client.add_peer(format!("127.0.0.1:{}", kaspad1.p2p_port).try_into().unwrap(), true).await.unwrap();
    }
    let check_client = rpc_client1.clone();
    wait_for(
        50,
        40,
        move || {
            let client = check_client.clone();
            Box::pin(async move { client.get_connected_peer_info().await.unwrap().peer_info.len() == 2 })
        },
        "kaspad1 did not see both peers connect",
    )
    .await;

    let (miner_sk, miner_pk) = secp256k1::generate_keypair(&mut thread_rng());
    let miner_address =
        Address::new(kaspad1.network.into(), kaspa_addresses::Version::PubKey, &miner_pk.x_only_public_key().0.serialize());
    let miner_schnorr_key = secp256k1::Keypair::from_secret_key(secp256k1::SECP256K1, &miner_sk);
    let alice_note_pk: [u8; 32] = miner_pk.x_only_public_key().0.serialize();
    let bob_key = secp256k1::Keypair::new(secp256k1::SECP256K1, &mut thread_rng());
    let bob_pk: [u8; 32] = bob_key.public_key().x_only_public_key().0.serialize();

    // Signs a one-group Transfer (rotate/split/merge — same shape, POOL-SPEC.md P5.3
    // draws no wire-level distinction between them) consuming `serials`, producing
    // `produced`, mirroring `daemon_ibd_pool_state_sync_test`'s `build_rotate`.
    let build_transfer = |signer: &secp256k1::Keypair, serials: Vec<Hash>, produced: Vec<NewNote>| -> Transaction {
        let outputs_hash = pool_hashing::transparent_outputs_hash(&[]);
        let msg_hash = pool_hashing::signing_hash(1, &serials, &produced, outputs_hash, 0);
        let msg = secp256k1::Message::from_digest(msg_hash.into());
        let signature = *signer.sign_schnorr(msg).as_ref();
        let op = PoolOp::Transfer(TransferOp {
            consumed: vec![SignedGroup { serials, signature }],
            produced,
            freshness: FreshnessAnchor { anchor_daa_score: 0 },
        });
        Transaction::new(TX_VERSION_TOCCATA, vec![], vec![], 0, SUBNETWORK_ID_NOTE_POOL, 0, op.encode_payload())
    };
    // Signs a one-group Redeem consuming `serials` into `outputs`.
    let build_redeem = |signer: &secp256k1::Keypair, serials: Vec<Hash>, outputs: Vec<TransactionOutput>| -> Transaction {
        let outputs_hash = pool_hashing::transparent_outputs_hash(&outputs);
        let msg_hash = pool_hashing::signing_hash(2, &serials, &[], outputs_hash, 0);
        let msg = secp256k1::Message::from_digest(msg_hash.into());
        let signature = *signer.sign_schnorr(msg).as_ref();
        let op = kaspa_consensus_core::notepool::RedeemOp {
            consumed: vec![SignedGroup { serials, signature }],
            freshness: FreshnessAnchor { anchor_daa_score: 0 },
        };
        Transaction::new(TX_VERSION_TOCCATA, vec![], outputs, 0, SUBNETWORK_ID_NOTE_POOL, 0, PoolOp::Redeem(op).encode_payload())
    };
    // Mines `n` blocks on kaspad1 and waits for a transaction to leave its mempool
    // (i.e. confirmed) before returning — P6.7's no-unconfirmed-chaining scope means
    // each op below must be buried before the next one (which consumes its output) can
    // enter the mempool.
    async fn mine_until_confirmed(rpc_client1: &GrpcClient, miner_address: &Address, txid: Hash) {
        for _ in 0..10 {
            let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
            rpc_client1.submit_block(template.block, false).await.unwrap();
        }
        let client = rpc_client1.clone();
        wait_for(
            50,
            40,
            move || {
                let client = client.clone();
                Box::pin(async move { client.get_mempool_entry(txid.into(), false, false).await.is_err() })
            },
            "pool-op transaction did not clear the mempool",
        )
        .await;
    }

    // Mine to a mature coinbase.
    let coinbase_maturity = SIMNET_PARAMS.coinbase_maturity();
    for _ in 0..(coinbase_maturity + 20) {
        let template = rpc_client1.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        rpc_client1.submit_block(template.block, false).await.unwrap();
    }

    // TX1 — Mint: 0.1 + 0.01 + 0.01 MAGLD to alice, from a real coinbase input.
    let utxos = fetch_spendable_utxos(&rpc_client1, miner_address.clone(), coinbase_maturity).await;
    let (outpoint, entry) = utxos.first().expect("mature utxo").clone();
    let mint_notes = vec![
        NewNote { d: DenominationTag::D0_1, pk: alice_note_pk },
        NewNote { d: DenominationTag::D0_01, pk: alice_note_pk },
        NewNote { d: DenominationTag::D0_01, pk: alice_note_pk },
    ];
    let notes_value: u64 = mint_notes.iter().map(|n| n.d.petals()).sum();
    let mint_fee = 2 * fee::calc_for_plain_standard_tx_with_extra_serialized_bytes(1, 1, 200);
    assert!(entry.amount > notes_value + mint_fee, "coinbase utxo too small to fund the mint");
    let change = entry.amount - notes_value - mint_fee;
    let unsigned_mint = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![TransactionInput::new(outpoint, vec![], 0, 1)],
        vec![TransactionOutput { value: change, script_public_key: pay_to_address_script(&miner_address), covenant: None }],
        0,
        SUBNETWORK_ID_NOTE_POOL,
        0,
        PoolOp::Mint(MintOp { new_notes: mint_notes }).encode_payload(),
    );
    let mint = sign(MutableTransaction::with_entries(unsigned_mint, vec![entry]), miner_schnorr_key).tx;
    let mint_id = mint.id();
    let d01_sn = pool_hashing::serial_hash(&mint_id, 0);
    let split_source_sn0 = pool_hashing::serial_hash(&mint_id, 1);
    let split_source_sn1 = pool_hashing::serial_hash(&mint_id, 2);
    rpc_client1.submit_transaction((&mint).into(), false).await.unwrap();
    mine_until_confirmed(&rpc_client1, &miner_address, mint_id).await;

    // TX2 — Split: 0.1 MAGLD -> 4 x 0.01 MAGLD to bob (0.06 fee).
    let split_produced: Vec<NewNote> = (0..4).map(|_| NewNote { d: DenominationTag::D0_01, pk: bob_pk }).collect();
    let split = build_transfer(&miner_schnorr_key, vec![d01_sn], split_produced);
    let split_id = split.id();
    let split_sns: Vec<Hash> = (0..4).map(|i| pool_hashing::serial_hash(&split_id, i)).collect();
    rpc_client1.submit_transaction((&split).into(), false).await.unwrap();
    mine_until_confirmed(&rpc_client1, &miner_address, split_id).await;

    // TX3 — Merge: alice's original two 0.01-MAGLD notes -> one 0.01 to bob (0.01 fee).
    let merge = build_transfer(
        &miner_schnorr_key,
        vec![split_source_sn0, split_source_sn1],
        vec![NewNote { d: DenominationTag::D0_01, pk: bob_pk }],
    );
    let merge_id = merge.id();
    let merged_sn = pool_hashing::serial_hash(&merge_id, 0);
    rpc_client1.submit_transaction((&merge).into(), false).await.unwrap();
    mine_until_confirmed(&rpc_client1, &miner_address, merge_id).await;

    // TX4 — Redeem: bob's 4 x 0.01 from the split -> a 0.03 transparent output (0.01 fee).
    let bob_address = Address::new(kaspad1.network.into(), kaspa_addresses::Version::PubKey, &bob_pk);
    let redeem_output = TransactionOutput::new(3 * DenominationTag::D0_01.petals(), pay_to_address_script(&bob_address));
    let redeem = build_redeem(&bob_key, split_sns, vec![redeem_output]);
    let redeem_id = redeem.id();
    rpc_client1.submit_transaction((&redeem).into(), false).await.unwrap();
    mine_until_confirmed(&rpc_client1, &miner_address, redeem_id).await;

    // Final live pool state: exactly the merged note. Wait for all three nodes to reach
    // the same sink (P2P relay + mining, not a hand-import), then compare.
    let target_sink = rpc_client1.get_block_dag_info().await.unwrap().sink;
    for client in [&rpc_client2, &rpc_client3] {
        let client = client.clone();
        wait_for(
            50,
            40,
            move || {
                let client = client.clone();
                Box::pin(async move { client.get_block_dag_info().await.unwrap().sink == target_sink })
            },
            "a peer did not sync to the miner's sink",
        )
        .await;
    }

    let commitment1 = rpc_client1.get_block(target_sink, false).await.unwrap().header.pool_commitment;
    let commitment2 = rpc_client2.get_block(target_sink, false).await.unwrap().header.pool_commitment;
    let commitment3 = rpc_client3.get_block(target_sink, false).await.unwrap().header.pool_commitment;
    assert_eq!(commitment1, commitment2, "kaspad2's pool commitment must match the miner's at the shared sink");
    assert_eq!(commitment1, commitment3, "kaspad3's pool commitment must match the miner's at the shared sink");

    let stats1 = rpc_client1.get_pool_stats().await.unwrap();
    let stats2 = rpc_client2.get_pool_stats().await.unwrap();
    let stats3 = rpc_client3.get_pool_stats().await.unwrap();
    assert_eq!(stats1, stats2, "kaspad2's live pool stats must match the miner's");
    assert_eq!(stats1, stats3, "kaspad3's live pool stats must match the miner's");
    assert_eq!(stats1[DenominationTag::D0_01 as usize], 1, "exactly the merged note should remain live");
    assert_eq!(stats1.iter().sum::<u64>(), 1, "no other denomination should have a live note");

    let merged = rpc_client1.get_notes_by_serial(vec![merged_sn]).await.unwrap();
    assert_eq!(merged, vec![RpcNoteEntry { sn: merged_sn, denomination: DenominationTag::D0_01 as u8, pk: bob_pk, lock: None }]);

    rpc_client1.disconnect().await.unwrap();
    rpc_client2.disconnect().await.unwrap();
    rpc_client3.disconnect().await.unwrap();
    kaspad1.shutdown();
    kaspad2.shutdown();
    kaspad3.shutdown();
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// Finality-anchor distribution tests (POOL-SPEC.md P5.8, FORK-PLAN P6.12)

mod finality_anchor_helpers {
    use super::*;
    use kaspa_consensus_core::config::params::FinalityAnchorParams;
    use kaspa_consensus_core::finality_anchor::{FinalityAnchor, TRUSTEE_COUNT, TrusteeKeys, signing_hash};

    pub fn trustee_keypairs() -> Vec<secp256k1::Keypair> {
        (101..=100 + TRUSTEE_COUNT as u8)
            .map(|seed| secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &[seed; 32]).unwrap())
            .collect()
    }

    pub fn trustee_keys(keypairs: &[secp256k1::Keypair]) -> TrusteeKeys {
        let keys: Vec<[u8; 32]> = keypairs.iter().map(|kp| kp.public_key().x_only_public_key().0.serialize()).collect();
        keys.try_into().unwrap()
    }

    pub fn sign_anchor(keypairs: &[secp256k1::Keypair], signers: &[u8], block: Hash, score: u64) -> FinalityAnchor {
        let mut sorted = signers.to_vec();
        sorted.sort_unstable();
        FinalityAnchor {
            anchored_block: block,
            anchored_daa_score: score,
            signer_bitmap: sorted.iter().fold(0u8, |bitmap, &i| bitmap | (1 << i)),
            signatures: sorted
                .iter()
                .map(|&i| {
                    let msg = secp256k1::Message::from_digest(signing_hash(&block, score).into());
                    *secp256k1::SECP256K1.sign_schnorr_no_aux_rand(&msg, &keypairs[i as usize]).as_ref()
                })
                .collect(),
        }
    }

    /// Writes a simnet override-params file with trustee keys pinned and a small
    /// depth/interval (test scale), returning its path. `test_tag` keeps concurrent
    /// tests' files apart.
    pub fn write_anchor_params_file(keypairs: &[secp256k1::Keypair], depth: u64, interval: u64, test_tag: &str) -> String {
        let mut params = SIMNET_PARAMS.clone();
        params.finality_anchor = FinalityAnchorParams {
            trustees: Some(trustee_keys(keypairs)),
            depth,
            launch_interval: interval,
            ..FinalityAnchorParams::LAUNCH_UNKEYED
        };
        let override_params: OverrideParams = params.into();
        let path = std::env::temp_dir().join(format!("marigold_anchor_params_{test_tag}.json"));
        fs::write(&path, serde_json::to_string_pretty(&override_params).unwrap()).unwrap();
        path.to_string_lossy().to_string()
    }

    /// Finds the highest selected-chain block whose DAA score is at least `depth`
    /// behind the current virtual DAA score, returning (hash, daa_score).
    pub async fn chain_block_at_depth(client: &GrpcClient, depth: u64) -> (Hash, u64) {
        let dag_info = client.get_block_dag_info().await.unwrap();
        let bound = client.get_server_info().await.unwrap().virtual_daa_score.saturating_sub(depth);
        let chain = client.get_virtual_chain_from_block(dag_info.pruning_point_hash, false, None).await.unwrap();
        for hash in chain.added_chain_block_hashes.iter().rev() {
            let header = client.get_block(*hash, false).await.unwrap().header;
            if header.daa_score <= bound {
                return (*hash, header.daa_score);
            }
        }
        panic!("no chain block at depth {depth}");
    }
}

/// P6.12 verify criterion 1: "a fresh node offered only an attacker chain refuses it
/// once it learns the latest anchor." Node A carries the honest (lighter) anchored
/// chain — its anchor submitted through real RPC into the real mempool (exercising the
/// anchor lane's zero-fee exemption and forced template inclusion). Node B mines a
/// strictly heavier anchor-free chain. A fresh node C syncs from A (learning the
/// anchor from A's chain and gossip), then connects to B — and must keep refusing B's
/// heavier chain, both at the P6.11 fork-choice guard and the P6.12 anchor-aware IBD
/// check.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_anchor_refuses_heavier_anchorless_chain_test() {
    use finality_anchor_helpers::*;

    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO,kaspa_testing_integration=trace");

    let keypairs = trustee_keypairs();
    let params_file = write_anchor_params_file(&keypairs, 3, 10, "refusal");
    let args = Args {
        simnet: true,
        unsafe_rpc: true,
        enable_unsynced_mining: true,
        disable_upnp: true,
        override_params_file: Some(params_file),
        ..Default::default()
    };
    let total_fd_limit = 10;

    // Node A: the honest chain.
    let mut kaspad_a = Daemon::new_random_with_args(args.clone(), total_fd_limit);
    let client_a = kaspad_a.start().await;
    let (_, miner_pk) = secp256k1::generate_keypair(&mut thread_rng());
    let miner_address =
        Address::new(kaspad_a.network.into(), kaspa_addresses::Version::PubKey, &miner_pk.x_only_public_key().0.serialize());
    for _ in 0..30 {
        let template = client_a.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        client_a.submit_block(template.block, false).await.unwrap();
    }

    // Craft and SUBMIT the anchor through RPC — mempool + forced template inclusion.
    let (anchored_block, anchored_score) = chain_block_at_depth(&client_a, 3).await;
    let anchor = sign_anchor(&keypairs, &[0, 1, 2], anchored_block, anchored_score);
    let anchor_tx = kaspa_trustee_signer::anchor_transaction(&anchor);
    client_a.submit_transaction((&anchor_tx).into(), false).await.unwrap();
    let anchor_tx_id = anchor_tx.id();
    for _ in 0..10 {
        let template = client_a.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        client_a.submit_block(template.block, false).await.unwrap();
    }
    let check_client = client_a.clone();
    wait_for(
        100,
        200,
        move || {
            let client = check_client.clone();
            Box::pin(async move { client.get_mempool_entry(anchor_tx_id.into(), false, false).await.is_err() })
        },
        "the anchor tx did not clear node A's mempool (template inclusion failed?)",
    )
    .await;
    let status_a = client_a.get_finality_anchor_status().await.unwrap();
    assert!(status_a.has_anchor, "node A must have accepted its own mined anchor");
    assert!(status_a.enforcing);
    assert_eq!(status_a.latest_anchored_block, anchored_block);

    // Node B: a strictly heavier, anchor-free chain, never connected to A.
    let mut kaspad_b = Daemon::new_random_with_args(args.clone(), total_fd_limit);
    let client_b = kaspad_b.start().await;
    for _ in 0..80 {
        let template = client_b.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        client_b.submit_block(template.block, false).await.unwrap();
    }
    let sink_a = client_a.get_block_dag_info().await.unwrap().sink;
    let sink_b = client_b.get_block_dag_info().await.unwrap().sink;
    let work_a = client_a.get_block(sink_a, false).await.unwrap().header.blue_work;
    let work_b = client_b.get_block(sink_b, false).await.unwrap().header.blue_work;
    assert!(work_b > work_a, "the attacker chain must be strictly heavier for this test to mean anything");
    assert!(!client_b.get_finality_anchor_status().await.unwrap().has_anchor);

    // Fresh node C: syncs from A first (learning the anchor), then meets B.
    let mut kaspad_c = Daemon::new_random_with_args(args, total_fd_limit);
    let client_c = kaspad_c.start().await;
    client_c.add_peer(format!("127.0.0.1:{}", kaspad_a.p2p_port).try_into().unwrap(), true).await.unwrap();
    let check_client = client_c.clone();
    wait_for(
        100,
        600,
        move || {
            let client = check_client.clone();
            Box::pin(async move { client.get_block_dag_info().await.unwrap().sink == sink_a })
        },
        "node C did not sync node A's chain",
    )
    .await;
    let status_c = client_c.get_finality_anchor_status().await.unwrap();
    assert!(status_c.has_anchor, "node C must have learned the anchor while syncing A's chain");
    assert!(status_c.enforcing);

    // Now C meets the heavier attacker chain.
    client_c.add_peer(format!("127.0.0.1:{}", kaspad_b.p2p_port).try_into().unwrap(), true).await.unwrap();
    for _ in 0..20 {
        let template = client_b.get_block_template(miner_address.clone(), vec![]).await.unwrap();
        client_b.submit_block(template.block, false).await.unwrap();
    }
    // Give relay/IBD every chance to (wrongly) capture C, then assert it held.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let sink_c = client_c.get_block_dag_info().await.unwrap().sink;
    assert_ne!(sink_c, client_b.get_block_dag_info().await.unwrap().sink, "node C must not adopt the heavier anchor-free chain");
    // The anchored block must still be on C's selected chain (the call errs if the
    // start hash is not a chain block).
    client_c
        .get_virtual_chain_from_block(anchored_block, false, None)
        .await
        .expect("the anchored block must remain on node C's selected chain");
    assert!(client_c.get_finality_anchor_status().await.unwrap().enforcing);

    client_a.disconnect().await.unwrap();
    client_b.disconnect().await.unwrap();
    client_c.disconnect().await.unwrap();
    kaspad_a.shutdown();
    kaspad_b.shutdown();
    kaspad_c.shutdown();
}

/// P6.12 verify criterion 2: "a 3-of-5 signer setup on the local testnet produces
/// anchors continuously and all nodes report finality within one cadence interval."
/// Three in-process `TrusteeSigner` instances (one key each, exchanging partials over
/// real localhost TCP) watch a continuously-mined node; both connected nodes must
/// reach `enforcing && !stale` — "reporting finality" — and the anchor must keep
/// advancing across cadence intervals.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_trustee_signers_produce_anchors_test() {
    use finality_anchor_helpers::*;
    use kaspa_trustee_signer::{SignerConfig, TrusteeSigner};

    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("INFO,kaspa_testing_integration=trace");

    let keypairs = trustee_keypairs();
    let params_file = write_anchor_params_file(&keypairs, 3, 10, "signers");
    let args = Args {
        simnet: true,
        unsafe_rpc: true,
        enable_unsynced_mining: true,
        disable_upnp: true,
        override_params_file: Some(params_file),
        ..Default::default()
    };
    let total_fd_limit = 10;

    let (_, miner_pk) = secp256k1::generate_keypair(&mut thread_rng());
    let mut kaspad_a = Daemon::new_random_with_args(args.clone(), total_fd_limit);
    let client_a = kaspad_a.start().await;
    let mut kaspad_b = Daemon::new_random_with_args(args, total_fd_limit);
    let client_b = kaspad_b.start().await;
    let miner_address =
        Address::new(kaspad_a.network.into(), kaspa_addresses::Version::PubKey, &miner_pk.x_only_public_key().0.serialize());
    client_b.add_peer(format!("127.0.0.1:{}", kaspad_a.p2p_port).try_into().unwrap(), true).await.unwrap();

    // Continuous miner on A.
    let stop_mining = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let miner = {
        let client = client_a.clone();
        let stop = stop_mining.clone();
        let address = miner_address.clone();
        tokio::spawn(async move {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                // Resilient to transient RPC errors under full-suite load: a panicking
                // spawned task would silently stop the chain (and with it the anchors)
                if let Ok(template) = client.get_block_template(address.clone(), vec![]).await {
                    let _ = client.submit_block(template.block, false).await;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
    };

    // Three signers (a 3-of-5 quorum), one key each, real TCP partial exchange.
    let free_port = || {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    };
    let ports: Vec<u16> = (0..3).map(|_| free_port()).collect();
    let mut signer_tasks = Vec::new();
    for i in 0..3u8 {
        let config = SignerConfig {
            rpc_server: format!("127.0.0.1:{}", kaspad_a.rpc_port),
            trustee_index: i,
            secret_key: keypairs[i as usize].secret_bytes(),
            listen_address: Some(format!("127.0.0.1:{}", ports[i as usize])),
            peer_addresses: ports
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i as usize)
                .map(|(_, p)| format!("127.0.0.1:{p}"))
                .collect(),
            depth: 3,
            interval: 10,
            poll_millis: 100,
            trustee_keys: Some(trustee_keys(&keypairs)),
            state_file: std::env::temp_dir().join(format!("marigold_signer_state_{i}_{}.txt", kaspad_a.rpc_port)),
        };
        let signer = TrusteeSigner::new(config).await.expect("signer init");
        signer_tasks.push(tokio::spawn(signer.run()));
    }

    // Both nodes must reach enforced, fresh (non-stale) anchored finality.
    for (name, client) in [("A", &client_a), ("B", &client_b)] {
        let client = client.clone();
        wait_for(
            100,
            600,
            move || {
                let client = client.clone();
                Box::pin(async move {
                    let status = client.get_finality_anchor_status().await.unwrap();
                    status.has_anchor && status.enforcing && !status.stale
                })
            },
            if name == "A" {
                "node A did not reach enforced anchored finality"
            } else {
                "node B did not reach enforced anchored finality"
            },
        )
        .await;
    }

    // Continuity: the anchor must keep advancing across cadence intervals.
    let first_score = client_a.get_finality_anchor_status().await.unwrap().latest_anchored_daa_score;
    let client = client_a.clone();
    wait_for(
        100,
        600,
        move || {
            let client = client.clone();
            let baseline = first_score;
            Box::pin(async move { client.get_finality_anchor_status().await.unwrap().latest_anchored_daa_score > baseline })
        },
        "the anchor did not advance to the next cadence interval",
    )
    .await;

    stop_mining.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = miner.await;
    for task in signer_tasks {
        task.abort();
    }
    client_a.disconnect().await.unwrap();
    client_b.disconnect().await.unwrap();
    kaspad_a.shutdown();
    kaspad_b.shutdown();
}

// The following test runtime parameters are required for a graceful shutdown of the gRPC server
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn daemon_cleaning_test() {
    init_allocator_with_default_settings();
    kaspa_core::log::try_init_logger("info,kaspa_grpc_core=trace,kaspa_grpc_server=trace,kaspa_grpc_client=trace,kaspa_core=trace");
    let args = Args { devnet: true, ..Default::default() };
    let consensus_manager;
    let async_runtime;
    let core;
    {
        let total_fd_limit = 10;
        let mut kaspad1 = Daemon::new_random_with_args(args, total_fd_limit);
        let dyn_consensus_manager = kaspad1.core.find(ConsensusManager::IDENT).unwrap();
        let dyn_async_runtime = kaspad1.core.find(AsyncRuntime::IDENT).unwrap();
        consensus_manager = Arc::downgrade(&Arc::downcast::<ConsensusManager>(dyn_consensus_manager.into_any_arc()).unwrap());
        async_runtime = Arc::downgrade(&Arc::downcast::<AsyncRuntime>(dyn_async_runtime.into_any_arc()).unwrap());
        core = Arc::downgrade(&kaspad1.core);

        let rpc_client1 = kaspad1.start().await;
        rpc_client1.disconnect().await.unwrap();
        drop(rpc_client1);
        kaspad1.shutdown();
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(consensus_manager.strong_count(), 0);
    assert_eq!(async_runtime.strong_count(), 0);
    assert_eq!(core.strong_count(), 0);
}
