// Motion-design GIFs for the README (assets/demo/*.gif).
//   npm install @resvg/resvg-js@2.6.2
//   node scripts/make-gifs.js download frames/download   (or: upload frames/upload)
//   ffmpeg -framerate 25 -i frames/download/f%04d.png -vf "fps=20,scale=800:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=192:stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a:diff_mode=rectangle" -loop 0 assets/demo/otm-download.gif
// Each frame is an SVG rendered by resvg; ffmpeg turns the PNGs into a GIF.
const { Resvg } = require('@resvg/resvg-js');
const fs = require('fs');
const path = require('path');

const REPO = path.join(__dirname, '..');
const W = 960, H = 540, FPS = 25;
const which = process.argv[2];
const out = process.argv[3];
fs.mkdirSync(out, { recursive: true });

const b64 = (f) => 'data:image/png;base64,' + fs.readFileSync(f).toString('base64');
const LOGO = b64(REPO + '/assets/otm-64.png');
const BRAND = {
  gdrive: b64(REPO + '/assets/brands/googledrive.png'),
  dropbox: b64(REPO + '/assets/brands/dropbox.png'),
  mega: b64(REPO + '/assets/brands/mega.png'),
  box: b64(REPO + '/assets/brands/box.png'),
};

// ---------------------------------------------------------------- helpers
const clamp = (x, a = 0, b = 1) => Math.max(a, Math.min(b, x));
const seg = (t, a, b) => clamp((t - a) / (b - a));           // 0..1 over [a, b]
const easeOut = (x) => 1 - Math.pow(1 - x, 3);
const easeInOut = (x) => (x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2);
const back = (x) => { const c1 = 1.70158, c3 = c1 + 1; return 1 + c3 * Math.pow(x - 1, 3) + c1 * Math.pow(x - 1, 2); };
const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;');
const C = { bg: '#0B0D12', surf: '#161922', alt: '#1E2230', border: '#2A3040', text: '#E8EAF0', muted: '#8A93A6', accent: '#A78BFA', btn: '#7C3AED', ok: '#22C55E' };

const defs = `
<defs>
  <linearGradient id="grad" x1="0" x2="1" y1="0" y2="0"><stop offset="0" stop-color="#FF0A8C"/><stop offset="0.55" stop-color="#7C3AED"/><stop offset="1" stop-color="#00C8F0"/></linearGradient>
  <radialGradient id="glowA" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#7C3AED" stop-opacity="0.45"/><stop offset="1" stop-color="#7C3AED" stop-opacity="0"/></radialGradient>
  <radialGradient id="glowB" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#00C8F0" stop-opacity="0.30"/><stop offset="1" stop-color="#00C8F0" stop-opacity="0"/></radialGradient>
  <radialGradient id="glowC" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#FF0A8C" stop-opacity="0.25"/><stop offset="1" stop-color="#FF0A8C" stop-opacity="0"/></radialGradient>
  <filter id="shadow" x="-20%" y="-20%" width="140%" height="160%"><feDropShadow dx="0" dy="14" stdDeviation="18" flood-color="#000" flood-opacity="0.55"/></filter>
  <filter id="soft" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="6"/></filter>
</defs>`;

function background(t) {
  // slowly drifting colour glows behind everything
  const a = 0.6; // fixed: a moving background makes the GIF several times larger
  return `<rect width="${W}" height="${H}" fill="${C.bg}"/>
  <circle cx="${200 + 40 * Math.sin(a)}" cy="${120 + 30 * Math.cos(a)}" r="320" fill="url(#glowA)"/>
  <circle cx="${800 + 40 * Math.cos(a)}" cy="${430 + 25 * Math.sin(a)}" r="300" fill="url(#glowB)"/>
  <circle cx="${720 + 30 * Math.sin(a * 1.3)}" cy="${80}" r="220" fill="url(#glowC)"/>`;
}

function brandBar(title) {
  return `<image href="${LOGO}" x="40" y="26" width="34" height="34"/>
  <text x="84" y="49" font-family="Inter" font-weight="600" font-size="17" fill="${C.text}">Open Transfer Manager</text>
  <text x="${W - 40}" y="49" text-anchor="end" font-family="Inter" font-size="14" fill="${C.muted}">${esc(title)}</text>`;
}

