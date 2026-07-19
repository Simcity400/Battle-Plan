# LANE REPORT — lane/f4-anomaly (Phase F4: mid-playback data-anomaly policy)

Plan item: `REPLAY_PRODUCTION_PLAN.md` Phase F — **F4** (crash/error policy:
"log + hold-last-good, never invent" for anomalies that appear **mid-playback**
in an otherwise-valid recording). Worktree `C:\Users\Example\hoi4_lanes\f4-anomaly`,
branch `lane/f4-anomaly`. Crates under work: `replay_fixture_gen` (fixtures +
manifest) and `replay_app` (policy at the `replay_core` **consumer seam**).
`replay_core` and `extractor/` untouched (read-only for this lane).

Startup fail-closed diagnostics already exist (the `--validate-recording`
path + the negative-fixture rejection matrix from lane/e-shell). F4 closes the
open gap: anomalies that surface **mid-stream** in a recording that opened
cleanly.

## Gate numbers (all green)

| Gate | Result |
|---|---|
| `cargo build -p replay_app` | **Finished, 0 errors**, no new warnings |
| `cargo test -p replay_app` | **178 passed / 0 failed** (+8 new anomaly-policy tests over the 170 e-shell baseline) |
| `cargo test -p replay_core` | **30 passed / 0 failed** (untouched; incl. `fixture_conformance` 3/3 — negative matrix 4/4 intact) |
| `cargo run -p replay_fixture_gen` regeneration | **byte-identical**: two fresh same-seed runs identical, and regen == committed across positive+anomaly+negative+manifest |
| Negative-fixture rejection matrix (e-shell) | **unchanged** — positive/ and negative/ fixture bytes are untouched in git; only `manifest.json` gained an additive `anomaly` section |

## Anomaly taxonomy (the 5 seeded fixtures)

All are packed **snapshot-flavor** `.battleplan` containers that OPEN through the
shared `ReplaySource` validator and then **degrade mid-stream** — the opposite
of the `negative/` set, which REJECTS at open. They live in a separate
`fixtures/anomaly/` tree + a separate `"anomaly"` manifest section, so the
strict-negative matrix semantics stay crisp (anomaly ⇒ opens+degrades;
malformed ⇒ rejects). Seed `424242`, 12 hourly snapshots each (11 for the gap).

