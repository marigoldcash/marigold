//! What the service takes from the program that hosts it, so that it never
//! links the node or the CPU miner itself: the terminal wallet and the desktop
//! app hand them in, the phone apps hand in neither. (PH-M1c.)

use crate::result::Result;
use async_trait::async_trait;
use kaspa_consensus_core::network::NetworkId;
use kaspa_rpc_core::RpcMinerStatus;
use kaspa_rpc_core::RpcResult;
use kaspa_rpc_core::api::miner::MinerControl;
use kaspa_wallet_core::prelude::Address;
use kaspa_wallet_core::rpc::{DynRpcApi, Rpc};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// The CPU miner a host program runs for the service: told where to pay and
/// which node to ask, started and stopped by share of the machine.
pub trait MinerHostApi: MinerControl {
    fn bind_address(&self, address: Address);
    fn bind(&self, rpc: Arc<DynRpcApi>);
    fn address_bound(&self) -> bool;
    fn start(&self, percent: u32) -> RpcResult<RpcMinerStatus>;
    fn stop(&self) -> RpcMinerStatus;
    /// The share of the machine it mines at, or `None` when it is not mining.
    fn mining(&self) -> Option<u32>;
}

/// Makes a miner host; the flag is the program's shutdown, which stops it.
pub type MinerFactory = Arc<dyn Fn(Arc<AtomicBool>) -> Arc<dyn MinerHostApi> + Send + Sync>;

/// A node running inside the host program's process.
#[async_trait]
pub trait NodeInside: Send + Sync {
    /// The wallet's connection to the node is up.
    async fn signal_connected(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}

/// Starts a node inside the process, with the miner (if any) in it, and
/// returns it with the wallet's handle on its RPC.
#[async_trait]
pub trait NodeStarter: Send + Sync {
    async fn start(&self, network_id: NetworkId, miner: Option<Arc<dyn MinerControl>>) -> Result<(Arc<dyn NodeInside>, Rpc)>;
}

/// What the host offers. Both empty is the phone's case.
#[derive(Clone, Default)]
pub struct Providers {
    pub miner: Option<MinerFactory>,
    pub node: Option<Arc<dyn NodeStarter>>,
}
