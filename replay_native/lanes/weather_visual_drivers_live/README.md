# weather_visual_drivers_live

Lane-local production stack for the remaining weather visual-driver extraction work.

This lane extends the existing compact `weather_live` domain (domain 24). The current
shared collector already emits validated per-region weather rows. The gap is the
per-province visual-driver portion that the current recorder schema reserves but rejects
when nonzero.

## Current production root contract

Current shared collector constants, from `replay_native/collector/src/lib.rs`:

- `CGameState = u64[module_base + 0x3316E90]` after the collector sentinels pass.
- `CWeatherManager = u64[CGameState + 0x688]`.
- `CWeatherManager` vtable: `module_base + 0x2960BE0`.
- Region vector: `CWeatherManager + 0x40`, size/capacity `305`.
- `SWeatherPerRegion` vtable: `module_base + 0x2960B90`.
- Region stride: `0x160`; `region_id` at `+0x08`; phenomenon enum at `+0x78`;
  active-modifier vector at `+0x30`; low-byte named flags at `+0x108`, `+0x110`,
  `+0x118`, `+0x120`, `+0x130`; temperature raw at `+0x138`; next-change raw at
  `+0x150`.

Lane-local province addition, now validated live on `870ebf` in
`replay_native/runs/weather_visual_drivers_live_870ebf_20260701_v2`:

- Province vector: `CWeatherManager + 0x10`, expected observed rows around `13414`.
- `SWeatherPerProvince` vtable for the current collector profile:
  `module_base + 0x2960B40` (`13414/13414` rows matched on pid `45880`).
- Province stride: `0x180`; `province_id` at `+0x08`; water raw at `+0x38`; snow raw
  at `+0x40`; displayed temperature raw at `+0x118`; temperature-change raw at
  `+0x128`; mud byte at `+0x130`; custom-modifier vector at `+0x148`.
- Custom-modifier vectors on `870ebf` use `{begin, size}` as the reliable count
  contract for this field; some valid non-empty rows have `size=1, cap=0`, so the
  production guard should require heap begin and bounded size, not `cap >= size`.

The province path must fail closed unless the manager vtable, province vector header,
province vtable, `province_id == index`, count bounds, and raw value bounds all pass.
No save/Python source is allowed in the runtime capture loop.

## Compact schema

`domain24_weather_visual_v2.rs` keeps the existing domain 24 header and region row
layout. It uses the already-reserved province counts:

```text
header (48 bytes)
  u64 elapsed_wall_ms
  u32 region_count
  u32 changed_rows
  u32 status_flags
  u32 province_count
  u32 observed_region_rows
  u32 observed_province_rows
  u32 scale_raw (=100000)
  u32 active_region_count
  u32 reserved0
  u32 reserved1

region rows (32 bytes each, current layout)
province visual rows (32 bytes each, new v2 tail)
  u32 province_id
  u32 flags
  i32 temperature_raw
  i32 temperature_change_raw
  i32 water_raw
  i32 snow_raw
  u32 custom_modifier_count
  u32 reserved0
```

Province flags distinguish measured zero from absent data:

- `TEMP_PRESENT`, `TEMP_CHANGE_PRESENT`, `WATER_PRESENT`, `SNOW_PRESENT`
- `MUD_PRESENT`, `MUD_ACTIVE`
- `CUSTOM_MODIFIER_COUNT_PRESENT`
- `STRUCT_VALIDATED`
- `MAP_RECOLOR_DRIVER` for temp/water/snow
- `DYNAMIC_LUT_DRIVER` for snow/mud

This preserves the UI contract where a measured `snow=0` or `mud=false` is still
captured data and can gate the weather visual path.

## Integration patch plan

1. Extend shared collector weather structs with `CompactWeatherProvinceRow` and
   add `previous_provinces` to `CompactWeatherTracker`.
2. Extend `CompactWeatherRoot` with `province_array` and `province_count`. Validate
   both region and province roots from the same `CWeatherManager`.
3. Sample the province array inside the existing weather worker after the region read.
   Emit a full province snapshot on the first weather frame, then delta rows only.
4. Replace the shared recorder error `weather province rows are not supported in this
   schema` with the v2 tail decoder from this lane.
5. Add recorder semantic counters:
   `weather_province_visual_rows`, `weather_observed_province_rows`,
   `weather_province_temp_rows`, `weather_province_water_rows`,
   `weather_province_snow_rows`, `weather_province_mud_measured_rows`,
   `weather_province_mud_active_rows`, `weather_province_custom_modifier_rows`,
   `weather_province_map_recolor_driver_rows`, and
   `weather_province_dynamic_lut_driver_rows`.
6. Run a compact speed-five proof with at least `weather_live`, `map_control`, and
   `map_detail` scheduled. Acceptance requires ordered/decodable compact records,
   nonzero `observed_province_rows`, `.battleplan` packing/verification, hourly-or-better
   cadence, zero skipped game hours, and no source/partial/bridge/worker failures.

## Local proof

Standalone tests:

```powershell
rustc --edition=2021 --test replay_native\lanes\weather_visual_drivers_live\domain24_weather_visual_v2.rs -o replay_native\target\weather_visual_drivers_live_domain24_tests.exe
replay_native\target\weather_visual_drivers_live_domain24_tests.exe --nocapture
```

Result: 4 passed, 0 failed.

Existing live compact stream evidence is summarized in
`replay_native/runs/weather_visual_drivers_live_20260701_v1/proof.json`.

Live `870ebf` province-root validation and the nonzero-province lane-local payload
are in `replay_native/runs/weather_visual_drivers_live_870ebf_20260701_v2/`.

## Internals audit

`WEATHER_INTERNALS_STATUS_20260701.md` is the lane-local status note for the
non-visual weather-internals question. It deliberately uses validated-slice
language: region/province realized weather and visual map drivers are proven,
but the whole weather concept is not marked complete. The note rejects weather
RNG cursors as a required current-state replay extraction row, classifies
`SWeatherPerRegion +0x58/+0x70` active vectors as redundant for non-visual
gameplay state when they agree with `phenomenon >= 1`, and leaves exact
generator/RNG/name parity as open-computed if product scope requires it.

`RESIDUAL_INTERNALS_AUDIT_20260701.md` is the explicit closeout for the residual
internals lane: no additional non-visual current-state compact records are
required beyond the validated domain-24 realized weather/visual-driver payload.
It does not claim whole-weather completion or close-zoom particle parity.
