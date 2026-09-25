const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;
const $ = (id) => document.getElementById(id);

let opened = null;
let watching = null;
let syncTimer = null;

// A sync of the app's own: say how far it is, every few seconds, until it is done.
async function followSync() {
  try {
    const st = await invoke("sync_state");
    const banner = $("syncing");
    if (st.synced) {
      banner.hidden = true;
      if (st.own_node) { banner.textContent = ""; }
      clearInterval(syncTimer); syncTimer = null;
      return;
    }
    banner.hidden = false;
    banner.textContent = st.own_node
      ? `Your copy of the network is catching up: ${st.headers.toLocaleString()} headers, ${st.blocks.toLocaleString()} blocks so far. Notes work through a public computer meanwhile only if you opened it that way; paying and requesting wait for the sync.`
      : `The node this wallet uses is still catching up (${st.blocks.toLocaleString()} blocks so far); figures may change until it has.`;
  } catch (_) {}
}
function startSyncWatch() {
  if (syncTimer) clearInterval(syncTimer);
  followSync(); syncTimer = setInterval(followSync, 5000);
}

function show(screen) {
  for (const s of document.querySelectorAll(".screen")) s.hidden = s.id !== `screen-${screen}`;
}
function tab(name) {
  for (const b of document.querySelectorAll(".tab")) b.classList.toggle("active", b.dataset.tab === name);
  for (const p of document.querySelectorAll(".tab-panel")) p.hidden = p.id !== `tab-${name}`;
  if (name === "balance") refreshBalance();
  if (name === "status") refreshStatus();
  if (name === "backup") refreshBackup(); else stopPairWatch();
}

