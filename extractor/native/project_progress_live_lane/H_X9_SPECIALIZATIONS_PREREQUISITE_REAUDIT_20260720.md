# H-X9 specialization prerequisite re-audit (2026-07-20)

## Superseded outcome

X9 is closed by the fresh exact-a729 serializer-token namespace proof and production wiring recorded
in `X9_SPECIALIZATION_TOKEN_NAMESPACE_PROMOTION_20260720.json`. The former same-tick four-value
permutation is optional regression evidence, not a human or production prerequisite.

No build, compile, Cargo, test, formatter, or verifier command was run in this subagent lane. Root
owns the single central validation pass.

## Fresh exact-20 Ghidra authority

The broker reported all 20 shards ready in read-only/no-analysis mode for:

- `hoi4.exe 1.19.2.0.a729`;
- image base `0x140000000`;
- SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

Six bounded jobs were submitted. Five completed and one broad export failed at
`EXTERNAL:00000107`; two smaller function exports superseded that broad request for all internal
functions while preserving the failure as negative operational evidence. The exact job IDs,
executing shard, and artifact or manifest hashes are pinned in the promotion receipt.

## Exact namespace join

The decisive evidence is the `CBreakthroughProgress` serializer pair:

- writer `0x1419F14F0` walks every ordered-tree node and calls the engine serializer with the key at
  `node+0x1C` and the points value at `node+0x20`;
- reader `0x1419F10C0` reads the parser token ID at `parser+0x30`, inserts that ID as the tree key at
  `node+0x1C`, and stores the parsed points at `node+0x20`.

Therefore the D41 `specialization_handle` is an engine serializer token ID. It is the same namespace
resolved by the collector's already-authenticated token-record scanner; it is not a coincidentally
equal integer from an unjoined namespace.

The authored semantic set is closed to:

1. `specialization_nuclear`
2. `specialization_naval`
3. `specialization_air`
4. `specialization_land`

The collector publishes a semantic code only after a complete committed-private heap census finds
one unambiguous authenticated token record for that handle and its text equals one member of this
closed set. An unresolved, ambiguous, or differently named token stays raw and unnamed.

## Production carrier and consumers

The existing D41 row remains 16 bytes:

`tag_db_id:u32, specialization_handle:u32, points_raw:u32, flags:u32`

Header extension 5 is the first version that admits specialization bits. Extensions 0 through 4
require those formerly reserved bits to stay zero.

Bit 0 marks an exact semantic enum, bits 1 through 3 carry codes 1=nuclear, 2=naval, 3=air, and
4=land, and bit 31 retains deletion semantics. Unknown flag combinations, invalid codes, a code
without the present bit, and impure deletion rows are rejected.

The fold preserves the raw handle and points in all admitted rows. It emits `specialization` and
`_r2_specialization_authority=exact_a729_serializer_token_record` only for an exact semantic code;
otherwise specialization is `null`. Replay core exposes a typed exact breakthrough view only when
every row has a closed authored name and that exact authority marker. The game UI adapter consumes
only this typed view and remains unavailable for raw-only, partial, ambiguous, or forged input.

## Retained dynamic evidence

The older retained census remains useful negative and regression evidence:

- 113 retained collector streams were scanned and 101 contained D41 data;
- those streams exposed only handles `{22214, 22215}` through the former record-zero path;
- the closest retained a729 live/save pair was two game hours apart and had no exact value matches;
- b49d save-only four-column vectors cannot authenticate a729 runtime values.

Those facts still prohibit naming numeric handles by order or visual correlation. They no longer
block production because semantics are acquired from the exact engine token namespace at runtime.

## Owner boundary

D41 specialization ownership remains the directly validated `CCountry` root, with live vtable and
`tag_db_id` equality rechecked at the read boundary. D65 facility ownership is a separate A23 lane;
no D65 candidate field or multiply-reachable facility is used to label D41 rows.

## Human-input disposition

No human input is required for X9. A coherent same-tick save/live observation containing all four
values is welcome as an independent regression fixture, but it is not an admission gate and no
manual transcription or semantic judgment is accepted.
