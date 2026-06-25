/* the game MP-stats owner-colored map viewer  --  offline, vendored, no CDN.
 *
 * Data flow (see README.md):
 *   province_ids.png  -> decode each pixel (id = R + G*256 + B*65536) ONCE
 *                        into a Uint32Array id-raster.
 *   province_to_state -> Int32Array stateByProvince[maxId+1]  (-1 = none).
 *   snapshot {state:{owner,controller}} + country_colors -> per-recolor
 *     packed color tables -> a single pixel loop writes the display ImageData.
 *
 * The recolor function is snapshot-driven so a future timeline scrubber can
 * feed it dated snapshots (see TODO: SCRUBBER below). Today there is exactly
 * one snapshot (the current melted save).
 */
'use strict';

// ---- asset locations (root-relative; server roots at the repo) -------------
const ASSETS = {
  // half-res by default (Codex pass-1: 2.9M px display+hit-test; full-res only
  // if it loads cleanly, else we fall back). See chooseRaster().
  pngHalf: '/map_pipeline/assets/province_ids_half.png',
  pngFull: '/map_pipeline/assets/province_ids.png',
  meta: '/map_pipeline/assets/province_ids_meta.json',
  provinceToState: '/map_pipeline/assets/province_to_state.json',
  stateMeta: '/map_pipeline/assets/state_meta.json',
  countryColors: '/map_pipeline/assets/country_colors.json',
  ownership: '/save_oracle/output/state_ownership.json',
  saveMeta: '/save_oracle/output/meta.json',
};

const WATER = [38, 50, 66];        // neutral sea/lake/unassigned
const NO_OWNER = [120, 120, 120];  // land province whose state has no owner
const UNKNOWN_PROV = [70, 60, 60]; // province id present in raster, no state

// ---- module state ----------------------------------------------------------
const M = {
  width: 0, height: 0,
  idRaster: null,            // Uint32Array W*H : province id per pixel
  stateByProvince: null,     // Int32Array (maxId+1) : state id, -1 none
  provinceToState: null,     // raw {prov:state}
  stateMeta: null,           // {state:{...}}
  colors: null,              // {TAG:[r,g,b]}
  packedColor: null,         // {TAG:uint32 0xAABBGGRR}
  snapshot: null,            // {state:{owner,controller}}  (the scrubber input)
  saveMeta: null,
  mode: 'owner',             // 'owner' | 'control'
  // per-recolor packed lookups, sized maxId+1, indexed by province id:
  primaryByProv: null,       // Uint32 fill color (owner color, or controller in control mode)
  altByProv: null,           // Uint32 alt stripe color for contested (control mode)
  contestedByProv: null,     // Uint8 1 = contested (controller!=owner)
  displayCanvas: null, displayCtx: null, displayImage: null,
  selectedProv: null,        // province id last clicked (sticky info panel)
  rafPending: false,
};

// little helpers
const $ = (id) => document.getElementById(id);
function setStatus(msg) { const el = $('status'); if (el) el.textContent = msg; }
// pack [r,g,b] into 0xAABBGGRR (canvas RGBA little-endian Uint32)
function pack(rgb) { return (0xff << 24 | rgb[2] << 16 | rgb[1] << 8 | rgb[0]) >>> 0; }
function clamp8(v) { return v < 0 ? 0 : v > 255 ? 255 : v | 0; }
// darken a color (used for the contested stripe so it reads as "owner under")
function darken(rgb, f) { return [clamp8(rgb[0] * f), clamp8(rgb[1] * f), clamp8(rgb[2] * f)]; }

async function fetchJSON(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error('fetch failed ' + url + ' (' + r.status + ')');
  return r.json();
}

function loadImage(url) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error('image load failed: ' + url));
    img.src = url;
  });
}