// Backup: where it goes, whether it runs by itself, and the bot behind it.
let pairTimer = null;
function stopPairWatch() { if (pairTimer) { clearInterval(pairTimer); pairTimer = null; } }
const when = (secs) => secs ? new Date(secs * 1000).toLocaleString() : "never";
const size = (bytes) => bytes < 1024 ? `${bytes} B` : bytes < 1048576 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1048576).toFixed(1)} MB`;
async function refreshBackup() {
  try {
    const st = await invoke("backup_status");
    $("backup-status").textContent =
      `Backups go to: ${st.destination}\nAutomatic: ${st.automatic}\nLast full copy: ${when(st.checkpoint_at)}${st.checkpoint_at ? ` (${size(st.checkpoint_bytes)})` : ""}\nChange sets since: ${st.deltas}\nLast post: ${when(st.last_post_at)}`;
    $("bot-setup").hidden = st.bot;
    $("bot-pair").hidden = !(st.bot && !st.paired);
    $("backup-actions").hidden = !(st.bot && st.paired);
    if (st.bot && !st.paired) {
      $("pair-code").textContent = st.pairing_code ? `/start ${st.pairing_code}` : "(the code has expired — make a new one)";
      $("bot-repair").hidden = !!st.pairing_code;
      if (!pairTimer) pairTimer = setInterval(refreshBackup, 5000);
    } else stopPairWatch();
    $("backup-toggle").hidden = st.automatic === "not started";
    $("backup-toggle").textContent = st.automatic === "on" ? "Pause automatic backups" : "Resume automatic backups";
    $("backup-toggle").dataset.on = st.automatic === "on" ? "1" : "";
  } catch (e) { $("backup-status").textContent = String(e); }
}
$("backup-now").addEventListener("click", async () => {
  $("backup-error").textContent = ""; $("backup-done").hidden = true; $("backup-now").disabled = true;
  try { const line = await invoke("backup_now"); $("backup-done").textContent = `Telegram backup: ${line}.`; $("backup-done").hidden = false; refreshBackup(); }
  catch (e) { $("backup-error").textContent = String(e); }
  $("backup-now").disabled = false;
});
$("backup-toggle").addEventListener("click", async () => {
  $("backup-error").textContent = "";
  try { await invoke("backup_automatic", { on: !$("backup-toggle").dataset.on }); refreshBackup(); } catch (e) { $("backup-error").textContent = String(e); }
});
$("backup-file").addEventListener("click", async () => {
  $("backup-error").textContent = ""; $("backup-done").hidden = true; $("backup-file").disabled = true;
  try { const path = await invoke("backup_file"); $("backup-done").textContent = `Saved to ${path}. It opens with your 24 words.`; $("backup-done").hidden = false; }
  catch (e) { $("backup-error").textContent = String(e); }
  $("backup-file").disabled = false;
});
async function setupBot(token, pin) {
  $("backup-error").textContent = ""; $("bot-setup-go").disabled = true;
  try { await invoke("telegram_setup", { token, pin }); $("bot-token").value = ""; $("bot-pin").value = ""; refreshBackup(); }
  catch (e) { $("backup-error").textContent = String(e); }
  $("bot-setup-go").disabled = false;
}
$("bot-setup-go").addEventListener("click", () => setupBot($("bot-token").value, $("bot-pin").value));
$("bot-repair").addEventListener("click", () => {
  // A new code needs the token and PIN again: back to the setup fields.
  $("bot-pair").hidden = true; $("bot-setup").hidden = false;
});

// Restore from Telegram: token and words first, then the forward.
let restoringTg = false;
$("go-restore-tg").addEventListener("click", () => {
  $("rt-error").textContent = ""; $("rt-done").hidden = true; $("rt-progress").hidden = true; $("rt-progress").textContent = "";
  show("restore-tg");
});
$("rt-back").addEventListener("click", () => { if (!restoringTg) show("open"); });
$("rt-start").addEventListener("click", async () => {
  $("rt-error").textContent = ""; $("rt-done").hidden = true; $("rt-start").disabled = true; restoringTg = true;
  $("rt-progress").hidden = false; $("rt-progress").textContent = "Waiting for the parts you forward to the bot (up to ten minutes)…";
  try {
    const line = await invoke("restore_telegram", { token: $("rt-token").value, words: $("rt-words").value, name: $("rt-name").value });
    $("rt-token").value = ""; $("rt-words").value = "";
    $("rt-done").textContent = line; $("rt-done").hidden = false;
    await loadWallets();
  } catch (e) { $("rt-error").textContent = String(e); }
  restoringTg = false; $("rt-start").disabled = false;
});
async function refreshBalance() {
  try {
    $("balance").textContent = await invoke("balance");
    $("history").textContent = await invoke("history");
  } catch (e) { $("balance").textContent = String(e); }
}
async function refreshStatus() {
  try { $("status").textContent = await invoke("status"); } catch (e) { $("status").textContent = String(e); }
  try {
    const m = await invoke("machine");
    $("machine").textContent = `Memory: ${m.memory}\n${m.mining}`;
    $("mine-note").textContent = m.can_mine ? "Mining pays to this wallet's ledger address and starts only once the sync is complete." : "Mining needs a network running on this computer or a sync of your own: through a public computer, its operator would see where your rewards go.";
    $("mine-start").hidden = !m.can_mine; $("mine-stop").hidden = !m.can_mine;
  } catch (e) { $("machine").textContent = String(e); }
}
$("mine-start").addEventListener("click", async () => {
  $("mine-error").textContent = "";
  try { $("machine").textContent = await invoke("mine", { action: "start" }); setTimeout(refreshStatus, 3000); } catch (e) { $("mine-error").textContent = String(e); }
});
$("mine-stop").addEventListener("click", async () => {
  $("mine-error").textContent = "";
  try { await invoke("mine", { action: "stop" }); refreshStatus(); } catch (e) { $("mine-error").textContent = String(e); }
});
async function copy(text, button, done) {
  try { await navigator.clipboard.writeText(text); button.textContent = done; setTimeout(() => (button.textContent = button.dataset.label), 1500); }
  catch (_) { button.textContent = "Select it and copy"; }
}

async function loadWallets(select_filename) {
  try {
    const list = await invoke("wallets");
    const select = $("wallet-list");
    select.innerHTML = "";
    for (const w of list) {
      const o = document.createElement("option");
      o.value = w.filename; o.textContent = w.title === w.filename ? w.filename : `${w.title} (${w.filename})`;
      select.appendChild(o);
    }
    if (select_filename) select.value = select_filename;
    $("open-error").textContent = list.length ? "" : "No wallet on this machine yet — create one below.";
  } catch (e) { $("open-error").textContent = String(e); }
}
async function init() {
  $("version").textContent = "v" + (await invoke("version"));
  await loadWallets();
  await listen("say", (event) => {
    $("say").textContent = event.payload;
    if (restoringTg) $("rt-progress").textContent += `\n${event.payload}`;
  });
  show("open");
}

// Create or restore a wallet.
let restoring = false;
function showCreate(restore) {
  restoring = restore;
  $("create-title").textContent = restore ? "Restore a wallet" : "Create a wallet";
  $("create").textContent = restore ? "Restore" : "Create";
  $("create-words-label").hidden = !restore; $("restore-hint").hidden = !restore;
  $("create-error").textContent = ""; $("create-password").value = ""; $("create-password2").value = ""; $("create-words").value = "";
  show("create");
}
$("go-create").addEventListener("click", () => showCreate(false));
$("go-restore").addEventListener("click", () => showCreate(true));
$("create-back").addEventListener("click", () => show("open"));
$("create").addEventListener("click", async () => {
  $("create-error").textContent = "";
  if ($("create-password").value !== $("create-password2").value) { $("create-error").textContent = "The two passwords differ."; return; }
  $("create").disabled = true;
  try {
    const made = await invoke("create_wallet", { name: $("create-name").value, password: $("create-password").value, words: restoring ? $("create-words").value : null });
    await loadWallets(made.filename);
    if (restoring) { show("open"); }
    else {
      madeWords = made.words.split(" ");
      const ol = $("words"); ol.innerHTML = "";
      for (const w of madeWords) { const li = document.createElement("li"); li.textContent = w; ol.appendChild(li); }
      $("words").hidden = false; $("words-done").hidden = false; $("words-check").hidden = true; $("check-error").textContent = "";
      show("words");
    }
  } catch (e) { $("create-error").textContent = String(e); }
  $("create").disabled = false;
});
// Two words from the paper, at random, before the wallet is used: a
// transcription error found now costs a minute; at recovery it costs
// everything. The list is hidden while asking, and comes back on request.
let madeWords = [], checkAt = [];
$("words-done").addEventListener("click", () => {
  const a = Math.floor(Math.random() * 24); const b = (a + 1 + Math.floor(Math.random() * 23)) % 24;
  checkAt = [Math.min(a, b), Math.max(a, b)];
  $("check-n1").textContent = checkAt[0] + 1; $("check-n2").textContent = checkAt[1] + 1;
  $("check-w1").value = ""; $("check-w2").value = ""; $("check-error").textContent = "";
  $("words").hidden = true; $("words-done").hidden = true; $("words-check").hidden = false; $("check-w1").focus();
});
$("words-show-again").addEventListener("click", () => {
  $("words").hidden = false; $("words-done").hidden = false; $("words-check").hidden = true;
});
$("check-words").addEventListener("click", () => {
  const given = [$("check-w1").value, $("check-w2").value].map(w => w.trim().toLowerCase());
  const wrong = checkAt.findIndex((i, k) => given[k] !== madeWords[i]);
  if (wrong >= 0) { $("check-error").textContent = `That is not word ${checkAt[wrong] + 1}. Check the paper, or show the words again.`; return; }
  madeWords = []; $("words").innerHTML = ""; $("words-check").hidden = true; show("open");
});

$("open").addEventListener("click", async () => {
  const button = $("open");
  button.disabled = true; $("open-error").textContent = "";
  $("say").textContent = $("node").value === "own" ? "Starting the sync here…" : "Connecting…";
  try {
    opened = await invoke("open", { wallet: $("wallet-list").value, password: $("password").value, node: $("node").value });
    $("password").value = "";
    $("context").textContent = `${opened.wallet} · ${opened.network} · ${opened.access}`;
    $("say").textContent = "";
    show("home"); tab("balance"); startSyncWatch();
  } catch (e) { $("open-error").textContent = String(e); $("say").textContent = ""; }
  button.disabled = false;
});
$("password").addEventListener("keydown", (e) => { if (e.key === "Enter") $("open").click(); });

for (const b of document.querySelectorAll(".tab")) b.addEventListener("click", () => tab(b.dataset.tab));

// A pasted code: the app says what it is before anything happens.
let pasted = null;
$("pay-code").addEventListener("input", async () => {
  const code = $("pay-code").value.trim();
  $("pay-done").hidden = true; $("take-done").hidden = true; $("pay-error").textContent = "";
  $("pay").hidden = true; $("take").hidden = true; $("pay-choose").hidden = true; $("pay-kind").textContent = ""; $("pay-amount").textContent = "";
  pasted = null;
  if (!code) return;
  try {
    const c = await invoke("classify", { code });
    pasted = c;
    if (c.kind === "request") {
      if (c.amount) { $("pay-amount").textContent = `${c.amount} ${opened.ticker}`; $("pay-kind").textContent = "A request for payment."; }
      else { $("pay-kind").textContent = "A request for payment that lets you choose the amount."; $("pay-choose").hidden = false; }
      $("pay").hidden = false;
    } else if (c.kind === "handover" || c.kind === "note") {
      $("pay-kind").textContent = c.kind === "note" ? "A single note handed to you." : "Notes handed to you. Taking them makes them yours alone.";
      $("take").hidden = false;
    } else if (c.kind === "receipt") {
      $("pay-kind").textContent = "A receipt for a payment somebody made. There is nothing to pay here.";
    } else {
      $("pay-error").textContent = "That is not a Marigold code.";
    }
  } catch (e) { $("pay-error").textContent = String(e); }
});
$("pay").addEventListener("click", async () => {
  $("pay").disabled = true; $("pay-error").textContent = "";
  try {
    const paid = await invoke("pay", { code: $("pay-code").value.trim(), amount: $("pay-chosen").value });
    $("pay-summary").textContent = `${paid.value} ${opened.ticker} in ${paid.notes} note(s), fee ${paid.fee} ${opened.ticker}.`;
    $("receipt").value = paid.receipt;
    $("pay-done").hidden = false; $("pay").hidden = true; $("pay-choose").hidden = true;
    $("pay-code").value = ""; $("pay-amount").textContent = ""; $("pay-kind").textContent = ""; $("pay-chosen").value = "";
    refreshBalance();
  } catch (e) { $("pay-error").textContent = String(e); }
  $("pay").disabled = false;
});
$("take").addEventListener("click", async () => {
  $("take").disabled = true; $("pay-error").textContent = "";
  try {
    const line = await invoke("take", { code: $("pay-code").value.trim() });
    $("take-done").textContent = line; $("take-done").hidden = false; $("take").hidden = true;
    $("pay-code").value = ""; $("pay-kind").textContent = "";
    refreshBalance();
  } catch (e) { $("pay-error").textContent = String(e); }
  $("take").disabled = false;
});
$("copy-receipt").dataset.label = "Copy the receipt";
$("copy-receipt").addEventListener("click", () => copy($("receipt").value, $("copy-receipt"), "Copied"));

// Give: notes as a code, like cash.
$("give").addEventListener("click", async () => {
  $("give").disabled = true; $("give-error").textContent = "";
  try {
    const locked = $("give-key").value.trim() !== "";
    const g = await invoke("give", { amount: $("give-amount").value, key: $("give-key").value, minutes: Number($("give-minutes").value) });
    $("give-summary").textContent = `${g.value} ${opened.ticker} in ${g.notes} note(s), fee ${g.fee} ${opened.ticker}.` + (locked ? " Only the key's holder can take it, within the time you chose; then it comes back to you." : "");
    $("give-qr").src = g.qr; $("give-code").value = g.code; $("give-done").hidden = false;
    $("give-amount").value = "";
    refreshBalance();
  } catch (e) { $("give-error").textContent = String(e); }
  $("give").disabled = false;
});
$("give-key").addEventListener("input", () => { $("give-minutes-label").hidden = $("give-key").value.trim() === ""; });
$("share-key").addEventListener("click", async () => {
  try {
    const k = await invoke("share_key");
    $("share-qr").src = k.qr; $("share-code").value = k.code; $("share-done").hidden = false;
  } catch (e) { $("request-error").textContent = String(e); }
});
$("copy-share").dataset.label = "Copy the key";
$("copy-share").addEventListener("click", () => copy($("share-code").value, $("copy-share"), "Copied"));
$("copy-give").dataset.label = "Copy the code";
$("copy-give").addEventListener("click", () => copy($("give-code").value, $("copy-give"), "Copied"));

// Request: make the code, show it as a QR, and watch until it is paid.
$("request").addEventListener("click", async () => {
  $("request").disabled = true; $("request-error").textContent = ""; $("request-paid").hidden = true;
  try {
    const r = await invoke("request", { amount: $("request-amount").value });
    $("request-qr").src = r.qr; $("request-code").value = r.code;
    $("request-done").hidden = false;
    $("request-watch").textContent = r.amount ? `Watching for ${r.amount} ${opened.ticker}…` : "Watching for the payment…";
    watch(r.code);
  } catch (e) { $("request-error").textContent = String(e); }
  $("request").disabled = false;
});
$("copy-request").dataset.label = "Copy the code";
$("copy-request").addEventListener("click", () => copy($("request-code").value, $("copy-request"), "Copied"));

async function watch(code) {
  watching = code;
  while (watching === code) {
    let result = null;
    try { result = await invoke("wait_request", { code, seconds: 25 }); } catch (e) { $("request-error").textContent = String(e); return; }
    if (watching !== code) return;
    if (result) {
      $("request-paid").textContent = result; $("request-paid").hidden = false;
      $("request-watch").textContent = "";
      watching = null;
      refreshBalance();
      return;
    }
  }
}

$("show-words").addEventListener("click", async () => {
  $("words-error").textContent = "";
  try {
    const text = await invoke("words", { password: $("words-password").value });
    $("words-password").value = "";
    const ol = $("words-again"); ol.innerHTML = "";
    for (const w of text.split(" ")) { const li = document.createElement("li"); li.textContent = w; ol.appendChild(li); }
    ol.hidden = false; $("hide-words").hidden = false;
  } catch (e) { $("words-error").textContent = String(e); }
});
$("hide-words").addEventListener("click", () => { $("words-again").innerHTML = ""; $("words-again").hidden = true; $("hide-words").hidden = true; });
$("close-wallet").addEventListener("click", () => { $("words-again").innerHTML = ""; $("words-again").hidden = true; $("hide-words").hidden = true; });
$("refresh-status").addEventListener("click", refreshStatus);
$("close-wallet").addEventListener("click", async () => {
  watching = null; stopPairWatch(); if (syncTimer) { clearInterval(syncTimer); syncTimer = null; } $("syncing").hidden = true;
  await invoke("close");
  opened = null; $("context").textContent = "";
  show("open");
});

init();
