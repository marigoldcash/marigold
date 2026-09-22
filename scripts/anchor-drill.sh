#!/usr/bin/env bash
#
# The anchor attack rehearsal on pve3 (PLAN P8.4a, 2026-09-22): the biggest miner
# builds a private, heavier chain and the network must refuse it while the
# trustees keep anchoring the honest one. Run as root on pve3, one phase at a time,
# and read `status` between phases. The production node (rpc1) is never touched:
# the attacker is a second node on this machine, synced from the production node
# over loopback and then cut off from every peer, and the CPU miner is moved to
# it for the duration.
#
#   scripts/anchor-drill.sh prepare     start the attacker node, synced from production
#   scripts/anchor-drill.sh isolate     restart it with no peers at all
#   scripts/anchor-drill.sh attack      move the CPU miner from production to it
#   scripts/anchor-drill.sh status      sink, DAA score and anchor state of both nodes
#   scripts/anchor-drill.sh reconnect   give it the production node as a peer again
#   scripts/anchor-drill.sh restore     miner back on production, attacker stopped
#   scripts/anchor-drill.sh purge       restore, then delete the attacker's data
#
# Expected: after `reconnect`, the production node logs "[FINALITY ANCHOR] Block …
# conflicts with the latest finality anchor" for the attacker's blocks and keeps its
# sink; the attacker node receives the anchors it missed and reorganises onto the
# honest chain, its private blocks orphaned. The fail-open bound is 1,500 DAA
# (about two and a half minutes), so `attack` should run for five minutes or more:
# the attacker's own node goes stale and stops enforcing anchors on itself, which
# is the case worth proving.
set -euo pipefail
BIN=/opt/marigold/bin
APPDIR=/var/lib/marigold/drill-attacker
UNIT=anchor-drill-node
MINER_UNIT=anchor-drill-miner
PROD_P2P=127.0.0.1:26211
PROD_WRPC=ws://127.0.0.1:27210
PROD_GRPC=127.0.0.1:26210
ATT_P2P=127.0.0.1:36211
ATT_GRPC=127.0.0.1:36210
ATT_WRPC_ADDR=127.0.0.1:37210
ATT_WRPC=ws://$ATT_WRPC_ADDR
NOWHERE=127.0.0.1:9
PAYOUT=$(grep -h ExecStart /etc/systemd/system/marigold-miner.service | grep -oE 'marigoldtest:[a-z0-9]+' | head -1)
SHARE=65

node_args() {
  # $1: the peer to connect to (and nothing else)
  echo --testnet --utxoindex --disable-upnp --nodnsseed --loglevel=info \
    --appdir=$APPDIR --listen=$ATT_P2P --rpclisten=$ATT_GRPC --rpclisten-borsh=$ATT_WRPC_ADDR \
    --connect="$1" --externalip=127.0.0.1
}

start_node() {
  systemctl stop $UNIT 2>/dev/null || true
  # shellcheck disable=SC2046
  systemd-run --unit=$UNIT --uid=marigold --gid=marigold --property=LimitNOFILE=65536 \
    --property=Restart=no --collect -- $BIN/marigoldd $(node_args "$1")
  sleep 3; systemctl is-active $UNIT >/dev/null || { echo "attacker node did not start"; journalctl -u $UNIT -n 20 --no-pager -o cat; exit 1; }
}

status_of() {
  # $1: label, $2: gRPC address. The rehearsal tool's status line reuses the
  # signer's client; a fetch failure prints as such rather than aborting.
  printf '%-12s ' "$1"
  $BIN/marigold-trustee-signer --rpc-server "$2" --drill-status 2>/dev/null || echo "(unreachable)"
}

case "${1:-}" in
  prepare)
    install -d -o marigold -g marigold -m 750 $APPDIR
    start_node $PROD_P2P
    echo "attacker node started at $ATT_WRPC, syncing from production over loopback; watch: journalctl -fu $UNIT"
    ;;
  isolate)
    start_node $NOWHERE
    echo "attacker node restarted with no peers"
    ;;
  attack)
    systemctl stop marigold-miner
    systemd-run --unit=$MINER_UNIT --uid=marigold --gid=marigold --collect -- \
      $BIN/marigold-cli mine-to "$PAYOUT" $SHARE --node $ATT_WRPC
    echo "miner moved to the attacker at $(date -u +%H:%M:%S) UTC; leave it five minutes or more"
    ;;
  status)
    status_of production $PROD_GRPC
    status_of attacker $ATT_GRPC
    journalctl -u marigoldd --since "-10min" --no-pager -o cat | grep -c "conflicts with the latest finality anchor" | sed 's/^/production: anchor-conflict refusals in the last 10 min: /'
    journalctl -u $UNIT --since "-10min" --no-pager -o cat | grep -cE "FINALITY ANCHOR|reorg|Pending" | sed 's/^/attacker: anchor lines in the last 10 min: /'
    ;;
  reconnect)
    start_node $PROD_P2P
    echo "attacker node reconnected to production at $(date -u +%H:%M:%S) UTC; run status in a minute"
    ;;
  restore)
    systemctl stop $MINER_UNIT 2>/dev/null || true
    systemctl start marigold-miner
    systemctl stop $UNIT 2>/dev/null || true
    echo "production miner running, attacker stopped"
    ;;
  purge)
    "$0" restore
    rm -rf $APPDIR
    echo "attacker data removed"
    ;;
  *) sed -n 2,30p "$0"; exit 2 ;;
esac
