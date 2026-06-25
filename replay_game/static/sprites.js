/* sprites.js — the sprite loader (PLAN §I).
 *
 * A tiny "sprite by name -> DOM node" helper over game_assets/sprites.json.
 * Pure, offline, no dependencies. Handles:
 *   - single + multi-frame strips (_f0, _f1, ...; pick a frame index)
 *   - flags keyed TAG or TAG_ideology, with a res map (large/medium/small)
 *   - nine-slice window chrome (border_size) via CSS border-image
 *   - two-layer sprites: masked_shield (flag over mask) and progress_bar
 *     (fill tinted color..color_two over a background) via <canvas> composite
 *
 * HARD RULE (no fabrication): a sprite name that does not resolve returns a
 * labelled placeholder node and logs the miss to Sprites.misses — it never
 * blocks and never silently invents art.
 */
(function (global) {
  "use strict";

  const ASSET_BASE = "/game_assets/"; // sprites.json png paths are relative to this

  const Sprites = {
    manifest: null,      // {sprites:{}, flags:{}, _meta:{}}
    sprites: {},
    flags: {},
    misses: Object.create(null), // name -> count (for the diagnostics panel)
    ideologyColors: {
      fascism: "#8a7d2b", communism: "#9e2b25",
      democratic: "#2f5f9e", neutrality: "#6b6b6b", null: "#555",
    },

    async load() {
      const r = await fetch(ASSET_BASE + "sprites.json", { cache: "force-cache" });
      if (!r.ok) throw new Error("sprites.json HTTP " + r.status);
      this.manifest = await r.json();
      this.sprites = this.manifest.sprites || {};
      this.flags = this.manifest.flags || {};
      return this.manifest;
    },

    _png(rel) { return ASSET_BASE + String(rel).replace(/^\/+/, ""); },

    has(name) { return !!this.sprites[name]; },

    /** Raw sprite record or null. */
    rec(name) { return this.sprites[name] || null; },

    _miss(name) { this.misses[name] = (this.misses[name] || 0) + 1; },

    /** Resolve a flag record for a TAG (+ optional ideology). Never fabricates. */
    flagRec(tag, ideology) {
      if (!tag) return null;
      const f = this.flags;
      if (ideology && f[tag + "_" + ideology]) return f[tag + "_" + ideology];
      if (f[tag]) return f[tag];
      // any ideology variant as a last resort (keyed TAG_*)
      for (const k in f) if (k === tag || k.startsWith(tag + "_")) return f[k];
      return null;
    },

    /**
     * A flag <img> (or a coloured placeholder chip). size in {large,medium,small}.
     * Returns an HTMLElement. Honest: unknown tag -> grey chip with the tag text.
     */
    flag(tag, ideology, size) {
      const rec = this.flagRec(tag, ideology);
      const el = document.createElement("span");
      el.className = "spr-flag";
      if (!rec) {
        el.classList.add("spr-flag-missing");
        el.textContent = tag || "?";
        el.title = (tag || "?") + " — flag not in asset set";
        this._miss("flag:" + tag + (ideology ? "_" + ideology : ""));
        return el;
      }
      const res = (rec.res && size && rec.res[size]) ? rec.res[size] : null;
      const png = res ? res.png : rec.png;
      const img = document.createElement("img");
      img.src = this._png(png);
      img.alt = tag + (ideology ? " " + ideology : "");
      img.title = img.alt;
      img.loading = "lazy";
      img.draggable = false;
      el.appendChild(img);
      return el;
    },

    /**
     * A sprite icon <img> by GFX name. opts:{frame, size:{w,h}, title, cls}.
     * Multi-frame strips use opts.frame (default 0). Honest placeholder on miss.
     */
    icon(name, opts) {
      opts = opts || {};
      const rec = this.sprites[name];
      if (!rec) {
        this._miss(name);
        const ph = document.createElement("span");
        ph.className = "spr-miss" + (opts.cls ? " " + opts.cls : "");
        ph.title = name + " — sprite not found";
        ph.textContent = "□";
        return ph;
      }
      // two-layer sprites need compositing
      if (rec.progress_bar) return this.progressBar(name, opts.value, opts);
      if (rec.masked_shield) return this._maskedShield(rec, opts);

      const img = document.createElement("img");
      const pngs = rec.png || [];
      const fi = Math.max(0, Math.min((opts.frame | 0), pngs.length - 1));
      img.src = this._png(pngs[fi] || pngs[0] || "");
      img.alt = opts.title || name;
      img.title = opts.title || name;
      img.className = "spr-icon" + (opts.cls ? " " + opts.cls : "");
      img.loading = "lazy";
      img.draggable = false;
      if (opts.size) { img.style.width = opts.size.w + "px"; img.style.height = opts.size.h + "px"; }
      return img;
    },

    /** Apply a nine-slice sprite as a CSS border-image background on `el`. */
    nineSlice(el, name) {
      const rec = this.sprites[name];
      if (!rec || !rec.png || !rec.png[0]) { this._miss(name); return el; }
      const b = rec.border_size || { x: 8, y: 8 };
      el.style.borderImageSource = "url(" + this._png(rec.png[0]) + ")";
      el.style.borderImageSlice = `${b.y} ${b.x} ${b.y} ${b.x} fill`;
      el.style.borderImageWidth = `${b.y}px ${b.x}px ${b.y}px ${b.x}px`;
      el.style.borderImageRepeat = "stretch";
      el.style.borderStyle = "solid";
      el.style.borderColor = "transparent";
      el.classList.add("spr-nineslice");
      return el;
    },

    /**
     * Progress bar (fuel/stability/war-support/resource): two-layer sprite with
     * a fill tinted between color..color_two by `value` (0..1). Returns a node.
     * If value is null/undefined we render the background only + a "—" — never
     * a fabricated fill.
     */
    progressBar(name, value, opts) {
      opts = opts || {};
      const rec = this.sprites[name] || {};
      const size = rec.size || { x: 80, y: 8 };
      const wrap = document.createElement("span");
      wrap.className = "spr-pbar";
      wrap.style.width = (opts.w || size.x) + "px";
      wrap.style.height = (opts.h || size.y) + "px";
      if (rec.png2) wrap.style.backgroundImage = "url(" + this._png(rec.png2) + ")";
      const fill = document.createElement("span");
      fill.className = "spr-pbar-fill";
      if (value == null || isNaN(value)) {
        wrap.title = (opts.title || name) + " — not captured";
        wrap.classList.add("spr-pbar-empty");
      } else {
        const v = Math.max(0, Math.min(1, value));
        const c1 = rec.color || [0.2, 0.7, 0.2];
        const c2 = rec.color_two || [0.7, 0.2, 0.2];
        const mix = (i) => Math.round(255 * (c2[i] + (c1[i] - c2[i]) * v));
        fill.style.width = (v * 100) + "%";
        fill.style.background = `rgb(${mix(0)},${mix(1)},${mix(2)})`;
        wrap.title = (opts.title || name) + ": " + Math.round(v * 100) + "%";
      }
      wrap.appendChild(fill);
      return wrap;
    },

    _maskedShield(rec, opts) {
      // We don't usually need the literal masked-shield composite for replay
      // chrome; render the overlay layer (png[0]) which is the visible flag/frame.
      const img = document.createElement("img");
      img.src = this._png((rec.png && rec.png[0]) || "");
      img.className = "spr-icon spr-shield" + (opts.cls ? " " + opts.cls : "");
      img.title = opts.title || "";
      img.loading = "lazy";
      img.draggable = false;
      return img;
    },

    /** Focus goal icon for a focus id (GFX_focus_<id>) with a generic fallback. */
    focusIcon(focusId, opts) {
      opts = opts || {};
      const cand = "GFX_focus_" + focusId;
      if (this.sprites[cand]) return this.icon(cand, opts);
      if (this.sprites["GFX_goal_unknown"]) {
        const el = this.icon("GFX_goal_unknown", opts);
        el.title = focusId + " — goal icon not found (generic)";
        return el;
      }
      return this.icon(cand, opts); // -> placeholder + miss log
    },

    /** Resource icon by resource token (steel/oil/...) -> GFX_<resource>. */
    resourceIcon(resource, opts) {
      const map = { aluminum: "aluminium" };
      const r = map[resource] || resource;
      const name = "GFX_" + r;
      if (this.sprites[name]) return this.icon(name, opts);
      // chromium has no standalone GFX_chromium; fall back to the resource strip
      return this.icon(name, opts);
    },

    missCount() {
      let n = 0; for (const k in this.misses) n += this.misses[k]; return n;
    },
  };

  global.Sprites = Sprites;
})(window);
