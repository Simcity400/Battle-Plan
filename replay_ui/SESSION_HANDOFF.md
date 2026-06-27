# SESSION HANDOFF — Native Battle Plan App Build

You are taking over as the **ORCHESTRATOR** of a large multi-agent build. Read this top to bottom before acting.

## 0. YOUR ROLE + STANDING RULES (non-negotiable)
- **Default to orchestration for large work.** Use subagents for execution and adversarial review where they add leverage; direct repo operations such as validation, staging, commit, and push are fine in a Codex-native session.
- **/goal is active: "finish the app 100%."** A Stop hook blocks stopping until met. Keep driving.
- **Adversarial review stays mandatory** for substantive RE findings, offset hypotheses, save-parse logic, and risky changes. In Codex-native sessions, use normal subagents/review agents; the old CLI command in `CODEX_PROTOCOL.md` was for Claude-only handoff compatibility.
- **NO escape hatches.** Reject any "optional / skippable / out-of-scope / deferred / approximated-and-left" from an agent — on the brief AND on review. The only admissible non-completion is a *proven* physical impossibility.
- **Output over design.** Only the output matters (correct + vision-aligned). Rip-and-replace prior design freely; never brief "minimal-risk / preserve semantics."
- **HARD CONSTRAINTS:** READ-ONLY game memory (never WriteProcessMemory); game stays PAUSED during scans; **NO git commits unless asked**; **NEVER put local home-directory, cloud-documents, machine-specific install paths, or attribution trailers in tracked/committed files.**
- **Isolated-copy validation:** cluster agents must validate in a SCRATCH COPY of the tree, NEVER stub/modify a sibling file in place (an in-place race truncated two cluster files mid-build).

## 1. THE PROJECT
A read-only, game-faithful, interactive **replay UI** in pure **Rust + wgpu** (NO Bevy/Tauri), reading game data **100% from LIVE MEMORY** (no saves at runtime; saves are only a discovery/validation oracle). Repo root: `mod/battle-plan/` (private, github Simcity400/battle-plan). Halves:
- `replay_native/` — the Rust app. Crates: snapshot, assets, render, gui_runtime, bind, scrub, focus_tree, combat_panel, widgets, app, recorder. Run: `cargo run --release -p app --bin replay_app`.
- `live_capture/` + `memory_reader/` — the Python external-memory reader (stdlib ctypes, READ-ONLY, build-gated on hoi4.exe sha256).
- `game_defs/` — 9 static DEF parsers (mod-over-vanilla). `coverage_audit/` — the data completeness manifest. `replay/` — snapshot store/diff/schema. `replay_ui/` — docs.
- **Master plan: `replay_ui/APP_COMPLETION_ROADMAP.md`** (the batch plan + the 7-criterion Definition of Done in §F). Per-view bindings: `replay_ui/BINDING_MAP_*.md` (COVERAGE.md = master inventory). Data status: `coverage_audit/memory_manifest.json` (status column) + `COVERAGE_REPORT.md` + `check_coverage.py`.

## 2. WHAT'S DONE (historically verified green — 1202 tests: 574 Rust + 628 Python, 0 failed)
- **Architecture (BATCH 0):** ViewRegistry (distributed via the `inventory` crate — clusters register in their own files, no central edit = conflict-free), generic WindowHost (z-bands/modal/hit-test/branchless rebind_all), BindContext/RenderContext with `Captured<T>`→`DisplayAtom` (type-enforced no-fabrication: no path from Absent to a fabricated 0).
- **Primitives (BATCH 1):** 8 widgets (N1 node_graph, N2 tabs, N3 trait_tree, N4 designer, N5 event_card, N6 naval_combat, N7 minimap, N8 map_overlay) + menubar + the `.gui`-set loader (loads all 77 parsed views).
- **All 10 view clusters = 114 views + 86 templates, integration-verified:** politics, production/construction/decisions, research/tech/doctrine, military(15), designers, leaders/traits, intel/diplomacy/trade, logistics/combat, chrome/outliner/maps, scripted-overlays(38), plus the late-registered diplomacy/combat windows. Each: real `.gui`, element-accounted, no-fab guard passes, scrub-byte-identical, reviewed.
- **Map terrain shader parity:** terrain pipeline ported from the game's own shaders (review verdict: zero achievable terrain-shader gaps) — per-pixel snow/mud/ice materials, gradient-border SDF, real season ColorCube, animated water/rivers/naval-dominance. This is not full map-product parity; overlays, labels, routes, tooltips, and side-by-side interaction parity remain tracked below.
- **Dynamic weather visual:** render branches + data (per-province mud + snow cracked, frozen-seas via per-region `arctic_water`) + the feed (`render/src/dynamic.rs from_weather_snapshot_full` + `strategic_regions.rs`). The scrubber drives the map's weather evolution.
- **Live data extraction:** the manifest has moved past the old 63-open-live snapshot. Current row-level
  coverage is: 142 `closed-by-reader`, 11 `closed-by-parser`, 26 `needs-wartime-save`, 15
  `open-live-extractable`, 14 `open-computed`, 2 `wall`, 62 `excluded`. Re-wired closures include the
  per-country `variables` script-var bag, weather, resources, cores, claims, dynamic modifiers, decisions,
  and related overlays.
