# A27 live design catalog

A27 is promotion-ready for `hoi4.exe 1.19.2.0.a729`. The production reader
captures the complete live catalog directly from process memory; the save is
loaded only after the native snapshot is frozen and is used solely as a
same-tick acceptance oracle.

## Frozen result

- Native shared-heap catalog: 5,281 equipment variants and 1,126 division
  templates across 440 countries.
- Same-tick oracle equality: exact stable IDs, owners, variant origins,
  lifecycle, archetypes, names, module-token multisets, upgrade-level
  multisets, template priority/lock state, and all regiment/support positions.
- Equipment-manager A/B vectors contain optional UI occurrences for 5,267
  variants. They are not a catalog root. The 14 catalog variants without an
  A/B occurrence are `1093:70` through `1101:70`, `1691:70` through `1694:70`,
  and `4540:70`.
- The live engine strings are UTF-8. The save JSON contains 31 variant names
  and 59 template names decoded as Latin-1; the acceptance gate repairs only
  that oracle-side encoding for semantic comparison and records the counts.
- Independent promotion gate: 12/12 checks passed, zero blockers.

Primary evidence:

- `proofs/a27_live_same_tick_v2/snapshot.json`, SHA-256
  `6fa4386b9bd688df47a613b6268ccb0f125fcd2fe75edd9c484154ff38d5628b`.
- `proofs/a27_live_same_tick_v2/report.json` (`status=PASS`).
- `proofs/a27_full_reader_fixture/live_promotion_gate.json` (independent gate).
- `proofs/a27_full_reader_fixture/template_scalar_semantics_ghidra.json`
  (static field semantics and complete artifact/manifest provenance).

## Production surface

- `live_a729_acceptance.py`: exact process-memory producer and frozen-snapshot
  re-verifier.
- `a729_layout.py`: pinned layout and binary identity constants.
- `a729_memory_reader.py`: compatibility entry point delegating to the live
  producer. The old save-built `SparseMemory`/`FixturePredicates` path has been
  removed.
- `validate_live_producer.py`: fail-closed payload gate. A
  `definition_complete=true` badge without the actual definition fails.

The producer uses vtable/type/stable-ID heap sweeps as catalog authority.
Country template vectors supply template ownership/order; A/B vectors are
joined as optional UI membership/order occurrences. Raw pointers are retained
only as same-snapshot identity and never become replay-stable IDs.

Run the frozen gates:

```powershell
python extractor/native/lanes/design_catalog_live/live_a729_acceptance.py --snapshot-in extractor/native/lanes/design_catalog_live/proofs/a27_live_same_tick_v2/snapshot.json --snapshot-out extractor/native/lanes/design_catalog_live/proofs/a27_live_same_tick_v2/snapshot.json --report-out extractor/native/lanes/design_catalog_live/proofs/a27_live_same_tick_v2/report.json --oracle C:\tmp\a27_current_same_tick\designs.json
python extractor/native/lanes/design_catalog_live/validate_live_producer.py --snapshot extractor/native/lanes/design_catalog_live/proofs/a27_live_same_tick_v2/snapshot.json --oracle C:\tmp\a27_current_same_tick\designs.json
python -m pytest extractor/native/lanes/design_catalog_live/test_validate_live_producer.py extractor/native/lanes/design_catalog_live/test_a729_memory_reader.py -q
```

## Useful non-obvious findings

- Variant `+0x1C` is the creator country tag, not an equipment-type token.
- Variant `+0x38` is the size field of the MSVC display-name string beginning
  at `+0x28`, not an interface-type field.
- Template outer `+0x1A4` is priority, `+0x1CC` is the raw lock-regime byte,
  `+0x235` is explicit `is_locked`, and `+0x236` is obsolete. The observed
  equality between `+0x1CC` and save `is_army_hq` is incidental.
- Dynamic tag IDs immediately after the immutable 440-row tag table resolve in
  raw-ID order as `D01`, `D02`, `D03`, ... . The packed table header stores
  capacity in its low `u32` and logical count in its high `u32`.
- No retained equipment-designer row pool existed in PID 13640; catalog
  completeness therefore comes from the native heap root, not materialized UI
  rows.
