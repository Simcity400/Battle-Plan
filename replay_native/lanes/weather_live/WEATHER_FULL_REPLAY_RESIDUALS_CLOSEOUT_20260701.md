# Weather Full Replay Residuals Closeout - 2026-07-01

Completion status: `COMPLETE` for this candidate-rejection/narrowing slice;
whole weather concept `NOT COMPLETE`.

Backlog reduced: yes.

Exact backlog item removed or narrowed: domain 24 no longer has an open
non-visual current-state extraction blocker. `SWeatherPerRegion +0x58/+0x70`
active vectors are rejected as an independent non-visual compact row because the
live probe shows they duplicate the already captured active-weather signal
(`phenomenon >= 1`) for every region row. Weather RNG cursors are rejected as a
domain-24 current-state compact row by the coverage manifest's current-state
boundary. Exact generator/RNG/name parity and close-zoom particle vector
contents remain open-computed/visual scope, not whole-weather complete.

Files changed:

- `replay_native/lanes/weather_live/weather_full_replay_residuals_probe.py`
- `replay_native/lanes/weather_live/README.md`
- `replay_native/lanes/weather_live/WEATHER_FULL_REPLAY_RESIDUALS_CLOSEOUT_20260701.md`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v1/weather_full_replay_residuals_probe.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v1/source_native_session_manifest.weather_visual_integrated.json`
- `replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v1/compact_replay_gate.weather_live.json`

Roots/offsets/functions validated or rejected:

- Validated `CGameState = u64[module_base + 0x3316E90]`.
- Validated `CWeatherManager = u64[CGameState + 0x688]`.
- Validated `CWeatherManager` vtable `module_base + 0x2960BE0`.
- Validated region vector `CWeatherManager + 0x40`, `305/305` rows.
- Validated `SWeatherPerRegion` vtable `module_base + 0x2960B90`, stride
  `0x160`, `region_id +0x08`, active modifiers `+0x30`, active-front vectors
  `+0x58`, weather-effect vectors `+0x70`, phenomenon `+0x78`, intensity
  `+0x5D`, named low-byte flags `+0x108/+0x110/+0x118/+0x120/+0x130`,
  temperature `+0x138`, next-change raw `+0x150`, accumulator candidate
  `+0x28`.
- Validated province vector `CWeatherManager + 0x10`, `13414/13414` rows.
- Validated `SWeatherPerProvince` vtable `module_base + 0x2960B40`, stride
  `0x180`, `province_id +0x08`, water `+0x38`, snow `+0x40`, displayed
  temperature `+0x118`, temperature change `+0x128`, mud byte `+0x130`, custom
  modifiers `+0x148`.
- Rejected `SWeatherPerRegion +0x58/+0x70` active vectors as an independent
  non-visual current-state row: `305/305` agreement with captured
  `phenomenon >= 1`; active signal rows `301`, phenomenon-active rows `301`.
- Rejected `weather.current_province/current_region/seed` as a domain-24
  current-state compact row: coverage manifest classes it
  `engine-internal`, `excluded-by-scope`, note `weather RNG state cursors`.
- Did not reject exact weather generator/formula/name parity as impossible:
  coverage manifest keeps `weather temperature formula / RNG /
  phenomenon-name strings` as `open-computed`. This remains a separate
  Ghidra/recompute target if product scope requires exact generator parity.

Compact schema changes: none. Domain 24 v2 already carries the realized
current-state and map-visible driver rows needed for this slice: region rows
plus province visual rows with temperature, temperature change, water, snow,
mud, and custom-modifier count.

Rust decoder/gate changes: none. Existing lane decoder/gate and shared compact
gate passed against the current domain-24 payload and replay container.

Proof commands/results:

```powershell
python replay_native\lanes\weather_live\weather_full_replay_residuals_probe.py --pid 45880 --out-dir replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v1
```

Result: `candidate_valid = true`, errors `[]`. Live proof validated build
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`, 305 region
rows, 13,414 province rows, `305/305` active-vector agreement, 13,414 map
recolor driver rows, and 13,414 dynamic LUT driver rows.

```powershell
cargo run --manifest-path replay_native\lanes\compact_replay_verification_gates\Cargo.toml -- --run-dir replay_native\runs\weather_visual_drivers_live_870ebf_integrated_20260701_v1 --require-domain weather_live --min-game-hours 5 --write-report replay_native\runs\weather_live_full_replay_residuals_870ebf_20260701_v1\compact_replay_gate.weather_live.json
```

Result: gate `passed = true`, errors `[]`, warnings `[]`; verified
`native_session.battleplan` for game id `native_6124_18be0901914bf9fc`.

```powershell
rustc --edition=2021 --test replay_native\lanes\weather_visual_drivers_live\domain24_weather_visual_v2.rs -o replay_native\target\weather_visual_drivers_live_domain24_tests.exe
replay_native\target\weather_visual_drivers_live_domain24_tests.exe --nocapture
```

Result: 4 passed, 0 failed.

```powershell
rustc --edition=2021 replay_native\lanes\weather_visual_drivers_live\weather_visual_gate.rs -o replay_native\target\weather_visual_drivers_live_gate.exe
replay_native\target\weather_visual_drivers_live_gate.exe replay_native\runs\weather_visual_drivers_live_870ebf_20260701_v2\domain24_weather_visual_v2_payload.bin
```

Result: decoded `439056` bytes, `13719` rows, `13414` province visual rows,
parse errors `[]`.

```powershell
cargo test --manifest-path replay_native\Cargo.toml -p collector compact_weather --lib --no-fail-fast
```

Result: 2 passed, 0 failed.

DLL-only: yes for production weather data. The Python probe was bounded
read-only RE/proof only, not a runtime source or hot-loop path.

Compact stream: yes.

Rust decoded: yes.

`.battleplan` verified: yes.

Speed-five/current-equivalent hourly-or-lower: yes. Integrated manifest advanced
50 game hours with 51 unique hours and met the hourly requirement.

Zero skipped hours: yes.

Any partial/source/worker/bridge failures: no.
