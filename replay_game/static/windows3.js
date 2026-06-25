/* windows3.js — Layer-2 tab ENRICHMENT pass 2 (PLAN §C7 completeness).
 *
 * The Diplomacy tab (national.js core + windows2.js enrichment) surfaces wars,
 * treaty/relation lists, the opinion matrix, and the per-war lifecycle — but it
 * shows the selected country's faction only as a NAME. The world block
 * `diplomacy_world.factions` carries the full roster (members[], leader, ideology,
 * icon, template, leader_change_date) and is never rendered. This module appends a
 * Faction roster panel by WRAPPING tab_diplomacy (calling the prior renderer first,
 * unchanged) so the chain national -> windows2 -> windows3 is preserved.
 *
 * It binds through the same generic store/presence path so it renders an honest
 * empty/absent state on a snapshot without factions and lights up when they appear.
 *
 * Two hard rules, enforced here (Codex-reviewed):
 *   - No fabrication. The selected country is linked to a faction by MEMBERSHIP
 *     FIRST (sel in members[]), then by an explicit faction id, then a unique
 *     template/object-key match, then a unique name match — never by `icon` (not a
 *     reliable unique linker), never by guessing. If nothing matches we still show
 *     the captured faction name and say the roster is "not linked", inventing
 *     nothing. Members/leader are shown only as captured.
 *   - Object keys / member lists are rendered in captured order (members) and
 *     factions iterated in sorted key order for deterministic output.
 */