function caption(t, steps) {
  // steps: [[from, to, text]] — cross-fading lines at the bottom
  let s = '';
  for (const [a, b, text] of steps) {
    const o = Math.min(seg(t, a, a + 0.3), 1 - seg(t, b - 0.3, b));
    if (o <= 0) continue;
    const dy = (1 - easeOut(seg(t, a, a + 0.4))) * 12;
    s += `<text x="${W / 2}" y="${506 + dy}" text-anchor="middle" font-family="Inter" font-weight="600" font-size="22" fill="${C.text}" opacity="${o.toFixed(3)}">${text}</text>`;
  }
  return s;
}

const check = (cx, cy, r, color = C.ok) =>
  `<circle cx="${cx}" cy="${cy}" r="${r}" fill="${color}"/>
   <path d="M${cx - r * 0.45} ${cy + r * 0.02} L${cx - r * 0.1} ${cy + r * 0.38} L${cx + r * 0.5} ${cy - r * 0.35}" fill="none" stroke="#fff" stroke-width="${(r * 0.22).toFixed(1)}" stroke-linecap="round" stroke-linejoin="round"/>`;

function pointer(x, y, pressed) {
  const s = pressed ? 0.9 : 1;
  return `<g transform="translate(${x} ${y}) scale(${s})" filter="url(#shadow)">
    <path d="M0 0 L0 22 L6 16.5 L10.5 26 L14 24.5 L9.6 15.3 L17 15.3 Z" fill="#fff" stroke="#111" stroke-width="1.4" stroke-linejoin="round"/></g>`;
}

function progressBar(x, y, w, h, frac, color) {
  const fw = Math.max(h, w * clamp(frac));
  return `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${h / 2}" fill="${C.alt}"/>
  ${frac > 0 ? `<rect x="${x}" y="${y}" width="${fw.toFixed(1)}" height="${h}" rx="${h / 2}" fill="${color || 'url(#grad)'}"/>` : ''}`;
}

