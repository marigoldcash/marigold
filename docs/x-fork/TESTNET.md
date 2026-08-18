# TESTNET.md — Public testnet deployment runbook

Deployment runbook for the topology recorded in [STATE.md](STATE.md)'s "Live
infrastructure" section: a small number of fixed-IP machines running `kaspad` as
public seed nodes, some of them also bridging an ASIC via `stratum-bridge`. This
is infrastructure work in support of FORK-PLAN's **P8.7 — Public testnet soak**;
it is not itself a numbered FORK-PLAN step, and running it does not close P8.7
(that needs months of actual soak time with outside participants).

Deployment is via the Ansible playbook in
[`deploy/ansible/`](../../deploy/ansible/) — built to be reusable, not a
one-off script for this specific testnet. The same playbook stands up a
mainnet node by changing one variable (`marigold_network: mainnet`); nothing in
it is hardcoded to exactly three hosts or to this testnet's IPs.

## 1. What you need before starting

- One or more Linux hosts on fixed, publicly-reachable IPv4 addresses, reachable
  over SSH with sudo.
- At least one of them with the Rust toolchain installed
  ([rustup.rs](https://rustup.rs/), matching the root [README.md](../../README.md)'s
  own build instructions) — this is the sole **build host**; every other host
  only receives the two binaries it produces (`kaspad`, `stratum-bridge`). This
  mirrors STATE.md's recorded plan: "build host (only machine with the
  toolchain — others get binaries)."
- Ansible on your own control machine (wherever you run `ansible-playbook`
  from — this does not need to be one of the target hosts):
  `pip install ansible` or your distro's package.
- If any host has an ASIC attached: confirmed already in STATE.md — PoW is
  unchanged kHeavyHash (ASICs mine Marigold natively with no firmware changes)
  and the in-repo `stratum-bridge` is both rebranded and P6.5-aware (its
  hasher mirrors the `pool_commitment` header field; without that, every ASIC
  share would hash wrong against a P6.5-or-later node).

## 2. Security posture (read this before running anything)

- **A public seed node's RPC never needs to be reachable from the internet,
  and this runbook never opens it.** `kaspad`'s gRPC listener defaults to
  loopback-only (`127.0.0.1`) whenever `--rpclisten` isn't passed, and its wRPC
  listeners (Borsh/JSON) don't start *at all* unless `--rpclisten-borsh` /
  `--rpclisten-json` is passed explicitly (the P7.7/P7.8 finding documented in
  [WALLET.md](WALLET.md) and [SMOKE.md](SMOKE.md)). The `deploy/ansible/roles/kaspad`
  systemd unit simply never passes any of those three flags, or `--unsaferpc`,
  on a public host — the safest posture is not having the flag to misconfigure
  in the first place. A local `stratum-bridge` instance still reaches its
  node fine, since they're on the same host talking over loopback.
- **P2P is meant to be public** — that's the entire point of a seed node —
  and needs no special handling: `--listen` defaults to all interfaces when
  omitted.
- **Firewall management is opt-in** (`ufw_manage: true` in your inventory),
  deliberately not the default. If you do turn it on: the `common` role adds
  an explicit SSH-allow rule *before* it ever runs `ufw enable`, so there's
  never a window where a host is reachable only by a rule that hasn't landed
  yet — but this only helps if the SSH port it allows (`ssh_port`, default
  `22`) is actually the one you use. Get this wrong on a remote box with no
  console access and you lose the box, not just a re-run. If you're not sure,
  leave `ufw_manage` unset and manage the firewall yourself.
- **`--unsaferpc`** ("Enable RPC commands which affect the state of the node")
  must never be passed on a host reachable from the internet. It is not in
  any template in `deploy/ansible/roles/kaspad` or `roles/bridge` — if you add
  custom flags via `kaspad_extra_args`, do not add it there either.

## 3. DNS seeders

`TESTNET_PARAMS.dns_seeders` (`consensus/core/src/config/params.rs`) is
already committed as three static hostnames: `tn-seed1.marigold.cash`,
`tn-seed2.marigold.cash`, `tn-seed3.marigold.cash`. Every node — including
future ones you never provision by hand — resolves these via plain DNS `A`
lookups and connects to whatever they return on the network's default P2P
port (no port encoding in DNS; see `dns_seed_single` in
`components/connectionmanager/src/lib.rs`), so:

1. Create one DNS-only `A` record per seed host in whatever registrar/DNS
   provider hosts `marigold.cash` (Cloudflare, per STATE.md) — **not
   proxied**. An orange-clouded/proxied record resolves to Cloudflare's edge
   IPs, not your node, and P2P breaks silently.
