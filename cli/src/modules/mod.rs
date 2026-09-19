use crate::imports::*;

pub mod about;
pub mod account;
pub mod address;
pub mod advanced;
pub mod auto;
pub mod balance;
pub mod broadcast;
pub mod close;
pub mod connect;
pub mod create;
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
pub mod backup;
pub mod import;
#[cfg(feature = "embedded-node")]
pub mod mine;
pub mod mint;
pub mod mobile;
pub mod mv;
#[cfg(not(feature = "embedded-node"))]
pub mod node;
#[cfg(feature = "embedded-node")]
#[path = "embnode.rs"]
pub mod node;
pub mod note;
pub mod open;
pub mod otp;
pub mod pay;
pub mod ping;
pub mod pskb;
pub mod quit;
pub mod receive;
pub mod reload;
pub mod request;
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
            about, account, address, advanced, auto, backup, balance, close, connect, create, details, disconnect, estimate, exchange,
            exit, export, guide, help, history, import, rpc, list, mint, mobile, mv, pay, receive, request, miner, message, monitor,
            mute, network, node, note, open, otp, ping, pskb, quit, reload, select, server, settings, sweep, track, transfer, utxos,
            wallet,
            // halt,
            // theme,  start, stop
        ]
    );

    // Registered separately so the everyday command list is identical whether
    // or not the node is compiled in: a build without the feature simply has no
    // `node` verb, rather than one that reports itself unavailable.
    //
    // The same for `mine`. It hashes in this process against this process's
    // node, so without the node compiled in there is nothing for it to mine
    // against. (The older `miner` verb is a different thing entirely — it
    // configures an external kaspa-miner binary — and stays under 'advanced'.)
    #[cfg(feature = "embedded-node")]
    register_handlers!(cli, cli.handlers(), [mine]);

    Ok(())
}
