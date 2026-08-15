# P4.1 -- Testnet-in-a-box: launches 3 local devnet nodes on one machine, peered together.
#
# Usage:
#   .\scripts\x-testnet-local.ps1 [-DataDir <path>]
#
# DataDir defaults to .\x-testnet-local-data (repo-relative). Re-running the script reuses
# an existing data dir (a stopped-and-restarted testnet resumes where it left off); delete
# the directory for a clean start.
#
# Stop all three nodes with: Get-Process kaspad | Stop-Process

param(
    [string]$DataDir
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $DataDir) { $DataDir = Join-Path $RepoRoot "x-testnet-local-data" }
$Kaspad = Join-Path $RepoRoot "target\release\kaspad.exe"

New-Item -ItemType Directory -Force -Path $DataDir | Out-Null

if (-not (Test-Path $Kaspad)) {
    Write-Output "kaspad release binary not found -- building it now (this can take a few minutes)..."
    Push-Location $RepoRoot
    cargo build --release --bin kaspad
    Pop-Location
}

Write-Output "kaspad: $Kaspad"
Write-Output "Data dir: $DataDir"
Write-Output ""

# Node 1 -- default devnet ports (gRPC 26610, borsh-wRPC 27610, JSON-wRPC 28610, P2P 26611)
Write-Output "Starting node 1 (defaults: gRPC :26610, P2P :26611)..."
$node1 = Start-Process -FilePath $Kaspad -ArgumentList @(
    "--devnet", "--enable-unsynced-mining", "--utxoindex",
    "--appdir=$DataDir\node1"
) -RedirectStandardOutput "$DataDir\node1.log" -RedirectStandardError "$DataDir\node1.err.log" -PassThru -NoNewWindow
Start-Sleep -Seconds 2

# Node 2 -- shifted ports, peered to node 1
Write-Output "Starting node 2 (gRPC :26620, P2P :26621, peered to node 1)..."
$node2 = Start-Process -FilePath $Kaspad -ArgumentList @(
    "--devnet", "--enable-unsynced-mining", "--utxoindex",
    "--appdir=$DataDir\node2",
    "--listen=127.0.0.1:26621",
    "--rpclisten=127.0.0.1:26620",
    "--rpclisten-borsh=127.0.0.1:27620",
    "--rpclisten-json=127.0.0.1:28620",
    "--addpeer=127.0.0.1:26611"
) -RedirectStandardOutput "$DataDir\node2.log" -RedirectStandardError "$DataDir\node2.err.log" -PassThru -NoNewWindow

# Node 3 -- shifted ports again, peered to node 1
Write-Output "Starting node 3 (gRPC :26630, P2P :26631, peered to node 1)..."
$node3 = Start-Process -FilePath $Kaspad -ArgumentList @(
    "--devnet", "--enable-unsynced-mining", "--utxoindex",
    "--appdir=$DataDir\node3",
    "--listen=127.0.0.1:26631",
    "--rpclisten=127.0.0.1:26630",
    "--rpclisten-borsh=127.0.0.1:27630",
    "--rpclisten-json=127.0.0.1:28630",
    "--addpeer=127.0.0.1:26611"
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
Write-Output "Testnet is up. PIDs: node1=$($node1.Id) node2=$($node2.Id) node3=$($node3.Id)"
Write-Output ""
Write-Output "RPC endpoints (gRPC):"
Write-Output "  node1: 127.0.0.1:26610"
Write-Output "  node2: 127.0.0.1:26620"
Write-Output "  node3: 127.0.0.1:26630"
Write-Output ""
Write-Output "Logs: $DataDir\node{1,2,3}.log"
Write-Output ""
Write-Output "To mine (uses the community kaspa-miner tool, or rothschild to get a funded devnet address):"
Write-Output "  1. Get a devnet address + private key:"
Write-Output "       $RepoRoot\target\release\rothschild.exe --network devnet"
Write-Output "     (prints a generated keypair/address; it will wait for the node to be reachable)"
Write-Output "  2. Mine to it:"
Write-Output "       kaspa-miner --mining-address <devnet-address> --kaspad-address 127.0.0.1 --port 26610 --threads 2 --mine-when-not-synced"
Write-Output ""
Write-Output "Blocks mined against node1 should appear on node2 and node3 within a few seconds (check their logs for `"via relay`")."
Write-Output ""
Write-Output "To stop all three nodes: Get-Process kaspad | Stop-Process"
