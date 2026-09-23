const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;
const $ = (id) => document.getElementById(id);

let opened = null;
let watching = null;

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
}
async function copy(text, button, done) {
  try { await navigator.clipboard.writeText(text); button.textContent = done; setTimeout(() => (button.textContent = button.dataset.label), 1500); }
  catch (_) { button.textContent = "Select it and copy"; }
}

async function init() {
  $("version").textContent = "v" + (await invoke("version"));
  try {
    const list = await invoke("wallets");
    const select = $("wallet-list");
    select.innerHTML = "";
    for (const w of list) {
      const o = document.createElement("option");
      o.value = w.filename; o.textContent = w.title === w.filename ? w.filename : `${w.title} (${w.filename})`;
      select.appendChild(o);
    }
    if (!list.length) $("open-error").textContent = "No wallet found on this machine.";
  } catch (e) { $("open-error").textContent = String(e); }
  await listen("say", (event) => { $("say").textContent = event.payload; });
  show("open");
}

$("open").addEventListener("click", async () => {
  const button = $("open");
  button.disabled = true; $("open-error").textContent = "";
  $("say").textContent = $("node").value === "own" ? "Starting the sync here…" : "Connecting…";
  try {
    opened = await invoke("open", { wallet: $("wallet-list").value, password: $("password").value, node: $("node").value });
    $("password").value = "";
    $("context").textContent = `${opened.wallet} · ${opened.network} · ${opened.own_node ? "your own sync" : "public computer"}`;
    $("say").textContent = "";
    show("home"); tab("balance");
  } catch (e) { $("open-error").textContent = String(e); $("say").textContent = ""; }
  button.disabled = false;
});
$("password").addEventListener("keydown", (e) => { if (e.key === "Enter") $("open").click(); });

for (const b of document.querySelectorAll(".tab")) b.addEventListener("click", () => tab(b.dataset.tab));

// Pay: the amount shows as soon as a code is pasted, before anything is spent.
$("pay-code").addEventListener("input", async () => {
  const code = $("pay-code").value.trim();
  $("pay-done").hidden = true; $("pay-error").textContent = "";
  if (!code) { $("pay-amount").textContent = ""; $("pay").disabled = true; return; }
  try {
    const amount = await invoke("request_amount", { code });
    $("pay-amount").textContent = `${amount} ${opened.ticker}`;
    $("pay").disabled = false;
  } catch (e) { $("pay-amount").textContent = ""; $("pay-error").textContent = String(e); $("pay").disabled = true; }
});
$("pay").addEventListener("click", async () => {
  $("pay").disabled = true; $("pay-error").textContent = "";
  try {
    const paid = await invoke("pay", { code: $("pay-code").value.trim() });
    $("pay-summary").textContent = `${paid.value} ${opened.ticker} in ${paid.notes} note(s), fee ${paid.fee} ${opened.ticker}.`;
    $("receipt").value = paid.receipt;
    $("pay-done").hidden = false;
    $("pay-code").value = ""; $("pay-amount").textContent = "";
    refreshBalance();
  } catch (e) { $("pay-error").textContent = String(e); $("pay").disabled = false; }
});
$("copy-receipt").dataset.label = "Copy the receipt";
$("copy-receipt").addEventListener("click", () => copy($("receipt").value, $("copy-receipt"), "Copied"));

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
  watching = null;
  await invoke("close");
  opened = null; $("context").textContent = "";
  show("open");
});

init();
