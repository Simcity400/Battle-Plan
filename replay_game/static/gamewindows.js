/* gamewindows.js — Layer-2 ADVANCED windows (PLAN phases 4–5, §C):
 *   - Commanders / characters  (countries[TAG].commanders, §3.9)
 *   - Designers                (snap.designs §11 + countries[TAG].mios §3.10)
 *   - Order of battle          (snap.order_of_battle §12, leaders/designs cross-ref)
 *   - Intelligence / operations (countries[TAG].intel deepened, §3.8)
 *
 * These attach as extra National-view tabs and bind through the SAME generic
 * Store.get / array-presence path as the core tabs. The store currently carries
 * an OLDER snapshot (no top-level designs/order_of_battle, no commanders, shallow
 * intel) — so every window renders "not captured" gracefully and lights up the
 * instant a newer snapshot carries the domain, with no rework here.
 *
 * Two hard rules, enforced here per Codex review:
 *   - No fabrication. Portraits resolve a SPRITE NAME via UI.portrait → real art
 *     or a labelled placeholder (a portrait-asset pass is separate). OOB leader
 *     ids resolve via the commanders index ONLY (no char_id/token/portrait guess);
 *     unresolved → raw id + "name not captured"/"ambiguous". Operation progress is
 *     derived ONLY when all of {snapshot save_date, op date, op duration} are
 *     captured, clamped, labelled D — never wall-clock, never imputed.
 *   - null is sometimes MEANINGFUL (assignment:null = reserve; per-country
 *     order_of_battle:null). We use UI.presence (key-on-object) + domain capture
 *     to tell "captured but none" from "absent", not Store.get alone.
 */
