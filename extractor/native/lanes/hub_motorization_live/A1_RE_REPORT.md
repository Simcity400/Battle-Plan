# A1 hub motorization inner-map RE report

Status: **PROVEN, IMPLEMENTED, PROMOTION-READY**  
Binary: `hoi4.exe 1.19.2.0.a729`  
SHA-256: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`  
Image base: `0x140000000`

## Exact law

`CCountrySupplySystem+0x188` is a contiguous `0x28`-stride node-settings array;
its count is at `+0x194`. For each node record:

```text
province_id          = u32[record + 0x00]
state_id             = u32[record + 0x04]
motorization_entries = u64[record + 0x10]
entry_count          = u32[record + 0x1c]

entry[j]             = motorization_entries + j * 0x08
actor_tag_db_id       = u32[entry + 0x00]
motorization_level    = u8 [entry + 0x04]
```

The key is already the country table/tag-db id. Lookup callers translate an
indexes the country-token table as `CGameState+0x358 + id*0x20`.

Valid persisted levels are exact:

- `1` = fixed-point `50000` = 50% truck fraction.
- `2` = fixed-point `100000` = 100% truck fraction.
- `0` means absent/off and is not a persisted entry.
- Any other nonzero level takes the engine assertion path and is rejected by capture.

## Function evidence

| Function | Finding | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|---|
| `0x14121c6d0` | CCSS save serializer proves node stride, entry stride, key at `+0`, level byte at `+4`, and exact iteration count. | `20260716T225321Z-a1-ccss-vtable-functions-v1-c1d89973` | `evidence/func_14121c6d0.decompile.c` | `a6d076684f4ca7ee150239141acf494173ce1213765150b93f8827f1ad096639` |
| `0x141939dd0` | Lookup converts caller reference, searches the flat key/value entries, and maps levels 1/2 to 50000/100000. | `20260716T225308Z-a1-motorization-script-bindings-v1-adc4b781` | `evidence/func_141939dd0.decompile.c` | `ccb68cb514ad8f51f5b560048e44081598799c8e5ec4453f19a9b9f4c1a4e855` |
| `0x141939f40` | Independent lookup repeats the same key comparison and level law. | `20260716T225308Z-a1-motorization-script-bindings-v1-adc4b781` | `evidence/func_141939f40.decompile.c` | `ccb68cb514ad8f51f5b560048e44081598799c8e5ec4453f19a9b9f4c1a4e855` |
| `0x140ba6240` | Converts an external country reference id through the CGameState `+0x340` remap before map lookup. | `20260716T225308Z-a1-motorization-script-bindings-v1-adc4b781` | `evidence/func_140ba6240.decompile.c` | `ccb68cb514ad8f51f5b560048e44081598799c8e5ec4453f19a9b9f4c1a4e855` |
| `0x140ba5c20` | Converts the stored map key directly to its country token at `CGameState+0x358 + id*0x20`; no second remap occurs. | `20260716T225759Z-a1-country-ref-token-resolver-v1-8e65b953` | `evidence/func_140ba5c20.decompile.c` | `c5e770b4d4b4ff9b49caaf09f9b17613ce25b712163ccc08488dff1d88a168a6` |

The vtable data also identifies the owning source unit as
`source/supply/country_supply.cpp`; the level assertion identifies
`source/supply/supply_node_settings.h:0x21`.

## Production implementation

Domain 62 schema v2 now emits one 32-byte row per inner entry:

`owner_tag_db_id`, `actor_tag_db_id`, `province_id`, `state_id`,
`record_index`, `map_entry_index`, `motorization_level`, `flags`.

The reader validates both owner and actor against the country table, accepts only
levels 1/2, bounds every count, performs one bounded map read, and re-reads the
record's map pointer/count after decoding to reject realloc races. The fold uses
`(owner, province, actor)` as its unique key and exposes exact actor identity,
level, and fixed-point truck fraction.

## Mechanical verification

- Lane tests: `cargo test` in `extractor/native/lanes/hub_motorization_live` —
  **10 passed, 0 failed**.
- Recorder integration: `cargo check` in `extractor/native/recorder_host` — **PASS**.
- Rust formatting/parser check for every changed Rust consumer — **PASS**.
- Coverage includes exact multi-actor decoding, schema round-trip, invalid level,
  unresolved actor, owner self-reference mismatch, vector incoherence/race, and
  proof that the anchored collector never performs a full-heap scan.
- The earlier D04 oracle remains independently exact for all outer fields and
  counts: 223 motorized nodes, 189 provinces, 483 inner entries, 69 settings
  countries, with 1338/1338 compared outer fields matching.

No A1 RE or data blocker remains.