// Decode a province-id PNG into a Uint32Array of province ids (ONCE).
// Returns {width,height,idRaster} or throws (e.g. canvas too large).
async function decodeRaster(url) {
  const img = await loadImage(url);
  const w = img.naturalWidth, h = img.naturalHeight;
  const c = document.createElement('canvas');
  c.width = w; c.height = h;
  const ctx = c.getContext('2d', { willReadFrequently: true });
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(img, 0, 0);
  // may throw / return blank if the browser refuses a canvas this large
  const src = ctx.getImageData(0, 0, w, h);
  const d = src.data;            // Uint8ClampedArray RGBA
  const n = w * h;
  const ids = new Uint32Array(n);
  for (let i = 0, p = 0; i < n; i++, p += 4) {
    // id = R + G*256 + B*65536  (per province_ids_meta.json)
    ids[i] = d[p] | (d[p + 1] << 8) | (d[p + 2] << 16);
  }
  return { width: w, height: h, idRaster: ids };
}

// Pick the display/hit-test raster: prefer half-res; fall back to full only if
// half is unavailable (it always should be). Full-res is heavy; we keep one
// raster for both display and hit-testing (Codex pass-1 rec D/E).
async function chooseRaster() {
  try {
    setStatus('Decoding province raster (half-res)...');
    return await decodeRaster(ASSETS.pngHalf);
  } catch (e) {
    console.warn('half-res decode failed, trying full-res', e);
    setStatus('Half-res failed; decoding full-res raster...');
    return await decodeRaster(ASSETS.pngFull);
  }
}

// Build Int32Array stateByProvince indexed by province id (-1 = none). id 0
// (sea/unassigned sentinel) stays -1.  (Codex pass-1 rec B.)
function buildStateByProvince(p2s, maxId) {
  const arr = new Int32Array(maxId + 1).fill(-1);
  for (const provStr in p2s) {
    const prov = +provStr;
    if (prov >= 0 && prov <= maxId) arr[prov] = p2s[provStr] | 0;
  }
  return arr;
}

// Build per-recolor packed color lookups indexed BY PROVINCE ID, so the hot
// pixel loop is a single array read (Codex pass-1 correction). Rebuilt each
// time mode or snapshot changes.
function buildColorTables() {
  const maxId = M.stateByProvince.length - 1;
  const primary = new Uint32Array(maxId + 1);
  const alt = new Uint32Array(maxId + 1);
  const contested = new Uint8Array(maxId + 1);
  const waterPacked = pack(WATER);
  const unknownPacked = pack(UNKNOWN_PROV);
  const noOwnerPacked = pack(NO_OWNER);

  for (let prov = 0; prov <= maxId; prov++) {
    if (prov === 0) { primary[prov] = waterPacked; alt[prov] = waterPacked; continue; }
    const st = M.stateByProvince[prov];
    if (st < 0) {
      // Province id present in raster but not in province_to_state == a sea/lake
      // province (the id-PNG encodes sea zones with their own real ids; id 0
      // never appears). These paint as neutral water.
      primary[prov] = waterPacked; alt[prov] = waterPacked; continue;
    }
    const own = M.snapshot[st];
    if (!own || !own.owner) {            // state with no owner (none in current data)
      primary[prov] = noOwnerPacked; alt[prov] = noOwnerPacked; continue;
    }
    const ownerTag = own.owner;
    const ctrlTag = own.controller || ownerTag;
    const ownerC = M.packedColor[ownerTag] || unknownPacked;
    const ctrlC = M.packedColor[ctrlTag] || unknownPacked;
    if (M.mode === 'control') {
      primary[prov] = ctrlC;             // base = controller color
      if (ctrlTag !== ownerTag) {        // contested: stripe in a darkened owner color
        contested[prov] = 1;
        const ownerRgb = M.colors[ownerTag] || UNKNOWN_PROV;
        alt[prov] = pack(darken(ownerRgb, 0.55));
      } else {
        alt[prov] = ctrlC;
      }
    } else {                              // owner mode
      primary[prov] = ownerC;
      alt[prov] = ownerC;
    }
  }
  M.primaryByProv = primary;
  M.altByProv = alt;
  M.contestedByProv = contested;
}

