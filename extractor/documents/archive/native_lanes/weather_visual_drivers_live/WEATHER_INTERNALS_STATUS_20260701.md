# Weather Internals Status - 2026-07-01

This note is lane-local status for domain 24 `weather_live` /
`weather_visual_drivers_live`. It does not mark the whole weather concept
complete. The accepted production slice is the realized region/province weather
state and visual map drivers already proven through the compact stream.

## Status

- Completion status: `COMPLETE` for the residual internals audit lane by exact
  candidate rejection/narrowing; `NOT COMPLETE` for the whole weather concept.
- Validated slice: realized weather state plus province visual drivers.
- Backlog reduced: yes.
- Exact backlog item narrowed: non-visual weather internals are no longer a
  compact domain-24 extraction blocker for current-state replay. Remaining
  unresolved work is visual/computed parity only: exact close-zoom weather
  particles if product scope requires them, and exact generative
  formula/RNG/name mapping if the product requires reconstructing the engine's
  weather generator rather than replaying captured realized frames.
- Detailed required report fields live in
  `RESIDUAL_INTERNALS_AUDIT_20260701.md`.

## Evidence

Existing integrated proof:

- `extractor/native/runs/weather_visual_drivers_live_870ebf_integrated_20260701_v1/native_session_manifest.json`
  proves domain 24 `weather_live` emitted 51 data records over 50 advanced game
  hours with `skipped_game_hours = 0`, ordered/decodable compact records,
  `source_unavailable_seen = false`, `partial_data_seen = false`,
  `bridge_unavailable_seen = false`, `worker_panic_seen = false`, and
  `.battleplan` output.
- The same manifest reports 684,114 observed province rows, 42,028 emitted
  province visual rows, 42,027 temperature/temp-change rows, 42,028 water/snow
  rows, 42,028 mud-measured rows, 250 mud-active rows, 42,028 custom-modifier
  rows, p95 weather read time 10,749 us, and `native_session.battleplan`.
- `weather_province_probe_870ebf.json` validates the live current-build root:
  `CGameState = u64[module_base + 0x3316E90]`,
  `CWeatherManager = u64[CGameState + 0x688]`, manager vtable
  `module_base + 0x2960BE0`, region vector `CWeatherManager + 0x40`, province
  vector `CWeatherManager + 0x10`, 305 region rows, and 13,414 province rows.
- That probe validates `SWeatherPerProvince` vtable `module_base + 0x2960B40`
  for 13,414/13,414 rows, `province_id == index` for 13,414/13,414 rows, and
  sane/current values for `water_raw +0x38`, `snow_raw +0x40`,
  `temperature_raw +0x118`, `temperature_change_raw +0x128`, `mud +0x130`,
  and `custom_modifiers +0x148`.

New lane-local internals probe:

- `probe_weather_internals_870ebf.py` validates region active-event vectors on
  the same root without touching shared entrypoints.
- It checks `SWeatherPerRegion +0x58` active-front-position vectors and
  `SWeatherPerRegion +0x70` weather-effect-entity vectors against
  `SWeatherPerRegion +0x78` phenomenon.
- As with the lane's province custom-modifier vectors, the active-front vector
  third dword is not treated as authoritative capacity on `870ebf`; validation
  uses heap begin plus bounded size.
- Current live result is in `proofs/weather_internals_870ebf_20260701.json`.
  It validates the current root and classifies those vectors as not an
  independent non-visual gameplay state when their active signal agrees with
  `phenomenon >= 1` for every region row. They remain visual/local-front
  particle detail, so exact close-zoom particle parity is a separate visual
  lane if it enters product scope.
- The current proof measured phenomenon counts `{0: 4, 1: 287, 2: 14}`,
  `active_signal_rows = 301`, `phenomenon_active_rows = 301`, and
  `phenomenon_active_vector_agree = 305`. It also measured 5,598 region
  active-front-position entries, 315 region weather-effect-entity entries, and
  10 active-modifier entries. The per-region accumulator candidate at
  `SWeatherPerRegion +0x28` ranged from `0` to `27,900,000`; it is treated as a
  future-generation/phase value, not an independent current-state replay field,
  because the compact stream already captures the realized phenomenon, named
  flags, active modifier count, and next-change raw value.