2. If you add, remove, or re-IP a seed node later, update its `A` record —
   there's no code change needed on the node side; `dns_seeders` only needs
   to change if you add/remove a *hostname*, not if a hostname's IP moves.
3. Verify with `dig +short tn-seed1.marigold.cash` from an unrelated network
   before relying on it — DNS propagation delay is a real, common cause of
   "new node can't find any peers" that has nothing to do with kaspad itself.

## 4. Deploy

```bash
cd deploy/ansible
ansible-galaxy collection install -r requirements.yml
cp inventory.example.ini inventory.ini
```

Edit `inventory.ini`: real IPs, SSH users, which group each host belongs to
(`bootstrap` — every fresh host, see below; `build` — exactly one host;
`seed_nodes` — every kaspad host; `bridges` — only hosts with an ASIC
attached), and per-host variables (`kaspad_archival`, `kaspad_external_ip`,
`kaspad_addpeers`, `kaspad_ram_scale`, `bridge_stratum_port`, ...).
`inventory.ini` is gitignored — it never lands in the repo.

**If a machine is genuinely fresh** (only a root/default admin account, no
`deploy` user yet), generate a deploy key and point `deploy_ssh_public_key_file`
at it if you haven't already:

```bash
ssh-keygen -t ed25519 -f ~/.ssh/marigold_deploy
```

and set `ansible_user=root` (or whatever your provider's default account is —
e.g. `ubuntu` on many cloud images) for that host in the `[bootstrap]` group
only. Every *other* group keeps `ansible_user=deploy`. If a machine already has
a working `deploy` account with sudo and your key installed, just leave it out
of `[bootstrap]` entirely. Then:

```bash
ansible-playbook playbook.yml
```

This runs four plays in order: **bootstrap** (apt-installs the handful of base
packages every later role assumes — `sudo`, `git`, `curl`, `ca-certificates` —
creates the `deploy` user with sudo and your SSH key, and deliberately stops
there: it does not touch `sshd_config`, so root/password SSH login stays
exactly as your image's default set it; harden that yourself once you've
confirmed `ssh deploy@<host>` works); **build** (clone the pinned `marigold_ref` on
the build host, `cargo build --release --bin kaspad --bin stratum-bridge`,
fetch the two binaries to your control machine, cached by commit hash so an
unchanged commit never gets rebuilt or re-copied on a later run — the same
stale-binary trap this project has hit twice before, see NOTES.md's P2.3 and
P4.2 entries, is exactly what the per-commit cache key is there to prevent);
**deploy kaspad** (copy the binary, template the systemd unit from each
host's variables, open the P2P port if `ufw_manage` is set, enable and start
`marigold-kaspad.service`); **deploy bridges** (same pattern for
`stratum-bridge`, only on hosts in the `bridges` group).

The bridge is started with `--node-mode external`, pointed at
`127.0.0.1:<grpc-port>` — **this matters**: `stratum-bridge` defaults to
spawning its *own* embedded kaspad (`--node-mode inprocess`) if you don't
override it, which would start a second, separate node competing with the
one this same playbook just installed as its own systemd service. External
mode just connects to the already-running node over loopback gRPC.

## 5. Verify

On each seed node:

```bash
sudo systemctl status marigold-kaspad
sudo journalctl -u marigold-kaspad -f
```

Look for `P2P Server starting on: 0.0.0.0:<port>` and, within a few minutes,
peer-connection log lines. Confirm gRPC really did stay loopback-only:

```bash
ss -tlnp | grep kaspad
```

should show the gRPC port bound to `127.0.0.1`, not `0.0.0.0`, and no wRPC
port listening at all (neither was ever requested). From a host on the public
internet (not the box itself), confirm the P2P port *is* reachable:

```bash
nc -zv <external-ip> <p2p-port>
```

On a bridge host:

```bash
sudo systemctl status marigold-bridge
sudo journalctl -u marigold-bridge -f
```

Point a testnet-capable ASIC at `stratum+tcp://<external-ip>:<bridge_stratum_port>`
and confirm shares are accepted (rejected shares at a nonzero rate immediately
after pointing an ASIC that was mining a *different* kHeavyHash coin usually
means it hasn't picked up the new job yet — give it a few seconds).

## 6. What this runbook does not cover

Per STATE.md's recorded sequence, provisioning is one step in a longer chain:
**this runbook → provision the real hosts → stand up a faucet (the one
genuinely new build item, not yet started) → invite outside testers → begin
the P8.7 incident log.** The faucet, tester outreach, and incident log are
separate, not-yet-started pieces of work — this document only gets kaspad and
stratum-bridge running and reachable.
