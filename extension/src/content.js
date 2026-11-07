/* Download button over web videos: hover a video, click "Download", pick a format (video or MP3). */
"use strict";

(() => {
  const api = globalThis.browser ?? globalThis.chrome;
  if (window.__otmVideoButton) return;
  window.__otmVideoButton = true;

  const MIN_W = 220, MIN_H = 124;
  let host = null, root = null, current = null, menuOpen = false, hideTimer = null;
  let settings = { videoButton: true, hiddenSites: [] };

  api.storage.local.get(["videoButton", "hiddenSites"]).then((s) => {
    settings = { videoButton: s.videoButton !== false, hiddenSites: s.hiddenSites || [] };
  });
  api.storage.onChanged.addListener((ch) => {
    if (ch.videoButton) settings.videoButton = ch.videoButton.newValue !== false;
    if (ch.hiddenSites) settings.hiddenSites = ch.hiddenSites.newValue || [];
    if (!enabled()) hide(true);
  });

  const enabled = () => settings.videoButton && !settings.hiddenSites.includes(location.hostname);

  const fmtSize = (n) => {
    if (n == null) return "";
    const u = ["B", "KB", "MB", "GB"]; let i = 0; let v = n;
    while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
    return "≈ " + (i ? v.toFixed(1) : v) + " " + u[i];
  };
  const fmtDur = (s) => s ? `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, "0")}` : "";

  // ---------------------------------------------------------------- UI (shadow DOM: page CSS cannot break it)
  function build() {
    host = document.createElement("div");
    host.style.cssText = "position:fixed;z-index:2147483646;top:0;left:0;display:none;";
    root = host.attachShadow({ mode: "closed" });
    root.innerHTML = `
      <style>
        :host { all: initial; }
        * { box-sizing: border-box; font-family: Inter, system-ui, -apple-system, "Segoe UI", Ubuntu, sans-serif; }
        .btn { display:flex; align-items:center; gap:8px; height:34px; padding:0 10px 0 6px; border-radius:17px;
               border:1px solid rgba(255,255,255,.14); background:rgba(15,17,23,.86); color:#fff; font-size:13px;
               font-weight:600; cursor:pointer; box-shadow:0 4px 14px rgba(0,0,0,.35); }
        .btn:hover { background:rgba(124,58,237,.92); }
        .logo { width:22px; height:22px; }
        .menu { position:absolute; top:42px; right:0; width:340px; max-height:430px; overflow:auto; border-radius:12px;
                border:1px solid #262B38; background:#161922; color:#E8EAF0; box-shadow:0 18px 40px rgba(0,0,0,.5);
                padding:8px; display:none; }
        .menu.open { display:block; }
        .head { padding:8px 10px 10px; border-bottom:1px solid #262B38; margin-bottom:4px; }
        .title { font-weight:600; font-size:13px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
        .sub { font-size:12px; color:#8A93A6; margin-top:2px; }
        .group { font-size:11px; font-weight:600; letter-spacing:.06em; color:#8A93A6; padding:8px 10px 4px; }
        .item { display:flex; align-items:center; gap:10px; width:100%; padding:7px 10px; border:0; border-radius:8px;
                background:transparent; color:#E8EAF0; text-align:left; cursor:pointer; font-size:13px; }
        .item:hover, .item:focus-visible { background:#241B47; outline:none; }
        .item.gold { background:rgba(228,190,85,.08); border:1px solid rgba(228,190,85,.38); }
        .item.gold .label, .item.gold .badge.audio { color:#E4BE55; }
        .item.gold .badge.audio { background:rgba(228,190,85,.14); }
        .item.gold:hover, .item.gold:focus-visible { background:rgba(228,190,85,.18); outline:2px solid #E4BE55; }
        .badge { flex:none; width:34px; height:34px; border-radius:9px; display:flex; flex-direction:column; align-items:center;
                 justify-content:center; gap:1px; font-size:8.5px; font-weight:700; }
        .badge.video { background:rgba(239,68,68,.16); color:#F87171; }
        .badge.audio { background:rgba(168,85,247,.16); color:#C084FC; }
        .grow { flex:1; min-width:0; }
        .label { font-weight:500; }
        .detail { font-size:12px; color:#8A93A6; }
        .size { font-size:12px; color:#C9CEDA; font-variant-numeric:tabular-nums; }
        .foot { display:flex; justify-content:space-between; padding:8px 10px 4px; margin-top:4px; border-top:1px solid #262B38; }
        .link { border:0; background:transparent; color:#8A93A6; font-size:12px; cursor:pointer; padding:0; }
        .link.accent { color:#A78BFA; }
        .msg { padding:16px 10px; font-size:12.5px; color:#C9CEDA; }
        .err { color:#F87171; }
        .toast { position:absolute; top:42px; right:0; white-space:nowrap; padding:10px 14px; border-radius:10px;
                 background:#161922; border:1px solid #262B38; color:#E8EAF0; font-size:13px; box-shadow:0 10px 30px rgba(0,0,0,.45); display:none; }
        .toast.show { display:block; }
      </style>
      <div style="position:relative">
        <button class="btn" part="button" aria-haspopup="menu" aria-label="Download this video with Open Transfer Manager">
          <img class="logo" alt="" src="${api.runtime.getURL("icons/32.png")}">Download
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 9l6 6 6-6"/></svg>
        </button>
        <div class="menu" role="menu" aria-label="Choose a format"></div>
        <div class="toast" role="status"></div>
      </div>`;
    root.querySelector(".btn").addEventListener("click", (e) => { e.stopPropagation(); e.preventDefault(); toggleMenu(); });
    host.addEventListener("mouseenter", () => clearTimeout(hideTimer));
    host.addEventListener("mouseleave", () => scheduleHide());
    (document.body || document.documentElement).appendChild(host);
  }

  function place() {
    if (!current || !host) return;
    const r = current.getBoundingClientRect();
    if (r.width < MIN_W || r.height < MIN_H || r.bottom < 0 || r.top > innerHeight) { hide(true); return; }
    host.style.top = Math.max(4, r.top + 10) + "px";
    host.style.left = Math.min(innerWidth - 130, r.right - 130) + "px";
  }

  function show(video) {
    if (!enabled()) return;
    if (!host) build();
    current = video;
    clearTimeout(hideTimer);
    host.style.display = "block";
    place();
  }

  function scheduleHide() {
    if (menuOpen) return;
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => hide(false), 1200);
  }

  function hide(force) {
    if (!host || (menuOpen && !force)) return;
    host.style.display = "none";
    closeMenu();
  }

  function closeMenu() {
    menuOpen = false;
    root?.querySelector(".menu").classList.remove("open");
  }

  // ---------------------------------------------------------------- menu
  async function toggleMenu() {
    const menu = root.querySelector(".menu");
    if (menuOpen) return closeMenu();
    menuOpen = true;
    menu.classList.add("open");
    menu.innerHTML = `<div class="msg">Looking for formats…</div>`;
    const url = location.href;
    let r;
    try {
      r = await api.runtime.sendMessage({ type: "formats", url });
    } catch (e) {
      r = { ok: false, error: String(e) };
    }
    if (!menuOpen) return;
    if (!r || !r.ok) {
      menu.innerHTML = `<div class="msg err"></div>`;
      menu.querySelector(".msg").textContent = (r && r.error) || "Open Transfer Manager did not answer.";
      return;
    }
    render(menu, r.data, url);
  }

  function render(menu, info, url) {
    menu.innerHTML = "";
    const head = document.createElement("div");
    head.className = "head";
    const t = document.createElement("div"); t.className = "title"; t.textContent = info.title || document.title;
    const choices = info.choices || [];
    const nv = choices.filter((c) => c.group === "video").length, na = choices.length - nv;
    const s = document.createElement("div"); s.className = "sub";
    s.textContent = [fmtDur(info.duration), `${nv} video`, `${na} audio`].filter(Boolean).join(" · ");
    head.append(t, s);
    menu.append(head);
    for (const [group, title] of [["video", "VIDEO"], ["audio", "AUDIO ONLY · CONVERTED BY THE APP"]]) {
      const items = choices.filter((c) => c.group === group);
      if (!items.length) continue;
      const g = document.createElement("div"); g.className = "group"; g.textContent = title;
      menu.append(g);
      for (const c of items) {
        const b = document.createElement("button");
        b.className = c.gold ? "item gold" : "item"; b.setAttribute("role", "menuitem");
        b.innerHTML = `<span class="badge ${group}">${group === "video"
          ? '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="M10 9.5v5l4.5-2.5z" fill="currentColor"/></svg>'
          : '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" aria-hidden="true"><path d="M9 17V6l11-2v11"/><circle cx="7" cy="17" r="2.2"/><circle cx="18" cy="15" r="2.2"/></svg>'
        }<span class="ext"></span></span><span class="grow"><div class="label"></div><div class="detail"></div></span><span class="size"></span>`;
        b.querySelector(".ext").textContent = c.ext;
        b.querySelector(".label").textContent = c.label;
        b.querySelector(".detail").textContent = c.detail;
        b.querySelector(".size").textContent = fmtSize(c.size);
        b.addEventListener("click", (e) => { e.stopPropagation(); choose(url, c, info.title); });
        menu.append(b);
      }
    }
    if (!choices.length) {
      const m = document.createElement("div"); m.className = "msg"; m.textContent = "No downloadable format found on this page.";
      menu.append(m);
    }
    const foot = document.createElement("div"); foot.className = "foot";
    const hideBtn = document.createElement("button"); hideBtn.className = "link"; hideBtn.textContent = "Hide on this site";
    hideBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const list = [...new Set([...(settings.hiddenSites || []), location.hostname])];
      await api.storage.local.set({ hiddenSites: list });
      hide(true);
    });
    const app = document.createElement("button"); app.className = "link accent"; app.textContent = "More options in the app";
    app.addEventListener("click", (e) => { e.stopPropagation(); api.runtime.sendMessage({ type: "show-app" }); closeMenu(); });
    foot.append(hideBtn, app);
    menu.append(foot);
  }

  async function choose(url, choice, title) {
    closeMenu();
    const toast = root.querySelector(".toast");
    toast.textContent = "Sending…";
    toast.classList.add("show");
    let r;
    try {
      r = await api.runtime.sendMessage({ type: "download-choice", url, opts: choice.opts, title });
    } catch (e) {
      r = { ok: false, error: String(e) };
    }
    toast.textContent = r && r.ok ? (r.data?.pending_confirmation
      ? "Confirmez le téléchargement dans Open Transfer Manager."
      : `Added to Open Transfer Manager · ${choice.ext} ${choice.label.replace(/^MP3 · /, "")}`) : (r && r.error) || "Failed";
    setTimeout(() => { toast.classList.remove("show"); scheduleHide(); }, 3500);
  }

  // ---------------------------------------------------------------- find videos
  document.addEventListener("mouseover", (e) => {
    if (!enabled()) return;
    const t = e.target;
    const v = t instanceof HTMLVideoElement ? t : findVideoUnder(e.clientX, e.clientY);
    if (v) {
      const r = v.getBoundingClientRect();
      if (r.width >= MIN_W && r.height >= MIN_H) show(v);
    } else if (current && !menuOpen) {
      scheduleHide();
    }
  }, { passive: true, capture: true });

  // players often cover the <video> with overlay divs: look for a video under the pointer
  function findVideoUnder(x, y) {
    for (const v of document.querySelectorAll("video")) {
      const r = v.getBoundingClientRect();
      if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return v;
    }
    return null;
  }

  addEventListener("scroll", place, { passive: true, capture: true });
  addEventListener("resize", place, { passive: true });
  document.addEventListener("click", () => { if (menuOpen) { closeMenu(); scheduleHide(); } });
  document.addEventListener("fullscreenchange", () => hide(true));
})();
