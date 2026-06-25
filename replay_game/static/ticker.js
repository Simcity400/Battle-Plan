/* ticker.js — the event ticker / log (PLAN §H).
 *
 * Renders timeline.json chronologically with source + confidence badges. Honest:
 * if the timeline is empty/absent (e.g. a single-snapshot replay has no diffs)
 * the ticker shows a clear "no events" note and disappears cleanly (Codex F).
 * Clicking an event jumps the scrubber to its hour_index and focuses the country.
 */
(function (global) {
  "use strict";
  const E = UI.el;

  const Ticker = {
    root: null, list: null, filterType: null, filterCountry: null, upToFrame: true,
    onJump: null,  // (hour_index, subjectTag) => void
    events: [],

    init(container) {
      this.root = E("div", "ticker");
      const head = E("div", "ticker-head");
      head.appendChild(E("span", "ticker-title", "Event ticker"));
      this.typeSel = E("select", "ticker-sel"); this.typeSel.appendChild(opt("", "All types"));
      this.countrySel = E("select", "ticker-sel"); this.countrySel.appendChild(opt("", "All countries"));
      const chk = E("label", "ticker-chk");
      const cb = E("input"); cb.type = "checkbox"; cb.checked = true;
      cb.addEventListener("change", () => { this.upToFrame = cb.checked; this.render(); });
      chk.appendChild(cb); chk.appendChild(document.createTextNode(" up to current frame"));
      this.typeSel.addEventListener("change", () => { this.filterType = this.typeSel.value || null; this.render(); });
      this.countrySel.addEventListener("change", () => { this.filterCountry = this.countrySel.value || null; this.render(); });
      head.appendChild(this.typeSel); head.appendChild(this.countrySel); head.appendChild(chk);
      this.count = E("span", "ticker-count");
      head.appendChild(this.count);
      this.root.appendChild(head);
      this.list = E("div", "ticker-list");
      this.root.appendChild(this.list);
      container.appendChild(this.root);
      return this;
    },

    setEvents(events) {
      this.events = (events || []).slice().sort((a, b) => (a.hour_index || 0) - (b.hour_index || 0));
      // populate filters
      const types = new Set(), tags = new Set();
      for (const ev of this.events) { if (ev.type) types.add(ev.type); for (const t of [ev.subject, ev.object]) if (t) tags.add(t); }
      for (const t of [...types].sort()) this.typeSel.appendChild(opt(t, t));
      for (const t of [...tags].sort()) this.countrySel.appendChild(opt(t, t));
      this.render();
    },

    setFrameHour(hour) { this.curHour = hour; if (this.upToFrame) this.render(); },

    render() {
      const list = UI.clear(this.list);
      let shown = 0;
      if (!this.events.length) {
        list.appendChild(noev("No events. A single-snapshot replay has no diffs to derive a timeline from (SCHEMA §9); add more snapshots to populate the ticker."));
        this.count.textContent = "";
        return;
      }
      for (const ev of this.events) {
        if (this.filterType && ev.type !== this.filterType) continue;
        if (this.filterCountry && ev.subject !== this.filterCountry && ev.object !== this.filterCountry) continue;
        if (this.upToFrame && this.curHour != null && (ev.hour_index || 0) > this.curHour) continue;
        list.appendChild(this.row(ev)); shown++;
      }
      if (!shown) list.appendChild(noev("No events match the current filter / frame."));
      this.count.textContent = shown + " event" + (shown === 1 ? "" : "s");
    },

    row(ev) {
      const r = E("div", "ev-row");
      r.appendChild(E("span", "ev-date", ev.date || ""));
      r.appendChild(UI.srcBadge(ev.source, "source: " + (ev.source || "?")));
      const conf = E("span", "ev-conf conf-" + (ev.confidence || ""), ev.confidence || "");
      r.appendChild(conf);
      r.appendChild(E("span", "ev-type", (ev.type || "").replace(/_/g, " ")));
      const desc = [];
      if (ev.subject) desc.push(ev.subject);
      // object = counterpart/occupied-tag for the new directed events
      // (lend_lease_*: counterpart; resistance/compliance_*: occupied tag). Show it
      // so a directed event reads as "GER → POL" rather than just "GER".
      if (ev.object && ev.object !== ev.subject) desc.push("→ " + ev.object);
      if (ev.key) desc.push("· " + ev.key);
      if (ev.from != null || ev.to != null) desc.push(`(${ev.from ?? "—"} → ${ev.to ?? "—"})`);
      r.appendChild(E("span", "ev-desc", desc.join(" ")));
      r.style.cursor = "pointer";
      r.addEventListener("click", () => { if (this.onJump) this.onJump(ev.hour_index, ev.subject); });
      return r;
    },
  };

  function opt(v, label) { const o = E("option", null, label); o.value = v; return o; }
  function noev(text) { return E("div", "ticker-empty", text); }

  global.Ticker = Ticker;
})(window);
