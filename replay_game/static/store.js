/* store.js — client-side snapshot access + provenance + coverage.
 *
 * Reuses the SERVER's reading of replay.store (we never re-implement the store
 * reader): the server exposes /replay_game/api/{games,snapshots,snapshot,timeline}.
 * This module fetches snapshots lazily (one full snapshot is multi-MB) and parks
 * on the scrubber's current frame.
 *
 * HARD RULES baked in here so every view inherits them:
 *   - get(obj, "a.b.c") returns {value, captured}: captured=false means the leaf
 *     was null OR the path was absent (== "not captured"); a measured 0/false is
 *     captured=true. Views must render the distinction (Codex: "unknown stays
 *     unknown").
 *   - coverage(scope, requiredFields) computes real coverage so a map mode only
 *     lights up when it has genuine data (per-mode spec table, not "any non-null").
 */
(function (global) {
  "use strict";

  const Store = {
    game: null,            // {game_id, root}
    index: null,           // {snapshots:[{hour_index,date,file,...}]}
    timeline: null,        // {events:[...]}
    frames: [],            // index.snapshots (lightweight)
    cache: new Map(),      // file -> full snapshot
    cur: 0,                // current frame idx

    async listGames() {
      const r = await fetch("/replay_game/api/games");
      return r.ok ? r.json() : { store: [], demo: [] };
    },

    async open(root, gameId) {
      this.game = { root, game_id: gameId };
      const r = await fetch(`/replay_game/api/snapshots/${root}/${gameId}`);
      this.index = r.ok ? await r.json() : { snapshots: [] };
      this.frames = this.index.snapshots || [];
      this.cache.clear();
      this.cur = 0;
      // timeline is optional — ticker disappears cleanly if empty/absent.
      try {
        const t = await fetch(`/replay_game/api/timeline/${root}/${gameId}`);
        this.timeline = t.ok ? await t.json() : { events: [] };
      } catch (e) { this.timeline = { events: [] }; }
      return this.frames.length;
    },

    get singleSnapshot() { return this.frames.length <= 1; },
    get nFrames() { return this.frames.length; },
    frameMeta(i) { return this.frames[i] || null; },

    async snapshot(i) {
      const fr = this.frames[i];
      if (!fr) return null;
      if (this.cache.has(fr.file)) return this.cache.get(fr.file);
      const r = await fetch(
        `/replay_game/api/snapshot/${this.game.root}/${this.game.game_id}/${fr.file}`);
      const snap = r.ok ? await r.json() : null;
      if (snap) this.cache.set(fr.file, snap);
      return snap;
    },

    async current() { return this.snapshot(this.cur); },

    // ---- generic field access with provenance ----------------------------
    /** Dig a dotted path. Returns {value, captured}. captured=false iff null/absent. */
    get(obj, dotted) {
      let cur = obj;
      const parts = dotted.split(".");
      for (let k = 0; k < parts.length; k++) {
        if (cur == null || typeof cur !== "object") return { value: null, captured: false };
        cur = cur[parts[k]];
      }
      return { value: cur, captured: cur !== null && cur !== undefined };
    },

    /** Which provenance channel(s) backed a snapshot domain (for badges). */
    domainsOf(snap) {
      const p = (snap && snap.provenance) || {};
      const d = new Set();
      for (const ch of ["save", "memory"]) {
        const c = p[ch];
        if (c && Array.isArray(c.domains)) c.domains.forEach((x) => d.add(x));
      }
      return d;
    },

    // ---- coverage for map-mode auto-enable (Codex D) ----------------------
    /**
     * Fraction of entities in `scope` ("states"|"provinces"|"countries") that
     * have ALL of `required` dotted fields captured (non-null), in snapshot.
     * Returns {coverage, n, captured}.
     */
    coverage(snap, scope, required) {
      const cont = (snap && snap[scope]) || {};
      const keys = Array.isArray(cont) ? cont.map((_, i) => i) : Object.keys(cont);
      const n = keys.length;
      if (!n) return { coverage: 0, n: 0, captured: 0 };
      let captured = 0;
      for (const k of keys) {
        const e = Array.isArray(cont) ? cont[k] : cont[k];
        let all = true;
        for (const f of required) {
          if (!this.get(e, f).captured) { all = false; break; }
        }
        if (all) captured++;
      }
      return { coverage: captured / n, n, captured };
    },

    /**
     * Evaluate a map-mode spec against ALL loaded (cached) snapshots OR a given
     * snapshot. spec = {scope, required, minCoverage, minEntities, containerOnly}.
     * containerOnly: a sparse list/event mode is "available" if the container
     * exists at all (presence counts even if empty). Returns true if available.
     */
    modeAvailable(spec, snap) {
      if (!snap) return false;
      if (spec.containerOnly) {
        const c = snap[spec.scope];
        return c != null;
      }
      const cov = this.coverage(snap, spec.scope, spec.required || []);
      return cov.captured >= (spec.minEntities || 1) &&
             cov.coverage >= (spec.minCoverage || 0);
    },
  };

  global.Store = Store;
})(window);