(function (global) {
  "use strict";
  const S = () => global.Sprites;
  const E = UI.el;
  const T = () => global.NatTable;

  // ===========================================================================
  // Shared helpers
  // ===========================================================================

  /** All military leaders (general/FM/admiral) across every country, for OOB join. */
  function buildLeaderIndex(snap) {
    // perCountry[TAG] / perCountryBare[TAG] = Map(id -> [entries]) keyed by the
    //   full "id:type" and by the BARE id respectively (so an army leader ref that
    //   is bare or full still matches its OWN country before any global fallback).
    //   Lists (not single values) so a reused id is detectable as ambiguous, not
    //   silently overwritten — Codex.
    // global / globalBare = same, across all countries (unique-only fallback).
    const perCountry = {}, perCountryBare = {}, glob = new Map(), globBare = new Map();
    const push = (map, k, e) => { if (!map.has(k)) map.set(k, []); map.get(k).push(e); };
    const cs = (snap && snap.countries) || {};
    for (const tag of Object.keys(cs)) {
      const cm = cs[tag] && cs[tag].commanders;
      if (!cm) continue;
      const m = (perCountry[tag] = perCountry[tag] || new Map());
      const mb = (perCountryBare[tag] = perCountryBare[tag] || new Map());
      for (const grp of ["generals", "field_marshals", "admirals"]) {
        const arr = cm[grp];
        if (!Array.isArray(arr)) continue;
        for (const L of arr) {
          if (!L || L.unit_leader_id == null) continue;
          const key = normId(L.unit_leader_id);
          const bare = key.split(":")[0];
          const entry = { name: L.name || null, role: L.role || singular(grp), tag };
          push(m, key, entry); push(mb, bare, entry);
          push(glob, key, entry); push(globBare, bare, entry);
        }
      }
    }
    return { perCountry, perCountryBare, glob, globBare };
  }
  function singular(grp) { return { generals: "general", field_marshals: "field_marshal", admirals: "admiral" }[grp] || grp; }
  function nonEmpty(a) { return Array.isArray(a) && a.length > 0; }

  /** Normalize a leader ref ("id:type" pair, [id,type], or bare id) to a string key. */
  function normId(ref) {
    if (ref == null) return null;
    if (Array.isArray(ref)) return ref.map(String).join(":");
    if (typeof ref === "object") {
      if (ref.id != null) return [ref.id, ref.type].filter((x) => x != null).map(String).join(":");
      return JSON.stringify(ref);
    }
    return String(ref);
  }

  /**
   * Resolve an OOB leader ref to a name WITHOUT fabricating (Codex B). Try the
   * owner country's index first; fall back to the global index only when exactly
   * one match exists. Returns {name|null, note} — note explains an unresolved ref.
   */
  function resolveLeader(ref, ownerTag, idx) {
    if (ref == null) return { name: null, note: "no leader captured" };
    const key = normId(ref);
    const bare = key.split(":")[0];
    const named = (entry) => (entry && entry.name)
      ? { name: entry.name, note: null }
      : { name: null, note: "name not captured (id " + key + ")" };
    // Resolve a candidate list: 1 named -> name; 1 unnamed -> "name not captured";
    // >1 -> ambiguous. null list -> caller continues to the next tier.
    const tryList = (list, where) => {
      if (!list || !list.length) return null;
      if (list.length === 1) return named(list[0]);
      return { name: null, note: "ambiguous (id " + key + " ×" + list.length + (where ? " in " + where : "") + ")" };
    };
    const own = idx.perCountry[ownerTag], ownB = idx.perCountryBare[ownerTag];
    // owner country first — EXACT id, then BARE id — MUST precede any global
    // fallback so a same-tag leader is never overridden by another country's
    // leader that happens to share the bare id (Codex).
    let r = own && tryList(own.get(key), ownerTag); if (r) return r;
    r = ownB && tryList(ownB.get(bare), ownerTag); if (r) return r;
    // global — exact id, then bare id — unique only.
    r = tryList(idx.glob.get(key), null); if (r) return r;
    r = tryList(idx.globBare.get(bare), null); if (r) return r;
    return { name: null, note: "name not captured (id " + key + ")" };
  }

  /** A name cell that shows a resolved leader name or an honest raw-id note. */
  function leaderCell(ref, ownerTag, idx) {
    const r = resolveLeader(ref, ownerTag, idx);
    if (r.name) return document.createTextNode(r.name);
    const span = E("span", "val-missing", r.note);
    return span;
  }

  /** equipment_variants is an OBJECT keyed "id:type" — get a name for a ref. */
  function variantName(snap, ref) {
    const cat = (snap.designs || {}).equipment_variants;
    if (!cat || typeof cat !== "object") return null;
    const key = normId(ref);
    const v = cat[key] || (ref != null && cat[String(ref)]);
    if (!v) return null;
    return v.name || v.archetype || key;
  }

  /**
   * Parse a Clausewitz "Y.M.D.H" date to a comparable day-number (365-day, no
   * leap). Returns null on a malformed/out-of-range date (Codex: a bad date must
   * yield "not derivable", never bogus progress) — month 1..12, day within the
   * month's length (non-leap).
   */
  const _MONTH_DAYS = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  const _MONTH_CUM = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
  function dateToDay(d) {
    if (!d || typeof d !== "string") return null;
    const p = d.split(".").map(Number);
    if (p.length < 3 || p.slice(0, 3).some(isNaN)) return null;
    const [y, m, day] = p;
    if (!Number.isInteger(m) || m < 1 || m > 12) return null;
    if (!Number.isInteger(day) || day < 1 || day > _MONTH_DAYS[m - 1]) return null;
    return y * 365 + _MONTH_CUM[m - 1] + (day - 1);
  }

  /**
   * Operation progress (Codex C): derived ONLY when the snapshot "now" date, the
   * op start `date`, and `duration` are all captured & well-formed. The "now"
   * date is the save channel's `save_date` (preferred — same authority the schema
   * names) and falls back to the snapshot's own `date` (the snapshot instant — an
   * authoritative timestamp, never fabricated). Returns {pct|null, note}; pct
   * clamped 0..1. No wall-clock time is ever used.
   */
  function operationProgress(snap, op) {
    const sv = (snap.provenance || {}).save || {};
    const nowDate = sv.save_date || snap.date;
    const startDay = dateToDay(op && op.date);
    const nowDay = dateToDay(nowDate);
    const dur = op && op.duration;
    if (startDay == null || nowDay == null || dur == null || !(dur > 0))
      return { pct: null, note: "progress not derivable (start date / now / duration not all captured)" };
    const pct = Math.max(0, Math.min(1, (nowDay - startDay) / dur));
    return { pct, note: null };
  }

  // ===========================================================================
  // C-extra-1: Commanders / characters  (§3.9)
  // ===========================================================================
  function tab_commanders(b) {
    const c = this.country();
    const cmP = UI.presence(c, "commanders");
    const domCap = this.domainCaptured("commanders");
    // The schema skeleton makes `commanders` a PRESENT-but-empty object even when
    // characters.json was never ingested. So "present object" alone is NOT proof
    // of capture — gate on provenance OR on the object actually holding content
    // (Codex: never render an empty skeleton as "none captured").
    const cm = (cmP.state === "value") ? cmP.value : null;
    const hasData = cm && (cm.leader || nonEmpty(cm.generals) || nonEmpty(cm.field_marshals) ||
      nonEmpty(cm.admirals) || nonEmpty(cm.advisors) || nonEmpty(cm.party_leaders) || cm.counts);
    if (!domCap && !hasData) {
      const p = E("div", "panel");
      p.appendChild(E("h3", null, "Commanders & characters")).appendChild(UI.srcBadge("S"));
      p.appendChild(UI.notCaptured("Commanders (characters.json)"));
      b.appendChild(p);
      return;
    }
    const CM = cm || {}; // domain captured but empty -> render honest empty rosters

    // -- country leader + party leaders -------------------------------------
    const lp = E("div", "panel");
    lp.appendChild(E("h3", null, "Country leader")).appendChild(UI.srcBadge("S"));
    const L = CM.leader;
    if (L && typeof L === "object") {
      const card = E("div", "char-card");
      card.appendChild(UI.portrait(L.portrait_gfx, L.name, 56));
      const meta = E("div", "char-meta");
      meta.appendChild(E("div", "char-name", L.name || "(unnamed)"));
      const sub = [];
      if (L.ideology) sub.push(L.ideology);
      if (CM.ruling_party) sub.push("party: " + CM.ruling_party);
      meta.appendChild(E("div", "char-sub", sub.join(" · ") || "—"));
      meta.appendChild(UI.traitChips(L.traits));
      card.appendChild(meta);
      lp.appendChild(card);
    } else lp.appendChild(UI.notCaptured("Country leader"));

    if (Array.isArray(CM.party_leaders) && CM.party_leaders.length) {
      const sub = E("div", "subblock");
      sub.appendChild(E("div", "subhead", "Party leaders"));
      const tray = E("div", "char-tray");
      for (const pl of CM.party_leaders) {
        const cell = E("div", "char-mini");
        cell.appendChild(UI.portrait(pl.portrait_gfx, pl.name, 40));
        cell.appendChild(E("span", "char-mini-name", pl.name || "?"));
        const bits = [pl.party || pl.ideology, pl.popularity != null ? UI.pct(pl.popularity) : null].filter(Boolean);
        cell.appendChild(E("span", "char-mini-sub", bits.join(" · ")));
        tray.appendChild(cell);
      }
      sub.appendChild(tray);
      lp.appendChild(sub);
    }
    b.appendChild(lp);

    // -- counts -------------------------------------------------------------
    if (CM.counts && typeof CM.counts === "object") {
      const cp = E("div", "panel");
      cp.appendChild(E("h3", null, "Roster")).appendChild(UI.srcBadge("S/D"));
      const g = (k) => Store.get(CM.counts, k);
      cp.appendChild(UI.stat("Generals", g("generals"), {}));
      cp.appendChild(UI.stat("Field marshals", g("field_marshals"), {}));
      cp.appendChild(UI.stat("Admirals", g("admirals"), {}));
      cp.appendChild(UI.stat("Advisors", g("advisors"), {}));
      cp.appendChild(UI.stat("Roster size", g("roster_size"), {}));
      b.appendChild(cp);
    }

    // -- military leaders (generals / FM / admirals) ------------------------
    for (const [grp, label, navy] of [
      ["field_marshals", "Field marshals", false],
      ["generals", "Generals", false],
      ["admirals", "Admirals", true],
    ]) {
      const arr = CM[grp];
      const p = E("div", "panel");
      p.appendChild(E("h3", null, `${label} (${Array.isArray(arr) ? arr.length : 0})`)).appendChild(UI.srcBadge("S"));
      if (!Array.isArray(arr) || !arr.length) {
        p.appendChild(UI.emptyList(label, domCap));
      } else {
        // §3.9: army leaders = attack/defense/planning/logistics; NAVY leaders
        // only carry maneuvering/coordination (no attack/def/plan/log) — don't
        // show non-schema columns for admirals.
        const t = T().table(["", "Name", "Skill",
          navy ? "Mvr/Coord" : "Atk/Def/Plan/Log", "Assignment", "Traits"]);
        for (const M of arr) {
          const tr = E("tr");
          const pc = E("td"); pc.appendChild(UI.portrait(M.portrait_gfx, M.name, 34)); tr.appendChild(pc);
          tr.appendChild(td(M.name || "?"));
          const sc = E("td"); sc.appendChild(UI.skillStars(M.level != null ? M.level : M.skill)); tr.appendChild(sc);
          const stats = navy
            ? [M.maneuvering, M.coordination]
            : [M.attack, M.defense, M.planning, M.logistics];
          tr.appendChild(td(stats.map((x) => (x == null ? "–" : UI.num(x))).join(" / ")));
          // assignment is meaningful-null: present-null = reserve; absent = not captured
          const ap = UI.presence(M, "assignment");
          const ac = E("td");
          if (ap.state === "value" && ap.value) {
            const a = ap.value;
            ac.textContent = (a.kind ? a.kind.replace(/_/g, " ") + ": " : "") + (a.name || "?");
          } else if (ap.state === "nullSet") ac.appendChild(E("span", "val-missing", "reserve"));
          else ac.appendChild(E("span", "val-missing", "not captured"));
          tr.appendChild(ac);
          const trc = E("td"); trc.appendChild(UI.traitChips(M.traits)); tr.appendChild(trc);
          if (M.captured) tr.classList.add("char-captured");
          t.body.appendChild(tr);
        }
        p.appendChild(t.table);
      }
      b.appendChild(p);
    }

    // -- advisors / cabinet by slot -----------------------------------------
    const adv = CM.advisors;
    const ap = E("div", "panel");
    ap.appendChild(E("h3", null, `Advisors & cabinet (${Array.isArray(adv) ? adv.length : 0})`)).appendChild(UI.srcBadge("S"));
    if (!Array.isArray(adv) || !adv.length) ap.appendChild(UI.emptyList("Advisors", domCap));
    else {
      // group by slot for the cabinet layout (prototype-less: slot tokens are
      // untrusted strings that could collide with Object.prototype keys).
      const bySlot = Object.create(null);
      for (const a of adv) { const s = a.slot || "(other)"; (bySlot[s] = bySlot[s] || []).push(a); }
      for (const slot of Object.keys(bySlot).sort()) {
        const sub = E("div", "subblock");
        sub.appendChild(E("div", "subhead", slot.replace(/_/g, " ")));
        const tray = E("div", "char-tray");
        for (const a of bySlot[slot]) {
          const cell = E("div", "char-mini");
          // an advisor's idea icon, else portrait
          const ideaCand = a.idea_token ? "GFX_idea_" + a.idea_token : null;
          if (ideaCand && S().has(ideaCand)) cell.appendChild(S().icon(ideaCand, { size: { w: 48, h: 28 }, title: a.idea_token }));
          else cell.appendChild(UI.portrait(a.portrait_gfx, a.name, 40));
          cell.appendChild(E("span", "char-mini-name", a.name || "?"));
          cell.title = (a.idea_token || a.token || a.name || "") + (Array.isArray(a.traits) ? " · " + a.traits.join(", ") : "");
          tray.appendChild(cell);
        }
        sub.appendChild(tray);
        ap.appendChild(sub);
      }
    }
    b.appendChild(ap);
  }

  // ===========================================================================
  // C-extra-2: Designers  (§11 designs + §3.10 mios)
  // ===========================================================================
  function tab_designs(b) {
    const snap = this.snap, tag = this.tag, c = this.country();
    const designs = snap.designs;
    const domCap = this.domainCaptured("designs");

    // -- division templates (regiment grid) ---------------------------------
    const tp = E("div", "panel");
    tp.appendChild(E("h3", null, "Division designer — templates")).appendChild(UI.srcBadge("S"));
    const tmplAll = (designs && Array.isArray(designs.division_templates)) ? designs.division_templates : null;
    const tmpl = tmplAll ? tmplAll.filter((t) => t.country === tag) : null;
    if (!tmplAll) tp.appendChild(UI.notCaptured("Division templates (designs.json)"));
    // reaching here means the WORLD template list exists (domain captured) but this
    // tag designed none -> a genuine captured-none, not "not captured".
    else if (!tmpl.length) tp.appendChild(UI.emptyList("Division templates for " + tag, true));
    else {
      for (const tm of tmpl) {
        tp.appendChild(renderTemplate(tm));
      }
      tp.appendChild(E("div", "note", "Combat stats (soft/hard attack, org, HP, width) are runtime-computed and NOT stored in the save (SCHEMA §11 hard limit) — only the regiment grid layout is shown."));
    }
    b.appendChild(tp);

    // -- equipment / ship / plane variants ----------------------------------
    const cat = designs && designs.equipment_variants;
    const vp = E("div", "panel");
    vp.appendChild(E("h3", null, "Equipment / ship / plane designer — variants")).appendChild(UI.srcBadge("S"));
    if (!cat || typeof cat !== "object") vp.appendChild(UI.notCaptured("Equipment variants"));
    else {
      const mine = Object.values(cat).filter((v) => v && v.creator === tag);
      if (!mine.length) vp.appendChild(UI.emptyList("Variants designed by " + tag, true));
      else {
        // prototype-less map so a family literally named "constructor"/"__proto__"
        // can't collide with Object.prototype (Codex).
        const byFam = Object.create(null);
        for (const v of mine) { const f = v.family || "(other)"; (byFam[f] = byFam[f] || []).push(v); }
        const famLabel = { land_vehicle: "Land vehicles", ship: "Ships", plane: "Planes", land_other: "Land (other)" };
        for (const fam of Object.keys(byFam)) {
          const sub = E("div", "subblock");
          sub.appendChild(E("div", "subhead", famLabel[fam] || fam));
          for (const v of byFam[fam]) sub.appendChild(renderVariant(v));
          vp.appendChild(sub);
        }
      }
    }
    b.appendChild(vp);

    // -- MIO panel (per country) --------------------------------------------
    const mios = c.mios;
    const mp = E("div", "panel");
    mp.appendChild(E("h3", null, `Military-industrial organisations (${Array.isArray(mios) ? mios.length : 0})`)).appendChild(UI.srcBadge("S"));
    if (!Array.isArray(mios) || !mios.length) {
      mp.appendChild(this.domainCaptured("designs") ? UI.emptyList("MIOs", true) : UI.notCaptured("MIOs (designs.json)"));
    } else {
      const t = T().table(["", "Name", "Type", "Level", "Funds", "Points", "Task cap", "Traits"]);
      for (const m of mios) {
        const tr = E("tr");
        const ic = E("td");
        const cand = m.icon || ("GFX_" + (m.key || ""));
        ic.appendChild(S().has(cand) ? S().icon(cand, { size: { w: 28, h: 28 }, title: m.name }) : E("span", "spr-miss", "□"));
        tr.appendChild(ic);
        tr.appendChild(td(m.name || m.key || "?"));
        tr.appendChild(td(m.type || "–"));
        tr.appendChild(td(m.level != null ? String(m.level) : "–"));
        tr.appendChild(td(m.funds != null ? UI.num(m.funds) : "–"));
        tr.appendChild(td(m.points != null ? UI.num(m.points) : "–"));
        tr.appendChild(td(m.task_capacity != null ? String(m.task_capacity) : "–"));
        const trc = E("td"); trc.appendChild(UI.traitChips(m.unlocked_traits)); tr.appendChild(trc);
        t.body.appendChild(tr);
      }
      mp.appendChild(t.table);
    }
    b.appendChild(mp);

    // -- scheduled designer presets -----------------------------------------
    const sv = c.scheduled_variants;
    if (Array.isArray(sv) && sv.length) {
      const sp = E("div", "panel");
      sp.appendChild(E("h3", null, `Scheduled designer presets (${sv.length})`)).appendChild(UI.srcBadge("S"));
      const tray = E("div", "diplo-tray");
      for (const p of sv) {
        const chip = E("span", "role-chip", (p.name || p.archetype || "?") + (p.obsolete ? " (obsolete)" : ""));
        chip.title = (p.family || "") + " · " + (p.archetype || "");
        tray.appendChild(chip);
      }
      sp.appendChild(tray);
      b.appendChild(sp);
    }

    // -- MIO policies (cost / cooldown) -------------------------------------
    const pol = c.mio_policies;
    if (pol && typeof pol === "object" && Object.keys(pol).length) {
      const pp = E("div", "panel");
      pp.appendChild(E("h3", null, "MIO policies")).appendChild(UI.srcBadge("S"));
      const t = T().table(["Policy", "Cost", "Cooldown"]);
      for (const [name, v] of Object.entries(pol)) {
        const o = (v && typeof v === "object") ? v : {};
        t.body.appendChild(T().row([name.replace(/_/g, " "),
          o.cost != null ? UI.num(o.cost) : "–", o.cooldown != null ? String(o.cooldown) : "–"]));
      }
      pp.appendChild(t.table);
      b.appendChild(pp);
    }
  }

  function renderTemplate(tm) {
    const box = E("div", "tmpl-box");
    const head = E("div", "tmpl-head");
    head.appendChild(E("span", "tmpl-name", tm.name || "(template)"));
    const tags = [];
    if (tm.role) tags.push(tm.role);
    if (tm.is_army_hq) tags.push("HQ");
    if (tm.is_locked) tags.push("locked");
    if (tm.obsolete) tags.push("obsolete");
    if (tm.n_regiments != null) tags.push(tm.n_regiments + " reg");
    if (tm.n_support != null) tags.push(tm.n_support + " sup");
    head.appendChild(E("span", "tmpl-tags", tags.join(" · ")));
    box.appendChild(head);

    // regiment grid: regiments carry {type,x,y} — lay out by (x column, y row)
    const regs = Array.isArray(tm.regiments) ? tm.regiments : [];
    if (regs.length && regs.every((r) => r && r.x != null && r.y != null)) {
      const maxX = Math.max(...regs.map((r) => r.x)) + 1;
      const maxY = Math.max(...regs.map((r) => r.y)) + 1;
      const grid = E("div", "reg-grid");
      grid.style.gridTemplateColumns = `repeat(${Math.max(1, maxX)}, 1fr)`;
      const at = {};
      for (const r of regs) at[r.x + "," + r.y] = r;
      for (let y = 0; y < maxY; y++) for (let x = 0; x < maxX; x++) {
        const r = at[x + "," + y];
        const cell = E("div", "reg-cell" + (r ? "" : " reg-empty"));
        if (r) { cell.textContent = String(r.type || "").replace(/_/g, " "); cell.title = r.type; }
        grid.appendChild(cell);
      }
      box.appendChild(grid);
    } else if (regs.length) {
      // no positions — list honestly rather than fake a grid
      const tray = E("div", "diplo-tray");
      for (const r of regs) tray.appendChild(E("span", "role-chip", String(r.type || r).replace(/_/g, " ")));
      box.appendChild(tray);
      box.appendChild(E("div", "note", "Regiment positions (x,y) not captured — listed, not laid out as a grid."));
    } else box.appendChild(UI.notCaptured("Regiments"));

    // support companies (shape may differ from regiments — don't assume x,y)
    const sup = Array.isArray(tm.support) ? tm.support : [];
    if (sup.length) {
      const sb = E("div", "tmpl-support");
      sb.appendChild(E("span", "subhead", "Support"));
      const tray = E("div", "diplo-tray");
      for (const s of sup) tray.appendChild(E("span", "role-chip", String((s && s.type) || s).replace(/_/g, " ")));
      sb.appendChild(tray);
      box.appendChild(sb);
    }
    return box;
  }

  function renderVariant(v) {
    const box = E("div", "var-box");
    const head = E("div", "var-head");
    const cand = "GFX_" + (v.archetype || "");
    head.appendChild(S().has(cand) ? S().icon(cand, { size: { w: 32, h: 24 }, title: v.archetype }) : E("span", "spr-miss", "□"));
    head.appendChild(E("span", "var-name", v.name || v.archetype || "(variant)"));
    const tags = [];
    if (v.version != null) tags.push("v" + v.version);
    if (v.obsolete) tags.push("obsolete");
    if (v.design_team_mio && v.design_team_mio.name) tags.push("MIO: " + v.design_team_mio.name);
    head.appendChild(E("span", "var-tags", tags.join(" · ")));
    box.appendChild(head);
    // modules by slot
    if (v.modules && typeof v.modules === "object" && Object.keys(v.modules).length) {
      const t = T().table(["Slot", "Module"]);
      for (const [slot, mod] of Object.entries(v.modules))
        t.body.appendChild(T().row([slot.replace(/_/g, " "), String(mod).replace(/_/g, " ")]));
      box.appendChild(t.table);
    }
    // upgrades
    if (v.upgrades && typeof v.upgrades === "object" && Object.keys(v.upgrades).length) {
      const tray = E("div", "diplo-tray");
      for (const [u, lvl] of Object.entries(v.upgrades)) tray.appendChild(E("span", "role-chip", u.replace(/_/g, " ") + ": " + lvl));
      box.appendChild(tray);
    }
    return box;
  }

  // ===========================================================================
  // C-extra-3: Order of battle  (§12)
  // ===========================================================================
  function tab_oob(b) {
    const snap = this.snap, tag = this.tag, c = this.country();
    const oob = snap.order_of_battle;
    const idx = buildLeaderIndex(snap);

    // per-country rollup (meaningful-null: present-null = captured-but-empty)
    const rollP = UI.presence(c, "order_of_battle");
    const rp = E("div", "panel");
    rp.appendChild(E("h3", null, "Order of battle — summary")).appendChild(UI.srcBadge("S/D"));
    if (rollP.state === "value") {
      const r = rollP.value, g = (k) => Store.get(r, k);
      const grid = E("div", "nat-grid");
      const mk = (label, key) => UI.stat(label, g(key), {});
      const col1 = E("div"); col1.appendChild(mk("Army groups", "army_groups")); col1.appendChild(mk("Armies", "armies")); col1.appendChild(mk("Divisions in armies", "divisions_in_armies"));
      const col2 = E("div"); col2.appendChild(mk("Fleets", "fleets")); col2.appendChild(mk("Task forces", "task_forces")); col2.appendChild(mk("Ships", "ships"));
      const col3 = E("div"); col3.appendChild(mk("Air wings", "air_wings")); col3.appendChild(mk("Planes", "planes"));
      grid.appendChild(col1); grid.appendChild(col2); grid.appendChild(col3);
      rp.appendChild(grid);
    } else if (rollP.state === "nullSet") rp.appendChild(UI.emptyList("OOB rollup", this.domainCaptured("order_of_battle")));
    else rp.appendChild(UI.notCaptured("Order of battle rollup (oob.json)"));
    b.appendChild(rp);

    const oobDom = this.domainCaptured("order_of_battle");
    if (!oob || typeof oob !== "object" || !Object.keys(oob).length) {
      const p = E("div", "panel");
      p.appendChild(E("h3", null, "Formations")).appendChild(UI.srcBadge("S"));
      p.appendChild(UI.notCaptured("Order of battle (oob.json)"));
      b.appendChild(p);
      return;
    }

    const byOwner = (list) => (Array.isArray(list) ? list.filter((x) => x && x.owner === tag) : []);
    // Index divisions by full "id:type" AND bare id (Codex: if a member ref carries
    // a type but the division's div_type is null — or vice-versa — fall back to the
    // bare id rather than wrongly showing "not in snapshot"). A bare key collides
    // only if two divisions share an id with differing types — rare; full key wins.
    const divById = new Map(), divByBare = new Map();
    for (const d of (snap.divisions || [])) {
      divById.set(normId({ id: d.div_id, type: d.div_type }), d);
      if (d.div_id != null && !divByBare.has(String(d.div_id))) divByBare.set(String(d.div_id), d);
    }
    const lookupDiv = (ref) => {
      const k = normId(ref);
      return divById.get(k) || divById.get(String(ref).split(":")[0]) || divByBare.get(k.split(":")[0]) || null;
    };

    // -- army groups + armies (land) ----------------------------------------
    const ags = byOwner(oob.army_groups), armies = byOwner(oob.armies);
    const lp = E("div", "panel");
    lp.appendChild(E("h3", null, `Army (groups: ${ags.length}, armies: ${armies.length})`)).appendChild(UI.srcBadge("S"));
    if (!ags.length && !armies.length) lp.appendChild(UI.emptyList("Land formations for " + tag, oobDom));
    else {
      // armies indexed by their id for nesting under groups
      const armyById = new Map(armies.map((a) => [normId(a.id), a]));
      const placed = new Set();
      for (const ag of ags) {
        const node = E("div", "oob-group");
        const h = E("div", "oob-group-head");
        if (ag.icon && S().has(ag.icon)) h.appendChild(S().icon(ag.icon, { size: { w: 22, h: 22 } }));
        h.appendChild(E("span", "oob-name", ag.name || "(army group)"));
        h.appendChild(E("span", "oob-lead")).appendChild(leaderCell(ag.leader, tag, idx));
        node.appendChild(h);
        node.appendChild(cachedNote(ag.cached));
        const refs = Array.isArray(ag.army_refs) ? ag.army_refs : [];
        for (const ref of refs) {
          const a = armyById.get(normId(ref));
          if (a) { placed.add(normId(a.id)); node.appendChild(renderArmy(a, tag, idx, lookupDiv)); }
          else { const m = E("div", "oob-army"); m.appendChild(E("span", "val-missing", "army " + normId(ref) + " — not captured")); node.appendChild(m); }
        }
        lp.appendChild(node);
      }
      // armies not under any captured group
      for (const a of armies) if (!placed.has(normId(a.id))) lp.appendChild(renderArmy(a, tag, idx, lookupDiv));
    }
    b.appendChild(lp);

    // -- fleets / task forces / ships (navy) --------------------------------
    const fleets = byOwner(oob.fleets);
    const np = E("div", "panel");
    np.appendChild(E("h3", null, `Navy — fleets (${fleets.length})`)).appendChild(UI.srcBadge("S"));
    if (!fleets.length) np.appendChild(UI.emptyList("Fleets for " + tag, oobDom));
    else for (const fl of fleets) {
      const node = E("div", "oob-group");
      const h = E("div", "oob-group-head");
      h.appendChild(E("span", "oob-name", fl.name || "(fleet)"));
      h.appendChild(E("span", "oob-lead")).appendChild(leaderCell(fl.leader, tag, idx));
      node.appendChild(h);
      const tfs = Array.isArray(fl.task_forces) ? fl.task_forces : [];
      for (const tf of tfs) {
        const tfn = E("div", "oob-army");
        const th = E("div", "oob-army-head");
        th.appendChild(E("span", "oob-tf", tf.name || "(task force)"));
        if (tf.mission) th.appendChild(E("span", "oob-mission", tf.mission));
        if (tf.location_province != null) th.appendChild(E("span", "oob-loc", "prov " + tf.location_province));
        tfn.appendChild(th);
        const ships = Array.isArray(tf.ships) ? tf.ships : [];
        if (ships.length) {
          const t = T().table(["Ship", "Definition", "Org", "Str", "Exp"]);
          for (const s of ships.slice(0, 60)) t.body.appendChild(T().row([
            s.name || "?", s.definition || "–",
            s.organisation != null ? UI.num(s.organisation) : "–",
            s.strength != null ? UI.num(s.strength) : "–",
            s.experience != null ? UI.num(s.experience) : "–",
          ]));
          tfn.appendChild(t.table);
          if (ships.length > 60) tfn.appendChild(E("div", "note", `Showing 60 of ${ships.length} ships.`));
        } else tfn.appendChild(UI.notCaptured("Ships"));
        node.appendChild(tfn);
      }
      np.appendChild(node);
    }
    b.appendChild(np);

    // -- air wings ----------------------------------------------------------
    const wings = byOwner(oob.air_wings);
    const ap = E("div", "panel");
    ap.appendChild(E("h3", null, `Air wings (${wings.length})`)).appendChild(UI.srcBadge("S"));
    if (!wings.length) ap.appendChild(UI.emptyList("Air wings for " + tag, oobDom));
    else {
      const t = T().table(["Wing", "Planes", "Mission", "Region", "Base / state", "Exp"]);
      for (const w of wings) {
        const planeName = variantName(snap, w.plane_definition) || w.plane_definition || "–";
        const baseStr = w.state_id != null ? ("state " + w.state_id) : (w.air_base != null ? ("base " + normId(w.air_base)) : "carrier/mobile");
        const tr = T().row([
          w.name || planeName,
          w.count != null ? UI.num(w.count) : "–",
          w.mission || w.mission_type || "–",
          w.strategic_region != null ? String(w.strategic_region) : "–",
          baseStr,
          w.experience != null ? UI.num(w.experience) : "–",
        ]);
        t.body.appendChild(tr);
      }
      ap.appendChild(t.table);
      ap.appendChild(E("div", "note", "effective_planes (runtime/cached) shown as-is where present; not authoritative (SCHEMA §12)."));
    }
    b.appendChild(ap);
  }

  function renderArmy(a, tag, idx, lookupDiv) {
    const node = E("div", "oob-army");
    const h = E("div", "oob-army-head");
    h.appendChild(E("span", "oob-tf", a.name || "(army)"));
    h.appendChild(E("span", "oob-lead")).appendChild(leaderCell(a.leader, tag, idx));
    if (a.member_count != null) h.appendChild(E("span", "oob-loc", a.member_count + " div"));
    node.appendChild(h);
    node.appendChild(cachedNote(a.cached));
    // member divisions resolved to template/strength
    const members = Array.isArray(a.members) ? a.members : [];
    if (members.length) {
      const t = T().table(["Division", "Template", "Org", "Str", "Province"]);
      let shown = 0;
      for (const ref of members) {
        if (shown >= 50) break;
        const d = lookupDiv(ref);
        if (d) {
          shown++;
          t.body.appendChild(T().row([
            "#" + d.div_id, d.template_name || d.template_role || "?",
            d.organisation != null ? UI.num(d.organisation) : "–",
            d.strength != null ? UI.num(d.strength) : "–",
            d.location_province != null ? String(d.location_province) : "–",
          ]));
        } else {
          shown++;
          t.body.appendChild(T().row(["#" + normId(ref), T().muted("not in snapshot"), "–", "–", "–"]));
        }
      }
      node.appendChild(t.table);
      if (members.length > 50) node.appendChild(E("div", "note", `Showing 50 of ${members.length} divisions.`));
    }
    // battle-plan orders (surface count + types honestly; geometry is memory-only)
    const orders = Array.isArray(a.orders) ? a.orders : [];
    if (orders.length) {
      const tray = E("div", "diplo-tray");
      for (const o of orders) tray.appendChild(E("span", "role-chip", (o.type || "order") + (o.operation ? ": " + o.operation : "")));
      const ob = E("div", "oob-orders"); ob.appendChild(E("span", "subhead", "Battle plans")); ob.appendChild(tray);
      node.appendChild(ob);
    }
    return node;
  }

  function cachedNote(cached) {
    if (!cached || typeof cached !== "object") return document.createComment("");
    const bits = [];
    for (const k of ["plan_value", "our_power", "enemy_power"])
      if (cached[k] != null) bits.push(k.replace(/_/g, " ") + " " + UI.num(cached[k]));
    if (!bits.length) return document.createComment("");
    const n = E("div", "oob-cached", "cached (runtime, non-authoritative): " + bits.join(", "));
    return n;
  }

  // ===========================================================================
  // C-extra-4: Intelligence / operations  (§3.8) — replaces the stub intel tab
  // ===========================================================================
  function tab_intel(b) {
    const snap = this.snap, c = this.country(), intel = c.intel || {};

    // -- agency -------------------------------------------------------------
    const agP = UI.presence(intel, "agency");
    const agp = E("div", "panel");
    agp.appendChild(E("h3", null, "Intelligence agency")).appendChild(UI.srcBadge("S"));
    if (agP.state === "value") {
      const ag = agP.value, g = (k) => Store.get(ag, k);
      const head = E("div", "char-card");
      const cand = ag.icon;
      if (cand && S().has(cand)) head.appendChild(S().icon(cand, { size: { w: 40, h: 40 }, title: ag.name }));
      const meta = E("div", "char-meta");
      meta.appendChild(E("div", "char-name", ag.name || "(agency)"));
      meta.appendChild(UI.stat("Created", g("is_created"), {}));
      meta.appendChild(UI.stat("Upgrade in progress", g("current_upgrade"), {}));
      meta.appendChild(UI.stat("Upgrade progress", g("upgrade_progress"), { fmt: (v) => (v <= 1 ? UI.pct(v) : UI.num(v)) }));
      head.appendChild(meta);
      agp.appendChild(head);
      const cap = ag.capacity;
      if (cap && typeof cap === "object") {
        const sub = E("div", "subblock"); sub.appendChild(E("div", "subhead", "Capacity"));
        sub.appendChild(UI.stat("Max operatives", Store.get(cap, "max_operatives"), {}));
        sub.appendChild(UI.stat("Usable slots", Store.get(cap, "usable_slots"), {}));
        sub.appendChild(UI.stat("Defense", Store.get(cap, "defense"), {}));
        agp.appendChild(sub);
      }
      if (ag.upgrades && typeof ag.upgrades === "object" && Object.keys(ag.upgrades).length) {
        const sub = E("div", "subblock"); sub.appendChild(E("div", "subhead", "Upgrades"));
        const tray = E("div", "diplo-tray");
        for (const [br, lvl] of Object.entries(ag.upgrades)) tray.appendChild(E("span", "role-chip", br.replace(/_/g, " ") + ": " + lvl));
        sub.appendChild(tray); agp.appendChild(sub);
      }
    } else {
      // older shallow intel: agency section ABSENT -> not captured (not "none")
      agp.appendChild(UI.notCaptured("Intelligence agency"));
    }
    b.appendChild(agp);

    // -- operatives roster --------------------------------------------------
    for (const [k, label] of [["operatives", "Operatives"], ["recruitable_operatives", "Recruitable operatives"]]) {
      const arr = intel[k];
      if (k === "recruitable_operatives" && (!Array.isArray(arr) || !arr.length) && !(k in intel)) continue;
      const p = E("div", "panel");
      p.appendChild(E("h3", null, `${label} (${Array.isArray(arr) ? arr.length : 0})`)).appendChild(UI.srcBadge("S"));
      if (!Array.isArray(arr) || !arr.length) {
        // present-empty list = "none" only when the intel domain was actually
        // captured (provenance); key absent OR domain uncaptured = "not captured".
        p.appendChild((k in intel) ? UI.emptyList(label, this.domainCaptured("intel")) : UI.notCaptured(label));
      } else {
        const t = T().table(["", "Name", "Skill", "Status", "Nationality", "Traits"]);
        for (const op of arr) {
          const tr = E("tr");
          const pc = E("td"); pc.appendChild(UI.portrait(op.gfx, op.name, 32)); tr.appendChild(pc);
          tr.appendChild(td(op.name || "?"));
          const sc = E("td"); sc.appendChild(UI.skillStars(op.skill, 5)); tr.appendChild(sc);
          let status = op.state || "–";
          if (op.captured) status = "captured" + (op.captured_by ? " by " + op.captured_by : "");
          else if (op.operation_id != null) status = (op.state || "on operation") + " #" + op.operation_id;
          tr.appendChild(td(status));
          tr.appendChild(td(Array.isArray(op.nationalities) ? op.nationalities.join(", ") : "–"));
          const trc = E("td"); trc.appendChild(UI.traitChips(op.traits)); tr.appendChild(trc);
          if (op.captured) tr.classList.add("char-captured");
          t.body.appendChild(tr);
        }
        p.appendChild(t.table);
      }
      b.appendChild(p);
    }

    // -- operations (running + finished) ------------------------------------
    const opsP = UI.presence(intel, "operations");
    const op = E("div", "panel");
    op.appendChild(E("h3", null, "Operations")).appendChild(UI.srcBadge("S / D"));
    if (opsP.state !== "value") op.appendChild(UI.notCaptured("Operations"));
    else {
      const ops = opsP.value;
      if (ops.priority != null) op.appendChild(UI.stat("Priority", { value: ops.priority, captured: true }, {}));
      const running = Array.isArray(ops.running) ? ops.running : [];
      const rb = E("div", "subblock"); rb.appendChild(E("div", "subhead", `Running (${running.length})`));
      if (!running.length) rb.appendChild(UI.emptyList("Running operations", true));
      else {
        const t = T().table(["Operation", "Target", "Started", "Progress (derived)", "Operatives"]);
        for (const o of running) {
          const prog = operationProgress(snap, o);
          const pcell = E("td");
          if (prog.pct == null) { const m = E("span", "val-missing", prog.note); pcell.appendChild(m); }
          else {
            const bar = E("span", "op-bar"); const fill = E("span", "op-bar-fill"); fill.style.width = (prog.pct * 100) + "%"; bar.appendChild(fill);
            pcell.appendChild(bar); pcell.appendChild(E("span", "op-pct", " " + UI.pct(prog.pct)));
            const dbadge = UI.srcBadge("D", "derived from save_date − op.date over duration (SCHEMA §3.8); clamped 0..1"); pcell.appendChild(dbadge);
          }
          const nOps = Array.isArray(o.operatives) ? o.operatives.length : null;
          const tr = E("tr");
          tr.appendChild(td(o.type || "?"));
          tr.appendChild(td(o.target || "–"));
          tr.appendChild(td(o.date || "–"));
          tr.appendChild(pcell);
          tr.appendChild(td(nOps != null ? String(nOps) : "–"));
          t.body.appendChild(tr);
        }
        rb.appendChild(t.table);
      }
      op.appendChild(rb);

      const finished = Array.isArray(ops.finished) ? ops.finished : [];
      const fb = E("div", "subblock"); fb.appendChild(E("div", "subhead", `Finished (${finished.length})`));
      if (!finished.length) fb.appendChild(UI.emptyList("Finished operations", true));
      else {
        const tray = E("div", "diplo-tray");
        for (const o of finished) tray.appendChild(E("span", "role-chip", (o.type || "op") + (o.target ? " → " + o.target : "")));
        fb.appendChild(tray);
      }
      op.appendChild(fb);
    }
    b.appendChild(op);

    // -- cryptology / decryption --------------------------------------------
    const decP = UI.presence(intel, "decryption");
    const dp = E("div", "panel");
    dp.appendChild(E("h3", null, "Cryptology / decryption")).appendChild(UI.srcBadge("S"));
    if (decP.state !== "value") dp.appendChild(UI.notCaptured("Decryption"));
    else {
      const dec = decP.value;
      if (dec.intel_source != null) dp.appendChild(UI.stat("Intel source", { value: dec.intel_source, captured: true }, {}));
      const targets = Array.isArray(dec.targets) ? dec.targets : [];
      if (!targets.length) dp.appendChild(UI.emptyList("Decryption targets", true));
      else {
        const t = T().table(["Target", "Actively decrypting", "Accumulated points"]);
        for (const tg of targets) {
          const tr = E("tr");
          const tc = E("td");
          if (tg.target) { tc.appendChild(S().flag(tg.target, null, "small")); tc.appendChild(document.createTextNode(" " + tg.target)); }
          else tc.textContent = "?";
          tr.appendChild(tc);
          tr.appendChild(td(tg.decryption ? "yes" : "no"));
          tr.appendChild(td(tg.amount != null ? UI.num(tg.amount) : "–"));
          t.body.appendChild(tr);
        }
        dp.appendChild(t.table);
      }
    }
    b.appendChild(dp);

    // -- peace conference ---------------------------------------------------
    const pcP = UI.presence(intel, "peace_conference");
    if (pcP.state === "value" && pcP.value && pcP.value.active) {
      const pp = E("div", "panel");
      pp.appendChild(E("h3", null, "Peace conference")).appendChild(UI.srcBadge("S"));
      pp.appendChild(UI.stat("Active", { value: true, captured: true }, {}));
      b.appendChild(pp);
    }

    // -- intel networks (reserved) ------------------------------------------
    if (Array.isArray(intel.intel_networks) && intel.intel_networks.length) {
      const nw = E("div", "panel");
      nw.appendChild(E("h3", null, `Intel networks (${intel.intel_networks.length})`)).appendChild(UI.srcBadge("S"));
      nw.appendChild(E("pre", "rawjson", JSON.stringify(intel.intel_networks, null, 1)));
      b.appendChild(nw);
    }
  }

  function td(text) { const e = E("td"); e.textContent = text == null ? "–" : String(text); return e; }

  // ---- register the four tab renderers onto National -----------------------
  function install() {
    const N = global.National;
    if (!N) return;
    N.tab_commanders = tab_commanders;
    N.tab_designs = tab_designs;
    N.tab_oob = tab_oob;
    N.tab_intel = tab_intel; // deepened — replaces the stub in national.js
  }
  install();

  global.GameWindows = { install, resolveLeader, operationProgress, buildLeaderIndex, dateToDay };
})(window);
