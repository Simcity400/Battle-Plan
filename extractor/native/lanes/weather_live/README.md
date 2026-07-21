# weather_live residuals lane
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/weather_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Lane-local proof package for domain 24 `weather_live` residuals. This lane does
not claim whole-weather completion. The newest superseding residual report is
`../../../documents/archive/native_lanes/weather_live/WEATHER_REMAINING_BLOCKERS_CLOSED_20260701.md`; the older closeout files are
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

## A20 retained temporal gate

The current exact20 semantic and production-wiring closeout is
`../../../documents/dossiers/A20_WEATHER_TEMPORAL_CLOSEOUT_20260720.json`.
It supersedes the airborne-priority arrays in the content-addressed historical
XN14A/XN14C v1 evidence without rewriting those frozen artifacts; the adjacent
replay authority is
`../../../../replay_interface/re_evidence/rendering_laws/XN14C/A20_PRIORITY_SUPERSESSION_20260720.json`.
In particular,
`SWeatherPerRegion+0x78` is a compatibility activity diagnostic, not a
weather-kind enum; weather kind comes only from the named flags. The exact
`0x14166ec50` consumer order is rain-light, rain-heavy, snow, blizzard,
sandstorm.

The retained A20 candidate is Zstandard storage, not raw H4CR. The production
gate is therefore the `weather_temporal_drift_gate` binary target in the
`weather_live_lane` package; it uses the same strict
raw/Zstandard file-boundary codec as the recorder and replay decoder. It hashes
the encoded and decoded streams, rejects torn H4CR headers, enforces exact
sequence and weather-hour continuity, validates every region and province row,
authenticates named weather-kind transitions and due-only expiry refreshes, and
emits the pointer-free JSON receipt on stdout.

Only the primary agent runs the retained evidence test, through the centralized
runner:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Evidence -Workspace Extractor -Package weather_live_lane -Bin weather_temporal_drift_gate -FilterExpression 'test(retained_a20_423_hour_stream_passes_exact_gate)' -CacheMode Reusable
```

The exact retained contract is 423 weather records / 423 unique hours,
`60794379..60794801`, 422 consecutive pairs, zero duplicate or nonconsecutive
hours, one full baseline plus 422 deltas, 305 reconstructed region ids, and
13,414 reconstructed province ids (`0..13413`, including engine-array
completeness row zero). The evidence test also pins the 17,484,345
encoded bytes and SHA-256
`25c020ba7f423759c945311f56fd78ecf27f7581835fbada13ffb1f542e986ee`
to the 1,488,548,730 decoded bytes and SHA-256
`3857e2d0afa1c30350eb0f3cfcef0d3ee6643d282674ce75bd064a50c20db3a1`.
Do not request another A20 capture unless this exact retained evidence test
fails after the corpus and gate have both passed their integrity checks.

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

See `../../../documents/archive/native_lanes/weather_live/WEATHER_FULL_REPLAY_RESIDUALS_CLOSEOUT_20260701.md` for the exact
accepted candidate-rejection slice, and
`../../../documents/archive/native_lanes/weather_live/WEATHER_FULL_REPLAY_RESIDUALS_FINAL_20260701.md` for the first residual
resolution pass, `../../../documents/archive/native_lanes/weather_live/WEATHER_FULL_REPLAY_RESIDUALS_CONTINUED_20260701.md` for
sandstorm flag blocker removal, and
`../../../documents/archive/native_lanes/weather_live/WEATHER_SANDSTORM_V3_LANE_LOCAL_PROOF_20260701.md` plus
`../../../documents/archive/native_lanes/weather_live/SANDSTORM_DOMAIN24_V3_REGISTRATION_PLAN_20260701.md` for the lane-local
compact/Rust v3 path. See
`../../../documents/archive/native_lanes/weather_live/WEATHER_REMAINING_BLOCKERS_CLOSED_20260701.md` for the `+0x78` enum-candidate
rejection, close-zoom particle/entity blocker evidence, and no-unblocked-weather
residual conclusion for this lane slice.
