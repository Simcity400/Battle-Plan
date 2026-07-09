# Lane report — D-UI completion (D6 + D7 + topbar formatting residual)

Branch `lane/d-ui-completion`. Additive `game_ui` bindings + one `replay_app`
overlay wiring; no edits to `extractor/`, `replay_core`, `map_viewer`, or
fixtures. All laws FILE/UNKNOWN-labelled, fixture-gated, no eyeballing.

## Deliverable 1 — D6: replay-native date/speed clock (DONE)

The clock shows the **recording time** (frame `date`, FILE-formatted) + the
**playback speed** (transport step). Implemented in `game_ui` (decoupled — no
`replay_app` dependency), then wired into the product overlay.

- `binding::clock_binding_from_frame(frame, playback_speed_step, loc)` → `ClockBinding { date_time, speed }`, merged via `apply_clock`.
- Date via `binding::format_topbar_date{,_english}` + `topbar::format_topbar_datetime` (FILE, see below). Fail-closed `None` on an unparseable date.
- Playback speed via `binding::playback_speed_button(step)`: 0-based transport `speed_step` → topbar buttons 1..=5 (clamped). Labelled **FIT (product UX)** — a video-player control, NOT the engine game-speed law, and NOT the recording's own `world.scenario.speed`.
- `replay_app` wiring: `RecordingTimeline::hour_label` now FILE-formats the date (falls back to the raw Clausewitz string), closing the explicit raw-date GAP note in `app.rs`.

## Deliverable 2 — topbar number-formatting residual

Mined the shipped `interface/topbar.gui` + `localisation/english/core_l_english.yml`.

**Date format — FILE (implemented + gated).** `interface/topbar.gui` `DateText`
placeholder `text = "7:00 29, September 1939"` is the exact runtime shape:
`{hour}:00 {day}, {MonthName} {year}` — no leading zeros, **full** month name,
4-digit year. Month names = `core_l_english.yml` `January`..`December` (loc-
resolved, English fallback). The prior `format_topbar_datetime` produced
`"{h}:00, {d} {Mon}, {y}"` (abbreviated month, wrong comma placement) — a FIT
that disagreed with the shipped placeholder; **retired and corrected**. Gate
reproduces the placeholder byte-exact.

**Scalar number format (thousands/decimals/percent scale/sign color) — UNKNOWN
(kept raw).** The topbar scalar text elements (`pol_power`, `manpower`,
`stability`, …) carry **no `text` data-function** in `topbar.gui` — only
fixed-size layout placeholders (`"999"`, `"100%"`, `"12/12/12"`, `"999d"`). The
runtime numeric format is engine code (computed-not-stored), so it is not
citable from files; the placeholders evidence *shape* only (integer percent with
`%`; factory triple `civ/mil/dockyard`). The localisation `$VAR|fmt$` grammar
(`|0/1/2` decimals, `|%`, `|+`, `|G/Y/R/H` colors) is FILE-derivable but topbar
scalars are not loc-substituted, so it does not apply. `binding.rs` keeps every
scalar **raw** (`format!("{}")`, module header + `topbar_binding.rs` assert raw)
— unchanged, so no regression. Ledger row records the UNKNOWN + closer.

## Deliverable 3 — D7 populate-or-descope sweep (COMPLETE)

Every UI_LAYOUT_LAW §6.3/§6.4 surface is accounted for (full table added to the
plan under "D7 populate-or-descope sweep"). Summary:

| §6.4 surface | Disposition |
|---|---|
| Topbar scalars / flag / clock | IMPLEMENTED (D1/D2/D6) |
| Outliner rows | IMPLEMENTED (D3) |
| Country/diplomacy panel | IMPLEMENTED (D4) |
| Tooltip — state owner/controller data | **IMPLEMENTED (D7)** `state_tooltip_from_frame` |
| Tooltip — localised sentence/breakdown | DESCOPE — computed-not-stored; data surfaced above; closer = bounded `[expr]` evaluator |
| Map-mode bar — active state | **IMPLEMENTED (D7, viewer-state)** `observer_default_mapmode` = political button 1 |
| Map-mode bar — *recorded* active mode | DESCOPE — active mode is player UI state, not world state (§6.2); nothing to bind |
| Menubar | DESCOPE — inert read-only chrome by design |
| 8 major views — diplomacy content | PARTIAL — D4 seam exists; compositor wiring deferred to Phase E |
| 8 major views — production/research/trade/focus/designer | DESCOPE — recording-gated + not in schema |
| Listbox/gridbox/scrollbar rows | DESCOPE — missing live replay state; geometry law is FILE |
| Scrollbar thumb travel (= D5) | DESCOPE — capture-gated (CAP7) |
| Scripted-GUI visibility rules | DESCOPE — computed-not-stored |

New timeline-driven bindings: `state_tooltip_from_frame` (owner+controller,
fail-closed) and `observer_default_mapmode`. Both additive on read-only
`replay_core`; no new recording-side dependency introduced (owner_by_state /
control_by_state / frame.date() are all pre-existing requirements), so
REPLAY_DATA_REQUIREMENTS.md / EXTRACTION_NEEDS.md need no edit this lane.

## Gates (exact counts)

- `cargo build -p game_ui` — green.
- `cargo test -p game_ui` — **78 passed / 0 failed** (all pre-existing layout/quad/markup/flag/binding gates held). New gates:
  - `binding::tests`: `topbar_date_reproduces_shipped_gui_placeholder`, `topbar_date_fail_closed_on_bad_input`, `topbar_date_uses_localisation_month_name`, `playback_speed_button_maps_and_clamps`, `observer_default_mapmode_is_political_button_one` (5).
  - `tests/ui_completion_binding.rs`: clock date+speed vs recording (all 12 frames × steps 0..6), fail-closed date, state-tooltip owner/controller flip at change hour, absent-state fail-closed (**4 passed**).
  - `topbar::tests::formats_topbar_date_like_hoi4` updated to assert the shipped placeholder byte-exact.
  - `topbar_binding.rs` A3 supply/convoys expectations unchanged (not regressed).
- `cargo build -p replay_app` — green; `cargo test -p replay_app` — **170 passed / 0 failed** (transport/timeline incl. the `hour_label` clock wiring).

## Ledger updates (FIDELITY_LEDGER.md §2.16)

- New FILE row: **Topbar clock date format** (`topbar.gui` DateText placeholder + `core_l_english.yml` months).
- New UNKNOWN row: **Topbar scalar number formatting** (computed-not-stored; kept raw).
- §2.16 category summary 8/0/0/2 → **9/0/0/3**; grand totals 78/24/38/46 → **79/24/38/47**.

## Files touched

- `game_ui/src/topbar.rs` — FILE date format + `month_name_english` (fix + cite).
- `game_ui/src/binding.rs` — D6 clock (`ClockDate`/`parse_clock_date`/`format_topbar_date{,_english}`/`ClockBinding`/`clock_binding_from_frame`/`apply_clock`/`playback_speed_button`); D7 `observer_default_mapmode`, `StateTooltipBinding`/`state_tooltip_from_frame`; unit tests.
- `game_ui/tests/ui_completion_binding.rs` — new fixture-driven D6/D7 gate.
- `replay_app/src/timeline.rs` — `hour_label` FILE-formats the recording date (D6 overlay wiring).
- `REPLAY_PRODUCTION_PLAN.md` — D6 DONE, D7 DONE + per-view sweep table.
- `FIDELITY_LEDGER.md` — clock-format FILE + scalar-format UNKNOWN rows; summary/totals bumped.
