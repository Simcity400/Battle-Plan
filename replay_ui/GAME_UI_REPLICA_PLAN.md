# GAME_UI_REPLICA_PLAN — a full generic Clausewitz-GUI runtime that replays recorded the game state

> **Status:** historical design document. The native Rust/wgpu direction below
> has since been implemented through the engineering DoD: 114 registered views,
> real `.gui` layout ingestion, map/weather wiring, no-fabrication guardrails, and
> host/scrub plumbing are in place. Product parity is still open and is tracked
> in `REPLAY_PARITY_COMPLETION_CHECKLIST.md`.
>
> **Decisions now LOCKED** (these supersede the v1 "limited renderer for ~30 curated screens"
> recommendation; the v1 caution is preserved as the engineering-risk register, not the scope):
>
> 1. **FULL GENERIC CLAUSEWITZ GUI RUNTIME — render EVERYTHING.** Not a curated set of screens.
>    The target is the *whole* in-game the game UI rendering and feeling like the real game. The
>    architecture pivots to a faithful, **generic interpreter of the game's `.gui`/`.gfx` format**
>    (a "GUI VM") that can lay out and draw ANY view, plus a **data-binding layer** that fills
>    every view from our snapshots.
> 2. ⚠️ **SUPERSEDED by the NATIVE RECONCILIATION (v3) section below — now pure Rust + `wgpu`, no web.** (Original v2 decision, kept for history:) **TypeScript** front-end, thin/fast build (esbuild or `tsc`), **no React/Vue/game-engine**.
>    The repo's vendored / minimal-build / stdlib-server ethos still holds; TS via a thin build is
>    now accepted. Canvas2D retained-mode scene-graph for the sprite-heavy panels and the map; DOM
>    only for trivial chrome.
> 3. ⚠️ **UPDATED by v3:** Rust is now the language for **both** the reader hot-path **and** the UI (one binary). (Original v2:) **Rust** an approved **separate parallel track** for the reader's scan hot-path only
>    (§J). It does **not** block the UI.
>
> **Adversarially reviewed** in two historical review rounds — once on the v1 curated plan, and
> again specifically pressure-testing this v2 generic-runtime + binding-DSL pivot. Codex-native
> sessions must use normal sub-agents/partner review, not the old CLI workflow. The v2 critique
> is folded throughout and stated verbatim in §H. Its one-line verdict on
> the pivot:
>
> > *"Generic runtime plus binding DSL is the right backbone for 'render everything', but only if
> > the design admits a substantial library of imperative domain adapters and special-widget
> > behaviors; a pure GUI VM plus raw declarative bindings will visibly fail on the important
> > screens."*
>
> That verdict is the spine of this revision: **the runtime is generic; the data is not poured in
> raw.** The honest pipeline is `snapshot → domain adapter (imperative view-model) → declarative
> template binding`, never `gui-element ← snapshot-path` for the hard screens. Every section below
> is written to that standard.

---

## NATIVE RECONCILIATION (v3 — 2026-06-26): pure Rust + wgpu, no web

> **This section supersedes locked-decision #2 (TypeScript / Canvas2D / loopback-server) and folds in #3 (the Rust reader track).** Everything else in this document — §0, §A–§I, the `.gui`/sprite/font surveys, the binding model, the adapter inventory, the trigger evaluator, the phasing, the hard limits, and the entire Codex v2 risk register — is **substrate-agnostic and stands verbatim.** It describes the game's UI *format* and *our data*, not the rendering technology. Only the rendering/serving substrate changes.

**Decision (user, 2026-06-26): the game-replica front-end is a single native Rust application rendering with `wgpu`.** No web layer, no TypeScript, no loopback server, no browser. Rationale: the replica is a hand-rendered game (GPU map-recolor + sprite/9-slice panels + BMFont glyph quads), *not* an HTML-shaped UI — so the web stack's one advantage (its component/layout ecosystem) buys nothing here, while a webview would impose an IPC boundary, a two-language schema, and a fight to make a browser look like the game. The memory reader is already going Rust (§J), so a Rust UI means **one language, one binary, the snapshot types shared directly with the reader, and zero per-frame data marshalling across a process boundary.**

**Within pure-Rust:** the early Bevy question was settled by the native slice: stay on raw `wgpu` + `winit`. The retained scene-graph, layout engine, hit testing, and renderer are now owned directly by `replay_native/`.

### Substrate swap — section by section (everything not listed is unchanged)

| Plan section | v2 (web) assumption | v3 (native) replacement |
|---|---|---|
| Locked-decision #2 | TypeScript + Canvas2D retained scene-graph + esbuild/tsc | **Rust** + `wgpu` retained scene-graph; Bevy for plumbing (or raw `wgpu`+`winit`) |
| §B.2 / assets | `.dds`/`.tga` → ~33k PNG pre-conversion | **load `.dds` directly to GPU textures** (`wgpu` consumes BCn/BC7 natively) — skip the PNG step; higher fidelity, smaller pipeline |
| §B.6 fonts | glyph quads drawn to canvas, `imageSmoothingEnabled=false` | BMFont glyph quads batched on the GPU; **nearest-neighbour sampler** for crisp bitmap text. `.fnt` parse + `core.gfx` map unchanged |
| §B.9 map | Historical browser prototypes used CPU recolour over `getImageData` | **GPU fragment-shader recolour**: `province_ids.png` → an id-texture; map mode = a per-state/per-province colour LUT texture sampled in-shader. Province pick = sample the id-texture at the cursor. `map_pipeline` products (`province_ids.png`, `province_to_state.json`, `state_meta.json`, `country_colors.json`) reused as-is |
| §B.10 data binding | snapshot `*.json` fetched from `127.0.0.1` loopback server | the Rust app reads the **on-disk snapshot/delta store directly** (`replay/store.py` format; `hour_index`-keyed series + diff timeline). No server, no HTTP, no JSON-over-the-wire. The global scrubber re-binds in-process |
| §B.1 charts | vendored Chart.js | a small Rust chart routine (or `egui` for the analytics-side panels only) |
| §B.11 "why TS" | typed `.gui` AST/binding/scene-graph in TS | same argument, **stronger in Rust**: typed AST/binding-spec/scene-graph + a snapshot crate shared with the reader; enums + match-exhaustiveness kill whole bug classes |
| §C.1 binding spec | declarative JSON/TS config + TS adapters | declarative Rust config (typed structs / RON) + adapters as Rust modules. The 3-stage `snapshot → adapter → binding` pipeline is identical |
| §5 / serving | stdlib loopback server, offline-local | **even more local**: a native binary, no network at all |
| §J reader track | Rust reader a *separate* optional backend track | **unified**: reader + UI are the same Rust codebase/binary; snapshot types a shared crate. Until the reader is ported, the boundary is the on-disk store the Python extractor writes |

### What does NOT change (the intellectual core — all of it)
The reframe (§0), the principles (§A), the **18-element / 7-sprite universe** (§B.1–B.2), the layout/scene-graph/hit-test model (§B.3–B.4), the **two tree adapters** (§B.5), the markup + tooltip engines (§B.7–B.8), the **3-stage binding pipeline + ~10 adapters + ~2,000-binding sweep** (§C), the read-only verbs (§D), the **deliberately-small trigger evaluator** (§E), the **phase structure** (§F — only the tech *inside* each phase swaps), the **hard limits** (§G), the **entire Codex v2 risk register** (§H), and effort sizing (§I). Native swaps the substrate; it does **not** shrink the adapter/binding work — which §H/§I already name as the real cost.

### The .gui parser boundary (transitional)
The `.gui` → tree-JSON parser is Python today (`game_assets/extract.py` → `game_assets/gui/*.json`; 13 priority views already parsed incl. `topbar.json`, `province_state.json`, `research.json`, `national_focus.json`, `production.json`, `construction.json`, `diplomacy.json`, `decisions.json`, `land_combat.json`, `naval_combat.json`). v3 keeps that: **Python parses `.gui`/`.gfx`/`.fnt`/loc → JSON/manifests offline; the Rust app loads those artifacts at runtime.** Porting the parser to Rust is optional and deferred (offline asset-prep, not a hot path).

