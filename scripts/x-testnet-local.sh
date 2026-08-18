#!/usr/bin/env bash
# P4.1 — Testnet-in-a-box: launches 3 local nodes on one machine, peered together.
#
# Usage:
#   ./scripts/x-testnet-local.sh [data-dir]
#   NETWORK=simnet ./scripts/x-testnet-local.sh [data-dir]
#
# NETWORK selects the network shape (default: devnet). P7.7 found that
# pool_activation is ForkActivation::never() on devnet specifically (mainnet/
# testnet/simnet are all always()) — devnet cannot run a single `note` command.
# Use NETWORK=simnet for anything touching the note-pool wallet (P7.8's SMOKE.md
# extension, WALLET.md's own walkthrough): it's also the only shape with
# skip_proof_of_work=true, so mined blocks confirm instantly, no real miner needed.
#
# data-dir defaults to ./x-testnet-local-data (repo-relative). Re-running the script
# reuses an existing data-dir (a stopped-and-restarted testnet resumes where it left
# off); delete the directory for a clean start. Note that switching NETWORK against an
# existing data-dir will fail (each node's datadir is bound to the network it was
# created under) — delete the directory first when changing NETWORK.
#
# Every node gets --rpclisten-borsh (P7.7 finding: wRPC Borsh, which kaspa-cli
# connects over, is NOT started by default — unlike gRPC/P2P) and --unsaferpc (this
# script is loopback-only local test infrastructure; a real public node must NOT
# pass --unsaferpc — see the P9 launch runbook).
#
# Stop all three nodes with: pkill -x kaspad

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DATA_DIR="${1:-$REPO_ROOT/x-testnet-local-data}"
KASPAD="$REPO_ROOT/target/release/kaspad"
NETWORK="${NETWORK:-devnet}"

case "$NETWORK" in
  devnet)
    GRPC_BASE=26610; P2P_BASE=26611; BORSH_BASE=27610; JSON_BASE=28610 ;;
  simnet)
    GRPC_BASE=26510; P2P_BASE=26511; BORSH_BASE=27510; JSON_BASE=28510 ;;
  *)
    echo "unsupported NETWORK: $NETWORK (expected devnet or simnet)" >&2
    exit 1 ;;
esac

mkdir -p "$DATA_DIR"

if [ ! -x "$KASPAD" ]; then
  echo "kaspad release binary not found — building it now (this can take a few minutes)..."
  (cd "$REPO_ROOT" && cargo build --release --bin kaspad)
fi

echo "kaspad: $KASPAD"
echo "Network: $NETWORK"
echo "Data dir: $DATA_DIR"
echo

# Node 1 — network defaults (gRPC $GRPC_BASE, borsh-wRPC $BORSH_BASE, JSON-wRPC $JSON_BASE, P2P $P2P_BASE)
echo "Starting node 1 (gRPC :$GRPC_BASE, P2P :$P2P_BASE, borsh-wRPC :$BORSH_BASE)..."
nohup "$KASPAD" --$NETWORK --enable-unsynced-mining --unsaferpc --utxoindex \
  --appdir="$DATA_DIR/node1" \
  --rpclisten-borsh=127.0.0.1:$BORSH_BASE \
  > "$DATA_DIR/node1.log" 2>&1 &
NODE1_PID=$!
sleep 2

# Node 2 — shifted ports (+10), peered to node 1
NODE2_GRPC=$((GRPC_BASE + 10)); NODE2_P2P=$((P2P_BASE + 10)); NODE2_BORSH=$((BORSH_BASE + 10)); NODE2_JSON=$((JSON_BASE + 10))
echo "Starting node 2 (gRPC :$NODE2_GRPC, P2P :$NODE2_P2P, peered to node 1)..."
nohup "$KASPAD" --$NETWORK --enable-unsynced-mining --unsaferpc --utxoindex \
  --appdir="$DATA_DIR/node2" \
  --listen=127.0.0.1:$NODE2_P2P \
  --rpclisten=127.0.0.1:$NODE2_GRPC \
  --rpclisten-borsh=127.0.0.1:$NODE2_BORSH \
  --rpclisten-json=127.0.0.1:$NODE2_JSON \
  --addpeer=127.0.0.1:$P2P_BASE \
  > "$DATA_DIR/node2.log" 2>&1 &
NODE2_PID=$!

# Node 3 — shifted ports (+20), peered to node 1
NODE3_GRPC=$((GRPC_BASE + 20)); NODE3_P2P=$((P2P_BASE + 20)); NODE3_BORSH=$((BORSH_BASE + 20)); NODE3_JSON=$((JSON_BASE + 20))
echo "Starting node 3 (gRPC :$NODE3_GRPC, P2P :$NODE3_P2P, peered to node 1)..."
nohup "$KASPAD" --$NETWORK --enable-unsynced-mining --unsaferpc --utxoindex \
  --appdir="$DATA_DIR/node3" \
  --listen=127.0.0.1:$NODE3_P2P \
  --rpclisten=127.0.0.1:$NODE3_GRPC \
  --rpclisten-borsh=127.0.0.1:$NODE3_BORSH \
  --rpclisten-json=127.0.0.1:$NODE3_JSON \
  --addpeer=127.0.0.1:$P2P_BASE \
  > "$DATA_DIR/node3.log" 2>&1 &
NODE3_PID=$!

echo
echo "Waiting for peers to connect..."
sleep 12

for i in 1 2 3; do
  if grep -q "Connected to\|Connected to incoming\|Connected to outgoing" "$DATA_DIR/node$i.log" 2>/dev/null; then
    echo "  node$i: peered"
  else
    echo "  node$i: no peer connection seen yet — check $DATA_DIR/node$i.log"
  fi
done

cat <<EOF

Testnet is up ($NETWORK). PIDs: node1=$NODE1_PID node2=$NODE2_PID node3=$NODE3_PID

RPC endpoints (gRPC):
  node1: 127.0.0.1:$GRPC_BASE
  node2: 127.0.0.1:$NODE2_GRPC
  node3: 127.0.0.1:$NODE3_GRPC

wRPC (Borsh) endpoints — what kaspa-cli connects to ('server <host:port>' then 'connect'):
  node1: 127.0.0.1:$BORSH_BASE
  node2: 127.0.0.1:$NODE2_BORSH
  node3: 127.0.0.1:$NODE3_BORSH

Logs: $DATA_DIR/node{1,2,3}.log

To mine (uses the community kaspa-miner tool, or rothschild to get a funded address):
  1. Get an address + private key:
       $REPO_ROOT/target/release/rothschild --network $NETWORK
     (prints a generated keypair/address; it will wait for the node to be reachable)
  2. Mine to it:
       kaspa-miner --mining-address <address> --kaspad-address 127.0.0.1 --port $GRPC_BASE \\
         --threads 2 --mine-when-not-synced

Blocks mined against node1 should appear on node2 and node3 within a few seconds (check their
logs for "via relay").

To stop all three nodes: pkill -x kaspad
EOF
