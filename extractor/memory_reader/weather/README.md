# weather

Live/readback support for HOI4 weather and environment data.

## Weather Data Notes

The older "weather is not saved, no oracle exists" claim is wrong for realized
weather. The melted save does persist realized weather:

- province weather scalars such as temperature, water, snow, and offsets
- region weather values such as temperature, next weather change, phenomenon
  flags, and `arctic_water`

Current collector/data status is in `../../IN_PROCESS_COLLECTOR_PLAN.md`; data
evidence is in `../../coverage_audit/COVERAGE_REPORT.md`.

## Evidence

- Province-level realized weather scalars are parsed/validated and feed dynamic
  map rendering slices.
- Snow, mud, and frozen-sea visual data are wired into the native map path.
- Weather clock/scan lessons and offsets are documented in
  `weather_offsets.draft.json` and `../../RESEARCH_weather.md`.

## Evidence Boundaries

- Treat generative weather internals separately: temperature formula, RNG, and
  phenomenon-name ordering are computed/RE work, not missing save persistence.

## Files

| File | Role |
| --- | --- |
| `weather_reader.py` | Read-only live/weather funnel and guardrails. |
| `weather_state_reader.py` | State extraction support for realized weather. |
| `weather_fields_live.py` | Live field extraction helpers. |
| `weather_offsets.draft.json` | Durable method/offset notes and residuals. |
| `precip_offsets.draft.json` | Precipitation-specific notes. |
| `weather_climatology.json` | Static climatology reference from game data. |
| `region_weather_mod.json`, `prov2region_mod.json` | Static region/province mapping data. |

Generated probes and `_*.json` scan outputs are scratch artifacts and should stay
ignored/untracked.
