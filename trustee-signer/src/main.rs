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
    #[arg(short, long, required_unless_present = "generate_key")]
    trustee_index: Option<u8>,

    /// Make a fresh trustee key: write the secret to this file (owner-only), print
    /// the public key to pin in params, and exit. Refuses to overwrite.
    #[arg(long, value_name = "PATH", exclusive = true)]
    generate_key: Option<PathBuf>,

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
    let trustee_index = args.trustee_index.expect("--trustee-index is required");

    let secret_key = match (&args.secret_key, &args.key_file) {
        (Some(hex), None) => parse_hex32(hex),
        (None, Some(path)) => {
            let content = std::fs::read_to_string(path).expect("cannot read key file");
            parse_hex32(content.lines().next().expect("empty key file"))
        }
        _ => panic!("exactly one of --secret-key or --key-file is required"),
    };

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
