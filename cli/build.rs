//! Say which version just got built.
//!
//! There is no other honest answer to "is this binary current?". We shipped a
//! Docker image and then could not tell, without hashing the binary, whether
//! it predated the commit we cared about. A version that is bumped every
//! commit fixes that only if you can see it, and the moment you want to see it
//! is the moment the build finishes.
//!
//! `cargo::warning` is the only channel a build script has to the terminal.
//! Cargo prints it as a warning, which it is not — but a line that says what
//! was built beats a clean build you cannot identify.

use std::process::Command;

fn main() {
    // Rerun every time. Without this the announcement appears only when
    // something else already forced the script to re-run, which is exactly the
    // build you do not need to be told about. The marker deliberately does not
    // exist: a rerun-if-changed on a missing path always retriggers.
    println!("cargo::rerun-if-changed=.build-version-marker");

    let version = env!("CARGO_PKG_VERSION");
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|hash| !hash.is_empty());

    // Dirty means the binary matches no commit at all, which is worth knowing
    // before you hand it to somebody.
    let dirty = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| !out.stdout.is_empty())
        .unwrap_or(false);

    // A build from a release tarball or a Docker context has no git at all;
    // the version alone is still the answer to the question being asked.
    let built = match (commit, dirty) {
        (Some(hash), true) => format!("Marigold v{version} ({hash}, uncommitted changes)"),
        (Some(hash), false) => format!("Marigold v{version} ({hash})"),
        (None, _) => format!("Marigold v{version}"),
    };
    println!("cargo::warning={built}");
}
