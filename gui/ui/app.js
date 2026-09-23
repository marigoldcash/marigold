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
}
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
    $("mine-note").textContent = m.can_mine ? "Mining pays to this wallet's ledger address and starts only once the sync is complete." : "Mining needs a node on this machine or a sync of your own: through a public computer, its operator would see where your rewards go.";
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
  await listen("say", (event) => { $("say").textContent = event.payload; });
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
      const ol = $("words"); ol.innerHTML = "";
      for (const w of made.words.split(" ")) { const li = document.createElement("li"); li.textContent = w; ol.appendChild(li); }
      show("words");
    }
  } catch (e) { $("create-error").textContent = String(e); }
  $("create").disabled = false;
});
$("words-done").addEventListener("click", () => { $("words").innerHTML = ""; show("open"); });

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
    const g = await invoke("give", { amount: $("give-amount").value });
    $("give-summary").textContent = `${g.value} ${opened.ticker} in ${g.notes} note(s), fee ${g.fee} ${opened.ticker}.`;
    $("give-qr").src = g.qr; $("give-code").value = g.code; $("give-done").hidden = false;
    $("give-amount").value = "";
    refreshBalance();
  } catch (e) { $("give-error").textContent = String(e); }
  $("give").disabled = false;
});
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

$("refresh-status").addEventListener("click", refreshStatus);
$("close-wallet").addEventListener("click", async () => {
  watching = null; if (syncTimer) { clearInterval(syncTimer); syncTimer = null; } $("syncing").hidden = true;
  await invoke("close");
  opened = null; $("context").textContent = "";
  show("open");
});

init();
