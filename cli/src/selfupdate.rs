//! The wallet updates itself: `update` fetches the release document, checks the
//! trustees' signatures over the newest release's file digests, downloads this
//! platform's file from the compiled-in release address, checks it against the
//! signed digest, puts it in place of the running program and starts again
//! (founder, 2026-10-08: "people just click OK on 'there is a new version' and it
//! installs").
//!
//! What keeps this honest: the download address is compiled in, so no document can
//! send the wallet anywhere; a file is installed only when its digest is one a
//! quorum of trustees signed; the wallet is closed first the way 'close' does it;
//! the program it replaces stays beside it as `.old`.

use crate::cli::KaspaCli;
use crate::imports::*;
use crate::release_check::{self, DOWNLOAD_URL, Version};
use std::path::{Path, PathBuf};

/// Where a release's files are: `<base>/v<version>/<asset>`. Compiled in on purpose.
pub const RELEASE_DOWNLOAD_BASE: &str = "https://github.com/marigoldcash/marigold-wallet/releases/download";

/// The release file for this build's platform, named as the release carries it.
pub fn asset_name() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "marigold-cli-linux-x86_64",
        ("linux", "aarch64") => "marigold-cli-linux-arm64",
        ("macos", "aarch64") => "marigold-cli-macos-arm64",
        ("macos", "x86_64") => "marigold-cli-macos-x86_64",
        ("windows", "x86_64") => "marigold-cli-windows-x86_64.exe",
        _ => return None,
    })
}

/// Inside a container the image is the unit of update, not the file.
pub fn in_docker() -> bool {
    Path::new("/.dockerenv").exists()
}

/// Whether this build can update itself at all.
pub fn available() -> bool {
    asset_name().is_some() && !in_docker()
}

/// What a verified manifest offers this build.
pub struct Offer {
    pub version: Version,
    pub asset: &'static str,
    pub digest: [u8; 32],
}

/// The offer in a document, or `None` when this build is current. An error says why
/// nothing can be offered: no manifest, no signatures that verify, no file for this
/// platform. `MARIGOLD_UPDATE_FORCE=1` takes the signed release even when it is not
/// newer — for testing the install path against a real release.
pub fn offer(document: &release_check::Document, network: Option<NetworkId>) -> Result<Option<Offer>> {
    let asset = asset_name().ok_or_else(|| Error::custom("there is no release file for this platform; download by hand"))?;
    let entry = document.release.as_ref().ok_or_else(|| Error::custom("the release document carries no signed manifest yet"))?;
    let manifest = release_check::verified_manifest(document, network).ok_or_else(|| {
        Error::custom("the release manifest does not verify against the trustees' keys of this network — nothing is installed")
    })?;
    let version = Version::parse(&entry.version).ok_or_else(|| Error::custom("the manifest's version is malformed"))?;
    let force = std::env::var("MARIGOLD_UPDATE_FORCE").is_ok_and(|v| !v.is_empty() && v != "0");
    if version <= Version::own() && !force {
        return Ok(None);
    }
    let digest = manifest.digest_of(asset).ok_or_else(|| Error::custom(format!("the signed manifest has no entry for {asset}")))?;
    Ok(Some(Offer { version, asset, digest }))
}

/// Where the new file is written: beside the running program, so the final
/// move is a rename on one filesystem.
fn staged_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().map_err(|e| Error::custom(format!("cannot tell where this program is: {e}")))?;
    let name = exe.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "marigold-cli".into());
    Ok(exe.with_file_name(format!("{name}.update")))
}

/// Downloads the offered file and checks its digest. Returns where it was put.
pub async fn download(offer: &Offer, say: &(dyn Fn(String) + Send + Sync)) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    let url = format!("{RELEASE_DOWNLOAD_BASE}/v{}/{}", offer.version, offer.asset);
    let staged = staged_path()?;
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .user_agent(format!("marigold-cli/{}", Version::own()))
        .build()
        .map_err(|e| Error::custom(e.to_string()))?;
    let mut response = client.get(&url).send().await.map_err(|e| Error::custom(format!("cannot reach the release: {e}")))?;
    if !response.status().is_success() {
        return Err(Error::custom(format!("{url} answered {}", response.status())));
    }
    let total = response.content_length();
    let mut file = {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o755);
        }
        options.open(&staged).map_err(|e| Error::custom(format!("cannot write next to this program ({}): {e}", staged.display())))?
    };
    let mut hasher = Sha256::new();
    let mut got: u64 = 0;
    let mut last_said: u64 = 0;
    while let Some(chunk) = response.chunk().await.map_err(|e| Error::custom(format!("the download broke off: {e}")))? {
        file.write_all(&chunk).map_err(|e| Error::custom(format!("cannot write {}: {e}", staged.display())))?;
        hasher.update(&chunk);
        got += chunk.len() as u64;
        if got - last_said >= 8 * 1024 * 1024 {
            last_said = got;
            match total {
                Some(total) if total > 0 => say(format!(
                    "{} of {} ({}%)",
                    crate::backup::human_size(got as usize),
                    crate::backup::human_size(total as usize),
                    got * 100 / total
                )),
                _ => say(format!("{} so far", crate::backup::human_size(got as usize))),
            }
        }
    }
    file.sync_all().ok();
    drop(file);
    let digest: [u8; 32] = hasher.finalize().into();
    if digest != offer.digest {
        let _ = std::fs::remove_file(&staged);
        return Err(Error::custom(
            "the downloaded file does not match what the trustees signed — it was deleted, nothing was installed",
        ));
    }
    Ok(staged)
}

