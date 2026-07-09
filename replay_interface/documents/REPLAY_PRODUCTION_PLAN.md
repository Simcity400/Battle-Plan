# REPLAY PRODUCTION PLAN — plan of record for the replay layer

**Status: PLAN OF RECORD (2026-07-09).** This document supersedes ad-hoc lane
queues as the authoritative statement of what remains between the current state
of `replay_interface/` and a production replay application. It is maintained by
the replay-layer orchestrator; every merged lane that closes an item updates its
row here in the same commit.

---

## 0. Mission and operating constraints

**Mission.** A production application that opens a `.battleplan` recording of a
multiplayer campaign and plays it back as a full-fidelity, read-only recreation
of the game map and UI — every visual parameter engine-derived (binary RE of
`hoi4.exe` + shipped installed content), no fitted guesses left unlabelled.

**Product model (user directive 2026-07-09): OBSERVER MODE.** The app should
feel like the game's observer mode. Default view = omniscient observer: whole
world visible, no fog of war. The viewer can additionally **observe as a
country** — enter a player country's POV and see its fog of war and only what
it could see (unspotted units hidden, furniture/UI keyed to that country) —
then toggle back to the world view. POV candidates = the session's player
countries (requirements item 4). Data dependency = requirements **item 13**
(per-country province visibility + spotted-unit sets, promoted from deferred
on 2026-07-09); render dependency = the FoW visual law (Phase C8 below).

**Constraint 1 — FULL EXTRACTOR DECOUPLING (user directive 2026-07-09).**
The extractor agent will not have a usable recording any time soon. The replay
layer must therefore be finished **completely independently**:

- Every recording-gated surface (political ownership over time, units, weather,
  UI scalars, diplomacy, …) is developed and numerically gated against
  **synthetic fixture recordings** shaped exactly like `REPLAY_SCHEMA.md` /
  `REPLAY_DATA_REQUIREMENTS.md` prescribe.
- The only contract with the extractor is
  `documents/REPLAY_DATA_REQUIREMENTS.md` (standing task: every newly
  discovered runtime-state dependency is added there in the same commit that
  discovers it).
- **Integration is a separate, final phase** (Phase H) that begins only when
  the user hands over a recording worth using. If the fixtures are faithful to
  the schema, integration is a data-source swap at the `ReplaySource` seam in
  `replay_core`, not a rewrite. **Precondition (Codex review 2026-07-09):**
  the swap promise holds only for **snapshot-flavor** containers — `source.rs`
  fail-closed refuses compact-stream-only recordings (`snapshots: []`) by
  design, requiring per-hour snapshot entries per HOI4REPLAY_CONTAINER_SPEC.md
  or a shared decoder crate from the extraction layer. Requirements **item 1**
  (hourly decoded snapshots in the container) is exactly this precondition;
  fixtures must therefore be packed snapshot-flavor containers too.

**Constraint 2 — fixture-first, seam-respecting design.** All consumers read
world state through `replay_core` (`ReplaySource` → `WorldTimeline` →
typed accessor views such as `owner_by_state()`). No renderer or UI module may
parse recording bytes directly. Fixtures and real recordings are
indistinguishable to consumers.

**Constraint 3 — fail-closed honesty (standing).** `replay_app` blocks any
product path whose law has FIT/UNKNOWN residuals (10 `*_product_path_blocked`
gates in `main.rs` today, plus the blocked CLI switches and env-override
rejections in `main.rs`/startup — G3 enumerates the canonical list). Production ≠ "unblock everything"; production =
close residuals with evidence until the blocks are *earned* off, and keep
fail-closed behavior for genuinely absent data. **No-eyeball rule:** only
pixel/byte parity vs engine-mined frames or oracle gates counts as verified.

**Constraint 4 — delegation.** All substantive work goes to Codex
(Mode A/B per `CODEX_PROTOCOL.md`); the orchestrator verifies every job
independently (own test runs, own pyghidra decompiles for binary claims, diff
scope review) before commit. Codex often cannot commit in linked worktrees
(sandbox excludes the main-repo `.git/worktrees/`); the orchestrator commits.

**Constraint 5 — mod compatibility.** All game-file reads go through
`mod_resolver` keyed by the recording's modset (`MOD_COMPATIBILITY.md`); no
spec may introduce a bare vanilla path join.

---

## 1. Current state (2026-07-09, branch `claude/replay-engine-extraction-18b40f`)

- **Tests:** **1214 passed / 0 failed** across `map_viewer`, `replay_app`,
  `game_ui`, `replay_core`, `terrain_capture` (incl. all pool/oracle/fixture
  gate suites). Was 820 at plan authoring; 1036 after wave-1; +178 from the
  2026-07-09 wave-2/3/4 orchestration (see §1a).
- **Fidelity ledger** (`FIDELITY_LEDGER.md` §1, LEDGER3 recount 2026-07-09 after
  the 8-lane wave + OBJ-1 fold): **90 FILE / 24 CAPTURE / 34 FIT / 49 UNKNOWN**
  map-render parameter rows (was 78/24/38/46 pre-wave). Net wave-2 movement:
  +12 FILE, −4 FIT, +3 UNKNOWN — the FILE gain is rivers/trees/buildings/naval
  file-derivations; UNKNOWN rises because monolithic naval/building-render
  placeholders were decomposed into precisely-scoped residuals. Plus 1 product-UX
  FIT tracked outside the render sum (playback speed table, e-shell). FIT+UNKNOWN =
  the burn-down (§3 there; §3.0 ranks it, top 5 offline-closable).
- **Parity oracle:** 38 vanilla political RenderDoc frames mined
  (zoom ladder); scoreboard mean |err| 0.014–0.03 at h>500, degrading to 0.135
  at h=50 — the close-zoom gap **is** the unrendered dynamic content (units,
  buildings, rails, shadows), i.e. Phase C in this plan.
- **Per-track completion estimate:** static map ~85–90 %; data spine (ingest →
  timeline → render keying) structurally present but thin; dynamic war content
  ~10–15 % (ledger row "Units/counters/rail/arrows": 1 FILE / 6 UNKNOWN); UI
  chrome layout law strong (8 FILE / 2 UNKNOWN) but binding/population barely
  started; playback UX minimal (timeline.rs + scrub tooling). Overall ~60 %.
- **Existing fixture substrate:** python `replay_interface/replay/` package
  (schema.py, store.py, diff_engine.py, snapshots/) — the natural fixture
  *generator*; Rust `replay_core` (source.rs 304 ln, world.rs 955 ln,
  tests/world_timeline.rs 709 ln) — `Leaf<T>` trichotomy,
  `owner_by_state`/`control_by_province` maps, `MapLayerSource`
  mirror-vs-walk. This seam is real and tested; Phase A widens it.
