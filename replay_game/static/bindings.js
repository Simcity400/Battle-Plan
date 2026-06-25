/* bindings.js — DATA BINDING registry: parsed .gui element NAME -> replay field.
 *
 * One table per window. Each entry is a binder consumed by GuiRenderer:
 *   - string  : a dotted path into the BOUND COUNTRY object (politics.political_power)
 *   - function(ctx) -> {value,captured} | {text} | {sprite,frame} | {flag:{tag,ideology,size}}
 *
 * ctx = { snap, tag, country, get(path), getOn(obj,path) }.
 *
 * HARD RULE: no fabrication. A binder that can't find the value returns
 * {value:null,captured:false} (or omits) so the renderer draws the game's empty
 * slot, never an invented figure. Element names come straight from the parsed .gui
 * (game_assets/gui/<view>.json), so they stay in lockstep with the real layout.
 *
 * Coverage note: the .gui defines MANY elements (music buttons, speed controls,
 * menu buttons) that have no replay meaning — those are simply left UNBOUND and
 * render as the static game chrome (correct: the game shows them too).
 */
(function (global) {
  "use strict";

  function captured(v) { return { value: v, captured: v !== null && v !== undefined }; }

  // ---- TOPBAR (interface/topbar.gui, window "topbar") ----------------------
  // element name -> snapshot field. Backgrounds/icons are static chrome (unbound).
  const topbar = {
    pol_power:           "politics.political_power",
    manpower:            (c) => captured(c.get("military.manpower.available").value),
    stability:           (c) => { const g = c.get("politics.stability"); return g.captured ? { text: Math.round(g.value * 100) + "%" } : { text: null }; },
    war_support:         (c) => { const g = c.get("politics.war_support"); return g.captured ? { text: Math.round(g.value * 100) + "%" } : { text: null }; },
    command_power_value: "politics.command_power",
    experience_value:    "military.xp.army",
    navy_experience_value: "military.xp.navy",
    air_experience_value:  "military.xp.air",
    industrial_capacity: (c) => {
      const civ = c.get("economy.factories.civ"), mil = c.get("economy.factories.mil"), dk = c.get("economy.factories.dockyard");
      if (!civ.captured && !mil.captured && !dk.captured) return { text: null };
      const n = (g) => g.captured ? g.value : "–";
      return { text: n(civ) + "/" + n(mil) + "/" + n(dk) };
    },
    convoys_count: (c) => {
      const cv = c.country && c.country.military && c.country.military.convoys;
      return Array.isArray(cv) ? { text: String(cv.length) } : { text: null };
    },
    fuel_value: (c) => { const g = c.get("economy.fuel.current"); return g.captured ? { text: Math.round(g.value) + "d" } : { text: null }; },
    DateText: (c) => ({ text: c.snap && c.snap.date ? c.snap.date : null }),
    player_flag: (c) => ({ flag: { tag: c.tag, ideology: (c.country && c.country.politics && c.country.politics.ruling_party) || null, size: "medium" } }),
  };

  // ---- COUNTRY VIEW: RESEARCH TAB (interface/countrytechtreeview.gui mapped to
  //      research.json; we prototype the research_slot_entry summary panel) ------
  // The research view's slot entries are data-cloned per slot; for a static replay
  // we bind the SUMMARY fields that exist on the country (research_slots, tech_count).
  const research_slot_entry = {
    // slot entries are dynamic; bound generically by the prototype which clones one
    // entry per in-progress tech. Element names within research_slot_entry:
    //   tech_name, progress_text, category_icon ... (populated by the prototype loop)
  };

  // ---- COMBAT VIEW (interface/combatview*.gui -> land_combat.json, window
  //      "landcombatview") -------------------------------------------------------
  // Bound to one entry of snap.combats (kind=="land"). The prototype passes the
  // chosen combat via ctx.combat; binders read it. country-db-id -> TAG is NOT
  // reliably resolvable from the snapshot here, so leader/flag stay honest-blank.
  const landcombatview = {
    window_title: (c) => ({ text: "BATTLE" }),
    combat_width: (c) => { const k = c.combat && c.combat.combat_width; return { text: k != null ? String(k) : null }; },
    attacker_army_width: (c) => { const k = c.combat && c.combat.combat_width; return { text: k != null ? String(k) : null }; },
    attacker_committed_label: (c) => { const a = c.combat && c.combat.attackers; return { text: Array.isArray(a) ? a.length + " Divisions" : null }; },
    defender_committed_label: (c) => { const d = c.combat && c.combat.defenders; return { text: Array.isArray(d) ? d.length + " Divisions" : null }; },
    attacker_reserves_label: (c) => ({ text: null }),  // reserves not captured -> blank
    defender_reserves_label: (c) => ({ text: null }),
    attacker_leader_name: (c) => ({ text: null }),     // commander->battle link not captured
    defender_leader_name: (c) => ({ text: null }),
    attacker_leader_skill: (c) => ({ text: null }),
    defender_leader_skill: (c) => ({ text: null }),
    // terrain picture: swap the sprite to the combat's terrain when we have one
    terrain_picture: (c) => {
      const t = c.combat && c.combat.terrain;
      const map = {
        plains: "GFX_terrain_plains", forest: "GFX_terrain_forest", hills: "GFX_terrain_hills",
        mountain: "GFX_terrain_mountain", marsh: "GFX_terrain_marsh", desert: "GFX_terrain_desert",
        urban: "GFX_terrain_urban", jungle: "GFX_terrain_jungle",
      };
      return t && map[t] ? { sprite: map[t] } : null; // null -> keep .gui default sprite
    },
  };

  global.Bindings = { topbar, research_slot_entry, landcombatview, captured };
})(window);
