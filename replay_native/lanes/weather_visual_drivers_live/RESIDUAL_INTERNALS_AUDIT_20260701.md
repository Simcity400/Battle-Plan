# Weather Residual Internals Audit - 2026-07-01

This is the lane-local closeout for the residual `weather_live` wording gap:
visual drivers are validated, while non-visual meteorological internals and
cause/effect needed an explicit extraction/rejection decision.

Completion status: `COMPLETE` for the residual internals audit lane only. This
does not mark the whole weather concept complete. Exact close-zoom particle
parity and exact generator/RNG/name recomputation remain visual/computed scope
if the product later requires them.

Backlog reduced: yes.

Exact backlog item removed or narrowed: non-visual weather internals are no
longer a domain-24 current-state extraction blocker. The replay-relevant
current state is the realized weather already captured through compact
`weather_live`: region phenomenon, named flags, intensity, active modifier
count, next-change raw, region temperature, province temperature/change, water,
snow, mud, and custom-modifier count. No additional current-state compact row is
required for the validated map-tone weather visuals.

## Field Decision

Accepted current-state fields:

- `SWeatherPerRegion`: `phenomenon +0x78`, low-byte weather flags, intensity,
  active-modifier count at `+0x30`, temperature `+0x138`, and next-change raw
  `+0x150`.
- `SWeatherPerProvince`: water `+0x38`, snow `+0x40`, displayed temperature
  `+0x118`, temperature-change `+0x128`, mud byte `+0x130`, and custom-modifier
  count at `+0x148`.

Rejected as independent non-visual current-state rows:

- `weather.current_province/current_region/seed` and generic weather RNG
  cursors. These are future-determinism inputs, not needed to replay already
  captured realized hourly-or-better frames.
- Region active-event vector presence at `SWeatherPerRegion +0x58/+0x70` as an
  independent gameplay state. The refreshed live proof validates 305/305
  agreement between that active signal and captured `phenomenon >= 1`.
- The per-region accumulator/phase candidate at `SWeatherPerRegion +0x28` as a
  compact current-state row. It is recorded as evidence (`0..27900000` in the
  refreshed probe), but realized phenomenon, modifiers, and next-change already
  carry the replay state this lane needs.

Not rejected as impossible, but not part of this residual non-visual lane:

- Province-local and region-local `+0x58/+0x70` vector contents. They are real
  and extractable candidates for close-zoom particle/local-front visual detail.
  They are not non-visual cause/effect state, and exact particle parity should
  be handled as a separate visual-particle lane if it becomes product scope.
- Exact generative weather formula, RNG, and enum-name mapping. This remains
  open-computed/reverse-engineering scope for reproducing the engine generator,
  not a missing current-state row for replaying captured realized frames.

## Roots, Offsets, And Evidence