- **DEF parsers (`game_defs/`):** 9 parsers complete (events 6329, scripted-gui var-maps, tech 557/557 + doctrine 119 with geometry/cost/XOR, focus PP, catalogs, templates, world names, 3018 characters).
- **Completeness manifest:** 272 rows, `check_coverage.py` PASS, scenario-corpus closed across a 5-save corpus.
- **Recorder (send-a-friend):** `.battleplan` container + app-side reader + writer wired into `capture.py` + distributable EXE built + live round-trip proven (`pack_replay verify: OK`).
- **Capture perf:** cold walk 45→18.7 min; full warm walk 135→39s (residual = `land_combats` proven-Stage-4 + diplomacy peacetime floor); Stage-3 arena-restrict ON.
- **QA:** 4 adversarial bug-test passes, **21 real bugs fixed** (incl. a stale-data commander pin, a `replace_path` nondeterminism no-fab hazard, a `.dds` integer-overflow heap-corruption-in-release, fabricated STATE_0/non-finite-coords). All Codex-vetted.

## 3. IN FLIGHT
- #21 recorder is upgraded for selectable cadence modes: in-game-hour de-duplication, every-1/2/6-hour GUI modes, manual/paused one-shot capture, skipped-hour diagnostics relative to the selected cadence, manifest cadence metadata, visible cadence status strings, and focused tests exist. Product parity is still open until a live unpaused MP-speed recording proves no missed scheduled capture hours or produces explicit skipped-hour diagnostics.
- #50 asset-sprite resolver/accounting is partially landed: portrait sprite resolution, explicit sprite-miss accounting, generic-placeholder rejection, selected-formation insignia binding, shared formation-insignia resolver coverage for explicit emblem fields and one-based army/army-group shield frames, `military.army` formation-row consumer proof for exact vs unavailable digest/accounting, anchor-gated leader portrait joins, anchor-gated land unit-counter count/owner/org/fractional-strength/type/dig-in logical entities, memory-scaled strength progressbars failing closed at the real anchor, and anchor-gated mapicon formation-insignia entities exist. The mapicon slices now reject `target_province` as a counter anchor and owner-gate division/OOB joins. Product parity is still open for full GUI counter chrome/art, commander selection, navy/air/rail/transport counters, HQ/supply/attrition, complete formation/insignia render extraction, and side-by-side visual verification.
- #26 map-overlay primitives are partially landed: native labels/vector arrows/trade planning exist, top-level weather/naval-air map-mode availability is fixed, derived controller-boundary front-line segments render with diagnostics, route/front/battleplan/trade lines carry deterministic semantic style metadata separate from fidelity, explicit measured route/save-backed battle-plan/`map.front_lines[]` map-point polylines ingest as exact, malformed/missing anchored geometry renders unavailable markers instead of fabricated fallback lines, exact/measured route or front geometry without points no longer falls back to derived province/controller paths, and live `oob_command.member_div_ids` now anchors live type-5 `battle_plan_orders[].target_states` into derived battle-plan arrows. Product parity is still open for real the game route/front extraction coverage, exact the game battleplan/front rendering semantics, remaining map-mode visual drivers, and side-by-side visual verification.
- Interaction parity now has a first-pass inventory in `replay_ui/INTERACTION_PARITY_INVENTORY.md` and a first executable view-level scaffold in `replay_native/app/src/views/interaction_contract.rs`. Focused app tests cover all 114 registered views and every required category status. Screenshot harness plumbing now includes a registry-derived `--shot-all-views=<dir>` manifest with native/HOI4/diff filenames and native shot args for all 114 views; `core.state_panel` fails closed without a state and receives a default state in manifest mode. The scaffold is accounting/plumbing only: tab/row/scroll/tooltip/drilldown/disabled-mutation/image capture/side-by-side screenshot proof work remains open.
- Tooltip parity has a first vertical slice: static `.gui` tooltip metadata is preserved, generic passive tooltip plans render through the host and enter bound digests, and tech-tree hover emits factual no-fabrication tooltips. Product parity is still open for game-style frames and per-domain tooltip composers.
- Earlier: the gap-closer `a0b6f6` COMPLETED the 3 missing views + host scrub test. Registry **114 views**, app workspace green.

## 4. ENGINEERING DoD STATUS — ALL 7 CRITERIA MET ✅ (not product parity)
The app has met the narrower engineering definition-of-done: build clean, all suites green, **114 views**
registered + menu-reachable, no-fab guard passes, dynamic weather driven, live-fold + check_coverage PASS,
and the scrubber drives every open window (now with a host-level integration test). This does **not** mean
the replay product is complete. Product parity still requires interaction parity, hourly capture proof,
side-by-side visual comparison, and closure/proof of the remaining manifest rows.