- Province-local `+0x58/+0x70` vectors are real and currently common
  (`12,642/13,414` province rows had local front/effect vector presence, with
  111,731 local front-position entries and 18,906 local effect-entity entries).
  They are not rejected as impossible. They are scoped as visual particle/local
  front detail, not a non-visual weather-system cause/effect.

Coverage-audit evidence:

- `coverage_audit/memory_manifest.json` classifies
  `weather.current_province/current_region/seed` as `engine-internal`,
  `excluded-by-scope`, note `weather RNG state cursors`.
- The same manifest keeps `weather temperature formula / RNG /
  phenomenon-name strings` as `open-computed`, with realized region
  temperature/phenomenon already save-persisted and live-captured while the
  generator function remains a Ghidra/recompute target.
- `coverage_audit/COVERAGE_REPORT.md` explicitly scopes RNG seeds/name cursors
  out of current-state visible replay and says they matter only for
  future-determinism/continuing the game identically.
- `extractor/native/render/src/dynamic.rs` documents that weather particles are a
  separate particle/entity system, not the terrain shader branch. The map-tone
  weather effect flows through snow/mud inputs, which the validated compact
  province rows now carry.

## Rejection / Narrowing

- Rejected as a required compact extraction row for current-state replay:
  `weather.current_province/current_region/seed` and generic RNG cursors. These
  are future-determinism inputs, not required to replay already captured
  realized hourly-or-better frames.
- Rejected as independent non-visual gameplay state when the current-build live
  probe validates 305/305 agreement: region active-event vector presence at
  `SWeatherPerRegion +0x58/+0x70` duplicates the captured
  `phenomenon >= 1` active signal. The vector contents are not discarded as
  impossible; they are scoped as visual/local-front particle detail.
- Rejected as a current-state replay extraction blocker: the per-region
  accumulator/phase candidate at `SWeatherPerRegion +0x28`. It is preserved as
  exact evidence in the proof, but not promoted to compact schema because no
  player-visible/current gameplay state is missing once realized phenomenon,
  next change, modifiers, and province scalars are captured hourly-or-better.
- Not rejected and not complete: exact generative formula/RNG/name mapping. This
  remains `open-computed` if the product needs exact engine-generator replay or
  exact enum-name proof. It is not needed to replay captured realized state.

## Proof Commands

```powershell
python extractor\native\lanes\weather_visual_drivers_live\probe_weather_internals_870ebf.py
rustc --edition=2021 --test extractor\native\lanes\weather_visual_drivers_live\domain24_weather_visual_v2.rs -o extractor\native\target\weather_visual_drivers_live_domain24_tests.exe
extractor\native\target\weather_visual_drivers_live_domain24_tests.exe --nocapture
rustc --edition=2021 extractor\native\lanes\weather_visual_drivers_live\weather_visual_gate.rs -o extractor\native\target\weather_visual_drivers_live_gate.exe
extractor\native\target\weather_visual_drivers_live_gate.exe extractor\native\runs\weather_visual_drivers_live_870ebf_20260701_v2\domain24_weather_visual_v2_payload.bin
cargo test --manifest-path extractor/native/Cargo.toml -p collector compact_weather --lib --no-fail-fast
cargo run --manifest-path extractor/native/Cargo.toml -p snapshot --bin pack_replay -- verify extractor/native/runs/weather_visual_drivers_live_870ebf_integrated_20260701_v1/native_session.battleplan extractor/native/runs/weather_visual_drivers_live_870ebf_integrated_20260701_v1 native_session
```

Observed results on 2026-07-01:

- Internals probe: current root valid; output
  `proofs/weather_internals_870ebf_20260701.json`.
- Lane decoder tests: 4 passed, 0 failed.
- Lane payload gate: 439,056-byte payload decoded, 13,719 rows, 13,414
  observed province rows, no parse errors.
- Collector weather tests: 2 passed, 0 failed.
- `.battleplan` verify: `verify: OK` for
  `native_6124_18be0901914bf9fc`.
