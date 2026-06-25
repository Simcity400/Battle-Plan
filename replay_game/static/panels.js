/* panels.js — State/province (§D), Division (§E), Combat (§G), Event ticker (§H).
 *
 * These open in a right-side drawer. Combat progress is handled per Codex C:
 * raw value shown verbatim; a bipolar bar is drawn ONLY when every observed
 * combat progress in the loaded snapshots lies within [-1, 1] (the schema's
 * declared bar range). Otherwise: "Progress: <raw>" + neutral indicator. Never
 * a fabricated percent.
 */
(function (global) {
  "use strict";
  const S = () => global.Sprites;
  const E = UI.el;
  const T = () => global.NatTable;

  const Drawer = {
    root: null, body: null, titleEl: null,
    init(container) {
      this.root = E("div", "drawer");
      const head = E("div", "drawer-head");
      this.titleEl = E("span", "drawer-title", "");
      const close = E("button", "drawer-close", "×");
      close.addEventListener("click", () => this.hide());
      head.appendChild(this.titleEl); head.appendChild(close);
      this.body = E("div", "drawer-body");
      this.root.appendChild(head); this.root.appendChild(this.body);
      container.appendChild(this.root);
      return this;
    },
    open(title, builder) {
      this.titleEl.textContent = title;
      const b = UI.clear(this.body);
      builder(b);
      this.root.classList.add("open");
    },
    hide() { this.root.classList.remove("open"); },
  };

  // ---- State / province view (§D) -----------------------------------------
  function showState(snap, stateId) {
    Drawer.open("State " + stateId, (b) => {
      const st = (snap.states || {})[stateId];
      if (!st) { b.appendChild(UI.notCaptured("State " + stateId)); return; }
      const g = (p) => Store.get(st, p);
      const head = E("div", "panel");
      head.appendChild(E("h3", null, "Ownership")).appendChild(UI.srcBadge("M/S"));
      const ow = E("div", "stat"); ow.appendChild(E("span", "stat-label", "Owner"));
      if (st.owner) ow.appendChild(S().flag(st.owner, null, "small"));
      ow.appendChild(E("span", "val", st.owner || "—")); head.appendChild(ow);
      const ct = E("div", "stat"); ct.appendChild(E("span", "stat-label", "Controller"));
      if (st.controller) ct.appendChild(S().flag(st.controller, null, "small"));
      ct.appendChild(E("span", "val", st.controller || "—")); head.appendChild(ct);
      head.appendChild(UI.stat("Category", g("category"), { src: "M/S" }));
      head.appendChild(UI.stat("Victory points", g("victory_points_total"), { src: "S" }));
      head.appendChild(UI.stat("Demilitarized", g("demilitarized"), {}));
      b.appendChild(head);

      // buildings with icons
      const bp = E("div", "panel");
      bp.appendChild(E("h3", null, "Buildings")).appendChild(UI.srcBadge("S / M*"));
      const all = Object.assign({}, st.buildings || {}, st.province_buildings || {});
      if (!Object.keys(all).length) bp.appendChild(UI.notCaptured("Buildings"));
      else {
        const tray = E("div", "bld-tray");
        for (const [name, lvl] of Object.entries(all)) {
          const cell = E("div", "bld-cell");
          const cand = bldSprite(name);
          cell.appendChild(S().has(cand) ? S().icon(cand, { size: { w: 28, h: 28 }, title: name }) : E("span", "bld-noicon", "▣"));
          cell.appendChild(E("span", "bld-name", name.replace(/_/g, " ")));
          cell.appendChild(E("span", "bld-lvl", lvl));
          tray.appendChild(cell);
        }
        bp.appendChild(tray);
      }
      b.appendChild(bp);

      // resources + manpower
      const rp = E("div", "panel");
      rp.appendChild(E("h3", null, "Resources & manpower")).appendChild(UI.srcBadge("S"));
      if (st.resources) for (const [r, v] of Object.entries(st.resources)) rp.appendChild(UI.stat(r, { value: v, captured: true }, {}));
      else rp.appendChild(UI.notCaptured("Resources"));
      if (st.manpower_pool) for (const [k, v] of Object.entries(st.manpower_pool)) rp.appendChild(UI.stat("manpower " + k, { value: v, captured: v != null }, {}));
      b.appendChild(rp);

      // occupation governance (resistance / compliance + occupation_law sibling)
      if (st.occupation || st.occupation_law != null) {
        const op = E("div", "panel");
        op.appendChild(E("h3", null, "Occupation governance")).appendChild(UI.srcBadge("S"));
        const occ = st.occupation || {};
        // occupier / occupied parties first (context for the bands)
        if (occ.occupied_country_tag != null)
          op.appendChild(UI.stat("Occupied country", { value: occ.occupied_country_tag, captured: true }, {}));
        if (occ.operational_status != null)
          op.appendChild(UI.stat("Operational status", { value: occ.operational_status, captured: true }, {}));
        // occupation_law is a SIBLING of occupation (states[id].occupation_law)
        op.appendChild(UI.stat("Occupation law", Store.get(st, "occupation_law"), { src: "S" }));
        // resistance / compliance with their speeds — the band data the diff engine
        // emits resistance_changed/compliance_changed events from.
        op.appendChild(occBand("Resistance", occ.resistance, occ.resistance_speed));
        op.appendChild(occBand("Compliance", occ.compliance, occ.compliance_speed));
        b.appendChild(op);
      }

      // divisions present in this state
      const here = (snap.divisions || []).filter((d) => d.state_id === stateId);
      if (here.length) {
        const dp = E("div", "panel");
        dp.appendChild(E("h3", null, `Divisions here (${here.length})`)).appendChild(UI.srcBadge("M/S"));
        const t = T().table(["Owner", "Template", "Org", "Str"]);
        for (const d of here.slice(0, 40)) {
          const tr = T().row([d.owner || "?", d.template_name || d.template_role || "?",
            d.organisation != null ? UI.num(d.organisation) : "—",
            d.strength != null ? UI.num(d.strength) : "—"]);
          tr.style.cursor = "pointer";
          tr.addEventListener("click", () => showDivision(snap, d));
          t.body.appendChild(tr);
        }
        dp.appendChild(t.table);
        b.appendChild(dp);
      }
    });
  }

  /** A resistance/compliance row with a 0..100 bar + speed (captured-aware). */
  function occBand(label, value, speed) {
    const row = E("div", "stat");
    row.appendChild(E("span", "stat-label", label));
    if (value == null) { row.appendChild(UI.value({ captured: false })); return row; }
    const v = Math.max(0, Math.min(100, value));
    const bar = E("span", "occ-bar");
    const fill = E("span", "occ-bar-fill" + (label === "Resistance" ? " resist" : " comply"));
    fill.style.width = v + "%";
    bar.appendChild(fill);
    row.appendChild(bar);
    row.appendChild(E("span", "val", UI.num(value)));
    if (speed != null) {
      const sp = E("span", "occ-speed", (speed > 0 ? "▲" : speed < 0 ? "▼" : "■") + " " + UI.num(speed));
      sp.title = label + " speed (per-day drift)";
      row.appendChild(sp);
    }
    return row;
  }

  function bldSprite(name) {
    // Verified stable building icons (no GFX_building_* family exists in this
    // asset set; the 31-frame GFX_buildings_strip has no published frame map, so
    // we use named texticons instead of guessing a frame — honest, never wrong).
    const map = {
      infrastructure: "GFX_construction_speed_infrastructure_texticon",
      industrial_complex: "GFX_civ_factory",
      arms_factory: "GFX_mil_factory",
      dockyard: "GFX_dockyard_icon",
      air_base: "GFX_construction_speed_airbase_texticon",
      naval_base: "GFX_construction_speed_navalbase_texticon",
      land_fort: "GFX_construction_speed_landfort_texticon",
      coastal_fort: "GFX_coastal_fort_texticon",
      anti_air_building: "GFX_anti_air",
      anti_air: "GFX_anti_air",
      radar_station: "GFX_construction_speed_radar_texticon",
      rocket_site: "GFX_construction_speed_rocket_texticon",
      nuclear_reactor: "GFX_nuclear_reactor_texticon",
      synthetic_refinery: "GFX_oil",
      fuel_silo: "GFX_fuel_state",
    };
    return map[name] || ("GFX_" + name); // a real GFX_<name> sometimes exists; else placeholder
  }

  // ---- Division view (§E) -------------------------------------------------
  function showDivision(snap, d) {
    Drawer.open("Division " + d.div_id, (b) => {
      const head = E("div", "panel");
      const h = E("h3", null, d.template_name || d.template_role || ("Division " + d.div_id));
      h.appendChild(UI.srcBadge("M/S")); head.appendChild(h);
      if (d.owner) { const o = E("div", "stat"); o.appendChild(E("span", "stat-label", "Owner")); o.appendChild(S().flag(d.owner, null, "small")); o.appendChild(E("span", "val", d.owner)); head.appendChild(o); }
      head.appendChild(UI.stat("Role", Store.get(d, "template_role"), { src: "S" }));
      head.appendChild(UI.stat("Organisation", Store.get(d, "organisation"), { src: "M" }));
      head.appendChild(UI.stat("Strength", Store.get(d, "strength"), { src: "M/S", srcTitle: "channel-specific scale (SCHEMA §6)" }));
      head.appendChild(UI.stat("Province", Store.get(d, "location_province"), { src: "M/S" }));
      head.appendChild(UI.stat("State", Store.get(d, "state_id"), {}));
      head.appendChild(UI.stat("In combat", Store.get(d, "in_combat"), { src: "M*" }));
      head.appendChild(UI.stat("Moving", Store.get(d, "moving"), { src: "M*" }));
      b.appendChild(head);

      const ep = E("div", "panel");
      ep.appendChild(E("h3", null, "Equipment")).appendChild(UI.srcBadge("M/S"));
      const eq = d.equipment || [];
      if (!eq.length) ep.appendChild(UI.notCaptured("Equipment"));
      else {
        const tray = E("div", "equip-tray");
        for (const it of eq) {
          const cell = E("div", "equip-cell");
          const sprite = "GFX_" + (it.archetype || "");
          cell.appendChild(S().has(sprite) ? S().icon(sprite, { size: { w: 32, h: 24 }, title: it.archetype }) : E("span", "equip-noicon", "□"));
          cell.appendChild(E("span", "equip-amt", UI.num(it.amount)));
          cell.title = (it.archetype || "?") + ": " + UI.num(it.amount);
          tray.appendChild(cell);
        }
        ep.appendChild(tray);
      }
      b.appendChild(ep);
    });
  }

  // ---- Combat view (§G) — Codex C raw-progress handling -------------------
  // "sticky-unsafe" gate: the bar is enabled only while EVERY combat progress
  // observed across all loaded snapshots so far lies within the schema's declared
  // [-1,1]. We recompute live against whatever is loaded (not a one-time memo) and,
  // crucially, once we ever see an out-of-range value the bar stays disabled for the
  // session — so a later out-of-range frame can never be "missed" after the bar was
  // shown (Codex). Default before any data: unsafe (raw only). No fabricated scale.
  let _everUnsafe = false;
  function progressBarIsSafe(allSnaps) {
    let sawAny = false;
    for (const snap of allSnaps) {
      for (const cb of (snap.combats || [])) {
        if (cb.progress == null) continue;
        sawAny = true;
        if (cb.progress < -1.0001 || cb.progress > 1.0001) { _everUnsafe = true; }
      }
    }
    return sawAny && !_everUnsafe;
  }
  function resetProgressSafety() { _everUnsafe = false; }

  function showCombat(snap, cb, allSnaps) {
    Drawer.open("Combat " + (cb.combat_id || ""), (b) => {
      const head = E("div", "panel");
      const h = E("h3", null, (cb.kind || "land") + " combat");
      h.appendChild(UI.srcBadge("M*", "combat domain — land cracked, naval/air M*")); head.appendChild(h);
      head.appendChild(UI.stat("Location province", Store.get(cb, "location_province"), { src: "M*" }));
      head.appendChild(UI.stat("Terrain", Store.get(cb, "terrain"), { src: "M*" }));
      head.appendChild(UI.stat("Combat width", Store.get(cb, "combat_width"), { src: "M*" }));

      // progress: raw + (conditional) bipolar bar
      const pr = E("div", "stat");
      pr.appendChild(E("span", "stat-label", "Progress (raw)"));
      if (cb.progress == null) pr.appendChild(UI.value({ captured: false }));
      else {
        pr.appendChild(E("span", "val", UI.num(cb.progress)));
        if (progressBarIsSafe(allSnaps || [snap])) {
          // safe to draw a bipolar -1..1 bar
          const bar = E("span", "combat-bar");
          const fill = E("span", "combat-bar-fill");
          const v = Math.max(-1, Math.min(1, cb.progress));
          fill.style.left = "50%";
          fill.style.width = Math.abs(v) * 50 + "%";
          if (v < 0) fill.style.transform = "translateX(-100%)";
          fill.classList.add(v >= 0 ? "atk" : "def");
          bar.appendChild(fill);
          pr.appendChild(bar);
        } else {
          const note = E("span", "raw-note", "raw — bar disabled (values outside −1..1)");
          note.title = "Schema declares progress −1..1, but observed values exceed it; showing raw, not a fabricated scale (Codex).";
          pr.appendChild(note);
        }
      }
      head.appendChild(pr);
      b.appendChild(head);

      // participants — resolve owners in THIS snapshot only (Codex B)
      for (const [side, units] of [["Attackers", cb.attackers], ["Defenders", cb.defenders]]) {
        const p = E("div", "panel");
        p.appendChild(E("h3", null, side)).appendChild(UI.srcBadge("M*"));
        const byId = new Map((snap.divisions || []).map((d) => [d.div_id, d]));
        const known = {}, unresolved = [];
        for (const u of (units || [])) {
          const id = Array.isArray(u) ? u[0] : u;
          const d = byId.get(id);
          if (d && d.owner) { (known[d.owner] = known[d.owner] || []).push(d); }
          else unresolved.push(id);
        }
        if (!Object.keys(known).length && !unresolved.length) p.appendChild(UI.notCaptured(side));
        else {
          const tray = E("div", "diplo-tray");
          for (const [tag, ds] of Object.entries(known)) {
            const chip = E("span", "diplo-chip");
            chip.appendChild(S().flag(tag, null, "small"));
            chip.appendChild(E("span", null, ` ${tag} ×${ds.length}`));
            tray.appendChild(chip);
          }
          if (unresolved.length) {
            const chip = E("span", "diplo-chip unresolved", `? ×${unresolved.length}`);
            chip.title = "Division id(s) not present in this snapshot — owner not captured (no inference).";
            tray.appendChild(chip);
          }
          p.appendChild(tray);
          // detail rows
          const t = T().table(["Division id", "Owner", "Template", "Org", "Str"]);
          for (const [tag, ds] of Object.entries(known)) for (const d of ds)
            t.body.appendChild(T().row([d.div_id, tag, d.template_name || d.template_role || "?",
              d.organisation != null ? UI.num(d.organisation) : "—",
              d.strength != null ? UI.num(d.strength) : "—"]));
          for (const id of unresolved)
            t.body.appendChild(T().row([id, T().muted("not captured"), T().muted("—"), "—", "—"]));
          p.appendChild(t.table);
        }
        b.appendChild(p);
      }

      // modifiers (§7) — captured obj|null with UNSPECIFIED keys. Honor no-fabrication
      // (Codex): render a raw key/value table of whatever scalar keys are present; do
      // NOT interpret tokens like day/duration semantically (the schema does not define
      // them). Nested values fall back to a compact "captured complex value" cell.
      const modP = UI.presence(cb, "modifiers");
      const mp = E("div", "panel");
      mp.appendChild(E("h3", null, "Combat modifiers")).appendChild(UI.srcBadge("M*"));
      const combatsCap = (function () {
        const pr = (snap && snap.provenance) || {};
        for (const ch of ["memory", "save"]) {
          const c = pr[ch];
          if (c && Array.isArray(c.domains) && c.domains.indexOf("combats") >= 0) return true;
        }
        return false;
      })();
      // Schema: modifiers obj|null, M* — `null` is the "not cracked / not captured"
      // default (SCHEMA §7), NOT a measured empty. So only an actual present object
      // distinguishes captured-empty ({} -> "none (captured)") from absent/null/
      // malformed-scalar (all -> "not captured"). (Codex.)
      if (modP.state !== "value" || typeof modP.value !== "object") {
        mp.appendChild(UI.notCaptured("Combat modifiers"));
      } else {
        const keys = Object.keys(modP.value).sort();
        if (!keys.length) mp.appendChild(UI.emptyList("Combat modifiers", combatsCap));
        else {
          const t = T().table(["Modifier", "Value"]);
          for (const k of keys) {
            const v = modP.value[k];
            let cellVal;
            if (v == null) cellVal = T().muted("not captured");
            else if (typeof v === "object") {
              const s = E("span", "val-missing", "captured complex value");
              try { s.title = JSON.stringify(v); } catch (e) { /* ignore */ }
              cellVal = s;
            } else cellVal = typeof v === "number" ? UI.num(v) : String(v);
            // key shown VERBATIM — the note promises raw, uninterpreted keys (Codex).
            t.body.appendChild(T().row([k, cellVal]));
          }
          mp.appendChild(t.table);
          mp.appendChild(E("div", "note", "Raw captured modifier keys/values shown verbatim; not interpreted (modifier keys are unspecified in SCHEMA §7)."));
        }
      }
      b.appendChild(mp);
    });
  }

  // ---- Naval / Air REGION report (§13 naval_air) --------------------------
  // Codex 2 (HARD): naval_air.{region_control,air_superiority} are keyed by
  // STRATEGIC_REGION_ID and there is NO region->map geometry asset, so we surface
  // them as an explicit LIST/REPORT (never painted onto the raster — that would
  // fabricate spatial precision). The whole report is PROXY-badged.
  function showRegionReport(snap, mode) {
    const na = (snap && snap.naval_air) || {};
    const isAir = mode.report === "air";
    const title = isAir ? "Air superiority — region report" : "Naval dominance — region report";
    Drawer.open(title, (b) => {
      // proxy banner — region data is NEVER authoritative
      const banner = E("div", "proxy-banner");
      banner.appendChild(E("span", "legend-proxy-chip", "PROXY / ESTIMATE"));
      if (na.confidence) banner.appendChild(E("span", "legend-conf", na.confidence));
      banner.appendChild(E("span", "legend-note",
        "Per STRATEGIC REGION. No region→map geometry is loaded, so this is a list, not a painted map. " +
        (na.source ? "Source: " + na.source + " (live)." : "")));
      b.appendChild(banner);

      const data = isAir ? na.air_superiority : na.region_control;
      if (!data || typeof data !== "object" || !Object.keys(data).length) {
        b.appendChild(UI.notCaptured(isAir ? "Air superiority" : "Naval dominance"));
        return;
      }

      if (isAir) {
        // {region_id: {by_owner:{TAG:{effective_planes,wings}}, leader, total}}
        const p = E("div", "panel");
        p.appendChild(E("h3", null, `Regions contested (${Object.keys(data).length})`)).appendChild(UI.srcBadge("M"));
        const t = T().table(["Region", "Leader", "Σ eff. planes", "Owners (eff. planes / wings)"]);
        const rids = Object.keys(data).sort((a, c) => +a - +c);
        for (const rid of rids) {
          const r = data[rid] || {};
          const by = r.by_owner || {};
          const owners = Object.keys(by)
            .sort((a, c) => (by[c].effective_planes || 0) - (by[a].effective_planes || 0))
            .map((tag) => `${tag}: ${UI.num(by[tag].effective_planes || 0)}/${by[tag].wings || 0}`)
            .join(", ");
          t.body.appendChild(T().row([
            "region " + rid,
            r.leader || T().muted("—"),
            r.total != null ? UI.num(r.total) : T().muted("—"),
            owners || T().muted("—"),
          ]));
        }
        p.appendChild(t.table);
        p.appendChild(E("div", "note", "Air superiority proxied by summed effective_planes per (region, owner); the exact superiority share is a drifting runtime cache (SCHEMA §13)."));
        b.appendChild(p);
      } else {
        // {region_id: [ {controller, current, target, ...} ]}
        const p = E("div", "panel");
        p.appendChild(E("h3", null, `Sea zones contested (${Object.keys(data).length})`)).appendChild(UI.srcBadge("M"));
        const t = T().table(["Region", "Dominant", "Current ratio", "Contestants"]);
        const rids = Object.keys(data).sort((a, c) => +a - +c);
        for (const rid of rids) {
          const ents = Array.isArray(data[rid]) ? data[rid].slice() : [];
          ents.sort((a, c) => (c.current || 0) - (a.current || 0));
          const top = ents[0] || {};
          const contestants = ents
            .map((e) => `${e.controller || "?"} ${UI.num(e.current || 0)}`)
            .join(", ");
          t.body.appendChild(T().row([
            "region " + rid,
            top.controller || T().muted("—"),
            top.current != null ? UI.num(top.current) : T().muted("—"),
            contestants || T().muted("—"),
          ]));
        }
        p.appendChild(t.table);
        p.appendChild(E("div", "note", "Sea-zone dominance 'current' is the engine's per-region dominance ratio; regions with no contest are omitted."));
        b.appendChild(p);
      }
    });
  }

  global.Panels = { Drawer, showState, showDivision, showCombat, showRegionReport, resetProgressSafety };
})(window);
