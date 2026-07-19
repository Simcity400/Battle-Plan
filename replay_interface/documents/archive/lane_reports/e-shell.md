# LANE REPORT — lane/e-shell (Playback product shell: E1 + E4 + E6-shell)

Plan items: REPLAY_PRODUCTION_PLAN.md Phase E — **E1** (transport controls),
**E4** (recording open/validate UX), **E6 shell** (observer POV UX skeleton).
Worktree `C:\Users\Example\hoi4_lanes\e-shell`, branch `lane/e-shell`.
Crate under work: `replay_interface/native/replay_app` (READ-ONLY against
`replay_core`; `extractor/` untouched).

## Gate numbers (all green)

| Gate | Result |
|---|---|
| `cargo build -p replay_app` | **Finished, 0 errors** (the historical build-only break is clear) |
| `cargo test -p replay_app` | **170 passed / 0 failed** (+28 new: clausewitz 4, transport 10, observer 6, validate 4, main-CLI 3, timeline seek==play 1) |
| `cargo test -p replay_core` | **13 passed / 0 failed** (untouched) |
| Negative-fixture rejection matrix | 4/4 → each surfaces its specific string (`required_features`, `sha256`, `disagrees with snapshot provenance.mod`, `empty snapshot index`) via the shared validator UX path |
| Positive fixtures open | 12/12 open through `replay_core::open_recording` in the E4 path |
| Seek-bar date math / speed table / toggle idempotence / jump clamp | explicit unit gates (see below) |

## What shipped

### E1 — Transport controls (`src/transport.rs`, `src/clausewitz.rs`)
- `Transport`: pure state machine over the recording's `(hour_index, date)`
  spine. Play/pause, speed steps, step ±1 hour, seek by index/hour/fraction,
  jump-to-date, per-frame advance with auto-pause at the end (product playback
  does not wrap).
- **Speed-step table is explicit**: `SPEED_STEPS_HOURS_PER_SEC = [1,2,4,12,48]`
  game-hours/sec, default step = 4 h/s (matches the prior autoplay rate).
  **Labeled FIT (product UX)** — a video-player-style playback rate, NOT an
  engine game-speed law; makes no parity claim. clamp-up/clamp-down gated.
- **Seek-bar date math**: `clausewitz::Date` parses `Y.M.D[.H]` (chronological
  ordering, malformed-rejecting); `date_ticks()` emits one tick per calendar
  day at the first recorded hour of that day (`Y.M.D` label). Exact, gated.
- **Jump-to-date**: resolves to the recorded hour at-or-before the target,
  clamped to `[first,last]`; unparseable input errors (never a guessed jump).
  Clamp cases gated (before-first→0, after-last→last, at-or-before, exact).
- **Wired to `replay_core` seek/play_to hooks**: `timeline.rs::RecordingTimeline`
  now exposes `seek_hour`/`play_to_hour` delegating to `WorldTimeline::seek`/
  `play_to`; conformance gate `seek_equals_play_to_on_every_recorded_hour`
  asserts they resolve identically on a fixture (the A4 equality invariant).
- Interactive wiring in `app.rs`: Space=play/pause, `[`/`]`/arrows=step,
  `+`/`-`=speed, `,`/`.`=prev/next day tick; scrub bar seeks by fraction. The
  political-color product block is untouched (badge still rendered).

### E4 — Recording open/validate UX (`src/validate.rs`, `main.rs`)
- `validate::open_and_validate(path)` runs the **SHARED validator**
  (`replay_core::open_recording` → `WorldTimeline::open` — the exact entry
  `source.rs` uses). **No parallel validation logic.** Classifies the outcome
  into a readable category (`NotFound`/`ContainerOrSchema`/`Modset`/
  `Unsupported`/`MultipleGames`/`Timeline`) while carrying the underlying
  precise validator string verbatim, so the specific rejection is surfaced.
- Product entry `replay_app --validate-recording <path>` (works without a map
  bundle; exit 0 on open, exit 2 on rejection with the specific message).
- Gated by the committed fixtures: negative rejection matrix (each negative →
  its `expected_error_contains` string + a category assertion) and every
  positive opens with the expected game id / hour count.

