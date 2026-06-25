/* gfx_registry.js — the .gfx sprite registry for the game-authentic renderer.
 *
 * Reads game_assets/sprites.json (produced by game_assets/extract.py, which parses
 * the EFFECTIVE .gfx set = vanilla overridden by mod, and pre-slices multi-frame
 * strips into per-frame PNGs). This module is the single source of truth the
 * gui_renderer uses to turn a spriteType NAME into a concrete image + its native
 * pixel dimensions + frame metadata.
 *
 * spriteType name -> { png_path(frame), no_of_frames, frame_width, width, height }
 *
 * Distinct from the legacy static/sprites.js (which builds bespoke topbar widgets):
 * this registry is consumed by ONE generic renderer that drives EVERY .gui window.
 *
 * HARD RULE (no fabrication): an unresolved sprite returns null + is recorded in
 * .misses — the renderer then draws a labelled placeholder, never invented art.
 */
(function (global) {
  "use strict";

  const ASSET_BASE = "/game_assets/"; // sprites.json png paths are relative to this

  const GfxRegistry = {
    manifest: null,
    sprites: {},          // GFX_name -> record
    flags: {},            // TAG / TAG_ideology -> record
    misses: Object.create(null),
    loaded: false,

    async load() {
      if (this.loaded) return this.manifest;
      const r = await fetch(ASSET_BASE + "sprites.json", { cache: "force-cache" });
      if (!r.ok) throw new Error("sprites.json HTTP " + r.status);
      this.manifest = await r.json();
      this.sprites = this.manifest.sprites || {};
      this.flags = this.manifest.flags || {};
      this.loaded = true;
      return this.manifest;
    },

    _url(rel) { return ASSET_BASE + String(rel).replace(/^\/+/, ""); },
    _miss(name) { this.misses[name] = (this.misses[name] || 0) + 1; return null; },

    has(name) { return !!this.sprites[name]; },
    rec(name) { return this.sprites[name] || null; },

    /**
     * Resolve a sprite NAME (+ frame index) to a concrete drawable descriptor:
     *   { url, width, height, no_of_frames, frame, nine_slice, border_size,
     *     progress_bar, masked_shield, color, color_two, bg_url }
     * Multi-frame strips were pre-sliced by the pipeline into png[] (one per frame),
     * so we simply index png[frame]. Returns null (and logs a miss) if unknown.
     */
    resolve(name, frame) {
      const rec = this.sprites[name];
      if (!rec) return this._miss(name);
      const pngs = rec.png || [];
      const nFrames = rec.no_of_frames || pngs.length || 1;
      let fi = frame == null ? 0 : (frame | 0);
      if (fi < 0) fi = 0;
      if (fi > pngs.length - 1) fi = pngs.length - 1;
      const out = {
        name,
        url: pngs[fi] != null ? this._url(pngs[fi]) : null,
        width: rec.width || (rec.size && rec.size.x) || null,
        height: rec.height || (rec.size && rec.size.y) || null,
        no_of_frames: nFrames,
        frame: fi,
        nine_slice: !!rec.nine_slice,
        border_size: rec.border_size || null,
        progress_bar: !!rec.progress_bar,
        masked_shield: !!rec.masked_shield,
        color: rec.color || null,
        color_two: rec.color_two || null,
        bg_url: rec.texturefile2 && rec.png2 ? this._url(rec.png2) : null,
      };
      // For a framed strip where the pipeline could NOT pre-slice (single png with
      // no_of_frames>1), expose frame_width so the renderer can crop via CSS.
      if (pngs.length === 1 && nFrames > 1 && out.width) {
        out.frame_width = Math.round(out.width / nFrames);
        out.strip = true; // single image; crop window of frame_width
      }
      return out;
    },

    /** Native pixel size of a sprite (frame 0), or null if unknown. */
    sizeOf(name) {
      const r = this.resolve(name, 0);
      if (!r) return null;
      const w = r.strip ? r.frame_width : r.width;
      return (w && r.height) ? { w, h: r.height } : null;
    },

    /** Resolve a flag record for TAG (+ optional ideology). size in large|medium|small. */
    flag(tag, ideology, size) {
      if (!tag) return null;
      const f = this.flags;
      let rec = (ideology && f[tag + "_" + ideology]) ? f[tag + "_" + ideology] : f[tag];
      if (!rec) { for (const k in f) { if (k === tag || k.startsWith(tag + "_")) { rec = f[k]; break; } } }
      if (!rec) { this._miss("flag:" + tag); return null; }
      const res = (rec.res && size && rec.res[size]) ? rec.res[size] : null;
      const png = res ? res.png : rec.png;
      return {
        url: png ? this._url(png) : null,
        width: res ? res.width : rec.width,
        height: res ? res.height : rec.height,
      };
    },

    missList() {
      return Object.keys(this.misses).sort().map((k) => ({ name: k, count: this.misses[k] }));
    },
  };

  global.GfxRegistry = GfxRegistry;
})(window);
