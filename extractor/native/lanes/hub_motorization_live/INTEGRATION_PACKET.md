# hub_motorization_live (domain 62) integration packet

Status: scheduled as `ValidatedSliceOpenGaps` after D04 value validation.

## Scope

This domain emits node-level supply-hub motorization membership only:

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
emit row iff map_size > 0
```

The inner multi-entry country-to-level hashmap attribution is intentionally not emitted.
`map_size` is promoted only as the saved entry count for the node.

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

Verdict: PASS for node-level membership and map_size entry count.

## Packet wiring

- Lane crate: `lanes/hub_motorization_live`.
- Collector: scheduled by `hub_motorization_live`, emits domain 62 perf + data records.
- Recorder host: compact summary decodes domain 62 row counts and diagnostics.
- World fold: projects rows under `supply._r2_hub_motorization_live`; failed or partial
  decodes mark the current hour unknown and retain the prior successful map.
