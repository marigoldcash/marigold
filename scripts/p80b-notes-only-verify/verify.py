"""P8.0b verify: a wallet that keeps notes only — no ledger account, no
ledger address ever derived — is paid, pays out to an address without
change, is backed up, and is restored into another notes-only wallet.

Three sessions against the live public testnet:

  pure   a fresh wallet that answers 'n' to the ledger question
  payer  an existing ledger+notes wallet holding notes (P8.0's wallet B)
  again  a fresh notes-only wallet made from pure's 24 words, restored
         from pure's vault backup

Usage:
  python3 scripts/p80b-notes-only-verify/verify.py <scratch> <payer-HOME> <fresh-HOME-pure> <fresh-HOME-again>
"""
import os, sys, re, json, time, pexpect

sp, payer_home, pure_home, again_home = sys.argv[1:5]
PW = "hunter2hunter2"
WORD = re.compile(r"\x1b\[(?:38;5;222|38;2;243;207;130)m([a-z]+)\x1b\[39m")


def plain(s):
    return re.sub(r"\x1b\[[0-9;]*[A-Za-z]", "", s)


def spawn(home, name):
    env = dict(os.environ, HOME=home, TERM="xterm-256color")
    c = pexpect.spawn("./target/release/marigold-cli", env=env, encoding="utf-8", timeout=180, dimensions=(44, 120))
    c.logfile_read = open(f"{sp}/{name}.log", "w")
    return c


def transcript(c):
    c.logfile_read.flush()
    return plain(open(c.logfile_read.name).read())


def step(c, exp, reply, timeout=180):
    c.expect_exact(exp, timeout=timeout)
    c.send(reply + "\r")


def run(c, command, timeout=120):
    """Send a command, return what it printed before the next prompt.

    The terminal redraws its prompt on its own (account activation, the
    balance in it changing), so a bare wait for '$' can match a redraw from
    before the command ran. Waiting for the command's own echo first pins
    the search to output that came after it."""
    c.send(command + "\r")
    c.expect_exact(command[:40], timeout=timeout)
    c.expect_exact("› ", timeout=timeout)
    return plain(c.before)


def command(c, line, timeout=240):
    c.send(line + "\r")
    c.expect_exact(line[:40], timeout=timeout)
    out = ""
    while True:
        i = c.expect_exact(["Enter wallet password", "› "], timeout=timeout)
        out += plain(c.before)
        if i == 0:
            c.send(PW + "\r")
        else:
            return out


def connect(c):
    c.send("connect\r")
    step(c, "[Y/n]", "n")
    c.expect_exact("Public node connected")
    c.expect_exact("›")


def create_notes_only(c, name, words=None):
    # The ceremony (the words this driver hands from one wallet to the next)
    # only runs with advanced on.
    run(c, "advanced on")
    for e, r in [("$", f"wallet create {name}"), ("different wallet name", ""), ("Keep a ledger account too?", "n"),
                 ("phishing hint", ""), ("encryption password", PW), ("Re-enter", PW)]:
        step(c, e, r)
    step(c, "or press <enter> to generate one", words or "")
    c.expect_exact("once you have written them down")
    found = WORD.findall(c.before)
    assert len(found) == 24, f"expected 24 painted words, found {len(found)}"
    c.send("\r")
    c.expect_exact("keeps notes only")
    c.expect_exact("›")
    wizard = transcript(c)
    assert "marigoldtest:" not in wizard, "a notes-only wizard printed a ledger address"
    assert "Default account title" not in wizard, "a notes-only wizard asked for an account title"
    assert "mnemonic passphrase" not in wizard, "a notes-only wizard asked for a bip39 passphrase"
    return " ".join(found)


verdict = {}

# --- 1. the purist -----------------------------------------------------------
pure = spawn(pure_home, "pure")
words = create_notes_only(pure, "pure")
connect(pure)
verdict["list_shows_no_account"] = "No accounts yet" in run(pure, "list")
refusals = {}
for refused in ["note mint 1", "address", "sweep"]:
    out = run(pure, refused)
    refusals[refused] = "keeps notes only" in out and "account create bip32" in out
verdict["ledger_commands_refuse"] = refusals
b0 = run(pure, "balance")
verdict["balance_has_no_ledger_row"] = "ledger" not in b0

# --- 2. the payer ------------------------------------------------------------
payer = spawn(payer_home, "payer")
which = payer.expect_exact(["Enter wallet password", "$"])
if which == 1:
    # A remembered wallet may still ask on its own a moment after the prompt.
    if payer.expect_exact(["Enter wallet password", pexpect.TIMEOUT], timeout=4) == 1:
        payer.send("wallet open beta\r")
        payer.expect_exact("Enter wallet password")
payer.send(PW + "\r")
# Opening offers to connect; the connect flow then asks its own [Y/n] (public
# node, answered as the P8.0 driver did). Take whichever comes, until the prompt.
connected = False
while True:
    i = payer.expect_exact(["Connect now? [Y/n]", "[Y/n]: ", "Public node connected", "$"], timeout=240)
    if i == 0:
        payer.send("y\r")
    elif i == 1:
        payer.send("n\r")
    elif i == 2:
        connected = True
        payer.expect(r"\[[0-9a-f]{8}\]", timeout=120)  # the prompt carries the account id once it is active
    else:
        break
if not connected and "Public node connected" not in transcript(payer):
    connect(payer)
