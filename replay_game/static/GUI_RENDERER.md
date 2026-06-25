# GUI-driven renderer — reuse the game's own UI

The replay UI is pivoting from hand-built CSS recreations to **rendering the game's
real windows**: read the engine's own `.gui` layout definitions and `.gfx` sprite
strips, and draw each window with the same frames, icons, fonts, and positions the
game uses — then fill the text fields from replay data. One renderer drives every
window; adding a window = naming its `.gui` window + writing a small binding table.

## Modules (all new, alongside the existing recreations — nothing was rewritten)

| File | Role |
|---|---|
| `gfx_registry.js` | `.gfx` sprite registry over `game_assets/sprites.json`. `resolve(name, frame)` -> `{url, width, height, no_of_frames, frame_width, nine_slice, progress_bar, ...}`. Frame strips were pre-sliced to per-frame PNGs by the pipeline; flags keyed `TAG`/`TAG_ideology`. Unresolved name -> `null` + logged miss (no fabrication). |
| `gui_parser.js` | Loads parsed `game_assets/gui/<view>.json` and **resolves the Clausewitz coordinate model** to absolute boxes. `layout(window, rootBox, sizeOf)` -> flat draw-ordered list of `{node, x, y, w, h, depth}`. Handles orientation anchors, nested-offset accumulation, `%%`/`%`/negative sizes, sprite-intrinsic sizing. |
| `fonts.js` | Maps the game font names -> CSS `{size, weight, stack}`. The game's UI fonts are **Ubuntu/Ubuntu Light** (16/18/20 bold) and **Roboto Mono** (typewriter; the bundled `assets/font.ttf`, copied to `static/fonts/roboto-mono.ttf`). Proprietary header faces (Orator Std, Garamond, Century Gothic) degrade to the closest free web stack at the correct px size. |
| `gui_renderer.js` | THE renderer. `render(host, {view, window, bindings, snap, tag, rootBox, scale, debug})` -> positioned DOM using real sprites + fonts + a binding map. |
| `bindings.js` | Per-window `elementName -> binder` tables (topbar, landcombatview, research). |
| `prototype.html` + `prototype.js` | The proof: renders 3 real windows end-to-end from a live replay snapshot. |

## How to view it

```bash
cd battle-plan/replay_game
python serve.py --port 8098 --no-open
# open http://127.0.0.1:8098/replay_game/static/prototype.html
```

Pick a game (the real `store · campaign` is the best demo) and a country. Toggle
**debug boxes** to overlay every resolved layout rectangle (hover = name + resolved
x,y,w,h + orientation). The **coverage report** button lists sprite misses.

## The Clausewitz coordinate model (validated, incl. Codex)

- A node's `position{x,y}` is an **offset from a corner/edge of the PARENT box**,
  chosen by `orientation`. The node's **own top-left** lands at that anchor + offset
  — orientation does NOT flip the node's pivot (an `UPPER_RIGHT` node at `x=-100`
  puts its left edge 100px in from the parent's right edge; it can overflow).
- `CENTER` = top-left at `parent_center + offset` (the GUI author offsets by
  `-w/2,-h/2` when true centering is wanted) — NOT auto-centered.
- Offsets **accumulate** down the tree.
- `size`: number = px; `"100%%"`/`"100%"` = fill parent on that axis; negative =
  `parent_extent + value`. A node with no size inherits its sprite's native pixels.

Verified live against the real topbar: `pol_power_icon` (UPPER_LEFT gui x=103) ->
x=99 (root `position.x=-4` propagated); `music_pause_button` (UPPER_RIGHT, nested in
a `x=-289` container) -> anchored near the right edge with accumulated offset;
`reopen_ingame_lobby` (LOWER_LEFT gui y=-22) -> y anchored to the bottom edge.

## Data binding (no fabrication)

`bindings = { "<.gui element name>": binder }`. A binder is either a **dotted path**
into the bound country object (`"politics.political_power"`) or a **function(ctx)**
returning `{value,captured}` | `{text}` | `{sprite,frame}` | `{flag:{tag,ideology,size}}`.
`ctx = {snap, tag, country, get(path)}`. A bound element whose value is missing
renders the game's empty slot (blank/"—" with a "not captured" title), never a fake.
Design-time numeric stubs in the `.gui` (`text = 999`) render blank when unbound.
Unbound decorative chrome (music/speed/menu buttons) renders as the real static art —
correct, the game shows it too.

## The 3 prototyped windows (proven end-to-end)

1. **Top bar** — `interface/topbar.gui` (the **mod override**), window `topbar`.
   81 sprites + 24 text fields, **0 sprite misses**. Real flag-in-shield, topbar
   background, PP / stability% / war-support% / manpower / factories / fuel / convoys
   / date, all bound from the snapshot in the game's Ubuntu font.
2. **Combat** — `land_combat.gui`, window `landcombatview`. 37 sprites + 16 texts,
   0 misses. Bound to one live `snap.combats` land battle: terrain picture, combat
   heading, leader frames, tactics, "N Divisions" from attacker/defender counts.
   commander->battle and country-db-id->TAG links aren't captured -> rendered blank.