### Capture model (cross-ref, not re-decided here)
The data the UI binds is produced by the **hourly date-gated sampler**: one raw full-world snapshot per in-game hour, hour-stamped via `current_game_date`@`gamestate+0x468`, decoded (assembler→schema), delta-stored. The UI is **fully decoupled** from capture — it consumes the on-disk store regardless of how the store is produced.

---

## 0. The reframe that makes "everything" tractable (read this first)

**This is a REPLAY of recorded state, not a live game.** That single fact is what turns "clone
the entire the game UI" from a multi-year tar pit into a large-but-bounded build.

The expensive 90% of a real UI clone is the **live trigger/effect/availability LOGIC**: *is this
focus available right now, given 40 script conditions; is this decision visible given the
country's flags and DLC; what does this designer preview compute; what does this tooltip's
modifier breakdown sum to.* A real game must **evaluate** all of that every frame.

A replay does **not**. The recorded snapshot already contains the **resolved** state:

- this focus is **completed / current / not-taken** (we don't ask "is it available?");
- this decision is **active with N days left** (we don't evaluate its `available` trigger);
- these divisions **are in** this combat at this province (we don't run the combat resolver);
- this country **owns/controls** these states (we don't run the war/peace logic).

So the generic runtime + a recorded-state binding **sidesteps the entire trigger/effect engine
for the things a replay shows.** That is the feasibility argument, and it holds for the large
majority of surfaces.

**But be precise about the RESIDUAL** (Codex v2-C, endorsed). A replay still avoids *"can I click
this?"* logic, yet it still needs *"was this element visible, and where was it positioned?"*
logic — and a sliver of that **is** pure script, not reflected in any recorded value:

1. **Focus-tree `offset` branches** change **layout** (node x/y), not just state. The snapshot
   records which focuses are completed/current, but not *which conditional offset branch was
   active*. To place nodes exactly we must either (a) evaluate the offset triggers, or (b) capture
   the resolved node positions at record time. (We choose a **limited offset-trigger evaluator**
   plus a fallback; see §C.3 and §E.)
2. **Conditional `visible = { … }` containers / scripted_gui content.** Some panels, advisor
   slots, decision categories, designer sub-panes, and tag/DLC/ideology UI variants appear or
   vanish based on script the snapshot doesn't record as a boolean. The replay must either
   evaluate a **small subset** of those conditions or **degrade visibly** (show the container in a
   neutral state, badged).
3. **Runtime-COMPUTED numbers we never stored** (combat modifier breakdowns, designer/template
   stat previews, some tooltip math). These are genuine **hard limits** (§G) — badged
   `unavailable`, never fabricated.

Everything else — the chrome, the sprites, the geometry, the lists/grids/trees of recorded
entities, the values, the progress bars — renders from data we have. **The job is therefore: build
the generic runtime once, then do a comprehensive but mostly-mechanical binding sweep, with a
bounded library of imperative view-model adapters for the ~10 screens whose data shape isn't
isomorphic to their `.gui` tree.** That is the whole plan.

---

## A. Vision & principles

Historically `replay_ui/` held browser analytics/dashboard experiments over the
snapshot series. Those runtime files are now removed; the active product is the
native **game replica**: open the same data and have it *look and feel like sitting in a
paused game*, navigable through time. Browse the focus tree, open the research tree, switch
map modes, click a combat bubble to see who's fighting, select a state/country/unit/leader to open
its real panel, hover anything for a tooltip — but you can **never change anything**. It is a
replay of recorded state.

### Principles (priority order)

1. **Read-only, forever.** Every *inspection/navigation* interaction is supported; every
   *mutation* (take focus, queue research, assign factory, draw a plan, design equipment, declare
   war, change law) is **shown but inert**. There is **no** code path that writes to the game or
   the save — read-only is structural, not a UI convention. (§D enumerates the verbs.)
2. **Generic-runtime, asset-driven — but not naïvely so.** Render the game's **real** `.gui`
   layouts and **real** sprites at the **real** coordinates, recolour the **real** province map; do
   not redraw the game by hand. The runtime is a *generic interpreter* of the format (§B). **But** the
   `.gui` files are the **shell + geometry only** — the rows in every list, the nodes in every
   tree, the tooltip bodies, the conditional show/hide are **runtime-populated** and live in our
   snapshot, not the layout file. So "generic" means: one engine renders any view's chrome and
   geometry; a **domain adapter + binding spec** fills the runtime content (§C). The distinction
   between *generic renderer* and *raw declarative pour-in* is the difference between a shippable
   architecture and Codex's "visibly fails on the important screens."
3. **No fabrication (inherited hard rule).** A schema-present-but-null field renders as "not
   captured", never `0`, never invented (SCHEMA §0.2). Every map mode and panel field is **badged**
   `exact / derived / proxy / unavailable` so the viewer always knows whether a number is ground
   truth, a derivation, a region-proxy, or simply not in our data. An "attractive UI hiding a data
   gap" is the failure mode we most guard against.
4. **Fully time-scrubbable.** A single global time scrubber drives the entire UI. Moving it
   re-binds every open surface to that instant's snapshot. The scrubber is the spine.
5. **Offline, local, private.** stdlib loopback server on `127.0.0.1`, no network, no CDN; art
   stays gitignored; **no personal install path in any committed file** — installed content come through
   the `game_assets` resolver / CLI args, never hardcoded.

### Explicit non-goals

- Not a game. No simulation, no "what if", no continuing forward.
- Not pixel-perfect to the last pixel. We target *"unmistakably the game"*. With the real BMFont
  bitmap fonts now in reach (§B.6) text fidelity is far better than v1 assumed, but the exact
  tooltip-composition math and a few computed numbers still drift; we manage drift with golden
  tests (§H), not by chasing the last 5%.
- Not a Clausewitz **trigger/effect engine**. We implement a **deliberately small** trigger
  evaluator for the two places layout genuinely needs it (focus offsets; a curated set of
  `visible` conditions) and **degrade visibly** everywhere else.
- **Out of scope entirely:** the ~28 **frontend / menu / lobby** `.gui` files (main menu,
  multiplayer lobby, matchmaking, chat, credits, settings, tutorial, load screen, music player,
  achievements) — these are not in-game replay surfaces. The runtime *could* render them, but we
  don't bind them. (See §C.4 inventory.)

---

## B. The GUI VM — the new engineering core

The heart of v2 is a **generic, retained-mode interpreter** of the Clausewitz GUI format. It is
built **once**; every windowed view is then "just data" (a parsed `.gui` tree + an adapter + a
binding spec). This section specifies it concretely. The survey below is exhaustive — it enumerates
**every element type and sprite type the entire 130-file game UI actually uses**, so the runtime's
surface area is *known and finite*, not open-ended.

### B.1 The complete element-type universe (surveyed across all 130 game `.gui` files)

There are **exactly 18** `*Type` element kinds in the whole game UI. (Counts = total occurrences
across all 130 vanilla `.gui`; the parser already normalises the casing variants seen in source —
`gridBoxType`/`gridboxtype`, `instantTextBoxType`/`instantTextboxType`, etc.)

| Element type | Count | What it is | How the VM renders + behaves |
|---|---:|---|---|
| **iconType** | 3020 | A sprite instance (the workhorse). | Resolve `spriteType` → manifest entry; draw frame `frame` of an `no_of_frames` strip at `position`+`size`; honour `alwaystransparent` (hit-test pass-through), tint, `orientation`/`origo` anchoring. |
| **instantTextBoxType** / textBoxType | 2795 / 23 | A text label (instant = no typewriter anim). | Resolve loc key + `$VAR$`, run the markup engine (§B.7), measure with the BMFont (§B.6), draw glyph quads; honour `font`, `format` (left/right/center), `maxWidth`/`maxHeight` (wrap/clip), `truncate`. |
| **buttonType** / guiButtonType / ButtonType | 2042 / 323 | A clickable button (sprite + optional text + click/hover frames + `clicksound`). | Draw button sprite (state frame), overlay text; in replay, **most are inert** (greyed/`read-only` cursor) but **navigation buttons** (open view, switch tab, select flag) are wired (§D). |
| **containerWindowType** / windowType | 1991 / 57 | A grouping/positioning container (the structural skeleton; may be a movable window, a sub-panel, a clip region, or a tab page). | Establish a coordinate frame + optional clip rect + optional `background`; recurse into children. **Conditional `visible` lives here** (§C.3). |
| **gridBoxType** | 601 | A **row/cell factory**: lays out N identical child instances on a grid (`slotsize`, `format`/flow direction, `max_slots`). **The grid template is in the `.gui`; the rows are runtime.** | The VM instantiates the child template once **per bound array element** (§C.2). This is a **special widget**, not a passive container (Codex v2-A). |
| **listBoxType** / smoothListBoxType | 15 / 14 | A scrollable vertical list (smooth = momentum). Same row-factory nature as gridbox + a scroll viewport + scrollbar. | Viewport clip + scrollbar + per-element row instancing; **virtualise** long lists (division table, event feed). |
| **scrollbarType** / extendedScrollbarType | 31 / 21 | Scrollbar widget (`slider`/`track`/`increaseButton`/`decreaseButton`, `tileSize`). | Render track+thumb from sprites; drive the parent viewport's scroll offset. |
| **OverlappingElementsBoxType** | 46 | A flow container that overlaps/spaces its children (e.g. ideology pips, medal rows). | Lay children left→right with `format`+`spacing`; bind from an array like a 1-D gridbox. |
| **checkBoxType** | 162 | A toggle (sprite frame = on/off). | Draw bound boolean as frame state; inert in replay. |
| **dropDownBoxType** | 37 | A dropdown (collapsed button + popup list). | **Special widget** — needs option population + open/close behaviour even read-only (show the *selected* value; popup shows the recorded set if we have it, else degrades). |
| **editBoxType** | 85 | A text input (search boxes, name fields). | Render as a static styled box showing recorded/placeholder text; **non-editable** in replay (search boxes that filter our own view are the exception — wired to client-side filter). |
| **positionType** | 162 | A named anchor point (no visual) used by code to place runtime children. | The VM exposes these as **named slots** the adapter can target (e.g. "place the combat roster here"). |
| **browserType** | 1 | A one-off embedded web-view widget (frontend only). | Out of scope (frontend). Render a placeholder if ever encountered. |
| **pieChartType** / lineChartType | 5 / 3 | Data charts (a few stat panels). | **Semantic transform**, not sprite draw — feed bound series to a small canvas chart routine (we already vendor Chart.js for the analytics mode; reuse). |

**Consequence:** the runtime must implement ~13 *passive* element kinds (straightforward) **plus a
"special-widget BEHAVIOR REGISTRY"** for the handful that are engine-semantic, not just layout:
`gridBoxType`/`listBoxType`/`smoothListBoxType` (row factories), `dropDownBoxType`, `scrollbarType`
(scroll state), the focus-tree `drag_scroll` canvas (§B.5), and charts. Codex v2-A is explicit:
*"the GUI VM is feasible as a renderer core, but not as a complete 'render any view' system without
a per-widget/per-view behavior registry."* We bake that registry in from Phase 0.

### B.2 The sprite-type universe (surveyed across all `.gfx`)

Exactly **7** sprite kinds, all already captured in the 26,756-entry `sprites.json` manifest:

| Sprite type | Count | Render rule |
|---|---:|---|
| **spriteType** | 23530 | Plain (optionally framed) texture. `no_of_frames` = horizontal strip; draw frame *i*. |
| **frameAnimatedSpriteType** | 152 | Animated strip (loops frames over time). Replay: draw frame 0 or animate at the recorded tick — animation is cosmetic. |
| **corneredTileSpriteType** | 133 | **9-slice** window chrome (`border_size` = corner insets). Draw 4 corners fixed, 4 edges tiled/stretched, centre tiled. |
| **progressbartype** | 112 | A bar with `color`/`color_two` gradient + authoritative `size`. Draw `value`-fraction of the gradient (fuel/stab/WS/org/strength/research bars). |
| **textSpriteType** | 77 | A sprite that *is* a glyph row (icon font). Treat as a framed icon strip. |
| **maskedShieldType** | 11 | Two-layer composite (`png` flag + `png2` mask/frame) — player-flag shields. Composite order: flag → mask → frame → ideology tint. |
| **pieChartType** / lineChartType | 5 / 3 | Chart sprites (data-driven, §B.1). |

The v1 manifest already encodes `no_of_frames`, `nine_slice`/`border_size`, `png2`, and progressbar
`color`/`color_two`/`size`. The **rendering-primitive hardening** (premultiplied-alpha halos, gamma,
exact frame indexing, 9-slice edge behaviour, masked-shield composite order, progressbar layering,
ideology/country tint) remains Phase-0 work with **visual golden tests**, because these bugs recur
on every screen if not fixed once up front.

### B.3 The LAYOUT model

The VM is a retained-mode scene-graph. Layout resolution per node:

- **position** `{ x y }` — parent-relative offset.
- **size** `{ x y }` or `{ width height }`; **percent sizes** (`x = 100%%` style) resolve against the
  parent's resolved size; `-1` / auto sizes resolve to content or sprite size.
- **orientation** (`upper_left`/`center`/`lower_right`/…) — which parent corner the child anchors
  to; **origo** — which corner of the *child* is the anchor point. Together they give the full
  9-anchor box model. (2393 `orientation` + 73 `origo` occurrences — pervasive; must be exact.)
- **gridBox flow:** `slotsize { width height }`, `format` (flow direction left/right/up/down),
  `max_slots` (wrap), `add_horizontal`/`add_vertical` spacing. Rows instanced from a bound array.
- **clipping** (708 occurrences) + scroll viewports — a node with `clipping = yes` establishes a
  scissor rect; scrollable containers offset their children by the scroll position and clip.
- **z-order / overlap** — child draw order = declaration order; `OverlappingElementsBoxType` and
  explicit layering for tooltips/popups draw above.

### B.4 The SCENE-GRAPH + hit-testing

- A view instantiates by **deep-cloning its parsed `.gui` tree** into runtime nodes, resolving
  layout top-down, then running its **adapter** to instance dynamic children and apply bindings.
- One retained scene-graph per open window; the compositor draws windows in stack order onto the
  shared canvas (the map is the bottom layer; panels stack above).
- **Hit-testing** walks the tree front-to-back honouring `alwaystransparent` (1880 occurrences — a
  transparent element passes the click to whatever is behind it; getting this wrong re-creates the
  v1 country-select "shine ate the click" bug). Hit → resolve the bound datum → dispatch the
  read-only verb (select / open / tab / tooltip).

### B.5 The two tree engines (focus + tech) — where node geometry actually lives, and why they need imperative adapters

This is the project-defining subtlety, and it's exactly where Codex v2-B says *"pure declaration
breaks."* Both trees get **imperative view-model adapters**, not raw bindings:

- **Tech / doctrine tree** — node positions are **static, embedded in the `.gui`**
  (`countrytechtreeview.gui`, 6678 lines; `countrydoctrinetreeview.gui`). Per-folder blocks
  (`infantry_folder`, `armour_folder`, …) place each tech as a positioned `iconType`/`instantTextBox`,
  often anchored relative to a sibling tech. **Adapter job:** parse the folder blocks → `{techId →
  rect}`; bind our `research` domain (completed levels, in-progress, slots, bonuses) onto each;
  draw link lines from prereqs. Geometry is static ⇒ **this is the easiest tree and the first
  proof** (§F Phase 2).
- **Focus tree** — node positions live in `common/national_focus/*.txt`: base `x`/`y` +
  `relative_position_id` (anchor to a sibling focus) **plus dynamic `offset = { x y trigger = { … }
  }` blocks** gated by arbitrary Clausewitz triggers (game rules, `has_completed_focus`, flags,
  tags, DLC). Prereq/`mutually_exclusive` **link lines** also come from these files. The `.gui`
  (`nationalfocusview.gui`) supplies only the **frame, search box, zoom slider, the `drag_scroll`
  scrollable `tree` container, and a node prototype** — **not the nodes**. **Adapter job:** parse
  the focus script → node+edge graph; resolve `relative_position_id` chains; apply the **limited
  offset-trigger evaluator** (§E) for the conditional offsets; bind `focus` snapshot
  (completed/current/progress) onto node state; route the link lines. This is a genuine imperative
  adapter feeding the generic `drag_scroll` canvas + node-prototype renderer.

The `drag_scroll` canvas itself (focus tree + maps) is a **special widget** in the behavior
registry: pan/clip, link-line layer, node layering, zoom.

### B.6 FONTS — bitmap-faithful via BMFont (a major v1 upgrade)

**v1 under-treated this and assumed web-font substitution + accepted drift. The survey overturns
that:** the game fonts are **AngelCode BMFont** bitmap fonts and are fully extractable, so text can be
rendered **pixel-faithfully**, not approximated.

- **Format:** `gfx/fonts/*.fnt` (59 descriptor files) pair with `.dds`/`.tga` glyph atlases. Each
  `.fnt` is standard AngelCode text: `info face="Ubuntu Light" size=18 …`, `common lineHeight=…
  base=… scaleW=… scaleH=… pages=…`, `page id=0 file="hoi_18_0.dds"`, then `char id=… x=… y=…
  width=… height=… xoffset=… yoffset=… xadvance=… page=… chnl=…` rows (+ optional `kerning`
  pairs). The named game fonts (`hoi_36header`, `hoi_18`, `hoi_22tech`, …) are declared in
  `interface/core.gfx` as `bitmapfont { name=… fontfiles={…} languages={…} }`, mapping a logical
  font name → one or more `.fnt`+atlas sets (with per-language variants).
- **Pipeline (Phase 0):** extend the asset extractor to (1) parse every `.fnt` into a glyph table
  `{codepoint → {atlas-page, src-rect, xoffset, yoffset, xadvance}}` + `lineHeight`/`base`/kerning;
  (2) convert the glyph atlases (`.dds`/`.tga` → PNG, same converter as sprites); (3) build the
  `core.gfx` name→`.fnt` map (honouring `languages` so we pick the English/Latin set). Atlases are
  **gitignored copyrighted art** like the textures.
- **Renderer:** text draws as **textured glyph quads** from the atlas (NOT canvas `fillText`),
  with `imageSmoothingEnabled = false` so bitmap glyphs stay crisp.
- **Codex v2-D gotchas, folded in:** (a) `£icon` inline tags are **sprite inserts**, not glyphs —
  they affect line-height/baseline/wrapping and must be measured as boxes; (b) `§Y…§!` colour
  spans change the **draw colour** but must **not** change glyph **advances**/measurement; (c)
  resolve `$VAR$`/nested-`$KEY$` substitution **before** measuring; (d) honour any `.fnt` **kerning**
  pairs, and where absent, **match the absence** (don't "improve" spacing); (e) replicate the
  outline/shadow channel (`chnl`, `outline=` in the `.fnt` `info`) faithfully or text reads wrong;
  (f) **width measurement is the real fidelity point** — wrap/`truncate`/ellipsis/auto-size/clip
  all depend on exact `xadvance`+offsets+lineHeight, so the measurement routine is shared by the
  layout pass and the draw pass; (g) Cyrillic/CJK fall back to language-variant atlas pages — for an
  English replay we load the Latin set and substitute a marker for unsupported codepoints (badged).

### B.7 The MARKUP / colour / icon-tag engine

A shared tokenizer used by both measurement and draw: split a resolved string into runs of
`{text, colour, iconSprite}` by parsing `§<code>…§!` colour spans and `£<key>[¤]` icon tags, and by
expanding `$VAR$`/`$KEY$` loc substitution (recursively, bounded depth). Colour codes map to the
game palette; icon keys map to manifest sprites sized to the line. This engine is what makes a
tooltip or a label read as the game rather than as raw markup.

### B.8 The TOOLTIP composition engine (generic, loc+data-driven)

A generic `tooltip(target)` resolver: given a hovered node + its bound datum, look up the tooltip
loc key, substitute `$VAR$` from the datum, run the markup engine, and render a styled box using the
game's tooltip frame sprites (9-slice). On top of the generic resolver sit **per-domain composers**
(focus / tech / equipment-variant / division / state / law / leader) that format their domain's
fields the game way (icons, signs, %, sorted modifier lines). **Honest divergence:** where the game
*computes* a tooltip value we don't model (a derived combat-modifier breakdown, a designer stat),
the tooltip shows what we *have* and greys/omits what we don't, **badged** — never a fabricated
modifier line. Realistic target: *"HOI4-looking explanatory tooltips over replay data,"* not the
exact in-game modifier-composition engine.

### B.9 The MAP engine (carried from v1, unchanged scope)

Build directly on `map_pipeline` (province-id raster: pixel→province→state in one decode).
The old `map_view`/`replay_render` browser prototypes were removed after their
ideas were absorbed into the native GPU map path. The map is the bottom
compositor layer; the GUI VM windows stack above it.

- **Map-mode switcher** — each mode is a recolor function over the same decoded id-raster, each
  carrying its fidelity badge:

  | Map mode | Data | Fidelity |
  |---|---|---|
  | Political (owner) | `states[*].owner` | **exact** |
  | Control (controller, contested hatch) | `states[*].controller`, `provinces[*].controller` | **exact** (province-level control is cracked) |
  | Resistance / Compliance | `states[*].occupation.{resistance,compliance}` | **exact** |
  | Victory points | per-state / per-province VP | **exact** |
  | Buildings (factories/infra/forts/airbase) | `states[*].buildings` | **exact** |
  | Resources | `states[*].resources` | **exact** |
  | Weather (temp/precip/snow) | per-province weather | **exact** values, **proxy** vs the game's exact visual mode |
  | Front lines | derived from control adjacency | **derived** (a proxy, not the battleplan front) |
  | Faction / wars / subjects | `diplomacy_world` | **exact** at country granularity |
  | Air superiority | `naval_air.air_superiority` | **proxy** (strategic-region granularity) |
  | Naval (sea-zone dominance) | `naval_air.region_control` | **proxy** (region only) |
  | Supply | `supply.by_province` (division-province proxy) | **proxy** — exact per-province supply is a **proven wall** (not stored). |

- **Selection / picking** — pixel→province→state in one `getImageData`; click a province → select
  its state → open the province/state panel; click a country → country view; click a unit counter →
  division panel; hover → tooltip.
- **Unit counters + combat bubbles** — promote `replay_render` division markers to interactive
  NATO-ish counters (owner-coloured, strength-sized) → division panel; place a clickable bubble at
  each active combat's `location_province` → combat panel.

### B.10 The DATA-BINDING runtime + time scrubber

- One **snapshot index** in memory: the `<hour>.snapshot.json` series keyed by `hour_index` (read
  from `index.json`, never reconstruct).
- Each open surface registers a `bind(snapshot)` that pulls only its domain(s) and updates its
  scene-graph/recolor. The **global scrubber** sets the current `hour_index` and calls `bind` on
  every open surface; play/pause auto-advances.
- **Performance discipline:** lazy-load snapshots on scrub; decode the id-raster once; precompute
  expensive mode rasters; diff-rebind (repaint only what changed between ticks); lazy-load sprites
  per-screen and atlas the common chrome (never load 33k PNGs upfront); LRU on snapshot JSON;
  **virtualise** long lists/grids (division table, event feed, combat roster).

### B.11 Why TypeScript, no framework

The runtime's hard parts — sprite atlas, BMFont glyph layout, 9-slice, map recolor, hit-testing,
the scene-graph — are framework-agnostic canvas work; React/Vue/a game-engine buys nothing and
violates the repo's no-heavy-framework ethos. **TypeScript** is now accepted (locked decision #2)
because a typed `.gui` AST, a typed binding spec, and a typed scene-graph are exactly the kind of
code where types prevent whole classes of bug; a thin `esbuild`/`tsc` step is acceptable (no
React/Vue/bundler-zoo). This paragraph is historical; the active implementation is the native
Rust/wgpu app with no loopback server.

---

## C. The GENERIC DATA-BINDING model — how every view gets its data

This is the second pillar and the one Codex pressed hardest. The honest architecture (Codex v2-B,
verbatim): *"A binding DSL can describe the final mapping, but these screens need imperative
adapters that produce view models first. The honest architecture is `snapshot → domain adapter →
declarative template binding`, not raw `gui-element ← snapshot-path`."* We adopt that exactly.

### C.1 The three-stage pipeline (per view)

```
recorded snapshot ──▶ [domain adapter]  ──▶ view-model  ──▶ [declarative binding spec] ──▶ scene-graph
                      (imperative, per-      (plain typed   (config: element-id/role ←
                       domain; only where     arrays/objs)    view-model path + formatter)
                       shape ≠ gui tree)
```

- **Domain adapter (imperative, TypeScript):** transforms our raw snapshot domains into a
  **view-model** whose shape *is* isomorphic to what the binding needs — sorting, grouping,
  aggregation, graph-building, hierarchy. Needed **only** for the non-isomorphic screens
  (enumerated in C.2). For simple panels (topbar, state panel) the adapter is a thin pass-through.
- **Binding spec (declarative config, JSON/TS):** for each dynamic element in the `.gui`
  (a textbox, icon, progressbar, gridbox, listbox), a rule:
  `{ target: <gui element name or role>, source: <view-model path>, format: <formatter id>,
  fidelity: <exact|derived|proxy|unavailable> }`. Arrays bind to gridbox/listbox **row templates**
  (one instance per element). The fidelity badge flows through to the rendered cell.
- The runtime applies the binding spec mechanically; **the only hand-written code per hard screen
  is its adapter.**

### C.2 Where pure-declarative binding holds vs where an imperative adapter is required

| Binding pattern | Example views | Declarative-only? | Adapter needed (why) |
|---|---|---|---|
| **Scalar fields** (text/icon/bar/frame ← one value) | topbar, state panel, leader panel, war overview | **Yes** | thin pass-through |
| **Flat array → gridbox/listbox rows** | research folders, decision list, diplomacy country list, division table, MIO list, officer corps, production lines, trade contracts, event feed | **Mostly** | adapter does sort/filter/group + per-row formatting; binding instances rows |
| **Graph + link lines** | **focus tree**, tech/doctrine trees | **No** | imperative adapter: node+edge graph, `relative_position_id` resolution, offset triggers, link routing (§B.5) |
| **Aggregated roster** | **combat panel** | **No** | adapter aggregates province-combat enumeration → attacker/defender groups, naval per-ship rows, duration/progress fallbacks, missing-detail degradation |
| **Nested / heterogeneous** | **outliner** | **No** | adapter builds the nested, priority-grouped, collapsible tree of mixed row types (country/army/research/production/focus) |
| **Conditional category headers** | decisions (categories), designers (module slots) | **Partly** | adapter groups + emits headers + applies recorded visibility (§C.3) |
| **Charts** | a few stat panels | **No** | adapter builds the series; chart routine draws |

So: **~5 imperative adapters** (focus graph, tech/doctrine graph, combat roster, outliner, decision
grouping) plus thin per-domain formatters do the heavy lifting; **everything else is the
declarative sweep.** This bounds the bespoke code to a known, small set — which is precisely the
condition Codex set for the architecture to be the right one.

### C.3 Conditional visibility in a replay (the residual trigger problem)

Most show/hide reduces to **"do we have a row for it?"** (no recorded combat → no bubble; no advisor
in the slot → empty slot) — which the data answers without any trigger eval. The residual cases:

- **`visible = { <trigger> }` on a container** where the condition isn't reflected in recorded data
  (tag/DLC/ideology/game-rule UI variants, scripted advisor/spirit slots, conditional decision
  categories). Strategy, in order: (1) if a recorded flag implies it, use the flag; (2) else evaluate
  it with the **limited trigger evaluator** (§E) *if* its triggers are in the supported subset; (3)
  else **degrade visibly** — render the container in a neutral default and badge it, rather than
  guess. We **log** every unsupported visibility trigger so the binding sweep can prioritise.
- **Focus `offset` layout branches** — handled by the offset evaluator (§E); on an unsupported
  trigger, fall back to the base x/y and badge the node's position as approximate.

### C.4 EXHAUSTIVE view inventory (all 130 game `.gui`, grouped by binding pattern)

The whole game ships **130 `.gui` files**. Grouped by what the replay does with each:

**(0) NON-REPLAY FRONTEND / MENU / LOBBY (not snapshot-bound, ~28 files):** `frontendmainview`, `frontendgamesetupview`,
`frontendmultiplayerview`, `frontend_friends_view`, `frontendcreditview`, `ingamelobby`, `playerlobby`,
`matchmaking`, `matchmaking_chat`, `chat`, `hotjoin`, `pops_login`, `social_view`, `subscription_message_view`,
`achievements`, `credits`, `settings`, `playeraiprefs`, `savedgamerules`, `messagesettings`, `msg_filters`,
`tutorialscreen`, `load_screen`, `browser`, `musicplayer`, `music_station_base`, `music_station_classic`,
`backend`. **Not snapshot-bound** (no recorded in-game replay state). The runtime can render them when a
host-runtime frontend surface is intentionally built; the in-game replay surface does not bind them as
game-state panels.
**(User-confirmed 2026-06-26: the in-game set is EXHAUSTIVE — every in-game view/panel/menu/tree/
designer/combat/stat, zero curation; the pre-game FRONTEND shell is the only non-replay frontend group,
because a replay of a recorded game has no lobby/main-menu state to bind. The lone sliver with data —
the player roster — may surface inside an in-game surface instead.)**

**(1) Nation-specific scripted_gui overlays (~19 files):** the `*_scripted_gui.gui` /
`*_factions_*` / `*_campaign_*` / `*_system_*` set (`GER_inner_circle`, `JAP_imperial_influence`,
`SOV_paranoia_system`, `USA_congress`, `SWE_rearnament_system`, `AST_*`, `BUL_*`, `GRE_*`, `RAJ_famine`,
`RAJ_eic_tax_fraud`, `war_escalation`, …). These are **scripted-GUI mini-panels** whose content is
driven by script variables/flags. **Mostly degrade-or-skip** in early phases: bind only the ones
whose backing variables we record; the rest render their chrome with a "scripted content not
captured" badge. (This is the clearest case of Codex v2-A's "scripted_gui attaches behavior outside
the `.gui` tree.")

**(2) CORE in-game views — the replay target (~55 files), by binding pattern:**

| Group | `.gui` views | Pattern | Adapter? | Data fidelity |
|---|---|---|---|---|
| **Topbar / clock / minimap / alerts / outliner** | `topbar`, `ingameclock`, `minimap`, `alerts`, `findview`, `rightclickmenu` + the **outliner** (in `menubar`/topbar) | scalar + nested (outliner) | outliner adapter | **exact** |
| **Menu / shortcut bar** | `menubar` | scalar/buttons | — | exact |
| **Map-mode chrome** | `mapmodes_interface`, `mapicons` | map engine | — | mixed (badged §B.9) |
| **State / province / occupation** | `countrystateview`, `countryoccupationview` | scalar + small grids | thin | **exact** |
| **Country politics / government** | `countrypoliticsview`, `powerbalanceview`, `command_character_view`, `advisorroleselectionview` | grids + conditional slots | grouping adapter | exact data; some scripted slots degrade |
| **Focus tree** | `nationalfocusview` (+ `common/national_focus/*.txt`) | **graph** | **focus adapter** | completed/current **exact**; availability/offset **degrade** (§E) |
| **Research / doctrine** | `countrytechnologyview`, `countrytechtreeview`, `countrydoctrinetreeview` | **graph (static geom)** | **tech adapter** | **exact** (levels + in-progress) |
| **Production / construction / deployment / equipment overview** | `countryproductionlineview`, `countryconstructionsview`, `countrydeploymentview`, `equipmentoverview` | array→grid | thin | **exact** (computed prod stats badged where derived) |
| **Designers** | `tank_designer_view`, `plane_designer_view`, `divisiondesignerview`, `equipmentdesignerview`, `equipmentupgradedesignerwindow` | grids + module slots | thin | layout+modules **exact**; **computed combat stats `unavailable`** (hard limit §G) |
| **Decisions** | `countrydecisionview`, `operationoverview` | array→grid + categories | grouping adapter | active/timed **exact**; one-shot availability not stored |
| **Diplomacy / trade / peace** | `countrydiplomacyview`, `countrytradeview`, `peaceconferencewindow`, `worldtensionwindowpopup`, `worldtensionwindow` | grids | thin | **exact** (opinion = cached sum) |
| **Intel / operatives / MIO** | `countryintelligenceagencyview`, `countryintelledger`, `operative`, `operativeleader`, `operationoverview` | grids | thin | **exact** (save-backed; live reader confirm) |
| **Officer corps / high command / leaders / traits** | `countryofficercorpview`, `command_character_view`, `unitleaderwindow`, `navyleaderview`, `leadergroups`, `acesview`, `armytraittreewindow`, `navytraittreewindow`, `traittreewindow_common` | grids + trait trees | thin + trait-tree adapter | **exact** (953/953 leaders) |
| **Army / navy / air mgmt + plans** | `countryarmyview`, `naviesview`, `airgroupview`, `airselectionview`, `airwingdetails`, `airwingreorganization`, `carrierairwingcompositionwindow`, `strategicairview`, `battleplantools`, `abilitylist`, `theatreselector`, `divisions_summary`, `railway_gun` | hierarchy + grids | OOB adapter | OOB **exact**; **battle-plan ORDER geometry partial** (§G) |
| **Logistics / supply** | `countrylogisticsview`, `garrisonlogview` | map + grids | thin | supply per-province **proxy** (wall) |
| **Combat** | `landcombat`, `navalcombat`, `combatlogview`, `navalcombatresultswindow`, `destroyedshipswindow`, `naval_losses`, `naval_strategic_region_view` | **roster aggregation** | **combat adapter** | enumeration→bubble+roster+lifecycle **exact**; **battle-detail (width/terrain/tactics/progress/modifiers) `unavailable`** (§G) |
| **Unit / division detail** | `unitview`, `divisions_summary` | scalar+grid | thin | **exact** (org/strength/equipment) |
| **Events / popups / notifications** | `eventwindow`, `popupwindow`, `detailedwindows`, `core` | array→feed | thin | fired-event ids **exact**; **event TEXT/options need event-file parse + loc** |
| **War / faction / insignia** | `waroverview`, `insigniaselection`, `insrevolutionwindow`, `peaceconferencewindow` | grids | thin | **exact** |

**Tally:** **130 total** → **~28 out of scope** (frontend), **~19 scripted_gui** (degrade/selective
bind), **~55 core in-game views in scope**. Of the ~55 in scope: roughly **40 render
generically + bind declaratively from data we have** (scalar + flat-array screens — the topbar,
state, production, construction, diplomacy, trade, intel, MIO, officer corps, unit/leader, war,
deployment, occupation, events feed, …), **~10 need an imperative view-model adapter** (focus graph,
tech/doctrine graph, combat roster, outliner, decisions grouping, OOB hierarchy, trait trees,
politics conditional slots, designers module-slots, scripted_gui), and **~5 carry genuine
`unavailable`/`proxy` data badges** (designer computed stats, supply, air/naval dominance,
battle-plan order geometry, the *faithful* combat dialog math). **No core view is fully blocked** —
every one has at least a read-only render from data we have; the gaps are about *fidelity of
specific fields*, not *existence*.

### C.5 Binding-sweep size estimate (the bulk of the work)

The sweep is "comprehensive but mechanical." Rough sizing: **~55 in-scope views × ~20–60 bindable
dynamic elements each ≈ 1,500–2,500 binding rules**, of which the large majority are one-line
declarative `{target, source, format, fidelity}` entries reusing a small set of **formatters**
(~40: percent, signed-modifier, days-remaining, flag, ideology-tinted-sprite, org-bar, strength-bar,
date, manpower, equipment-icon, …). The **non-declarative** cost concentrates in the **~10
adapters** (each a bounded, testable TS module). So the effort curve is: build the runtime (hard,
once) → write ~10 adapters (medium each) → grind ~2,000 mostly-trivial bindings (long but mechanical,
parallelisable, and incrementally shippable view-by-view).

---

## D. Interaction model (read-only verbs)

**Supported (inspection / navigation):** **navigate** (open any view from a shortcut bar) ·
**select** (province/state/country/unit/leader/formation → context for panels) · **open/close**
(stackable windows) · **tab** (within a view) · **hover → tooltip** · **click → drill-in** (flag →
country; advisor → leader; combat bubble → combat panel; production line → detail) · **map-mode
switch** (badged) · **time-scrub** (global scrubber re-binds every open surface; play/pause) ·
**search / filter** (focus search, tech/division filters, event feed — wired to client-side filter
over our data) · **pan / zoom** (map + the two big trees).

**DISABLED (every mutation), shown-but-inert:** take/queue focus · queue/cancel research ·
assign/change production · draw/modify a battle plan · create/modify a variant or template · take a
decision/mission · any diplomacy action · change a law / appoint advisor / recruit operative / start
operation · build anything. These render with their **real buttons** but are **non-interactive**
(greyed or "replay — read-only" cursor). There is **no write path** in the tool — read-only is
structural.

---

## E. The deliberately-small trigger evaluator (scoped, not a full engine)

A replay needs *some* trigger evaluation for exactly two things: **focus `offset` layout branches**
and a **curated set of `visible` conditions** (§C.3). We build a **deliberately small** evaluator:

- **Supported subset (target):** the cheap, common, data-backed triggers — `tag` / `original_tag`,
  `has_dlc`, `has_game_rule`, `has_country_flag` (if recorded), `has_completed_focus` (we record
  completed!), `has_idea` / `has_country_leader_ideology` (recorded), simple `AND`/`OR`/`NOT`/`NAND`
  nesting. These cover the majority of focus offsets and visibility gates because they key off
  exactly the resolved state we already record.
- **Out of scope:** anything needing live computation (counts, comparisons against live modifiers,
  `num_of_factories > x`, war/combat predicates not in our data). On an unsupported trigger:
  **log it, fall back** (base offset / neutral visibility), and **badge** the affected node/panel
  approximate. Never promise exact availability.
- This is the §0 "residual" made concrete. It is the **only** piece of the Clausewitz logic engine
  we reimplement, and it is bounded by the supported-subset list — not open-ended.

---

## F. Phasing — build the runtime first, then sweep

Re-phased for **build-the-runtime-first**. Each phase ships something usable.

### Phase 0 — Foundation: the `.gui` parser→runtime CORE + loc + fonts + asset hardening *(unblocks all)*
- **Loc layer:** parse `localisation/english/*.yml` (mod-over-vanilla), resolve `$VAR$` / nested-key
  / `§colour` / `£icon` markup into the render model (§B.7).
- **BMFont pipeline (§B.6):** parse `.fnt` glyph tables + `core.gfx` name-map; convert glyph atlases;
  the shared measure+draw glyph-quad routine.
- **Asset/primitive hardening:** sprite lookup + frame + 9-slice + masked-shield composite-order +
  progressbar layering + ideology/country tint + lazy load + common-chrome atlas, each with a
  **visual golden test**.
- **The GUI VM core (§B):** parse a `gui/<view>.json` tree → retained scene-graph; implement the
  ~13 passive element types + the layout model (orientation/origo/percent/clip/scroll) + hit-testing
  + the **special-widget behavior-registry skeleton** (gridbox row-factory, scrollbar, drag_scroll).
- **Render one diverse view end-to-end** to validate the spine: the **topbar (#1)** — real
  `topbar.gui`, real sprites, real BMFont text, loc, bound to the live snapshot, working tooltips.
- **Deliverable:** a pixel-faithful, data-bound topbar proves the parser→runtime→bind→draw path.

### Phase 1 — Runtime hardened across ALL element types + map + selection *(unblocks navigation)*
- Harden the VM on the **most complex element behaviours**: the **gridbox/listbox row-factory**
  (validated on the **division table** and **diplomacy country list**), the **scroll viewport +
  scrollbar**, the **drag_scroll canvas**, dropdown, overlapping-elements, charts. This is where the
  behavior registry earns its keep.
- App shell: shortcut/menu bar, the **global time scrubber** (the spine), game selector,
  fidelity-badge legend.
- **Map engine** (§B.9): mode switcher (political/control/resistance/compliance/VP/buildings/
  resources first — all exact), pan/zoom, the **selection model** (province→state→country, unit
  counters, hover tooltips), province/state panel (#9) + country-view scaffold wired to selection.
- **Deliverable:** a navigable, time-scrubbable HOI4-chrome map you can click around, with the
  runtime proven on the hardest *generic* widgets.

### Phase 2 — Research / doctrine trees (the static-geometry tree proof)
- The **tech adapter** (§B.5): harvest node geometry from `countrytechtreeview.gui` /
  `countrydoctrinetreeview.gui`; build the graph renderer (scroll, zoom, link lines); bind
  `research` (completed levels, in-progress, slots, bonuses); tooltips; handle shared/xor folders,
  small-vs-big tech, doctrine XOR.
- **Deliverable:** browse research + doctrine at any replay time. Validates **graph rendering +
  node binding + scroll + tooltips** for the focus tree to reuse.

### Phase 2b — Focus tree (the script-conditional tree)
- The **focus adapter** (§B.5): parse `national_focus/*.txt` → node+edge graph; render
  completed/current/progress + prereq/mutually-exclusive link lines into the `drag_scroll` canvas;
  apply the **limited offset-trigger evaluator** (§E) — log + visibly fall back on unsupported
  triggers; never promise exact availability.
- **Deliverable:** browse the focus tree like in-game for completed/current; offsets/availability
  best-effort + badged.

### Phase 3 — The BINDING DSL + the binding sweep across all views (the BULK)
- Formalise the **declarative binding spec** (§C) + the ~40 shared formatters; write the **~10
  imperative adapters** (combat roster, outliner, decisions grouping, OOB hierarchy, politics
  conditional slots, designer module-slots, trait trees, scripted_gui-selective, plus the two tree
  adapters already done).
- **Grind the ~2,000 bindings view-by-view**, shipping each view as it's bound: topbar/state/
  production/construction/diplomacy/trade/intel/MIO/officer-corps/unit/leader/war/deployment/
  occupation/events. This phase is long but **mechanical, parallelisable, and incrementally
  shippable**.
- Includes the **combat bubble→panel** (#25): bubbles at active-combat provinces; panel = attacker/
  defender rosters + naval per-ship + location + duration + lifecycle; **badge** the uncracked
  battle-detail `unavailable`.
- **Deliverable:** the whole in-game UI renders and binds; most surfaces feel like the game.

### Phase 4 — Residual trigger-eval, computed-value badging, tooltips-everywhere, polish *(integration)*
- The per-domain **tooltip composers** across all surfaces; the **proxy map modes**
  (air/naval/supply/logistics, badged); front lines; event feed text (event-file parse + loc);
  scripted_gui selective binding; the residual `visible` evaluations.
- Performance pass (lazy load, raster cache, diff-rebind, virtualise, LRU); golden-test sweep;
  scrubber-integration polish so every open surface re-binds smoothly.

**Dependency spine:** Phase 0 gates everything · the hardened widgets (Phase 1) gate every list/grid
screen · the tree machinery (Phase 2) is reused by 2b · the binding DSL + adapters (Phase 3) are the
bulk and depend on a hardened runtime · tooltips/triggers/polish (Phase 4) improve every prior phase
retroactively.

---

## G. HARD LIMITS (honest; badged, never fabricated)

Genuinely out of reach for a replay, by data or by design — each rendered with an explicit
`unavailable`/`proxy` badge:

| Limit | Surfaces | Why | Replay behaviour |
|---|---|---|---|
| **Designer/template COMBAT STATS** | tank/ship/plane/division designers (#15/16) | runtime-computed from modules, **not stored** | show layout + modules + grid; computed stats badged `unavailable` |
| **Combat battle-DETAIL** (width/terrain/tactics/progress −1..1/modifiers/reinforce/air support) | combat panel (#25) | `combats[]` detail **not cracked** (enumeration is) | show roster + location + duration + lifecycle; detail badged `unavailable` |
| **Per-province exact supply** | supply/logistics map (#6/#23) | **proven wall** — transient, not stored | division-province **proxy**, badged |
| **Air/naval dominance at province/sea-zone** | air/naval map (#7) | only strategic-region granularity stored | **region proxy**, badged |
| **Battle-plan ORDER geometry** (drawn arrows/fronts) | army mgmt (#24), front lines (#8) | orders polymorphic/runtime-cache, **partially cracked** | OOB hierarchy exact; drawn-order geometry → control-adjacency **derived** front, badged |
| **Computed tooltip math** (modifier breakdowns) | tooltips everywhere | the game composes from loc + computed modifiers we don't model | show what we have; grey/omit computed lines, badged |
| **Event TEXT + options** | event popups (#29) | fired-event **ids** stored, not text | needs event-file parse + loc; until then, id + timestamp |
| **scripted_gui dynamic content** | the ~19 nation overlays | content driven by script vars we may not record | bind where vars recorded; else chrome + "not captured" badge |

**Asset gaps (small, already skipped-not-faked):** a few `.dds` edge cases (BC7/RXGB/online assets);
Cyrillic/CJK font pages (we load the Latin set; unsupported codepoints marked). None block a core
view.

---

## H. Risks & the Codex v2 critique (folded in, not softened)

The locked "render everything" scope must **not** make this doc over-optimistic. The Codex v2 pass
exists specifically to keep it honest. Its findings, as constraints already baked into §B/§C/§E:

1. **A finite element/sprite universe is necessary, NOT sufficient (Codex v2-A).** 18 element + 7
   sprite types bound the *renderer*, but `gridBox`/`listBox`/`smoothListBox` are **row factories**
   with engine-populated templates + scroll/selection/virtualisation state; `drag_scroll` focus
   canvases, `dropDownBox`, `browserType`, charts, and `scripted_gui` carry **engine semantics not
   implied by the `.gui` alone.** *Mitigation:* the **special-widget behavior registry** (§B.1/§B.4)
   from Phase 0 — the VM is "renderer core + behavior registry", never "renderer alone."
2. **Pure declaration breaks where data shape ≠ gui tree (Codex v2-B).** Focus graph, combat
   roster, outliner, decision categories, division table all need **imperative view-model
   adapters** first. *Mitigation:* the `snapshot → adapter → declarative binding` pipeline (§C.1);
   the bespoke code is bounded to ~10 adapters (§C.2).
3. **Recorded-state sidesteps availability/effects, but NOT all visibility/layout (Codex v2-C).**
   Focus `offset` branches affect **layout**; conditional `visible` containers + scripted_gui +
   DLC/tag/ideology variants affect **existence**; if the resolved boolean isn't recorded the replay
   must evaluate a subset or **mismatch**. *"Replay avoids 'can I click this?' but still needs 'was
   this visible and where?'."* *Mitigation:* the deliberately-small trigger evaluator (§E) + visible
   degradation + logging (§C.3).
4. **BMFont solves the biggest text risk but has traps (Codex v2-D).** `£icon` tags are sprite
   inserts affecting wrap/baseline; `§colour` spans must not change advances; resolve `$VAR$` before
   measuring; match `.fnt` kerning (incl. its absence); replicate outline/shadow channels; **width
   measurement drives wrap/truncate/clip**; disable canvas smoothing. *Mitigation:* §B.6/§B.7 fold
   all of these in; shared measure+draw routine.
5. **Browser rendering-primitive mismatch** (premultiplied-alpha halos, 9-slice edges, masked-shield
   order, progressbar layering, tinting). *Mitigation:* **visual golden tests** per primitive in
   Phase 0 — fix once, before the sweep.
6. **Data gaps behind an attractive UI** (worst outcome, violates no-fabrication). *Mitigation:* the
   `exact/derived/proxy/unavailable` badge on **every** map mode and panel field, unconditionally.

**Biggest underestimated risk (Codex v2-E, verbatim):** *"The biggest risk is not parsing `.gui` or
drawing sprites. It is reconstructing engine-side view models and widget behavior that are only
implied by C++ code, scripted GUI, and naming conventions. The files describe skins and partial
layout; they do not fully describe population, ordering, conditional visibility, interaction state,
or semantic transforms."* — i.e. the work is in the **adapters + behavior registry**, not the
renderer. This plan front-loads the renderer (Phases 0–1) precisely so that risk lands in the
**bounded, parallelisable** adapter/binding phase (Phase 3), not as an open-ended surprise.

**Codex v2 verdict (verbatim):** *"Generic runtime plus binding DSL is the right backbone for
'render everything', but only if the design admits a substantial library of imperative domain
adapters and special-widget behaviors; a pure GUI VM plus raw declarative bindings will visibly fail
on the important screens."* — **endorsed and designed-to.**

---

## I. Effort sizing (honest: a large, multi-month build)

State it plainly: **this is a large, multi-month effort.** The payoff of the generic-runtime
approach is not that it's small — it's that it makes "everything" **tractable and incrementally
shippable** instead of an N-bespoke-screen tar pit. Relative sizing (S/M/L/XL; assumes the
cracked-data foundation already exists):

| Phase | Scope | Effort |
|---|---|---|
| 0 | loc + BMFont pipeline + asset/primitive hardening + the GUI VM core + topbar proof | **L** (the real foundation; the most intellectually load-bearing) |
| 1 | runtime hardened on all special widgets + shell + scrubber + map modes + selection + state panel | **L** (the row-factory/scroll/drag_scroll widgets are the hard generic engineering) |
| 2 / 2b | research/doctrine tree (engine) → focus tree (adapter + offset eval) | **M / M** (2 builds the tree machinery; 2b reuses it + the trigger eval) |
| 3 | binding DSL + ~10 adapters + the ~2,000-binding sweep + combat panel | **XL** (the bulk by volume — but mechanical, parallelisable, ships view-by-view) |
| 4 | tooltips-everywhere + residual trigger-eval + proxy map modes + events + perf/polish | **L** (touches every surface) |

**Honest read:** the two foundational **L** phases (0, 1) are the *risk*; the **XL** sweep (3) is the
*volume*. A credible first usable milestone (topbar + map + selection + research tree) is the end of
Phase 2; "feels like the game across most screens" is mid-to-late Phase 3; full fidelity + tooltips
is Phase 4. The generic runtime is what lets Phase 3 be a grind rather than a redesign — which is the
whole argument for it over N bespoke screens. **The architecture is the right one *because* it
converts an unbounded problem (clone every screen by hand) into a bounded one (one runtime + a finite
behavior registry + a finite adapter set + a mechanical binding sweep).**

---

## J. Rust reader hot-path (separate parallel track, does NOT block the UI)

Locked decision #3. The **memory-reader scan loop** (`live_capture/` + `memory_reader/`:
`winmem`/`fastscan`/`locate` + `country_index_bridge`) is a candidate for a Rust extension **if and
only if live-capture throughput needs it** — **measure first.** The UI consumes recorded snapshots
and is **entirely decoupled** from how they're produced, so this is a pure backend optimisation:

- **Scope:** port only the hot scan inner-loops (page walk, pattern/anchor scan, arena discovery)
  to Rust (e.g. a `pyo3`/`maturin` extension or a standalone Rust capture binary emitting the same
  snapshot JSON). The Python orchestration, parsers, and snapshot store stay as-is.
- **Trigger:** only if profiling shows the Python+ctypes scan can't keep the desired capture cadence
  on the host. The prior live-coverage work already hit ~3x with `fastscan` in Python; Rust is the
  next lever **if measured necessary**, not by default.
- **Boundary:** read-only, same as today; no new write path; does not touch `replay/` or the UI.
- **Independence:** this track can start, stall, or ship on its own timeline; the UI plan above
  neither waits on it nor changes if it lands.

---

## K. MOD AWARENESS — the UI must reflect the SELECTED mod, not vanilla (user, 2026-06-26)

**Requirement.** Different mods (and vanilla) change the UI — `.gui` layouts, `.gfx`/sprites, `gfx/fonts`, and loc all differ. The app must let the user **select their active mod(s)** and render the UI **as that mod presents it.** Showing vanilla chrome over a replay recorded from a mod that changed it is wrong.

**This is an ASSET-RESOLUTION concern, orthogonal to the engine.** `gui_runtime` / `render` / `bind` are unchanged — they render whatever resolved assets they're handed. Mod-awareness lives entirely in the resolver + a mod-selection config.

- **Active mod-set:** resolution is parameterized by an ORDERED list of mods layered over vanilla per Clausewitz rules (file-level override by path; `replace_path` replaces whole dirs; load order respected; the `.mod` descriptor parsed for `replace_path` / dependencies / supported version). Covers a **single mod AND a playset**. `game_assets/extract.py` already does mod-over-vanilla for one mod → generalize to the ordered mod-set with proper descriptor/load-order handling.
- **Parse against resolved assets:** the whole parse pipeline (`.gui`→JSON, sprite manifest, `core.gfx` font map, loc) runs against the *mod-applied* asset set → a per-mod asset bundle (re-run extract per mod-set offline, or resolve at app load).
- **Mod selection in the app:** point at the game mod dir + pick mods, or load a launcher playset.
- **PER-REPLAY MOD IDENTITY (the safeguard):** record the active mod-set (names + workshop ids + version) in the snapshot provenance **at capture time**, read from the host's the game launcher config (`dlc_load.json` / playset) — NOT from memory. The UI **auto-selects the matching mod-set for a replay and WARNS on mismatch**, so vanilla-UI-over-a-mod-replay is structurally prevented.
- **Binding impact:** element→snapshot-field maps are mostly mod-agnostic (fields are the data); only the element SET is per-`.gui`-version, so a mod that changes a view needs its assets re-resolved + a small binding-map DELTA on the vanilla base. Unknown/added elements degrade gracefully.
- **NEAR-TERM:** verify whether the currently-extracted assets are vanilla or the user's mod; since the user's replays come from their mod, default the resolver to the user's mod + re-parse and confirm the mod's views render. **DONE 2026-06-26:** the prior extract was ALREADY mod-resolved (mod-only `.gfx` like `host_tool_scripted_gfx`/`SOV_custom` + the `replace/bruh_interface.gfx` override were present), but the resolver was single-mod-only and `replace_path`-unaware. Now generalized to the ordered mod-set, driven by `dlc_load.json` (active set = `["mod/test.mod"]` = AMSB-DEV). Re-extracted: 77 core views (the mod's copies of topbar/tech-tree/doctrine-tree/etc. win over vanilla), `fonts.json`, `active_modset.json`. The mod has **no** `replace_path` on `interface/`/`gfx/fonts/`/`localisation/`, so its UI overrides are pure file-level — but the resolver implements `replace_path` generally for other playsets.

---

## Appendix — what the new phase builds on (already in the repo)

- **Data:** ~22 primary domains cracked **live** + validated vs save (`re_scratch/
  LIVE_COVERAGE_LEDGER.md`); canonical shapes in `replay/SCHEMA.md` / `replay/schema.py`; the
  snapshot store + diff/event timeline (`replay/store.py`, `diff_engine.py`); live capture proven
  save-free (`live_capture/`).
- **Assets:** `game_assets/` — `sprites.json` (~26.8k sprites + flags, with frames/9-slice/
  png2/progressbar metadata); `gui/*.json` (**all 77 in-scope core views parsed**; `PRIORITY_GUI`
  is the editable stem list); `fonts.json` (`core.gfx` bitmapfont name→atlas map);
  `active_modset.json` (resolved load order); `portraits.*`; the `.dds`/`.tga`→PNG converter
  (~33k PNGs). **Mod-aware resolver (done 2026-06-26):** `extract.py` resolves an ORDERED active
  mod-set over vanilla (file-level override; **non-recursive** `replace_path`; load order from
  the launcher `dlc_load.json`; single-mod AND playset; `.mod` descriptor parsed for
  name/path/replace_path/deps/version). Verified: the user's prior extract WAS already the mod, but
  single-mod-only — now generalized + driven from `dlc_load.json` (= `["mod/test.mod"]`, AMSB-DEV).
  The mod **overrides 7 in-scope core `.gui`** (topbar, tech-tree, doctrine-tree, battleplantools,
  division/tank designer, unitview, popupwindow) → re-extracted from the mod, not vanilla.
  **Still TODO for v2 runtime:** parse `gfx/fonts/*.fnt` glyph tables + convert glyph atlases, and
  full `$VAR$`/markup loc resolution (the loc *file set* is already resolved mod-over-vanilla).
- **Map:** `map_pipeline/` (province-id raster + state/colour meta) consumed by the native GPU map.
- **UI shell:** `replay_native/` is the active Rust/wgpu runtime. `replay_ui/`
  now contains product parity docs and binding maps only.

> **No personal install path is recorded in this document.** Installed content are located through the
> `game_assets` resolver (vanilla via CLI/default, mod via the mod's `interface/`), and converted
> art (textures, glyph atlases) stays gitignored under the existing copyright/privacy rule.
