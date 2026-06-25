/* gui_parser.js — loads parsed .gui window trees and resolves the Clausewitz
 * coordinate / orientation model into absolute browser pixel boxes.
 *
 * Input: game_assets/gui/<view>.json (produced by game_assets/extract.py from the
 * EFFECTIVE .gui set = vanilla overridden by mod). Each file is { view, source,
 * windows:[ node ] }; a node = { type, name, position{x,y}, size{width,height},
 * orientation, sprite, text{text,font,format,maxWidth,maxHeight}, tooltip,
 * children:[...] }. Child draw order is preserved.
 *
 * THE COORDINATE MODEL (Clausewitz gui, as used by HOI4/EU4/CK):
 *  - A node's position{x,y} is an OFFSET measured from a CORNER/edge of its PARENT
 *    container's box, chosen by `orientation`. The node's OWN top-left is what gets
 *    placed at that anchor (orientation does NOT flip the node's own pivot; a node
 *    at UPPER_RIGHT x=-100 puts the node's left edge 100px in from the parent's
 *    right edge — it can overflow the parent if |x| < node_width).
 *  - Offsets ACCUMULATE down the tree: each level adds its orientation-resolved
 *    offset onto the parent's resolved absolute origin.
 *  - size width/height: a number is px; "100%%"/"100%" means "fill parent on that
 *    axis"; a negative number means parent_size + value. A node with no size that
 *    carries a sprite inherits the sprite's native pixel dimensions (resolved by
 *    the caller via the gfx registry).
 *
 * This module is data-only (no DOM). gui_renderer.js consumes resolved boxes.
 */