3. **Country view — research slot** — `research.gui`, window `research_slot_entry`,
   cloned per research slot. Proves a country-view tab fragment renders from `.gui`.

## Sprite + font coverage

Swept 12 representative views (topbar, menubar, land/naval combat, research,
production, construction, diplomacy, decisions, national_focus, combat_log,
province_state):

- **Sprites: 534 distinct referenced, 506 resolve = 94.8%.**
- **Fonts: 18 distinct used, 18 mapped = 100%** (Ubuntu/Roboto-Mono real;
  Orator/Garamond/Century-Gothic = closest-free at correct size).

### What's missing (needs extraction)

The dominant gap is a **whole sprite class the extraction pipeline skips**:
`game_assets/extract.py` parses `spriteType` / `frameAnimatedSpriteType` /
`corneredTileSpriteType` but **not `textSpriteType`**. There are **~76 `textSpriteType`
defs** (the generic button frames in `interface/core.gfx`: `GFX_button_94x31`,
`GFX_button_221x34`, `button_type_6/7`, `GFX_list_button`, `GFX_sort_button_*`, …).
**Fix:** add `"textspritetype"` to `_SPRITE_KEYS` in `extract.py` and re-run
`--only sprites --merge` — that closes the bulk of the 28 misses in one pass.

A few truly-absent names (`GFX_button_war_overview`, `GFX_msg_combat`,
`GFX_messageicon_pause`, `GFX_mailbox_icon`) are in no scanned `.gfx` — likely
DLC/message sprites named dynamically; minor, non-blocking (placeholder renders).

## Which window archetypes the `.gui`->DOM approach handles vs needs special work

| Archetype | Approach | Notes |
|---|---|---|
| Top bar, country-view tabs, diplomacy/decisions panels, combat view | **DOM, clean** | Fixed layouts; the renderer handles them as-is. The bulk of the UI. |
| Lists / `gridboxType` (production queue, division lists, combat division rows) | DOM + a small **grid layout helper** | Clone the slot entry window per data row at `slotsize` steps (row-major for `UPPER_LEFT`). The parser preserves the grid nodes; a `renderGrid(entryWindow, items, slotsize)` wrapper is the next addition. |
| Focus tree | DOM for nodes + **SVG/Canvas overlay** for prerequisite lines | Goal icons place fine via gridbox; the bezier/prereq connectors need a vector layer drawn from `focus.prerequisite` + relative positions. |
| Unit / tank / ship designers | DOM but **bespoke** | Deeply nested data-cloned slot grids + drag affordances; render statically (read-only replay) but each needs its own binding pass. |

Caveats that affect any archetype: clipping/masks, scrollbars, `show_animation`,
and engine-side scripted visibility are not modelled (we render the resting state) —
fine for a static replay scrubber.

## Migration plan (recreations -> `.gui`-driven, incremental)

The new renderer lives **alongside** the existing `topbar.js`/`national.js`/etc.;
nothing was removed. Migrate one window at a time:

1. **Topbar** (done as prototype): swap `topbar.js`'s hand-built strip for a
   `GuiRenderer.render(view:"topbar", window:"topbar", bindings:Bindings.topbar)`
   call. Same data, real chrome. Lowest risk, highest visual win — do first.
2. **Country-view tabs** (`national.js`): each tab maps to a `countryXview.gui`
   window. Add a binding table per tab (politics/economy/military/research/focus);
   the snapshot fields are already the ones `national.js` reads, so binders are
   mostly the same dotted paths. Migrate tab-by-tab behind a feature flag.
3. **Combat** (prototype done): wire `landcombatview` into the panel system, binding
   `snap.combats`; add the division-row gridbox helper to list participants.
4. **Lists/panels** (`panels.js`, `windows2/3.js`): introduce `renderGrid` and move
   production/construction/diplomacy lists over; bind the existing array fields.
5. **Focus tree**: last — needs the SVG connector layer; reuse the gridbox node
   placement and draw lines from `focus` prerequisites.

Each step: keep the old module until the `.gui` version reaches parity behind a
toggle, then delete the recreation. Bindings are the only per-window work; the
renderer, parser, registry, and fonts are shared and done.

## Upstream pipeline notes (for `game_assets/extract.py`, not changed here)

To make the renderer pixel-complete, two small extractor enhancements help:
- add `textspritetype` to `_SPRITE_KEYS` (closes ~76 button-frame sprites);
- in `gui_node_to_json`, parse `slotsize`/`max_slots` `{x,y}`/`{w,h}` values and keep
  `max_slots_horizontal` / `add_horizontal` / gridbox `format` as first-class fields
  (currently `slotsize`/`max_slots` survive as empty child markers and `format` is
  mis-bucketed under `text.format`). The renderer compensates today, but first-class
  grid metadata makes the gridbox helper exact.
```
