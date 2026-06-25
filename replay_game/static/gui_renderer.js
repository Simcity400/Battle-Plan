/* gui_renderer.js — ONE renderer that turns ANY parsed .gui window into DOM using
 * the game's real sprites, fonts, and coordinates, filled from replay data.
 *
 * This is the foundation of the "reuse the game's UI" pivot: instead of hand-coding
 * each window's CSS, we render the engine's own layout (game_assets/gui/*.json) with
 * the engine's own art (game_assets/sprites.json) and a small per-window DATA BINDING
 * map. Add a window = add its .gui name + a binding table; the renderer does the rest.
 *
 * Pipeline:
 *   GuiParser.layout(window, rootBox, sizeOf) -> flat placed boxes (draw order)
 *   for each box: draw its sprite (GfxRegistry) and/or text (Fonts), bound to data
 *
 * DATA BINDING (no fabrication):
 *   bindings = { "<element name>": binder }  where binder is either
 *     - a string dotted path into the bound country object ("politics.political_power")
 *     - a function (ctx) -> { text } | { value, captured } | { sprite, frame } | null
 *   ctx = { snap, tag, country, get(path) }  (get returns {value,captured})
 *   A bound text element with captured=false renders the game's empty slot (blank/
 *   "—"), never a fake number. An unbound text element renders its .gui literal text
 *   if present (labels like "LOBBY"), else nothing. Sprites are static (frame 0)
 *   unless a binder supplies a frame (e.g. a law/ideology strip index).
 *
 * Depends on: GuiParser, GfxRegistry, Fonts, UI (helpers), Store (get()).
 */
