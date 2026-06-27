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

## 2. WHAT'S DONE (verified green — 1202 tests: 574 Rust + 628 Python, 0 failed)
- **Architecture (BATCH 0):** ViewRegistry (distributed via the `inventory` crate — clusters register in their own files, no central edit = conflict-free), generic WindowHost (z-bands/modal/hit-test/branchless rebind_all), BindContext/RenderContext with `Captured<T>`→`DisplayAtom` (type-enforced no-fabrication: no path from Absent to a fabricated 0).
- **Primitives (BATCH 1):** 8 widgets (N1 node_graph, N2 tabs, N3 trait_tree, N4 designer, N5 event_card, N6 naval_combat, N7 minimap, N8 map_overlay) + menubar + the `.gui`-set loader (loads all 77 parsed views).
- **All 10 view clusters = 111 views + 86 templates, integration-verified:** politics, production/construction/decisions, research/tech/doctrine, military(15), designers, leaders/traits, intel/diplomacy/trade, logistics/combat, chrome/outliner/maps, scripted-overlays(38). Each: real `.gui`, element-accounted, no-fab guard passes, scrub-byte-identical, Codex-vetted.
- **Map 1:1:** terrain pipeline ported from the game's own shaders (Codex verdict "zero achievable shader gaps") — per-pixel snow/mud/ice materials, gradient-border SDF, real season ColorCube, animated water/rivers/naval-dominance. Only proven runtime walls remain.
- **Dynamic weather visual:** render branches + data (per-province mud + snow cracked, frozen-seas via per-region `arctic_water`) + the feed (`render/src/dynamic.rs from_weather_snapshot_full` + `strategic_regions.rs`). The scrubber drives the map's weather evolution.
- **Live data extraction:** 21 of 63 open-live gaps cracked + validated live==save + wired (the per-country `variables` script-var bag → 32 overlays; weather; resources; cores; claims; dynamic_modifier; decision_status; etc.). Re-wired into the views (45 overlay slots flipped from badged to live).
- **DEF parsers (`game_defs/`):** 9 parsers complete (events 6329, scripted-gui var-maps, tech 557/557 + doctrine 119 with geometry/cost/XOR, focus PP, catalogs, templates, world names, 3018 characters).
- **Completeness manifest:** 272 rows, `check_coverage.py` PASS, scenario-corpus closed across a 5-save corpus.
- **Recorder (send-a-friend):** `.battleplan` container + app-side reader + writer wired into `capture.py` + distributable EXE built + live round-trip proven (`pack_replay verify: OK`).
- **Capture perf:** cold walk 45→18.7 min; full warm walk 135→39s (residual = `land_combats` proven-Stage-4 + diplomacy peacetime floor); Stage-3 arena-restrict ON.
- **QA:** 4 adversarial bug-test passes, **21 real bugs fixed** (incl. a stale-data commander pin, a `replace_path` nondeterminism no-fab hazard, a `.dds` integer-overflow heap-corruption-in-release, fabricated STATE_0/non-finite-coords). All Codex-vetted.

## 3. IN FLIGHT
- **No agents running.**
- #50 asset sprites is DONE: portrait/insignia sprite resolution is wired with explicit sprite-miss accounting, generic placeholder art is never counted as exact, leader-ref joins fail closed, and focused sprite/app tests pass.
- #26 map overlay subsystem is DONE: native map labels, movement/battle-plan arrows, and trade-route lines are planned/drawn from measured snapshot/static data only; missing anchors/routes are diagnostics, not guessed geometry.
- Earlier: the gap-closer `a0b6f6` COMPLETED the 3 missing views + host scrub test. Registry **114 views**, app workspace green.

## 4. DEFINITION-OF-DONE STATUS — ALL 7 CRITERIA MET ✅ (Codex-vetted)
The app is at full definition-of-done: build clean, all suites green, **114 views** registered + menu-reachable, no-fab guard passes, dynamic weather driven, live-fold + check_coverage PASS, the scrubber drives every open window (now with a host-level integration test). What remains is NOT DoD criteria — it's the badged-pending content (§5) + the user's external dependencies (§6).

## 5. REMAINING WORK (ordered; all either badged-honestly or external) — full DoD is already MET
1. ~~DoD gap (3 views)~~ — DONE (a0b6f6).
2. ~~Asset sprites — task #50~~ — DONE. Portrait/insignia sprite resolution is integrated; absent/missing/generic-placeholder sprites remain explicit unavailable/miss states, never fabricated art.
3. ~~Map overlay subsystem — task #26~~ — DONE. Map labels, movement/battle-plan arrows, and trade-route lines are integrated over the native map with no guessed endpoints or synthetic label text.
4. **Computed-tier (badged unavailable, mostly genuine):** world_tension aggregate scalar, research fraction/ETA — Codex agreed these are genuinely not-stored, not fabrication-in-disguise. Could be Ghidra-cracked (see the Ghidra memory) but are honestly badged now.
5. **Hard walls (permanent, never faked):** exact per-province supply NUMBER (computed-not-stored), Steam real names.
6. **Optional perf increment:** the single shared-pass (Stage 4) to pin `land_combats` — diminishing returns; the agent proved it's the honest peacetime floor.

## 6. WAITING ON THE USER (external dependencies)
- **A mid-war PAUSED save** (a 1941–43 SOV/ENG). The current oracle is peacetime USA-1936, so **26 data rows are empty-in-peacetime** (`needs-wartime-save` in the manifest): peace_conference, naval battles, special-projects/nukes, war casualties/score, operative code-names, the alerts ledger, civil-war/exile/collaboration keys. The moment the user loads+pauses a wartime save, dispatch a live-extraction agent to crack those 26 against it (read live current_game_date+ROOT to content-match the save oracle).
- **The user's real in-game test** against a running hoi4.exe.

## 7. LIVE GAME STATE
A game is running, **paused, USA-1936** (`1936.2.7.6`, build `16022851`), oracle save `USA_1936_02_07_06.hoi4` (content-match by reading live current_game_date + ROOT tag, NOT filename/mtime — mtime is unreliable here). ONE agent owns the live scanner + Python tree at a time; throttle live-scan agents to ~3.

## 8. ORCHESTRATION LESSONS (hard-won)
- **The app crate compiles all clusters together** → any cluster's WIP breaks `cargo build -p app`. Cluster agents validate in a SCRATCH COPY (the in-place stub race truncated 2 files). Inventory registration keeps the production tree conflict-free.
- **The harness's "agent running" view diverges from the user's panel.** Trust the panel. Ping non-responsive agents via SendMessage (resume = recover; work is on disk). Agents drop on transient API errors — recover, don't restart from scratch.
- **Codex earns its keep** — it caught the stale-data pin, the replace_path nondeterminism, and the DoD registration gap that per-feature reviews missed. Always xhigh+priority.
- **Cross-cutting QA passes find what per-feature reviews miss.** Do them.
- Two Python suites (`save_oracle`, `game_defs`) are **pytest-style** — a `unittest`-only "all green" run silently skips 56 tests. Run them under pytest.

## 9. KEY TASK IDS (TaskList)
#5 bug-test (DONE), #16 warm-perf (DONE), #22 per-candidate (DONE), #24 dynamic-weather (DONE), #25 live-extraction (DONE, 21 closed), #21 recorder (DONE), #26 map-overlay-subsystem (DONE), #50 asset-sprites (DONE). #3 ("close open tickets") is a stale pre-session ticket — verify relevance before acting.
