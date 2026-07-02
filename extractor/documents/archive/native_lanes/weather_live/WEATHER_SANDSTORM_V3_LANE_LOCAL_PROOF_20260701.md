# Weather Sandstorm V3 Lane-Local Proof - 2026-07-01

Completion status: COMPLETE for lane-local compact/Rust implementation. Whole
weather concept NOT COMPLETE.

Backlog reduced: yes.

Exact backlog item removed or narrowed: `SWeatherPerRegion +0x128` sandstorm is
now implemented in a lane-local domain-24 v3 compact payload and Rust decoder
gate. The shared DLL/recorder registration remains pending; no production
`.battleplan` contains the sandstorm bit yet.

Files changed:

- `extractor/native/lanes/weather_live/domain24_weather_sandstorm_v3.rs`
- `extractor/native/lanes/weather_live/weather_sandstorm_v3_gate.rs`
- `extractor/native/lanes/weather_live/weather_sandstorm_v3_payload_probe.py`
- `extractor/native/lanes/weather_live/SANDSTORM_DOMAIN24_V3_REGISTRATION_PLAN_20260701.md`
- `extractor/native/lanes/weather_live/WEATHER_SANDSTORM_V3_LANE_LOCAL_PROOF_20260701.md`
- `extractor/native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/domain24_weather_sandstorm_v3_payload.bin`
- `extractor/native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/weather_sandstorm_v3_payload_manifest.json`
- `extractor/native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/weather_sandstorm_v3_gate.json`
- `extractor/native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/compact_replay_gate.weather_live.json`

Roots/offsets/functions validated or rejected:

- Validated `CGameState = u64[module_base + 0x3316E90]`.
- Validated `CWeatherManager = u64[CGameState + 0x688]`.
- Validated region vector `CWeatherManager +0x40`, `305/305` rows.
- Validated province vector `CWeatherManager +0x10`, `13414/13414` rows.
- Validated `SWeatherPerRegion +0x128` as sandstorm named flag.
- Validated static function root:
  `CIsSandStorm@CIsFightingInWeatherTrigger` vtable slot reads
  `byte ptr [rdx+0x128]`.
- Lane-local live payload saw sandstorm rows in regions `28`, `225`, and `254`.
- Raw enum-name parity remains blocked for unobserved values `3..7`: static
  table probe found no complete ordered pointer table and live proof only
  observed phenomenon values `0`, `1`, `2`.
- Close-zoom particle/entity parity remains blocked: weather-local vectors and
  static assets are validated, but `+0x70` resolves to `CModifier`/null rather
  than stable weather particle/entity IDs.

Compact schema changes: lane-local v3 implemented. It keeps the existing
domain-24 payload shape: 48-byte header, 32-byte region rows, 32-byte province
rows, reserved header fields zero. It adds `REGION_FLAG_SANDSTORM = 1 << 7`
and decoder unit `weather_region_sandstorm_rows`.

Rust decoder/gate changes: lane-local `domain24_weather_sandstorm_v3.rs` and
`weather_sandstorm_v3_gate.rs` added. The gate requires a nonzero
`weather_region_sandstorm_rows` count and fails on parse errors. The exact
shared registration plan now also covers the recorder-host decoder mask at
`extractor/native/recorder_host/src/main.rs` current line `9181`, a shared
`weather_region_sandstorm_rows` unit count in the same region-row loop, and a
recorder-host test that keeps unknown region bits rejected.

Proof commands/results:

```powershell
python extractor\native\lanes\weather_live\weather_sandstorm_v3_payload_probe.py --pid 45880 --out-dir extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: emitted full domain payload
`domain24_weather_sandstorm_v3_payload.bin`; payload bytes `439056`, region
rows `305`, province rows `13414`, changed rows `13719`, active region rows
`301`, sandstorm region rows `3`.

```powershell
rustc --edition=2021 --test extractor\native\lanes\weather_live\domain24_weather_sandstorm_v3.rs -o extractor\native\target\weather_live_domain24_sandstorm_v3_tests.exe
extractor\native\target\weather_live_domain24_sandstorm_v3_tests.exe --nocapture
rustc --edition=2021 extractor\native\lanes\weather_live\weather_sandstorm_v3_gate.rs -o extractor\native\target\weather_live_sandstorm_v3_gate.exe
extractor\native\target\weather_live_sandstorm_v3_gate.exe extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2\domain24_weather_sandstorm_v3_payload.bin
```

Result: tests `2 passed`; gate decoded payload bytes `439056`, data rows
`13719`, `weather_region_sandstorm_rows=3`, parse errors `[]`.

```powershell
cargo test --manifest-path extractor\native\Cargo.toml -p collector compact_weather --lib --no-fail-fast
```

Result: 2 passed, 0 failed.

```powershell
cargo run --manifest-path extractor\native\lanes\compact_replay_verification_gates\Cargo.toml -- --run-dir extractor\native\runs\weather_visual_drivers_live_870ebf_integrated_20260701_v1 --require-domain weather_live --min-game-hours 5 --write-report extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2\compact_replay_gate.weather_live.json
```

Result: existing production stream gate `passed=true`, errors `[]`,
warnings `[]`; existing `.battleplan` verified for
`native_6124_18be0901914bf9fc`.

DLL-only: no for the new sandstorm bit. The lane-local proof used bounded
Python to emit a payload from live roots; shared DLL registration is still
required.

Compact stream: yes for existing domain 24; yes for lane-local compact payload;
no shared DLL compact stream contains the sandstorm bit yet.

Rust decoded: yes. The lane-local v3 Rust gate decoded the live full-size
payload and counted `weather_region_sandstorm_rows=3`.

`.battleplan` verified: yes for the existing production domain-24 stream; no for
the sandstorm bit because it is not registered into the DLL/recorder stream yet.

Speed-five/current-equivalent hourly-or-lower: yes for the existing production
weather run: `50` game hours advanced, `51` unique hours, hourly requirement met.
No new sandstorm DLL run was captured.

Zero skipped hours: yes for the existing production weather run: `0`. Not
applicable to the lane-local single-payload sandstorm proof.

Partial/source/worker/bridge failures: no for the verified existing production
weather run. The lane-local sandstorm payload path is proof-only and has no
worker/bridge layer.
