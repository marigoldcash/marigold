//! The hook a node's RPC service uses to reach a miner running in the same
//! process (FORK-PLAN P8.3c). marigoldd has none; the wallet's background
//! miner registers one so a wallet on the same machine can read and steer it.

use crate::{RpcMinerStatus, RpcResult};

pub trait MinerControl: Send + Sync {
    fn status(&self) -> RpcMinerStatus;
    /// `mining: true` starts (or resizes to `percent`), `false` stops.
    fn control(&self, mining: bool, percent: Option<u32>) -> RpcResult<RpcMinerStatus>;
}
