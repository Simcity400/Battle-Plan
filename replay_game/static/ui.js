/* ui.js — shared rendering helpers, centered on the captured/derived/raw/unknown
 * distinction (Codex's #1 risk: "build a visible distinction from day one").
 *
 * Every value displayed in the game UI goes through one of these so the user can
 * always tell measured data from a gap from a derived/raw figure.
 */
(function (global) {
  "use strict";

  const UI = {
    el(tag, cls, text) {
      const e = document.createElement(tag);
      if (cls) e.className = cls;
      if (text != null) e.textContent = text;
      return e;
    },

    /** A small source badge (M/S/D/marker). */
    srcBadge(src, title) {
      const b = UI.el("span", "srcbadge src-" + (src || "").replace(/[^a-z]/gi, "").toLowerCase(), src);
      if (title) b.title = title;
      return b;
    },

    /**
     * Render a leaf value honestly. {value, captured} from Store.get.
     * captured=false -> a muted "not captured" chip (NEVER 0/blank that reads as data).
     * fmt is an optional formatter for captured values.
     */
    value(got, fmt) {
      if (!got || !got.captured) {
        return UI.el("span", "val-missing", "not captured");
      }
      const v = got.value;
      const span = UI.el("span", "val");
      span.textContent = fmt ? fmt(v) : (typeof v === "number" ? UI.num(v) : String(v));
      return span;
    },

    /** A labelled stat row: label + value(+optional source badge). */
    stat(label, got, opts) {
      opts = opts || {};
      const row = UI.el("div", "stat");
      row.appendChild(UI.el("span", "stat-label", label));
      const valEl = UI.value(got, opts.fmt);
      row.appendChild(valEl);
      if (opts.src) row.appendChild(UI.srcBadge(opts.src, opts.srcTitle));
      return row;
    },

    num(v) {
      if (v == null) return "—";
      if (Math.abs(v) >= 1e6) return (v / 1e6).toFixed(2) + "M";
      if (Math.abs(v) >= 1e4) return (v / 1e3).toFixed(1) + "k";
      if (Number.isInteger(v)) return String(v);
      return (Math.round(v * 100) / 100).toString();
    },
    pct(v) { return v == null ? "—" : Math.round(v * 100) + "%"; },

    /** A panel with a title + optional source badge. Returns {panel, body}. */
    panel(title, src, srcTitle) {
      const panel = UI.el("div", "panel");
      const h = UI.el("h3", null, title);
      if (src) h.appendChild(UI.srcBadge(src, srcTitle));
      panel.appendChild(h);
      const body = UI.el("div", "panel-body");
      panel.appendChild(body);
      return { panel, body };
    },

    /** "not captured" placeholder block for an empty section. */
    notCaptured(what) {
      const d = UI.el("div", "block-missing");
      d.textContent = (what || "This data") + " — not captured this snapshot.";
      return d;
    },

    /**
     * Empty-list block that distinguishes measured-zero from not-captured using
     * the snapshot provenance domain (Codex: an empty *captured* list means "none",
     * not "missing"). domainCaptured=true -> "none (captured)"; else "not captured".
     */
    emptyList(what, domainCaptured) {
      const d = UI.el("div", "block-missing");
      d.textContent = domainCaptured
        ? (what || "This list") + " — none (captured: this country has zero)."
        : (what || "This data") + " — not captured this snapshot.";
      return d;
    },

    /** Clear a node. */
    clear(n) { while (n.firstChild) n.removeChild(n.firstChild); return n; },

    /** Date string -> friendly. */
    fdate(d) { if (!d) return "—"; const p = d.split("."); return `${p[2]} / ${p[1]} / ${p[0]}`; },

    // ---- presence-aware leaf access (Codex D: null can be MEANINGFUL) ---------
    /**
     * Distinguish three states for a property the schema may set to a meaningful
     * null (e.g. `assignment: null` = reserve; per-country `order_of_battle: null`).
     *   - absent  : the KEY is not on the object  -> "not captured"
     *   - nullSet : key present but value is null  -> caller decides ("none"/"—")
     *   - value   : a real value
     * Returns {state:"absent"|"nullSet"|"value", value}.
     */
    presence(obj, key) {
      if (obj == null || typeof obj !== "object" || !(key in obj))
        return { state: "absent", value: null };
      const v = obj[key];
      return (v === null || v === undefined)
        ? { state: "nullSet", value: null }
        : { state: "value", value: v };
    },

    /**
     * A labelled portrait. `gfx` is a SPRITE NAME (never image data). If the
     * sprite exists -> the real art; else a labelled placeholder bearing the
     * character's name + the missing gfx (a portrait-asset pass is separate —
     * we NEVER fabricate a portrait, and NEVER block). Returns an element.
     */
    portrait(gfx, name, sz) {
      const S = global.Sprites;
      sz = sz || 48;
      const wrap = UI.el("div", "portrait");
      wrap.style.width = sz + "px"; wrap.style.height = Math.round(sz * 1.2) + "px";
      if (gfx && S && S.has(gfx)) {
        wrap.appendChild(S.icon(gfx, { title: name || gfx, cls: "portrait-img" }));
      } else {
        const ph = UI.el("div", "portrait-ph");
        ph.textContent = (name ? name.split(/\s+/).map((w) => w[0]).join("").slice(0, 3).toUpperCase() : "?");
        ph.title = gfx ? (gfx + " — portrait asset not extracted") : "no portrait_gfx captured";
        if (gfx) wrap.classList.add("portrait-pending");
        wrap.appendChild(ph);
      }
      return wrap;
    },

    /** Skill / level as filled stars out of `max` (default 10 — the game leader cap). */
    skillStars(level, max) {
      max = max || 10;
      const wrap = UI.el("span", "skill-stars");
      if (level == null) { wrap.appendChild(UI.value({ captured: false })); return wrap; }
      const n = Math.max(0, Math.min(max, Math.round(level)));
      wrap.title = "level " + level;
      wrap.appendChild(UI.el("span", "stars-on", "★".repeat(n)));
      if (n < max) wrap.appendChild(UI.el("span", "stars-off", "☆".repeat(max - n)));
      wrap.appendChild(UI.el("span", "stars-num", " " + level));
      return wrap;
    },

    /** Tray of small trait chips from a list of trait tokens. */
    traitChips(traits) {
      const tray = UI.el("span", "trait-tray");
      if (!Array.isArray(traits) || !traits.length) { tray.appendChild(UI.el("span", "val-missing", "—")); return tray; }
      for (const t of traits) {
        const chip = UI.el("span", "trait-chip", String(t).replace(/_/g, " "));
        chip.title = t;
        tray.appendChild(chip);
      }
      return tray;
    },
  };

  global.UI = UI;
})(window);
