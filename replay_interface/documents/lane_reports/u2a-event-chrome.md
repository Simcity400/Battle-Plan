# LANE REPORT — u2a-event-chrome (U2 wave: event popup chrome)

**Branch:** `lane/u2a-event-chrome` · **Date:** 2026-07-09 · **No git commit.**
**Scope stayed in `game_ui`:** new `src/event_chrome.rs` + new
`tests/event_chrome.rs` + `lib.rs` module registration. No `extractor/`,
`replay_core`, `map_viewer`, or fixture edits; no module renamed/moved.

(Note: the pre-existing `LANE_REPORT.md` in this worktree held an unrelated
stale lane report with merge-conflict markers — replaced with this lane's
deliverable.)

## What was derived

The ~7,000 vanilla events are **data-driven** — every one renders through the
handful of chrome windows in `interface/eventwindow.gui`. This lane derives that
chrome and the event-def→window mapping; the per-moment CONTENT is gated on a
NEW recording stream (below), so it is fail-closed today.

### 1. Layout gates (FILE — `interface/eventwindow.gui`)

`load_event_layout` + `event_window_chrome` / `event_option_entry_chrome` pull
each window variant's chrome through the shared `gui` parser. Gated byte-exact
vs the shipped `.gui` in `tests/event_chrome.rs`:

| Element | Window | FILE value gated |
|---|---|---|
| window size | `EventWindow` | 581×427; bg `GFX_tiled_window_transparent` |
| `Title` | `EventWindow` | pos (14,70), font `hoi4_typewriter22`, format `centre`, maxWidth 551 |
| `Description` | `EventWindow` | pos (31,0), font `hoi4_typewriter16`, format `left`, maxWidth 512 |
| `event_picture` | `EventWindow` | iconType pos (5,5), sprite `GFX_report_event_001` |
| `options_grid` | `EventWindow` | gridBox pos (215,-1), size 300×30 |
| `event_option_entry` | (template) | size 300×47; `Name` font `hoi_20bs`, pos (5,0), format `center` |
| window size / bg | `EventWindow_News` | 528×595, bg `GFX_event_news_bg`, picture `GFX_news_event_001`, title maxWidth 490 |

All **4 variants** (`EventWindow`, `EventWindow_Operative`, `EventWindow_leader`,
`EventWindow_News`) + the option-row template resolve.

### 2. Event-def → window resolution (FILE — loc + gfx joins)

`load_events` parses every `events/*.txt` root-level definition (nested
firing-effect `country_event`s excluded); `resolve_events` joins each def's loc
keys + picture GFX against the loaded localisation + full sprite registry.

**Census (real install, gated):**

- **6941 events parsed** across **128 files, 0 parse failures**.
- **184 hidden** (`hidden = yes`, no window) → **6757 visible** (renderable denominator).
- **6580 (97.4%) resolve to a fully renderable window** (title + ≥1 desc +
  picture-ok + ≥1 option all resolve).
- Field resolve: title 6699 · desc 6706 · options 6720 · picture declared 6712 / resolved 6602.
- By kind (visible / renderable): country 6042 / 5900 · news 670 / 637 ·
  state 5 / 4 · unit_leader 33 / 33 · operative_leader 7 / 6.

The 177 non-renderable visible events are shipped-file gaps (missing loc entry
or picture GFX, e.g. DLC-gated pictures absent from the base registry), **not
fits** — fail-closed, never fabricated.

### 3. Fail-closed fired-event binding

`event_window_content(def, loc, fired)`: with `fired == None` returns the empty
"no event data" form (Constraint 3). With a `FiredEvent` it binds the window
(selected by kind), resolving title/desc/options from the def + loc and carrying
the chosen-option index.

## Honesty labels

- **Window selection by event TYPE** (news→`_News`, unit_leader→`_leader`,
  operative_leader→`_Operative`, else `EventWindow`) is FILE-motivated; the
  engine's leader/operative variant is content-gated (a scoped character), so a
  `country_event` that shows a leader maps to the plain window here — documented
  **UNKNOWN edge**, not fabricated.
- **desc branch** shown is `trigger`-gated (computed-not-stored). With a fired
  event the binding takes the FIRST loc-resolvable desc — documented **FIT**;
  the fired-event stream may later carry the exact shown desc.
- All content **fail-closed** absent the stream.

## NEW recording data ask (expected)

**Fired-event stream** — filed in both docs:
`{event_id, target_tag, chosen_option_index, fire_date}` per fired event
(on-change, tiny volume; NOT the event text — joined by `event_id` into the
file-derived def + item 9 loc join).

- `REPLAY_DATA_REQUIREMENTS.md` **§14** + coverage row #14.
- `EXTRACTION_NEEDS.md` **§A A18** (R-EVENTS, OPEN extractor-side).

## Docs updated (same change)

- `UI_COVERAGE_MATRIX.md` — Event-window row → **chrome-DERIVED (U2)** with the
  97.4% resolution figure; §9 item 13 marked FILED.
- `FIDELITY_LEDGER.md` §2.16 — new **Event popup chrome (U2)** row (FILE
  layout + def→window resolution; fail-closed content).
- `REPLAY_DATA_REQUIREMENTS.md` §14 + coverage row #14.
- `EXTRACTION_NEEDS.md` §A A18.

## Gates / verification

- `cargo build -p game_ui` — clean.
- `cargo test -p game_ui` — **all green** (62 lib incl. 5 new event_chrome unit
  tests; all integration incl. 4 new `event_chrome` tests). Pre-existing gates
  hold.
- `cargo clippy -p game_ui --tests` — no warnings from `event_chrome`.
- No-eyeball: every number is a gated file/loc/gfx join count, not an
  eyeballed render.
