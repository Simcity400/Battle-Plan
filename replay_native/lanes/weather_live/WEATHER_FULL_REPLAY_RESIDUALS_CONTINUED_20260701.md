# Weather Full Replay Residuals Continued Report - 2026-07-01

Completion status: COMPLETE by blocker removal. Production capture for the newly
validated sandstorm flag is NOT shipped until the shared domain-24 v3 patch is
registered.

Backlog reduced: yes.

Exact backlog item removed or narrowed: the remaining "map-visible weather
driver" residual is no longer a broad unknown. Province map-tone weather drivers
remain covered by existing domain 24, but a named region sandstorm flag was
validated at `SWeatherPerRegion +0x128`. The blocker is removed: implementation
can proceed from exact root/offset/function evidence with no more discovery.
Generator/RNG exact future-determinism remains rejected for current-frame replay
because realized outputs are captured directly. Raw phenomenon enum constants
for unobserved values `3..7` remain blocked by lack of an enum-compare trace or
controlled `set_weather` fixture. Close-zoom particle/vector parity remains
blocked on stable render/entity identity mapping.

Files changed:

- `replay_native/lanes/weather_live/README.md`
- `replay_native/lanes/weather_live/SANDSTORM_DOMAIN24_V3_REGISTRATION_PLAN_20260701.md`
- `replay_native/lanes/weather_live/WEATHER_FULL_REPLAY_RESIDUALS_CONTINUED_20260701.md`
- `replay_native/lanes/weather_live/WEATHER_FULL_REPLAY_RESIDUALS_FINAL_20260701.md`
- `replay_native/lanes/weather_live/domain24_weather_sandstorm_v3.rs`
- `replay_native/lanes/weather_live/weather_full_replay_residuals_probe.py`
- `replay_native/lanes/weather_live/weather_generator_name_probe.py`
- `replay_native/lanes/weather_live/weather_name_static_table_probe.py`
- `replay_native/lanes/weather_live/weather_predicate_static_probe.py`
- `replay_native/lanes/weather_live/weather_manager_vector_inventory.py`
- `replay_native/lanes/weather_live/weather_particle_vector_probe.py`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/compact_replay_gate.weather_live.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/source_native_session_manifest.weather_visual_integrated.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/weather_full_replay_residuals_probe.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/weather_generator_name_probe.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/weather_manager_vector_inventory.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/weather_name_static_table_probe.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/weather_particle_vector_probe.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/weather_predicate_static_probe.json`

Roots/offsets/functions validated or rejected:

- Validated build:
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- Validated `CGameState = u64[module_base + 0x3316E90]`.
- Validated `CWeatherManager = u64[CGameState + 0x688]`.
- Validated `CWeatherManager` vtable `module_base +0x2960BE0`.
- Validated region vector `CWeatherManager +0x40`, `305/305` rows.
- Validated `SWeatherPerRegion` vtable `module_base +0x2960B90`, stride
  `0x160`, phenomenon `+0x78`, rain light `+0x108`, rain heavy `+0x110`,
  snow `+0x118`, blizzard `+0x120`, sandstorm `+0x128`, arctic water
  `+0x130`, temperature `+0x138`, next-change raw `+0x150`.
- Validated current-build static predicate RTTI/vtables for `CIsLightRain`,
  `CIsHeavyRain`, `CIsSnow`, `CIsBlizzard`, `CIsArticWater`,
  `CIsSandStorm`, and `CIsMud`.
- Validated static predicate flag offsets:
  rain light `0x108`, rain heavy `0x110`, snow `0x118`, blizzard `0x120`,
  sandstorm `0x128`, arctic water `0x130`.
- Validated live sandstorm rows: `named_flag_counts.sandstorm = 1`.
- Rejected exact raw phenomenon enum-name parity as complete from current proof:
  live values only observed `0`, `1`, `2`, and the static table probe found no
  complete ordered engine pointer table. Static `common/weather.txt` file order
  remains a hypothesis, not sufficient by itself for values `3..7`.
- Rejected `+0x58/+0x70` as stable close-zoom particle identity roots:
  region `+0x70` resolved to `CModifier=301`, `0x0=14`, and `0` clean
  weather/particle RTTI records.
- Validated close-zoom static assets but blocked particle production mapping:
  `weatherpositions.txt` rows `588`, weather entities `17`, particles `173`,
  missing weather effect entities `[]`, missing entity particles `[]`.

Compact schema changes: none shipped to shared production code. Lane-local v3
schema is specified in `domain24_weather_sandstorm_v3.rs`: add
`REGION_FLAG_SANDSTORM = 1 << 7`, keep 32-byte region rows and 48-byte headers,
keep reserved header fields zero, and add decoder unit
`weather_region_sandstorm_rows`.

Rust decoder/gate changes: lane-local decoder fixture added and verified. Shared
decoder/gate patch plan is in
`SANDSTORM_DOMAIN24_V3_REGISTRATION_PLAN_20260701.md`.

Proof commands/results:

```powershell
python replay_native\lanes\weather_live\weather_full_replay_residuals_probe.py --pid 45880 --out-dir replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: `candidate_valid=true`, errors `[]`, `SWeatherPerRegion_sandstorm_flag_off=0x128`,
`named_flag_counts.sandstorm=1`, `305/305` active-vector agreement,
`13414` province map recolor driver rows, `13414` dynamic LUT driver rows.