// The RECOLOR pass: iterate the id-raster once, write display ImageData.
// In control mode, contested provinces get a diagonal hatch derived purely
// from pixel coords ( ((x+y)&7)<4 ) and the snapshot colors -- no per-snapshot
// caching, so it stays scrubber-friendly (Codex pass-1 rec C).
function recolor() {
  buildColorTables();
  const { width: w, height: h, idRaster } = M;
  const out = M.displayImage.data;        // Uint8ClampedArray
  const out32 = new Uint32Array(out.buffer); // view as packed pixels
  const primary = M.primaryByProv, alt = M.altByProv, contested = M.contestedByProv;
  const controlMode = (M.mode === 'control');
  const n = w * h;
  if (controlMode) {
    for (let i = 0; i < n; i++) {
      const prov = idRaster[i];
      if (contested[prov]) {
        const x = i % w, y = (i / w) | 0;
        out32[i] = (((x + y) & 7) < 4) ? primary[prov] : alt[prov];
      } else {
        out32[i] = primary[prov];
      }
    }
  } else {
    for (let i = 0; i < n; i++) out32[i] = primary[idRaster[i]];
  }
  M.displayCtx.putImageData(M.displayImage, 0, 0);
}

// Coalesce recolor calls to one per animation frame (scrubber-friendly).
function scheduleRecolor() {
  if (M.rafPending) return;
  M.rafPending = true;
  requestAnimationFrame(() => { M.rafPending = false; recolor(); });
}

// Single entry point after a snapshot/mode change: repaint, refresh legend, and
// re-render the sticky info panel for the currently selected province so its
// owner/controller text never goes stale. Both the mode switcher and the future
// scrubber call this -- keeping the whole view snapshot-driven.
function applyViewChange() {
  buildLegend();
  scheduleRecolor();
  if (M.selectedProv != null) renderInfo(describeProvince(M.selectedProv));
}

// ---- legend: states-per-owner from the current snapshot --------------------
function buildLegend() {
  const counts = {};
  for (const st in M.snapshot) {
    const key = (M.mode === 'control')
      ? (M.snapshot[st].controller || M.snapshot[st].owner)
      : M.snapshot[st].owner;
    if (!key) continue;
    counts[key] = (counts[key] || 0) + 1;
  }
  const sorted = Object.entries(counts).sort((a, b) => b[1] - a[1]);
  const top = sorted.slice(0, 16);
  const ul = $('legend-list');
  ul.textContent = '';
  for (const [tag, count] of top) {
    const li = document.createElement('li');
    const sw = document.createElement('span');
    sw.className = 'swatch';
    const rgb = M.colors[tag] || UNKNOWN_PROV;
    sw.style.background = 'rgb(' + rgb[0] + ',' + rgb[1] + ',' + rgb[2] + ')';
    const label = document.createElement('span');
    label.textContent = tag + '  (' + count + ')'; // textContent => HTML-safe
    li.appendChild(sw);
    li.appendChild(label);
    ul.appendChild(li);
  }
  const more = sorted.length - top.length;
  $('legend-more').textContent = more > 0 ? ('+ ' + more + ' more') : '';
  $('legend-title').textContent =
    (M.mode === 'control' ? 'Top controllers' : 'Top owners') +
    ' (' + sorted.length + ' tags)';
}

// ---- hover / click info panel ----------------------------------------------
function provinceAtClientXY(clientX, clientY) {
  const rect = M.displayCanvas.getBoundingClientRect();
  // map CSS coords -> raster pixel coords (canvas is scaled by CSS)
  const sx = M.width / rect.width, sy = M.height / rect.height;
  const x = Math.floor((clientX - rect.left) * sx);
  const y = Math.floor((clientY - rect.top) * sy);
  if (x < 0 || y < 0 || x >= M.width || y >= M.height) return null;
  return M.idRaster[y * M.width + x];
}

function describeProvince(prov) {
  if (prov === 0) return { water: true };
  const st = (prov < M.stateByProvince.length) ? M.stateByProvince[prov] : -1;
  if (st < 0) return { prov, sea: true };  // sea/lake province (has an id, no state)
  const meta = M.stateMeta[st] || {};
  const own = M.snapshot[st] || {};
  return {
    prov, state: st,
    name: meta.name_key || ('STATE_' + st),
    owner: own.owner || '(none)',
    controller: own.controller || own.owner || '(none)',
    contested: !!(own.owner && own.controller && own.owner !== own.controller),
    vp: meta.vp_total || 0,
    provCount: (meta.provinces ? meta.provinces.length : 0),
    category: meta.state_category || '',
    manpower: meta.manpower || 0,
  };
}