(function (global) {
  "use strict";

  const GUI_BASE = "/game_assets/gui/";

  // Orientation -> which parent corner the offset is anchored from, expressed as
  // multipliers of the parent box. ax/ay in {0,0.5,1} select left/center/right and
  // top/center/bottom of the PARENT; the node's own top-left lands at that point + (x,y).
  const ORIENT = {
    upper_left:   { ax: 0, ay: 0 },
    upper_right:  { ax: 1, ay: 0 },
    lower_left:   { ax: 0, ay: 1 },
    lower_right:  { ax: 1, ay: 1 },
    center:       { ax: 0.5, ay: 0.5 },
    center_up:    { ax: 0.5, ay: 0 },
    center_down:  { ax: 0.5, ay: 1 },
    center_left:  { ax: 0, ay: 0.5 },
    center_right: { ax: 1, ay: 0.5 },
    left:         { ax: 0, ay: 0 },
    right:        { ax: 1, ay: 0 },
  };

  function orientAnchor(o) {
    return ORIENT[String(o || "upper_left").toLowerCase()] || ORIENT.upper_left;
  }

  /** Parse a size value (px number | "N%%" | "N%" | negative) against a parent extent. */
  function resolveExtent(v, parentExtent) {
    if (v == null) return null;
    if (typeof v === "number") return v < 0 ? Math.max(0, parentExtent + v) : v;
    const s = String(v).trim();
    const pct = s.match(/^(-?\d+(?:\.\d+)?)\s*%+$/);
    if (pct) return Math.round(parentExtent * (parseFloat(pct[1]) / 100));
    const n = parseFloat(s);
    if (!isNaN(n)) return n < 0 ? Math.max(0, parentExtent + n) : n;
    return null;
  }

  const GuiParser = {
    cache: new Map(), // view -> {view, source, windows, index}

    async loadView(view) {
      if (this.cache.has(view)) return this.cache.get(view);
      const r = await fetch(GUI_BASE + view + ".json", { cache: "force-cache" });
      if (!r.ok) throw new Error("gui/" + view + ".json HTTP " + r.status);
      const doc = await r.json();
      doc.index = this._indexByName(doc.windows || []);
      this.cache.set(view, doc);
      return doc;
    },

    _indexByName(windows) {
      const idx = Object.create(null);
      const walk = (n) => {
        if (n && n.name && !idx[n.name]) idx[n.name] = n;
        (n.children || []).forEach(walk);
      };
      windows.forEach(walk);
      return idx;
    },

    /** Find a top-level (or nested) window node by name within a loaded view. */
    findWindow(doc, name) {
      return (doc.index && doc.index[name]) || null;
    },

    /**
     * Normalize a raw parsed node into the fields the renderer cares about,
     * repairing the known extraction quirks:
     *   - gridbox/positionType `format` was mis-bucketed into text.format; for
     *     gridboxType this is the grid FILL direction, not a text alignment.
     *   - keep size as {width,height}; keep orientation; keep sprite/text/tooltip.
     */
    norm(node) {
      const t = (node.type || "").toLowerCase();
      const out = {
        type: node.type,
        ktype: t,
        name: node.name || null,
        position: node.position || { x: 0, y: 0 },
        size: node.size || null,
        orientation: node.orientation || "upper_left",
        sprite: node.sprite || null,
        tooltip: node.tooltip || null,
        text: null,
        font: null,
        format: null,
        maxWidth: null,
        maxHeight: null,
        gridFormat: null,
        children: node.children || [],
      };
      const tx = node.text || null;
      if (tx) {
        out.text = (tx.text != null) ? tx.text : null;
        out.font = tx.font || null;
        out.maxWidth = tx.maxWidth != null ? tx.maxWidth : null;
        out.maxHeight = tx.maxHeight != null ? tx.maxHeight : null;
        if (t === "gridboxtype") out.gridFormat = tx.format || null;
        else out.format = tx.format || null;
      }
      return out;
    },

    /**
     * Resolve the full layout tree to absolute boxes. Returns a flat list of
     * placed elements (draw order), each:
     *   { node(normalized), x, y, w, h, depth }
     * `rootBox` = {x,y,w,h} the screen rectangle the root window maps onto (the
     * renderer supplies this — e.g. the topbar spans the full viewport width).
     * `sizeOf(spriteName)` -> {w,h}|null lets a sprite supply a node's intrinsic size.
     */
    layout(windowNode, rootBox, sizeOf) {
      const placed = [];
      sizeOf = sizeOf || (() => null);

      const visit = (raw, parent, depth) => {
        const n = this.norm(raw);
        const a = orientAnchor(n.orientation);

        // intrinsic size: explicit size -> else sprite native size -> else 0 (text
        // nodes get a width from maxWidth; sprites get native dims).
        let w = null, h = null;
        if (n.size) {
          w = resolveExtent(n.size.width, parent.w);
          h = resolveExtent(n.size.height, parent.h);
        }
        if ((w == null || h == null) && n.sprite) {
          const sz = sizeOf(n.sprite);
          if (sz) { if (w == null) w = sz.w; if (h == null) h = sz.h; }
        }
        if (w == null && n.maxWidth != null) w = n.maxWidth;
        if (h == null && n.maxHeight != null) h = n.maxHeight;
        if (w == null) w = 0;
        if (h == null) h = 0;

        // anchor point inside the parent box, then add this node's offset.
        const anchorX = parent.x + a.ax * parent.w;
        const anchorY = parent.y + a.ay * parent.h;
        const x = anchorX + (n.position.x || 0);
        const y = anchorY + (n.position.y || 0);

        const box = { node: n, x, y, w, h, depth };
        placed.push(box);

        // children resolve against THIS node's box.
        const childParent = { x, y, w: w || parent.w, h: h || parent.h };
        for (const c of n.children) {
          // skip pure geometry/markup sub-blocks that carry no drawable content.
          const ct = (c.type || "").toLowerCase();
          if (ct === "slotsize" || ct === "max_slots" || ct === "bordersize" ||
              ct === "padding" || ct === "margin" || ct === "positiontype" ||
              ct === "tilesize" || ct === "show_position") continue;
          visit(c, childParent, depth + 1);
        }
      };

      visit(windowNode, rootBox, 0);
      return placed;
    },

    // expose helpers for callers/tests
    _resolveExtent: resolveExtent,
    _orientAnchor: orientAnchor,
  };

  global.GuiParser = GuiParser;
})(window);
