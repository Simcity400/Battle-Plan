/* topbar.js — the game top resource/status bar (PLAN §A1).
 *
 * Built from the REAL topbar sprite names (game_assets/gui/topbar.json) so the
 * icons match the game; laid out as a responsive strip backed by the real
 * GFX_topbar_background_extended chrome. Every figure is bound generically to
 * the snapshot and renders "not captured" when null (no fabrication).
 *
 * Items (sprite -> field), per topbar.json:
 *   player_flag        flag(TAG, ideology)
 *   pol_power_icon     politics.political_power
 *   stability_icon     politics.stability (0..1, + progress bar)
 *   war_support_icon   politics.war_support (0..1, + progress bar)
 *   manpower_icon      military.manpower.available
 *   command_power      politics.command_power
 *   industrial_capacity_icon  civ/mil/dockyard factories
 *   fuel_state         economy.fuel.current / .max  (+ ratio bar)
 *   topbar_convoys     military.convoys[].length
 *   resources strip    economy.resources.available_derived.*
 */
(function (global) {
  "use strict";

  const S = () => global.Sprites;
  const RES_ORDER = ["steel", "oil", "aluminium", "tungsten", "chromium", "rubber", "coal"];

  function iconWrap(spriteName, opts) {
    const w = UI.el("span", "tb-icon");
    w.appendChild(S().icon(spriteName, opts || {}));
    return w;
  }

  function item(spriteName, label, got, fmt, opts) {
    opts = opts || {};
    const it = UI.el("span", "tb-item");
    if (spriteName) it.appendChild(iconWrap(spriteName, { title: label, size: opts.size }));
    const v = UI.el("span", "tb-val");
    if (!got || !got.captured) { v.textContent = "—"; v.classList.add("tb-missing"); v.title = label + " — not captured"; }
    else { v.textContent = fmt ? fmt(got.value) : UI.num(got.value); v.title = label; }
    it.appendChild(v);
    it.title = label;
    return it;
  }

  const Topbar = {
    el: null,
    init(container) {
      this.el = UI.el("div", "topbar");
      S().nineSlice ? null : null;
      // background chrome
      const bg = S().rec("GFX_topbar_background_extended");
      if (bg && bg.png && bg.png[0]) {
        this.el.style.backgroundImage = "url(/game_assets/" + bg.png[0].replace(/^\/+/, "") + ")";
        this.el.style.backgroundRepeat = "repeat-x";
      }
      container.appendChild(this.el);
      return this;
    },

    render(snap, tag) {
      const root = UI.clear(this.el);
      if (!snap || !tag) return;
      const c = (snap.countries || {})[tag] || {};
      const pol = c.politics || {}, eco = c.economy || {}, mil = c.military || {};
      const g = (o, p) => Store.get(o, p);

      // flag + name
      const flagWrap = UI.el("span", "tb-flagwrap");
      flagWrap.appendChild(S().flag(tag, pol.ruling_party, "medium"));
      const nm = UI.el("span", "tb-tag", tag);
      flagWrap.appendChild(nm);
      root.appendChild(flagWrap);

      // PP / stability / war support / CP
      root.appendChild(item("GFX_pol_power_icon", "Political power", g(pol, "political_power")));
      root.appendChild(this.gauge("GFX_stability_icon", "Stability", g(pol, "stability")));
      root.appendChild(this.gauge("GFX_war_support_icon", "War support", g(pol, "war_support")));
      root.appendChild(item("GFX_command_power", "Command power", g(pol, "command_power")));

      // factories: civ / mil / dock
      root.appendChild(item("GFX_civ_factory", "Civilian factories", g(eco, "factories.civ")));
      root.appendChild(item("GFX_mil_factory", "Military factories", g(eco, "factories.mil")));
      root.appendChild(item("GFX_dockyard_icon", "Dockyards", g(eco, "factories.dockyard")));

      // manpower
      root.appendChild(item("GFX_manpower_icon", "Available manpower", g(mil, "manpower.available")));

      // convoys (count)
      const convoys = (mil.convoys && Array.isArray(mil.convoys)) ?
        { value: mil.convoys.length, captured: true } : { value: null, captured: false };
      root.appendChild(item("GFX_topbar_convoys", "Convoys", convoys));

      // fuel (current / max + ratio bar)
      const fc = g(eco, "fuel.current"), fm = g(eco, "fuel.max");
      const fuelItem = item("GFX_fuel_state", "Fuel", fc);
      if (fc.captured && fm.captured && fm.value > 0) {
        const bar = S().progressBar("GFX_topbar_fuel_bar", fc.value / fm.value,
          { title: "Fuel " + UI.num(fc.value) + " / " + UI.num(fm.value), w: 50, h: 7 });
        fuelItem.appendChild(bar);
      }
      root.appendChild(fuelItem);

      // resources strip
      root.appendChild(this.resources(eco));

      // date + scrub note (transport is wired separately; show the date here)
      const dt = UI.el("span", "tb-date");
      dt.textContent = snap.date || "—";
      dt.title = "snapshot date";
      root.appendChild(dt);
    },

    gauge(sprite, label, got) {
      const it = UI.el("span", "tb-item tb-gauge");
      it.appendChild(iconWrap(sprite, { title: label }));
      const bar = S().progressBar("GFX_topbar_fuel_bar", got.captured ? got.value : null,
        { title: label, w: 46, h: 7 });
      it.appendChild(bar);
      const v = UI.el("span", "tb-val");
      v.textContent = got.captured ? UI.pct(got.value) : "—";
      if (!got.captured) v.classList.add("tb-missing");
      it.appendChild(v);
      it.title = label + (got.captured ? ": " + UI.pct(got.value) : " — not captured");
      return it;
    },

    resources(eco) {
      const wrap = UI.el("span", "tb-res");
      const derived = (eco.resources && eco.resources.available_derived) || null;
      if (!derived) {
        const m = UI.el("span", "tb-missing", "resources n/c");
        m.title = "Resources — not captured";
        wrap.appendChild(m);
        return wrap;
      }
      for (let i = 0; i < RES_ORDER.length; i++) {
        const r = RES_ORDER[i];
        const v = derived[r];
        if (v == null) continue;
        const it = UI.el("span", "tb-res-item");
        // Use the NAMED per-resource icon (GFX_steel/GFX_oil/...) so the icon is
        // guaranteed correct — the 7-frame GFX_resources_strip has no published
        // frame->resource map, and guessing a frame could show the wrong resource
        // (a fabrication). chromium has no standalone icon -> labelled placeholder.
        it.appendChild(S().resourceIcon(r, { title: r, size: { w: 16, h: 16 } }));
        it.appendChild(UI.el("span", "tb-res-val", UI.num(v)));
        it.title = r + ": " + UI.num(v);
        wrap.appendChild(it);
      }
      return wrap;
    },
  };

  global.Topbar = Topbar;
})(window);