function renderInfo(info) {
  const el = $('info');
  el.textContent = '';
  if (!info) { el.textContent = 'Hover or click a province.'; return; }
  if (info.water) { el.textContent = 'Open water (no province id here).'; return; }
  if (info.sea) {
    el.textContent = 'Sea / lake province ' + info.prov + ' (no land state).';
    return;
  }
  const rows = [
    ['State', info.name + '  (#' + info.state + ')'],
    ['Province id', String(info.prov)],
    ['Owner', info.owner],
    ['Controller', info.controller + (info.contested ? '  [CONTESTED]' : '')],
    ['VP total', String(info.vp)],
    ['Provinces', String(info.provCount)],
    ['Category', info.category],
    ['Manpower', info.manpower.toLocaleString()],
  ];
  for (const [k, v] of rows) {
    const dt = document.createElement('dt'); dt.textContent = k;
    const dd = document.createElement('dd'); dd.textContent = v; // HTML-safe
    el.appendChild(dt); el.appendChild(dd);
  }
}

// ---- wiring -----------------------------------------------------------------
function wireUI() {
  // view-mode switcher
  for (const r of document.querySelectorAll('input[name="mode"]')) {
    r.addEventListener('change', (e) => {
      M.mode = e.target.value;
      applyViewChange();
    });
  }

  // hover -> live tooltip line; click -> sticky panel
  M.displayCanvas.addEventListener('mousemove', (e) => {
    const prov = provinceAtClientXY(e.clientX, e.clientY);
    const tip = $('tooltip');
    if (prov == null) { tip.style.display = 'none'; return; }
    const info = describeProvince(prov);
    let text;
    if (info.water) text = 'open water';
    else if (info.sea) text = 'sea / lake (prov ' + info.prov + ')';
    else text = info.name + '  o:' + info.owner + ' c:' + info.controller +
                (info.contested ? '  *' : '');
    tip.textContent = text;
    tip.style.left = (e.clientX + 14) + 'px';
    tip.style.top = (e.clientY + 14) + 'px';
    tip.style.display = 'block';
  });
  M.displayCanvas.addEventListener('mouseleave', () => {
    $('tooltip').style.display = 'none';
  });
  M.displayCanvas.addEventListener('click', (e) => {
    const prov = provinceAtClientXY(e.clientX, e.clientY);
    M.selectedProv = prov;   // remember selection so the panel can refresh on snapshot change
    renderInfo(prov == null ? null : describeProvince(prov));
  });

  // ---- TODO: SCRUBBER --------------------------------------------------------
  // Today there is a single snapshot (current melted save), so the slider is
  // disabled. To wire a real timeline later:
  //   1. Load an array of dated snapshots: SNAPS = [{date, ownership:{state:{owner,controller}}}, ...]
  //   2. Set the slider max = SNAPS.length-1, enable it, label with SNAPS[i].date.
  //   3. On input: M.snapshot = SNAPS[i].ownership; applyViewChange();
  // recolor()/buildColorTables() read M.snapshot, and applyViewChange() also
  // refreshes the legend and the sticky info panel for the selected province,
  // so nothing else changes -- the data flow is fully snapshot-driven by design.
  const slider = $('scrubber');
  slider.disabled = true;
  $('scrub-label').textContent = (M.saveMeta && M.saveMeta.save_date)
    ? ('Snapshot: ' + M.saveMeta.save_date + ' (only one; timeline TODO)')
    : 'Single snapshot (timeline TODO)';
}

