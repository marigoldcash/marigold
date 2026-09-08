use crate::imports::*;

pub mod account;
pub mod address;
pub mod advanced;
pub mod auto;
pub mod balance;
pub mod broadcast;
pub mod close;
pub mod connect;
#[path = "create-unsigned-tx.rs"]
pub mod create_unsigned_tx;
pub mod details;
pub mod disconnect;
pub mod estimate;
pub mod exchange;
pub mod exit;
pub mod export;
pub mod guide;
pub mod halt;
pub mod help;
pub mod history;
pub mod list;
pub mod message;
pub mod miner;
pub mod monitor;
pub mod mute;
pub mod network;
// The upstream `node` module drives a kaspad child process through the NW.js
// daemon runtime; it is inert here (its verb() returns None without a Daemons
// handle) and the embedded node takes the name.
#[cfg(not(feature = "embedded-node"))]
pub mod node;
#[cfg(feature = "embedded-node")]
#[path = "embnode.rs"]
pub mod node;
pub mod note;
pub mod open;
pub mod ping;
pub mod pskb;
pub mod quit;
pub mod reload;
pub mod rpc;
pub mod select;
pub mod server;
pub mod settings;
pub mod sign;
pub mod start;
pub mod stop;
pub mod sweep;
// pub mod test;
pub mod theme;
pub mod track;
pub mod transfer;
pub mod utxos;
pub mod wallet;

// this module is registered manually within
// applications that support metrics
pub mod metrics;

// TODO
// broadcast
// create-unsigned-tx
// sign

pub fn register_handlers(cli: &Arc<KaspaCli>) -> Result<()> {
    register_handlers!(
        cli,
        cli.handlers(),
        [
            account, address, advanced, auto, balance, close, connect, details, disconnect, estimate, exchange, exit, export, guide, help, history, rpc, list,
            miner, message, monitor, mute, network, node, note, open, ping, pskb, quit, reload, select, server, settings, sweep,
            track, transfer, utxos, wallet,
            // halt,
            // theme,  start, stop
        ]
    );

    // Registered separately so the everyday command list is identical whether
    // or not the node is compiled in: a build without the feature simply has no
    // `node` verb, rather than one that reports itself unavailable.

    Ok(())
}
