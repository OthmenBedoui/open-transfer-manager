/* Popup: on/off switch, live download list, "download video from this page". */
"use strict";

const $ = (id) => document.getElementById(id);
const HELP = "https://github.com/OthmenBedoui/open-transfer-manager/blob/main/docs/BROWSER_EXTENSIONS.md";
let port = null;
let nextId = 1;
const pending = new Map();
let activeTab = null;

// ---------------------------------------------------------------- live connection
function connect() {
  try {
    port = api.runtime.connectNative(HOST);
  } catch (e) {
    showProblem(bridgeError(e));
    return;
  }
  port.onMessage.addListener((msg) => {
    if (msg.type === "downloads") return render(msg);
    const p = pending.get(msg.id);
    if (p) {
      pending.delete(msg.id);
      msg.ok ? p.resolve(msg.data) : p.reject(new Error(msg.error));
    }
  });
  port.onDisconnect.addListener(() => {
    const err = api.runtime.lastError || port?.error;
    port = null;
    showProblem(bridgeError(err || "The browser bridge stopped"));
  });
  call({ cmd: "ping" }).then((d) => {
    $("status").textContent = d.app_running ? `Connected · bridge ${d.host}` : "App not running";
  }).catch(() => {});
  port.postMessage({ id: nextId++, cmd: "subscribe" });
}

function call(msg) {
  if (!port) return Promise.reject(new Error("Not connected"));
  const id = nextId++;
  port.postMessage({ ...msg, id });
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    setTimeout(() => { if (pending.delete(id)) reject(new Error("Timed out")); }, 20000);
  });
}

function showProblem(err) {
  $("status").textContent = "Not connected";
  $("problem").hidden = false;
  $("problem-text").textContent = err.message;
  $("problem-help").href = HELP;
}

// ---------------------------------------------------------------- list
function render(msg) {
  if (!msg.app_running) {
    $("status").textContent = "App not running";
    $("list").replaceChildren();
    $("empty").hidden = false;
    $("empty").textContent = "Open Transfer Manager is not running.";
    $("total-rate").textContent = "";
    return;
  }
  $("problem").hidden = true;
  const items = (msg.downloads || []).slice().reverse().slice(0, 12);
  $("empty").hidden = items.length > 0;
  $("empty").textContent = "No downloads yet.";
  $("total-rate").textContent = msg.total_rate ? `↓ ${fmtSize(msg.total_rate)}/s` : "";
  const list = $("list");
  list.replaceChildren(...items.map(row));
}

function row(d) {
  const li = document.createElement("li");
  if (d.status === "Complete") li.className = "done";
  if (d.status === "Error") li.className = "error";
  if (d.gold) li.classList.add("gold");

  const name = document.createElement("div");
  name.className = "name";
  name.textContent = d.name;
  name.title = d.url;

  const meta = document.createElement("div");
  meta.className = "meta";
  const parts = [];
  if (d.status === "Error") parts.push(d.error || "Error");
  else {
    if (d.size) parts.push(`${fmtSize(d.downloaded)} / ${fmtSize(d.size)}`);
    if (d.running && d.rate) parts.push(`${fmtSize(d.rate)}/s`);
    if (d.running && d.eta != null) parts.push(`${fmtEta(d.eta)} left`);
    if (!d.running) parts.push(d.status_text);
  }
  meta.textContent = parts.join(" · ");

  const actions = document.createElement("div");
  actions.className = "actions";
  const btn = (label, title, action) => {
    const b = document.createElement("button");
    b.className = "icon";
    b.textContent = label;
    b.title = title;
    b.addEventListener("click", () => call({ cmd: "control", dl: d.id, action }).catch(() => {}));
    actions.append(b);
  };
  if (d.status === "Complete") {
    btn("▶", "Open", "open");
    btn("📁", "Show in folder", "reveal");
  } else if (d.running) {
    btn("❚❚", "Pause", "pause");
  } else {
    btn("▶", "Resume", "resume");
  }

  const bar = document.createElement("div");
  bar.className = "bar";
  const fill = document.createElement("div");
  fill.style.width = `${Math.round((d.status === "Complete" ? 1 : d.progress || 0) * 100)}%`;
  bar.append(fill);

  li.append(name, actions, meta, bar);
  return li;
}

// ---------------------------------------------------------------- page actions
async function init() {
  const s = await getSettings();
  $("enabled").checked = s.enabled;
  $("quality").value = s.defaultQuality;
  $("enabled").addEventListener("change", (e) => api.storage.local.set({ enabled: e.target.checked }));
  $("quality").addEventListener("change", (e) => api.storage.local.set({ defaultQuality: e.target.value }));
  $("options").addEventListener("click", () => api.runtime.openOptionsPage());
  $("open-app").addEventListener("click", () => {
    nativeRequest({ cmd: "show" }).catch((e) => showProblem(e));
    window.close();
  });

  [activeTab] = await api.tabs.query({ active: true, currentWindow: true });
  const url = activeTab?.url || "";
  if (/^https?:/i.test(url)) {
    $("page-host").textContent = new URL(url).hostname;
  } else {
    $("send-page").disabled = true;
    $("page-host").textContent = "Open a web page to download its video.";
  }
  $("send-page").addEventListener("click", async () => {
    $("send-page").disabled = true;
    $("page-result").textContent = "Sending…";
    const r = await api.runtime.sendMessage({ type: "send-page", url, quality: $("quality").value });
    $("send-page").disabled = false;
    if (r && r.ok) {
      $("page-result").className = "small success";
      $("page-result").textContent = r.data.pending_confirmation
        ? "Confirmez le téléchargement dans Open Transfer Manager."
        : `Added: ${r.data.download.name}`;
    } else {
      $("page-result").className = "small danger";
      $("page-result").textContent = (r && r.error) || "Failed";
    }
  });
  api.runtime.sendMessage({ type: "clear-error" }).catch(() => {});
  connect();
}

init();