// ---- validation report (printed to the on-page console + browser console) --
function validationReport() {
  // distinct province ids in the raster
  const seen = new Set();
  for (let i = 0; i < M.idRaster.length; i++) seen.add(M.idRaster[i]);
  const distinct = seen.size;            // includes id 0 (water sentinel)
  const distinctProv = distinct - (seen.has(0) ? 1 : 0);

  // pixel classification: owned land vs sea/lake (no state) vs no-owner land
  let ownedPixels = 0, seaPixels = 0, noOwnerPixels = 0, totalPixels = M.idRaster.length;
  for (let i = 0; i < M.idRaster.length; i++) {
    const prov = M.idRaster[i];
    if (prov === 0) { seaPixels++; continue; } // id 0 sentinel (absent in current asset)
    const st = (prov < M.stateByProvince.length) ? M.stateByProvince[prov] : -1;
    if (st < 0) { seaPixels++; continue; }      // sea/lake province (id present, no state)
    const own = M.snapshot[st];
    if (own && own.owner) ownedPixels++; else noOwnerPixels++;
  }

  const contestedStates = Object.values(M.snapshot)
    .filter((o) => o.owner && o.controller && o.owner !== o.controller).length;
  const noOwnerStates = Object.values(M.snapshot).filter((o) => !o.owner).length;

  // spot-checks
  const spot = [];
  // state 64 == GER capital (Berlin area)
  const s64 = M.snapshot['64'];
  spot.push('state 64 (Berlin area) owner=' + (s64 ? s64.owner : '?') +
            ' controller=' + (s64 ? s64.controller : '?'));
  // first contested state
  const firstContested = Object.entries(M.snapshot)
    .find(([, o]) => o.owner && o.controller && o.owner !== o.controller);
  if (firstContested) {
    spot.push('contested e.g. state ' + firstContested[0] +
              ' owner=' + firstContested[1].owner +
              ' controller=' + firstContested[1].controller);
  }

  const lines = [
    'distinct province ids in raster: ' + distinct + ' (excl. water sentinel: ' + distinctProv + ')',
    'land pixels with owner color: ' + ownedPixels +
      ' (' + (100 * ownedPixels / totalPixels).toFixed(1) + '% of ' + totalPixels + ' px)',
    'sea/lake (water) pixels: ' + seaPixels +
      ' (' + (100 * seaPixels / totalPixels).toFixed(1) + '%)  | no-owner land pixels: ' + noOwnerPixels,
    'states: ' + Object.keys(M.snapshot).length +
      '  contested: ' + contestedStates + '  no-owner: ' + noOwnerStates,
    ...spot,
  ];
  console.log('[map_view validation]\n' + lines.join('\n'));
  const box = $('validation');
  box.textContent = lines.join('\n');
  return { distinct, ownedPixels, contestedStates };
}

// ---- boot -------------------------------------------------------------------
async function boot() {
  try {
    setStatus('Loading data...');
    const [p2s, stateMeta, colors, ownership, saveMeta] = await Promise.all([
      fetchJSON(ASSETS.provinceToState),
      fetchJSON(ASSETS.stateMeta),
      fetchJSON(ASSETS.countryColors),
      fetchJSON(ASSETS.ownership),
      fetchJSON(ASSETS.saveMeta).catch(() => null),
    ]);
    M.provinceToState = p2s;
    M.stateMeta = stateMeta;
    M.colors = colors;
    M.snapshot = ownership;          // <-- the single scrubber snapshot (today)
    M.saveMeta = saveMeta;

    // packed color per tag
    M.packedColor = {};
    for (const tag in colors) M.packedColor[tag] = pack(colors[tag]);

    // decode raster ONCE
    const r = await chooseRaster();
    M.width = r.width; M.height = r.height; M.idRaster = r.idRaster;

    // stateByProvince indexed array
    let maxProv = 0;
    for (const k in p2s) { const v = +k; if (v > maxProv) maxProv = v; }
    // also cover any raw id that appears in the raster (defensive)
    for (let i = 0; i < M.idRaster.length; i++) if (M.idRaster[i] > maxProv) maxProv = M.idRaster[i];
    M.stateByProvince = buildStateByProvince(p2s, maxProv);

    // display canvas sized to raster
    M.displayCanvas = $('map');
    M.displayCanvas.width = M.width;
    M.displayCanvas.height = M.height;
    M.displayCtx = M.displayCanvas.getContext('2d', { willReadFrequently: false });
    M.displayCtx.imageSmoothingEnabled = false;
    M.displayImage = M.displayCtx.createImageData(M.width, M.height);

    wireUI();
    buildLegend();
    recolor();
    renderInfo(null);
    const v = validationReport();
    setStatus('Ready - ' + M.width + 'x' + M.height + ' raster, ' +
              v.distinct + ' province ids, ' + Object.keys(M.snapshot).length + ' states.');
  } catch (e) {
    console.error(e);
    setStatus('ERROR: ' + e.message + '  (is serve.py running? open via http://127.0.0.1:8777/map_view/)');
  }
}

window.addEventListener('DOMContentLoaded', boot);
