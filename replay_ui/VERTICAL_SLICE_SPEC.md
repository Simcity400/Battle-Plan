# VERTICAL SLICE SPEC — native Rust/wgpu replay UI (topbar + map + state panel)

> **Purpose.** The first build step of the native replica (`GAME_UI_REPLICA_PLAN.md` v3). A
> deliberately *thin* end-to-end slice that drives **every layer** of the stack once — asset
> load, scene-graph, layout, sprite/9-slice draw, BMFont text, GPU map recolour, hit-testing,
> data binding, and the time scrubber — through **three real surfaces**, so the runtime is
> proven before the ~2,000-binding sweep. It is the v3 realisation of plan Phase 0 (topbar) +
> a sliver of Phase 1 (map + state panel + scrubber).
>
> **Not in this slice:** any imperative adapter (no focus/tech tree, no combat roster, no
> outliner), tooltips beyond one trivial hover, any map mode except Political, any view beyond
> topbar + state panel, the trigger evaluator, and most bindings. Those are later phases. The
> slice exists to de-risk the **engine**, not to cover surfaces.

> **Codex adversarial review (2026-06-26, folded in below).** Verdict: the native/`wgpu` choice
> is *defensible* and rendering is **not** the risk — **behavioural / view-model fidelity is**
> (text markup, clipping/scrolling, hidden aggregations). Three consequences are now baked in:
> (a) the slice MUST also exercise a real **scroll viewport + scissor/clip**, a **wrapped,
> marked-up localized text box**, and an explicit **z-order / `alwaystransparent` hit-test** case —
> these are runtime foundations, not later polish; (b) stand up a **golden screenshot + hit-test
> regression harness from day one** (the single biggest underweighted risk — a fast renderer that
> is subtly wrong everywhere); (c) the Python→Rust boundary needs a **frozen, versioned,
> fixture-tested schema** — "shared snapshot crate" is only literally true once the reader is Rust
> too. Details in §3.1 / §5 / §7.

---

## 1. Why these three surfaces

| Surface | What it proves (exercised exactly once, minimally) |
|---|---|
| **Topbar** (`game_assets/gui/topbar.json`) | `.gui`-JSON → scene-graph; the layout model (`position`/`size`/`orientation`/`origo`/percent); `iconType` sprite draw from `.dds`; `corneredTileSpriteType` 9-slice chrome; `instantTextBoxType` BMFont glyph-quad text + loc + `$VAR$`; **scalar binding** (the thin pass-through path); `progressbartype` (stability/WS bars). |
| **Map** (`map_pipeline/assets/*`) | GPU **fragment-shader recolour** of the province-id raster via a per-province colour LUT; pan/zoom; **province picking** (id-texture sample at cursor → province → state); the map as the bottom compositor layer with the topbar stacked above. |
| **State panel** (`game_assets/gui/province_state.json`) | a **second** window opened by a read-only verb (pick → select → open); hit-testing + window stacking + `alwaystransparent` pass-through; one **minimal gridbox row-factory** (the buildings list — the first special-widget); a second view's scalar bindings. |
| **Scrubber** (global) | the spine: one `hour_index` drives **in-process** re-bind of all three surfaces; play/pause auto-advance; map recolours, topbar values change, panel updates — no server, no fetch. |

Together these touch all 13 passive element kinds' core path + 2 special widgets (gridbox
row-factory, the map drag canvas), both render primitives that recur everywhere (9-slice,
glyph quads), the GPU map, the selection/window model, and the binding+scrubber spine. Nothing
in Phase 1's "hardest generic widgets" list is skipped wholesale — each is touched in its
simplest form so the hard version in Phase 1 is an extension, not a first encounter.

---

## 2. Inputs (all already on disk)

**Parsed game UI (offline Python → JSON, committed-shape):**
- `game_assets/gui/topbar.json`, `game_assets/gui/province_state.json` — real parsed `.gui` trees.
- `game_assets/sprites.json` — 26,756-entry sprite manifest (frames / 9-slice `border_size` /
  `png2` / progressbar `color`/`size`).

