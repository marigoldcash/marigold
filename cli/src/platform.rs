//! What this binary knows about the machine it is on, for the first run on a
//! platform nobody has run it on before. `marigold-cli --platform` prints it
//! and exits; the release workflow runs it on every runner it builds on, so a
//! guarded-out piece (disk-space check, mining priority, file permissions)
//! is found on the build log rather than by a tester.

use std::fmt::Write;

fn env_or(name: &str, missing: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| missing.to_string())
}

fn home() -> String {
    std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| "(no HOME or USERPROFILE)".to_string())
}

pub fn report() -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Marigold wallet {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        out,
        "target:        os={} arch={} family={} env={} pointer={}-bit endian={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::consts::FAMILY,
        if cfg!(target_env = "msvc") {
            "msvc"
        } else if cfg!(target_env = "gnu") {
            "gnu"
        } else if cfg!(target_env = "musl") {
            "musl"
        } else {
            "-"
        },
        std::mem::size_of::<usize>() * 8,
        if cfg!(target_endian = "little") { "little" } else { "big" }
    );
    let _ = writeln!(out, "cfg:           unix={} windows={}", cfg!(unix), cfg!(windows));
    let _ = writeln!(out, "network sync:  {}", if cfg!(feature = "embedded-node") { "compiled in" } else { "NOT compiled in — 'connect' can only reach a public computer" });
    let _ = writeln!(
        out,
        "disk space:    {}",
        if cfg!(unix) { "measured before a sync, mining, or a sweep" } else { "NOT measured on this platform — the wallet assumes there is room" }
    );
    let _ = writeln!(
        out,
        "mining:        {}",
        if cfg!(target_os = "linux") {
            "threads run at idle priority (SCHED_IDLE)"
        } else if cfg!(target_os = "macos") {
            "threads run in the background quality-of-service class"
        } else if cfg!(windows) {
            "threads run at idle priority (THREAD_PRIORITY_IDLE)"
        } else {
            "threads are niced (setpriority 19) — the whole process, on this platform"
        }
    );
    let _ = writeln!(
        out,
        "backup files:  {}",
        if cfg!(unix) { "written owner-only (0600)" } else { "written with the platform's default permissions" }
    );
    let _ = writeln!(out, "home:          {}", home());
    let _ = writeln!(out, "wallet folder: {}", kaspa_wallet_core::storage::local::default_storage_folder().replace('~', &home()));
    #[cfg(feature = "embedded-node")]
    {
        let network = kaspa_consensus_core::network::NetworkId::with_suffix(kaspa_consensus_core::network::NetworkType::Testnet, 10);
        match crate::embedded::default_appdir(network) {
            Ok(dir) => {
                let _ = writeln!(out, "sync data:     {}", dir.display());
            }
            Err(err) => {
                let _ = writeln!(out, "sync data:     could not be resolved: {err}");
            }
        }
    }
    let _ = writeln!(out, "colour:        {:?} (TERM={} COLORTERM={} NO_COLOR={})", crate::ui::depth(), env_or("TERM", "-"), env_or("COLORTERM", "-"), env_or("NO_COLOR", "-"));
    out
}
