# Domain 58 Probe Runsheet v2

Lane: `country_economy_counters_live`  
Build gate: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`  
Process: `hoi4.exe`  
Status: root chain validated; factory/resource/consumer counters remain candidate until the same-save oracle pass validates them across countries.

## Non-Negotiable Same-Save Rule

The oracle JSON must be parsed from the exact save currently loaded in `hoi4.exe`, and the game must not unpause or advance a tick between loading that save and running the oracle-backed probe. If this rule is not met, equality matches are not validation evidence.

The currently targeted live run is the 1943 Ireland save. If this save is `HOI4bin`, either pass `--rakaly` or provide an uncompressed/melted text save from the same loaded game state.

## Compile Check

```powershell
python -m py_compile extractor\native\lanes\country_economy_counters_live\probe_870ebf_country_economy_roots.py extractor\native\lanes\country_economy_counters_live\compare_870ebf_country_economy_probes.py extractor\native\lanes\country_economy_counters_live\build_country_economy_oracle.py
```

## Build The Save Oracle

Preferred command for the loaded 1943 Ireland save:

```powershell
python extractor\native\lanes\country_economy_counters_live\build_country_economy_oracle.py --save "C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\IRE_1943_11_09_17.hoi4" --out-dir extractor\save_oracle\output
```

If the save is binary and `rakaly.exe` is available:

```powershell
python extractor\native\lanes\country_economy_counters_live\build_country_economy_oracle.py --save "C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\IRE_1943_11_09_17.hoi4" --rakaly "C:\path\to\rakaly.exe" --out-dir extractor\save_oracle\output
```

If `oracle_status.json` reports `blocked_binary_save_needs_rakaly_or_uncompressed_text_save`, stop and create/load an uncompressed save for the same game state, then rerun the command with that text save path.

Expected outputs:

- `extractor\save_oracle\output\economy.json`
- `extractor\save_oracle\output\production.json`
- `extractor\save_oracle\output\oracle_status.json`

The builder augments the existing save-oracle schema with:

- `factories.civilian_used_derived` and `factories.civilian_free_derived` from construction queue active factories.
- `factories.military_used_assigned` and `factories.military_free_derived` from production factory totals.
- `factories.dockyard_used_assigned` and `factories.dockyard_free_derived` from naval production factory totals.
- resource maps exposed by the save, including `produced`, `exported`, `to_export`, and any available `imported`/`to_use` style maps.

## Oracle-Backed Live Probe

Run this immediately after building the oracle while `hoi4.exe` is still on that same save:

```powershell
python extractor\native\lanes\country_economy_counters_live\probe_870ebf_country_economy_roots.py --out extractor\native\runs\country_economy_counters_live_870ebf_20260703_ireland_v1\country_economy_probe_with_oracle_v2.json --snapshot-label ireland_1943_same_save_oracle_v2 --focus-tags IRE --sample-tags IRE ENG GER USA SOV JAP ITA FRA --economy-oracle extractor\save_oracle\output\economy.json --production-oracle extractor\save_oracle\output\production.json
```

## Decisive Factory Field Validation Re-Probe

Run this while `hoi4.exe` is still paused on the same `1943.11.9.17` tick. This command emits `factory_consumer_scalar_census.oracle_matches.field_validation_tables`, which rereads each candidate factory/consumer offset for every sampled country and compares it to that country's own oracle value.

```powershell
python extractor\native\lanes\country_economy_counters_live\probe_870ebf_country_economy_roots.py --out extractor\native\runs\country_economy_counters_live_870ebf_20260703_ireland_v1\country_economy_probe_factory_field_validation_v3.json --snapshot-label ireland_1943_factory_field_validation_v3 --focus-tags IRE --sample-tags IRE ENG GER USA SOV JAP ITA FRA --min-countries 8 --max-countries 8 --economy-oracle extractor\save_oracle\output\economy.json --production-oracle extractor\save_oracle\output\production.json
```

## Raw-Oracle Factory Field Re-Probe

Run this as the final same-tick static-oracle attempt. It uses `economy_raw.json`, which omits free/used derived fields and compares only raw stored save fields or sums of raw stored save rows.

```powershell
python extractor\native\lanes\country_economy_counters_live\probe_870ebf_country_economy_roots.py --out extractor\native\runs\country_economy_counters_live_870ebf_20260703_ireland_v1\country_economy_probe_factory_raw_validation_v4.json --snapshot-label ireland_1943_factory_raw_validation_v4 --focus-tags IRE --sample-tags IRE ENG GER USA SOV JAP ITA FRA --min-countries 8 --max-countries 8 --economy-oracle extractor\save_oracle\output\economy_raw.json --production-oracle extractor\save_oracle\output\production.json
```

## Decision Tree

- If `build_gate_ok` is false, stop. Do not use any candidates.
- If `roots.status` is not `read`, or `selected_countries` lacks the sample tags, stop and return the JSON.
- Factory/consumer scalar validation lives at `factory_consumer_scalar_census.oracle_matches.cross_country_matches`.
- A scalar offset is promotable only if its status is `VALIDATED_ORACLE_ALL_SAMPLED`, its `missing_tags` list is empty, and its provenance is this same-save oracle run.
- Factory/consumer decisive field validation lives at `factory_consumer_scalar_census.oracle_matches.field_validation_tables`.
- A factory/consumer offset is promotable only if its field-table status is `VALIDATED_FIELD_ALL_SAMPLED_NONCONSTANT`, it maps to exactly one named oracle field, and it is not duplicated by another offset with the same live value vector.
- Resource-vector validation lives at `resource_flow_vector_candidates.oracle_matches.matches`.
- A resource vector is promotable only if its status is `VALIDATED_RESOURCE_VECTOR_ORACLE_ALL_SAMPLED`, its scale is consistent, and every comparable nonzero sampled country matched.
- `OVERLAP_DOMAIN14_*` entries are stability/war-support evidence only. Do not emit them in domain 58.
- Fuel remains domain 10. Do not emit it in domain 58.
- Existing rejected resource candidates (`CCountry+0x5B8`, `CCountry+0x1218`) stay rejected unless new evidence directly answers the prior rejection.
- Partial matches, single-country matches, zero-only matches, and shape-only resource vectors remain `CANDIDATE` and must not enter encode paths.

## Optional UI Values

If the operator can read screen values at the same tick, pass `--ui-values <path>`. Fill only values actually visible; omitted values are safer than guesses.

```json
{
  "countries": {
    "IRE": {
      "factories": {
        "civilian_total": 0,
        "civilian_used": 0,
        "civilian_free": 0,
        "civilian_damaged": 0,
        "military_total": 0,
        "military_used": 0,
        "military_free": 0,
        "military_damaged": 0,
        "dockyard_total": 0,
        "dockyard_used": 0,
        "dockyard_free": 0,
        "dockyard_damaged": 0
      },
      "consumer_goods": {
        "factor": 0.0,
        "factories": 0
      },
      "resources": {
        "steel": {"produced": 0, "imported": 0, "exported": 0, "used": 0, "deficit": 0},
        "aluminium": {"produced": 0, "imported": 0, "exported": 0, "used": 0, "deficit": 0},
        "oil": {"produced": 0, "imported": 0, "exported": 0, "used": 0, "deficit": 0},
        "rubber": {"produced": 0, "imported": 0, "exported": 0, "used": 0, "deficit": 0},
        "tungsten": {"produced": 0, "imported": 0, "exported": 0, "used": 0, "deficit": 0},
        "chromium": {"produced": 0, "imported": 0, "exported": 0, "used": 0, "deficit": 0}
      }
    }
  }
}
```

## Promotion Bar

Promote a lane-local offset only when the JSON result includes:

- exact build SHA match;
- validated CCountry root and tag echo;
- same-save oracle or UI equality across the sampled countries, not a single-country coincidence;
- no contradiction with known rejected candidates;
- provenance path under `extractor/native/runs/...` with date and snapshot label.

Until then, the Rust lane must remain badge-only for unproven slices.
