# Domain 24 Sandstorm V3 Registration Plan - 2026-07-01

Scope: add the validated named region sandstorm flag to the existing
`weather_live` compact stream without changing row lengths or adding a second
collector.

Validated root/function evidence:

- Static predicate RTTI:
  `CIsSandStorm@CIsFightingInWeatherTrigger` vtable slot reads
  `byte ptr [rdx + 0x128]`.
- Live root: `CWeatherManager = u64[CGameState + 0x688]`, region vector at
  `CWeatherManager +0x40`, `305/305` `SWeatherPerRegion` rows.
- Live field: `SWeatherPerRegion +0x128` is a bool-like sandstorm named flag.
- Live proof: `weather_full_replay_residuals_probe.json` counted
  `named_flag_counts.sandstorm = 1` in the current fixture.
- Lane-local compact proof:
  `domain24_weather_sandstorm_v3_payload.bin` is a full domain-24 payload with
  `305` region rows, `13414` province rows, `439056` bytes, and
  `sandstorm_region_rows = 3`.
- Lane-local Rust proof:
  `weather_sandstorm_v3_gate.json` decoded the payload with
  `weather_region_sandstorm_rows = 3` and `parse_errors = []`.

Shared collector patch plan:

1. In `extractor/native/collector/src/lib.rs`, near the weather offsets around
   current lines `115`..`119`, add:

```rust
const WEATHER_REGION_SANDSTORM_OFF: usize = 0x128;
```

2. In `read_compact_weather_region_row` around current lines
   `19618`..`19622`, read:

```rust
let sandstorm = weather_bool_byte(body, WEATHER_REGION_SANDSTORM_OFF)?;
```

after `blizzard` and before `arctic_water`.

3. In region flag packing around current lines `19646`..`19652`, add:

```rust
if sandstorm {
    flags |= 1 << 7;
}
```

This is a flag-bit extension only. `CompactWeatherRegionRow` stays 32 bytes,
the payload header stays 48 bytes, and the two reserved header u32s remain zero.

Rust decoder/gate patch plan:

1. In `extractor/native/lanes/weather_visual_drivers_live/domain24_weather_visual_v2.rs`,
   near current lines `47`..`60`, add:

```rust
pub const REGION_FLAG_SANDSTORM: u32 = 1 << 7;
```

and include it in `REGION_KNOWN_FLAGS`.

2. In `validate_region_row`, add:

```rust
if flags & REGION_FLAG_SANDSTORM != 0 {
    add_unit(summary, "weather_region_sandstorm_rows", 1);
}
```

3. Add a decoder test equivalent to
`extractor/native/lanes/weather_live/domain24_weather_sandstorm_v3.rs`:

- sandstorm flag decodes without an unknown flag error;
- `weather_region_sandstorm_rows` increments;
- unrelated unknown flags remain rejected.

Lane-local implementation files:

- `extractor/native/lanes/weather_live/domain24_weather_sandstorm_v3.rs`
- `extractor/native/lanes/weather_live/weather_sandstorm_v3_gate.rs`
- `extractor/native/lanes/weather_live/weather_sandstorm_v3_payload_probe.py`

Shared recorder-host patch plan:

1. In `extractor/native/recorder_host/src/main.rs`, near current lines
   `378`..`397`, add region flag constants or an equivalent local known-mask:

```rust
const COMPACT_WEATHER_REGION_FLAG_SANDSTORM: u32 = 1 << 7;
const COMPACT_WEATHER_REGION_KNOWN_FLAGS: u32 = 0x7f | COMPACT_WEATHER_REGION_FLAG_SANDSTORM;
```

The minimal acceptable version is replacing the current hardcoded mask at line
`9181` with `0xff`; constants are preferred so the bit has a stable name.

2. In `decode_compact_weather_data_payload`, current line `9181`, change:

```rust
if flags & !0x7f != 0 {
```

to either:

```rust
if flags & !COMPACT_WEATHER_REGION_KNOWN_FLAGS != 0 {
```

or the minimal literal form:

```rust
if flags & !0xff != 0 {
```

3. In the same region row loop, after the existing numeric validations and
before the row-reserved check around current lines `9222`..`9227`, add:

```rust
if flags & COMPACT_WEATHER_REGION_FLAG_SANDSTORM != 0 {
    add_compact_domain_data_unit(summary, 24, "weather_region_sandstorm_rows", 1);
}
```

If the literal-mask variant is used, the equivalent condition is
`if flags & (1 << 7) != 0`.

4. Add a recorder-host test alongside the weather summary tests around current
lines `13214`..`13319`: construct a weather payload with the first region
`flags |= 1 << 7`, assert no parse errors, and assert
`weather_region_sandstorm_rows == Some(1)`. Keep the existing bad-flag test so
`1 << 8` or any unknown region bit still fails.

Replay gate patch plan:

1. Rebuild the single collector DLL and recorder host after the collector and
   decoder patches.
2. Run the usual compact proof gate requiring `weather_live`.
3. Add a proof assertion or report check for `weather_region_sandstorm_rows > 0`
   when the fixture contains active sandstorm rows; do not require it in all
   weather runs because sandstorms are conditional.

Required proof after registration:

```powershell
cargo test --manifest-path extractor\native\Cargo.toml -p collector compact_weather --lib --no-fail-fast
rustc --edition=2021 --test extractor\native\lanes\weather_visual_drivers_live\domain24_weather_visual_v2.rs -o extractor\native\target\weather_visual_drivers_live_domain24_tests.exe
extractor\native\target\weather_visual_drivers_live_domain24_tests.exe --nocapture
cargo run --manifest-path extractor\native\lanes\compact_replay_verification_gates\Cargo.toml -- --run-dir <new-weather-live-run> --require-domain weather_live --min-game-hours 5 --write-report <new-weather-live-run>\compact_replay_gate.weather_live.json
```

Acceptance criteria:

- DLL-only compact weather stream still decodes.
- `.battleplan` still verifies.
- Hourly-or-better cadence still passes with zero skipped hours.
- No source/partial/bridge/worker failures.
- Weather decoder reports `weather_region_sandstorm_rows` when the live fixture
  contains sandstorm rows.

Current lane-local proof commands/results:

```powershell
python extractor\native\lanes\weather_live\weather_sandstorm_v3_payload_probe.py --pid 45880 --out-dir extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: emitted
`extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2\domain24_weather_sandstorm_v3_payload.bin`,
`payload_bytes=439056`, `region_rows=305`, `province_rows=13414`,
`sandstorm_region_rows=3`.

```powershell
rustc --edition=2021 --test extractor\native\lanes\weather_live\domain24_weather_sandstorm_v3.rs -o extractor\native\target\weather_live_domain24_sandstorm_v3_tests.exe
extractor\native\target\weather_live_domain24_sandstorm_v3_tests.exe --nocapture
rustc --edition=2021 extractor\native\lanes\weather_live\weather_sandstorm_v3_gate.rs -o extractor\native\target\weather_live_sandstorm_v3_gate.exe
extractor\native\target\weather_live_sandstorm_v3_gate.exe extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2\domain24_weather_sandstorm_v3_payload.bin
```

Result: tests `2 passed`; gate decoded `13719` rows,
`weather_region_sandstorm_rows=3`, parse errors `[]`.
