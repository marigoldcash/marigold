cfg_if::cfg_if! {
    if #[cfg(target_arch = "wasm32")] {
        fn main() {}
    } else {
        use kaspa_cli_lib::{kaspa_cli, TerminalOptions};

        #[tokio::main]
        async fn main() {
            // Two flags that answer and leave, for scripts and for the first
            // run on a new platform: no terminal, no wallet.
            let args: Vec<String> = std::env::args().skip(1).collect();
            if args.iter().any(|a| a == "--platform") {
                print!("{}", kaspa_cli_lib::platform::report());
                return;
            }
            if args.iter().any(|a| a == "--version" || a == "-V") {
                println!("marigold-cli {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            // Whoever gets the binary should be able to start without docs:
            // the wallet, and the miner on its own, on one screen.
            if args.first().map(|a| a.as_str()).is_some_and(|a| a == "--help" || a == "-h" || a == "help") {
                println!("{}", kaspa_cli_lib::HELP.replace("{version}", env!("CARGO_PKG_VERSION")));
                return;
            }
            // The miner as a service: no terminal, no wallet, stays in the
            // foreground for systemd or Docker to look after.
            if args.first().map(|a| a.as_str()) == Some("mine-to") {
                #[cfg(feature = "embedded-node")]
                {
                    if let Err(err) = kaspa_cli_lib::headless::mine_to(args[1..].to_vec()).await {
                        eprintln!("{err}");
                        std::process::exit(1);
                    }
                    return;
                }
                #[cfg(not(feature = "embedded-node"))]
                {
                    eprintln!("This build has no node in it, so it cannot mine. Use a release build.");
                    std::process::exit(1);
                }
            }
            let result = kaspa_cli(TerminalOptions::new().with_prompt("› "), None).await;
            if let Err(err) = result {
                println!("{err}");
            }
        }
    }
}
