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
            let result = kaspa_cli(TerminalOptions::new().with_prompt("› "), None).await;
            if let Err(err) = result {
                println!("{err}");
            }
        }
    }
}
