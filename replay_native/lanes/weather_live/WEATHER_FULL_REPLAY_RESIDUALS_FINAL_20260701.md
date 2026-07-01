# Weather Full Replay Residuals Final Report - 2026-07-01

Completion status: COMPLETE.

This is COMPLETE only by residual resolution: each remaining weather residual is
now either production-covered by the existing domain-24 compact stream, rejected
as not a separate replay-current-state field, or blocked with exact root/fixture
evidence. This does not claim whole weather visual/particle parity is production
captured.

Backlog reduced: yes.

Exact backlog item removed or narrowed:

- Generator/RNG parity narrowed: realized replay weather outputs are production
  captured directly; exact future-determinism RNG/formula replay is not required
  for current-frame replay and remains blocked on a controlled weather-update RNG
  fixture or static trace of the update function.
- Phenomenon name parity narrowed: static names and current-build predicate RTTI
  are validated, but live observation only covered enum values `0`, `1`, and `2`.
  Enum values `3`..`7` are badged until a controlled `set_weather` sweep or
  static `CIs*` predicate compare trace maps them exactly.
- Close-zoom particle/vector visual parity narrowed to an exact blocker: static
  weather particle assets and weather-local vector headers exist, but the live
  payloads at `SWeatherPerRegion +0x58/+0x70` and
  `SWeatherPerProvince +0x58/+0x70` do not expose stable weather particle/entity
  IDs. `+0x70` resolves to `CModifier`/null in the region sample, not particle
  entities. A render/entity root or controlled close-zoom fixture is required.
- Remaining map-visible weather driver residual rejected: domain 24 already
  carries the realized province drivers used by the map-tone layer: temperature,
  temperature change, water, snow, mud, custom modifier count, map recolor driver
  bit, and dynamic LUT driver bit. Close-zoom particles are separate from this
  map-tone driver closure and remain blocked as above.

Files changed:

- `replay_native/lanes/weather_live/README.md`
- `replay_native/lanes/weather_live/WEATHER_FULL_REPLAY_RESIDUALS_CLOSEOUT_20260701.md`
- `replay_native/lanes/weather_live/WEATHER_FULL_REPLAY_RESIDUALS_FINAL_20260701.md`
- `replay_native/lanes/weather_live/weather_full_replay_residuals_probe.py`
- `replay_native/lanes/weather_live/weather_generator_name_probe.py`
- `replay_native/lanes/weather_live/weather_manager_vector_inventory.py`
- `replay_native/lanes/weather_live/weather_particle_vector_probe.py`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v1/compact_replay_gate.weather_live.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v1/source_native_session_manifest.weather_visual_integrated.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v1/weather_full_replay_residuals_probe.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/compact_replay_gate.weather_live.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/weather_generator_name_probe.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/weather_manager_vector_inventory.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/weather_particle_vector_probe.json`

Roots/offsets/functions validated or rejected:

- Validated build gate:
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- Validated `CGameState = u64[module_base + 0x3316E90]`.
- Validated `CWeatherManager = u64[CGameState + 0x688]`.
- Validated `CWeatherManager` vtable `module_base + 0x2960BE0`.
- Validated region vector `CWeatherManager + 0x40`, `305/305` rows.
- Validated `SWeatherPerRegion` vtable `module_base + 0x2960B90`, stride
  `0x160`, `region_id +0x08`, active modifiers `+0x30`, active-front vectors
  `+0x58`, effect/modifier vectors `+0x70`, phenomenon `+0x78`, intensity
  `+0x5D`, named low-byte flags `+0x108/+0x110/+0x118/+0x120/+0x130`,
  temperature `+0x138`, next-change raw `+0x150`, accumulator candidate
  `+0x28`.
- Validated province vector `CWeatherManager +0x10`, `13414/13414` rows.
- Validated `SWeatherPerProvince` vtable `module_base + 0x2960B40`, stride
  `0x180`, `province_id +0x08`, water `+0x38`, snow `+0x40`, displayed
  temperature `+0x118`, temperature change `+0x128`, mud byte `+0x130`, custom
  modifiers `+0x148`, local-front vectors `+0x58`, local effect/modifier vectors
  `+0x70`.
- Validated generated province outputs live: rows `13414`, temperature rows
  `13413`, temperature-change rows `13413`, water rows `13414`, snow rows
  `13414`, mud measured rows `13414`, mud active rows `289`.
- Validated current-frame region phenomenon counts: `0=4`, `1=287`, `2=14`.
- Validated static weather name hypothesis from `common/weather.txt`:
  `0=no_phenomenon`, `1=rain_light`, `2=rain_heavy`, `3=snow`,
  `4=blizzard`, `5=arctic_water`, `6=mud`, `7=sandstorm`.
- Validated current-build weather predicate/function RTTI strings, including
  `CIsLightRain`, `CIsHeavyRain`, `CIsSnow`, `CIsBlizzard`, `CIsArticWater`,
  `CIsMud`, `CIsSandStorm`, `CWeatherManager`, `SWeatherPerRegion`,
  `SWeatherPerProvince`, `SLocalWeatherPosition`, and
  `CRandomizeWeatherEffect`.
- Rejected `SWeatherPerRegion +0x58/+0x70` as an independent non-visual
  current-state compact row: prior proof showed `305/305` active-vector
  agreement with captured `phenomenon >= 1`.
- Rejected `SWeatherPerRegion +0x70` / `SWeatherPerProvince +0x70` as stable
  close-zoom weather particle entity identity roots for current build:
  region payload had `315` records with type counts `CModifier=301`, `0x0=14`,
  and `0` clean weather/particle entity RTTI records.
- Validated static close-zoom assets: `map/weatherpositions.txt` has `588` rows
  over `287` regions; `common/weather.txt` references weather effect entities;
  `weather_entities.asset` has `17` entities; `particles.gfx` has `173`
  particles; missing weather effect entities `[]`; missing entity particles
  `[]`.
- Validated `CWeatherManager` vector scan over `0x2000` bytes: known arrays at
  `+0x10` and `+0x40`, `56` vector-like candidates total, no clean
  weather-particle/entity container. `CWeatherTerrainModifier` RTTI appeared in
  heuristic vector-like samples but not as an uncovered particle root.
- Validated map-visible driver closure through existing decoder rules:
  province flags set `MAP_RECOLOR_DRIVER` from temperature/water/snow and
  `DYNAMIC_LUT_DRIVER` from snow/mud; the live integrated manifest recorded
  `42028` map recolor driver rows and `42028` dynamic LUT driver rows.

Compact schema changes: none shipped. Domain 24 v2 remains the production
compact schema for realized weather state and map-visible drivers. No particle
tail was added because the remaining particle vector payloads lack stable
entity IDs and would otherwise serialize volatile pointers/raw records.

Rust decoder/gate changes: none shipped. Existing domain-24 Rust decoder/gate
was rerun and decoded the weather compact payload successfully. If the particle
blocker is later removed, the exact registration patch plan is:

- Add a domain-24 v3 tail or sibling record after stable particle/entity IDs are
  available; do not serialize raw heap pointers as durable replay identity.
- Extend the lane decoder to parse stable particle rows only after the row can
  carry region/province scope, stable asset/entity token, placement index, and
  flags.
- Register the v3 tail in the single collector/coordinator and recorder host
  only after a fixture proves the render/entity root mapping.
- Extend compact verification gates with required particle units and keep the
  current v2 map driver gate unchanged for non-particle weather.

Proof commands/results:

```powershell
python replay_native\lanes\weather_live\weather_full_replay_residuals_probe.py --pid 45880 --out-dir replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v1
```

Result: `candidate_valid=true`, errors `[]`, `305` region rows, `13414`
province rows, `305/305` active-vector agreement, `13414` map recolor driver
rows, `13414` dynamic LUT driver rows.

```powershell
python replay_native\lanes\weather_live\weather_generator_name_probe.py --pid 45880 --out-dir replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: errors `[]`; generated outputs classified
`resolved_by_direct_capture`; exact RNG classified blocked for future
determinism without a controlled RNG/update-function fixture; name parity
classified `partial_current_frame_only` with unobserved values `3,4,5,6,7`.

