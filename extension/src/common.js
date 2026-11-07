/* Shared by the background, popup and options pages. */
"use strict";

const api = globalThis.browser ?? globalThis.chrome;
const HOST = "org.opentransfermanager.host";

const DEFAULTS = {
  enabled: true,            // capture browser downloads
  allTypes: false,          // capture every file type, not only the list below
  captureExts:
    "zip rar 7z tar gz tgz bz2 xz zst iso img dmg pkg exe msi deb rpm appimage apk " +
    "mp4 mkv webm avi mov wmv flv m4v mpg mpeg ts mp3 flac wav m4a aac ogg opus " +
    "pdf epub doc docx xls xlsx ppt pptx odt bin",
  minSizeMB: 0,             // only capture files at least this big (when the size is known)
  excludedSites: "",        // one host per line, downloads from them stay in the browser
  defaultQuality: "best",   // video quality for "download video"
};

async function getSettings() {
  const stored = await api.storage.local.get(Object.keys(DEFAULTS));
  return { ...DEFAULTS, ...stored };
}

/** One-shot request to the native host. Resolves with `data`, rejects with a readable Error. */
async function nativeRequest(msg) {
  let reply;
  try {
    reply = await api.runtime.sendNativeMessage(HOST, msg);
  } catch (e) {
    throw bridgeError(e);
  }
  if (!reply) throw new Error("No answer from Open Transfer Manager");
  if (!reply.ok) throw new Error(reply.error || "Request failed");
  return reply.data;
}

function bridgeError(e) {
  const text = String((e && e.message) || e || "");
  if (/not found|not exist|no such native|specified native messaging host/i.test(text)) {
    const err = new Error("Open Transfer Manager is not installed, or the browser bridge is not registered. Run: otm register-browsers");
    err.code = "NO_HOST";
    return err;
  }
  if (/forbidden|access to the specified/i.test(text)) {
    const err = new Error("The bridge does not allow this extension ID. Run: otm register-browsers (and update the app)");
    err.code = "FORBIDDEN";
    return err;
  }
  return new Error(text || "Could not reach Open Transfer Manager");
}

function fmtSize(n) {
  if (n == null) return "";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n;
  for (const u of units) {
    v /= 1024;
    if (v < 1024 || u === "TB") return `${v.toFixed(1)} ${u}`;
  }
  return "";
}

function fmtEta(s) {
  if (s == null) return "";
  s = Math.round(s);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(s / 3600)}h ${String(Math.floor((s % 3600) / 60)).padStart(2, "0")}m`;
}

if (typeof module !== "undefined") module.exports = { DEFAULTS, fmtSize, fmtEta };
