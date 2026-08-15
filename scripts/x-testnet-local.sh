#!/usr/bin/env bash
# P4.1 — Testnet-in-a-box: launches 3 local devnet nodes on one machine, peered together.
#
# Usage:
#   ./scripts/x-testnet-local.sh [data-dir]
#
# data-dir defaults to ./x-testnet-local-data (repo-relative). Re-running the script reuses
# an existing data-dir (a stopped-and-restarted testnet resumes where it left off); delete
# the directory for a clean start.
#
# Stop all three nodes with: pkill -x kaspad

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DATA_DIR="${1:-$REPO_ROOT/x-testnet-local-data}"
KASPAD="$REPO_ROOT/target/release/kaspad"

mkdir -p "$DATA_DIR"

if [ ! -x "$KASPAD" ]; then
  echo "kaspad release binary not found — building it now (this can take a few minutes)..."
  (cd "$REPO_ROOT" && cargo build --release --bin kaspad)
fi

echo "kaspad: $KASPAD"
echo "Data dir: $DATA_DIR"
echo

# Node 1 — default devnet ports (gRPC 26610, borsh-wRPC 27610, JSON-wRPC 28610, P2P 26611)
echo "Starting node 1 (defaults: gRPC :26610, P2P :26611)..."
nohup "$KASPAD" --devnet --enable-unsynced-mining --utxoindex \
  --appdir="$DATA_DIR/node1" \
  > "$DATA_DIR/node1.log" 2>&1 &
NODE1_PID=$!
sleep 2

# Node 2 — shifted ports, peered to node 1
echo "Starting node 2 (gRPC :26620, P2P :26621, peered to node 1)..."
nohup "$KASPAD" --devnet --enable-unsynced-mining --utxoindex \
  --appdir="$DATA_DIR/node2" \
  --listen=127.0.0.1:26621 \
  --rpclisten=127.0.0.1:26620 \
  --rpclisten-borsh=127.0.0.1:27620 \
  --rpclisten-json=127.0.0.1:28620 \
  --addpeer=127.0.0.1:26611 \
  > "$DATA_DIR/node2.log" 2>&1 &
NODE2_PID=$!

# Node 3 — shifted ports again, peered to node 1
echo "Starting node 3 (gRPC :26630, P2P :26631, peered to node 1)..."
nohup "$KASPAD" --devnet --enable-unsynced-mining --utxoindex \
  --appdir="$DATA_DIR/node3" \
  --listen=127.0.0.1:26631 \
  --rpclisten=127.0.0.1:26630 \
  --rpclisten-borsh=127.0.0.1:27630 \
  --rpclisten-json=127.0.0.1:28630 \
  --addpeer=127.0.0.1:26611 \
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

Testnet is up. PIDs: node1=$NODE1_PID node2=$NODE2_PID node3=$NODE3_PID

RPC endpoints (gRPC):
  node1: 127.0.0.1:26610
  node2: 127.0.0.1:26620
  node3: 127.0.0.1:26630

Logs: $DATA_DIR/node{1,2,3}.log

To mine (uses the community kaspa-miner tool, or rothschild to get a funded devnet address):
  1. Get a devnet address + private key:
       $REPO_ROOT/target/release/rothschild --network devnet
     (prints a generated keypair/address; it will wait for the node to be reachable)
  2. Mine to it:
       kaspa-miner --mining-address <devnet-address> --kaspad-address 127.0.0.1 --port 26610 \\
         --threads 2 --mine-when-not-synced

Blocks mined against node1 should appear on node2 and node3 within a few seconds (check their
logs for "via relay").

To stop all three nodes: pkill -x kaspad
EOF