(function (global) {
  "use strict";

  const TEXT_TYPES = new Set(["instanttextboxtype", "texteditboxtype", "editboxtype"]);
  const SPRITE_TYPES = new Set([
    "icontype", "buttontype", "guibuttontype", "background", "checkboxtype",
  ]);

  function fmtNum(v) {
    if (v == null) return "";
    if (typeof v !== "number") return String(v);
    if (Math.abs(v) >= 1e6) return (v / 1e6).toFixed(2) + "M";
    if (Math.abs(v) >= 1e4) return (v / 1e3).toFixed(1) + "k";
    if (Number.isInteger(v)) return String(v);
    return (Math.round(v * 100) / 100).toString();
  }

  const GuiRenderer = {
    /**
     * Render one window into a host element.
     *  opts = {
     *    view, window,          // .gui view file + window name
     *    bindings,              // { elementName: binder }
     *    snap, tag,             // replay snapshot + bound country tag
     *    rootBox,               // {x,y,w,h} screen rect for the root window
     *    scale,                 // optional uniform scale (default 1)
     *    showGrid, debug,       // overlay diagnostics
     *  }
     * Returns { el, placed, misses } (el is positioned relative; caller positions it).
     */
    async render(host, opts) {
      const G = global.GfxRegistry, P = global.GuiParser, F = global.Fonts;
      await Promise.all([G.load(), P.loadView(opts.view)]);
      F.init();

      const doc = P.cache.get(opts.view);
      const win = P.findWindow(doc, opts.window);
      if (!win) {
        const err = document.createElement("div");
        err.className = "gr-error";
        err.textContent = "window '" + opts.window + "' not in " + opts.view + ".gui";
        host.appendChild(err);
        return { el: err, placed: [], misses: [] };
      }

      const sizeOf = (name) => G.sizeOf(name);
      const placed = P.layout(win, opts.rootBox || { x: 0, y: 0, w: 0, h: 0 }, sizeOf);

      const scale = opts.scale || 1;
      const root = document.createElement("div");
      root.className = "gr-window";
      root.style.position = "relative";
      root.style.width = (opts.rootBox.w * scale) + "px";
      root.style.height = (opts.rootBox.h * scale) + "px";
      root.style.transformOrigin = "top left";

      const country = (opts.snap && opts.snap.countries && opts.snap.countries[opts.tag]) || null;
      const ctx = {
        snap: opts.snap, tag: opts.tag, country,
        get: (path) => (global.Store ? global.Store.get(country, path)
                                     : { value: null, captured: false }),
        getOn: (obj, path) => (global.Store ? global.Store.get(obj, path)
                                            : { value: null, captured: false }),
      };

      const localMisses = Object.create(null);
      const note = (name) => { localMisses[name] = (localMisses[name] || 0) + 1; };

      for (const box of placed) {
        const n = box.node;
        const t = n.ktype;
        const binder = (opts.bindings && n.name) ? opts.bindings[n.name] : null;

        // position element
        const elx = (box.x - opts.rootBox.x) * scale;
        const ely = (box.y - opts.rootBox.y) * scale;

        // ---- sprite element (icon/button/background) -----------------------
        let spriteName = n.sprite;
        let frame = 0;
        let bindText = null, bindCaptured = true;
        if (binder) {
          const out = (typeof binder === "function")
            ? binder(ctx)
            : ctx.get(binder); // string path -> {value,captured}
          if (out && typeof out === "object") {
            if (out.sprite) spriteName = out.sprite;
            if (out.frame != null) frame = out.frame;
            if ("value" in out) { bindText = out.value; bindCaptured = !!out.captured; }
            if ("text" in out) { bindText = out.text; bindCaptured = out.text != null; }
            if (out.flag) { this._drawFlag(root, out.flag, box, opts, elx, ely, scale, note); }
          }
        }

        if (spriteName && (SPRITE_TYPES.has(t) || t === "icontype" || !TEXT_TYPES.has(t))) {
          const node = this._drawSprite(spriteName, frame, box, scale, note);
          if (node) {
            node.style.left = elx + "px";
            node.style.top = ely + "px";
            if (n.tooltip) node.title = n.tooltip;
            if (n.name) node.dataset.gui = n.name;
            root.appendChild(node);
          }
        }

        // ---- text element --------------------------------------------------
        if (TEXT_TYPES.has(t)) {
          const tnode = this._drawText(n, box, scale, bindText, bindCaptured, binder);
          tnode.style.left = elx + "px";
          tnode.style.top = ely + "px";
          if (n.name) tnode.dataset.gui = n.name;
          root.appendChild(tnode);
        }

        // ---- debug overlay -------------------------------------------------
        if (opts.debug) {
          const dbg = document.createElement("div");
          dbg.className = "gr-dbg";
          dbg.style.left = elx + "px"; dbg.style.top = ely + "px";
          dbg.style.width = (box.w * scale) + "px"; dbg.style.height = (box.h * scale) + "px";
          dbg.title = (n.name || n.type) + " @" + Math.round(box.x) + "," + Math.round(box.y) +
                      " " + Math.round(box.w) + "x" + Math.round(box.h) + " [" + n.orientation + "]";
          root.appendChild(dbg);
        }
      }

      host.appendChild(root);
      return { el: root, placed, misses: Object.keys(localMisses) };
    },

    _drawSprite(name, frame, box, scale, note) {
      const G = global.GfxRegistry;
      const r = G.resolve(name, frame);
      if (!r || !r.url) {
        note(name);
        const ph = document.createElement("div");
        ph.className = "gr-sprite gr-miss";
        ph.style.position = "absolute";
        ph.style.width = ((box.w || 16) * scale) + "px";
        ph.style.height = ((box.h || 16) * scale) + "px";
        ph.title = name + " — sprite not in asset set";
        ph.textContent = "□";
        return ph;
      }
      const el = document.createElement("div");
      el.className = "gr-sprite";
      el.style.position = "absolute";
      const w = (r.strip ? r.frame_width : r.width) || box.w || 16;
      const h = r.height || box.h || 16;
      el.style.width = (w * scale) + "px";
      el.style.height = (h * scale) + "px";
      el.style.backgroundImage = "url('" + r.url + "')";
      el.style.backgroundRepeat = "no-repeat";
      if (r.strip) {
        // single image strip: crop to frame window
        el.style.backgroundPosition = (-frame * r.frame_width * scale) + "px 0";
        el.style.backgroundSize = (r.width * scale) + "px " + (r.height * scale) + "px";
      } else {
        el.style.backgroundSize = (w * scale) + "px " + (h * scale) + "px";
      }
      return el;
    },

    _drawFlag(root, flagOpt, box, opts, elx, ely, scale, note) {
      const G = global.GfxRegistry;
      const rec = G.flag(flagOpt.tag, flagOpt.ideology, flagOpt.size || "medium");
      const el = document.createElement("div");
      el.className = "gr-sprite gr-flag";
      el.style.position = "absolute";
      el.style.left = elx + "px"; el.style.top = ely + "px";
      if (!rec || !rec.url) {
        note("flag:" + flagOpt.tag);
        el.classList.add("gr-miss");
        el.textContent = flagOpt.tag || "?";
        el.style.width = ((box.w || 24) * scale) + "px";
        el.style.height = ((box.h || 16) * scale) + "px";
      } else {
        el.style.width = ((box.w || rec.width) * scale) + "px";
        el.style.height = ((box.h || rec.height) * scale) + "px";
        el.style.backgroundImage = "url('" + rec.url + "')";
        el.style.backgroundSize = "100% 100%";
      }
      root.appendChild(el);
    },

    _drawText(n, box, scale, bindText, bindCaptured, hasBinder) {
      const F = global.Fonts;
      const el = document.createElement("div");
      el.className = "gr-text";
      el.style.position = "absolute";
      F.apply(el, n.font);
      if (scale !== 1) {
        const sz = parseFloat(el.style.fontSize) * scale;
        el.style.fontSize = sz + "px"; el.style.lineHeight = sz + "px";
      }
      // width: maxWidth governs wrap/alignment box; height from maxHeight
      const w = (n.maxWidth || box.w || 0);
      if (w) el.style.width = (w * scale) + "px";
      if (n.maxHeight) el.style.height = (n.maxHeight * scale) + "px";
      // format -> text-align (left|right|centre/center)
      const fmt = String(n.format || "left").toLowerCase();
      el.style.textAlign = (fmt === "right") ? "right"
        : (fmt === "centre" || fmt === "center") ? "center" : "left";
      el.style.whiteSpace = "nowrap";
      el.style.overflow = "hidden";

      // value resolution: bound value > literal .gui text > nothing
      let content, missing = false;
      if (hasBinder) {
        if (!bindCaptured) { content = ""; missing = true; }
        else content = (typeof bindText === "number") ? fmtNum(bindText) : String(bindText == null ? "" : bindText);
      } else {
        // literal layout text (labels like "LOBBY"); numeric placeholders in the
        // raw .gui (e.g. text=999) are DESIGN-TIME stubs -> render blank, not fake.
        const lit = n.text;
        content = (lit == null || typeof lit === "number") ? "" : String(lit);
      }
      el.textContent = content;
      if (missing) { el.classList.add("gr-missing"); el.title = (n.name || "value") + " — not captured"; }
      if (n.tooltip) el.title = n.tooltip;
      return el;
    },
  };

  global.GuiRenderer = GuiRenderer;
})(window);
