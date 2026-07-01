# UI Source-Of-Truth And Parity Report

Purpose: give UI agents a clear operating directive for the native replay UI,
including the strategic map and every map-adjacent interaction. This report does
not replace `MASTER_PROGRESS.md`, `BINDING_MAP_COVERAGE.md`,
`INTERACTION_PARITY_INVENTORY.md`, or the executable interaction contracts. It
states the product standard and the proof expected before an agent claims a UI
surface behaves like the game.

## Product Standard

The replay UI should feel as close as possible to playing the real in-game the game
UI, except that it is strictly read-only. This includes the map, not just panels
and menus. The user should be able to inspect the recorded game through the same
kinds of panels, tabs, lists, maps, map modes, counters, overlays, labels,
tooltips, buttons, hover states, drilldowns, camera interactions, and visual
states they use in HOI4.

Do not invent the game behavior from memory, aesthetic guesswork, or a generic
"strategy game UI" idea. Prefer direct game-file evidence. Where direct
game-file evidence is insufficient, use recorded replay state, static definition
parsers, bounded replay adapters, and explicit badges. Any remaining gap must be
listed clearly so the user can help explain the real-game behavior.

## Source-Of-Truth Hierarchy

Use this order when deciding how a UI surface should look or behave:

1. **Effective installed content, resolved mod-over-vanilla.**
   - UI layout and static chrome should come from extracted `interface/*.gui`
     files in `replay_interface/game_assets/gui/*.json`.
   - Sprite names, texture paths, frame counts, progress-bar sizes, 9-slice
     metadata, masks, and flags should come from `replay_interface/game_assets/sprites.json` and
     the resolver.
   - Bitmap fonts should resolve through `interface/core.gfx` and
     `replay_interface/game_assets/fonts.json`.
   - Localisation should resolve from the effective localisation set, with mod
     keys overriding vanilla keys where applicable.
   - The active bundle must match the replay's recorded mod-set when the replay
     provides one. A vanilla UI over a modded replay is a mismatch, not a
     harmless fallback.

2. **Static game definitions, resolved mod-over-vanilla.**
   - Use `extractor/game_defs/` for definitions outside `interface/`: focuses, scripted
     GUIs, events, technologies, doctrines, catalogs, templates, world
     definitions, characters, traits, decisions, ideas, and scripted-localisation
     joins.
   - Do not create a second override system. `extractor/game_defs` should reuse the same
     Clausewitz file-level override and `replace_path` model as `replay_interface/game_assets`.

3. **Recorded replay state.**
   - Runtime state shown to the user should come from the `.battleplan`
     snapshots produced by the live host `hoi4.exe` collector.
   - Saves and old memory-reader paths are development fixtures or validation
     aids. They are not runtime truth for a shipped replay.
   - If a value is absent, null, unsupported, or not captured, render it as
     unavailable. Never silently coerce it to `0`, an empty row, a guessed icon,
     or a plausible-looking label.

4. **Replay-specific adapters and interaction contracts.**
   - Some the game UI behavior is engine-side and is not fully expressed in `.gui`
     files. For those surfaces, write explicit adapters and contracts.
   - Adapter code is allowed only to translate game-file structure plus recorded
     state into read-only replay behavior. It must not invent missing game logic.
   - Every read-only interaction must be documented and tested. Every mutating
     interaction must be visibly disabled or inert and tested as a no-op.

5. **User-confirmed behavior for unresolved gaps.**
   - When the files and replay state do not explain a UI connection clearly,
     record the gap and ask for user confirmation or a reference save/screenshot.
   - The user knows the game. Use that knowledge to close gaps, but capture the
     result as a testable rule, not as undocumented agent intuition.

## What Must Come From Installed content

Agents should maximize direct use of game assets and definitions for:

- Map source art, province/state/strategic-region geometry, borders, terrain,
  rivers, weather/season visuals, country colors, flag/counter art, map icons,
  labels, and map-mode button art from the effective game/mod files or generated
  artifacts that are directly derived from those files.
- Window/panel layout, anchoring, clipping, z-order, draw order, and element
  names from `.gui`.
