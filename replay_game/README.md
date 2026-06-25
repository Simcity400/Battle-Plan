# replay_game — Layer 2: the game-authentic Battle Plan UI

A faithful, **offline** recreation of the Hearts of Iron IV interface, rendered
with the game's **real assets** (`game_assets/`) and driven by **replay snapshots**
(`replay/SCHEMA.md`). You scrub/play across a game's snapshot series and watch the
world evolve in something that looks like the game. Pairs with **Layer 1**
(`replay_ui/`, the stats/graphs view) under one shell with a mode toggle.

Spec: **`PLAN.md`** (window→data mapping, map modes §B.1, sprite convention §I).

## Run it

```
Open Game Replay.bat            (Windows; finds Python, launches the server)
  — or —
python replay_game/serve.py     (from the repo root)
```

Then open `http://127.0.0.1:8090/`. Loopback only, no network, stdlib only.

The art PNGs (`game_assets/assets/`) are gitignored and must exist on disk; if
they're missing, run `python game_assets/extract.py` from the repo root (the UI
still loads — sprites just fall back to labelled placeholders, never blank).

Optional: `python replay_game/build_centroids.py` precomputes province/state
centroids into `output/centroids.json` (a cache for combat-bubble / division-counter
placement). It is **optional** — if absent, the client computes the same centroids
at boot from the province raster it already decodes (no Pillow, no build step).

## What's built (PLAN phases 1–3 + the shell)

- **Sprite loader** (`static/sprites.js`) — `Sprites.icon(name)` / `flag(tag, ideology)`
  / `progressBar` / `nineSlice` / `focusIcon` over `sprites.json`. Handles multi-frame
  strips (`_fN`), flags keyed `TAG`/`TAG_ideology` with the `res` map, nine-slice chrome
  (`border_size` → CSS border-image), and two-layer sprites (masked_shield, progress_bar).
  A missing sprite → a labelled placeholder + a logged miss; **never blank, never faked**.
- **Topbar** (`static/topbar.js`) — flag + PP / stability / war-support / CP / manpower /
  civ-mil-dock factories / convoys / fuel(+ratio bar) / resources strip, using the real
  topbar sprite names from `game_assets/gui/topbar.json`.
- **Map** (`static/map.js` + `static/map_gl.js` + `static/modes.js`) — raster recolor
  reusing the documented `replay_render` pipeline (decode `province_ids_half.png` once →
  `province_to_state` → owner → `country_colors`). Map modes: **Political,
  Controller/occupation, Faction, Diplomatic, Victory points, Resources, Manpower,
  Industry/buildings, Front lines** (+ the being-cracked modes). Combat bubbles
  (attacker-vs-defender flags, click → combat view) and division counters (flag + count,
  sized by Σstrength) sit in an overlay layer scaled to the display.

  **Recolor backend — WebGL with a Canvas2D fallback** (`static/map_gl.js`). The recolor
  is the per-pixel step that runs on every mode switch / scrub / playback frame. By default
  it runs on the **GPU**: the province id PNG is uploaded as a texture (the PNG already
  encodes `province_id = R + G·256 + B·65536`, reconstructed in the fragment shader with
  NEAREST sampling and no colorspace/premultiply mangling), a **static** prov→state lookup
  texture is built once, and only a **small per-frame color LUT** (≤ ~13.4k texels:
  state→RGB for per-state modes, province→RGB for per-province modes) is re-uploaded each
  recolor. SEA (`id 0` / `state < 0`) and UNKNOWN (key absent from the LUT) are reproduced
  byte-for-byte against the CPU contract. The renderer is **WebGL-only**: if WebGL is
  genuinely unavailable, or the context / shader-compile / real-fragment-`highp` probe fails,
  `map_gl.create()` returns `null` and the map shows a one-line **"requires WebGL"** message —
  there is **no Canvas2D fallback renderer** (deliberate: this tool runs on machines that run
  HOI4, and WebGL has been universal in browsers for ~a decade, so a second CPU renderer was
  dead weight). Province picking/hover/tooltips and combat/division centroids read the CPU
  `idRaster` independently of how the map is painted.

  *Which backend is active* is logged to the console at boot: `[map] recolor backend: WebGL
  (webgl2|webgl1)`. **To visually test:** open the replay, watch the political map paint, then
  click through every mode button (per-state: Political/Faction/Diplomatic/VP/Resources/
  Manpower/Industry; per-province: Controller/Front lines) and scrub/play — colors, sea, and
  the grey UNKNOWN must look correct. (Before the CPU path was removed, the WebGL recolor was
  validated **byte-for-byte** against it — max channel delta 0, 0 mismatches across all modes.)
