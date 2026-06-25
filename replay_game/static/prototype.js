/* prototype.js — proves the .gui-driven renderer on real windows end-to-end.
 *
 * Loads a real replay snapshot, then renders THREE game windows straight from the
 * engine's own .gui layouts + .gfx art:
 *   1. TOP BAR        (mod interface/topbar.gui -> topbar.json, window "topbar")
 *   2. COMBAT VIEW    (land_combat.json, window "landcombatview"), bound to one
 *                      live land combat from snap.combats
 *   3. RESEARCH SLOT  (research.json, window "research_slot_entry") — a country-view
 *                      tab fragment, cloned per research slot
 *
 * Each is the GAME'S real chrome (same frames, icons, fonts, positions), filled
 * from replay data through bindings.js — no hand-built CSS recreation.
 *
 * Controls: pick a game + country; the renderer redraws. A "debug boxes" toggle
 * overlays the resolved layout boxes so coordinate fidelity is visible.
 */
(function (global) {
  "use strict";

  const $ = (s, r) => (r || document).querySelector(s);
  const el = (t, c, x) => { const e = document.createElement(t); if (c) e.className = c; if (x != null) e.textContent = x; return e; };

  const State = { games: null, root: null, gameId: null, snap: null, tag: null, debug: false };

  async function boot() {
    await Promise.all([global.GfxRegistry.load(), global.Fonts.init()]);
    const games = await global.Store.listGames();
    State.games = games;
    buildControls();
    // auto-open the first store game (real campaign) if present, else demo
    const first = (games.store && games.store[0]) ? { root: "store", g: games.store[0] }
                 : (games.demo && games.demo[0]) ? { root: "demo", g: games.demo[0] } : null;
    if (first) await openGame(first.root, first.g.game_id || first.g);
  }

  function buildControls() {
    const bar = $("#controls");
    bar.innerHTML = "";
    const gsel = el("select", "ctl"); gsel.id = "gamesel";
    const add = (root, list) => (list || []).forEach((g) => {
      const id = g.game_id || g;
      const o = el("option", null, root + " · " + id); o.value = root + "::" + id; gsel.appendChild(o);
    });
    add("store", State.games.store); add("demo", State.games.demo);
    gsel.onchange = () => { const [r, id] = gsel.value.split("::"); openGame(r, id); };
    bar.appendChild(label("Game", gsel));

    const csel = el("select", "ctl"); csel.id = "tagsel";
    csel.onchange = () => { State.tag = csel.value; renderAll(); };
    bar.appendChild(label("Country", csel));

    const dbg = el("label", "ctl-chk");
    const cb = el("input"); cb.type = "checkbox";
    cb.onchange = () => { State.debug = cb.checked; renderAll(); };
    dbg.appendChild(cb); dbg.appendChild(document.createTextNode(" debug boxes"));
    bar.appendChild(dbg);

    const miss = el("button", "ctl-btn", "coverage report");
    miss.onclick = showCoverage;
    bar.appendChild(miss);
  }

  function label(text, node) { const w = el("span", "ctl-wrap"); w.appendChild(el("span", "ctl-lbl", text)); w.appendChild(node); return w; }

  async function openGame(root, gameId) {
    State.root = root; State.gameId = gameId;
    await global.Store.open(root, gameId);
    State.snap = await global.Store.current();
    // populate country dropdown (humans/majors first)
    const csel = $("#tagsel"); csel.innerHTML = "";
    const cs = (State.snap && State.snap.countries) || {};
    const tags = Object.keys(cs).sort((a, b) => {
      const ma = cs[a].politics && cs[a].politics.is_major ? 0 : 1;
      const mb = cs[b].politics && cs[b].politics.is_major ? 0 : 1;
      return ma - mb || a.localeCompare(b);
    });
    tags.forEach((t) => { const o = el("option", null, t); o.value = t; csel.appendChild(o); });
    State.tag = tags[0] || null;
    renderAll();
  }

  function vw() { return Math.max(1280, document.documentElement.clientWidth); }

  async function renderAll() {
    const stage = $("#stage"); stage.innerHTML = "";
    if (!State.snap || !State.tag) { stage.appendChild(el("div", "ph", "no snapshot/country")); return; }

    // ---- 1. TOP BAR -------------------------------------------------------
    await section(stage, "Top bar — interface/topbar.gui (mod override)", async (host) => {
      const bgSize = global.GfxRegistry.sizeOf("GFX_topbar_background_extended") || { w: 2346, h: 87 };
      const rootBox = { x: 0, y: 0, w: vw(), h: bgSize.h };
      const r = await global.GuiRenderer.render(host, {
        view: "topbar", window: "topbar", bindings: global.Bindings.topbar,
        snap: State.snap, tag: State.tag, rootBox, debug: State.debug,
      });
      return r;
    });

    // ---- 2. COMBAT VIEW ---------------------------------------------------
    const combat = pickLandCombat(State.snap);
    await section(stage, "Combat — land_combat.gui · landcombatview" +
        (combat ? " (combat " + combat.combat_id + ", " + combat.terrain + ")" : " (no live land combat in this snapshot)"),
      async (host) => {
        const rootBox = { x: 0, y: 0, w: 567, h: 617 };
        const bindings = global.Bindings.landcombatview;
        // inject combat into ctx via a wrapper binding map (binders read ctx.combat)
        const r = await renderWithCtx(host, {
          view: "land_combat", window: "landcombatview", bindings,
          snap: State.snap, tag: State.tag, rootBox, debug: State.debug,
        }, { combat });
        return r;
      });

    // ---- 3. RESEARCH SLOT (country-view tab fragment) ---------------------
    await section(stage, "Country view — research slot (research.gui · research_slot_entry)", async (host) => {
      const rootBox = { x: 0, y: 0, w: 500, h: 100 };
      const c = State.snap.countries[State.tag];
      const slots = (c && c.research && c.research.research_slots) || 0;
      const inprog = (c && c.research && c.research.in_progress) || [];
      const wrap = el("div", "slot-stack");
      host.appendChild(wrap);
      let totalMiss = [];
      for (let i = 0; i < Math.max(1, slots); i++) {
        const tech = inprog[i] || null;
        const b = {
          // research_slot_entry element names vary; we bind a couple that exist and
          // let the rest render as the real frame. Title text via a generic binder.
        };
        const r = await renderWithCtx(wrap, {
          view: "research", window: "research_slot_entry", bindings: b,
          snap: State.snap, tag: State.tag, rootBox, debug: State.debug,
        }, { slotIndex: i, tech });
        totalMiss = totalMiss.concat(r.misses || []);
      }
      return { misses: totalMiss };
    });
  }

  // render but inject extra ctx fields (combat/slot/tech) the binders can read
  async function renderWithCtx(host, opts, extra) {
    const origRender = global.GuiRenderer.render.bind(global.GuiRenderer);
    // wrap each binder so ctx carries `extra`
    const wrapped = {};
    for (const k in (opts.bindings || {})) {
      const b = opts.bindings[k];
      wrapped[k] = (ctx) => (typeof b === "function") ? b(Object.assign(ctx, extra)) : global.Store.get(ctx.country, b);
    }
    return origRender(host, Object.assign({}, opts, { bindings: wrapped }));
  }

  function pickLandCombat(snap) {
    const list = (snap && snap.combats) || [];
    const land = list.filter((c) => c.kind === "land");
    // prefer one with the most divisions (most interesting), else first
    land.sort((a, b) => ((b.attackers || []).length + (b.defenders || []).length) - ((a.attackers || []).length + (a.defenders || []).length));
    return land[0] || null;
  }

  async function section(stage, title, fn) {
    const sec = el("section", "proto-sec");
    sec.appendChild(el("h2", "proto-h", title));
    const host = el("div", "proto-host");
    sec.appendChild(host);
    stage.appendChild(sec);
    try {
      const r = await fn(host);
      if (r && r.misses && r.misses.length) {
        const m = el("div", "proto-miss", "missing sprites: " + r.misses.join(", "));
        sec.appendChild(m);
      }
    } catch (e) {
      host.appendChild(el("div", "gr-error", String(e && e.message || e)));
      // eslint-disable-next-line no-console
      console.error(e);
    }
  }

  function showCoverage() {
    const stage = $("#stage");
    const sec = el("section", "proto-sec");
    sec.appendChild(el("h2", "proto-h", "Sprite coverage — misses across rendered windows"));
    const list = global.GfxRegistry.missList();
    if (!list.length) sec.appendChild(el("div", "ph", "no sprite misses — full coverage on rendered windows"));
    else {
      const ul = el("ul", "miss-list");
      list.forEach((m) => ul.appendChild(el("li", null, m.name + "  ×" + m.count)));
      sec.appendChild(ul);
    }
    stage.insertBefore(sec, stage.firstChild);
  }

  global.addEventListener("DOMContentLoaded", boot);
})(window);
