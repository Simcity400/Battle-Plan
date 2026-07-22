# A27 live design catalog

A27's exact **single-hour census** is proven for
`hoi4.exe 1.19.2.0.a729`. The temporal production path is now implemented but
awaits root central validation and one autonomous H-A27 live run. During
capture the recorder owns one hidden repeated-census worker and a separately
authenticated physical-input action driver. Every accepted hour is a
whole-sweep-sealed full replacement; packaging derives exact stable-ID upserts
and absence-only tombstones and rejects any gap against the compact replay
spine. The save remains a same-tick acceptance oracle only.

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
- Independent single-hour census gate: 12/12 checks passed, zero blockers. This
  does not prove create/edit/rename/obsolete/delete history.

Primary evidence:

- `proofs/a27_live_same_tick_v2/snapshot.json`, SHA-256
  `6fa4386b9bd688df47a613b6268ccb0f125fcd2fe75edd9c484154ff38d5628b`.
- `proofs/a27_live_same_tick_v2/report.json` (`status=PASS`).
- `proofs/a27_full_reader_fixture/live_promotion_gate.json` (independent gate).
- `proofs/a27_full_reader_fixture/template_scalar_semantics_ghidra.json`
  (static field semantics and complete artifact/manifest provenance).

## Production surface

- `live_a729_acceptance.py`: exact process-memory producer and frozen-snapshot
  re-verifier. A live capture re-reads the date, country/tag roots, every
  membership vector, every definition/lifecycle field, and both full heap
  censuses before admitting the snapshot.
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

The portable v3 artifact preserves the complete native payload after removing
process-local relocation fields. Its smaller replay/UI record projection is not
the complete native definition. D78 display order may join only when its
`frame_hour` has an authenticated A27 census.

## Temporal production path

- `capture_timeline.py` repeats the exact coherent reader and seals monotonic
  hour/sequence observations. Every admitted hour also requires a later
  byte-identical same-hour census. Every recheck digest is retained and must
  equal that hour's canonical snapshot digest; a changed same-hour census is
  fatal because it cannot be bound to a D78 transition without an in-process
  hook.
- The recorder starts that worker before live capture. Attached targets seal a
  baseline before pipe/hook exposure; suspended early-launch targets seal it
  after resume but before collector COMMIT. The recorder stops the worker after
  the pipe closes, requires exact compact-spine coverage, and packages portable
  v3 deltas.
- Upserts retain full pointer-free native rows. Tombstones exist only when a
  stable identity is absent from the next coherent full replacement.
- Replay reconstructs only an explicitly captured hour; missing-hour and
  cross-hour D78 joins reject.
- `Invoke-A27TimelineCapture.ps1` plus `regrade_timeline.py` is the independent
  owner-runnable H-A27 evidence route. It no longer attaches and polls while
  waiting for an operator mutation. The wrapper runs
  `Invoke-A27AuthenticatedTransactions.ps1` with the pinned 1920x1080 profile,
  which renames, priority-edits, decommissions, and reactivates the same first
  domestic division template.
- Before each physical-input transaction the producer requires a byte-equal
  same-hour recheck, then writes a create-new `ready` receipt and suspends all
  censuses. The driver must preserve the paused source hour, perform the UI
  transaction, advance exactly one hour, repause, and publish a create-new
  `release` receipt. Only then does the producer resume. This makes every
  mutation belong to one exact adjacent-hour transition without a race.
- The v2 independent regrade ignores action labels as semantic evidence. It
  requires a same-stable-ID native row delta proving, in order, a name-field
  change, a non-name/non-lifecycle edit, `obsolete: false->true`, and
  `obsolete: true->false`. A missed click, reordered template, torn sweep, or
  unexpected UI state fails closed and leaves the immutable handshake receipts.

The examined creation and designer-refresh routes are not a universal lifecycle
root. H-A27 therefore requires one external machine observation after root
validation, but no human action or semantic interpretation. If the action
profile misses or the external sweep cannot seal every advancing hour, that
machine failure is retained and cannot be promoted by trusting the input
receipt.

The retained frozen snapshot predates the whole-sweep coherence receipt and is
historical evidence only. Production admission now rejects it until a new live
capture supplies `current_date_raw`, `capture_hour_index`, and the double-read
coherence seal. After re-capture, run the gates with the new snapshot:

```powershell
python extractor/native/lanes/design_catalog_live/live_a729_acceptance.py --snapshot-out C:\tmp\a27_current_same_tick\snapshot.json --report-out C:\tmp\a27_current_same_tick\report.json --oracle C:\tmp\a27_current_same_tick\designs.json
python extractor/native/lanes/design_catalog_live/validate_live_producer.py --snapshot C:\tmp\a27_current_same_tick\snapshot.json --oracle C:\tmp\a27_current_same_tick\designs.json
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