- **National view** (`static/national.js` + `static/gamewindows.js`) — tabbed: Politics,
  National Focus (goal-icon tree), Research (queue + completed), Production (construction
  + equipment), Logistics, Military, **Commanders**, **Designers**, **Order of Battle**,
  Diplomacy, Decisions, **Intel** (the bold four are the advanced windows — see below).
- **Detail drawers** (`static/panels.js`) — State/province (§D), Division (§E), Combat (§G,
  with raw progress + a bipolar bar only when safe — see below).
- **Event ticker** (`static/ticker.js`) — `timeline.json` with source + confidence badges;
  click an event to jump the scrubber and focus the country.
- **Shell / transport / layer toggle** (`static/app.js`) — game/country pickers, outliner,
  the scrub/play transport, the map-mode bar with honest auto-enable, and the
  Layer-1 ⇄ Layer-2 switch (Layer 1 is iframed; both read the same store).

## The two hard rules, enforced

- **No fabrication.** Every leaf goes through `Store.get → {value, captured}`: a measured
  `0`/`false` renders as data; a `null`/absent field renders **"not captured"** (muted).
  Map modes paint a neutral UNKNOWN where a state/province has no value for the active
  mode. Combat participant flags resolve `div_id → divisions[] → owner` **in the current
  snapshot only**; unresolved ids render `? ×N`, never an inferred owner. Combat `progress`
  is shown raw; the −1..1 bar is drawn **only** if every observed value across the loaded
  snapshots lies within −1..1 (else: raw value + "bar disabled" note).
- **Graceful degradation.** A single-snapshot replay disables the scrubber/play and shows
  an honest empty-ticker note; a multi-snapshot replay enables them.

## Map-mode auto-enable (deferred / being-cracked domains light up with no rework)

Modes whose data isn't captured yet — **Supply, Railway, Intel, Naval, Air superiority,
Weather** — render as **disabled** toggles ("not captured" / "capturing…"). A mode is
available iff its colorizer produces a non-empty paint for the current snapshot (and any
declared coverage spec passes). The instant the store carries that domain (e.g. a
memory-RE layer lands `provinces[*].supply` or a sea-zone control map), the colorizer
yields output and the mode lights up — **no code change required**. New domains bind the
same way: add a colorizer, the transport/scrub is shared.

## Boundaries

Self-contained under `replay_game/`. Reads `game_assets/`, `replay/`, `replay_ui/`,
`map_pipeline/`, `replay_render/` read-only. Does **not** touch or reference `dashboard/`,
`memory_reader/`, `save_oracle/`, `mod/`. No CDN, no network, no Chart.js in Layer 2
(the iframed Layer 1 brings its own). Generated/large output (`output/`, caches) is
gitignored.

## Advanced windows (PLAN phases 4–5) — built (`static/gamewindows.js`)

Four game-authentic National-view tabs, bound through the **same** generic store path
(so they render "not captured" on an older snapshot and **light up automatically** when
a newer snapshot carries the domain — no rework):

- **Commanders** (`countries[*].commanders`, §3.9) — country leader + party leaders,
  the general / field-marshal / admiral roster (skill stars, the four army stat lines
  or navy maneuvering/coordination, traits, assignment), and advisors/cabinet grouped
  by slot. Portraits resolve a `portrait_gfx` **sprite name** via `UI.portrait` → real
  art if the asset exists, else a labelled placeholder (a portrait-asset pass is
  separate — never fabricated, never blocks).
- **Designers** (`designs` §11 + `countries[*].mios` §3.10) — the division designer
  (regiment grid laid out by `{x,y}`), the equipment/ship/plane variant catalog
  (modules by slot + upgrades, grouped by `family`), and the MIO panel (+ scheduled
  presets, MIO policies). Template **combat stats** are deliberately **not** shown —
  the save stores only the grid (SCHEMA §11 hard limit), surfaced as a note.
- **Order of battle** (`order_of_battle` §12) — the army-group → army → division tree,
  fleets → task forces → ships, and air wings. Commander names resolve via the
  `commanders` `unit_leader_id` index (owner country first, then a unique global match;
  unresolved → raw id + "name not captured"/"ambiguous", never a guessed name). Member
  divisions and wing plane definitions cross-ref `divisions[]` / `designs`. `cached.*`
  plan/power values are labelled **runtime, non-authoritative**.
- **Intelligence / operations** (`countries[*].intel`, §3.8 deepened) — the agency
  (capacity/upgrades), the operative roster (status/missions/traits/nationality),
  running + finished operations (progress **derived** from snapshot date − op start over
  duration, clamped, labelled `D` — only when all inputs are captured; never wall-clock),
  and cryptology/decryption per target.

`null` is treated as **meaningful** where the schema uses it so (`assignment: null` =
reserve, per-country `order_of_battle: null` = captured-empty): these go through
`UI.presence` (key-on-object) + provenance, not `Store.get` alone, so "captured but
none" is never confused with "not captured".