/// Puts the staged file in place of the running program. The old one stays
/// beside it as `.old`.
pub fn install(staged: &Path) -> Result<PathBuf> {
    let exe = std::env::current_exe().map_err(|e| Error::custom(format!("cannot tell where this program is: {e}")))?;
    let old = exe.with_extension(if cfg!(windows) { "old.exe" } else { "old" });
    let _ = std::fs::remove_file(&old);
    // A running program can be renamed on every platform, and overwritten on
    // none that matters here: move the old aside, the new in.
    std::fs::rename(&exe, &old).map_err(|e| Error::custom(format!("cannot move {} aside: {e}", exe.display())))?;
    if let Err(e) = std::fs::rename(staged, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(Error::custom(format!("cannot put the new program at {}: {e}", exe.display())));
    }
    Ok(exe)
}

/// Starts the program at `exe` with this process's arguments and leaves.
pub fn restart(exe: &Path) -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(exe).args(&args).exec();
        Err(Error::custom(format!("could not start {}: {err}", exe.display())))
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new(exe)
            .args(&args)
            .spawn()
            .map_err(|e| Error::custom(format!("could not start {}: {e}", exe.display())))?;
        std::process::exit(0);
    }
}

/// The `update` command.
pub async fn command(cli: &Arc<KaspaCli>) -> Result<()> {
    if in_docker() {
        tprintln!(
            cli,
            "This wallet runs in Docker: 'docker pull' brings the image up to date, and the next start is the new version."
        );
        return Ok(());
    }
    if asset_name().is_none() {
        tprintln!(cli, "There is no release file for this platform; download the wallet by hand at {DOWNLOAD_URL}.");
        return Ok(());
    }
    tprintln!(cli, "Checking for a newer release…");
    let document = release_check::fetch().await.map_err(|e| Error::custom(format!("cannot read the release document: {e}")))?;
    let network = release_check::network_of(cli);
    let Some(offer) = offer(&document, network)? else {
        tprintln!(cli, "This is the newest release, {}.", Version::own());
        return Ok(());
    };
    tprintln!(cli, "");
    tprintln!(cli, "Release {} is out; this is {}. Downloading {}…", offer.version, Version::own(), offer.asset);
    let cli_ = cli.clone();
    let say = move |line: String| tprintln!(cli_, "  {line}");
    let staged = download(&offer, &say).await?;
    tprintln!(cli, "{}", style("Downloaded and checked: the file matches what the trustees signed.").green());
    tprintln!(cli, "");
    if cli.wallet().is_open() {
        tprintln!(
            cli,
            "{}",
            crate::ui::dim("Installing closes the wallet first (its backup goes out as usual) and restarts the program.")
        );
    } else {
        tprintln!(cli, "{}", crate::ui::dim("Installing restarts the program."));
    }
    let answer = cli.term().ask(false, &format!("Install {} and restart now? [Y/n]: ", offer.version)).await?.trim().to_lowercase();
    if answer.starts_with('n') {
        let _ = std::fs::remove_file(&staged);
        tprintln!(cli, "Not now. 'update' downloads it again when you are ready.");
        return Ok(());
    }
    if cli.wallet().is_open() {
        cli.exec_within("close").await?;
    }
    #[cfg(feature = "embedded-node")]
    if cli.embedded_node_running() {
        cli.stop_embedded_node().await?;
    }
    cli.stop_telegram_bot();
    let exe = install(&staged)?;
    tprintln!(cli, "{}", style(format!("Installed {}. Restarting…", offer.version)).green());
    tprintln!(
        cli,
        "{}",
        crate::ui::dim(format!(
            "The previous version stays beside it as {}.",
            exe.with_extension(if cfg!(windows) { "old.exe" } else { "old" }).display()
        ))
    );
    restart(&exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_build_has_a_release_file_name() {
        // Every platform the release builds for maps; a build on another would
        // simply offer nothing.
        if cfg!(any(target_os = "linux", target_os = "macos", target_os = "windows"))
            && cfg!(any(target_arch = "x86_64", target_arch = "aarch64"))
        {
            assert!(asset_name().is_some());
        }
    }
}
