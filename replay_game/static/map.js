/* map.js — the central map: raster recolor + map modes + overlays.
 *
 * Reuses the DOCUMENTED replay_render pipeline (decode province_ids_half.png ONCE
 * into a Uint32 id raster, then a single packed-colour pixel loop per repaint),
 * fed by per-mode colorizers (snapshot) -> {state|prov id : [r,g,b]}.
 *
 * Coordinate space is the raster's (2816x1024). The wrapper locks aspect-ratio;
 * the overlay layer scales by rect (Codex E). Centroids are computed CLIENT-SIDE
 * from the decoded raster (no Pillow, no build step) with snap-to-nearest-visible
 * pixel; an optional /replay_game/output/centroids.json cache is used if present.
 */
(function (global) {
  "use strict";

  const MAP = {
    width: 0, height: 0,
    idRaster: null,        // Uint32Array province id per pixel
    stateByProvince: null, // Int32Array prov -> state (-1 if none)
    colors: {},            // TAG -> [r,g,b]
    centroids: { provinces: {}, states: {} }, // id -> [x,y] in raster px
    canvas: null,
    gl: null,              // WebGL recolor backend (WebGL-only; no CPU fallback)
    glImg: null,           // the decoded id PNG, kept for GL texture upload
    maxProvId: 0,          // highest province id seen (LUT sizing)
    overlay: null,         // absolute-positioned layer for bubbles/counters
    wrap: null,
    mode: "political",
    snap: null,
    selCountry: null,      // TAG for the diplomatic mode
    showDivs: true, showCombat: true,
    onProvinceClick: null, // (provId, stateId)=>void
    onCombatClick: null,   // (combat)=>void
    ro: null,
  };

  const UNKNOWN = [60, 60, 70];
  const SEA = [12, 18, 28];

  function pack(rgb) { return (255 << 24) | (rgb[2] << 16) | (rgb[1] << 8) | rgb[0]; }

  async function loadImage(src) {
    return new Promise((res, rej) => {
      const im = new Image();
      im.onload = () => res(im);
      im.onerror = () => rej(new Error("image load failed: " + src));
      im.src = src;
    });
  }

  function decodeRaster(img) {
    const c = document.createElement("canvas");
    c.width = img.naturalWidth; c.height = img.naturalHeight;
    const cx = c.getContext("2d", { willReadFrequently: true });
    cx.imageSmoothingEnabled = false;
    cx.drawImage(img, 0, 0);
    const img2d = cx.getImageData(0, 0, c.width, c.height);
    const d = img2d.data;
    const n = c.width * c.height;
    const ids = new Uint32Array(n);
    for (let i = 0, p = 0; i < n; i++, p += 4) {
      ids[i] = d[p] | (d[p + 1] << 8) | (d[p + 2] << 16);
    }
    // Return the exact decoded RGBA bytes too: the GL id-texture uploads THESE
    // (not the <img>), so the GPU sees byte-identical channels to the CPU idRaster
    // — no Canvas2D-vs-WebGL image-decode divergence (Codex SAFE #7).
    return { width: c.width, height: c.height, idRaster: ids, rgba: img2d };
  }

  function buildStateByProvince(p2s, maxP) {
    const arr = new Int32Array(maxP + 1).fill(-1);
    for (const k in p2s) { const v = +k; if (v >= 0 && v <= maxP) arr[v] = p2s[k]; }
    return arr;
  }

  /** Client-side centroids from the decoded raster (Codex A). Snap to a real pixel. */
  function computeCentroids() {
    const W = MAP.width, H = MAP.height, ids = MAP.idRaster, sbp = MAP.stateByProvince;
    const px = new Map(), py = new Map(), pc = new Map();       // province sums
    const anyPixel = new Map();                                 // province -> [x,y]
    const sx = new Map(), sy = new Map(), sc = new Map();       // state sums
    const sAny = new Map();
    for (let y = 0; y < H; y++) {
      const row = y * W;
      for (let x = 0; x < W; x++) {
        const pid = ids[row + x];
        if (!pid) continue;
        px.set(pid, (px.get(pid) || 0) + x);
        py.set(pid, (py.get(pid) || 0) + y);
        pc.set(pid, (pc.get(pid) || 0) + 1);
        if (!anyPixel.has(pid)) anyPixel.set(pid, [x, y]);
        const sid = (pid < sbp.length) ? sbp[pid] : -1;
        if (sid >= 0) {
          sx.set(sid, (sx.get(sid) || 0) + x);
          sy.set(sid, (sy.get(sid) || 0) + y);
          sc.set(sid, (sc.get(sid) || 0) + 1);
          if (!sAny.has(sid)) sAny.set(sid, [x, y]);
        }
      }
    }
    const provC = {}, stateC = {};
    function snap(cx, cy, predFn, fallback) {
      // is the arithmetic centroid actually on the right id? if not, snap to the
      // nearest visible pixel we already saw (any pixel of that id) — cheap + honest.
      const ix = Math.round(cx), iy = Math.round(cy);
      if (ix >= 0 && iy >= 0 && ix < W && iy < H && predFn(ids[iy * W + ix])) return [ix, iy];
      return fallback; // first-seen pixel of this id (guaranteed on-shape)
    }
    for (const [pid, n] of pc) {
      provC[pid] = snap(px.get(pid) / n, py.get(pid) / n,
        (v) => v === pid, anyPixel.get(pid));
    }
    for (const [sid, n] of sc) {
      stateC[sid] = snap(sx.get(sid) / n, sy.get(sid) / n,
        (v) => (v < sbp.length ? sbp[v] : -1) === sid, sAny.get(sid));
    }
    return { provinces: provC, states: stateC };
  }

  // ---- map-mode colorizers: (snapshot) -> {id: [r,g,b]} (by state unless noted)
  const Modes = global.MapModes; // defined in modes.js (loaded first)

  /** Build the per-frame {key -> packedRGBA} LUT from the active mode's colorize.
   *  Shared by the GL and CPU paths so colors are byte-identical between them. */
  function buildColorLUT() {
    const snap = MAP.snap;
    const mode = Modes.byId[MAP.mode] || Modes.byId.political;
    const result = mode.colorize(snap, MAP); // {stateId|provId: [r,g,b]}
    const byProvince = !!mode.perProvince;
    const packed = new Map();
    for (const k in result) packed.set(+k, pack(result[k]));
    return { packed, byProvince };
  }

  function recolor() {
    // WebGL-only: if the context was lost mid-session (MAP.gl is null until it
    // restores), keep the last painted frame and just refresh overlays — there is
    // no CPU fallback path by design, and we never throw.
    if (MAP.gl) {
      const { packed, byProvince } = buildColorLUT();
      recolorGL(packed, byProvince);
    }
    drawOverlays();
  }

  function recolorGL(packed, byProvince) {
    // The GPU reproduces SEA (id 0 / state<0) and UNKNOWN (key absent) exactly as
    // the CPU path; we hand it the same SEA/UNKNOWN colors and packed LUT.
    MAP.gl.recolor(packed, byProvince, SEA, UNKNOWN);
  }

  // ---- overlay layer: combat bubbles + division counters ------------------
  function rasterToDisplay() {
    const r = MAP.canvas.getBoundingClientRect();
    return { sx: r.width / MAP.width, sy: r.height / MAP.height };
  }

  function provCentroid(provId) {
    return MAP.centroids.provinces[provId] || MAP.centroids.provinces[String(provId)] || null;
  }
  function stateCentroid(stateId) {
    return MAP.centroids.states[stateId] || MAP.centroids.states[String(stateId)] || null;
  }

  function drawOverlays() {
    const ov = MAP.overlay;
    ov.innerHTML = "";
    const snap = MAP.snap;
    if (!snap) return;
    const { sx, sy } = rasterToDisplay();
    const unplaced = [];

    if (MAP.showDivs && Array.isArray(snap.divisions)) {
      // group by province, sum strength, dominant owner
      const byProv = new Map();
      for (const d of snap.divisions) {
        const p = d.location_province;
        if (p == null) continue;
        let g = byProv.get(p);
        if (!g) { g = { n: 0, str: 0, strCap: 0, owners: {} }; byProv.set(p, g); }
        g.n++;
        // only fold strength when it's actually captured; track how many had it so
        // the counter can show "Σstrength N (k of m captured)" instead of a fake 0.
        if (d.strength != null) { g.str += d.strength; g.strCap++; }
        if (d.owner) g.owners[d.owner] = (g.owners[d.owner] || 0) + 1;
      }
      for (const [p, g] of byProv) {
        const c = provCentroid(p);
        const owner = Object.entries(g.owners).sort((a, b) => b[1] - a[1])[0];
        if (!c) { unplaced.push({ kind: "div", prov: p, n: g.n }); continue; }
        const node = divCounter(owner ? owner[0] : null, g.n, g.strCap > 0 ? g.str : null, g.strCap, g.n);
        place(node, c, sx, sy);
        ov.appendChild(node);
      }
    }

    if (MAP.showCombat && Array.isArray(snap.combats)) {
      for (const cb of snap.combats) {
        const p = cb.location_province;
        const c = p != null ? provCentroid(p) : null;
        if (!c) { unplaced.push({ kind: "combat", prov: p, id: cb.combat_id }); continue; }
        const node = combatBubble(cb, snap);
        place(node, c, sx, sy);
        node.addEventListener("click", (e) => {
          e.stopPropagation();
          if (MAP.onCombatClick) MAP.onCombatClick(cb);
        });
        ov.appendChild(node);
      }
    }
    MAP.unplaced = unplaced;
    if (MAP.onUnplaced) MAP.onUnplaced(unplaced);
  }

  function place(node, c, sx, sy) {
    node.style.left = (c[0] * sx) + "px";
    node.style.top = (c[1] * sy) + "px";
  }

  function divCounter(ownerTag, n, str, strCap, total) {
    const el = document.createElement("div");
    el.className = "ov-div";
    const flag = global.Sprites.flag(ownerTag, null, "small");
    flag.classList.add("ov-div-flag");
    el.appendChild(flag);
    const lbl = document.createElement("span");
    lbl.className = "ov-div-n";
    lbl.textContent = n;
    el.appendChild(lbl);
    // scale by captured strength only (visual); if no strength captured, neutral size.
    const scale = str == null ? 1.0 : Math.min(1.6, 0.8 + Math.log10(1 + str) * 0.22);
    el.style.transform = `translate(-50%,-50%) scale(${scale.toFixed(2)})`;
    const strTxt = str == null ? "strength not captured"
      : `Σstrength ${Math.round(str)}` + (strCap < total ? ` (${strCap} of ${total} captured)` : "");
    el.title = `${ownerTag || "owner not captured"}: ${n} division(s), ${strTxt}`;
    return el;
  }

  /** Resolve combat participant [div_id,div_type] -> owner TAG in THIS snap (Codex B). */
  function resolveOwners(units, snap) {
    const byId = snap._divById || (snap._divById = indexDivs(snap));
    const known = {}, unknown = [];
    for (const u of (units || [])) {
      const id = Array.isArray(u) ? u[0] : u;
      const d = byId.get(id);
      if (d && d.owner) known[d.owner] = (known[d.owner] || 0) + 1;
      else unknown.push(id);
    }
    return { known, unknown };
  }
  function indexDivs(snap) {
    const m = new Map();
    for (const d of (snap.divisions || [])) m.set(d.div_id, d);
    return m;
  }

  function combatBubble(cb, snap) {
    const el = document.createElement("div");
    el.className = "ov-combat ov-combat-" + (cb.kind || "land");
    const atk = resolveOwners(cb.attackers, snap);
    const def = resolveOwners(cb.defenders, snap);
    const aTag = Object.keys(atk.known)[0] || null;
    const dTag = Object.keys(def.known)[0] || null;
    const af = global.Sprites.flag(aTag, null, "small"); af.classList.add("ov-cb-flag");
    const df = global.Sprites.flag(dTag, null, "small"); df.classList.add("ov-cb-flag");
    el.appendChild(af);
    const vs = document.createElement("span"); vs.className = "ov-cb-vs"; vs.textContent = "⚔"; el.appendChild(vs);
    el.appendChild(df);
    el.title = `Combat ${cb.combat_id || ""} (${cb.kind || "?"}) @prov ${cb.location_province}`
      + `\nattackers: ${Object.keys(atk.known).join(",") || "?"}${atk.unknown.length ? " +?" + atk.unknown.length : ""}`
      + `\ndefenders: ${Object.keys(def.known).join(",") || "?"}${def.unknown.length ? " +?" + def.unknown.length : ""}`
      + (cb.terrain ? `\nterrain: ${cb.terrain}` : "");
    return el;
  }

  // ---- hit testing --------------------------------------------------------
  function provAt(clientX, clientY) {
    const r = MAP.canvas.getBoundingClientRect();
    const x = Math.floor((clientX - r.left) * MAP.width / r.width);
    const y = Math.floor((clientY - r.top) * MAP.height / r.height);
    if (x < 0 || y < 0 || x >= MAP.width || y >= MAP.height) return null;
    return MAP.idRaster[y * MAP.width + x];
  }

  // ---- public API ---------------------------------------------------------
  MAP.init = async function init(wrapEl) {
    MAP.wrap = wrapEl;
    wrapEl.classList.add("map-wrap");
    MAP.canvas = document.createElement("canvas");
    MAP.canvas.className = "map-canvas";
    MAP.overlay = document.createElement("div");
    MAP.overlay.className = "map-overlay";
    MAP.tooltip = document.createElement("div");
    MAP.tooltip.className = "map-tip"; MAP.tooltip.style.display = "none";
    wrapEl.appendChild(MAP.canvas);
    wrapEl.appendChild(MAP.overlay);
    wrapEl.appendChild(MAP.tooltip);

    // colors
    const cc = await (await fetch("/map_pipeline/assets/country_colors.json")).json();
    MAP.colors = cc;

    // raster
    const meta = await (await fetch("/map_pipeline/assets/province_ids_meta.json")).json();
    const pngUrl = "/map_pipeline/assets/" + meta.downscaled_display.file;
    const img = await loadImage(pngUrl);
    const r = decodeRaster(img);              // (uses a throwaway canvas, not MAP.canvas)
    MAP.width = r.width; MAP.height = r.height; MAP.idRaster = r.idRaster;
    MAP.glImg = r.rgba;                        // exact decoded RGBA for the GL id-texture
    MAP.canvas.width = MAP.width; MAP.canvas.height = MAP.height;

    // province -> state (must exist before we build the GL prov->state LUT)
    const p2s = await (await fetch("/map_pipeline/assets/province_to_state.json")).json();
    let maxP = 0;
    for (const k in p2s) { const v = +k; if (v > maxP) maxP = v; }
    for (let i = 0; i < MAP.idRaster.length; i++) if (MAP.idRaster[i] > maxP) maxP = MAP.idRaster[i];
    MAP.stateByProvince = buildStateByProvince(p2s, maxP);
    MAP.maxProvId = maxP;

    // Renderer: WebGL ONLY. This tool runs on machines that run the game (a GPU game),
    // and WebGL has been universal in browsers for ~a decade — so there is no
    // Canvas2D fallback renderer to maintain (deliberate, per user: no fallback
    // bloat). If WebGL is genuinely unavailable we fail LOUD with a one-line message
    // instead of silently shipping a slow second path.
    MAP.gl = global.MapGL ? global.MapGL.create(MAP.canvas, MAP.glImg, MAP.stateByProvince, maxP) : null;
    if (!MAP.gl) {
      const msg = document.createElement("div");
      msg.className = "map-webgl-required";
      msg.textContent = "This replay map requires WebGL, which isn't available in this browser/device.";
      MAP.wrap.appendChild(msg);
      console.error("[map] WebGL unavailable — map cannot render (WebGL-only by design; no CPU fallback).");
    } else {
      console.log("[map] recolor backend: WebGL (" + (MAP.gl.backend || "gl") + ")");
    }

    // WebGL context-loss handling (Codex Low #3 — the Mac/ANGLE/mobile risk). On
    // loss we suppress the default (which would freeze a black map) and mark GL
    // unavailable so recolor() becomes a no-op rather than throwing; on restore we
    // rebuild the GL backend and repaint. We can't re-acquire 2D on a GL canvas, so
    // a permanently-lost context simply leaves the last painted frame (no crash).
    if (MAP.gl) {
      MAP.canvas.addEventListener("webglcontextlost", (e) => {
        e.preventDefault();
        console.warn("[map] WebGL context lost — recolor paused until restore");
        MAP.gl = null;
      }, false);
      MAP.canvas.addEventListener("webglcontextrestored", () => {
        console.log("[map] WebGL context restored — rebuilding backend");
        MAP.gl = global.MapGL ? global.MapGL.create(MAP.canvas, MAP.glImg, MAP.stateByProvince, MAP.maxProvId) : null;
        if (MAP.snap) MAP.render();
      }, false);
    }

    // centroids: prefer the cached build, else compute client-side.
    try {
      const cr = await fetch("/replay_game/output/centroids.json", { cache: "force-cache" });
      if (cr.ok) {
        const cj = await cr.json();
        if (cj && cj.raster_width === MAP.width) {
          MAP.centroids = { provinces: cj.provinces || {}, states: cj.states || {} };
        }
      }
    } catch (e) { /* fall through */ }
    if (!Object.keys(MAP.centroids.provinces).length) {
      MAP.centroids = computeCentroids();
    }

    // wiring: pointer
    MAP.canvas.addEventListener("mousemove", (e) => {
      const p = provAt(e.clientX, e.clientY);
      const t = MAP.tooltip;
      if (p == null || p === 0) { t.style.display = "none"; return; }
      const sid = (p < MAP.stateByProvince.length) ? MAP.stateByProvince[p] : -1;
      const st = sid >= 0 && MAP.snap ? (MAP.snap.states || {})[sid] : null;
      const owner = st ? st.owner : null, ctrl = st ? st.controller : null;
      t.innerHTML = `prov <b>${p}</b>` + (sid >= 0 ? ` · state <b>${sid}</b>` : " · sea")
        + (owner ? `<br>owner ${owner}${ctrl && ctrl !== owner ? " · ctrl " + ctrl : ""}` : "");
      const wr = MAP.wrap.getBoundingClientRect();
      t.style.left = (e.clientX - wr.left + 12) + "px";
      t.style.top = (e.clientY - wr.top + 12) + "px";
      t.style.display = "block";
    });
    MAP.canvas.addEventListener("mouseleave", () => { MAP.tooltip.style.display = "none"; });
    MAP.canvas.addEventListener("click", (e) => {
      const p = provAt(e.clientX, e.clientY);
      if (p == null || p === 0) return;
      const sid = (p < MAP.stateByProvince.length) ? MAP.stateByProvince[p] : -1;
      if (MAP.onProvinceClick) MAP.onProvinceClick(p, sid >= 0 ? sid : null);
    });

    // resize: overlays track CSS display size
    MAP.ro = new ResizeObserver(() => { if (MAP.snap) drawOverlays(); });
    MAP.ro.observe(MAP.canvas);
    return MAP;
  };

  MAP.setSnapshot = function (snap) { MAP.snap = snap; if (snap) { snap._divById = null; } };
  MAP.setMode = function (id) { MAP.mode = id; };
  MAP.render = function () { if (MAP.snap) recolor(); };
  MAP.setShow = function (opts) {
    if ("divs" in opts) MAP.showDivs = opts.divs;
    if ("combat" in opts) MAP.showCombat = opts.combat;
    if (MAP.snap) drawOverlays();
  };
  MAP.setSelectedCountry = function (tag) { MAP.selCountry = tag; };

  global.GameMap = MAP;
})(window);