Validated roots and offsets on build
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`:

- `CGameState = u64[module_base + 0x3316E90]`.
- `CWeatherManager = u64[CGameState + 0x688]`.
- `CWeatherManager` vtable `module_base + 0x2960BE0`.
- Region vector `CWeatherManager + 0x40`, size/capacity `305/305`.
- `SWeatherPerRegion` vtable `module_base + 0x2960B90`, stride `0x160`.
- Province vector `CWeatherManager + 0x10`, size/capacity `13414/13414`.
- `SWeatherPerProvince` vtable `module_base + 0x2960B40`, stride `0x180`.

Refreshed internals probe:

- Command: `python replay_native\lanes\weather_visual_drivers_live\probe_weather_internals_870ebf.py`
- Result: `candidate_valid = true`, errors `[]`.
- Region active signal rows: `301`.
- Region phenomenon-active rows: `301`.
- Region active-vector agreement: `305/305`.
- Region active-front-position entries: `5614`.
- Region weather-effect-entity entries: `315`.
- Province local-front vector rows: `12665/13414`.
- Province local-effect vector rows: `12665/13414`.

Integrated production evidence:

- Manifest: `replay_native/runs/weather_visual_drivers_live_870ebf_integrated_20260701_v1/native_session_manifest.json`.
- `weather_live` data records: `51`.
- Game hours advanced: `50`.
- Unique game hours: `51`.
- Skipped game hours: `0`.
- Ordered compact stream: `true`.
- Decodable compact stream: `true`.
- Source unavailable: `false`.
- Partial data: `false`.
- Bridge unavailable: `false`.
- Worker panic: `false`.
- `.battleplan`: `native_session.battleplan`.

## Schema And Decoder

Compact schema/record changes: no new residual-internals compact record is
needed. The existing domain-24 v2 lane schema remains the production-shaped
weather visual/current-state payload:

- Header: existing 48-byte domain-24 header.
- Region rows: existing 32-byte layout.
- Province visual rows: 32-byte v2 tail with province id, flags, temperature,
  temperature change, water, snow, custom-modifier count, and reserved zero.

Rust decoder/gate changes: no new residual-internals decoder is needed. The
lane-local gate already validates the domain-24 v2 payload and measured-zero
province semantics.

## Proof Commands And Results

```powershell
rustc --edition=2021 --test replay_native\lanes\weather_visual_drivers_live\domain24_weather_visual_v2.rs -o replay_native\target\weather_visual_drivers_live_domain24_tests.exe
replay_native\target\weather_visual_drivers_live_domain24_tests.exe --nocapture
```

Result: 4 passed, 0 failed.

```powershell
rustc --edition=2021 replay_native\lanes\weather_visual_drivers_live\weather_visual_gate.rs -o replay_native\target\weather_visual_drivers_live_gate.exe
replay_native\target\weather_visual_drivers_live_gate.exe replay_native\runs\weather_visual_drivers_live_870ebf_20260701_v2\domain24_weather_visual_v2_payload.bin
```

Result: decoded 439,056 bytes, 13,719 rows, 13,414 province visual rows, no
parse errors.

```powershell
cargo test --manifest-path replay_native/Cargo.toml -p collector compact_weather --lib --no-fail-fast
```

Result: 2 passed, 0 failed.

```powershell
cargo run --manifest-path replay_native/Cargo.toml -p snapshot --bin pack_replay -- verify replay_native/runs/weather_visual_drivers_live_870ebf_integrated_20260701_v1/native_session.battleplan replay_native/runs/weather_visual_drivers_live_870ebf_integrated_20260701_v1 native_session
```

Result: `verify: OK` for `native_6124_18be0901914bf9fc`.

## Required Report Fields

- Completion status: `COMPLETE` for residual internals audit; whole weather
  concept `NOT COMPLETE`.
- Backlog reduced: yes.
- Exact backlog item removed or narrowed: non-visual meteorological internals
  and cause/effect no longer block current-state replay extraction; remaining
  work is visual-particle or open-computed generator parity only.
- Exact files changed: `replay_native/lanes/weather_visual_drivers_live/RESIDUAL_INTERNALS_AUDIT_20260701.md`,
  `replay_native/lanes/weather_visual_drivers_live/README.md`,
  `replay_native/lanes/weather_visual_drivers_live/WEATHER_INTERNALS_STATUS_20260701.md`.
- Exact roots/offsets/functions validated or rejected: listed above.
- Compact schema/record changes: none for residual internals; domain-24 v2
  visual/current-state rows remain sufficient.
- Rust decoder/gate changes: none for residual internals; existing lane gate
  passed.
- Proof commands/results: listed above.
- DLL-only: yes for accepted production weather data; Python probe was
  discovery/rejection evidence only, not a runtime source.
- Compact stream: yes.
- Rust decoded: yes.
- `.battleplan` verified: yes.
- Speed-five/current-equivalent hourly-or-lower: yes.
- Zero skipped hours: yes.
- Any partial/source/worker/bridge failures: no.