- **Extractor side (context only, not a dependency):** R1 hourly snapshots,
  D56 naval supremacy, D24 weather, D18 cosmetics, D31 air activity, D48
  building damage already landed in `extractor/` per the 2026-07-09
  reconciliation sweep — evidence the schema in REPLAY_DATA_REQUIREMENTS.md is
  being honored, which de-risks fixture fidelity.

## 1a-bis. Progress log — 2026-07-09 orchestration waves 2–4 (full-scope pivot)

Merged (each verified independently: own workspace test runs, pyghidra/​shader-
file citation audits on the RE lanes, diff-scope review, `--ours` on the
transient ledger-totals conflicts pending a fresh recount). Full-workspace gate
**1214 / 0** at the end of wave 4.

- **A5 keying swap DONE** (critical path): political fill / borders / labels /
  furniture key off `WorldTimeline` via `map_viewer/keying.rs`
  (`KeyingTables::from_world_frame`), byte-identical to the static path at t0,
  divergence-gated on flip fixtures. Unblocked all of C/D. Also fixed
  `player_countries` (reads `world.scenario.players`).
- **Static-map residuals:** B1 water tail 8,534→8,537/8,560 + f32-noise-floor
  proof; B1 island loops pinch-handedness POOL-REFUTED (docs-only, RE target
  set — BORD12); B2 labels kerning/advance + atlas-mips CLOSED (FILE),
  emission flood-fill pyghidra-confirmed; B3 buildings material-texture law
  93/93; B4 trees scatter FILE-gated 8/8; B5 rivers palette + width (fabricated
  table retired); B6 shadow-PCF + camera-interp UNKNOWN/FIT→FILE (gates C7
  h=50); C5 rails files-first; cap4 mining CLOSED B12 sea (was FoW), sharpened
  B9/B11 asks.
- **Dynamic war content:** C2 unit placement (unitstacks FILE anchors,
  step-hold); C3 battle-bubble asset chain 17/17; C6 naval supremacy tint;
  weather/snow feed (c-weather, winter-confound fix).
- **UI + playback:** D1/D2 topbar+ideology bindings; D3/D4 outliner+diplomacy
  panels; D6 clock + D7 populate-or-descope sweep; E1/E4/E6 transport /
  validate / observer-POV shell; F4 mid-playback anomaly policy; G3 blocker
  registry; rc-reconcile closed replay_core diplomacy/air/alerts/ratio views.
- **Full-scope pivot (2026-07-09 user directive):** v1 now targets EVERY UI
  panel + ALL map modes (not the informational shell). Backbone landed:
  `UI_COVERAGE_MATRIX.md` (133 panel files / 2,009 windows / 105 in-scope, 3–6
  populated) + `MAPMODE_COVERAGE_MATRIX.md` (16 modes + 6 toggles: 1 done,
  8 law-known, 7 not-started); Phase D + E3 rewritten; new §6a wave plan
  (U1 war-HUD / M1 8 law-known modes / U2 event-decision chrome). This is the
  large remaining body of work — see the two matrices for the residual ledger.

---

## 1a. Progress log — 2026-07-09 orchestration wave 1

Merged to the session branch (each verified independently: own test runs, own
pyghidra re-checks of every binary citation, diff-scope review). Lane reports
archived under `documents/lane_reports/`.

- **A1+A2 DONE** — `replay_fixture_gen` crate (uses the shared
  `extractor/native/snapshot::pack` API) emits 12 scenario + 4 negative packed
  **snapshot-flavor** `.battleplan` fixtures + a manifest; determinism is
  seed-exact (verified byte-identical same-seed, committed == regenerated);
  `replay_core/tests/fixture_conformance.rs` loads every positive through the
  real `ReplaySource` and rejects every negative.
- **A3 skeleton DONE** — `replay_core` now exposes typed `WorldTimeline` views
  for all requirements items (units/session/supremacy/weather/identity/
  diplomacy/topbar/buildings + `visibility_for(pov)`), a `ViewStatus`
  absent-vs-unavailable distinction (A6), and `seek`/`play_to` hooks (A4).
  Additive — zero public items removed. Fixture-gated round-trips (the full A3
  gate) are the next step now that A1/A2 fixtures exist.
- **B1 narrowed** — water border pool 8,500 → **8,534/8,560** via the ported
  post-smoothing f32 emission law (15 instruction citations, all Ghidra-verified
  by me); land held 27,018/27,018. 26 sub-pixel rows + island loops remain.
- **C1 files-first DONE** — `unit_counter_assets.rs` decodes the full counter
  asset chain (mapicons/subuniticons/texticons/fonts via the resolver);
  `UNIT_COUNTER_LAW.md` classifies FILE vs CAPTURE-GATED; CAP6 ask sharpened.
- **C8 FoW law CRACKED** — from the live CAP4 observe-on/off pair (see below):
  carrier = screen-space `ShadowMap` SRV, FoW color/alpha packed `.g/.b`,
  `ApplyFOW` at `pdxmap.shader:388`, producer `fow.fxh` (world-position
  sampled). `fow_law.rs` + parity gate (full-frame mean abs 0.00109). This is
  the marquee result of the day. Residual: hidden-unit culling (B10 follow-up).
- **Capture automation** — scaffold merged (4 fail-closed control commands +
  game-clock/FoW heartbeat, render-thread-marshalled); the engine-RE follow-up
  (camera object / console dispatch / render-view / owner-map roots) is
  **in progress** on an isolated Ghidra project copy. Closes future-session toil.
- **Live capture session (`session_WAVE1_CAP4`)** — war-era player save;
  delivered the **FoW pair (= plan CAP10)** cleanly + partial counters + auto
  zoom rounds; preserved twice (F: full 8.2 GB, C: jewels). Session-harness
  fixes shipped: CAPFIX4/5 (idle-anchored dwell), the `replay_app` non-test
  build fix, the fixture gitignore fix, runbook control-path correction.

---

## 2. The decoupling architecture (how every phase stays extractor-free)

```
                     ┌────────────────────────────────────────────┐
   extractor agent → │  .battleplan (real)     … arrives LATER    │
                     └────────────────┬───────────────────────────┘
                                      │  Phase H swap point
  fixture generator ─→ .battleplan (synthetic, REPLAY_SCHEMA-shaped)
                                      │
                              ReplaySource (replay_core/source.rs)
                                      │
                              WorldTimeline (world.rs)
                        typed views: owner/controller/cosmetic/
                        ideology/war/faction/weather/scalars/units…
                                      │
        ┌──────────────┬──────────────┼───────────────┬────────────┐
   political fill   borders GB    labels/furniture   units/counters  UI chrome
   (COL3 law)       (BORD/GB law) (NAME/FURN law)    (Phase C)       (game_ui)
```

**Fixture-recording rules:**
1. Fixtures are written by a deterministic generator (extend the python
   `replay/` package; seeded, versioned, committed as small binary/JSONL
   fixtures or regenerated in-test) — never hand-edited blobs.
