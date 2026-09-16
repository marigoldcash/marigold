"""P8.0 verify, wallet B: a blank wallet created from A's 24 words must derive
A's ledger address on its own (words alone recover the ledger), and
`note vault restore` must then bring back A's notes (words + files)."""
import os, sys, re, json, time, pexpect
sp, home = sys.argv[1], sys.argv[2]
a = json.load(open(f"{sp}/wallet_a.json"))
env = dict(os.environ, HOME=home, TERM="xterm-256color")
c = pexpect.spawn("./target/release/marigold-cli", env=env, encoding="utf-8", timeout=240, dimensions=(44, 120))
log = open(f"{sp}/wallet_b.log", "w"); c.logfile_read = log
def step(exp, reply): c.expect_exact(exp); c.send(reply + "\r")
for e, r in [("›", "advanced on"), ("›", "wallet create beta"), ("different wallet name", ""), ("Default account title", ""),
             ("phishing hint", ""), ("encryption password", "hunter2hunter2"), ("Re-enter", "hunter2hunter2"),
             ("mnemonic passphrase (optional)", "")]:
    step(e, r)
step("or press <enter> to generate one", a["words"])
step("once you have written them down", "")
c.expect(r"(marigoldtest:[a-z0-9]{50,})")
address_b = c.match.group(1)
c.expect_exact("›"); c.send("connect public\r")
c.expect_exact("Public computer connected")
c.expect_exact("›"); c.send(f"note vault restore {sp}/backup {a['words']}\r")
c.expect_exact("Enter wallet password"); c.send("hunter2hunter2\r")
c.expect(r"recovered (\d+) live note\(s\); (\d+) stale; (\d+) corrupted", timeout=240)
live, stale, corrupted = map(int, c.match.groups())
c.expect_exact("›", timeout=300)
time.sleep(40)  # let the restore-rotation confirm so balance is the verified figure
c.send("balance\r"); c.expect_exact("›", timeout=120)
before = c.before
c.send("exit\r"); c.expect(pexpect.EOF, timeout=120)
plain = re.sub(r"\x1b\[[0-9;]*m", "", before)
m = re.search(r"notes\s+([0-9,]+\.?\d*)\s+TMAGLD", plain)
verdict = {
  "address_a": a["address"], "address_b": address_b, "ledger_rederived": address_b == a["address"],
  "recovered_live": live, "stale": stale, "corrupted": corrupted,
  "notes_balance_after_restore": m.group(1) if m else None,
  "ceremonies_in_A": a["ceremonies_seen"],
}
json.dump(verdict, open(f"{sp}/wallet_b.json","w"), indent=1)
print(json.dumps(verdict, indent=1))