// ---------------------------------------------------------------- GIF 1: download
function downloadFrame(t) {
  const T = 7.6;
  const fadeLoop = 1 - seg(t, T - 0.35, T); // fade out at the end for a clean loop
  const win = easeOut(seg(t, 0, 0.6));
  const wx = 100, wy = 84 + (1 - win) * 30, ww = 760, wh = 370;
  let s = background(t) + brandBar('Videos & MP3 from 1000+ sites');
  s += `<g opacity="${(win * fadeLoop).toFixed(3)}">`;
  s += `<rect x="${wx}" y="${wy}" width="${ww}" height="${wh}" rx="18" fill="${C.surf}" stroke="${C.border}" filter="url(#shadow)"/>`;
  // window title
  s += `<circle cx="${wx + 22}" cy="${wy + 22}" r="5.5" fill="#FF5F57"/><circle cx="${wx + 40}" cy="${wy + 22}" r="5.5" fill="#FEBC2E"/><circle cx="${wx + 58}" cy="${wy + 22}" r="5.5" fill="#28C840"/>`;
  s += `<text x="${wx + ww / 2}" y="${wy + 27}" text-anchor="middle" font-family="Inter" font-weight="600" font-size="13" fill="${C.muted}">New download</text>`;
  // URL field + typing
  const url = 'https://youtube.com/watch?v=d_HlPboLRL8';
  const typed = url.slice(0, Math.floor(url.length * seg(t, 0.6, 1.8)));
  const fx = wx + 28, fy = wy + 50, fw = ww - 56;
  s += `<rect x="${fx}" y="${fy}" width="${fw}" height="46" rx="11" fill="#0F1117" stroke="${t > 0.55 && t < 2 ? C.btn : C.border}" stroke-width="${t > 0.55 && t < 2 ? 2 : 1}"/>`;
  s += `<path d="M${fx + 18} ${fy + 26} l4 -4 m-1 -3 l3 -3 a3 3 0 0 1 4 4 l-3 3 m-3 1 l-3 3 a3 3 0 0 1 -4 -4" fill="none" stroke="${C.muted}" stroke-width="1.8" stroke-linecap="round"/>`;
  s += `<text x="${fx + 42}" y="${fy + 29}" font-family="Inter" font-size="15.5" fill="${typed ? C.text : C.muted}">${esc(typed || 'Paste a link…')}</text>`;
  if (t > 0.5 && t < 2.2 && Math.floor(t * 2.5) % 2 === 0) {
    const cw = typed.length * 7.95;
    s += `<rect x="${fx + 43 + cw}" y="${fy + 13}" width="1.6" height="20" fill="${C.text}"/>`;
  }
  // format chips
  const chips = [
    ['1080p', 'MP4 · 412 MB', '#EF4444'],
    ['720p', 'MP4 · 198 MB', '#EF4444'],
    ['MP3 320 kbps', 'best · 9.8 MB', '#A855F7'],
    ['MP3 128 kbps', 'small · 3.9 MB', '#A855F7'],
  ];
  const pick = 2;
  const cw = (fw - 3 * 12) / 4;
  const chosen = t > 2.95;
  chips.forEach(([a, b, col], i) => {
    const p = back(seg(t, 1.9 + i * 0.12, 2.35 + i * 0.12));
    if (p <= 0) return;
    const x = fx + i * (cw + 12), y = fy + 64 + (1 - p) * 14;
    const sel = chosen && i === pick;
    s += `<g opacity="${clamp(p).toFixed(3)}">
      <rect x="${x}" y="${y}" width="${cw}" height="62" rx="12" fill="${sel ? '#241B47' : C.alt}" stroke="${sel ? 'url(#grad)' : C.border}" stroke-width="${sel ? 2.2 : 1}"/>
      <rect x="${x + 12}" y="${y + 14}" width="34" height="34" rx="9" fill="${col}" fill-opacity="0.18"/>
      ${i >= 2 ? `<path d="M${x + 24} ${y + 39} V${y + 23} l13 -3 v14" fill="none" stroke="${col}" stroke-width="2" stroke-linejoin="round"/><circle cx="${x + 21.5}" cy="${y + 39}" r="3" fill="none" stroke="${col}" stroke-width="2"/><circle cx="${x + 34.5}" cy="${y + 34}" r="3" fill="none" stroke="${col}" stroke-width="2"/>`
               : `<rect x="${x + 19}" y="${y + 22}" width="20" height="17" rx="3" fill="none" stroke="${col}" stroke-width="2"/><path d="M${x + 26} ${y + 26.5} l6 4 -6 4z" fill="${col}"/>`}
      <text x="${x + 56}" y="${y + 28}" font-family="Inter" font-weight="600" font-size="14" fill="${C.text}">${a}</text>
      <text x="${x + 56}" y="${y + 47}" font-family="Inter" font-size="12" fill="${C.muted}">${b}</text>
    </g>`;
  });
  // Download button
  const bx = wx + ww - 28 - 150, by = fy + 146;
  const pressed = t > 3.15 && t < 3.35;
  if (t > 2.3) {
    const o = seg(t, 2.3, 2.6);
    s += `<g opacity="${o.toFixed(3)}"><rect x="${bx}" y="${by}" width="150" height="40" rx="10" fill="${chosen ? C.btn : '#3A3F50'}" ${pressed ? 'transform="translate(2 1)"' : ''}/>
      <text x="${bx + 75}" y="${by + 25.5}" text-anchor="middle" font-family="Inter" font-weight="600" font-size="14.5" fill="#fff">Download MP3</text></g>`;
  }
  // download row
  const rowIn = easeOut(seg(t, 3.35, 3.8));
  if (rowIn > 0) {
    const ry = by + 56 + (1 - rowIn) * 16;
    const frac = easeInOut(seg(t, 3.5, 5.7));
    const done = t > 5.75;
    s += `<g opacity="${rowIn.toFixed(3)}">
      <rect x="${fx}" y="${ry}" width="${fw}" height="62" rx="12" fill="#12151D" stroke="${done ? C.ok : C.border}" stroke-opacity="${done ? 0.6 : 1}"/>
      <rect x="${fx + 12}" y="${ry + 13}" width="36" height="36" rx="9" fill="#A855F7" fill-opacity="0.18"/>
      <text x="${fx + 30}" y="${ry + 46}" text-anchor="middle" font-family="Inter" font-weight="700" font-size="8.5" fill="#C084FC">MP3</text>
      <path d="M${fx + 25} ${ry + 33} V${ry + 21} l11 -2.5 v11.5" fill="none" stroke="#C084FC" stroke-width="1.8"/>
      <text x="${fx + 60}" y="${ry + 27}" font-family="Inter" font-weight="600" font-size="14.5" fill="${C.text}">AURORA - Runaway.mp3</text>`;
    if (!done) {
      const speed = (9 + 4 * Math.sin(t * 7) + 2 * Math.sin(t * 13)).toFixed(1);
      const label = frac > 0.86 ? 'Converting to MP3 · 320 kbps…' : `${Math.round(frac * 100)}% · ${speed} MB/s · 8 connections`;
      s += `<text x="${fx + 60}" y="${ry + 46}" font-family="Inter" font-size="12" fill="${C.muted}">${label}</text>`;
      s += progressBar(fx + fw - 250, ry + 27, 228, 8, frac);
    } else {
      const pop = back(seg(t, 5.75, 6.1));
      s += `<text x="${fx + 60}" y="${ry + 46}" font-family="Inter" font-size="12" fill="${C.ok}">Saved in Downloads / Music · 9.8 MB</text>`;
      s += `<g transform="translate(${fx + fw - 34} ${ry + 31}) scale(${pop.toFixed(3)})">${check(0, 0, 15)}</g>`;
      // ripple
      const rp = seg(t, 5.75, 6.5);
      if (rp < 1) s += `<circle cx="${fx + fw - 34}" cy="${ry + 31}" r="${15 + rp * 30}" fill="none" stroke="${C.ok}" stroke-width="2" opacity="${(1 - rp).toFixed(3)}"/>`;
    }
    s += `</g>`;
  }
  s += `</g>`;
  // pointer moves to the MP3 320 chip, clicks, then the Download button
  if (t > 2.4 && t < 3.6) {
    const chipX = fx + pick * (cw + 12) + cw * 0.6, chipY = fy + 64 + 40;
    const btnX = bx + 110, btnY = by + 26;
    let px, py;
    if (t < 2.95) { const k = easeInOut(seg(t, 2.4, 2.9)); px = 760 + (chipX - 760) * k; py = 470 + (chipY - 470) * k; }
    else { const k = easeInOut(seg(t, 3.0, 3.15)); px = chipX + (btnX - chipX) * k; py = chipY + (btnY - chipY) * k; }
    const click = (t > 2.88 && t < 3.0) || (t > 3.15 && t < 3.3);
    if (click) s += `<circle cx="${px}" cy="${py}" r="16" fill="#fff" opacity="0.18"/>`;
    s += pointer(px, py, click);
  }
  s += caption(t, [
    [0.2, 1.9, '1 · Paste a link from YouTube, TikTok, Vimeo…'],
    [1.9, 3.4, '2 · Pick a quality — or MP3 at 320 kbps'],
    [3.4, 5.8, '3 · Multi-connection download, converted on the fly'],
    [5.8, 7.6, 'Done — sorted into Music automatically'],
  ]);
  return s;
}