2. Every fixture exercises a *specific consumer contract* and carries a
   scenario name (e.g. `fx_owner_flip_barbarossa`, `fx_cosmetic_raj`,
   `fx_war_matrix_1941`, `fx_topbar_scalars_sweep`).
3. A fixture is only trusted after a **conformance gate at BOTH levels**: the
   same validation code that will later reject a malformed real recording must
   accept every fixture (shared validator, not parallel implementations), and
   fixtures are **packed snapshot-flavor `.battleplan` containers** validated
   at the container level too — manifest/container version, required-features,
   identity consistency, digests, per-entry snapshot hashes per
   HOI4REPLAY_CONTAINER_SPEC.md — with malformed-manifest / bad-hash / wrong
   modset negative fixtures alongside the positive set.
4. When REPLAY_DATA_REQUIREMENTS.md gains a field, the generator + a fixture +
   a conformance test gain it in the same commit.

---

## 3. Phases

Phases A–G are extractor-independent and can overlap; the dependency graph is
in §6. Phase H is deferred by directive.

### Phase A — Data spine: fixture generator + full-schema ingest (FOUNDATION)

Goal: `replay_core` can ingest a schema-complete recording (synthetic) and
serve **every** field in REPLAY_DATA_REQUIREMENTS.md through typed views, with
the trichotomy (value / inherited / unavailable) preserved.

