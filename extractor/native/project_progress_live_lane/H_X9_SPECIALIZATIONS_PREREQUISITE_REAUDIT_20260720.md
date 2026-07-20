# H-X9 specialization prerequisite re-audit (2026-07-20)

## Outcome

The production prerequisite for carrying all four breakthrough values is now engineered in source,
but the semantic name mapping is still correctly blocked.

The old d41 collector read only `CProgramStatus` record 0 and accepted a partial tree prefix as usable
data. The updated collector and lane library walk all three validated contiguous records at stride
`0x160`, read each inline `CBreakthroughProgress` at `+0x50`, merge identical aliases, reject
same-tree duplicates or conflicting aliases, and reject a whole country's snapshot if any tree is
torn. The collector also
revalidates the cached `CCountry` vtable and live `tag_db_id` at the d41 read boundary. A rejected
country increments the existing invalid count, which makes d41 emit `PARTIAL_DATA`; the fold then
keeps special-project state unknown for that hour rather than fabricating disappearance or
persistence.

No build, compile, Cargo, test, or verifier command was run in this subagent lane. The static tests
were added for the primary agent's one central validation pass.

## Fresh Ghidra gate

The fresh broker status was pinned to the correct binary identity:

- `hoi4.exe 1.19.2.0.a729`
- image base `0x140000000`
- SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`

It reported only 10 ready shards, not the lane-required exact 20. No Ghidra work was submitted. The
root subsequently froze broker submissions while restarting cached brokers. The grounded follow-up
seeds are therefore packaged, unexecuted, in
`H_X9_SPECIALIZATIONS_GHIDRA_SEEDS_PENDING_20_SHARDS_20260720.json`.

## Retained evidence exhausted

The existing exhaustive census remains decisive:

- 113 retained `collector_stream.bin` files were fully scanned;
- 101 contained d41 data;
- every one of those 101 contained exactly handles `{22214, 22215}`;
- no retained stream contained four handles;
- the closest retained a729 live frames were at raw hour `60830439`, while the D04 save oracle was
  raw hour `60830441`;
- across 56 known-tag anchor comparisons, that two-hour-off pair produced zero exact value matches.

It is therefore invalid to name `22214` or `22215`, and the two handles hidden by the record-zero
collector cannot be inferred.

Two newly retained save-side discriminators were also mined:

| Save oracle | Build | Countries | Any nonzero | Distinct four-value vectors | Admission |
|---|---:|---:|---:|---:|---|
| `SOV_1946_05_19_09/country_extra.json` | `1.19.1.0.b49d (1722)` | 143 | 46 | 47 | Rejected for a729 mapping |
| `GER_1940_09_10_13/country_extra.json` | `1.19.1.0.b49d (d4f6)` | 1 | 1 | 1 | Rejected for a729 mapping |

The GER vector is especially discriminating—nuclear `6204`, naval `11895`, air `7932`, land
`19390`—but both sources are save-only, from a different executable build, and lack an exact-tick
live d41 pairing. They cannot authenticate a729 runtime handles.

The authored game-file universe remains exactly:

1. `specialization_nuclear`
2. `specialization_naval`
3. `specialization_air`
4. `specialization_land`

Retained Ghidra evidence proves literal presence for nuclear/naval/land and a runtime
construction/hash/object-identity consumer for naval. It does not prove a portable four-handle
table. The exact `specialization_air` literal was absent from the retained executable string search;
that negative result must be preserved, not repaired by guessing.

The collector's air-variant path contains an authenticated token-record heap scanner that can map a
wanted u32 token id to exact text while rejecting ambiguity. It is not admissible for X9 yet: no
retained producer/consumer join proves that `CBreakthroughProgress` tree keys inhabit that same token
id namespace. Equal u32 values across unjoined namespaces would still be correlation. The pending
Ghidra package explicitly targets that join; the exact-boundary live/save permutation is the
independent dynamic route.

## Owner equality

The d41 and d65 owner questions are separate:

- d41 starts from the validated `CCountry` identity row. The new boundary recheck requires the live
  object's country vtable and `CCountry` `tag_db_id` to equal the cached identity before reading any
  status record. This is the correct owner for that country's breakthrough values.
- d65 starts from `CProgram` pointers that can be reachable from multiple country traversals. Those
  multiply-reachable facilities remain dropped. `CProgram+0x170` is still only a candidate owner
  field because retained coverage is 1/26 and lacks adjacent negative controls.

No d65 owner inference is used to label d41 breakthrough rows.

## Compact, fold, and replay semantics

No wire expansion is needed for four values. A d41 breakthrough row is already 16 bytes:

`tag_db_id:u32, specialization_handle:u32, points_raw:u32, flags:u32`

The stable key is `(tag_db_id, specialization_handle)`. The collector delta tracker therefore
supports four rows per country without a schema change. The fold rejects zero or duplicate keys,
rejects unknown flags, applies valid full/delta rows, and retains the domain as unknown on
`PARTIAL_DATA`.

The folded projection preserves each raw value under
`special_projects._r2_breakthroughs` with `country`, `tag_db_id`, `points_raw`, and
`_r2_specialization_handle`. `specialization` intentionally remains `null`. Replay core retains the
entire raw top-level JSON map in `WorldFrame` and exposes it through `raw()` and `domain()`, so the
four raw rows survive replay ingestion without data loss. There is no typed semantic specialization
view yet; adding one before the exact mapping would only turn an unknown into a guess.

## Source changes awaiting central validation

- `extractor/native/collector/src/lib.rs`
  - all-record d41 acquisition;
  - country-atomic fail-closed merge;
  - live country owner equality recheck;
  - tests for three-record collection, alias dedupe/conflict, torn-tree rejection, and stale owner
    rejection.
- `extractor/native/project_progress_live_lane/src/lib.rs`
  - reusable all-record reader;
  - a four-value regression fixture across three status records;
  - conflict and torn-tree rejection fixtures.

## Minimum remaining human input

One coherent paused a729 process/save boundary is still required after the all-record collector or
schema-v2 probe is deployed. The same raw hour must provide both live status records and
`projects.json`.

Promotion requires all of the following mechanically:

- exactly four distinct live handles;
- zero record parse failures;
- zero cross-record conflicts;
- at least one compared country row;
- exactly one zero-mismatch permutation to the four authored save keys;
- zero live tags missing from the save.

If that observation fails any condition, production must continue emitting raw numeric handles with
`specialization: null`. No additional human transcription or judgment is required.