- Button, tab, list, grid, scrollbar, text, icon, progress-bar, 9-slice, flag,
  and frame metadata from `.gui` and `.gfx`.
- Sprite and art identity from `sprites.json` plus the game resolver.
- Fonts from `core.gfx` and extracted font maps.
- Localized display strings from localisation files, not English strings
  invented in Rust or Python.
- Focus, tech, doctrine, trait, event, scripted-GUI, decision, idea, law,
  advisor, operation, and catalog data from `extractor/game_defs`.
- Mod-over-vanilla load order from `active_modset.json` or the replay's recorded
  provenance.

If a view or map surface currently hardcodes geometry, labels, colors, button
identity, counter placement, overlay semantics, or a row model, the agent must
either prove that the hardcode is a temporary adapter over real game-file
targets, or file it as a gap.

## Map Design Is In Scope

The map is part of the UI source-of-truth requirement. Treat it with the same
standard as any the game panel:

- Base map visuals should come from real game map assets or generated artifacts
  directly derived from them, not from an invented map style.
- Province picking, state selection, ownership/control colors, country colors,
  strategic regions, terrain, borders, rivers, weather, mud/snow/frozen-sea
  visuals, map labels, counters, combat bubbles, front lines, battle plans,
  trade routes, supply overlays, naval/air regions, map icons, and map modes
  must be tied to installed content, parsed map data, recorded replay state, or clearly
  badged proxy/unavailable evidence.
- Map camera behavior, selection behavior, hover readouts, minimap recentering,
  map-mode switching, popups, legends, and map-linked drilldowns are interaction
  parity work. They need contracts and tests just like buttons, tabs, and lists.
- If the app uses a proxy, simplification, or derived approximation for a map
  mode or overlay, label it as such. Do not let a plausible-looking overlay imply
  exact the game parity.
- Side-by-side real the game screenshots are especially important for map design,
  because small color, layer, icon, counter, and overlay differences can change
  how players read the game state.

## What Cannot Be Assumed From `.gui` Alone

The `.gui` files are a critical source of truth, but they are not the whole game
UI engine. The extraction pipeline explicitly covers static layout skeletons and
sprite references. It does not by itself capture:

- Runtime visibility conditions.
- Listbox, gridbox, and scrollbar population.
- Scripted-GUI runtime behavior.
- Engine-side button commands.
- Full trigger/effect evaluation.
- Runtime-computed tooltip lines.
- Combat, designer, battle-plan, supply, operation, and other computed breakdowns
  that the game assembles in code.

Those areas need static definition parsers, replay adapters, limited evaluators,
recorded live data, or explicit unavailable badges.

## Required Gap Classification

Every UI gap must be listed with one of these classifications:

- **Missing extracted game file:** the relevant `.gui`, `.gfx`, font, loc, or art
  file is not currently extracted or loaded.
- **Missing static definition parser:** the behavior exists in `common/`,
  `events/`, `history/`, or localisation, but no parser or join exists yet.
- **Missing live replay state:** the value exists in game state but the live
  collector or snapshot schema does not capture it yet.
- **Computed-not-stored:** the game computes the value from several inputs or engine
  code. Recompute only with evidence; otherwise badge unavailable.
- **Engine-only or unknown behavior:** the connection is not visible in game
  files or snapshots. Ask for user help, a reference screenshot, or a fixture.
- **Intentional read-only divergence:** the real control mutates game state, so
  the replay must show it disabled or inert.
- **Permanent wall:** proven unavailable or out of scope. Keep it badged.

Each gap entry should include:

- View id and source `.gui` or definition file if known.
- The exact element/control/row/tooltip involved.
- What evidence has been checked.
- What is missing.
- Proposed closure path.
- Whether the user can help with gameplay knowledge, screenshot, save, or live
  fixture.

## Current Known Risks To Address

These are the immediate risks agents should check before claiming UI parity:

1. **`replay_interface/game_assets/gui` must be populated before real `.gui` coverage claims.**
   The code expects `replay_interface/game_assets/gui/*.json`, but the current checkout may not
   have those generated files present. If `replay_interface/game_assets/gui/topbar.json` and
   `replay_interface/game_assets/gui/_index.json` are absent, regenerate the asset bundle or
   restore the generated layouts before claiming real game-layout parity.

