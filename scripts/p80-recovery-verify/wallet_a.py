"""P8.0 verify, wallet A: create (one ceremony), connect, import the faucet
notes, let them confirm, verify on chain, back the vault up, and hand the
24 words + ledger address to the second half."""
import os, sys, re, json, time, pexpect
sp, home = sys.argv[1], sys.argv[2]
claim = json.load(open(f"{sp}/claim.json"))
payloads = [n["payload"] for n in sorted(claim["notes"], key=lambda n: -float(n["denomination_magld"].replace(",", "")))]
env = dict(os.environ, HOME=home, TERM="xterm-256color")
c = pexpect.spawn("./target/release/marigold-cli", env=env, encoding="utf-8", timeout=180, dimensions=(44, 120))
log = open(f"{sp}/wallet_a.log", "w"); c.logfile_read = log
def step(exp, reply): c.expect_exact(exp); c.send(reply + "\r")
for e, r in [("$", "wallet create alpha"), ("different wallet name", ""), ("Default account title", ""),
             ("phishing hint", ""), ("encryption password", "hunter2hunter2"), ("Re-enter", "hunter2hunter2"),
             ("mnemonic passphrase (optional)", ""), ("or press <enter> to generate one", "")]:
    step(e, r)
# The words come framed and numbered, each painted separately (256-colour
# index 222, or truecolor when COLORTERM says so). Read up to the prompt,
# then pull every painted word out of what arrived, in order.
c.expect_exact("once you have written them down")
WORD = re.compile(r"\x1b\[(?:38;5;222|38;2;243;207;130)m([a-z]+)\x1b\[39m")
words_list = WORD.findall(c.before)
assert len(words_list) == 24, f"expected 24 painted words, found {len(words_list)}"
words = " ".join(words_list)
# One ceremony: the panel title appears once, and no second phrase is offered.
ceremonies = c.before.count("recovery words")
c.send("\r")
c.expect(r"(marigoldtest:[a-z0-9]{50,})")
address = c.match.group(1)
c.expect_exact("›"); c.send("connect\r"); step("[Y/n]", "n")
c.expect_exact("Public node connected")
for p in payloads:
    c.expect_exact("›"); c.send(f"note import {p}\r")
    c.expect_exact("Enter wallet password"); c.send("hunter2hunter2\r")
    c.expect_exact("$", timeout=180)
time.sleep(45)  # let the import rotations confirm before the backup is judged against the chain
c.send("note verify\r"); c.expect_exact("$", timeout=180)
c.send(f"note vault backup {sp}/backup\r"); c.expect_exact("copied vault files", timeout=60)
c.expect_exact("›"); c.send("balance\r"); c.expect_exact("$", timeout=60)
c.send("exit\r"); c.expect(pexpect.EOF, timeout=120)
json.dump({"words": words, "address": address, "ceremonies_seen": ceremonies}, open(f"{sp}/wallet_a.json", "w"))
print(json.dumps({"address": address, "words": words.split()[:3] + ["..."], "notes_imported": len(payloads)}))