**Assets (gitignored copyrighted art, resolved via `game_assets` resolver / CLI, never hardcoded):**
- Source `.dds`/`.tga` textures referenced by `sprites.json` (loaded **directly** to GPU — no PNG step).
- `gfx/fonts/*.fnt` + atlases, mapped by `interface/core.gfx` (English/Latin set).

**Map data (`map_pipeline/assets/`):**
- `province_ids.png` (province-id raster; `province_ids_meta.json` for the id encoding),
  `province_to_state.json`, `state_meta.json`, `country_colors.json`.

**Data (the contract — `replay/SCHEMA.md`):**
- A snapshot series under `replay/snapshots/<game>/` keyed by `hour_index` (Clausewitz
  `((year*365+doy)*24+hour)`); `date` string `"YYYY.M.D.H"`; per-field source/units per SCHEMA §0.
- No-fabrication is structural (SCHEMA §0.2): `null` = not captured (render "—"), `0` = measured zero.

---

## 3. Crate layout (Rust workspace) — the boundaries that survive past the slice

```
replay_native/                     (new cargo workspace, native binary — no web, no server)
├── snapshot/      shared crate: serde types mirroring replay/SCHEMA.md + the store reader
│                  (hour_index-keyed series + diff timeline). SHARED with the future Rust reader.
├── assets/        .dds → wgpu::Texture (BCn/BC7 native); BMFont .fnt → glyph table; sprites.json
│                  manifest loader; map_pipeline artifact loader. game-path via resolver/CLI.
├── gui_runtime/   load gui/*.json → retained scene-graph; layout (position/size/orientation/
│                  origo/percent/clip); the ~13 passive element kinds; hit-testing
│                  (alwaystransparent-aware); the special-widget behaviour-registry SKELETON
│                  (gridbox row-factory + the map drag canvas only, this slice).
├── render/        wgpu draw: sprite quads, 9-slice, BMFont glyph-quad batching (nearest sampler),
│                  progressbar; the MAP shader (id-texture + colour LUT recolour); compositor.
├── bind/          the declarative binding spec (typed structs/RON) + ~8 formatters used here
│                  (percent, signed, manpower, date, flag, ideology-tint, factories, fuel).
└── app/           Bevy app (or raw winit+wgpu — DECIDED IN THIS SLICE, §6): window, input,
                   scrubber, surface stacking, selection state, wiring it all together.
```

`snapshot/` and `gui_runtime/` are the load-bearing crates that the whole plan reuses; the
slice is also a test of whether their boundaries are right.

### 3.1 Codex hardening (folded into the crate responsibilities)

- **`assets/` — DDS is not free.** `wgpu` samples BC1/2/3/4/5/6H/7 **only** under the
  `TEXTURE_COMPRESSION_BC` feature. **Audit the real the game `.dds` headers in M1** and keep a **CPU
  transcode fallback** for the edge cases (RXGB/normal swizzle, premultiplied DXT2/4,
  BGRA/uncompressed/A8/luminance, mip chains, odd dimensions, sRGB choice, mod assets). Don't
  assume every `.dds` uploads directly. **AUDIT DONE (M1, 21,815 real files):** 90.7% UNCOMPRESSED BGRA8, 5.8% DXT5/BC3, 2.0% RGB555-16bit, 1.1% DXT1/BC1, 0.2% RGB24, 0.1% DXT3/BC2, 1 RXGB → the game UI is **predominantly uncompressed BGRA8**, BCn a minority; the "mostly BCn" assumption was wrong. The CPU fallback is proven on the real RGB555 + RXGB files; BCn direct-upload proven on real DXT5.