| # | Item | Gate | Status |
|---|---|---|---|
| A1 | Fixture generator v1: extend `replay_interface/replay/` to emit schema-complete synthetic recordings (hourly snapshots + compact deltas, all container roles) | shared-validator conformance pass on every fixture | **DONE 2026-07-09** — `replay_fixture_gen` crate via `snapshot::pack`; snapshot-flavor packed containers; seed-exact determinism verified |
| A2 | Scenario fixture set: owner/controller flips, cosmetic-tag + ideology changes, war/faction matrix evolution, capital moves, weather sweep, **snow/mud accumulator evolution (SnowMudData texture + fade phase per requirements item 6)**, topbar scalar sweep, division movement, naval/air activity, building damage, **per-POV visibility evolution (FoW flips + unit spot/lose events)**, **DLC-set variation (NAME1b DLC-gated cosmetics) + localisation-language selection** | one named fixture per consumer contract; enumerated in a manifest | **DONE 2026-07-09** — 12 positive scenario fixtures (incl. `fx_pov_visibility`, `fx_snow_mud_accumulator`, `fx_dlc_language_variation`) + 4 negative in `replay_interface/fixtures/manifest.json` |
| A3 | `replay_core` ingest of every requirements item (1–10 + 7b/8b/9b/**13**): parse, index by time, expose typed views (incl. `visibility_for(pov_tag)` province view + spotted-units view) | round-trip gates: generator value == view value at every probed hour, incl. delta-reconstruction == snapshot cross-check | **SKELETON DONE 2026-07-09** — typed views for all items + `ViewStatus` (A6) + `seek`/`play_to` (A4) landed, additive; **remaining: wire the views to A1/A2 packed fixtures for the round-trip gate** (inline-synthetic tests only so far) |
| A4 | Time semantics: hourly resolution, seek/scrub to arbitrary hour, forward/backward, mid-file start (snapshot + delta replay) | seek(h) == linear-play-to(h) equality gate on all fixtures | PARTIAL (timeline.rs scrub exists for captured moments) |
| A5 | Renderer keying swap: political fill/borders/labels/furniture read owner+controller+cosmetic from WorldTimeline instead of static history parse | pixel-identical output at t0 vs current static path on the 1936 baseline; then divergence gates on flip fixtures (expected-diff oracles) | OPEN — **this is the single highest-leverage item in the plan** |
| A6 | Fail-closed data absence: every view distinguishes "field absent from recording" and renderers surface it per Constraint 3 | absence-injection fixture gates | PARTIAL (Leaf trichotomy exists) |

### Phase B — Static-map residual closure (burn-down of FIT/UNKNOWN)

Goal: the FIDELITY_LEDGER §3 top-10 items that are *offline-crackable* go to
FILE/CAPTURE-LAW; capture-gated ones get their capture executed (§4) and then
closed. Product blocks earned off one by one.

Offline (Codex jobs, no user session needed):
| # | Item | Gate | Status |
|---|---|---|---|
| B1 | 60 water-strip residuals (sea|sea smoothing tail) + ~277 island loops + fade interior curve → unblock SeaRegion + `--borders-from-files` | pool gate 8,560/8,560 water, 27,018 land held; loops traced | OPEN (BORD10 got 8,534/8,560; 26 water rows remain) |
| B2 | Label residuals: component-emission limits, kerning table, anchor rule; glyph atlas mips | oracle63 float-exact held + new held-out capture rows | OPEN (arc/size/tracking laws closed) |
| B3 | Building `.mesh` importer or map_objects_pass derivation → replace preview blocks | byte parity vs captured object pass | OPEN |
| B4 | Tree scatter anchor/rotation/model-selection + season CB1 from files | parity vs live_trees_measure corpus | OPEN |
| B5 | River class-3 width ramp + palette semantics | defines/shader citation or capture | OPEN |
| B6 | Remaining ledger tail (#16–21): moon `.z`, height scale, PCF/shadow path, occupation stripes, camera interpolation shape | per-row cited closure | OPEN, lower priority |

Capture-dependent (blocked on §4 sessions, then a closing Codex job each):
B7 terrain vSnap frustum (needs §4-CAP1) → 170/176 target; B8 night `FoW.w`
path + night `.x/.yzw(hour)` (§4-CAP3); B9 unclamped exposure branch (§4-CAP4);
B10 label owner-vs-controller keying + per-label `Transp_OffsetX.x` (§4-CAP5);
B11 VP color columns 1/3 confirmation (§4-CAP2); B12 no-FOW sea base look;
B13 snow/mud — **recording-fed** (SNOW_LAW §6), so its product path rides
Phase A fixtures, not a capture.

### Phase C — Dynamic war content (the biggest remaining body)

Goal: everything that makes a *war* replay legible — currently 1 FILE /
6 UNKNOWN. All of it is fixture-drivable (positions/strengths come from the
recording); the RE work is the *rendering law*, which is engine-derived and
extractor-independent.

| # | Item | Law source | Gate | Status |
|---|---|---|---|---|
| C1 | Unit counters (NATO/sprite): quad geometry, atlas, strength/org bars, stacking/aggregation-by-zoom | hoi4.exe RE + interface gfx files + one live counter capture (§4-CAP6) | pixel parity vs captured counter frame; fixture-driven placement gates | **FILES-FIRST DONE 2026-07-09** (`unit_counter_assets.rs` + `UNIT_COUNTER_LAW.md`: all FILE rows decoded/gated); **remaining: CAPTURE-GATED rows** (screen-space size/anchor/stacking/selection) need CAP6 — CAP4's `counters_close` round was budget-truncated |
| C2 | Division/fleet/wing positions over time served from recording; interpolation between hourly samples matches engine hourly snap (no invented smoothing without evidence) | REPLAY_DATA_REQUIREMENTS units items + fixtures | seek==play equality; placement matches fixture ground truth | OPEN (schema listed) |
| C3 | Battle bubbles / combat indicators | RE + capture | parity gate | **FILES-FIRST DONE 2026-07-09** (`combat_indicator_assets.rs` + `COMBAT_INDICATOR_LAW.md`: land + naval on-map combat layouts, 17 sprites N/N resolved, 2 progress bars, 5 GUI containers, font — all FILE rows decoded/gated); recording fields ADDED (REPLAY_DATA_REQUIREMENTS item 3 land display + item 8 naval combats; EXTRACTION_NEEDS A16). **Remaining: CAPTURE-GATED rows** (screen-space size/anchor/zoom-scale, combat→frame/colour selection, progress-bar fill, direction rotation) need CAP11 |
| C4 | Front lines + movement/plan arrows (or explicit descope for v1 if not in recording — arrows are player-plan state; **decision needed, tracked in REPLAY_DATA_REQUIREMENTS**) | RE + requirements | parity or descope note | OPEN |
| C5 | Rail/supply overlay | RE + defines | parity gate | **FILES-FIRST DONE 2026-07-09** (`map_viewer/src/rail_files.rs` + FIDELITY_LEDGER §2.18): railways.txt/supply_nodes.txt parse (reusing `map_static_data`), world-space polyline geometry from province centroids + `RAILWAY_Y_OFFSET`, and the `railroad.shader`+`00_graphics.lua` render law (all FILE), gated on shipped files (909 railways, 727 hubs, exact level histogram, full referential integrity, in-bounds). **Remaining CAPTURE/RE rows:** ribbon world width per level / UV tiling / inter-centroid curve / hub icon frame (CAP9 rail VB or ribbon-builder RE); **built-rail state stays A8** (extractor-side) |
| C6 | Sea-zone naval-supremacy tint (recording R6/D56 shaped) | GB-field law + fixtures | parity vs captured supremacy frame | OPEN |
| C7 | Close-zoom 3D pass completion (units/buildings/rails/shadows) measured by the existing oracle: h=50 scoreboard mean |err| 0.135 → ≤0.03 | all of C1–C6 + B3/B4 + **B6 shadow/PCF path** + an explicit **particles decision** (descope candidate — if descoped, restate the h=50 target on particle-free frames or a documented delta) | RenderDoc-oracle scoreboard at low h | OPEN — **this is the definition-of-done metric for Phase C** |
| C8 | **FoW visual law (observer country-POV)**: terrain shader transform is now mined: CAP4 proves the carrier is the screen-space `ShadowMap` SRV, with FoW color/alpha packed in `.g/.b`; shipped `fow.fxh` / `pdxmap.shader` define the final blend and producer packing. Remaining scope: regenerate the producer inputs (`IntelTex`/`ExtraFOW`/visibility state) for arbitrary camera/time and mine hidden-unit culling from a labelled counter/UI pair. Scene-CB0 FoW gate did not change in CAP4, so C8 is texture/draw-state driven for this pair. | installed content first (shader sources) + CAP4 pair; follow-up §4-CAP10 for producer/counter residuals | `map_viewer` FoW gate: identical terrain draw sets, only `ShadowMap` terrain SRV differs; clear-region parity plus adversarial mismatch; future POV-fixture unit-hiding gates | PARTIAL — carrier + shader law implemented/gated in tooling (`fow_law.rs`); product path still blocked by producer-input + hidden-unit evidence |

### Phase D — UI completion: EVERY panel at full fidelity (game_ui)

**Scope (user directive 2026-07-09): v1 renders EVERY UI panel, not the
informational-shell subset.** All 133 shipped `.gui` panel files (130 base +
3 DLC music), ~2,009 `containerWindowType` window definitions — every major
view, HUD element, popup, event/decision chrome, tooltip, and country
scripted-GUI — rendered read-only and populated from the timeline. Every
interactive control (designers, deployment drag, nudger, sliders) renders
**inert / read-only snapshot**; it is in scope, not descoped, per the directive.
Frontend/menu/lobby/dev panels (25) + DLC music (3) are the only v1 descopes
(they never appear during an in-game replay).

**Residual ledger:** `documents/UI_COVERAGE_MATRIX.md` is the authoritative
per-panel coverage matrix (category, source `.gui`, status, data dependency,
capture/RE need) and the definition-of-remaining for this phase. It counts:
105 in-scope panels, of which **3 are POPULATED today** (topbar D1/D2, outliner
D3, diplomacy D4) and 102 need data-binding beyond the (solid) layout law; it
also enumerates **14 new recording-data dependencies** (§9 there) to fold into
REPLAY_DATA_REQUIREMENTS.md.

Goal: the full read-only UI rendered and *populated from the timeline*. Layout
law is strong (GetGlobalPosition verbatim, listbox/gridbox/scrollbar law); the
gap is bindings + population across all 105 panels + the remaining widgets. The
rows below (D1–D8) track the FIRST-WAVE panels; the matrix tracks the long tail.
See the **UI derivation wave plan** in §6a for how the long tail is batched.

| # | Item | Gate | Status |
|---|---|---|---|
| D1 | Topbar bindings: politics/stability/war-support/factories/convoys/fuel/nukes/tension… bound to WorldTimeline scalar views (9/9b) | fixture sweep renders every scalar; layout unchanged vs .gui law gates | OPEN (layout law done) |
| D2 | Ruling-party/ideology-driven texts + flag/portrait keying (7b, LAB4 `<TAG>_<ideology>` localisation) | fixture ideology-flip changes rendered text correctly | OPEN |
| D3 | Outliner (army/navy/air lists) populated from unit views | listbox law gates + fixture population counts | OPEN |
| D4 | Country/diplomacy panels (read-only): relations, wars, factions from 8/8b views | fixture war-matrix renders correct rows | OPEN |
| D5 | Scrollbar thumb-travel law (UNTRACED, fail-closed) + any remaining widget primitives (checkbox/tab states as static chrome) | RE citation or capture (§4-CAP7 open-panel frame) | OPEN |
| D6 | Date/speed clock — replay-native (shows recording time; speed = playback speed) | unit gates | **DONE 2026-07-09 (D-UI lane)** — FILE clock format (`interface/topbar.gui` DateText placeholder `"7:00 29, September 1939"` + `core_l_english.yml` months) in `topbar::format_topbar_datetime`/`binding::format_topbar_date{,_english}`; `binding::clock_binding_from_frame` binds the recording date + playback `speed_step` (mapped onto the five topbar buttons); wired into `replay_app` overlay `hour_label` (raw-date GAP closed). Gates: `binding::tests::topbar_date_*`, `ui_completion_binding::clock_binding_matches_recording_date_and_playback_speed` |
| D7 | Remaining UI production surface per UI_LAYOUT_LAW §6.3: map-mode bar active state, tooltip content bindings, scripted-GUI visibility rules, and population of the major views (rows/progress/counts) — each view either implemented or **explicitly descoped for v1 in this table** | per-view fixture population gates; layout law gates held | **DONE 2026-07-09 (D-UI lane) — populate-or-descope sweep complete, see D7 table below** |
| D8 | Localisation-language + DLC-gated identity binding: label/UI text keyed by the recording's language + DLC set (requirements items 4/9; ledger language-selection UNKNOWN) | A2 DLC/language fixtures render correct variants | OPEN |

#### D7 populate-or-descope sweep (UI_LAYOUT_LAW §6.3/§6.4 surfaces)

Every §6.3/§6.4 UI surface is accounted for below: **implemented** = a
fixture-gated `game_ui::binding` population seam exists; **descoped** = a named
v1-descope row (what is missing, why acceptable for v1, what closes it). The
static layout of every surface already composes (probe: all 453 `.gui`, 0
failures; 8 major views composited) — this table is only about *dynamic
population*.

| §6.4 surface | Disposition | Binding / evidence, or descope rationale + closer |
|---|---|---|
| Topbar chrome — scalars | **IMPLEMENTED (D1)** | `binding::topbar_binding_from_frame` (scalars raw; formatting UNKNOWN except date) — `topbar_binding.rs` sweep gate |
| Topbar chrome — flag/name/ideology | **IMPLEMENTED (D2)** | `binding::{topbar_flag_key_candidates,country_name_key_candidates}` — `topbar_binding.rs` ideology-flip gate |
| Topbar chrome — date/speed clock | **IMPLEMENTED (D6)** | `binding::clock_binding_from_frame` — FILE date format; playback speed step. `ui_completion_binding` clock gate |
| Outliner (army/navy/air rows) | **IMPLEMENTED (D3)** | `binding::outliner_binding_from_frame` — `outliner_binding.rs` population gate |
| Country/diplomacy panel | **IMPLEMENTED (D4)** | `binding::diplomacy_panel_from_frame` — `diplomacy_binding.rs` war/faction gate |
| Tooltip frame — data (state owner/controller) | **IMPLEMENTED (D7)** | `binding::state_tooltip_from_frame` — owner/controller from the timeline, fail-closed. `ui_completion_binding` tooltip flip gate |
| Tooltip frame — localised sentence + breakdown | **DESCOPE (v1)** | *Missing:* the composed localised tooltip text (e.g. "Owned by X, controlled by Y", building/supply breakdowns). *Why acceptable:* computed-not-stored (UI_LAYOUT_LAW §6.4 gap-class 2) — the engine builds these at runtime from scripted data functions; the underlying data (owner/controller) IS surfaced above. *Closes it:* a bounded data-function/`[expr]` evaluator + the loc format grammar, or an unavailable badge. |
| Minimap / map-mode bar — active state | **IMPLEMENTED (D7, viewer-state)** | `binding::observer_default_mapmode` — political (`game_map_mode_country`, button 1); the map-mode bar highlights the viewer's mode. Gate `binding::tests::observer_default_mapmode_is_political_button_one` |
| Minimap / map-mode bar — *recorded* active mode | **DESCOPE (v1)** | *Missing:* a per-tick recorded active map mode. *Why acceptable:* the active mode is player UI state, NOT world state (UI_LAYOUT_LAW §6.2 "active mode/tick = runtime"); the observer picks the mode, so there is nothing to bind. *Closes it:* only if a recording ever captures the player's selected mode (not in REPLAY_DATA_REQUIREMENTS). |
| Menubar | **DESCOPE (v1)** | *Missing:* nothing dynamic. *Why acceptable:* every menubar control is a mutating action; a read-only replay renders it as inert static chrome (UI_LAYOUT_LAW §6.4 gap-class 3 — intentional read-only divergence). *Closes it:* n/a (by design). |
| 8 major views — diplomacy content | **PARTIAL (D4 seam exists; composition deferred)** | The timeline data (relations/wars/factions) is bound by `binding::diplomacy_panel_from_frame`; wiring those rows into the `major_views.rs` compositor (which today overlays static sample text) is a compositor task deferred to Phase E product wiring. |
| 8 major views — production/construction/research/trade/focus/division-designer content | **DESCOPE (v1)** | *Missing:* rows/progress/counts for these views. *Why acceptable:* recording-gated + computed-not-stored (UI_LAYOUT_LAW §6.4) — production lines, research slots, trade rows, focus progress, division designer stats are not in REPLAY_DATA_REQUIREMENTS' current schema and are largely player-facing derived state. *Closes it:* extractor feeds for the specific view + a value-format law; each view's static chrome already composes. |
| Listbox/gridbox/scrollbar — row population | **DESCOPE (v1)** | *Missing:* runtime item rows + population order + autoscroll. *Why acceptable:* missing live replay state; the declared row/slot geometry law is already FILE (UI_LAYOUT_LAW §6.3, `widget_layout.rs`). *Closes it:* per-panel recording feeds + the open-panel capture (CAP7) for the pixel referee. |
| Scrollbar thumb travel | **DESCOPE (v1) — capture-gated (= D5)** | *Missing:* value→thumb-position law. *Why acceptable:* UNTRACED, fails closed (UI_LAYOUT_LAW §6.3); no thumb is fabricated from min/max/start. *Closes it:* §4-CAP7 open-scrollable-panel capture. |
| Scripted-GUI visibility rules | **DESCOPE (v1)** | *Missing:* per-element visibility from scripted triggers. *Why acceptable:* computed-not-stored (UI_LAYOUT_LAW §6.4 gap-class 2); a read-only replay cannot evaluate live scripted GUI triggers without a bounded evaluator. *Closes it:* a bounded trigger evaluator over recorded scope state, or an unavailable badge per element. |

### Phase E — Playback UX (the product shell)

| # | Item | Gate | Status |
|---|---|---|---|
| E1 | Transport controls: play/pause, speed steps, seek bar with date ticks, jump-to-date | seek==play gate reused from A4; input-latency budget | OPEN (scrub tooling exists, product-blocked) |
| E2 | Free camera with engine pan/zoom law (CAMDEF1 clamps landed; interpolation shape pending B6) | camera law gates | PARTIAL |
| E3 | Map-mode switching — **ALL modes (user scope 2026-07-09)**: 16 selectable modes (12 toolbar clicksound-identified + 4 engine hard-coded: states/diplomacy/players/ideology) + 6 display toggles (day-night/player-counters/counter-color/allied-plans/fog/radar); colors computed replay-side from inputs (NEVER recorded; standing override). Residual ledger = `documents/MAPMODE_COVERAGE_MATRIX.md` (per-mode fill-law source, data dep, status): 1 done (political), 8 law-known (naval/terrain/resource/factions/states/diplomacy/players/ideology), 7 not-started (air/operatives/supply/resistance/compliance/infrastructure/raids). Mods add scripted modes → generic scripted-layer composer (type/color/text/thickness per `common/map_modes/documentation.md`). | per-mode parity gates | PARTIAL (political done; naval/FoW law-known; 7 modes not-started) |
| E4 | Recording open/validate UX: schema validation errors, modset mismatch surfacing (mod_resolver), version handshake | malformed-fixture rejection matrix | OPEN |
| E5 | Session ergonomics: window resize, monitor DPI, save-position — plain Win32 hygiene | smoke gates | OPEN |
| E6 | **Observer POV mode (core UX, 2026-07-09 directive)**: default = omniscient world view (no FoW); country picker over the session's player countries; entering a POV switches on the FoW layer (C8) from `visibility_for(pov)`, hides unspotted units, re-keys viewed-tag-dependent surfaces (VP-icon diplomatic columns already parameterized on `viewed_tag` per FURN-REL law; topbar/outliner re-bind to the POV country); one-key toggle back to world view | POV fixture: same hour renders world-view vs POV-view with expected-diff oracle (fogged provinces darken, unspotted units absent, VP columns flip); toggle is stateless/idempotent | OPEN |

### Phase F — Performance & hardening

| # | Item | Gate | Status |
|---|---|---|---|
| F1 | Playback frame budget on the weak-PC mandate: full political map + labels + furniture + counters at interactive rates; measure, then optimize (no speculative work). **Budget: ≤33 ms/frame (30 FPS floor) at 1920×1080, p99 over a scripted 5-minute play+scrub run, on a named weak-PC reference config (pick the reference machine/GPU class with the user before F1 starts; the number is the gate, the reference is the contract)** | frame-time budget gate in headless bench | OPEN |
| F2 | Timeline memory: bounded working set for multi-year hourly recordings (snapshot spine + delta windows, LRU) | synthetic 3-year fixture under memory ceiling | OPEN |
| F3 | Long-run soak: 8-hour scrub/play loop, zero leaks/device-removed | soak harness | OPEN |
| F4 | Crash/error policy: fail-closed startup diagnostics already exist; extend to mid-playback data anomalies (log + hold-last-good, never invent) | anomaly-injection fixtures | **DONE 2026-07-09** — `replay_app/src/anomaly.rs` policy at the `replay_core` consumer seam (read-only): classifies the five mid-stream classes (MissingDomain Absent-after-Present, TypeFlip, OutOfRange, HourGap, IdentityMutation), logs one structured diagnostic per episode, holds last-good VISIBLY MARKED, and reports gap hours as unavailable (never interpolated). 5 seeded `anomaly/` fixtures in a separate manifest section (they OPEN + degrade; the negative/ reject matrix is untouched). Gates: `replay_app` 178 pass, `replay_core` 30 pass, regen byte-identical. No `replay_core` change needed (see documents/lane_reports/f4-anomaly.md). |

### Phase G — Acceptance & release QA

| # | Item | Gate |
|---|---|---|
| G1 | Full-ladder pixel parity vs the 38-frame RenderDoc oracle: mean |err| ≤0.03 at ALL heights incl. h=50 (non-snow frames per the winter confound) | scoreboard |
| G2 | Fixture end-to-end: open synthetic 1936–1945 recording → play → scrub → **enter a country POV → verify FoW/unit-hiding → toggle back** → every surface updates; recorded as a scripted headless run with frame hashes | e2e hash gate |
| G3 | Fidelity ledger: zero unlabelled FIT in any product-path parameter; every remaining UNKNOWN either product-blocked or explicitly descoped in this doc | ledger audit. **Blocker-list machinery DONE 2026-07-09** — `replay_app/src/blockers.rs` is the canonical registry (29 surfaces: 23 blocked CLI switches + 6 env-override rejections; 10 carry a `*_product_path_blocked` gate). All `main.rs` gates route their fail-closed message through the registry (pure indirection, no behavior change); `--list-blockers` emits it deterministically; a coverage test asserts the registry covers every `*_product_path_blocked` symbol so no block can be added without a row (plus a `--list-blockers` snapshot test). Enumerated table + closure refs in `lane/g3-blockers` LANE_REPORT.md. **Remaining G3:** the ledger FIT/UNKNOWN audit itself. |
| G4 | Full test suite green + Codex adversarial release review (read-only, xhigh) | 100 % pass + review disposition log |

### Phase H — Integration (DEFERRED until a real recording exists)

Explicitly out of scope until the user delivers a worthwhile `.battleplan`:
swap `ReplaySource` input to the real file; run the same conformance validator;
diff real-vs-fixture field coverage; reconcile gaps back into
REPLAY_DATA_REQUIREMENTS.md; re-run G1–G4 on the real recording. If Phases A–G
held the seam, H is days, not weeks.

---

## 4. Consolidated capture-session asks (user-run live sessions; batch them)

Each ask is one frame/short session; several can share a session. These gate
Phase B/C/D closures but nothing in Phase A/E/F.

**Status note (2026-07-09):** the `session_WAVE1_CAP4` live session delivered
**CAP10 (FoW pair) — DONE** and partially addressed CAP2/CAP6 (see rows). Once
the capture-automation RE lane lands, CAP1/CAP3/CAP5/CAP7-ladder become
near-zero-touch (scripted camera/console), so batch the rest into one automated
session.

| # | Ask | Closes |
|---|---|---|
| CAP1 | Single-frame, NO-pan, wide-frustum mixed-LOD terrain capture recording render-view side planes / tessellation visibility flags (signed-byte owner map) | B7: vSnap 146→170/176. **Capture tooling landed** (`render_view_capture` moment, fail-closed pending the RE lane's render-view/owner-map roots) |
| CAP2 | Player-POV political frame with enemy-owned + same-faction VPs on screen, **capturing the on-map furniture / VP-icon draw pass** (draws binding `GFX_onmap_victorypoints_strip` / `onmap_victorypoints_strip.dds`), with the sprite frame index (or strip UV sub-rect) per VP draw | B11: VP diplomatic color columns 1/3. **NOT CLOSABLE FROM CAP4 (mined 2026-07-09)** — the CAP4 `fow_on_pov_front`/`fow_off` rounds captured ONLY the map terrain/water/rivers/trees passes; NO VP-icon / map-furniture / UI / sprite draw pass exists anywhere in the corpus (SRV declared names are all terrain/water/tree/river maps). The FURN-REL law (`map_furniture.rs`: only columns 0/2/4 reachable, 1/3 unreached) stays static-RE-only. The re-capture must include the icon pass AND span at-war/self/same-faction/neutral VP owners so columns 0/2/4 each appear and 1/3 are confirmed absent, byte-exact. |
| CAP3 | Game-speed night capture with `FoW.w≠0` + multi-hour night sweep | B8: night look + `.x/.yzw(hour)` |
| CAP4 | Bright close-up `restore_pass` frame **whose scene geometric-mean HDR luminance exceeds 0.5**, capturing the `restore_pass` bundle (MainScene + AverageLuminanceTexture) | B9: unclamped exposure branch. **NOT CLOSABLE FROM `session_WAVE1_CAP4` (mined 2026-07-09)** — the CAP4 rounds captured NO `restore_pass` bundle at all, and reducing the last captured map-pass HDR RT (trees `om_post_pass`, the MainScene proxy) through the ported chain gives geo-mean **0.237** in BOTH rounds (fow_off 0.23671 / fow_on 0.23654), which CLAMPS to the 0.5 floor (only ~0.35% of pixels exceed 0.5 linear, max ~0.62). The unclamped branch was NOT exercised. Gate `luminance_law::gate_wave1_cap4_scene_clamps_to_floor_unclamped_branch_unexercised`. Re-capture needs a frame that FILLS the view with high-albedo terrain (snow/desert) at close zoom, minimal dark ocean/shadow, so `exp(mean(log(luminance)))` > 0.5 — AND must dump the `restore_pass` bundle. |
| CAP5 | Multi-zoom label captures over occupied territory (owner≠controller) | B10: label keying + per-label alpha |
| CAP6 | Close-zoom frame with land units + counters visible (and a naval stack) | C1: counter geometry/atlas ground truth. **PARTIAL** — CAP4 `counters_close` round budget-truncated (101 draws); re-capture with a higher payload budget |
| CAP7 | Frame with an open scrollable panel (thumb positions at known content sizes) | D5: scrollbar thumb travel |
| CAP8 | No-FOW sea frame | B12: sea base look referee. **CLEAN REFEREE ESTABLISHED from CAP4 `fow_off_observer_front` (mined 2026-07-09)** — that no-FOW round's water pass has a fog-clean sea (`ShadowMap` `FogAlphaFactor` byte-mean 0.48/255 ≈ 0 over the 1,009,749-px sea mask); clean open-sea base HDR = (0.1007, 0.1314, 0.1919) linear, dark reflective blue, cross-checked vs `live_PAN2`. The pale grey-green political sea was FOW contamination — no separate pale-sea law exists (see FIDELITY_LEDGER "Political sea BASE look"). RESIDUAL: byte-exact pixel parity of our pdxwater output vs this referee needs the water D3D11 pass re-rendered at matching state (a render op, not offline mining). |
| CAP9 | Tree instance buffer + cities_pass (standing ledger asks) | B3/B4 |
| CAP10 | Follow-up FoW pair: same-camera/same-clock observe-as-country (FoW ON) vs observer world view (FoW OFF), with ShadowMap producer inputs or pre-FoW intermediate captured, and a labelled unit/counter/UI draw pass containing hidden enemy units | C8 residuals: producer-input parity + hidden-unit culling referee |
| CAP11 | Close-zoom frame with an active **land battle bubble** on screen (`land_combat_mapicon`) — ideally one land combat with a progress bar mid-fill, plus if possible a shore-bombarded combat and an air-superiority modifier; and one **naval combat bubble** (`naval_combat_mapicon` / results icon) at a sea zone. Batches with CAP6 (counters) — same close-zoom war session. | C3: battle-bubble screen-space size/anchor/zoom-scale, combat→frame/colour selection, progress-bar fill mapping, direction-indicator rotation law |

---

## 5. Standing extractor-requirements process (unchanged by decoupling)

`REPLAY_DATA_REQUIREMENTS.md` remains the single handoff artifact. Rules:
every lane/job that discovers a new runtime-state dependency adds it (plus an
EXTRACTION_NEEDS §A row) in the same commit; Codex job prompts for render/UI
work carry this rule verbatim; the fixture generator (A1) mirrors the doc, so
schema drift is caught by the shared validator, not by integration day.

## 6. Dependency graph / execution order

- **A1→A2→A3→A5** is the critical path; nothing in C/D/E ships without A.
  A4/A6 ride alongside A3.
- B offline items (B1–B6) are parallel Codex lanes, independent of A.
- B capture items (B7–B12) wait on §4 sessions — schedule the session early;
  the closing jobs are quick once evidence exists.
- C needs A3 (unit views) + its own RE lanes; C7 (the h=50 oracle metric)
  additionally needs B3/B4 + B6 (shadow/PCF) + the particles decision.
- D needs A3 (scalar/diplomacy/ideology views); D5 needs §4-CAP7.
- E6 (observer POV) needs A3 (`visibility_for` views) + C8 (FoW law); the UX
  shell and world-view default need neither and can land first.
- E needs A4 (seek) and is otherwise independent; F needs A+C volume to be
  meaningful; G last; H deferred.
- Suggested wave 1 (now): A1+A2 (one Codex lane), A3 **design + ingest
  skeleton only** (second lane — its fixture-gated round-trip implementation
  waits for A1/A2 output, per the critical path), B1 water residuals (third
  lane), C1 counter RE prep from installed content (fourth lane) — all disjoint
  files.

## 6a. UI + map-mode derivation wave plan (Phase D/E3 batching)

The 105 in-scope panels (`UI_COVERAGE_MATRIX.md`) and 16 map modes
(`MAPMODE_COVERAGE_MATRIX.md`) are batched into lanes by **shared law**, so one
lane closes N similar surfaces, ordered by **replay-war value** (what fires
during a war replay first). Every wave depends on Phase A3 typed views for its
data class; layout is already composable for all of them. Waves are parallel
where files are disjoint.

**Wave U1 — war-legibility HUD + popups (highest value, mostly item 3/8/8b data).**
Panels that appear/fire during a war: `mapicons` (VP + unit counters, ties to
Phase C1), `unitview`, `divisions_summary`, `landcombat`, `navalcombat`,
`navalcombatresultswindow`, `combatlogview`, `waroverview`, `alerts`,
`worldtensionwindowpopup`. Shared law: unit/combat/relation views (already
skeletoned in A3) + counter render (C1). Finishes D3-adjacent HUD. **Do first.**
- **U1a DONE (2026-07-09)** — on-map unit-counter **CONTENT** bound
  (`game_ui::binding_counters`): per-unit name / str-org bar fractions / NATO
  symbol key / owner-colour key / A15 badge states, fail-closed, from
  `replay_core` unit views. Gated `cargo test -p game_ui` (`counter_binding` 3
  fixture gates + 6 unit tests). Ledger §2.18a; gaps → item 3 / A15 (org-bar
  max, non-served badges, naval TF name). Counter GEOMETRY/stacking/bar-render
  stays C1/CAP6.

**Wave U2 — event/decision chrome (data-driven, tiny panel count, huge coverage).**
`eventwindow`, `popupwindow`, `countrydecisionview`. One binding lane renders
hundreds of event/decision instances from the fired-event stream (new dep 13) +
decision state (new dep 11) + localisation join (item 9). Very high coverage per
unit effort. **Do second.**

**Wave U3 — country economy/politics majors (shared listbox/gridbox law).**
`countrypoliticsview`, `countryproductionlineview`, `countryconstructionsview`,
`countrytradeview`, `equipmentoverview`, `countrystateview`. Shared law: gridbox/
listbox row population + progress bars. Needs new deps 3/4/7 + item 10.

**Wave U4 — research + focus trees (shared tree-layout law).**
`countrytechnologyview`, `countrytechtreeview`, `countrydoctrinetreeview`,
`nationalfocusview`. One tree-layout + prereq-line law lane covers all four; new
deps 1/2 (research/focus state).

**Wave U5 — military rosters + leaders (shared roster/trait law).**
`countryarmyview`, `naviesview`, `strategicairview`, `airgroupview`,
`countryofficercorpview`, `unitleaderwindow`, `navyleaderview`, `leadergroups`,
`acesview`, `armytraittreewindow`, `navytraittreewindow`, `traittreewindow_common`.
Shared law: roster listbox + trait tree. New dep 9 (leader roster/traits).

**Wave U6 — designers (interactive→read-only snapshot, shared template law).**
`divisiondesignerview`, `equipmentdesignerview`, `equipmentupgradedesignerwindow`,
`tank_designer_view`, `plane_designer_view`. One read-only-snapshot lane; new dep 5
(templates/variants).

**Wave U7 — intel + occupation + logistics (LaR/specialist).**
`countryintelligenceagencyview`, `countryintelledger`, `operationoverview`,
`operative`, `operativeleader`, `countryoccupationview`, `garrisonlogview`,
`countrylogisticsview`, `powerbalanceview`. New deps 6/8/10 + items 10/11. These
also unblock the resistance/compliance/supply/operatives/infrastructure MAP MODES
(shared occupation/supply/intel data), so pair U7 with map-mode wave M2.

**Wave U8 — country scripted-GUI (generic composer, mod-critical).**
The 23 `*_scripted_gui.gui` national-mechanic panels. Do NOT hand-build 23; build
ONE generic scripted-GUI composer driven by recorded scripted-variable state (new
dep 14), so vanilla + arbitrary mod scripted-GUIs render from the same lane.

**Wave U9 — remaining HUD/popup tail + tooltips.** `command_character_view`,
`abilitylist`, `battleplantools` (inert), `findview` (read-only), `rightclickmenu`
(inert), music/social (inert), `airwingdetails`, `carrierairwingcompositionwindow`,
`insrevolutionwindow`, `naval_losses`, `destroyedshipswindow`, `detailedwindows`,
`railway_gun`, `advisorroleselectionview`, `insigniaselection`,
`airwingreorganization` + the dynamic-breakdown tooltip evaluator.

**Map-mode waves (parallel to U-waves):**
- **M0 (done):** political (COL3). **M1 (law-known → gate):** naval (C6), terrain,
  resource, factions, states, diplomacy, players, ideology — 8 modes whose fill
  law is file-derivable/ported; one lane per shared primitive (engine layer types)
  binds + gates them. **M1c IMPL 2026-07-09:** `mapmode_diplomacy.rs` (relation
  classification extends FURN-REL to whole-country fill, per-POV; fill RGB
  UNKNOWN/fail-closed — VP strip DDS is grayscale, no define) + `mapmode_factions.rs`
  (membership fold; non-member = `FACTIONS_COLOR_NOT_MEMBER` FILE, member =
  leader colour DERIVED). Both gated on `fx_war_faction_matrix` (flips @ tick 3/7);
  +16 map_viewer tests. See FIDELITY_LEDGER §2.23/§2.24. **M2 (partial — M1d
  lane):** **air** (`mapmode_air.rs`) and
  (FILE colours/geometry, recording-fed via items 8/10; air POV thresholds +
  infra normalisation-denominator remain capture-gated, ledger §2.20/2.21);
  supply still rides item 11 (not-started), then resistance, compliance,
  operatives, raids (LaR/BBA specialist RE, pair with U7). **M3:** generic
  scripted-map-mode layer composer for mods.

**Ordering rationale:** U1+U2 make a war replay legible with the least work
(reuse A3 unit/combat/event views + the tiny event/decision chrome). U3–U6 are
the deep country panels grouped by shared law (tree, listbox, designer template)
so each lane amortizes. U7+M2 share occupation/intel/supply data. U8 is one
generic lane covering an unbounded (mod) class. Every wave is gated by the
matrix's per-panel/per-mode status flip to POPULATED/done.

## 7. Definition of production

1. All Phase G gates green on synthetic recordings.
2. Every blocker in the **canonical blocker list** (the 10
   `*_product_path_blocked` gates + the blocked CLI switches + env-override
   rejections in `replay_app/src/main.rs` — G3 emits the enumerated list)
   either earned off with cited law or documented here as a v1 descope with
   the block retained. **Descope acceptance = a named row in this doc stating
   what is missing, why it is acceptable for v1, and what would close it.**
3. Ledger: 0 unlabelled FIT on product paths; UNKNOWN only where descoped or
   recording-fed-and-fixture-proven.
4. Weak-PC frame budget met per F1's numeric gate (≤33 ms p99 on the reference
   config) and 8-hour soak clean (F3).
5. REPLAY_DATA_REQUIREMENTS.md current, enforced mechanically: the shared
   validator's schema version is pinned to the doc revision, and a drift check
   fails CI/tests when the generator, validator, or doc diverge — so Phase H
   can start the day a real recording lands.

## 8. Top risks

| Risk | Mitigation |
|---|---|
| Fixture drift from what the extractor actually writes | shared validator (A1 rule 3) + the reconciliation sweeps against `extractor/` source (read-only) like 2026-07-09's |
| Units/counters RE turns out capture-heavy | front-load §4-CAP6 into the next live session; game interface gfx files first per game-files-first directive |
| Arrows/plans not in recording (player intent, not world state) | C4 decision: descope for v1 unless the extractor confirms availability |
| Close-zoom parity target (≤0.03 at h=50) unreachable without shadows/particles | treat particles as v1 descope candidate; shadows are ledger #21 with an engine-law lane |
| Capture sessions delayed | nothing on the A/C/D/E critical path waits on them; only B residual closure does |
