//! The sealed bundle code moved to the wallet service crate (PH-M1b3), so the
//! phone apps use the same code; this module keeps its old path and the one
//! function that needs the terminal's context.

use crate::cli::KaspaCli;
use crate::result::Result;
use std::sync::Arc;

pub use kaspa_wallet_service::bundle::*;

/// The open wallet's files, for the terminal wallet.
pub async fn wallet_files(cli: &Arc<KaspaCli>) -> Result<WalletFiles> {
    let descriptor = cli.store().descriptor().ok_or_else(|| crate::error::Error::custom("no wallet is open"))?;
    Ok(WalletFiles::of_wallet(&cli.wallet(), &descriptor.filename).await?)
}
