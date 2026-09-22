//! The trustee signer daemon binary (PLAN P6.12) — see the library doc for the
//! signing discipline and transport. One instance per trustee key.

use clap::Parser;
use kaspa_core::info;
use kaspa_trustee_signer::{SignerConfig, TrusteeSigner};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about = "Marigold finality-anchor trustee signer", long_about = None)]
struct Args {
    /// The node's RPC address: `host:port` for gRPC, or a `ws://` URL for a node
    /// that speaks only wRPC (the wallet's embedded node)
    #[arg(short, long, default_value = "127.0.0.1:26110")]
    rpc_server: String,

    /// This signer's trustee index (0..5) into the network's pinned key set
    #[arg(short, long, required_unless_present_any = ["generate_key", "rehearse_forgeries", "drill_status"])]
    trustee_index: Option<u8>,

    /// Make a fresh trustee key: write the secret to this file (owner-only), print
    /// the public key to pin in params, and exit. Refuses to overwrite.
    #[arg(long, value_name = "PATH", exclusive = true)]
    generate_key: Option<PathBuf>,

    /// Rehearse anchor forgeries against the node at --rpc-server instead of running:
    /// submit every shape of bad anchor and report that each was refused and the
    /// node's anchor state did not move. Needs no key. Exit code 1 on any failure.
    #[arg(long)]
    rehearse_forgeries: bool,

    /// Print one line with the node's sink, DAA score and anchor state, and exit
    /// (the anchor drill's status probe, scripts/anchor-drill.sh).
    #[arg(long)]
    drill_status: bool,

    /// Sign a release notice instead of running: the oldest wallet release
    /// (`major.release`, e.g. `2.58`) that still follows this network's consensus.
    /// Prints one JSON line with this trustee's signature and exits. Needs
    /// --trustee-index, one of the key options, and --network.
    #[arg(long, value_name = "MAJOR.RELEASE")]
    sign_release_notice: Option<String>,

    /// The network the release notice is for, as the wallet names it: `testnet-10`
    /// or `mainnet`
    #[arg(long, requires = "sign_release_notice")]
    network: Option<String>,

    /// Unix seconds the release notice is issued at; every trustee must sign the
    /// same value. Defaults to now.
    #[arg(long, requires = "sign_release_notice")]
    issued_at: Option<u64>,

    /// This trustee's BIP340 secret key, hex-encoded (64 chars). Prefer --key-file.
    #[arg(long, conflicts_with = "key_file")]
    secret_key: Option<String>,

    /// Path to a file whose first line is the hex-encoded secret key
    #[arg(long)]
    key_file: Option<PathBuf>,

    /// TCP address to receive peer signer partials on (omit to disable the listener)
    #[arg(short, long)]
    listen: Option<String>,

    /// Comma-separated peer signer partial-exchange addresses
    #[arg(short, long, value_delimiter = ',')]
    peers: Vec<String>,

    /// Anchoring depth in DAA-score units (must match the network's params)
    #[arg(long, default_value_t = kaspa_consensus_core::finality_anchor::FINALITY_ANCHOR_DEPTH)]
    depth: u64,

    /// Cadence interval in DAA-score units (must match the network's active stage)
    #[arg(long, default_value_t = kaspa_consensus_core::finality_anchor::FINALITY_ANCHOR_LAUNCH_INTERVAL)]
    interval: u64,

    /// Node poll period in milliseconds
    #[arg(long, default_value_t = 1000)]
    poll_millis: u64,

    /// Comma-separated hex-encoded trustee public keys (all 5, ascending index), for
    /// verifying incoming partials before aggregation. Optional but recommended.
    #[arg(long, value_delimiter = ',')]
    trustee_keys: Vec<String>,

    /// Path of the last-signed persistence file (the equivocation-safety state).
    /// MUST be stable across restarts — see the library doc for why.
    #[arg(long, default_value = "trustee-signer.state")]
    state_file: PathBuf,
}

fn parse_hex32(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    faster_hex::hex_decode(s.trim().as_bytes(), &mut out).expect("expected 64 hex chars");
    out
}