### E6 shell — Observer POV UX skeleton (`src/observer.rs`)
- `PovState`: default omniscient world view; `enter_pov(tag)` flips the state
  object (idempotent); `toggle()` is **stateless and idempotent under pairing**
  (`toggle();toggle();` returns to the start), one-key back to world. Gated.
- Country picker `povs_from_session(&SessionMetadataView)` derives choices from
  the `replay_core` session view (never a raw-snapshot parse behind the seam);
  drops non-measured tags; fails closed to `PovPicker::Unavailable` on
  absent/unobtained/empty lists.
- **Fail-closed POV render contract**: `render_status()` returns `FowBlocked`
  with `FOW_NOT_LAW_CERTIFIED` ("FoW layer not yet law-certified … showing the
  omniscient world view, not a faked fog look") — the state flips so the UI can
  show which country is observed and offer the toggle, but the fog RENDER stays
  blocked. Hidden-unit / UI re-keying = later lanes (per mission).
- Interactive wiring in `app.rs`: digits `1-9` pick a player country, `O`
  toggles world↔POV, status line shows the observer note + FoW diagnostic.

## Blocked-path surfacing behavior (fail-closed honesty)
- **POV entry**: never fakes fog; renders omniscient and surfaces
  `FOW_NOT_LAW_CERTIFIED`. No existing `*_product_path_blocked` gate in
  `main.rs` was removed or weakened.
- **Country picker on today's fixtures**: fails closed to `Unavailable` (see
  the API gap below) rather than inventing countries.
- **E4 rejections**: the underlying schema/hash/modset/version string is
  surfaced verbatim; the CLI returns a non-zero exit.

## replay_core API gap (recorded per mission — needs the parallel lane)
**`SessionMetadataView.player_countries` does not surface the fixtures' player
list.** `build_session_metadata` (world.rs:1907) reads player countries only
from a top-level `session.player_countries` object. The snapshot-flavor
recordings/fixtures carry their players under **`world.scenario.players`**
(objects `{tag,slot,label}`) and have no top-level `session` key, so the typed
view returns `Leaf::Absent` for **every** current fixture. DLC/language already
fall back to `world.scenario`; player_countries and speed_events do **not**.

- Consequence: the E6 picker fails closed (`SESSION_PLAYERS_GAP`) on today's
  fixtures — a passing gate (`picker_fails_closed_when_absent`) documents this.
- This is a **replay_core view gap, not a recording-side dependency** (the data
  exists in the recording; REPLAY_DATA_REQUIREMENTS item 4 already covers the
  player↔country mapping). No `REPLAY_DATA_REQUIREMENTS.md` / `EXTRACTION_NEEDS`
  change is warranted; the fix is a `world.scenario.players` fallback in the
  session-view builder, owned by the parallel `replay_core` lane. Once it lands,
  the picker becomes `Available` on the fixtures with no change to this lane.

## Known follow-ups (honest, out of scope for the shell)
- Interactive **text entry** for jump-to-date: the logic + clamping are
  implemented and gated; the in-window date-entry widget is a later lane (`,`/`.`
  day-tick navigation ships as the interim date-based control).
- Seek-bar **date-tick labels** are computed (`date_ticks`) and tested but the
  overlay still draws index-spaced hour ticks; rendering the dated labels is a
  small overlay follow-up (no law/parity dependency).
- Dates for the transport spine are fetched via `WorldTimeline` frame loads at
  startup; fine for the small fixtures, revisit under Phase F2 (bounded working
  set) for multi-year recordings.

## Files touched
- New: `replay_app/src/clausewitz.rs`, `transport.rs`, `observer.rs`, `validate.rs`.
- Edited: `replay_app/src/main.rs` (module decls, `--validate-recording` entry,
  CLI tests), `app.rs` (transport + POV wiring), `timeline.rs`
  (`date_at`/`pov_picker`/`seek_hour`/`play_to_hour` + conformance test).
- Untouched: `replay_core/*`, `extractor/*`, all `*_product_path_blocked` gates.