- **`render/` — map texture constraints (hard):** the province-id raster is an **integer (`Uint`)
  texture, `nearest` filter, no mips, no sRGB**; the colour LUT likewise `nearest`. **Pick from the
  CPU-side `province_ids.png`** at the cursor — **avoid GPU readback**. LUT-per-tick (~52 KB /
  ~13k provinces) is noise; keep it.
- **`gui_runtime/` — the registry is bigger than 'gridbox only'.** The slice's behaviour-registry
  skeleton MUST also include the **scroll viewport + scrollbar + a scissor/clip stack**, and
  **glyph measurement AFTER `$VAR$`/`§colour`/`£icon` expansion** (measurement drives
  wrap/truncate). Codex: these are generic foundations; deferring them is the trap. (Still no
  focus/tech tree, no combat adapter.)
- **`snapshot/` — 'shared crate' corrected.** While the producer stays Python it is **not**
  literally a shared crate. The contract is a **frozen, versioned schema** + **golden fixtures**
  asserting `null ≠ 0`, missing-field ≠ default, and old-snapshot back-compat, tested on **both**
  sides; plus a **scrub-latency benchmark**. If hourly JSON is too slow to deserialize, move the
  store to **indexed binary chunks / mmap / windowed prefetch / panel-level lazy reads** — native
  alone doesn't fix big-JSON cost. The types become a true shared crate only once the reader is
  also Rust.
- **`app/` — Bevy is plumbing ONLY** (window/input/asset-load/render-graph). **Never `bevy_ui`.**
  Keep our own scene-graph/layout/scissor/hit-test/draw-extraction; if Bevy's ECS/render phases
  start dictating UI structure, **bail to raw `wgpu`+`winit`** (§6).

---

## 4. The three bindings (concrete — kept minimal, all `exact` source per SCHEMA)

> **AUTHORITATIVE binding source: `BINDING_MAP_topbar_state.md`** (every element of both views mapped + Codex-verified, 2026-06-26) — use it for exact paths. It corrected this sketch: **factories = `economy.factories.{civ,mil,dockyard}` (+ `under_construction`), NOT `production.*`**; state-panel population = **`manpower_pool.total`** (not `.available`); `political_power` renders as **unsigned** int; and **this `topbar.gui` has NO stability/war-support progressbar** (icon+text only — the `progressbartype` primitive is exercised by the state-panel building bars / convoy bars, which are data-`unavailable`-badged). 17 elements across the two views are badged `unavailable` (listed in the doc), none fabricated.

**Topbar** (`topbar.json` element name → snapshot path → formatter):
`political_power → countries.<TAG>.politics.political_power → signed_int` ·
`stability → ….stability → percent` (+ its progressbar) · `war_support → ….war_support → percent`
(+ bar) · `manpower → ….military.manpower.available → manpower` ·
`civ/mil/dockyard factories → ….economy.factories.{civ,mil,dockyard} (+ under_construction) → int` ·
`fuel → ….economy.fuel.current → fuel` · `date → snapshot.date → clock` ·
`flag → <TAG> → flag_sprite (ideology-tinted)`. Outliner, alerts, speed control: **rendered
chrome, inert/deferred** (outliner needs an adapter — out of slice).

**Map** (Political only): per scrub tick, build a province→RGBA LUT: for each province, look up
its state (`province_to_state.json`), the state's `owner` in the snapshot, the owner's colour
(`country_colors.json`); upload the LUT as a 1-D texture; the fragment shader samples
`province_ids` → province id → LUT → colour. `null` owner → neutral grey (not fabricated). Sea /
impassable / unowned render as neutral terrain; **controller-vs-owner, occupation hatching, province
borders, and overlays are deliberately Phase-1 fidelity items** — this slice is **owner-fill only**,
badged `derived` where it diverges from the in-game political map.

