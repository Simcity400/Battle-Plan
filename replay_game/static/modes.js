/* modes.js — map-mode definitions (PLAN §B.1).
 *
 * Each mode = {id, label, group, perProvince?, spec, colorize(snap, MAP)->{id:[r,g,b]}}.
 *   - spec: a coverage spec evaluated by Store.modeAvailable so a mode lights up
 *     ONLY with genuine data (Codex D). Modes with no spec are always available
 *     (static base layers like political).
 *   - "Being cracked" modes (supply/rail/intel/naval/air/weather) have a real spec
 *     too — they auto-enable the instant their field appears in the store, with no
 *     rework. Until then Store.modeAvailable returns false and the UI greys them
 *     with a "capturing…" note. NEVER faked.
 *
 * Colorizers return [r,g,b]; map.js packs + paints. Honest: an entity with no
 * data for the active mode is simply omitted from the result and painted UNKNOWN.
 */
(function (global) {
  "use strict";

  // ---- color helpers ------------------------------------------------------
  function ideologyTint(base, ideology) {
    // nudge a country color toward its ideology hue so the political map reads
    // like the game (factions still distinct by base color). Subtle, not faked.
    const t = { fascism: [120, 100, 30], communism: [150, 40, 35],
                democratic: [50, 90, 150], neutrality: base };
    const k = t[ideology];
    if (!k || k === base) return base;
    return [Math.round((base[0] + k[0]) / 2), Math.round((base[1] + k[1]) / 2),
            Math.round((base[2] + k[2]) / 2)];
  }
  function ramp(v, lo, hi, c0, c1) {
    const t = Math.max(0, Math.min(1, (v - lo) / (hi - lo || 1)));
    return [Math.round(c0[0] + (c1[0] - c0[0]) * t),
            Math.round(c0[1] + (c1[1] - c0[1]) * t),
            Math.round(c0[2] + (c1[2] - c0[2]) * t)];
  }
  function hashColor(str) {
    let h = 0; for (let i = 0; i < (str || "").length; i++) h = (h * 31 + str.charCodeAt(i)) | 0;
    return [80 + (Math.abs(h) % 150), 70 + (Math.abs(h >> 3) % 150), 70 + (Math.abs(h >> 6) % 150)];
  }
  function colorOf(MAP, tag) { return (tag && MAP.colors[tag]) || [90, 90, 100]; }
  /** Province controller map, resilient to which mirror a snapshot carries. */
  function controlByProvince(snap) {
    const mcp = snap.map && snap.map.control_by_province;
    if (mcp && Object.keys(mcp).length) return mcp;
    // fall back to provinces[*].controller (the other schema mirror)
    const out = {};
    for (const pid in (snap.provinces || {})) {
      const t = snap.provinces[pid].controller;
      if (t) out[pid] = t;
    }
    return out;
  }
  /** Are world factions captured in this snapshot? */
  function factionsCaptured(snap) {
    const f = snap.diplomacy_world && snap.diplomacy_world.factions;
    return f && Object.keys(f).length > 0;
  }

  // ---- modes --------------------------------------------------------------
  const modes = [
    {
      id: "political", label: "Political", group: "full",
      colorize(snap, MAP) {
        const out = {};
        const obs = (snap.map && snap.map.owner_by_state) || {};
        const states = snap.states || {};
        const src = Object.keys(obs).length ? obs : null;
        const ids = src ? Object.keys(src) : Object.keys(states);
        for (const sid of ids) {
          const owner = src ? src[sid] : (states[sid] || {}).owner;
          if (!owner) continue;
          const ideo = ((snap.countries || {})[owner] || {}).politics
            ? snap.countries[owner].politics.ruling_party : null;
          out[sid] = ideologyTint(colorOf(MAP, owner), ideo);
        }
        return out;
      },
    },
    {
      id: "controller", label: "Controller / occupation", group: "full",
      perProvince: true,
      spec: { scope: "provinces", required: ["controller"], minEntities: 10, minCoverage: 0.2 },
      colorize(snap, MAP) {
        const out = {};
        const pc = controlByProvince(snap);
        for (const pid in pc) { const t = pc[pid]; if (t) out[pid] = colorOf(MAP, t); }
        return out;
      },
    },
    {
      id: "faction", label: "Faction", group: "full",
      colorize(snap, MAP) {
        const out = {};
        // Only paint if factions are actually captured. Otherwise return {} so the
        // mode auto-disables — painting "non-aligned grey" everywhere would imply a
        // measured "no faction" when diplomacy simply wasn't captured (Codex A).
        if (!factionsCaptured(snap)) return out;
        const facOf = {}; // TAG -> faction id
        const fcolor = {};
        const facs = snap.diplomacy_world.factions;
        let i = 0;
        const palette = [[60, 110, 200], [200, 70, 60], [80, 170, 90],
                         [180, 150, 50], [150, 90, 180], [80, 170, 180]];
        for (const fid in facs) {
          fcolor[fid] = palette[i % palette.length]; i++;
          for (const m of (facs[fid].members || [])) facOf[m] = fid;
        }
        const states = snap.states || {};
        for (const sid in states) {
          const owner = states[sid].owner;
          if (!owner) continue;
          const fid = facOf[owner];
          out[sid] = fid ? fcolor[fid] : [70, 70, 80]; // non-aligned (factions ARE captured)
        }
        return out;
      },
    },
    {
      id: "diplomatic", label: "Diplomatic (selected)", group: "full",
      colorize(snap, MAP) {
        const out = {};
        const sel = MAP.selCountry;
        const states = snap.states || {};
        const c = (snap.countries || {})[sel] || {};
        const dip = c.diplomacy || {};
        // Require the selected country's diplomacy to be captured; otherwise return
        // {} (mode disables) rather than painting a fake "all neutral" map (Codex A).
        if (!sel || c.diplomacy == null) return out;
        const enemies = new Set();
        for (const w of (dip.wars || [])) if (w.against) enemies.add(w.against);
        const allies = new Set();
        const myFac = (c.politics && c.politics.faction) ? c.politics.faction.name : null;
        const facs = (snap.diplomacy_world && snap.diplomacy_world.factions) || {};
        if (myFac) for (const fid in facs)
          if ((facs[fid].template || facs[fid].icon || String(facs[fid].id)) === myFac)
            for (const m of (facs[fid].members || [])) allies.add(m);
        for (const sid in states) {
          const owner = states[sid].owner; if (!owner) continue;
          if (owner === sel) out[sid] = [60, 120, 200];
          else if (enemies.has(owner)) out[sid] = [200, 60, 60];
          else if (allies.has(owner)) out[sid] = [70, 160, 90];
          else out[sid] = [90, 90, 100];
        }
        return out;
      },
    },
    {
      id: "vp", label: "Victory points", group: "full",
      spec: { scope: "states", required: ["victory_points_total"], minEntities: 10, minCoverage: 0.2 },
      colorize(snap) {
        const out = {}; const states = snap.states || {};
        let max = 1;
        for (const sid in states) { const v = states[sid].victory_points_total; if (v > max) max = v; }
        for (const sid in states) {
          const v = states[sid].victory_points_total;
          if (v == null) continue;
          out[sid] = v > 0 ? ramp(Math.log10(1 + v), 0, Math.log10(1 + max), [40, 40, 60], [255, 215, 80])
                           : [35, 38, 48];
        }
        return out;
      },
    },
    {
      id: "resources", label: "Resources", group: "full",
      spec: { scope: "states", required: ["resources"], minEntities: 10, minCoverage: 0.1 },
      colorize(snap) {
        const out = {}; const states = snap.states || {};
        let max = 1;
        const tot = (r) => r ? Object.values(r).reduce((a, b) => a + (b || 0), 0) : 0;
        for (const sid in states) { const t = tot(states[sid].resources); if (t > max) max = t; }
        for (const sid in states) {
          if (states[sid].resources == null) continue;
          const t = tot(states[sid].resources);
          out[sid] = t > 0 ? ramp(t, 0, max, [40, 45, 40], [120, 200, 110]) : [35, 38, 40];
        }
        return out;
      },
    },
    {
      id: "manpower", label: "Manpower", group: "full",
      spec: { scope: "states", required: ["manpower_pool"], minEntities: 10, minCoverage: 0.2 },
      colorize(snap) {
        const out = {}; const states = snap.states || {};
        const val = (m) => (m && (m.total != null ? m.total : m.available)) || 0;
        let max = 1;
        for (const sid in states) { const v = val(states[sid].manpower_pool); if (v > max) max = v; }
        for (const sid in states) {
          const m = states[sid].manpower_pool; if (m == null) continue;
          out[sid] = ramp(Math.log10(1 + val(m)), 0, Math.log10(1 + max), [40, 40, 50], [220, 120, 200]);
        }
        return out;
      },
    },
    {
      id: "buildings", label: "Industry / buildings", group: "full",
      spec: { scope: "states", required: ["buildings"], minEntities: 10, minCoverage: 0.2 },
      colorize(snap) {
        const out = {}; const states = snap.states || {};
        const ind = (b) => b ? ((b.industrial_complex || 0) + (b.arms_factory || 0) + (b.dockyard || 0)) : 0;
        let max = 1;
        for (const sid in states) { const v = ind(states[sid].buildings); if (v > max) max = v; }
        for (const sid in states) {
          const b = states[sid].buildings; if (b == null) continue;
          const v = ind(b);
          out[sid] = v > 0 ? ramp(v, 0, max, [45, 45, 55], [255, 170, 60]) : [38, 40, 48];
        }
        return out;
      },
    },
    {
      id: "frontlines", label: "Front lines", group: "full",
      perProvince: true,
      // available when controller differs from owner anywhere (derived fronts)
      spec: { scope: "provinces", required: ["controller"], minEntities: 10, minCoverage: 0.2 },
      colorize(snap, MAP) {
        // color provinces by controller; map.js draws no separate front line, but
        // controller!=owner contrast reads as the front. Honest: pure control map.
        const out = {};
        const pc = controlByProvince(snap);
        const states = snap.states || {};
        const sbp = MAP.stateByProvince;
        for (const pid in pc) {
          const ctrl = pc[pid];
          const sid = (+pid < sbp.length) ? sbp[+pid] : -1;
          const owner = sid >= 0 && states[sid] ? states[sid].owner : null;
          if (ctrl && owner && ctrl !== owner) out[pid] = [230, 70, 60];        // contested
          else if (ctrl) out[pid] = colorOf(MAP, ctrl).map((x) => Math.round(x * 0.6 + 20));
        }
        return out;
      },
    },

    // ---- LEND-LEASE (country-level value painted over owned states) ---------
    // Codex 5: NOT spatial trade geography — a country-level diplomatic value
    // smeared over its owned states. Named + badged as such. Per-country directed
    // entries (incoming/outgoing) are correct and NOT double-counted (the same
    // contract surfaces on both parties, but here we only sum the OWNER's own
    // directed view, never both sides of one contract). Owners with no captured
    // lend_lease are omitted (honest empty), never painted neutral.
    { id: "lend_lease", label: "Lend-Lease (by owner)", group: "full",
      proxy: { kind: "country-choropleth",
               note: "Country-level net lend-lease value painted over each owner's states (NOT spatial trade geography). Source: save market contracts." },
      colorize(snap, MAP) {
        const out = {};
        const countries = snap.countries || {};
        const states = snap.states || {};
        // net = Σ(outgoing equipment amounts) − Σ(incoming) for each country that
        // actually has a captured lend_lease list.
        const net = {};
        for (const tag in countries) {
          const ll = (countries[tag].diplomacy || {}).lend_lease;
          if (!Array.isArray(ll)) continue;          // not captured -> skip (no paint)
          let v = 0;
          for (const c of ll) {
            const vol = contractVolume(c);
            if (c.direction === "outgoing") v += vol;
            else if (c.direction === "incoming") v -= vol;
          }
          net[tag] = v;
        }
        if (!Object.keys(net).length) return out;     // honest empty
        for (const sid in states) {
          const owner = states[sid].owner;
          if (owner == null || !(owner in net)) continue; // owner's LL not captured -> UNKNOWN
          const v = net[owner];
          if (v > 0) out[sid] = [70, 120 + Math.min(110, Math.round(Math.log10(1 + v) * 40)), 90];   // net seller (green)
          else if (v < 0) out[sid] = [120 + Math.min(110, Math.round(Math.log10(1 - v) * 40)), 80, 70]; // net buyer (red)
          else out[sid] = [80, 80, 95];               // measured net-zero (captured) — distinct from UNKNOWN
        }
        return out;
      } },

    // ---- OCCUPATION (resistance) — state-keyed, real state data --------------
    // Codex 6: resistance ramp; states without an occupation object are omitted
    // (honest empty). occupation_law is surfaced in the state drawer (panels.js).
    { id: "occupation_resistance", label: "Occupation (resistance)", group: "full",
      spec: { scope: "states", required: ["occupation"], minEntities: 1, minCoverage: 0 },
      colorize(snap) {
        const out = {}; const states = snap.states || {};
        for (const sid in states) {
          const occ = states[sid].occupation;
          if (!occ || typeof occ !== "object") continue;       // not occupied / not captured
          const r = occ.resistance;
          if (r == null) continue;                              // measured? only paint if captured
          // resistance 0..100 -> calm (green) to high-resistance (red)
          out[sid] = ramp(r, 0, 100, [60, 150, 90], [210, 60, 55]);
        }
        return out;
      } },

    // ---- partial / static: real specs, auto-enable --------------------------
    // SUPPLY — live division-province PROXY (snap.supply.by_province). Codex 1:
    // ramp by current/capacity when capacity>0 (raw `current` has no guaranteed
    // range); provinces with no proxy stay UNKNOWN, never green. PROXY-badged.
    { id: "supply", label: "Supply", group: "partial",
      perProvince: true,
      proxy: { kind: "live-proxy", confidenceFrom: "supply.confidence",
               note: "Live division-province supply proxy — where troops are (validated). The exact tooltip supply NUMBER is a documented wall; this is a proxy, not authoritative." },
      colorize(snap) {
        const out = {};
        const sup = (snap.supply || {}).by_province;
        if (!sup || typeof sup !== "object") return out;        // not captured -> honest empty
        for (const pid in sup) {
          const rec = sup[pid]; if (!rec) continue;
          let frac;
          if (rec.capacity != null && rec.capacity > 0 && rec.current != null) {
            frac = Math.max(0, Math.min(1, rec.current / rec.capacity));
          } else if (rec.current != null) {
            // no usable capacity denominator -> scale raw current to a soft range
            // rather than fabricate a fraction (still honest, just coarser).
            frac = Math.max(0, Math.min(1, rec.current / 20));
          } else continue;                                      // nothing measured -> UNKNOWN
          out[pid] = ramp(frac, 0, 1, [150, 50, 45], [60, 180, 95]);
        }
        return out;
      } },
    // RAILWAY — state-keyed buildings (states[id].buildings.rail_way). Codex 4:
    // distinct from province rail_usage telemetry (different meaning); we paint
    // the save-buildings level, not usage.
    { id: "rail", label: "Railway (buildings)", group: "partial",
      spec: { scope: "states", required: ["buildings.rail_way"], minEntities: 10, minCoverage: 0.05 },
      colorize(snap) {
        const out = {}; const states = snap.states || {};
        let max = 1;
        for (const sid in states) { const b = states[sid].buildings; const r = b && b.rail_way; if (r > max) max = r; }
        for (const sid in states) {
          const b = states[sid].buildings; if (!b || b.rail_way == null) continue;
          const r = b.rail_way;
          out[sid] = r > 0 ? ramp(r, 0, max, [45, 45, 55], [240, 220, 120]) : [38, 40, 48];
        }
        return out;
      } },
    // ---- NAVAL / AIR — region-keyed PROXY data with NO map geometry ----------
    // Codex 2 (HARD): snap.naval_air.{region_control,air_superiority} are keyed by
    // STRATEGIC_REGION_ID, and the map pipeline has NO region->state/province asset
    // to translate that to paintable ids. Painting coastal/sea-adjacent provinces
    // would FABRICATE spatial precision we don't have. So these are NOT color modes
    // — they are explicit, clickable, non-map REPORTS (report:true). The mode bar
    // opens a region-list report (panels.js) instead of recoloring; the report is
    // PROXY-badged (region_proxy). available() gates on the data actually existing.
    { id: "naval", label: "Naval (region report)", group: "cracking", report: "naval",
      available(snap) { const na = snap.naval_air || {}; return !!na.region_control && Object.keys(na.region_control).length > 0; },
      proxy: { kind: "region-proxy", confidenceFrom: "naval_air.confidence",
               note: "Sea-zone dominance per STRATEGIC REGION (region proxy). No region→map geometry is loaded, so this is a list/report, not a painted map." },
      colorize() { return {}; } },     // never paints (no geometry); report only
    { id: "air", label: "Air superiority (region report)", group: "cracking", report: "air",
      available(snap) { const na = snap.naval_air || {}; return !!na.air_superiority && Object.keys(na.air_superiority).length > 0; },
      proxy: { kind: "region-proxy", confidenceFrom: "naval_air.confidence",
               note: "Air superiority per STRATEGIC REGION (region proxy, effective-planes sums). No region→map geometry is loaded, so this is a list/report, not a painted map." },
      colorize() { return {}; } },     // never paints (no geometry); report only

    // ---- still being cracked (no schema field yet) — honest-empty -----------
    // intel: the schema has countries[TAG].intel.intel_networks (a reserved graph,
    // not a per-state level) — there is NO per-state intel field to paint. The old
    // `intel.intel_by_state` accessor was a DEAD path (no such schema field). This
    // mode stays disabled until a real per-state intel layer lands (NOTE: data-layer
    // follow-up — `intel_by_state` is MISSING from the schema).
    { id: "intel", label: "Intel", group: "cracking",
      colorize(snap, MAP) {
        const out = {};
        const sel = MAP && MAP.selCountry;
        const intel = sel && (snap.countries || {})[sel] && snap.countries[sel].intel;
        const by = intel && intel.intel_by_state;   // not in schema today -> stays {}
        if (!by) return out;
        for (const sid in by) { const v = by[sid]; if (v != null) out[sid] = ramp(v, 0, 100, [40, 40, 55], [120, 160, 230]); }
        return out;
      } },
    { id: "weather", label: "Weather", group: "cracking",
      // expected: map.weather_by_state {state_id: token} (memory_reader/weather; memory-only)
      colorize(snap) {
        const out = {};
        const w = snap.map && snap.map.weather_by_state;
        if (!w) return out;
        const tint = { clear: [120, 150, 80], rain: [70, 90, 130], storm: [50, 60, 110],
                       snow: [200, 210, 230], frozen: [150, 190, 220], mud: [110, 90, 60] };
        for (const sid in w) { const t = w[sid]; if (t) out[sid] = tint[t] || hashColor(t); }
        return out;
      } },
  ];

  // contract equipment volume: sum of equipment amounts (honest 0 if none captured).
  function contractVolume(c) {
    let v = 0;
    const eq = c && c.equipments;
    if (Array.isArray(eq)) for (const e of eq) {
      const a = (e && (e.amount != null ? e.amount : e.count));
      if (typeof a === "number") v += a;
    }
    // fall back to convoy count as a coarse volume proxy if no equipment amounts
    if (!v && c && c.convoys && typeof c.convoys === "object" && c.convoys.total != null) v = c.convoys.total;
    return v;
  }

  const byId = {};
  for (const m of modes) byId[m.id] = m;

  global.MapModes = { list: modes, byId, contractVolume };
})(window);