// ---------------------------------------------------------------- GIF 2: send & services
const SERVICES = [
  { key: 'gdrive', name: 'Google Drive', color: '#4285F4' },
  { key: 'onedrive', name: 'OneDrive', color: '#0078D4' },
  { key: 'dropbox', name: 'Dropbox', color: '#0061FF' },
  { key: 'mega', name: 'MEGA', color: '#D9272E' },
  { key: 'sftp', name: 'SFTP server', color: '#CBD5E1' },
  { key: 'box', name: 'Box', color: '#0061D5' },
  { key: 's3', name: 'S3 · B2 · R2', color: '#FBBF24' },
  { key: 'pcloud', name: 'pCloud', color: '#17BED0' },
];

function serviceIcon(key, cx, cy, size) {
  const svc = SERVICES.find((x) => x.key === key);
  const h = size / 2;
  let s = `<rect x="${cx - h}" y="${cy - h}" width="${size}" height="${size}" rx="${size * 0.26}" fill="${key === 'sftp' || key === 's3' ? '#232838' : '#F4F6FA'}"/>`;
  const k = size / 24; // 24-unit icon grid, centred
  const g = (d, color, w = 1.9) => `<g transform="translate(${cx - 12 * k * 0.62} ${cy - 12 * k * 0.62}) scale(${(k * 0.62).toFixed(3)})"><path d="${d}" fill="none" stroke="${color}" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round"/></g>`;
  if (BRAND[key]) {
    const im = size * 0.62;
    s += `<image href="${BRAND[key]}" x="${cx - im / 2}" y="${cy - im / 2}" width="${im}" height="${im}"/>`;
  } else if (key === 'onedrive' || key === 'pcloud') {
    s += g('M7 18 C3.5 18 2.8 13.5 6.3 11 C7 7 11 5.2 14.6 7.1 C16.5 8.2 17 9.3 17 9.3 C20.5 9.6 22 13.8 19.6 16.3 C18.7 17.4 17.6 18 16.5 18 Z', svc.color, 2.2);
    if (key === 'pcloud') s += `<text x="${cx}" y="${cy + size * 0.12}" text-anchor="middle" font-family="Inter" font-weight="600" font-size="${size * 0.26}" fill="${svc.color}">p</text>`;
  } else if (key === 'sftp') {
    s += g('M3 4 h18 v16 h-18 z M7 9 l3 3 -3 3 M12 15 h5', svc.color);
  } else if (key === 's3') {
    s += g('M4 7 L6 19 H18 L20 7 Z M4 7 C4 3 20 3 20 7', svc.color);
  }
  return s;
}

