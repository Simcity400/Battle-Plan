# Lane g2-e2e — G2 fixture end-to-end playback gate

Plan item **G2** (REPLAY_PRODUCTION_PLAN.md §G): a scripted headless end-to-end
run over a synthetic recording with frame/state hashes — open → play → scrub →
enter a country POV → verify FoW/unit-hiding → toggle back → assert every surface
updated, recorded as deterministic hashes. Worktree `C:\Users\Example\hoi4_lanes\g2-e2e`,
branch `lane/g2-e2e`.

> Note: this file arrived from the branch base carrying unresolved merge-conflict
> markers + stale f4-anomaly/border content; this lane owns `LANE_REPORT.md` and
> replaced it with the G2 report.

## Deliverable

`replay_interface/native/replay_app/src/e2e_playback.rs`, wired into `main.rs` as
`#[cfg(test)] mod e2e_playback;` (test-only — never compiled into the product
binary). `replay_app` is a bin-only crate with no `lib` target, so a `tests/`
integration file cannot reach the internal `validate`/`observer`/`timeline`
modules; a `#[cfg(test)]` sibling module is the minimal, non-invasive way to
drive the internal product surfaces as a consumer. No existing module was
renamed/moved; no edits to `extractor/`, `replay_core`, `map_viewer`, or the
fixtures.

## The public consumer path the script drives

1. **Open** — `validate::open_and_validate(path)` → `OpenOutcome::Opened` (the
   SHARED `replay_core` validator, the exact ingest the product uses).
2. **Play / scrub** — `timeline::RecordingTimeline` (the product scrub provider);
   at each checkpoint `seek_hour == play_to_hour` and both land the exact hour
   (E1 conformance re-checked end-to-end). Owners flow through `owners_at`.
3. **Country picker** — `RecordingTimeline::pov_picker()` → `PovPicker::Available`
   (GER/ENG/SOV; see finding below).
4. **Enter POV** — the e-shell `observer::PovState::enter_pov("GER")`.
5. **Toggle back** — `exit_to_world()` / `toggle()`.

## Fixtures + why

* `positive/fx_pov_visibility.battleplan` — the POV gate (12 hours; carries the
  `visibility` domain + spotted-unit sets + `world.scenario.players`).
* `positive/fx_topbar_scalars_sweep.battleplan` — the companion topbar-binding
  surface (no visibility domain, so it isolates the derived topbar bindings and
  proves the binding hash updates each swept hour).

## Hash points (state hash, not pixels)

Headless AND the FoW *render* layer is law-blocked (C8 is tooling-only), so the
gate is on the DATA-level typed views the renderer keys off, folded via a
hand-rolled deterministic **FNV-1a/64** over a canonical string:

```
hour | observer-mode | render-marker | owners(measured) | controls(measured)
     | effective-visible-province-set | effective-spotted-units | topbar-bindings
```

* Omniscient effective-visible = the whole province universe (no fog); a POV's =
  that country's `level=="visible"` set. Omniscient spotted = `ALL`; a POV's =
  its `spotted_units` (unit-hiding surface).
* `render-marker` = the fail-closed contract: `OMNISCIENT` for world, or
  `FOWBLOCKED:GER:<FOW_NOT_LAW_CERTIFIED>` while a POV is active (never faked fog).

### Checkpoints (tick index into the 12-hour series)

`[0, 5, 7, 11]` → hours `[16959365, 16959370, 16959372, 16959376]`.

### Captured hash sequence (world_hash, pov_hash) per checkpoint

| hour | tick | world_hash (omniscient) | pov_hash (GER) |
|---|---|---|---|
| 16959365 | 0 | 17270081818805149942 | 6144089801615558050 |
| 16959370 | 5 | 6551351422545227256 | 2012908130392971866 |
| 16959372 | 7 | 8281673079917680402 | 824996207530415716 |
| 16959376 | 11 | 11448956550244400262 | 8350529044689850136 |

## POV-vs-world expected diff (derived from the generator SOURCE, not the SUT)

`replay_fixture_gen` `fx_pov_visibility` — province universe `{1000,1001,2000,3000}`:

* GER: `1000` visible when tick<5 else fogged; `1001` always visible; `2000` &
  `3000` always fogged. Spotted foreign units `{div:[201:1], fleets:[501:1]}`
  when tick<5, else empty.
* ENG: `3000` always visible; `2000` visible when tick>=7; `1000`,`1001` fogged.

Asserted, per checkpoint:

| tick | GER visible | fogged diff (world − pov) |
|---|---|---|
| 0 | {1000, 1001} | **{2000, 3000}** |
| 5 | {1001} | **{1000, 2000, 3000}** |
| 7 | {1001} | {1000, 2000, 3000} |
| 11 | {1001} | {1000, 2000, 3000} |

`diff == expected_ger_fogged(tick)` exactly, `diff` non-empty (POV hides ≥1
province), `pov_visible ⊂ world_visible`. ENG independence checked separately
(`{3000}` @tick0, `{2000,3000}` @tick7).

**Unit-hiding:** @tick0 GER spots the foreign SOV division `201:1` (+ fleet
`501:1`); from tick 5 GER spots nothing, yet the omniscient frame still carries
that SOV division (`frame.divisions()`) — a unit the POV hides but the world
view shows.

## Determinism proof

The full checkpoint script runs as two independent passes (`pass_a`, `pass_b`);
`assert_eq!(pass_a, pass_b)` — identical ordered hash sequences (values above).
The topbar companion likewise runs twice with `assert_eq!` and additionally
asserts all 12 swept-hour hashes are distinct (the binding surface updates).

## Toggle-back idempotence

From the GER POV, `exit_to_world()` reproduces the checkpoint's world_hash
exactly; a paired `toggle();toggle();` reproduces the pov_hash. `world_hash !=
pov_hash` at every checkpoint (POV changes the keyed state).

## Gate numbers

* `cargo build -p replay_app` — clean (6 pre-existing warnings, none in the new file).
* `cargo test -p replay_app` — **188 passed** / 0 failed (was 186; +2 e2e).
* `cargo test -p replay_core` — **36 passed** / 0 failed (unchanged; read-only respected).

## Finding — stale note in `observer.rs`

`observer.rs` (module note, lines ~20–26 / 38–42) claims `povs_from_session`
returns `Unavailable` on today's fixtures because the player list lives under
`world.scenario.players` "which the typed view does not yet surface." That note
is **stale**: `world.rs::build_session_metadata` (~line 2121) DOES fall through
to `world.scenario.players` via `leaf_player_countries`, so the picker is
`Available([GER, SOV, ENG])` on `fx_pov_visibility`. The e2e asserts the picker
is Available (documenting the real behavior). `observer.rs` is e-shell-owned and
was left untouched; that lane could refresh the `SESSION_PLAYERS_GAP` doc-comment.
