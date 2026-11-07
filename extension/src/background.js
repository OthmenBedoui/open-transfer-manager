/* Background: captures downloads, adds context menus, talks to the app. */
"use strict";

if (typeof importScripts === "function") importScripts("common.js"); // Chromium service worker

let settings = { ...DEFAULTS };
const ready = getSettings().then((s) => { settings = s; updateBadge(); });
const letBrowserHandle = new Set(); // URLs we hand back to the browser (fallback)

api.storage.onChanged.addListener(async () => {
  settings = await getSettings();
  updateBadge();
});

// ---------------------------------------------------------------- badge
function updateBadge(error) {
  const action = api.action;
  if (error) {
    action.setBadgeText({ text: "!" });
    action.setBadgeBackgroundColor({ color: "#dc2626" });
    action.setTitle({ title: "Open Transfer Manager: " + error });
    return;
  }
  action.setBadgeText({ text: settings.enabled ? "" : "OFF" });
  action.setBadgeBackgroundColor({ color: "#64748b" });
  action.setTitle({ title: settings.enabled ? "Open Transfer Manager: capturing downloads" : "Open Transfer Manager: capture is off" });
}

// ---------------------------------------------------------------- helpers
function extOf(name) {
  const clean = (name || "").split(/[?#]/)[0];
  const base = clean.split(/[\\/]/).pop() || "";
  const i = base.lastIndexOf(".");
  return i > 0 ? base.slice(i + 1).toLowerCase() : "";
}

function baseName(path) {
  return (path || "").split(/[\\/]/).pop() || "";
}

function hostOf(url) {
  try { return new URL(url).hostname.toLowerCase(); } catch { return ""; }
}

function isExcluded(url) {
  const host = hostOf(url);
  return settings.excludedSites.split(/\s+/).filter(Boolean)
    .some((h) => host === h.toLowerCase() || host.endsWith("." + h.toLowerCase()));
}

function shouldCapture(item) {
  if (!settings.enabled) return false;
  const url = item.finalUrl || item.url || "";
  if (!/^https?:/i.test(url)) return false;         // blob:, data:, file: stay in the browser
  if (item.byExtensionId) return false;              // started by an extension
  if (isExcluded(url) || isExcluded(item.referrer || "")) return false;
  const size = item.fileSize > 0 ? item.fileSize : item.totalBytes;
  if (settings.minSizeMB > 0 && size > 0 && size < settings.minSizeMB * 1024 * 1024) return false;
  if (settings.allTypes) return true;
  const ext = extOf(item.filename) || extOf(url);
  return settings.captureExts.split(/\s+/).includes(ext);
}

async function cookieHeader(url) {
  try {
    const list = await api.cookies.getAll({ url });
    return list.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch {
    return ""; // permission not granted (Firefox) or not available
  }
}

async function sendToApp(url, extra = {}) {
  const body = {
    cmd: "add",
    url,
    referer: extra.referer || "",
    filename: extra.filename || "",
    user_agent: navigator.userAgent,
    start: true,
    confirm: true,
  };
  if (extra.media !== undefined) body.media = extra.media;
  if (extra.quality) body.quality = extra.quality;
  if (extra.withCookies !== false) body.cookies = await cookieHeader(url);
  const data = await nativeRequest(body);
  updateBadge();
  return data;
}

// ---------------------------------------------------------------- download capture
api.downloads.onCreated.addListener(async (item) => {
  await ready;
  const url = item.finalUrl || item.url;
  if (letBrowserHandle.has(url)) { letBrowserHandle.delete(url); return; }
  if (!shouldCapture(item)) return;
  try { await api.downloads.cancel(item.id); } catch { /* already finished */ }
  try { await api.downloads.erase({ id: item.id }); } catch { /* ignore */ }
  try {
    await sendToApp(url, { referer: item.referrer, filename: baseName(item.filename), media: false });
  } catch (e) {
    // The app could not take it: give the download back to the browser so nothing is lost.
    updateBadge(e.message);
    letBrowserHandle.add(url);
    try { await api.downloads.download({ url }); } catch { /* ignore */ }
  }
});

// ---------------------------------------------------------------- context menus
function createMenus() {
  api.contextMenus.removeAll(() => {
    api.contextMenus.create({ id: "otm-link", title: "Download with Open Transfer Manager", contexts: ["link", "image", "video", "audio"] });
    api.contextMenus.create({ id: "otm-page-video", title: "Download video from this page", contexts: ["page", "video", "frame"] });
    api.contextMenus.create({ id: "otm-page-audio", title: "Download audio from this page", contexts: ["page", "video", "audio", "frame"] });
  });
}
api.runtime.onInstalled.addListener(createMenus);
api.runtime.onStartup?.addListener(createMenus);

api.contextMenus.onClicked.addListener(async (info, tab) => {
  await ready;
  try {
    if (info.menuItemId === "otm-link") {
      let url = info.linkUrl || info.srcUrl;
      let media;
      if (!url || !/^https?:/i.test(url)) { url = info.pageUrl; media = true; } // blob: video -> page
      await sendToApp(url, { referer: info.pageUrl || tab?.url || "", media });
    } else if (info.menuItemId === "otm-page-video" || info.menuItemId === "otm-page-audio") {
      const url = info.frameUrl && /^https?:/i.test(info.frameUrl) && info.frameUrl !== info.pageUrl
        ? info.frameUrl : (info.pageUrl || tab?.url);
      await sendToApp(url, {
        media: true,
        quality: info.menuItemId === "otm-page-audio" ? "audio" : settings.defaultQuality,
        withCookies: false,
      });
    }
  } catch (e) {
    updateBadge(e.message);
  }
});

// ---------------------------------------------------------------- messages from the popup
api.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg && msg.type === "send-page") {
    sendToApp(msg.url, { media: true, quality: msg.quality, withCookies: false })
      .then((d) => sendResponse({ ok: true, data: d }))
      .catch((e) => sendResponse({ ok: false, error: e.message }));
    return true; // async answer
  }
  if (msg && msg.type === "formats") {
    // list of formats for the Download button over videos (asks yt-dlp through the app)
    nativeRequest({ cmd: "formats", url: msg.url })
      .then((d) => sendResponse({ ok: true, data: d }))
      .catch((e) => sendResponse({ ok: false, error: e.message }));
    return true;
  }
  if (msg && msg.type === "download-choice") {
    nativeRequest({ cmd: "add", url: msg.url, media: true, opts: msg.opts, start: true, confirm: true })
      .then((d) => { updateBadge(); sendResponse({ ok: true, data: d }); })
      .catch((e) => { updateBadge(e.message); sendResponse({ ok: false, error: e.message }); });
    return true;
  }
  if (msg && msg.type === "show-app") {
    nativeRequest({ cmd: "show" }).catch(() => {});
    return false;
  }
  if (msg && msg.type === "clear-error") updateBadge();
  return false;
});