```powershell
python replay_native\lanes\weather_live\weather_predicate_static_probe.py --out-dir replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: errors `[]`; `predicate_rtti_vtables_located=7`;
`expected_region_flag_offsets_seen` includes sandstorm `0x128`;
classification `predicate_region_flag_offsets_validated_enum_constants_not_fully_proven`.

```powershell
python replay_native\lanes\weather_live\weather_name_static_table_probe.py --out-dir replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: errors `[]`; no complete ordered engine pointer table found;
classification `not_resolved`.

```powershell
python replay_native\lanes\weather_live\weather_generator_name_probe.py --pid 45880 --out-dir replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: errors `[]`; realized generated outputs classified
`resolved_by_direct_capture`; raw phenomenon enum names remain
`partial_current_frame_only`; unobserved enum values `3,4,5,6,7`.

```powershell
python replay_native\lanes\weather_live\weather_particle_vector_probe.py --pid 45880 --out-dir replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
python replay_native\lanes\weather_live\weather_manager_vector_inventory.py --pid 45880 --scan-bytes 0x2000 --out-dir replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: errors `[]`; particle assets and weather-local vectors validated, but
stable particle/entity semantics blocked; manager scan found known weather
arrays and no clean weather particle/entity root.

```powershell
rustc --edition=2021 --test replay_native\lanes\weather_live\domain24_weather_sandstorm_v3.rs -o replay_native\target\weather_live_domain24_sandstorm_v3_tests.exe
replay_native\target\weather_live_domain24_sandstorm_v3_tests.exe --nocapture
```

Result: 2 passed, 0 failed.

```powershell
cargo test --manifest-path replay_native\Cargo.toml -p collector compact_weather --lib --no-fail-fast
```

Result: 2 passed, 0 failed.

```powershell
cargo run --manifest-path replay_native\lanes\compact_replay_verification_gates\Cargo.toml -- --run-dir replay_native\runs\weather_visual_drivers_live_870ebf_integrated_20260701_v1 --require-domain weather_live --min-game-hours 5 --write-report replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2\compact_replay_gate.weather_live.json
```

Result: `passed=true`, errors `[]`, warnings `[]`; verified
`native_session.battleplan` for `native_6124_18be0901914bf9fc`.

DLL-only: yes for the existing production weather stream; the new sandstorm
flag is validated for DLL capture but not emitted until the shared v3 patch is
registered.

Compact stream: yes for existing domain 24; no for the newly validated
sandstorm bit until v3 registration.

Rust decoded: yes for existing domain 24 and yes for the lane-local v3
sandstorm decoder fixture; no shared v3 stream decoded yet.

`.battleplan` verified: yes for the existing domain-24 production stream; no
new `.battleplan` contains the sandstorm bit yet.

Speed-five/current-equivalent hourly-or-lower: yes for the existing production
weather run: `50` game hours advanced, `51` unique hours, hourly requirement met.

Zero skipped hours: yes for the existing production weather run: `0`.

Partial/source/worker/bridge failures: no for the verified integrated weather
run: `source_unavailable_seen=false`, `partial_data_seen=false`,
`bridge_unavailable_seen=false`, `worker_panic_seen=false`.
