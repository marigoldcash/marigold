# P4.1 -- Testnet-in-a-box: launches 3 local nodes on one machine, peered together.
#
# Usage:
#   .\scripts\x-testnet-local.ps1 [-DataDir <path>] [-Network devnet|simnet]
#
# -Network selects the network shape (default: devnet). P7.7 found that
# pool_activation is ForkActivation::never() on devnet specifically (mainnet/
# testnet/simnet are all always()) -- devnet cannot run a single `note` command.
# Use -Network simnet for anything touching the note-pool wallet (P7.8's SMOKE.md
# extension, WALLET.md's own walkthrough): it's also the only shape with
# skip_proof_of_work=true, so mined blocks confirm instantly, no real miner needed.
#
# DataDir defaults to .\x-testnet-local-data (repo-relative). Re-running the script reuses
# an existing data dir (a stopped-and-restarted testnet resumes where it left off); delete
# the directory for a clean start. Switching -Network against an existing data dir will
# fail (each node's datadir is bound to the network it was created under) -- delete the
# directory first when changing -Network.
#
# Every node gets -rpclisten-borsh (P7.7 finding: wRPC Borsh, which kaspa-cli connects
# over, is NOT started by default -- unlike gRPC/P2P) and -unsaferpc (this script is
# loopback-only local test infrastructure; a real public node must NOT pass -unsaferpc --
# see the P9 launch runbook).
#
# Stop all three nodes with: Get-Process marigoldd | Stop-Process

param(
    [string]$DataDir,
    [ValidateSet("devnet", "simnet")]
    [string]$Network = "devnet"
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $DataDir) { $DataDir = Join-Path $RepoRoot "x-testnet-local-data" }
$Kaspad = Join-Path $RepoRoot "target\release\marigoldd.exe"

if ($Network -eq "simnet") {
    $GrpcBase = 26510; $P2pBase = 26511; $BorshBase = 27510; $JsonBase = 28510
} else {
    $GrpcBase = 26610; $P2pBase = 26611; $BorshBase = 27610; $JsonBase = 28610
}

New-Item -ItemType Directory -Force -Path $DataDir | Out-Null

if (-not (Test-Path $Kaspad)) {
    Write-Output "marigoldd release binary not found -- building it now (this can take a few minutes)..."
    Push-Location $RepoRoot
    cargo build --release --bin marigoldd
    Pop-Location
}

Write-Output "marigoldd: $Kaspad"
Write-Output "Network: $Network"
Write-Output "Data dir: $DataDir"
Write-Output ""

# Node 1 -- network defaults (gRPC $GrpcBase, borsh-wRPC $BorshBase, JSON-wRPC $JsonBase, P2P $P2pBase)
Write-Output "Starting node 1 (gRPC :$GrpcBase, P2P :$P2pBase, borsh-wRPC :$BorshBase)..."
$node1 = Start-Process -FilePath $Kaspad -ArgumentList @(
    "--$Network", "--enable-unsynced-mining", "--unsaferpc", "--utxoindex",
    "--appdir=$DataDir\node1",
    "--rpclisten-borsh=127.0.0.1:$BorshBase"
) -RedirectStandardOutput "$DataDir\node1.log" -RedirectStandardError "$DataDir\node1.err.log" -PassThru -NoNewWindow
Start-Sleep -Seconds 2

# Node 2 -- shifted ports (+10), peered to node 1
$Node2Grpc = $GrpcBase + 10; $Node2P2p = $P2pBase + 10; $Node2Borsh = $BorshBase + 10; $Node2Json = $JsonBase + 10
Write-Output "Starting node 2 (gRPC :$Node2Grpc, P2P :$Node2P2p, peered to node 1)..."
$node2 = Start-Process -FilePath $Kaspad -ArgumentList @(
    "--$Network", "--enable-unsynced-mining", "--unsaferpc", "--utxoindex",
    "--appdir=$DataDir\node2",
    "--listen=127.0.0.1:$Node2P2p",
    "--rpclisten=127.0.0.1:$Node2Grpc",
    "--rpclisten-borsh=127.0.0.1:$Node2Borsh",
    "--rpclisten-json=127.0.0.1:$Node2Json",
    "--addpeer=127.0.0.1:$P2pBase"
) -RedirectStandardOutput "$DataDir\node2.log" -RedirectStandardError "$DataDir\node2.err.log" -PassThru -NoNewWindow

# Node 3 -- shifted ports (+20), peered to node 1
$Node3Grpc = $GrpcBase + 20; $Node3P2p = $P2pBase + 20; $Node3Borsh = $BorshBase + 20; $Node3Json = $JsonBase + 20
Write-Output "Starting node 3 (gRPC :$Node3Grpc, P2P :$Node3P2p, peered to node 1)..."
$node3 = Start-Process -FilePath $Kaspad -ArgumentList @(
    "--$Network", "--enable-unsynced-mining", "--unsaferpc", "--utxoindex",
    "--appdir=$DataDir\node3",
    "--listen=127.0.0.1:$Node3P2p",
    "--rpclisten=127.0.0.1:$Node3Grpc",
    "--rpclisten-borsh=127.0.0.1:$Node3Borsh",
    "--rpclisten-json=127.0.0.1:$Node3Json",
    "--addpeer=127.0.0.1:$P2pBase"
) -RedirectStandardOutput "$DataDir\node3.log" -RedirectStandardError "$DataDir\node3.err.log" -PassThru -NoNewWindow

Write-Output ""
Write-Output "Waiting for peers to connect..."
Start-Sleep -Seconds 12

foreach ($i in 1..3) {
    $log = "$DataDir\node$i.log"
    if ((Test-Path $log) -and (Select-String -Path $log -Pattern "Connected to" -Quiet)) {
        Write-Output "  node${i}: peered"
    } else {
        Write-Output "  node${i}: no peer connection seen yet -- check $log"
    }
}

Write-Output ""
Write-Output "Testnet is up ($Network). PIDs: node1=$($node1.Id) node2=$($node2.Id) node3=$($node3.Id)"
Write-Output ""
Write-Output "RPC endpoints (gRPC):"
Write-Output "  node1: 127.0.0.1:$GrpcBase"
Write-Output "  node2: 127.0.0.1:$Node2Grpc"
Write-Output "  node3: 127.0.0.1:$Node3Grpc"
Write-Output ""
Write-Output "wRPC (Borsh) endpoints -- what kaspa-cli connects to ('server <host:port>' then 'connect'):"
Write-Output "  node1: 127.0.0.1:$BorshBase"
Write-Output "  node2: 127.0.0.1:$Node2Borsh"
Write-Output "  node3: 127.0.0.1:$Node3Borsh"
Write-Output ""
Write-Output "Logs: $DataDir\node{1,2,3}.log"
Write-Output ""
Write-Output "To mine (uses the community kaspa-miner tool, or rothschild to get a funded address):"
Write-Output "  1. Get an address + private key:"
Write-Output "       $RepoRoot\target\release\rothschild.exe --network $Network"
Write-Output "     (prints a generated keypair/address; it will wait for the node to be reachable)"
Write-Output "  2. Mine to it:"
Write-Output "       kaspa-miner --mining-address <address> --kaspad-address 127.0.0.1 --port $GrpcBase --threads 2 --mine-when-not-synced"
Write-Output ""
Write-Output "Blocks mined against node1 should appear on node2 and node3 within a few seconds (check their logs for `"via relay`")."
Write-Output ""
Write-Output "To stop all three nodes: Get-Process marigoldd | Stop-Process"
