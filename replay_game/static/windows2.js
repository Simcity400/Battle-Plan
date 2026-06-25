/* windows2.js — Layer-2 tab ENRICHMENT (PLAN §C completeness pass).
 *
 * The core National tabs (national.js) and the four advanced tabs (gamewindows.js)
 * cover every PLAN §C window, but several tabs surface only a SUBSET of the fields
 * the snapshot schema actually carries. This module fills those gaps by WRAPPING
 * the existing tab renderer (calling it first, unchanged) and then appending the
 * remaining schema-backed panels:
 *
 *   - Diplomacy (C7)  + guarantees / non-aggression / embargoes / opinion /
 *                       lend-lease / volunteers / attaches / expeditionary,
 *                       and the world war-id lifecycle from diplomacy_world.wars.
 *   - Politics (C1)   + cabinet (leader/advisors/theorists), capital state, autonomy.
 *   - Research (C3)   + research bonuses (per category).
 *   - Production (C4) + factory breakdown (offmap / from-states / under-construction),
 *                       gross IC by equipment, ships-by-hull.
 *   - Logistics (C5)  + to-export channel + lend-lease summary.
 *
 * It binds through the SAME generic store path (Store.get / UI.presence /
 * domainCaptured) so every added panel renders "not captured" on an older
 * snapshot and lights up automatically when its field appears — no rework.
 *
 * Two hard rules, enforced here (Codex-confirmed):
 *   - No fabrication. A null/absent field => "not captured"; a present-empty list
 *     => "none (captured)" ONLY when its provenance domain was captured. A field
 *     that is "null until a pass runs" (gross_ic_by_equipment, ships_by_hull,
 *     opinion, cabinet) => "not captured" even when the broader domain WAS captured.
 *   - Diplomacy list element shapes are partly unspecified: a string renders as a
 *     TAG (flag only when it passes the TAG validator); an object renders via the
 *     first string-valued known key (target/country/against/tag/subject/overlord),
 *     else a compact raw-detail line — never an inferred/guessed TAG.
 *   - Object keys are sorted before render for deterministic output.
 */