| Fixture | Class | Injection (crossing = tick 6) | Detected via (consumer-seam signal) | Policy behavior |
|---|---|---|---|---|
| `fx_anomaly_missing_domain` | MissingDomain | `weather` domain present hours 0–5, then the whole key removed hours 6–11 | `frame.weather_status()` transitions `Present → DomainAbsent` (Absent-after-Present) | log once; hold last-good weather value MARKED `HELD weather from hour <5>`; never invent rows |
| `fx_anomaly_type_flip` | TypeFlip | `countries.GER.politics.political_power` number→string at hour 6 | `CountryCore.political_power` goes `Measured → Unobtained` **and** a `coverage_warnings` entry names the field `wrong-typed` | log once; hold last-good number MARKED; never coerced |
| `fx_anomaly_out_of_range` | OutOfRange | GER `manpower.available = -5000` + province id `987654` (absent from the map) from hour 6 | `topbar.manpower_available = Measured(<0)`; `control_by_province` key not in the known map set | flag both (two distinct monitors, each once); hold last-good manpower MARKED; province flagged not rendered; never clamp |
| `fx_anomaly_hour_gap` | HourGap | the mid-recording snapshot at tick 6 is dropped (series jumps by 2) | adjacent recorded hours differ by >1 (`AnomalyPolicy::gaps`) | log once; `classify_hour(missing) → GapUnavailable`; scrubs as unavailable, **never interpolated** |
| `fx_anomaly_identity_mutation` | IdentityMutation | `provenance.mod.modset_id` + `save_version` change at hour 6 (tick-0 keeps the manifest identity, so the container still opens — the reader's identity cross-check is representative/first-snapshot only) | identity fingerprint `(save_version, provenance.mod.modset_id)` differs from the first frame's | log once; hold the baseline identity MARKED; never adopt the mutated identity |

## Policy design (consumer seam, `replay_app/src/anomaly.rs`)

- `AnomalyPolicy` is a **thin consumer-seam** component. It reads only the typed
  `replay_core::WorldFrame` surface — `ViewStatus` per view, `Leaf` value
  trichotomy, `coverage_warnings()`, `save_version()`, and raw `domain()` for
  `provenance` — and never touches recording bytes or `replay_core` internals.
- **Log**: each anomaly episode emits exactly one structured `Diagnostic`
  (`class`, `hour`, `monitor`, `detail`) via `Diagnostic::log_line()`. It fires
  on the *transition* into the anomaly, not every hour it persists; a monitor
  that recovers (view back to `Present`, value back in range) clears so a later
  re-break re-logs.
- **Hold last-good, MARKED**: `Hold { class, monitor, since_hour, marker }`,
  where `marker` is e.g. `HELD weather from hour 41`. `active_holds_at(hour)`
  serves the marked holds for any (scrub-reachable) hour. A held value is never
  presented as live.
- **Never invent**: out-of-range values are flagged, never clamped; an
  hour-gap hour resolves to `HourResolution::GapUnavailable` (never
  interpolated); a domain that never had a good value holds nothing.
- **Wiring**: `timeline.rs::build_provider` scans the recording at open
  (`scan_timeline`), logs every mid-stream diagnostic to stderr, and stores the
  policy in `RecordingTimeline`. The playback loop surfaces the marked holds +
  gap-unavailability through a new fail-safe default `OwnerProvider::anomaly_note(hour)`
  appended to the app status line (empty on clean recordings). The known-province
  set for the out-of-range province check comes from the bundle's
  `province_to_state` map.

## Gate tests (`replay_app/src/anomaly.rs`, inline `#[cfg(test)]`)

8 tests, all green:
- `all_anomaly_fixtures_open_and_play_without_panic` — every anomaly fixture
  OPENS (degrades, not rejects) and playback crosses every recorded hour.
- one test per class: the diagnostic **fires exactly once**, at the **crossing
  hour**, with the **right classification & monitor**; the held value is
  **MARKED** and dated from the last-good hour; **pre-anomaly hours have no
  holds** (unaffected).
- `hour_gap_is_unavailable_not_interpolated` — recorded hours resolve `Exact`,
  the missing hour resolves `GapUnavailable` with the surrounding hours.
- `clean_positive_fixture_raises_no_anomalies` — no false positives on a
  well-formed positive fixture (empty diagnostics, no holds).
- `diagnostic_log_line_is_structured`.

## `replay_core` API asks

**None.** The full policy is implementable at the consumer seam with today's
`replay_core` surface — `ViewStatus` (DomainAbsent/DomainUnavailable/Present),
the `Leaf` trichotomy, `coverage_warnings()` (wrong-typed values are already
`Unobtained` + a warning), `hours()` (gap detection), `save_version()`, and raw
`domain("provenance")` for the modset fingerprint. No additive API is required
for the post-a5 reconciliation lane. (Optional nicety, not needed: a typed
`provenance.mod.modset_id` accessor would let the identity fingerprint avoid the
one raw-domain read; today's raw read through the public `domain()` accessor is
seam-legal and sufficient.)

## Files touched
- New: `replay_app/src/anomaly.rs` (policy + 8 gate tests),
  `replay_interface/fixtures/anomaly/*.battleplan` (5 seeded fixtures).
- Edited: `replay_fixture_gen/src/main.rs` (anomaly scenarios + `anomaly`
  manifest section; positive/negative generation untouched),
  `replay_interface/fixtures/manifest.json` (additive `anomaly` section),
  `replay_app/src/main.rs` (`mod anomaly`), `replay_app/src/timeline.rs`
  (policy scan at open + `anomaly_note`), `replay_app/src/app.rs` (status-line
  hold marker), `REPLAY_PRODUCTION_PLAN.md` (F4 row → DONE).
- Untouched: `replay_core/*`, `extractor/*`, all `*_product_path_blocked`
  gates, `fixtures/positive/*`, `fixtures/negative/*`.