**State panel** (`province_state.json`): `name → state_meta + state id` · `owner/controller →
states.<id>.{owner,controller} → flag` · `category → ….category` · buildings grid
(`….buildings.{infrastructure,civilian_factory,military_factory,dockyard,...} → int` rows) ·
`victory_points → ….victory_points_total → int` · `resources → ….resources.* → int rows` ·
`manpower → ….manpower → manpower`. Each cell carries its `exact/derived/proxy/unavailable` badge.
**Honesty (Codex):** our store holds *decoded game state*, not display-ready values — so even these
"thin" bindings carry real formatter + ordering/visibility logic (the ~8 formatters; buildings
ordered + icon-mapped). "Thin" means a *small view-model*, not *zero logic* — the plan's §C
3-stage pipeline applies even here.

---

## 5. Build order (milestones inside the slice — each independently demoable)

- **M0 — data contract in Rust.** `snapshot/` reads the on-disk store; load one game, print a
  country's scalars + a state's buildings for a chosen `hour_index`. *Proves the Python-store ↔
  Rust boundary.* **Add golden fixtures** (`null ≠ 0`, missing ≠ default, back-compat) + a
  scrub-latency micro-benchmark on real hourly snapshots.
- **M0.5 — golden harness (stand it up NOW — Codex's #1 risk).** parsed `.gui` + a fixed snapshot
  → a **deterministic screenshot** + asserted **hit-test points**, baseline committed; wire
  asset/shader/gui **hot-reload** to recover the dev loop the browser gave. *Without this, the
  renderer can be fast and subtly wrong everywhere.*
- **M1 — render primitives.** `assets/` + `render/`: load one `.dds` → texture and one `.fnt` →
  glyph table; open a window; draw a sprite, a 9-slice frame, and a BMFont string. *Proves
  `.dds`-direct + glyph quads + nearest sampler crispness.* **Includes a `.dds`-header audit + the
  CPU transcode fallback wired** (§3.1).
- **M2 — runtime renders the topbar (static).** `gui_runtime/` loads `topbar.json` → scene-graph
  → layout → draw chrome/sprites/text at real coordinates, no binding. *Proves the parser→VM→draw
  path and the layout model.*
- **M3 — bind the topbar.** `bind/` + the snapshot: real values for a chosen country+hour;
  no-fabrication on nulls; one trivial hover tooltip. *Proves the scalar binding path.*
- **M4 — the GPU map.** id-texture + LUT shader recolour (Political); pan/zoom; pick
  (cursor→province→state); hover shows state name. *Proves the map engine + selection.*
  **(Audit-confirmed SUFFICIENT:** the province-id raster is integer-exact + shader-safe
  (`id = R + G*256 + B*65536`, 8-bit RGB, no alpha/sRGB/mip; half-res loses 0 provinces), and the
  joins are complete (1081 states bijective, 0 orphans; all owners resolve). **ONE hazard to fix:**
  `province_to_state.json` has string KEYS but int VALUES, while `state_meta`/`owner_by_state` use
  string keys — NORMALIZE all map ids to one type at load (a typed lookup helper), or a missed
  `to_string()` in the Rust reader silently renders sea/unowned = fabrication-shaped.)*
- **M5 — the state panel (+ the three foundations Codex flagged).** pick → select → open
  `province_state.json` window, **stacked over the map with correct z-order**; verify
  `alwaystransparent` (transparent regions fall through to the map, opaque panel captures). Bind
  scalars + the buildings **gridbox row-factory**; make at least one list **genuinely overflow**
  (clipped viewport + working **scrollbar**); render one **wrapped, marked-up, localized text box**
  whose wrap is computed from glyph metrics **after** `$VAR$`/`§`/`£` expansion. *Proves the 2nd
  surface, window stacking, and the scroll/clip/wrap foundations in one go.*
- **M6 — the scrubber.** a global `hour_index` slider + play/pause re-binds topbar + map + panel
  in-process. *Proves the spine.*
  **(Audit gate, measured:** a full-world JSON snapshot is ~12 MB and full-parses in ~100 ms — so a
  naive per-tick re-parse is the real M6 risk. Fix WITHOUT a binary/mmap rewrite: a **windowed LRU**
  of parsed snapshots (3–8 entries; cache-hit ≈ 0 ms) + a **per-domain on-disk split** so a view
  loads only its domain (the map needs `states`+`map` ≈ 1 MB, not the ~3 MB `divisions` array;
  requires a domain-aware loader, not the monolithic dict) + windowed prefetch on direction change.
  **HARD no-fabrication rule:** a cached snapshot is a SHARED dict instance — it MUST be treated as
  immutable (deep-copy at any mutation boundary), or a view that defaults/annotates in place will
  persist fabricated values across every subsequent scrub tick.)*
- **M7 — decision + retrospective.** record the Bevy-vs-raw-`wgpu` verdict (§6) and a short list
  of what Phase 1 must harden (discovered from real friction, not guessed).

---

## 6. The one decision settled inside the slice: Bevy vs raw wgpu+winit

Build M1–M2 on **Bevy** first (it hands over windowing, input, the `wgpu` render graph, sprite
batching, asset loading, and an ECS that maps cleanly onto a retained scene-graph of many UI
entities). **Keep raw `wgpu`+`winit` as the fallback** and switch only if a concrete friction
shows up: (a) Bevy's coordinate/UI assumptions fight absolute Clausewitz `orientation`/`origo`
layout; (b) the custom BMFont/9-slice/map-shader passes are easier to own directly than to thread
through Bevy's render graph; (c) ECS churn outweighs its benefit for a mostly-retained tree. The
slice produces a **written verdict with the specific reason** (M7), not a vibe.

---

## 7. Acceptance criteria (the slice is DONE when all hold)

1. Launches as a **native window** — no browser, no loopback server, no HTTP.
2. **Topbar** renders from `topbar.json` with real `.dds` sprites, real BMFont text, real 9-slice
   chrome, at real coordinates, bound to a chosen country+`hour_index`; spot-checked values match
   the snapshot; `null` fields show "—", never `0`.
3. **Map** renders below the topbar in Political mode, GPU-shader recoloured by `owner`; pan + zoom
   work; hover identifies the province's state; click selects the state.
4. Click **opens the state panel** bound to that state's snapshot data, correctly stacked over the
   map (clicks on transparent regions fall through; clicks on the panel don't leak to the map).
5. The **scrubber** moves time: topbar values change, the map recolours, the panel updates — all
   **in-process**, on the same snapshot series.
6. **No-fabrication** is visibly honoured and every bound field carries its fidelity badge.
7. The **golden regression harness** runs locally: parsed `.gui` + a fixed snapshot → a
   deterministic screenshot + asserted hit-test points; a known-good baseline is committed.
8. The panel contains **one genuinely overflowing scroll list** (clipped, scrollbar-driven) and
   **one wrapped, marked-up, localized text box** whose wrap is computed from glyph metrics **after**
   `$VAR$`/`§`/`£` expansion.
9. An explicit **z-order / `alwaystransparent`** case passes: transparent regions fall through,
   opaque panel regions capture, overlapping windows resolve front-to-back.
10. A committed **Bevy-vs-raw-`wgpu` verdict** + a Phase-1 hardening list.

**Out of scope / explicitly deferred:** focus/tech trees, combat panel + bubbles, outliner,
decisions/production/diplomacy views, tooltips-everywhere, the trigger evaluator, every map mode
but Political, virtualised long lists, golden-test sweep. (All are named in the plan's later phases.)

---

## 8. Privacy / read-only invariants (carried from the plan, non-negotiable)

- **Read-only, structural.** The slice reads the on-disk snapshot store only. It never reads live
  memory and never writes the game or a save. (Live capture is a separate, already-proven, also
  read-only track.)
- **No personal path committed.** Game-asset and store locations come through the resolver / CLI
  args; converted art + the source `.dds`/atlases stay gitignored under the existing copyright/
  privacy rule. No local home-directory, cloud-documents, or machine-specific Steam-library path
  in any committed file.
