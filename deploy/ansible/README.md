# Marigold node/bridge deployment

Ansible playbook to build `kaspad` and `stratum-bridge` once on a designated build
host and deploy them as systemd services across any number of seed nodes and
mining bridges. Written for the Marigold public testnet, but nothing here is
testnet-specific — set `marigold_network: mainnet` in your inventory and the same
playbook stands up a mainnet node.

Includes a `bootstrap` play for genuinely fresh machines (only a root/default
admin account) that creates the `deploy` user everything else connects as —
see the `[bootstrap]` group in `inventory.example.ini`. Skip it for hosts that
already have a working `deploy` account.

Full narrative, security posture, and DNS-seeder setup: see
[docs/x-fork/TESTNET.md](../../docs/x-fork/TESTNET.md). Quick start:

```bash
cd deploy/ansible
ansible-galaxy collection install -r requirements.yml
cp inventory.example.ini inventory.ini   # then edit with your real hosts/IPs
ansible-playbook playbook.yml
```

`inventory.ini` is gitignored — your IPs and SSH users never land in the repo.