#[tokio::main]
async fn main() {
    kaspa_core::log::init_logger(None, "info");
    let args = Args::parse();

    if let Some(path) = &args.generate_key {
        let (sk, pk) = secp256k1::generate_keypair(&mut secp256k1::rand::thread_rng());
        let (xonly, _) = pk.x_only_public_key();
        let hex_secret = faster_hex::hex_string(&sk.secret_bytes());
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .expect("cannot create the key file (does it exist already?)");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600)).expect("cannot set the key file's mode");
        }
        use std::io::Write;
        writeln!(file, "{hex_secret}").expect("cannot write the key file");
        println!("{}", faster_hex::hex_string(&xonly.serialize()));
        return;
    }
    if args.drill_status {
        let client = kaspa_trustee_signer::connect_rpc(&args.rpc_server).await.expect("cannot reach the node");
        println!("{}", kaspa_trustee_signer::rehearsal::drill_status_line(&client).await.expect("status failed"));
        return;
    }
    if args.rehearse_forgeries {
        let client = kaspa_trustee_signer::connect_rpc(&args.rpc_server).await.expect("cannot reach the node");
        let report = kaspa_trustee_signer::rehearsal::rehearse_forgeries(&client).await.expect("rehearsal failed to run");
        println!("anchor status before: {}", report.status_before);
        for case in &report.cases {
            println!("[{}] {} — {}", if case.passed() { "ok" } else { "FAIL" }, case.name, case.answer);
        }
        println!("anchor status after:  {}", report.status_after);
        if report.ratchet_moved {
            println!("[FAIL] the anchor ratchet moved to the forgeries' target");
        }
        println!("{}", if report.passed() { "rehearsal passed: every forgery refused, ratchet unmoved" } else { "REHEARSAL FAILED" });
        std::process::exit(if report.passed() { 0 } else { 1 });
    }
    let trustee_index = args.trustee_index.expect("--trustee-index is required");

    let secret_key = match (&args.secret_key, &args.key_file) {
        (Some(hex), None) => parse_hex32(hex),
        (None, Some(path)) => {
            let content = std::fs::read_to_string(path).expect("cannot read key file");
            parse_hex32(content.lines().next().expect("empty key file"))
        }
        _ => panic!("exactly one of --secret-key or --key-file is required"),
    };

    if let Some(min_version) = &args.sign_release_notice {
        let (major, release) = min_version
            .split_once('.')
            .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
            .expect("--sign-release-notice takes MAJOR.RELEASE, for example 2.58");
        let network = args.network.clone().expect("--network is required with --sign-release-notice");
        let issued_at = args.issued_at.unwrap_or_else(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock before 1970").as_secs()
        });
        let notice = kaspa_consensus_core::finality_anchor::release_notice::ReleaseNotice {
            network,
            min_major: major,
            min_release: release,
            issued_at,
        };
        let keypair = secp256k1::Keypair::from_seckey_slice(secp256k1::SECP256K1, &secret_key).expect("invalid secret key");
        let signature = notice.sign(&keypair);
        println!(
            "{{\"trustee\": {}, \"network\": \"{}\", \"min_version\": \"{}.{}\", \"issued_at\": {}, \"signature\": \"{}\"}}",
            trustee_index,
            notice.network,
            notice.min_major,
            notice.min_release,
            notice.issued_at,
            faster_hex::hex_string(&signature)
        );
        return;
    }

    let trustee_keys = if args.trustee_keys.is_empty() {
        None
    } else {
        assert_eq!(
            args.trustee_keys.len(),
            kaspa_consensus_core::finality_anchor::TRUSTEE_COUNT,
            "--trustee-keys requires all 5 keys"
        );
        let keys: Vec<[u8; 32]> = args.trustee_keys.iter().map(|k| parse_hex32(k)).collect();
        Some(keys.try_into().unwrap())
    };

    let config = SignerConfig {
        rpc_server: args.rpc_server,
        trustee_index,
        secret_key,
        listen_address: args.listen,
        peer_addresses: args.peers,
        depth: args.depth,
        interval: args.interval,
        poll_millis: args.poll_millis,
        trustee_keys,
        state_file: args.state_file,
    };

    info!("Marigold trustee signer starting (trustee index {})", config.trustee_index);
    let signer = TrusteeSigner::new(config).await.expect("signer initialization failed");
    signer.run().await;
}
