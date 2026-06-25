/* national.js — National view (country window) + state/division/combat panels.
 *
 * Tabs mirror the game's national window (PLAN §C). Every tab binds generically
 * to the snapshot and shows "not captured" for null/absent — so deferred domains
 * (commanders / designers / OOB / intel-ops) light up automatically when their
 * fields appear, with no rework here.
 */
(function (global) {
  "use strict";
  const S = () => global.Sprites;
  const E = UI.el;

  const TABS = [
    ["politics", "Politics"], ["focus", "National Focus"], ["research", "Research"],
    ["production", "Production"], ["logistics", "Logistics"], ["military", "Military"],
    ["commanders", "Commanders"], ["designs", "Designers"], ["oob", "Order of Battle"],
    ["diplomacy", "Diplomacy"], ["decisions", "Decisions"], ["intel", "Intel"],
  ];

  const National = {
    root: null, header: null, tabbar: null, body: null,
    tab: "politics", tag: null, snap: null,

    init(container) {
      this.root = E("div", "natview");
      this.header = E("div", "nat-header");
      this.tabbar = E("div", "nat-tabs");
      this.body = E("div", "nat-body");
      for (const [id, label] of TABS) {
        const b = E("button", "nat-tab" + (id === this.tab ? " active" : ""), label);
        b.dataset.tab = id;
        b.addEventListener("click", () => { this.tab = id; this.renderTabs(); this.renderBody(); });
        this.tabbar.appendChild(b);
      }
      this.root.appendChild(this.header);
      this.root.appendChild(this.tabbar);
      this.root.appendChild(this.body);
      container.appendChild(this.root);
      return this;
    },

    show(tag, snap) { this.tag = tag; this.snap = snap; this.renderHeader(); this.renderBody(); },
    update(snap) { this.snap = snap; if (this.tag) { this.renderHeader(); this.renderBody(); } },

    country() { return (this.snap && this.snap.countries && this.snap.countries[this.tag]) || {}; },

    /** Was a given save/memory domain captured in this snapshot? (for empty-vs-absent) */
    domainCaptured(domain) {
      const p = (this.snap && this.snap.provenance) || {};
      for (const ch of ["save", "memory"]) {
        const c = p[ch];
        if (c && Array.isArray(c.domains) && c.domains.indexOf(domain) >= 0) return true;
      }
      return false;
    },

    renderTabs() {
      for (const b of this.tabbar.children) b.classList.toggle("active", b.dataset.tab === this.tab);
    },

    renderHeader() {
      const h = UI.clear(this.header);
      const c = this.country(); const pol = c.politics || {};
      h.appendChild(S().flag(this.tag, pol.ruling_party, "medium"));
      const info = E("div", "nat-id");
      info.appendChild(E("div", "nat-tag", this.tag));
      const sub = E("div", "nat-sub");
      const bits = [];
      if (pol.ruling_party) bits.push(pol.ruling_party);
      if (pol.faction && pol.faction.name) bits.push("faction: " + pol.faction.name);
      if (pol.is_human) bits.push("HUMAN");
      if (pol.is_major) bits.push("major");
      sub.textContent = bits.join(" · ") || "—";
      info.appendChild(sub);
      h.appendChild(info);
    },

    renderBody() {
      const b = UI.clear(this.body);
      const fn = this["tab_" + this.tab];
      if (fn) fn.call(this, b);
    },

    // ---- C1 Politics ------------------------------------------------------
    tab_politics(b) {
      const c = this.country(), pol = c.politics || {}, g = (p) => Store.get(pol, p);
      const grid = E("div", "nat-grid");
      const left = E("div", "panel");
      left.appendChild(E("h3", null, "Government")).appendChild(UI.srcBadge("M/S"));
      left.appendChild(UI.stat("Stability", g("stability"), { fmt: UI.pct, src: "M" }));
      left.appendChild(UI.stat("War support", g("war_support"), { fmt: UI.pct, src: "M" }));
      left.appendChild(UI.stat("Political power", g("political_power"), { src: "M" }));
      left.appendChild(UI.stat("Command power", g("command_power"), { src: "M" }));
      left.appendChild(UI.stat("Ruling party", g("ruling_party"), { src: "S" }));
      // party popularities
      if (pol.party_popularities) {
        const pp = E("div", "subblock");
        pp.appendChild(E("div", "subhead", "Party popularity"));
        for (const [ideo, frac] of Object.entries(pol.party_popularities))
          pp.appendChild(UI.stat(ideo, { value: frac, captured: frac != null }, { fmt: UI.pct }));
        left.appendChild(pp);
      }
      grid.appendChild(left);

      // laws
      const laws = E("div", "panel");
      laws.appendChild(E("h3", null, "Laws")).appendChild(UI.srcBadge("S"));
      const L = pol.laws || {};
      laws.appendChild(UI.stat("Economy", Store.get(L, "economy"), { src: "S" }));
      laws.appendChild(UI.stat("Trade", Store.get(L, "trade"), { src: "S" }));
      laws.appendChild(UI.stat("Conscription", Store.get(L, "conscription"), { src: "S" }));
      grid.appendChild(laws);

      // spirits / advisors (idea icons)
      const ideas = E("div", "panel");
      ideas.appendChild(E("h3", null, "National spirits / advisors")).appendChild(UI.srcBadge("S"));
      const arr = pol.ideas_other || [];
      if (!arr.length) ideas.appendChild(UI.notCaptured("Spirits/advisors"));
      else {
        const tray = E("div", "icon-tray");
        for (const idea of arr) {
          const cand = "GFX_idea_" + idea;
          const node = S().has(cand) ? S().icon(cand, { title: idea, size: { w: 48, h: 28 } })
                                     : (() => { const x = E("span", "idea-chip", idea); return x; })();
          tray.appendChild(node);
        }
        ideas.appendChild(tray);
      }
      grid.appendChild(ideas);
      b.appendChild(grid);
    },

    // ---- C2 National Focus (tree w/ goal icons) ---------------------------
    tab_focus(b) {
      const c = this.country(), f = c.focus || {};
      const head = E("div", "panel");
      head.appendChild(E("h3", null, "National Focus")).appendChild(UI.srcBadge("S/D"));
      head.appendChild(UI.stat("Tree", Store.get(f, "focus_tree"), { src: "S" }));
      head.appendChild(UI.stat("Completed", Store.get(f, "completed_count"), { src: "S" }));
      head.appendChild(UI.stat("In progress", Store.get(f, "current"), { src: "S/M" }));
      if (f.current != null) head.appendChild(UI.stat("Progress", Store.get(f, "progress"), { src: "S/M" }));
      if (f.paused) head.appendChild(UI.stat("Paused", { value: true, captured: true }, {}));
      b.appendChild(head);

      const done = f.completed || [];
      const panel = E("div", "panel");
      panel.appendChild(E("h3", null, `Completed focuses (${done.length})`)).appendChild(UI.srcBadge("S/D"));
      if (!done.length) panel.appendChild(UI.emptyList("Completed focuses", this.domainCaptured("focus")));
      else {
        const tray = E("div", "focus-tray");
        for (const fid of done) {
          const cell = E("div", "focus-cell");
          cell.appendChild(S().focusIcon(fid, { size: { w: 40, h: 40 }, title: fid }));
          cell.appendChild(E("span", "focus-id", fid.replace(/^[A-Z]{3}_/, "")));
          cell.title = fid;
          tray.appendChild(cell);
        }
        // current in-progress focus, highlighted
        if (f.current) {
          const cell = E("div", "focus-cell focus-current");
          cell.appendChild(S().focusIcon(f.current, { size: { w: 40, h: 40 }, title: f.current }));
          cell.appendChild(E("span", "focus-id", "▶ " + f.current.replace(/^[A-Z]{3}_/, "")));
          tray.appendChild(cell);
        }
        panel.appendChild(tray);
        panel.appendChild(E("div", "note", "Goal icons from gfx/interface/goals (GFX_focus_<id>); generic fallback where an icon is absent. Tree adjacency/prerequisites are not captured in the snapshot — completion order shown."));
      }
      b.appendChild(panel);
    },

    // ---- C3 Research ------------------------------------------------------
    tab_research(b) {
      const c = this.country(), r = c.research || {};
      const head = E("div", "panel");
      head.appendChild(E("h3", null, "Research")).appendChild(UI.srcBadge("S"));
      head.appendChild(UI.stat("Technologies", Store.get(r, "tech_count"), { src: "S" }));
      head.appendChild(UI.stat("Research slots", Store.get(r, "research_slots"), { src: "S" }));
      b.appendChild(head);

      // queue
      const q = E("div", "panel");
      q.appendChild(E("h3", null, "Research queue")).appendChild(UI.srcBadge("S / M*"));
      const inp = r.in_progress || [];
      if (!inp.length) q.appendChild(UI.emptyList("Research queue", this.domainCaptured("tech")));
      else {
        const t = table(["Technology", "Points factor", "Progress"]);
        for (const it of inp) {
          const pct = it.progress_pct;
          t.body.appendChild(row([
            it.name || "?",
            it.points_factor != null ? UI.num(it.points_factor) : "—",
            pct != null ? UI.pct(pct) : muted("indeterminate (M*)"),
          ]));
        }
        q.appendChild(t.table);
      }
      b.appendChild(q);

      // completed techs (latest few)
      const done = r.technologies || [];
      const dp = E("div", "panel");
      dp.appendChild(E("h3", null, `Completed technologies (${done.length})`)).appendChild(UI.srcBadge("S/D"));
      if (!done.length) dp.appendChild(UI.notCaptured("Technologies"));
      else {
        const sorted = done.slice().sort((a, b2) => String(b2.date || "").localeCompare(String(a.date || "")));
        const t = table(["Technology", "Level", "Completed"]);
        for (const tech of sorted.slice(0, 60))
          t.body.appendChild(row([tech.name || "?", tech.level != null ? tech.level : "—", tech.date || muted("—")]));
        dp.appendChild(t.table);
        if (sorted.length > 60) dp.appendChild(E("div", "note", `Showing 60 of ${sorted.length} most recent.`));
      }
      b.appendChild(dp);
    },

    // ---- C4 Production / construction -------------------------------------
    tab_production(b) {
      const c = this.country(), eco = c.economy || {};
      const cq = eco.construction_queue || [];
      const cp = E("div", "panel");
      cp.appendChild(E("h3", null, `Construction queue (${cq.length})`)).appendChild(UI.srcBadge("S"));
      if (!cq.length) cp.appendChild(UI.emptyList("Construction queue", this.domainCaptured("economy")));
      else {
        const t = table(["Project", "State", "Factories", "Produced", "Cost", "Pri"]);
        for (const it of cq) t.body.appendChild(row([
          it.template || "?", it.location_state != null ? it.location_state : muted("—"),
          it.active_factories != null ? it.active_factories : muted("—"),
          it.produced != null ? UI.num(it.produced) : muted("—"),
          it.cost != null ? UI.num(it.cost) : muted("—"),
          it.priority != null ? it.priority : muted("—"),
        ]));
        cp.appendChild(t.table);
      }
      b.appendChild(cp);

      // equipment stockpile (with icons)
      const stock = c.military && c.military.equipment_stockpile || [];
      const sp = E("div", "panel");
      sp.appendChild(E("h3", null, `Equipment stockpile (${stock.length})`)).appendChild(UI.srcBadge("S"));
      if (!stock.length) sp.appendChild(UI.emptyList("Equipment stockpile", this.domainCaptured("production")));
      else {
        const tray = E("div", "equip-tray");
        for (const it of stock.slice(0, 80)) {
          const cell = E("div", "equip-cell");
          const sprite = "GFX_" + (it.archetype || "");
          cell.appendChild(S().has(sprite) ? S().icon(sprite, { size: { w: 32, h: 24 }, title: it.archetype })
                                           : E("span", "equip-noicon", "□"));
          cell.appendChild(E("span", "equip-amt", UI.num(it.amount)));
          cell.title = (it.archetype || "?") + ": " + UI.num(it.amount);
          tray.appendChild(cell);
        }
        sp.appendChild(tray);
      }
      b.appendChild(sp);
    },

    // ---- C5 Logistics & trade ---------------------------------------------
    tab_logistics(b) {
      const c = this.country(), eco = c.economy || {}, mil = c.military || {};
      const fp = E("div", "panel");
      fp.appendChild(E("h3", null, "Fuel")).appendChild(UI.srcBadge("S / M*"));
      fp.appendChild(UI.stat("Current", Store.get(eco, "fuel.current"), { src: "S" }));
      fp.appendChild(UI.stat("Max", Store.get(eco, "fuel.max"), { src: "S" }));
      fp.appendChild(UI.stat("Convoys", mil.convoys ? { value: mil.convoys.length, captured: true } : { captured: false }, {}));
      b.appendChild(fp);

      const res = eco.resources || {};
      const rp = E("div", "panel");
      rp.appendChild(E("h3", null, "Resources")).appendChild(UI.srcBadge("S/D"));
      const channels = [["produced", "Produced"], ["available_derived", "Available"], ["exported", "Exported"]];
      let any = false;
      for (const [k, label] of channels) {
        if (!res[k]) continue;
        any = true;
        const sub = E("div", "subblock");
        sub.appendChild(E("div", "subhead", label));
        for (const [r, v] of Object.entries(res[k])) {
          const row2 = E("div", "stat");
          row2.appendChild(S().resourceIcon(r, { size: { w: 14, h: 14 } }));
          row2.appendChild(E("span", "stat-label", r));
          row2.appendChild(E("span", "val", UI.num(v)));
          sub.appendChild(row2);
        }
        rp.appendChild(sub);
      }
      if (!any) rp.appendChild(UI.notCaptured("Resources"));
      b.appendChild(rp);
    },

    // ---- C6 Military ------------------------------------------------------
    tab_military(b) {
      const c = this.country(), mil = c.military || {}, g = (p) => Store.get(mil, p);
      const p = E("div", "panel");
      p.appendChild(E("h3", null, "Forces")).appendChild(UI.srcBadge("M/S"));
      p.appendChild(UI.stat("Divisions", g("division_count"), { src: "M/D" }));
      p.appendChild(UI.stat("Ships", g("num_ships"), { src: "M/S" }));
      p.appendChild(UI.stat("Available manpower", g("manpower.available"), { src: "S" }));
      p.appendChild(UI.stat("Army XP", g("xp.army"), { src: "S" }));
      p.appendChild(UI.stat("Navy XP", g("xp.navy"), { src: "S" }));
      p.appendChild(UI.stat("Air XP", g("xp.air"), { src: "S" }));
      p.appendChild(UI.stat("Killed", g("casualties.killed"), { src: "S/D" }));
      p.appendChild(UI.stat("Captured", g("casualties.captured"), { src: "S/D" }));
      b.appendChild(p);

      // template/role breakdown from this tag's divisions
      const divs = (this.snap.divisions || []).filter((d) => d.owner === this.tag);
      const rp = E("div", "panel");
      rp.appendChild(E("h3", null, `Divisions (${divs.length})`)).appendChild(UI.srcBadge("M/S"));
      if (!divs.length) rp.appendChild(UI.notCaptured("Divisions"));
      else {
        const byRole = {};
        for (const d of divs) { const r = d.template_role || d.template_name || "?"; byRole[r] = (byRole[r] || 0) + 1; }
        const tray = E("div", "role-tray");
        for (const [r, n] of Object.entries(byRole).sort((a, b2) => b2[1] - a[1]))
          tray.appendChild(E("span", "role-chip", `${r}: ${n}`));
        rp.appendChild(tray);
      }
      b.appendChild(rp);
    },

    // ---- C7 Diplomacy -----------------------------------------------------
    tab_diplomacy(b) {
      const c = this.country(), dip = c.diplomacy || {};
      const wp = E("div", "panel");
      wp.appendChild(E("h3", null, "Wars")).appendChild(UI.srcBadge("S"));
      const wars = dip.wars || [];
      if (!wars.length) wp.appendChild(UI.emptyList("Wars", this.domainCaptured("diplomacy")));
      else {
        const tray = E("div", "diplo-tray");
        for (const w of wars) {
          const chip = E("span", "diplo-chip war");
          chip.appendChild(S().flag(w.against, null, "small"));
          chip.appendChild(E("span", null, " vs " + (w.against || "?")));
          tray.appendChild(chip);
        }
        wp.appendChild(tray);
      }
      b.appendChild(wp);

      // faction / subjects from world block
      const fp = E("div", "panel");
      fp.appendChild(E("h3", null, "Faction & subjects")).appendChild(UI.srcBadge("S"));
      const pol = c.politics || {};
      fp.appendChild(UI.stat("Faction", pol.faction ? { value: pol.faction.name, captured: !!pol.faction.name } : { captured: false }, {}));
      fp.appendChild(UI.stat("Faction leader", pol.faction ? { value: !!pol.faction.is_leader, captured: pol.faction.is_leader != null } : { captured: false }, {}));
      const subs = ((this.snap.diplomacy_world || {}).subjects || []).filter((s) => s.overlord === this.tag);
      if (subs.length) {
        const sub = E("div", "subblock"); sub.appendChild(E("div", "subhead", "Subjects"));
        for (const s of subs) sub.appendChild(UI.stat(s.subject, { value: s.autonomy_state, captured: !!s.autonomy_state }, {}));
        fp.appendChild(sub);
      }
      b.appendChild(fp);

      // -- lend-lease / equipment market (§3.7) -------------------------------
      // This country's DIRECTED contracts (incoming as buyer, outgoing as seller).
      // SCHEMA §3.7: the SAME contract is attached to BOTH parties; here we show
      // only THIS tag's own directed view, so there is no double-counting. We still
      // group by contract_id (a tag can in principle list one contract once per
      // direction) to render one row per (contract, direction).
      const llp = E("div", "panel");
      llp.appendChild(E("h3", null, "Lend-lease / equipment market")).appendChild(UI.srcBadge("S"));
      const llRaw = Array.isArray(dip.lend_lease) ? dip.lend_lease : null;
      if (!llRaw) {
        llp.appendChild(UI.emptyList("Lend-lease", this.domainCaptured("market") || this.domainCaptured("diplomacy")));
      } else if (!llRaw.length) {
        llp.appendChild(UI.emptyList("Lend-lease", true));
      } else {
        // de-dupe by contract_id|direction (one row per directed contract)
        const seen = new Set();
        const ll = [];
        for (const ct of llRaw) {
          const key = (ct.contract_id || "") + "|" + (ct.direction || "");
          if (seen.has(key)) continue;
          seen.add(key); ll.push(ct);
        }
        const incoming = ll.filter((x) => x.direction === "incoming");
        const outgoing = ll.filter((x) => x.direction === "outgoing");
        for (const [arr, label, dirWord] of [
          [outgoing, "Outgoing (this country supplies)", "to"],
          [incoming, "Incoming (this country receives)", "from"],
        ]) {
          if (!arr.length) continue;
          const sub = E("div", "subblock");
          sub.appendChild(E("div", "subhead", `${label} — ${arr.length}`));
          const t = table(["Counterpart", "Equipment", "Convoys", "Delivery", "State"]);
          for (const ct of arr) {
            const cp = ct.counterpart || (dirWord === "to" ? ct.buyer : ct.seller) || "?";
            const eq = Array.isArray(ct.equipments) && ct.equipments.length
              ? ct.equipments.map((e) => (e && (e.archetype || e.equip_type || e.name)) || "?").slice(0, 3).join(", ")
                + (ct.equipments.length > 3 ? " +" + (ct.equipments.length - 3) : "")
              : "—";
            const conv = ct.convoys && typeof ct.convoys === "object"
              ? `${ct.convoys.assigned != null ? ct.convoys.assigned : "?"}/${ct.convoys.total != null ? ct.convoys.total : "?"}`
              : "—";
            const eff = ct.delivery_efficiency != null ? UI.pct(ct.delivery_efficiency <= 1 ? ct.delivery_efficiency : ct.delivery_efficiency / 100) : "—";
            const cpCell = E("td");
            if (cp && cp !== "?") cpCell.appendChild(S().flag(cp, null, "small"));
            cpCell.appendChild(document.createTextNode(" " + dirWord + " " + cp));
            const tr = E("tr");
            tr.appendChild(cpCell);
            tr.appendChild(td(eq));
            tr.appendChild(td(conv));
            tr.appendChild(td(eff + (ct.completed ? " ✓" : "")));
            // delivery_state is an OBJECT {factory_cic_progress, equipment_transfer_progress,
            // collected} (parse_market) — surface the transfer progress, not [object Object].
            tr.appendChild(td(deliveryStateLabel(ct)));
            t.body.appendChild(tr);
          }
          sub.appendChild(t.table);
          llp.appendChild(sub);
        }
      }
      b.appendChild(llp);

      // -- world equipment-market contracts (canonical, one per contract) -----
      // SCHEMA §7b: diplomacy_world.trade_contracts is the single source of truth
      // (one entry per contract), so world totals use THIS, never the per-party
      // duplicated view (avoids double-counting). Filter to contracts touching
      // this tag for relevance.
      const world = ((this.snap.diplomacy_world || {}).trade_contracts) || null;
      if (Array.isArray(world) && world.length) {
        const mine = world.filter((ct) => ct.seller === this.tag || ct.buyer === this.tag);
        if (mine.length) {
          const wp = E("div", "panel");
          wp.appendChild(E("h3", null, `World market contracts involving ${this.tag} (${mine.length})`)).appendChild(UI.srcBadge("S"));
          const t = table(["Seller", "Buyer", "Equipment", "Efficiency", "Done"]);
          for (const ct of mine) {
            const eq = Array.isArray(ct.equipments) && ct.equipments.length
              ? ct.equipments.map((e) => (e && (e.archetype || e.equip_type || e.name)) || "?").slice(0, 2).join(", ")
              : "—";
            const eff = ct.delivery_efficiency != null ? UI.pct(ct.delivery_efficiency <= 1 ? ct.delivery_efficiency : ct.delivery_efficiency / 100) : "—";
            t.body.appendChild(row([ct.seller || "?", ct.buyer || "?", eq, eff, ct.completed ? "✓" : "—"]));
          }
          wp.appendChild(t.table);
          wp.appendChild(E("div", "note", "World list is the canonical one-per-contract source (SCHEMA §7b); the per-country panel above shows this country's directed view (no double-count)."));
          b.appendChild(wp);
        }
      }
    },

    // ---- C8 Decisions -----------------------------------------------------
    tab_decisions(b) {
      const c = this.country(), dec = c.decisions || {};
      for (const [k, label] of [["active", "Active decisions"], ["timed", "Timed decisions"], ["missions", "Missions"]]) {
        const p = E("div", "panel");
        p.appendChild(E("h3", null, label)).appendChild(UI.srcBadge("S"));
        const arr = dec[k];
        if (!arr || !arr.length) p.appendChild(UI.notCaptured(label));
        else {
          const tray = E("div", "diplo-tray");
          for (const it of arr) tray.appendChild(E("span", "role-chip", typeof it === "string" ? it : (it.name || JSON.stringify(it))));
          p.appendChild(tray);
        }
        b.appendChild(p);
      }
      b.appendChild(E("div", "note", "One-shot decision clicks are not stored (SCHEMA §3.6); only persistent decisions/missions appear."));
    },

    // ---- C9 Intel, plus C-extra Commanders/Designers/Order-of-Battle ------
    // These four advanced tabs (intel deepened, + commanders/designs/oob) are
    // defined in gamewindows.js and installed onto National at load (they bind
    // the §3.8/§3.9/§3.10/§11/§12 domains). gamewindows.js's tab_intel replaces
    // any stub here. Kept out of this file so the moving schema lives in one place.
  };

  // ---- table helpers --------------------------------------------------------
  function table(headers) {
    const t = E("table", "data-table");
    const thead = E("thead"); const tr = E("tr");
    for (const h of headers) tr.appendChild(E("th", null, h));
    thead.appendChild(tr); t.appendChild(thead);
    const body = E("tbody"); t.appendChild(body);
    return { table: t, body };
  }
  function row(cells) {
    const tr = E("tr");
    for (const c of cells) {
      const td = E("td");
      if (c instanceof Node) td.appendChild(c); else td.textContent = c;
      tr.appendChild(td);
    }
    return tr;
  }
  function muted(text) { return E("span", "val-missing", text); }
  function td(text) { const e = E("td"); e.textContent = text == null ? "—" : String(text); return e; }
  /** delivery_state is {factory_cic_progress, equipment_transfer_progress, collected}
   *  (parse_market) — render a compact label, never "[object Object]". */
  function deliveryStateLabel(ct) {
    const ds = ct && ct.delivery_state;
    if (ds && typeof ds === "object") {
      const tp = ds.equipment_transfer_progress;
      if (tp != null) return "transfer " + UI.pct(tp <= 1 ? tp : tp / 100);
      const col = ds.collected;
      if (col != null) return "collected " + UI.num(col);
      return ct.delivery_country || "—";
    }
    if (ds != null) return String(ds);
    return ct && ct.delivery_country ? ct.delivery_country : "—";
  }

  global.National = National;
  global.NatTable = { table, row, muted };
})(window);