```powershell
python replay_native\lanes\weather_live\weather_particle_vector_probe.py --pid 45880 --out-dir replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: errors `[]`; static assets validated; region front records `5614`;
region `+0x70` records `315`; type counts `CModifier=301`, `0x0=14`; clean
weather/particle entity records `0`; province front/effect vector rows `12691`.
Classification:
`static_assets_and_vector_headers_validated_but_stable_particle_entity_semantics_blocked`.

```powershell
python replay_native\lanes\weather_live\weather_manager_vector_inventory.py --pid 45880 --scan-bytes 0x2000 --out-dir replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: errors `[]`; roots validated; candidate count `56`; known arrays at
`CWeatherManager +0x10` and `+0x40`; no stable weather particle/entity root in
the manager scan.

```powershell
cargo run --manifest-path replay_native\lanes\compact_replay_verification_gates\Cargo.toml -- --run-dir replay_native\runs\weather_visual_drivers_live_870ebf_integrated_20260701_v1 --require-domain weather_live --min-game-hours 5 --write-report replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2\compact_replay_gate.weather_live.json
```

Result: `passed=true`, errors `[]`, warnings `[]`; verified
`native_session.battleplan` for game id `native_6124_18be0901914bf9fc`;
collector compact stream bytes `1368136`.

```powershell
rustc --edition=2021 --test replay_native\lanes\weather_visual_drivers_live\domain24_weather_visual_v2.rs -o replay_native\target\weather_visual_drivers_live_domain24_tests.exe
replay_native\target\weather_visual_drivers_live_domain24_tests.exe --nocapture
rustc --edition=2021 replay_native\lanes\weather_visual_drivers_live\weather_visual_gate.rs -o replay_native\target\weather_visual_drivers_live_gate.exe
replay_native\target\weather_visual_drivers_live_gate.exe replay_native\runs\weather_visual_drivers_live_870ebf_20260701_v2\domain24_weather_visual_v2_payload.bin
```

Result: tests `4 passed`; payload gate decoded `439056` bytes, `13719` rows,
`13414` province visual rows, parse errors `[]`.

```powershell
cargo test --manifest-path replay_native\Cargo.toml -p collector compact_weather --lib --no-fail-fast
```

Result: failed before weather execution due current shared `collector/src/lib.rs`
initializer drift outside this lane: missing
`observed_targeted_timed_decision_rows`, `targeted_timed_live`,
`targeted_timed_live_count`, and `CountryDecisionsRow.targeted_timed_live`.
This lane did not edit `collector/src/lib.rs`.

DLL-only: yes for production replay weather data. The Python files are bounded
RE/proof probes only, not hot-loop replay sources.

Compact stream: yes.

Rust decoded: yes for the existing domain-24 production payload; no new
particle/name compact rows were added because those residuals are blocked or
rejected as described above.

`.battleplan` verified: yes.

Speed-five/current-equivalent hourly-or-lower: yes. The integrated manifest
advanced `50` game hours with `51` unique hours and met the hourly requirement.

Zero skipped hours: yes.

Partial/source/worker/bridge failures: no in the verified integrated live run:
`source_unavailable_seen=false`, `partial_data_seen=false`,
`bridge_unavailable_seen=false`, `worker_panic_seen=false`. The current broad
collector test compile failure is a separate shared-code compile blocker, not a
live source/worker/bridge failure from this weather run.