2. **Static layout is not runtime behavior.**
   `replay_interface/game_assets` can prove panel skeletons, sprites, and element names. It does
   not prove dynamic visibility, row population, scripted-GUI behavior, or full
   interaction parity.

3. **Interaction coverage is still open.**
   The inventory and executable scaffold exist, and many first slices are
   proven. Product parity remains open until every read-only interaction and
   every disabled mutation target is implemented, visible, and tested.

4. **Tooltips need special scrutiny.**
   Game-style tooltip frames, timing, localization, markup, trigger-derived
   lines, and computed breakdowns must be proven or badged. Do not silently omit
   a line that the game would show unless it is tracked as a gap.

5. **Map, counter, combat, designer, operation, supply, and battle-plan surfaces
   contain high-risk computed behavior.**
   These should not be filled with plausible substitutes. They need captured
   live data, static parser evidence, faithful recomputation, or visible
   unavailable/proxy badges.

6. **Mod identity matters.**
   The selected asset bundle must match the replay's mod-set. If replay
   provenance is unknown, show an advisory. If it is known and mismatched, warn.

## Required Workflow For UI Work

For every UI view or interaction an agent touches:

1. Identify the view id, source `.gui`, relevant `extractor/game_defs` parser output, and
   snapshot domain paths.
2. Confirm the effective mod-over-vanilla source, not just vanilla, unless the
   replay is explicitly vanilla.
3. Load or regenerate the needed extracted assets before making fidelity claims.
4. Bind from recorded state or parsed definitions. Do not invent fallback values.
5. Classify each unavailable value as exact, derived, proxy, unavailable, or
   intentionally inert.
6. Implement read-only interactions as navigation, selection, filtering, scroll,
   hover, tooltip, drilldown, scrub, or map-recenter commands only.
7. Implement mutation controls as disabled/inert no-ops with visible treatment.
8. Add tests that prove:
   - correct target identity from real laid-out elements when possible;
   - no-op behavior for mutating controls;
   - no fabricated data on absent/null inputs;
   - scrub A-to-B-to-A does not carry stale state;
   - missing selected rows fail closed instead of selecting a fallback.
9. Update the appropriate canonical ledger:
   - `MASTER_PROGRESS.md` for current status changes;
   - `INTERACTION_PARITY_INVENTORY.md` for interaction evidence;
   - `BINDING_MAP_COVERAGE.md` or `BINDING_MAP_*.md` for binding and gap changes;
   - `extractor/coverage_audit/memory_manifest.json` and `COVERAGE_REPORT.md` for data-row
     coverage changes.

## User-Help Requests To Surface Early

Ask the user for help when any of these are true:

- The installed content show a control, but the agent cannot prove when or why it is
  visible.
- A tooltip or detail panel depends on a computed value that is not captured.
- A view has several plausible drilldown targets and the files do not reveal the
  real in-game path.
- A modded overlay or scripted GUI changes behavior in a way static parsing does
  not explain.
- Side-by-side screenshots disagree and the cause is not obvious from files.
- A wartime, eventful, or specific-country fixture is needed to expose a row.

When asking, provide a short, concrete prompt: view name, control name, current
evidence, and exactly what behavior needs confirmation.

## Non-Negotiable Rules

- No guessed UI behavior.
- No fabricated data.
- No hidden fallback from modded replay UI to vanilla UI.
- No silent omission of known the game UI content.
- No playable/mutating code path.
- No absolute user paths or local install paths in committed files.
- No committing generated Paradox art or other large binary artifacts.

## Practical Definition Of Done

A UI surface is not done because it opens or looks similar. It is done only when:

- Its real source files are identified.
- Its static layout and art are game-file-backed.
- Its displayed values are exact, derived with evidence, proxy-badged, or
  unavailable-badged.
- Its read-only interactions work and are tested.
- Its mutating controls are visible and inert or disabled.
- Its tooltips and hover states are implemented or gap-listed.
- Its behavior survives scrub/rebind without stale state.
- Its gaps are listed clearly enough that the user can help close them.
