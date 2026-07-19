# hub_motorization_live (domain 62) integration packet

Status: `PROVEN` and production-implemented for exact inner country-to-level attribution.

## Scope

This domain emits one exact row per saved country-to-level entry:

```text
CCountrySupplySystem vtable[0] == module + 0x298BBF8
CCountrySupplySystem vtable[1] == module + 0x298BC48
u32[CCSS + 0x194] > 0 selects the persistent settings object
country        = u64[CCSS + 0x78]
owner tag id   = u32[country + 0x08], self-ref checked through CGameState country table
node array     = u64[CCSS + 0x188]
record i       = node array + i * 0x28
province       = u32[record + 0x00]
state          = u32[record + 0x04]
motor_map_ptr  = u64[record + 0x10]
map_size       = u32[record + 0x1c]
entry j        = motor_map_ptr + j * 0x08
actor tag id   = u32[entry + 0x00]
level          = u8[entry + 0x04] (1 = 50%, 2 = 100%)
emit one row per entry; reject all other levels
```

The actor id is already the tag-db/country-table id. Engine lookup callers convert
to entry `+0x00`; the save serializer converts the same stored id to a country token
country table and verifies the map pointer/count did not change while read.

## D04 value validation

Date: 2026-07-05. Build SHA:
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
Live process: `hoi4.exe` PID 19204, read-only `ReadProcessMemory`.

Pause gate was checked before validation:
`u32[u64[module_base + 0x3316E90] + 0x468] == 60830441`.

Oracle:
`extractor/native/runs/division_manpower_assignment_870ebf_20260703_wave2/00_d04_1944_melted.txt`
at the same D04 1944.2.11.18 instant.

The validation parser compared every emitted field against the melt: outer TAG,
owner tag-db id, province id, state id, record index, and map_size.

Result:

- Melt oracle: 223 motorized rows, 189 unique provinces, 483 map entries, 915
  total settings nodes across 69 countries.
- Live decode: 223 motorized rows, 189 unique provinces, 483 map entries, 915
  total settings nodes across 69 settings objects.
- Row keys: 223/223 matched, 0 live-only, 0 melt-only.
- Emitted fields: 1338/1338 matched, 0 mismatches.
- Per-country settings node counts: 69/69 matched.
- Candidate guard rejections while scanning were discarded before promotion:
  `arr_ptr=2`, `province_range=5`, `country_vtable=4`, `country_ptr=4`.

Verdict: PASS. The prior D04 comparison remains the exact outer-node/map-size oracle;
the pinned serializer and lookup laws close the inner attribution and level encoding.

## Production wire contract (schema v2)

Each 32-byte row contains:

`owner_tag_db_id`, `actor_tag_db_id`, `province_id`, `state_id`, `record_index`,
`map_entry_index`, `motorization_level`, and validation flags.

The world fold keys rows by `(owner, province, actor)` and exposes both country
identities, exact `motorization_level`, and engine-fixed `truck_fraction_raw`
(`50000` or `100000`). Any bad actor reference, level, short read, duplicate key,
or pointer/count race makes the current domain hour unknown and retains the last
clean accumulator.

## Packet wiring

- Lane crate: `lanes/hub_motorization_live`.
- Collector: scheduled by `hub_motorization_live`, emits domain 62 perf + data records.
- Recorder host: compact summary decodes domain 62 row counts and diagnostics.
- World fold: projects rows under `supply._r2_hub_motorization_live`; failed or partial
  decodes mark the current hour unknown and retain the prior successful map.