payer_address = re.search(r"(marigoldtest:[a-z0-9]{50,})", run(payer, "address")).group(1)
run(payer, "balance", timeout=120)
# --- 3. the request, then the payment, inside the request's 120 s window ---
# `request` asks the password, prints the QR and its text form, then waits.
pure.send("request 1.12\r")
step(pure, "Enter wallet password", PW)
pure.expect_exact("requesting 1.12")
lines = [l.strip() for l in plain(pure.before).splitlines() if l.strip()]
request = next(l for l in reversed(lines) if re.fullmatch(r"[A-Za-z0-9:_\-]{40,}", l))
pure.expect_exact("watching for payment")

payer.send(f"pay {request}\r")
step(payer, "Enter wallet password", PW)
payer.expect(r"paid (\d+) note\(s\) \(fee ([0-9.,]+) TMAGLD\)", timeout=240)
verdict["payer_paid_notes"] = int(payer.match.group(1))
payer.expect_exact("›")

def payer_notes():
    m = re.search(r"notes\s+([0-9,]+\.?\d*)\s+TMAGLD", run(payer, "balance", timeout=120))
    return float(m.group(1).replace(",", "")) if m else None


# --- 4. the purist is paid ------------------------------------------------------
pure.expect(r"payment received: ([0-9.,]+) TMAGLD in (\d+) note\(s\)", timeout=300)
verdict["pure_received"] = pure.match.group(1)
verdict["pure_received_notes"] = int(pure.match.group(2))
pure.expect_exact("›")
time.sleep(45)  # let the payment confirm before redeeming from it
b1 = run(pure, "balance")
m = re.search(r"notes\s+([0-9,]+\.?\d*)\s+TMAGLD", b1)
verdict["pure_notes_balance"] = m.group(1) if m else None
verdict["balance_still_has_no_ledger_row"] = "ledger" not in b1

payer_notes_before_payout = payer_notes()

# --- 5. paying out without change ---------------------------------------------
# 0.05 from a wallet whose smallest cover is a whole 1: far over, and there is
# no ledger for the difference — refused, nothing spent.
over = command(pure, f"exchange {payer_address} 0.05")
verdict["far_over_is_refused"] = "over the" in over and "ledger" in over
# 1.1 is covered to within one 0.01 (1 + 0.1 + 0.01): the whole 1.11 less the
# fee goes to the address.
paid = command(pure, f"exchange {payer_address} 1.1")
m = re.search(r"Sent ([0-9.,]+) TMAGLD to \S+ from (\d+) note\(s\) \(fee ([0-9.,]+) TMAGLD\)", paid)
verdict["payout"] = {"sent": m.group(1), "notes": int(m.group(2)), "fee": m.group(3)} if m else {"failed": paid[-600:]}
verdict["payout_named_notes_only_path"] = "Paying from notes" in paid
# 0.01 left now: asking for more is a shortfall, and there is no ledger send to fall back on.
short = command(pure, f"exchange {payer_address} 0.5")
verdict["shortfall_is_refused"] = "do not cover" in short and "no ledger" in short

# --- 6. backup, then restore into another notes-only wallet -----------------
backup = command(pure, f"note vault backup {sp}/backup")
verdict["backup_copied"] = "copied vault files" in backup
b2 = run(pure, "balance")
m = re.search(r"notes\s+([0-9,]+\.?\d*)\s+TMAGLD", b2)
verdict["pure_notes_after_payout"] = m.group(1) if m else None
pure.send("exit\r"); pure.expect(pexpect.EOF, timeout=120)

again = spawn(again_home, "again")
create_notes_only(again, "again", words)
connect(again)
again.send(f"note vault restore {sp}/backup {words}\r")
step(again, "Enter wallet password", PW)
again.expect(r"recovered (\d+) live note\(s\); (\d+) stale; (\d+) corrupted", timeout=300)
live, stale, corrupted = map(int, again.match.groups())
verdict["restore"] = {"live": live, "stale": stale, "corrupted": corrupted}
again.expect_exact("$", timeout=300)
verdict["again_list_shows_no_account"] = "No accounts yet" in run(again, "list")
verdict["again_never_printed_a_ledger_address"] = "marigoldtest:" not in transcript(again)
again.send("exit\r"); again.expect(pexpect.EOF, timeout=120)

# --- 7. the payer got the deposit -------------------------------------------
# The payer keeps a ledger with auto-mint armed (the default at open when the
# key has no passphrase), so a deposit is turned into notes within the minute:
# what shows is the notes figure rising by about the amount paid, not a ledger
# row. With auto-mint off it would be the ledger row instead; accept either.
time.sleep(60)
b3 = run(payer, "balance", timeout=120)
m = re.search(r"ledger\s+([0-9,]+\.\d+)\s+TMAGLD", b3)
after = payer_notes()
verdict["payer_notes_before_payout"] = payer_notes_before_payout
verdict["payer_notes_after_payout"] = after
verdict["payer_ledger_row"] = m.group(1) if m else None
verdict["payer_received_deposit"] = (
    (after is not None and payer_notes_before_payout is not None and after - payer_notes_before_payout >= 1.0)
    or (m is not None and float(m.group(1).replace(",", "")) >= 1.0)
)
payer.send("exit\r"); payer.expect(pexpect.EOF, timeout=120)

verdict["pass"] = all([
    verdict["list_shows_no_account"], all(refusals.values()), verdict["balance_has_no_ledger_row"],
    verdict["pure_received_notes"] > 0, verdict["far_over_is_refused"], "sent" in verdict["payout"],
    verdict["shortfall_is_refused"], verdict["backup_copied"], live >= 1 and corrupted == 0, verdict["again_list_shows_no_account"],
    verdict["again_never_printed_a_ledger_address"], verdict["payer_received_deposit"],
])
json.dump(verdict, open(f"{sp}/verdict.json", "w"), indent=1)
print(json.dumps(verdict, indent=1))
