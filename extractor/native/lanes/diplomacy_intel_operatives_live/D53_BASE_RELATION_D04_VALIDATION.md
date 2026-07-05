# Domain 53 Base Relation D04 Validation

Date: 2026-07-05.
Process: `hoi4.exe` PID 19204.
Build SHA256: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
Save/oracle: `extractor/native/runs/cracklist_b4_d53_base/melt_relations.json`, generated from the D04 1944.2.11.18 melted save.

## Live Gate

- Module base: `0x7FF7551A0000`
- `CGameState`: `0x1E1A4FD3F10`
- Date counter: `u32[CGameState+0x468] = 60830441`
- Country database: `0x1E0D6718AF0`
- Country table: `0x1E0D15CF670`
- Country count: 440
- Validated live countries: 437

## Chain Validated

```text
mgr   = u64[CCountry + 0xF88]
table = u64[mgr + 0x08]
rel   = u64[table + target_tag_db_id * 8]
cached_sum            = i32[rel + 0x300]
border_friction_claim = i32[rel + 0x308] / 100000
```

## Results

- Melt oracle: 103 source countries, 44,766 relation pairs.
- `cached_sum @+0x300 i32`: 10,354/10,354 non-missing rows matched, 0 mismatches, 2 tag-alias misses.
- `border_friction @+0x308 i32x100000`: 44,382/44,382 non-missing rows matched, 0 mismatches, 304 tag-alias misses.
- Any-field relation pairs: 44,461 matched, 0 mismatches, 305 tag-alias misses.
- Miss reason: live tag table lacks the melt dynamic-tag names for those pairs; there were no chain/value failures.

## Production Decision

The current collector/recorder path already decouples unresolved operation detail, intel-map percentages, incoming action queue, and war lifecycle detail as lane-local diagnostic status bits. Those bits are outside `COMPACT_STATUS_FAILURE_MASK`, and lane tests assert that unresolved operation/intel gaps do not set source-unavailable or partial-data failure bits.

No Rust change was required for this packet. The relation base root remains scheduled under domain 53 as `ValidatedSliceOpenGaps`; relation scalar value proof is now recorded here and reflected in `REVALIDATION_870EBF.md`.
