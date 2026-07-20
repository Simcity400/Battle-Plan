# A23/X9 retained-source exhaustion — 2026-07-20

Supported binary: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

This is the fail-closed closeout of retained saves, authored installed content, custom compact captures,
and preserved static artifacts for the two remaining special-project joins. It does not promote
an owner or a specialization name. The machine-readable source hashes, complete 26-facility
oracle, capture census, and exact admission predicates are in
`A23_X9_RETAINED_SOURCE_EXHAUSTION_20260720.json`.

## Outcome

Neither remaining join can be promoted from retained data alone.

- A23/d65 has a strong current-build owner-handle candidate at `CProgram+0x170`, but the retained
  live evidence reads that field on only one GER object. It has no complete 26-object equality
  table and no adjacent negative controls.
- X9 has exactly four authored specialization tokens, but all 101 retained streams containing a
  d41 baseline carry only handles `22214` and `22215`. No retained stream contains four handles,
  and the closest two frames are two raw game hours before the D04 save oracle.
- Source inspection explains the two-handle capture: d41 reads only record 0's
  `CProgramStatus+0x50` breakthrough tree. The separately validated d65 layout walks up to three
  contiguous status records at stride `0x160`. Bytes from records that were never read are not
  recoverable from a compact stream.

The only remaining human prerequisite is one paused, same-boundary process/save pair. One
read-only observation can close both gates; no unpause, mutation, heap scan, or fabricated mapping
is required.

## A23/d65 owner equality

The D04 save oracle serializes 26 facilities. The retained pointer-alias census emitted 21
single-owner objects and dropped five multiply-reachable objects:

| Serialized owner | Program id | Province | Building | Unread rewards | Competing traversal owner(s) |
|---|---:|---:|---|---:|---|
| CHL | 28 | 8222 | `naval_facility` | 0 | BUL |
| IRQ | 25 | 2004 | `land_facility` | 1 | HUN, BRA |
| IRQ | 27 | 4951 | `air_facility` | 1 | HUN, BRA |
| SWE | 23 | 9083 | `air_facility` | 0 | ROM |
| SWE | 24 | 383 | `land_facility` | 0 | ROM |

The current-build constructor/caller evidence proves this duplicate field relationship:

```text
country/tag-handle input -> CProgram+0x18 -> CProgram+0x170
province input           -> CProgram+0x1c -> CProgram+0x174
```

It does not prove that every runtime `+0x170` resolves to the serialized facility owner. The one
retained byte-window observation (`GER`, value `1`) is 1/26 coverage and has no adjacent controls.
The referenced discovery file `05_programs_codex_resolve.json` no longer survives in the retained
filesystem, so pointer identities or the missing windows cannot be reconstructed offline.

The current collector correctly does not read `+0x170`; it emits only pointers reached from one
country and marks the frame `PARTIAL_DATA` when it drops an ambiguous pointer. Keep that behavior
until the exact gate below passes.

## X9 breakthrough handle semantics

### What the authored files prove

`common/special_projects/specialization/specializations.txt` defines exactly:

1. `specialization_nuclear`
2. `specialization_naval`
3. `specialization_air`
4. `specialization_land`

The sibling game documentation says the directory supplies the list of possible specialization
tokens. The definitions contain colors/GFX metadata, not portable numeric handles. A full text-root
census found 270/314/372/324 occurrences respectively, confirming all four symbolic names are
actively authored but adding no runtime-id proof.

The preserved executable search found nuclear, naval, and land strings, but not the exact air
literal. The preserved naval consumer constructs/hashes the naval token and compares interned
object identity. That proves runtime identity semantics, not a stable numeric four-token table.

### What every retained compact capture proves

All 113 retained `extractor/native/runs/**/collector_stream.bin` files were fully framed and
searched for their first d41 data record:

| Result | Count |
|---|---:|
| Streams with d41 data | 101 |
| Streams without d41 data | 12 |
| d41 streams whose exact handle set is `{22214, 22215}` | 101 |
| d41 streams with four handles | 0 |

Baseline breakthrough row counts were 708 in 2 streams, 710 in 49 streams, and 712 in 50 streams.
The first-frame raw-hour range was `60759371..60866468`. This is corpus-wide, not a selected-run
sample.

The D04 oracle date `1944.2.11.18` maps through the repository calendar contract to raw hour
`60830441`. The two closest retained frames (`ab_latch_20260710/w2_on` and `w3_on`) are both at
`60830439`, two hours early, and each has 355 rows for each of the two handles. They fail both
admission preconditions: two handles instead of four and not the same tick. Across the seven
retained exact tag-id anchors (GER, ENG, SOV, FRA, ITA, JAP, USA), none of the 56 comparisons of
the two live columns against the four save columns is equal. The frame cannot be relabelled.

The D04 save itself is an excellent future discriminator: 439 country rows, 113 with any nonzero
points, 326 all-zero rows, and 113 distinct nonzero four-column vectors. The missing half is a
same-boundary live read containing all status records.

## Probe correction

`tools/breakthrough_state_probe.py` is now schema v2. It walks up to three contiguous
primary-vtable-valid `CProgramStatus` records at stride `0x160`, reads every record's embedded
breakthrough tree at `+0x50`, deduplicates identical cross-record handles, and rejects conflicting
point values. Its existing hard gate remains: exactly four handles, zero parse failures, a unique
zero-mismatch permutation, at least one compared row, and no live tag absent from the save.

The probe was not executed in this pass because the user prohibited per-agent heavy/live work. No
Cargo, test runner, verifier, Ghidra process, or game mutation was run.

## Exact missing observation set

### A23-D65-OWNER-EQUALITY

At one coherent paused boundary, record all 26 distinct vtable-valid `CProgram` objects, their
`u32` window covering `+0x18`, `+0x170`, `+0x1c`, `+0x174` and adjacent controls, the validated
`tag_db_id -> TAG` table, and a save oracle from that exact raw hour.

Accept only if:

- `+0x18 == +0x170` and `+0x1c == +0x174` for 26/26 objects;
- `+0x170` resolves to the exact serialized owner for 26/26;
- each of the five multiply-reachable objects selects exactly one serialized owner; and
- adjacent controls fail owner equality.

Any failure retains the five-row drop and `PARTIAL_DATA`.

### X9-BREAKTHROUGH-ALL-RECORD-JOIN

At the same boundary, run the v2 read-only probe against `projects.json` from the exact raw hour.

Accept only if:

- exactly four distinct handles survive all-record deduplication;
- there are zero record parse failures and zero cross-record point conflicts;
- exactly one permutation maps them to the four authored save keys with zero mismatches;
- at least one row is compared; and
- no live tag is absent from the save.

Any failure retains raw numeric handles and forbids semantic specialization names.

## Minimum human input

Provide or authorize one paused D04-equivalent process/save pair at an identical raw game hour.
That single read-only observation is sufficient for both gates. Nothing else found in the retained
corpus can replace it.
