# Native Replay App — Breadth-Build Roadmap to 100% (master plan)

Source: roadmap agent (read-only assessment) + Codex (xhigh+priority). Detail in `BINDING_MAP_*.md` (per-view bindings) + `coverage_audit/COVERAGE_REPORT.md` §5b / `memory_manifest.json` (gap status) + `game_defs/out/` (static data).

## Current state — 5 surfaces wired, ALL PARTIAL; ~110 views unwired
- map + 13 modes (`render/modes.rs`, `app/main.rs`) — 5 modes proxy/badged
- topbar (`bind`, `compose.rs`) — ~17 of 118 elements
- state panel `province_state` (`bind/state.rs`) — shell + selected binds
- focus tree (`focus_tree` crate) — custom graph, missing view chrome
- land combat (`combat_panel`) — bubbles + roster only
Reusable primitives: framed sprite, 9-slice, BMFont glyph, progressbar, gridbox row-factory+scrollbar+scissor, GPU province-id map+LUT, drag_scroll canvas. Scene-graph parses ANY `.gui`; `bind` = element→path→formatter→fidelity with Absent/Null/Measured.

## CRITICAL: one `.gui` ≠ one view — register ROOTS/TEMPLATES, not files (files hold multiple window roots + repeated row/card templates).

## BATCH 0 — architecture split (BLOCKS ALL; one team). Crates: app, bind, gui_runtime, compose, drill.
0.1 Live snapshot fold (Python, in flight) — fold commanders/OOB/intel/world/strategic_air/economy.logistics/supply/naval_air/designs, TIME-ALIGNED per replay frame (not one-time).
0.2 ViewRegistry + ViewBuilder/BoundView traits — per-view descriptor (id, .gui path + ROOT/TEMPLATE name, required domains, builder fn, menu route); builders return scene + ViewCommands, do NOT mutate app globals.
0.3 Generic window/modal host — move window stack, z-order, modal stack, hit-test, scroll/drag capture, open/close routing OUT of main.rs; compose.rs iterates registered windows, NO per-view branches.
0.4 BindContext/RenderContext — snapshot + game_defs static + loc + assets + formatters + fidelity, passed to every builder.
0.5 Row/template factory API from the registry.
0.6 menubar = the open surface; pre-create one empty per-cluster module + registry slot (2a–2j) so clusters never edit a shared file.
Migrate the existing 5 surfaces onto the registry (must render + scrub byte-identical after).

## BATCH 1 — shared new primitives (after 0; freeze interfaces). New `widgets` crate / generalize focus_tree+combat_panel.
- N1 node-graph/tree (generalize focus_tree placed-graph+links+pan/zoom) — tech_tree, doctrine, research, power_balance, focus. HIGHEST LEVERAGE.
- N2 tab/sub-window manager — politics, mil-mgmt, intel, find, designers.
- N3 trait-tree (thin N1 specialization) — army/navy trait trees, command_character, officer_corps.
- N4 designer module-slot layout — all 5 designers.
- N5 event/popup card (portrait + wrapped events.json text) — event/popup/alerts/decisions/detailed.
- N6 naval-combat bubble (combat_panel variant) — naval_combat(_results).
- N7 minimap viewport. N8 map-overlay icon layer (province-centroid placement) — mapicons, divisions on-map, VP counters.

## BATCH 2a–2j — view clusters (FULLY PARALLEL after 0+1). Each owns bind/src/<cluster>.rs + app/src/views/<cluster>.rs + registry slots (no shared file).
2a politics_government(5)[N1,N2,N3] · 2b production_construction_decisions(3)[gridbox] · 2c research+menubar+tech_doctrine_trees[N1] · 2d military_management(15)[N2, sub-split army/navy/air] · 2e designers(5)[N4] · 2f leaders_officers_traits(8)[N3] · 2g intel_diplomacy_trade(11)[N2,N5] · 2h logistics_combat_units_events(12)[N5,N6] · 2i chrome_outliner_maps(7)[N7,N8] · 2j scripted_gui_overlays(38): 3 partial-bind buildable now, 32 chrome-only blocked on script-var bag, 1 static, 2 dev-tool.

## Blocked AFTER the live fold (build chrome now, badge field `unavailable` — never fabricate): 32 scripted overlays + topbarresources → countries.*.{variables,flags} script-var bag; alerts → alerts ledger; war cards → casualty+score split; world_tension popup → tension scalar(computed); operative → code_name; state resources → states.resources; occupation/garrison → dynamic_modifier; research% → research fraction(computed); designer stats → computed; combat detail/naval → detail rows/fixture; insignia/portraits → gfx asset. Walls (never faked): exact per-province supply NUMBER, Steam real names.

## Definition of Done — "app 100%"
1. Every in-game view ROOT + repeated template + all 38 overlays has a BoundView reachable from menubar (excl. frontend shell + 2 dev tools). 2. Every bindable field bound or badged (no fabricated 0; trichotomy preserved). 3. Dynamic visual layer (13 modes + overlay icons + bubbles) driven from snapshot w/ fidelity badge. 4. Scrubber rebuilds EVERY open registered window per tick (immutable LRU); scrub A→B→A byte-identical. 5. No-fabrication everywhere; asset-miss retries not negative-cached. 6. Live-fold complete + time-aligned. 7. All suites green: per-view golden PNG + scrub byte-compare + cargo test all crates + check_coverage.py PASS + per-view element-accounting (walk == element_type_counts).