function fileCard(x, y, scale, name, sub, ext, color) {
  return `<g transform="translate(${x} ${y}) scale(${scale})" filter="url(#shadow)">
    <rect x="-110" y="-46" width="220" height="92" rx="16" fill="${C.surf}" stroke="${C.border}"/>
    <rect x="-94" y="-28" width="56" height="56" rx="13" fill="${color}" fill-opacity="0.18"/>
    <rect x="-80" y="-14" width="28" height="21" rx="4" fill="none" stroke="${color}" stroke-width="2.4"/>
    <path d="M-70 -8 l8 5 -8 5z" fill="${color}"/>
    <text x="-66" y="21" text-anchor="middle" font-family="Inter" font-weight="700" font-size="9" fill="${color}">${ext}</text>
    <text x="-26" y="-4" font-family="Inter" font-weight="600" font-size="15" fill="${C.text}">${name}</text>
    <text x="-26" y="17" font-family="Inter" font-size="12.5" fill="${C.muted}">${sub}</text>
  </g>`;
}

function curve(x1, y1, x2, y2) {
  const mx = (x1 + x2) / 2;
  return { d: `M${x1} ${y1} C${mx} ${y1} ${mx} ${y2} ${x2} ${y2}`, at: (u) => {
    // cubic Bezier point
    const p0 = [x1, y1], p1 = [mx, y1], p2 = [mx, y2], p3 = [x2, y2];
    const v = 1 - u;
    return [0, 1].map((i) => v * v * v * p0[i] + 3 * v * v * u * p1[i] + 3 * v * u * u * p2[i] + u * u * u * p3[i]);
  } };
}

