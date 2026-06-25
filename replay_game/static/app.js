/* app.js — the orchestrator. Wires:
 *   - the layer toggle (Layer 1 stats via iframe  <->  Layer 2 this game UI)
 *   - the scrub/play transport (shared hour_index, persisted across modes)
 *   - the game + country pickers and the outliner
 *   - the map-mode bar with honest auto-enable (Codex D specs)
 *   - topbar / map / national / panels / ticker
 *
 * No build step, no CDN, no Chart.js in Layer 2 (Layer 1's iframe brings its own).
 */
(function () {
  "use strict";
  const E = UI.el;
  const $ = (id) => document.getElementById(id);

  const APP = {
    layer: "game",          // "game" | "stats"
    playing: false, timer: null, lastTick: 0,
    selCountry: null,
    games: { store: [], demo: [] },
    cur: null,              // {root, game_id}
  };

  // ---------- boot ----------
  async function boot() {
    setStatus("loading sprite manifest…");
    try { await Sprites.load(); } catch (e) { setStatus("sprites.json failed: " + e.message); }
    APP.games = await Store.listGames();

    // build chrome
    buildShell();
    Topbar.init($("topbarHost"));
    await GameMap.init($("mapHost"));
    GameMap.onProvinceClick = onProvinceClick;
    GameMap.onCombatClick = (cb) => Panels.showCombat(GameMap.snap, cb, [...Store.cache.values()]);
    GameMap.onUnplaced = renderUnplaced;
    National.init($("natHost"));
    Panels.Drawer.init(document.body);
    Ticker.init($("tickerHost"));
    Ticker.onJump = (hour, tag) => { jumpToHour(hour); if (tag) selectCountry(tag); };

    buildModeBar();
    populateGames();
    await openSelectedGame();
    setStatus("");
  }

  // ---------- shell ----------
  function buildShell() {
    const app = $("app");
    app.innerHTML = "";

    // header: title + layer toggle + game/country pickers
    const header = E("div", "g-header");
    header.appendChild(E("span", "g-title", "Battle Plan"));
    const layerWrap = E("div", "layer-toggle");
    const b1 = E("button", "lt-btn active", "Game replay"); b1.dataset.layer = "game";
    const b2 = E("button", "lt-btn", "Stats (Layer 1)"); b2.dataset.layer = "stats";
    for (const b of [b1, b2]) b.addEventListener("click", () => switchLayer(b.dataset.layer));
    layerWrap.appendChild(b1); layerWrap.appendChild(b2);
    header.appendChild(layerWrap);

    const pickers = E("div", "g-pickers");
    APP.gameSel = E("select", "g-sel");
    APP.gameSel.addEventListener("change", openSelectedGame);
    const gl = E("label", "g-lbl", "Game "); gl.appendChild(APP.gameSel);
    pickers.appendChild(gl);
    APP.countrySel = E("select", "g-sel");
    APP.countrySel.addEventListener("change", () => selectCountry(APP.countrySel.value));
    const cl = E("label", "g-lbl", "Country "); cl.appendChild(APP.countrySel);
    pickers.appendChild(cl);
    APP.gameMeta = E("span", "g-meta");
    pickers.appendChild(APP.gameMeta);
    header.appendChild(pickers);
    app.appendChild(header);

    // topbar host
    app.appendChild(elId("topbarHost", "topbar-host"));

    // transport (scrubber)
    const transport = E("div", "transport");
    APP.playBtn = E("button", "tr-play", "▶");
    APP.playBtn.addEventListener("click", togglePlay);
    APP.scrub = E("input", "tr-scrub"); APP.scrub.type = "range"; APP.scrub.min = 0; APP.scrub.max = 0; APP.scrub.value = 0;
    APP.scrub.addEventListener("input", (e) => { stop(); gotoFrame(+e.target.value); });
    APP.frameDate = E("span", "tr-date", "—");
    APP.frameProv = E("span", "tr-prov", "");
    transport.appendChild(APP.playBtn); transport.appendChild(APP.scrub);
    transport.appendChild(APP.frameDate); transport.appendChild(APP.frameProv);
    app.appendChild(transport);

    // main: map+modebar (game layer) and the stats iframe (stats layer)
    const main = E("div", "g-main");
    APP.gameLayer = E("div", "layer game-layer active");
    APP.modeBar = E("div", "mode-bar");
    APP.gameLayer.appendChild(APP.modeBar);
    const mapRow = E("div", "map-row");
    mapRow.appendChild(elId("mapHost", "map-host"));
    APP.outliner = elId("outliner", "outliner");
    mapRow.appendChild(APP.outliner);
    APP.gameLayer.appendChild(mapRow);
    APP.unplaced = elId("unplaced", "unplaced");
    APP.gameLayer.appendChild(APP.unplaced);
    APP.gameLayer.appendChild(elId("natHost", "nat-host"));
    APP.gameLayer.appendChild(elId("tickerHost", "ticker-host"));
    main.appendChild(APP.gameLayer);

    APP.statsLayer = E("div", "layer stats-layer");
    APP.statsFrame = E("iframe", "stats-frame");
    APP.statsFrame.title = "Layer 1 — stats";
    APP.statsLayer.appendChild(APP.statsFrame);
    main.appendChild(APP.statsLayer);
    app.appendChild(main);

    APP.status = E("div", "g-status");
    app.appendChild(APP.status);
  }
  function elId(id, cls) { const e = E("div", cls); e.id = id; return e; }
  function setStatus(t) { if (APP.status) APP.status.textContent = t || ""; APP.status && APP.status.classList.toggle("hidden", !t); }

  // ---------- layer toggle ----------
  function switchLayer(layer) {
    APP.layer = layer;
    document.querySelectorAll(".lt-btn").forEach((b) => b.classList.toggle("active", b.dataset.layer === layer));
    APP.gameLayer.classList.toggle("active", layer === "game");
    APP.statsLayer.classList.toggle("active", layer === "stats");
    if (layer === "stats") loadStatsIframe();
  }
  function loadStatsIframe() {
    // share the scrubber position via the URL hash; Layer 1 reads its own store.
    const want = "/replay_ui/app.html";
    if (!APP.statsFrame.src || APP.statsFrame.src.indexOf("/replay_ui/") < 0) APP.statsFrame.src = want;
  }

  // ---------- games / country ----------
  function populateGames() {
    const sel = UI.clear(APP.gameSel);
    let first = null;
    for (const root of ["store", "demo"]) {
      for (const g of (APP.games[root] || [])) {
        const o = E("option", null, `${g.game_id} (${g.n_snapshots} snap${g.n_snapshots === 1 ? "" : "s"})`);
        o.value = root + "/" + g.game_id;
        sel.appendChild(o);
        if (!first) first = o.value;
      }
    }
    if (first) sel.value = first;
    if (!first) { sel.appendChild(E("option", null, "no games found")); }
  }

  async function openSelectedGame() {
    const v = APP.gameSel.value;
    if (!v || v === "no games found") { setStatus("No games in the store."); return; }
    const [root, gameId] = v.split("/");
    APP.cur = { root, game_id: gameId };
    setStatus("loading game…");
    Panels.resetProgressSafety();
    const n = await Store.open(root, gameId);
    APP.gameMeta.textContent = n + " snapshot" + (n === 1 ? "" : "s")
      + (Store.singleSnapshot ? " · scrubber disabled" : "");
    APP.scrub.max = Math.max(0, n - 1);
    APP.scrub.value = 0;
    APP.scrub.disabled = Store.singleSnapshot;
    APP.playBtn.disabled = Store.singleSnapshot;
    Ticker.setEvents((Store.timeline || {}).events || []);
    await gotoFrame(0, true);
    setStatus("");
  }

  function populateCountries(snap) {
    const sel = UI.clear(APP.countrySel);
    const tags = Object.keys(snap.countries || {});
    // humans + majors first
    const rank = (t) => { const p = (snap.countries[t].politics || {}); return (p.is_human ? 0 : 2) + (p.is_major ? 0 : 1); };
    tags.sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
    for (const t of tags) {
      const p = snap.countries[t].politics || {};
      const tag = p.is_human ? "★ " : (p.is_major ? "● " : "  ");
      const o = E("option", null, tag + t); o.value = t;
      sel.appendChild(o);
    }
    if (!APP.selCountry || !snap.countries[APP.selCountry]) {
      // prefer the country whose save backed this snapshot (e.g. GER_1941_melted.txt
      // -> GER), else the first human, else the first tag. Honest, deterministic.
      const fn = ((snap.provenance || {}).save || {}).source_filename || "";
      const m = fn.match(/^([A-Z]{3})[_.]/);
      const fromSave = m && snap.countries[m[1]] ? m[1] : null;
      APP.selCountry = fromSave
        || tags.find((t) => (snap.countries[t].politics || {}).is_human)
        || tags[0] || null;
    }
    sel.value = APP.selCountry || "";
  }

  function selectCountry(tag) {
    if (!tag) return;
    APP.selCountry = tag;
    APP.countrySel.value = tag;
    GameMap.setSelectedCountry(tag);
    refreshCurrent();
  }

  // ---------- transport ----------
  async function gotoFrame(i, full) {
    Store.cur = Math.max(0, Math.min(Store.nFrames - 1, i));
    APP.scrub.value = Store.cur;
    const snap = await Store.current();
    if (!snap) return;
    if (full) populateCountries(snap);
    GameMap.setSnapshot(snap);
    GameMap.setSelectedCountry(APP.selCountry); // so the diplomatic mode can paint
    Topbar.render(snap, APP.selCountry);
    National.show(APP.selCountry, snap);
    GameMap.render();
    refreshModeBar(snap);
    renderOutliner(snap);
    const meta = Store.frameMeta(Store.cur);
    APP.frameDate.textContent = (meta && meta.date) || snap.date || "—";
    APP.frameProv.textContent = provenanceLabel(snap);
    Ticker.setFrameHour(snap.hour_index);
  }

  async function refreshCurrent() {
    const snap = await Store.current();
    if (!snap) return;
    Topbar.render(snap, APP.selCountry);
    National.show(APP.selCountry, snap);
    GameMap.render();
    refreshModeBar(snap); // diplomatic-mode availability depends on the selected country
    renderOutliner(snap);
  }

  function jumpToHour(hour) {
    if (hour == null) return;
    let best = 0, bestD = Infinity;
    for (let i = 0; i < Store.nFrames; i++) {
      const m = Store.frameMeta(i);
      const d = Math.abs((m.hour_index || 0) - hour);
      if (d < bestD) { bestD = d; best = i; }
    }
    stop(); gotoFrame(best);
  }

  function provenanceLabel(snap) {
    const p = snap.provenance || {};
    const bits = [];
    if (p.save && p.save.captured) bits.push("S: " + (p.save.source_filename || "save"));
    if (p.memory && p.memory.captured) bits.push("M: live");
    return bits.join(" · ");
  }

  function togglePlay() { APP.playing ? stop() : play(); }
  function play() {
    if (Store.singleSnapshot) return;
    if (Store.cur >= Store.nFrames - 1) Store.cur = 0;
    APP.playing = true; APP.playBtn.textContent = "⏸"; APP.lastTick = performance.now();
    const FRAME_MS = 800;
    const loop = (t) => {
      if (!APP.playing) return;
      if (t - APP.lastTick >= FRAME_MS) {
        APP.lastTick = t;
        if (Store.cur >= Store.nFrames - 1) { stop(); return; }
        gotoFrame(Store.cur + 1);
      }
      APP.timer = requestAnimationFrame(loop);
    };
    APP.timer = requestAnimationFrame(loop);
  }
  function stop() { APP.playing = false; APP.playBtn.textContent = "▶"; if (APP.timer) cancelAnimationFrame(APP.timer); APP.timer = null; }

  // ---------- map mode bar ----------
  function buildModeBar() {
    const bar = UI.clear(APP.modeBar);
    APP.modeBtns = {};
    for (const m of MapModes.list) {
      const b = E("button", "mode-btn group-" + m.group + (m.report ? " mode-report" : ""), m.label);
      b.dataset.mode = m.id;
      b.addEventListener("click", () => {
        if (b.disabled) return;
        if (m.report) {
          // Non-map REPORT mode (naval/air region proxy): no recolor — open the
          // region report drawer. The painted map keeps whatever color mode was
          // last active (we do NOT switch GameMap.mode), so the report is purely
          // additive and never fabricates map geometry it doesn't have (Codex 2).
          // Surface the PROXY/ESTIMATE legend in the bar too so the proxy status
          // is visible at-a-glance, not only inside the drawer (Codex JS review).
          renderModeLegend(m);
          Panels.showRegionReport(GameMap.snap, m);
          return;
        }
        GameMap.setMode(m.id);
        for (const k in APP.modeBtns) {
          if (MapModes.byId[k] && MapModes.byId[k].report) continue; // report btns aren't "active" map modes
          APP.modeBtns[k].classList.toggle("active", k === m.id);
        }
        GameMap.render();
        renderModeLegend(m);
      });
      APP.modeBtns[m.id] = b;
      bar.appendChild(b);
    }
    APP.modeBtns.political.classList.add("active");
    // div/combat overlay toggles
    const sep = E("span", "mode-sep", "");
    bar.appendChild(sep);
    APP.divToggle = toggle("Divisions", true, (on) => GameMap.setShow({ divs: on }));
    APP.cbToggle = toggle("Combat bubbles", true, (on) => GameMap.setShow({ combat: on }));
    bar.appendChild(APP.divToggle.label); bar.appendChild(APP.cbToggle.label);
    // legend strip for proxy/estimate badging of the active mode — placed directly
    // after the mode bar (APP.modeBar is already in APP.gameLayer via buildShell).
    APP.modeLegend = E("div", "mode-legend hidden");
    APP.modeBar.insertAdjacentElement("afterend", APP.modeLegend);
    renderModeLegend(MapModes.byId.political);
  }

  /**
   * Render the active map mode's legend chip. HARD REQUIREMENT: a mode whose data
   * is a proxy/estimate (m.proxy) MUST be visibly badged so region/proxy data is
   * never shown as authoritative. The confidence label travels with the data
   * (e.g. supply.confidence = "division_province_proxy").
   */
  function renderModeLegend(m) {
    const leg = APP.modeLegend;
    if (!leg) return;
    UI.clear(leg);
    if (!m || !m.proxy) { leg.classList.add("hidden"); return; }
    leg.classList.remove("hidden");
    const chip = E("span", "legend-proxy-chip", "PROXY / ESTIMATE");
    leg.appendChild(chip);
    // pull the confidence label that travels WITH the data, if present
    let conf = null;
    if (m.proxy.confidenceFrom && GameMap.snap) {
      conf = Store.get(GameMap.snap, m.proxy.confidenceFrom).value || null;
    }
    if (conf) leg.appendChild(E("span", "legend-conf", conf));
    leg.appendChild(E("span", "legend-note", m.proxy.note || "estimated / non-authoritative data"));
  }
  function toggle(label, on, cb) {
    const lbl = E("label", "mode-chk");
    const inp = E("input"); inp.type = "checkbox"; inp.checked = on;
    inp.addEventListener("change", () => cb(inp.checked));
    lbl.appendChild(inp); lbl.appendChild(document.createTextNode(" " + label));
    return { label: lbl, input: inp };
  }

  function refreshModeBar(snap) {
    for (const m of MapModes.list) {
      const b = APP.modeBtns[m.id];
      let avail;
      if (m.report) {
        // Non-map REPORT mode: it never paints; availability is its own data
        // predicate (e.g. naval/air region data actually present this snapshot).
        try { avail = !!(m.available && m.available(snap)); } catch (e) { avail = false; }
      } else {
        // The single honest predicate: a mode is available iff its colorizer can
        // actually paint THIS snapshot (non-empty output) AND (if it declares a
        // stricter coverage spec) that spec passes. This means:
        //   - "full" base modes (political/faction/diplomatic) are always-on (they
        //     paint from owner data that's always present);
        //   - data-gated modes (vp/resources/manpower/buildings/controller/supply/
        //     rail/occupation/lend-lease) enable only with genuine coverage;
        //   - "cracking" modes (intel/weather) stay disabled until their memory-RE
        //     layer lands a real colorizer output — never faked.
        let paints = false;
        try { paints = Object.keys(m.colorize(snap, GameMap) || {}).length > 0; }
        catch (e) { paints = false; }
        const specOk = m.spec ? Store.modeAvailable(m.spec, snap) : true;
        // base modes that always paint (no spec) are available; others need data.
        avail = (!m.spec && paints) || (m.spec && specOk && paints) ||
                (!m.spec && m.group === "full" && paints);
      }
      b.disabled = !avail;
      if (avail) {
        b.title = m.proxy
          ? m.label + " — PROXY/ESTIMATE: " + (m.proxy.note || "non-authoritative")
          : m.label;
        b.classList.remove("capturing");
      } else {
        b.title = m.group === "cracking"
          ? m.label + " — capturing… (memory-RE in progress; lights up automatically when its data lands)"
          : m.label + " — not captured this snapshot";
        b.classList.toggle("capturing", m.group === "cracking");
      }
    }
    // if the active map mode just went unavailable, fall back to political (report
    // modes are never the "active" map mode, so they don't trigger this).
    const active = GameMap.mode;
    if (APP.modeBtns[active] && APP.modeBtns[active].disabled &&
        !(MapModes.byId[active] && MapModes.byId[active].report)) {
      GameMap.setMode("political");
      for (const k in APP.modeBtns) {
        if (MapModes.byId[k] && MapModes.byId[k].report) continue;
        APP.modeBtns[k].classList.toggle("active", k === "political");
      }
      renderModeLegend(MapModes.byId.political);
    } else {
      renderModeLegend(MapModes.byId[active]);
    }
  }

  // ---------- outliner ----------
  function renderOutliner(snap) {
    const o = UI.clear(APP.outliner);
    o.appendChild(E("div", "outliner-head", "Outliner"));
    const tags = Object.keys(snap.countries || {})
      .filter((t) => { const p = snap.countries[t].politics || {}; return p.is_human || p.is_major; });
    const rank = (t) => { const p = snap.countries[t].politics || {}; return (p.is_human ? 0 : 1); };
    tags.sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
    for (const t of tags) {
      const c = snap.countries[t]; const pol = c.politics || {}, mil = c.military || {}, eco = c.economy || {};
      const row = E("div", "out-row" + (t === APP.selCountry ? " sel" : ""));
      row.appendChild(Sprites.flag(t, pol.ruling_party, "small"));
      const id = E("span", "out-tag", t + (pol.is_human ? " ★" : "")); row.appendChild(id);
      const stats = E("span", "out-stats");
      const f = eco.factories || {};
      stats.textContent = `${mil.division_count != null ? mil.division_count : "—"}div · ${(f.civ ?? "—")}/${(f.mil ?? "—")}/${(f.dockyard ?? "—")}`;
      row.appendChild(stats);
      row.addEventListener("click", () => selectCountry(t));
      o.appendChild(row);
    }
  }

  function renderUnplaced(list) {
    const u = UI.clear(APP.unplaced);
    if (!list || !list.length) { u.classList.add("hidden"); return; }
    u.classList.remove("hidden");
    const combats = list.filter((x) => x.kind === "combat").length;
    const divs = list.filter((x) => x.kind === "div").length;
    u.textContent = `Unplaced overlays (no visible map pixel for that province): ${combats} combat(s), ${divs} division stack(s) — listed in panels, not faked on the map.`;
  }

  // ---------- province click ----------
  function onProvinceClick(provId, stateId) {
    if (stateId != null) Panels.showState(GameMap.snap, stateId);
  }

  window.addEventListener("DOMContentLoaded", boot);
})();