(function (global) {
  "use strict";
  const S = () => global.Sprites;
  const E = UI.el;
  const TAG_RE = /^[A-Z0-9]{3}$/;
  const isTag = (x) => typeof x === "string" && TAG_RE.test(x);

  /** A flag+TAG chip (flag only for a valid TAG; degrades honestly otherwise). */
  function tagChip(tag, extraCls) {
    const chip = E("span", "diplo-chip" + (extraCls ? " " + extraCls : ""));
    if (isTag(tag)) { chip.appendChild(S().flag(tag, null, "small")); chip.appendChild(E("span", null, " " + tag)); }
    else chip.appendChild(E("span", null, tag == null ? "—" : String(tag)));
    return chip;
  }

  /**
   * Link the selected TAG to a faction object in diplomacy_world.factions WITHOUT
   * fabricating (Codex matching order). Returns {fid, fac, how} or null.
   *   1) membership:  sel listed in fac.members[]   (authoritative)
   *   2) explicit id: politics.faction.id === fid / fac.id
   *   3) template:    UNIQUE fac.template === faction.name
   *   4) name:        UNIQUE (fid|fac.id|fac.template) === faction.name
   * `icon` is never used as a linker.
   */
  function linkFaction(sel, polFaction, facs) {
    const ids = Object.keys(facs);
    // 1) membership — the only authoritative link. If `sel` is listed in MORE THAN
    //    ONE faction's members (corrupt/contradictory capture), do NOT fabricate a
    //    single authoritative link from an ambiguous state — return an ambiguous
    //    marker so the renderer is honest about it (Codex).
    const memberOf = ids.filter((fid) => Array.isArray(facs[fid].members) && facs[fid].members.indexOf(sel) >= 0);
    if (memberOf.length === 1) return { fid: memberOf[0], fac: facs[memberOf[0]], how: "membership" };
    if (memberOf.length > 1) return { ambiguous: true, fids: memberOf };
    if (!polFaction || typeof polFaction !== "object") return null;
    // 2) explicit id on the country's faction stub.
    if (polFaction.id != null) {
      const want = String(polFaction.id);
      for (const fid of ids) if (fid === want || String(facs[fid].id) === want)
        return { fid, fac: facs[fid], how: "id" };
    }
    const name = polFaction.name;
    if (name == null) return null;
    // 3) unique template match.
    const byTpl = ids.filter((fid) => facs[fid].template === name);
    if (byTpl.length === 1) return { fid: byTpl[0], fac: facs[byTpl[0]], how: "template" };
    // 4) unique match on key / id / template == name.
    const byAny = ids.filter((fid) => fid === name || String(facs[fid].id) === name || facs[fid].template === name);
    if (byAny.length === 1) return { fid: byAny[0], fac: facs[byAny[0]], how: "name" };
    return null;
  }

  function tab_diplomacy(b) {
    if (origDiplomacy) origDiplomacy.call(this, b); // national -> windows2 chain, unchanged

    const sel = this.tag, c = this.country(), pol = c.politics || {};
    const facs = (this.snap.diplomacy_world || {}).factions;
    const domCap = this.domainCaptured("diplomacy");

    const fp = E("div", "panel");
    fp.appendChild(E("h3", null, "Faction roster")).appendChild(UI.srcBadge("S"));

    if (!facs || typeof facs !== "object") { fp.appendChild(UI.notCaptured("World factions")); b.appendChild(fp); return; }
    if (!Object.keys(facs).length) { fp.appendChild(UI.emptyList("World factions", domCap)); b.appendChild(fp); return; }

    const link = linkFaction(sel, pol.faction, facs);
    if (link && link.ambiguous) {
      // sel appears in multiple captured faction rosters — honest, no single pick.
      fp.appendChild(E("div", "block-missing", sel + " is listed in " + link.fids.length +
        " captured faction rosters (ids " + link.fids.join(", ") + ") — ambiguous; no single faction shown (no inference)."));
      b.appendChild(fp);
      // still show the world overview table below so the picture isn't lost.
      appendWorldFactions(b, facs, null);
      return;
    }
    if (!link) {
      // Honest fallback (Codex): show the captured faction name if any, say roster not linked.
      const nm = pol.faction && pol.faction.name;
      if (nm) {
        fp.appendChild(UI.stat("Faction (captured name)", { value: nm, captured: true }, { src: "S" }));
        fp.appendChild(E("div", "note", sel + " is not listed in any captured diplomacy_world.factions roster and no id/template/name uniquely matches — faction roster not linked (no inference)."));
      } else {
        fp.appendChild(UI.emptyList(sel + " faction membership", domCap));
        fp.appendChild(E("div", "note", "No faction membership captured for " + sel + "."));
      }
      b.appendChild(fp);
      return;
    }

    // Render the linked faction.
    const fac = link.fac;
    const head = E("div", "char-card");
    if (fac.icon && S().has(fac.icon)) head.appendChild(S().icon(fac.icon, { size: { w: 40, h: 40 }, title: fac.template || fac.id }));
    const meta = E("div", "char-meta");
    meta.appendChild(E("div", "char-name", fac.template ? fac.template.replace(/^faction_template_/, "").replace(/_/g, " ") : ("faction " + (fac.id != null ? fac.id : link.fid))));
    const sub = [];
    if (fac.ideology) sub.push(fac.ideology);
    if (fac.id != null) sub.push("id " + fac.id);
    sub.push("linked by " + link.how);
    meta.appendChild(E("div", "char-sub", sub.join(" · ")));
    head.appendChild(meta);
    fp.appendChild(head);

    // leader.
    const leadRow = E("div", "stat");
    leadRow.appendChild(E("span", "stat-label", "Faction leader"));
    if (fac.leader != null) { leadRow.appendChild(tagChip(fac.leader, fac.leader === sel ? "war" : null)); }
    else leadRow.appendChild(E("span", "val-missing", "not captured"));
    fp.appendChild(leadRow);
    if (fac.leader_change_date) fp.appendChild(UI.stat("Leader since", { value: fac.leader_change_date, captured: true }, { src: "S" }));

    // members (captured order; highlight the selected country + leader).
    const members = Array.isArray(fac.members) ? fac.members : null;
    const sub2 = E("div", "subblock");
    sub2.appendChild(E("div", "subhead", "Members" + (members ? " (" + members.length + ")" : "")));
    if (!members) sub2.appendChild(UI.notCaptured("Faction members"));
    else if (!members.length) sub2.appendChild(UI.emptyList("Faction members", domCap));
    else {
      const tray = E("div", "diplo-tray");
      for (const m of members) {
        // leader -> the existing .war accent; the selected country -> an inline
        // accent (no new CSS class; app.css is outside this track's .js scope).
        const chip = tagChip(m, m === fac.leader ? "war" : null);
        if (m === sel) { chip.style.outline = "1px solid var(--accent, #6aa0ff)"; chip.title = "selected country"; }
        else if (m === fac.leader) chip.title = "faction leader";
        tray.appendChild(chip);
      }
      sub2.appendChild(tray);
    }
    fp.appendChild(sub2);
    b.appendChild(fp);

    // World factions overview (all factions, deterministic order).
    appendWorldFactions(b, facs, link ? link.fid : null);
  }

  /** All-factions roster table — the broader diplomatic picture the Faction map
   * mode colors; here as a list, deterministic by fid. highlightFid -> accent row. */
  function appendWorldFactions(b, facs, highlightFid) {
    const wp = E("div", "panel");
    const ids = Object.keys(facs).sort((a, d) => a.localeCompare(d, undefined, { numeric: true }));
    wp.appendChild(E("h3", null, "All factions (" + ids.length + ")")).appendChild(UI.srcBadge("S"));
    const t = global.NatTable.table(["Faction", "Ideology", "Leader", "Members"]);
    for (const fid of ids) {
      const f = facs[fid];
      const nameCell = E("td");
      if (f.icon && S().has(f.icon)) { const ic = S().icon(f.icon, { size: { w: 20, h: 20 } }); ic.style.verticalAlign = "middle"; nameCell.appendChild(ic); }
      nameCell.appendChild(document.createTextNode(" " + (f.template ? f.template.replace(/^faction_template_/, "") : ("#" + (f.id != null ? f.id : fid)))));
      const leadCell = E("td");
      if (f.leader != null) leadCell.appendChild(tagChip(f.leader));
      else leadCell.appendChild(global.NatTable.muted("—"));
      const memCell = E("td", null, Array.isArray(f.members) ? String(f.members.length) : "—");
      const tr = E("tr");
      if (highlightFid != null && fid === highlightFid) tr.style.background = "#243049";
      tr.appendChild(nameCell);
      tr.appendChild(E("td", null, f.ideology || "—"));
      tr.appendChild(leadCell);
      tr.appendChild(memCell);
      t.body.appendChild(tr);
    }
    wp.appendChild(t.table);
    b.appendChild(wp);
  }

  tab_diplomacy._w3 = true; // marker so re-install can detect double-wrapping

  // ---- install: wrap whatever tab_diplomacy is current (national or windows2) ----
  // Capture the CURRENT renderer as orig only if it isn't already this wrapper, so:
  //  - normal load order (national -> windows2 -> windows3) captures the windows2
  //    wrapper and the chain is national->windows2->windows3;
  //  - if Windows2.install() is later re-run it clobbers us with its own wrapper;
  //    re-running install() here re-captures THAT wrapper (not our own — the _w3
  //    marker prevents wrapping ourselves into an infinite loop) and re-asserts the
  //    chain. Idempotent + self-healing (Codex).
  let origDiplomacy;
  function install() {
    const N = global.National;
    if (!N) return;
    const cur = N.tab_diplomacy;
    if (cur && !cur._w3) origDiplomacy = cur; // never capture our own wrapper
    N.tab_diplomacy = tab_diplomacy;
  }
  install();

  global.Windows3 = { install, linkFaction };
})(window);