(function (global) {
  "use strict";
  const S = () => global.Sprites;
  const E = UI.el;
  const T = () => global.NatTable;

  // A base TAG is 3 chars (A-Z, plus digits for some modded tags). Flags degrade
  // honestly via Sprites.flag (grey chip) for anything not in the asset set, so we
  // validate loosely and never fabricate — a non-TAG string is shown as plain text.
  const TAG_RE = /^[A-Z0-9]{3}$/;
  function isTag(x) { return typeof x === "string" && TAG_RE.test(x); }

  /** A flag+label cell for a value that MAY be a TAG (string) or not. */
  function tagCell(v) {
    const span = E("span", "diplo-chip");
    if (isTag(v)) { span.appendChild(S().flag(v, null, "small")); span.appendChild(E("span", null, " " + v)); }
    else span.appendChild(E("span", null, String(v)));
    return span;
  }

  /** Resolve a diplomacy list element to a display chip without fabricating a TAG. */
  const ELEM_KEYS = ["target", "country", "against", "tag", "subject", "overlord", "to", "with"];
  function diploElemChip(el) {
    if (el == null) return E("span", "diplo-chip unresolved", "—");
    if (typeof el === "string" || typeof el === "number") return tagCell(String(el));
    if (typeof el === "object") {
      // first known key whose value is a string -> primary subject (+ flag if TAG)
      let primary = null;
      for (const k of ELEM_KEYS) { if (typeof el[k] === "string") { primary = el[k]; break; } }
      const chip = E("span", "diplo-chip");
      if (primary != null) {
        if (isTag(primary)) { chip.appendChild(S().flag(primary, null, "small")); chip.appendChild(E("span", null, " " + primary)); }
        else chip.appendChild(E("span", null, primary));
      } else {
        // no known string key — show a compact raw-detail line, labelled as raw.
        chip.classList.add("unresolved");
        chip.appendChild(E("span", null, compact(el)));
        chip.title = "raw captured detail (element shape not specified in schema)";
        return chip;
      }
      // append a couple of secondary scalar attributes (raw, never interpreted)
      const extra = [];
      for (const k of Object.keys(el).sort()) {
        if (ELEM_KEYS.indexOf(k) >= 0) continue;
        const val = el[k];
        if (val == null || typeof val === "object") continue;
        extra.push(k.replace(/_/g, " ") + ": " + (typeof val === "number" ? UI.num(val) : String(val)));
      }
      if (extra.length) { const s = E("span", "diplo-extra", " · " + extra.join(", ")); s.style.opacity = ".7"; chip.appendChild(s); }
      return chip;
    }
    return E("span", "diplo-chip unresolved", String(el));
  }
  function compact(o) {
    try { const s = JSON.stringify(o); return s.length > 80 ? s.slice(0, 77) + "…" : s; }
    catch (e) { return String(o); }
  }

  /** A panel that renders a diplomacy relation list with honest empty/absent states. */
  function relationPanel(self, dip, key, label, domCap) {
    const p = E("div", "panel");
    const arr = dip ? dip[key] : undefined;
    const n = Array.isArray(arr) ? arr.length : 0;
    p.appendChild(E("h3", null, `${label}` + (Array.isArray(arr) ? ` (${n})` : ""))).appendChild(UI.srcBadge("S"));
    if (!(dip && key in dip)) p.appendChild(UI.notCaptured(label));
    else if (!Array.isArray(arr) || !arr.length) p.appendChild(UI.emptyList(label, domCap));
    else {
      const tray = E("div", "diplo-tray");
      for (const el of arr) tray.appendChild(diploElemChip(el));
      p.appendChild(tray);
    }
    return p;
  }

  // ===========================================================================
  // C7 — Diplomacy (enriched)
  // ===========================================================================
  function tab_diplomacy(b) {
    // 1) the original wars + faction/subjects panels, unchanged.
    if (origDiplomacy) origDiplomacy.call(this, b);

    const c = this.country(), dip = c.diplomacy || {};
    const domCap = this.domainCaptured("diplomacy");

    // 2) treaty/relation lists the core tab omits.
    for (const [k, label] of [
      ["guarantees", "Guarantees"],
      ["non_aggression", "Non-aggression pacts"],
      ["embargoes", "Embargoes"],
      ["lend_lease", "Lend-lease"],
      ["volunteers", "Volunteers"],
      ["attaches", "Military attachés"],
      ["expeditionary", "Expeditionary forces"],
    ]) {
      // only show a panel if the country domain object exists at all (avoid a wall
      // of "not captured" on a country with no diplomacy block); but if the key is
      // present we always render it (so a captured-empty list reads honestly).
      if (!(k in dip) && !domCap) continue;
      b.appendChild(relationPanel(this, dip, k, label, domCap));
    }

    // 3) opinion matrix (large obj; null until parsed) — sorted, top by |value|.
    const op = E("div", "panel");
    op.appendChild(E("h3", null, "Opinion")).appendChild(UI.srcBadge("S"));
    const opn = UI.presence(dip, "opinion");
    if (opn.state !== "value" || typeof opn.value !== "object") op.appendChild(UI.notCaptured("Opinion matrix"));
    else {
      const ents = Object.entries(opn.value).filter(([, v]) => v != null);
      if (!ents.length) op.appendChild(UI.emptyList("Opinion", domCap));
      else {
        ents.sort((a, d) => Math.abs(d[1]) - Math.abs(a[1]) || a[0].localeCompare(d[0]));
        const t = T().table(["Country", "Opinion"]);
        for (const [tag, v] of ents.slice(0, 60)) {
          const tc = E("td");
          if (isTag(tag)) { tc.appendChild(S().flag(tag, null, "small")); tc.appendChild(document.createTextNode(" " + tag)); }
          else tc.textContent = tag;
          const tr = E("tr"); tr.appendChild(tc);
          const vc = E("td", null, UI.num(v)); if (v < 0) vc.style.color = "var(--bad)"; else if (v > 0) vc.style.color = "var(--good)";
          tr.appendChild(vc); t.body.appendChild(tr);
        }
        op.appendChild(t.table);
        if (ents.length > 60) op.appendChild(E("div", "note", `Showing 60 of ${ents.length} (largest magnitude first).`));
      }
    }
    b.appendChild(op);

    // 4) world war-id lifecycle (diplomacy_world.wars) involving this tag — the
    //    authoritative per-war record with casualties + war score (§7b).
    const wars = (this.snap.diplomacy_world || {}).wars;
    if (wars && typeof wars === "object") {
      const mine = Object.entries(wars).filter(([, w]) => w && (w.a === this.tag || w.b === this.tag));
      if (mine.length) {
        const wp = E("div", "panel");
        wp.appendChild(E("h3", null, `Wars — detailed (${mine.length})`)).appendChild(UI.srcBadge("S"));
        const t = T().table(["Side A", "Side B", "Started", "Casualties A", "Casualties B", "Score A→B", "Score B→A"]);
        for (const [, w] of mine) {
          const ca = E("td"); if (isTag(w.a)) { ca.appendChild(S().flag(w.a, null, "small")); ca.appendChild(document.createTextNode(" " + w.a)); } else ca.textContent = w.a || "?";
          const cb = E("td"); if (isTag(w.b)) { cb.appendChild(S().flag(w.b, null, "small")); cb.appendChild(document.createTextNode(" " + w.b)); } else cb.textContent = w.b || "?";
          const tr = E("tr");
          tr.appendChild(ca); tr.appendChild(cb);
          tr.appendChild(cell(w.start_date));
          tr.appendChild(cell(w.a_casualties != null ? UI.num(w.a_casualties) : null));
          tr.appendChild(cell(w.b_casualties != null ? UI.num(w.b_casualties) : null));
          tr.appendChild(cell(w.war_score_a_vs_b != null ? UI.num(w.war_score_a_vs_b) : null));
          tr.appendChild(cell(w.war_score_b_vs_a != null ? UI.num(w.war_score_b_vs_a) : null));
          t.body.appendChild(tr);
        }
        wp.appendChild(t.table);
        b.appendChild(wp);
      }
    }
  }

  // ===========================================================================
  // C1 — Politics (enriched: cabinet, capital, autonomy)
  // ===========================================================================
  function tab_politics(b) {
    if (origPolitics) origPolitics.call(this, b);
    const c = this.country(), pol = c.politics || {};

    // capital + autonomy (small extras).
    const ep = E("div", "panel");
    ep.appendChild(E("h3", null, "State & subject status")).appendChild(UI.srcBadge("S"));
    ep.appendChild(UI.stat("Capital state", Store.get(pol, "capital_state"), { src: "S" }));
    ep.appendChild(UI.stat("Major power", Store.get(pol, "is_major"), { src: "S" }));
    ep.appendChild(UI.stat("Autonomy level", Store.get(pol, "autonomy.level"), { src: "S" }));
    const ov = Store.get(pol, "autonomy.overlord");
    const ovRow = E("div", "stat"); ovRow.appendChild(E("span", "stat-label", "Overlord"));
    if (ov.captured && isTag(ov.value)) { ovRow.appendChild(S().flag(ov.value, null, "small")); ovRow.appendChild(E("span", "val", ov.value)); }
    else ovRow.appendChild(UI.value(ov));
    ep.appendChild(ovRow);
    b.appendChild(ep);

    // cabinet (null until parsed -> not captured even if politics captured).
    const cabP = UI.presence(pol, "cabinet");
    const cp = E("div", "panel");
    cp.appendChild(E("h3", null, "Cabinet")).appendChild(UI.srcBadge("S"));
    if (cabP.state !== "value" || typeof cabP.value !== "object") cp.appendChild(UI.notCaptured("Cabinet (characters)"));
    else {
      const cab = cabP.value;
      if (cab.leader && typeof cab.leader === "object") {
        const card = E("div", "char-card");
        card.appendChild(UI.portrait(cab.leader.portrait_gfx, cab.leader.name, 44));
        const meta = E("div", "char-meta");
        meta.appendChild(E("div", "char-name", cab.leader.name || "(leader)"));
        meta.appendChild(E("div", "char-sub", cab.leader.ideology || cab.leader.token || "—"));
        card.appendChild(meta); cp.appendChild(card);
      } else if (cab.leader != null) cp.appendChild(UI.stat("Leader", { value: String(cab.leader), captured: true }, {}));
      for (const [k, label] of [["advisors", "Advisors"], ["theorists", "Theorists"]]) {
        const arr = cab[k];
        const sub = E("div", "subblock"); sub.appendChild(E("div", "subhead", `${label} (${Array.isArray(arr) ? arr.length : 0})`));
        if (!Array.isArray(arr) || !arr.length) sub.appendChild(UI.emptyList(label, true));
        else {
          const tray = E("div", "diplo-tray");
          for (const a of arr) tray.appendChild(E("span", "role-chip", (typeof a === "string" ? a : (a && (a.name || a.token || a.idea_token)) || compact(a)).replace(/_/g, " ")));
          sub.appendChild(tray);
        }
        cp.appendChild(sub);
      }
    }
    b.appendChild(cp);
  }

  // ===========================================================================
  // C3 — Research (enriched: research bonuses)
  // ===========================================================================
  function tab_research(b) {
    if (origResearch) origResearch.call(this, b);
    const c = this.country(), r = c.research || {};
    const arr = r.research_bonuses;
    const p = E("div", "panel");
    p.appendChild(E("h3", null, `Research bonuses (${Array.isArray(arr) ? arr.length : 0})`)).appendChild(UI.srcBadge("S"));
    if (!("research_bonuses" in r)) p.appendChild(UI.notCaptured("Research bonuses"));
    else if (!Array.isArray(arr) || !arr.length) p.appendChild(UI.emptyList("Research bonuses", this.domainCaptured("tech")));
    else {
      const t = T().table(["Name", "Categories", "Bonus", "Kind"]);
      for (const bn of arr) t.body.appendChild(T().row([
        (bn.name || "?").replace(/_/g, " "),
        Array.isArray(bn.categories) ? bn.categories.map((x) => String(x).replace(/_/g, " ")).join(", ") : "–",
        bn.bonus != null ? (bn.bonus <= 1 && bn.bonus >= -1 ? UI.pct(bn.bonus) : UI.num(bn.bonus)) : "–",
        bn.kind || "–",
      ]));
      p.appendChild(t.table);
    }
    b.appendChild(p);
  }

  // ===========================================================================
  // C4 — Production (enriched: factory breakdown, gross IC, ships-by-hull)
  // ===========================================================================
  function tab_production(b) {
    if (origProduction) origProduction.call(this, b);
    const c = this.country(), eco = c.economy || {}, mil = c.military || {};

    // factory breakdown (offmap / from-states / under-construction).
    const f = eco.factories || {};
    const fp = E("div", "panel");
    fp.appendChild(E("h3", null, "Factory breakdown")).appendChild(UI.srcBadge("S"));
    const g = (p) => Store.get(eco, "factories." + p);
    fp.appendChild(UI.stat("Civ (effective)", g("civ"), { src: "S" }));
    fp.appendChild(UI.stat("Mil (effective)", g("mil"), { src: "S" }));
    fp.appendChild(UI.stat("Dockyards (effective)", g("dockyard"), { src: "S" }));
    fp.appendChild(UI.stat("Under construction", g("under_construction"), { src: "S" }));
    if (f.offmap || f.civ_from_states != null) {
      const sub = E("div", "subblock"); sub.appendChild(E("div", "subhead", "From states vs offmap"));
      sub.appendChild(UI.stat("Civ from states", g("civ_from_states"), {}));
      sub.appendChild(UI.stat("Mil from states", g("mil_from_states"), {}));
      sub.appendChild(UI.stat("Dock from states", g("dockyard_from_states"), {}));
      sub.appendChild(UI.stat("Offmap civ", g("offmap.civ"), {}));
      sub.appendChild(UI.stat("Offmap mil", g("offmap.mil"), {}));
      sub.appendChild(UI.stat("Offmap dock", g("offmap.dockyard"), {}));
      fp.appendChild(sub);
    }
    b.appendChild(fp);

    // gross IC by equipment (null until the production-delta pass runs -> not captured).
    const giP = UI.presence(eco, "gross_ic_by_equipment");
    const gp = E("div", "panel");
    gp.appendChild(E("h3", null, "Gross IC output by equipment")).appendChild(UI.srcBadge("S/D"));
    if (giP.state !== "value" || typeof giP.value !== "object") {
      gp.appendChild(UI.notCaptured("Gross IC by equipment (production-delta pass)"));
    } else {
      const ents = Object.entries(giP.value).filter(([, v]) => v != null);
      if (!ents.length) gp.appendChild(UI.emptyList("Gross IC by equipment", this.domainCaptured("production")));
      else {
        ents.sort((a, d) => (d[1] - a[1]) || a[0].localeCompare(d[0]));
        const tray = E("div", "equip-tray");
        for (const [arch, v] of ents.slice(0, 80)) {
          const cell0 = E("div", "equip-cell");
          const sprite = "GFX_" + arch;
          cell0.appendChild(S().has(sprite) ? S().icon(sprite, { size: { w: 32, h: 24 }, title: arch }) : E("span", "equip-noicon", "□"));
          cell0.appendChild(E("span", "equip-amt", UI.num(v)));
          cell0.title = arch + ": " + UI.num(v) + " IC";
          tray.appendChild(cell0);
        }
        gp.appendChild(tray);
      }
    }
    b.appendChild(gp);

    // ships by hull (null until parsed).
    const shP = UI.presence(mil, "ships_by_hull");
    const sp = E("div", "panel");
    sp.appendChild(E("h3", null, "Ships by hull")).appendChild(UI.srcBadge("S/D"));
    if (shP.state !== "value" || typeof shP.value !== "object") sp.appendChild(UI.notCaptured("Ships by hull"));
    else {
      const ents = Object.entries(shP.value).filter(([, v]) => v != null);
      if (!ents.length) sp.appendChild(UI.emptyList("Ships by hull", this.domainCaptured("military")));
      else {
        ents.sort((a, d) => a[0].localeCompare(d[0]));
        const t = T().table(["Hull", "Count", "Tonnage"]);
        for (const [hull, o] of ents) {
          const oo = (o && typeof o === "object") ? o : {};
          t.body.appendChild(T().row([hull.replace(/_/g, " "),
            oo.count != null ? UI.num(oo.count) : "–", oo.tonnage != null ? UI.num(oo.tonnage) : "–"]));
        }
        sp.appendChild(t.table);
      }
    }
    b.appendChild(sp);
  }

  // ===========================================================================
  // C5 — Logistics (enriched: to-export channel + lend-lease summary)
  // ===========================================================================
  function tab_logistics(b) {
    if (origLogistics) origLogistics.call(this, b);
    const c = this.country(), eco = c.economy || {};
    const res = eco.resources || {};

    // to_export channel — the core tab shows produced/available/exported only.
    const te = res.to_export;
    if (te && typeof te === "object") {
      const p = E("div", "panel");
      p.appendChild(E("h3", null, "Resources — to export")).appendChild(UI.srcBadge("S"));
      const ents = Object.entries(te).filter(([, v]) => v != null).sort((a, d) => a[0].localeCompare(d[0]));
      if (!ents.length) p.appendChild(UI.emptyList("To-export", this.domainCaptured("economy")));
      else for (const [r, v] of ents) {
        const row2 = E("div", "stat");
        row2.appendChild(S().resourceIcon(r, { size: { w: 14, h: 14 } }));
        row2.appendChild(E("span", "stat-label", r));
        row2.appendChild(E("span", "val", UI.num(v)));
        p.appendChild(row2);
      }
      b.appendChild(p);
    }

    // lend-lease summary (count) — the detail lives on the Diplomacy tab.
    const dip = c.diplomacy || {};
    if ("lend_lease" in dip || "imported" in res) {
      const p = E("div", "panel");
      p.appendChild(E("h3", null, "Trade & lend-lease")).appendChild(UI.srcBadge("S"));
      const ll = dip.lend_lease;
      p.appendChild(UI.stat("Active lend-lease agreements", Array.isArray(ll) ? { value: ll.length, captured: true } : { captured: false }, {}));
      const imp = res.imported;
      if (imp && typeof imp === "object") {
        const sub = E("div", "subblock"); sub.appendChild(E("div", "subhead", "Imported resources"));
        const ents = Object.entries(imp).filter(([, v]) => v != null).sort((a, d) => a[0].localeCompare(d[0]));
        if (!ents.length) sub.appendChild(UI.emptyList("Imported", this.domainCaptured("economy")));
        else for (const [r, v] of ents) sub.appendChild(UI.stat(r, { value: v, captured: true }, {}));
        p.appendChild(sub);
      }
      b.appendChild(p);
    }
  }

  function cell(text) { const e = E("td"); e.textContent = text == null ? "–" : String(text); return e; }

  // ---- install: wrap the core renderers (capture originals first) -----------
  let origDiplomacy, origPolitics, origResearch, origProduction, origLogistics;
  function install() {
    const N = global.National;
    if (!N) return;
    // idempotent: only capture an original once (so a re-install doesn't wrap a wrapper).
    if (!install._done) {
      origDiplomacy = N.tab_diplomacy;
      origPolitics = N.tab_politics;
      origResearch = N.tab_research;
      origProduction = N.tab_production;
      origLogistics = N.tab_logistics;
      install._done = true;
    }
    N.tab_diplomacy = tab_diplomacy;
    N.tab_politics = tab_politics;
    N.tab_research = tab_research;
    N.tab_production = tab_production;
    N.tab_logistics = tab_logistics;
  }
  install();

  global.Windows2 = { install, isTag, diploElemChip };
})(window);
