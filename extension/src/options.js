"use strict";

const $ = (id) => document.getElementById(id);
const FIELDS = ["enabled", "allTypes", "captureExts", "minSizeMB", "excludedSites", "defaultQuality"];
let savedTimer = null;

async function load() {
  const s = await getSettings();
  for (const k of FIELDS) {
    const el = $(k);
    if (el.type === "checkbox") el.checked = s[k];
    else el.value = s[k];
    el.addEventListener("change", save);
  }
  const extra = await api.storage.local.get(["videoButton", "hiddenSites"]);
  $("videoButton").checked = extra.videoButton !== false;
  $("videoButton").addEventListener("change", (e) => api.storage.local.set({ videoButton: e.target.checked }));
  const showHidden = (list) => ($("hiddenSites").textContent = list && list.length ? list.join(", ") : "none");
  showHidden(extra.hiddenSites);
  $("resetHidden").addEventListener("click", async () => {
    await api.storage.local.set({ hiddenSites: [] });
    showHidden([]);
  });
  testConnection();
  $("test").addEventListener("click", testConnection);
  checkPermissions();
}

async function save() {
  const out = {};
  for (const k of FIELDS) {
    const el = $(k);
    if (el.type === "checkbox") out[k] = el.checked;
    else if (el.type === "number") out[k] = Math.max(0, Number(el.value) || 0);
    else if (k === "captureExts") out[k] = el.value.toLowerCase().split(/[\s,;.]+/).filter(Boolean).join(" ");
    else out[k] = el.value;
  }
  await api.storage.local.set(out);
  $("saved").style.opacity = 1;
  clearTimeout(savedTimer);
  savedTimer = setTimeout(() => ($("saved").style.opacity = 0), 1500);
}

async function testConnection() {
  $("conn").textContent = "Checking…";
  $("conn").className = "";
  $("conn-hint").textContent = "";
  try {
    const d = await nativeRequest({ cmd: "ping" });
    $("conn").textContent = `Bridge OK (version ${d.host})`;
    $("conn").className = "success";
    $("conn-hint").textContent = d.app_running ? "The app is running." : "The app is not running; it will start when needed.";
  } catch (e) {
    $("conn").textContent = "Not connected";
    $("conn").className = "danger";
    $("conn-hint").textContent = e.message;
  }
}

async function checkPermissions() {
  if (!api.permissions?.contains) return;
  const has = await api.permissions.contains({ origins: ["<all_urls>"] });
  $("perm-card").hidden = has;
  $("grant").addEventListener("click", async () => {
    const ok = await api.permissions.request({ origins: ["<all_urls>"] });
    $("perm-card").hidden = ok;
  });
}

load();
