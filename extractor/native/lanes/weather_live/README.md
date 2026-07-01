# weather_live residuals lane

Lane-local proof package for domain 24 `weather_live` residuals. This lane does
not claim whole-weather completion. The newest superseding residual report is
`WEATHER_REMAINING_BLOCKERS_CLOSED_20260701.md`; the older closeout files are
accepted backlog-reduction slices only. The current acceptance boundary keeps
shared sandstorm registration/proof orchestrator-owned and keeps the whole
weather concept/domain 24 marked not complete. The lane narrows the user's audit
from "weather is only visual drivers" to:

- production path still lives in the existing DLL compact `weather_live` stream;
- realized region/province weather state and map-visible snow/mud/frozen drivers
  are already represented by domain 24;
- non-visual current-state residuals do not require a new compact row in this
  lane;
- `SWeatherPerRegion +0x128` has a lane-local compact/Rust v3 implementation
  and exact shared registration plan; shared production acceptance remains
  orchestrator-owned;
- exact generator/RNG/name parity and close-zoom particle vector contents are
  either rejected as replay-current-state requirements or blocked with exact
  fixture/root evidence.

## Probe

Run the bounded live probe:

```powershell
python extractor\native\lanes\weather_live\weather_full_replay_residuals_probe.py --pid 45880 --out-dir extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v1
```

The probe is read-only. It validates current-build roots and offsets from the
running `hoi4.exe`, reads no save/Python source in a hot loop, and writes proof
JSON under `extractor/native/runs/weather_live_*`.

Validated on build
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`:

- `CGameState = u64[module_base + 0x3316E90]`.
- `CWeatherManager = u64[CGameState + 0x688]`.
- `CWeatherManager` vtable `module_base + 0x2960BE0`.
- Region vector `CWeatherManager + 0x40`, `305/305` rows.
- `SWeatherPerRegion` vtable `module_base + 0x2960B90`, stride `0x160`.
- Province vector `CWeatherManager + 0x10`, `13414/13414` rows.
- `SWeatherPerProvince` vtable `module_base + 0x2960B40`, stride `0x180`.

Key residual evidence:

- `SWeatherPerRegion +0x58/+0x70` active vectors agreed with
  `phenomenon >= 1` for `305/305` region rows.
- Region active signal rows: `301`; phenomenon-active rows: `301`.
- Province visual driver rows validated live: `13414`.
- Map recolor driver rows validated live: `13414`.
- Dynamic LUT driver rows validated live: `13414`.

## Production proof

Historical shared-stream proof recorded in this lane directory. Do not use this
section alone to mark the current subagent lane complete; the current user
acceptance boundary above keeps shared sandstorm registration/proof
orchestrator-owned.

```powershell
cargo run --manifest-path extractor\native\lanes\compact_replay_verification_gates\Cargo.toml --bin compact_replay_gate -- --run-dir extractor\native\runs\weather_live_sandstorm_shared_20260701T185714 --require-domain weather_live --min-game-hours 5 --write-report extractor\native\runs\weather_live_sandstorm_shared_20260701T185714\compact_gate_report.json
cargo run --manifest-path extractor\native\snapshot\Cargo.toml --bin pack_replay -- verify extractor\native\runs\weather_live_sandstorm_shared_20260701T185714\native_session.battleplan
```

Result: gate passed with no errors or warnings, `pack_replay verify` returned
OK, and the manifest recorded `weather_region_sandstorm_rows = 1`,
`33` game hours advanced, `34` unique game-hour samples, `0` skipped game
hours, and no source/partial/worker/bridge failures.

Additional lane decoder checks:

```powershell
rustc --edition=2021 --test extractor\native\lanes\weather_visual_drivers_live\domain24_weather_visual_v2.rs -o extractor\native\target\weather_visual_drivers_live_domain24_tests.exe
extractor\native\target\weather_visual_drivers_live_domain24_tests.exe --nocapture
rustc --edition=2021 extractor\native\lanes\weather_visual_drivers_live\weather_visual_gate.rs -o extractor\native\target\weather_visual_drivers_live_gate.exe
extractor\native\target\weather_visual_drivers_live_gate.exe extractor\native\runs\weather_visual_drivers_live_870ebf_20260701_v2\domain24_weather_visual_v2_payload.bin
cargo test --manifest-path extractor\native\Cargo.toml -p collector compact_weather --lib --no-fail-fast
```

Results: lane tests `4 passed`; payload gate decoded `439056` bytes, `13719`
rows, `13414` province visual rows, no parse errors; collector weather tests
`2 passed`.

See `WEATHER_FULL_REPLAY_RESIDUALS_CLOSEOUT_20260701.md` for the exact
accepted candidate-rejection slice, and
`WEATHER_FULL_REPLAY_RESIDUALS_FINAL_20260701.md` for the first residual
resolution pass, `WEATHER_FULL_REPLAY_RESIDUALS_CONTINUED_20260701.md` for
sandstorm flag blocker removal, and
`WEATHER_SANDSTORM_V3_LANE_LOCAL_PROOF_20260701.md` plus
`SANDSTORM_DOMAIN24_V3_REGISTRATION_PLAN_20260701.md` for the lane-local
compact/Rust v3 path. See
`WEATHER_REMAINING_BLOCKERS_CLOSED_20260701.md` for the `+0x78` enum-candidate
rejection, close-zoom particle/entity blocker evidence, and no-unblocked-weather
residual conclusion for this lane slice.