## 5. REMAINING WORK (product parity; engineering DoD is already met)
1. ~~DoD gap (3 views)~~ — DONE (a0b6f6).
2. **Recorder cadence — task #21:** selectable cadence modes are implemented. Run the live unpaused recording proof at intended MP speed; fix any skipped-hour/cadence failures before parity claims.
3. **Asset sprites — task #50:** partially complete. The shared resolver now proves formation insignia/emblem metadata, shield-frame miss accounting, and `military.army` row consumption for explicit, one-based-frame, missing-base, generic-placeholder, and out-of-range cases; mapicons now place anchored land counter count/owner/org/fractional-strength/type/dig-in logical entities, badge memory-scaled strength progressbars unavailable at the real counter anchor, place anchored army/army-group formation-insignia sprite entities with deterministic miss accounting, reject target-only anchors, and require matching-owner division/OOB joins. Full GUI counter chrome/art, commander selection, navy/air/rail/transport counters, HQ/supply/attrition, full military formation render extraction, and side-by-side visual tests still block parity.
4. **Map overlay subsystem — task #26:** partially complete. The diagnostic-marker/semantic-line-style/measured-route-ingestion slices, save-backed measured battle-plan polyline ingestion, explicit measured `map.front_lines[]` geometry ingestion, exact-route/front malformed-geometry guardrails, and live member-div-id battle-plan arrow join are closed; close real the game route/front extraction coverage, exact the game battleplan/front/trade overlays, remaining map-mode visual drivers, and side-by-side visual checks before marking parity done.
5. **Computed-tier (badged unavailable, mostly genuine):** world_tension aggregate scalar, research fraction/ETA and the remaining `open-computed` rows are not stored directly. They still block product parity until faithfully recomputed or reviewed as permanent badges.
6. **Interaction parity:** expand the first executable view-level scaffold into per-target contracts/tests; implement tabs, row selections, scroll, additional hover/tooltips, drilldowns, and disabled mutation proof.
7. **Hard walls (permanent, never faked):** exact per-province supply NUMBER (computed-not-stored), Steam real names.
8. **Perf decision after live proof:** decide whether the single shared-pass Stage-4 work for `land_combats` is required after measuring the live unpaused hourly recorder test.

## 6. WAITING ON THE USER (external dependencies)
- **A mid-war/eventful PAUSED save** (for example 1941–43 SOV/ENG). The current oracle is peacetime
  USA-1936, so **26 data rows are empty-in-peacetime** (`needs-wartime-save` in the manifest):
  peace_conference, sunk-ship/naval battle history, ships-built history, war casualties/score, operative
  code-names, alerts, and civil-war/exile/collaboration keys. Broad project/nuke status is **not** open:
  `program`, `countries.*.program_status`, and `countries.*.nukes` are closed. An active special-project
  save would only be optional per-instance hardening if that granularity is added later. The moment the
  user loads+pauses a suitable save, dispatch a live-extraction agent to crack the 26 rows against it
  (read live current_game_date+ROOT to content-match the save oracle).
- **The user's real in-game test** against a running hoi4.exe.

## 7. LIVE GAME STATE
A game is running, **paused, USA-1936** (`1936.2.7.6`, build `16022851`), oracle save `USA_1936_02_07_06.hoi4` (content-match by reading live current_game_date + ROOT tag, NOT filename/mtime — mtime is unreliable here). ONE agent owns the live scanner + Python tree at a time; throttle live-scan agents to ~3.

## 8. ORCHESTRATION LESSONS (hard-won)
- **The app crate compiles all clusters together** → any cluster's WIP breaks `cargo build -p app`. Cluster agents validate in a SCRATCH COPY (the in-place stub race truncated 2 files). Inventory registration keeps the production tree conflict-free.
- **The harness's "agent running" view diverges from the user's panel.** Trust the panel. Ping non-responsive agents via SendMessage (resume = recover; work is on disk). Agents drop on transient API errors — recover, don't restart from scratch.
- **Adversarial review earns its keep** — prior review caught the stale-data pin, the replace_path nondeterminism, and the DoD registration gap that per-feature reviews missed. Continue review in the current Codex-native workflow; do not use the old local CLI protocol.
- **Cross-cutting QA passes find what per-feature reviews miss.** Do them.
- Two Python suites (`save_oracle`, `game_defs`) are **pytest-style** — a `unittest`-only "all green" run silently skips 56 tests. Run them under pytest.

## 9. KEY TASK IDS (TaskList)
#5 bug-test (DONE), #16 warm-perf (DONE), #22 per-candidate (DONE), #24 dynamic-weather (DONE), #25 live-extraction (DONE, 21 closed), #21 recorder (selectable cadence modes implemented; live proof pending), #26 map-overlay-subsystem (PARTIAL), #50 asset-sprites (PARTIAL). #3 ("close open tickets") is a stale pre-session ticket — verify relevance before acting.