function uploadFrame(t) {
  const T = 9.2;
  let s = background(t) + brandBar('Send anywhere · move between drives');
  const sceneA = 1 - seg(t, 4.9, 5.3);
  const sceneB = seg(t, 5.1, 5.5) * (1 - seg(t, T - 0.35, T));
  const introA = seg(t, 0, 0.4);
  // ---------- scene A: one file, many destinations
  if (sceneA > 0) {
    s += `<g opacity="${(sceneA * introA).toFixed(3)}">`;
    const fx = 220, fy = 270;
    const cardIn = back(seg(t, 0.1, 0.6));
    // tiles: 2 columns x 4 rows on the right
    const tiles = SERVICES.map((svc, i) => ({ ...svc, x: 640 + (i % 2) * 190, y: 118 + Math.floor(i / 2) * 92 }));
    const chosen = ['gdrive', 'sftp', 's3', 'dropbox'];
    const speeds = { gdrive: 1.0, sftp: 1.35, s3: 1.15, dropbox: 0.85 };
    // paths first (under the tiles)
    tiles.forEach((tile) => {
      if (!chosen.includes(tile.key)) return;
      const c = curve(fx + 110, fy, tile.x - 52, tile.y);
      const draw = seg(t, 1.6, 2.2);
      if (draw <= 0) return;
      s += `<path d="${c.d}" fill="none" stroke="url(#grad)" stroke-width="2.4" stroke-dasharray="6 7" opacity="${(0.85 * draw).toFixed(3)}"/>`;
      // particles travelling along the path
      const run = seg(t, 2.2, 2.2 + 2.2 / speeds[tile.key]);
      if (run > 0 && run < 1) {
        for (let k = 0; k < 4; k++) {
          const u = (t * 0.9 * speeds[tile.key] + k / 4) % 1;
          const [px, py] = c.at(u);
          s += `<circle cx="${px.toFixed(1)}" cy="${py.toFixed(1)}" r="7" fill="${tile.color}" opacity="0.35" filter="url(#soft)"/><circle cx="${px.toFixed(1)}" cy="${py.toFixed(1)}" r="3.6" fill="#fff"/>`;
        }
      }
    });
    tiles.forEach((tile, i) => {
      const p = back(seg(t, 0.35 + i * 0.07, 0.75 + i * 0.07));
      if (p <= 0) return;
      const isChosen = chosen.includes(tile.key);
      const selected = isChosen && t > 1.1 + chosen.indexOf(tile.key) * 0.12;
      const run = seg(t, 2.2, 2.2 + 2.2 / (speeds[tile.key] || 1));
      const done = isChosen && run >= 1;
      s += `<g transform="translate(${tile.x} ${tile.y}) scale(${p.toFixed(3)})">
        <rect x="-62" y="-36" width="174" height="72" rx="14" fill="${selected ? '#1B1734' : C.surf}" stroke="${selected ? (done ? C.ok : C.btn) : C.border}" stroke-width="${selected ? 1.8 : 1}" opacity="${isChosen || t < 1.1 ? 1 : 0.55}"/>
        ${serviceIcon(tile.key, -26, 0, 44)}
        <text x="6" y="${selected && !done ? -2 : 5}" font-family="Inter" font-weight="600" font-size="13.5" fill="${C.text}">${tile.name}</text>`;
      if (selected && !done && run > 0) s += progressBar(6, 9, 90, 6, easeInOut(run));
      if (selected && !done && run <= 0) s += `<text x="6" y="17" font-family="Inter" font-size="11.5" fill="${C.muted}">waiting…</text>`;
      if (done) {
        const pop = back(seg(t, 2.2 + 2.2 / speeds[tile.key], 2.5 + 2.2 / speeds[tile.key]));
        s += `<g transform="translate(96 -22) scale(${pop.toFixed(3)})">${check(0, 0, 10)}</g>`;
        s += `<text x="6" y="22" font-family="Inter" font-size="11.5" fill="${C.ok}">sent · link copied</text>`;
      }
      s += `</g>`;
    });
    s += fileCard(fx, fy, cardIn, 'demo-video.mp4', '248 MB', 'MP4', '#EF4444');
    // drop hint at the start: the card falls in with a dashed zone
    const dz = 1 - seg(t, 0.6, 1.0);
    if (dz > 0) s += `<rect x="${fx - 130}" y="${fy - 66}" width="260" height="132" rx="20" fill="none" stroke="#A78BFA" stroke-width="2" stroke-dasharray="8 6" opacity="${dz.toFixed(3)}"/>`;
    s += `</g>`;
  }
  // ---------- scene B: drive to drive
  if (sceneB > 0) {
    s += `<g opacity="${sceneB.toFixed(3)}">`;
    const L = { x: 250, y: 250 }, R = { x: 710, y: 250 };
    const tIn = back(seg(t, 5.2, 5.7));
    for (const [pos, key, name, folder] of [[L, 'gdrive', 'Google Drive', 'My Drive / Projects'], [R, 'onedrive', 'OneDrive', 'Backups']]) {
      s += `<g transform="translate(${pos.x} ${pos.y}) scale(${tIn.toFixed(3)})" filter="url(#shadow)">
        <rect x="-120" y="-96" width="240" height="192" rx="22" fill="${C.surf}" stroke="${C.border}"/>
        ${serviceIcon(key, 0, -30, 76)}
        <text x="0" y="40" text-anchor="middle" font-family="Inter" font-weight="600" font-size="18" fill="${C.text}">${name}</text>
        <text x="0" y="64" text-anchor="middle" font-family="Inter" font-size="13" fill="${C.muted}">${folder}</text>
      </g>`;
    }
    // arc between them, with the folder travelling along it
    const arc = `M${L.x + 125} ${L.y - 20} Q${(L.x + R.x) / 2} ${L.y - 150} ${R.x - 125} ${R.y - 20}`;
    const draw = seg(t, 5.6, 6.0);
    s += `<path d="${arc}" fill="none" stroke="url(#grad)" stroke-width="2.6" stroke-dasharray="7 7" opacity="${(0.9 * draw).toFixed(3)}"/>`;
    const mv = easeInOut(seg(t, 6.0, 8.0));
    const qx = (u) => (1 - u) * (1 - u) * (L.x + 125) + 2 * (1 - u) * u * ((L.x + R.x) / 2) + u * u * (R.x - 125);
    const qy = (u) => (1 - u) * (1 - u) * (L.y - 20) + 2 * (1 - u) * u * (L.y - 150) + u * u * (R.y - 20);
    if (t > 5.9 && t < 8.15) {
      const x = qx(mv), y = qy(mv);
      s += `<g transform="translate(${x.toFixed(1)} ${y.toFixed(1)})" filter="url(#shadow)">
        <rect x="-62" y="-22" width="124" height="44" rx="12" fill="#1B1734" stroke="#7C3AED"/>
        <path d="M-48 -8 h9 l3 3 h14 v14 h-26 z" fill="#FBBF24" fill-opacity="0.25" stroke="#FBBF24" stroke-width="1.8" stroke-linejoin="round"/>
        <text x="-14" y="5" font-family="Inter" font-weight="600" font-size="13" fill="${C.text}">Releases</text>
      </g>`;
    }
    // progress + done
    s += progressBar(330, 410, 300, 8, easeInOut(seg(t, 6.0, 8.0)));
    const done = t > 8.05;
    s += `<text x="480" y="440" text-anchor="middle" font-family="Inter" font-size="13" fill="${done ? C.ok : C.muted}">${done ? '3.4 GB copied · nothing written to this computer' : 'Streaming 3.4 GB · ' + (10 + 3 * Math.sin(t * 6)).toFixed(1) + ' MB/s'}</text>`;
    if (done) {
      const pop = back(seg(t, 8.05, 8.4));
      s += `<g transform="translate(${R.x + 98} ${R.y - 80}) scale(${pop.toFixed(3)})">${check(0, 0, 16)}</g>`;
    }
    s += `</g>`;
  }
  s += caption(t, [
    [0.2, 1.6, 'Drop a file once…'],
    [1.6, 4.9, '…send it to Google Drive, Dropbox, SFTP and S3 at the same time'],
    [5.2, 9.2, 'Move files between drives — Google Drive → OneDrive in one click'],
  ]);
  return s;
}

// ---------------------------------------------------------------- render
const frameFn = which === 'download' ? downloadFrame : uploadFrame;
const duration = which === 'download' ? 7.6 : 9.2;
const n = Math.round(duration * FPS);
const opts = {
  font: {
    fontFiles: [REPO + '/assets/fonts/Inter-Regular.ttf', REPO + '/assets/fonts/Inter-SemiBold.ttf'],
    loadSystemFonts: false,
    defaultFontFamily: 'Inter',
  },
  fitTo: { mode: 'width', value: W },
};
const only = process.argv[4] ? Number(process.argv[4]) : null; // render one frame (seconds) for checks
for (let i = 0; i < n; i++) {
  const t = only !== null ? only : i / FPS;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">${defs}${frameFn(t)}</svg>`;
  const png = new Resvg(svg, opts).render().asPng();
  fs.writeFileSync(path.join(out, only !== null ? 'check.png' : `f${String(i).padStart(4, '0')}.png`), png);
  if (only !== null) break;
}
console.log(which, only !== null ? 'check frame' : n + ' frames');
